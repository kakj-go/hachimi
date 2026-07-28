//! Desktop adapter for persistent prompt schedules and background Agent Runs.

use std::{path::PathBuf, sync::Arc, time::Duration};

use hachimi_agent::{AgentRunCreateRequest, AgentRunFactory, AgentRunPriority};
use hachimi_policy::expand_permission_profile;
use hachimi_protocol::{
    ApprovalPolicy, BehaviorMode, CheckoutKind, ExecutionTarget, ItemPayload, LlmSettings,
    MutationContext, PermissionProfile, ProviderCapabilities, RunBudget, RunOrigin, RunPurpose,
    RunRecord, RunStatus, ScheduleAuthorizationScope, ScheduleContextTemplate,
    ScheduleCreateRequest, ScheduleDefinition, ScheduleGrantRecord, ScheduleId, SchedulePreview,
    ScheduleSkillSelection, ScheduleSnapshot, ScheduleSpec, ScheduleUpdateRequest,
    SessionContextBinding, SkillDiagnosticSeverity, TaskInteractiveContinuation, TaskRunId,
    TaskRunRecord, TaskRunStatus, TranscriptItemKind, WorkbenchTaskSnapshot,
};
use hachimi_scheduler::{
    NotificationAdapter, NotificationFuture, ScheduleLaunchError, ScheduleLaunchFuture,
    ScheduleRunCompletion, ScheduleRunLauncher, TaskNotification,
};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_notification::NotificationExt;
use tokio_util::sync::CancellationToken;

use super::{CommandError, ControlMethod, DesktopState, epoch_millis, require_window};

pub(super) const TASK_NOTIFICATION_EVENT: &str = "workbench-task-notification";

pub(super) fn start_desktop_scheduler(
    app: &AppHandle,
    scheduler: Arc<hachimi_scheduler::SchedulerService>,
    enabled: bool,
) {
    if !enabled {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = scheduler.reconcile_startup().await {
            tracing::warn!(%error, "Scheduler startup reconciliation failed");
        }
        *app.state::<DesktopState>().scheduler_handle.lock() = Some(scheduler.start());
    });
}

#[derive(Clone)]
pub(super) struct DesktopScheduleRunLauncher {
    app: AppHandle,
}

impl DesktopScheduleRunLauncher {
    #[must_use]
    pub(super) fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl ScheduleRunLauncher for DesktopScheduleRunLauncher {
    fn launch(
        &self,
        schedule: ScheduleDefinition,
        task_run: TaskRunRecord,
        cancellation: CancellationToken,
    ) -> ScheduleLaunchFuture {
        let app = self.app.clone();
        Box::pin(
            async move { launch_scheduled_agent_run(app, schedule, task_run, cancellation).await },
        )
    }
}

#[derive(Clone)]
pub(super) struct DesktopTaskNotificationAdapter {
    app: AppHandle,
}

impl DesktopTaskNotificationAdapter {
    #[must_use]
    pub(super) fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl NotificationAdapter for DesktopTaskNotificationAdapter {
    fn deliver(&self, notification: TaskNotification) -> NotificationFuture {
        let app = self.app.clone();
        Box::pin(async move {
            let title = format!("Hachimi · {}", notification.task_name);
            let body = task_notification_status(notification.status);
            app.notification()
                .builder()
                .title(title)
                .body(body)
                .show()
                .map_err(|error| format!("system_notification_failed:{error}"))?;
            let _ = app.emit(TASK_NOTIFICATION_EVENT, &notification.task_run_id);
            Ok(())
        })
    }
}

fn task_notification_status(status: hachimi_protocol::TaskRunStatus) -> &'static str {
    use hachimi_protocol::TaskRunStatus;

    match status {
        TaskRunStatus::NeedsAttention => "需要处理",
        TaskRunStatus::Succeeded => "已完成",
        TaskRunStatus::Failed => "执行失败",
        TaskRunStatus::TimedOut => "执行超时",
        TaskRunStatus::Cancelled => "已取消",
        TaskRunStatus::Lost => "执行中断",
        TaskRunStatus::Skipped => "已跳过",
        TaskRunStatus::Queued | TaskRunStatus::Preparing | TaskRunStatus::Running => "状态已更新",
    }
}

fn authorize(
    window: &WebviewWindow,
    state: &DesktopState,
) -> Result<hachimi_protocol::ClientContext, CommandError> {
    if !state.control_plane.feature_flags().scheduler {
        return Err(CommandError::new(
            "scheduler_disabled",
            "the persistent task scheduler is disabled in this build",
        ));
    }
    let client = state.authorize(window, ControlMethod::WorkbenchWindow)?;
    require_window(window, "workbench")?;
    Ok(client)
}

fn app_context(client: hachimi_protocol::ClientContext) -> hachimi_control_plane::AppServerContext {
    hachimi_control_plane::AppServerContext {
        principal: client.client_id.0.clone(),
        client,
    }
}

async fn dispatch_schedule(
    state: &DesktopState,
    context: &hachimi_control_plane::AppServerContext,
    request: hachimi_control_plane::ScheduleAppRequest,
) -> Result<hachimi_control_plane::ScheduleAppResponse, CommandError> {
    match state
        .app_server
        .dispatch(
            context,
            hachimi_control_plane::AppServerRequest::Domain(Box::new(
                hachimi_control_plane::AppServerDomainRequest::Schedule(Box::new(request)),
            )),
        )
        .await
        .map_err(|error| CommandError::operation("schedule_app_server_failed", error))?
    {
        hachimi_control_plane::AppServerResponse::Domain(response) => match *response {
            hachimi_control_plane::AppServerDomainResponse::Schedule(response) => Ok(response),
            _ => Err(CommandError::new(
                "schedule_app_server_protocol_mismatch",
                "App Server returned a response for a different domain",
            )),
        },
        _ => Err(CommandError::new(
            "schedule_app_server_protocol_mismatch",
            "App Server returned a response for a different domain",
        )),
    }
}

async fn dispatch_task(
    state: &DesktopState,
    context: &hachimi_control_plane::AppServerContext,
    request: hachimi_control_plane::TaskAppRequest,
) -> Result<hachimi_control_plane::TaskAppResponse, CommandError> {
    match state
        .app_server
        .dispatch(
            context,
            hachimi_control_plane::AppServerRequest::Domain(Box::new(
                hachimi_control_plane::AppServerDomainRequest::Task(request),
            )),
        )
        .await
        .map_err(|error| CommandError::operation("task_app_server_failed", error))?
    {
        hachimi_control_plane::AppServerResponse::Domain(response) => match *response {
            hachimi_control_plane::AppServerDomainResponse::Task(response) => Ok(*response),
            _ => Err(CommandError::new(
                "task_app_server_protocol_mismatch",
                "App Server returned a response for a different domain",
            )),
        },
        _ => Err(CommandError::new(
            "task_app_server_protocol_mismatch",
            "App Server returned a response for a different domain",
        )),
    }
}

fn scheduler_error(error: impl std::fmt::Display) -> CommandError {
    CommandError::operation("scheduler_failed", error)
}

#[cfg(test)]
fn mutation_fingerprint<T: serde::Serialize>(
    resource_id: &str,
    input: &T,
) -> Result<String, CommandError> {
    let bytes = serde_json::to_vec(input)
        .map_err(|error| CommandError::operation("mutation_fingerprint_failed", error))?;
    let hash = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("{resource_id}:{hash}"))
}

