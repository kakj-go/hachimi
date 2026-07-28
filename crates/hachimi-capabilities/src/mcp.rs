// SPDX-License-Identifier: Apache-2.0
// Adapted from OpenAI Codex commit 4c43465133428898aa84f0bfc02c306ed65fb66a:
// codex-rs/rmcp-client/src/* and codex-rs/app-server-protocol/src/protocol/v2/mcp.rs.
//! Local stdio MCP transport. Remote transports and credential injection are intentionally absent.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use hachimi_protocol::{
    McpPrompt, McpPromptResult, McpResource, McpResourceContent, McpResourceTemplate,
};
use hachimi_sandbox::{SandboxBackend, SandboxLaunchSpec};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use specta::Type;
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use crate::mcp_elicitation::{McpRunCorrelation, McpServerRequestHandler, dispatch_server_request};
use crate::mcp_inventory::{
    MAX_INVENTORY_PAGES, MAX_PROMPT_COUNT, MAX_RESOURCE_COUNT, MAX_TEMPLATE_COUNT, McpResourcePage,
    McpResourceTemplatePage, next_cursor, parse_prompt, parse_prompt_result, parse_resource,
    parse_resource_contents, parse_resource_page, parse_resource_template,
    parse_resource_template_page, validate_cursor,
};
use crate::mcp_progress::{McpProgressHandler, dispatch_progress_notification};

pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const DEFAULT_MAX_MESSAGE_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_TOOL_COUNT: usize = 512;
pub(crate) const MAX_TOOL_PAGES: usize = 32;
pub(crate) const MAX_SERVER_TEXT_CHARS: usize = 4_096;

#[derive(Debug, Clone)]
pub struct McpStdioServerConfig {
    pub server_id: String,
    pub command: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: Option<PathBuf>,
    pub startup_timeout: Duration,
    pub request_timeout: Duration,
    pub max_message_bytes: usize,
}

impl McpStdioServerConfig {
    #[must_use]
    pub fn new(server_id: impl Into<String>, command: impl Into<PathBuf>) -> Self {
        Self {
            server_id: server_id.into(),
            command: command.into(),
            args: Vec::new(),
            cwd: None,
            startup_timeout: Duration::from_secs(15),
            request_timeout: Duration::from_secs(60),
            max_message_bytes: DEFAULT_MAX_MESSAGE_BYTES,
        }
    }

