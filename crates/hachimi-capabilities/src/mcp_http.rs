//! MCP Streamable HTTP transport. Redirects are disabled so credentials never cross origins.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use futures_util::StreamExt;
use hachimi_protocol::{
    McpPrompt, McpPromptResult, McpResource, McpResourceContent, McpResourceTemplate,
};
use reqwest::{
    Client, Response, StatusCode,
    header::{ACCEPT, CONTENT_TYPE, HeaderName, HeaderValue},
};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::mcp::{
    MAX_SERVER_TEXT_CHARS, MAX_TOOL_COUNT, MAX_TOOL_PAGES, MCP_PROTOCOL_VERSION, McpCallResult,
    McpClientError, McpServerInfo, McpToolDefinition, bounded_text, parse_initialize, parse_tool,
    valid_tool_name, validate_prompt_request, validate_uri,
};
use crate::mcp_elicitation::{McpRunCorrelation, McpServerRequestHandler, dispatch_server_request};
use crate::mcp_inventory::{
    MAX_INVENTORY_PAGES, MAX_PROMPT_COUNT, MAX_RESOURCE_COUNT, MAX_TEMPLATE_COUNT, McpResourcePage,
    McpResourceTemplatePage, next_cursor, parse_prompt, parse_prompt_result, parse_resource,
    parse_resource_contents, parse_resource_page, parse_resource_template,
    parse_resource_template_page, validate_cursor,
};
use crate::mcp_progress::{McpProgressHandler, dispatch_progress_notification};

const MCP_SESSION_ID: &str = "mcp-session-id";

#[derive(Debug, Clone)]
pub struct McpHttpServerConfig {
    pub server_id: String,
    pub url: Url,
    pub headers: BTreeMap<String, String>,
    pub startup_timeout: Duration,
    pub request_timeout: Duration,
    pub max_message_bytes: usize,
}

