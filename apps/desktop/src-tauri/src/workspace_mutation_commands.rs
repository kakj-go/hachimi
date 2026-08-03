// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex/codex-rs/app-server-protocol/src/protocol/v2/fs.rs
// @ 4c43465133428898aa84f0bfc02c306ed65fb66a.
// Modified for Hachimi: explicit-user saves, ETag conflicts, persistent
// idempotency, Run generation fencing, and restricted Workspace dispatch.
//! Mutating Workspace commands are isolated from the read-only browser adapter.

use std::{path::Path, sync::Arc, time::Duration};

use hachimi_policy::{
    DefaultPolicy, PolicyContext, PolicyDecision, PolicyEngine, expand_permission_profile,
};
use hachimi_protocol::{
    ApprovalPolicy, BehaviorMode, CapabilityGrantSet, ClientId, ControlMethod, EntryProfile,
    FsWriteRequest, FsWriteResponse, GitMutation, GitMutationRequest, GitMutationResponse,
    MutationContext, PermissionProfile, Scope, ToolEffect, WorkloadKind,
};
use hachimi_storage::IdempotentMutationClaim;
use hachimi_workspace::{
    WorkspaceError, WorkspaceErrorCode, WorkspaceHostClient, WorkspaceOperation, WorkspaceOutput,
};
use sha2::{Digest, Sha256};
use tauri::{State, WebviewWindow};
use tokio_util::sync::CancellationToken;

use super::{
    CommandError, DesktopState, enter_sandbox_activity, epoch_millis, require_window,
    sandbox_sidecar_path, workspace_worker_path,
};
use crate::workspace_commands::{ResolvedWorkspace, resolve_session_workspace};

const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
const MUTATION_METHOD: &str = "workspace.write";
const GIT_MUTATION_METHOD: &str = "workspace.git.mutate";

#[derive(Clone)]
struct InteractiveWorkspaceLaunchGuard {
    store: hachimi_storage::AgentStore,
}

impl hachimi_workspace::WorkspaceLaunchGuard for InteractiveWorkspaceLaunchGuard {
    fn validate(
        &self,
        check: hachimi_workspace::WorkspaceLaunchCheck,
    ) -> hachimi_workspace::WorkspaceLaunchValidationFuture {
        let store = self.store.clone();
        Box::pin(async move {
            let run = store
                .get_run(&check.run_id)
                .await
                .map_err(|error| workspace_guard_error("Run lookup", error))?
                .ok_or_else(|| {
                    WorkspaceError::new(
                        WorkspaceErrorCode::StaleGeneration,
                        "interactive save Run no longer exists",
                    )
                })?;
            if run.session_id != check.session_id || run.generation != check.run_generation {
                return Err(WorkspaceError::new(
                    WorkspaceErrorCode::StaleGeneration,
                    "interactive save Run generation changed",
                ));
            }
            let latest = store
                .list_runs(&check.session_id)
                .await
                .map_err(|error| workspace_guard_error("Run projection", error))?
                .into_iter()
                .last()
                .ok_or_else(|| {
                    WorkspaceError::new(
                        WorkspaceErrorCode::StaleGeneration,
                        "interactive save Session no longer has a Run",
                    )
                })?;
            if latest.id != check.run_id || latest.generation != check.run_generation {
                return Err(WorkspaceError::new(
                    WorkspaceErrorCode::StaleGeneration,
                    "interactive save no longer targets the latest Run generation",
                ));
            }
            let session = store
                .get_session(&check.session_id)
                .await
                .map_err(|error| workspace_guard_error("Session lookup", error))?
                .ok_or_else(|| {
                    WorkspaceError::new(
                        WorkspaceErrorCode::Unauthorized,
                        "interactive save Session no longer exists",
                    )
                })?;
            if session.context.checkout_id() != Some(&check.checkout_id)
                || check.effect != ToolEffect::WorkspaceWrite
            {
                return Err(WorkspaceError::new(
                    WorkspaceErrorCode::Unauthorized,
                    "interactive save Checkout/effect binding is invalid",
                ));
            }
            Ok(())
        })
    }
}

