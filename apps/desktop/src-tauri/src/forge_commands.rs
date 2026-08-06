use hachimi_approvals::ApprovalBroker;
use hachimi_forge::{ForgeCredentialStore, ForgeError, SystemForgeCredentialStore};
use hachimi_protocol::{
    ApprovalGrantScope, ApprovalId, ApprovalRequestRecord, ApprovalStatus,
    ForgeChangeMutationRequest, ForgeChangeQueryRequest, ForgeChangeRecord, ForgeCredentialState,
    ForgeCredentialUpdateRequest, GitPushRequest, GitPushResponse, GitRemoteListRequest,
    GitRemoteRecord, RunStatus, SideEffectExecutionId, SideEffectExecutionRecord,
    SideEffectExecutionStatus, ToolCallId,
};
use hachimi_workspace::WorkspaceHostClient;
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use tauri::{State, WebviewWindow};
use tokio_util::sync::CancellationToken;

use super::{CommandError, DesktopState, epoch_millis, require_window, workspace_worker_path};
use crate::workspace_commands::{ResolvedWorkspace, resolve_session_workspace};

fn authorize(
    window: &WebviewWindow,
    state: &DesktopState,
) -> Result<hachimi_protocol::ClientContext, CommandError> {
    let client = state.authorize(window, hachimi_protocol::ControlMethod::WorkbenchWindow)?;
    require_window(window, "workbench")?;
    Ok(client)
}

fn require_git_remote_mutations(state: &DesktopState) -> Result<(), CommandError> {
    state
        .control_plane
        .feature_flags()
        .runtime_features
        .git_remote_mutations
        .then_some(())
        .ok_or_else(|| CommandError::new("feature_disabled", "git_remote_mutations"))
}

fn validate_context(
    context: &hachimi_protocol::MutationContext,
    client: &hachimi_protocol::ClientId,
) -> Result<(hachimi_protocol::RunId, u64), CommandError> {
    if context.protocol_version != hachimi_protocol::CONTROL_PROTOCOL_VERSION
        || &context.client_id != client
        || context.request_id.0.trim().is_empty()
        || context.idempotency_key.trim().is_empty()
        || context.idempotency_key.len() > 128
    {
        return Err(CommandError::new(
            "forge_context_invalid",
            "Forge mutations require the authenticated v25 mutation context",
        ));
    }
    Ok((
        context
            .expected_run_id
            .clone()
            .ok_or_else(|| CommandError::new("forge_run_required", "selected Run is required"))?,
        context.expected_generation.ok_or_else(|| {
            CommandError::new(
                "forge_generation_required",
                "selected Run generation is required",
            )
        })?,
    ))
}

fn require_current_run(
    workspace: &ResolvedWorkspace,
    run_id: &hachimi_protocol::RunId,
    generation: u64,
) -> Result<(), CommandError> {
    if &workspace.run.id == run_id && workspace.run.generation == generation {
        Ok(())
    } else {
        Err(CommandError::new(
            "forge_run_precondition_failed",
            "selected Run or generation changed before dispatch",
        ))
    }
}

fn workspace_host(workspace: &ResolvedWorkspace) -> WorkspaceHostClient {
    WorkspaceHostClient::new(
        workspace_worker_path(),
        &workspace.checkout.path,
        workspace.checkout.id.as_str(),
        workspace.run.generation,
    )
}

#[tauri::command]
pub(super) async fn list_git_remotes(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: GitRemoteListRequest,
) -> Result<Vec<GitRemoteRecord>, CommandError> {
    authorize(&window, &state)?;
    let workspace =
        resolve_session_workspace(&state, &request.session_id, &request.checkout_id).await?;
    crate::git_forge_host::list_git_remotes(&workspace_host(&workspace), CancellationToken::new())
        .await
        .map_err(host_error)
}

