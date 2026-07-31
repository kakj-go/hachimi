use hachimi_protocol::{
    RunId, RunRecoveryDecisionAction, RunRecoveryDecisionRequest, RunRecoveryId, RunRecoveryRecord,
    RunRecoverySnapshot, RunRecoveryState, RunStatus, RunStepCheckpoint, RunStepCheckpointId,
    RunStepPhase, SessionId, SideEffectExecutionId, ToolCallId, ToolRecoveryPolicy,
};
use serde_json::{Value, json};
use sqlx::{Row, Sqlite, Transaction};

use super::{AgentStore, AgentStoreError, append_event_tx};

#[derive(Debug, Clone, PartialEq)]
pub enum RecoveryToolFence {
    ReuseCompleted {
        succeeded: bool,
        persisted_result: Option<Value>,
    },
    RetryWithIdempotencyKey(String),
}

impl AgentStore {
    pub async fn record_run_step_checkpoint(
        &self,
        checkpoint: &RunStepCheckpoint,
    ) -> Result<RunStepCheckpoint, AgentStoreError> {
        let mut checkpoint = checkpoint.clone();
        let run = self
            .get_run(&checkpoint.run_id)
            .await?
            .ok_or_else(|| AgentStoreError::RunNotFound(checkpoint.run_id.clone()))?;
        if run.session_id != checkpoint.session_id || run.generation != checkpoint.run_generation {
            return Err(AgentStoreError::RunPreconditionFailed);
        }
        if checkpoint.side_effect_execution_id.is_none()
            && let Some(tool_call_id) = checkpoint.tool_call_id.as_ref()
        {
            checkpoint.side_effect_execution_id = sqlx::query_scalar::<_, String>(
                "SELECT id FROM side_effect_executions WHERE run_id = ? AND run_generation = ? AND tool_call_id = ? ORDER BY created_at_ms DESC, id DESC LIMIT 1",
            )
            .bind(checkpoint.run_id.as_str())
            .bind(i64::try_from(checkpoint.run_generation).unwrap_or(i64::MAX))
            .bind(tool_call_id.as_str())
            .fetch_optional(&self.pool)
            .await?
            .map(SideEffectExecutionId::new);
        }
        sqlx::query(
            "INSERT INTO run_step_checkpoints (id, session_id, run_id, run_generation, step_index, phase, tool_call_id, tool_name, side_effect_execution_id, recovery_policy, parameter_hash, world_revision, provider_revision, revision_snapshot_json, created_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(checkpoint.id.as_str())
        .bind(checkpoint.session_id.as_str())
        .bind(checkpoint.run_id.as_str())
        .bind(i64::try_from(checkpoint.run_generation).unwrap_or(i64::MAX))
        .bind(i64::try_from(checkpoint.step_index).unwrap_or(i64::MAX))
        .bind(checkpoint.phase.as_str())
        .bind(checkpoint.tool_call_id.as_ref().map(ToolCallId::as_str))
        .bind(&checkpoint.tool_name)
        .bind(
            checkpoint
                .side_effect_execution_id
                .as_ref()
                .map(SideEffectExecutionId::as_str),
        )
        .bind(checkpoint.recovery_policy.as_str())
        .bind(&checkpoint.parameter_hash)
        .bind(&checkpoint.world_revision)
        .bind(&checkpoint.provider_revision)
        .bind(serde_json::to_string(&checkpoint.revision_snapshot)?)
        .bind(checkpoint.created_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(checkpoint)
    }

    pub async fn latest_run_step_checkpoint(
        &self,
        run_id: &RunId,
    ) -> Result<Option<RunStepCheckpoint>, AgentStoreError> {
        let row = sqlx::query(
            "SELECT * FROM run_step_checkpoints WHERE run_id = ? ORDER BY run_generation DESC, step_index DESC, created_at_ms DESC, rowid DESC LIMIT 1",
        )
        .bind(run_id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(checkpoint_from_row).transpose()
    }

    pub async fn latest_run_step_checkpoint_for_tool(
        &self,
        run_id: &RunId,
        run_generation: u64,
        tool_call_id: &ToolCallId,
    ) -> Result<Option<RunStepCheckpoint>, AgentStoreError> {
        let row = sqlx::query(
            "SELECT * FROM run_step_checkpoints WHERE run_id = ? AND run_generation = ? AND tool_call_id = ? ORDER BY step_index DESC, created_at_ms DESC, rowid DESC LIMIT 1",
        )
        .bind(run_id.as_str())
        .bind(i64::try_from(run_generation).unwrap_or(i64::MAX))
        .bind(tool_call_id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(checkpoint_from_row).transpose()
    }

    pub async fn get_run_recovery_snapshot(
        &self,
        recovery_id: &RunRecoveryId,
    ) -> Result<Option<RunRecoverySnapshot>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM run_recoveries WHERE id = ?")
            .bind(recovery_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let recovery = recovery_from_row(&row)?;
        let checkpoint = if let Some(checkpoint_id) = recovery.checkpoint_id.as_ref() {
            let row = sqlx::query("SELECT * FROM run_step_checkpoints WHERE id = ?")
                .bind(checkpoint_id.as_str())
                .fetch_optional(&self.pool)
                .await?;
            row.as_ref().map(checkpoint_from_row).transpose()?
        } else {
            None
        };
        Ok(Some(RunRecoverySnapshot {
            recovery,
            checkpoint,
        }))
    }

    pub async fn list_pending_run_recoveries(
        &self,
    ) -> Result<Vec<RunRecoverySnapshot>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT id FROM run_recoveries WHERE state IN ('eligible_auto', 'awaiting_user', 'resuming') ORDER BY created_at_ms ASC, id ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut snapshots = Vec::with_capacity(rows.len());
        for row in rows {
            let id = RunRecoveryId::new(row.get::<String, _>("id"));
            if let Some(snapshot) = self.get_run_recovery_snapshot(&id).await? {
                snapshots.push(snapshot);
            }
        }
        Ok(snapshots)
    }

    pub async fn resolve_run_recovery(
        &self,
        request: &RunRecoveryDecisionRequest,
        principal: &str,
        resolved_at_ms: i64,
    ) -> Result<RunRecoverySnapshot, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query("SELECT * FROM run_recoveries WHERE id = ?")
            .bind(request.recovery_id.as_str())
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| AgentStoreError::RunRecoveryNotFound(request.recovery_id.clone()))?;
        let recovery = recovery_from_row(&row)?;
        if recovery.run_id == request.expected_run_id
            && recovery.interrupted_generation == request.expected_interrupted_generation
            && recovery.decision_action == Some(request.action)
            && recovery.decision_idempotency_key.as_deref()
                == Some(request.context.idempotency_key.as_str())
        {
            transaction.commit().await?;
            return self
                .get_run_recovery_snapshot(&recovery.id)
                .await?
                .ok_or_else(|| AgentStoreError::RunRecoveryNotFound(recovery.id));
        }
        if recovery.run_id != request.expected_run_id
            || recovery.interrupted_generation != request.expected_interrupted_generation
            || !matches!(
                recovery.state,
                RunRecoveryState::EligibleAuto | RunRecoveryState::AwaitingUser
            )
        {
            return Err(AgentStoreError::InvalidRunRecoveryDecision);
        }
        if request.action == RunRecoveryDecisionAction::AbandonRun {
            let changed = sqlx::query(
                "UPDATE runs SET status = 'cancelled', failure_code = 'recovery_abandoned', updated_at_ms = ? WHERE id = ? AND generation = ? AND status IN ('recovering', 'waiting_recovery_decision')",
            )
            .bind(resolved_at_ms)
            .bind(recovery.run_id.as_str())
            .bind(i64::try_from(recovery.interrupted_generation).unwrap_or(i64::MAX))
            .execute(&mut *transaction)
            .await?
            .rows_affected();
            if changed != 1 {
                return Err(AgentStoreError::InvalidRunRecoveryDecision);
            }
            update_recovery_state_tx(
                &mut transaction,
                &recovery.id,
                RunRecoveryState::Abandoned,
                principal,
                request.action,
                &request.context.idempotency_key,
                resolved_at_ms,
            )
            .await?;
            append_event_tx(
                &mut transaction,
                &recovery.session_id,
                Some(&recovery.run_id),
                "run.recovery_abandoned",
                json!({ "recoveryId": recovery.id, "generation": recovery.interrupted_generation }),
                resolved_at_ms,
            )
            .await?;
            transaction.commit().await?;
            return self
                .get_run_recovery_snapshot(&recovery.id)
                .await?
                .ok_or_else(|| AgentStoreError::RunRecoveryNotFound(recovery.id));
        }

        let checkpoint = recovery_checkpoint_tx(&mut transaction, &recovery).await?;
        match request.action {
            RunRecoveryDecisionAction::ResumeSafeRemainder => {
                if !matches!(
                    checkpoint.as_ref().map(|value| value.recovery_policy),
                    Some(
                        ToolRecoveryPolicy::ReadOnlyReplayable
                            | ToolRecoveryPolicy::IdempotentWithReceipt
                    )
                ) {
                    return Err(AgentStoreError::InvalidRunRecoveryDecision);
                }
                if let Some(side_effect_id) = recovery.side_effect_execution_id.as_ref() {
                    let effect = sqlx::query(
                        "SELECT status, result_code FROM side_effect_executions WHERE id = ?",
                    )
                    .bind(side_effect_id.as_str())
                    .fetch_optional(&mut *transaction)
                    .await?
                    .ok_or(AgentStoreError::InvalidRunRecoveryDecision)?;
                    let status = effect.get::<String, _>("status");
                    let proven_safe = matches!(status.as_str(), "succeeded" | "failed")
                        || status == "cancelled"
                            && effect.get::<Option<String>, _>("result_code").as_deref()
                                == Some("cancelled_before_dispatch_on_restart");
                    if !proven_safe {
                        return Err(AgentStoreError::InvalidRunRecoveryDecision);
                    }
                }
            }
            RunRecoveryDecisionAction::ConfirmEffectSucceeded => {
                resolve_indeterminate_side_effect_tx(
                    &mut transaction,
                    recovery.side_effect_execution_id.as_ref(),
                    "succeeded",
                    "user_confirmed_succeeded_after_restart",
                    resolved_at_ms,
                )
                .await?;
            }
            RunRecoveryDecisionAction::RetryIdempotentEffect => {
                if checkpoint.as_ref().map(|value| value.recovery_policy)
                    != Some(ToolRecoveryPolicy::IdempotentWithReceipt)
                {
                    return Err(AgentStoreError::InvalidRunRecoveryDecision);
                }
                resolve_indeterminate_side_effect_tx(
                    &mut transaction,
                    recovery.side_effect_execution_id.as_ref(),
                    "failed",
                    "user_confirmed_failed_before_idempotent_retry",
                    resolved_at_ms,
                )
                .await?;
            }
            RunRecoveryDecisionAction::AbandonRun => unreachable!(),
        }

        let changed = sqlx::query(
            "UPDATE runs SET status = 'queued', generation = ?, failure_code = NULL, updated_at_ms = ? WHERE id = ? AND generation = ? AND status IN ('recovering', 'waiting_recovery_decision')",
        )
        .bind(i64::try_from(recovery.resume_generation).unwrap_or(i64::MAX))
        .bind(resolved_at_ms)
        .bind(recovery.run_id.as_str())
        .bind(i64::try_from(recovery.interrupted_generation).unwrap_or(i64::MAX))
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if changed != 1 {
            return Err(AgentStoreError::InvalidRunRecoveryDecision);
        }
        update_recovery_state_tx(
            &mut transaction,
            &recovery.id,
            RunRecoveryState::Resuming,
            principal,
            request.action,
            &request.context.idempotency_key,
            resolved_at_ms,
        )
        .await?;
        append_event_tx(
            &mut transaction,
            &recovery.session_id,
            Some(&recovery.run_id),
            "run.recovery_resuming",
            json!({
                "recoveryId": recovery.id,
                "fromGeneration": recovery.interrupted_generation,
                "toGeneration": recovery.resume_generation,
                "decision": request.action,
            }),
            resolved_at_ms,
        )
        .await?;
        transaction.commit().await?;
        self.get_run_recovery_snapshot(&recovery.id)
            .await?
            .ok_or_else(|| AgentStoreError::RunRecoveryNotFound(recovery.id))
    }

    pub async fn finish_run_recovery(
        &self,
        run_id: &RunId,
        run_generation: u64,
        succeeded: bool,
        updated_at_ms: i64,
    ) -> Result<(), AgentStoreError> {
        let next = if succeeded {
            RunRecoveryState::Resumed
        } else {
            RunRecoveryState::Failed
        };
        sqlx::query(
            "UPDATE run_recoveries SET state = ?, updated_at_ms = ? WHERE run_id = ? AND resume_generation = ? AND state = 'resuming'",
        )
        .bind(next.as_str())
        .bind(updated_at_ms)
        .bind(run_id.as_str())
        .bind(i64::try_from(run_generation).unwrap_or(i64::MAX))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Resolves the one exact tool call covered by an active recovery decision.
    ///
    /// The fence is keyed by trusted persisted tool name plus canonical parameter
    /// hash. A newly sampled call ID cannot bypass it, and an idempotent retry
    /// receives the original Host idempotency key instead of inventing a new one.
    pub async fn recovery_tool_fence(
        &self,
        run_id: &RunId,
        resume_generation: u64,
        tool_name: &str,
        parameter_hash: &str,
    ) -> Result<Option<RecoveryToolFence>, AgentStoreError> {
        let row = sqlx::query(
            "SELECT r.interrupted_generation, r.decision_action, r.side_effect_execution_id, c.tool_call_id FROM run_recoveries r JOIN run_step_checkpoints c ON c.id = r.checkpoint_id WHERE r.run_id = ? AND r.resume_generation = ? AND r.state = 'resuming' AND c.tool_name = ? AND c.parameter_hash = ? LIMIT 1",
        )
        .bind(run_id.as_str())
        .bind(i64::try_from(resume_generation).unwrap_or(i64::MAX))
        .bind(tool_name)
        .bind(parameter_hash)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let interrupted_generation = u64::try_from(row.get::<i64, _>("interrupted_generation"))
            .map_err(|_| invalid("run recovery generation", -1))?;
        let tool_call_id = row.get::<Option<String>, _>("tool_call_id");
        let side_effect_id = row.get::<Option<String>, _>("side_effect_execution_id");
        let effect = if let Some(side_effect_id) = side_effect_id {
            sqlx::query("SELECT * FROM side_effect_executions WHERE id = ?")
                .bind(side_effect_id)
                .fetch_optional(&self.pool)
                .await?
        } else if let Some(tool_call_id) = tool_call_id {
            sqlx::query(
                "SELECT * FROM side_effect_executions WHERE run_id = ? AND run_generation = ? AND tool_call_id = ? ORDER BY updated_at_ms DESC LIMIT 1",
            )
            .bind(run_id.as_str())
            .bind(i64::try_from(interrupted_generation).unwrap_or(i64::MAX))
            .bind(tool_call_id)
            .fetch_optional(&self.pool)
            .await?
        } else {
            None
        };
        let Some(effect) = effect else {
            return Ok(None);
        };
        let action = row
            .get::<Option<String>, _>("decision_action")
            .and_then(|value| RunRecoveryDecisionAction::parse(&value));
        if action == Some(RunRecoveryDecisionAction::RetryIdempotentEffect)
            || action == Some(RunRecoveryDecisionAction::ResumeSafeRemainder)
                && effect.get::<String, _>("status") == "cancelled"
                && effect.get::<Option<String>, _>("result_code").as_deref()
                    == Some("cancelled_before_dispatch_on_restart")
        {
            return Ok(Some(RecoveryToolFence::RetryWithIdempotencyKey(
                effect.get("idempotency_key"),
            )));
        }
        let status = effect.get::<String, _>("status");
        if matches!(status.as_str(), "succeeded" | "failed") {
            let persisted_result = effect
                .get::<Option<String>, _>("result_json")
                .map(|value| serde_json::from_str(&value))
                .transpose()?;
            return Ok(Some(RecoveryToolFence::ReuseCompleted {
                succeeded: status == "succeeded",
                persisted_result,
            }));
        }
        Ok(None)
    }
}

async fn recovery_checkpoint_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    recovery: &RunRecoveryRecord,
) -> Result<Option<RunStepCheckpoint>, AgentStoreError> {
    let Some(checkpoint_id) = recovery.checkpoint_id.as_ref() else {
        return Ok(None);
    };
    let row = sqlx::query("SELECT * FROM run_step_checkpoints WHERE id = ?")
        .bind(checkpoint_id.as_str())
        .fetch_optional(&mut **transaction)
        .await?;
    row.as_ref().map(checkpoint_from_row).transpose()
}

async fn update_recovery_state_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    recovery_id: &RunRecoveryId,
    state: RunRecoveryState,
    principal: &str,
    action: RunRecoveryDecisionAction,
    idempotency_key: &str,
    updated_at_ms: i64,
) -> Result<(), AgentStoreError> {
    sqlx::query(
        "UPDATE run_recoveries SET state = ?, decision_action = ?, decision_idempotency_key = ?, resolved_by = ?, resolved_at_ms = ?, updated_at_ms = ? WHERE id = ?",
    )
    .bind(state.as_str())
    .bind(action.as_str())
    .bind(idempotency_key)
    .bind(principal)
    .bind(updated_at_ms)
    .bind(updated_at_ms)
    .bind(recovery_id.as_str())
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn resolve_indeterminate_side_effect_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    side_effect_id: Option<&SideEffectExecutionId>,
    status: &str,
    result_code: &str,
    updated_at_ms: i64,
) -> Result<(), AgentStoreError> {
    let Some(side_effect_id) = side_effect_id else {
        return Err(AgentStoreError::InvalidRunRecoveryDecision);
    };
    let changed = sqlx::query(
        "UPDATE side_effect_executions SET status = ?, result_code = ?, updated_at_ms = ? WHERE id = ? AND status = 'indeterminate'",
    )
    .bind(status)
    .bind(result_code)
    .bind(updated_at_ms)
    .bind(side_effect_id.as_str())
    .execute(&mut **transaction)
    .await?
    .rows_affected();
    if changed != 1 {
        return Err(AgentStoreError::InvalidRunRecoveryDecision);
    }
    Ok(())
}