fn workspace_guard_error(operation: &str, error: impl std::fmt::Display) -> WorkspaceError {
    WorkspaceError::new(
        WorkspaceErrorCode::Unauthorized,
        format!("interactive save {operation} failed: {error}"),
    )
}

fn authorize(
    window: &WebviewWindow,
    state: &DesktopState,
) -> Result<hachimi_protocol::ClientContext, CommandError> {
    let client = state.authorize(window, ControlMethod::WorkbenchWindow)?;
    require_window(window, "workbench")?;
    Ok(client)
}

fn validate_mutation_context(
    context: &MutationContext,
    authenticated_client: &ClientId,
) -> Result<(hachimi_protocol::RunId, u64), CommandError> {
    if context.protocol_version != hachimi_protocol::CONTROL_PROTOCOL_VERSION {
        return Err(CommandError::new(
            "protocol_version_mismatch",
            "the Workspace write protocol version is not supported",
        ));
    }
    if &context.client_id != authenticated_client {
        return Err(CommandError::new(
            "client_precondition_failed",
            "the Workspace write client does not match the authenticated Workbench",
        ));
    }
    if context.request_id.0.trim().is_empty()
        || context.idempotency_key.trim().is_empty()
        || context.idempotency_key.len() > 128
    {
        return Err(CommandError::new(
            "invalid_mutation_context",
            "request ID and a bounded idempotency key are required",
        ));
    }
    let run_id = context.expected_run_id.as_ref().ok_or_else(|| {
        CommandError::new(
            "workspace_mutation_run_required",
            "Workspace mutations require the selected Run",
        )
    })?;
    let generation = context.expected_generation.ok_or_else(|| {
        CommandError::new(
            "workspace_mutation_generation_required",
            "Workspace mutations require the selected Run generation",
        )
    })?;
    Ok((run_id.clone(), generation))
}

fn validate_write_request(
    request: &FsWriteRequest,
    authenticated_client: &ClientId,
) -> Result<(hachimi_protocol::RunId, u64), CommandError> {
    let binding = validate_mutation_context(&request.context, authenticated_client)?;
    if request.path.trim().is_empty()
        || request.path.len() > 4_096
        || request.path.contains('\0')
        || request.content.len() > MAX_TEXT_BYTES
    {
        return Err(CommandError::new(
            "workspace_write_invalid",
            "path and UTF-8 content must be bounded",
        ));
    }
    Ok(binding)
}

fn expected_sha256(if_match: &str) -> Result<String, CommandError> {
    let digest = if_match.strip_prefix("sha256:").ok_or_else(|| {
        CommandError::new(
            "workspace_write_etag_invalid",
            "ifMatch must be the SHA-256 ETag returned by fs.read_chunk",
        )
    })?;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CommandError::new(
            "workspace_write_etag_invalid",
            "ifMatch must contain exactly one SHA-256 digest",
        ));
    }
    Ok(digest.to_ascii_lowercase())
}

fn mutation_fingerprint(request: &FsWriteRequest) -> String {
    let mut hasher = Sha256::new();
    let generation = request
        .context
        .expected_generation
        .map_or_else(String::new, |value| value.to_string());
    for value in [
        request.session_id.as_str(),
        request.checkout_id.as_str(),
        request
            .context
            .expected_run_id
            .as_ref()
            .map_or("", hachimi_protocol::RunId::as_str),
        generation.as_str(),
        &request.path,
        &request.if_match,
        &request.content,
    ] {
        hasher.update(value.len().to_le_bytes());
        hasher.update(value.as_bytes());
    }
    let digest = hasher.finalize();
    let digest = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("workspace-write:{digest}")
}

fn git_mutation_fingerprint(request: &GitMutationRequest) -> Result<String, CommandError> {
    let binding = (
        &request.session_id,
        &request.checkout_id,
        &request.context.expected_run_id,
        request.context.expected_generation,
        &request.mutation,
    );
    let bytes = serde_json::to_vec(&binding)
        .map_err(|error| CommandError::operation("workspace_git_request_invalid", error))?;
    let digest = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("workspace-git:{digest}"))
}

