//! Transactional event ingress ledger and Event Schedule fan-out.

use hachimi_protocol::{
    DeliveryPolicy, DeliveryStatus, ScheduleContextTemplate, ScheduleDefinition,
    ScheduleEventContext, ScheduleEventMatcher, ScheduleEventReceipt, ScheduleEventReceiptStatus,
    ScheduleSpec, TaskRunId, TaskRunRecord, TaskRunStatus, TaskRunTrigger,
};
use sha2::{Digest, Sha256};
use sqlx::Row;

use super::{
    AgentStore, AgentStoreError, enum_from_db, enum_to_db,
    schedule::{
        ScheduleInvocationClaim, claim_schedule_invocation_tx, schedule_from_row, task_run_from_row,
    },
};

const EVENT_LEDGER_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1_000;
const EVENT_LEDGER_CLEANUP_BATCH: i64 = 500;

#[derive(Debug, Clone)]
pub struct ScheduleEventLaunchClaim {
    pub schedule: ScheduleDefinition,
    pub claim: ScheduleInvocationClaim,
}

#[derive(Debug, Clone)]
pub struct ScheduleEventIngestClaim {
    pub receipt: ScheduleEventReceipt,
    pub launch_claims: Vec<ScheduleEventLaunchClaim>,
}

impl AgentStore {
    pub async fn ingest_schedule_event(
        &self,
        event: &ScheduleEventContext,
    ) -> Result<ScheduleEventIngestClaim, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let source_kind = enum_to_db(&event.source.kind)?;
        let existing = sqlx::query(
            "SELECT * FROM schedule_event_ledger WHERE source_kind = ? AND source_principal = ? AND source_id = ? AND event_id = ?",
        )
        .bind(&source_kind)
        .bind(&event.source.principal)
        .bind(&event.source.id)
        .bind(&event.event_id)
        .fetch_optional(&mut *transaction)
        .await?;

        if let Some(row) = existing {
            let fingerprint: String = row.try_get("fingerprint")?;
            if fingerprint != event.fingerprint {
                sqlx::query(
                    "UPDATE schedule_event_ledger SET processing_status = 'conflict', conflict_count = conflict_count + 1, last_received_at_ms = ? WHERE source_kind = ? AND source_principal = ? AND source_id = ? AND event_id = ?",
                )
                .bind(event.received_at_ms)
                .bind(&source_kind)
                .bind(&event.source.principal)
                .bind(&event.source.id)
                .bind(&event.event_id)
                .execute(&mut *transaction)
                .await?;
                transaction.commit().await?;
                return Err(AgentStoreError::ScheduleEventConflict);
            }
            let persisted = event_context_from_row(&row)?;
            sqlx::query(
                "UPDATE schedule_event_ledger SET processing_status = 'replayed', replay_count = replay_count + 1, last_received_at_ms = ? WHERE source_kind = ? AND source_principal = ? AND source_id = ? AND event_id = ?",
            )
            .bind(event.received_at_ms)
            .bind(&source_kind)
            .bind(&event.source.principal)
            .bind(&event.source.id)
            .bind(&event.event_id)
            .execute(&mut *transaction)
            .await?;
            let receipt = event_receipt_tx(
                &mut transaction,
                &persisted,
                ScheduleEventReceiptStatus::Replayed,
            )
            .await?;
            transaction.commit().await?;
            return Ok(ScheduleEventIngestClaim {
                receipt,
                launch_claims: Vec::new(),
            });
        }

        sqlx::query(
            "INSERT INTO schedule_event_ledger (source_kind, source_principal, source_id, event_id, fingerprint, event_type, subject, labels_json, resource_json, occurred_at_ms, received_at_ms, processing_status, matched_schedule_count, replay_count, conflict_count, last_received_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'accepted', 0, 0, 0, ?)",
        )
        .bind(&source_kind)
        .bind(&event.source.principal)
        .bind(&event.source.id)
        .bind(&event.event_id)
        .bind(&event.fingerprint)
        .bind(&event.event_type)
        .bind(&event.subject)
        .bind(serde_json::to_string(&event.labels)?)
        .bind(event.resource.as_ref().map(serde_json::to_string).transpose()?)
        .bind(event.occurred_at_ms)
        .bind(event.received_at_ms)
        .bind(event.received_at_ms)
        .execute(&mut *transaction)
        .await?;

