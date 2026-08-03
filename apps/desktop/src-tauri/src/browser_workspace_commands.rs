use hachimi_browser::{CefBounds, CefHostCommand, normalized_browser_input};
use hachimi_protocol::{
    BrowserAutomationLeaseStatus, BrowserDataKind, BrowserDownloadAction,
    BrowserDownloadActionRequest, BrowserDownloadSnapshot, BrowserDownloadStatus,
    BrowserHistoryEntry, BrowserSurfaceLayoutRequest, BrowserTabId, BrowserWorkspace,
    BrowserWorkspaceId, BrowserWorkspaceMutation, BrowserWorkspaceMutationRequest,
    ClearEmbeddedBrowserDataRequest, EmbeddedBrowserPermissionRequest,
    EmbeddedBrowserPermissionResolutionRequest, EmbeddedBrowserSettings,
    EmbeddedBrowserSettingsUpdate, EmbeddedBrowserSitePermission, RunStatus, SessionId,
};
use hachimi_storage::BrowserTabRuntimeUpdate;
use tauri::{State, WebviewWindow};

use crate::{CommandError, DesktopState, embedded_browser::EmbeddedBrowserError, require_window};

#[tauri::command]
pub(super) async fn open_browser_workspace(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    session_id: SessionId,
    initial_url: Option<String>,
) -> Result<BrowserWorkspace, CommandError> {
    require_window(&window, "workbench")?;
    let initial_url = initial_url
        .as_deref()
        .map(normalized_browser_input)
        .transpose()
        .map_err(|error| CommandError::operation("browser_url_invalid", error))?;
    let workspace = state
        .agent_store
        .get_or_create_browser_workspace(&session_id, initial_url.as_deref())
        .await
        .map_err(|error| CommandError::operation("browser_workspace_failed", error))?;
    state
        .embedded_browser
        .open_workspace(&window, &workspace)
        .await
        .map_err(browser_error)
}