    fn validate(&self) -> Result<(), McpClientError> {
        if !valid_server_id(&self.server_id) {
            return Err(McpClientError::InvalidConfiguration(
                "server ID must contain 1-64 ASCII letters, numbers, '.', '_' or '-'".into(),
            ));
        }
        if self.command.as_os_str().is_empty() {
            return Err(McpClientError::InvalidConfiguration(
                "stdio server command is empty".into(),
            ));
        }
        if self.args.len() > 128 {
            return Err(McpClientError::InvalidConfiguration(
                "stdio server has too many arguments".into(),
            ));
        }
        if self.startup_timeout.is_zero() || self.request_timeout.is_zero() {
            return Err(McpClientError::InvalidConfiguration(
                "MCP timeouts must be greater than zero".into(),
            ));
        }
        if !(4 * 1024..=16 * 1024 * 1024).contains(&self.max_message_bytes) {
            return Err(McpClientError::InvalidConfiguration(
                "MCP message budget must be between 4 KiB and 16 MiB".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpServerInfo {
    pub server_id: String,
    pub name: String,
    pub version: String,
    pub protocol_version: String,
    pub tools_supported: bool,
    pub resources_supported: bool,
    pub resource_templates_supported: bool,
    pub prompts_supported: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpToolDefinition {
    pub name: String,
    pub description: Option<String>,
    #[specta(type = specta_typescript::Unknown)]
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpCallResult {
    #[specta(type = Vec<specta_typescript::Unknown>)]
    pub content: Vec<Value>,
    #[specta(type = Option<specta_typescript::Unknown>)]
    pub structured_content: Option<Value>,
    pub is_error: bool,
}

#[must_use]
pub fn mcp_exposed_tool_name(server_id: &str, tool_name: &str) -> String {
    let safe_component = |value: &str, max_chars: usize| {
        value
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                    character
                } else {
                    '_'
                }
            })
            .take(max_chars)
            .collect::<String>()
    };
    let hash = Sha256::digest(format!("{server_id}\0{tool_name}").as_bytes());
    let suffix = hash[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!(
        "mcp_{}_{}_{}",
        safe_component(server_id, 16),
        safe_component(tool_name, 30),
        suffix
    )
}

#[derive(Debug, Error)]
pub enum McpClientError {
    #[error("invalid MCP configuration: {0}")]
    InvalidConfiguration(String),
    #[error("failed to start MCP server: {0}")]
    Spawn(String),
    #[error("MCP stdio host sandbox rejected the process: {0}")]
    HostSandbox(&'static str),
    #[error("MCP server is disconnected")]
    Disconnected,
    #[error("MCP request timed out")]
    TimedOut,
    #[error("MCP request was cancelled")]
    Cancelled,
    #[error("MCP transport failed: {0}")]
    Transport(String),
    #[error("MCP server emitted an invalid response: {0}")]
    InvalidResponse(&'static str),
    #[error("MCP server rejected the request ({code}): {message}")]
    Rpc { code: i64, message: String },
    #[error("MCP server selected unsupported protocol version: {0}")]
    UnsupportedProtocol(String),
    #[error("MCP tool definition is invalid: {0}")]
    InvalidTool(String),
    #[error("MCP resource inventory is invalid: {0}")]
    InvalidInventory(String),
    #[error("MCP prompt is invalid: {0}")]
    InvalidPrompt(String),
}

impl McpClientError {
    #[must_use]
    pub const fn stable_code(&self) -> &'static str {
        match self {
            Self::InvalidConfiguration(_) => "invalid_configuration",
            Self::Spawn(_) => "spawn_failed",
            Self::HostSandbox(code) => code,
            Self::Disconnected => "disconnected",
            Self::TimedOut => "timeout",
            Self::Cancelled => "cancelled",
            Self::Transport(_) => "transport_error",
            Self::InvalidResponse(_) => "invalid_response",
            Self::Rpc { .. } => "rpc_error",
            Self::UnsupportedProtocol(_) => "unsupported_protocol",
            Self::InvalidTool(_) => "invalid_tool",
            Self::InvalidInventory(_) => "invalid_inventory",
            Self::InvalidPrompt(_) => "invalid_prompt",
        }
    }

    fn breaks_transport(&self) -> bool {
        matches!(
            self,
            Self::Disconnected
                | Self::Transport(_)
                | Self::InvalidResponse(_)
                | Self::TimedOut
                | Self::Cancelled
        )
    }
}

#[derive(Debug)]
pub struct McpStdioClient {
    server_info: McpServerInfo,
    process: Mutex<Option<McpProcess>>,
    request_timeout: Duration,
    max_message_bytes: usize,
}

impl McpStdioClient {
    /// Bypasses the OS sandbox for protocol-level integration tests only.
    #[doc(hidden)]
    pub async fn connect_unrestricted_for_tests(
        config: McpStdioServerConfig,
        cancellation: CancellationToken,
    ) -> Result<Self, McpClientError> {
        config.validate()?;
        if cancellation.is_cancelled() {
            return Err(McpClientError::Cancelled);
        }
        let process = McpProcess::spawn(&config)?;
        Self::initialize(config, process, cancellation).await
    }

    /// Starts stdio through the attested OS sandbox. Production supervisors
    /// use this path; unrestricted spawning is explicitly named as a test seam.
    pub async fn connect_sandboxed(
        config: McpStdioServerConfig,
        backend: Arc<dyn SandboxBackend>,
        launch: SandboxLaunchSpec,
        cancellation: CancellationToken,
    ) -> Result<Self, McpClientError> {
        config.validate()?;
        if cancellation.is_cancelled() {
            return Err(McpClientError::Cancelled);
        }
        if launch.executable != config.command
            || launch.args != config.args
            || launch.cwd != config.cwd.clone().unwrap_or_else(|| launch.cwd.clone())
            || launch.stdin.is_some()
            || !launch.interactive_stdin
        {
            return Err(McpClientError::HostSandbox(
                "mcp_host_launch_binding_invalid",
            ));
        }
        let process =
            McpProcess::spawn_sandboxed(backend, launch, cancellation.child_token()).await?;
        Self::initialize(config, process, cancellation).await
    }

    async fn initialize(
        config: McpStdioServerConfig,
        process: McpProcess,
        cancellation: CancellationToken,
    ) -> Result<Self, McpClientError> {
        let mut client = Self {
            server_info: McpServerInfo {
                server_id: config.server_id.clone(),
                name: config.server_id.clone(),
                version: "unknown".into(),
                protocol_version: MCP_PROTOCOL_VERSION.into(),
                tools_supported: false,
                resources_supported: false,
                resource_templates_supported: false,
                prompts_supported: false,
            },
            process: Mutex::new(Some(process)),
            request_timeout: config.request_timeout,
            max_message_bytes: config.max_message_bytes,
        };
        let initialize = client
            .request_with_timeout(
                "initialize",
                json!({
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": { "elicitation": {} },
                    "clientInfo": { "name": "hachimi", "version": env!("CARGO_PKG_VERSION") }
                }),
                config.startup_timeout,
                cancellation.child_token(),
            )
            .await?;
        client.server_info = parse_initialize(&config.server_id, &initialize)?;
        client
            .notify("notifications/initialized", json!({}), cancellation)
            .await?;
        Ok(client)
    }

    #[must_use]
    pub const fn server_info(&self) -> &McpServerInfo {
        &self.server_info
    }

    pub async fn ping(&self, cancellation: CancellationToken) -> Result<(), McpClientError> {
        self.request("ping", json!({}), cancellation).await?;
        Ok(())
    }

    pub async fn list_tools(
        &self,
        cancellation: CancellationToken,
    ) -> Result<Vec<McpToolDefinition>, McpClientError> {
        if !self.server_info.tools_supported {
            return Ok(Vec::new());
        }
        let mut tools = Vec::new();
        let mut names = BTreeSet::new();
        let mut cursor = None::<String>;
        for _ in 0..MAX_TOOL_PAGES {
            let params = cursor
                .as_ref()
                .map_or_else(|| json!({}), |cursor| json!({ "cursor": cursor }));
            let result = self
                .request("tools/list", params, cancellation.child_token())
                .await?;
            let page = result
                .get("tools")
                .and_then(Value::as_array)
                .ok_or(McpClientError::InvalidResponse("tools/list omitted tools"))?;
            for value in page {
                let tool = parse_tool(value, self.max_message_bytes)?;
                if !names.insert(tool.name.clone()) {
                    return Err(McpClientError::InvalidTool(format!(
                        "duplicate tool name {}",
                        tool.name
                    )));
                }
                tools.push(tool);
                if tools.len() > MAX_TOOL_COUNT {
                    return Err(McpClientError::InvalidTool(
                        "server advertised more than 512 tools".into(),
                    ));
                }
            }
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if cursor.is_none() {
                return Ok(tools);
            }
        }
        Err(McpClientError::InvalidResponse(
            "tools/list exceeded pagination limit",
        ))
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> Result<McpCallResult, McpClientError> {
        self.call_tool_with_handlers(name, arguments, None, None, None, cancellation)
            .await
    }

    pub async fn call_tool_with_handler(
        &self,
        name: &str,
        arguments: Value,
        correlation: Option<McpRunCorrelation>,
        handler: Option<Arc<dyn McpServerRequestHandler>>,
        cancellation: CancellationToken,
    ) -> Result<McpCallResult, McpClientError> {
        self.call_tool_with_handlers(name, arguments, correlation, handler, None, cancellation)
            .await
    }

    pub async fn call_tool_with_handlers(
        &self,
        name: &str,
        arguments: Value,
        correlation: Option<McpRunCorrelation>,
        request_handler: Option<Arc<dyn McpServerRequestHandler>>,
        progress_handler: Option<Arc<dyn McpProgressHandler>>,
        cancellation: CancellationToken,
    ) -> Result<McpCallResult, McpClientError> {
        if !valid_tool_name(name) || !arguments.is_object() {
            return Err(McpClientError::InvalidTool(
                "tool call requires a valid name and object arguments".into(),
            ));
        }
        let mut params = json!({ "name": name, "arguments": arguments });
        if let Some(correlation) = &correlation {
            params["_meta"] = json!({ "progressToken": correlation.tool_call_id });
        }
        let result = self
            .request_with_server_handler(
                "tools/call",
                params,
                correlation,
                request_handler,
                progress_handler,
                cancellation,
            )
            .await?;
        let content = result
            .get("content")
            .and_then(Value::as_array)
            .cloned()
            .ok_or(McpClientError::InvalidResponse(
                "tools/call omitted content",
            ))?;
        Ok(McpCallResult {
            content,
            structured_content: result.get("structuredContent").cloned(),
            is_error: result
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }

    pub async fn list_resources(
        &self,
        cancellation: CancellationToken,
    ) -> Result<Vec<McpResource>, McpClientError> {
        if !self.server_info.resources_supported {
            return Ok(Vec::new());
        }
        let mut resources = Vec::new();
        let mut seen_cursors = BTreeSet::new();
        let mut cursor = None::<String>;
        for _ in 0..MAX_INVENTORY_PAGES {
            let params = cursor
                .as_ref()
                .map_or_else(|| json!({}), |cursor| json!({ "cursor": cursor }));
            let result = self
                .request("resources/list", params, cancellation.child_token())
                .await?;
            let page = result.get("resources").and_then(Value::as_array).ok_or(
                McpClientError::InvalidResponse("resources/list omitted resources"),
            )?;
            for value in page {
                resources.push(parse_resource(value)?);
                if resources.len() > MAX_RESOURCE_COUNT {
                    return Err(McpClientError::InvalidInventory(
                        "server advertised too many resources".into(),
                    ));
                }
            }
            cursor = next_cursor(&result, &mut seen_cursors)?;
            if cursor.is_none() {
                return Ok(resources);
            }
        }
        Err(McpClientError::InvalidResponse(
            "resources/list exceeded pagination limit",
        ))
    }

    pub async fn list_resources_page(
        &self,
        cursor: Option<&str>,
        cancellation: CancellationToken,
    ) -> Result<McpResourcePage, McpClientError> {
        validate_cursor(cursor)?;
        if !self.server_info.resources_supported {
            return Ok(McpResourcePage {
                resources: Vec::new(),
                next_cursor: None,
            });
        }
        let params = cursor.map_or_else(|| json!({}), |cursor| json!({ "cursor": cursor }));
        let result = self.request("resources/list", params, cancellation).await?;
        parse_resource_page(&result)
    }

    pub async fn list_resource_templates(
        &self,
        cancellation: CancellationToken,
    ) -> Result<Vec<McpResourceTemplate>, McpClientError> {
        if !self.server_info.resource_templates_supported {
            return Ok(Vec::new());
        }
        let mut templates = Vec::new();
        let mut seen_cursors = BTreeSet::new();
        let mut cursor = None::<String>;
        for _ in 0..MAX_INVENTORY_PAGES {
            let params = cursor
                .as_ref()
                .map_or_else(|| json!({}), |cursor| json!({ "cursor": cursor }));
            let result = self
                .request(
                    "resources/templates/list",
                    params,
                    cancellation.child_token(),
                )
                .await?;
            let page = result
                .get("resourceTemplates")
                .and_then(Value::as_array)
                .ok_or(McpClientError::InvalidResponse(
                    "resources/templates/list omitted resourceTemplates",
                ))?;
            for value in page {
                templates.push(parse_resource_template(value)?);
                if templates.len() > MAX_TEMPLATE_COUNT {
                    return Err(McpClientError::InvalidInventory(
                        "server advertised too many resource templates".into(),
                    ));
                }
            }
            cursor = next_cursor(&result, &mut seen_cursors)?;
            if cursor.is_none() {
                return Ok(templates);
            }
        }
        Err(McpClientError::InvalidResponse(
            "resources/templates/list exceeded pagination limit",
        ))
    }

    pub async fn list_resource_templates_page(
        &self,
        cursor: Option<&str>,
        cancellation: CancellationToken,
    ) -> Result<McpResourceTemplatePage, McpClientError> {
        validate_cursor(cursor)?;
        if !self.server_info.resource_templates_supported {
            return Ok(McpResourceTemplatePage {
                resource_templates: Vec::new(),
                next_cursor: None,
            });
        }
        let params = cursor.map_or_else(|| json!({}), |cursor| json!({ "cursor": cursor }));
        let result = self
            .request("resources/templates/list", params, cancellation)
            .await?;
        parse_resource_template_page(&result)
    }

    pub async fn read_resource(
        &self,
        uri: &str,
        cancellation: CancellationToken,
    ) -> Result<Vec<McpResourceContent>, McpClientError> {
        validate_uri(uri)?;
        let result = self
            .request("resources/read", json!({ "uri": uri }), cancellation)
            .await?;
        parse_resource_contents(&result, self.max_message_bytes)
    }

    pub async fn list_prompts(
        &self,
        cancellation: CancellationToken,
    ) -> Result<Vec<McpPrompt>, McpClientError> {
        if !self.server_info.prompts_supported {
            return Ok(Vec::new());
        }
        let mut prompts = Vec::new();
        let mut seen_cursors = BTreeSet::new();
        let mut cursor = None::<String>;
        for _ in 0..MAX_INVENTORY_PAGES {
            let params = cursor
                .as_ref()
                .map_or_else(|| json!({}), |cursor| json!({ "cursor": cursor }));
            let result = self
                .request("prompts/list", params, cancellation.child_token())
                .await?;
            let page = result.get("prompts").and_then(Value::as_array).ok_or(
                McpClientError::InvalidResponse("prompts/list omitted prompts"),
            )?;
            for value in page {
                prompts.push(parse_prompt(value)?);
                if prompts.len() > MAX_PROMPT_COUNT {
                    return Err(McpClientError::InvalidPrompt(
                        "server advertised too many prompts".into(),
                    ));
                }
            }
            cursor = next_cursor(&result, &mut seen_cursors)?;
            if cursor.is_none() {
                return Ok(prompts);
            }
        }
        Err(McpClientError::InvalidResponse(
            "prompts/list exceeded pagination limit",
        ))
    }

    pub async fn get_prompt(
        &self,
        name: &str,
        arguments: BTreeMap<String, String>,
        cancellation: CancellationToken,
    ) -> Result<McpPromptResult, McpClientError> {
        validate_prompt_request(name, &arguments)?;
        let result = self
            .request(
                "prompts/get",
                json!({ "name": name, "arguments": arguments }),
                cancellation,
            )
            .await?;
        parse_prompt_result(&result)
    }

    pub async fn shutdown(&self) -> Result<(), McpClientError> {
        let mut guard = self.process.lock().await;
        if let Some(mut process) = guard.take() {
            process.terminate().await?;
        }
        Ok(())
    }

    async fn request(
        &self,
        method: &str,
        params: Value,
        cancellation: CancellationToken,
    ) -> Result<Value, McpClientError> {
        self.request_with_timeout(method, params, self.request_timeout, cancellation)
            .await
    }

    async fn request_with_server_handler(
        &self,
        method: &str,
        params: Value,
        correlation: Option<McpRunCorrelation>,
        handler: Option<Arc<dyn McpServerRequestHandler>>,
        progress_handler: Option<Arc<dyn McpProgressHandler>>,
        cancellation: CancellationToken,
    ) -> Result<Value, McpClientError> {
        self.request_with_timeout_and_handler(
            method,
            params,
            self.request_timeout,
            correlation,
            handler,
            progress_handler,
            cancellation,
        )
        .await
    }

    async fn request_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<Value, McpClientError> {
        self.request_with_timeout_and_handler(
            method,
            params,
            timeout,
            None,
            None,
            None,
            cancellation,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn request_with_timeout_and_handler(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        correlation: Option<McpRunCorrelation>,
        handler: Option<Arc<dyn McpServerRequestHandler>>,
        progress_handler: Option<Arc<dyn McpProgressHandler>>,
        cancellation: CancellationToken,
    ) -> Result<Value, McpClientError> {
        let mut guard = self.process.lock().await;
        let outcome = {
            let process = guard.as_mut().ok_or(McpClientError::Disconnected)?;
            let request_cancellation = cancellation.child_token();
            let request = process.request(
                &self.server_info.server_id,
                method,
                params,
                self.max_message_bytes,
                correlation,
                handler,
                progress_handler,
                request_cancellation.clone(),
            );
            tokio::pin!(request);
            let timer = tokio::time::sleep(timeout);
            tokio::pin!(timer);
            tokio::select! {
                () = cancellation.cancelled() => {
                    request_cancellation.cancel();
                    // Give a brokered elicitation one bounded poll window to cancel its persisted
                    // UserInput before the transport future is dropped and the process is killed.
                    let _ = tokio::time::timeout(Duration::from_millis(250), &mut request).await;
                    Err(McpClientError::Cancelled)
                },
                () = &mut timer => {
                    request_cancellation.cancel();
                    let _ = tokio::time::timeout(Duration::from_millis(250), &mut request).await;
                    Err(McpClientError::TimedOut)
                },
                result = &mut request => result,
            }
        };
        if outcome
            .as_ref()
            .is_err_and(McpClientError::breaks_transport)
            && let Some(mut process) = guard.take()
        {
            let _ = process.terminate().await;
        }
        outcome
    }

    async fn notify(
        &self,
        method: &str,
        params: Value,
        cancellation: CancellationToken,
    ) -> Result<(), McpClientError> {
        let mut guard = self.process.lock().await;
        let process = guard.as_mut().ok_or(McpClientError::Disconnected)?;
        let notification = process.notify(method, params);
        let outcome = tokio::select! {
            () = cancellation.cancelled() => Err(McpClientError::Cancelled),
            result = tokio::time::timeout(self.request_timeout, notification) => {
                match result {
                    Ok(result) => result,
                    Err(_) => Err(McpClientError::TimedOut),
                }
            }
        };
        if outcome.is_err()
            && let Some(mut process) = guard.take()
        {
            let _ = process.terminate().await;
        }
        outcome
    }
}

struct McpProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr_task: JoinHandle<()>,
    next_id: u64,
}

impl std::fmt::Debug for McpProcess {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpProcess")
            .field("child_id", &self.child.id())
            .field("next_id", &self.next_id)
            .finish_non_exhaustive()
    }
}

impl McpProcess {
    fn spawn(config: &McpStdioServerConfig) -> Result<Self, McpClientError> {
        let mut command = Command::new(&config.command);
        hide_background_window(&mut command);
        command
            .args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .env_clear();
        if let Some(cwd) = &config.cwd {
            command.current_dir(cwd);
        }
        for (name, value) in sanitized_environment() {
            command.env(name, value);
        }
        let child = command
            .spawn()
            .map_err(|error| McpClientError::Spawn(error.to_string()))?;
        Self::from_child(child)
    }

    async fn spawn_sandboxed(
        backend: Arc<dyn SandboxBackend>,
        launch: SandboxLaunchSpec,
        cancellation: CancellationToken,
    ) -> Result<Self, McpClientError> {
        let child = backend
            .spawn_restricted(launch, cancellation)
            .await
            .map_err(|_| McpClientError::HostSandbox("mcp_host_sandbox_rejected"))?
            .into_child()
            .map_err(|_| McpClientError::HostSandbox("mcp_host_child_unavailable"))?;
        Self::from_child(child)
    }

    fn from_child(mut child: Child) -> Result<Self, McpClientError> {
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpClientError::Spawn("stdio server did not expose stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpClientError::Spawn("stdio server did not expose stdout".into()))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| McpClientError::Spawn("stdio server did not expose stderr".into()))?;
        let stderr_task = tokio::spawn(async move {
            let mut buffer = [0_u8; 8 * 1024];
            loop {
                match tokio::io::AsyncReadExt::read(&mut stderr, &mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            stderr_task,
            next_id: 1,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn request(
        &mut self,
        server_id: &str,
        method: &str,
        params: Value,
        max_message_bytes: usize,
        correlation: Option<McpRunCorrelation>,
        handler: Option<Arc<dyn McpServerRequestHandler>>,
        progress_handler: Option<Arc<dyn McpProgressHandler>>,
        cancellation: CancellationToken,
    ) -> Result<Value, McpClientError> {
        if self
            .child
            .try_wait()
            .map_err(|error| McpClientError::Transport(error.to_string()))?
            .is_some()
        {
            return Err(McpClientError::Disconnected);
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.write_message(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .await?;
        let mut skipped = 0_usize;
        loop {
            let response = read_bounded_json_line(&mut self.stdout, max_message_bytes).await?;
            if response.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
                return Err(McpClientError::InvalidResponse(
                    "jsonrpc version is missing or unsupported",
                ));
            }
            let response_id = response.get("id");
            if response_id == Some(&json!(id)) {
                if let Some(error) = response.get("error") {
                    return Err(McpClientError::Rpc {
                        code: error.get("code").and_then(Value::as_i64).unwrap_or(-32_000),
                        message: bounded_text(
                            error
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("MCP request failed"),
                            MAX_SERVER_TEXT_CHARS,
                        ),
                    });
                }
                return response
                    .get("result")
                    .cloned()
                    .ok_or(McpClientError::InvalidResponse(
                        "response omitted result and error",
                    ));
            }
            if response_id.is_some() && response.get("method").is_some() {
                let response = dispatch_server_request(
                    server_id,
                    &response,
                    correlation.clone(),
                    handler.clone(),
                    cancellation.child_token(),
                )
                .await;
                self.write_message(&response).await?;
            } else if response_id.is_none() && response.get("method").is_some() {
                dispatch_progress_notification(
                    server_id,
                    &response,
                    correlation.clone(),
                    progress_handler.clone(),
                )
                .await;
            }
            skipped = skipped.saturating_add(1);
            if skipped > 128 {
                return Err(McpClientError::InvalidResponse(
                    "too many messages arrived before the response",
                ));
            }
        }
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), McpClientError> {
        self.write_message(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
        .await
    }

    async fn write_message(&mut self, value: &Value) -> Result<(), McpClientError> {
        let mut encoded = serde_json::to_vec(value)
            .map_err(|error| McpClientError::Transport(error.to_string()))?;
        encoded.push(b'\n');
        self.stdin
            .write_all(&encoded)
            .await
            .map_err(|error| McpClientError::Transport(error.to_string()))?;
        self.stdin
            .flush()
            .await
            .map_err(|error| McpClientError::Transport(error.to_string()))
    }

    async fn terminate(&mut self) -> Result<(), McpClientError> {
        let _ = self.child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await;
        self.stderr_task.abort();
        Ok(())
    }
}

#[cfg(windows)]
fn hide_background_window(command: &mut Command) {
    use std::os::windows::process::CommandExt as _;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.as_std_mut().creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_background_window(_command: &mut Command) {}

async fn read_bounded_json_line(
    reader: &mut BufReader<ChildStdout>,
    max_bytes: usize,
) -> Result<Value, McpClientError> {
    let mut line = Vec::new();
    loop {
        let (take, newline, eof) = {
            let available = reader
                .fill_buf()
                .await
                .map_err(|error| McpClientError::Transport(error.to_string()))?;
            if available.is_empty() {
                (0, false, true)
            } else if let Some(position) = available.iter().position(|byte| *byte == b'\n') {
                let take = position + 1;
                if line.len().saturating_add(take) > max_bytes {
                    return Err(McpClientError::InvalidResponse(
                        "message exceeded configured byte budget",
                    ));
                }
                line.extend_from_slice(&available[..take]);
                (take, true, false)
            } else {
                let take = available.len();
                if line.len().saturating_add(take) > max_bytes {
                    return Err(McpClientError::InvalidResponse(
                        "message exceeded configured byte budget",
                    ));
                }
                line.extend_from_slice(available);
                (take, false, false)
            }
        };
        if eof {
            return Err(McpClientError::Disconnected);
        }
        reader.consume(take);
        if newline {
            break;
        }
    }
    while matches!(line.last(), Some(b'\n' | b'\r')) {
        line.pop();
    }
    serde_json::from_slice(&line)
        .map_err(|_| McpClientError::InvalidResponse("message was not valid JSON"))
}

pub(crate) fn parse_initialize(
    server_id: &str,
    value: &Value,
) -> Result<McpServerInfo, McpClientError> {
    let protocol_version = value.get("protocolVersion").and_then(Value::as_str).ok_or(
        McpClientError::InvalidResponse("initialize omitted protocolVersion"),
    )?;
    if protocol_version != MCP_PROTOCOL_VERSION {
        return Err(McpClientError::UnsupportedProtocol(bounded_text(
            protocol_version,
            64,
        )));
    }
    let server = value.get("serverInfo").unwrap_or(&Value::Null);
    Ok(McpServerInfo {
        server_id: server_id.into(),
        name: bounded_text(
            server
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(server_id),
            256,
        ),
        version: bounded_text(
            server
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or("unknown"),
            128,
        ),
        protocol_version: protocol_version.into(),
        tools_supported: value
            .pointer("/capabilities/tools")
            .is_some_and(Value::is_object),
        resources_supported: value
            .pointer("/capabilities/resources")
            .is_some_and(Value::is_object),
        resource_templates_supported: value
            .pointer("/capabilities/resources")
            .is_some_and(Value::is_object),
        prompts_supported: value
            .pointer("/capabilities/prompts")
            .is_some_and(Value::is_object),
    })
}

pub(crate) fn parse_tool(
    value: &Value,
    max_schema_bytes: usize,
) -> Result<McpToolDefinition, McpClientError> {
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| McpClientError::InvalidTool("tool omitted name".into()))?;
    if !valid_tool_name(name) {
        return Err(McpClientError::InvalidTool(format!(
            "tool name is invalid: {}",
            bounded_text(name, 128)
        )));
    }
    let input_schema = value
        .get("inputSchema")
        .cloned()
        .ok_or_else(|| McpClientError::InvalidTool(format!("tool {name} omitted inputSchema")))?;
    if !input_schema.is_object() {
        return Err(McpClientError::InvalidTool(format!(
            "tool {name} inputSchema is not an object"
        )));
    }
    if serde_json::to_vec(&input_schema)
        .map_err(|error| McpClientError::InvalidTool(error.to_string()))?
        .len()
        > max_schema_bytes.min(256 * 1024)
    {
        return Err(McpClientError::InvalidTool(format!(
            "tool {name} inputSchema exceeded the budget"
        )));
    }
    Ok(McpToolDefinition {
        name: name.into(),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .map(|description| bounded_text(description, MAX_SERVER_TEXT_CHARS)),
        input_schema,
    })
}

fn valid_server_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

pub(crate) fn valid_tool_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
}

pub(crate) fn bounded_text(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

pub(crate) fn validate_uri(uri: &str) -> Result<(), McpClientError> {
    if uri.is_empty() || uri.chars().count() > 4_096 || uri.chars().any(char::is_control) {
        return Err(McpClientError::InvalidInventory(
            "resource URI is empty, too long or contains control characters".into(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_prompt_request(
    name: &str,
    arguments: &BTreeMap<String, String>,
) -> Result<(), McpClientError> {
    if !valid_tool_name(name)
        || arguments.len() > 128
        || arguments.iter().any(|(name, value)| {
            !valid_tool_name(name)
                || value.chars().count() > 16_384
                || value.chars().any(char::is_control)
        })
    {
        return Err(McpClientError::InvalidPrompt(
            "prompt name or arguments are invalid".into(),
        ));
    }
    Ok(())
}

pub(crate) fn sanitized_environment() -> BTreeMap<OsString, OsString> {
    const ALLOWED: &[&str] = &[
        "PATH",
        "PATHEXT",
        "SystemRoot",
        "SystemDrive",
        "WINDIR",
        "ComSpec",
        "TEMP",
        "TMP",
        "LANG",
        "LC_ALL",
    ];
    ALLOWED
        .iter()
        .filter_map(|name| std::env::var_os(name).map(|value| (OsString::from(name), value)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_and_tool_names_are_strict() {
        assert!(valid_server_id("local.docs-1"));
        assert!(!valid_server_id("bad server"));
        assert!(valid_tool_name("docs/read-file"));
        assert!(!valid_tool_name("docs read"));
    }

    #[test]
    fn tool_schema_must_be_bounded_object() {
        let valid = parse_tool(
            &json!({
                "name": "echo",
                "description": "Echo text",
                "inputSchema": { "type": "object" }
            }),
            16 * 1024,
        )
        .expect("tool");
        assert_eq!(valid.name, "echo");
        assert!(parse_tool(&json!({ "name": "echo", "inputSchema": [] }), 16 * 1024).is_err());
    }
}