        let rows = sqlx::query(
            "SELECT * FROM schedule_definitions WHERE enabled = 1 AND health IN ('healthy', 'needs_authorization') ORDER BY id",
        )
        .fetch_all(&mut *transaction)
        .await?;
        let schedules = rows
            .iter()
            .map(schedule_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let mut launch_claims = Vec::new();
        let mut task_runs = Vec::new();
        let mut matched_schedule_ids = Vec::new();
        for schedule in schedules {
            let ScheduleSpec::Event { matcher } = &schedule.schedule else {
                continue;
            };
            if !event_matches(matcher, event) {
                continue;
            }
            let occurrence_count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM task_runs WHERE schedule_id = ? AND status <> 'skipped'",
            )
            .bind(schedule.id.as_str())
            .fetch_one(&mut *transaction)
            .await?;
            let reached_limit = schedule
                .stop_conditions
                .max_occurrences
                .is_some_and(|limit| {
                    u64::try_from(occurrence_count).unwrap_or_default() >= u64::from(limit)
                });
            let reached_end = schedule
                .stop_conditions
                .end_at_ms
                .is_some_and(|end_at| event.received_at_ms > end_at);
            if reached_limit || reached_end {
                sqlx::query(
                    "UPDATE schedule_definitions SET enabled = 0, next_run_at_ms = NULL, config_revision = config_revision + 1, updated_at_ms = ? WHERE id = ? AND enabled = 1",
                )
                .bind(event.received_at_ms)
                .bind(schedule.id.as_str())
                .execute(&mut *transaction)
                .await?;
                continue;
            }
            let task = event_task_record(&schedule, event);
            let claim = claim_schedule_invocation_tx(
                &mut transaction,
                &schedule.id,
                schedule.config_revision,
                &task,
            )
            .await?;
            sqlx::query(
                "INSERT INTO schedule_event_task_runs (source_kind, source_principal, source_id, event_id, schedule_id, task_run_id) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&source_kind)
            .bind(&event.source.principal)
            .bind(&event.source.id)
            .bind(&event.event_id)
            .bind(schedule.id.as_str())
            .bind(claim.task_run.id.as_str())
            .execute(&mut *transaction)
            .await?;
            matched_schedule_ids.push(schedule.id.clone());
            task_runs.push(claim.task_run.clone());
            launch_claims.push(ScheduleEventLaunchClaim { schedule, claim });
        }
        sqlx::query(
            "UPDATE schedule_event_ledger SET matched_schedule_count = ? WHERE source_kind = ? AND source_principal = ? AND source_id = ? AND event_id = ?",
        )
        .bind(i64::try_from(matched_schedule_ids.len()).unwrap_or(i64::MAX))
        .bind(&source_kind)
        .bind(&event.source.principal)
        .bind(&event.source.id)
        .bind(&event.event_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;

        Ok(ScheduleEventIngestClaim {
            receipt: ScheduleEventReceipt {
                status: ScheduleEventReceiptStatus::Accepted,
                event: event.clone(),
                matched_schedule_ids,
                task_runs,
            },
            launch_claims,
        })
    }

    pub async fn cleanup_schedule_event_ledger(&self, now_ms: i64) -> Result<u64, AgentStoreError> {
        let cutoff = now_ms.saturating_sub(EVENT_LEDGER_RETENTION_MS);
        let result = sqlx::query(
            "DELETE FROM schedule_event_ledger WHERE rowid IN (SELECT ledger.rowid FROM schedule_event_ledger AS ledger WHERE ledger.last_received_at_ms < ? AND NOT EXISTS (SELECT 1 FROM schedule_event_task_runs AS links INNER JOIN task_runs ON task_runs.id = links.task_run_id WHERE links.source_kind = ledger.source_kind AND links.source_principal = ledger.source_principal AND links.source_id = ledger.source_id AND links.event_id = ledger.event_id AND task_runs.status IN ('queued', 'preparing', 'running')) ORDER BY ledger.last_received_at_ms LIMIT ?)",
        )
        .bind(cutoff)
        .bind(EVENT_LEDGER_CLEANUP_BATCH)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn list_schedule_event_receipts(
        &self,
        limit: u32,
    ) -> Result<Vec<ScheduleEventReceipt>, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let rows = sqlx::query(
            "SELECT * FROM schedule_event_ledger ORDER BY last_received_at_ms DESC, source_kind, source_principal, source_id, event_id LIMIT ?",
        )
        .bind(i64::from(limit.clamp(1, 200)))
        .fetch_all(&mut *transaction)
        .await?;
        let mut receipts = Vec::with_capacity(rows.len());
        for row in rows {
            let event = event_context_from_row(&row)?;
            let status = match row.try_get::<String, _>("processing_status")?.as_str() {
                "accepted" => ScheduleEventReceiptStatus::Accepted,
                "replayed" => ScheduleEventReceiptStatus::Replayed,
                "conflict" => ScheduleEventReceiptStatus::Conflict,
                value => {
                    return Err(AgentStoreError::InvalidPersistedValue {
                        kind: "schedule event processing status",
                        value: value.to_owned(),
                    });
                }
            };
            receipts.push(event_receipt_tx(&mut transaction, &event, status).await?);
        }
        transaction.commit().await?;
        Ok(receipts)
    }
}

async fn event_receipt_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    event: &ScheduleEventContext,
    status: ScheduleEventReceiptStatus,
) -> Result<ScheduleEventReceipt, AgentStoreError> {
    let source_kind = enum_to_db(&event.source.kind)?;
    let rows = sqlx::query(
        "SELECT task_runs.* FROM schedule_event_task_runs INNER JOIN task_runs ON task_runs.id = schedule_event_task_runs.task_run_id WHERE schedule_event_task_runs.source_kind = ? AND schedule_event_task_runs.source_principal = ? AND schedule_event_task_runs.source_id = ? AND schedule_event_task_runs.event_id = ? ORDER BY schedule_event_task_runs.schedule_id",
    )
    .bind(source_kind)
    .bind(&event.source.principal)
    .bind(&event.source.id)
    .bind(&event.event_id)
    .fetch_all(&mut **transaction)
    .await?;
    let task_runs = rows
        .iter()
        .map(task_run_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    let matched_schedule_ids = task_runs
        .iter()
        .filter_map(|task| task.schedule_id.clone())
        .collect();
    Ok(ScheduleEventReceipt {
        status,
        event: event.clone(),
        matched_schedule_ids,
        task_runs,
    })
}

fn event_matches(matcher: &ScheduleEventMatcher, event: &ScheduleEventContext) -> bool {
    matcher.source == event.source
        && matcher.event_type == event.event_type
        && matcher.subject_prefix.as_ref().is_none_or(|prefix| {
            event
                .subject
                .as_ref()
                .is_some_and(|subject| subject.starts_with(prefix))
        })
        && matcher
            .labels
            .iter()
            .all(|(key, value)| event.labels.get(key) == Some(value))
        && matcher
            .resource
            .as_ref()
            .is_none_or(|resource| event.resource.as_ref() == Some(resource))
}

fn event_task_record(schedule: &ScheduleDefinition, event: &ScheduleEventContext) -> TaskRunRecord {
    let id = TaskRunId::random();
    let invocation_key = event_invocation_key(schedule, event);
    TaskRunRecord {
        id,
        schedule_id: Some(schedule.id.clone()),
        schedule_revision: Some(schedule.config_revision),
        trigger: TaskRunTrigger::Event,
        scheduled_for_ms: Some(event.occurred_at_ms),
        event_context: Some(event.clone()),
        invocation_key,
        requester_session_id: match &schedule.context_template {
            ScheduleContextTemplate::SessionContinuation { session_id } => Some(session_id.clone()),
            _ => None,
        },
        execution_session_id: None,
        run_id: None,
        permission_snapshot_hash: None,
        status: TaskRunStatus::Queued,
        progress_percent: None,
        result_summary: None,
        error_code: None,
        error_summary: None,
        artifact_ids: Vec::new(),
        delivery_status: if schedule.delivery_policy == DeliveryPolicy::TaskTabAndSystemNotification
        {
            DeliveryStatus::Pending
        } else {
            DeliveryStatus::NotRequested
        },
        delivery_error_code: None,
        created_at_ms: event.received_at_ms,
        started_at_ms: None,
        finished_at_ms: None,
        updated_at_ms: event.received_at_ms,
    }
}

fn event_invocation_key(schedule: &ScheduleDefinition, event: &ScheduleEventContext) -> String {
    let material = format!(
        "{}\0{}\0{}\0{}\0{}\0{}\0{}",
        enum_to_db(&event.source.kind).unwrap_or_default(),
        event.source.principal,
        event.source.id,
        event.event_id,
        event.fingerprint,
        schedule.id,
        schedule.config_revision,
    );
    let digest = Sha256::digest(material.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("event:{digest}")
}

fn event_context_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<ScheduleEventContext, AgentStoreError> {
    Ok(ScheduleEventContext {
        event_id: row.try_get("event_id")?,
        source: hachimi_protocol::ScheduleEventSource {
            kind: enum_from_db(row.try_get("source_kind")?, "schedule event source kind")?,
            principal: row.try_get("source_principal")?,
            id: row.try_get("source_id")?,
        },
        event_type: row.try_get("event_type")?,
        subject: row.try_get("subject")?,
        labels: serde_json::from_str(row.try_get("labels_json")?)?,
        resource: row
            .try_get::<Option<String>, _>("resource_json")?
            .map(|value| serde_json::from_str(&value))
            .transpose()?,
        fingerprint: row.try_get("fingerprint")?,
        occurred_at_ms: row.try_get("occurred_at_ms")?,
        received_at_ms: row.try_get("received_at_ms")?,
    })
}
