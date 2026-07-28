use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use super::{McpServerId, RunId, SessionId, ToolCallId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum McpAuthStatus {
    Unsupported,
    NotLoggedIn,
    BearerToken,
    #[serde(rename = "oauth")]
    OAuth,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpAuthStatusRecord {
    pub server_id: McpServerId,
    pub status: McpAuthStatus,
    pub scopes_supported: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthLoginRequest {
    pub server_id: McpServerId,
    pub scopes: Vec<String>,
    pub timeout_secs: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthLoginResponse {
    pub authorization_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum McpCallOutcome {
    Succeeded,
    ToolError,
    TransportError,
    Cancelled,
}

impl McpCallOutcome {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::ToolError => "tool_error",
            Self::TransportError => "transport_error",
            Self::Cancelled => "cancelled",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "succeeded" => Self::Succeeded,
            "tool_error" => Self::ToolError,
            "transport_error" => Self::TransportError,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpCallSummaryRecord {
    pub id: ToolCallId,
    pub server_id: McpServerId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub tool_name: String,
    pub outcome: McpCallOutcome,
    #[specta(type = specta_typescript::Number)]
    pub duration_ms: u64,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpCallSummaryListRequest {
    pub server_id: Option<McpServerId>,
    pub session_id: Option<SessionId>,
    pub limit: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpToolProgressRecord {
    pub server_id: McpServerId,
    pub session_id: SessionId,
    pub run_id: RunId,
    #[specta(type = specta_typescript::Number)]
    pub run_generation: u64,
    pub tool_call_id: ToolCallId,
    pub progress: f64,
    pub total: Option<f64>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub mime_type: Option<String>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub size: Option<u64>,
    #[specta(type = Option<specta_typescript::Unknown>)]
    pub annotations: Option<Value>,
    #[specta(type = Option<specta_typescript::Unknown>)]
    pub meta: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpResourceTemplate {
    pub uri_template: String,
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub mime_type: Option<String>,
    #[specta(type = Option<specta_typescript::Unknown>)]
    pub annotations: Option<Value>,
    #[specta(type = Option<specta_typescript::Unknown>)]
    pub meta: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpResourceContent {
    pub uri: String,
    pub mime_type: Option<String>,
    pub text: Option<String>,
    pub blob_base64: Option<String>,
    #[serde(default)]
    pub content_reference: Option<McpMediaReference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpMediaReference {
    pub id: String,
    pub mime_type: String,
    #[specta(type = specta_typescript::Number)]
    pub byte_length: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpPromptArgument {
    pub name: String,
    pub description: Option<String>,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpPrompt {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub arguments: Vec<McpPromptArgument>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum McpPromptRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpPromptMessage {
    pub role: McpPromptRole,
    #[specta(type = specta_typescript::Unknown)]
    pub content: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpPromptResult {
    pub description: Option<String>,
    pub messages: Vec<McpPromptMessage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpInventorySnapshot {
    pub server_id: McpServerId,
    pub resources: Vec<McpResource>,
    pub resource_templates: Vec<McpResourceTemplate>,
    pub prompts: Vec<McpPrompt>,
    pub errors: BTreeMap<String, String>,
    pub stale: bool,
    #[specta(type = specta_typescript::Number)]
    pub refreshed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpResourceReadRequest {
    pub server_id: McpServerId,
    pub uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpPromptGetRequest {
    pub server_id: McpServerId,
    pub name: String,
    pub arguments: BTreeMap<String, String>,
}
