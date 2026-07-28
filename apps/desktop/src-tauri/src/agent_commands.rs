//! Tauri adapter for the transport-neutral Agent lifecycle service.

use hachimi_protocol::{
    ControlInitializeRequest, ControlInitializeResponse, EventSubscriptionId,
    EventSubscriptionRequest, EventSubscriptionSnapshot, RunControlRequest, RunRecord,
    RunSteerRecord, SessionForkRequest, SessionMetadataUpdateRequest, SessionPage, SessionRecord,
    SessionResumeRequest, SessionResumeSnapshot, SessionSearchRequest, UserInputRequestRecord,
    UserInputResolution,
};
use tauri::{Emitter, State, WebviewWindow};
use tokio_util::sync::CancellationToken;

use super::{CommandError, ControlMethod, DesktopState, require_window};
use hachimi_control_plane::{AppServerContext, AppServerRequest, AppServerResponse};

pub(super) const AGENT_EVENT_BATCH: &str = "agent:events";

fn authorize(
    window: &WebviewWindow,
    state: &DesktopState,
) -> Result<hachimi_protocol::ClientContext, CommandError> {
    let client = state.authorize(window, ControlMethod::WorkbenchWindow)?;
    require_window(window, "workbench")?;
    Ok(client)
}

fn lifecycle_error(error: impl std::fmt::Display) -> CommandError {
    CommandError::operation("agent_lifecycle_failed", error)
}

fn app_context(client: &hachimi_protocol::ClientContext) -> AppServerContext {
    AppServerContext {
        client: client.clone(),
        principal: client.client_id.0.clone(),
    }
}

#[tauri::command]
pub(super) async fn initialize_agent_control(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ControlInitializeRequest,
) -> Result<ControlInitializeResponse, CommandError> {
    let client = authorize(&window, &state)?;
    let context = app_context(&client);
    let AppServerResponse::Initialized(mut response) = state
        .app_server
        .dispatch(&context, AppServerRequest::Initialize(request))
        .await
        .map_err(lifecycle_error)?
    else {
        return Err(CommandError::new(
            "agent_response_mismatch",
            "initialize response mismatch",
        ));
    };
    response.sandbox = state.sandbox_snapshot().report;
    Ok(response)
}

#[tauri::command]
pub(super) async fn search_agent_sessions(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: SessionSearchRequest,
) -> Result<SessionPage, CommandError> {
    let client = authorize(&window, &state)?;
    let AppServerResponse::Sessions(page) = state
        .app_server
        .dispatch(
            &app_context(&client),
            AppServerRequest::SearchSessions(request),
        )
        .await
        .map_err(lifecycle_error)?
    else {
        return Err(CommandError::new(
            "agent_response_mismatch",
            "session search response mismatch",
        ));
    };
    Ok(page)
}

#[tauri::command]
pub(super) async fn resume_agent_session(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: SessionResumeRequest,
) -> Result<SessionResumeSnapshot, CommandError> {
    let client = authorize(&window, &state)?;
    let AppServerResponse::Resumed(snapshot) = state
        .app_server
        .dispatch(
            &app_context(&client),
            AppServerRequest::ResumeSession(request),
        )
        .await
        .map_err(lifecycle_error)?
    else {
        return Err(CommandError::new(
            "agent_response_mismatch",
            "session resume response mismatch",
        ));
    };
    Ok(*snapshot)
}

#[tauri::command]
pub(super) async fn fork_agent_session(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: SessionForkRequest,
) -> Result<SessionRecord, CommandError> {
    let client = authorize(&window, &state)?;
    let AppServerResponse::Session(session) = state
        .app_server
        .dispatch(
            &app_context(&client),
            AppServerRequest::ForkSession(request),
        )
        .await
        .map_err(lifecycle_error)?
    else {
        return Err(CommandError::new(
            "agent_response_mismatch",
            "session fork response mismatch",
        ));
    };
    Ok(session)
}

#[tauri::command]
pub(super) async fn update_agent_session_metadata(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: SessionMetadataUpdateRequest,
) -> Result<SessionRecord, CommandError> {
    let client = authorize(&window, &state)?;
    let AppServerResponse::Session(session) = state
        .app_server
        .dispatch(
            &app_context(&client),
            AppServerRequest::UpdateSession(request),
        )
        .await
        .map_err(lifecycle_error)?
    else {
        return Err(CommandError::new(
            "agent_response_mismatch",
            "session update response mismatch",
        ));
    };
    Ok(session)
}

#[tauri::command]
pub(super) async fn steer_agent_run(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: RunControlRequest,
) -> Result<RunSteerRecord, CommandError> {
    let client = authorize(&window, &state)?;
    let AppServerResponse::Steer(record) = state
        .app_server
        .dispatch(&app_context(&client), AppServerRequest::SteerRun(request))
        .await
        .map_err(lifecycle_error)?
    else {
        return Err(CommandError::new(
            "agent_response_mismatch",
            "run steer response mismatch",
        ));
    };
    Ok(record)
}