#[tauri::command]
pub(super) async fn create_schedule(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ScheduleCreateRequest,
) -> Result<ScheduleSnapshot, CommandError> {
    let client = authorize(&window, &state)?;
    match dispatch_schedule(
        &state,
        &app_context(client),
        hachimi_control_plane::ScheduleAppRequest::Create(request),
    )
    .await?
    {
        hachimi_control_plane::ScheduleAppResponse::Created(snapshot) => Ok(snapshot),
        _ => Err(CommandError::new(
            "schedule_response_mismatch",
            "expected Schedule snapshot",
        )),
    }
}

#[tauri::command]
pub(super) async fn get_schedule(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    schedule_id: ScheduleId,
) -> Result<Option<ScheduleSnapshot>, CommandError> {
    let context = app_context(authorize(&window, &state)?);
    match dispatch_schedule(
        &state,
        &context,
        hachimi_control_plane::ScheduleAppRequest::Get(schedule_id),
    )
    .await?
    {
        hachimi_control_plane::ScheduleAppResponse::Snapshot(snapshot) => Ok(snapshot),
        _ => Err(CommandError::new(
            "schedule_response_mismatch",
            "expected Schedule snapshot",
        )),
    }
}

#[tauri::command]
pub(super) async fn list_schedules(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Vec<ScheduleDefinition>, CommandError> {
    let context = app_context(authorize(&window, &state)?);
    match dispatch_schedule(
        &state,
        &context,
        hachimi_control_plane::ScheduleAppRequest::List,
    )
    .await?
    {
        hachimi_control_plane::ScheduleAppResponse::Schedules(schedules) => Ok(schedules),
        _ => Err(CommandError::new(
            "schedule_response_mismatch",
            "expected Schedule list",
        )),
    }
}

#[tauri::command]
pub(super) async fn preview_schedule(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    schedule: ScheduleSpec,
    count: usize,
) -> Result<SchedulePreview, CommandError> {
    let context = app_context(authorize(&window, &state)?);
    match dispatch_schedule(
        &state,
        &context,
        hachimi_control_plane::ScheduleAppRequest::Preview { schedule, count },
    )
    .await?
    {
        hachimi_control_plane::ScheduleAppResponse::Preview(preview) => Ok(preview),
        _ => Err(CommandError::new(
            "schedule_response_mismatch",
            "expected preview",
        )),
    }
}

#[tauri::command]
pub(super) async fn update_schedule(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ScheduleUpdateRequest,
) -> Result<ScheduleDefinition, CommandError> {
    let client = authorize(&window, &state)?;
    match dispatch_schedule(
        &state,
        &app_context(client),
        hachimi_control_plane::ScheduleAppRequest::Update(request),
    )
    .await?
    {
        hachimi_control_plane::ScheduleAppResponse::Schedule(schedule) => Ok(schedule),
        _ => Err(CommandError::new(
            "schedule_response_mismatch",
            "expected Schedule",
        )),
    }
}

#[tauri::command]
pub(super) async fn set_schedule_enabled(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    context: MutationContext,
    schedule_id: ScheduleId,
    enabled: bool,
    expected_config_revision: u64,
) -> Result<ScheduleDefinition, CommandError> {
    let client = authorize(&window, &state)?;
    match dispatch_schedule(
        &state,
        &app_context(client),
        hachimi_control_plane::ScheduleAppRequest::SetEnabled {
            context,
            schedule_id,
            enabled,
            expected_config_revision,
        },
    )
    .await?
    {
        hachimi_control_plane::ScheduleAppResponse::Schedule(schedule) => Ok(schedule),
        _ => Err(CommandError::new(
            "schedule_response_mismatch",
            "expected Schedule",
        )),
    }
}

#[tauri::command]
pub(super) async fn remove_schedule(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    context: MutationContext,
    schedule_id: ScheduleId,
) -> Result<bool, CommandError> {
    let client = authorize(&window, &state)?;
    match dispatch_schedule(
        &state,
        &app_context(client),
        hachimi_control_plane::ScheduleAppRequest::Remove {
            context,
            schedule_id,
        },
    )
    .await?
    {
        hachimi_control_plane::ScheduleAppResponse::Removed(removed) => Ok(removed),
        _ => Err(CommandError::new(
            "schedule_response_mismatch",
            "expected removal result",
        )),
    }
}

#[tauri::command]
pub(super) async fn reauthorize_schedule(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    context: MutationContext,
    schedule_id: ScheduleId,
) -> Result<ScheduleGrantRecord, CommandError> {
    let client = authorize(&window, &state)?;
    match dispatch_schedule(
        &state,
        &app_context(client),
        hachimi_control_plane::ScheduleAppRequest::Reauthorize {
            context,
            schedule_id,
        },
    )
    .await?
    {
        hachimi_control_plane::ScheduleAppResponse::Grant(Some(grant)) => Ok(grant),
        _ => Err(CommandError::new(
            "schedule_response_mismatch",
            "expected Schedule grant",
        )),
    }
}

#[tauri::command]
pub(super) async fn revoke_schedule_grant(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    context: MutationContext,
    schedule_id: ScheduleId,
) -> Result<Option<ScheduleGrantRecord>, CommandError> {
    let client = authorize(&window, &state)?;
    match dispatch_schedule(
        &state,
        &app_context(client),
        hachimi_control_plane::ScheduleAppRequest::RevokeGrant {
            context,
            schedule_id,
        },
    )
    .await?
    {
        hachimi_control_plane::ScheduleAppResponse::Grant(grant) => Ok(grant),
        _ => Err(CommandError::new(
            "schedule_response_mismatch",
            "expected Schedule grant",
        )),
    }
}

#[tauri::command]
pub(super) async fn run_schedule_now(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    context: MutationContext,
    schedule_id: ScheduleId,
) -> Result<TaskRunRecord, CommandError> {
    let client = authorize(&window, &state)?;
    match dispatch_schedule(
        &state,
        &app_context(client),
        hachimi_control_plane::ScheduleAppRequest::RunNow {
            context,
            schedule_id,
        },
    )
    .await?
    {
        hachimi_control_plane::ScheduleAppResponse::Task(task) => Ok(task),
        _ => Err(CommandError::new(
            "schedule_response_mismatch",
            "expected TaskRun",
        )),
    }
}

#[tauri::command]
pub(super) async fn get_task_run(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    task_run_id: TaskRunId,
) -> Result<Option<TaskRunRecord>, CommandError> {
    let context = app_context(authorize(&window, &state)?);
    match dispatch_task(
        &state,
        &context,
        hachimi_control_plane::TaskAppRequest::Get(task_run_id),
    )
    .await?
    {
        hachimi_control_plane::TaskAppResponse::Task(task) => Ok(task),
        _ => Err(CommandError::new(
            "task_response_mismatch",
            "expected TaskRun",
        )),
    }
}

#[tauri::command]
pub(super) async fn list_task_runs(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    schedule_id: Option<ScheduleId>,
    limit: u32,
) -> Result<Vec<TaskRunRecord>, CommandError> {
    let context = app_context(authorize(&window, &state)?);
    match dispatch_task(
        &state,
        &context,
        hachimi_control_plane::TaskAppRequest::List { schedule_id, limit },
    )
    .await?
    {
        hachimi_control_plane::TaskAppResponse::Tasks(tasks) => Ok(tasks),
        _ => Err(CommandError::new(
            "task_response_mismatch",
            "expected TaskRun list",
        )),
    }
}

#[tauri::command]
pub(super) async fn cancel_task_run(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    context: MutationContext,
    task_run_id: TaskRunId,
) -> Result<TaskRunRecord, CommandError> {
    let client = authorize(&window, &state)?;
    match dispatch_task(
        &state,
        &app_context(client),
        hachimi_control_plane::TaskAppRequest::Cancel {
            context,
            task_run_id,
        },
    )
    .await?
    {
        hachimi_control_plane::TaskAppResponse::Updated(task) => Ok(task),
        _ => Err(CommandError::new(
            "task_response_mismatch",
            "expected TaskRun",
        )),
    }
}

#[tauri::command]
pub(super) async fn retry_task_run(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    context: MutationContext,
    task_run_id: TaskRunId,
) -> Result<TaskRunRecord, CommandError> {
    let client = authorize(&window, &state)?;
    match dispatch_task(
        &state,
        &app_context(client),
        hachimi_control_plane::TaskAppRequest::Retry {
            context,
            task_run_id,
        },
    )
    .await?
    {
        hachimi_control_plane::TaskAppResponse::Updated(task) => Ok(task),
        _ => Err(CommandError::new(
            "task_response_mismatch",
            "expected TaskRun",
        )),
    }
}

#[tauri::command]
pub(super) async fn continue_task_interactively(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    context: MutationContext,
    task_run_id: TaskRunId,
) -> Result<TaskInteractiveContinuation, CommandError> {
    let client = authorize(&window, &state)?;
    match dispatch_task(
        &state,
        &app_context(client),
        hachimi_control_plane::TaskAppRequest::ContinueInteractively {
            context,
            task_run_id,
        },
    )
    .await?
    {
        hachimi_control_plane::TaskAppResponse::Continuation(continuation) => Ok(*continuation),
        _ => Err(CommandError::new(
            "task_response_mismatch",
            "expected interactive continuation",
        )),
    }
}

pub(super) async fn continue_task_interactively_inner(
    app: AppHandle,
    state: &DesktopState,
    client: hachimi_protocol::ClientContext,
    task_run_id: TaskRunId,
    idempotency_key: String,
) -> Result<TaskInteractiveContinuation, CommandError> {
    let task = state
        .agent_store
        .get_task_run(&task_run_id)
        .await
        .map_err(scheduler_error)?
        .ok_or_else(|| CommandError::new("task_not_found", "TaskRun does not exist"))?;
    let schedule_id = task
        .schedule_id
        .clone()
        .ok_or_else(|| CommandError::new("schedule_not_found", "TaskRun has no Schedule"))?;
    let schedule = state
        .agent_store
        .get_schedule(&schedule_id)
        .await
        .map_err(scheduler_error)?
        .ok_or_else(|| CommandError::new("schedule_not_found", "Schedule does not exist"))?;
    let source = if let Some(run_id) = &task.run_id {
        state
            .agent_store
            .get_run(run_id)
            .await
            .map_err(scheduler_error)?
    } else {
        None
    };
    let (created, project_snapshot) = create_agent_run_for_schedule(
        state,
        &schedule,
        &task,
        ScheduleRunCreateInputs {
            principal: client.client_id.0.clone(),
            idempotency_key,
            origin: source.map(|source_run| RunOrigin::Handoff {
                source_session_id: source_run.session_id,
                source_run_id: source_run.id,
            }),
            interactive: true,
            cancellation: CancellationToken::new(),
        },
    )
    .await?;
    state
        .agent_store
        .bind_task_run_requester(&task.id, &created.session.id, now_ms())
        .await
        .map_err(scheduler_error)?;
    spawn_created_run(
        app,
        CreatedRunExecution {
            client,
            schedule,
            created: created.clone(),
            project_snapshot,
            schedule_authorization: None,
            schedule_grant_hash: None,
            cancellation: CancellationToken::new(),
        },
    );
    Ok(TaskInteractiveContinuation {
        task_run: task,
        session: created.session,
        run: created.run,
    })
}

async fn launch_scheduled_agent_run(
    app: AppHandle,
    schedule: ScheduleDefinition,
    task_run: TaskRunRecord,
    cancellation: CancellationToken,
) -> Result<ScheduleRunCompletion, ScheduleLaunchError> {
    let state = app.state::<DesktopState>();
    let authorization = match validate_schedule_runtime(&state, &schedule, &task_run).await {
        Ok(value) => value,
        Err(error) => return Ok(needs_attention_completion(error)),
    };
    let (created, project_snapshot) = create_agent_run_for_schedule(
        &state,
        &schedule,
        &task_run,
        ScheduleRunCreateInputs {
            principal: "service:scheduler".into(),
            idempotency_key: format!("schedule-run:{}", task_run.id),
            origin: None,
            interactive: false,
            cancellation: cancellation.child_token(),
        },
    )
    .await
    .map_err(command_to_launch_error)?;
    state
        .agent_store
        .bind_task_run_execution(&task_run.id, &created.session.id, &created.run.id, now_ms())
        .await
        .map_err(store_to_launch_error)?;
    state
        .agent_store
        .transition_task_run(
            &task_run.id,
            TaskRunStatus::Running,
            Some(0),
            None,
            None,
            None,
            &[],
            now_ms(),
        )
        .await
        .map_err(store_to_launch_error)?;
    let run_id = created.run.id.clone();
    let generation = created.run.generation;
    let registry = Arc::clone(state.agent_executor.registry());
    let bridge_cancellation = cancellation.clone();
    let bridge = tokio::spawn(async move {
        bridge_cancellation.cancelled().await;
        let _ = registry.cancel(&run_id, generation);
    });
    let client = hachimi_protocol::ClientContext::for_internal("scheduler");
    let execution = execute_created_run(
        &state,
        CreatedRunExecution {
            client,
            schedule: schedule.clone(),
            created: created.clone(),
            project_snapshot,
            schedule_authorization: Some(authorization.scope),
            schedule_grant_hash: Some(authorization.grant_hash),
            cancellation: cancellation.clone(),
        },
    );
    tokio::pin!(execution);
    let result = tokio::select! {
        result = &mut execution => result,
        () = tokio::time::sleep(Duration::from_millis(schedule.timeout_ms)) => {
            cancellation.cancel();
            let _ = (&mut execution).await;
            Err(CommandError::new("task_timed_out", "the scheduled Agent Run exceeded its timeout"))
        }
    };
    bridge.abort();
    let run = state
        .agent_store
        .get_run(&created.run.id)
        .await
        .map_err(store_to_launch_error)?
        .unwrap_or(created.run);
    let summary = latest_assistant_summary(&state.agent_store, &run).await;
    let timed_out = result
        .as_ref()
        .err()
        .is_some_and(|error| error.code == "task_timed_out");
    let run_events = state
        .agent_store
        .list_events(&run.session_id, 0)
        .await
        .map_err(store_to_launch_error)?;
    let elicitation_needs_attention = run_events.iter().any(|event| {
        event.run_id.as_ref() == Some(&run.id)
            && matches!(
                &event.payload,
                hachimi_protocol::RunEventPayload::Generic { event, .. }
                    if event == "mcp.elicitation.needs_attention"
            )
    });
    let runtime_drift_needs_attention = run_events.iter().any(|event| {
        event.run_id.as_ref() == Some(&run.id)
            && matches!(
                &event.payload,
                hachimi_protocol::RunEventPayload::Generic { event, .. }
                    if event == "runtime.extension_drift.needs_attention"
            )
    });
    let needs_attention = elicitation_needs_attention || runtime_drift_needs_attention;
    let status = scheduled_completion_status(run.status, timed_out, needs_attention);
    let execution_error_code = result.err().map(|error| error.code);
    Ok(ScheduleRunCompletion {
        status,
        result_summary: summary,
        error_code: if runtime_drift_needs_attention {
            Some("runtime_extension_drift_needs_attention".into())
        } else if elicitation_needs_attention {
            Some("mcp_elicitation_requires_interaction".into())
        } else {
            execution_error_code
        },
        error_summary: if runtime_drift_needs_attention {
            Some(
                "a pinned Skill, MCP binding, Workspace Host, or Sandbox capability changed while the scheduled Run was active"
                    .into(),
            )
        } else if elicitation_needs_attention {
            Some(
                "an MCP server requested interactive input outside the persisted ScheduleGrant"
                    .into(),
            )
        } else {
            (!matches!(status, TaskRunStatus::Succeeded)).then(|| {
                run.failure_code
                    .clone()
                    .unwrap_or_else(|| "scheduled Agent Run did not succeed".into())
            })
        },
        artifact_ids: Vec::new(),
    })
}

#[derive(Debug)]
struct RuntimeAuthorization {
    scope: ScheduleAuthorizationScope,
    grant_hash: String,
}

async fn validate_schedule_runtime(
    state: &DesktopState,
    schedule: &ScheduleDefinition,
    task: &TaskRunRecord,
) -> Result<RuntimeAuthorization, ScheduleLaunchError> {
    let grant = state
        .agent_store
        .active_schedule_grant(&schedule.id)
        .await
        .map_err(store_to_launch_error)?
        .ok_or_else(|| {
            attention(
                "schedule_authorization_required",
                "Schedule authorization is missing",
            )
        })?;
    let current_scope = schedule_scope_with_extension_snapshots(state, schedule)
        .await
        .map_err(|error| attention(error.code, error.message))?;
    if grant.permission_revision != schedule.permission_revision
        || task.permission_snapshot_hash.as_deref() != Some(grant.scope_hash.as_str())
        || grant.scope != current_scope
    {
        return Err(attention(
            "schedule_authorization_stale",
            "the persisted Schedule authorization no longer matches this invocation",
        ));
    }
    if schedule.permission_config.permission_profile != PermissionProfile::ReadOnly
        && state.sandbox_status() != hachimi_sandbox::SandboxStatus::Enforced
    {
        return Err(attention(
            "sandbox_not_enforced",
            "background write, process, network, or external side effects require an enforced Sandbox",
        ));
    }
    if matches!(
        schedule.context_template,
        ScheduleContextTemplate::Project {
            execution_target: ExecutionTarget::ManagedWorktree { .. },
            ..
        }
    ) && state
        .agent_store
        .count_schedule_retained_worktrees(&schedule.id)
        .await
        .map_err(store_to_launch_error)?
        >= 5
    {
        return Err(attention(
            "schedule_dirty_worktree_limit",
            "five unreviewed dirty Worktrees are already retained for this Schedule",
        ));
    }
    validate_schedule_extensions(state, &schedule.id)
        .await
        .map_err(|error| attention(error.code, error.message))?;
    Ok(RuntimeAuthorization {
        scope: grant.scope,
        grant_hash: grant.scope_hash,
    })
}

async fn validate_schedule_extensions(
    state: &DesktopState,
    schedule_id: &ScheduleId,
) -> Result<(), CommandError> {
    let schedule = state
        .agent_store
        .get_schedule(schedule_id)
        .await
        .map_err(scheduler_error)?
        .ok_or_else(|| CommandError::new("schedule_not_found", "Schedule does not exist"))?;
    validate_definition_extensions(state, &schedule).await
}

async fn validate_definition_extensions(
    state: &DesktopState,
    schedule: &ScheduleDefinition,
) -> Result<(), CommandError> {
    schedule_scope_with_extension_snapshots(state, schedule)
        .await
        .map(|_| ())
}

async fn schedule_scope_with_extension_snapshots(
    state: &DesktopState,
    schedule: &ScheduleDefinition,
) -> Result<ScheduleAuthorizationScope, CommandError> {
    let skill_context = match &schedule.context_template {
        ScheduleContextTemplate::General => hachimi_skills::SkillCatalogContext::default(),
        ScheduleContextTemplate::Project { project_id, .. } => {
            let project = state
                .agent_store
                .get_project(project_id)
                .await
                .map_err(scheduler_error)?
                .ok_or_else(|| {
                    CommandError::new(
                        "schedule_project_unavailable",
                        "Schedule Project no longer exists",
                    )
                })?;
            hachimi_skills::SkillCatalogContext {
                project_root: Some(PathBuf::from(project.root_path)),
                checkout_root: None,
            }
        }
    };
    let skills = state
        .skill_host
        .list_for_context(&skill_context)
        .await
        .map_err(|error| CommandError::operation("skill_validation_failed", error))?;
    let mcp_servers = state
        .mcp_control
        .list()
        .await
        .map_err(|error| CommandError::operation("schedule_mcp_validation_failed", error))?;
    let mut skill_revisions = Vec::with_capacity(schedule.skill_allowlist.len());
    for skill_id in &schedule.skill_allowlist {
        let skill = skills
            .iter()
            .find(|skill| &skill.id == skill_id && skill.enabled)
            .ok_or_else(|| {
                CommandError::new(
                    "schedule_skill_unavailable",
                    format!("Skill {skill_id} is disabled, missing, or invalid"),
                )
            })?;
        if skill
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == SkillDiagnosticSeverity::Error)
        {
            return Err(CommandError::new(
                "schedule_skill_invalid",
                format!("Skill {skill_id} has blocking dependency or content diagnostics"),
            ));
        }
        for dependency in &skill.dependencies {
            if dependency.kind.eq_ignore_ascii_case("mcp")
                && !mcp_dependency_available(dependency, &mcp_servers)
            {
                return Err(CommandError::new(
                    "schedule_skill_dependency_missing",
                    format!(
                        "Skill {} requires unavailable MCP dependency {}",
                        skill.qualified_name, dependency.value
                    ),
                ));
            }
        }
        skill_revisions.push(ScheduleSkillSelection {
            skill_id: skill.id.clone(),
            content_hash: skill.content_hash.clone(),
            tree_revision: skill.tree_revision.clone(),
        });
    }
    if !schedule.mcp_tool_allowlist.is_empty() && !state.control_plane.feature_flags().mcp_runtime {
        return Err(CommandError::new(
            "schedule_mcp_unavailable",
            "MCP connectors are disabled",
        ));
    }
    if !schedule.mcp_tool_allowlist.is_empty() {
        let runtimes =
            state.mcp_control.ready_runtimes().await.map_err(|error| {
                CommandError::operation("schedule_mcp_validation_failed", error)
            })?;
        for selection in &schedule.mcp_tool_allowlist {
            let runtime = runtimes.iter().find(|runtime| {
                runtime.configuration.id == selection.server_id
                    && runtime.tools.iter().any(|tool| {
                        tool.name == selection.tool_name
                            && schema_hash(&tool.input_schema) == selection.schema_hash
                            && hachimi_control_plane::mcp_host_identity_hash(&runtime.configuration)
                                == selection.host_identity_hash
                    })
            });
            let Some(runtime) = runtime else {
                return Err(CommandError::new(
                    "schedule_mcp_schema_changed",
                    format!(
                        "MCP tool {} on {} is unavailable or its schema changed",
                        selection.tool_name, selection.server_id
                    ),
                ));
            };
            if !runtime
                .configuration
                .read_only_tools
                .contains(&selection.tool_name)
            {
                let exact_target = format!(
                    "mcp:{}:{}",
                    selection.server_id.as_str(),
                    selection.tool_name
                );
                if schedule.permission_config.permission_profile
                    != PermissionProfile::ExternalSandbox
                    || !schedule
                        .permission_config
                        .external_targets
                        .contains(&exact_target)
                {
                    return Err(CommandError::new(
                        "schedule_mcp_side_effect_not_authorized",
                        format!(
                            "MCP side-effect tool {} on {} requires an exact persisted target and ExternalSandbox permission",
                            selection.tool_name, selection.server_id
                        ),
                    ));
                }
            }
        }
    }
    let mut scope = schedule_scope(schedule);
    scope.skill_revisions = skill_revisions;
    scope.tool_allowlist.sort();
    scope.tool_allowlist.dedup();
    scope.skill_allowlist.sort();
    scope.skill_allowlist.dedup();
    scope
        .skill_revisions
        .sort_by(|left, right| left.skill_id.cmp(&right.skill_id));
    scope
        .skill_revisions
        .dedup_by(|left, right| left.skill_id == right.skill_id);
    scope.mcp_tool_allowlist.sort_by(|left, right| {
        (
            &left.server_id,
            &left.tool_name,
            &left.schema_hash,
            &left.host_identity_hash,
        )
            .cmp(&(
                &right.server_id,
                &right.tool_name,
                &right.schema_hash,
                &right.host_identity_hash,
            ))
    });
    scope.mcp_tool_allowlist.dedup();
    scope.permission_config.external_targets.sort();
    scope.permission_config.external_targets.dedup();
    Ok(scope)
}