impl McpHttpServerConfig {
    pub fn validate(&self) -> Result<(), McpClientError> {
        let loopback_http = self.url.scheme() == "http"
            && self
                .url
                .host_str()
                .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1"));
        if self.url.scheme() != "https" && !loopback_http {
            return Err(McpClientError::InvalidConfiguration(
                "remote MCP requires HTTPS, except for loopback HTTP".into(),
            ));
        }
        if self.url.username() != ""
            || self.url.password().is_some()
            || self.url.fragment().is_some()
        {
            return Err(McpClientError::InvalidConfiguration(
                "remote MCP URL cannot contain credentials or a fragment".into(),
            ));
        }
        if self.headers.len() > 64
            || self.headers.iter().any(|(name, value)| {
                HeaderName::from_bytes(name.as_bytes()).is_err()
                    || HeaderValue::from_str(value).is_err()
                    || value.len() > 8_192
                    || reserved_header(name)
            })
        {
            return Err(McpClientError::InvalidConfiguration(
                "remote MCP headers are invalid or reserved".into(),
            ));
        }
        if self.startup_timeout.is_zero()
            || self.request_timeout.is_zero()
            || !(4 * 1024..=16 * 1024 * 1024).contains(&self.max_message_bytes)
        {
            return Err(McpClientError::InvalidConfiguration(
                "remote MCP limits are invalid".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct McpHttpClient {
    client: Client,
    config: McpHttpServerConfig,
    server_info: McpServerInfo,
    session_id: Arc<Mutex<Option<String>>>,
    next_id: AtomicU64,
}

impl McpHttpClient {
    pub async fn connect(
        config: McpHttpServerConfig,
        cancellation: CancellationToken,
    ) -> Result<Self, McpClientError> {
        config.validate()?;
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(config.startup_timeout)
            .build()
            .map_err(|error| McpClientError::Transport(error.to_string()))?;
        let mut instance = Self {
            client,
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
            config,
            session_id: Arc::new(Mutex::new(None)),
            next_id: AtomicU64::new(1),
        };
        let initialize = instance
            .request_with_timeout(
                "initialize",
                json!({
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": { "elicitation": {} },
                    "clientInfo": { "name": "hachimi", "version": env!("CARGO_PKG_VERSION") }
                }),
                instance.config.startup_timeout,
                cancellation.child_token(),
            )
            .await?;
        instance.server_info = parse_initialize(&instance.config.server_id, &initialize)?;
        instance
            .notify("notifications/initialized", json!({}), cancellation)
            .await?;
        Ok(instance)
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
                let tool = parse_tool(value, self.config.max_message_bytes)?;
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
        parse_resource_contents(&result, self.config.max_message_bytes)
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
        let session_id = self.session_id.lock().await.clone();
        let Some(session_id) = session_id else {
            return Ok(());
        };
        let response = self
            .client
            .delete(self.config.url.clone())
            .header(MCP_SESSION_ID, session_id)
            .send()
            .await;
        match response {
            Ok(response)
                if response.status().is_success() || response.status() == StatusCode::NOT_FOUND =>
            {
                Ok(())
            }
            Ok(_) | Err(_) => Ok(()),
        }
    }

    async fn request(
        &self,
        method: &str,
        params: Value,
        cancellation: CancellationToken,
    ) -> Result<Value, McpClientError> {
        self.request_with_timeout(method, params, self.config.request_timeout, cancellation)
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
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let payload = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let response = self
            .send_with_server_handler(
                payload,
                self.config.request_timeout,
                correlation,
                handler,
                progress_handler,
                cancellation,
            )
            .await?;
        parse_rpc_response(response, id)
    }

    async fn request_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<Value, McpClientError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let payload = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let response = self.send(payload, timeout, cancellation).await?;
        parse_rpc_response(response, id)
    }

    async fn notify(
        &self,
        method: &str,
        params: Value,
        cancellation: CancellationToken,
    ) -> Result<(), McpClientError> {
        let payload = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        self.send(payload, self.config.request_timeout, cancellation)
            .await?;
        Ok(())
    }

    async fn send(
        &self,
        payload: Value,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<Option<Value>, McpClientError> {
        let mut request = self
            .client
            .post(self.config.url.clone())
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json, text/event-stream")
            .json(&payload);
        for (name, value) in &self.config.headers {
            request = request.header(name, value);
        }
        if let Some(session_id) = self.session_id.lock().await.as_ref() {
            request = request.header(MCP_SESSION_ID, session_id);
        }
        let response = tokio::select! {
            () = cancellation.cancelled() => return Err(McpClientError::Cancelled),
            response = tokio::time::timeout(timeout, request.send()) => {
                match response {
                    Ok(Ok(response)) => response,
                    Ok(Err(error)) => return Err(McpClientError::Transport(error.to_string())),
                    Err(_) => return Err(McpClientError::TimedOut),
                }
            }
        };
        if response.status().is_redirection() {
            return Err(McpClientError::Transport(
                "remote MCP redirects are disabled".into(),
            ));
        }
        if !response.status().is_success() {
            return Err(McpClientError::Transport(format!(
                "remote MCP returned HTTP {}",
                response.status().as_u16()
            )));
        }
        if let Some(session_id) = response
            .headers()
            .get(MCP_SESSION_ID)
            .and_then(|value| value.to_str().ok())
        {
            *self.session_id.lock().await = Some(session_id.to_owned());
        }
        if matches!(
            response.status(),
            StatusCode::ACCEPTED | StatusCode::NO_CONTENT
        ) {
            return Ok(None);
        }
        read_response(response, self.config.max_message_bytes, cancellation).await
    }

    async fn send_with_server_handler(
        &self,
        payload: Value,
        timeout: Duration,
        correlation: Option<McpRunCorrelation>,
        handler: Option<Arc<dyn McpServerRequestHandler>>,
        progress_handler: Option<Arc<dyn McpProgressHandler>>,
        cancellation: CancellationToken,
    ) -> Result<Option<Value>, McpClientError> {
        let mut request = self
            .client
            .post(self.config.url.clone())
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json, text/event-stream")
            .json(&payload);
        for (name, value) in &self.config.headers {
            request = request.header(name, value);
        }
        if let Some(session_id) = self.session_id.lock().await.as_ref() {
            request = request.header(MCP_SESSION_ID, session_id);
        }
        let response = tokio::select! {
            () = cancellation.cancelled() => return Err(McpClientError::Cancelled),
            response = tokio::time::timeout(timeout, request.send()) => {
                match response {
                    Ok(Ok(response)) => response,
                    Ok(Err(error)) => return Err(McpClientError::Transport(error.to_string())),
                    Err(_) => return Err(McpClientError::TimedOut),
                }
            }
        };
        self.validate_response_status(&response)?;
        if let Some(session_id) = response
            .headers()
            .get(MCP_SESSION_ID)
            .and_then(|value| value.to_str().ok())
        {
            *self.session_id.lock().await = Some(session_id.to_owned());
        }
        if matches!(
            response.status(),
            StatusCode::ACCEPTED | StatusCode::NO_CONTENT
        ) {
            return Ok(None);
        }
        if !is_sse_response(&response) {
            return read_response(response, self.config.max_message_bytes, cancellation).await;
        }
        self.read_sse_with_server_requests(
            response,
            correlation,
            handler,
            progress_handler,
            cancellation,
        )
        .await
    }

    fn validate_response_status(&self, response: &Response) -> Result<(), McpClientError> {
        if response.status().is_redirection() {
            return Err(McpClientError::Transport(
                "remote MCP redirects are disabled".into(),
            ));
        }
        if !response.status().is_success() {
            return Err(McpClientError::Transport(format!(
                "remote MCP returned HTTP {}",
                response.status().as_u16()
            )));
        }
        Ok(())
    }

    async fn read_sse_with_server_requests(
        &self,
        response: Response,
        correlation: Option<McpRunCorrelation>,
        handler: Option<Arc<dyn McpServerRequestHandler>>,
        progress_handler: Option<Arc<dyn McpProgressHandler>>,
        cancellation: CancellationToken,
    ) -> Result<Option<Value>, McpClientError> {
        let mut stream = response.bytes_stream();
        let mut buffer = Vec::new();
        let mut total_bytes = 0_usize;
        let mut server_request_count = 0_usize;
        loop {
            let next = tokio::select! {
                () = cancellation.cancelled() => return Err(McpClientError::Cancelled),
                next = stream.next() => next,
            };
            let Some(chunk) = next else { break };
            let chunk = chunk.map_err(|error| McpClientError::Transport(error.to_string()))?;
            total_bytes = total_bytes.saturating_add(chunk.len());
            if total_bytes > self.config.max_message_bytes {
                return Err(McpClientError::InvalidResponse(
                    "message exceeded configured byte budget",
                ));
            }
            buffer.extend_from_slice(&chunk);
            while let Some(event) = take_sse_event(&mut buffer) {
                let Some(message) = parse_sse_event(&event)? else {
                    continue;
                };
                if message.get("id").is_some() && message.get("method").is_some() {
                    server_request_count = server_request_count.saturating_add(1);
                    if server_request_count > 128 {
                        return Err(McpClientError::InvalidResponse(
                            "too many server requests arrived before the response",
                        ));
                    }
                    let response = dispatch_server_request(
                        &self.config.server_id,
                        &message,
                        correlation.clone(),
                        handler.clone(),
                        cancellation.child_token(),
                    )
                    .await;
                    self.post_server_response(response, cancellation.child_token())
                        .await?;
                } else if message.get("id").is_none() && message.get("method").is_some() {
                    dispatch_progress_notification(
                        &self.config.server_id,
                        &message,
                        correlation.clone(),
                        progress_handler.clone(),
                    )
                    .await;
                } else if message.get("id").is_some() {
                    return Ok(Some(message));
                }
            }
        }
        if !buffer.is_empty()
            && let Some(message) = parse_sse_event(&buffer)?
            && message.get("id").is_some()
            && message.get("method").is_none()
        {
            return Ok(Some(message));
        }
        Err(McpClientError::InvalidResponse(
            "SSE response omitted the JSON-RPC response",
        ))
    }

    async fn post_server_response(
        &self,
        payload: Value,
        cancellation: CancellationToken,
    ) -> Result<(), McpClientError> {
        let mut request = self
            .client
            .post(self.config.url.clone())
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json, text/event-stream")
            .json(&payload);
        for (name, value) in &self.config.headers {
            request = request.header(name, value);
        }
        if let Some(session_id) = self.session_id.lock().await.as_ref() {
            request = request.header(MCP_SESSION_ID, session_id);
        }
        let response = tokio::select! {
            () = cancellation.cancelled() => return Err(McpClientError::Cancelled),
            response = tokio::time::timeout(self.config.request_timeout, request.send()) => {
                match response {
                    Ok(Ok(response)) => response,
                    Ok(Err(error)) => return Err(McpClientError::Transport(error.to_string())),
                    Err(_) => return Err(McpClientError::TimedOut),
                }
            }
        };
        self.validate_response_status(&response)
    }
}

fn is_sse_response(response: &Response) -> bool {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("text/event-stream"))
}

fn take_sse_event(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    let (position, separator_len) = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| (position, 4))
        .or_else(|| {
            buffer
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|position| (position, 2))
        })?;
    let event = buffer[..position].to_vec();
    buffer.drain(..position + separator_len);
    Some(event)
}

fn parse_sse_event(event: &[u8]) -> Result<Option<Value>, McpClientError> {
    let text = std::str::from_utf8(event)
        .map_err(|_| McpClientError::InvalidResponse("SSE response was not UTF-8"))?;
    let mut data = String::new();
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(value.trim_start());
        }
    }
    if data.is_empty() {
        return Ok(None);
    }
    serde_json::from_str(&data)
        .map(Some)
        .map_err(|_| McpClientError::InvalidResponse("SSE data was not valid JSON"))
}

