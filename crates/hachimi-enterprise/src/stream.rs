use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
    net::TcpStream,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use hachimi_protocol::{
    ChannelMention, ChannelMentionKind, IntegrationProviderId, RemoteMediaDescriptor,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(test)]
use serde_json::json;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tungstenite::{
    Message, connect,
    stream::{MaybeTlsStream, NoDelay},
};

use crate::{EnterpriseApiClient, EnterpriseCredential};

const MAX_STREAM_FRAME_BYTES: usize = 4 * 1024 * 1024;
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnterpriseStreamEndpoint {
    pub url: String,
    pub reconnect_delay: Duration,
    pub ping_interval: Duration,
}

impl EnterpriseStreamEndpoint {
    pub(crate) fn from_bootstrap(
        platform: IntegrationProviderId,
        value: &Value,
    ) -> Result<Self, ()> {
        let (url, reconnect_ms, ping_ms) = match platform {
            IntegrationProviderId::DingTalk => {
                let endpoint = value
                    .get("endpoint")
                    .or_else(|| value.pointer("/data/endpoint"))
                    .and_then(Value::as_str)
                    .ok_or(())?;
                let ticket = value
                    .get("ticket")
                    .or_else(|| value.pointer("/data/ticket"))
                    .and_then(Value::as_str)
                    .ok_or(())?;
                let separator = if endpoint.contains('?') { '&' } else { '?' };
                (
                    format!("{endpoint}{separator}ticket={ticket}"),
                    3_000,
                    30_000,
                )
            }
            IntegrationProviderId::Feishu => (
                value
                    .pointer("/data/URL")
                    .or_else(|| value.pointer("/data/url"))
                    .and_then(Value::as_str)
                    .ok_or(())?
                    .to_owned(),
                value
                    .pointer("/data/ClientConfig/ReconnectInterval")
                    .and_then(Value::as_u64)
                    .unwrap_or(120)
                    .saturating_mul(1_000),
                value
                    .pointer("/data/ClientConfig/PingInterval")
                    .and_then(Value::as_u64)
                    .unwrap_or(120)
                    .saturating_mul(1_000),
            ),
            IntegrationProviderId::WecomApp => return Err(()),
            _ => return Err(()),
        };
        validate_websocket_url(&url)?;
        Ok(Self {
            url,
            reconnect_delay: Duration::from_millis(reconnect_ms).min(MAX_RECONNECT_DELAY),
            ping_interval: Duration::from_millis(ping_ms.max(1_000)),
        })
    }

