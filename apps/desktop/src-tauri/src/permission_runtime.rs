use hachimi_approvals::ApprovalBroker;
use hachimi_protocol::{
    AgentPermissionPolicy, ApprovalId, EntryProfile, RunRecord, RunStatus, SessionId,
    SessionPermissionConfig, SkillId,
};
use hachimi_storage::AgentStore;
use hachimi_user_input::UserInputBroker;

use crate::{CommandError, DesktopState};

pub(super) fn entry_profile_key(profile: EntryProfile) -> &'static str {
    match profile {
        EntryProfile::Workbench => "workbench",
        EntryProfile::PetConversation => "pet_conversation",
    }
}

pub(super) async fn read_session_permission_config(
    store: &AgentStore,
    session_id: Option<&SessionId>,
    entry_profile: EntryProfile,
) -> Result<SessionPermissionConfig, CommandError> {
    if let Some(session_id) = session_id.filter(|_| entry_profile != EntryProfile::PetConversation)
    {
        let session = store
            .get_session(session_id)
            .await
            .map_err(|error| CommandError::operation("session_permission_lookup_failed", error))?
            .ok_or_else(|| CommandError::new("session_not_found", "Session does not exist"))?;
        if session.entry_profile != entry_profile {
            return Err(CommandError::new(
                "session_permission_profile_mismatch",
                "Session entry profile does not match the permission request",
            ));
        }
        let scope_key = format!("session:{}", session_id.as_str());
        if let Some(policy) = store
            .permission_policy(&scope_key)
            .await
            .map_err(|error| CommandError::operation("session_permission_lookup_failed", error))?
        {
            return Ok(SessionPermissionConfig {
                policy,
                skill_ids: store
                    .permission_skill_ids(&scope_key)
                    .await
                    .map_err(|error| {
                        CommandError::operation("session_permission_lookup_failed", error)
                    })?,
                extra_authorizations: session_extra_authorizations(store, session_id).await?,
            });
        }
    }
    let scope_key = format!("profile:{}", entry_profile_key(entry_profile));
    Ok(SessionPermissionConfig {
        policy: store
            .permission_policy(&scope_key)
            .await
            .map_err(|error| CommandError::operation("session_permission_lookup_failed", error))?
            .unwrap_or_default(),
        skill_ids: store
            .permission_skill_ids(&scope_key)
            .await
            .map_err(|error| CommandError::operation("session_permission_lookup_failed", error))?,
        extra_authorizations: Vec::new(),
    })
}

pub(super) async fn session_extra_authorizations(
    store: &AgentStore,
    session_id: &SessionId,
) -> Result<Vec<hachimi_protocol::SessionExtraAuthorizationSummary>, CommandError> {
    let mut summaries = store
        .list_session_tool_authorities(session_id)
        .await
        .map_err(|error| CommandError::operation("session_extra_authority_lookup_failed", error))?
        .into_iter()
        .map(
            |approval| hachimi_protocol::SessionExtraAuthorizationSummary {
                id: approval.id,
                action: approval.action,
                resource: approval.resource,
                target_host: approval.target_host,
                granted_at_ms: approval.resolved_at_ms.unwrap_or(approval.created_at_ms),
            },
        )
        .collect::<Vec<_>>();
    summaries.extend(
        store
            .list_session_host_authorities(session_id)
            .await
            .map_err(|error| {
                CommandError::operation("session_host_authority_lookup_failed", error)
            })?
            .into_iter()
            .map(
                |authorization| hachimi_protocol::SessionExtraAuthorizationSummary {
                    id: ApprovalId::new(authorization.id),
                    action: authorization.action,
                    resource: authorization.resource,
                    target_host: authorization.target_host,
                    granted_at_ms: authorization.granted_at_ms,
                },
            ),
    );
    summaries.sort_by(|left, right| {
        right
            .granted_at_ms
            .cmp(&left.granted_at_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(summaries)
}

pub(super) fn validate_permission_revision(
    expected: u64,
    current: u64,
) -> Result<(), CommandError> {
    if expected == current {
        return Ok(());
    }
    Err(CommandError::new(
        "permission_revision_conflict",
        format!(
            "Permission policy changed; expected revision {expected}, current revision {current}"
        ),
    ))
}

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

pub(super) async fn persist_config_and_cancel_revoked(
    state: &DesktopState,
    owner_key: &str,
    policy: &AgentPermissionPolicy,
    skill_ids: &[SkillId],
    timestamp_ms: i64,
) -> Result<(), CommandError> {
    let (previous_policy, previous_skill_ids) = tokio::try_join!(
        state.agent_store.permission_policy(owner_key),
        state.agent_store.permission_skill_ids(owner_key),
    )
    .map_err(|error| CommandError::operation("permission_config_lookup_failed", error))?;
    state
        .agent_store
        .store_permission_config(owner_key, policy, skill_ids, timestamp_ms)
        .await
        .map_err(|error| CommandError::operation("permission_config_store_failed", error))?;

    let policy_revoked = previous_policy
        .as_ref()
        .is_some_and(|previous| !hachimi_policy::permission_policy_covers(policy, previous));
    let skill_revoked = previous_skill_ids
        .iter()
        .any(|previous| !skill_ids.contains(previous));
    if policy_revoked || skill_revoked {
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