fn evaluate_direct_user_policy(
    mut client: hachimi_protocol::ClientContext,
    action: &str,
    resource: &str,
) -> Result<(), CommandError> {
    // The Workbench shell does not retain privileged runtime scopes. The
    // adapter adds one direct-user scope only for this explicit save and never
    // exposes it to the Agent or a later Run.
    client.scopes.insert(Scope::WorkspaceWrite);
    let decision = DefaultPolicy.evaluate(&PolicyContext {
        client: &client,
        method: None,
        required_scope: Scope::WorkspaceWrite,
        entry_profile: EntryProfile::Workbench,
        workload: WorkloadKind::Coding,
        behavior_mode: BehaviorMode::Default,
        approval_policy: ApprovalPolicy::OnlyWhenNeeded,
        permission_profile: PermissionProfile::WorkspaceWrite,
        effect: ToolEffect::WorkspaceWrite,
        action,
        resource,
        capability_host: Some("workspace-worker"),
        schedule_grant_hash: None,
    });
    match decision {
        PolicyDecision::Allow => Ok(()),
        PolicyDecision::Deny { code } | PolicyDecision::RequireApproval { code } => Err(
            CommandError::new(code, "interactive Workspace save was denied by policy"),
        ),
    }
}

fn interactive_grants(workspace: &ResolvedWorkspace, source: &str) -> CapabilityGrantSet {
    let mut grants = expand_permission_profile(
        PermissionProfile::WorkspaceWrite,
        BehaviorMode::Default,
        workspace.session_id.clone(),
        workspace.run.id.clone(),
        workspace.checkout.path.clone(),
    );
    grants.source = source.into();
    grants.review_each_command = false;
    grants.process = hachimi_protocol::ProcessGrant {
        spawn: true,
        interactive: false,
        allowed_commands: vec!["hachimi-workspace-worker".into()],
    };
    grants.network = Default::default();
    grants.computer = Default::default();
    grants
}

fn deterministic_test_sandbox(state: &DesktopState) -> bool {
    #[cfg(all(debug_assertions, feature = "desktop-e2e"))]
    {
        state.sandbox_snapshot().report.backend == "desktop-e2e-deterministic"
    }
    #[cfg(not(all(debug_assertions, feature = "desktop-e2e")))]
    {
        let _ = state;
        false
    }
}

fn require_mutating_sandbox(status: hachimi_sandbox::SandboxStatus) -> Result<(), CommandError> {
    if status == hachimi_sandbox::SandboxStatus::Enforced {
        Ok(())
    } else {
        Err(CommandError::new(
            "sandbox_not_enforced",
            "file saving is disabled until Windows sandbox attestation succeeds",
        ))
    }
}

fn require_expected_run(
    actual_run_id: &hachimi_protocol::RunId,
    actual_generation: u64,
    expected_run_id: &hachimi_protocol::RunId,
    expected_generation: u64,
    code: &'static str,
    message: &'static str,
) -> Result<(), CommandError> {
    if actual_run_id == expected_run_id && actual_generation == expected_generation {
        Ok(())
    } else {
        Err(CommandError::new(code, message))
    }
}

fn restricted_workspace_client(
    state: &DesktopState,
    workspace: &ResolvedWorkspace,
    grant_source: &str,
) -> Result<WorkspaceHostClient, CommandError> {
    require_mutating_sandbox(state.sandbox_status())?;
    let worker_program = workspace_worker_path();
    let mut client = WorkspaceHostClient::new(
        &worker_program,
        &workspace.checkout.path,
        workspace.checkout.id.as_str(),
        workspace.run.generation,
    );
    if deterministic_test_sandbox(state) {
        return Ok(client);
    }
    let backend = state.sandbox_backend().ok_or_else(|| {
        CommandError::new(
            "sandbox_backend_unavailable",
            "Sandbox reported Enforced without a restricted process backend",
        )
    })?;
    let read_only_roots = hachimi_sandbox::prepare_workspace_acl(
        Path::new(&workspace.checkout.path),
        client.run_temp_dir(),
        &worker_program,
    )
    .map_err(|error| CommandError::operation("sandbox_acl_prepare_failed", error))?;
    hachimi_sandbox::attest_workspace_boundaries(
        &sandbox_sidecar_path("hachimi-sandbox-launcher"),
        &sandbox_sidecar_path("hachimi-sandbox-canary"),
        Path::new(&workspace.checkout.path),
        client.run_temp_dir(),
        &worker_program,
        &read_only_roots,
    )
    .map_err(|error| CommandError::operation("sandbox_workspace_attestation_failed", error))?;
    client = client.with_sandbox(
        backend,
        hachimi_workspace::WorkspaceSandboxContext {
            session_id: workspace.session_id.clone(),
            run_id: workspace.run.id.clone(),
            grants: interactive_grants(workspace, grant_source),
        },
        Arc::new(InteractiveWorkspaceLaunchGuard {
            store: state.agent_store.clone(),
        }),
    );
    Ok(client)
}

