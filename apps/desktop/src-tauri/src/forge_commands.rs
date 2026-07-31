use std::time::Duration;

use hachimi_approvals::ApprovalBroker;
use hachimi_forge::{ForgeClient, ForgeCredentialStore, ForgeError, SystemForgeCredentialStore};
use hachimi_protocol::{
    ApprovalGrantScope, ApprovalId, ApprovalRequestRecord, ApprovalStatus, ForgeChangeMutation,
    ForgeChangeMutationRequest, ForgeChangeQueryRequest, ForgeChangeRecord, ForgeCredentialState,
    ForgeCredentialUpdateRequest, ForgeOperationId, ForgeOperationRecord, ForgeOperationStatus,
    GitPushRequest, GitPushResponse, GitRemoteListRequest, GitRemoteRecord, SideEffectExecutionId,
    SideEffectExecutionRecord, SideEffectExecutionStatus, ToolCallId,
};
use hachimi_workspace::{WorkspaceHostClient, WorkspaceOperation, WorkspaceOutput};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use tauri::{State, WebviewWindow};
use tokio_util::sync::CancellationToken;

use super::{CommandError, DesktopState, epoch_millis, require_window, workspace_worker_path};
use crate::workspace_commands::{ResolvedWorkspace, resolve_session_workspace};