fn mcp_dependency_available(
    dependency: &hachimi_protocol::SkillToolDependency,
    servers: &[hachimi_protocol::McpServerView],
) -> bool {
    servers.iter().any(|server| {
        if !server.configuration.enabled
            || server.health.state != hachimi_protocol::McpServerHealthState::Ready
        {
            return false;
        }
        if server.configuration.id.as_str() == dependency.value {
            return true;
        }
        match &server.configuration.transport {
            hachimi_protocol::McpServerTransport::StreamableHttp { url } => dependency
                .url
                .as_ref()
                .is_some_and(|expected| expected == url),
            hachimi_protocol::McpServerTransport::Stdio { command, .. } => dependency
                .command
                .as_ref()
                .is_some_and(|expected| expected == command),
        }
    })
}

struct ScheduleRunCreateInputs {
    principal: String,
    idempotency_key: String,
    origin: Option<RunOrigin>,
    interactive: bool,
    cancellation: CancellationToken,
}

struct CreatedRunExecution {
    client: hachimi_protocol::ClientContext,
    schedule: ScheduleDefinition,
    created: hachimi_storage::CreatedAgentRun,
    project_snapshot: Option<WorkbenchTaskSnapshot>,
    schedule_authorization: Option<ScheduleAuthorizationScope>,
    schedule_grant_hash: Option<String>,
    cancellation: CancellationToken,
}