#[tauri::command]
pub(super) async fn mutate_browser_workspace(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: BrowserWorkspaceMutationRequest,
) -> Result<BrowserWorkspace, CommandError> {
    require_window(&window, "workbench")?;
    if request.idempotency_key.trim().is_empty() || request.idempotency_key.len() > 128 {
        return Err(CommandError::new(
            "invalid_idempotency_key",
            "Browser mutation idempotency key must contain 1 to 128 bytes.",
        ));
    }
    let current = state
        .agent_store
        .browser_workspace(&request.workspace_id)
        .await
        .map_err(|error| CommandError::operation("browser_workspace_failed", error))?;
    if current.revision != request.expected_revision {
        return Err(CommandError::new(
            "browser_workspace_revision_conflict",
            "The browser workspace changed. Refresh it before retrying.",
        ));
    }

    match request.mutation {
        BrowserWorkspaceMutation::NewTab { url } => {
            let url = url
                .as_deref()
                .map(normalized_browser_input)
                .transpose()
                .map_err(|error| CommandError::operation("browser_url_invalid", error))?;
            let updated = state
                .agent_store
                .create_browser_tab(&request.workspace_id, current.revision, url.as_deref())
                .await
                .map_err(|error| CommandError::operation("browser_tab_create_failed", error))?;
            let tab = updated
                .tabs
                .iter()
                .find(|tab| tab.id == updated.active_tab_id)
                .ok_or_else(|| {
                    CommandError::new("browser_tab_missing", "The new browser tab was not saved.")
                })?;
            if let Err(error) = state
                .embedded_browser
                .create_tab_runtime(&window, &updated.id, &tab.id, &tab.url)
                .await
            {
                let _ = state
                    .agent_store
                    .close_browser_tab(&updated.id, &tab.id, updated.revision)
                    .await;
                return Err(browser_error(error));
            }
            state
                .embedded_browser
                .command(
                    &window,
                    CefHostCommand::ActivateTab {
                        tab_id: tab.id.clone(),
                    },
                )
                .await
                .map_err(browser_error)?;
            Ok(updated)
        }
        BrowserWorkspaceMutation::ActivateTab { tab_id } => {
            let updated = state
                .agent_store
                .activate_browser_tab(&request.workspace_id, &tab_id, current.revision)
                .await
                .map_err(|error| CommandError::operation("browser_tab_activate_failed", error))?;
            state
                .embedded_browser
                .command(&window, CefHostCommand::ActivateTab { tab_id })
                .await
                .map_err(browser_error)?;
            Ok(updated)
        }
        BrowserWorkspaceMutation::CloseTab { tab_id } => {
            state
                .embedded_browser
                .close_tab_runtime(&window, &tab_id)
                .await
                .map_err(browser_error)?;
            let updated = state
                .agent_store
                .close_browser_tab(&request.workspace_id, &tab_id, current.revision)
                .await
                .map_err(|error| CommandError::operation("browser_tab_close_failed", error))?;
            let active = updated
                .tabs
                .iter()
                .find(|tab| tab.id == updated.active_tab_id)
                .ok_or_else(|| {
                    CommandError::new("browser_tab_missing", "The active browser tab is missing.")
                })?;
            if !state.embedded_browser.is_tab_loaded(&active.id) {
                state
                    .embedded_browser
                    .create_tab_runtime(&window, &updated.id, &active.id, &active.url)
                    .await
                    .map_err(browser_error)?;
            }
            state
                .embedded_browser
                .command(
                    &window,
                    CefHostCommand::ActivateTab {
                        tab_id: active.id.clone(),
                    },
                )
                .await
                .map_err(browser_error)?;
            Ok(updated)
        }
        BrowserWorkspaceMutation::Navigate { tab_id, url } => {
            require_workspace_tab(&current, &tab_id)?;
            suspend_agent_for_user_command(&window, &state, &current, &tab_id).await?;
            let url = normalized_browser_input(&url)
                .map_err(|error| CommandError::operation("browser_url_invalid", error))?;
            state
                .embedded_browser
                .command(
                    &window,
                    CefHostCommand::Navigate {
                        tab_id: tab_id.clone(),
                        url: url.clone(),
                    },
                )
                .await
                .map_err(browser_error)?;
            state
                .agent_store
                .update_browser_tab_runtime(
                    &request.workspace_id,
                    &tab_id,
                    BrowserTabRuntimeUpdate {
                        url: Some(url),
                        loading: Some(true),
                        navigation_error: Some(None),
                        user_input: true,
                        ..BrowserTabRuntimeUpdate::default()
                    },
                )
                .await
                .map_err(|error| CommandError::operation("browser_navigation_failed", error))
        }
        BrowserWorkspaceMutation::Back { tab_id } => {
            browser_control_command(&window, &state, &current, tab_id, CefControl::Back).await
        }
        BrowserWorkspaceMutation::Forward { tab_id } => {
            browser_control_command(&window, &state, &current, tab_id, CefControl::Forward).await
        }
        BrowserWorkspaceMutation::Reload { tab_id } => {
            browser_control_command(&window, &state, &current, tab_id, CefControl::Reload).await
        }
        BrowserWorkspaceMutation::Stop { tab_id } => {
            browser_control_command(&window, &state, &current, tab_id, CefControl::Stop).await
        }
        BrowserWorkspaceMutation::TakeOverAutomation => {
            let active_tab_id = current.active_tab_id.clone();
            suspend_agent_for_user_command(&window, &state, &current, &active_tab_id).await
        }
        BrowserWorkspaceMutation::ResumeAutomation => {
            let lease = current.automation_lease.as_ref().ok_or_else(|| {
                CommandError::new(
                    "browser_lease_unavailable",
                    "No suspended Agent browser lease is available.",
                )
            })?;
            if lease.status != BrowserAutomationLeaseStatus::Suspended {
                return Err(CommandError::new(
                    "browser_lease_not_suspended",
                    "The Agent browser lease is not suspended.",
                ));
            }
            let run = state
                .agent_store
                .get_run(&lease.owner_run_id)
                .await
                .map_err(|error| CommandError::operation("browser_run_failed", error))?
                .ok_or_else(|| {
                    CommandError::new("browser_run_missing", "The Agent Run no longer exists.")
                })?;
            if run.session_id != current.owner_session_id
                || run.generation != lease.run_generation
                || run.status.is_terminal()
                || run.status == RunStatus::Cancelling
            {
                return Err(CommandError::new(
                    "browser_run_stale",
                    "The Agent Run can no longer resume browser control.",
                ));
            }
            let updated = state
                .agent_store
                .transition_browser_workspace_automation(
                    &request.workspace_id,
                    current.revision,
                    BrowserAutomationLeaseStatus::Suspended,
                    BrowserAutomationLeaseStatus::Active,
                )
                .await
                .map_err(|error| CommandError::operation("browser_resume_failed", error))?;
            let allowed_origins = state
                .agent_store
                .embedded_browser_allowed_origins(&lease.owner_session_id, &lease.owner_run_id)
                .await
                .map_err(|error| {
                    CommandError::operation("browser_permission_list_failed", error)
                })?;
            if let Some(tab_id) = lease.tab_id.as_ref() {
                state
                    .embedded_browser
                    .command(
                        &window,
                        CefHostCommand::SetAgentNavigationPolicy {
                            tab_id: tab_id.clone(),
                            allowed_origins,
                        },
                    )
                    .await
                    .map_err(browser_error)?;
            }
            Ok(updated)
        }
    }
}