const FORGE_TIMEOUT: Duration = Duration::from_secs(90);

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
    match workspace_host(&workspace)
        .execute(
            WorkspaceOperation::GitRemotes,
            Duration::from_secs(20),
            CancellationToken::new(),
        )
        .await
        .map_err(workspace_error)?
    {
        WorkspaceOutput::GitRemotes { remotes } => Ok(remotes),
        _ => Err(CommandError::new(
            "git_remote_protocol_mismatch",
            "Workspace Host did not return Git remotes",
        )),
    }
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
    let output = workspace_host(&workspace)
        .execute(
            WorkspaceOperation::GitPush {
                remote_name: request.remote_name,
                expected_remote_url_hash: request.expected_remote_url_hash,
                source_ref: request.source_ref,
                target_ref: request.target_ref,
                expected_commit_oid: request.expected_commit_oid,
            },
            FORGE_TIMEOUT,
            CancellationToken::new(),
        )
        .await;
    match output {
        Ok(WorkspaceOutput::GitPush { response }) => {
            finish_side_effect(&state, &side_effect.id, &response).await?;
            Ok(response)
        }
        Ok(_) => {
            mark_indeterminate(&state, &side_effect.id, "git_push_protocol_mismatch").await?;
            Err(CommandError::new(
                "git_push_indeterminate",
                "Git push dispatch returned an unexpected receipt",
            ))
        }
        Err(error) => {
            mark_indeterminate(&state, &side_effect.id, "git_push_unknown_outcome").await?;
            Err(CommandError::new(
                "git_push_indeterminate",
                format!(
                    "Git push may have reached the remote; refresh the remote ref before retrying ({})",
                    error.message
                ),
            ))
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
    ForgeClient::system()
        .map_err(forge_error)?
        .query(&request.repository, request.number)
        .await
        .map_err(forge_error)
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
    let parameter_hash = request_hash(&(
        &request.session_id,
        &request.checkout_id,
        &request.repository,
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
    let (operation_kind, source_ref, target_ref, resource, risk) =
        mutation_metadata(&request.mutation, &request.repository);
    let approval_id = resolve_approval(
        &state,
        request.approval_id.clone(),
        &workspace,
        &client.client_id.0,
        call_id.clone(),
        operation_kind,
        &resource,
        &parameter_hash,
        risk,
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
    let now = now_ms();
    let operation = ForgeOperationRecord {
        id: ForgeOperationId::random(),
        session_id: request.session_id.clone(),
        run_id: Some(run_id.clone()),
        run_generation: Some(generation),
        operation_kind: operation_kind.into(),
        repository: request.repository.clone(),
        source_ref,
        target_ref,
        commit_oid: request.expected_commit_oid.clone(),
        expected_revision: request.expected_revision.clone(),
        approval_id: Some(approval_id),
        idempotency_key: request.context.idempotency_key.clone(),
        request_hash: parameter_hash.clone(),
        status: ForgeOperationStatus::Claimed,
        result: None,
        error_code: None,
        created_at_ms: now,
        updated_at_ms: now,
    };
    let operation = match state.agent_store.claim_forge_operation(&operation).await {
        Ok(operation) => operation,
        Err(error) => {
            let _ = state
                .agent_store
                .cancel_claimed_side_effect(&side_effect.id, now_ms())
                .await;
            return Err(CommandError::operation(
                "forge_operation_claim_failed",
                error,
            ));
        }
    };
    if operation.status == ForgeOperationStatus::Confirmed {
        return operation.result.ok_or_else(|| {
            CommandError::new("forge_receipt_missing", "confirmed Forge result is missing")
        });
    }
    if operation.status != ForgeOperationStatus::Claimed {
        return Err(CommandError::new(
            "forge_operation_indeterminate",
            "the previous Forge mutation is not safe to repeat; query the PR/MR first",
        ));
    }
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
    state
        .agent_store
        .update_forge_operation(
            &operation.id,
            ForgeOperationStatus::Claimed,
            ForgeOperationStatus::Dispatched,
            None,
            None,
            now_ms(),
        )
        .await
        .map_err(|error| CommandError::operation("forge_dispatch_ledger_failed", error))?;
    let result = ForgeClient::system()
        .map_err(forge_error)?
        .mutate(
            &request.repository,
            &request.mutation,
            request.expected_revision.as_deref(),
            &request.expected_commit_oid,
        )
        .await;
    match result {
        Ok(result) => {
            state
                .agent_store
                .update_forge_operation(
                    &operation.id,
                    ForgeOperationStatus::Dispatched,
                    ForgeOperationStatus::Confirmed,
                    Some(&result),
                    None,
                    now_ms(),
                )
                .await
                .map_err(|error| CommandError::operation("forge_receipt_store_failed", error))?;
            finish_side_effect(&state, &side_effect.id, &result).await?;
            Ok(result)
        }
        Err(error) if forge_unknown(&error) => {
            state
                .agent_store
                .update_forge_operation(
                    &operation.id,
                    ForgeOperationStatus::Dispatched,
                    ForgeOperationStatus::Indeterminate,
                    None,
                    Some("forge_unknown_outcome"),
                    now_ms(),
                )
                .await
                .map_err(|store| CommandError::operation("forge_unknown_store_failed", store))?;
            mark_indeterminate(&state, &side_effect.id, "forge_unknown_outcome").await?;
            Err(CommandError::new(
                "forge_operation_indeterminate",
                format!("Forge mutation outcome is unknown; query before retrying ({error})"),
            ))
        }
        Err(error) => {
            state
                .agent_store
                .update_forge_operation(
                    &operation.id,
                    ForgeOperationStatus::Dispatched,
                    ForgeOperationStatus::Failed,
                    None,
                    Some("forge_rejected"),
                    now_ms(),
                )
                .await
                .map_err(|store| CommandError::operation("forge_failure_store_failed", store))?;
            state
                .agent_store
                .finish_side_effect(
                    &side_effect.id,
                    SideEffectExecutionStatus::Failed,
                    Some("forge_rejected"),
                    None,
                    None,
                    now_ms(),
                )
                .await
                .map_err(|store| CommandError::operation("forge_failure_finish_failed", store))?;
            Err(forge_error(error))
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
        return Ok(id);
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
    let resolved = state
        .approval_broker
        .request(approval, CancellationToken::new())
        .await
        .map_err(|error| CommandError::operation("forge_approval_failed", error))?;
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

fn mutation_metadata(
    mutation: &ForgeChangeMutation,
    repository: &hachimi_protocol::ForgeRepositoryIdentity,
) -> (
    &'static str,
    Option<String>,
    Option<String>,
    String,
    &'static str,
) {
    let repo = format!(
        "{}:{}/{}",
        repository.forge_kind.as_str(),
        repository.owner,
        repository.repository
    );
    match mutation {
        ForgeChangeMutation::Create {
            source_ref,
            target_ref,
            ..
        } => (
            "forge.change.create",
            Some(source_ref.clone()),
            Some(target_ref.clone()),
            repo,
            "Create a PR/MR on an external Forge",
        ),
        ForgeChangeMutation::Update {
            number,
            source_ref,
            target_ref,
            ..
        } => (
            "forge.change.update",
            Some(source_ref.clone()),
            Some(target_ref.clone()),
            format!("{repo}#{number}"),
            "Update an external PR/MR",
        ),
        ForgeChangeMutation::Close { number } => (
            "forge.change.close",
            None,
            None,
            format!("{repo}#{number}"),
            "Close an external PR/MR",
        ),
        ForgeChangeMutation::Merge { number, .. } => (
            "forge.change.merge",
            None,
            None,
            format!("{repo}#{number}"),
            "High risk: merge an external PR/MR into its target branch",
        ),
    }
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

fn forge_unknown(error: &ForgeError) -> bool {
    matches!(error, ForgeError::Indeterminate(_))
        || matches!(error, ForgeError::Http { status, .. } if status.is_server_error())
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

fn workspace_error(error: hachimi_workspace::WorkspaceError) -> CommandError {
    CommandError::new(
        format!("workspace_{:?}", error.code).to_lowercase(),
        error.message,
    )
}

fn now_ms() -> i64 {
    i64::try_from(epoch_millis()).unwrap_or(i64::MAX)
}
