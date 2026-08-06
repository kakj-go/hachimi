use hachimi_protocol::{ChannelChatKind, IntegrationProviderId, VerifiedChannelMessage};

use crate::{
    ProviderAdapter, ProviderError, ProviderEventFrame, TransportProof,
    normalize::{NormalizedFields, finish, string},
};

#[derive(Debug, Clone, Copy, Default)]
pub struct WecomAppAdapter;

impl WecomAppAdapter {
    #[must_use]
    pub fn callback_path(account_id: &str) -> Option<String> {
        valid_account_id(account_id)
            .then(|| format!("/v1/channels/wecom_app/{account_id}/callback"))
    }
}

impl ProviderAdapter for WecomAppAdapter {
    fn provider_id(&self) -> IntegrationProviderId {
        IntegrationProviderId::WecomApp
    }

    fn normalize(
        &self,
        frame: ProviderEventFrame,
    ) -> Result<VerifiedChannelMessage, ProviderError> {
        let TransportProof::SignedCallback { received_at_ms, .. } = frame.proof else {
            return Err(ProviderError::InvalidTransportProof);
        };
        let payload = &frame.payload;
        let message_id =
            string(payload, &["/MsgId", "/msgid"]).ok_or(ProviderError::InvalidEvent)?;
        let actor_id =
            string(payload, &["/FromUserName", "/from_user"]).ok_or(ProviderError::InvalidEvent)?;
        let chat_id = string(payload, &["/ChatId", "/chatid"]);
        let (chat_kind, stable_chat_id) = match chat_id {
            Some(chat_id) => (ChannelChatKind::Group, chat_id),
            None => (ChannelChatKind::Dm, actor_id),
        };
        finish(NormalizedFields {
            provider_id: self.provider_id(),
            account_id: &frame.account_id,
            tenant_key: &frame.tenant_key,
            message_id,
            chat_kind,
            chat_id: stable_chat_id,
            topic_id: None,
            actor_id,
            actor_name: None,
            text: string(payload, &["/Content", "/content"]),
            payload,
            received_at_ms,
        })
    }
}

fn valid_account_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
