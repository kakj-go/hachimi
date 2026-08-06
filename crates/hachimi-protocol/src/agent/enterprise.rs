//! Integration Provider, Connector, and EventSource contracts.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::{
    ArtifactId, AttachmentId, ChannelAccountState, ChannelAuthorization, ConnectorAccountId,
    IntegrationProviderId, MutationContext, PluginId, RunId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationTransport {
    EncryptedCallback,
    Stream,
    LongConnection,
    WebSocket,
    QrLongPoll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationCapability {
    ApiAccess,
    Messaging,
    Dm,
    Group,
    Topic,
    MediaReceive,
    MediaSend,
    ProactiveDelivery,
    QrLogin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationAuthMethod {
    ClientSecret,
    BotSecret,
    CallbackSecret,
    QrCode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CredentialFieldKind {
    Text,
    Secret,
    Integer,
    HttpsUrl,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CredentialFieldDefinition {
    pub id: String,
    pub label: String,
    pub kind: CredentialFieldKind,
    pub required: bool,
    pub capability: Option<IntegrationCapability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationProviderDefinition {
    pub id: IntegrationProviderId,
    pub name_zh: String,
    pub name_en: String,
    pub icon_asset: String,
    pub transport: IntegrationTransport,
    pub auth_method: IntegrationAuthMethod,
    pub capabilities: Vec<IntegrationCapability>,
    pub credential_fields: Vec<CredentialFieldDefinition>,
    pub source_status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "providerId",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum IntegrationCredentialInput {
    #[serde(rename = "dingtalk")]
    DingTalk {
        client_id: String,
        client_secret: String,
        agent_id: Option<String>,
        robot_code: Option<String>,
    },
    #[serde(rename = "feishu")]
    Feishu { app_id: String, app_secret: String },
    #[serde(rename = "wecom_ai_bot")]
    WecomAiBot { bot_id: String, secret: String },
    #[serde(rename = "wecom_app")]
    WecomApp {
        corp_id: String,
        corp_secret: String,
        agent_id: String,
        callback_token: String,
        encoding_aes_key: String,
        external_https_url: String,
    },
    #[serde(rename = "wechat_ilink")]
    WechatIlink {
        bot_token: String,
        bot_id: String,
        base_url: String,
    },
}

impl IntegrationCredentialInput {
    #[must_use]
    pub const fn provider_id(&self) -> IntegrationProviderId {
        match self {
            Self::DingTalk { .. } => IntegrationProviderId::DingTalk,
            Self::Feishu { .. } => IntegrationProviderId::Feishu,
            Self::WecomAiBot { .. } => IntegrationProviderId::WecomAiBot,
            Self::WecomApp { .. } => IntegrationProviderId::WecomApp,
            Self::WechatIlink { .. } => IntegrationProviderId::WechatIlink,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationProviderAccount {
    pub id: String,
    pub display_name: String,
    pub provider_id: IntegrationProviderId,
    pub connector_account_id: Option<ConnectorAccountId>,
    pub channel_account_id: Option<String>,
    pub tenant_identity_hash: String,
    pub transport: IntegrationTransport,
    pub state: ChannelAccountState,
    pub diagnostic: Option<String>,
    pub api_access_enabled: bool,
    pub messaging_enabled: bool,
    pub authorizations: Vec<ChannelAuthorization>,
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
    #[specta(type = specta_typescript::Number)]
    pub consecutive_failures: u32,
    pub probe: Option<IntegrationAccountProbeSnapshot>,
    #[specta(type = specta_typescript::Number)]
    pub credential_revision: u64,
    #[specta(type = specta_typescript::Number)]
    pub config_revision: u64,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationAccountUpsert {
    pub id: String,
    pub display_name: String,
    pub credential: IntegrationCredentialInput,
    pub api_access_enabled: bool,
    pub messaging_enabled: bool,
    #[specta(type = Option<specta_typescript::Number>)]
    pub expected_config_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationAccountCapabilitiesUpdate {
    pub id: String,
    pub api_access_enabled: bool,
    pub messaging_enabled: bool,
    #[specta(type = specta_typescript::Number)]
    pub expected_config_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationProbeDimension {
    pub ok: bool,
    pub result_code: String,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationAccountProbeSnapshot {
    pub credential: IntegrationProbeDimension,
    pub ingress: IntegrationProbeDimension,
    pub egress: IntegrationProbeDimension,
    pub api: IntegrationProbeDimension,
    #[specta(type = specta_typescript::Number)]
    pub probed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationAccountProbeResult {
    pub account: IntegrationProviderAccount,
    pub credential: IntegrationProbeDimension,
    pub ingress: IntegrationProbeDimension,
    pub egress: IntegrationProbeDimension,
    pub api: IntegrationProbeDimension,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IlinkQrSession {
    pub account_id: String,
    pub qr_content: String,
    pub state: String,
    #[specta(type = specta_typescript::Number)]
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IlinkQrLoginRequest {
    pub account_id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EnterpriseAttachmentDownloadRequest {
    pub context: MutationContext,
    pub provider_id: IntegrationProviderId,
    pub account_id: String,
    pub event_id: String,
    pub remote_id: String,
    pub metadata_hash: String,
    pub run_id: RunId,
    #[specta(type = specta_typescript::Number)]
    pub run_generation: u64,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EnterpriseAttachmentDownloadResult {
    pub artifact_id: ArtifactId,
    pub attachment_id: AttachmentId,
    pub content_hash: String,
    pub mime_type: String,
    #[specta(type = specta_typescript::Number)]
    pub byte_size: u64,
    pub duplicate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EnterprisePluginIdentity {
    pub plugin_id: PluginId,
    pub provider_id: IntegrationProviderId,
    pub connector_id: String,
    pub channel_id: String,
}
