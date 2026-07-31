//! Durable local Channel/Gateway contracts.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use super::{ChannelDeliveryId, ChannelMessageId, PluginId, RunId, SessionId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ChannelProviderRuntimeKind {
    Builtin,
    SandboxedStdioJsonRpc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelProviderManifest {
    pub id: String,
    pub plugin_id: Option<PluginId>,
    pub runtime_kind: ChannelProviderRuntimeKind,
    pub entrypoint: Option<String>,
    pub content_hash: String,
    pub required_scopes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ChannelProviderHealthState {
    Disabled,
    Starting,
    Healthy,
    NeedsAttention,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelProviderHealth {
    pub provider_id: String,
    pub state: ChannelProviderHealthState,
    pub diagnostic: Option<String>,
    #[specta(type = specta_typescript::Number)]
    pub config_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelProviderAccount {
    pub id: String,
    pub provider_id: String,
    pub display_name: String,
    pub secret_ref: Option<String>,
    pub enabled: bool,
    pub route_allowlist: Vec<ChannelRouteKey>,
    #[specta(type = specta_typescript::Number)]
    pub config_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelProviderAccountUpsert {
    pub id: String,
    pub provider_id: String,
    pub display_name: String,
    pub credential: Option<String>,
    pub enabled: bool,
    pub route_allowlist: Vec<ChannelRouteKey>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub expected_config_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelConfigRevision {
    pub provider_id: String,
    #[specta(type = specta_typescript::Number)]
    pub revision: u64,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelRouteKey {
    pub channel: String,
    pub account: String,
    pub peer: String,
    pub thread: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelEnvelope {
    pub message_id: ChannelMessageId,
    pub route: ChannelRouteKey,
    pub sender: String,
    pub text: String,
    #[specta(type = specta_typescript::Unknown)]
    pub metadata: Value,
    pub authenticated: bool,
    pub bot_generated: bool,
    #[specta(type = specta_typescript::Number)]
    pub received_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum IngressStatus {
    Accepted,
    Duplicate,
    Rejected,
    Claimed,
    Completed,
    NeedsAttention,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IngressReceipt {
    pub message_id: ChannelMessageId,
    pub status: IngressStatus,
    pub session_id: Option<SessionId>,
    pub run_id: Option<RunId>,
    pub result_code: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryAttemptStatus {
    Pending,
    Claimed,
    Delivered,
    RetryScheduled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryAttempt {
    pub id: ChannelDeliveryId,
    pub route: ChannelRouteKey,
    pub idempotency_key: String,
    pub text: String,
    pub status: DeliveryAttemptStatus,
    pub attempt: u32,
    #[specta(type = Option<specta_typescript::Number>)]
    pub next_attempt_at_ms: Option<i64>,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GatewayHealth {
    pub running: bool,
    pub startup_registered: bool,
    pub channels: Vec<String>,
    pub pending_ingress: u32,
    pub pending_deliveries: u32,
    #[specta(type = specta_typescript::Number)]
    pub revision: u64,
}
