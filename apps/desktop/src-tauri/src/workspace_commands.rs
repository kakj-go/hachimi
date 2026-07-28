use std::time::Duration;

use hachimi_protocol::{
    CheckoutId, DiffReadFileRequest, DiffReadFileResponse, DiffScope, FsFileChunk, FsListPage,
    FsListRequest, FsReadChunkRequest, FsSearchId, FsSearchSnapshot, FsSearchStartRequest,
    FsSearchUpdateRequest, FsWatchId, FsWatchRegistration, FsWatchRequest, GitWorkspaceRequest,
    GitWorkspaceSnapshot, RunDiffSnapshot, RunRecord, SessionId,
};
use hachimi_workspace::{WorkspaceHostClient, WorkspaceOperation, WorkspaceOutput};

use super::*;

const FILE_OPERATION_TIMEOUT: Duration = Duration::from_secs(20);
pub(super) const WORKSPACE_CHANGE_EVENT: &str = "workbench-fs-change";
pub(super) const WORKSPACE_SEARCH_EVENT: &str = "workbench-fs-search";

async fn dispatch_fs(
    window: &WebviewWindow,
    state: &DesktopState,
    request: hachimi_control_plane::FsAppRequest,
) -> Result<hachimi_control_plane::FsAppResponse, CommandError> {
    let client = state.authorize(window, ControlMethod::WorkbenchWindow)?;
    require_window(window, "workbench")?;
    let context = hachimi_control_plane::AppServerContext {
        principal: client.client_id.0.clone(),
        client,
    };
    match state
        .app_server
        .dispatch(
            &context,
            hachimi_control_plane::AppServerRequest::Domain(Box::new(
                hachimi_control_plane::AppServerDomainRequest::Fs(request),
            )),
        )
        .await
        .map_err(|error| CommandError::operation("workspace_app_server_failed", error))?
    {
        hachimi_control_plane::AppServerResponse::Domain(response) => match *response {
            hachimi_control_plane::AppServerDomainResponse::Fs(response) => Ok(response),
            _ => Err(CommandError::new(
                "workspace_app_server_protocol_mismatch",
                "App Server returned a response for a different domain",
            )),
        },
        _ => Err(CommandError::new(
            "workspace_app_server_protocol_mismatch",
            "App Server returned a response for a different domain",
        )),
    }
}

#[derive(Debug, Clone)]
pub(super) struct ActiveWorkspaceWatch {
    pub session_id: SessionId,
    pub checkout_id: CheckoutId,
    pub generation: u64,
    pub cancellation: CancellationToken,
    pub watch: Arc<tokio::sync::Mutex<Option<hachimi_workspace::WorkspaceWatchSession>>>,
}

#[derive(Debug, Clone)]
pub(super) struct ActiveWorkspaceSearch {
    pub session_id: SessionId,
    pub checkout_id: CheckoutId,
    pub generation: u64,
    pub session: hachimi_workspace::WorkspaceSearchSession,
}

pub(super) fn cancel_all_workspace_transients(state: &DesktopState) {
    cancel_all_agent_event_streams(state);
    let watches = std::mem::take(&mut *state.workspace_watches.lock());
    for active in watches.into_values() {
        active.cancellation.cancel();
    }
    let searches = std::mem::take(&mut *state.workspace_searches.lock());
    for active in searches.into_values() {
        active.session.cancel();
    }
}

pub(super) fn cancel_workspace_transients_for_checkout(
    state: &DesktopState,
    checkout_id: &CheckoutId,
) {
    let watch_ids = state
        .workspace_watches
        .lock()
        .iter()
        .filter(|(_, active)| &active.checkout_id == checkout_id)
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    for watch_id in watch_ids {
        if let Some(active) = state.workspace_watches.lock().remove(&watch_id) {
            active.cancellation.cancel();
        }
    }
    let search_ids = state
        .workspace_searches
        .lock()
        .iter()
        .filter(|(_, active)| &active.checkout_id == checkout_id)
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    for search_id in search_ids {
        if let Some(active) = state.workspace_searches.lock().remove(&search_id) {
            active.session.cancel();
        }
    }
}