async fn read_response(
    response: Response,
    max_bytes: usize,
    cancellation: CancellationToken,
) -> Result<Option<Value>, McpClientError> {
    let is_sse = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("text/event-stream"));
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    loop {
        let next = tokio::select! {
            () = cancellation.cancelled() => return Err(McpClientError::Cancelled),
            next = stream.next() => next,
        };
        let Some(chunk) = next else { break };
        let chunk = chunk.map_err(|error| McpClientError::Transport(error.to_string()))?;
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(McpClientError::InvalidResponse(
                "message exceeded configured byte budget",
            ));
        }
        body.extend_from_slice(&chunk);
    }
    if body.is_empty() {
        return Ok(None);
    }
    if is_sse {
        parse_sse(&body).map(Some)
    } else {
        serde_json::from_slice(&body)
            .map(Some)
            .map_err(|_| McpClientError::InvalidResponse("response was not valid JSON"))
    }
}

fn parse_sse(body: &[u8]) -> Result<Value, McpClientError> {
    let text = std::str::from_utf8(body)
        .map_err(|_| McpClientError::InvalidResponse("SSE response was not UTF-8"))?;
    let mut data = String::new();
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(value.trim_start());
        } else if line.is_empty() && !data.is_empty() {
            return serde_json::from_str(&data)
                .map_err(|_| McpClientError::InvalidResponse("SSE data was not valid JSON"));
        }
    }
    if !data.is_empty() {
        return serde_json::from_str(&data)
            .map_err(|_| McpClientError::InvalidResponse("SSE data was not valid JSON"));
    }
    Err(McpClientError::InvalidResponse(
        "SSE response omitted a data event",
    ))
}