fn safe_to_abandon_claim(code: WorkspaceErrorCode) -> bool {
    matches!(
        code,
        WorkspaceErrorCode::InvalidRequest
            | WorkspaceErrorCode::Unauthorized
            | WorkspaceErrorCode::StaleGeneration
            | WorkspaceErrorCode::PathOutsideCheckout
            | WorkspaceErrorCode::NotFound
            | WorkspaceErrorCode::NotText
            | WorkspaceErrorCode::TooLarge
            | WorkspaceErrorCode::Conflict
            | WorkspaceErrorCode::ProcessFailed
    )
}

fn workspace_command_error(error: WorkspaceError) -> CommandError {
    CommandError::new(
        format!("workspace_{:?}", error.code).to_lowercase(),
        error.message,
    )
}

#[tauri::command]
pub(super) async fn write_workspace_file(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: FsWriteRequest,
) -> Result<FsWriteResponse, CommandError> {
    let _sandbox_activity = enter_sandbox_activity(&state)?;
    let client = authorize(&window, &state)?;
    let (expected_run_id, expected_generation) =
        validate_write_request(&request, &client.client_id)?;
    let expected_sha256 = expected_sha256(&request.if_match)?;
    evaluate_direct_user_policy(client.clone(), "workspace.editor.save", &request.path)?;
    let workspace =
        resolve_session_workspace(&state, &request.session_id, &request.checkout_id).await?;
    require_expected_run(
        &workspace.run.id,
        workspace.run.generation,
        &expected_run_id,
        expected_generation,
        "workspace_write_precondition_failed",
        "the selected Run or generation changed before the save",
    )?;
    let host = restricted_workspace_client(&state, &workspace, "interactive_editor")?;
    let fingerprint = mutation_fingerprint(&request);
    let now = i64::try_from(epoch_millis()).unwrap_or(i64::MAX);
    match state
        .agent_store
        .claim_idempotent_mutation::<FsWriteResponse>(
            &client.client_id.0,
            MUTATION_METHOD,
            &request.context.idempotency_key,
            &fingerprint,
            now,
        )
        .await
        .map_err(|error| CommandError::operation("workspace_write_claim_failed", error))?
    {
        IdempotentMutationClaim::Completed(previous) => return Ok(previous),
        IdempotentMutationClaim::Indeterminate => {
            return Err(CommandError::new(
                "workspace_write_indeterminate",
                "the original file save was dispatched but has no confirmed result; reload before editing",
            ));
        }
        IdempotentMutationClaim::Claimed => {}
    }
    let output = host
        .execute(
            WorkspaceOperation::WriteFile {
                path: request.path,
                content: request.content,
                expected_sha256: Some(expected_sha256),
            },
            WRITE_TIMEOUT,
            CancellationToken::new(),
        )
        .await;
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            if safe_to_abandon_claim(error.code) {
                let _ = state
                    .agent_store
                    .abandon_idempotent_mutation(
                        &client.client_id.0,
                        MUTATION_METHOD,
                        &request.context.idempotency_key,
                    )
                    .await;
                return Err(workspace_command_error(error));
            }
            return Err(CommandError::new(
                "workspace_write_indeterminate",
                format!(
                    "file save outcome could not be confirmed; reload before editing ({})",
                    error.message
                ),
            ));
        }
    };
    let WorkspaceOutput::Write {
        path,
        sha256,
        byte_size,
        ..
    } = output
    else {
        return Err(CommandError::new(
            "workspace_protocol_mismatch",
            "workspace worker did not return a write result",
        ));
    };
    let response = FsWriteResponse {
        path,
        byte_size,
        etag: format!("sha256:{sha256}"),
    };
    state
        .agent_store
        .complete_idempotent_mutation(
            &client.client_id.0,
            MUTATION_METHOD,
            &request.context.idempotency_key,
            &response,
        )
        .await
        .map_err(|error| CommandError::operation("workspace_write_completion_failed", error))?;
    Ok(response)
}