enum CefControl {
    Back,
    Forward,
    Reload,
    Stop,
}

async fn browser_control_command(
    window: &WebviewWindow,
    state: &State<'_, DesktopState>,
    workspace: &BrowserWorkspace,
    tab_id: BrowserTabId,
    control: CefControl,
) -> Result<BrowserWorkspace, CommandError> {
    require_workspace_tab(workspace, &tab_id)?;
    let workspace = suspend_agent_for_user_command(window, state, workspace, &tab_id).await?;
    let command = match control {
        CefControl::Back => CefHostCommand::Back { tab_id },
        CefControl::Forward => CefHostCommand::Forward { tab_id },
        CefControl::Reload => CefHostCommand::Reload {
            tab_id,
            ignore_cache: false,
        },
        CefControl::Stop => CefHostCommand::Stop { tab_id },
    };
    state
        .embedded_browser
        .command(window, command)
        .await
        .map_err(browser_error)?;
    state
        .agent_store
        .browser_workspace(&workspace.id)
        .await
        .map_err(|error| CommandError::operation("browser_workspace_failed", error))
}

async fn suspend_agent_for_user_command(
    window: &WebviewWindow,
    state: &State<'_, DesktopState>,
    workspace: &BrowserWorkspace,
    tab_id: &BrowserTabId,
) -> Result<BrowserWorkspace, CommandError> {
    let Some(lease) = workspace.automation_lease.as_ref() else {
        return Ok(workspace.clone());
    };
    if lease.status != BrowserAutomationLeaseStatus::Active || lease.tab_id.as_ref() != Some(tab_id)
    {
        return Ok(workspace.clone());
    }
    state
        .embedded_browser
        .command(
            window,
            CefHostCommand::ClearAgentNavigationPolicy {
                tab_id: tab_id.clone(),
            },
        )
        .await
        .map_err(browser_error)?;
    state
        .agent_store
        .transition_browser_workspace_automation(
            &workspace.id,
            workspace.revision,
            BrowserAutomationLeaseStatus::Active,
            BrowserAutomationLeaseStatus::Suspended,
        )
        .await
        .map_err(|error| CommandError::operation("browser_takeover_failed", error))
}

