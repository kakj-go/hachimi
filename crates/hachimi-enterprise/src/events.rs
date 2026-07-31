use aes::Aes256;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use cbc::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
use hachimi_protocol::{
    EnterpriseAttachmentMetadata, EnterpriseMention, EnterpriseMentionKind, EnterprisePlatform,
};
use serde_json::Value;
use sha1::{Digest as _, Sha1};
use sha2::Sha256;
use thiserror::Error;
use zeroize::Zeroize;

use crate::EnterpriseCredential;

const MAX_EVENT_SKEW_MS: i64 = 5 * 60 * 1_000;
const MAX_EVENT_TEXT_CHARS: usize = 32_000;
const MAX_ATTACHMENTS: usize = 32;
const MAX_MENTIONS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnterpriseEventAuth {
    WecomCallback {
        timestamp: String,
        nonce: String,
        signature: String,
        encrypted: String,
    },
    Stream {
        timestamp_ms: i64,
        connection_id: String,
        transport_authenticated: bool,
    },
    LongConnection {
        timestamp_ms: i64,
        connection_id: String,
        transport_authenticated: bool,
        verification_token: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnterpriseRawEvent {
    pub platform: EnterprisePlatform,
    pub account_id: String,
    pub tenant_id: String,
    pub event_id: Option<String>,
    pub event_type: Option<String>,
    pub peer: Option<String>,
    pub thread: Option<String>,
    pub sender: Option<String>,
    pub text: Option<String>,
    pub mentions: Vec<EnterpriseMention>,
    pub attachments: Vec<EnterpriseAttachmentMetadata>,
    pub payload: Value,
    pub auth: EnterpriseEventAuth,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedEnterpriseEvent {
    pub platform: EnterprisePlatform,
    pub account_id: String,
    pub tenant_id: String,
    pub event_id: String,
    pub event_type: String,
    pub peer: String,
    pub thread: String,
    pub sender: String,
    pub text: String,
    pub mentions: Vec<EnterpriseMention>,
    pub attachments: Vec<EnterpriseAttachmentMetadata>,
    pub payload_hash: String,
    pub received_at_ms: i64,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EnterpriseEventError {
    #[error("enterprise event credential does not match its platform")]
    CredentialMismatch,
    #[error("enterprise event transport is not authenticated")]
    Unauthenticated,
    #[error("enterprise event signature is invalid")]
    InvalidSignature,
    #[error("enterprise event is outside its replay window")]
    ReplayWindow,
    #[error("enterprise event tenant identity does not match")]
    TenantMismatch,
    #[error("enterprise event payload is invalid")]
    InvalidPayload,
}

impl EnterpriseEventError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::CredentialMismatch => "enterprise_event_credential_mismatch",
            Self::Unauthenticated => "enterprise_event_unauthenticated",
            Self::InvalidSignature => "enterprise_event_signature_invalid",
            Self::ReplayWindow => "enterprise_event_replay_window",
            Self::TenantMismatch => "enterprise_event_tenant_mismatch",
            Self::InvalidPayload => "enterprise_event_payload_invalid",
        }
    }
}

pub fn verify_enterprise_event(
    credential: &EnterpriseCredential,
    raw: EnterpriseRawEvent,
    now_ms: i64,
) -> Result<VerifiedEnterpriseEvent, EnterpriseEventError> {
    if credential.platform() != raw.platform
        || raw.account_id.trim().is_empty()
        || raw.account_id.len() > 128
        || raw.attachments.len() > MAX_ATTACHMENTS
        || raw.mentions.len() > MAX_MENTIONS
        || !mentions_valid(&raw.mentions)
    {
        return Err(EnterpriseEventError::CredentialMismatch);
    }
    match raw.auth.clone() {
        EnterpriseEventAuth::WecomCallback {
            timestamp,
            nonce,
            signature,
            encrypted,
        } => verify_wecom(
            credential,
            raw.account_id,
            timestamp,
            nonce,
            signature,
            encrypted,
            raw.mentions,
            raw.attachments,
            now_ms,
        ),
        EnterpriseEventAuth::Stream {
            timestamp_ms,
            connection_id,
            transport_authenticated,
        } => {
            if raw.platform != EnterprisePlatform::DingTalk {
                return Err(EnterpriseEventError::CredentialMismatch);
            }
            verify_stream(
                credential,
                raw,
                timestamp_ms,
                &connection_id,
                transport_authenticated,
                None,
                now_ms,
            )
        }
        EnterpriseEventAuth::LongConnection {
            timestamp_ms,
            connection_id,
            transport_authenticated,
            verification_token,
        } => {
            if raw.platform != EnterprisePlatform::Feishu {
                return Err(EnterpriseEventError::CredentialMismatch);
            }
            verify_stream(
                credential,
                raw,
                timestamp_ms,
                &connection_id,
                transport_authenticated,
                verification_token.as_deref(),
                now_ms,
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn verify_wecom(
    credential: &EnterpriseCredential,
    account_id: String,
    timestamp: String,
    nonce: String,
    signature: String,
    encrypted: String,
    mut mentions: Vec<EnterpriseMention>,
    attachments: Vec<EnterpriseAttachmentMetadata>,
    now_ms: i64,
) -> Result<VerifiedEnterpriseEvent, EnterpriseEventError> {
    let timestamp_seconds = timestamp
        .parse::<i64>()
        .map_err(|_| EnterpriseEventError::InvalidPayload)?;
    validate_replay_window(timestamp_seconds.saturating_mul(1_000), now_ms)?;
    if nonce.is_empty() || nonce.len() > 256 || signature.len() != 40 || encrypted.len() > 1_048_576
    {
        return Err(EnterpriseEventError::InvalidPayload);
    }
    let (token, encoding_key) = credential
        .wecom_callback()
        .ok_or(EnterpriseEventError::CredentialMismatch)?;
    let mut parts = [
        token,
        timestamp.as_str(),
        nonce.as_str(),
        encrypted.as_str(),
    ];
    parts.sort_unstable();
    let mut hasher = Sha1::new();
    for part in parts {
        hasher.update(part.as_bytes());
    }
    let expected = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if !constant_time_eq(expected.as_bytes(), signature.as_bytes()) {
        return Err(EnterpriseEventError::InvalidSignature);
    }
    let mut message = decrypt_wecom(encoding_key, &encrypted)?;
    let tenant_id = message.1.clone();
    if tenant_id != credential.tenant_id() {
        message.0.zeroize();
        return Err(EnterpriseEventError::TenantMismatch);
    }
    let xml = String::from_utf8(message.0).map_err(|_| EnterpriseEventError::InvalidPayload)?;
    let event_id = xml_tag(&xml, "MsgId")
        .or_else(|| xml_tag(&xml, "EventId"))
        .unwrap_or_else(|| digest_hex(xml.as_bytes()));
    let event_type = xml_tag(&xml, "Event")
        .or_else(|| xml_tag(&xml, "MsgType"))
        .unwrap_or_else(|| "message".into());
    let sender = xml_tag(&xml, "FromUserName").unwrap_or_else(|| "unknown".into());
    let peer = xml_tag(&xml, "ChatId")
        .or_else(|| xml_tag(&xml, "ToUserName"))
        .unwrap_or_else(|| sender.clone());
    let text = xml_tag(&xml, "Content").unwrap_or_default();
    if mentions.is_empty() {
        mentions = mentions_from_wecom_xml(&xml, &text);
    }
    validate_normalized(&event_id, &event_type, &peer, &sender, &text)?;
    Ok(VerifiedEnterpriseEvent {
        platform: EnterprisePlatform::Wecom,
        account_id,
        tenant_id,
        event_id,
        event_type,
        peer: peer.clone(),
        thread: peer,
        sender,
        text,
        mentions,
        attachments,
        payload_hash: digest_hex(xml.as_bytes()),
        received_at_ms: now_ms,
    })
}

#[allow(clippy::too_many_arguments)]
fn verify_stream(
    credential: &EnterpriseCredential,
    raw: EnterpriseRawEvent,
    timestamp_ms: i64,
    connection_id: &str,
    transport_authenticated: bool,
    verification_token: Option<&str>,
    now_ms: i64,
) -> Result<VerifiedEnterpriseEvent, EnterpriseEventError> {
    validate_replay_window(timestamp_ms, now_ms)?;
    if !transport_authenticated || connection_id.is_empty() || connection_id.len() > 256 {
        return Err(EnterpriseEventError::Unauthenticated);
    }
    if raw.tenant_id != credential.tenant_id() {
        return Err(EnterpriseEventError::TenantMismatch);
    }
    if credential.platform() == EnterprisePlatform::Feishu
        && let Some(expected) = credential.feishu_verification_token()
        && !constant_time_eq(
            expected.as_bytes(),
            verification_token.unwrap_or_default().as_bytes(),
        )
    {
        return Err(EnterpriseEventError::InvalidSignature);
    }
    let event_id = raw.event_id.ok_or(EnterpriseEventError::InvalidPayload)?;
    let event_type = raw.event_type.ok_or(EnterpriseEventError::InvalidPayload)?;
    let peer = raw.peer.ok_or(EnterpriseEventError::InvalidPayload)?;
    let thread = raw.thread.unwrap_or_else(|| peer.clone());
    let sender = raw.sender.ok_or(EnterpriseEventError::InvalidPayload)?;
    let text = raw.text.unwrap_or_default();
    validate_normalized(&event_id, &event_type, &peer, &sender, &text)?;
    let payload_hash = digest_hex(
        &serde_json::to_vec(&raw.payload).map_err(|_| EnterpriseEventError::InvalidPayload)?,
    );
    Ok(VerifiedEnterpriseEvent {
        platform: raw.platform,
        account_id: raw.account_id,
        tenant_id: raw.tenant_id,
        event_id,
        event_type,
        peer,
        thread,
        sender,
        text,
        mentions: raw.mentions,
        attachments: raw.attachments,
        payload_hash,
        received_at_ms: now_ms,
    })
}

fn decrypt_wecom(
    encoding_key: &str,
    encrypted: &str,
) -> Result<(Vec<u8>, String), EnterpriseEventError> {
    let mut key = STANDARD
        .decode(format!("{encoding_key}="))
        .map_err(|_| EnterpriseEventError::InvalidPayload)?;
    if key.len() != 32 {
        key.zeroize();
        return Err(EnterpriseEventError::InvalidPayload);
    }
    let iv = key[..16].to_vec();
    let mut ciphertext = STANDARD
        .decode(encrypted)
        .map_err(|_| EnterpriseEventError::InvalidPayload)?;
    let plaintext = cbc::Decryptor::<Aes256>::new_from_slices(&key, &iv)
        .map_err(|_| EnterpriseEventError::InvalidPayload)?
        .decrypt_padded_mut::<Pkcs7>(&mut ciphertext)
        .map_err(|_| EnterpriseEventError::InvalidPayload)?;
    if plaintext.len() < 20 {
        key.zeroize();
        ciphertext.zeroize();
        return Err(EnterpriseEventError::InvalidPayload);
    }
    let length = u32::from_be_bytes(
        plaintext[16..20]
            .try_into()
            .map_err(|_| EnterpriseEventError::InvalidPayload)?,
    ) as usize;
    if length > 1_048_576 || plaintext.len() < 20 + length {
        key.zeroize();
        ciphertext.zeroize();
        return Err(EnterpriseEventError::InvalidPayload);
    }
    let message = plaintext[20..20 + length].to_vec();
    let tenant = String::from_utf8(plaintext[20 + length..].to_vec())
        .map_err(|_| EnterpriseEventError::InvalidPayload)?;
    key.zeroize();
    ciphertext.zeroize();
    Ok((message, tenant))
}

fn validate_replay_window(timestamp_ms: i64, now_ms: i64) -> Result<(), EnterpriseEventError> {
    if now_ms.abs_diff(timestamp_ms) > u64::try_from(MAX_EVENT_SKEW_MS).unwrap_or(u64::MAX) {
        Err(EnterpriseEventError::ReplayWindow)
    } else {
        Ok(())
    }
}

fn validate_normalized(
    event_id: &str,
    event_type: &str,
    peer: &str,
    sender: &str,
    text: &str,
) -> Result<(), EnterpriseEventError> {
    if event_id.is_empty()
        || event_id.len() > 512
        || event_type.is_empty()
        || event_type.len() > 256
        || peer.is_empty()
        || peer.len() > 512
        || sender.is_empty()
        || sender.len() > 512
        || text.chars().count() > MAX_EVENT_TEXT_CHARS
    {
        return Err(EnterpriseEventError::InvalidPayload);
    }
    Ok(())
}

fn mentions_valid(mentions: &[EnterpriseMention]) -> bool {
    mentions.iter().all(|mention| {
        mention
            .target_id
            .as_deref()
            .is_none_or(|value| !value.is_empty() && value.len() <= 512)
            && mention
                .display_text
                .as_deref()
                .is_none_or(|value| !value.is_empty() && value.chars().count() <= 512)
            && (mention.kind == EnterpriseMentionKind::All || mention.target_id.is_some())
    })
}

fn mentions_from_wecom_xml(xml: &str, text: &str) -> Vec<EnterpriseMention> {
    let mut mentions = Vec::new();
    if text.contains("@all") || text.contains("@所有人") {
        mentions.push(EnterpriseMention {
            kind: EnterpriseMentionKind::All,
            target_id: None,
            display_text: Some("@all".into()),
        });
    }
    if let Some(user_id) = xml_tag(xml, "MentionedUserId")
        && !user_id.is_empty()
    {
        mentions.push(EnterpriseMention {
            kind: EnterpriseMentionKind::User,
            target_id: Some(user_id),
            display_text: None,
        });
    }
    mentions
}

fn xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    let value = xml[start..end]
        .trim()
        .strip_prefix("<![CDATA[")
        .and_then(|value| value.strip_suffix("]]>"))
        .unwrap_or(xml[start..end].trim());
    Some(
        value
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&")
            .replace("&quot;", "\"")
            .replace("&apos;", "'"),
    )
}

fn digest_hex(bytes: &[u8]) -> String {
    <Sha256 as sha2::Digest>::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use cbc::cipher::{BlockEncryptMut, block_padding::Pkcs7};

    fn wecom_fixture(
        token: &str,
        encoding_key: &str,
        tenant: &str,
        timestamp: &str,
        nonce: &str,
        xml: &str,
    ) -> (String, String) {
        let key = STANDARD
            .decode(format!("{encoding_key}="))
            .expect("encoding key");
        let mut plaintext = vec![0x5a; 16];
        plaintext.extend(u32::try_from(xml.len()).expect("xml length").to_be_bytes());
        plaintext.extend(xml.as_bytes());
        plaintext.extend(tenant.as_bytes());
        let message_len = plaintext.len();
        plaintext.resize(message_len + 16, 0);
        let encrypted = cbc::Encryptor::<Aes256>::new_from_slices(&key, &key[..16])
            .expect("cipher")
            .encrypt_padded_mut::<Pkcs7>(&mut plaintext, message_len)
            .expect("encrypt");
        let encrypted = STANDARD.encode(encrypted);
        let mut parts = [token, timestamp, nonce, encrypted.as_str()];
        parts.sort_unstable();
        let mut hasher = Sha1::new();
        for part in parts {
            hasher.update(part.as_bytes());
        }
        let signature = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        (encrypted, signature)
    }

    #[test]
    fn wecom_encrypted_callback_fixture_verifies_and_normalizes_mentions() {
        let token = "callback-token";
        let tenant = "corp-fixture";
        let timestamp = "2000";
        let nonce = "nonce-fixture";
        let key = [0x42_u8; 32];
        let encoding_key = STANDARD.encode(key).trim_end_matches('=').to_owned();
        let xml = "<xml><ToUserName><![CDATA[corp-fixture]]></ToUserName><FromUserName><![CDATA[user-1]]></FromUserName><MsgType><![CDATA[text]]></MsgType><Content><![CDATA[hello @all]]></Content><MsgId>event-wecom-1</MsgId><MentionedUserId>user-2</MentionedUserId></xml>";
        let (encrypted, signature) =
            wecom_fixture(token, &encoding_key, tenant, timestamp, nonce, xml);
        let credential = EnterpriseCredential::parse(&format!(
            r#"{{"platform":"wecom","corpId":"{tenant}","corpSecret":"secret","agentId":1,"callbackToken":"{token}","encodingAesKey":"{encoding_key}"}}"#
        ))
        .expect("credential");
        let verified = verify_enterprise_event(
            &credential,
            EnterpriseRawEvent {
                platform: EnterprisePlatform::Wecom,
                account_id: "account-wecom".into(),
                tenant_id: tenant.into(),
                event_id: None,
                event_type: None,
                peer: None,
                thread: None,
                sender: None,
                text: None,
                mentions: Vec::new(),
                attachments: Vec::new(),
                payload: Value::Null,
                auth: EnterpriseEventAuth::WecomCallback {
                    timestamp: timestamp.into(),
                    nonce: nonce.into(),
                    signature,
                    encrypted,
                },
            },
            2_000_000,
        )
        .expect("verified callback");
        assert_eq!(verified.event_id, "event-wecom-1");
        assert_eq!(verified.sender, "user-1");
        assert!(
            verified
                .mentions
                .iter()
                .any(|mention| mention.kind == EnterpriseMentionKind::All)
        );
        assert!(verified.mentions.iter().any(|mention| {
            mention.kind == EnterpriseMentionKind::User
                && mention.target_id.as_deref() == Some("user-2")
        }));
    }

    #[test]
    fn stream_events_require_transport_auth_and_exact_tenant() {
        let credential = EnterpriseCredential::parse(
            r#"{"platform":"ding_talk","appKey":"tenant","appSecret":"secret","agentId":null,"robotCode":"robot"}"#,
        )
        .expect("credential");
        let raw = EnterpriseRawEvent {
            platform: EnterprisePlatform::DingTalk,
            account_id: "account".into(),
            tenant_id: "tenant".into(),
            event_id: Some("event".into()),
            event_type: Some("chat.message".into()),
            peer: Some("peer".into()),
            thread: Some("thread".into()),
            sender: Some("sender".into()),
            text: Some("hello".into()),
            mentions: vec![EnterpriseMention {
                kind: EnterpriseMentionKind::User,
                target_id: Some("sender".into()),
                display_text: Some("@sender".into()),
            }],
            attachments: Vec::new(),
            payload: serde_json::json!({"eventId": "event"}),
            auth: EnterpriseEventAuth::Stream {
                timestamp_ms: 1_000,
                connection_id: "connection".into(),
                transport_authenticated: true,
            },
        };
        assert!(verify_enterprise_event(&credential, raw, 1_000).is_ok());
    }

    #[test]
    fn replay_window_is_bounded() {
        assert_eq!(
            validate_replay_window(0, MAX_EVENT_SKEW_MS + 1),
            Err(EnterpriseEventError::ReplayWindow)
        );
    }
}
