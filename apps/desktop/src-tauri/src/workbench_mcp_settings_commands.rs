use super::*;

#[tauri::command]
pub(crate) fn get_mcp_echo_server_url(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<String, CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    Ok(state.mcp_echo_server.url().to_owned())
}

#[tauri::command]
pub(crate) async fn list_mcp_servers(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Vec<McpServerView>, CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    state
        .mcp_control
        .list()
        .await
        .map_err(|error| CommandError::operation("mcp_list_failed", error))
}

#[tauri::command]
pub(crate) async fn get_mcp_server(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    server_id: hachimi_protocol::McpServerId,
) -> Result<McpServerView, CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    state
        .mcp_control
        .get(&server_id)
        .await
        .map_err(|error| CommandError::operation("mcp_get_failed", error))
}

#[tauri::command]
pub(crate) async fn upsert_mcp_server(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: McpServerUpsertRequest,
) -> Result<McpServerView, CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    if request.enabled {
        require_mcp_runtime(&state)?;
    }
    let _sandbox_activity = request
        .enabled
        .then(|| enter_sandbox_activity(&state))
        .transpose()?;
    let now = i64::try_from(epoch_millis()).unwrap_or(i64::MAX);
    let existing = state
        .mcp_control
        .list()
        .await
        .map_err(|error| CommandError::operation("mcp_list_failed", error))?
        .into_iter()
        .find(|view| view.configuration.id == request.id)
        .map(|view| view.configuration);
    let previous_headers = existing
        .as_ref()
        .map(|configuration| configuration.headers.as_slice())
        .unwrap_or_default();
    let (headers, created_references) =
        state
            .mcp_secrets
            .prepare_headers(&request.id, &request.headers, previous_headers)?;
    let mut read_only_tools = request.read_only_tools;
    read_only_tools.sort();
    read_only_tools.dedup();
    let record = McpServerRecord {
        id: request.id,
        display_name: request.display_name,
        enabled: request.enabled,
        transport: request.transport,
        headers,
        read_only_tools,
        startup_timeout_ms: request.startup_timeout_ms,
        request_timeout_ms: request.request_timeout_ms,
        max_message_bytes: request.max_message_bytes,
        created_at_ms: existing.as_ref().map_or(now, |record| record.created_at_ms),
        updated_at_ms: now,
    };
    let outcome = state
        .mcp_control
        .upsert(&record)
        .await
        .map_err(|error| CommandError::operation("mcp_upsert_failed", error));
    match outcome {
        Ok(view) => {
            let cleanup_failures = state
                .mcp_secrets
                .cleanup_replaced(previous_headers, &record.headers);
            defer_mcp_secret_cleanup_failures(&state.agent_store, cleanup_failures).await;
            Ok(view)
        }
        Err(error) => {
            let mut cleanup_failures = Vec::new();
            for reference in created_references {
                if state.mcp_secrets.clear(&reference).is_err() {
                    cleanup_failures.push(reference);
                }
            }
            defer_mcp_secret_cleanup_failures(&state.agent_store, cleanup_failures).await;
            Err(error)
        }
    }
}

#[tauri::command]
pub(crate) async fn test_mcp_server(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: McpServerUpsertRequest,
) -> Result<hachimi_protocol::McpConnectionTestResult, CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    require_mcp_runtime(&state)?;
    let _sandbox_activity = enter_sandbox_activity(&state)?;
    let existing = state
        .mcp_control
        .list()
        .await
        .map_err(|error| CommandError::operation("mcp_list_failed", error))?
        .into_iter()
        .find(|view| view.configuration.id == request.id)
        .map(|view| view.configuration);
    let previous_headers = existing
        .as_ref()
        .map(|configuration| configuration.headers.as_slice())
        .unwrap_or_default();
    let resolved_headers = state
        .mcp_secrets
        .resolve_inputs(&request.headers, previous_headers)?;
    let now = i64::try_from(epoch_millis()).unwrap_or(i64::MAX);
    let record = McpServerRecord {
        id: request.id,
        display_name: request.display_name,
        enabled: true,
        transport: request.transport,
        headers: Vec::new(),
        read_only_tools: request.read_only_tools,
        startup_timeout_ms: request.startup_timeout_ms,
        request_timeout_ms: request.request_timeout_ms,
        max_message_bytes: request.max_message_bytes,
        created_at_ms: now,
        updated_at_ms: now,
    };
    Ok(state
        .mcp_control
        .test_connection(&record, resolved_headers)
        .await)
}

