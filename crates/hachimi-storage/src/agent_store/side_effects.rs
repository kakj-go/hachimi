use hachimi_protocol::{
    ApprovalGrantScope, ApprovalId, ApprovalStatus, ArtifactId, RunId, RunStepCheckpointId,
    RunStepPhase, SideEffectExecutionId, SideEffectExecutionRecord, SideEffectExecutionStatus,
};
use serde_json::Value;
use sqlx::Row;

use super::{
    AgentStore, AgentStoreError, SQLITE_BEGIN_IMMEDIATE, enum_from_db, enum_to_db, get_run_tx,
};

#[derive(Debug, Clone, PartialEq)]
pub struct SideEffectClaim {
    pub record: SideEffectExecutionRecord,
    pub created: bool,
    pub persisted_result: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SideEffectAuthority<'a> {
    pub action: &'a str,
    pub resource: &'a str,
    pub target_host: &'a str,
}

impl AgentStore {
    pub async fn list_side_effects_for_run(
        &self,
        run_id: &RunId,
    ) -> Result<Vec<SideEffectExecutionRecord>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM side_effect_executions WHERE run_id = ? ORDER BY created_at_ms, id",
        )
        .bind(run_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(side_effect_from_row).collect()
    }

    pub async fn get_side_effect_claim(
        &self,
        run_id: &RunId,
        run_generation: u64,
        idempotency_key: &str,
    ) -> Result<Option<SideEffectClaim>, AgentStoreError> {
        let row = sqlx::query(
            "SELECT * FROM side_effect_executions WHERE run_id = ? AND run_generation = ? AND idempotency_key = ?",
        )
        .bind(run_id.as_str())
        .bind(i64::try_from(run_generation).unwrap_or(i64::MAX))
        .bind(idempotency_key)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            let record = side_effect_from_row(&row)?;
            let persisted_result = row
                .get::<Option<String>, _>("result_json")
                .map(|value| serde_json::from_str(&value))
                .transpose()?;
            Ok(SideEffectClaim {
                record,
                created: false,
                persisted_result,
            })
        })
        .transpose()
    }

    pub async fn claim_side_effect(
        &self,
        record: &SideEffectExecutionRecord,
    ) -> Result<SideEffectClaim, AgentStoreError> {
        self.claim_side_effect_with_authority(record, None).await
    }

    pub async fn claim_side_effect_with_authority(
        &self,
        record: &SideEffectExecutionRecord,
        authority: Option<SideEffectAuthority<'_>>,
    ) -> Result<SideEffectClaim, AgentStoreError> {
        if record.status != SideEffectExecutionStatus::Claimed {
            return Err(AgentStoreError::InvalidSideEffectTransition {
                from: record.status,
                to: SideEffectExecutionStatus::Claimed,
            });
        }
        let mut transaction = self.pool.begin_with(SQLITE_BEGIN_IMMEDIATE).await?;
        let run = get_run_tx(&mut transaction, &record.run_id)
            .await?
            .ok_or_else(|| AgentStoreError::RunNotFound(record.run_id.clone()))?;
        if run.session_id != record.session_id || run.generation != record.run_generation {
            return Err(AgentStoreError::RunPreconditionFailed);
        }
        if let Some(row) = sqlx::query(
            "SELECT * FROM side_effect_executions WHERE run_id = ? AND run_generation = ? AND idempotency_key = ?",
        )
        .bind(record.run_id.as_str())
        .bind(i64::try_from(record.run_generation).unwrap_or(i64::MAX))
        .bind(&record.idempotency_key)
        .fetch_optional(&mut *transaction)
        .await?
        {
            let existing = side_effect_from_row(&row)?;
            if existing.parameter_hash != record.parameter_hash
                || existing.tool_call_id != record.tool_call_id
            {
                return Err(AgentStoreError::SideEffectIdempotencyConflict);
            }
            let persisted_result = row
                .get::<Option<String>, _>("result_json")
                .map(|value| serde_json::from_str(&value))
                .transpose()?;
            transaction.commit().await?;
            return Ok(SideEffectClaim {
                record: existing,
                created: false,
                persisted_result,
            });
        }
        if run.status != hachimi_protocol::RunStatus::Running {
            return Err(AgentStoreError::RunPreconditionFailed);
        }
        if let Some(approval_id) = &record.approval_id {
            consume_approval(&mut transaction, approval_id, record, authority).await?;
        }
        sqlx::query(
            "INSERT INTO side_effect_executions (id, session_id, run_id, run_generation, tool_call_id, idempotency_key, parameter_hash, approval_id, host_request_id, status, result_code, result_artifact_id, result_json, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?)",
        )
        .bind(record.id.as_str())
        .bind(record.session_id.as_str())
        .bind(record.run_id.as_str())
        .bind(i64::try_from(record.run_generation).unwrap_or(i64::MAX))
        .bind(record.tool_call_id.as_str())
        .bind(&record.idempotency_key)
        .bind(&record.parameter_hash)
        .bind(record.approval_id.as_ref().map(ApprovalId::as_str))
        .bind(&record.host_request_id)
        .bind(enum_to_db(&record.status)?)
        .bind(&record.result_code)
        .bind(record.result_reference.as_ref().map(ArtifactId::as_str))
        .bind(record.created_at_ms)
        .bind(record.updated_at_ms)
        .execute(&mut *transaction)
        .await?;
        copy_side_effect_checkpoint_tx(
            &mut transaction,
            record,
            RunStepPhase::ToolClaimed,
            record.created_at_ms,
        )
        .await?;
        transaction.commit().await?;
        Ok(SideEffectClaim {
            record: record.clone(),
            created: true,
            persisted_result: None,
        })
    }

    pub async fn mark_side_effect_dispatched(
        &self,
        id: &SideEffectExecutionId,
        host_request_id: &str,
        updated_at_ms: i64,
    ) -> Result<SideEffectExecutionRecord, AgentStoreError> {
        self.transition_side_effect(
            id,
            SideEffectExecutionStatus::Claimed,
            SideEffectExecutionStatus::Dispatched,
            Some(host_request_id),
            None,
            None,
            None,
            updated_at_ms,
        )
        .await
    }

    pub async fn mark_side_effect_dispatched_if_current(
        &self,
        id: &SideEffectExecutionId,
        run_id: &RunId,
        expected_generation: u64,
        host_request_id: &str,
        updated_at_ms: i64,
    ) -> Result<SideEffectExecutionRecord, AgentStoreError> {
        let mut transaction = self.pool.begin_with(SQLITE_BEGIN_IMMEDIATE).await?;
        let run = get_run_tx(&mut transaction, run_id)
            .await?
            .ok_or_else(|| AgentStoreError::RunNotFound(run_id.clone()))?;
        if run.generation != expected_generation
            || run.status != hachimi_protocol::RunStatus::Running
        {
            return Err(AgentStoreError::RunPreconditionFailed);
        }
        let row = sqlx::query("SELECT * FROM side_effect_executions WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| AgentStoreError::SideEffectNotFound(id.clone()))?;
        let current = side_effect_from_row(&row)?;
        if current.run_id != *run_id
            || current.run_generation != expected_generation
            || current.status != SideEffectExecutionStatus::Claimed
        {
            return Err(AgentStoreError::RunPreconditionFailed);
        }
        sqlx::query(
            "UPDATE side_effect_executions SET status = ?, host_request_id = ?, updated_at_ms = ? WHERE id = ? AND status = ?",
        )
        .bind(enum_to_db(&SideEffectExecutionStatus::Dispatched)?)
        .bind(host_request_id)
        .bind(updated_at_ms)
        .bind(id.as_str())
        .bind(enum_to_db(&SideEffectExecutionStatus::Claimed)?)
        .execute(&mut *transaction)
        .await?;
        let row = sqlx::query("SELECT * FROM side_effect_executions WHERE id = ?")
            .bind(id.as_str())
            .fetch_one(&mut *transaction)
            .await?;
        let updated = side_effect_from_row(&row)?;
        copy_side_effect_checkpoint_tx(
            &mut transaction,
            &updated,
            RunStepPhase::ToolDispatched,
            updated_at_ms,
        )
        .await?;
        transaction.commit().await?;
        Ok(updated)
    }

    pub async fn cancel_claimed_side_effect(
        &self,
        id: &SideEffectExecutionId,
        updated_at_ms: i64,
    ) -> Result<SideEffectExecutionRecord, AgentStoreError> {
        self.transition_side_effect(
            id,
            SideEffectExecutionStatus::Claimed,
            SideEffectExecutionStatus::Cancelled,
            None,
            Some("cancelled_before_dispatch"),
            None,
            None,
            updated_at_ms,
        )
        .await
    }

    pub async fn finish_side_effect(
        &self,
        id: &SideEffectExecutionId,
        status: SideEffectExecutionStatus,
        result_code: Option<&str>,
        result_reference: Option<&ArtifactId>,
        persisted_result: Option<&Value>,
        updated_at_ms: i64,
    ) -> Result<SideEffectExecutionRecord, AgentStoreError> {
        if !matches!(
            status,
            SideEffectExecutionStatus::Succeeded
                | SideEffectExecutionStatus::Failed
                | SideEffectExecutionStatus::Cancelled
                | SideEffectExecutionStatus::Indeterminate
        ) {
            return Err(AgentStoreError::InvalidSideEffectTransition {
                from: SideEffectExecutionStatus::Dispatched,
                to: status,
            });
        }
        self.transition_side_effect(
            id,
            SideEffectExecutionStatus::Dispatched,
            status,
            None,
            result_code,
            result_reference,
            persisted_result,
            updated_at_ms,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn transition_side_effect(
        &self,
        id: &SideEffectExecutionId,
        expected: SideEffectExecutionStatus,
        next: SideEffectExecutionStatus,
        host_request_id: Option<&str>,
        result_code: Option<&str>,
        result_reference: Option<&ArtifactId>,
        persisted_result: Option<&Value>,
        updated_at_ms: i64,
    ) -> Result<SideEffectExecutionRecord, AgentStoreError> {
        let mut transaction = self.pool.begin_with(SQLITE_BEGIN_IMMEDIATE).await?;
        let row = sqlx::query("SELECT * FROM side_effect_executions WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| AgentStoreError::SideEffectNotFound(id.clone()))?;
        let current = side_effect_from_row(&row)?;
        if current.status != expected {
            return Err(AgentStoreError::InvalidSideEffectTransition {
                from: current.status,
                to: next,
            });
        }
        sqlx::query(
            "UPDATE side_effect_executions SET status = ?, host_request_id = COALESCE(?, host_request_id), result_code = ?, result_artifact_id = ?, result_json = ?, updated_at_ms = ? WHERE id = ? AND status = ?",
        )
        .bind(enum_to_db(&next)?)
        .bind(host_request_id)
        .bind(result_code)
        .bind(result_reference.map(ArtifactId::as_str))
        .bind(persisted_result.map(serde_json::to_string).transpose()?)
        .bind(updated_at_ms)
        .bind(id.as_str())
        .bind(enum_to_db(&expected)?)
        .execute(&mut *transaction)
        .await?;
        let row = sqlx::query("SELECT * FROM side_effect_executions WHERE id = ?")
            .bind(id.as_str())
            .fetch_one(&mut *transaction)
            .await?;
        let updated = side_effect_from_row(&row)?;
        transaction.commit().await?;
        Ok(updated)
    }
}

async fn copy_side_effect_checkpoint_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    record: &SideEffectExecutionRecord,
    phase: RunStepPhase,
    created_at_ms: i64,
) -> Result<(), AgentStoreError> {
    let source = sqlx::query(
        "SELECT step_index, tool_name, recovery_policy, parameter_hash, world_revision, provider_revision, revision_snapshot_json FROM run_step_checkpoints WHERE run_id = ? AND run_generation = ? AND tool_call_id = ? ORDER BY step_index DESC, created_at_ms DESC, rowid DESC LIMIT 1",
    )
    .bind(record.run_id.as_str())
    .bind(i64::try_from(record.run_generation).unwrap_or(i64::MAX))
    .bind(record.tool_call_id.as_str())
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(source) = source else {
        return Ok(());
    };
    sqlx::query(
        "INSERT OR IGNORE INTO run_step_checkpoints (id, session_id, run_id, run_generation, step_index, phase, tool_call_id, tool_name, side_effect_execution_id, recovery_policy, parameter_hash, world_revision, provider_revision, revision_snapshot_json, created_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(RunStepCheckpointId::random().as_str())
    .bind(record.session_id.as_str())
    .bind(record.run_id.as_str())
    .bind(i64::try_from(record.run_generation).unwrap_or(i64::MAX))
    .bind(source.get::<i64, _>("step_index"))
    .bind(phase.as_str())
    .bind(record.tool_call_id.as_str())
    .bind(source.get::<Option<String>, _>("tool_name"))
    .bind(record.id.as_str())
    .bind(source.get::<String, _>("recovery_policy"))
    .bind(source.get::<Option<String>, _>("parameter_hash"))
    .bind(source.get::<String, _>("world_revision"))
    .bind(source.get::<String, _>("provider_revision"))
    .bind(source.get::<String, _>("revision_snapshot_json"))
    .bind(created_at_ms)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn consume_approval(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    approval_id: &ApprovalId,
    record: &SideEffectExecutionRecord,
    authority: Option<SideEffectAuthority<'_>>,
) -> Result<(), AgentStoreError> {
    let grant_scope =
        sqlx::query_scalar::<_, String>("SELECT grant_scope FROM approval_requests WHERE id = ?")
            .bind(approval_id.as_str())
            .fetch_optional(&mut **transaction)
            .await?
            .ok_or(AgentStoreError::SideEffectApprovalInvalid)?;
    let grant_scope = enum_from_db(&grant_scope, "approval grant scope")?;
    let affected = match grant_scope {
        ApprovalGrantScope::Session => {
            let Some(authority) = authority else {
                return Err(AgentStoreError::SideEffectApprovalInvalid);
            };
            sqlx::query(
                "UPDATE approval_requests SET uses_remaining = uses_remaining - 1 WHERE id = ? AND session_id = ? AND action = ? AND resource = ? AND target_host = ? AND grant_scope = ? AND status = ? AND uses_remaining > 0",
            )
            .bind(approval_id.as_str())
            .bind(record.session_id.as_str())
            .bind(authority.action)
            .bind(authority.resource)
            .bind(authority.target_host)
            .bind("session")
            .bind(ApprovalStatus::Approved.as_str())
            .execute(&mut **transaction)
            .await?
            .rows_affected()
        }
        ApprovalGrantScope::Once | ApprovalGrantScope::TimedLease => sqlx::query(
            "UPDATE approval_requests SET uses_remaining = uses_remaining - 1 WHERE id = ? AND session_id = ? AND run_id = ? AND run_generation = ? AND tool_call_id = ? AND parameter_hash = ? AND status = ? AND uses_remaining > 0",
        )
        .bind(approval_id.as_str())
        .bind(record.session_id.as_str())
        .bind(record.run_id.as_str())
        .bind(i64::try_from(record.run_generation).unwrap_or(i64::MAX))
        .bind(record.tool_call_id.as_str())
        .bind(&record.parameter_hash)
        .bind(ApprovalStatus::Approved.as_str())
        .execute(&mut **transaction)
        .await?
        .rows_affected(),
    };
    if affected != 1 {
        return Err(AgentStoreError::SideEffectApprovalInvalid);
    }
    Ok(())
}

fn side_effect_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<SideEffectExecutionRecord, AgentStoreError> {
    Ok(SideEffectExecutionRecord {
        id: SideEffectExecutionId::new(row.get::<String, _>("id")),
        session_id: hachimi_protocol::SessionId::new(row.get::<String, _>("session_id")),
        run_id: RunId::new(row.get::<String, _>("run_id")),
        run_generation: u64::try_from(row.get::<i64, _>("run_generation")).unwrap_or_default(),
        tool_call_id: hachimi_protocol::ToolCallId::new(row.get::<String, _>("tool_call_id")),
        idempotency_key: row.get("idempotency_key"),
        parameter_hash: row.get("parameter_hash"),
        approval_id: row
            .get::<Option<String>, _>("approval_id")
            .map(ApprovalId::new),
        host_request_id: row.get("host_request_id"),
        status: enum_from_db(row.get("status"), "side-effect status")?,
        result_code: row.get("result_code"),
        result_reference: row
            .get::<Option<String>, _>("result_artifact_id")
            .map(ArtifactId::new),
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

#[cfg(test)]
mod tests {
    use hachimi_protocol::{
        ApprovalRequestRecord, ApprovalResolution, ApprovalStatus, RunStatus, SessionId, ToolCallId,
    };

    use super::*;
    use crate::agent_store::tests::{
        create_running_run, seeded_store, seeded_store_at, side_effect,
    };

    async fn approve(
        store: &AgentStore,
        session_id: &SessionId,
        run: &hachimi_protocol::RunRecord,
        scope: ApprovalGrantScope,
    ) -> ApprovalRequestRecord {
        store
            .transition_run(&run.id, RunStatus::WaitingApproval, None)
            .await
            .expect("wait for approval");
        let timestamp = super::super::now_ms();
        let approval = ApprovalRequestRecord {
            id: ApprovalId::random(),
            session_id: session_id.clone(),
            run_id: run.id.clone(),
            tool_call_id: ToolCallId::from("call-side-effect"),
            run_generation: run.generation,
            status: ApprovalStatus::Pending,
            action: "git.push".into(),
            resource: "workspace-checkout".into(),
            parameter_hash: "sha256:original".into(),
            risk_summary: "push commits".into(),
            target_host: "git-host".into(),
            required_scopes: vec!["git.mutate".into()],
            grant_scope: scope,
            uses_remaining: if scope == ApprovalGrantScope::Session {
                u32::MAX
            } else {
                1
            },
            requester_principal: "user".into(),
            resolved_by: None,
            expires_at_ms: None,
            created_at_ms: timestamp,
            resolved_at_ms: None,
        };
        store.create_approval(&approval).await.expect("approval");
        store
            .resolve_approval(&ApprovalResolution {
                approval_id: approval.id.clone(),
                decision: ApprovalStatus::Approved,
                parameter_hash: approval.parameter_hash.clone(),
                run_generation: run.generation,
                resolved_by: "user".into(),
                resolved_at_ms: timestamp + 1,
            })
            .await
            .expect("resolve approval");
        store
            .transition_run(&run.id, RunStatus::Running, None)
            .await
            .expect("resume run");
        approval
    }

    #[tokio::test]
    async fn session_approval_reuse_requires_the_approved_authority_tuple() {
        let (store, session) = seeded_store().await;
        let first_run = create_running_run(&store, &session, "session-authority-first").await;
        let approval = approve(&store, &session.id, &first_run, ApprovalGrantScope::Session).await;
        let second_run = create_running_run(&store, &session, "session-authority-second").await;
        let record = side_effect(
            &session,
            &second_run,
            "session-authority-effect",
            "sha256:changed-parameters",
            Some(approval.id.clone()),
        );

        for authority in [
            SideEffectAuthority {
                action: "forge.change.mutate",
                resource: &approval.resource,
                target_host: &approval.target_host,
            },
            SideEffectAuthority {
                action: &approval.action,
                resource: "different-resource",
                target_host: &approval.target_host,
            },
            SideEffectAuthority {
                action: &approval.action,
                resource: &approval.resource,
                target_host: "different-host",
            },
        ] {
            assert!(matches!(
                store
                    .claim_side_effect_with_authority(&record, Some(authority))
                    .await,
                Err(AgentStoreError::SideEffectApprovalInvalid)
            ));
        }

        let claim = store
            .claim_side_effect_with_authority(
                &record,
                Some(SideEffectAuthority {
                    action: &approval.action,
                    resource: &approval.resource,
                    target_host: &approval.target_host,
                }),
            )
            .await
            .expect("reuse session authority");
        assert!(claim.created);
        let remaining = sqlx::query_scalar::<_, i64>(
            "SELECT uses_remaining FROM approval_requests WHERE id = ?",
        )
        .bind(approval.id.as_str())
        .fetch_one(store.pool())
        .await
        .expect("remaining uses");
        assert_eq!(remaining, i64::from(u32::MAX) - 1);
    }

    #[tokio::test]
    async fn once_approval_still_requires_exact_call_and_parameters() {
        let (store, session) = seeded_store().await;
        let run = create_running_run(&store, &session, "once-authority").await;
        let approval = approve(&store, &session.id, &run, ApprovalGrantScope::Once).await;
        let mut wrong = side_effect(
            &session,
            &run,
            "once-authority-wrong",
            "sha256:changed",
            Some(approval.id.clone()),
        );
        assert!(matches!(
            store
                .claim_side_effect_with_authority(
                    &wrong,
                    Some(SideEffectAuthority {
                        action: &approval.action,
                        resource: &approval.resource,
                        target_host: &approval.target_host,
                    })
                )
                .await,
            Err(AgentStoreError::SideEffectApprovalInvalid)
        ));

        wrong.parameter_hash = approval.parameter_hash;
        assert!(
            store
                .claim_side_effect_with_authority(
                    &wrong,
                    Some(SideEffectAuthority {
                        action: "untrusted-action-is-irrelevant-for-once",
                        resource: "untrusted-resource",
                        target_host: "untrusted-host",
                    })
                )
                .await
                .expect("exact once approval")
                .created
        );
    }

    #[tokio::test]
    async fn dispatch_waits_for_a_competing_writer_before_reading_preconditions() {
        let fixture = tempfile::tempdir().expect("side-effect fixture");
        let database = fixture.path().join("agent.sqlite3");
        let (store, session) = seeded_store_at(&database).await;
        let competing = AgentStore::connect(&database)
            .await
            .expect("competing store");
        let run = create_running_run(&store, &session, "dispatch-writer-contention").await;
        let record = side_effect(
            &session,
            &run,
            "dispatch-writer-contention-effect",
            "sha256:contention",
            None,
        );
        store.claim_side_effect(&record).await.expect("claim");

        let mut writer = competing
            .pool()
            .begin_with(SQLITE_BEGIN_IMMEDIATE)
            .await
            .expect("competing writer");
        sqlx::query("UPDATE sessions SET updated_at_ms = updated_at_ms + 1 WHERE id = ?")
            .bind(session.id.as_str())
            .execute(&mut *writer)
            .await
            .expect("hold competing write");

        let dispatch_store = store.clone();
        let effect_id = record.id.clone();
        let run_id = run.id.clone();
        let generation = run.generation;
        let dispatch = tokio::spawn(async move {
            dispatch_store
                .mark_side_effect_dispatched_if_current(
                    &effect_id,
                    &run_id,
                    generation,
                    "host-request",
                    super::super::now_ms(),
                )
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(!dispatch.is_finished());
        writer.commit().await.expect("release competing writer");

        let dispatched = dispatch.await.expect("join").expect("dispatch");
        assert_eq!(dispatched.status, SideEffectExecutionStatus::Dispatched);
    }
}