#[tauri::command]
pub(super) async fn update_browser_surface_layout(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: BrowserSurfaceLayoutRequest,
) -> Result<(), CommandError> {
    require_window(&window, "workbench")?;
    let workspace = state
        .agent_store
        .browser_workspace(&request.workspace_id)
        .await
        .map_err(|error| CommandError::operation("browser_workspace_failed", error))?;
    require_workspace_tab(&workspace, &request.tab_id)?;
    let scale = request.bounds.scale_factor;
    if !scale.is_finite() || !(0.5..=4.0).contains(&scale) {
        return Err(CommandError::new(
            "browser_surface_bounds_invalid",
            "Browser surface scale factor is outside the supported range.",
        ));
    }
    let physical = CefBounds {
        x: scale_i32(request.bounds.x, scale)?,
        y: scale_i32(request.bounds.y, scale)?,
        width: scale_u32(request.bounds.width, scale)?,
        height: scale_u32(request.bounds.height, scale)?,
    };
    state
        .embedded_browser
        .update_layout(
            &window,
            &request.tab_id,
            physical,
            request.visible,
            request.layout_revision,
        )
        .await
        .map_err(browser_error)
}

#[tauri::command]
pub(super) async fn get_browser_history(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    query: String,
    limit: u32,
) -> Result<Vec<BrowserHistoryEntry>, CommandError> {
    require_window(&window, "workbench")?;
    state
        .agent_store
        .browser_history(&query, limit)
        .await
        .map_err(|error| CommandError::operation("browser_history_failed", error))
}

