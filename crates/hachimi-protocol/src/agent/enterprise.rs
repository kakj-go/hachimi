//! Enterprise Connector, Channel, and EventSource contracts.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::{ArtifactId, ConnectorAccountId, MutationContext, PluginId, RunId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum EnterprisePlatform {
    Wecom,
    DingTalk,
    Feishu,
}

impl EnterprisePlatform {
    /// Stable storage and wire identifier used by enterprise records.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Wecom => "wecom",
            Self::DingTalk => "ding_talk",
            Self::Feishu => "feishu",
        }
    }

    /// Stable Channel Provider identifier. This intentionally differs from the
    /// storage spelling for DingTalk and must not be used in database columns.
    #[must_use]
    pub const fn channel_provider_id(self) -> &'static str {
        match self {
            Self::Wecom => "wecom",
            Self::DingTalk => "dingtalk",
            Self::Feishu => "feishu",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::EnterprisePlatform;

    #[test]
    fn storage_and_channel_platform_ids_are_explicit() {
        assert_eq!(EnterprisePlatform::DingTalk.as_str(), "ding_talk");
        assert_eq!(
            EnterprisePlatform::DingTalk.channel_provider_id(),
            "dingtalk"
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum EnterpriseIngressMode {
    EncryptedCallback,
    Stream,
    LongConnection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum EnterpriseIntegrationState {
    Disabled,
    Starting,
    Healthy,
    RateLimited,
    Revoked,
    NeedsAttention,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EnterpriseIntegrationAccount {
    pub id: String,
    pub platform: EnterprisePlatform,
    pub connector_account_id: Option<ConnectorAccountId>,
    pub channel_account_id: Option<String>,
    pub tenant_identity_hash: String,
    pub ingress_mode: EnterpriseIngressMode,
    pub event_source_id: String,
    pub state: EnterpriseIntegrationState,
    pub diagnostic: Option<String>,
    #[specta(type = specta_typescript::Number)]
    pub credential_revision: u64,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum EnterpriseEventReceiptStatus {
    Accepted,
    Duplicate,
    Acknowledged,
    DeadLetter,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EnterpriseEventReceipt {
    pub platform: EnterprisePlatform,
    pub account_id: String,
    pub event_id: String,
    pub event_type: String,
    pub payload_hash: String,
    pub status: EnterpriseEventReceiptStatus,
    pub result_code: String,
    #[specta(type = specta_typescript::Number)]
    pub received_at_ms: i64,
    #[specta(type = Option<specta_typescript::Number>)]
    pub acknowledged_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EnterpriseAttachmentMetadata {
    pub platform: EnterprisePlatform,
    pub remote_id: String,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub declared_size_bytes: Option<u64>,
    pub download_required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum EnterpriseMentionKind {
    User,
    Bot,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EnterpriseMention {
    pub kind: EnterpriseMentionKind,
    pub target_id: Option<String>,
    pub display_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EnterpriseAttachmentDownloadRequest {
    pub context: MutationContext,
    pub platform: EnterprisePlatform,
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
    pub platform: EnterprisePlatform,
    pub connector_id: String,
    pub channel_id: String,
}