async fn create_agent_run_for_schedule(
    state: &DesktopState,
    schedule: &ScheduleDefinition,
    task: &TaskRunRecord,
    inputs: ScheduleRunCreateInputs,
) -> Result<
    (
        hachimi_storage::CreatedAgentRun,
        Option<WorkbenchTaskSnapshot>,
    ),
    CommandError,
> {
    let model_snapshot = state.settings.read().llm.clone();
    let now = now_ms();
    let (context, execution_target, project_and_checkout) = match &schedule.context_template {
        ScheduleContextTemplate::General => (SessionContextBinding::General, None, None),
        ScheduleContextTemplate::Project {
            project_id,
            execution_target,
        } => {
            let project = state
                .agent_store
                .get_project(project_id)
                .await
                .map_err(scheduler_error)?
                .ok_or_else(|| {
                    CommandError::new("project_not_found", "Schedule Project does not exist")
                })?;
            let checkout = state
                .workbench
                .prepare_checkout(execution_target, &inputs.cancellation)
                .await
                .map_err(|error| CommandError::operation("schedule_checkout_failed", error))?;
            (
                SessionContextBinding::Project {
                    project_id: project.id.clone(),
                    checkout_id: checkout.id.clone(),
                },
                Some(execution_target.clone()),
                Some((project, checkout)),
            )
        }
    };
    let created = AgentRunFactory::new(state.agent_store.clone())
        .create(AgentRunCreateRequest {
            principal: inputs.principal,
            idempotency_key: inputs.idempotency_key,
            context,
            origin: inputs.origin.unwrap_or_else(|| {
                if inputs.interactive {
                    RunOrigin::Interactive
                } else {
                    RunOrigin::Scheduled {
                        schedule_id: schedule.id.clone(),
                        task_run_id: task.id.clone(),
                        scheduled_for_ms: task.scheduled_for_ms.unwrap_or(now),
                    }
                }
            }),
            title: schedule.name.clone(),
            prompt: schedule.prompt.clone(),
            attachment_ids: Vec::new(),
            parent_session_id: task.execution_session_id.clone(),
            source_run_id: task.run_id.clone(),
            purpose: RunPurpose::Task,
            model_snapshot: model_snapshot.clone(),
            entry_profile: schedule.entry_profile,
            workload_override: schedule.workload_override,
            behavior_mode: BehaviorMode::Default,
            execution_target,
            approval_policy: if inputs.interactive {
                ApprovalPolicy::OnlyWhenNeeded
            } else {
                ApprovalPolicy::NeverPrompt
            },
            permission_profile: schedule.permission_config.permission_profile,
            budget: RunBudget {
                model_timeout_ms: schedule.timeout_ms.min(120_000),
                tool_timeout_ms: schedule.timeout_ms.min(120_000),
                ..RunBudget::default()
            },
            requested_capabilities: requested_capabilities(&model_snapshot),
            created_at_ms: now,
        })
        .await
        .map_err(|error| CommandError::operation("schedule_run_create_failed", error))?;
    let snapshot = project_and_checkout.map(|(project, checkout)| WorkbenchTaskSnapshot {
        project,
        checkout,
        session: created.session.clone(),
        run: created.run.clone(),
    });
    Ok((created, snapshot))
}