#[tauri::command]
pub(super) async fn push_git_remote(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: GitPushRequest,
) -> Result<GitPushResponse, CommandError> {
    let client = authorize(&window, &state)?;
    require_git_remote_mutations(&state)?;
    let (run_id, generation) = validate_context(&request.context, &client.client_id)?;
    validate_push(&request)?;
    let workspace =
        resolve_session_workspace(&state, &request.session_id, &request.checkout_id).await?;
    require_current_run(&workspace, &run_id, generation)?;
    let parameter_hash = request_hash(&(
        &request.session_id,
        &request.checkout_id,
        &request.remote_name,
        &request.expected_remote_url_hash,
        &request.source_ref,
        &request.target_ref,
        &request.expected_commit_oid,
    ))?;
    if let Some(previous) = existing_side_effect::<GitPushResponse>(
        &state,
        &run_id,
        generation,
        &request.context.idempotency_key,
        &parameter_hash,
        &tool_call_id("git-push", &request.context.idempotency_key),
    )
    .await?
    {
        return previous;
    }
    let approval_id = resolve_approval(
        &state,
        request.approval_id,
        &workspace,
        &client.client_id.0,
        tool_call_id("git-push", &request.context.idempotency_key),
        "git.push",
        &format!("{}:{}", request.remote_name, request.target_ref),
        &parameter_hash,
        "Push changes to an external Git remote",
    )
    .await?;
    let side_effect = claim_side_effect(
        &state,
        &workspace,
        &request.context.idempotency_key,
        &parameter_hash,
        tool_call_id("git-push", &request.context.idempotency_key),
        Some(approval_id),
    )
    .await?;
    let host_request_id = format!("git-push:{}", request.context.request_id.0);
    state
        .agent_store
        .mark_side_effect_dispatched_if_current(
            &side_effect.id,
            &run_id,
            generation,
            &host_request_id,
            now_ms(),
        )
        .await
        .map_err(|error| CommandError::operation("git_push_dispatch_claim_failed", error))?;
    let output = crate::git_forge_host::push_git_remote(
        &workspace_host(&workspace),
        crate::git_forge_host::GitPushSpec {
            remote_name: request.remote_name,
            expected_remote_url_hash: request.expected_remote_url_hash,
            source_ref: request.source_ref,
            target_ref: request.target_ref,
            expected_commit_oid: request.expected_commit_oid,
        },
        CancellationToken::new(),
    )
    .await;
    match output {
        Ok(response) => {
            finish_side_effect(&state, &side_effect.id, &response).await?;
            Ok(response)
        }
        Err(error) if error.indeterminate => {
            mark_indeterminate(&state, &side_effect.id, error.code).await?;
            Err(host_error(error))
        }
        Err(error) => {
            finish_failed_side_effect(&state, &side_effect.id, error.code).await?;
            Err(host_error(error))
        }
    }
}

#[tauri::command]
pub(super) async fn query_forge_change(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ForgeChangeQueryRequest,
) -> Result<ForgeChangeRecord, CommandError> {
    authorize(&window, &state)?;
    crate::git_forge_host::query_forge_change(&request.repository, request.number)
        .await
        .map_err(host_error)
}

#[tauri::command]
pub(super) async fn mutate_forge_change(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ForgeChangeMutationRequest,
) -> Result<ForgeChangeRecord, CommandError> {
    let client = authorize(&window, &state)?;
    require_git_remote_mutations(&state)?;
    let (run_id, generation) = validate_context(&request.context, &client.client_id)?;
    validate_oid(&request.expected_commit_oid)?;
    let workspace =
        resolve_session_workspace(&state, &request.session_id, &request.checkout_id).await?;
    require_current_run(&workspace, &run_id, generation)?;
    let repository = crate::git_forge_host::resolve_forge_repository_by_hash(
        &workspace_host(&workspace),
        &request.repository.remote_url_hash,
        CancellationToken::new(),
    )
    .await
    .map_err(host_error)?;
    let parameter_hash = request_hash(&(
        &request.session_id,
        &request.checkout_id,
        &repository,
        &request.mutation,
        &request.expected_revision,
        &request.expected_commit_oid,
    ))?;
    let call_id = tool_call_id("forge", &request.context.idempotency_key);
    if let Some(previous) = existing_side_effect::<ForgeChangeRecord>(
        &state,
        &run_id,
        generation,
        &request.context.idempotency_key,
        &parameter_hash,
        &call_id,
    )
    .await?
    {
        return previous;
    }
    let metadata = crate::git_forge_host::mutation_metadata(&request.mutation, &repository);
    let approval_id = resolve_approval(
        &state,
        request.approval_id.clone(),
        &workspace,
        &client.client_id.0,
        call_id.clone(),
        metadata.operation_kind,
        &metadata.resource,
        &parameter_hash,
        metadata.risk,
    )
    .await?;
    let side_effect = claim_side_effect(
        &state,
        &workspace,
        &request.context.idempotency_key,
        &parameter_hash,
        call_id,
        Some(approval_id.clone()),
    )
    .await?;
    state
        .agent_store
        .mark_side_effect_dispatched_if_current(
            &side_effect.id,
            &run_id,
            generation,
            &format!("forge:{}", request.context.request_id.0),
            now_ms(),
        )
        .await
        .map_err(|error| CommandError::operation("forge_dispatch_claim_failed", error))?;
    let result = crate::git_forge_host::mutate_forge_change(
        &state.agent_store,
        &repository,
        &request.mutation,
        crate::git_forge_host::ForgeMutationLedgerContext {
            session_id: request.session_id.clone(),
            run_id,
            run_generation: generation,
            operation_kind: metadata.operation_kind.into(),
            source_ref: metadata.source_ref,
            target_ref: metadata.target_ref,
            expected_commit_oid: request.expected_commit_oid,
            expected_revision: request.expected_revision,
            approval_id: Some(approval_id),
            idempotency_key: request.context.idempotency_key,
            request_hash: parameter_hash,
        },
    )
    .await;
    match result {
        Ok(result) => {
            finish_side_effect(&state, &side_effect.id, &result).await?;
            Ok(result)
        }
        Err(error) if error.indeterminate => {
            mark_indeterminate(&state, &side_effect.id, error.code).await?;
            Err(host_error(error))
        }
        Err(error) => {
            finish_failed_side_effect(&state, &side_effect.id, error.code).await?;
            Err(host_error(error))
        }
    }
}