fn checkpoint_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<RunStepCheckpoint, AgentStoreError> {
    let phase_value = row.get::<String, _>("phase");
    let policy_value = row.get::<String, _>("recovery_policy");
    let world_revision = row.get::<String, _>("world_revision");
    let provider_revision = row.get::<String, _>("provider_revision");
    let revision_snapshot_json = row.get::<String, _>("revision_snapshot_json");
    let legacy_snapshot = revision_snapshot_json.trim() == "{}";
    let mut revision_snapshot = serde_json::from_str::<hachimi_protocol::RecoveryRevisionSnapshot>(
        &revision_snapshot_json,
    )?;
    if legacy_snapshot {
        revision_snapshot.host_revision.clone_from(&world_revision);
        revision_snapshot
            .provider_revision
            .clone_from(&provider_revision);
    }
    Ok(RunStepCheckpoint {
        id: RunStepCheckpointId::new(row.get::<String, _>("id")),
        session_id: SessionId::new(row.get::<String, _>("session_id")),
        run_id: RunId::new(row.get::<String, _>("run_id")),
        run_generation: u64::try_from(row.get::<i64, _>("run_generation")).map_err(|_| {
            invalid(
                "run checkpoint generation",
                row.get::<i64, _>("run_generation"),
            )
        })?,
        step_index: u64::try_from(row.get::<i64, _>("step_index"))
            .map_err(|_| invalid("run checkpoint step", row.get::<i64, _>("step_index")))?,
        phase: RunStepPhase::parse(&phase_value)
            .ok_or_else(|| invalid("run checkpoint phase", phase_value))?,
        tool_call_id: row
            .get::<Option<String>, _>("tool_call_id")
            .map(ToolCallId::new),
        tool_name: row.get("tool_name"),
        side_effect_execution_id: row
            .get::<Option<String>, _>("side_effect_execution_id")
            .map(SideEffectExecutionId::new),
        recovery_policy: ToolRecoveryPolicy::parse(&policy_value)
            .ok_or_else(|| invalid("tool recovery policy", policy_value))?,
        parameter_hash: row.get("parameter_hash"),
        world_revision,
        provider_revision,
        revision_snapshot,
        created_at_ms: row.get("created_at_ms"),
    })
}

