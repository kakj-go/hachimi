// SPDX-License-Identifier: MIT
// Copyright (c) 2026 OpenClaw Foundation
// Adapted from openclaw/openclaw src/cron/service/task-runs.ts, src/tasks/task-registry.store.sqlite.ts, and src/cron/config-revision.ts
// Commit: f6d456235cf011004f7cffc71a95acf6fbf1fa0a
// Modified for Hachimi: transactional ScheduleGrant snapshots, invocation keys, Session/Run lineage, and SQLite projections.

use hachimi_protocol::{
    ArtifactId, DeliveryStatus, RunId, ScheduleDefinition, ScheduleGrantId, ScheduleGrantRecord,
    ScheduleGrantStatus, ScheduleHealth, ScheduleId, ScheduleSnapshot, SessionId, TaskRunId,
    TaskRunRecord, TaskRunStatus,
};
use serde::{Serialize, de::DeserializeOwned};
use sqlx::{Row, Sqlite, Transaction};

use super::{AgentStore, AgentStoreError, enum_from_db, enum_to_db};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleInvocationClaim {
    pub task_run: TaskRunRecord,
    pub should_launch: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdempotentMutationClaim<T> {
    Claimed,
    Completed(T),
    Indeterminate,
}

const IDEMPOTENCY_CLAIMED: &str = "{\"state\":\"claimed\"}";

impl AgentStore {
    pub async fn claim_idempotent_mutation<T: DeserializeOwned>(
        &self,
        principal: &str,
        method: &str,
        idempotency_key: &str,
        resource_id: &str,
        now_ms: i64,
    ) -> Result<IdempotentMutationClaim<T>, AgentStoreError> {
        let result = sqlx::query(
            "INSERT OR IGNORE INTO idempotency_records (principal, method, idempotency_key, resource_id, response_json, created_at_ms) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(principal)
        .bind(method)
        .bind(idempotency_key)
        .bind(resource_id)
        .bind(IDEMPOTENCY_CLAIMED)
        .bind(now_ms)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 1 {
            return Ok(IdempotentMutationClaim::Claimed);
        }
        let row = sqlx::query(
            "SELECT resource_id, response_json FROM idempotency_records WHERE principal = ? AND method = ? AND idempotency_key = ?",
        )
        .bind(principal)
        .bind(method)
        .bind(idempotency_key)
        .fetch_one(&self.pool)
        .await?;
        let persisted_resource: String = row.try_get("resource_id")?;
        if persisted_resource != resource_id {
            return Err(AgentStoreError::IdempotencyConflict);
        }
        let response: String = row.try_get("response_json")?;
        if response == IDEMPOTENCY_CLAIMED {
            return Ok(IdempotentMutationClaim::Indeterminate);
        }
        Ok(IdempotentMutationClaim::Completed(serde_json::from_str(
            &response,
        )?))
    }

    pub async fn complete_idempotent_mutation<T: Serialize>(
        &self,
        principal: &str,
        method: &str,
        idempotency_key: &str,
        response: &T,
    ) -> Result<(), AgentStoreError> {
        let response = serde_json::to_string(response)?;
        let result = sqlx::query(
            "UPDATE idempotency_records SET response_json = ? WHERE principal = ? AND method = ? AND idempotency_key = ? AND response_json = ?",
        )
        .bind(response)
        .bind(principal)
        .bind(method)
        .bind(idempotency_key)
        .bind(IDEMPOTENCY_CLAIMED)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            return Err(AgentStoreError::IdempotencyConflict);
        }
        Ok(())
    }

    pub async fn abandon_idempotent_mutation(
        &self,
        principal: &str,
        method: &str,
        idempotency_key: &str,
    ) -> Result<(), AgentStoreError> {
        sqlx::query(
            "DELETE FROM idempotency_records WHERE principal = ? AND method = ? AND idempotency_key = ? AND response_json = ?",
        )
        .bind(principal)
        .bind(method)
        .bind(idempotency_key)
        .bind(IDEMPOTENCY_CLAIMED)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn create_schedule_idempotent(
        &self,
        principal: &str,
        idempotency_key: &str,
        definition: &ScheduleDefinition,
        grant: Option<&ScheduleGrantRecord>,
    ) -> Result<ScheduleSnapshot, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        if let Some(existing_id) = sqlx::query_scalar::<_, String>(
            "SELECT resource_id FROM idempotency_records WHERE principal = ? AND method = 'schedule.create' AND idempotency_key = ?",
        )
        .bind(principal)
        .bind(idempotency_key)
        .fetch_optional(&mut *transaction)
        .await?
        {
            let definition = get_schedule_tx(&mut transaction, &ScheduleId::new(existing_id))
                .await?
                .ok_or_else(|| AgentStoreError::ScheduleNotFound(definition.id.clone()))?;
            let active_grant = active_schedule_grant_tx(&mut transaction, &definition.id).await?;
            transaction.commit().await?;
            return Ok(ScheduleSnapshot {
                definition,
                active_grant,
                recent_runs: Vec::new(),
            });
        }

        insert_schedule_tx(&mut transaction, definition).await?;
        if let Some(grant) = grant {
            upsert_schedule_grant_tx(&mut transaction, grant).await?;
        }
        sqlx::query(
            "INSERT INTO idempotency_records (principal, method, idempotency_key, resource_id, response_json, created_at_ms) VALUES (?, 'schedule.create', ?, ?, '{}', ?)",
        )
        .bind(principal)
        .bind(idempotency_key)
        .bind(definition.id.as_str())
        .bind(definition.created_at_ms)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(ScheduleSnapshot {
            definition: definition.clone(),
            active_grant: grant.cloned(),
            recent_runs: Vec::new(),
        })
    }

    pub async fn get_schedule(
        &self,
        schedule_id: &ScheduleId,
    ) -> Result<Option<ScheduleDefinition>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM schedule_definitions WHERE id = ?")
            .bind(schedule_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(schedule_from_row).transpose()
    }

    pub async fn list_schedules(&self) -> Result<Vec<ScheduleDefinition>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM schedule_definitions ORDER BY enabled DESC, next_run_at_ms IS NULL, next_run_at_ms ASC, updated_at_ms DESC, id ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(schedule_from_row).collect()
    }

    pub async fn get_schedule_snapshot(
        &self,
        schedule_id: &ScheduleId,
    ) -> Result<Option<ScheduleSnapshot>, AgentStoreError> {
        let Some(definition) = self.get_schedule(schedule_id).await? else {
            return Ok(None);
        };
        let active_grant = self.active_schedule_grant(schedule_id).await?;
        let recent_runs = self.list_task_runs(Some(schedule_id), 50).await?;
        Ok(Some(ScheduleSnapshot {
            definition,
            active_grant,
            recent_runs,
        }))
    }

    pub async fn list_due_schedules(
        &self,
        now_ms: i64,
    ) -> Result<Vec<ScheduleDefinition>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM schedule_definitions WHERE enabled = 1 AND health = 'healthy' AND next_run_at_ms IS NOT NULL AND next_run_at_ms <= ? ORDER BY next_run_at_ms ASC, id ASC",
        )
        .bind(now_ms)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(schedule_from_row).collect()
    }

    pub async fn next_schedule_wakeup(&self) -> Result<Option<i64>, AgentStoreError> {
        Ok(sqlx::query_scalar(
            "SELECT MIN(next_run_at_ms) FROM schedule_definitions WHERE enabled = 1 AND health = 'healthy' AND next_run_at_ms IS NOT NULL",
        )
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn update_schedule(
        &self,
        definition: &ScheduleDefinition,
        expected_config_revision: u64,
    ) -> Result<ScheduleDefinition, AgentStoreError> {
        if definition.config_revision != expected_config_revision.saturating_add(1) {
            return Err(AgentStoreError::ScheduleRevisionConflict);
        }
        let result = sqlx::query(
            "UPDATE schedule_definitions SET name = ?, enabled = ?, prompt = ?, schedule_json = ?, entry_profile = ?, workload_override = ?, context_template_json = ?, tool_allowlist_json = ?, skill_allowlist_json = ?, mcp_tool_allowlist_json = ?, permission_config_json = ?, permission_revision = ?, timeout_ms = ?, misfire_policy = ?, delivery_policy = ?, config_revision = ?, next_run_at_ms = ?, health = ?, health_reason = ?, updated_at_ms = ? WHERE id = ? AND config_revision = ?",
        )
        .bind(&definition.name)
        .bind(definition.enabled)
        .bind(&definition.prompt)
        .bind(serde_json::to_string(&definition.schedule)?)
        .bind(enum_to_db(&definition.entry_profile)?)
        .bind(
            definition
                .workload_override
                .as_ref()
                .map(enum_to_db)
                .transpose()?,
        )
        .bind(serde_json::to_string(&definition.context_template)?)
        .bind(serde_json::to_string(&definition.tool_allowlist)?)
        .bind(serde_json::to_string(&definition.skill_allowlist)?)
        .bind(serde_json::to_string(&definition.mcp_tool_allowlist)?)
        .bind(serde_json::to_string(&definition.permission_config)?)
        .bind(i64::try_from(definition.permission_revision).unwrap_or(i64::MAX))
        .bind(i64::try_from(definition.timeout_ms).unwrap_or(i64::MAX))
        .bind(enum_to_db(&definition.misfire_policy)?)
        .bind(enum_to_db(&definition.delivery_policy)?)
        .bind(i64::try_from(definition.config_revision).unwrap_or(i64::MAX))
        .bind(definition.next_run_at_ms)
        .bind(definition.health.as_str())
        .bind(&definition.health_reason)
        .bind(definition.updated_at_ms)
        .bind(definition.id.as_str())
        .bind(i64::try_from(expected_config_revision).unwrap_or(i64::MAX))
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            return Err(AgentStoreError::ScheduleRevisionConflict);
        }
        Ok(definition.clone())
    }

    pub async fn remove_schedule(&self, schedule_id: &ScheduleId) -> Result<bool, AgentStoreError> {
        let result = sqlx::query("DELETE FROM schedule_definitions WHERE id = ?")
            .bind(schedule_id.as_str())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn set_schedule_enabled(
        &self,
        schedule_id: &ScheduleId,
        enabled: bool,
        expected_config_revision: u64,
        next_run_at_ms: Option<i64>,
        updated_at_ms: i64,
    ) -> Result<ScheduleDefinition, AgentStoreError> {
        let result = sqlx::query(
            "UPDATE schedule_definitions SET enabled = ?, next_run_at_ms = ?, config_revision = config_revision + 1, updated_at_ms = ? WHERE id = ? AND config_revision = ?",
        )
        .bind(enabled)
        .bind(next_run_at_ms)
        .bind(updated_at_ms)
        .bind(schedule_id.as_str())
        .bind(i64::try_from(expected_config_revision).unwrap_or(i64::MAX))
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            return Err(AgentStoreError::ScheduleRevisionConflict);
        }
        self.get_schedule(schedule_id)
            .await?
            .ok_or_else(|| AgentStoreError::ScheduleNotFound(schedule_id.clone()))
    }

    pub async fn update_schedule_next_run(
        &self,
        schedule_id: &ScheduleId,
        next_run_at_ms: Option<i64>,
        updated_at_ms: i64,
    ) -> Result<(), AgentStoreError> {
        let result = sqlx::query(
            "UPDATE schedule_definitions SET next_run_at_ms = ?, updated_at_ms = ? WHERE id = ?",
        )
        .bind(next_run_at_ms)
        .bind(updated_at_ms)
        .bind(schedule_id.as_str())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            return Err(AgentStoreError::ScheduleNotFound(schedule_id.clone()));
        }
        Ok(())
    }

    pub async fn mark_schedule_health(
        &self,
        schedule_id: &ScheduleId,
        health: ScheduleHealth,
        reason: Option<&str>,
        updated_at_ms: i64,
    ) -> Result<(), AgentStoreError> {
        let result = sqlx::query(
            "UPDATE schedule_definitions SET health = ?, health_reason = ?, updated_at_ms = ? WHERE id = ?",
        )
        .bind(health.as_str())
        .bind(reason)
        .bind(updated_at_ms)
        .bind(schedule_id.as_str())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            return Err(AgentStoreError::ScheduleNotFound(schedule_id.clone()));
        }
        Ok(())
    }

    pub async fn active_schedule_grant(
        &self,
        schedule_id: &ScheduleId,
    ) -> Result<Option<ScheduleGrantRecord>, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let grant = active_schedule_grant_tx(&mut transaction, schedule_id).await?;
        transaction.commit().await?;
        Ok(grant)
    }

    pub async fn reauthorize_schedule(
        &self,
        grant: &ScheduleGrantRecord,
    ) -> Result<ScheduleGrantRecord, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let schedule = get_schedule_tx(&mut transaction, &grant.schedule_id)
            .await?
            .ok_or_else(|| AgentStoreError::ScheduleNotFound(grant.schedule_id.clone()))?;
        if schedule.permission_revision != grant.permission_revision {
            return Err(AgentStoreError::ScheduleRevisionConflict);
        }
        upsert_schedule_grant_tx(&mut transaction, grant).await?;
        sqlx::query(
            "UPDATE schedule_definitions SET health = 'healthy', health_reason = NULL, updated_at_ms = ? WHERE id = ?",
        )
        .bind(grant.created_at_ms)
        .bind(grant.schedule_id.as_str())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(grant.clone())
    }

    pub async fn revoke_schedule_grant(
        &self,
        schedule_id: &ScheduleId,
        revoked_at_ms: i64,
    ) -> Result<Option<ScheduleGrantRecord>, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let Some(mut grant) = active_schedule_grant_tx(&mut transaction, schedule_id).await? else {
            transaction.commit().await?;
            return Ok(None);
        };
        sqlx::query(
            "UPDATE schedule_grants SET status = 'revoked', revoked_at_ms = ? WHERE id = ? AND status = 'active'",
        )
        .bind(revoked_at_ms)
        .bind(grant.id.as_str())
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE schedule_definitions SET health = 'needs_authorization', health_reason = 'schedule_grant_revoked', updated_at_ms = ? WHERE id = ?",
        )
        .bind(revoked_at_ms)
        .bind(schedule_id.as_str())
        .execute(&mut *transaction)
        .await?;
        grant.status = ScheduleGrantStatus::Revoked;
        grant.revoked_at_ms = Some(revoked_at_ms);
        transaction.commit().await?;
        Ok(Some(grant))
    }

    pub async fn create_task_run(
        &self,
        task: &TaskRunRecord,
    ) -> Result<TaskRunRecord, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        insert_task_run_tx(&mut transaction, task).await?;
        transaction.commit().await?;
        Ok(task.clone())
    }

    pub async fn claim_schedule_invocation(
        &self,
        schedule_id: &ScheduleId,
        expected_schedule_revision: u64,
        task: &TaskRunRecord,
    ) -> Result<ScheduleInvocationClaim, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        if let Some(row) = sqlx::query("SELECT * FROM task_runs WHERE invocation_key = ?")
            .bind(&task.invocation_key)
            .fetch_optional(&mut *transaction)
            .await?
        {
            let existing = task_run_from_row(&row)?;
            transaction.commit().await?;
            return Ok(ScheduleInvocationClaim {
                // The first claimant owns dispatch. A repeated timer/request observes the
                // existing ledger row but must never create a second side effect.
                should_launch: false,
                task_run: existing,
            });
        }

        let schedule = get_schedule_tx(&mut transaction, schedule_id)
            .await?
            .ok_or_else(|| AgentStoreError::ScheduleNotFound(schedule_id.clone()))?;
        if schedule.config_revision != expected_schedule_revision {
            return Err(AgentStoreError::ScheduleRevisionConflict);
        }
        let active_grant = active_schedule_grant_tx(&mut transaction, schedule_id).await?;
        let active_task = sqlx::query(
            "SELECT task_runs.* FROM schedule_runtime_state JOIN task_runs ON task_runs.id = schedule_runtime_state.active_task_run_id WHERE schedule_runtime_state.schedule_id = ?",
        )
        .bind(schedule_id.as_str())
        .fetch_optional(&mut *transaction)
        .await?
        .as_ref()
        .map(task_run_from_row)
        .transpose()?;

        let mut claimed = task.clone();
        let should_launch = if active_task.is_some_and(|active| !active.status.is_terminal()) {
            claimed.status = TaskRunStatus::Skipped;
            claimed.error_code = Some("schedule_overlap_skipped".into());
            claimed.error_summary = Some("a previous invocation is still active".into());
            claimed.finished_at_ms = Some(claimed.created_at_ms);
            false
        } else if let Some(grant) =
            active_grant.filter(|grant| grant.permission_revision == schedule.permission_revision)
        {
            claimed.permission_snapshot_hash = Some(grant.scope_hash);
            !claimed.status.is_terminal()
        } else {
            claimed.status = TaskRunStatus::NeedsAttention;
            claimed.error_code = Some("schedule_authorization_required".into());
            claimed.error_summary =
                Some("the current permission revision is not authorized".into());
            claimed.finished_at_ms = Some(claimed.created_at_ms);
            sqlx::query(
                "UPDATE schedule_definitions SET health = 'needs_authorization', health_reason = 'schedule_authorization_required', updated_at_ms = ? WHERE id = ?",
            )
            .bind(claimed.created_at_ms)
            .bind(schedule_id.as_str())
            .execute(&mut *transaction)
            .await?;
            false
        };

        insert_task_run_tx(&mut transaction, &claimed).await?;
        sqlx::query(
            "UPDATE schedule_runtime_state SET last_scheduled_for_ms = ?, last_invocation_key = ?, active_task_run_id = ?, timer_generation = timer_generation + 1, updated_at_ms = ? WHERE schedule_id = ?",
        )
        .bind(claimed.scheduled_for_ms)
        .bind(&claimed.invocation_key)
        .bind(should_launch.then_some(claimed.id.as_str()))
        .bind(claimed.updated_at_ms)
        .bind(schedule_id.as_str())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(ScheduleInvocationClaim {
            task_run: claimed,
            should_launch,
        })
    }

    pub async fn get_task_run(
        &self,
        task_run_id: &TaskRunId,
    ) -> Result<Option<TaskRunRecord>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM task_runs WHERE id = ?")
            .bind(task_run_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(task_run_from_row).transpose()
    }

    pub async fn bind_task_run_execution(
        &self,
        task_run_id: &TaskRunId,
        session_id: &SessionId,
        run_id: &RunId,
        updated_at_ms: i64,
    ) -> Result<TaskRunRecord, AgentStoreError> {
        let result = sqlx::query(
            "UPDATE task_runs SET execution_session_id = ?, run_id = ?, status = 'preparing', updated_at_ms = ? WHERE id = ? AND status = 'queued'",
        )
        .bind(session_id.as_str())
        .bind(run_id.as_str())
        .bind(updated_at_ms)
        .bind(task_run_id.as_str())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            return Err(AgentStoreError::InvalidTaskRunTransition);
        }
        self.get_task_run(task_run_id)
            .await?
            .ok_or_else(|| AgentStoreError::TaskRunNotFound(task_run_id.clone()))
    }

    pub async fn bind_task_run_requester(
        &self,
        task_run_id: &TaskRunId,
        session_id: &SessionId,
        updated_at_ms: i64,
    ) -> Result<TaskRunRecord, AgentStoreError> {
        let result = sqlx::query(
            "UPDATE task_runs SET requester_session_id = ?, updated_at_ms = ? WHERE id = ? AND requester_session_id IS NULL",
        )
        .bind(session_id.as_str())
        .bind(updated_at_ms)
        .bind(task_run_id.as_str())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            return Err(AgentStoreError::InvalidTaskRunTransition);
        }
        self.get_task_run(task_run_id)
            .await?
            .ok_or_else(|| AgentStoreError::TaskRunNotFound(task_run_id.clone()))
    }

    pub async fn list_task_runs(
        &self,
        schedule_id: Option<&ScheduleId>,
        limit: u32,
    ) -> Result<Vec<TaskRunRecord>, AgentStoreError> {
        let limit = i64::from(limit.clamp(1, 500));
        let rows = if let Some(schedule_id) = schedule_id {
            sqlx::query(
                "SELECT * FROM task_runs WHERE schedule_id = ? ORDER BY created_at_ms DESC, id ASC LIMIT ?",
            )
            .bind(schedule_id.as_str())
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query("SELECT * FROM task_runs ORDER BY created_at_ms DESC, id ASC LIMIT ?")
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
        };
        rows.iter().map(task_run_from_row).collect()
    }

    pub async fn count_schedule_retained_worktrees(
        &self,
        schedule_id: &ScheduleId,
    ) -> Result<u64, AgentStoreError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(DISTINCT workspace_checkouts.id) FROM task_runs INNER JOIN sessions ON sessions.id = task_runs.execution_session_id INNER JOIN workspace_checkouts ON workspace_checkouts.id = json_extract(sessions.context_json, '$.checkout_id') WHERE task_runs.schedule_id = ? AND sessions.context_kind = 'project' AND workspace_checkouts.kind = 'managed_worktree' AND workspace_checkouts.status = 'cleanup_blocked'",
        )
        .bind(schedule_id.as_str())
        .fetch_one(&self.pool)
        .await?;
        Ok(u64::try_from(count).unwrap_or_default())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn transition_task_run(
        &self,
        task_run_id: &TaskRunId,
        next: TaskRunStatus,
        progress_percent: Option<u8>,
        result_summary: Option<&str>,
        error_code: Option<&str>,
        error_summary: Option<&str>,
        artifact_ids: &[ArtifactId],
        updated_at_ms: i64,
    ) -> Result<TaskRunRecord, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query("SELECT * FROM task_runs WHERE id = ?")
            .bind(task_run_id.as_str())
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| AgentStoreError::TaskRunNotFound(task_run_id.clone()))?;
        let current = task_run_from_row(&row)?;
        if current.status != next && !current.status.can_transition_to(next) {
            return Err(AgentStoreError::InvalidTaskRunTransition);
        }
        let started_at_ms = if next == TaskRunStatus::Running && current.started_at_ms.is_none() {
            Some(updated_at_ms)
        } else {
            current.started_at_ms
        };
        let finished_at_ms = next.is_terminal().then_some(updated_at_ms);
        sqlx::query(
            "UPDATE task_runs SET status = ?, progress_percent = ?, result_summary = ?, error_code = ?, error_summary = ?, artifact_ids_json = ?, started_at_ms = ?, finished_at_ms = ?, updated_at_ms = ? WHERE id = ?",
        )
        .bind(next.as_str())
        .bind(progress_percent.map(i64::from))
        .bind(result_summary)
        .bind(error_code)
        .bind(error_summary)
        .bind(serde_json::to_string(artifact_ids)?)
        .bind(started_at_ms)
        .bind(finished_at_ms)
        .bind(updated_at_ms)
        .bind(task_run_id.as_str())
        .execute(&mut *transaction)
        .await?;
        if next.is_terminal() {
            sqlx::query(
                "UPDATE schedule_runtime_state SET active_task_run_id = NULL, updated_at_ms = ? WHERE active_task_run_id = ?",
            )
            .bind(updated_at_ms)
            .bind(task_run_id.as_str())
            .execute(&mut *transaction)
            .await?;
        }
        let row = sqlx::query("SELECT * FROM task_runs WHERE id = ?")
            .bind(task_run_id.as_str())
            .fetch_one(&mut *transaction)
            .await?;
        let task = task_run_from_row(&row)?;
        transaction.commit().await?;
        Ok(task)
    }

    pub async fn update_task_delivery(
        &self,
        task_run_id: &TaskRunId,
        status: DeliveryStatus,
        error_code: Option<&str>,
        updated_at_ms: i64,
    ) -> Result<TaskRunRecord, AgentStoreError> {
        let result = sqlx::query(
            "UPDATE task_runs SET delivery_status = ?, delivery_error_code = ?, updated_at_ms = ? WHERE id = ?",
        )
        .bind(enum_to_db(&status)?)
        .bind(error_code)
        .bind(updated_at_ms)
        .bind(task_run_id.as_str())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            return Err(AgentStoreError::TaskRunNotFound(task_run_id.clone()));
        }
        self.get_task_run(task_run_id)
            .await?
            .ok_or_else(|| AgentStoreError::TaskRunNotFound(task_run_id.clone()))
    }
}

