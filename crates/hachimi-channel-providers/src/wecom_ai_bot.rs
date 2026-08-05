use hachimi_protocol::{ChannelChatKind, IntegrationProviderId, VerifiedChannelMessage};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::{
    ProviderAdapter, ProviderError, ProviderEventFrame, TransportProof,
    normalize::{NormalizedFields, finish, string},
};

pub const OPENWS_ENDPOINT: &str = "wss://openws.work.weixin.qq.com";
pub const SUBSCRIBE_ACK_TIMEOUT_SECS: u64 = 10;
pub const HEARTBEAT_INTERVAL_SECS: u64 = 30;

#[derive(Debug, Clone, Copy, Default)]
pub struct WecomAiBotAdapter;

impl WecomAiBotAdapter {
    #[must_use]
    pub fn subscribe_frame(bot_id: &str, secret: &str) -> Value {
        json!({"cmd":"aibot_subscribe","headers":{"req_id":uuid_like_request_id()},"body":{"bot_id":bot_id,"secret":secret}})
    }
}

impl ProviderAdapter for WecomAiBotAdapter {
    fn provider_id(&self) -> IntegrationProviderId {
        IntegrationProviderId::WecomAiBot
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
            string(payload, &["/msgid", "/msg_id"]).ok_or(ProviderError::InvalidEvent)?;
        let actor_id = string(payload, &["/from/userid", "/userid", "/sender/userid"])
            .ok_or(ProviderError::InvalidEvent)?;
        let chat_id = string(payload, &["/chatid"]);
        let (chat_kind, stable_chat_id) = match chat_id {
            Some(chat_id) => (ChannelChatKind::Group, chat_id),
            None => (ChannelChatKind::Dm, actor_id),
        };
        let normalized_payload = normalized_wecom_payload(payload)?;
        finish(NormalizedFields {
            provider_id: self.provider_id(),
            account_id: &frame.account_id,
            tenant_key: &frame.tenant_key,
            message_id,
            chat_kind,
            chat_id: stable_chat_id,
            topic_id: None,
            actor_id,
            actor_name: string(payload, &["/from/name"]),
            text: string(&normalized_payload, &["/text"]),
            payload: &normalized_payload,
            received_at_ms,
        })
    }
}

fn normalized_wecom_payload(payload: &Value) -> Result<Value, ProviderError> {
    let message_type = string(payload, &["/msgtype"]).unwrap_or("text");
    let mut text = String::new();
    let mut attachments = Vec::new();
    let mut media_secrets = Vec::new();
    if message_type == "text" || message_type == "markdown" {
        text = string(payload, &["/text/content", "/markdown/content", "/content"])
            .unwrap_or_default()
            .to_owned();
    } else if message_type == "voice" {
        text = string(payload, &["/voice/content"])
            .unwrap_or_default()
            .to_owned();
        append_media(payload, "voice", 0, &mut attachments, &mut media_secrets)?;
    } else if message_type == "mixed" {
        for (index, item) in payload
            .pointer("/mixed/msg_item")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let item_type = string(item, &["/msgtype"]).unwrap_or_default();
            if item_type == "text" {
                if let Some(value) = string(item, &["/text/content"]) {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(value);
                }
            } else {
                append_media(item, item_type, index, &mut attachments, &mut media_secrets)?;
            }
        }
    } else {
        append_media(
            payload,
            message_type,
            0,
            &mut attachments,
            &mut media_secrets,
        )?;
    }
    Ok(json!({
        "text": text,
        "attachments": attachments,
        "mentions": payload.get("mentions"),
        "_media_secrets": media_secrets,
        "_hachimi_req_id": payload.get("_hachimi_req_id"),
    }))
}

fn append_media(
    payload: &Value,
    message_type: &str,
    index: usize,
    attachments: &mut Vec<Value>,
    media_secrets: &mut Vec<Value>,
) -> Result<(), ProviderError> {
    let (field, kind) = match message_type {
        "image" => ("image", "image"),
        "file" => ("file", "file"),
        "voice" => ("voice", "audio"),
        "video" => ("video", "video"),
        _ => return Err(ProviderError::InvalidEvent),
    };
    let details = payload.get(field).ok_or(ProviderError::InvalidEvent)?;
    let download_url = string(details, &["/url", "/fileurl"])
        .filter(|value| !value.is_empty())
        .ok_or(ProviderError::InvalidEvent)?;
    let aes_key = string(details, &["/aeskey"])
        .filter(|value| !value.is_empty())
        .ok_or(ProviderError::InvalidEvent)?;
    let remote_id = string(details, &["/sdkfileid", "/fileid", "/md5sum", "/md5"])
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let digest = Sha256::digest(format!("{download_url}:{index}").as_bytes());
            digest.iter().map(|byte| format!("{byte:02x}")).collect()
        });
    let size = details
        .get("filesize")
        .or_else(|| details.get("size"))
        .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()));
    attachments.push(json!({
        "kind": kind,
        "remote_id": remote_id,
        "resource_key": "wecom_ai_encrypted_media",
        "file_name": string(details, &["/filename", "/name"]),
        "size": size,
    }));
    media_secrets.push(json!({
        "remote_id": remote_id,
        "cipher": "aes_256_cbc",
        "download_url": download_url,
        "aes_key": aes_key,
    }));
    Ok(())
}

fn uuid_like_request_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("hachimi-{nanos:032x}")
}
