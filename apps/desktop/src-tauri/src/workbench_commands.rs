use super::*;
use hachimi_agent::ModelRuntime;
#[cfg(all(debug_assertions, feature = "desktop-e2e"))]
use hachimi_agent::ModelRuntimeError;
use hachimi_control_plane::{AppServerContext, AppServerRequest, AppServerResponse};

#[path = "workbench_mcp_settings_commands.rs"]
mod mcp_settings_commands;
pub(super) use mcp_settings_commands::*;
#[path = "workbench_interactive_grants.rs"]
mod interactive_grants;

#[tauri::command]
pub(super) async fn list_workbench_projects(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Vec<ProjectRecord>, CommandError> {
    state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    state
        .workbench
        .projects()
        .await
        .map_err(|error| CommandError::operation("workbench_projects_failed", error))
}

pub(super) fn require_mcp_runtime(state: &DesktopState) -> Result<(), CommandError> {
    if state.control_plane.feature_flags().mcp_runtime {
        Ok(())
    } else {
        Err(CommandError::new(
            "mcp_runtime_disabled",
            "MCP runtime is disabled by the emergency kill switch",
        ))
    }
}

#[tauri::command]
pub(super) async fn list_run_recoveries(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Vec<hachimi_protocol::RunRecoverySnapshot>, CommandError> {
    state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    if !state
        .control_plane
        .feature_flags()
        .runtime_features
        .run_recovery
    {
        return Err(CommandError::new("feature_disabled", "run_recovery"));
    }
    state
        .agent_store
        .list_pending_run_recoveries()
        .await
        .map_err(|error| CommandError::operation("run_recovery_list_failed", error))
}

#[tauri::command]
pub(super) async fn resolve_run_recovery(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: hachimi_protocol::RunRecoveryDecisionRequest,
) -> Result<hachimi_protocol::RunRecoverySnapshot, CommandError> {
    let client = state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    if !state
        .control_plane
        .feature_flags()
        .runtime_features
        .run_recovery
    {
        return Err(CommandError::new("feature_disabled", "run_recovery"));
    }
    validate_recovery_context(&request, &client.client_id)?;
    let before = state
        .agent_store
        .get_run_recovery_snapshot(&request.recovery_id)
        .await
        .map_err(|error| CommandError::operation("run_recovery_get_failed", error))?
        .ok_or_else(|| CommandError::new("run_recovery_not_found", "Run recovery not found"))?;
    let resolved = state
        .agent_store
        .resolve_run_recovery(&request, &client.client_id.0, now_ms())
        .await
        .map_err(|error| CommandError::operation("run_recovery_resolve_failed", error))?;
    if matches!(
        before.recovery.state,
        hachimi_protocol::RunRecoveryState::EligibleAuto
            | hachimi_protocol::RunRecoveryState::AwaitingUser
    ) && resolved.recovery.state == hachimi_protocol::RunRecoveryState::Resuming
    {
        launch_recovered_workbench_run(app, &state, client, &resolved).await?;
    }
    Ok(resolved)
}

fn validate_recovery_context(
    request: &hachimi_protocol::RunRecoveryDecisionRequest,
    client_id: &hachimi_protocol::ClientId,
) -> Result<(), CommandError> {
    let context = &request.context;
    if context.protocol_version != hachimi_protocol::CONTROL_PROTOCOL_VERSION
        || &context.client_id != client_id
        || context.idempotency_key.trim().is_empty()
        || context.idempotency_key.len() > 128
        || context.expected_run_id.as_ref() != Some(&request.expected_run_id)
        || context.expected_generation != Some(request.expected_interrupted_generation)
    {
        return Err(CommandError::new(
            "run_recovery_context_invalid",
            "Run recovery requires an authenticated, generation-fenced mutation context",
        ));
    }
    Ok(())
}

async fn launch_recovered_workbench_run(
    app: AppHandle,
    state: &DesktopState,
    client: ClientContext,
    recovery: &hachimi_protocol::RunRecoverySnapshot,
) -> Result<(), CommandError> {
    let run = state
        .agent_store
        .get_run(&recovery.recovery.run_id)
        .await
        .map_err(|error| CommandError::operation("run_recovery_run_get_failed", error))?
        .ok_or_else(|| CommandError::new("run_not_found", "Recovered Run not found"))?;
    let session = state
        .agent_store
        .get_session(&run.session_id)
        .await
        .map_err(|error| CommandError::operation("run_recovery_session_get_failed", error))?
        .ok_or_else(|| CommandError::new("session_not_found", "Recovered Session not found"))?;
    if session.entry_profile != hachimi_protocol::EntryProfile::Workbench {
        return Err(CommandError::new(
            "run_recovery_profile_unsupported",
            "Automatic recovery currently requires a Workbench Session",
        ));
    }
    let (project, checkout) = match &session.context {
        hachimi_protocol::SessionContextBinding::Project {
            project_id,
            checkout_id,
        } => {
            let project = state
                .agent_store
                .get_project(project_id)
                .await
                .map_err(|error| {
                    CommandError::operation("run_recovery_project_get_failed", error)
                })?;
            let checkout = state
                .agent_store
                .get_checkout(checkout_id)
                .await
                .map_err(|error| {
                    CommandError::operation("run_recovery_checkout_get_failed", error)
                })?;
            (project, checkout)
        }
        hachimi_protocol::SessionContextBinding::Workspace { .. } => (None, None),
    };
    spawn_workbench_run_with_recovery(
        app,
        client,
        WorkbenchTaskSnapshot {
            project,
            checkout,
            session,
            run,
        },
        Vec::new(),
        recovery.checkpoint.clone(),
    );
    Ok(())
}

pub(super) fn schedule_auto_resume_runs(app: AppHandle, run_ids: Vec<hachimi_protocol::RunId>) {
    if run_ids.is_empty() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        for run_id in run_ids {
            let state = app.state::<DesktopState>();
            let pending = match state.agent_store.list_pending_run_recoveries().await {
                Ok(pending) => pending,
                Err(error) => {
                    tracing::warn!(%run_id, %error, "failed to list recoverable Runs");
                    continue;
                }
            };
            let Some(recovery) = pending.into_iter().find(|candidate| {
                candidate.recovery.run_id == run_id
                    && candidate.recovery.state == hachimi_protocol::RunRecoveryState::EligibleAuto
            }) else {
                continue;
            };
            let principal = hachimi_protocol::ClientId("system:restart".to_string());
            let request = hachimi_protocol::RunRecoveryDecisionRequest {
                context: hachimi_protocol::MutationContext {
                    request_id: hachimi_protocol::RequestId(format!(
                        "recovery:{}",
                        recovery.recovery.id
                    )),
                    client_id: principal.clone(),
                    protocol_version: hachimi_protocol::CONTROL_PROTOCOL_VERSION,
                    idempotency_key: format!("auto-resume:{}", recovery.recovery.id),
                    expected_run_id: Some(run_id.clone()),
                    expected_generation: Some(recovery.recovery.interrupted_generation),
                },
                recovery_id: recovery.recovery.id.clone(),
                expected_run_id: run_id.clone(),
                expected_interrupted_generation: recovery.recovery.interrupted_generation,
                action: hachimi_protocol::RunRecoveryDecisionAction::ResumeSafeRemainder,
            };
            let resolved = match state
                .agent_store
                .resolve_run_recovery(&request, &principal.0, now_ms())
                .await
            {
                Ok(resolved) => resolved,
                Err(error) => {
                    tracing::warn!(%run_id, %error, "automatic Run recovery decision failed");
                    continue;
                }
            };
            let mut client = ClientContext::for_window(hachimi_core::WindowKind::Service);
            client.client_id = principal;
            if let Err(error) =
                launch_recovered_workbench_run(app.clone(), &state, client, &resolved).await
            {
                tracing::warn!(%run_id, code = error.code, "automatic Run recovery launch failed");
            }
        }
    });
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

#[tauri::command]
pub(super) async fn add_workbench_project(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Option<ProjectRecord>, CommandError> {
    state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    let selected = match desktop_e2e_path("HACHIMI_DESKTOP_E2E_PROJECT_PATH") {
        Some(path) => Some(path),
        None => rfd::AsyncFileDialog::new()
            .pick_folder()
            .await
            .map(|handle| handle.path().to_path_buf()),
    };
    let Some(folder) = selected else {
        return Ok(None);
    };
    let project = state
        .workbench
        .add_project(&folder)
        .await
        .map_err(|error| CommandError::operation("workbench_project_add_failed", error))?;
    let _ = crate::project_git_commands::inspect_project_git_state(&state, &project.id).await?;
    state
        .agent_store
        .get_project(&project.id)
        .await
        .map_err(|error| CommandError::operation("workbench_project_get_failed", error))
}

#[tauri::command]
pub(super) async fn manage_workbench_project(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    project_id: ProjectId,
    action: String,
    value: Option<String>,
) -> Result<ProjectRecord, CommandError> {
    state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    let project = state
        .workbench
        .store()
        .get_project(&project_id)
        .await
        .map_err(|error| CommandError::operation("workbench_project_get_failed", error))?
        .ok_or_else(|| {
            CommandError::new("workbench_project_not_found", "project does not exist")
        })?;
    match action.as_str() {
        "open" => {
            #[cfg(target_os = "windows")]
            let mut command = hachimi_process_policy::std_command(
                "explorer.exe",
                hachimi_process_policy::ProcessPolicy::VisibleApplication,
            );
            #[cfg(target_os = "macos")]
            let mut command = hachimi_process_policy::std_command(
                "open",
                hachimi_process_policy::ProcessPolicy::VisibleApplication,
            );
            #[cfg(all(unix, not(target_os = "macos")))]
            let mut command = hachimi_process_policy::std_command(
                "xdg-open",
                hachimi_process_policy::ProcessPolicy::VisibleApplication,
            );
            command
                .arg(&project.root_path)
                .spawn()
                .map_err(|error| CommandError::operation("workbench_project_open_failed", error))?;
            Ok(project)
        }
        "rename" => state
            .workbench
            .rename_project(&project_id, value.as_deref().unwrap_or_default())
            .await
            .map_err(|error| CommandError::operation("workbench_project_rename_failed", error)),
        "create_permanent_worktree" => {
            let base_revision = value.unwrap_or_default();
            let target = hachimi_protocol::ExecutionTarget::ManagedWorktree {
                project_id: project_id.clone(),
                base_revision,
            };
            let checkout = state
                .workbench
                .prepare_checkout(&target, &CancellationToken::new())
                .await
                .map_err(|error| {
                    CommandError::operation("workbench_worktree_create_failed", error)
                })?;
            state
                .workbench
                .pin_checkout(&checkout.id, true)
                .await
                .map_err(|error| CommandError::operation("workbench_worktree_pin_failed", error))?;
            Ok(project)
        }
        _ => Err(CommandError::new(
            "workbench_project_action_invalid",
            "project action is not supported",
        )),
    }
}

#[tauri::command]
pub(super) async fn import_workbench_attachment(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Option<AttachmentRecord>, CommandError> {
    state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    if !state.control_plane.feature_flags().workspace_tools {
        return Err(CommandError::new(
            "workspace_tools_disabled",
            "workspace tools are disabled in this build",
        ));
    }
    let selected = match desktop_e2e_path("HACHIMI_DESKTOP_E2E_ATTACHMENT_PATH") {
        Some(path) => Some(path),
        None => rfd::AsyncFileDialog::new()
            .pick_file()
            .await
            .map(|handle| handle.path().to_path_buf()),
    };
    let Some(file) = selected else {
        return Ok(None);
    };
    state
        .workbench
        .import_attachment(&file)
        .await
        .map(Some)
        .map_err(|error| CommandError::operation("workbench_attachment_import_failed", error))
}

#[tauri::command]
pub(super) async fn read_workbench_attachment(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    attachment_id: hachimi_protocol::AttachmentId,
) -> Result<hachimi_protocol::WorkbenchAttachmentPreview, CommandError> {
    state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    state
        .workbench
        .attachment_preview(&attachment_id)
        .await
        .map_err(|error| CommandError::operation("workbench_attachment_preview_failed", error))
}

#[tauri::command]
pub(super) async fn list_workbench_sessions(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    project_id: Option<ProjectId>,
) -> Result<Vec<hachimi_protocol::WorkbenchSessionListItem>, CommandError> {
    state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    state
        .workbench
        .sessions(project_id.as_ref())
        .await
        .map_err(|error| CommandError::operation("workbench_sessions_failed", error))
}

#[tauri::command]
pub(super) async fn get_workbench_session(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    session_id: SessionId,
) -> Result<WorkbenchSessionSnapshot, CommandError> {
    state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    state
        .workbench
        .session_snapshot(&session_id)
        .await
        .map_err(|error| CommandError::operation("workbench_session_get_failed", error))
}

#[tauri::command]
pub(super) async fn resolve_workbench_approval(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ApprovalDecisionRequest,
) -> Result<hachimi_protocol::ApprovalRequestRecord, CommandError> {
    let client = state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    if !matches!(
        request.decision,
        ApprovalStatus::Approved | ApprovalStatus::Denied
    ) {
        return Err(CommandError::new(
            "invalid_approval_decision",
            "approval decision must be approved or denied",
        ));
    }
    let approval = state
        .workbench
        .store()
        .get_approval(&request.approval_id)
        .await
        .map_err(|error| CommandError::operation("workbench_approval_get_failed", error))?
        .ok_or_else(|| {
            CommandError::new("workbench_approval_not_found", "approval does not exist")
        })?;
    if approval.run_id != request.expected_run_id
        || approval.run_generation != request.expected_generation
    {
        return Err(CommandError::new(
            "workbench_approval_precondition_failed",
            "approval does not belong to the expected Run generation",
        ));
    }
    state
        .workbench
        .store()
        .assert_run_precondition(
            &approval.run_id,
            &request.expected_run_id,
            request.expected_generation,
        )
        .await
        .map_err(|error| {
            CommandError::operation("workbench_approval_precondition_failed", error)
        })?;
    let resolution = ApprovalResolution {
        approval_id: approval.id,
        decision: request.decision,
        parameter_hash: approval.parameter_hash,
        run_generation: approval.run_generation,
        resolved_by: client.client_id.0.clone(),
        resolved_at_ms: i64::try_from(epoch_millis()).unwrap_or(i64::MAX),
    };
    let principal = resolution.resolved_by.clone();
    let AppServerResponse::Approval(record) = state
        .app_server
        .dispatch(
            &AppServerContext { client, principal },
            AppServerRequest::ResolveApproval(resolution),
        )
        .await
        .map_err(|error| CommandError::operation("workbench_approval_resolve_failed", error))?
    else {
        return Err(CommandError::new(
            "agent_response_mismatch",
            "approval resolve response mismatch",
        ));
    };
    Ok(record)
}

#[tauri::command]
pub(super) async fn accept_workbench_plan(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: PlanAcceptanceRequest,
) -> Result<WorkbenchPlanAcceptanceSnapshot, CommandError> {
    let client = state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    if !state.control_plane.feature_flags().workspace_tools {
        return Err(CommandError::new(
            "workspace_tools_disabled",
            "workspace tools are disabled in this build",
        ));
    }
    if request.idempotency_key.trim().is_empty() || request.idempotency_key.len() > 128 {
        return Err(CommandError::new(
            "invalid_idempotency_key",
            "idempotency key must contain 1-128 bytes",
        ));
    }
    let model_snapshot = state.settings.read().llm.clone();
    let accepted = state
        .workbench
        .accept_plan(&request, model_snapshot, &client.client_id.0)
        .await
        .map_err(|error| CommandError::operation("workbench_plan_accept_failed", error))?;
    if let Ok(environment) = state
        .workbench
        .environment_snapshot(&accepted.task.session.id)
        .await
    {
        crate::environment_commands::emit_workbench_environment(
            &app,
            &environment,
            vec![hachimi_protocol::WorkbenchEnvironmentChangeReason::Plan],
        );
    }
    if accepted.task.run.status == hachimi_protocol::RunStatus::Queued {
        spawn_workbench_run(app, client, accepted.task.clone(), Vec::new());
    }
    Ok(accepted)
}

#[tauri::command]
pub(super) async fn list_project_git_refs(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    project_id: ProjectId,
) -> Result<Vec<GitRefRecord>, CommandError> {
    state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    let snapshot =
        crate::project_git_commands::inspect_project_git_state(&state, &project_id).await?;
    if !matches!(
        snapshot.state,
        hachimi_protocol::ProjectGitState::Ready { .. }
            | hachimi_protocol::ProjectGitState::Detached { .. }
    ) {
        return Ok(Vec::new());
    }
    state
        .workbench
        .git_refs(&project_id)
        .await
        .map_err(|error| CommandError::operation("workbench_git_refs_failed", error))
}

#[tauri::command]
pub(super) async fn pin_workbench_checkout(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    checkout_id: hachimi_protocol::CheckoutId,
    pinned: bool,
) -> Result<hachimi_protocol::CheckoutRecord, CommandError> {
    state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    state
        .workbench
        .pin_checkout(&checkout_id, pinned)
        .await
        .map_err(|error| CommandError::operation("workbench_checkout_pin_failed", error))
}

#[tauri::command]
pub(super) async fn cleanup_workbench_checkout(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    checkout_id: hachimi_protocol::CheckoutId,
) -> Result<hachimi_protocol::CheckoutRecord, CommandError> {
    state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    cancel_workspace_transients_for_checkout(&state, &checkout_id);
    state
        .workbench
        .cleanup_checkout(&checkout_id)
        .await
        .map_err(|error| CommandError::operation("workbench_checkout_cleanup_failed", error))
}

#[tauri::command]
pub(super) async fn start_workbench_task(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: WorkbenchTaskStartRequest,
) -> Result<WorkbenchTaskSnapshot, CommandError> {
    let client = state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    if !state.control_plane.feature_flags().workspace_tools {
        return Err(CommandError::new(
            "workspace_tools_disabled",
            "workspace tools are disabled in this build",
        ));
    }
    if request.idempotency_key.trim().is_empty() || request.idempotency_key.len() > 128 {
        return Err(CommandError::new(
            "invalid_idempotency_key",
            "idempotency key must contain 1-128 bytes",
        ));
    }
    let model_snapshot = state.settings.read().llm.clone();
    let idempotency_key = request.idempotency_key.clone();
    let snapshot = state
        .workbench
        .create_task(
            &request,
            model_snapshot,
            &client.client_id.0,
            &idempotency_key,
            &CancellationToken::new(),
        )
        .await
        .map_err(|error| CommandError::operation("workbench_task_start_failed", error))?;
    if !request.attachment_ids.is_empty()
        && let Ok(environment) = state
            .workbench
            .environment_snapshot(&snapshot.session.id)
            .await
    {
        crate::environment_commands::emit_workbench_environment(
            &app,
            &environment,
            vec![hachimi_protocol::WorkbenchEnvironmentChangeReason::Sources],
        );
    }
    if snapshot.run.status == hachimi_protocol::RunStatus::Queued {
        spawn_workbench_run(app.clone(), client, snapshot.clone(), request.skill_ids);
    }
    Ok(snapshot)
}

#[tauri::command]
pub(super) async fn compact_workbench_session(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: hachimi_protocol::ManualCompactionRequest,
) -> Result<hachimi_protocol::ManualCompactionResult, CommandError> {
    let client = state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    if request.idempotency_key.trim().is_empty() || request.idempotency_key.len() > 128 {
        return Err(CommandError::new(
            "invalid_idempotency_key",
            "idempotency key must contain 1-128 bytes",
        ));
    }
    let snapshot = state
        .workbench
        .session_snapshot(&request.session_id)
        .await
        .map_err(|error| CommandError::operation("manual_compaction_session_not_found", error))?;
    if state
        .agent_executor
        .registry()
        .has_active_session(&request.session_id)
        || snapshot.runs.iter().any(|run| !run.status.is_terminal())
    {
        return Err(CommandError::new(
            "manual_compaction_active_run",
            "manual compaction requires an idle Session",
        ));
    }
    let resource_id = request.session_id.as_str();
    match state
        .agent_store
        .claim_idempotent_mutation::<hachimi_protocol::ManualCompactionResult>(
            &client.client_id.0,
            "workbench.session.compact",
            &request.idempotency_key,
            resource_id,
            now_ms(),
        )
        .await
        .map_err(|error| CommandError::operation("manual_compaction_claim_failed", error))?
    {
        hachimi_storage::IdempotentMutationClaim::Completed(result) => return Ok(result),
        hachimi_storage::IdempotentMutationClaim::Indeterminate => {
            return Err(CommandError::new(
                "manual_compaction_indeterminate",
                "manual compaction with this idempotency key is still unresolved",
            ));
        }
        hachimi_storage::IdempotentMutationClaim::Claimed => {}
    }
    let current_model = state.settings.read().llm.clone();
    let mut configuration = snapshot
        .runs
        .iter()
        .max_by_key(|run| run.created_at_ms)
        .map(|run| run.configuration.clone())
        .unwrap_or_else(|| hachimi_protocol::RunConfiguration {
            model_snapshot: current_model.clone(),
            driver: hachimi_protocol::RunDriverKind::ToolLoop,
            entry_profile: snapshot.session.entry_profile,
            workload_override: None,
            behavior_mode: hachimi_protocol::BehaviorMode::Default,
            execution_target: match &snapshot.session.context {
                hachimi_protocol::SessionContextBinding::Project { project_id, .. } => {
                    Some(hachimi_protocol::ExecutionTarget::Local {
                        project_id: project_id.clone(),
                    })
                }
                hachimi_protocol::SessionContextBinding::Workspace { .. } => None,
            },
            approval_policy: hachimi_protocol::ApprovalPolicy::OnlyWhenNeeded,
            permission_profile: hachimi_protocol::PermissionProfile::Writable,
            budget: hachimi_protocol::RunBudget::default(),
            accepted_plan_id: None,
            accepted_plan_revision: None,
        });
    configuration.model_snapshot = current_model;
    let result = state
        .agent_executor
        .compact_session(
            &configuration,
            &request.session_id,
            CancellationToken::new(),
        )
        .await
        .map(|checkpoint| match checkpoint {
            Some(checkpoint) => hachimi_protocol::ManualCompactionResult {
                status: hachimi_protocol::ManualCompactionStatus::Compacted,
                checkpoint_id: Some(checkpoint.id),
                token_snapshot: checkpoint.lifecycle.token_snapshot,
            },
            None => hachimi_protocol::ManualCompactionResult {
                status: hachimi_protocol::ManualCompactionStatus::NoEligibleHistory,
                checkpoint_id: None,
                token_snapshot: None,
            },
        })
        .map_err(|error| CommandError::operation("manual_compaction_failed", error));
    match result {
        Ok(result) => {
            state
                .agent_store
                .complete_idempotent_mutation(
                    &client.client_id.0,
                    "workbench.session.compact",
                    &request.idempotency_key,
                    &result,
                )
                .await
                .map_err(|error| {
                    CommandError::operation("manual_compaction_persist_failed", error)
                })?;
            Ok(result)
        }
        Err(error) => {
            let _ = state
                .agent_store
                .abandon_idempotent_mutation(
                    &client.client_id.0,
                    "workbench.session.compact",
                    &request.idempotency_key,
                )
                .await;
            Err(error)
        }
    }
}

pub(super) fn spawn_workbench_run(
    app: AppHandle,
    client: ClientContext,
    snapshot: WorkbenchTaskSnapshot,
    explicit_skill_ids: Vec<hachimi_protocol::SkillId>,
) {
    spawn_workbench_run_with_recovery(app, client, snapshot, explicit_skill_ids, None);
}

pub(super) fn spawn_workbench_run_with_recovery(
    app: AppHandle,
    client: ClientContext,
    snapshot: WorkbenchTaskSnapshot,
    explicit_skill_ids: Vec<hachimi_protocol::SkillId>,
    recovery_checkpoint: Option<hachimi_protocol::RunStepCheckpoint>,
) {
    tauri::async_runtime::spawn(async move {
        let run_id = snapshot.run.id.clone();
        let finalization_snapshot = snapshot.clone();
        let state = app.state::<DesktopState>();
        let store = state.agent_store.clone();
        let executor = state.agent_executor.clone();
        let sandbox_snapshot = state.sandbox_snapshot();
        let sandbox_report = sandbox_snapshot.report;
        let attachments = store
            .list_run_managed_attachments(&snapshot.run.id)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|attachment| attachment.attachment.id)
            .collect();
        let authority = store
            .authority_snapshot(&snapshot.run.id)
            .await
            .ok()
            .flatten();
        let Some(authority) = authority else {
            tracing::warn!(run_id = %run_id, "workbench Run has no immutable authority snapshot");
            if let Ok(Some(current)) = store.get_run(&run_id).await
                && current.status == hachimi_protocol::RunStatus::Queued
            {
                let _ = store
                    .transition_run(&run_id, hachimi_protocol::RunStatus::Preparing, None)
                    .await;
                let _ = store
                    .transition_run(
                        &run_id,
                        hachimi_protocol::RunStatus::Failed,
                        Some("run_authority_snapshot_missing"),
                    )
                    .await;
            }
            if let Ok(Some(run)) = store.get_run(&run_id).await {
                emit_plan_environment_if_present(&app, &state, &run.session_id).await;
                emit_workbench_run_completion(&app, run);
            }
            return;
        };
        let policy = authority.policy.clone();
        let workspace_root = authority.workspace_root.clone();
        let mut capability_grants = hachimi_policy::expand_permission_policy(
            &policy,
            hachimi_protocol::AuthorityMode::Interactive,
            snapshot.run.configuration.behavior_mode,
            snapshot.session.id.clone(),
            snapshot.run.id.clone(),
            workspace_root,
        );
        if snapshot.run.configuration.behavior_mode != hachimi_protocol::BehaviorMode::Plan {
            capability_grants
                .network
                .protocols
                .extend(["managed-connector".into(), "model-runtime".into()]);
        }
        let capability_grants = interactive_grants::for_workbench_run(
            capability_grants,
            snapshot.run.configuration.behavior_mode,
        );
        let execution = executor
            .execute(hachimi_agent::AgentRunRequest {
                principal: client.client_id.0,
                session: snapshot.session.clone(),
                run: snapshot.run.clone(),
                authority,
                priority: hachimi_agent::AgentRunPriority::Interactive,
                user_input_availability: hachimi_agent::UserInputAvailability::Available,
                capability_grants,
                sandbox_snapshot: sandbox_report,
                attachment_ids: attachments,
                skill_allowlist: explicit_skill_ids,
                mcp_tool_allowlist: Vec::new(),
                run_tool_allowlist: None,
                host_revision_snapshot: None,
                workload_override: snapshot.run.configuration.workload_override,
                recovery_checkpoint,
                parent_agent_task_id: None,
                parent_run_id: None,
                agent_depth: 0,
            })
            .await;
        if let Err(error) = &execution {
            tracing::warn!(run_id = %run_id, %error, "workbench agent run ended with an error");
            if let Ok(Some(current)) = store.get_run(&run_id).await
                && current.status == hachimi_protocol::RunStatus::Queued
            {
                let _ = store
                    .transition_run(&run_id, hachimi_protocol::RunStatus::Preparing, None)
                    .await;
                let _ = store
                    .transition_run(
                        &run_id,
                        hachimi_protocol::RunStatus::Failed,
                        Some("agent_setup_failed"),
                    )
                    .await;
            }
        }
        if execution.is_ok()
            && let Err(error) = finalize_review_run(&store, &finalization_snapshot).await
        {
            tracing::warn!(run_id = %run_id, %error, "failed to persist structured Review output");
        }
        if let Ok(Some(run)) = store.get_run(&run_id).await {
            emit_plan_environment_if_present(&app, &state, &run.session_id).await;
            emit_workbench_run_completion(&app, run);
        }
    });
}

#[cfg(all(debug_assertions, feature = "desktop-e2e"))]
#[derive(Debug)]
pub(super) struct DesktopE2eModel;

#[cfg(all(debug_assertions, feature = "desktop-e2e"))]
impl hachimi_agent::ModelRuntime for DesktopE2eModel {
    fn capabilities(&self) -> hachimi_protocol::ProviderCapabilities {
        hachimi_protocol::ProviderCapabilities {
            tool_calls: true,
            strict_json_schema: true,
            text_input: true,
            streaming_usage: true,
            context_window: Some(128_000),
            max_output_tokens: Some(4_096),
            ..hachimi_protocol::ProviderCapabilities::default()
        }
    }

    fn stream(
        &self,
        request: hachimi_protocol::ModelRequest,
        cancellation: CancellationToken,
    ) -> hachimi_agent::ModelEventStream {
        use futures_util::stream;
        use hachimi_protocol::{
            ModelEvent, ModelFinishReason, ModelRole, ModelToolCall, TokenUsage, ToolCallId,
        };

        if cancellation.is_cancelled() {
            return Box::pin(stream::iter([Err(ModelRuntimeError::Cancelled)]));
        }
        if crate::desktop_e2e_agent_tools::waits_for_process_restart(&request) {
            return Box::pin(stream::once(async move {
                cancellation.cancelled().await;
                Err(ModelRuntimeError::Cancelled)
            }));
        }
        if let Some(response) = crate::desktop_e2e_agent_tools::response(&request) {
            return Box::pin(stream::iter(response));
        }
        let plan_mode = request.messages.iter().any(|message| {
            message.role == ModelRole::System && message.content.contains("mode=Plan")
        });
        let review_mode = request.messages.iter().any(|message| {
            message.role == ModelRole::System
                && message
                    .content
                    .contains("Review mode is isolated and read-only")
        });
        let completed_tools = request
            .messages
            .iter()
            .filter(|message| message.role == ModelRole::Tool)
            .filter_map(|message| message.name.as_deref())
            .collect::<Vec<_>>();
        let scheduled_success = request.messages.iter().any(|message| {
            message.role == ModelRole::User
                && message.content.contains("[desktop-e2e:schedule-success]")
        });
        let scheduled_hosts = request.messages.iter().any(|message| {
            message.role == ModelRole::User
                && message.content.contains("[desktop-e2e:schedule-hosts]")
        });
        let scheduled_wait = request.messages.iter().any(|message| {
            message.role == ModelRole::User
                && message.content.contains("[desktop-e2e:schedule-wait]")
        });
        let office_workflow = request.messages.iter().any(|message| {
            message.role == ModelRole::User
                && message.content.contains("[desktop-e2e:office-skills]")
        });
        let office_interruption = request.messages.iter().any(|message| {
            message.role == ModelRole::User
                && message
                    .content
                    .contains("[desktop-e2e:office-interruption]")
        });
        let implicit_office_workflow = request.messages.iter().any(|message| {
            message.role == ModelRole::User
                && message
                    .content
                    .contains("[desktop-e2e:office-implicit-recovery]")
        });
        let pet_cross_window = request.messages.iter().any(|message| {
            message.role == ModelRole::User
                && message.content.contains("[desktop-e2e:pet-cross-window]")
        });
        if scheduled_wait {
            return Box::pin(stream::once(async move {
                cancellation.cancelled().await;
                Err(ModelRuntimeError::Cancelled)
            }));
        }
        let response = if pet_cross_window && !completed_tools.contains(&"workspace_write_file") {
            tool_call_events(ModelToolCall {
                id: ToolCallId::from("desktop-e2e-pet-workspace-write"),
                name: "workspace_write_file".into(),
                arguments: serde_json::json!({
                    "path": "pet-cross-window-evidence.txt",
                    "content": "Pet unified Agent workspace evidence\n",
                    "expectedSha256": null
                }),
            })
        } else if pet_cross_window {
            desktop_e2e_text_response(
                "Pet writable Workspace action completed on one Agent Run without interactive Plan tools.",
            )
        } else if implicit_office_workflow && !completed_tools.contains(&"skills.list") {
            tool_call_events(ModelToolCall {
                id: ToolCallId::from("desktop-e2e-skills-list"),
                name: "skills.list".into(),
                arguments: serde_json::json!({"cursor": 0, "limit": 50}),
            })
        } else if implicit_office_workflow && !completed_tools.contains(&"skills.read") {
            match desktop_e2e_skill_id(&request.messages, "office-documents") {
                Some(skill_id) => tool_call_events(ModelToolCall {
                    id: ToolCallId::from("desktop-e2e-skills-read"),
                    name: "skills.read".into(),
                    arguments: serde_json::json!({
                        "skillId": skill_id,
                        "path": "SKILL.md",
                        "startLine": 1,
                        "lineLimit": 300
                    }),
                }),
                None => desktop_e2e_text_response(
                    "Implicit Office Skill discovery failed before activation.",
                ),
            }
        } else if implicit_office_workflow {
            let skill_results = request
                .messages
                .iter()
                .filter(|message| {
                    message.role == ModelRole::Tool
                        && message.name.as_deref() == Some("skills.read")
                })
                .collect::<Vec<_>>();
            let skill_id = desktop_e2e_skill_id(&request.messages, "office-documents");
            if skill_results.len() == 1 {
                match skill_id {
                    Some(skill_id) => tool_call_events(ModelToolCall {
                        id: ToolCallId::from("desktop-e2e-skill-resource-fail"),
                        name: "skills.read".into(),
                        arguments: serde_json::json!({
                            "skillId": skill_id,
                            "path": "references/missing-validation.md",
                            "startLine": 1,
                            "lineLimit": 100
                        }),
                    }),
                    None => desktop_e2e_text_response(
                        "Implicit Office Skill activation lost its stable Skill identity.",
                    ),
                }
            } else if skill_results.iter().any(|message| {
                message
                    .content
                    .contains("SkillHost rejected the resource read")
            }) && !skill_results
                .iter()
                .any(|message| message.content.contains("# Document validation"))
            {
                match skill_id {
                    Some(skill_id) => tool_call_events(ModelToolCall {
                        id: ToolCallId::from("desktop-e2e-skill-resource-retry"),
                        name: "skills.read".into(),
                        arguments: serde_json::json!({
                            "skillId": skill_id,
                            "path": "references/validation.md",
                            "startLine": 1,
                            "lineLimit": 300
                        }),
                    }),
                    None => desktop_e2e_text_response(
                        "Implicit Office Skill recovery lost its stable Skill identity.",
                    ),
                }
            } else if skill_results
                .iter()
                .any(|message| message.content.contains("# Document validation"))
                && request.messages.iter().any(|message| {
                    message.role == ModelRole::System && message.content.contains("workload=Office")
                })
            {
                desktop_e2e_text_response(
                    "Implicit Office Skill activated the Office overlay and recovered from a missing reference by loading the bounded document validation guidance.",
                )
            } else {
                desktop_e2e_text_response(
                    "Implicit Office Skill recovery did not reach a validated bounded resource.",
                )
            }
        } else if review_mode && !completed_tools.contains(&"workspace_review_diff") {
            tool_call_events(ModelToolCall {
                id: ToolCallId::from("desktop-e2e-review-diff"),
                name: "workspace_review_diff".into(),
                arguments: serde_json::json!({}),
            })
        } else if review_mode {
            vec![
                Ok(ModelEvent::TextDelta {
                    delta: serde_json::json!({
                        "findings": [{
                            "title": "Deterministic review finding",
                            "body": "The fixture change is projected as structured read-only Review evidence.",
                            "confidenceScore": 0.98,
                            "priority": 2,
                            "codeLocation": {
                                "filePath": "desktop-e2e-evidence.txt",
                                "lineRange": { "start": 1, "end": 1 }
                            }
                        }],
                        "overallCorrectness": "incorrect",
                        "overallExplanation": "The deterministic Review completed without write, Exec, Approval, UserInput, Skill, or MCP capability.",
                        "overallConfidenceScore": 0.98
                    })
                    .to_string(),
                }),
                Ok(ModelEvent::Usage {
                    usage: TokenUsage {
                        input_tokens: 72,
                        output_tokens: 48,
                    },
                }),
                Ok(ModelEvent::Completed {
                    finish_reason: ModelFinishReason::Stop,
                }),
            ]
        } else if office_interruption {
            match request
                .tools
                .iter()
                .find(|tool| tool.name.contains("_create_document_"))
                .filter(|tool| !completed_tools.contains(&tool.name.as_str()))
            {
                Some(tool) => tool_call_events(ModelToolCall {
                    id: ToolCallId::from("desktop-e2e-office-interruption"),
                    name: tool.name.clone(),
                    arguments: serde_json::json!({
                        "title": "Hachimi Office interruption recovery",
                        "body": "interrupt-before-receipt"
                    }),
                }),
                None => desktop_e2e_text_response(
                    "Interrupted Office stdio MCP recovered without replaying an acknowledged result.",
                ),
            }
        } else if office_workflow {
            let next = [
                "create_document",
                "create_spreadsheet",
                "create_presentation",
                "create_pdf",
                "inspect_artifact",
                "modify_artifact",
                "diff_artifact",
                "export_artifact",
                "preview_file_plan",
                "send_artifact",
            ]
            .into_iter()
            .find_map(|original_name| {
                request
                    .tools
                    .iter()
                    .find(|tool| tool.name.contains(&format!("_{original_name}_")))
                    .filter(|tool| !completed_tools.contains(&tool.name.as_str()))
                    .map(|tool| (original_name, tool.name.clone()))
            });
            if let Some((original_name, exposed_name)) = next {
                let arguments = match original_name {
                    "send_artifact" => serde_json::json!({
                        "artifactId": "desktop-e2e-create_pdf",
                        "target": "team@example.invalid"
                    }),
                    "preview_file_plan" => serde_json::json!({
                        "root": "desktop-e2e-authorized-root",
                        "actions": ["preview rename duplicate.txt -> duplicate (1).txt"],
                    }),
                    "inspect_artifact" | "diff_artifact" => serde_json::json!({
                        "artifactId": "desktop-e2e-create_document"
                    }),
                    "modify_artifact" => serde_json::json!({
                        "artifactId": "desktop-e2e-create_document",
                        "body": "Revised and revalidated Office document"
                    }),
                    "export_artifact" => serde_json::json!({
                        "artifactId": "desktop-e2e-create_pdf",
                        "format": "pdf"
                    }),
                    _ => serde_json::json!({
                        "title": "Hachimi Office E2E",
                        "body": format!("Validated {original_name} artifact")
                    }),
                };
                tool_call_events(ModelToolCall {
                    id: ToolCallId::new(format!("desktop-e2e-{original_name}")),
                    name: exposed_name,
                    arguments,
                })
            } else {
                vec![
                    Ok(ModelEvent::TextDelta {
                        delta: "Office extension workflow created, inspected, modified, diffed, exported and validated four artifacts, then completed the persisted authorized delivery."
                            .into(),
                    }),
                    Ok(ModelEvent::Usage {
                        usage: TokenUsage {
                            input_tokens: 128,
                            output_tokens: 24,
                        },
                    }),
                    Ok(ModelEvent::Completed {
                        finish_reason: ModelFinishReason::Stop,
                    }),
                ]
            }
        } else if scheduled_hosts && !completed_tools.contains(&"connector_list_accounts") {
            tool_call_events(ModelToolCall {
                id: ToolCallId::from("desktop-e2e-schedule-connector-list"),
                name: "connector_list_accounts".into(),
                arguments: serde_json::json!({}),
            })
        } else if scheduled_hosts && !completed_tools.contains(&"connector_invoke") {
            match desktop_e2e_tool_json(&request.messages, "connector_list_accounts").and_then(
                |value| {
                    value
                        .as_array()
                        .and_then(|accounts| accounts.first())
                        .cloned()
                },
            ) {
                Some(account) => {
                    let account_id = account.get("id").and_then(serde_json::Value::as_str);
                    let revision = account.get("revision");
                    match (account_id, revision) {
                        (Some(account_id), Some(revision)) => tool_call_events(ModelToolCall {
                            id: ToolCallId::from("desktop-e2e-schedule-connector-search"),
                            name: "connector_invoke".into(),
                            arguments: serde_json::json!({
                                "accountId": account_id,
                                "action": "search",
                                "arguments": { "query": "" },
                                "idempotencyKey": "desktop-e2e-schedule-hosts-search-v1",
                                "expectedRevision": revision
                            }),
                        }),
                        _ => desktop_e2e_model_failure(
                            "Connector account metadata omitted its ID or pinned revision",
                        ),
                    }
                }
                None => desktop_e2e_model_failure(
                    "Scheduled Host E2E could not decode a Connector account",
                ),
            }
        } else if scheduled_hosts && !completed_tools.contains(&"browser_start") {
            match std::env::var("HACHIMI_DESKTOP_E2E_BROWSER_URL") {
                Ok(initial_url) => tool_call_events(ModelToolCall {
                    id: ToolCallId::from("desktop-e2e-schedule-browser-start"),
                    name: "browser_start".into(),
                    arguments: serde_json::json!({
                        "initialUrl": initial_url,
                        "surface": "embedded"
                    }),
                }),
                Err(_) => desktop_e2e_model_failure("HACHIMI_DESKTOP_E2E_BROWSER_URL is missing"),
            }
        } else if scheduled_hosts
            && (!completed_tools.contains(&"browser_observe")
                || !desktop_e2e_tool_json(&request.messages, "browser_observe").is_some_and(
                    |observation| {
                        observation
                            .get("text")
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(|text| text.contains("Managed Browser scheduled fixture"))
                    },
                ))
        {
            match desktop_e2e_tool_json(&request.messages, "browser_start")
                .and_then(|value| value.pointer("/lease/id").cloned())
            {
                Some(lease_id) => {
                    let attempt = request
                        .messages
                        .iter()
                        .filter(|message| {
                            message.role == ModelRole::Tool
                                && message.name.as_deref() == Some("browser_observe")
                        })
                        .count()
                        + 1;
                    tool_call_events(ModelToolCall {
                        id: ToolCallId::new(format!(
                            "desktop-e2e-schedule-browser-observe-{attempt}"
                        )),
                        name: "browser_observe".into(),
                        arguments: serde_json::json!({
                            "leaseId": lease_id
                        }),
                    })
                }
                None => desktop_e2e_model_failure(
                    "Scheduled Host E2E Browser start did not return an automation lease",
                ),
            }
        } else if scheduled_hosts && !completed_tools.contains(&"browser_act") {
            match desktop_e2e_tool_json(&request.messages, "browser_observe") {
                Some(observation) => {
                    let lease_id = observation.get("leaseId");
                    let observation_id = observation.get("observationId");
                    let tab_revision = observation.get("tabRevision");
                    let input_epoch = observation.get("inputEpoch");
                    match (lease_id, observation_id, tab_revision, input_epoch) {
                        (
                            Some(lease_id),
                            Some(observation_id),
                            Some(tab_revision),
                            Some(input_epoch),
                        ) => tool_call_events(ModelToolCall {
                            id: ToolCallId::from("desktop-e2e-schedule-browser-click"),
                            name: "browser_act".into(),
                            arguments: serde_json::json!({
                                "leaseId": lease_id,
                                "observationId": observation_id,
                                "expectedTabRevision": tab_revision,
                                "expectedInputEpoch": input_epoch,
                                "action": {
                                    "kind": "click",
                                    "selector": "h1"
                                }
                            }),
                        }),
                        _ => desktop_e2e_model_failure(
                            "Browser observation omitted its fencing identifiers",
                        ),
                    }
                }
                None => desktop_e2e_model_failure(
                    "Scheduled Host E2E could not decode the Browser observation",
                ),
            }
        } else if scheduled_hosts && !completed_tools.contains(&"browser_stop") {
            match desktop_e2e_tool_json(&request.messages, "browser_start")
                .and_then(|value| value.pointer("/lease/id").cloned())
            {
                Some(lease_id) => tool_call_events(ModelToolCall {
                    id: ToolCallId::from("desktop-e2e-schedule-browser-stop"),
                    name: "browser_stop".into(),
                    arguments: serde_json::json!({
                        "leaseId": lease_id
                    }),
                }),
                None => desktop_e2e_model_failure(
                    "Scheduled Host E2E lost Browser lease ownership before Stop",
                ),
            }
        } else if scheduled_hosts {
            let connector_ok = desktop_e2e_tool_json(&request.messages, "connector_invoke")
                .and_then(|value| {
                    value
                        .get("action")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .as_deref()
                == Some("search");
            let observation_ok = desktop_e2e_tool_json(&request.messages, "browser_observe")
                .and_then(|value| {
                    value
                        .get("text")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .is_some_and(|text| text.contains("Managed Browser scheduled fixture"));
            let action_ok = desktop_e2e_tool_json(&request.messages, "browser_act")
                .and_then(|value| {
                    value
                        .get("resultCode")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .as_deref()
                == Some("performed");
            if connector_ok && observation_ok && action_ok {
                desktop_e2e_text_response(
                    "Scheduled Host E2E completed sample-crm search, embedded Browser Observe and Act, and lease Stop on one fresh Run.",
                )
            } else {
                desktop_e2e_model_failure(
                    "Scheduled Host E2E did not produce verified Connector, Observe, and Act results",
                )
            }
        } else if scheduled_success {
            vec![
                Ok(ModelEvent::TextDelta {
                    delta: "Scheduled Desktop E2E task completed.".into(),
                }),
                Ok(ModelEvent::Usage {
                    usage: TokenUsage {
                        input_tokens: 32,
                        output_tokens: 8,
                    },
                }),
                Ok(ModelEvent::Completed {
                    finish_reason: ModelFinishReason::Stop,
                }),
            ]
        } else if plan_mode && !completed_tools.contains(&"request_user_input") {
            tool_call_events(ModelToolCall {
                id: ToolCallId::from("desktop-e2e-plan-input"),
                name: "request_user_input".into(),
                arguments: serde_json::json!({
                    "questions": [{
                        "id": "verification_label",
                        "header": "Verification",
                        "question": "Choose the deterministic verification strategy.",
                        "options": [
                            {
                                "label": "Git status",
                                "description": "Verify the generated evidence file with Git status."
                            },
                            {
                                "label": "File tree",
                                "description": "Verify the generated evidence file in the workspace tree."
                            }
                        ]
                    }]
                }),
            })
        } else if plan_mode {
            vec![
                Ok(ModelEvent::TextDelta {
                    delta: "I have the verification choice. I will now finalize the plan.\n\n<proposed_"
                        .into(),
                }),
                Ok(ModelEvent::TextDelta {
                    delta: "plan>\n# Create deterministic Desktop E2E evidence\n\n## Goal\nCreate the deterministic E2E evidence file.\n\n## Steps\n1. Write the file.\n2. Run Git status to verify the change.\n</proposed_plan>"
                        .into(),
                }),
                Ok(ModelEvent::Usage {
                    usage: TokenUsage {
                        input_tokens: 64,
                        output_tokens: 32,
                    },
                }),
                Ok(ModelEvent::Completed {
                    finish_reason: ModelFinishReason::Stop,
                }),
            ]
        } else if !completed_tools.contains(&"workspace_write_file") {
            tool_call_events(ModelToolCall {
                id: ToolCallId::from("desktop-e2e-write"),
                name: "workspace_write_file".into(),
                arguments: serde_json::json!({
                    "path": "desktop-e2e-evidence.txt",
                    "content": "Hachimi Desktop E2E evidence\n",
                    "expectedSha256": null
                }),
            })
        } else if !completed_tools.contains(&"workspace_exec") {
            tool_call_events(ModelToolCall {
                id: ToolCallId::from("desktop-e2e-exec"),
                name: "workspace_exec".into(),
                arguments: serde_json::json!({
                    "program": "git",
                    "args": ["status", "--short"],
                    "cwd": "",
                    "timeoutMs": 30000
                }),
            })
        } else {
            vec![
                Ok(ModelEvent::TextDelta {
                    delta:
                        "Desktop E2E task completed with file, user-input, and command evidence."
                            .into(),
                }),
                Ok(ModelEvent::Usage {
                    usage: TokenUsage {
                        input_tokens: 96,
                        output_tokens: 16,
                    },
                }),
                Ok(ModelEvent::Completed {
                    finish_reason: ModelFinishReason::Stop,
                }),
            ]
        };
        Box::pin(stream::iter(response))
    }
}

#[cfg(all(debug_assertions, feature = "desktop-e2e"))]
fn tool_call_events(
    call: hachimi_protocol::ModelToolCall,
) -> Vec<Result<hachimi_protocol::ModelEvent, ModelRuntimeError>> {
    vec![
        Ok(hachimi_protocol::ModelEvent::ToolCallCompleted { call }),
        Ok(hachimi_protocol::ModelEvent::Completed {
            finish_reason: hachimi_protocol::ModelFinishReason::ToolCalls,
        }),
    ]
}

#[cfg(all(debug_assertions, feature = "desktop-e2e"))]
fn desktop_e2e_text_response(
    text: &str,
) -> Vec<Result<hachimi_protocol::ModelEvent, ModelRuntimeError>> {
    vec![
        Ok(hachimi_protocol::ModelEvent::TextDelta { delta: text.into() }),
        Ok(hachimi_protocol::ModelEvent::Usage {
            usage: hachimi_protocol::TokenUsage {
                input_tokens: 48,
                output_tokens: 16,
            },
        }),
        Ok(hachimi_protocol::ModelEvent::Completed {
            finish_reason: hachimi_protocol::ModelFinishReason::Stop,
        }),
    ]
}

#[cfg(all(debug_assertions, feature = "desktop-e2e"))]
fn desktop_e2e_model_failure(
    message: &str,
) -> Vec<Result<hachimi_protocol::ModelEvent, ModelRuntimeError>> {
    vec![Err(ModelRuntimeError::Provider(message.to_owned()))]
}

#[cfg(all(debug_assertions, feature = "desktop-e2e"))]
fn desktop_e2e_tool_json(
    messages: &[hachimi_protocol::ModelMessage],
    tool_name: &str,
) -> Option<serde_json::Value> {
    messages
        .iter()
        .rev()
        .find(|message| {
            message.role == hachimi_protocol::ModelRole::Tool
                && message.name.as_deref() == Some(tool_name)
        })
        .and_then(|message| desktop_e2e_parse_tool_json(&message.content))
}

#[cfg(all(debug_assertions, feature = "desktop-e2e"))]
fn desktop_e2e_parse_tool_json(content: &str) -> Option<serde_json::Value> {
    serde_json::from_str(content).ok().or_else(|| {
        let (_, payload) = content.split_once('\n')?;
        serde_json::from_str(payload).ok()
    })
}

#[cfg(all(debug_assertions, feature = "desktop-e2e"))]
fn desktop_e2e_skill_id(
    messages: &[hachimi_protocol::ModelMessage],
    qualified_name: &str,
) -> Option<String> {
    let value = messages
        .iter()
        .rev()
        .filter(|message| {
            message.role == hachimi_protocol::ModelRole::Tool
                && message.name.as_deref() == Some("skills.list")
        })
        .find_map(|message| desktop_e2e_parse_tool_json(&message.content))?;
    value
        .as_array()
        .or_else(|| value.get("skills").and_then(serde_json::Value::as_array))?
        .iter()
        .find(|record| {
            record.get("name").and_then(serde_json::Value::as_str) == Some(qualified_name)
        })?
        .get("id")?
        .as_str()
        .map(str::to_owned)
}

#[derive(Debug, Clone)]
pub(super) struct DesktopModelRuntimeFactory {
    store: AgentStore,
    runtime_features: hachimi_core::RuntimeFeatureSet,
}

impl DesktopModelRuntimeFactory {
    pub(super) fn new(
        store: AgentStore,
        runtime_features: hachimi_core::RuntimeFeatureSet,
    ) -> Self {
        Self {
            store,
            runtime_features,
        }
    }
}

pub(super) fn provider_settings_for_runtime(
    mut settings: hachimi_protocol::LlmSettings,
    features: hachimi_core::RuntimeFeatureSet,
) -> hachimi_protocol::LlmSettings {
    if !features.provider_extensions {
        settings.protocol = hachimi_protocol::ProviderProtocolKind::ChatCompletions;
        settings.compatibility_profile_id = "openai-strict".into();
        settings.provider_endpoint_id = None;
        settings.provider_account_id = None;
        settings.embedding_model_name.clear();
        settings.reasoning_summary = hachimi_protocol::ReasoningSummaryMode::None;
        settings.remote_compaction = false;
    } else if !features.provider_remote_context {
        settings.reasoning_summary = hachimi_protocol::ReasoningSummaryMode::None;
        settings.remote_compaction = false;
    }
    settings
}

impl hachimi_agent::ModelRuntimeFactory for DesktopModelRuntimeFactory {
    fn create_session(
        &self,
        configuration: &hachimi_protocol::RunConfiguration,
    ) -> hachimi_agent::ModelClientFuture {
        let configuration = configuration.clone();
        let store = self.store.clone();
        let runtime_features = self.runtime_features;
        Box::pin(async move {
            if deterministic_e2e_provider_enabled() {
                #[cfg(all(debug_assertions, feature = "desktop-e2e"))]
                {
                    return Ok(
                        Arc::new(DesktopE2eModel) as Arc<dyn hachimi_agent::ModelClientSession>
                    );
                }
                #[cfg(not(all(debug_assertions, feature = "desktop-e2e")))]
                unreachable!("desktop E2E provider cannot be enabled in this build")
            }
            let settings =
                provider_settings_for_runtime(configuration.model_snapshot, runtime_features);
            let api_key = hachimi_llm::SystemApiKeyStore
                .get()
                .map_err(|error| hachimi_agent::ModelRuntimeError::Provider(error.to_string()))?;
            let structured_probe = hachimi_llm::resolve_structured_output_capabilities(
                &settings,
                api_key.as_deref(),
                false,
            )
            .await;
            let mut runtime = hachimi_llm::OpenAiCompatibleRuntime::tool_calling_with_probe(
                settings.clone(),
                api_key,
                &structured_probe,
            )
            .map_err(|error| hachimi_agent::ModelRuntimeError::Provider(error.to_string()))?;
            let proposed = runtime.capabilities();
            let verified = if runtime_features.provider_extensions
                && let Some(endpoint_id) = settings.provider_endpoint_id.as_ref()
            {
                store
                    .latest_provider_probe(endpoint_id)
                    .await
                    .map_err(|error| hachimi_agent::ModelRuntimeError::Provider(error.to_string()))?
                    .filter(|report| {
                        report.status == hachimi_protocol::ProviderProbeStatus::Succeeded
                            && report.account_id == settings.provider_account_id
                            && report.capability_revision
                                == provider_runtime_capability_revision(&settings, &proposed)
                    })
            } else {
                None
            };
            runtime.apply_verified_optional_capabilities(
                runtime_features.provider_remote_context
                    && verified
                        .as_ref()
                        .is_some_and(|report| report.capabilities.reasoning_summary),
                runtime_features.provider_remote_context
                    && verified
                        .as_ref()
                        .is_some_and(|report| report.capabilities.remote_compaction),
                runtime_features.provider_extensions
                    && verified
                        .as_ref()
                        .is_some_and(|report| report.capabilities.embeddings),
            );
            Ok(Arc::new(runtime) as Arc<dyn hachimi_agent::ModelClientSession>)
        })
    }
}

fn provider_runtime_capability_revision(
    settings: &hachimi_protocol::LlmSettings,
    capabilities: &hachimi_protocol::ProviderCapabilities,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(&(settings, capabilities)).unwrap_or_default());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

static MANAGED_WORKSPACE_WORKER: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
static MANAGED_SANDBOX_RUNTIME_ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

pub(super) fn set_managed_workspace_runtime(
    runtime_root: PathBuf,
    worker: PathBuf,
) -> Result<(), String> {
    if !runtime_root.is_absolute() || !runtime_root.is_dir() || !worker.is_file() {
        return Err("managed Workspace Worker path is invalid".into());
    }
    if let (Some(existing_root), Some(existing_worker)) = (
        MANAGED_SANDBOX_RUNTIME_ROOT.get(),
        MANAGED_WORKSPACE_WORKER.get(),
    ) {
        return if existing_root == &runtime_root && existing_worker == &worker {
            Ok(())
        } else {
            Err("managed Workspace Runtime was already initialized to another path".into())
        };
    }
    MANAGED_SANDBOX_RUNTIME_ROOT
        .set(runtime_root)
        .map_err(|_| "managed Sandbox Runtime root was already initialized".to_owned())?;
    MANAGED_WORKSPACE_WORKER
        .set(worker)
        .map_err(|_| "managed Workspace Worker path was already initialized".into())
}

pub(super) fn workspace_worker_path() -> PathBuf {
    if let Some(path) = MANAGED_WORKSPACE_WORKER.get() {
        return path.clone();
    }
    if let Some(path) = std::env::var_os("HACHIMI_WORKSPACE_WORKER_PATH") {
        return PathBuf::from(path);
    }
    let executable_name = if cfg!(windows) {
        "hachimi-workspace-worker.exe"
    } else {
        "hachimi-workspace-worker"
    };
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join(executable_name)))
        .unwrap_or_else(|| PathBuf::from(executable_name))
}

pub(super) fn sandbox_sidecar_path(name: &str) -> PathBuf {
    let executable_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    MANAGED_SANDBOX_RUNTIME_ROOT
        .get()
        .map(|root| root.join(&executable_name))
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|path| path.parent().map(|parent| parent.join(&executable_name)))
        })
        .unwrap_or_else(|| PathBuf::from(name))
}

#[tauri::command]
pub(super) async fn cancel_workbench_run(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    run_id: RunId,
    expected_generation: u64,
) -> Result<RunRecord, CommandError> {
    state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    let store = state.workbench.store();
    let expected = store
        .assert_run_precondition(&run_id, &run_id, expected_generation)
        .await
        .map_err(|error| CommandError::operation("workbench_run_precondition_failed", error))?;
    if let Some(active) = state.agent_executor.registry().get(&run_id) {
        if active.run_id != run_id
            || active.session_id != expected.session_id
            || active.run_generation != expected_generation
        {
            return Err(CommandError::new(
                "workbench_run_precondition_failed",
                "the active Run or generation changed",
            ));
        }
        state
            .agent_executor
            .registry()
            .cancel(&run_id, expected_generation)
            .map_err(|error| CommandError::operation("workbench_run_precondition_failed", error))?;
    }
    state
        .approval_broker
        .cancel_run(run_id.clone())
        .await
        .map_err(|error| CommandError::operation("workbench_approval_cancel_failed", error))?;
    state
        .user_input_broker
        .cancel_run(run_id.clone())
        .await
        .map_err(|error| CommandError::operation("workbench_user_input_cancel_failed", error))?;
    for _ in 0..3 {
        let current = store
            .get_run(&run_id)
            .await
            .map_err(|error| CommandError::operation("workbench_run_get_failed", error))?
            .ok_or_else(|| CommandError::new("workbench_run_not_found", "run does not exist"))?;
        let next = match current.status {
            hachimi_protocol::RunStatus::Queued | hachimi_protocol::RunStatus::Preparing => {
                hachimi_protocol::RunStatus::Cancelled
            }
            hachimi_protocol::RunStatus::Running
            | hachimi_protocol::RunStatus::WaitingApproval
            | hachimi_protocol::RunStatus::WaitingUserInput => {
                hachimi_protocol::RunStatus::Cancelling
            }
            _ => return Ok(current),
        };
        match store.transition_run(&run_id, next, None).await {
            Ok(updated) => return Ok(updated),
            Err(hachimi_storage::AgentStoreError::InvalidRunTransition { .. }) => {
                tokio::task::yield_now().await;
            }
            Err(error) => {
                return Err(CommandError::operation(
                    "workbench_run_cancel_failed",
                    error,
                ));
            }
        }
    }
    store
        .get_run(&run_id)
        .await
        .map_err(|error| CommandError::operation("workbench_run_get_failed", error))?
        .ok_or_else(|| CommandError::new("workbench_run_not_found", "run does not exist"))
}