pub(super) struct ResolvedWorkspace {
    pub(super) session_id: SessionId,
    pub(super) checkout: hachimi_protocol::CheckoutRecord,
    pub(super) run: RunRecord,
}

#[tauri::command]
pub(super) async fn list_workspace_files(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: FsListRequest,
) -> Result<FsListPage, CommandError> {
    match dispatch_fs(
        &window,
        &state,
        hachimi_control_plane::FsAppRequest::List(request),
    )
    .await?
    {
        hachimi_control_plane::FsAppResponse::List(page) => Ok(page),
        _ => Err(CommandError::new(
            "workspace_response_mismatch",
            "expected file list",
        )),
    }
}

#[tauri::command]
pub(super) async fn read_workspace_file_chunk(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: FsReadChunkRequest,
) -> Result<FsFileChunk, CommandError> {
    match dispatch_fs(
        &window,
        &state,
        hachimi_control_plane::FsAppRequest::ReadChunk(request),
    )
    .await?
    {
        hachimi_control_plane::FsAppResponse::FileChunk(chunk) => Ok(chunk),
        _ => Err(CommandError::new(
            "workspace_response_mismatch",
            "expected file chunk",
        )),
    }
}

#[tauri::command]
pub(super) async fn get_workspace_git(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: GitWorkspaceRequest,
) -> Result<GitWorkspaceSnapshot, CommandError> {
    authorize_workspace(&window, &state)?;
    let workspace =
        resolve_session_workspace(&state, &request.session_id, &request.checkout_id).await?;
    let output = workspace_client(&workspace)
        .execute(
            WorkspaceOperation::GitWorkspaceSnapshot {
                history_limit: request.history_limit.clamp(1, 50),
            },
            FILE_OPERATION_TIMEOUT,
            CancellationToken::new(),
        )
        .await
        .map_err(workspace_error)?;
    match output {
        WorkspaceOutput::GitWorkspaceSnapshot { snapshot } => Ok(snapshot),
        _ => Err(unexpected_output("Git workspace snapshot")),
    }
}

#[tauri::command]
pub(super) async fn watch_workspace_files(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: FsWatchRequest,
) -> Result<FsWatchRegistration, CommandError> {
    authorize_workspace(&window, &state)?;
    let registration = match dispatch_fs(
        &window,
        &state,
        hachimi_control_plane::FsAppRequest::Watch(request),
    )
    .await?
    {
        hachimi_control_plane::FsAppResponse::Watch(registration) => registration,
        _ => {
            return Err(CommandError::new(
                "workspace_response_mismatch",
                "expected Watch",
            ));
        }
    };
    let expected_session_id = registration.session_id.clone();
    let expected_checkout_id = registration.checkout_id.clone();
    let slot = state
        .workspace_watches
        .lock()
        .get(&registration.id)
        .map(|active| Arc::clone(&active.watch))
        .ok_or_else(|| {
            CommandError::new("workspace_watch_not_found", "Watch was not registered")
        })?;
    let mut watch = slot.lock().await.take().ok_or_else(|| {
        CommandError::new(
            "workspace_watch_stream_taken",
            "Watch stream is already active",
        )
    })?;
    let watch_id = registration.id.clone();
    let app = window.app_handle().clone();
    let emitter = window.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(result) = watch.recv().await {
            let event = match result {
                Ok(event) => event,
                Err(_) => hachimi_protocol::FsChangeEvent {
                    watch_id: watch_id.clone(),
                    generation: registration.generation,
                    kind: hachimi_protocol::FsChangeKind::Invalidated,
                    paths: Vec::new(),
                    overflowed: true,
                },
            };
            let current = app
                .state::<DesktopState>()
                .workspace_watches
                .lock()
                .get(&watch_id)
                .is_some_and(|active| {
                    active.generation == event.generation
                        && active.session_id == expected_session_id
                        && active.checkout_id == expected_checkout_id
                });
            if !current {
                break;
            }
            let _ = emitter.emit(WORKSPACE_CHANGE_EVENT, &event);
            if event.kind == hachimi_protocol::FsChangeKind::Invalidated {
                break;
            }
        }
        let managed = app.state::<DesktopState>();
        let mut watches = managed.workspace_watches.lock();
        if watches
            .get(&watch_id)
            .is_some_and(|active| active.generation == registration.generation)
        {
            watches.remove(&watch_id);
        }
    });
    Ok(registration)
}

