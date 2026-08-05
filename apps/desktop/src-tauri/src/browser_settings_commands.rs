use super::*;
use hachimi_protocol::{
    BrowserAutomationLease, BrowserAutomationLeaseId, BrowserAutomationLeaseStatus,
    BrowserAutomationPreference, BrowserAutomationSurfaceKind, BrowserHostSettings,
    BrowserHostSettingsUpdate, BrowserPairingId, BrowserSitePolicy, BrowserSitePolicyUpdate,
    HostAccessDecisionRequest, HostAccessRequestRecord, RunStatus, SessionId, SystemBrowserKind,
};
use sqlx::Row;

#[tauri::command]
pub(super) async fn stop_browser_automation(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    lease_id: BrowserAutomationLeaseId,
) -> Result<BrowserAutomationLease, CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    let lease = state
        .agent_store
        .browser_automation_lease(&lease_id)
        .await
        .map_err(|error| CommandError::operation("browser_lease_load_failed", error))?;
    if !matches!(
        lease.status,
        BrowserAutomationLeaseStatus::Active | BrowserAutomationLeaseStatus::Suspended
    ) {
        return Err(CommandError::new(
            "browser_lease_inactive",
            "The Browser automation lease is no longer active.",
        ));
    }
    if lease.surface == BrowserAutomationSurfaceKind::ExternalChrome {
        if let Some(browser_session_id) = state
            .agent_store
            .external_browser_session_for_lease(&lease.id)
            .await
            .map_err(|error| CommandError::operation("browser_lease_load_failed", error))?
        {
            let session = state
                .browser_host
                .stop(&browser_session_id, &lease.owner_run_id)
                .await
                .map_err(|error| CommandError::operation("browser_stop_failed", error))?;
            state
                .agent_store
                .upsert_session_browser(&session)
                .await
                .map_err(|error| CommandError::operation("browser_stop_failed", error))?;
        }
    } else if let Some(tab_id) = lease.tab_id.as_ref() {
        state
            .embedded_browser
            .command(
                &window,
                hachimi_browser::CefHostCommand::ClearAgentNavigationPolicy {
                    tab_id: tab_id.clone(),
                },
            )
            .await
            .map_err(|error| CommandError::operation("browser_stop_failed", error))?;
    }
    let updated = state
        .agent_store
        .set_browser_automation_lease_status(
            &lease.id,
            lease.revision,
            BrowserAutomationLeaseStatus::Expired,
        )
        .await
        .map_err(|error| CommandError::operation("browser_stop_failed", error))?;
    emit_browser_environment(&window, &state, &lease.owner_session_id).await;
    Ok(updated)
}

#[tauri::command]
pub(super) async fn take_over_browser_automation(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    lease_id: BrowserAutomationLeaseId,
) -> Result<BrowserAutomationLease, CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    let lease = state
        .agent_store
        .browser_automation_lease(&lease_id)
        .await
        .map_err(|error| CommandError::operation("browser_lease_load_failed", error))?;
    if lease.status != BrowserAutomationLeaseStatus::Active {
        return Err(CommandError::new(
            "browser_lease_inactive",
            "Only an active Browser automation lease can be taken over.",
        ));
    }
    if lease.surface != BrowserAutomationSurfaceKind::ExternalChrome {
        return Err(CommandError::new(
            "browser_surface_mismatch",
            "Embedded Browser takeover is handled by its persistent Workspace.",
        ));
    }
    let browser_session_id = state
        .agent_store
        .external_browser_session_for_lease(&lease.id)
        .await
        .map_err(|error| CommandError::operation("browser_lease_load_failed", error))?
        .ok_or_else(|| {
            CommandError::new(
                "browser_session_missing",
                "The external Chrome target is unavailable.",
            )
        })?;
    let session = state
        .browser_host
        .take_over(&browser_session_id, &lease.owner_run_id)
        .await
        .map_err(|error| CommandError::operation("browser_takeover_failed", error))?;
    state
        .agent_store
        .upsert_session_browser(&session)
        .await
        .map_err(|error| CommandError::operation("browser_takeover_failed", error))?;
    let updated = state
        .agent_store
        .set_browser_automation_lease_status(
            &lease.id,
            lease.revision,
            BrowserAutomationLeaseStatus::Suspended,
        )
        .await
        .map_err(|error| CommandError::operation("browser_takeover_failed", error))?;
    emit_browser_environment(&window, &state, &lease.owner_session_id).await;
    Ok(updated)
}

