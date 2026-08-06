use hachimi_protocol::{ChannelChatKind, IntegrationProviderId, VerifiedChannelMessage};

use crate::{
    ProviderAdapter, ProviderError, ProviderEventFrame, TransportProof,
    normalize::{NormalizedFields, finish, string},
};

#[derive(Debug, Clone, Default)]
pub struct FeishuAdapter {
    bot_open_id: Option<String>,
}

impl FeishuAdapter {
    #[must_use]
    pub const fn new(bot_open_id: Option<String>) -> Self {
        Self { bot_open_id }
    }
}

impl ProviderAdapter for FeishuAdapter {
    fn provider_id(&self) -> IntegrationProviderId {
        IntegrationProviderId::Feishu
    }

    fn normalize(
        &self,
        frame: ProviderEventFrame,
    ) -> Result<VerifiedChannelMessage, ProviderError> {
        let TransportProof::Stream { received_at_ms, .. } = frame.proof else {
            return Err(ProviderError::InvalidTransportProof);
        };
        let payload = &frame.payload;
        let message_id = string(payload, &["/event/message/message_id", "/message_id"])
            .ok_or(ProviderError::InvalidEvent)?;
        let actor_id = string(
            payload,
            &["/event/sender/sender_id/open_id", "/sender/open_id"],
        )
        .ok_or(ProviderError::InvalidEvent)?;
        if self.bot_open_id.as_deref() == Some(actor_id) {
            return Err(ProviderError::BotLoop);
        }
        let chat_id = string(payload, &["/event/message/chat_id", "/chat_id"])
            .ok_or(ProviderError::InvalidEvent)?;
        let chat_kind = match string(payload, &["/event/message/chat_type", "/chat_type"]) {
            Some("p2p") => ChannelChatKind::Dm,
            _ => ChannelChatKind::Group,
        };
        // A message id is never a topic. Only stable thread/root identifiers qualify.
        let topic_id = string(
            payload,
            &[
                "/event/message/thread_id",
                "/event/message/root_id",
                "/thread_id",
                "/root_id",
            ],
        )
        .filter(|topic| *topic != message_id);
        finish(NormalizedFields {
            provider_id: self.provider_id(),
            account_id: &frame.account_id,
            tenant_key: &frame.tenant_key,
            message_id,
            chat_kind,
            chat_id,
            topic_id,
            actor_id,
            actor_name: string(payload, &["/event/sender/name"]),
            text: string(payload, &["/text", "/event/message/text"]),
            payload,
            received_at_ms,
        })
    }
}
