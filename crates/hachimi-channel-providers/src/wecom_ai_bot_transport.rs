use std::{
    collections::BTreeMap,
    io,
    net::TcpStream,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender, TryRecvError, sync_channel},
    },
    time::{Duration, Instant},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::sync::{mpsc, oneshot};
use tungstenite::{
    Message, WebSocket, connect,
    stream::{MaybeTlsStream, NoDelay},
};
use zeroize::Zeroizing;

use crate::{
    HEARTBEAT_INTERVAL_SECS, OPENWS_ENDPOINT, ProviderError, SUBSCRIBE_ACK_TIMEOUT_SECS,
    WecomAiBotAdapter,
};

const MAX_FRAME_BYTES: usize = 1024 * 1024;
const COMMAND_CAPACITY: usize = 256;
const DELIVERY_ACK_TIMEOUT_SECS: u64 = 7;
const MEDIA_ACK_TIMEOUT_SECS: u64 = 120;
const MEDIA_CHUNK_BYTES: usize = 512 * 1024;
const MAX_MEDIA_BYTES: usize = 25 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum WecomAiBotTransportEvent {
    Message {
        payload: Value,
        connection_id: String,
        received_at_ms: i64,
    },
    Degraded,
    AuthenticationExpired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WecomAiBotDeliveryResult {
    Delivered { provider_receipt: String },
    Retryable,
    Permanent,
    Indeterminate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WecomAiBotMediaKind {
    Image,
    File,
    Video,
    Voice,
}

enum DeliveryPayload {
    Text(String),
    Media {
        kind: WecomAiBotMediaKind,
        file_name: String,
        bytes: Vec<u8>,
    },
}

struct DeliveryCommand {
    chat_id: String,
    group: bool,
    payload: DeliveryPayload,
    idempotency_key: String,
    result: oneshot::Sender<WecomAiBotDeliveryResult>,
}

enum PendingDelivery {
    Final(oneshot::Sender<WecomAiBotDeliveryResult>),
    MediaInit(DeliveryCommand),
    MediaChunk {
        command: DeliveryCommand,
        upload_id: String,
        chunk_index: usize,
    },
    MediaFinish(DeliveryCommand),
}

pub struct WecomAiBotTransport {
    commands: SyncSender<DeliveryCommand>,
    cancelled: Arc<AtomicBool>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for WecomAiBotTransport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WecomAiBotTransport")
            .finish_non_exhaustive()
    }
}

impl WecomAiBotTransport {
    pub fn spawn(
        bot_id: String,
        secret: String,
    ) -> Result<(Self, mpsc::Receiver<WecomAiBotTransportEvent>), ProviderError> {
        if bot_id.trim().is_empty() || secret.trim().is_empty() {
            return Err(ProviderError::InvalidEvent);
        }
        let (command_sender, command_receiver) = sync_channel(COMMAND_CAPACITY);
        let (event_sender, event_receiver) = mpsc::channel(COMMAND_CAPACITY);
        let cancelled = Arc::new(AtomicBool::new(false));
        let task_cancelled = Arc::clone(&cancelled);
        let handle = tokio::task::spawn_blocking(move || {
            let secret = Zeroizing::new(secret);
            run_connection_loop(
                &bot_id,
                secret.as_str(),
                &command_receiver,
                &event_sender,
                &task_cancelled,
            );
        });
        Ok((
            Self {
                commands: command_sender,
                cancelled,
                handle: Some(handle),
            },
            event_receiver,
        ))
    }

    pub async fn send_text(
        &self,
        chat_id: String,
        group: bool,
        text: String,
        idempotency_key: String,
    ) -> WecomAiBotDeliveryResult {
        if chat_id.trim().is_empty() || text.trim().is_empty() || idempotency_key.trim().is_empty()
        {
            return WecomAiBotDeliveryResult::Permanent;
        }
        let (sender, receiver) = oneshot::channel();
        if self
            .commands
            .try_send(DeliveryCommand {
                chat_id,
                group,
                payload: DeliveryPayload::Text(text),
                idempotency_key,
                result: sender,
            })
            .is_err()
        {
            return WecomAiBotDeliveryResult::Retryable;
        }
        match tokio::time::timeout(Duration::from_secs(DELIVERY_ACK_TIMEOUT_SECS), receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => WecomAiBotDeliveryResult::Retryable,
            Err(_) => WecomAiBotDeliveryResult::Indeterminate,
        }
    }

    pub async fn send_media(
        &self,
        chat_id: String,
        group: bool,
        kind: WecomAiBotMediaKind,
        file_name: String,
        bytes: Vec<u8>,
        idempotency_key: String,
    ) -> WecomAiBotDeliveryResult {
        let chunks = bytes.len().div_ceil(MEDIA_CHUNK_BYTES);
        if chat_id.trim().is_empty()
            || file_name.trim().is_empty()
            || file_name.chars().count() > 255
            || file_name.contains(['/', '\\', '\0', '\r', '\n'])
            || bytes.len() < 5
            || bytes.len() > MAX_MEDIA_BYTES
            || chunks > 100
            || idempotency_key.trim().is_empty()
        {
            return WecomAiBotDeliveryResult::Permanent;
        }
        let (sender, receiver) = oneshot::channel();
        if self
            .commands
            .try_send(DeliveryCommand {
                chat_id,
                group,
                payload: DeliveryPayload::Media {
                    kind,
                    file_name,
                    bytes,
                },
                idempotency_key,
                result: sender,
            })
            .is_err()
        {
            return WecomAiBotDeliveryResult::Retryable;
        }
        match tokio::time::timeout(Duration::from_secs(MEDIA_ACK_TIMEOUT_SECS), receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => WecomAiBotDeliveryResult::Retryable,
            Err(_) => WecomAiBotDeliveryResult::Indeterminate,
        }
    }

    pub async fn stop(mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
    }
}

impl Drop for WecomAiBotTransport {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

fn run_connection_loop(
    bot_id: &str,
    secret: &str,
    commands: &Receiver<DeliveryCommand>,
    events: &mpsc::Sender<WecomAiBotTransportEvent>,
    cancelled: &AtomicBool,
) {
    let mut reconnect_attempt = 0_u32;
    while !cancelled.load(Ordering::Acquire) {
        match run_connection(bot_id, secret, commands, events, cancelled) {
            SessionResult::Cancelled => break,
            SessionResult::AuthenticationExpired => {
                let _ = events.blocking_send(WecomAiBotTransportEvent::AuthenticationExpired);
                break;
            }
            SessionResult::Disconnected => {
                let _ = events.blocking_send(WecomAiBotTransportEvent::Degraded);
                reconnect_attempt = reconnect_attempt.saturating_add(1);
                let seconds = 2_u64.saturating_pow(reconnect_attempt.min(4)).min(30);
                interruptible_wait(cancelled, Duration::from_secs(seconds));
            }
        }
    }
}

fn run_connection(
    bot_id: &str,
    secret: &str,
    commands: &Receiver<DeliveryCommand>,
    events: &mpsc::Sender<WecomAiBotTransportEvent>,
    cancelled: &AtomicBool,
) -> SessionResult {
    let Ok((mut socket, _)) = connect(OPENWS_ENDPOINT) else {
        return SessionResult::Disconnected;
    };
    if socket.get_mut().set_nodelay(true).is_err()
        || set_read_timeout(
            socket.get_mut(),
            Some(Duration::from_secs(SUBSCRIBE_ACK_TIMEOUT_SECS)),
        )
        .is_err()
    {
        return SessionResult::Disconnected;
    }
    let subscribe = WecomAiBotAdapter::subscribe_frame(bot_id, secret);
    let subscribe_req_id = subscribe
        .pointer("/headers/req_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if socket
        .send(Message::Text(subscribe.to_string().into()))
        .is_err()
    {
        return SessionResult::Disconnected;
    }
    let Ok(message) = socket.read() else {
        return SessionResult::Disconnected;
    };
    let Some(ack) = json_frame(message) else {
        return SessionResult::AuthenticationExpired;
    };
    if ack.pointer("/headers/req_id").and_then(Value::as_str) != Some(&subscribe_req_id)
        || ack.get("errcode").and_then(Value::as_i64) != Some(0)
    {
        return SessionResult::AuthenticationExpired;
    }
    if set_read_timeout(socket.get_mut(), Some(Duration::from_millis(250))).is_err() {
        return SessionResult::Disconnected;
    }
    let connection_id = stable_id(&subscribe_req_id);
    let mut last_heartbeat = Instant::now();
    let mut pending = BTreeMap::<String, PendingDelivery>::new();
    while !cancelled.load(Ordering::Acquire) {
        drain_commands(&mut socket, commands, &mut pending);
        if last_heartbeat.elapsed() >= Duration::from_secs(HEARTBEAT_INTERVAL_SECS) {
            let heartbeat = json!({
                "cmd": "ping",
                "headers": {"req_id": format!("ping_{}", stable_id(&now_ms().to_string()))},
            });
            if socket
                .send(Message::Text(heartbeat.to_string().into()))
                .is_err()
            {
                finish_pending(&mut pending, WecomAiBotDeliveryResult::Indeterminate);
                return SessionResult::Disconnected;
            }
            last_heartbeat = Instant::now();
        }
        match socket.read() {
            Ok(message) if message.len() > MAX_FRAME_BYTES => {
                finish_pending(&mut pending, WecomAiBotDeliveryResult::Indeterminate);
                return SessionResult::Disconnected;
            }
            Ok(Message::Ping(payload)) => {
                let _ = socket.send(Message::Pong(payload));
            }
            Ok(Message::Close(_)) => {
                finish_pending(&mut pending, WecomAiBotDeliveryResult::Indeterminate);
                return SessionResult::Disconnected;
            }
            Ok(message) => {
                if let Some(frame) = json_frame(message) {
                    handle_frame(
                        Some(&mut socket),
                        frame,
                        events,
                        &connection_id,
                        &mut pending,
                    );
                }
            }
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                finish_pending(&mut pending, WecomAiBotDeliveryResult::Indeterminate);
                return SessionResult::Disconnected;
            }
            Err(_) => {
                finish_pending(&mut pending, WecomAiBotDeliveryResult::Indeterminate);
                return SessionResult::Disconnected;
            }
        }
    }
    finish_pending(&mut pending, WecomAiBotDeliveryResult::Retryable);
    let _ = socket.close(None);
    SessionResult::Cancelled
}

fn drain_commands(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    commands: &Receiver<DeliveryCommand>,
    pending: &mut BTreeMap<String, PendingDelivery>,
) {
    loop {
        let command = match commands.try_recv() {
            Ok(command) => command,
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
        };
        let base_id = stable_id(&command.idempotency_key);
        let (req_id, frame, state) = match &command.payload {
            DeliveryPayload::Text(text) => {
                let req_id = format!("aibot_send_msg_{base_id}");
                (
                    req_id.clone(),
                    json!({
                        "cmd": "aibot_send_msg",
                        "headers": {"req_id": req_id},
                        "body": {
                            "chatid": command.chat_id,
                            "chat_type": if command.group { 2 } else { 1 },
                            "msgtype": "markdown",
                            "markdown": {"content": text},
                        },
                    }),
                    PendingDelivery::Final(command.result),
                )
            }
            DeliveryPayload::Media {
                kind,
                file_name,
                bytes,
            } => {
                let req_id = format!("aibot_upload_init_{base_id}");
                (
                    req_id.clone(),
                    json!({
                        "cmd": "aibot_upload_media_init",
                        "headers": {"req_id": req_id},
                        "body": {
                            "type": media_kind_name(*kind),
                            "filename": file_name,
                            "total_size": bytes.len(),
                            "total_chunks": bytes.len().div_ceil(MEDIA_CHUNK_BYTES),
                            "md5": format!("{:x}", md5::compute(bytes)),
                        },
                    }),
                    PendingDelivery::MediaInit(command),
                )
            }
        };
        if socket
            .send(Message::Text(frame.to_string().into()))
            .is_err()
        {
            complete_pending(state, WecomAiBotDeliveryResult::Retryable);
            continue;
        }
        pending.insert(req_id, state);
    }
}

fn handle_frame(
    mut socket: Option<&mut WebSocket<MaybeTlsStream<TcpStream>>>,
    frame: Value,
    events: &mpsc::Sender<WecomAiBotTransportEvent>,
    connection_id: &str,
    pending: &mut BTreeMap<String, PendingDelivery>,
) {
    if frame.get("cmd").and_then(Value::as_str) == Some("aibot_msg_callback") {
        if let Some(mut payload) = frame.get("body").cloned() {
            if let Some(object) = payload.as_object_mut()
                && let Some(req_id) = frame.pointer("/headers/req_id").and_then(Value::as_str)
            {
                object.insert("_hachimi_req_id".into(), Value::String(req_id.into()));
            }
            let _ = events.blocking_send(WecomAiBotTransportEvent::Message {
                payload,
                connection_id: connection_id.into(),
                received_at_ms: now_ms(),
            });
        }
        return;
    }
    let Some(req_id) = frame.pointer("/headers/req_id").and_then(Value::as_str) else {
        return;
    };
    let Some(state) = pending.remove(req_id) else {
        return;
    };
    if frame
        .get("errcode")
        .and_then(Value::as_i64)
        .unwrap_or_default()
        != 0
    {
        complete_pending(state, WecomAiBotDeliveryResult::Permanent);
        return;
    }
    match state {
        PendingDelivery::Final(result) => {
            let _ = result.send(WecomAiBotDeliveryResult::Delivered {
                provider_receipt: req_id.into(),
            });
        }
        PendingDelivery::MediaInit(command) => {
            let Some(upload_id) = frame
                .pointer("/body/upload_id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty() && value.len() <= 512)
                .map(str::to_owned)
            else {
                complete_command(command, WecomAiBotDeliveryResult::Permanent);
                return;
            };
            send_media_chunk(socket.as_deref_mut(), pending, command, upload_id, 0);
        }
        PendingDelivery::MediaChunk {
            command,
            upload_id,
            chunk_index,
        } => {
            let total = media_bytes(&command).len().div_ceil(MEDIA_CHUNK_BYTES);
            if chunk_index.saturating_add(1) < total {
                send_media_chunk(
                    socket.as_deref_mut(),
                    pending,
                    command,
                    upload_id,
                    chunk_index.saturating_add(1),
                );
            } else {
                send_media_finish(socket.as_deref_mut(), pending, command, upload_id);
            }
        }
        PendingDelivery::MediaFinish(command) => {
            let Some(media_id) = frame
                .pointer("/body/media_id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty() && value.len() <= 2048)
            else {
                complete_command(command, WecomAiBotDeliveryResult::Permanent);
                return;
            };
            send_media_message(socket, pending, command, media_id);
        }
    }
}

fn send_media_chunk(
    socket: Option<&mut WebSocket<MaybeTlsStream<TcpStream>>>,
    pending: &mut BTreeMap<String, PendingDelivery>,
    command: DeliveryCommand,
    upload_id: String,
    chunk_index: usize,
) {
    let start = chunk_index.saturating_mul(MEDIA_CHUNK_BYTES);
    let end = start
        .saturating_add(MEDIA_CHUNK_BYTES)
        .min(media_bytes(&command).len());
    let req_id = format!(
        "aibot_upload_chunk_{}_{}",
        stable_id(&command.idempotency_key),
        chunk_index
    );
    let frame = json!({
        "cmd": "aibot_upload_media_chunk",
        "headers": {"req_id": req_id},
        "body": {
            "upload_id": upload_id,
            "chunk_index": chunk_index,
            "base64_data": BASE64.encode(&media_bytes(&command)[start..end]),
        },
    });
    let state = PendingDelivery::MediaChunk {
        command,
        upload_id,
        chunk_index,
    };
    send_media_frame(socket, pending, req_id, frame, state);
}

fn send_media_finish(
    socket: Option<&mut WebSocket<MaybeTlsStream<TcpStream>>>,
    pending: &mut BTreeMap<String, PendingDelivery>,
    command: DeliveryCommand,
    upload_id: String,
) {
    let req_id = format!(
        "aibot_upload_finish_{}",
        stable_id(&command.idempotency_key)
    );
    let frame = json!({
        "cmd": "aibot_upload_media_finish",
        "headers": {"req_id": req_id},
        "body": {"upload_id": upload_id},
    });
    send_media_frame(
        socket,
        pending,
        req_id,
        frame,
        PendingDelivery::MediaFinish(command),
    );
}

fn send_media_message(
    socket: Option<&mut WebSocket<MaybeTlsStream<TcpStream>>>,
    pending: &mut BTreeMap<String, PendingDelivery>,
    command: DeliveryCommand,
    media_id: &str,
) {
    let req_id = format!("aibot_send_media_{}", stable_id(&command.idempotency_key));
    let kind = media_kind_name(media_kind(&command));
    let frame = json!({
        "cmd": "aibot_send_msg",
        "headers": {"req_id": req_id},
        "body": {
            "chatid": command.chat_id,
            "chat_type": if command.group { 2 } else { 1 },
            "msgtype": kind,
            kind: {"media_id": media_id},
        },
    });
    let result = command.result;
    send_media_frame(
        socket,
        pending,
        req_id,
        frame,
        PendingDelivery::Final(result),
    );
}

fn send_media_frame(
    socket: Option<&mut WebSocket<MaybeTlsStream<TcpStream>>>,
    pending: &mut BTreeMap<String, PendingDelivery>,
    req_id: String,
    frame: Value,
    state: PendingDelivery,
) {
    let sent =
        socket.is_some_and(|socket| socket.send(Message::Text(frame.to_string().into())).is_ok());
    if sent {
        pending.insert(req_id, state);
    } else {
        complete_pending(state, WecomAiBotDeliveryResult::Indeterminate);
    }
}

fn media_kind(command: &DeliveryCommand) -> WecomAiBotMediaKind {
    match &command.payload {
        DeliveryPayload::Media { kind, .. } => *kind,
        DeliveryPayload::Text(_) => unreachable!("media state contains media payload"),
    }
}

fn media_bytes(command: &DeliveryCommand) -> &[u8] {
    match &command.payload {
        DeliveryPayload::Media { bytes, .. } => bytes,
        DeliveryPayload::Text(_) => unreachable!("media state contains media payload"),
    }
}

const fn media_kind_name(kind: WecomAiBotMediaKind) -> &'static str {
    match kind {
        WecomAiBotMediaKind::Image => "image",
        WecomAiBotMediaKind::File => "file",
        WecomAiBotMediaKind::Video => "video",
        WecomAiBotMediaKind::Voice => "voice",
    }
}

fn json_frame(message: Message) -> Option<Value> {
    match message {
        Message::Text(text) => serde_json::from_str(&text).ok(),
        Message::Binary(bytes) => serde_json::from_slice(&bytes).ok(),
        _ => None,
    }
}

fn finish_pending(
    pending: &mut BTreeMap<String, PendingDelivery>,
    outcome: WecomAiBotDeliveryResult,
) {
    for (_, state) in std::mem::take(pending) {
        complete_pending(state, outcome.clone());
    }
}

fn complete_pending(state: PendingDelivery, outcome: WecomAiBotDeliveryResult) {
    match state {
        PendingDelivery::Final(result) => {
            let _ = result.send(outcome);
        }
        PendingDelivery::MediaInit(command)
        | PendingDelivery::MediaChunk { command, .. }
        | PendingDelivery::MediaFinish(command) => complete_command(command, outcome),
    }
}

fn complete_command(command: DeliveryCommand, outcome: WecomAiBotDeliveryResult) {
    let _ = command.result.send(outcome);
}

fn set_read_timeout(
    stream: &mut MaybeTlsStream<TcpStream>,
    timeout: Option<Duration>,
) -> io::Result<()> {
    match stream {
        MaybeTlsStream::Plain(stream) => stream.set_read_timeout(timeout),
        MaybeTlsStream::Rustls(stream) => stream.sock.set_read_timeout(timeout),
        _ => Err(io::Error::other("unsupported TLS stream")),
    }
}

fn stable_id(value: &str) -> String {
    Sha256::digest(value.as_bytes())[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(i64::MAX)
}

fn interruptible_wait(cancelled: &AtomicBool, duration: Duration) {
    let deadline = Instant::now() + duration;
    while !cancelled.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
}

enum SessionResult {
    Cancelled,
    Disconnected,
    AuthenticationExpired,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_and_delivery_ack_frames_are_classified() {
        let (events, mut receiver) = mpsc::channel(2);
        let mut pending = BTreeMap::new();
        handle_frame(
            None,
            json!({
                "cmd": "aibot_msg_callback",
                "headers": {"req_id": "callback-1"},
                "body": {"msgid": "message-1"}
            }),
            &events,
            "connection-1",
            &mut pending,
        );
        let event = receiver.try_recv().expect("callback event");
        assert!(matches!(event, WecomAiBotTransportEvent::Message { .. }));

        let (result_sender, result_receiver) = oneshot::channel();
        pending.insert("send-1".into(), PendingDelivery::Final(result_sender));
        handle_frame(
            None,
            json!({"headers":{"req_id":"send-1"},"errcode":0}),
            &events,
            "connection-1",
            &mut pending,
        );
        assert_eq!(
            result_receiver.blocking_recv().expect("delivery result"),
            WecomAiBotDeliveryResult::Delivered {
                provider_receipt: "send-1".into()
            }
        );
    }

    #[test]
    fn media_commands_are_bounded_and_named_by_type() {
        assert_eq!(media_kind_name(WecomAiBotMediaKind::Image), "image");
        assert_eq!(media_kind_name(WecomAiBotMediaKind::File), "file");
        assert_eq!((MAX_MEDIA_BYTES).div_ceil(MEDIA_CHUNK_BYTES), 50);
    }
}