#[tauri::command]
pub(super) async fn get_embedded_browser_settings(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<EmbeddedBrowserSettings, CommandError> {
    require_window(&window, "workbench")?;
    let full_cdp_access_allowed = state.settings.read().developer_mode;
    state
        .agent_store
        .embedded_browser_settings(full_cdp_access_allowed)
        .await
        .map_err(|error| CommandError::operation("browser_settings_failed", error))
}

#[tauri::command]
pub(super) async fn choose_browser_download_directory(
    window: WebviewWindow,
) -> Result<Option<String>, CommandError> {
    require_window(&window, "workbench")?;
    Ok(rfd::AsyncFileDialog::new()
        .pick_folder()
        .await
        .map(|folder| folder.path().to_string_lossy().into_owned()))
}

#[tauri::command]
pub(super) async fn update_embedded_browser_settings(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    update: EmbeddedBrowserSettingsUpdate,
) -> Result<EmbeddedBrowserSettings, CommandError> {
    require_window(&window, "workbench")?;
    let full_cdp_access_allowed = state.settings.read().developer_mode;
    if update.full_cdp_access && !full_cdp_access_allowed {
        return Err(CommandError::new(
            "browser_full_cdp_not_allowed",
            "Enable application Developer mode and restart before enabling full CDP access.",
        ));
    }
    let download_directory = update
        .download_directory
        .as_deref()
        .map(validate_download_directory)
        .transpose()?;
    let update = EmbeddedBrowserSettingsUpdate {
        download_directory,
        ..update
    };
    let settings = state
        .agent_store
        .update_embedded_browser_settings(&update, full_cdp_access_allowed)
        .await
        .map_err(|error| CommandError::operation("browser_settings_update_failed", error))?;
    state
        .embedded_browser
        .command(
            &window,
            CefHostCommand::ConfigureDownloads {
                directory: settings.download_directory.clone(),
                ask_where_to_save: settings.ask_where_to_save_downloads,
            },
        )
        .await
        .map_err(browser_error)?;
    Ok(settings)
}

#[tauri::command]
pub(super) async fn clear_embedded_browser_data(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ClearEmbeddedBrowserDataRequest,
) -> Result<bool, CommandError> {
    require_window(&window, "workbench")?;
    if request.data.is_empty() {
        return Err(CommandError::new(
            "browser_data_selection_empty",
            "Select at least one browser data type to clear.",
        ));
    }
    let history = request.data.contains(&BrowserDataKind::History);
    let cookies = request.data.contains(&BrowserDataKind::Cookies);
    let cache = request.data.contains(&BrowserDataKind::Cache);
    if history {
        state
            .agent_store
            .clear_embedded_browser_history()
            .await
            .map_err(|error| CommandError::operation("browser_history_clear_failed", error))?;
    }
    if cookies || cache {
        state
            .embedded_browser
            .command(
                &window,
                CefHostCommand::ClearBrowsingData { cookies, cache },
            )
            .await
            .map_err(browser_error)?;
    }
    Ok(true)
}

#[tauri::command]
pub(super) async fn get_browser_downloads(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    workspace_id: BrowserWorkspaceId,
    limit: u32,
) -> Result<Vec<BrowserDownloadSnapshot>, CommandError> {
    require_window(&window, "workbench")?;
    state
        .agent_store
        .browser_downloads(&workspace_id, limit)
        .await
        .map_err(|error| CommandError::operation("browser_downloads_failed", error))
}

#[tauri::command]
pub(super) async fn manage_browser_download(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: BrowserDownloadActionRequest,
) -> Result<BrowserDownloadSnapshot, CommandError> {
    require_window(&window, "workbench")?;
    if request.download_id.trim().is_empty() || request.download_id.len() > 512 {
        return Err(CommandError::new(
            "browser_download_id_invalid",
            "Browser download id is invalid.",
        ));
    }
    let download = state
        .agent_store
        .browser_download(&request.download_id)
        .await
        .map_err(|error| CommandError::operation("browser_download_missing", error))?;
    if download.workspace_id != request.workspace_id {
        return Err(CommandError::new(
            "browser_download_ownership_mismatch",
            "Browser download does not belong to this workspace.",
        ));
    }
    match request.action {
        BrowserDownloadAction::Cancel => {
            if !matches!(
                download.status,
                BrowserDownloadStatus::Pending | BrowserDownloadStatus::InProgress
            ) {
                return Err(CommandError::new(
                    "browser_download_not_active",
                    "Browser download is no longer active.",
                ));
            }
            let prefix = format!("cef:{}:", download.tab_id);
            let runtime_id = download
                .id
                .strip_prefix(&prefix)
                .and_then(|value| value.parse::<u32>().ok())
                .ok_or_else(|| {
                    CommandError::new(
                        "browser_download_id_invalid",
                        "Browser download runtime id is invalid.",
                    )
                })?;
            state
                .embedded_browser
                .command(
                    &window,
                    CefHostCommand::CancelDownload {
                        tab_id: download.tab_id.clone(),
                        download_id: runtime_id,
                    },
                )
                .await
                .map_err(browser_error)?;
        }
        BrowserDownloadAction::OpenInFolder => {
            if download.status != BrowserDownloadStatus::Completed {
                return Err(CommandError::new(
                    "browser_download_incomplete",
                    "Only completed downloads can be opened in Explorer.",
                ));
            }
            let destination = download.destination.as_deref().ok_or_else(|| {
                CommandError::new(
                    "browser_download_destination_missing",
                    "Browser download destination is unavailable.",
                )
            })?;
            open_path_in_folder(destination)
                .map_err(|error| CommandError::operation("browser_download_open_failed", error))?;
        }
    }
    Ok(download)
}

#[tauri::command]
pub(super) async fn list_embedded_browser_permission_requests(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    session_id: Option<SessionId>,
) -> Result<Vec<EmbeddedBrowserPermissionRequest>, CommandError> {
    require_window(&window, "workbench")?;
    state
        .agent_store
        .embedded_browser_permission_requests(session_id.as_ref())
        .await
        .map_err(|error| CommandError::operation("browser_permission_list_failed", error))
}

#[tauri::command]
pub(super) async fn list_embedded_browser_site_permissions(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Vec<EmbeddedBrowserSitePermission>, CommandError> {
    require_window(&window, "workbench")?;
    state
        .agent_store
        .embedded_browser_site_permissions()
        .await
        .map_err(|error| CommandError::operation("browser_permission_list_failed", error))
}

#[tauri::command]
pub(super) async fn resolve_embedded_browser_permission(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: EmbeddedBrowserPermissionResolutionRequest,
) -> Result<EmbeddedBrowserPermissionRequest, CommandError> {
    require_window(&window, "workbench")?;
    state
        .agent_store
        .resolve_embedded_browser_permission_request(&request.request_id, request.decision)
        .await
        .map_err(|error| CommandError::operation("browser_permission_resolve_failed", error))
}

#[tauri::command]
pub(super) async fn revoke_embedded_browser_site_permission(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    permission_id: String,
) -> Result<bool, CommandError> {
    require_window(&window, "workbench")?;
    if permission_id.trim().is_empty() || permission_id.len() > 256 {
        return Err(CommandError::new(
            "browser_permission_id_invalid",
            "Browser permission id is invalid.",
        ));
    }
    state
        .agent_store
        .revoke_embedded_browser_site_permission(&permission_id)
        .await
        .map_err(|error| CommandError::operation("browser_permission_revoke_failed", error))
}

#[tauri::command]
pub(super) fn open_system_browser(
    window: WebviewWindow,
    address: String,
) -> Result<(), CommandError> {
    require_window(&window, "workbench")?;
    let url = normalized_browser_input(&address)
        .map_err(|error| CommandError::operation("browser_url_invalid", error))?;
    if url == "about:blank" {
        return Err(CommandError::new(
            "browser_url_invalid",
            "A blank page cannot be opened in the system browser.",
        ));
    }
    open_url_with_system_handler(&url)
        .map_err(|error| CommandError::operation("system_browser_open_failed", error))
}

fn require_workspace_tab(
    workspace: &BrowserWorkspace,
    tab_id: &BrowserTabId,
) -> Result<(), CommandError> {
    workspace
        .tabs
        .iter()
        .any(|tab| &tab.id == tab_id)
        .then_some(())
        .ok_or_else(|| {
            CommandError::new(
                "browser_tab_missing",
                "The browser tab does not belong to this workspace.",
            )
        })
}

fn scale_i32(value: i32, scale: f64) -> Result<i32, CommandError> {
    let scaled = f64::from(value) * scale;
    if !scaled.is_finite() || scaled < f64::from(i32::MIN) || scaled > f64::from(i32::MAX) {
        return Err(CommandError::new(
            "browser_surface_bounds_invalid",
            "Browser surface position overflowed.",
        ));
    }
    Ok(scaled.round() as i32)
}

fn scale_u32(value: u32, scale: f64) -> Result<u32, CommandError> {
    let scaled = f64::from(value) * scale;
    if !scaled.is_finite() || !(1.0..=16_384.0).contains(&scaled) {
        return Err(CommandError::new(
            "browser_surface_bounds_invalid",
            "Browser surface size is outside the supported range.",
        ));
    }
    Ok(scaled.round() as u32)
}

fn browser_error(error: EmbeddedBrowserError) -> CommandError {
    CommandError::new(error.code(), error.to_string())
}

fn validate_download_directory(value: &str) -> Result<String, CommandError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 32_768 || value.contains('\0') {
        return Err(CommandError::new(
            "browser_download_directory_invalid",
            "The browser download directory is invalid.",
        ));
    }
    let path = std::path::Path::new(value);
    if !path.is_absolute() || !path.is_dir() {
        return Err(CommandError::new(
            "browser_download_directory_invalid",
            "The browser download directory must be an existing absolute directory.",
        ));
    }
    path.canonicalize()
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|error| CommandError::operation("browser_download_directory_invalid", error))
}

#[cfg(windows)]
fn open_url_with_system_handler(url: &str) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("rundll32.exe")
        .arg("url.dll,FileProtocolHandler")
        .arg(url)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
}

#[cfg(windows)]
fn open_path_in_folder(path: &str) -> std::io::Result<()> {
    use std::{os::windows::process::CommandExt, path::Path};
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let path = Path::new(path);
    if !path.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "downloaded file no longer exists",
        ));
    }
    std::process::Command::new("explorer.exe")
        .arg(format!("/select,{}", path.display()))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
}

#[cfg(not(windows))]
fn open_url_with_system_handler(_url: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "system browser integration is only enabled on Windows",
    ))
}

#[cfg(not(windows))]
fn open_path_in_folder(_path: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "download folder integration is only enabled on Windows",
    ))
}
