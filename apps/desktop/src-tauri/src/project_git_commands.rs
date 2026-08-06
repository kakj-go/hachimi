// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/git-utils/src/{info,operations}.rs
// @ 4c43465133428898aa84f0bfc02c306ed65fb66a.
// Modified for Hachimi: project reconciliation, unborn-repository UX, restricted
// Workspace Host dispatch, and persistent direct-user idempotency.

use std::{path::Path, sync::Arc, time::Duration};

use hachimi_policy::{
    DefaultPolicy, PolicyContext, PolicyDecision, PolicyEngine, expand_permission_policy,
};
use hachimi_protocol::{
    AgentPermissionPolicy, ApprovalPolicy, AuthorityMode, BehaviorMode, CapabilityGrantSet,
    ClientContext, ClientId, ControlMethod, EntryProfile, PermissionProfile,
    ProjectGitInitialCommitRequest, ProjectGitInitialCommitResponse, ProjectGitSnapshot,
    ProjectGitState, ProjectId, RunId, Scope, SessionId, ToolEffect, WorkloadKind,
};
use hachimi_storage::{AgentStoreError, IdempotentMutationClaim};
use hachimi_workspace::{
    WorkspaceError, WorkspaceErrorCode, WorkspaceHostClient, WorkspaceLaunchCheck,
    WorkspaceLaunchGuard, WorkspaceLaunchValidationFuture, WorkspaceOperation, WorkspaceOutput,
};
use sha2::{Digest, Sha256};
use tauri::{State, WebviewWindow};
use tokio_util::sync::CancellationToken;

use super::{
    CommandError, DesktopState, enter_sandbox_activity, epoch_millis, require_window,
    sandbox_sidecar_path, workspace_worker_path,
};

const INSPECT_TIMEOUT: Duration = Duration::from_secs(20);
const MUTATE_TIMEOUT: Duration = Duration::from_secs(30);
const INITIAL_COMMIT_METHOD: &str = "project.git.create_empty_initial_commit";

#[derive(Clone)]
struct ProjectGitLaunchGuard {
    session_id: SessionId,
    run_id: RunId,
    checkout_id: hachimi_protocol::CheckoutId,
}

impl WorkspaceLaunchGuard for ProjectGitLaunchGuard {
    fn validate(&self, check: WorkspaceLaunchCheck) -> WorkspaceLaunchValidationFuture {
        let allowed = check.session_id == self.session_id
            && check.run_id == self.run_id
            && check.checkout_id == self.checkout_id
            && check.run_generation == 0
            && check.effect == ToolEffect::WorkspaceWrite;
        Box::pin(async move {
            allowed.then_some(()).ok_or_else(|| {
                WorkspaceError::new(
                    WorkspaceErrorCode::Unauthorized,
                    "project Git launch binding changed before dispatch",
                )
            })
        })
    }
}

