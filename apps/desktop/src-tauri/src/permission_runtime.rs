use hachimi_approvals::ApprovalBroker;
use hachimi_protocol::{AgentPermissionPolicy, RunRecord, RunStatus};
use hachimi_user_input::UserInputBroker;

use crate::{CommandError, DesktopState};

pub(super) async fn persist_policy_and_cancel_revoked(
    state: &DesktopState,
    owner_key: &str,
    policy: &AgentPermissionPolicy,
    timestamp_ms: i64,
) -> Result<(), CommandError> {
    let previous = state
        .agent_store
        .permission_policy(owner_key)
        .await
        .map_err(|error| CommandError::operation("permission_policy_lookup_failed", error))?;
    state
        .agent_store
        .store_permission_policy(owner_key, policy, timestamp_ms)
        .await
        .map_err(|error| CommandError::operation("permission_policy_store_failed", error))?;

    if previous
        .as_ref()
        .is_some_and(|previous| !hachimi_policy::permission_policy_covers(policy, previous))
    {
        cancel_runs_for_permission_owner(
            state,
            owner_key,
            "permission_authority_revoked",
            timestamp_ms,
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn cancel_runs_for_permission_owner(
    state: &DesktopState,
    owner_key: &str,
    reason: &str,
    timestamp_ms: i64,
) -> Result<Vec<RunRecord>, CommandError> {
    let runs = state
        .agent_store
        .active_runs_for_permission_owner(owner_key)
        .await
        .map_err(|error| CommandError::operation("permission_active_runs_lookup_failed", error))?;
    let mut cancelled = Vec::with_capacity(runs.len());
    for run in runs {
        if let Some(active) = state.agent_executor.registry().get(&run.id) {
            let _ = state
                .agent_executor
                .registry()
                .cancel(&run.id, active.run_generation);
        }
        state
            .approval_broker
            .cancel_run(run.id.clone())
            .await
            .map_err(|error| CommandError::operation("permission_approval_cancel_failed", error))?;
        state
            .user_input_broker
            .cancel_run(run.id.clone())
            .await
            .map_err(|error| {
                CommandError::operation("permission_user_input_cancel_failed", error)
            })?;
        state
            .agent_store
            .invalidate_run_capability_grants(&run.session_id, &run.id, timestamp_ms)
            .await
            .map_err(|error| {
                CommandError::operation("permission_grant_invalidation_failed", error)
            })?;

        let Some(current) = state
            .agent_store
            .get_run(&run.id)
            .await
            .map_err(|error| CommandError::operation("permission_run_lookup_failed", error))?
        else {
            continue;
        };
        let target = match current.status {
            RunStatus::Queued
            | RunStatus::Preparing
            | RunStatus::Recovering
            | RunStatus::WaitingRecoveryDecision => Some(RunStatus::Cancelled),
            RunStatus::Running | RunStatus::WaitingApproval | RunStatus::WaitingUserInput => {
                Some(RunStatus::Cancelling)
            }
            RunStatus::Cancelling => None,
            status if status.is_terminal() => None,
            _ => None,
        };
        if let Some(target) = target {
            let updated = state
                .agent_store
                .transition_run(&run.id, target, Some(reason))
                .await
                .map_err(|error| CommandError::operation("permission_run_cancel_failed", error))?;
            cancelled.push(updated);
        } else {
            cancelled.push(current);
        }
    }
    Ok(cancelled)
}