#[tauri::command]
pub(super) async fn update_forge_credential(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ForgeCredentialUpdateRequest,
) -> Result<ForgeCredentialState, CommandError> {
    authorize(&window, &state)?;
    require_git_remote_mutations(&state)?;
    let store = SystemForgeCredentialStore;
    match request.secret.as_deref() {
        Some(secret) => store
            .set(&request.secret_ref, secret)
            .map_err(forge_error)?,
        None => store.clear(&request.secret_ref).map_err(forge_error)?,
    }
    Ok(ForgeCredentialState {
        secret_ref: request.secret_ref.clone(),
        configured: store
            .get(&request.secret_ref)
            .map_err(forge_error)?
            .is_some(),
    })
}

#[allow(clippy::too_many_arguments)]
async fn resolve_approval(
    state: &DesktopState,
    supplied: Option<ApprovalId>,
    workspace: &ResolvedWorkspace,
    principal: &str,
    tool_call_id: ToolCallId,
    action: &str,
    resource: &str,
    parameter_hash: &str,
    risk: &str,
) -> Result<ApprovalId, CommandError> {
    if let Some(id) = supplied {
        let approval = state
            .agent_store
            .get_approval(&id)
            .await
            .map_err(|error| CommandError::operation("forge_approval_get_failed", error))?
            .ok_or_else(|| {
                CommandError::new(
                    "forge_approval_denied",
                    "supplied Forge approval does not exist",
                )
            })?;
        if !forge_approval_matches(
            &approval,
            &workspace.session_id,
            &workspace.run.id,
            workspace.run.generation,
            principal,
            &tool_call_id,
            action,
            resource,
            parameter_hash,
            now_ms(),
        ) {
            return Err(CommandError::new(
                "forge_approval_denied",
                "supplied Forge approval does not authorize these exact parameters",
            ));
        }
        return Ok(approval.id);
    }
    let now = now_ms();
    let approval = ApprovalRequestRecord {
        id: ApprovalId::random(),
        session_id: workspace.session_id.clone(),
        run_id: workspace.run.id.clone(),
        tool_call_id,
        run_generation: workspace.run.generation,
        status: ApprovalStatus::Pending,
        action: action.into(),
        resource: resource.chars().take(1_024).collect(),
        parameter_hash: parameter_hash.into(),
        risk_summary: risk.into(),
        target_host: "forge-broker".into(),
        required_scopes: vec!["network".into(), "forge.mutate".into()],
        grant_scope: ApprovalGrantScope::Once,
        uses_remaining: 1,
        requester_principal: principal.into(),
        resolved_by: None,
        expires_at_ms: Some(now.saturating_add(10 * 60 * 1_000)),
        created_at_ms: now,
        resolved_at_ms: None,
    };
    enter_approval_wait(state, workspace).await?;
    let resolution = state
        .approval_broker
        .request(approval, CancellationToken::new())
        .await;
    let leave_result = leave_approval_wait(state, workspace).await;
    let resolved = match (resolution, leave_result) {
        (Ok(resolved), Ok(())) => resolved,
        (Err(error), Ok(())) => {
            return Err(CommandError::operation("forge_approval_failed", error));
        }
        (Ok(_), Err(error)) | (Err(_), Err(error)) => return Err(error),
    };
    if resolved.status != ApprovalStatus::Approved
        || resolved.parameter_hash != parameter_hash
        || resolved.run_generation != workspace.run.generation
    {
        return Err(CommandError::new(
            "forge_approval_denied",
            "Forge mutation was not approved for these exact parameters",
        ));
    }
    Ok(resolved.id)
}

