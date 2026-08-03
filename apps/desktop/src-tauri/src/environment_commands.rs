use hachimi_protocol::{
    ControlMethod, SessionId, WorkbenchEnvironmentChangeReason, WorkbenchEnvironmentChanged,
    WorkbenchEnvironmentSnapshot, WorkbenchHandoffRequest, WorkbenchHandoffResponse,
};
use tauri::{AppHandle, Emitter, State, WebviewWindow};

use super::{CommandError, DesktopState, require_window};

pub(super) const WORKBENCH_ENVIRONMENT_EVENT: &str = "workbench:environment-changed";

pub(super) fn emit_workbench_environment(
    app: &AppHandle,
    snapshot: &WorkbenchEnvironmentSnapshot,
    reasons: Vec<WorkbenchEnvironmentChangeReason>,
) {
    let _ = app.emit(
        WORKBENCH_ENVIRONMENT_EVENT,
        WorkbenchEnvironmentChanged {
            session_id: snapshot.session_id.clone(),
            revision: snapshot.revision,
            reasons,
        },
    );
}

#[tauri::command]
pub(super) async fn get_workbench_environment(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    session_id: SessionId,
) -> Result<WorkbenchEnvironmentSnapshot, CommandError> {
    state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    state
        .workbench
        .environment_snapshot(&session_id)
        .await
        .map_err(|error| CommandError::operation("workbench_environment_failed", error))
}

#[tauri::command]
pub(super) async fn handoff_workbench_session(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: WorkbenchHandoffRequest,
) -> Result<WorkbenchHandoffResponse, CommandError> {
    let client = state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    let response = state
        .workbench
        .handoff_session(&request, &client.client_id.0)
        .await
        .map_err(|error| CommandError::operation("workbench_handoff_failed", error))?;
    emit_workbench_environment(
        &app,
        &response.environment,
        vec![
            WorkbenchEnvironmentChangeReason::Binding,
            WorkbenchEnvironmentChangeReason::Git,
            WorkbenchEnvironmentChangeReason::Files,
        ],
    );
    Ok(response)
}