    #[cfg(test)]
    fn fixture(url: String) -> Self {
        Self {
            url,
            reconnect_delay: Duration::from_millis(20),
            ping_interval: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnterpriseStreamEvent {
    pub platform: IntegrationProviderId,
    pub connection_id: String,
    pub event_id: String,
    pub event_type: String,
    pub timestamp_ms: i64,
    pub verification_token: Option<String>,
    pub peer: String,
    pub thread: String,
    pub sender: String,
    pub text: String,
    pub mentions: Vec<ChannelMention>,
    pub attachments: Vec<RemoteMediaDescriptor>,
    pub payload: Value,
}

pub struct EnterpriseStreamRuntime {
    cancellation: CancellationToken,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for EnterpriseStreamRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EnterpriseStreamRuntime")
            .finish_non_exhaustive()
    }
}

impl EnterpriseStreamRuntime {
    pub async fn stop(mut self) {
        self.cancellation.cancel();
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
    }
}

impl Drop for EnterpriseStreamRuntime {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

pub fn spawn_enterprise_stream(
    api: EnterpriseApiClient,
    credential: EnterpriseCredential,
) -> (
    EnterpriseStreamRuntime,
    mpsc::Receiver<EnterpriseStreamEvent>,
) {
    let cancellation = CancellationToken::new();
    let task_cancellation = cancellation.clone();
    let (sender, receiver) = mpsc::channel(256);
    let handle = tokio::spawn(async move {
        let recent = Arc::new(Mutex::new(RecentEventIds::default()));
        while !task_cancellation.is_cancelled() {
            let endpoint = match api.stream_endpoint(&credential).await {
                Ok(endpoint) => endpoint,
                Err(_) => {
                    if wait_or_cancel(&task_cancellation, Duration::from_secs(3)).await {
                        break;
                    }
                    continue;
                }
            };
            let platform = credential.platform();
            let session_sender = sender.clone();
            let session_cancel = task_cancellation.clone();
            let session_recent = Arc::clone(&recent);
            let session_endpoint = endpoint.clone();
            let _ = tokio::task::spawn_blocking(move || {
                run_stream_session(
                    platform,
                    &session_endpoint,
                    &session_sender,
                    &session_cancel,
                    &session_recent,
                )
            })
            .await;
            if wait_or_cancel(&task_cancellation, endpoint.reconnect_delay).await {
                break;
            }
        }
    });
    (
        EnterpriseStreamRuntime {
            cancellation,
            handle: Some(handle),
        },
        receiver,
    )
}

async fn wait_or_cancel(cancellation: &CancellationToken, duration: Duration) -> bool {
    tokio::select! {
        () = cancellation.cancelled() => true,
        () = tokio::time::sleep(duration) => false,
    }
}

fn run_stream_session(
    platform: IntegrationProviderId,
    endpoint: &EnterpriseStreamEndpoint,
    sender: &mpsc::Sender<EnterpriseStreamEvent>,
    cancellation: &CancellationToken,
    recent: &Arc<Mutex<RecentEventIds>>,
) -> Result<(), StreamError> {
    let (mut socket, _) = connect(endpoint.url.as_str()).map_err(|_| StreamError::Transport)?;
    socket
        .get_mut()
        .set_nodelay(true)
        .map_err(|_| StreamError::Transport)?;
    set_read_timeout(socket.get_mut(), Some(Duration::from_millis(250)))?;
    let connection_id = connection_id(&endpoint.url);
    let service_id = service_id(&endpoint.url);
    let mut last_ping = Instant::now();
    let mut fragments = FeishuFragments::default();
    while !cancellation.is_cancelled() {
        if last_ping.elapsed() >= endpoint.ping_interval {
            match platform {
                IntegrationProviderId::DingTalk => socket
                    .send(Message::Ping(Vec::new().into()))
                    .map_err(|_| StreamError::Transport)?,
                IntegrationProviderId::Feishu => socket
                    .send(Message::Binary(encode_feishu_ping(service_id).into()))
                    .map_err(|_| StreamError::Transport)?,
                IntegrationProviderId::WecomApp => return Err(StreamError::InvalidFrame),
                _ => return Err(StreamError::InvalidFrame),
            }
            last_ping = Instant::now();
        }
        let message = match socket.read() {
            Ok(message) => message,
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => break,
            Err(_) => return Err(StreamError::Transport),
        };
        if message.len() > MAX_STREAM_FRAME_BYTES {
            return Err(StreamError::FrameTooLarge);
        }
        match message {
            Message::Ping(payload) => socket
                .send(Message::Pong(payload))
                .map_err(|_| StreamError::Transport)?,
            Message::Pong(_) => {}
            Message::Close(_) => break,
            Message::Text(text) if platform == IntegrationProviderId::DingTalk => {
                let (event, ack) = decode_dingtalk(text.as_bytes(), &connection_id)?;
                deliver_once(event, sender, recent)?;
                socket
                    .send(Message::Text(ack.into()))
                    .map_err(|_| StreamError::Transport)?;
            }
            Message::Binary(bytes) if platform == IntegrationProviderId::Feishu => {
                let frame = decode_feishu_frame(&bytes)?;
                if frame.method == 0 {
                    continue;
                }
                let Some((frame, payload)) = fragments.push(frame)? else {
                    continue;
                };
                let event = normalize_feishu(&frame, &payload, &connection_id)?;
                deliver_once(event, sender, recent)?;
                socket
                    .send(Message::Binary(encode_feishu_ack(frame).into()))
                    .map_err(|_| StreamError::Transport)?;
            }
            _ => return Err(StreamError::InvalidFrame),
        }
    }
    let _ = socket.close(None);
    Ok(())
}

fn deliver_once(
    event: EnterpriseStreamEvent,
    sender: &mpsc::Sender<EnterpriseStreamEvent>,
    recent: &Arc<Mutex<RecentEventIds>>,
) -> Result<(), StreamError> {
    let mut recent = recent.lock().map_err(|_| StreamError::Transport)?;
    if recent.insert(&event.event_id) {
        sender
            .blocking_send(event)
            .map_err(|_| StreamError::Transport)?;
    }
    Ok(())
}

#[derive(Default)]
struct RecentEventIds {
    set: BTreeSet<String>,
    order: VecDeque<String>,
}

impl RecentEventIds {
    fn insert(&mut self, value: &str) -> bool {
        if !self.set.insert(value.to_owned()) {
            return false;
        }
        self.order.push_back(value.to_owned());
        if self.order.len() > 4_096
            && let Some(oldest) = self.order.pop_front()
        {
            self.set.remove(&oldest);
        }
        true
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DingTalkFrame {
    #[serde(rename = "type")]
    frame_type: String,
    #[serde(default)]
    time: i64,
    headers: BTreeMap<String, String>,
    data: String,
}

#[derive(Debug, Serialize)]
struct DingTalkAck {
    code: u16,
    headers: BTreeMap<String, String>,
    message: &'static str,
    data: &'static str,
}

fn decode_dingtalk(
    bytes: &[u8],
    connection_id: &str,
) -> Result<(EnterpriseStreamEvent, String), StreamError> {
    if bytes.len() > MAX_STREAM_FRAME_BYTES {
        return Err(StreamError::FrameTooLarge);
    }
    let frame: DingTalkFrame =
        serde_json::from_slice(bytes).map_err(|_| StreamError::InvalidFrame)?;
    let payload: Value =
        serde_json::from_str(&frame.data).map_err(|_| StreamError::InvalidFrame)?;
    let event_id = string_at(&payload, &["/msgId", "/eventId"])
        .or_else(|| frame.headers.get("messageId").cloned())
        .ok_or(StreamError::InvalidFrame)?;
    let event_type = frame
        .headers
        .get("topic")
        .cloned()
        .unwrap_or(frame.frame_type);
    let peer = string_at(
        &payload,
        &["/conversationId", "/chatId", "/openConversationId"],
    )
    .ok_or(StreamError::InvalidFrame)?;
    let sender = string_at(&payload, &["/senderStaffId", "/senderId", "/senderCorpId"])
        .ok_or(StreamError::InvalidFrame)?;
    let text = string_at(&payload, &["/text/content", "/content"]).unwrap_or_default();
    let mentions = dingtalk_mentions(&payload);
    let attachments = normalized_attachments(IntegrationProviderId::DingTalk, &payload)?;
    let mut ack_headers = BTreeMap::new();
    ack_headers.insert("contentType".into(), "application/json".into());
    ack_headers.insert("messageId".into(), event_id.clone());
    let ack = serde_json::to_string(&DingTalkAck {
        code: 200,
        headers: ack_headers,
        message: "ok",
        data: "",
    })
    .map_err(|_| StreamError::InvalidFrame)?;
    Ok((
        EnterpriseStreamEvent {
            platform: IntegrationProviderId::DingTalk,
            connection_id: connection_id.into(),
            event_id,
            event_type,
            timestamp_ms: frame
                .headers
                .get("time")
                .and_then(|value| value.parse().ok())
                .unwrap_or(frame.time),
            verification_token: None,
            thread: peer.clone(),
            peer,
            sender,
            text,
            mentions,
            attachments,
            payload,
        },
        ack,
    ))
}

fn dingtalk_mentions(payload: &Value) -> Vec<ChannelMention> {
    let mut mentions = payload
        .get("atUsers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| {
            string_at(value, &["/staffId", "/dingtalkId", "/id"]).map(|target_id| ChannelMention {
                kind: ChannelMentionKind::User,
                target_id: Some(target_id),
                display_text: value.get("name").and_then(Value::as_str).map(str::to_owned),
            })
        })
        .collect::<Vec<_>>();
    if payload.get("isAtAll").and_then(Value::as_bool) == Some(true) {
        mentions.push(ChannelMention {
            kind: ChannelMentionKind::All,
            target_id: None,
            display_text: Some("@all".into()),
        });
    }
    mentions
}

#[derive(Debug, Clone, Default)]
struct FeishuFrame {
    seq_id: u64,
    log_id: u64,
    service: i32,
    method: i32,
    headers: Vec<(String, String)>,
    payload_encoding: String,
    payload_type: String,
    payload: Vec<u8>,
    log_id_new: String,
}

impl FeishuFrame {
    fn header(&self, key: &str) -> Option<&str> {
        self.headers
            .iter()
            .find_map(|(name, value)| (name == key).then_some(value.as_str()))
    }
}

#[derive(Default)]
struct FeishuFragments {
    pending: PendingFeishuFragments,
}

type FeishuFragmentParts = Vec<Option<Vec<u8>>>;
type PendingFeishuFragments = BTreeMap<String, (FeishuFrame, FeishuFragmentParts)>;

impl FeishuFragments {
    fn push(
        &mut self,
        mut frame: FeishuFrame,
    ) -> Result<Option<(FeishuFrame, Vec<u8>)>, StreamError> {
        let sum = frame
            .header("sum")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1);
        let seq = frame
            .header("seq")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        if sum == 0 || sum > 256 || seq >= sum {
            return Err(StreamError::InvalidFrame);
        }
        if sum == 1 {
            let payload = std::mem::take(&mut frame.payload);
            return Ok(Some((frame, payload)));
        }
        let message_id = frame
            .header("message_id")
            .ok_or(StreamError::InvalidFrame)?
            .to_owned();
        if self.pending.len() >= 128 && !self.pending.contains_key(&message_id) {
            return Err(StreamError::FrameTooLarge);
        }
        let entry = self
            .pending
            .entry(message_id.clone())
            .or_insert_with(|| (frame.clone(), vec![None; sum]));
        if entry.1.len() != sum {
            return Err(StreamError::InvalidFrame);
        }
        entry.1[seq] = Some(std::mem::take(&mut frame.payload));
        if entry.1.iter().any(Option::is_none) {
            return Ok(None);
        }
        let (frame, parts) = self
            .pending
            .remove(&message_id)
            .ok_or(StreamError::InvalidFrame)?;
        let mut payload = Vec::new();
        for part in parts.into_iter().flatten() {
            if payload.len().saturating_add(part.len()) > MAX_STREAM_FRAME_BYTES {
                return Err(StreamError::FrameTooLarge);
            }
            payload.extend(part);
        }
        Ok(Some((frame, payload)))
    }
}

fn normalize_feishu(
    frame: &FeishuFrame,
    bytes: &[u8],
    connection_id: &str,
) -> Result<EnterpriseStreamEvent, StreamError> {
    let payload: Value = serde_json::from_slice(bytes).map_err(|_| StreamError::InvalidFrame)?;
    let event_id = string_at(&payload, &["/header/event_id"])
        .or_else(|| frame.header("message_id").map(str::to_owned))
        .ok_or(StreamError::InvalidFrame)?;
    let event_type = string_at(&payload, &["/header/event_type"])
        .or_else(|| frame.header("type").map(str::to_owned))
        .ok_or(StreamError::InvalidFrame)?;
    let peer = string_at(&payload, &["/event/message/chat_id", "/event/chat_id"])
        .ok_or(StreamError::InvalidFrame)?;
    let thread = string_at(
        &payload,
        &["/event/message/thread_id", "/event/message/message_id"],
    )
    .unwrap_or_else(|| peer.clone());
    let sender = string_at(
        &payload,
        &[
            "/event/sender/sender_id/open_id",
            "/event/sender/sender_id/user_id",
        ],
    )
    .ok_or(StreamError::InvalidFrame)?;
    let content = string_at(&payload, &["/event/message/content"]).unwrap_or_default();
    let content_value = serde_json::from_str::<Value>(&content).unwrap_or(Value::String(content));
    let text = content_value
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let mentions = feishu_mentions(&payload);
    let attachments = feishu_attachments(&payload, &content_value)?;
    let timestamp_ms = string_at(&payload, &["/header/create_time"])
        .and_then(|value| value.parse().ok())
        .or_else(|| {
            frame
                .header("timestamp")
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or_default();
    Ok(EnterpriseStreamEvent {
        platform: IntegrationProviderId::Feishu,
        connection_id: connection_id.into(),
        event_id,
        event_type,
        timestamp_ms,
        verification_token: string_at(&payload, &["/header/token"]),
        peer,
        thread,
        sender,
        text,
        mentions,
        attachments,
        payload,
    })
}

fn feishu_mentions(payload: &Value) -> Vec<ChannelMention> {
    payload
        .pointer("/event/message/mentions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|value| {
            let key = value.get("key").and_then(Value::as_str).unwrap_or_default();
            let target = string_at(value, &["/id/open_id", "/id/user_id", "/id/union_id"]);
            ChannelMention {
                kind: if key.eq_ignore_ascii_case("@all") {
                    ChannelMentionKind::All
                } else {
                    ChannelMentionKind::User
                },
                target_id: target,
                display_text: value.get("name").and_then(Value::as_str).map(str::to_owned),
            }
        })
        .collect()
}

fn normalized_attachments(
    platform: IntegrationProviderId,
    payload: &Value,
) -> Result<Vec<RemoteMediaDescriptor>, StreamError> {
    payload
        .get("attachments")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| StreamError::InvalidFrame)
        .map(|value| value.unwrap_or_default())
        .and_then(|attachments: Vec<RemoteMediaDescriptor>| {
            attachments
                .iter()
                .all(|attachment| attachment.provider_id == platform)
                .then_some(attachments)
                .ok_or(StreamError::InvalidFrame)
        })
}

fn feishu_attachments(
    payload: &Value,
    content: &Value,
) -> Result<Vec<RemoteMediaDescriptor>, StreamError> {
    if let Some(attachments) = payload.get("attachments") {
        return serde_json::from_value(attachments.clone()).map_err(|_| StreamError::InvalidFrame);
    }
    let message_type = payload
        .pointer("/event/message/message_type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let remote_id = match message_type {
        "file" => content.get("file_key"),
        "image" => content.get("image_key"),
        _ => None,
    }
    .and_then(Value::as_str);
    Ok(remote_id
        .map(|remote_id| RemoteMediaDescriptor {
            provider_id: IntegrationProviderId::Feishu,
            remote_id: remote_id.to_owned(),
            resource_key: Some(message_type.to_owned()),
            file_name: content
                .get("file_name")
                .and_then(Value::as_str)
                .map(str::to_owned),
            mime_type: None,
            declared_size_bytes: None,
            content_hash: None,
            download_required: true,
        })
        .into_iter()
        .collect())
}

fn decode_feishu_frame(bytes: &[u8]) -> Result<FeishuFrame, StreamError> {
    if bytes.len() > MAX_STREAM_FRAME_BYTES {
        return Err(StreamError::FrameTooLarge);
    }
    let mut input = bytes;
    let mut frame = FeishuFrame::default();
    while !input.is_empty() {
        let key = read_varint(&mut input)?;
        let field = key >> 3;
        let wire = key & 7;
        match (field, wire) {
            (1, 0) => frame.seq_id = read_varint(&mut input)?,
            (2, 0) => frame.log_id = read_varint(&mut input)?,
            (3, 0) => frame.service = read_varint(&mut input)? as i32,
            (4, 0) => frame.method = read_varint(&mut input)? as i32,
            (5, 2) => frame.headers.push(decode_header(read_bytes(&mut input)?)?),
            (6, 2) => frame.payload_encoding = read_string(&mut input)?,
            (7, 2) => frame.payload_type = read_string(&mut input)?,
            (8, 2) => frame.payload = read_bytes(&mut input)?.to_vec(),
            (9, 2) => frame.log_id_new = read_string(&mut input)?,
            (_, 0) => {
                let _ = read_varint(&mut input)?;
            }
            (_, 2) => {
                let _ = read_bytes(&mut input)?;
            }
            _ => return Err(StreamError::InvalidFrame),
        }
    }
    Ok(frame)
}

fn encode_feishu_ack(mut frame: FeishuFrame) -> Vec<u8> {
    frame.payload = br#"{"code":200,"headers":{},"data":null}"#.to_vec();
    frame.headers.push(("biz_rt".into(), "0".into()));
    encode_feishu_frame(&frame)
}

fn encode_feishu_ping(service: i32) -> Vec<u8> {
    encode_feishu_frame(&FeishuFrame {
        service,
        method: 0,
        headers: vec![("type".into(), "ping".into())],
        ..FeishuFrame::default()
    })
}

fn encode_feishu_frame(frame: &FeishuFrame) -> Vec<u8> {
    let mut output = Vec::new();
    put_varint_field(&mut output, 1, frame.seq_id);
    put_varint_field(&mut output, 2, frame.log_id);
    put_varint_field(&mut output, 3, frame.service as u64);
    put_varint_field(&mut output, 4, frame.method as u64);
    for (key, value) in &frame.headers {
        let mut header = Vec::new();
        put_bytes_field(&mut header, 1, key.as_bytes());
        put_bytes_field(&mut header, 2, value.as_bytes());
        put_bytes_field(&mut output, 5, &header);
    }
    if !frame.payload_encoding.is_empty() {
        put_bytes_field(&mut output, 6, frame.payload_encoding.as_bytes());
    }
    if !frame.payload_type.is_empty() {
        put_bytes_field(&mut output, 7, frame.payload_type.as_bytes());
    }
    if !frame.payload.is_empty() {
        put_bytes_field(&mut output, 8, &frame.payload);
    }
    if !frame.log_id_new.is_empty() {
        put_bytes_field(&mut output, 9, frame.log_id_new.as_bytes());
    }
    output
}

fn decode_header(mut bytes: &[u8]) -> Result<(String, String), StreamError> {
    let mut key = None;
    let mut value = None;
    while !bytes.is_empty() {
        match read_varint(&mut bytes)? {
            10 => key = Some(read_string(&mut bytes)?),
            18 => value = Some(read_string(&mut bytes)?),
            _ => return Err(StreamError::InvalidFrame),
        }
    }
    Ok((
        key.ok_or(StreamError::InvalidFrame)?,
        value.ok_or(StreamError::InvalidFrame)?,
    ))
}

fn read_varint(input: &mut &[u8]) -> Result<u64, StreamError> {
    let mut value = 0_u64;
    for shift in (0..=63).step_by(7) {
        let (&byte, rest) = input.split_first().ok_or(StreamError::InvalidFrame)?;
        *input = rest;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(StreamError::InvalidFrame)
}

fn read_bytes<'a>(input: &mut &'a [u8]) -> Result<&'a [u8], StreamError> {
    let length = usize::try_from(read_varint(input)?).map_err(|_| StreamError::FrameTooLarge)?;
    if length > MAX_STREAM_FRAME_BYTES || input.len() < length {
        return Err(StreamError::FrameTooLarge);
    }
    let (bytes, rest) = input.split_at(length);
    *input = rest;
    Ok(bytes)
}

fn read_string(input: &mut &[u8]) -> Result<String, StreamError> {
    String::from_utf8(read_bytes(input)?.to_vec()).map_err(|_| StreamError::InvalidFrame)
}

fn put_varint(output: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        output.push((value as u8) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

fn put_varint_field(output: &mut Vec<u8>, field: u64, value: u64) {
    put_varint(output, field << 3);
    put_varint(output, value);
}

fn put_bytes_field(output: &mut Vec<u8>, field: u64, value: &[u8]) {
    put_varint(output, (field << 3) | 2);
    put_varint(output, value.len() as u64);
    output.extend(value);
}

fn string_at(value: &Value, pointers: &[&str]) -> Option<String> {
    pointers
        .iter()
        .find_map(|pointer| value.pointer(pointer).and_then(Value::as_str))
        .filter(|value| !value.is_empty() && value.len() <= 32_000)
        .map(str::to_owned)
}

fn connection_id(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|url| {
            url.query_pairs()
                .find(|(key, _)| key == "device_id" || key == "connectionId")
                .map(|(_, value)| value.into_owned())
        })
        .unwrap_or_else(|| "stream".into())
}

fn service_id(url: &str) -> i32 {
    url::Url::parse(url)
        .ok()
        .and_then(|url| {
            url.query_pairs()
                .find(|(key, _)| key == "service_id")
                .and_then(|(_, value)| value.parse().ok())
        })
        .unwrap_or_default()
}

fn validate_websocket_url(url: &str) -> Result<(), ()> {
    let parsed = url::Url::parse(url).map_err(|_| ())?;
    if parsed.scheme() == "wss"
        || (parsed.scheme() == "ws"
            && matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "::1")))
    {
        Ok(())
    } else {
        Err(())
    }
}

fn set_read_timeout(
    stream: &mut MaybeTlsStream<TcpStream>,
    timeout: Option<Duration>,
) -> Result<(), StreamError> {
    match stream {
        MaybeTlsStream::Plain(stream) => stream.set_read_timeout(timeout),
        MaybeTlsStream::Rustls(stream) => stream.sock.set_read_timeout(timeout),
        _ => return Err(StreamError::Transport),
    }
    .map_err(|_| StreamError::Transport)
}

#[derive(Debug)]
enum StreamError {
    Transport,
    InvalidFrame,
    FrameTooLarge,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::TcpListener, thread};