fn spawn_created_run(app: AppHandle, execution: CreatedRunExecution) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<DesktopState>();
        let _ = execute_created_run(&state, execution).await;
    });
}

async fn execute_created_run(
    state: &DesktopState,
    execution: CreatedRunExecution,
) -> Result<(), CommandError> {
    let CreatedRunExecution {
        client,
        schedule,
        created,
        project_snapshot,
        schedule_authorization,
        schedule_grant_hash,
        cancellation: external_cancellation,
    } = execution;
    let executor = state.agent_executor.clone();
    let created_run_id = created.run.id.clone();
    let managed_checkout_id = project_snapshot.as_ref().and_then(|snapshot| {
        (snapshot.checkout.kind == CheckoutKind::ManagedWorktree)
            .then(|| snapshot.checkout.id.clone())
    });
    if external_cancellation.is_cancelled() {
        return Err(CommandError::new(
            "scheduled_agent_cancelled",
            "the scheduled Agent Run was cancelled before dispatch",
        ));
    }
    let workspace_root = project_snapshot.as_ref().map_or_else(
        || "general://extensions".to_owned(),
        |snapshot| snapshot.checkout.path.clone(),
    );
    let mut capability_grants = expand_permission_profile(
        created.run.configuration.permission_profile,
        created.run.configuration.behavior_mode,
        created.session.id.clone(),
        created.run.id.clone(),
        workspace_root,
    );
    if project_snapshot.is_none() {
        capability_grants
            .file_system
            .retain(|grant| grant.access == hachimi_protocol::FileSystemAccess::Read);
        capability_grants.process = hachimi_protocol::ProcessGrant::default();
    }
    if let Some(hash) = &schedule_grant_hash {
        capability_grants.source = format!("schedule_grant:{hash}");
        capability_grants.review_each_command = false;
    }
    let run_tool_allowlist = schedule_authorization.as_ref().map(|scope| {
        let mut tools = scope.tool_allowlist.clone();
        if !scope.skill_allowlist.is_empty() {
            tools.extend([
                hachimi_agent::SKILLS_LIST_TOOL.into(),
                hachimi_agent::SKILLS_READ_TOOL.into(),
            ]);
        }
        tools.extend(scope.mcp_tool_allowlist.iter().map(|selection| {
            hachimi_capabilities::mcp_exposed_tool_name(
                selection.server_id.as_str(),
                &selection.tool_name,
            )
        }));
        tools.sort();
        tools.dedup();
        tools
    });
    let skill_allowlist = schedule_authorization.as_ref().map_or_else(
        || schedule.skill_allowlist.clone(),
        |scope| scope.skill_allowlist.clone(),
    );
    let mcp_tool_allowlist = schedule_authorization.as_ref().map_or_else(
        || schedule.mcp_tool_allowlist.clone(),
        |scope| scope.mcp_tool_allowlist.clone(),
    );
    let operation = executor.execute(hachimi_agent::AgentRunRequest {
        principal: client.client_id.0,
        session: created.session,
        run: created.run,
        priority: AgentRunPriority::Background,
        capability_grants,
        sandbox_snapshot: state.sandbox_snapshot().report,
        attachment_ids: Vec::new(),
        skill_allowlist,
        mcp_tool_allowlist,
        run_tool_allowlist,
        workload_override: schedule.workload_override,
    });
    let mut result = operation
        .await
        .map_err(|error| CommandError::operation("scheduled_agent_execution_failed", error));
    if result.is_err() {
        ensure_run_failed(&state.agent_store, &created_run_id).await;
    }
    if let Some(checkout_id) = managed_checkout_id {
        match state.workbench.cleanup_checkout(&checkout_id).await {
            Ok(_) | Err(hachimi_workbench::WorkbenchError::CheckoutDirty) => {}
            Err(error) if result.is_ok() => {
                result = Err(CommandError::operation(
                    "schedule_worktree_cleanup_failed",
                    error,
                ));
            }
            Err(error) => {
                tracing::warn!(%error, %checkout_id, "Scheduled Worktree cleanup failed after the Run failed");
            }
        }
    }
    result
}