#[tauri::command]
pub(super) async fn unwatch_workspace_files(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    watch_id: FsWatchId,
) -> Result<bool, CommandError> {
    let client = state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    let context = hachimi_control_plane::AppServerContext {
        principal: client.client_id.0.clone(),
        client,
    };
    match state
        .app_server
        .dispatch(
            &context,
            hachimi_control_plane::AppServerRequest::Domain(Box::new(
                hachimi_control_plane::AppServerDomainRequest::Fs(
                    hachimi_control_plane::FsAppRequest::Unwatch(watch_id),
                ),
            )),
        )
        .await
        .map_err(|error| CommandError::operation("workspace_app_server_failed", error))?
    {
        hachimi_control_plane::AppServerResponse::Domain(response) => match *response {
            hachimi_control_plane::AppServerDomainResponse::Fs(
                hachimi_control_plane::FsAppResponse::Unwatched(removed),
            ) => Ok(removed),
            _ => Err(CommandError::new(
                "workspace_response_mismatch",
                "expected Unwatch",
            )),
        },
        _ => Err(CommandError::new(
            "workspace_response_mismatch",
            "expected Unwatch",
        )),
    }
}

#[tauri::command]
pub(super) async fn start_workspace_file_search(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: FsSearchStartRequest,
) -> Result<FsSearchSnapshot, CommandError> {
    let initial = match dispatch_fs(
        &window,
        &state,
        hachimi_control_plane::FsAppRequest::SearchStart(request),
    )
    .await?
    {
        hachimi_control_plane::FsAppResponse::Search(snapshot) => snapshot,
        _ => {
            return Err(CommandError::new(
                "workspace_response_mismatch",
                "expected Search",
            ));
        }
    };
    let active = state
        .workspace_searches
        .lock()
        .get(&initial.search_id)
        .cloned()
        .ok_or_else(|| {
            CommandError::new("workspace_search_not_found", "Search was not registered")
        })?;
    spawn_search_projection(
        window,
        active.session,
        active.session_id,
        active.checkout_id,
    );
    Ok(initial)
}

#[tauri::command]
pub(super) async fn update_workspace_file_search(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: FsSearchUpdateRequest,
) -> Result<FsSearchSnapshot, CommandError> {
    match dispatch_fs(
        &window,
        &state,
        hachimi_control_plane::FsAppRequest::SearchUpdate(request),
    )
    .await?
    {
        hachimi_control_plane::FsAppResponse::Search(snapshot) => Ok(snapshot),
        _ => Err(CommandError::new(
            "workspace_response_mismatch",
            "expected Search",
        )),
    }
}

#[tauri::command]
pub(super) async fn cancel_workspace_file_search(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    search_id: FsSearchId,
) -> Result<bool, CommandError> {
    match dispatch_fs(
        &window,
        &state,
        hachimi_control_plane::FsAppRequest::SearchCancel(search_id),
    )
    .await?
    {
        hachimi_control_plane::FsAppResponse::SearchCancelled(cancelled) => Ok(cancelled),
        _ => Err(CommandError::new(
            "workspace_response_mismatch",
            "expected Search cancel",
        )),
    }
}

#[tauri::command]
pub(super) async fn get_workspace_diff(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    scope: DiffScope,
) -> Result<RunDiffSnapshot, CommandError> {
    match dispatch_fs(
        &window,
        &state,
        hachimi_control_plane::FsAppRequest::DiffGet(scope),
    )
    .await?
    {
        hachimi_control_plane::FsAppResponse::Diff(diff) => Ok(diff),
        _ => Err(CommandError::new(
            "workspace_response_mismatch",
            "expected Diff",
        )),
    }
}

