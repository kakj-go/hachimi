use aes::{
    Aes128,
    cipher::{Block, BlockEncrypt as _, KeyInit as _},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use hachimi_protocol::{ChannelChatKind, IntegrationProviderId, VerifiedChannelMessage};
use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;
use uuid::Uuid;

use crate::{
    ProviderAdapter, ProviderError, ProviderEventFrame, TransportProof,
    normalize::{NormalizedFields, finish, string},
};

pub const ILINK_ORIGIN: &str = "https://ilinkai.weixin.qq.com";
pub const ILINK_RELOGIN_CODE: i64 = -14;
const ILINK_APP_ID: &str = "bot";
const ILINK_CLIENT_VERSION: &str = "131072";
const ILINK_CHANNEL_VERSION: &str = "2.0.0";
const ILINK_LONG_POLL_TIMEOUT_SECS: u64 = 35;
pub const ILINK_TEXT_LIMIT: usize = 4_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IlinkQrCode {
    pub qrcode: String,
    pub image_content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IlinkQrStatus {
    Waiting,
    Scanned,
    Expired,
    Confirmed(IlinkConfirmedCredential),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IlinkConfirmedCredential {
    pub bot_token: String,
    pub bot_id: String,
    pub base_url: String,
    pub ilink_user_id: String,
}

#[derive(Debug, Clone)]
pub struct WechatIlinkClient {
    http: reqwest::Client,
    base_url: String,
    bot_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IlinkUpdateBatch {
    pub cursor: String,
    pub messages: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IlinkSendReceipt {
    pub client_id: String,
    pub external_message_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IlinkMediaKind {
    Image,
    File,
    Video,
}

#[derive(Debug, Deserialize)]
struct IlinkApiResponse {
    #[serde(default)]
    ret: i64,
    #[serde(default)]
    errcode: i64,
    #[serde(default)]
    get_updates_buf: String,
    #[serde(default)]
    msgs: Vec<Value>,
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    upload_full_url: String,
    #[serde(default)]
    upload_param: String,
}

#[derive(Debug, Deserialize)]
struct FetchQrResponse {
    qrcode: String,
    qrcode_img_content: String,
}

#[derive(Debug, Deserialize)]
struct PollQrResponse {
    status: String,
    bot_token: Option<String>,
    ilink_bot_id: Option<String>,
    baseurl: Option<String>,
    ilink_user_id: Option<String>,
}

impl Default for WechatIlinkClient {
    fn default() -> Self {
        Self {
            http: ilink_http_client().expect("fixed iLink HTTP client configuration"),
            base_url: ILINK_ORIGIN.into(),
            bot_token: None,
        }
    }
}

impl WechatIlinkClient {
    pub fn authenticated(
        base_url: impl Into<String>,
        bot_token: impl Into<String>,
    ) -> Result<Self, ProviderError> {
        let base_url = base_url.into();
        WechatIlinkAdapter::validate_base_url(&base_url)?;
        let bot_token = bot_token.into();
        if bot_token.trim().is_empty() {
            return Err(ProviderError::InvalidEvent);
        }
        Ok(Self {
            http: ilink_http_client()?,
            base_url,
            bot_token: Some(bot_token),
        })
    }

    pub async fn fetch_qr_code(&self) -> Result<IlinkQrCode, ProviderError> {
        let response = self
            .http
            .get(format!("{ILINK_ORIGIN}/ilink/bot/get_bot_qrcode"))
            .query(&[("bot_type", "3")])
            .send()
            .await
            .map_err(|_| ProviderError::RuntimeNotReady)?
            .error_for_status()
            .map_err(|_| ProviderError::RuntimeNotReady)?
            .json::<FetchQrResponse>()
            .await
            .map_err(|_| ProviderError::InvalidEvent)?;
        if response.qrcode.trim().is_empty() || response.qrcode_img_content.trim().is_empty() {
            return Err(ProviderError::InvalidEvent);
        }
        Ok(IlinkQrCode {
            qrcode: response.qrcode,
            image_content: response.qrcode_img_content,
        })
    }

    pub async fn poll_qr_status(&self, qrcode: &str) -> Result<IlinkQrStatus, ProviderError> {
        if qrcode.trim().is_empty() {
            return Err(ProviderError::InvalidEvent);
        }
        let response = self
            .http
            .get(format!("{ILINK_ORIGIN}/ilink/bot/get_qrcode_status"))
            .query(&[("qrcode", qrcode)])
            .header("iLink-App-Id", ILINK_APP_ID)
            .header("iLink-App-ClientVersion", ILINK_CLIENT_VERSION)
            .send()
            .await
            .map_err(|_| ProviderError::RuntimeNotReady)?
            .error_for_status()
            .map_err(|_| ProviderError::RuntimeNotReady)?
            .json::<PollQrResponse>()
            .await
            .map_err(|_| ProviderError::InvalidEvent)?;
        map_qr_response(response)
    }

    pub async fn get_updates(&self, cursor: &str) -> Result<IlinkUpdateBatch, ProviderError> {
        let response = self
            .post_api(
                "ilink/bot/getupdates",
                &json!({
                    "get_updates_buf": cursor,
                    "base_info": {"channel_version": ILINK_CHANNEL_VERSION},
                }),
                ILINK_LONG_POLL_TIMEOUT_SECS + 5,
            )
            .await?;
        Ok(IlinkUpdateBatch {
            cursor: if response.get_updates_buf.is_empty() {
                cursor.to_owned()
            } else {
                response.get_updates_buf
            },
            messages: response.msgs,
        })
    }

    pub async fn send_text(
        &self,
        to_user_id: &str,
        text: &str,
        context_token: &str,
        idempotency_key: &str,
    ) -> Result<IlinkSendReceipt, ProviderError> {
        if to_user_id.trim().is_empty()
            || context_token.trim().is_empty()
            || text.trim().is_empty()
            || text.chars().count() > ILINK_TEXT_LIMIT
        {
            return Err(ProviderError::InvalidEvent);
        }
        let client_id = stable_client_id(idempotency_key)?;
        let response = self
            .post_api(
                "ilink/bot/sendmessage",
                &json!({
                    "msg": {
                        "from_user_id": "",
                        "to_user_id": to_user_id,
                        "client_id": client_id,
                        "message_type": 2,
                        "message_state": 2,
                        "item_list": [{"type": 1, "text_item": {"text": text}}],
                        "context_token": context_token,
                    },
                    "base_info": {"channel_version": ILINK_CHANNEL_VERSION},
                }),
                15,
            )
            .await?;
        Ok(IlinkSendReceipt {
            client_id,
            external_message_id: response.message_id,
        })
    }

    pub async fn send_media(
        &self,
        to_user_id: &str,
        context_token: &str,
        kind: IlinkMediaKind,
        file_name: &str,
        bytes: &[u8],
        idempotency_key: &str,
    ) -> Result<IlinkSendReceipt, ProviderError> {
        if to_user_id.trim().is_empty()
            || context_token.trim().is_empty()
            || file_name.trim().is_empty()
            || file_name.chars().count() > 255
            || bytes.is_empty()
            || bytes.len() > 25 * 1024 * 1024
        {
            return Err(ProviderError::MediaLimit);
        }
        let client_id = stable_client_id(idempotency_key)?;
        let upload = self
            .upload_media(to_user_id, kind, bytes, idempotency_key)
            .await?;
        let media = json!({
            "encrypt_query_param": upload.encrypt_query_param,
            "aes_key": upload.aes_key_b64,
            "encrypt_type": 1,
        });
        let item = match kind {
            IlinkMediaKind::Image => json!({
                "type": 2,
                "image_item": {"media": media, "mid_size": upload.ciphertext_size},
            }),
            IlinkMediaKind::File => json!({
                "type": 4,
                "file_item": {"media": media, "file_name": file_name, "len": bytes.len().to_string()},
            }),
            IlinkMediaKind::Video => json!({
                "type": 5,
                "video_item": {"media": media, "video_size": upload.ciphertext_size},
            }),
        };
        let response = self
            .post_api(
                "ilink/bot/sendmessage",
                &json!({
                    "msg": {
                        "from_user_id": "",
                        "to_user_id": to_user_id,
                        "client_id": client_id,
                        "message_type": 2,
                        "message_state": 2,
                        "item_list": [item],
                        "context_token": context_token,
                    },
                    "base_info": {"channel_version": ILINK_CHANNEL_VERSION},
                }),
                30,
            )
            .await?;
        Ok(IlinkSendReceipt {
            client_id,
            external_message_id: response.message_id,
        })
    }

    async fn upload_media(
        &self,
        to_user_id: &str,
        kind: IlinkMediaKind,
        bytes: &[u8],
        idempotency_key: &str,
    ) -> Result<IlinkMediaUpload, ProviderError> {
        let mut key = [0_u8; 16];
        key.copy_from_slice(&Uuid::new_v4().into_bytes());
        let encrypted = encrypt_ilink_media(bytes, &key)?;
        let file_key = stable_file_key(idempotency_key);
        let raw_md5 = format!("{:x}", md5::compute(bytes));
        let response = self
            .post_api(
                "ilink/bot/getuploadurl",
                &json!({
                    "filekey": file_key,
                    "media_type": match kind { IlinkMediaKind::Image => 1, IlinkMediaKind::Video => 2, IlinkMediaKind::File => 3 },
                    "to_user_id": to_user_id,
                    "rawsize": bytes.len(),
                    "rawfilemd5": raw_md5,
                    "filesize": encrypted.len(),
                    "aeskey": hex_key(&key),
                    "no_need_thumb": true,
                    "base_info": {"channel_version": ILINK_CHANNEL_VERSION},
                }),
                30,
            )
            .await?;
        let url = ilink_upload_url(&response, &file_key)?;
        let result = self
            .http
            .post(url)
            .timeout(std::time::Duration::from_secs(120))
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(encrypted.clone())
            .send()
            .await
            .map_err(|_| ProviderError::RuntimeNotReady)?;
        if !result.status().is_success() {
            return Err(if result.status().is_server_error() {
                ProviderError::RuntimeNotReady
            } else {
                ProviderError::InvalidEvent
            });
        }
        let encrypt_query_param = result
            .headers()
            .get("x-encrypted-param")
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty() && value.len() <= 4096)
            .ok_or(ProviderError::InvalidEvent)?
            .to_owned();
        Ok(IlinkMediaUpload {
            encrypt_query_param,
            aes_key_b64: BASE64.encode(hex_key(&key).as_bytes()),
            ciphertext_size: encrypted.len(),
        })
    }

    async fn post_api(
        &self,
        endpoint: &str,
        body: &Value,
        timeout_secs: u64,
    ) -> Result<IlinkApiResponse, ProviderError> {
        let token = self
            .bot_token
            .as_deref()
            .ok_or(ProviderError::AuthenticationExpired)?;
        let endpoint = format!("{}/{}", self.base_url.trim_end_matches('/'), endpoint);
        let response = self
            .http
            .post(endpoint)
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .header("AuthorizationType", "ilink_bot_token")
            .header("Authorization", format!("Bearer {token}"))
            .header("X-WECHAT-UIN", random_wechat_uin())
            .header("iLink-App-Id", ILINK_APP_ID)
            .header("iLink-App-ClientVersion", ILINK_CLIENT_VERSION)
            .json(body)
            .send()
            .await
            .map_err(|_| ProviderError::RuntimeNotReady)?
            .error_for_status()
            .map_err(|_| ProviderError::RuntimeNotReady)?
            .json::<IlinkApiResponse>()
            .await
            .map_err(|_| ProviderError::InvalidEvent)?;
        classify_api_response(response)
    }
}

fn ilink_http_client() -> Result<reqwest::Client, ProviderError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ProviderError::RuntimeNotReady)
}

struct IlinkMediaUpload {
    encrypt_query_param: String,
    aes_key_b64: String,
    ciphertext_size: usize,
}

fn encrypt_ilink_media(bytes: &[u8], key: &[u8; 16]) -> Result<Vec<u8>, ProviderError> {
    let mut encrypted = bytes.to_vec();
    let padding = 16 - encrypted.len() % 16;
    encrypted.extend(std::iter::repeat_n(padding as u8, padding));
    let cipher = Aes128::new_from_slice(key).map_err(|_| ProviderError::InvalidEvent)?;
    for block in encrypted.chunks_exact_mut(16) {
        cipher.encrypt_block(Block::<Aes128>::from_mut_slice(block));
    }
    Ok(encrypted)
}

fn ilink_upload_url(
    response: &IlinkApiResponse,
    file_key: &str,
) -> Result<reqwest::Url, ProviderError> {
    let url = if response.upload_full_url.is_empty() {
        if response.upload_param.is_empty() {
            return Err(ProviderError::InvalidEvent);
        }
        let mut url = reqwest::Url::parse("https://novac2c.cdn.weixin.qq.com/c2c/upload")
            .map_err(|_| ProviderError::EndpointDenied)?;
        url.query_pairs_mut()
            .append_pair("encrypted_query_param", &response.upload_param)
            .append_pair("filekey", file_key);
        url
    } else {
        reqwest::Url::parse(&response.upload_full_url).map_err(|_| ProviderError::EndpointDenied)?
    };
    if url.scheme() != "https"
        || url.host_str() != Some("novac2c.cdn.weixin.qq.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|port| port != 443)
    {
        return Err(ProviderError::EndpointDenied);
    }
    Ok(url)
}

fn stable_file_key(value: &str) -> String {
    use sha2::{Digest as _, Sha256};
    Sha256::digest(format!("ilink-upload:{value}").as_bytes())[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hex_key(key: &[u8]) -> String {
    key.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn classify_api_response(response: IlinkApiResponse) -> Result<IlinkApiResponse, ProviderError> {
    if response.ret == ILINK_RELOGIN_CODE || response.errcode == ILINK_RELOGIN_CODE {
        return Err(ProviderError::AuthenticationExpired);
    }
    if response.ret != 0 || response.errcode != 0 {
        return Err(ProviderError::RuntimeNotReady);
    }
    Ok(response)
}

fn random_wechat_uin() -> String {
    let bytes = Uuid::new_v4().into_bytes();
    BASE64.encode(u32::from_le_bytes(bytes[..4].try_into().expect("uuid prefix")).to_string())
}

fn stable_client_id(idempotency_key: &str) -> Result<String, ProviderError> {
    use sha2::{Digest as _, Sha256};
    if idempotency_key.trim().is_empty() {
        return Err(ProviderError::InvalidEvent);
    }
    Ok(Sha256::digest(idempotency_key.as_bytes())[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn map_qr_response(response: PollQrResponse) -> Result<IlinkQrStatus, ProviderError> {
    match response.status.as_str() {
        "wait" => Ok(IlinkQrStatus::Waiting),
        "scaned" => Ok(IlinkQrStatus::Scanned),
        "expired" => Ok(IlinkQrStatus::Expired),
        "confirmed" => {
            let credential = IlinkConfirmedCredential {
                bot_token: required(response.bot_token)?,
                bot_id: required(response.ilink_bot_id)?,
                base_url: required(response.baseurl)?,
                ilink_user_id: required(response.ilink_user_id)?,
            };
            WechatIlinkAdapter::validate_base_url(&credential.base_url)?;
            Ok(IlinkQrStatus::Confirmed(credential))
        }
        _ => Err(ProviderError::InvalidEvent),
    }
}

fn required(value: Option<String>) -> Result<String, ProviderError> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or(ProviderError::InvalidEvent)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WechatIlinkAdapter;

impl WechatIlinkAdapter {
    pub fn validate_base_url(value: &str) -> Result<(), ProviderError> {
        let url = Url::parse(value).map_err(|_| ProviderError::EndpointDenied)?;
        let host = url.host_str().ok_or(ProviderError::EndpointDenied)?;
        if url.scheme() != "https"
            || (!host.eq_ignore_ascii_case("ilinkai.weixin.qq.com")
                && !host.ends_with(".ilinkai.weixin.qq.com"))
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(ProviderError::EndpointDenied);
        }
        Ok(())
    }

    pub fn classify_error(code: i64) -> Result<(), ProviderError> {
        if code == ILINK_RELOGIN_CODE {
            Err(ProviderError::AuthenticationExpired)
        } else {
            Ok(())
        }
    }
}

impl ProviderAdapter for WechatIlinkAdapter {
    fn provider_id(&self) -> IntegrationProviderId {
        IntegrationProviderId::WechatIlink
    }

    fn normalize(
        &self,
        frame: ProviderEventFrame,
    ) -> Result<VerifiedChannelMessage, ProviderError> {
        let TransportProof::IlinkPoll {
            bot_id,
            received_at_ms,
        } = &frame.proof
        else {
            return Err(ProviderError::InvalidTransportProof);
        };
        let payload = &frame.payload;
        if string(payload, &["/bot_id"]).is_some_and(|value| value != bot_id) {
            return Err(ProviderError::InvalidTransportProof);
        }
        if string(payload, &["/context_token"]).is_none() {
            return Err(ProviderError::InvalidEvent);
        }
        match payload.get("message_type").and_then(Value::as_i64) {
            Some(1) | None => {}
            Some(2) => return Err(ProviderError::BotLoop),
            Some(_) => return Err(ProviderError::InvalidEvent),
        }
        let message_id =
            string(payload, &["/message_id", "/msgid"]).ok_or(ProviderError::InvalidEvent)?;
        let actor_id =
            string(payload, &["/from_user_id", "/from_user"]).ok_or(ProviderError::InvalidEvent)?;
        let normalized_payload = normalized_ilink_payload(payload)?;
        finish(NormalizedFields {
            provider_id: self.provider_id(),
            account_id: &frame.account_id,
            tenant_key: &frame.tenant_key,
            message_id,
            chat_kind: ChannelChatKind::Dm,
            chat_id: actor_id,
            topic_id: None,
            actor_id,
            actor_name: string(payload, &["/from_user_name"]),
            text: string(&normalized_payload, &["/text"]),
            payload: &normalized_payload,
            received_at_ms: *received_at_ms,
        })
    }
}

fn normalized_ilink_payload(payload: &Value) -> Result<Value, ProviderError> {
    let mut text = String::new();
    let mut attachments = Vec::new();
    let mut media_secrets = Vec::new();
    let items = payload
        .get("item_list")
        .and_then(Value::as_array)
        .ok_or(ProviderError::InvalidEvent)?;
    for item in items {
        match item.get("type").and_then(Value::as_i64) {
            Some(1) => {
                if let Some(value) = string(item, &["/text_item/text"]) {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(value);
                }
            }
            Some(kind @ 2..=5) => {
                let (attachment, secret) = ilink_media_descriptor(item, kind)?;
                attachments.push(attachment);
                media_secrets.push(secret);
            }
            _ => {}
        }
    }
    Ok(json!({
        "text": text,
        "attachments": attachments,
        "_media_secrets": media_secrets,
        "context_token": payload.get("context_token"),
    }))
}

fn ilink_media_descriptor(item: &Value, kind: i64) -> Result<(Value, Value), ProviderError> {
    let key = match kind {
        2 => "image_item",
        3 => "voice_item",
        4 => "file_item",
        5 => "video_item",
        _ => return Err(ProviderError::InvalidEvent),
    };
    let details = item.get(key).ok_or(ProviderError::InvalidEvent)?;
    let media = details.get("media").ok_or(ProviderError::InvalidEvent)?;
    let remote_id = string(media, &["/encrypt_query_param"])
        .filter(|value| !value.is_empty())
        .ok_or(ProviderError::InvalidEvent)?;
    let aes_key = string(details, &["/aeskey"])
        .or_else(|| string(media, &["/aes_key"]))
        .filter(|value| !value.is_empty())
        .ok_or(ProviderError::InvalidEvent)?;
    let size = details
        .get("len")
        .or_else(|| details.get("mid_size"))
        .or_else(|| details.get("video_size"))
        .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()));
    let kind_name = match kind {
        2 => "image",
        3 => "audio",
        5 => "video",
        _ => "file",
    };
    Ok((
        json!({
            "kind": kind_name,
            "remote_id": remote_id,
            "resource_key": "ilink_cdn",
            "file_name": details.get("file_name").and_then(Value::as_str),
            "size": size,
        }),
        json!({
            "remote_id": remote_id,
            "cipher": "aes_128_ecb",
            "aes_key": aes_key,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_status_requires_complete_confirmed_credentials_and_allowed_host() {
        let scanned = map_qr_response(PollQrResponse {
            status: "scaned".into(),
            bot_token: None,
            ilink_bot_id: None,
            baseurl: None,
            ilink_user_id: None,
        })
        .expect("scanned");
        assert_eq!(scanned, IlinkQrStatus::Scanned);

        let confirmed = map_qr_response(PollQrResponse {
            status: "confirmed".into(),
            bot_token: Some("secret".into()),
            ilink_bot_id: Some("bot-1".into()),
            baseurl: Some(ILINK_ORIGIN.into()),
            ilink_user_id: Some("user-1".into()),
        })
        .expect("confirmed");
        assert!(matches!(confirmed, IlinkQrStatus::Confirmed(_)));

        let untrusted = map_qr_response(PollQrResponse {
            status: "confirmed".into(),
            bot_token: Some("secret".into()),
            ilink_bot_id: Some("bot-1".into()),
            baseurl: Some("https://example.com".into()),
            ilink_user_id: Some("user-1".into()),
        });
        assert_eq!(untrusted, Err(ProviderError::EndpointDenied));
    }

    #[test]
    fn update_items_become_typed_parts_and_bot_messages_are_rejected() {
        let adapter = WechatIlinkAdapter;
        let message = adapter
            .normalize(ProviderEventFrame {
                account_id: "account-1".into(),
                tenant_key: "bot-1".into(),
                payload: json!({
                    "message_id": "message-1",
                    "message_type": 1,
                    "from_user_id": "user-1",
                    "context_token": "context-1",
                    "item_list": [
                        {"type": 1, "text_item": {"text": "hello"}},
                        {"type": 2, "image_item": {"media": {
                            "encrypt_query_param": "remote-image",
                            "aes_key": "encrypted-key"
                        }, "mid_size": 12}}
                    ]
                }),
                proof: TransportProof::IlinkPoll {
                    bot_id: "bot-1".into(),
                    received_at_ms: 42,
                },
            })
            .expect("normalized update");
        assert_eq!(message.parts.len(), 2);
        assert_eq!(message.address.chat_id, "user-1");

        let bot = adapter.normalize(ProviderEventFrame {
            account_id: "account-1".into(),
            tenant_key: "bot-1".into(),
            payload: json!({
                "message_id": "message-2",
                "message_type": 2,
                "from_user_id": "bot-1",
                "context_token": "context-1",
                "item_list": [{"type": 1, "text_item": {"text": "loop"}}]
            }),
            proof: TransportProof::IlinkPoll {
                bot_id: "bot-1".into(),
                received_at_ms: 43,
            },
        });
        assert_eq!(bot, Err(ProviderError::BotLoop));
    }

    #[test]
    fn api_error_and_idempotency_mapping_are_deterministic() {
        assert!(matches!(
            classify_api_response(IlinkApiResponse {
                ret: -14,
                errcode: 0,
                get_updates_buf: String::new(),
                msgs: Vec::new(),
                message_id: None,
                upload_full_url: String::new(),
                upload_param: String::new(),
            }),
            Err(ProviderError::AuthenticationExpired)
        ));
        assert_eq!(stable_client_id("run:item:0").expect("client id").len(), 16);
        assert_eq!(
            stable_client_id("run:item:0"),
            stable_client_id("run:item:0")
        );
    }

    #[test]
    fn media_encryption_and_upload_host_are_deterministic_and_restricted() {
        let key = [0x31_u8; 16];
        let encrypted = encrypt_ilink_media(b"fixture", &key).expect("encrypt");
        assert_eq!(encrypted.len(), 16);
        assert_ne!(&encrypted[..7], b"fixture");

        let legacy = IlinkApiResponse {
            ret: 0,
            errcode: 0,
            get_updates_buf: String::new(),
            msgs: Vec::new(),
            message_id: None,
            upload_full_url: String::new(),
            upload_param: "opaque".into(),
        };
        assert_eq!(
            ilink_upload_url(&legacy, "file-key")
                .expect("legacy URL")
                .host_str(),
            Some("novac2c.cdn.weixin.qq.com")
        );
        let denied = IlinkApiResponse {
            upload_full_url: "https://example.com/upload".into(),
            upload_param: String::new(),
            ..legacy
        };
        assert_eq!(
            ilink_upload_url(&denied, "file-key"),
            Err(ProviderError::EndpointDenied)
        );
    }
}
