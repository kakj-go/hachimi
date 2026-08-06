use hachimi_protocol::{
    ChannelActor, ChannelChatKind, ChannelConversationAddress, ChannelEventKey, ChannelMention,
    ChannelMentionKind, ChannelMessagePart, IntegrationProviderId, RemoteMediaDescriptor,
    VerifiedChannelMessage,
};
use serde_json::Value;

use crate::{MAX_FILE_BYTES, MAX_MESSAGE_BYTES, MAX_MESSAGE_PARTS, ProviderError};

pub(crate) struct NormalizedFields<'a> {
    pub provider_id: IntegrationProviderId,
    pub account_id: &'a str,
    pub tenant_key: &'a str,
    pub message_id: &'a str,
    pub chat_kind: ChannelChatKind,
    pub chat_id: &'a str,
    pub topic_id: Option<&'a str>,
    pub actor_id: &'a str,
    pub actor_name: Option<&'a str>,
    pub text: Option<&'a str>,
    pub payload: &'a Value,
    pub received_at_ms: i64,
}

pub(crate) fn finish(
    fields: NormalizedFields<'_>,
) -> Result<VerifiedChannelMessage, ProviderError> {
    for value in [
        fields.account_id,
        fields.tenant_key,
        fields.message_id,
        fields.chat_id,
        fields.actor_id,
    ] {
        if value.trim().is_empty() || value.len() > 512 {
            return Err(ProviderError::InvalidEvent);
        }
    }
    if fields
        .topic_id
        .is_some_and(|topic| topic.trim().is_empty() || topic.len() > 512)
    {
        return Err(ProviderError::InvalidEvent);
    }
    let mut parts = Vec::new();
    if let Some(text) = fields.text.filter(|text| !text.trim().is_empty()) {
        if text.chars().count() > 32_000 {
            return Err(ProviderError::InvalidEvent);
        }
        parts.push(ChannelMessagePart::Text { text: text.into() });
    }
    parts.extend(media_parts(fields.provider_id, fields.payload)?);
    if parts.is_empty() || parts.len() > MAX_MESSAGE_PARTS {
        return Err(ProviderError::InvalidEvent);
    }
    let mentions = mentions(fields.payload);
    Ok(VerifiedChannelMessage {
        event_key: ChannelEventKey {
            provider_id: fields.provider_id.as_str().into(),
            account_id: fields.account_id.into(),
            external_message_id: fields.message_id.into(),
        },
        address: ChannelConversationAddress {
            provider_id: fields.provider_id.as_str().into(),
            account_id: fields.account_id.into(),
            tenant_key: fields.tenant_key.into(),
            chat_kind: fields.chat_kind,
            chat_id: fields.chat_id.into(),
            topic_id: fields.topic_id.map(str::to_owned),
        },
        actor: ChannelActor {
            external_id: fields.actor_id.into(),
            display_name: fields.actor_name.map(str::to_owned),
            is_bot: false,
        },
        parts,
        mentions,
        quote: None,
        provider_context: fields.payload.clone(),
        received_at_ms: fields.received_at_ms,
    })
}

fn media_parts(
    provider_id: IntegrationProviderId,
    payload: &Value,
) -> Result<Vec<ChannelMessagePart>, ProviderError> {
    let Some(values) = payload.get("attachments").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    if values.len() > MAX_MESSAGE_PARTS {
        return Err(ProviderError::MediaLimit);
    }
    let mut total = 0_u64;
    values
        .iter()
        .map(|value| {
            let remote_id = value
                .get("remote_id")
                .or_else(|| value.get("remoteId"))
                .or_else(|| value.get("file_key"))
                .or_else(|| value.get("image_key"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty() && value.len() <= 1024)
                .ok_or(ProviderError::InvalidEvent)?;
            let declared_size_bytes = value
                .get("size")
                .or_else(|| value.get("declaredSizeBytes"))
                .and_then(Value::as_u64);
            if declared_size_bytes.is_some_and(|size| size > MAX_FILE_BYTES) {
                return Err(ProviderError::MediaLimit);
            }
            total = total.saturating_add(declared_size_bytes.unwrap_or(0));
            if total > MAX_MESSAGE_BYTES {
                return Err(ProviderError::MediaLimit);
            }
            let media = RemoteMediaDescriptor {
                provider_id,
                remote_id: remote_id.into(),
                resource_key: value
                    .get("resource_key")
                    .or_else(|| value.get("resourceKey"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                file_name: value
                    .get("file_name")
                    .or_else(|| value.get("fileName"))
                    .and_then(Value::as_str)
                    .map(safe_file_name),
                mime_type: value
                    .get("mime_type")
                    .or_else(|| value.get("mimeType"))
                    .and_then(Value::as_str)
                    .filter(|value| value.len() <= 256)
                    .map(str::to_owned),
                declared_size_bytes,
                content_hash: None,
                download_required: true,
            };
            Ok(
                match value.get("kind").and_then(Value::as_str).unwrap_or("file") {
                    "image" => ChannelMessagePart::Image { media },
                    "audio" => ChannelMessagePart::Audio { media },
                    "video" => ChannelMessagePart::Video { media },
                    _ => ChannelMessagePart::File { media },
                },
            )
        })
        .collect()
}

fn mentions(payload: &Value) -> Vec<ChannelMention> {
    payload
        .get("mentions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(128)
        .filter_map(|value| {
            let kind = match value.get("kind").and_then(Value::as_str)? {
                "bot" => ChannelMentionKind::Bot,
                "all" => ChannelMentionKind::All,
                _ => ChannelMentionKind::User,
            };
            Some(ChannelMention {
                kind,
                target_id: value
                    .get("target_id")
                    .or_else(|| value.get("targetId"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                display_text: value
                    .get("display_text")
                    .or_else(|| value.get("displayText"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect()
}

fn safe_file_name(value: &str) -> String {
    value
        .chars()
        .filter(|character| !matches!(character, '/' | '\\' | '\0'))
        .take(255)
        .collect()
}

pub(crate) fn string<'a>(value: &'a Value, pointers: &[&str]) -> Option<&'a str> {
    pointers
        .iter()
        .find_map(|pointer| value.pointer(pointer).and_then(Value::as_str))
}