#[tauri::command]
pub(super) async fn resume_browser_automation(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    lease_id: BrowserAutomationLeaseId,
) -> Result<BrowserAutomationLease, CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    let lease = state
        .agent_store
        .browser_automation_lease(&lease_id)
        .await
        .map_err(|error| CommandError::operation("browser_lease_load_failed", error))?;
    if lease.surface != BrowserAutomationSurfaceKind::ExternalChrome
        || lease.status != BrowserAutomationLeaseStatus::Suspended
    {
        return Err(CommandError::new(
            "browser_lease_not_suspended",
            "No suspended external Chrome lease is available.",
        ));
    }
    let run = state
        .agent_store
        .get_run(&lease.owner_run_id)
        .await
        .map_err(|error| CommandError::operation("browser_run_failed", error))?
        .ok_or_else(|| CommandError::new("browser_run_missing", "The Agent Run is missing."))?;
    if run.session_id != lease.owner_session_id
        || run.generation != lease.run_generation
        || run.status.is_terminal()
        || run.status == RunStatus::Cancelling
    {
        return Err(CommandError::new(
            "browser_run_stale",
            "The Agent Run can no longer resume Browser control.",
        ));
    }
    let browser_session_id = state
        .agent_store
        .external_browser_session_for_lease(&lease.id)
        .await
        .map_err(|error| CommandError::operation("browser_lease_load_failed", error))?
        .ok_or_else(|| {
            CommandError::new(
                "browser_session_missing",
                "The external Chrome target is unavailable.",
            )
        })?;
    let session = state
        .browser_host
        .resume(&browser_session_id, &lease.owner_run_id)
        .await
        .map_err(|error| CommandError::operation("browser_resume_failed", error))?;
    state
        .agent_store
        .upsert_session_browser(&session)
        .await
        .map_err(|error| CommandError::operation("browser_resume_failed", error))?;
    let updated = match state
        .agent_store
        .set_browser_automation_lease_status(
            &lease.id,
            lease.revision,
            BrowserAutomationLeaseStatus::Active,
        )
        .await
    {
        Ok(updated) => updated,
        Err(error) => {
            let _ = state
                .browser_host
                .take_over(&browser_session_id, &lease.owner_run_id)
                .await;
            return Err(CommandError::operation("browser_resume_failed", error));
        }
    };
    emit_browser_environment(&window, &state, &lease.owner_session_id).await;
    Ok(updated)
}

#[tauri::command]
pub(super) async fn list_browser_site_policies(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Vec<BrowserSitePolicy>, CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    state
        .agent_store
        .list_browser_site_policies()
        .await
        .map_err(|error| CommandError::operation("browser_site_policy_list_failed", error))
}

#[tauri::command]
pub(super) async fn update_browser_site_policy(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    update: BrowserSitePolicyUpdate,
) -> Result<BrowserSitePolicy, CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    let origin = hachimi_browser::normalized_origin(&update.origin)
        .map_err(|error| CommandError::operation("browser_origin_invalid", error))?;
    let private_network = match hachimi_browser::validate_agent_browser_target(&origin, false).await
    {
        Ok(()) => false,
        Err(hachimi_browser::BrowserHostError::PrivateNetworkDenied) => true,
        Err(error) => return Err(CommandError::operation("browser_origin_invalid", error)),
    };
    state
        .agent_store
        .upsert_browser_site_policy(&update, &origin, private_network, false)
        .await
        .map_err(|error| CommandError::operation("browser_site_policy_store_failed", error))
}

#[tauri::command]
pub(super) async fn update_private_browser_site_policy(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    update: BrowserSitePolicyUpdate,
) -> Result<BrowserSitePolicy, CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    if !state.settings.read().developer_mode {
        return Err(CommandError::new(
            "developer_mode_required",
            "Persistent private-network Browser policies require Developer mode.",
        ));
    }
    let origin = hachimi_browser::normalized_origin(&update.origin)
        .map_err(|error| CommandError::operation("browser_origin_invalid", error))?;
    match hachimi_browser::validate_agent_browser_target(&origin, false).await {
        Err(hachimi_browser::BrowserHostError::PrivateNetworkDenied) => {}
        Ok(()) => {
            return Err(CommandError::new(
                "browser_origin_not_private",
                "This Developer command only manages private-network Origins.",
            ));
        }
        Err(error) => return Err(CommandError::operation("browser_origin_invalid", error)),
    }
    state
        .agent_store
        .upsert_browser_site_policy(&update, &origin, true, true)
        .await
        .map_err(|error| CommandError::operation("browser_site_policy_store_failed", error))
}

#[tauri::command]
pub(super) async fn remove_browser_site_policy(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    origin: String,
) -> Result<bool, CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    let origin = hachimi_browser::normalized_origin(&origin)
        .map_err(|error| CommandError::operation("browser_origin_invalid", error))?;
    state
        .agent_store
        .remove_browser_site_policy(&origin)
        .await
        .map_err(|error| CommandError::operation("browser_site_policy_remove_failed", error))
}

#[tauri::command]
pub(super) async fn list_host_access_requests(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    session_id: Option<SessionId>,
) -> Result<Vec<HostAccessRequestRecord>, CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    state
        .agent_store
        .list_host_access_requests(session_id.as_ref())
        .await
        .map_err(|error| CommandError::operation("host_access_request_list_failed", error))
}

#[tauri::command]
pub(super) async fn resolve_host_access_request(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: HostAccessDecisionRequest,
) -> Result<HostAccessRequestRecord, CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    let resolved = state
        .agent_store
        .resolve_host_access_request(&request.request_id, request.decision, false)
        .await
        .map_err(|error| CommandError::operation("host_access_request_resolve_failed", error))?;
    emit_browser_environment(&window, &state, &resolved.owner_session_id).await;
    Ok(resolved)
}

