use hachimi_protocol::{
    ApprovalId, ApprovalStatus, ArtifactId, RunId, RunStepCheckpointId, RunStepPhase,
    SideEffectExecutionId, SideEffectExecutionRecord, SideEffectExecutionStatus,
};
use serde_json::Value;
use sqlx::Row;

use super::{AgentStore, AgentStoreError, enum_from_db, enum_to_db, get_run_tx};

#[derive(Debug, Clone, PartialEq)]
pub struct SideEffectClaim {
    pub record: SideEffectExecutionRecord,
    pub created: bool,
    pub persisted_result: Option<Value>,
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

    pub async fn claim_side_effect(
        &self,
        record: &SideEffectExecutionRecord,
    ) -> Result<SideEffectClaim, AgentStoreError> {
        if record.status != SideEffectExecutionStatus::Claimed {
            return Err(AgentStoreError::InvalidSideEffectTransition {
                from: record.status,
                to: SideEffectExecutionStatus::Claimed,
            });
        }
        let mut transaction = self.pool.begin().await?;
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
            consume_approval(
                &mut transaction,
                approval_id,
                &record.run_id,
                record.run_generation,
                record.tool_call_id.as_str(),
                &record.parameter_hash,
            )
            .await?;
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
        let mut transaction = self.pool.begin().await?;
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
        let mut transaction = self.pool.begin().await?;
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
    run_id: &RunId,
    generation: u64,
    tool_call_id: &str,
    parameter_hash: &str,
) -> Result<(), AgentStoreError> {
    let affected = sqlx::query(
        "UPDATE approval_requests SET uses_remaining = uses_remaining - 1 WHERE id = ? AND run_id = ? AND run_generation = ? AND tool_call_id = ? AND parameter_hash = ? AND status = ? AND uses_remaining > 0",
    )
    .bind(approval_id.as_str())
    .bind(run_id.as_str())
    .bind(i64::try_from(generation).unwrap_or(i64::MAX))
    .bind(tool_call_id)
    .bind(parameter_hash)
    .bind(ApprovalStatus::Approved.as_str())
    .execute(&mut **transaction)
    .await?
    .rows_affected();
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