async fn ensure_run_failed(store: &hachimi_storage::AgentStore, run_id: &hachimi_protocol::RunId) {
    let Ok(Some(run)) = store.get_run(run_id).await else {
        return;
    };
    if run.status.is_terminal() {
        return;
    }
    if run.status == RunStatus::Queued {
        let _ = store
            .transition_run(run_id, RunStatus::Preparing, None)
            .await;
    }
    let _ = store
        .transition_run(run_id, RunStatus::Failed, Some("agent_setup_failed"))
        .await;
}

async fn latest_assistant_summary(
    store: &hachimi_storage::AgentStore,
    run: &RunRecord,
) -> Option<String> {
    store
        .list_transcript(&run.session_id)
        .await
        .ok()?
        .into_iter()
        .rev()
        .find_map(|item| {
            if item.kind != TranscriptItemKind::Assistant || item.run_id.as_ref() != Some(&run.id) {
                return None;
            }
            match item.payload {
                ItemPayload::Assistant { text } => Some(text.chars().take(2_000).collect()),
                _ => None,
            }
        })
}

fn schedule_scope(schedule: &ScheduleDefinition) -> ScheduleAuthorizationScope {
    ScheduleAuthorizationScope {
        entry_profile: schedule.entry_profile,
        workload_override: schedule.workload_override,
        context_template: schedule.context_template.clone(),
        tool_allowlist: schedule.tool_allowlist.clone(),
        skill_allowlist: schedule.skill_allowlist.clone(),
        skill_revisions: Vec::new(),
        mcp_tool_allowlist: schedule.mcp_tool_allowlist.clone(),
        permission_config: schedule.permission_config.clone(),
    }
}

