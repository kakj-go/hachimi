//! Durable, provider-neutral Channel and Gateway contracts.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::RuntimeComponentState;

use super::{ChannelDeliveryId, ChannelMessageId, PluginId, RunId, SessionId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationProviderId {
    #[serde(rename = "dingtalk")]
    DingTalk,
    Feishu,
    WecomAiBot,
    WecomApp,
    WechatIlink,
}

impl IntegrationProviderId {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DingTalk => "dingtalk",
            Self::Feishu => "feishu",
            Self::WecomAiBot => "wecom_ai_bot",
            Self::WecomApp => "wecom_app",
            Self::WechatIlink => "wechat_ilink",
        }
    }

    #[must_use]
    pub const fn supports_enterprise_api(self) -> bool {
        matches!(self, Self::DingTalk | Self::Feishu | Self::WecomApp)
    }
}

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
pub enum ChannelAccountState {
    Draft,
    AwaitingAuth,
    Starting,
    Healthy,
    Degraded,
    NeedsAttention,
    Revoked,
    Removing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ChannelProviderHealthState {
    Disabled,
    Starting,
    Healthy,
    Degraded,
    NeedsAttention,
    Revoked,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelProviderHealth {
    pub provider_id: String,
    pub account_id: Option<String>,
    pub state: ChannelProviderHealthState,
    pub diagnostic: Option<String>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub last_event_at_ms: Option<i64>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub last_delivery_at_ms: Option<i64>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub last_handshake_at_ms: Option<i64>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub last_frame_at_ms: Option<i64>,
    pub last_error_code: Option<String>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub next_reconnect_at_ms: Option<i64>,
    pub consecutive_failures: u32,
    #[specta(type = specta_typescript::Number)]
    pub config_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelProviderAccount {
    pub id: String,
    pub provider_id: String,
    pub display_name: String,
    pub tenant_key: String,
    pub credential_ref: Option<String>,
    pub enabled: bool,
    pub state: ChannelAccountState,
    #[specta(type = specta_typescript::Unknown)]
    pub config: Value,
    #[specta(type = specta_typescript::Number)]
    pub credential_revision: u64,
    #[specta(type = specta_typescript::Number)]
    pub config_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelProviderAccountUpsert {
    pub id: String,
    pub provider_id: String,
    pub display_name: String,
    pub tenant_key: String,
    pub credential: Option<String>,
    pub enabled: bool,
    #[specta(type = specta_typescript::Unknown)]
    pub config: Value,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ChannelChatKind {
    Dm,
    Group,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelConversationAddress {
    pub provider_id: String,
    pub account_id: String,
    pub tenant_key: String,
    pub chat_kind: ChannelChatKind,
    pub chat_id: String,
    pub topic_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelEventKey {
    pub provider_id: String,
    pub account_id: String,
    pub external_message_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelActor {
    pub external_id: String,
    pub display_name: Option<String>,
    pub is_bot: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ChannelMessagePartKind {
    Text,
    Image,
    File,
    Audio,
    Video,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteMediaDescriptor {
    pub provider_id: IntegrationProviderId,
    pub remote_id: String,
    pub resource_key: Option<String>,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub declared_size_bytes: Option<u64>,
    pub content_hash: Option<String>,
    pub download_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ChannelMessagePart {
    Text { text: String },
    Image { media: RemoteMediaDescriptor },
    File { media: RemoteMediaDescriptor },
    Audio { media: RemoteMediaDescriptor },
    Video { media: RemoteMediaDescriptor },
}

impl ChannelMessagePart {
    #[must_use]
    pub const fn kind(&self) -> ChannelMessagePartKind {
        match self {
            Self::Text { .. } => ChannelMessagePartKind::Text,
            Self::Image { .. } => ChannelMessagePartKind::Image,
            Self::File { .. } => ChannelMessagePartKind::File,
            Self::Audio { .. } => ChannelMessagePartKind::Audio,
            Self::Video { .. } => ChannelMessagePartKind::Video,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ChannelMentionKind {
    User,
    Bot,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMention {
    pub kind: ChannelMentionKind,
    pub target_id: Option<String>,
    pub display_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelQuoteContext {
    pub external_message_id: String,
    pub actor_id: Option<String>,
    pub text_preview: Option<String>,
}

/// Only Provider implementations can construct this after transport authentication,
/// tenant validation, bot-loop detection, and replay checks have succeeded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedChannelMessage {
    pub event_key: ChannelEventKey,
    pub address: ChannelConversationAddress,
    pub actor: ChannelActor,
    pub parts: Vec<ChannelMessagePart>,
    pub mentions: Vec<ChannelMention>,
    pub quote: Option<ChannelQuoteContext>,
    #[specta(type = specta_typescript::Unknown)]
    pub provider_context: Value,
    #[specta(type = specta_typescript::Number)]
    pub received_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ChannelDmPolicy {
    Pairing,
    Allowlist,
    Open,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ChannelGroupHistoryPolicy {
    Shared,
    PerSender,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ChannelTopicPolicy {
    InheritGroup,
    IsolateTopic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ChannelMentionPolicy {
    Required,
    AllMessages,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ChannelAuthorizationTarget {
    DmIdentity,
    GroupConversation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChannelGrant {
    pub skill_ids: Vec<String>,
    pub mcp_server_ids: Vec<String>,
    pub connector_selections: Vec<super::ScheduleConnectorSelection>,
    pub read_only_workspace_roots: Vec<String>,
    pub network_hosts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelAccessPolicy {
    pub account_id: String,
    pub dm_policy: ChannelDmPolicy,
    pub allowlist_actor_ids: Vec<String>,
    pub grant_ceiling: ChannelGrant,
    #[specta(type = specta_typescript::Number)]
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelAccessPolicyUpsert {
    pub account_id: String,
    pub dm_policy: ChannelDmPolicy,
    pub allowlist_actor_ids: Vec<String>,
    pub grant_ceiling: ChannelGrant,
    #[specta(type = specta_typescript::Number)]
    pub expected_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelAuthorization {
    pub id: String,
    pub account_id: String,
    pub target: ChannelAuthorizationTarget,
    pub address: ChannelConversationAddress,
    pub actor_id: Option<String>,
    pub group_history_policy: Option<ChannelGroupHistoryPolicy>,
    pub topic_policy: ChannelTopicPolicy,
    pub mention_policy: ChannelMentionPolicy,
    pub grant: ChannelGrant,
    pub enabled: bool,
    #[specta(type = specta_typescript::Number)]
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelAuthorizationUpsert {
    pub id: String,
    pub account_id: String,
    pub target: ChannelAuthorizationTarget,
    pub address: ChannelConversationAddress,
    pub actor_id: Option<String>,
    pub group_history_policy: Option<ChannelGroupHistoryPolicy>,
    pub topic_policy: ChannelTopicPolicy,
    pub mention_policy: ChannelMentionPolicy,
    pub grant: ChannelGrant,
    pub enabled: bool,
    #[specta(type = Option<specta_typescript::Number>)]
    pub expected_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelPairingCodeRequest {
    pub account_id: String,
    pub target: ChannelAuthorizationTarget,
    pub group_history_policy: Option<ChannelGroupHistoryPolicy>,
    pub topic_policy: ChannelTopicPolicy,
    pub mention_policy: ChannelMentionPolicy,
    pub grant: ChannelGrant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelPairingCode {
    pub id: String,
    pub code: String,
    pub account_id: String,
    pub target: ChannelAuthorizationTarget,
    #[specta(type = specta_typescript::Number)]
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelIdentityLinkCodeRequest {
    pub account_id: String,
    pub actor_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelIdentityLinkCode {
    pub id: String,
    pub code: String,
    pub account_id: String,
    pub actor_id: String,
    #[specta(type = specta_typescript::Number)]
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelIdentityGroup {
    pub id: String,
    pub session_id: SessionId,
    pub member_count: u32,
    #[specta(type = specta_typescript::Number)]
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelIdentityTransferMember {
    pub external_identity_id: String,
    pub provider_id: String,
    pub account_id: String,
    pub tenant_key: String,
    pub actor_id: String,
    pub display_name: Option<String>,
    pub identity_group_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelIdentityTransferPreview {
    pub id: String,
    pub source: ChannelIdentityTransferMember,
    pub target: ChannelIdentityTransferMember,
    pub source_group_id: Option<String>,
    pub target_group_id: Option<String>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub source_group_revision: Option<u64>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub target_group_revision: Option<u64>,
    #[specta(type = specta_typescript::Number)]
    pub revision: u64,
    #[specta(type = specta_typescript::Number)]
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelIdentityTransferCommitRequest {
    pub id: String,
    #[specta(type = specta_typescript::Number)]
    pub expected_revision: u64,
    #[specta(type = Option<specta_typescript::Number>)]
    pub expected_source_group_revision: Option<u64>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub expected_target_group_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelIdentityTransferResult {
    pub identity_group: ChannelIdentityGroup,
    pub previous_source_group_id: Option<String>,
    pub previous_target_group_id: Option<String>,
    pub session_id: SessionId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum IngressStatus {
    Accepted,
    Duplicate,
    Rejected,
    Claimed,
    RunCreated,
    Completed,
    NeedsAttention,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IngressReceipt {
    pub event_key: ChannelEventKey,
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
    PermanentFailure,
    Indeterminate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelOutboundPayload {
    pub parts: Vec<ChannelMessagePart>,
    pub reply_to_external_message_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryAttempt {
    pub id: ChannelDeliveryId,
    pub address: ChannelConversationAddress,
    pub idempotency_key: String,
    pub payload: ChannelOutboundPayload,
    pub status: DeliveryAttemptStatus,
    pub attempt: u32,
    pub claim_token: Option<String>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub next_attempt_at_ms: Option<i64>,
    pub error_code: Option<String>,
    pub provider_receipt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GatewayHealth {
    pub running: bool,
    pub state: RuntimeComponentState,
    #[specta(type = Option<specta_typescript::Number>)]
    pub last_heartbeat_ms: Option<i64>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub last_started_at_ms: Option<i64>,
    pub restart_attempt: u32,
    pub last_error_code: Option<String>,
    pub channels: Vec<String>,
    pub pending_ingress: u32,
    pub pending_deliveries: u32,
    #[specta(type = specta_typescript::Number)]
    pub revision: u64,
}

impl VerifiedChannelMessage {
    #[must_use]
    pub fn message_id(&self) -> ChannelMessageId {
        ChannelMessageId::new(self.event_key.external_message_id.clone())
    }

    #[must_use]
    pub fn text(&self) -> String {
        self.parts
            .iter()
            .filter_map(|part| match part {
                ChannelMessagePart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
