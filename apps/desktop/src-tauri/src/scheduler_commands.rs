//! Desktop adapter for persistent prompt schedules and background Agent Runs.

use std::{path::PathBuf, sync::Arc, time::Duration};

use hachimi_agent::{
    AgentRunCreateRequest, AgentRunLaunchRequest, AgentRunLauncher, AgentRunPriority,
    LaunchedAgentRun,
};
use hachimi_approvals::ApprovalBroker;
use hachimi_protocol::{
    ApprovalPolicy, AuthorityMode, BehaviorMode, CheckoutKind, ItemId, ItemPayload, ItemRelations,
    ItemStatus, LlmSettings, MutationContext, PermissionProfile, ProviderCapabilities, RunBudget,
    RunOrigin, RunPurpose, RunRecord, RunStatus, ScheduleContextTemplate, ScheduleCreateRequest,
    ScheduleDefinition, ScheduleEventIngressRequest, ScheduleEventReceipt,
    ScheduleEventReceiptStatus, ScheduleEventSourceKind, ScheduleId, SchedulePreview,
    ScheduleSkillSelection, ScheduleSnapshot, ScheduleSpec, ScheduleUpdateRequest,
    SessionContextBinding, SkillDiagnosticSeverity, TaskInteractiveContinuation, TaskRunId,
    TaskRunRecord, TaskRunStatus, TranscriptItem, TranscriptItemKind, WorkbenchTaskSnapshot,
};
use hachimi_scheduler::{
    NotificationAdapter, NotificationFuture, ScheduleLaunchError, ScheduleLaunchFuture,
    ScheduleRunCompletion, ScheduleRunLauncher, TaskNotification,
};
use hachimi_user_input::UserInputBroker;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_notification::NotificationExt;
use tokio_util::sync::CancellationToken;

use super::{CommandError, ControlMethod, DesktopState, epoch_millis, require_window};

pub(super) const TASK_NOTIFICATION_EVENT: &str = "workbench-task-notification";

#[tauri::command]
pub(super) async fn choose_schedule_workspace_directory(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Option<String>, CommandError> {
    authorize(&window, &state)?;
    let picked = rfd::AsyncFileDialog::new()
        .set_title("Schedule Workspace")
        .pick_folder()
        .await;
    let Some(folder) = picked else {
        return Ok(None);
    };
    reject_selected_workspace_inside_data_root(&state, folder.path())?;
    Ok(Some(folder.path().to_string_lossy().into_owned()))
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
            hachimi_control_plane::AppServerDomainResponse::Schedule(response) => Ok(*response),
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
        hachimi_control_plane::ScheduleAppResponse::Created(snapshot) => {
            persist_schedule_permission_policy(&state, &snapshot.definition).await?;
            Ok(snapshot)
        }
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
        hachimi_control_plane::ScheduleAppResponse::Schedule(schedule) => {
            persist_schedule_permission_policy(&state, &schedule).await?;
            cancel_schedule_runs(&state, &schedule.id, "schedule_definition_changed").await?;
            Ok(schedule)
        }
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
        hachimi_control_plane::ScheduleAppResponse::Schedule(schedule) => {
            if !schedule.enabled {
                cancel_schedule_runs(&state, &schedule.id, "schedule_disabled").await?;
            }
            Ok(schedule)
        }
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
            schedule_id: schedule_id.clone(),
        },
    )
    .await?
    {
        hachimi_control_plane::ScheduleAppResponse::Removed(removed) => {
            if removed {
                cancel_schedule_runs(&state, &schedule_id, "schedule_removed").await?;
            }
            Ok(removed)
        }
        _ => Err(CommandError::new(
            "schedule_response_mismatch",
            "expected removal result",
        )),
    }
}

async fn persist_schedule_permission_policy(
    state: &DesktopState,
    schedule: &ScheduleDefinition,
) -> Result<(), CommandError> {
    let mut policy = schedule.permission_policy.clone();
    policy.revision = schedule.permission_revision;
    state
        .agent_store
        .store_permission_policy(&format!("schedule:{}", schedule.id), &policy, now_ms())
        .await
        .map_err(|error| CommandError::operation("schedule_permission_store_failed", error))
}