#[tauri::command]
pub(super) async fn read_workspace_diff_file(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: DiffReadFileRequest,
) -> Result<DiffReadFileResponse, CommandError> {
    match dispatch_fs(
        &window,
        &state,
        hachimi_control_plane::FsAppRequest::DiffReadFile(request),
    )
    .await?
    {
        hachimi_control_plane::FsAppResponse::DiffFile(diff) => Ok(diff),
        _ => Err(CommandError::new(
            "workspace_response_mismatch",
            "expected file Diff",
        )),
    }
}

fn spawn_search_projection(
    window: WebviewWindow,
    search: hachimi_workspace::WorkspaceSearchSession,
    expected_session_id: SessionId,
    expected_checkout_id: CheckoutId,
) {
    let app = window.app_handle().clone();
    let emitter = window.clone();
    let search_id = search.search_id.clone();
    let mut updates = search.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            if updates.changed().await.is_err() {
                break;
            }
            let Some(result) = updates.borrow_and_update().clone() else {
                continue;
            };
            let Ok(snapshot) = result else {
                break;
            };
            let current = app
                .state::<DesktopState>()
                .workspace_searches
                .lock()
                .get(&search_id)
                .is_some_and(|active| {
                    active.generation == snapshot.generation
                        && active.session_id == expected_session_id
                        && active.checkout_id == expected_checkout_id
                });
            if current {
                let _ = emitter.emit(WORKSPACE_SEARCH_EVENT, &snapshot);
            }
        }
        let managed = app.state::<DesktopState>();
        let removed = managed.workspace_searches.lock().remove(&search_id);
        if let Some(active) = removed {
            active.session.cancel();
        }
    });
}

pub(super) async fn resolve_session_workspace(
    state: &DesktopState,
    session_id: &SessionId,
    checkout_id: &CheckoutId,
) -> Result<ResolvedWorkspace, CommandError> {
    let store = state.workbench.store();
    let session = store
        .get_session(session_id)
        .await
        .map_err(|error| CommandError::operation("workspace_session_failed", error))?
        .ok_or_else(|| {
            CommandError::new("workspace_session_not_found", "session does not exist")
        })?;
    if session.context.checkout_id() != Some(checkout_id) {
        return Err(CommandError::new(
            "workspace_checkout_mismatch",
            "checkout is not bound to this session",
        ));
    }
    let checkout = store
        .get_checkout(checkout_id)
        .await
        .map_err(|error| CommandError::operation("workspace_checkout_failed", error))?
        .ok_or_else(|| {
            CommandError::new("workspace_checkout_not_found", "checkout does not exist")
        })?;
    let run = store
        .list_runs(session_id)
        .await
        .map_err(|error| CommandError::operation("workspace_runs_failed", error))?
        .into_iter()
        .last()
        .ok_or_else(|| CommandError::new("workspace_run_not_found", "session has no run"))?;
    Ok(ResolvedWorkspace {
        session_id: session.id,
        checkout,
        run,
    })
}

fn workspace_client(workspace: &ResolvedWorkspace) -> WorkspaceHostClient {
    WorkspaceHostClient::new(
        workspace_worker_path(),
        &workspace.checkout.path,
        workspace.checkout.id.as_str(),
        workspace.run.generation,
    )
}

fn authorize_workspace(window: &WebviewWindow, state: &DesktopState) -> Result<(), CommandError> {
    state.authorize(window, ControlMethod::WorkbenchWindow)?;
    require_window(window, "workbench")
}

fn workspace_error(error: hachimi_workspace::WorkspaceError) -> CommandError {
    CommandError::new(
        format!("workspace_{:?}", error.code).to_lowercase(),
        error.message,
    )
}

fn unexpected_output(expected: &str) -> CommandError {
    CommandError::new(
        "workspace_protocol_mismatch",
        format!("workspace worker did not return {expected}"),
    )
}