#[tauri::command]
pub(super) async fn interrupt_agent_run(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: RunControlRequest,
) -> Result<RunRecord, CommandError> {
    let client = authorize(&window, &state)?;
    let AppServerResponse::Interrupted(run) = state
        .app_server
        .dispatch(
            &app_context(&client),
            AppServerRequest::PrepareInterrupt(request.clone()),
        )
        .await
        .map_err(lifecycle_error)?
    else {
        return Err(CommandError::new(
            "agent_response_mismatch",
            "run interrupt response mismatch",
        ));
    };
    let expected_generation = request
        .context
        .expected_generation
        .ok_or_else(|| CommandError::new("run_precondition_failed", "generation is required"))?;
    super::cancel_workbench_run(window, state, run.id, expected_generation).await
}

#[tauri::command]
pub(super) async fn subscribe_agent_events(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: EventSubscriptionRequest,
) -> Result<EventSubscriptionSnapshot, CommandError> {
    let client = authorize(&window, &state)?;
    let AppServerResponse::Subscription(snapshot) = state
        .app_server
        .dispatch(
            &app_context(&client),
            AppServerRequest::SubscribeEvents(request),
        )
        .await
        .map_err(lifecycle_error)?
    else {
        return Err(CommandError::new(
            "agent_response_mismatch",
            "event subscription response mismatch",
        ));
    };
    let cancellation = CancellationToken::new();
    state
        .agent_event_streams
        .lock()
        .insert(snapshot.subscription.id.clone(), cancellation.clone());
    let receiver = state
        .app_server
        .open_event_stream(&snapshot.subscription.id, cancellation.clone())
        .map_err(lifecycle_error)?;
    spawn_agent_event_stream(window, receiver, cancellation);
    Ok(snapshot)
}

#[tauri::command]
pub(super) async fn unsubscribe_agent_events(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    subscription_id: EventSubscriptionId,
) -> Result<bool, CommandError> {
    let client = authorize(&window, &state)?;
    if let Some(cancellation) = state.agent_event_streams.lock().remove(&subscription_id) {
        cancellation.cancel();
    }
    let AppServerResponse::Unsubscribed(result) = state
        .app_server
        .dispatch(
            &app_context(&client),
            AppServerRequest::UnsubscribeEvents(subscription_id),
        )
        .await
        .map_err(lifecycle_error)?
    else {
        return Err(CommandError::new(
            "agent_response_mismatch",
            "event unsubscribe response mismatch",
        ));
    };
    Ok(result)
}

pub(super) fn cancel_all_agent_event_streams(state: &DesktopState) {
    let streams = std::mem::take(&mut *state.agent_event_streams.lock());
    for cancellation in streams.into_values() {
        cancellation.cancel();
    }
    state
        .app_server
        .unsubscribe_client(&hachimi_protocol::ClientId("window:workbench".into()));
}

fn spawn_agent_event_stream(
    window: WebviewWindow,
    mut receiver: tokio::sync::mpsc::Receiver<EventSubscriptionSnapshot>,
    cancellation: CancellationToken,
) {
    tauri::async_runtime::spawn(async move {
        loop {
            let batch = tokio::select! {
                () = cancellation.cancelled() => break,
                batch = receiver.recv() => batch,
            };
            let Some(batch) = batch else {
                break;
            };
            if window.emit(AGENT_EVENT_BATCH, &batch).is_err() {
                break;
            }
        }
    });
}

#[tauri::command]
pub(super) async fn list_pending_user_input(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    session_id: Option<hachimi_protocol::SessionId>,
) -> Result<Vec<UserInputRequestRecord>, CommandError> {
    authorize(&window, &state)?;
    state
        .workbench
        .store()
        .list_pending_user_inputs(session_id.as_ref())
        .await
        .map_err(|error| CommandError::operation("user_input_list_failed", error))
}

#[tauri::command]
pub(super) async fn resolve_user_input(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    mut resolution: UserInputResolution,
) -> Result<UserInputRequestRecord, CommandError> {
    let client = authorize(&window, &state)?;
    resolution.resolved_by = client.client_id.0.clone();
    let principal = resolution.resolved_by.clone();
    let AppServerResponse::UserInput(record) = state
        .app_server
        .dispatch(
            &AppServerContext { client, principal },
            AppServerRequest::ResolveUserInput(resolution),
        )
        .await
        .map_err(|error| CommandError::operation("user_input_resolve_failed", error))?
    else {
        return Err(CommandError::new(
            "agent_response_mismatch",
            "user input resolve response mismatch",
        ));
    };
    Ok(record)
}

#[tauri::command]
pub(super) async fn cancel_user_input(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: RunControlRequest,
) -> Result<RunRecord, CommandError> {
    interrupt_agent_run(window, state, request).await
}