#[tauri::command]
pub(super) async fn get_browser_host_settings(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<BrowserHostSettings, CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    read_browser_host_settings(&state).await
}

#[tauri::command]
pub(super) async fn update_browser_host_settings(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    update: BrowserHostSettingsUpdate,
) -> Result<BrowserHostSettings, CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    if update.automation_preference == BrowserAutomationPreference::ExternalChrome
        && state.browser_host.latest_confirmed_pairing().is_none()
    {
        return Err(CommandError::new(
            "browser_pairing_required",
            "Confirm an unexpired Chrome extension pairing before selecting external Chrome",
        ));
    }
    sqlx::query(
        "UPDATE browser_host_settings SET automation_enabled = ?, automation_preference = ?, updated_at_ms = ? WHERE singleton = 1",
    )
    .bind(update.automation_enabled)
    .bind(preference_key(update.automation_preference))
    .bind(now_ms())
    .execute(state.agent_store.pool())
    .await
    .map_err(|error| CommandError::operation("browser_host_settings_store_failed", error))?;
    read_browser_host_settings(&state).await
}

async fn read_browser_host_settings(
    state: &DesktopState,
) -> Result<BrowserHostSettings, CommandError> {
    let row = sqlx::query(
        "SELECT automation_enabled, automation_preference FROM browser_host_settings WHERE singleton = 1",
    )
    .fetch_one(state.agent_store.pool())
    .await
    .map_err(|error| CommandError::operation("browser_host_settings_load_failed", error))?;
    let automation_preference = match row.get::<String, _>("automation_preference").as_str() {
        "auto" => BrowserAutomationPreference::Auto,
        "embedded" => BrowserAutomationPreference::Embedded,
        "external_chrome" => BrowserAutomationPreference::ExternalChrome,
        _ => {
            return Err(CommandError::new(
                "browser_host_settings_invalid",
                "invalid Browser automation preference",
            ));
        }
    };
    Ok(BrowserHostSettings {
        automation_enabled: row.get("automation_enabled"),
        automation_preference,
        latest_pairing: state.browser_host.latest_confirmed_pairing(),
        pending_authorization: state.browser_host.latest_pending_pairing(),
        detected_browsers: crate::browser_detection::detect_system_browsers(),
    })
}

#[tauri::command]
pub(super) async fn approve_browser_extension(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    pairing_id: BrowserPairingId,
) -> Result<BrowserHostSettings, CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    let pairing = state
        .browser_host
        .approve_extension_authorization(&pairing_id)
        .map_err(|error| {
            CommandError::operation("browser_extension_authorization_failed", error)
        })?;
    let identity = pairing.extension_identity.as_deref().ok_or_else(|| {
        CommandError::new("browser_extension_identity_missing", "扩展安装身份无效。")
    })?;
    crate::browser_extension_trust::trust(identity).map_err(|error| {
        tracing::error!(%error, "Browser extension trust could not be stored");
        CommandError::new(
            "browser_extension_trust_store_failed",
            "无法保存扩展授权，请检查系统凭据存储。",
        )
    })?;
    read_browser_host_settings(&state).await
}

#[tauri::command]
pub(super) fn install_browser_extension(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    browser: SystemBrowserKind,
) -> Result<(), CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    let installation = crate::browser_detection::detect_system_browsers()
        .into_iter()
        .find(|installation| installation.kind == browser)
        .ok_or_else(|| {
            CommandError::new("system_browser_not_found", "未检测到受支持的系统浏览器。")
        })?;
    let url = installation.extension_store_url.ok_or_else(|| {
        CommandError::new(
            "browser_extension_store_unavailable",
            "当前构建未配置正式扩展商店地址。",
        )
    })?;
    open_store_url(&url)
        .map_err(|error| CommandError::operation("browser_extension_store_open_failed", error))
}

#[cfg(windows)]
fn open_store_url(url: &str) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("rundll32.exe")
        .arg("url.dll,FileProtocolHandler")
        .arg(url)
        .creation_flags(0x0800_0000)
        .spawn()
        .map(|_| ())
}

#[cfg(not(windows))]
fn open_store_url(_url: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "unsupported OS",
    ))
}

const fn preference_key(preference: BrowserAutomationPreference) -> &'static str {
    match preference {
        BrowserAutomationPreference::Auto => "auto",
        BrowserAutomationPreference::Embedded => "embedded",
        BrowserAutomationPreference::ExternalChrome => "external_chrome",
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(i64::MAX)
}

async fn emit_browser_environment(
    window: &WebviewWindow,
    state: &DesktopState,
    session_id: &SessionId,
) {
    if let Ok(snapshot) = state.workbench.environment_snapshot(session_id).await {
        crate::environment_commands::emit_workbench_environment(
            window.app_handle(),
            &snapshot,
            vec![hachimi_protocol::WorkbenchEnvironmentChangeReason::Browser],
        );
    }
}
