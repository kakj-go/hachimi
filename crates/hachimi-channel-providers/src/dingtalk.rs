use hachimi_protocol::{ChannelChatKind, IntegrationProviderId, VerifiedChannelMessage};

use crate::{
    ProviderAdapter, ProviderError, ProviderEventFrame, TransportProof,
    normalize::{NormalizedFields, finish, string},
};

#[derive(Debug, Clone, Copy, Default)]
pub struct DingTalkAdapter;

impl ProviderAdapter for DingTalkAdapter {
    fn provider_id(&self) -> IntegrationProviderId {
        IntegrationProviderId::DingTalk
    }

    fn normalize(
        &self,
        frame: ProviderEventFrame,
    ) -> Result<VerifiedChannelMessage, ProviderError> {
        let TransportProof::Stream { received_at_ms, .. } = frame.proof else {
            return Err(ProviderError::InvalidTransportProof);
        };
        let payload = &frame.payload;
        let message_id =
            string(payload, &["/msgId", "/messageId"]).ok_or(ProviderError::InvalidEvent)?;
        let actor_id =
            string(payload, &["/senderStaffId", "/senderId"]).ok_or(ProviderError::InvalidEvent)?;
        let conversation_type = string(payload, &["/conversationType"]).unwrap_or("2");
        let (chat_kind, chat_id) = if conversation_type == "1" {
            (ChannelChatKind::Dm, actor_id)
        } else {
            (
                ChannelChatKind::Group,
                string(payload, &["/conversationId"]).ok_or(ProviderError::InvalidEvent)?,
            )
        };
        finish(NormalizedFields {
            provider_id: self.provider_id(),
            account_id: &frame.account_id,
            tenant_key: &frame.tenant_key,
            message_id,
            chat_kind,
            chat_id,
            topic_id: None,
            actor_id,
            actor_name: string(payload, &["/senderNick"]),
            text: string(payload, &["/text/content", "/content"]),
            payload,
            received_at_ms,
        })
    }
}