fn authorize(window: &WebviewWindow, state: &DesktopState) -> Result<ClientContext, CommandError> {
    let client = state.authorize(window, ControlMethod::WorkbenchWindow)?;
    require_window(window, "workbench")?;
    Ok(client)
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

fn project_git_ids(project_id: &ProjectId) -> (SessionId, RunId, hachimi_protocol::CheckoutId) {
    (
        SessionId::new(format!("project-git-session:{}", project_id.as_str())),
        RunId::new(format!("project-git-run:{}", project_id.as_str())),
        hachimi_protocol::CheckoutId::new(format!("project-git-checkout:{}", project_id.as_str())),
    )
}

fn validate_context(
    request: &ProjectGitInitialCommitRequest,
    authenticated_client: &ClientId,
) -> Result<(), CommandError> {
    let context = &request.context;
    if context.protocol_version != hachimi_protocol::CONTROL_PROTOCOL_VERSION
        || &context.client_id != authenticated_client
        || context.request_id.0.trim().is_empty()
        || context.idempotency_key.trim().is_empty()
        || context.idempotency_key.len() > 128
        || context.expected_run_id.is_some()
        || context.expected_generation.is_some()
    {
        return Err(CommandError::new(
            "project_git_context_invalid",
            "project Git initialization requires a bounded direct-user mutation context",
        ));
    }
    Ok(())
}

fn evaluate_policy(
    mut client: ClientContext,
    project: &hachimi_protocol::ProjectRecord,
) -> Result<(), CommandError> {
    client.scopes.insert(Scope::WorkspaceWrite);
    match DefaultPolicy.evaluate(&PolicyContext {
        client: &client,
        method: None,
        required_scope: Scope::WorkspaceWrite,
        entry_profile: EntryProfile::Workbench,
        workload: WorkloadKind::Coding,
        behavior_mode: BehaviorMode::Default,
        approval_policy: ApprovalPolicy::OnlyWhenNeeded,
        permission_profile: PermissionProfile::Writable,
        effect: ToolEffect::WorkspaceWrite,
        action: "project.git.create_empty_initial_commit",
        resource: &project.root_path,
        capability_host: Some("workspace-worker"),
    }) {
        PolicyDecision::Allow => Ok(()),
        PolicyDecision::Deny { code } | PolicyDecision::RequireApproval { code } => Err(
            CommandError::new(code, "project Git initialization was denied by policy"),
        ),
    }
}

fn initial_commit_fingerprint(request: &ProjectGitInitialCommitRequest) -> String {
    let mut hasher = Sha256::new();
    for value in [
        request.project_id.as_str(),
        request.author_name.trim(),
        request.author_email.trim(),
    ] {
        hasher.update(value.len().to_le_bytes());
        hasher.update(value.as_bytes());
    }
    let digest = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("project-git-initial:{digest}")
}

async fn project(
    state: &DesktopState,
    project_id: &ProjectId,
) -> Result<hachimi_protocol::ProjectRecord, CommandError> {
    state
        .agent_store
        .get_project(project_id)
        .await
        .map_err(|error| CommandError::operation("workbench_project_get_failed", error))?
        .ok_or_else(|| CommandError::new("workbench_project_not_found", "project does not exist"))
}

pub(super) async fn inspect_project_git_state(
    state: &DesktopState,
    project_id: &ProjectId,
) -> Result<ProjectGitSnapshot, CommandError> {
    let project = project(state, project_id).await?;
    let host = WorkspaceHostClient::new(
        workspace_worker_path(),
        &project.root_path,
        format!("project-inspect:{}", project.id.as_str()),
        0,
    );
    let snapshot = match host
        .execute(
            WorkspaceOperation::GitProjectInspect {
                project_id: project.id.clone(),
            },
            INSPECT_TIMEOUT,
            CancellationToken::new(),
        )
        .await
    {
        Ok(WorkspaceOutput::ProjectGitSnapshot { snapshot }) => snapshot,
        Ok(_) => {
            return Err(CommandError::new(
                "workspace_protocol_mismatch",
                "workspace worker did not return project Git state",
            ));
        }
        Err(error) => ProjectGitSnapshot {
            project_id: project.id.clone(),
            git_root: project.git_root.clone(),
            state: ProjectGitState::Unavailable {
                error_code: format!("workspace_{:?}", error.code).to_lowercase(),
            },
            observed_at_ms: i64::try_from(epoch_millis()).unwrap_or(i64::MAX),
        },
    };
    if !matches!(snapshot.state, ProjectGitState::Unavailable { .. }) {
        state
            .agent_store
            .update_project_git_root(
                project_id,
                snapshot.git_root.as_deref(),
                snapshot.observed_at_ms,
            )
            .await
            .map_err(|error| CommandError::operation("project_git_reconcile_failed", error))?;
    }
    Ok(snapshot)
}

#[tauri::command]
pub(super) async fn inspect_project_git(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    project_id: ProjectId,
) -> Result<ProjectGitSnapshot, CommandError> {
    authorize(&window, &state)?;
    inspect_project_git_state(&state, &project_id).await
}

#[tauri::command]
pub(super) async fn refresh_project_git(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    project_id: ProjectId,
) -> Result<ProjectGitSnapshot, CommandError> {
    authorize(&window, &state)?;
    inspect_project_git_state(&state, &project_id).await
}

#[tauri::command]
pub(super) async fn create_project_empty_initial_commit(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ProjectGitInitialCommitRequest,
) -> Result<ProjectGitInitialCommitResponse, CommandError> {
    let _sandbox_activity = enter_sandbox_activity(&state)?;
    let client = authorize(&window, &state)?;
    validate_context(&request, &client.client_id)?;
    let project = project(&state, &request.project_id).await?;
    evaluate_policy(client.clone(), &project)?;
    let current = inspect_project_git_state(&state, &project.id).await?;
    if !matches!(current.state, ProjectGitState::Unborn { .. }) {
        return Err(CommandError::new(
            "project_git_not_unborn",
            "an empty initial commit is only available for an unborn branch",
        ));
    }
    if state.sandbox_status() != hachimi_sandbox::SandboxStatus::Enforced {
        return Err(CommandError::new(
            "sandbox_not_enforced",
            "Git initialization is disabled until Windows sandbox attestation succeeds",
        ));
    }
    let fingerprint = initial_commit_fingerprint(&request);
    let now = i64::try_from(epoch_millis()).unwrap_or(i64::MAX);
    match state
        .agent_store
        .claim_idempotent_mutation::<ProjectGitInitialCommitResponse>(
            &client.client_id.0,
            INITIAL_COMMIT_METHOD,
            &request.context.idempotency_key,
            &fingerprint,
            now,
        )
        .await
        .map_err(|error| match error {
            AgentStoreError::IdempotencyConflict => CommandError::new(
                "idempotency_conflict",
                "the idempotency key was already used with different initial-commit parameters",
            ),
            other => CommandError::operation("project_git_claim_failed", other),
        })? {
        IdempotentMutationClaim::Completed(response) => return Ok(response),
        IdempotentMutationClaim::Indeterminate => {
            return Err(CommandError::new(
                "project_git_indeterminate",
                "the initial commit outcome is unknown; refresh Git state before continuing",
            ));
        }
        IdempotentMutationClaim::Claimed => {}
    }

    let (session_id, run_id, checkout_id) = project_git_ids(&project.id);
    let worker_program = workspace_worker_path();
    let mut host =
        WorkspaceHostClient::new(&worker_program, &project.root_path, checkout_id.as_str(), 0);
    if !deterministic_test_sandbox(&state) {
        let backend = state.sandbox_backend().ok_or_else(|| {
            CommandError::new(
                "sandbox_backend_unavailable",
                "Sandbox reported Enforced without a restricted process backend",
            )
        })?;
        let read_only_roots = hachimi_sandbox::prepare_workspace_acl(
            Path::new(&project.root_path),
            host.run_temp_dir(),
            &worker_program,
        )
        .map_err(|error| CommandError::operation("sandbox_acl_prepare_failed", error))?;
        hachimi_sandbox::attest_workspace_boundaries(
            &sandbox_sidecar_path("hachimi-sandbox-launcher"),
            &sandbox_sidecar_path("hachimi-sandbox-canary"),
            Path::new(&project.root_path),
            host.run_temp_dir(),
            &worker_program,
            &read_only_roots,
        )
        .map_err(|error| CommandError::operation("sandbox_workspace_attestation_failed", error))?;
        let mut grants: CapabilityGrantSet = expand_permission_policy(
            &AgentPermissionPolicy {
                level: PermissionProfile::Writable,
                ..AgentPermissionPolicy::default()
            },
            AuthorityMode::Interactive,
            BehaviorMode::Default,
            session_id.clone(),
            run_id.clone(),
            project.root_path.clone(),
        );
        grants.source = "direct_user_project_git".into();
        grants.review_each_command = false;
        grants.network = Default::default();
        grants.computer = Default::default();
        host = host.with_sandbox(
            backend,
            hachimi_workspace::WorkspaceSandboxContext {
                session_id: session_id.clone(),
                run_id: run_id.clone(),
                grants,
            },
            Arc::new(ProjectGitLaunchGuard {
                session_id,
                run_id,
                checkout_id,
            }),
        );
    }
    // Establish and attest the normal read-only Git boundary first. The fixed
    // initial-commit operation then receives a narrowly scoped, temporary ACL
    // upgrade; preparing the Workspace ACL after this upgrade would silently
    // replace it with the normal deny-write ACE before the Worker launches.
    let git_acl = if deterministic_test_sandbox(&state) {
        None
    } else {
        Some(
            hachimi_sandbox::prepare_git_mutation_acl(Path::new(&project.root_path)).map_err(
                |error| CommandError::operation("project_git_acl_prepare_failed", error),
            )?,
        )
    };
    let output = host
        .execute(
            WorkspaceOperation::GitCreateEmptyInitialCommit {
                author_name: request.author_name.trim().to_owned(),
                author_email: request.author_email.trim().to_owned(),
                history_limit: 20,
            },
            MUTATE_TIMEOUT,
            CancellationToken::new(),
        )
        .await;
    if let Err(error) = &output {
        tracing::warn!(
            error_code = ?error.code,
            "restricted Workspace Host failed the fixed empty initial commit operation"
        );
    }
    if let Some(acl) = &git_acl
        && let Err(error) = hachimi_sandbox::restore_git_mutation_acl(acl)
    {
        return Err(CommandError::new(
            "project_git_indeterminate",
            format!("initial commit outcome is unknown and Git ACL restoration failed: {error}"),
        ));
    }
    let output = match output {
        Ok(output) => output,
        Err(error)
            if matches!(
                error.code,
                WorkspaceErrorCode::InvalidRequest
                    | WorkspaceErrorCode::Conflict
                    | WorkspaceErrorCode::Unauthorized
            ) =>
        {
            let _ = state
                .agent_store
                .abandon_idempotent_mutation(
                    &client.client_id.0,
                    INITIAL_COMMIT_METHOD,
                    &request.context.idempotency_key,
                )
                .await;
            return Err(CommandError::new(
                format!("workspace_{:?}", error.code).to_lowercase(),
                error.message,
            ));
        }
        Err(error) => {
            return Err(CommandError::new(
                "project_git_indeterminate",
                format!(
                    "initial commit outcome could not be confirmed: {}",
                    error.message
                ),
            ));
        }
    };
    let WorkspaceOutput::GitMutation { response: git } = output else {
        return Err(CommandError::new(
            "workspace_protocol_mismatch",
            "workspace worker did not return an initial commit result",
        ));
    };
    let commit_sha = git.commit_sha.ok_or_else(|| {
        CommandError::new(
            "project_git_commit_missing",
            "Git did not return the initial commit ID",
        )
    })?;
    let git_snapshot = git.snapshot;
    let snapshot = ProjectGitSnapshot {
        project_id: project.id.clone(),
        git_root: current.git_root.or_else(|| Some(project.root_path.clone())),
        state: if git_snapshot.detached {
            ProjectGitState::Detached {
                head: commit_sha.clone(),
            }
        } else {
            ProjectGitState::Ready {
                branch: git_snapshot.branch,
                head: commit_sha.clone(),
            }
        },
        observed_at_ms: i64::try_from(epoch_millis()).unwrap_or(i64::MAX),
    };
    state
        .agent_store
        .update_project_git_root(
            &project.id,
            snapshot.git_root.as_deref(),
            snapshot.observed_at_ms,
        )
        .await
        .map_err(|error| CommandError::operation("project_git_reconcile_failed", error))?;
    let response = ProjectGitInitialCommitResponse {
        snapshot,
        commit_sha,
    };
    state
        .agent_store
        .complete_idempotent_mutation(
            &client.client_id.0,
            INITIAL_COMMIT_METHOD,
            &request.context.idempotency_key,
            &response,
        )
        .await
        .map_err(|error| CommandError::operation("project_git_completion_failed", error))?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use hachimi_protocol::{
        ClientId, MutationContext, ProjectGitInitialCommitRequest, ProjectGitInitialCommitResponse,
        ProjectGitSnapshot, ProjectGitState, ProjectId, RequestId,
    };
    use hachimi_storage::{AgentStore, AgentStoreError, IdempotentMutationClaim};

    use super::{INITIAL_COMMIT_METHOD, initial_commit_fingerprint};

    fn request(key: &str, author_name: &str) -> ProjectGitInitialCommitRequest {
        ProjectGitInitialCommitRequest {
            context: MutationContext {
                request_id: RequestId("request".into()),
                client_id: ClientId("window:workbench".into()),
                protocol_version: hachimi_protocol::CONTROL_PROTOCOL_VERSION,
                idempotency_key: key.into(),
                expected_run_id: None,
                expected_generation: None,
            },
            project_id: ProjectId::new("project"),
            author_name: author_name.into(),
            author_email: "user@example.com".into(),
        }
    }

    fn response() -> ProjectGitInitialCommitResponse {
        ProjectGitInitialCommitResponse {
            snapshot: ProjectGitSnapshot {
                project_id: ProjectId::new("project"),
                git_root: Some("C:\\project".into()),
                state: ProjectGitState::Ready {
                    branch: Some("main".into()),
                    head: "0123456789abcdef".into(),
                },
                observed_at_ms: 7,
            },
            commit_sha: "0123456789abcdef".into(),
        }
    }

    #[tokio::test]
    async fn initial_commit_claim_is_persistent_replay_safe_and_parameter_bound() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let first = request("initial-key", "Hachimi User");
        let fingerprint = initial_commit_fingerprint(&first);
        assert_eq!(
            store
                .claim_idempotent_mutation::<ProjectGitInitialCommitResponse>(
                    "window:workbench",
                    INITIAL_COMMIT_METHOD,
                    &first.context.idempotency_key,
                    &fingerprint,
                    1,
                )
                .await
                .expect("claim"),
            IdempotentMutationClaim::Claimed
        );
        assert_eq!(
            store
                .claim_idempotent_mutation::<ProjectGitInitialCommitResponse>(
                    "window:workbench",
                    INITIAL_COMMIT_METHOD,
                    &first.context.idempotency_key,
                    &fingerprint,
                    2,
                )
                .await
                .expect("in-flight replay"),
            IdempotentMutationClaim::Indeterminate
        );

        let completed = response();
        store
            .complete_idempotent_mutation(
                "window:workbench",
                INITIAL_COMMIT_METHOD,
                &first.context.idempotency_key,
                &completed,
            )
            .await
            .expect("complete");
        assert_eq!(
            store
                .claim_idempotent_mutation::<ProjectGitInitialCommitResponse>(
                    "window:workbench",
                    INITIAL_COMMIT_METHOD,
                    &first.context.idempotency_key,
                    &fingerprint,
                    3,
                )
                .await
                .expect("completed replay"),
            IdempotentMutationClaim::Completed(completed)
        );

        let conflicting = request("initial-key", "Different User");
        assert!(matches!(
            store
                .claim_idempotent_mutation::<ProjectGitInitialCommitResponse>(
                    "window:workbench",
                    INITIAL_COMMIT_METHOD,
                    &conflicting.context.idempotency_key,
                    &initial_commit_fingerprint(&conflicting),
                    4,
                )
                .await,
            Err(AgentStoreError::IdempotencyConflict)
        ));
    }
}
