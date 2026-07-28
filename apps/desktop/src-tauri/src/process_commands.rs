//! Desktop adapter for the Codex-style Process Session protocol.
//!
//! The Registry owns live PTY/pipe handles and bounded replay. This module only
//! authenticates the Workbench caller, resolves the bound Checkout/Run, and
//! persists metadata; it never exposes raw handles or arbitrary host paths.

use hachimi_protocol::{
    ProcessEvent, ProcessListRequest, ProcessReadRequest, ProcessReadSnapshot,
    ProcessResizeRequest, ProcessSessionId, ProcessSessionRecord, ProcessSpawnRequest,
    ProcessTerminateRequest, ProcessWriteRequest,
};
use parking_lot::Mutex;
use std::collections::BTreeSet;
use tauri::{Emitter, Manager, State, WebviewWindow};

use super::{CommandError, ControlMethod, DesktopState, require_window};

pub(super) const PROCESS_EVENT: &str = "workbench-process-event";

#[derive(Debug, Default)]
pub(super) struct ProcessEventBridgeRegistry {
    active: Mutex<BTreeSet<ProcessSessionId>>,
}

impl ProcessEventBridgeRegistry {
    fn claim(&self, process_id: &ProcessSessionId) -> bool {
        self.active.lock().insert(process_id.clone())
    }

    fn release(&self, process_id: &ProcessSessionId) {
        self.active.lock().remove(process_id);
    }
}

async fn dispatch_process(
    window: &WebviewWindow,
    state: &DesktopState,
    request: hachimi_control_plane::ProcessAppRequest,
) -> Result<hachimi_control_plane::ProcessAppResponse, CommandError> {
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
                hachimi_control_plane::AppServerDomainRequest::Process(request),
            )),
        )
        .await
        .map_err(|error| CommandError::operation("process_app_server_failed", error))?
    {
        hachimi_control_plane::AppServerResponse::Domain(response) => match *response {
            hachimi_control_plane::AppServerDomainResponse::Process(response) => Ok(response),
            _ => Err(CommandError::new(
                "process_app_server_protocol_mismatch",
                "App Server returned a response for a different domain",
            )),
        },
        _ => Err(CommandError::new(
            "process_app_server_protocol_mismatch",
            "App Server returned a response for a different domain",
        )),
    }
}

#[tauri::command]
pub(super) async fn spawn_process(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ProcessSpawnRequest,
) -> Result<ProcessSessionRecord, CommandError> {
    match dispatch_process(
        &window,
        &state,
        hachimi_control_plane::ProcessAppRequest::Spawn(request),
    )
    .await?
    {
        hachimi_control_plane::ProcessAppResponse::Process(process) => {
            spawn_process_event_bridge(window.app_handle().clone(), process.id.clone());
            Ok(process)
        }
        _ => Err(CommandError::new(
            "process_response_mismatch",
            "expected Process Session",
        )),
    }
}

fn spawn_process_event_bridge(app: tauri::AppHandle, process_id: ProcessSessionId) {
    let state = app.state::<DesktopState>();
    if !state.process_event_bridges.claim(&process_id) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let state = app.state::<DesktopState>();
        let events = state.process_registry.subscribe(&process_id).await;
        let Ok(mut events) = events else {
            state.process_event_bridges.release(&process_id);
            return;
        };
        loop {
            match events.recv().await {
                Ok(event) => {
                    if let Ok(record) = state.process_registry.get(&process_id).await {
                        let _ = state.agent_store.upsert_process_session(&record).await;
                    }
                    let _ = app.emit(PROCESS_EVENT, &event);
                    if matches!(event, ProcessEvent::Closed { .. }) {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
        state.process_event_bridges.release(&process_id);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_bridge_claim_is_single_owner_until_release() {
        let registry = ProcessEventBridgeRegistry::default();
        let process_id = ProcessSessionId::random();
        assert!(registry.claim(&process_id));
        assert!(!registry.claim(&process_id));
        registry.release(&process_id);
        assert!(registry.claim(&process_id));
    }
}

#[tauri::command]
pub(super) async fn write_process_stdin(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ProcessWriteRequest,
) -> Result<(), CommandError> {
    match dispatch_process(
        &window,
        &state,
        hachimi_control_plane::ProcessAppRequest::Write(request),
    )
    .await?
    {
        hachimi_control_plane::ProcessAppResponse::Acknowledged => Ok(()),
        _ => Err(CommandError::new(
            "process_response_mismatch",
            "expected acknowledgement",
        )),
    }
}

#[tauri::command]
pub(super) async fn resize_process(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ProcessResizeRequest,
) -> Result<(), CommandError> {
    match dispatch_process(
        &window,
        &state,
        hachimi_control_plane::ProcessAppRequest::Resize(request),
    )
    .await?
    {
        hachimi_control_plane::ProcessAppResponse::Acknowledged => Ok(()),
        _ => Err(CommandError::new(
            "process_response_mismatch",
            "expected acknowledgement",
        )),
    }
}

#[tauri::command]
pub(super) async fn terminate_process(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ProcessTerminateRequest,
) -> Result<ProcessSessionRecord, CommandError> {
    match dispatch_process(
        &window,
        &state,
        hachimi_control_plane::ProcessAppRequest::Terminate(request),
    )
    .await?
    {
        hachimi_control_plane::ProcessAppResponse::Process(process) => Ok(process),
        _ => Err(CommandError::new(
            "process_response_mismatch",
            "expected Process Session",
        )),
    }
}

#[tauri::command]
pub(super) async fn read_process(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ProcessReadRequest,
) -> Result<ProcessReadSnapshot, CommandError> {
    match dispatch_process(
        &window,
        &state,
        hachimi_control_plane::ProcessAppRequest::Read(request),
    )
    .await?
    {
        hachimi_control_plane::ProcessAppResponse::Read(snapshot) => Ok(snapshot),
        _ => Err(CommandError::new(
            "process_response_mismatch",
            "expected Process output",
        )),
    }
}

#[tauri::command]
pub(super) async fn list_processes(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ProcessListRequest,
) -> Result<Vec<ProcessSessionRecord>, CommandError> {
    match dispatch_process(
        &window,
        &state,
        hachimi_control_plane::ProcessAppRequest::List(request),
    )
    .await?
    {
        hachimi_control_plane::ProcessAppResponse::Processes(processes) => Ok(processes),
        _ => Err(CommandError::new(
            "process_response_mismatch",
            "expected Process list",
        )),
    }
}