    #[test]
    fn dingtalk_fixture_websocket_acks_and_deduplicates() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut socket = tungstenite::accept(stream).expect("websocket");
            let frame = json!({
                "specVersion":"1.0","type":"CALLBACK","time":1000,
                "headers":{"topic":"chat.message","messageId":"event-1","time":"1000"},
                "data":serde_json::to_string(&json!({
                    "msgId":"event-1","conversationId":"peer","senderStaffId":"sender",
                    "text":{"content":"hello"},"atUsers":[{"staffId":"user-1","name":"Ada"}]
                })).expect("payload")
            });
            for _ in 0..2 {
                socket
                    .send(Message::Text(frame.to_string().into()))
                    .expect("send");
                let ack = socket.read().expect("ack");
                assert!(ack.to_text().expect("text ack").contains("\"code\":200"));
            }
            socket.close(None).expect("close");
        });
        let endpoint =
            EnterpriseStreamEndpoint::fixture(format!("ws://{address}/?connectionId=fixture"));
        let (sender, mut receiver) = mpsc::channel(8);
        let cancellation = CancellationToken::new();
        run_stream_session(
            IntegrationProviderId::DingTalk,
            &endpoint,
            &sender,
            &cancellation,
            &Arc::new(Mutex::new(RecentEventIds::default())),
        )
        .expect("session");
        server.join().expect("server");
        let event = receiver.try_recv().expect("event");
        assert_eq!(event.event_id, "event-1");
        assert_eq!(event.mentions.len(), 1);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn dingtalk_fixture_reconnects_and_preserves_cross_connection_dedup() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let frame = |event_id: &str| {
            json!({
                "specVersion":"1.0","type":"CALLBACK","time":1000,
                "headers":{"topic":"chat.message","messageId":event_id,"time":"1000"},
                "data":serde_json::to_string(&json!({
                    "msgId":event_id,"conversationId":"peer","senderStaffId":"sender",
                    "text":{"content":event_id}
                })).expect("payload")
            })
        };
        let first = frame("event-reconnect-1");
        let duplicate = first.clone();
        let second = frame("event-reconnect-2");
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("first accept");
            let mut socket = tungstenite::accept(stream).expect("first websocket");
            socket
                .send(Message::Text(first.to_string().into()))
                .expect("first send");
            assert!(
                socket
                    .read()
                    .expect("first ack")
                    .to_text()
                    .expect("first ack text")
                    .contains("\"code\":200")
            );
            socket.close(None).expect("first close");

            let (stream, _) = listener.accept().expect("second accept");
            let mut socket = tungstenite::accept(stream).expect("second websocket");
            for frame in [duplicate, second] {
                socket
                    .send(Message::Text(frame.to_string().into()))
                    .expect("reconnect send");
                assert!(
                    socket
                        .read()
                        .expect("reconnect ack")
                        .to_text()
                        .expect("reconnect ack text")
                        .contains("\"code\":200")
                );
            }
            socket.close(None).expect("second close");
        });
        let endpoint = EnterpriseStreamEndpoint::fixture(format!(
            "ws://{address}/?connectionId=reconnect-fixture"
        ));
        let (sender, mut receiver) = mpsc::channel(8);
        let recent = Arc::new(Mutex::new(RecentEventIds::default()));
        for _ in 0..2 {
            run_stream_session(
                IntegrationProviderId::DingTalk,
                &endpoint,
                &sender,
                &CancellationToken::new(),
                &recent,
            )
            .expect("session");
        }
        server.join().expect("server");
        assert_eq!(
            receiver.try_recv().expect("first event").event_id,
            "event-reconnect-1"
        );
        assert_eq!(
            receiver.try_recv().expect("second event").event_id,
            "event-reconnect-2"
        );
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn feishu_fixture_websocket_uses_protobuf_ack_and_reassembles_frames() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let payload = serde_json::to_vec(&json!({
            "header":{"event_id":"event-2","event_type":"im.message.receive_v1","create_time":"2000"},
            "event":{"sender":{"sender_id":{"open_id":"sender"}},"message":{"message_id":"message-2","chat_id":"peer","message_type":"text","content":"{\"text\":\"hello\"}","mentions":[{"key":"@_user_1","id":{"open_id":"user-1"},"name":"Ada"}]}}
        })).expect("payload");
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut socket = tungstenite::accept(stream).expect("websocket");
            for (seq, part) in payload.chunks(payload.len().div_ceil(2)).enumerate() {
                let frame = FeishuFrame {
                    seq_id: 1,
                    log_id: 2,
                    service: 3,
                    method: 1,
                    headers: vec![
                        ("type".into(), "event".into()),
                        ("message_id".into(), "event-2".into()),
                        ("timestamp".into(), "2000".into()),
                        ("sum".into(), "2".into()),
                        ("seq".into(), seq.to_string()),
                    ],
                    payload: part.to_vec(),
                    ..FeishuFrame::default()
                };
                socket
                    .send(Message::Binary(encode_feishu_frame(&frame).into()))
                    .expect("send");
            }
            let ack = socket.read().expect("ack");
            let frame = decode_feishu_frame(&ack.into_data()).expect("ack frame");
            assert!(
                std::str::from_utf8(&frame.payload)
                    .expect("ack json")
                    .contains("\"code\":200")
            );
            socket.close(None).expect("close");
        });
        let endpoint = EnterpriseStreamEndpoint::fixture(format!(
            "ws://{address}/?device_id=fixture&service_id=3"
        ));
        let (sender, mut receiver) = mpsc::channel(8);
        run_stream_session(
            IntegrationProviderId::Feishu,
            &endpoint,
            &sender,
            &CancellationToken::new(),
            &Arc::new(Mutex::new(RecentEventIds::default())),
        )
        .expect("session");
        server.join().expect("server");
        let event = receiver.try_recv().expect("event");
        assert_eq!(event.event_id, "event-2");
        assert_eq!(event.text, "hello");
        assert_eq!(event.mentions[0].target_id.as_deref(), Some("user-1"));
    }
}