fn git_operation(mutation: GitMutation) -> WorkspaceOperation {
    match mutation {
        GitMutation::Stage { paths } => WorkspaceOperation::GitStage {
            paths,
            history_limit: 20,
        },
        GitMutation::Unstage { paths } => WorkspaceOperation::GitUnstage {
            paths,
            history_limit: 20,
        },
        GitMutation::Commit { message } => WorkspaceOperation::GitCommit {
            message,
            history_limit: 20,
        },
        GitMutation::CreateEmptyInitialCommit {
            author_name,
            author_email,
        } => WorkspaceOperation::GitCreateEmptyInitialCommit {
            author_name,
            author_email,
            history_limit: 20,
        },
    }
}

fn git_policy_binding(mutation: &GitMutation) -> (&'static str, &'static str) {
    match mutation {
        GitMutation::Stage { .. } => ("workspace.git.stage", "git:index"),
        GitMutation::Unstage { .. } => ("workspace.git.unstage", "git:index"),
        GitMutation::Commit { .. } => ("workspace.git.commit", "git:local-history"),
        GitMutation::CreateEmptyInitialCommit { .. } => {
            ("workspace.git.create_initial_commit", "git:local-history")
        }
    }
}

fn safe_to_abandon_git_claim(code: WorkspaceErrorCode) -> bool {
    matches!(
        code,
        WorkspaceErrorCode::InvalidRequest
            | WorkspaceErrorCode::Unauthorized
            | WorkspaceErrorCode::StaleGeneration
            | WorkspaceErrorCode::PathOutsideCheckout
            | WorkspaceErrorCode::NotFound
            | WorkspaceErrorCode::NotText
            | WorkspaceErrorCode::TooLarge
            | WorkspaceErrorCode::Conflict
    )
}

