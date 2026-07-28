// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/app-server-protocol/src/protocol/v2/mcp.rs
// and codex-rs/app-server/tests/suite/v2/mcp_server_elicitation.rs.
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// Modified for Hachimi: transport-neutral request dispatch, bounded identities, and Run correlation.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use hachimi_protocol::{RunId, SessionId, ToolCallId};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const MAX_REQUEST_ID_CHARS: usize = 256;
const MAX_METHOD_CHARS: usize = 128;
const MAX_ERROR_CHARS: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpServerRequestId {
    Integer(i64),
    String(String),
}

impl McpServerRequestId {
    pub(crate) fn parse(value: &Value) -> Option<Self> {
        if let Some(value) = value.as_i64() {
            return Some(Self::Integer(value));
        }
        let value = value.as_str()?;
        if value.is_empty() || value.chars().count() > MAX_REQUEST_ID_CHARS {
            return None;
        }
        Some(Self::String(value.to_owned()))
    }

    #[must_use]
    pub fn as_json(&self) -> Value {
        match self {
            Self::Integer(value) => json!(value),
            Self::String(value) => json!(value),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpRunCorrelation {
    pub session_id: SessionId,
    pub run_id: RunId,
    pub run_generation: u64,
    pub tool_call_id: ToolCallId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct McpServerRequest {
    pub server_id: String,
    pub request_id: McpServerRequestId,
    pub method: String,
    pub params: Value,
    pub correlation: Option<McpRunCorrelation>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum McpServerRequestResponse {
    Result(Value),
    Error { code: i64, message: String },
}

impl McpServerRequestResponse {
    #[must_use]
    pub fn result(value: Value) -> Self {
        Self::Result(value)
    }

    #[must_use]
    pub fn method_not_found() -> Self {
        Self::Error {
            code: -32_601,
            message: "server-initiated method is not supported".into(),
        }
    }

    #[must_use]
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::Error {
            code: -32_602,
            message: bounded(message.into(), MAX_ERROR_CHARS),
        }
    }
}

pub type McpServerRequestFuture = BoxFuture<'static, McpServerRequestResponse>;

pub trait McpServerRequestHandler: Send + Sync {
    fn handle(
        &self,
        request: McpServerRequest,
        cancellation: CancellationToken,
    ) -> McpServerRequestFuture;
}

pub(crate) async fn dispatch_server_request(
    server_id: &str,
    message: &Value,
    correlation: Option<McpRunCorrelation>,
    handler: Option<Arc<dyn McpServerRequestHandler>>,
    cancellation: CancellationToken,
) -> Value {
    let Some(request_id) = message.get("id").and_then(McpServerRequestId::parse) else {
        return json!({
            "jsonrpc": "2.0",
            "id": message.get("id").cloned().unwrap_or(Value::Null),
            "error": { "code": -32600, "message": "server request ID is invalid" }
        });
    };
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return json!({
            "jsonrpc": "2.0",
            "id": request_id.as_json(),
            "error": { "code": -32600, "message": "server request method is missing" }
        });
    };
    if method.is_empty() || method.chars().count() > MAX_METHOD_CHARS {
        return json!({
            "jsonrpc": "2.0",
            "id": request_id.as_json(),
            "error": { "code": -32600, "message": "server request method is invalid" }
        });
    }

    let response = if let Some(handler) = handler {
        handler
            .handle(
                McpServerRequest {
                    server_id: server_id.to_owned(),
                    request_id: request_id.clone(),
                    method: method.to_owned(),
                    params: message.get("params").cloned().unwrap_or_else(|| json!({})),
                    correlation,
                },
                cancellation,
            )
            .await
    } else if method == "elicitation/create" {
        // Connections outlive Runs. Cancel when no current Run owns this request so an MCP server
        // cannot fabricate input or wait forever after a disconnect.
        McpServerRequestResponse::Result(json!({ "action": "cancel" }))
    } else {
        McpServerRequestResponse::method_not_found()
    };

    match response {
        McpServerRequestResponse::Result(result) => json!({
            "jsonrpc": "2.0",
            "id": request_id.as_json(),
            "result": result,
        }),
        McpServerRequestResponse::Error { code, message } => json!({
            "jsonrpc": "2.0",
            "id": request_id.as_json(),
            "error": { "code": code, "message": bounded(message, MAX_ERROR_CHARS) },
        }),
    }
}

fn bounded(value: String, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use futures_util::FutureExt;

    use super::*;

    struct Accept;

    impl McpServerRequestHandler for Accept {
        fn handle(
            &self,
            request: McpServerRequest,
            _cancellation: CancellationToken,
        ) -> McpServerRequestFuture {
            async move {
                assert_eq!(
                    request.request_id,
                    McpServerRequestId::String("ask-1".into())
                );
                McpServerRequestResponse::result(json!({
                    "action": "accept",
                    "content": { "confirmed": true }
                }))
            }
            .boxed()
        }
    }

    #[tokio::test]
    async fn preserves_request_identity_for_handler_response() {
        let response = dispatch_server_request(
            "fixture",
            &json!({
                "jsonrpc": "2.0",
                "id": "ask-1",
                "method": "elicitation/create",
                "params": { "message": "Confirm" }
            }),
            None,
            Some(Arc::new(Accept)),
            CancellationToken::new(),
        )
        .await;
        assert_eq!(response["id"], "ask-1");
        assert_eq!(response["result"]["action"], "accept");
    }

    #[tokio::test]
    async fn missing_interactive_handler_cancels_elicitation() {
        let response = dispatch_server_request(
            "fixture",
            &json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "elicitation/create",
                "params": {}
            }),
            None,
            None,
            CancellationToken::new(),
        )
        .await;
        assert_eq!(response["id"], 7);
        assert_eq!(response["result"]["action"], "cancel");
    }
}
