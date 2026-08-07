use std::{collections::BTreeMap, sync::Arc};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::future::join_all;
use hachimi_protocol::{
    ComputerAppCandidate, ComputerAppPolicy, ComputerAppPolicyUpdate, ComputerControlSession,
    ComputerFrameId, ComputerFramePreview, ComputerHostSettings, ComputerHostSettingsUpdate,
    ControlMethod, RuntimeComponentId, RuntimeComponentState, SessionId,
};
use sqlx::Row;
use tauri::{State, WebviewWindow};

use crate::{CommandError, DesktopState, require_window};

pub(super) fn start_computer_runtime(supervisor: crate::runtime_supervisor::RuntimeSupervisor) {
    publish_computer_health(&supervisor);
    let retry = supervisor.retry_signal(RuntimeComponentId::ComputerUse);
    tauri::async_runtime::spawn(async move {
        loop {
            retry.notified().await;
            publish_computer_health(&supervisor);
        }
    });
}

fn publish_computer_health(supervisor: &crate::runtime_supervisor::RuntimeSupervisor) {
    let health = hachimi_computer::computer_runtime_health();
    if let Some(code) = health.error_code.as_deref() {
        supervisor.update(
            RuntimeComponentId::ComputerUse,
            RuntimeComponentState::Degraded,
            Some(code),
            true,
            0,
            None,
        );
    } else {
        supervisor.ready(RuntimeComponentId::ComputerUse);
    }
}