#[tauri::command]
pub(super) async fn mutate_workspace_git(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: GitMutationRequest,
) -> Result<GitMutationResponse, CommandError> {
    let _sandbox_activity = enter_sandbox_activity(&state)?;
    let client = authorize(&window, &state)?;
    let (expected_run_id, expected_generation) =
        validate_mutation_context(&request.context, &client.client_id)?;
    let (action, resource) = git_policy_binding(&request.mutation);
    evaluate_direct_user_policy(client.clone(), action, resource)?;
    let workspace =
        resolve_session_workspace(&state, &request.session_id, &request.checkout_id).await?;
    require_expected_run(
        &workspace.run.id,
        workspace.run.generation,
        &expected_run_id,
        expected_generation,
        "workspace_git_precondition_failed",
        "the selected Run or generation changed before the Git mutation",
    )?;
    let host = restricted_workspace_client(&state, &workspace, "interactive_git")?;
    let fingerprint = git_mutation_fingerprint(&request)?;
    let now = i64::try_from(epoch_millis()).unwrap_or(i64::MAX);
    match state
        .agent_store
        .claim_idempotent_mutation::<GitMutationResponse>(
            &client.client_id.0,
            GIT_MUTATION_METHOD,
            &request.context.idempotency_key,
            &fingerprint,
            now,
        )
        .await
        .map_err(|error| CommandError::operation("workspace_git_claim_failed", error))?
    {
        IdempotentMutationClaim::Completed(previous) => return Ok(previous),
        IdempotentMutationClaim::Indeterminate => {
            return Err(CommandError::new(
                "workspace_git_indeterminate",
                "the original Git mutation has no confirmed result; refresh status before continuing",
            ));
        }
        IdempotentMutationClaim::Claimed => {}
    }
    let git_acl = if deterministic_test_sandbox(&state) {
        None
    } else {
        match hachimi_sandbox::prepare_git_mutation_acl(Path::new(&workspace.checkout.path)) {
            Ok(acl) => Some(acl),
            Err(error) => {
                let _ = state
                    .agent_store
                    .abandon_idempotent_mutation(
                        &client.client_id.0,
                        GIT_MUTATION_METHOD,
                        &request.context.idempotency_key,
                    )
                    .await;
                return Err(CommandError::operation(
                    "workspace_git_acl_prepare_failed",
                    error,
                ));
            }
        }
    };
    let output = host
        .execute(
            git_operation(request.mutation),
            WRITE_TIMEOUT,
            CancellationToken::new(),
        )
        .await;
    if let Some(acl) = &git_acl
        && let Err(error) = hachimi_sandbox::restore_git_mutation_acl(acl)
    {
        return Err(CommandError::new(
            "workspace_git_indeterminate",
            format!(
                "Git mutation completed with an unknown outcome and its temporary ACL could not be restored: {error}"
            ),
        ));
    }
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            if safe_to_abandon_git_claim(error.code) {
                let _ = state
                    .agent_store
                    .abandon_idempotent_mutation(
                        &client.client_id.0,
                        GIT_MUTATION_METHOD,
                        &request.context.idempotency_key,
                    )
                    .await;
                return Err(workspace_command_error(error));
            }
            return Err(CommandError::new(
                "workspace_git_indeterminate",
                format!(
                    "Git mutation outcome could not be confirmed; refresh status before continuing ({:?})",
                    error.code
                ),
            ));
        }
    };
    let WorkspaceOutput::GitMutation { response } = output else {
        return Err(CommandError::new(
            "workspace_protocol_mismatch",
            "workspace worker did not return a Git mutation result",
        ));
    };
    state
        .agent_store
        .complete_idempotent_mutation(
            &client.client_id.0,
            GIT_MUTATION_METHOD,
            &request.context.idempotency_key,
            &response,
        )
        .await
        .map_err(|error| CommandError::operation("workspace_git_completion_failed", error))?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hachimi_protocol::{
        CheckoutId, GitWorkspaceSnapshot, MutationContext, RequestId, RunId, SessionId,
    };

    fn request() -> FsWriteRequest {
        FsWriteRequest {
            context: MutationContext {
                request_id: RequestId("request".into()),
                client_id: ClientId("window:workbench".into()),
                protocol_version: hachimi_protocol::CONTROL_PROTOCOL_VERSION,
                idempotency_key: "save-1".into(),
                expected_run_id: Some(RunId::from("run")),
                expected_generation: Some(7),
            },
            session_id: SessionId::from("session"),
            checkout_id: CheckoutId::from("checkout"),
            path: "src/lib.rs".into(),
            content: "updated\n".into(),
            if_match: format!("sha256:{}", "a".repeat(64)),
        }
    }

    #[test]
    fn etag_must_be_an_authoritative_sha256() {
        assert_eq!(
            expected_sha256(&format!("sha256:{}", "A".repeat(64))).unwrap(),
            "a".repeat(64)
        );
        assert!(expected_sha256("etag").is_err());
        assert!(expected_sha256("sha256:xyz").is_err());
    }

    #[test]
    fn mutation_fingerprint_changes_with_parameters_but_not_request_id() {
        let first = request();
        let mut same = first.clone();
        same.context.request_id = RequestId("retry".into());
        assert_eq!(mutation_fingerprint(&first), mutation_fingerprint(&same));
        same.content.push_str("different");
        assert_ne!(mutation_fingerprint(&first), mutation_fingerprint(&same));
    }

    #[test]
    fn uncertain_worker_failures_keep_the_claim() {
        assert!(safe_to_abandon_claim(WorkspaceErrorCode::Conflict));
        assert!(safe_to_abandon_claim(WorkspaceErrorCode::StaleGeneration));
        assert!(!safe_to_abandon_claim(WorkspaceErrorCode::TimedOut));
        assert!(!safe_to_abandon_claim(WorkspaceErrorCode::HostDisconnected));
        assert!(!safe_to_abandon_claim(WorkspaceErrorCode::Io));
    }

    fn git_request() -> GitMutationRequest {
        GitMutationRequest {
            context: request().context,
            session_id: SessionId::from("session"),
            checkout_id: CheckoutId::from("checkout"),
            mutation: GitMutation::Commit {
                message: "local commit".into(),
            },
        }
    }

    #[test]
    fn git_fingerprint_fences_parameter_changes_but_allows_request_retries() {
        let first = git_request();
        let mut retry = first.clone();
        retry.context.request_id = RequestId("retry".into());
        assert_eq!(
            git_mutation_fingerprint(&first).unwrap(),
            git_mutation_fingerprint(&retry).unwrap()
        );
        retry.mutation = GitMutation::Commit {
            message: "different commit".into(),
        };
        assert_ne!(
            git_mutation_fingerprint(&first).unwrap(),
            git_mutation_fingerprint(&retry).unwrap()
        );
    }

    #[test]
    fn git_mutation_rejects_stale_generation_and_unenforced_sandbox() {
        let run_id = RunId::from("run");
        let stale = require_expected_run(
            &run_id,
            8,
            &run_id,
            7,
            "workspace_git_precondition_failed",
            "stale",
        )
        .expect_err("stale generation");
        assert_eq!(stale.code, "workspace_git_precondition_failed");
        for status in [
            hachimi_sandbox::SandboxStatus::Disabled,
            hachimi_sandbox::SandboxStatus::SetupRequired,
            hachimi_sandbox::SandboxStatus::Degraded,
        ] {
            assert_eq!(
                require_mutating_sandbox(status).unwrap_err().code,
                "sandbox_not_enforced"
            );
        }
        require_mutating_sandbox(hachimi_sandbox::SandboxStatus::Enforced)
            .expect("enforced sandbox");
    }

    #[test]
    fn git_process_failures_are_not_automatically_replayed() {
        assert!(safe_to_abandon_git_claim(WorkspaceErrorCode::Conflict));
        assert!(safe_to_abandon_git_claim(
            WorkspaceErrorCode::StaleGeneration
        ));
        assert!(!safe_to_abandon_git_claim(
            WorkspaceErrorCode::ProcessFailed
        ));
        assert!(!safe_to_abandon_git_claim(WorkspaceErrorCode::TimedOut));
        assert!(!safe_to_abandon_git_claim(
            WorkspaceErrorCode::HostDisconnected
        ));
    }

    #[tokio::test]
    async fn completed_git_mutation_replays_result_without_a_second_dispatch_claim() {
        let store = hachimi_storage::AgentStore::connect_in_memory()
            .await
            .expect("store");
        let request = git_request();
        let fingerprint = git_mutation_fingerprint(&request).expect("fingerprint");
        let client = request.context.client_id.0.as_str();
        let key = &request.context.idempotency_key;
        assert_eq!(
            store
                .claim_idempotent_mutation::<GitMutationResponse>(
                    client,
                    GIT_MUTATION_METHOD,
                    key,
                    &fingerprint,
                    1,
                )
                .await
                .expect("claim"),
            IdempotentMutationClaim::Claimed
        );
        assert_eq!(
            store
                .claim_idempotent_mutation::<GitMutationResponse>(
                    client,
                    GIT_MUTATION_METHOD,
                    key,
                    &fingerprint,
                    2,
                )
                .await
                .expect("in-flight retry"),
            IdempotentMutationClaim::Indeterminate
        );
        let response = GitMutationResponse {
            snapshot: GitWorkspaceSnapshot {
                branch: Some("main".into()),
                head_sha: Some("abc".into()),
                status_fingerprint: "clean".into(),
                detached: false,
                status: Vec::new(),
                recent_commits: Vec::new(),
            },
            commit_sha: Some("abc".into()),
        };
        store
            .complete_idempotent_mutation(client, GIT_MUTATION_METHOD, key, &response)
            .await
            .expect("complete");
        assert_eq!(
            store
                .claim_idempotent_mutation::<GitMutationResponse>(
                    client,
                    GIT_MUTATION_METHOD,
                    key,
                    &fingerprint,
                    3,
                )
                .await
                .expect("completed retry"),
            IdempotentMutationClaim::Completed(response)
        );
    }
}