async fn enter_approval_wait(
    state: &DesktopState,
    workspace: &ResolvedWorkspace,
) -> Result<(), CommandError> {
    let run = state
        .agent_store
        .get_run(&workspace.run.id)
        .await
        .map_err(|error| CommandError::operation("forge_approval_run_get_failed", error))?
        .ok_or_else(|| {
            CommandError::new(
                "forge_approval_run_missing",
                "selected Run disappeared before Forge approval",
            )
        })?;
    if run.generation != workspace.run.generation {
        return Err(CommandError::new(
            "forge_approval_run_drift",
            "selected Run generation changed before Forge approval",
        ));
    }
    match run.status {
        RunStatus::Running => {
            state
                .agent_store
                .transition_run(&run.id, RunStatus::WaitingApproval, None)
                .await
                .map_err(|error| {
                    CommandError::operation("forge_approval_wait_enter_failed", error)
                })?;
            Ok(())
        }
        RunStatus::WaitingApproval => Ok(()),
        status => Err(CommandError::new(
            "forge_approval_run_state_invalid",
            format!("selected Run cannot wait for Forge approval while it is {status:?}"),
        )),
    }
}

async fn leave_approval_wait(
    state: &DesktopState,
    workspace: &ResolvedWorkspace,
) -> Result<(), CommandError> {
    let run = state
        .agent_store
        .get_run(&workspace.run.id)
        .await
        .map_err(|error| CommandError::operation("forge_approval_run_get_failed", error))?
        .ok_or_else(|| {
            CommandError::new(
                "forge_approval_run_missing",
                "selected Run disappeared after Forge approval",
            )
        })?;
    if run.generation != workspace.run.generation {
        return Err(CommandError::new(
            "forge_approval_run_drift",
            "selected Run generation changed while Forge approval was pending",
        ));
    }
    if run.status == RunStatus::WaitingApproval {
        state
            .agent_store
            .transition_run(&run.id, RunStatus::Running, None)
            .await
            .map_err(|error| CommandError::operation("forge_approval_wait_leave_failed", error))?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn forge_approval_matches(
    approval: &ApprovalRequestRecord,
    session_id: &hachimi_protocol::SessionId,
    run_id: &hachimi_protocol::RunId,
    run_generation: u64,
    principal: &str,
    tool_call_id: &ToolCallId,
    action: &str,
    resource: &str,
    parameter_hash: &str,
    now: i64,
) -> bool {
    approval.status == ApprovalStatus::Approved
        && &approval.session_id == session_id
        && &approval.run_id == run_id
        && approval.run_generation == run_generation
        && &approval.tool_call_id == tool_call_id
        && approval.action == action
        && approval.resource == resource.chars().take(1_024).collect::<String>()
        && approval.parameter_hash == parameter_hash
        && approval.target_host == "forge-broker"
        && approval.grant_scope == ApprovalGrantScope::Once
        && approval.uses_remaining == 1
        && approval
            .resolved_by
            .as_deref()
            .is_some_and(|resolved| resolved == principal)
        && approval.expires_at_ms.is_none_or(|expires| expires > now)
        && approval
            .required_scopes
            .iter()
            .any(|scope| scope == "network")
        && approval
            .required_scopes
            .iter()
            .any(|scope| scope == "forge.mutate")
}

async fn claim_side_effect(
    state: &DesktopState,
    workspace: &ResolvedWorkspace,
    idempotency_key: &str,
    parameter_hash: &str,
    tool_call_id: ToolCallId,
    approval_id: Option<ApprovalId>,
) -> Result<SideEffectExecutionRecord, CommandError> {
    let now = now_ms();
    let claim = state
        .agent_store
        .claim_side_effect(&SideEffectExecutionRecord {
            id: SideEffectExecutionId::random(),
            session_id: workspace.session_id.clone(),
            run_id: workspace.run.id.clone(),
            run_generation: workspace.run.generation,
            tool_call_id,
            idempotency_key: idempotency_key.into(),
            parameter_hash: parameter_hash.into(),
            approval_id,
            host_request_id: None,
            status: SideEffectExecutionStatus::Claimed,
            result_code: None,
            result_reference: None,
            created_at_ms: now,
            updated_at_ms: now,
        })
        .await
        .map_err(|error| CommandError::operation("forge_side_effect_claim_failed", error))?;
    Ok(claim.record)
}

async fn existing_side_effect<T: DeserializeOwned>(
    state: &DesktopState,
    run_id: &hachimi_protocol::RunId,
    generation: u64,
    idempotency_key: &str,
    parameter_hash: &str,
    tool_call_id: &ToolCallId,
) -> Result<Option<Result<T, CommandError>>, CommandError> {
    let existing = state
        .agent_store
        .list_side_effects_for_run(run_id)
        .await
        .map_err(|error| CommandError::operation("forge_side_effect_list_failed", error))?
        .into_iter()
        .find(|record| {
            record.run_generation == generation && record.idempotency_key == idempotency_key
        });
    let Some(existing) = existing else {
        return Ok(None);
    };
    if existing.parameter_hash != parameter_hash || &existing.tool_call_id != tool_call_id {
        return Err(CommandError::new(
            "forge_idempotency_conflict",
            "idempotency key was reused for different Forge parameters",
        ));
    }
    let result = match existing.status {
        SideEffectExecutionStatus::Succeeded => {
            let mut replay = existing.clone();
            replay.status = SideEffectExecutionStatus::Claimed;
            let claim = state
                .agent_store
                .claim_side_effect(&replay)
                .await
                .map_err(|error| CommandError::operation("forge_receipt_read_failed", error))?;
            claim
                .persisted_result
                .ok_or_else(|| {
                    CommandError::new("forge_receipt_missing", "persisted receipt is missing")
                })
                .and_then(|value| {
                    serde_json::from_value(value)
                        .map_err(|error| CommandError::operation("forge_receipt_invalid", error))
                })
        }
        SideEffectExecutionStatus::Claimed => {
            return Ok(None);
        }
        _ => Err(CommandError::new(
            "forge_operation_indeterminate",
            "the prior external operation is not safe to repeat; query remote state",
        )),
    };
    Ok(Some(result))
}

async fn finish_side_effect<T: Serialize>(
    state: &DesktopState,
    id: &SideEffectExecutionId,
    response: &T,
) -> Result<(), CommandError> {
    let value = serde_json::to_value(response)
        .map_err(|error| CommandError::operation("forge_receipt_invalid", error))?;
    state
        .agent_store
        .finish_side_effect(
            id,
            SideEffectExecutionStatus::Succeeded,
            Some("confirmed"),
            None,
            Some(&value),
            now_ms(),
        )
        .await
        .map_err(|error| CommandError::operation("forge_receipt_store_failed", error))?;
    Ok(())
}

async fn mark_indeterminate(
    state: &DesktopState,
    id: &SideEffectExecutionId,
    code: &str,
) -> Result<(), CommandError> {
    state
        .agent_store
        .finish_side_effect(
            id,
            SideEffectExecutionStatus::Indeterminate,
            Some(code),
            None,
            None,
            now_ms(),
        )
        .await
        .map_err(|error| CommandError::operation("forge_unknown_store_failed", error))?;
    Ok(())
}

async fn finish_failed_side_effect(
    state: &DesktopState,
    id: &SideEffectExecutionId,
    code: &str,
) -> Result<(), CommandError> {
    state
        .agent_store
        .finish_side_effect(
            id,
            SideEffectExecutionStatus::Failed,
            Some(code),
            None,
            None,
            now_ms(),
        )
        .await
        .map_err(|error| CommandError::operation("forge_failure_finish_failed", error))?;
    Ok(())
}

fn request_hash(value: &impl Serialize) -> Result<String, CommandError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| CommandError::operation("forge_request_invalid", error))?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn tool_call_id(prefix: &str, idempotency_key: &str) -> ToolCallId {
    ToolCallId::new(format!(
        "{prefix}:{}",
        idempotency_key.chars().take(96).collect::<String>()
    ))
}

fn validate_push(request: &GitPushRequest) -> Result<(), CommandError> {
    if request.remote_name.trim().is_empty()
        || request.expected_remote_url_hash.len() != 64
        || request.expected_commit_oid.len() != 40
        || !request
            .expected_remote_url_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || !request
            .expected_commit_oid
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(CommandError::new(
            "git_push_invalid",
            "Git push identity, URL hash, or commit OID is invalid",
        ));
    }
    Ok(())
}