#[tauri::command]
pub(super) async fn get_computer_host_settings(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<ComputerHostSettings, CommandError> {
    require_window(&window, "workbench")?;
    let row =
        sqlx::query("SELECT automation_enabled FROM computer_host_settings WHERE singleton = 1")
            .fetch_one(state.agent_store.pool())
            .await
            .map_err(|error| CommandError::operation("computer_settings_load_failed", error))?;
    Ok(ComputerHostSettings {
        automation_enabled: row.get("automation_enabled"),
        runtime_health: hachimi_computer::computer_runtime_health(),
    })
}

#[tauri::command]
pub(super) async fn update_computer_host_settings(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    update: ComputerHostSettingsUpdate,
) -> Result<ComputerHostSettings, CommandError> {
    require_window(&window, "workbench")?;
    sqlx::query(
        "UPDATE computer_host_settings SET automation_enabled = ?, updated_at_ms = ? WHERE singleton = 1",
    )
    .bind(update.automation_enabled)
    .bind(now_ms())
    .execute(state.agent_store.pool())
    .await
    .map_err(|error| CommandError::operation("computer_settings_store_failed", error))?;
    Ok(ComputerHostSettings {
        automation_enabled: update.automation_enabled,
        runtime_health: hachimi_computer::computer_runtime_health(),
    })
}

#[tauri::command]
pub(super) async fn list_computer_app_candidates(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Vec<ComputerAppCandidate>, CommandError> {
    require_window(&window, "workbench")?;
    let windows = state
        .computer_host
        .list_windows()
        .await
        .map_err(|error| CommandError::operation("computer_window_list_failed", error))?;
    let mut apps = BTreeMap::<String, (hachimi_protocol::ComputerAppDescriptor, u32)>::new();
    for window in windows {
        let entry = apps
            .entry(window.app.identity_hash.clone())
            .or_insert((window.app, 0));
        entry.1 = entry.1.saturating_add(1);
    }
    let host = Arc::clone(&state.computer_host);
    let candidates = join_all(apps.into_values().map(|(app, window_count)| {
        let host = Arc::clone(&host);
        async move {
            let icon_png_base64 = match host.app_icon_png(&app).await {
                Ok(Some(bytes)) => Some(STANDARD.encode(bytes)),
                Ok(None) => None,
                Err(error) => {
                    tracing::debug!(
                        app = %app.display_name,
                        error = %error,
                        "Computer Use app icon unavailable"
                    );
                    None
                }
            };
            ComputerAppCandidate {
                app,
                window_count,
                icon_png_base64,
            }
        }
    }))
    .await;
    Ok(candidates)
}

#[tauri::command]
pub(super) async fn list_computer_app_policies(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Vec<ComputerAppPolicy>, CommandError> {
    require_window(&window, "workbench")?;
    state
        .agent_store
        .list_computer_app_policies()
        .await
        .map_err(|error| CommandError::operation("computer_policy_list_failed", error))
}

#[tauri::command]
pub(super) async fn update_computer_app_policy(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    update: ComputerAppPolicyUpdate,
) -> Result<ComputerAppPolicy, CommandError> {
    require_window(&window, "workbench")?;
    let current = state
        .computer_host
        .list_windows()
        .await
        .map_err(|error| CommandError::operation("computer_window_list_failed", error))?
        .into_iter()
        .find(|window| window.app.identity_hash == update.identity_hash)
        .map(|window| window.app);
    let app = if let Some(app) = current {
        app
    } else {
        state
            .agent_store
            .list_computer_app_policies()
            .await
            .map_err(|error| CommandError::operation("computer_policy_list_failed", error))?
            .into_iter()
            .find(|policy| policy.app.identity_hash == update.identity_hash)
            .map(|policy| policy.app)
            .ok_or_else(|| {
                CommandError::new(
                    "computer_app_identity_invalid",
                    "Computer application identity is not a current trusted candidate.",
                )
            })?
    };
    state
        .agent_store
        .upsert_computer_app_policy(&app, update.decision, update.expected_revision)
        .await
        .map_err(|error| CommandError::operation("computer_policy_store_failed", error))
}

#[tauri::command]
pub(super) async fn take_over_computer_control(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    session_id: SessionId,
) -> Result<ComputerControlSession, CommandError> {
    require_window(&window, "workbench")?;
    let epoch = state.computer_host.take_over(&session_id);
    state
        .agent_store
        .set_computer_control_observation(
            &session_id,
            None,
            None,
            epoch,
            "taken_over",
            None,
            now_ms(),
        )
        .await
        .map_err(|error| CommandError::operation("computer_takeover_failed", error))?;
    computer_control(&state, &session_id).await
}

#[tauri::command]
pub(super) async fn resume_computer_control(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    session_id: SessionId,
) -> Result<ComputerControlSession, CommandError> {
    require_window(&window, "workbench")?;
    let current = computer_control(&state, &session_id).await?;
    state
        .agent_store
        .set_computer_control_observation(
            &session_id,
            current.app.as_ref().map(|app| app.app_id.as_str()),
            current
                .window
                .as_ref()
                .map(|window| window.fingerprint.as_str()),
            current
                .latest_frame
                .as_ref()
                .map_or(current.revision, |frame| {
                    frame.input_epoch.saturating_add(1)
                }),
            "observing",
            None,
            now_ms(),
        )
        .await
        .map_err(|error| CommandError::operation("computer_resume_failed", error))?;
    computer_control(&state, &session_id).await
}

#[tauri::command]
pub(super) async fn stop_computer_control(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    session_id: SessionId,
) -> Result<ComputerControlSession, CommandError> {
    require_window(&window, "workbench")?;
    let epoch = state.computer_host.take_over(&session_id);
    state
        .agent_store
        .set_computer_control_observation(&session_id, None, None, epoch, "stopped", None, now_ms())
        .await
        .map_err(|error| CommandError::operation("computer_stop_failed", error))?;
    computer_control(&state, &session_id).await
}

#[tauri::command]
pub(super) async fn get_computer_control_frame(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    session_id: SessionId,
    frame_id: ComputerFrameId,
) -> Result<ComputerFramePreview, CommandError> {
    state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    let frame = state
        .computer_host
        .frame_snapshot(&frame_id)
        .filter(|frame| frame.session_id == session_id)
        .ok_or_else(|| {
            CommandError::new(
                "computer_frame_unavailable",
                "The in-memory Computer frame is no longer available.",
            )
        })?;
    let image = state
        .computer_host
        .frame_image_for_session(&session_id, &frame_id)
        .await
        .map_err(|error| CommandError::operation("computer_frame_unavailable", error))?;
    Ok(ComputerFramePreview {
        frame_id,
        media_type: image.media_type,
        data_base64: STANDARD.encode(image.bytes),
        sha256: image.sha256,
        expires_at_ms: frame.expires_at_ms,
    })
}

async fn computer_control(
    state: &DesktopState,
    session_id: &SessionId,
) -> Result<ComputerControlSession, CommandError> {
    state
        .agent_store
        .list_session_computer_control_sessions(session_id)
        .await
        .map_err(|error| CommandError::operation("computer_control_load_failed", error))?
        .into_iter()
        .next()
        .ok_or_else(|| {
            CommandError::new(
                "computer_control_missing",
                "No Computer control session is available.",
            )
        })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(i64::MAX)
}