fn parse_rpc_response(response: Option<Value>, id: u64) -> Result<Value, McpClientError> {
    let response = response.ok_or(McpClientError::InvalidResponse(
        "request response body was empty",
    ))?;
    if response.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || response.get("id") != Some(&json!(id))
    {
        return Err(McpClientError::InvalidResponse(
            "JSON-RPC response ID or version did not match",
        ));
    }
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
    response
        .get("result")
        .cloned()
        .ok_or(McpClientError::InvalidResponse(
            "response omitted result and error",
        ))
}

fn reserved_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "host"
            | "content-length"
            | "content-type"
            | "accept"
            | "connection"
            | "transfer-encoding"
            | MCP_SESSION_ID
    )
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        sync::Mutex,
        thread,
    };

    use futures_util::FutureExt;
    use hachimi_protocol::{RunId, SessionId, ToolCallId};

    use super::*;

    struct FixtureResponse {
        status: &'static str,
        content_type: &'static str,
        body: String,
        headers: Vec<(&'static str, &'static str)>,
        delay: Duration,
    }

    fn response(body: Value) -> FixtureResponse {
        FixtureResponse {
            status: "200 OK",
            content_type: "application/json",
            body: body.to_string(),
            headers: Vec::new(),
            delay: Duration::ZERO,
        }
    }

    fn spawn_fixture(
        responses: Vec<FixtureResponse>,
    ) -> (Url, thread::JoinHandle<Result<Vec<String>, String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture");
        let address = listener.local_addr().expect("fixture address");
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .map_err(|error| error.to_string())?;
                requests.push(read_request(&mut stream)?);
                if !response.delay.is_zero() {
                    thread::sleep(response.delay);
                }
                let extra_headers = response
                    .headers
                    .iter()
                    .map(|(name, value)| format!("{name}: {value}\r\n"))
                    .collect::<String>();
                let wire = format!(
                    "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n{}",
                    response.status,
                    response.content_type,
                    response.body.len(),
                    extra_headers,
                    response.body,
                );
                // Cancellation tests may close the socket before the delayed response is written.
                let _ = stream.write_all(wire.as_bytes());
            }
            Ok(requests)
        });
        (
            Url::parse(&format!("http://{address}/mcp")).expect("fixture URL"),
            handle,
        )
    }

    fn read_request(stream: &mut TcpStream) -> Result<String, String> {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 2048];
        let header_end = loop {
            let read = stream
                .read(&mut buffer)
                .map_err(|error| error.to_string())?;
            if read == 0 {
                return Err("fixture request closed before headers".into());
            }
            bytes.extend_from_slice(&buffer[..read]);
            if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break position + 4;
            }
        };
        let headers =
            std::str::from_utf8(&bytes[..header_end]).map_err(|error| error.to_string())?;
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
            })
            .unwrap_or(0);
        while bytes.len().saturating_sub(header_end) < content_length {
            let read = stream
                .read(&mut buffer)
                .map_err(|error| error.to_string())?;
            if read == 0 {
                return Err("fixture request closed before body".into());
            }
            bytes.extend_from_slice(&buffer[..read]);
        }
        String::from_utf8(bytes).map_err(|error| error.to_string())
    }

    fn fixture_config(url: Url) -> McpHttpServerConfig {
        McpHttpServerConfig {
            server_id: "http-fixture".into(),
            url,
            headers: BTreeMap::from([("Authorization".into(), "Bearer test-only".into())]),
            startup_timeout: Duration::from_secs(2),
            request_timeout: Duration::from_secs(2),
            max_message_bytes: 1024 * 1024,
        }
    }

    #[test]
    fn remote_url_and_reserved_headers_fail_closed() {
        let mut config = McpHttpServerConfig {
            server_id: "test".into(),
            url: Url::parse("http://example.test/mcp").unwrap(),
            headers: BTreeMap::new(),
            startup_timeout: Duration::from_secs(1),
            request_timeout: Duration::from_secs(1),
            max_message_bytes: 1024 * 1024,
        };
        assert!(matches!(
            config.validate(),
            Err(McpClientError::InvalidConfiguration(_))
        ));
        config.url = Url::parse("http://127.0.0.1:3000/mcp").unwrap();
        config.headers.insert("Host".into(), "evil.test".into());
        assert!(matches!(
            config.validate(),
            Err(McpClientError::InvalidConfiguration(_))
        ));
    }

    #[test]
    fn parses_json_rpc_sse_data_without_exposing_event_metadata() {
        let value = parse_sse(
            b"id: opaque\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n",
        )
        .expect("SSE");
        assert_eq!(value["id"], 1);
    }

    struct AcceptHttpElicitation;

    #[derive(Default)]
    struct CaptureHttpProgress(Mutex<Vec<crate::McpProgressNotification>>);

    impl crate::McpProgressHandler for CaptureHttpProgress {
        fn progress(
            &self,
            notification: crate::McpProgressNotification,
        ) -> crate::McpProgressFuture {
            self.0.lock().expect("progress capture").push(notification);
            async {}.boxed()
        }
    }

    #[tokio::test]
    async fn streamable_http_projects_progress_without_posting_a_rpc_response() {
        let initialize = json!({
            "jsonrpc": "2.0", "id": 1,
            "result": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "progress", "version": "1.0" }
            }
        });
        let progress_notification = json!({
            "jsonrpc": "2.0",
            "method": "notifications/progress",
            "params": {
                "progressToken": "tool-call",
                "progress": 1,
                "total": 2,
                "message": "HTTP working"
            }
        });
        let tool_result = json!({
            "jsonrpc": "2.0", "id": 2,
            "result": {
                "content": [{ "type": "text", "text": "done" }],
                "isError": false
            }
        });
        let (url, server) = spawn_fixture(vec![
            response(initialize),
            FixtureResponse {
                status: "202 Accepted",
                content_type: "application/json",
                body: String::new(),
                headers: Vec::new(),
                delay: Duration::ZERO,
            },
            FixtureResponse {
                content_type: "text/event-stream",
                body: format!(
                    "event: message\ndata: {progress_notification}\n\nevent: message\ndata: {tool_result}\n\n"
                ),
                ..response(Value::Null)
            },
        ]);
        let client = McpHttpClient::connect(fixture_config(url), CancellationToken::new())
            .await
            .expect("connect");
        let progress = Arc::new(CaptureHttpProgress::default());
        let result = client
            .call_tool_with_handlers(
                "echo",
                json!({}),
                Some(McpRunCorrelation {
                    session_id: SessionId::from("session"),
                    run_id: RunId::from("run"),
                    run_generation: 5,
                    tool_call_id: ToolCallId::from("tool-call"),
                }),
                None,
                Some(progress.clone()),
                CancellationToken::new(),
            )
            .await
            .expect("tool call");
        assert_eq!(result.content[0]["text"], "done");
        let captured = progress.0.lock().expect("progress capture");
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].message.as_deref(), Some("HTTP working"));
        drop(captured);
        let requests = server.join().expect("server thread").expect("requests");
        assert_eq!(
            requests.len(),
            3,
            "notifications must not receive JSON-RPC responses"
        );
    }

    impl McpServerRequestHandler for AcceptHttpElicitation {
        fn handle(
            &self,
            request: crate::McpServerRequest,
            _cancellation: CancellationToken,
        ) -> crate::McpServerRequestFuture {
            async move {
                assert_eq!(request.method, "elicitation/create");
                assert_eq!(request.correlation.expect("correlation").run_generation, 3);
                crate::McpServerRequestResponse::result(json!({
                    "action": "accept",
                    "content": { "confirmed": true }
                }))
            }
            .boxed()
        }
    }

    #[tokio::test]
    async fn streamable_http_answers_server_elicitation_before_tool_result() {
        let initialize = json!({
            "jsonrpc": "2.0", "id": 1,
            "result": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "elicitation", "version": "1.0" }
            }
        });
        let server_request = json!({
            "jsonrpc": "2.0",
            "id": "http-ask-1",
            "method": "elicitation/create",
            "params": {
                "mode": "form",
                "message": "Confirm",
                "requestedSchema": {
                    "type": "object",
                    "properties": { "confirmed": { "type": "boolean" } },
                    "required": ["confirmed"]
                }
            }
        });
        let tool_result = json!({
            "jsonrpc": "2.0", "id": 2,
            "result": {
                "content": [{ "type": "text", "text": "accepted" }],
                "structuredContent": { "confirmed": true },
                "isError": false
            }
        });
        let (url, server) = spawn_fixture(vec![
            response(initialize),
            FixtureResponse {
                status: "202 Accepted",
                content_type: "application/json",
                body: String::new(),
                headers: Vec::new(),
                delay: Duration::ZERO,
            },
            FixtureResponse {
                content_type: "text/event-stream",
                body: format!(
                    "event: message\ndata: {server_request}\n\nevent: message\ndata: {tool_result}\n\n"
                ),
                ..response(Value::Null)
            },
            FixtureResponse {
                status: "202 Accepted",
                content_type: "application/json",
                body: String::new(),
                headers: Vec::new(),
                delay: Duration::ZERO,
            },
        ]);
        let client = McpHttpClient::connect(fixture_config(url), CancellationToken::new())
            .await
            .expect("connect");
        let result = client
            .call_tool_with_handler(
                "elicit",
                json!({}),
                Some(McpRunCorrelation {
                    session_id: SessionId::from("session"),
                    run_id: RunId::from("run"),
                    run_generation: 3,
                    tool_call_id: ToolCallId::from("tool-call"),
                }),
                Some(Arc::new(AcceptHttpElicitation)),
                CancellationToken::new(),
            )
            .await
            .expect("tool call");
        assert_eq!(result.content[0]["text"], "accepted");
        let requests = server.join().expect("server thread").expect("requests");
        assert!(requests[0].contains("\"elicitation\":{}"));
        assert!(requests[3].contains("\"id\":\"http-ask-1\""));
        assert!(requests[3].contains("\"action\":\"accept\""));
    }

    #[tokio::test]
    async fn initializes_tracks_session_and_reads_paginated_json_and_sse_tools() {
        let initialize = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "fixture", "version": "1.0" }
            }
        });
        let first_page = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "tools": [{
                    "name": "first_tool",
                    "description": "first",
                    "inputSchema": { "type": "object", "properties": {} }
                }],
                "nextCursor": "page-2"
            }
        });
        let second_page = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "result": {
                "tools": [{
                    "name": "second_tool",
                    "description": "second",
                    "inputSchema": { "type": "object", "properties": {} }
                }]
            }
        });
        let (url, server) = spawn_fixture(vec![
            FixtureResponse {
                headers: vec![(MCP_SESSION_ID, "fixture-session")],
                ..response(initialize)
            },
            FixtureResponse {
                status: "202 Accepted",
                content_type: "application/json",
                body: String::new(),
                headers: Vec::new(),
                delay: Duration::ZERO,
            },
            FixtureResponse {
                content_type: "text/event-stream",
                body: format!("event: message\ndata: {first_page}\n\n"),
                ..response(Value::Null)
            },
            response(second_page),
        ]);

        let client = McpHttpClient::connect(fixture_config(url), CancellationToken::new())
            .await
            .expect("connect HTTP MCP");
        let tools = client
            .list_tools(CancellationToken::new())
            .await
            .expect("paginated tools");
        assert_eq!(
            tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            vec!["first_tool", "second_tool"]
        );
        let requests = server.join().expect("server thread").expect("requests");
        assert!(requests[1].contains("notifications/initialized"));
        assert!(
            requests[1]
                .to_ascii_lowercase()
                .contains("mcp-session-id: fixture-session")
        );
        assert!(requests[3].contains("\"cursor\":\"page-2\""));
    }

    #[tokio::test]
    async fn http_transport_exposes_bounded_resources_templates_and_prompts() {
        let initialize = json!({
            "jsonrpc": "2.0", "id": 1,
            "result": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": { "resources": {}, "prompts": {} },
                "serverInfo": { "name": "inventory", "version": "1.0" }
            }
        });
        let responses = vec![
            response(initialize),
            FixtureResponse {
                status: "202 Accepted",
                content_type: "application/json",
                body: String::new(),
                headers: Vec::new(),
                delay: Duration::ZERO,
            },
            response(json!({
                "jsonrpc": "2.0", "id": 2,
                "result": { "resources": [{ "uri": "memo://one", "name": "one" }] }
            })),
            response(json!({
                "jsonrpc": "2.0", "id": 3,
                "result": { "resourceTemplates": [{
                    "uriTemplate": "memo://{id}", "name": "memo"
                }] }
            })),
            response(json!({
                "jsonrpc": "2.0", "id": 4,
                "result": { "contents": [{ "uri": "memo://one", "text": "hello" }] }
            })),
            response(json!({
                "jsonrpc": "2.0", "id": 5,
                "result": { "prompts": [{ "name": "brief" }] }
            })),
            response(json!({
                "jsonrpc": "2.0", "id": 6,
                "result": { "messages": [{
                    "role": "user", "content": { "type": "text", "text": "hello" }
                }] }
            })),
        ];
        let (url, server) = spawn_fixture(responses);
        let client = McpHttpClient::connect(fixture_config(url), CancellationToken::new())
            .await
            .expect("connect");
        assert_eq!(
            client
                .list_resources(CancellationToken::new())
                .await
                .expect("resources")[0]
                .uri,
            "memo://one"
        );
        assert_eq!(
            client
                .list_resource_templates(CancellationToken::new())
                .await
                .expect("templates")[0]
                .uri_template,
            "memo://{id}"
        );
        assert_eq!(
            client
                .read_resource("memo://one", CancellationToken::new())
                .await
                .expect("read")[0]
                .text
                .as_deref(),
            Some("hello")
        );
        assert_eq!(
            client
                .list_prompts(CancellationToken::new())
                .await
                .expect("prompts")[0]
                .name,
            "brief"
        );
        assert_eq!(
            client
                .get_prompt("brief", BTreeMap::new(), CancellationToken::new())
                .await
                .expect("prompt")
                .messages[0]
                .content["text"],
            "hello"
        );
        let requests = server.join().expect("server thread").expect("requests");
        assert!(requests[4].contains("\"uri\":\"memo://one\""));
        assert!(requests[6].contains("\"name\":\"brief\""));
    }

    #[tokio::test]
    async fn cancellation_interrupts_a_delayed_http_response() {
        let initialize = json!({
            "jsonrpc": "2.0", "id": 1,
            "result": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "fixture", "version": "1.0" }
            }
        });
        let (url, server) = spawn_fixture(vec![
            response(initialize),
            FixtureResponse {
                status: "202 Accepted",
                content_type: "application/json",
                body: String::new(),
                headers: Vec::new(),
                delay: Duration::ZERO,
            },
            FixtureResponse {
                delay: Duration::from_millis(250),
                ..response(json!({ "jsonrpc": "2.0", "id": 2, "result": { "tools": [] } }))
            },
        ]);
        let client = McpHttpClient::connect(fixture_config(url), CancellationToken::new())
            .await
            .expect("connect");
        let cancellation = CancellationToken::new();
        let trigger = cancellation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            trigger.cancel();
        });
        assert!(matches!(
            client.list_tools(cancellation).await,
            Err(McpClientError::Cancelled)
        ));
        server.join().expect("server thread").expect("requests");
    }

    #[tokio::test]
    async fn redirect_and_oversized_response_fail_closed() {
        let (url, redirect_server) = spawn_fixture(vec![FixtureResponse {
            status: "302 Found",
            content_type: "text/plain",
            body: String::new(),
            headers: vec![("Location", "http://127.0.0.1:9/credential-sink")],
            delay: Duration::ZERO,
        }]);
        let redirected =
            McpHttpClient::connect(fixture_config(url), CancellationToken::new()).await;
        assert!(matches!(redirected, Err(McpClientError::Transport(_))));
        redirect_server
            .join()
            .expect("redirect server")
            .expect("redirect request");

        let (url, size_server) = spawn_fixture(vec![FixtureResponse {
            body: "x".repeat(5_000),
            ..response(Value::Null)
        }]);
        let mut config = fixture_config(url);
        config.max_message_bytes = 4 * 1024;
        assert!(matches!(
            McpHttpClient::connect(config, CancellationToken::new()).await,
            Err(McpClientError::InvalidResponse(
                "message exceeded configured byte budget"
            ))
        ));
        size_server
            .join()
            .expect("size server")
            .expect("size request");
    }
}