fn validate_oid(oid: &str) -> Result<(), CommandError> {
    if oid.len() == 40 && oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(CommandError::new(
            "forge_commit_oid_invalid",
            "Forge mutation requires the exact 40-character source commit OID",
        ))
    }
}

fn forge_error(error: ForgeError) -> CommandError {
    let code = match &error {
        ForgeError::CredentialMissing | ForgeError::CredentialStore => "forge_credential_failed",
        ForgeError::RevisionConflict => "forge_revision_conflict",
        ForgeError::CommitConflict => "forge_commit_conflict",
        ForgeError::SourceRefConflict => "forge_source_ref_conflict",
        ForgeError::Indeterminate(_) => "forge_operation_indeterminate",
        ForgeError::Http { .. } => "forge_http_failed",
        ForgeError::QueryFailed(_) => "forge_query_failed",
        ForgeError::InvalidConfiguration(_) | ForgeError::InvalidResponse(_) => {
            "forge_protocol_failed"
        }
    };
    CommandError::new(code, error.to_string())
}

fn host_error(error: crate::git_forge_host::GitForgeHostError) -> CommandError {
    CommandError::new(error.code, error.message)
}

fn now_ms() -> i64 {
    i64::try_from(epoch_millis()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use hachimi_protocol::{
        ApprovalGrantScope, ApprovalId, ApprovalRequestRecord, ApprovalStatus, RunId, SessionId,
        ToolCallId,
    };

    use super::forge_approval_matches;

    fn approval() -> ApprovalRequestRecord {
        ApprovalRequestRecord {
            id: ApprovalId::from("forge-approval"),
            session_id: SessionId::from("forge-session"),
            run_id: RunId::from("forge-run"),
            tool_call_id: ToolCallId::from("forge-call"),
            run_generation: 3,
            status: ApprovalStatus::Approved,
            action: "forge.change.merge".into(),
            resource: "github:team/repository".into(),
            parameter_hash: "a".repeat(64),
            risk_summary: "merge".into(),
            target_host: "forge-broker".into(),
            required_scopes: vec!["network".into(), "forge.mutate".into()],
            grant_scope: ApprovalGrantScope::Once,
            uses_remaining: 1,
            requester_principal: "client".into(),
            resolved_by: Some("client".into()),
            expires_at_ms: Some(2_000),
            created_at_ms: 1,
            resolved_at_ms: Some(2),
        }
    }

    fn matches(record: &ApprovalRequestRecord) -> bool {
        forge_approval_matches(
            record,
            &SessionId::from("forge-session"),
            &RunId::from("forge-run"),
            3,
            "client",
            &ToolCallId::from("forge-call"),
            "forge.change.merge",
            "github:team/repository",
            &"a".repeat(64),
            1_000,
        )
    }

    #[test]
    fn supplied_forge_approval_is_exact_and_cannot_be_reused_for_another_generation() {
        let valid = approval();
        assert!(matches(&valid));
        let mut stale = valid.clone();
        stale.run_generation = 2;
        assert!(!matches(&stale));
        let mut different_parameters = valid.clone();
        different_parameters.parameter_hash = "b".repeat(64);
        assert!(!matches(&different_parameters));
        let mut expired = valid;
        expired.expires_at_ms = Some(999);
        assert!(!matches(&expired));

        let mut already_consumed = approval();
        already_consumed.uses_remaining = 0;
        assert!(!matches(&already_consumed));
        let mut wider_scope = approval();
        wider_scope.grant_scope = ApprovalGrantScope::Session;
        assert!(!matches(&wider_scope));
        let mut different_action = approval();
        different_action.action = "forge.change.update".into();
        assert!(!matches(&different_action));
    }
}
