//! Account-scoped adapters for the five official Channel integrations.

mod dingtalk;
mod feishu;
mod normalize;
mod runtime;
mod wechat_ilink;
mod wecom_ai_bot;
mod wecom_ai_bot_transport;
mod wecom_app;

pub use dingtalk::DingTalkAdapter;
pub use feishu::FeishuAdapter;
pub use runtime::{AccountRuntime, AccountRuntimeConfig, AccountRuntimeSnapshot, RuntimeProbe};
pub use wechat_ilink::{ILINK_ORIGIN, ILINK_RELOGIN_CODE, ILINK_TEXT_LIMIT};
pub use wechat_ilink::{
    IlinkConfirmedCredential, IlinkMediaKind, IlinkQrCode, IlinkQrStatus, IlinkSendReceipt,
    IlinkUpdateBatch, WechatIlinkAdapter, WechatIlinkClient,
};
pub use wecom_ai_bot::{
    HEARTBEAT_INTERVAL_SECS, OPENWS_ENDPOINT, SUBSCRIBE_ACK_TIMEOUT_SECS, WecomAiBotAdapter,
};
pub use wecom_ai_bot_transport::{
    WecomAiBotDeliveryResult, WecomAiBotMediaKind, WecomAiBotTransport, WecomAiBotTransportEvent,
};
pub use wecom_app::WecomAppAdapter;

use hachimi_protocol::{IntegrationProviderId, VerifiedChannelMessage};
use serde_json::Value;
use thiserror::Error;

pub const MAX_MESSAGE_PARTS: usize = 8;
pub const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
pub const MAX_MESSAGE_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportProof {
    Stream {
        connection_id: String,
        received_at_ms: i64,
    },
    SignedCallback {
        signature_fingerprint: String,
        received_at_ms: i64,
    },
    IlinkPoll {
        bot_id: String,
        received_at_ms: i64,
    },
}

impl TransportProof {
    #[must_use]
    pub const fn received_at_ms(&self) -> i64 {
        match self {
            Self::Stream { received_at_ms, .. }
            | Self::SignedCallback { received_at_ms, .. }
            | Self::IlinkPoll { received_at_ms, .. } => *received_at_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderEventFrame {
    pub account_id: String,
    pub tenant_key: String,
    pub payload: Value,
    pub proof: TransportProof,
}

pub trait ProviderAdapter: Send + Sync {
    fn provider_id(&self) -> IntegrationProviderId;

    fn normalize(&self, frame: ProviderEventFrame)
    -> Result<VerifiedChannelMessage, ProviderError>;
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProviderError {
    #[error("provider transport proof does not match the adapter")]
    InvalidTransportProof,
    #[error("provider event is malformed")]
    InvalidEvent,
    #[error("provider event belongs to the bot")]
    BotLoop,
    #[error("provider media exceeds the configured limits")]
    MediaLimit,
    #[error("provider endpoint is not allowed")]
    EndpointDenied,
    #[error("provider authentication must be renewed")]
    AuthenticationExpired,
    #[error("provider account runtime is not ready")]
    RuntimeNotReady,
}

pub fn official_adapters(bot_identity: Option<String>) -> Vec<Box<dyn ProviderAdapter>> {
    vec![
        Box::new(DingTalkAdapter),
        Box::new(FeishuAdapter::new(bot_identity)),
        Box::new(WecomAiBotAdapter),
        Box::new(WecomAppAdapter),
        Box::new(WechatIlinkAdapter),
    ]
}
