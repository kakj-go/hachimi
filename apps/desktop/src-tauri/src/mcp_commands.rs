use super::*;

use hachimi_control_plane::{
    AppServerContext, AppServerDomainRequest, AppServerDomainResponse, AppServerRequest,
    AppServerResponse, McpAppRequest, McpAppResponse,
};

pub(super) fn configured_mcp_control(
    store: &AgentStore,
    control_plane: &ControlPlane,
    sandbox_backend: Option<&Arc<dyn SandboxBackend>>,
    data_dir: &Path,
    secrets: McpKeyring,
) -> McpControlService {
    let supervisor = if deterministic_e2e_sandbox_report().is_some() {
        Arc::new(hachimi_capabilities::McpSupervisor::allow_unrestricted_stdio_for_tests())
    } else {
        sandbox_backend.map_or_else(
            || Arc::new(hachimi_capabilities::McpSupervisor::default()),
            |backend| {
                Arc::new(hachimi_capabilities::McpSupervisor::with_stdio_sandbox(
                    hachimi_capabilities::McpStdioSandboxHost::new(
                        Arc::clone(backend),
                        data_dir.join("mcp-hosts"),
                    ),
                ))
            },
        )
    };
    McpControlService::with_secret_resolver(
        store.clone(),
        supervisor,
        Arc::clone(control_plane.capability_registry()),
        Arc::new(secrets),
    )
}

fn require_mcp_window(
    window: &WebviewWindow,
    state: &DesktopState,
) -> Result<AppServerContext, CommandError> {
    let client = state.authorize(window, ControlMethod::ConnectorsManage)?;
    require_window(window, "workbench")?;
    Ok(AppServerContext {
        principal: client.client_id.0.clone(),
        client,
    })
}

async fn dispatch_mcp(
    window: &WebviewWindow,
    state: &DesktopState,
    request: McpAppRequest,
) -> Result<McpAppResponse, CommandError> {
    let context = require_mcp_window(window, state)?;
    match state
        .app_server
        .dispatch(
            &context,
            AppServerRequest::Domain(Box::new(AppServerDomainRequest::Mcp(request))),
        )
        .await
        .map_err(|error| CommandError::operation("mcp_app_server_failed", error))?
    {
        AppServerResponse::Domain(response) => match *response {
            AppServerDomainResponse::Mcp(response) => Ok(response),
            _ => Err(CommandError::new(
                "mcp_app_server_protocol_mismatch",
                "App Server returned a response for a different domain",
            )),
        },
        _ => Err(CommandError::new(
            "mcp_app_server_protocol_mismatch",
            "App Server returned a response for a different domain",
        )),
    }
}

#[tauri::command]
pub(super) async fn get_mcp_inventory(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    server_id: hachimi_protocol::McpServerId,
) -> Result<hachimi_protocol::McpInventorySnapshot, CommandError> {
    match dispatch_mcp(&window, &state, McpAppRequest::Inventory(server_id)).await? {
        McpAppResponse::Inventory(snapshot) => Ok(snapshot),
        _ => Err(CommandError::new(
            "mcp_response_mismatch",
            "expected MCP inventory",
        )),
    }
}

#[tauri::command]
pub(super) async fn refresh_mcp_inventory(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    server_id: hachimi_protocol::McpServerId,
) -> Result<hachimi_protocol::McpInventorySnapshot, CommandError> {
    match dispatch_mcp(&window, &state, McpAppRequest::RefreshInventory(server_id)).await? {
        McpAppResponse::Inventory(snapshot) => Ok(snapshot),
        _ => Err(CommandError::new(
            "mcp_response_mismatch",
            "expected MCP inventory",
        )),
    }
}

#[tauri::command]
pub(super) async fn read_mcp_resource(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: hachimi_protocol::McpResourceReadRequest,
) -> Result<Vec<hachimi_protocol::McpResourceContent>, CommandError> {
    match dispatch_mcp(&window, &state, McpAppRequest::ReadResource(request)).await? {
        McpAppResponse::Resource(content) => Ok(content),
        _ => Err(CommandError::new(
            "mcp_response_mismatch",
            "expected MCP resource",
        )),
    }
}

#[tauri::command]
pub(super) async fn get_mcp_prompt(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: hachimi_protocol::McpPromptGetRequest,
) -> Result<hachimi_protocol::McpPromptResult, CommandError> {
    match dispatch_mcp(&window, &state, McpAppRequest::GetPrompt(request)).await? {
        McpAppResponse::Prompt(prompt) => Ok(prompt),
        _ => Err(CommandError::new(
            "mcp_response_mismatch",
            "expected MCP prompt",
        )),
    }
}

#[tauri::command]
pub(super) async fn list_mcp_call_summaries(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: hachimi_protocol::McpCallSummaryListRequest,
) -> Result<Vec<hachimi_protocol::McpCallSummaryRecord>, CommandError> {
    match dispatch_mcp(&window, &state, McpAppRequest::ListCalls(request)).await? {
        McpAppResponse::Calls(calls) => Ok(calls),
        _ => Err(CommandError::new(
            "mcp_response_mismatch",
            "expected MCP calls",
        )),
    }
}

#[tauri::command]
pub(super) async fn get_mcp_auth_status(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    server_id: hachimi_protocol::McpServerId,
) -> Result<hachimi_protocol::McpAuthStatusRecord, CommandError> {
    match dispatch_mcp(&window, &state, McpAppRequest::AuthStatus(server_id)).await? {
        McpAppResponse::Auth(status) => Ok(status),
        _ => Err(CommandError::new(
            "mcp_response_mismatch",
            "expected MCP auth status",
        )),
    }
}

#[tauri::command]
pub(super) async fn start_mcp_oauth_login(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: hachimi_protocol::McpOAuthLoginRequest,
) -> Result<hachimi_protocol::McpOAuthLoginResponse, CommandError> {
    match dispatch_mcp(&window, &state, McpAppRequest::StartOauth(request)).await? {
        McpAppResponse::Oauth(response) => Ok(response),
        _ => Err(CommandError::new(
            "mcp_response_mismatch",
            "expected MCP OAuth response",
        )),
    }
}

#[tauri::command]
pub(super) async fn logout_mcp_oauth(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    server_id: hachimi_protocol::McpServerId,
) -> Result<hachimi_protocol::McpAuthStatusRecord, CommandError> {
    match dispatch_mcp(&window, &state, McpAppRequest::Logout(server_id)).await? {
        McpAppResponse::Auth(status) => Ok(status),
        _ => Err(CommandError::new(
            "mcp_response_mismatch",
            "expected MCP auth status",
        )),
    }
}