fn requested_capabilities(settings: &LlmSettings) -> ProviderCapabilities {
    let structured =
        settings.structured_output_mode != hachimi_protocol::StructuredOutputMode::Disabled;
    ProviderCapabilities {
        tool_calls: true,
        parallel_tool_calls: true,
        strict_json_schema: structured,
        output_schema: structured,
        text_input: true,
        streaming_usage: true,
        http_transport: true,
        context_window: (settings.max_input_tokens > 0)
            .then_some(u64::from(settings.max_input_tokens)),
        max_output_tokens: (settings.max_output_tokens > 0)
            .then_some(u64::from(settings.max_output_tokens)),
        ..ProviderCapabilities::default()
    }
}

fn task_status_from_run(status: RunStatus) -> TaskRunStatus {
    match status {
        RunStatus::Succeeded => TaskRunStatus::Succeeded,
        RunStatus::TimedOut => TaskRunStatus::TimedOut,
        RunStatus::Cancelled => TaskRunStatus::Cancelled,
        RunStatus::Lost | RunStatus::Interrupted => TaskRunStatus::Lost,
        RunStatus::Queued
        | RunStatus::Preparing
        | RunStatus::Running
        | RunStatus::WaitingApproval
        | RunStatus::WaitingUserInput
        | RunStatus::Cancelling
        | RunStatus::Failed => TaskRunStatus::Failed,
    }
}