fn recovery_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<RunRecoveryRecord, AgentStoreError> {
    let previous_value = row.get::<String, _>("previous_status");
    let state_value = row.get::<String, _>("state");
    Ok(RunRecoveryRecord {
        id: RunRecoveryId::new(row.get::<String, _>("id")),
        session_id: SessionId::new(row.get::<String, _>("session_id")),
        run_id: RunId::new(row.get::<String, _>("run_id")),
        previous_status: RunStatus::parse(&previous_value)
            .ok_or_else(|| invalid("run recovery previous status", previous_value))?,
        interrupted_generation: u64::try_from(row.get::<i64, _>("interrupted_generation"))
            .map_err(|_| {
                invalid(
                    "run recovery generation",
                    row.get::<i64, _>("interrupted_generation"),
                )
            })?,
        resume_generation: u64::try_from(row.get::<i64, _>("resume_generation")).map_err(|_| {
            invalid(
                "run recovery resume generation",
                row.get::<i64, _>("resume_generation"),
            )
        })?,
        state: RunRecoveryState::parse(&state_value)
            .ok_or_else(|| invalid("run recovery state", state_value))?,
        reason_code: row.get("reason_code"),
        checkpoint_id: row
            .get::<Option<String>, _>("checkpoint_id")
            .map(RunStepCheckpointId::new),
        side_effect_execution_id: row
            .get::<Option<String>, _>("side_effect_execution_id")
            .map(SideEffectExecutionId::new),
        decision_action: row
            .get::<Option<String>, _>("decision_action")
            .map(|value| {
                RunRecoveryDecisionAction::parse(&value)
                    .ok_or_else(|| invalid("run recovery decision action", value))
            })
            .transpose()?,
        decision_idempotency_key: row.get("decision_idempotency_key"),
        resolved_by: row.get("resolved_by"),
        resolved_at_ms: row.get("resolved_at_ms"),
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

fn invalid(kind: &'static str, value: impl ToString) -> AgentStoreError {
    AgentStoreError::InvalidPersistedValue {
        kind,
        value: value.to_string(),
    }
}