#[tauri::command]
pub(crate) async fn list_mcp_tools(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    server_id: hachimi_protocol::McpServerId,
) -> Result<Vec<hachimi_protocol::McpToolView>, CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    state
        .mcp_control
        .list_tools(&server_id)
        .await
        .map_err(|error| CommandError::operation("mcp_tools_failed", error))
}

#[tauri::command]
pub(crate) async fn discover_mcp_tools(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    server_id: hachimi_protocol::McpServerId,
) -> Result<hachimi_protocol::McpConnectionTestResult, CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    require_mcp_runtime(&state)?;
    let _sandbox_activity = enter_sandbox_activity(&state)?;
    state
        .mcp_control
        .discover_tools(&server_id)
        .await
        .map_err(|error| CommandError::operation("mcp_discovery_failed", error))
}

#[tauri::command]
pub(crate) async fn set_mcp_tool_enabled(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    server_id: hachimi_protocol::McpServerId,
    tool_name: String,
    enabled: bool,
) -> Result<hachimi_protocol::McpToolView, CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    state
        .mcp_control
        .set_tool_enabled(
            &server_id,
            &tool_name,
            enabled,
            i64::try_from(epoch_millis()).unwrap_or(i64::MAX),
        )
        .await
        .map_err(|error| CommandError::operation("mcp_tool_enable_failed", error))
}

#[tauri::command]
pub(crate) async fn set_mcp_server_enabled(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    server_id: hachimi_protocol::McpServerId,
    enabled: bool,
) -> Result<McpServerView, CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    require_mcp_runtime(&state)?;
    let _sandbox_activity = enter_sandbox_activity(&state)?;
    state
        .mcp_control
        .set_enabled(
            &server_id,
            enabled,
            i64::try_from(epoch_millis()).unwrap_or(i64::MAX),
        )
        .await
        .map_err(|error| CommandError::operation("mcp_enable_failed", error))
}

#[tauri::command]
pub(crate) async fn refresh_mcp_server(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    server_id: hachimi_protocol::McpServerId,
) -> Result<McpServerHealthRecord, CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    require_mcp_runtime(&state)?;
    let _sandbox_activity = enter_sandbox_activity(&state)?;
    state
        .mcp_control
        .refresh_health(&server_id)
        .await
        .map_err(|error| CommandError::operation("mcp_health_failed", error))
}

#[tauri::command]
pub(crate) async fn remove_mcp_server(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    server_id: hachimi_protocol::McpServerId,
) -> Result<bool, CommandError> {
    state.authorize(&window, ControlMethod::ConnectorsManage)?;
    require_window(&window, "workbench")?;
    let previous = state
        .mcp_control
        .list()
        .await
        .map_err(|error| CommandError::operation("mcp_list_failed", error))?
        .into_iter()
        .find(|view| view.configuration.id == server_id)
        .map(|view| view.configuration.headers)
        .unwrap_or_default();
    let removed = state
        .mcp_control
        .remove(&server_id)
        .await
        .map_err(|error| CommandError::operation("mcp_remove_failed", error))?;
    if removed {
        let cleanup_failures = state.mcp_secrets.cleanup_replaced(&previous, &[]);
        defer_mcp_secret_cleanup_failures(&state.agent_store, cleanup_failures).await;
    }
    Ok(removed)
}