async fn insert_schedule_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    definition: &ScheduleDefinition,
) -> Result<(), AgentStoreError> {
    sqlx::query(
        "INSERT INTO schedule_definitions (id, name, enabled, prompt, schedule_json, entry_profile, workload_override, context_template_json, tool_allowlist_json, skill_allowlist_json, mcp_tool_allowlist_json, permission_config_json, permission_revision, timeout_ms, misfire_policy, delivery_policy, config_revision, created_by, next_run_at_ms, health, health_reason, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(definition.id.as_str())
    .bind(&definition.name)
    .bind(definition.enabled)
    .bind(&definition.prompt)
    .bind(serde_json::to_string(&definition.schedule)?)
    .bind(enum_to_db(&definition.entry_profile)?)
    .bind(
        definition
            .workload_override
            .as_ref()
            .map(enum_to_db)
            .transpose()?,
    )
    .bind(serde_json::to_string(&definition.context_template)?)
    .bind(serde_json::to_string(&definition.tool_allowlist)?)
    .bind(serde_json::to_string(&definition.skill_allowlist)?)
    .bind(serde_json::to_string(&definition.mcp_tool_allowlist)?)
    .bind(serde_json::to_string(&definition.permission_config)?)
    .bind(i64::try_from(definition.permission_revision).unwrap_or(i64::MAX))
    .bind(i64::try_from(definition.timeout_ms).unwrap_or(i64::MAX))
    .bind(enum_to_db(&definition.misfire_policy)?)
    .bind(enum_to_db(&definition.delivery_policy)?)
    .bind(i64::try_from(definition.config_revision).unwrap_or(i64::MAX))
    .bind(&definition.created_by)
    .bind(definition.next_run_at_ms)
    .bind(definition.health.as_str())
    .bind(&definition.health_reason)
    .bind(definition.created_at_ms)
    .bind(definition.updated_at_ms)
    .execute(&mut **transaction)
    .await?;
    sqlx::query(
        "INSERT INTO schedule_runtime_state (schedule_id, timer_generation, updated_at_ms) VALUES (?, 0, ?)",
    )
        .bind(definition.id.as_str())
        .bind(definition.updated_at_ms)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn get_schedule_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    schedule_id: &ScheduleId,
) -> Result<Option<ScheduleDefinition>, AgentStoreError> {
    let row = sqlx::query("SELECT * FROM schedule_definitions WHERE id = ?")
        .bind(schedule_id.as_str())
        .fetch_optional(&mut **transaction)
        .await?;
    row.as_ref().map(schedule_from_row).transpose()
}

fn schedule_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<ScheduleDefinition, AgentStoreError> {
    let health_value: String = row.get("health");
    let health = ScheduleHealth::parse(&health_value).ok_or_else(|| {
        AgentStoreError::InvalidPersistedValue {
            kind: "schedule health",
            value: health_value,
        }
    })?;
    Ok(ScheduleDefinition {
        id: ScheduleId::new(row.get::<String, _>("id")),
        name: row.get("name"),
        enabled: row.get("enabled"),
        prompt: row.get("prompt"),
        schedule: serde_json::from_str(row.get("schedule_json"))?,
        entry_profile: enum_from_db(row.get("entry_profile"), "entry profile")?,
        workload_override: row
            .get::<Option<String>, _>("workload_override")
            .map(|value| enum_from_db(&value, "workload override"))
            .transpose()?,
        context_template: serde_json::from_str(row.get("context_template_json"))?,
        tool_allowlist: serde_json::from_str(row.get("tool_allowlist_json"))?,
        skill_allowlist: serde_json::from_str(row.get("skill_allowlist_json"))?,
        mcp_tool_allowlist: serde_json::from_str(row.get("mcp_tool_allowlist_json"))?,
        permission_config: serde_json::from_str(row.get("permission_config_json"))?,
        permission_revision: u64::try_from(row.get::<i64, _>("permission_revision"))
            .unwrap_or_default(),
        timeout_ms: u64::try_from(row.get::<i64, _>("timeout_ms")).unwrap_or_default(),
        misfire_policy: enum_from_db(row.get("misfire_policy"), "misfire policy")?,
        delivery_policy: enum_from_db(row.get("delivery_policy"), "delivery policy")?,
        config_revision: u64::try_from(row.get::<i64, _>("config_revision")).unwrap_or_default(),
        created_by: row.get("created_by"),
        next_run_at_ms: row.get("next_run_at_ms"),
        health,
        health_reason: row.get("health_reason"),
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

async fn upsert_schedule_grant_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    grant: &ScheduleGrantRecord,
) -> Result<(), AgentStoreError> {
    sqlx::query(
        "UPDATE schedule_grants SET status = 'superseded', revoked_at_ms = ? WHERE schedule_id = ? AND status = 'active' AND id <> ?",
    )
    .bind(grant.created_at_ms)
    .bind(grant.schedule_id.as_str())
    .bind(grant.id.as_str())
    .execute(&mut **transaction)
    .await?;
    sqlx::query(
        "INSERT INTO schedule_grants (id, schedule_id, permission_revision, scope_hash, scope_json, status, granted_by, created_at_ms, revoked_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(schedule_id, permission_revision) DO UPDATE SET id = excluded.id, scope_hash = excluded.scope_hash, scope_json = excluded.scope_json, status = excluded.status, granted_by = excluded.granted_by, created_at_ms = excluded.created_at_ms, revoked_at_ms = excluded.revoked_at_ms",
    )
    .bind(grant.id.as_str())
    .bind(grant.schedule_id.as_str())
    .bind(i64::try_from(grant.permission_revision).unwrap_or(i64::MAX))
    .bind(&grant.scope_hash)
    .bind(serde_json::to_string(&grant.scope)?)
    .bind(grant.status.as_str())
    .bind(&grant.granted_by)
    .bind(grant.created_at_ms)
    .bind(grant.revoked_at_ms)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn active_schedule_grant_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    schedule_id: &ScheduleId,
) -> Result<Option<ScheduleGrantRecord>, AgentStoreError> {
    let row = sqlx::query(
        "SELECT * FROM schedule_grants WHERE schedule_id = ? AND status = 'active' ORDER BY created_at_ms DESC LIMIT 1",
    )
    .bind(schedule_id.as_str())
    .fetch_optional(&mut **transaction)
    .await?;
    row.as_ref().map(schedule_grant_from_row).transpose()
}

fn schedule_grant_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<ScheduleGrantRecord, AgentStoreError> {
    let status_value: String = row.get("status");
    let status = ScheduleGrantStatus::parse(&status_value).ok_or_else(|| {
        AgentStoreError::InvalidPersistedValue {
            kind: "schedule grant status",
            value: status_value,
        }
    })?;
    Ok(ScheduleGrantRecord {
        id: ScheduleGrantId::new(row.get::<String, _>("id")),
        schedule_id: ScheduleId::new(row.get::<String, _>("schedule_id")),
        permission_revision: u64::try_from(row.get::<i64, _>("permission_revision"))
            .unwrap_or_default(),
        scope_hash: row.get("scope_hash"),
        scope: serde_json::from_str(row.get("scope_json"))?,
        status,
        granted_by: row.get("granted_by"),
        created_at_ms: row.get("created_at_ms"),
        revoked_at_ms: row.get("revoked_at_ms"),
    })
}

async fn insert_task_run_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    task: &TaskRunRecord,
) -> Result<(), AgentStoreError> {
    sqlx::query(
        "INSERT INTO task_runs (id, schedule_id, schedule_revision, trigger, scheduled_for_ms, invocation_key, requester_session_id, execution_session_id, run_id, permission_snapshot_hash, status, progress_percent, result_summary, error_code, error_summary, artifact_ids_json, delivery_status, delivery_error_code, created_at_ms, started_at_ms, finished_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(task.id.as_str())
    .bind(task.schedule_id.as_ref().map(ScheduleId::as_str))
    .bind(task.schedule_revision.map(|value| i64::try_from(value).unwrap_or(i64::MAX)))
    .bind(enum_to_db(&task.trigger)?)
    .bind(task.scheduled_for_ms)
    .bind(&task.invocation_key)
    .bind(task.requester_session_id.as_ref().map(SessionId::as_str))
    .bind(task.execution_session_id.as_ref().map(SessionId::as_str))
    .bind(task.run_id.as_ref().map(RunId::as_str))
    .bind(&task.permission_snapshot_hash)
    .bind(task.status.as_str())
    .bind(task.progress_percent.map(i64::from))
    .bind(&task.result_summary)
    .bind(&task.error_code)
    .bind(&task.error_summary)
    .bind(serde_json::to_string(&task.artifact_ids)?)
    .bind(enum_to_db(&task.delivery_status)?)
    .bind(&task.delivery_error_code)
    .bind(task.created_at_ms)
    .bind(task.started_at_ms)
    .bind(task.finished_at_ms)
    .bind(task.updated_at_ms)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn task_run_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<TaskRunRecord, AgentStoreError> {
    let status_value: String = row.get("status");
    let status = TaskRunStatus::parse(&status_value).ok_or_else(|| {
        AgentStoreError::InvalidPersistedValue {
            kind: "task run status",
            value: status_value,
        }
    })?;
    Ok(TaskRunRecord {
        id: TaskRunId::new(row.get::<String, _>("id")),
        schedule_id: row
            .get::<Option<String>, _>("schedule_id")
            .map(ScheduleId::new),
        schedule_revision: row
            .get::<Option<i64>, _>("schedule_revision")
            .and_then(|value| u64::try_from(value).ok()),
        trigger: enum_from_db(row.get("trigger"), "task run trigger")?,
        scheduled_for_ms: row.get("scheduled_for_ms"),
        invocation_key: row.get("invocation_key"),
        requester_session_id: row
            .get::<Option<String>, _>("requester_session_id")
            .map(SessionId::new),
        execution_session_id: row
            .get::<Option<String>, _>("execution_session_id")
            .map(SessionId::new),
        run_id: row.get::<Option<String>, _>("run_id").map(RunId::new),
        permission_snapshot_hash: row.get("permission_snapshot_hash"),
        status,
        progress_percent: row
            .get::<Option<i64>, _>("progress_percent")
            .and_then(|value| u8::try_from(value).ok()),
        result_summary: row.get("result_summary"),
        error_code: row.get("error_code"),
        error_summary: row.get("error_summary"),
        artifact_ids: serde_json::from_str(row.get("artifact_ids_json"))?,
        delivery_status: enum_from_db(row.get("delivery_status"), "delivery status")?,
        delivery_error_code: row.get("delivery_error_code"),
        created_at_ms: row.get("created_at_ms"),
        started_at_ms: row.get("started_at_ms"),
        finished_at_ms: row.get("finished_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}