async fn cancel_schedule_runs(
    state: &DesktopState,
    schedule_id: &ScheduleId,
    reason: &str,
) -> Result<(), CommandError> {
    crate::permission_runtime::cancel_runs_for_permission_owner(
        state,
        &format!("schedule:{schedule_id}"),
        reason,
        now_ms(),
    )
    .await
    .map(|_| ())
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
pub(super) async fn ingest_schedule_event(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ScheduleEventIngressRequest,
) -> Result<ScheduleEventReceipt, CommandError> {
    let client = authorize(&window, &state)?;
    let context = app_context(client);
    let source_kind = request.source_kind;
    let source_id = request.source_id.clone();
    let event_id = request.event_id.clone();
    let event = hachimi_control_plane::LocalScheduleEvent::from(request);
    let result = match source_kind {
        ScheduleEventSourceKind::Workspace => {
            state
                .app_server
                .ingest_workspace_event(&context, event)
                .await
        }
        ScheduleEventSourceKind::Plugin => {
            state.app_server.ingest_plugin_event(&context, event).await
        }
        ScheduleEventSourceKind::Connector => {
            state
                .app_server
                .ingest_connector_event(&context, event)
                .await
        }
        ScheduleEventSourceKind::Channel => {
            state.app_server.ingest_channel_event(&context, event).await
        }
        ScheduleEventSourceKind::Gateway => {
            state.app_server.ingest_gateway_event(&context, event).await
        }
    };
    match result {
        Ok(receipt) => Ok(receipt),
        Err(error)
            if matches!(
                &error,
                hachimi_control_plane::AppServerError::Domain { code, .. }
                    if code == "schedule_event_conflict"
            ) =>
        {
            state
                .agent_store
                .list_schedule_event_receipts(200)
                .await
                .map_err(|store_error| {
                    CommandError::operation("schedule_event_receipt_lookup_failed", store_error)
                })?
                .into_iter()
                .find(|receipt| {
                    receipt.status == ScheduleEventReceiptStatus::Conflict
                        && receipt.event.event_id == event_id
                        && receipt.event.source.kind == source_kind
                        && receipt.event.source.principal == context.principal
                        && receipt.event.source.id == source_id
                })
                .ok_or_else(|| schedule_app_server_error(error))
        }
        Err(error) => Err(schedule_app_server_error(error)),
    }
}

fn schedule_app_server_error(error: hachimi_control_plane::AppServerError) -> CommandError {
    match error {
        hachimi_control_plane::AppServerError::Domain { code, message } => {
            CommandError::new(code, message)
        }
        other => CommandError::operation("schedule_app_server_failed", other),
    }
}

#[tauri::command]
pub(super) async fn list_schedule_event_receipts(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    limit: u32,
) -> Result<Vec<ScheduleEventReceipt>, CommandError> {
    let context = app_context(authorize(&window, &state)?);
    match dispatch_schedule(
        &state,
        &context,
        hachimi_control_plane::ScheduleAppRequest::ListEvents { limit },
    )
    .await?
    {
        hachimi_control_plane::ScheduleAppResponse::EventReceipts(receipts) => Ok(receipts),
        _ => Err(CommandError::new(
            "schedule_response_mismatch",
            "expected Schedule event receipts",
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
    let (launched, project_snapshot) = create_agent_run_for_schedule(
        state,
        &schedule,
        &task,
        ScheduleRunCreateInputs {
            principal: client.client_id.0.clone(),
            idempotency_key,
            origin: Some(RunOrigin::Manual),
            interactive: true,
        },
    )
    .await?;
    let created = launched.created;
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
            authority: launched.authority,
            capability_grants: launched.capability_grants,
            project_snapshot,
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
    match validate_schedule_runtime(&state, &schedule, &task_run).await {
        Ok(()) => {}
        Err(error) => return Ok(needs_attention_completion(error)),
    }
    let (launched, project_snapshot) = match create_agent_run_for_schedule(
        &state,
        &schedule,
        &task_run,
        ScheduleRunCreateInputs {
            principal: "service:scheduler".into(),
            idempotency_key: format!("schedule-run:{}", task_run.id),
            origin: None,
            interactive: false,
        },
    )
    .await
    {
        Ok(value) => value,
        Err(error) => return schedule_creation_error_outcome(error),
    };
    let created = launched.created;
    state
        .agent_store
        .bind_task_run_execution(&task_run.id, &created.session.id, &created.run.id, now_ms())
        .await
        .map_err(store_to_launch_error)?;
    state
        .agent_store
        .append_transcript_item(TranscriptItem {
            id: ItemId::random(),
            session_id: created.session.id.clone(),
            run_id: Some(created.run.id.clone()),
            sequence: 0,
            kind: TranscriptItemKind::SystemContext,
            status: ItemStatus::Completed,
            payload: ItemPayload::SystemContext {
                code: "schedule.heartbeat".into(),
                message: format!(
                    "Scheduled occurrence {} started as a fresh Run.",
                    task_run.invocation_key
                ),
            },
            relations: ItemRelations::default(),
            created_at_ms: now_ms(),
        })
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
            authority: launched.authority,
            capability_grants: launched.capability_grants,
            project_snapshot,
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
    let host_revision_needs_attention = has_host_revision_attention(&run_events, &run.id);
    let authority_needs_attention = run_events.iter().any(|event| {
        event.run_id.as_ref() == Some(&run.id)
            && matches!(
                &event.payload,
                hachimi_protocol::RunEventPayload::Generic { event, .. }
                    if event == hachimi_agent::AUTHORITY_NEEDS_ATTENTION_EVENT
            )
    }) || run.failure_code.as_deref()
        == Some("authority_needs_attention");
    let needs_attention = elicitation_needs_attention
        || runtime_drift_needs_attention
        || host_revision_needs_attention
        || authority_needs_attention;
    let status = scheduled_completion_status(run.status, timed_out, needs_attention);
    let execution_error_code = result.err().map(|error| error.code);
    Ok(ScheduleRunCompletion {
        status,
        result_summary: summary,
        error_code: if authority_needs_attention {
            Some("authority_needs_attention".into())
        } else if host_revision_needs_attention {
            Some("host_revision_snapshot_needs_attention".into())
        } else if runtime_drift_needs_attention {
            Some("runtime_extension_drift_needs_attention".into())
        } else if elicitation_needs_attention {
            Some("mcp_elicitation_requires_interaction".into())
        } else {
            execution_error_code
        },
        error_summary: if authority_needs_attention {
            Some(
                "the scheduled Run attempted a filesystem, process, Browser, Computer, MCP, or Connector action outside its configured authority"
                    .into(),
            )
        } else if host_revision_needs_attention {
            Some(
                "a pinned Connector account, action, or contribution revision no longer authorizes the scheduled attachment download"
                    .into(),
            )
        } else if runtime_drift_needs_attention {
            Some(
                "a pinned Skill, MCP binding, Workspace Host, or Sandbox capability changed while the scheduled Run was active"
                    .into(),
            )
        } else if elicitation_needs_attention {
            Some(
                "an MCP server requested interactive input outside the persisted Schedule policy"
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

async fn validate_schedule_runtime(
    state: &DesktopState,
    schedule: &ScheduleDefinition,
    task: &TaskRunRecord,
) -> Result<(), ScheduleLaunchError> {
    if task.schedule_revision != Some(schedule.config_revision) {
        return Err(attention(
            "schedule_definition_stale",
            "the Schedule definition changed after this invocation was claimed",
        ));
    }
    validate_schedule_runtime_revisions(state, schedule)
        .await
        .map_err(|error| attention(error.code, error.message))?;
    if schedule.permission_policy.level != PermissionProfile::ReadOnly
        && state.sandbox_status() != hachimi_sandbox::SandboxStatus::Enforced
    {
        return Err(attention(
            "sandbox_not_enforced",
            "background write, process, network, or external side effects require an enforced Sandbox",
        ));
    }
    validate_schedule_extensions(state, &schedule.id)
        .await
        .map_err(|error| attention(error.code, error.message))?;
    Ok(())
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
    let browser = &schedule.permission_policy.rules.browser;
    if schedule.permission_policy.level == hachimi_protocol::PermissionProfile::FullAccess
        || browser.observe
        || browser.act
        || browser.upload
        || browser.download
        || browser.cookie_storage
        || browser.cdp
    {
        state
            .embedded_browser
            .attest()
            .map_err(|error| CommandError::operation("schedule_browser_host_not_ready", error))?;
    }
    validate_schedule_runtime_revisions(state, schedule)
        .await
        .map(|_| ())
}

async fn validate_schedule_runtime_revisions(
    state: &DesktopState,
    schedule: &ScheduleDefinition,
) -> Result<(), CommandError> {
    let full_access = schedule.permission_policy.level == PermissionProfile::FullAccess;
    validate_unattended_browser_policy(schedule)?;
    if !full_access {
        crate::host_revision_snapshots::validate_enterprise_attachment_scope(schedule)
            .map_err(|error| CommandError::new(error.code, error.message))?;
        crate::host_revision_snapshots::validate_connector_revision_selections(
            &state.plugin_host,
            &schedule.host_revision_snapshot.connectors,
        )
        .await
        .map_err(|error| CommandError::new(error.code, error.message))?;
        state
            .plugin_host
            .verify_contribution_revisions(&schedule.contribution_revisions)
            .await
            .map_err(|error| CommandError::operation("schedule_contribution_drift", error))?;
    }
    let skill_context = match &schedule.context_template {
        ScheduleContextTemplate::Workspace { workspace, .. } => {
            let project_root = match workspace {
                hachimi_protocol::ScheduleWorkspaceSpec::Managed => state
                    .agent_store
                    .workspace_for_owner(hachimi_storage::WorkspaceOwnerRef::Schedule(&schedule.id))
                    .await
                    .map_err(scheduler_error)?
                    .map(|workspace| PathBuf::from(workspace.root_path)),
                hachimi_protocol::ScheduleWorkspaceSpec::SelectedDirectory { root_path } => {
                    Some(PathBuf::from(root_path))
                }
            };
            hachimi_skills::SkillCatalogContext {
                project_root,
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
    if !full_access
        && !schedule.mcp_tool_allowlist.is_empty()
        && !state.control_plane.feature_flags().mcp_runtime
    {
        return Err(CommandError::new(
            "schedule_mcp_unavailable",
            "MCP connectors are disabled",
        ));
    }
    if !full_access && !schedule.mcp_tool_allowlist.is_empty() {
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
            let requires_write = !runtime
                .configuration
                .read_only_tools
                .contains(&selection.tool_name);
            if !schedule.permission_policy.allows_mcp(
                &selection.server_id,
                &selection.tool_name,
                &selection.schema_hash,
                requires_write,
            ) {
                return Err(CommandError::new(
                    "schedule_mcp_tool_not_authorized",
                    format!(
                        "MCP tool {} on {} requires an exact persisted rule",
                        selection.tool_name, selection.server_id
                    ),
                ));
            }
        }
    }
    if !full_access {
        for selection in &schedule.host_revision_snapshot.connectors {
            for action in &selection.allowed_actions {
                let requires_write =
                    !schedule
                        .permission_policy
                        .rules
                        .connectors
                        .iter()
                        .any(|rule| {
                            rule.account_id == selection.account_id
                                && rule
                                    .read_only_actions
                                    .iter()
                                    .any(|read_only| read_only == action)
                        });
                if !schedule.permission_policy.allows_connector(
                    &selection.account_id,
                    action,
                    requires_write,
                ) {
                    return Err(CommandError::new(
                        "schedule_connector_action_not_authorized",
                        format!(
                            "Connector action {} on {} requires an exact persisted rule",
                            action, selection.account_id
                        ),
                    ));
                }
            }
        }
    }
    skill_revisions.sort();
    skill_revisions.dedup();
    if skill_revisions != schedule.skill_revisions {
        return Err(CommandError::new(
            "schedule_skill_revision_changed",
            "a pinned Skill changed after the Schedule definition was saved",
        ));
    }
    Ok(())
}

fn validate_unattended_browser_policy(schedule: &ScheduleDefinition) -> Result<(), CommandError> {
    if schedule.permission_policy.level == hachimi_protocol::PermissionProfile::FullAccess {
        return Ok(());
    }
    let browser = &schedule.permission_policy.rules.browser;
    if !(browser.observe
        || browser.act
        || browser.upload
        || browser.download
        || browser.cookie_storage
        || browser.cdp)
    {
        return Ok(());
    }
    if browser.origins.is_empty() {
        return Err(CommandError::new(
            "schedule_browser_grant_invalid",
            "an unattended Browser grant requires a document origin and capability",
        ));
    }
    if browser.upload {
        return Err(CommandError::new(
            "schedule_browser_upload_unattended_unsupported",
            "unattended Browser upload requires a separately pinned file grant, which is not supported in this release",
        ));
    }
    for origin in &browser.origins {
        let normalized = hachimi_browser::normalized_origin(origin)
            .map_err(|error| CommandError::operation("schedule_browser_origin_invalid", error))?;
        if normalized != *origin {
            return Err(CommandError::new(
                "schedule_browser_origin_not_canonical",
                format!("Browser origin must be stored as {normalized}"),
            ));
        }
    }
    Ok(())
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
}

struct CreatedRunExecution {
    client: hachimi_protocol::ClientContext,
    schedule: ScheduleDefinition,
    created: hachimi_storage::CreatedAgentRun,
    authority: hachimi_protocol::RunAuthoritySnapshot,
    capability_grants: hachimi_protocol::CapabilityGrantSet,
    project_snapshot: Option<WorkbenchTaskSnapshot>,
    cancellation: CancellationToken,
}

async fn create_agent_run_for_schedule(
    state: &DesktopState,
    schedule: &ScheduleDefinition,
    task: &TaskRunRecord,
    inputs: ScheduleRunCreateInputs,
) -> Result<(LaunchedAgentRun, Option<WorkbenchTaskSnapshot>), CommandError> {
    let model_snapshot = state.settings.read().llm.clone();
    let now = now_ms();
    let ScheduleContextTemplate::Workspace {
        workspace,
        conversation_mode,
    } = &schedule.context_template;
    let workspace = ensure_schedule_workspace(state, schedule, workspace, now).await?;
    let existing_session =
        if *conversation_mode == hachimi_protocol::ScheduleConversationMode::SharedSession {
            state
                .agent_store
                .session_for_workspace(&workspace.id)
                .await
                .map_err(scheduler_error)?
        } else {
            None
        };
    let context = SessionContextBinding::Workspace {
        workspace_id: workspace.id,
    };
    let create_request = AgentRunCreateRequest {
        principal: inputs.principal,
        idempotency_key: inputs.idempotency_key,
        context,
        origin: inputs.origin.unwrap_or_else(|| {
            if inputs.interactive {
                RunOrigin::Manual
            } else {
                RunOrigin::Scheduled {
                    schedule_id: schedule.id.clone(),
                    task_run_id: task.id.clone(),
                    scheduled_for_ms: task.scheduled_for_ms.unwrap_or(now),
                    event_context: task.event_context.clone(),
                }
            }
        }),
        title: schedule.name.clone(),
        prompt: schedule.prompt.clone(),
        attachment_ids: Vec::new(),
        parent_session_id: existing_session
            .is_none()
            .then(|| task.execution_session_id.clone())
            .flatten(),
        source_run_id: task.run_id.clone(),
        purpose: RunPurpose::Task,
        model_snapshot: model_snapshot.clone(),
        entry_profile: schedule.entry_profile,
        workload_override: schedule.workload_override,
        behavior_mode: BehaviorMode::Default,
        execution_target: None,
        approval_policy: if inputs.interactive {
            ApprovalPolicy::OnlyWhenNeeded
        } else {
            ApprovalPolicy::NeverPrompt
        },
        permission_profile: schedule.permission_policy.level,
        budget: RunBudget {
            model_timeout_ms: schedule.timeout_ms.min(120_000),
            tool_timeout_ms: schedule.timeout_ms.min(120_000),
            ..RunBudget::default()
        },
        requested_capabilities: requested_capabilities(&model_snapshot),
        created_at_ms: now,
    };
    let policy = state
        .agent_store
        .permission_policy(&format!("schedule:{}", schedule.id))
        .await
        .map_err(scheduler_error)?
        .unwrap_or_else(|| {
            let mut policy = schedule.permission_policy.clone();
            policy.revision = schedule.permission_revision;
            policy
        });
    let launcher = AgentRunLauncher::new(state.agent_store.clone());
    let launch_request = AgentRunLaunchRequest {
        create: create_request,
        policy,
        authority_mode: if inputs.interactive {
            AuthorityMode::Interactive
        } else {
            AuthorityMode::Unattended
        },
    };
    let owner_key = format!("schedule:{}", schedule.id);
    let launched = if let Some(session) = existing_session {
        launcher
            .launch_in_session_with_policy_owner(launch_request, session, owner_key)
            .await
    } else {
        launcher
            .launch_new_with_policy_owner(launch_request, owner_key)
            .await
    }
    .map_err(|error| CommandError::operation("schedule_run_create_failed", error))?;
    Ok((launched, None))
}

async fn ensure_schedule_workspace(
    state: &DesktopState,
    schedule: &ScheduleDefinition,
    spec: &hachimi_protocol::ScheduleWorkspaceSpec,
    timestamp_ms: i64,
) -> Result<hachimi_protocol::AgentWorkspace, CommandError> {
    let owner = hachimi_storage::WorkspaceOwnerRef::Schedule(&schedule.id);
    let workspace_id = state
        .agent_store
        .workspace_for_owner(owner)
        .await
        .map_err(scheduler_error)?
        .map_or_else(hachimi_protocol::WorkspaceId::random, |workspace| {
            workspace.id
        });
    match spec {
        hachimi_protocol::ScheduleWorkspaceSpec::Managed => state
            .agent_store
            .ensure_managed_workspace(workspace_id, owner, timestamp_ms)
            .await
            .map_err(scheduler_error),
        hachimi_protocol::ScheduleWorkspaceSpec::SelectedDirectory { root_path } => {
            reject_selected_workspace_inside_data_root(state, std::path::Path::new(root_path))?;
            state
                .agent_store
                .ensure_selected_workspace(
                    workspace_id,
                    owner,
                    std::path::Path::new(root_path),
                    timestamp_ms,
                )
                .await
                .map_err(|error| {
                    CommandError::new(
                        "schedule_workspace_unavailable",
                        format!("Selected Schedule directory is unavailable: {error}"),
                    )
                })
        }
    }
}

fn reject_selected_workspace_inside_data_root(
    state: &DesktopState,
    root: &std::path::Path,
) -> Result<(), CommandError> {
    let data_root = std::fs::canonicalize(&state.storage_layout.root).map_err(|error| {
        CommandError::operation("schedule_workspace_data_root_unavailable", error)
    })?;
    let selected = std::fs::canonicalize(root)
        .map_err(|error| CommandError::operation("schedule_workspace_unavailable", error))?;
    if selected.starts_with(&data_root) {
        return Err(CommandError::new(
            "schedule_workspace_inside_data_root",
            "Scheduled explicit directories cannot be inside Hachimi managed data",
        ));
    }
    Ok(())
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
        authority,
        capability_grants,
        project_snapshot,
        cancellation: external_cancellation,
    } = execution;
    let executor = state.agent_executor.clone();
    let created_run_id = created.run.id.clone();
    let managed_checkout_id = project_snapshot.as_ref().and_then(|snapshot| {
        snapshot.checkout.as_ref().and_then(|checkout| {
            (checkout.kind == CheckoutKind::ManagedWorktree).then(|| checkout.id.clone())
        })
    });
    if external_cancellation.is_cancelled() {
        return Err(CommandError::new(
            "scheduled_agent_cancelled",
            "the scheduled Agent Run was cancelled before dispatch",
        ));
    }
    let skill_allowlist = schedule.skill_allowlist.clone();
    let mcp_tool_allowlist = schedule.mcp_tool_allowlist.clone();
    let connector_revisions = schedule.host_revision_snapshot.connectors.as_slice();
    let host_revision_snapshot = crate::host_revision_snapshots::snapshot_from_permission_policy(
        &schedule.permission_policy,
        connector_revisions,
    );
    let priority = match authority.mode {
        hachimi_protocol::AuthorityMode::Interactive => AgentRunPriority::Interactive,
        hachimi_protocol::AuthorityMode::Unattended => AgentRunPriority::Background,
    };
    let operation = executor.execute(hachimi_agent::AgentRunRequest {
        principal: client.client_id.0,
        session: created.session,
        run: created.run,
        authority,
        priority,
        user_input_availability: match priority {
            AgentRunPriority::Interactive => hachimi_agent::UserInputAvailability::Available,
            AgentRunPriority::Background => hachimi_agent::UserInputAvailability::Unavailable,
        },
        capability_grants,
        sandbox_snapshot: state.sandbox_snapshot().report,
        attachment_ids: Vec::new(),
        skill_allowlist,
        mcp_tool_allowlist,
        run_tool_allowlist: None,
        host_revision_snapshot,
        workload_override: schedule.workload_override,
        recovery_checkpoint: None,
        parent_agent_task_id: None,
        parent_run_id: None,
        agent_depth: 0,
    });
    tokio::pin!(operation);
    let mut result = tokio::select! {
        execution = &mut operation => execution
            .map_err(|error| CommandError::operation("scheduled_agent_execution_failed", error)),
        () = external_cancellation.cancelled() => {
            if let Some(active) = executor.registry().get(&created_run_id) {
                let _ = executor.registry().cancel(&created_run_id, active.run_generation);
            }
            let _ = state.approval_broker.cancel_run(created_run_id.clone()).await;
            let _ = state.user_input_broker.cancel_run(created_run_id.clone()).await;
            let _ = operation.await;
            Err(CommandError::new(
                "scheduled_agent_cancelled",
                "the scheduled Agent Run was cancelled because its authority changed",
            ))
        }
    };
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
                ItemPayload::Assistant { text, .. } => Some(text.chars().take(2_000).collect()),
                _ => None,
            }
        })
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
        image_input: true,
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
        | RunStatus::Recovering
        | RunStatus::WaitingRecoveryDecision
        | RunStatus::Cancelling
        | RunStatus::Failed => TaskRunStatus::Failed,
    }
}

pub(super) fn scheduled_completion_status(
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

pub(super) fn has_host_revision_attention(
    events: &[hachimi_protocol::RunEventEnvelope],
    run_id: &hachimi_protocol::RunId,
) -> bool {
    events.iter().any(|event| {
        event.run_id.as_ref() == Some(run_id)
            && matches!(
                &event.payload,
                hachimi_protocol::RunEventPayload::Generic { event, .. }
                    if event == crate::host_revision_snapshots::HOST_REVISION_ATTENTION_EVENT
            )
    })
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

fn schedule_creation_error_outcome(
    error: CommandError,
) -> Result<ScheduleRunCompletion, ScheduleLaunchError> {
    let error = command_to_launch_error(error);
    if error.code == "schedule_workspace_unavailable" {
        Ok(needs_attention_completion(error))
    } else {
        Err(error)
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
        schedule_creation_error_outcome, scheduled_completion_status, task_notification_status,
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
                failure_count: 0,
                next_retry_at_ms: None,
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

    #[test]
    fn unavailable_selected_workspace_is_reported_as_needs_attention() {
        let completion = schedule_creation_error_outcome(super::CommandError::new(
            "schedule_workspace_unavailable",
            "selected directory no longer exists",
        ))
        .expect("workspace attention completion");
        assert_eq!(completion.status, TaskRunStatus::NeedsAttention);
        assert_eq!(
            completion.error_code.as_deref(),
            Some("schedule_workspace_unavailable")
        );

        let error = schedule_creation_error_outcome(super::CommandError::new(
            "schedule_run_create_failed",
            "database unavailable",
        ))
        .expect_err("unrelated launch errors remain failures");
        assert_eq!(error.code, "schedule_run_create_failed");
    }
}