fn scheduled_completion_status(
    run_status: RunStatus,
    timed_out: bool,
    elicitation_needs_attention: bool,
) -> TaskRunStatus {
    if timed_out {
        TaskRunStatus::TimedOut
    } else if elicitation_needs_attention {
        TaskRunStatus::NeedsAttention
    } else {
        task_status_from_run(run_status)
    }
}

fn needs_attention_completion(error: ScheduleLaunchError) -> ScheduleRunCompletion {
    ScheduleRunCompletion {
        status: TaskRunStatus::NeedsAttention,
        result_summary: None,
        error_code: Some(error.code),
        error_summary: Some(error.message),
        artifact_ids: Vec::new(),
    }
}

fn schema_hash(value: &serde_json::Value) -> String {
    Sha256::digest(serde_json::to_vec(value).unwrap_or_default())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn attention(code: impl Into<String>, message: impl Into<String>) -> ScheduleLaunchError {
    ScheduleLaunchError {
        code: code.into(),
        message: message.into(),
    }
}

fn command_to_launch_error(error: CommandError) -> ScheduleLaunchError {
    ScheduleLaunchError {
        code: error.code,
        message: error.message,
    }
}

fn store_to_launch_error(error: impl std::fmt::Display) -> ScheduleLaunchError {
    attention("schedule_store_failed", error.to_string())
}

fn now_ms() -> i64 {
    i64::try_from(epoch_millis()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::{
        mcp_dependency_available, mutation_fingerprint, needs_attention_completion,
        scheduled_completion_status, task_notification_status,
    };
    use hachimi_protocol::{
        McpServerHealthRecord, McpServerHealthState, McpServerId, McpServerRecord,
        McpServerTransport, McpServerView, RunStatus, SkillToolDependency, TaskRunStatus,
    };

    fn mcp_server(state: McpServerHealthState) -> McpServerView {
        let id = McpServerId::from("calendar");
        McpServerView {
            configuration: McpServerRecord {
                id: id.clone(),
                display_name: "Calendar".into(),
                enabled: true,
                transport: McpServerTransport::StreamableHttp {
                    url: "https://calendar.example/mcp".into(),
                },
                headers: Vec::new(),
                read_only_tools: Vec::new(),
                startup_timeout_ms: 5_000,
                request_timeout_ms: 60_000,
                max_message_bytes: 1_048_576,
                created_at_ms: 1,
                updated_at_ms: 1,
            },
            health: McpServerHealthRecord {
                server_id: id,
                state,
                server_name: None,
                server_version: None,
                protocol_version: None,
                tool_count: 1,
                error_code: None,
                checked_at_ms: 1,
            },
        }
    }

    #[test]
    fn notification_copy_exposes_only_a_terminal_status() {
        assert_eq!(task_notification_status(TaskRunStatus::Succeeded), "已完成");
        assert_eq!(
            task_notification_status(TaskRunStatus::NeedsAttention),
            "需要处理"
        );
        assert_eq!(task_notification_status(TaskRunStatus::Failed), "执行失败");
    }

    #[test]
    fn mutation_fingerprint_rejects_parameter_changes_under_the_same_key() {
        let first = mutation_fingerprint("schedule-1", &(true, 3_u64)).expect("fingerprint");
        let replay = mutation_fingerprint("schedule-1", &(true, 3_u64)).expect("fingerprint");
        let changed = mutation_fingerprint("schedule-1", &(false, 3_u64)).expect("fingerprint");
        assert_eq!(first, replay);
        assert_ne!(first, changed);
    }

    #[test]
    fn background_elicitation_overrides_success_with_needs_attention() {
        assert_eq!(
            scheduled_completion_status(RunStatus::Succeeded, false, true),
            TaskRunStatus::NeedsAttention
        );
        assert_eq!(
            scheduled_completion_status(RunStatus::Succeeded, true, true),
            TaskRunStatus::TimedOut
        );
    }

    #[test]
    fn unavailable_skill_mcp_dependency_is_a_needs_attention_completion() {
        let dependency = SkillToolDependency {
            kind: "mcp".into(),
            value: "calendar".into(),
            description: None,
            transport: Some("streamable_http".into()),
            command: None,
            url: Some("https://calendar.example/mcp".into()),
        };
        assert!(mcp_dependency_available(
            &dependency,
            &[mcp_server(McpServerHealthState::Ready)]
        ));
        assert!(!mcp_dependency_available(
            &dependency,
            &[mcp_server(McpServerHealthState::Failed)]
        ));
        let completion = needs_attention_completion(super::ScheduleLaunchError {
            code: "schedule_skill_dependency_missing".into(),
            message: "calendar dependency is unavailable".into(),
        });
        assert_eq!(completion.status, TaskRunStatus::NeedsAttention);
        assert_eq!(
            completion.error_code.as_deref(),
            Some("schedule_skill_dependency_missing")
        );
    }
}
