use std::{path::Path, sync::Arc, time::Duration};

#[cfg(not(test))]
use std::{
    sync::{Mutex, OnceLock},
    time::Instant,
};

#[cfg(test)]
use hachimi_protocol::ChannelRouteKey;
use hachimi_protocol::{ChannelEnvelope, IngressReceipt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const LOOPBACK_ADDRESS: &str = "127.0.0.1:42371";
const LOOPBACK_PATH: &str = "/v1/channels/loopback-webhook";
const LOOPBACK_OUTBOX_PATH: &str = "/v1/channels/loopback-webhook/outbox/claim";
const WECOM_CALLBACK_PATH: &str = "/v1/channels/wecom/callback";
const DESKTOP_WAKE_ADDRESS: &str = "127.0.0.1:42373";
const MAX_HTTP_REQUEST_BYTES: usize = 64 * 1024;

pub(super) fn run(data_root: &Path) {
    let database = data_root.join("agent.sqlite3");
    let token = keyring::Entry::new("com.hachimi.channel", "loopback-webhook:local")
        .and_then(|entry| entry.get_password())
        .unwrap_or_else(|error| panic!("failed to read per-user Gateway credential: {error}"));
    if token.len() < 32 || token.len() > 128 {
        panic!("per-user Gateway token is invalid");
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|error| panic!("failed to start local Gateway runtime: {error}"));
    runtime.block_on(async move {
        let listener = match tokio::net::TcpListener::bind(LOOPBACK_ADDRESS).await {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => return,
            Err(error) => panic!("failed to bind loopback Gateway endpoint: {error}"),
        };
        let store = hachimi_storage::AgentStore::connect(database)
            .await
            .unwrap_or_else(|error| panic!("failed to open Gateway ledger: {error}"));
        let builtins = hachimi_gateway::local_builtin_providers(store.clone(), &token)
            .unwrap_or_else(|error| panic!("failed to register builtin providers: {error}"));
        let providers = builtins.registry.clone();
        let sandbox_runtime_root = data_root
            .join("sandbox/windows/runtime")
            .join(hachimi_sandbox::SANDBOX_POLICY_VERSION);
        let backend: Arc<dyn hachimi_sandbox::SandboxBackend> = Arc::new(
            hachimi_sandbox::WindowsSandboxReadinessProbe::new(
                data_root.join("sandbox/windows/setup.json"),
            )
            .with_runtime(
                sandbox_runtime_root.join(executable_name("hachimi-sandbox-launcher")),
                sandbox_runtime_root.join(executable_name("hachimi-sandbox-canary")),
                data_root.join("sandbox/windows/attestation"),
            ),
        );
        let plugins = hachimi_extensions::PluginHost::new(store.clone(), data_root.join("plugins"));
        for definition in plugins
            .enabled_channel_sidecars()
            .await
            .unwrap_or_else(|error| panic!("failed to load channel contributions: {error}"))
        {
            let provider = hachimi_gateway::SandboxedStdioChannelProvider::new(
                Arc::clone(&backend),
                definition.manifest,
                definition.bundle_root,
                definition.executable,
                definition.args,
            )
            .unwrap_or_else(|error| panic!("failed to load channel provider: {error}"));
            providers
                .register(Arc::new(provider))
                .unwrap_or_else(|error| panic!("failed to register channel provider: {error}"));
        }
        let gateway = hachimi_gateway::GatewayHost::with_registry(store, providers.clone())
            .with_provider_ingress_enabled();
        gateway
            .bootstrap_provider_accounts(&builtins.accounts)
            .await
            .unwrap_or_else(|error| panic!("failed to load provider configuration: {error}"));
        gateway
            .reconcile_startup(now_ms())
            .await
            .unwrap_or_else(|error| panic!("failed to reconcile Gateway ledger: {error}"));
        let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
        loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    match plugins.enabled_channel_sidecars().await {
                        Ok(definitions) => {
                            for definition in definitions {
                                let unchanged = providers
                                    .resolve(&definition.manifest.id)
                                    .is_some_and(|provider| {
                                        provider.manifest().content_hash == definition.manifest.content_hash
                                    });
                                if unchanged {
                                    continue;
                                }
                                match hachimi_gateway::SandboxedStdioChannelProvider::new(
                                    Arc::clone(&backend),
                                    definition.manifest,
                                    definition.bundle_root,
                                    definition.executable,
                                    definition.args,
                                ) {
                                    Ok(provider) => {
                                        if let Err(error) = gateway
                                            .register_provider(Arc::new(provider), true)
                                            .await
                                        {
                                            tracing::warn!(%error, "Gateway plugin provider registration failed");
                                        }
                                    }
                                    Err(error) => tracing::warn!(%error, "Gateway plugin provider rejected"),
                                }
                            }
                        }
                        Err(error) => tracing::warn!(%error, "Gateway plugin provider discovery failed"),
                    }
                    gateway
                        .reload_configuration()
                        .await
                        .unwrap_or_else(|error| panic!("failed to reload Gateway providers: {error}"));
                    gateway
                        .heartbeat(std::process::id(), now_ms())
                        .await
                        .unwrap_or_else(|error| panic!("failed to persist Gateway heartbeat: {error}"));
                    for _ in 0..32 {
                        match gateway.process_next_provider_ingress().await {
                            Ok(Some(_)) => notify_desktop(&token).await,
                            Ok(None) => break,
                            Err(error) => {
                                tracing::warn!(%error, "Gateway provider ingress failed");
                                break;
                            }
                        }
                    }
                    for _ in 0..32 {
                        match gateway.process_next_provider_delivery(now_ms()).await {
                            Ok(Some(_)) => {}
                            Ok(None) => break,
                            Err(error) => {
                                tracing::warn!(%error, "Gateway provider delivery failed");
                                break;
                            }
                        }
                    }
                }
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, address)) if address.ip().is_loopback() => {
                            let gateway = gateway.clone();
                            let token = token.clone();
                            tokio::spawn(async move {
                                let _ = serve_loopback(stream, &gateway, &token).await;
                            });
                        }
                        Ok((mut stream, _)) => {
                            let _ = write_response(&mut stream, 403, "forbidden", None).await;
                        }
                        Err(error) => tracing::warn!(%error, "Gateway loopback accept failed"),
                    }
                }
            }
        }
    });
}

pub(super) fn ensure_running(executable: &Path) -> std::io::Result<()> {
    if std::net::TcpStream::connect_timeout(
        &LOOPBACK_ADDRESS.parse().expect("static Gateway address"),
        Duration::from_millis(150),
    )
    .is_ok()
    {
        return Ok(());
    }
    let mut command = std::process::Command::new(executable);
    command.arg("--gateway");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(0x0800_0000);
    }
    command.spawn().map(|_| ())
}

async fn serve_loopback(
    mut stream: tokio::net::TcpStream,
    gateway: &hachimi_gateway::GatewayHost,
    token: &str,
) -> Result<(), std::io::Error> {
    let request = match read_request(&mut stream).await {
        Ok(request) => request,
        Err(code) => return write_response(&mut stream, code, "invalid_request", None).await,
    };
    let (request_path, query) = request_target(&request.path);
    if request_path == WECOM_CALLBACK_PATH && request.method == "GET" {
        let Some(query) = wecom_query(query, true) else {
            return write_response(&mut stream, 400, "invalid_wecom_callback", None).await;
        };
        let credential = match keyring::Entry::new(
            "com.hachimi.channel",
            &format!("wecom:{}", query.account_id),
        )
        .and_then(|entry| entry.get_password())
        .ok()
        .and_then(|raw| hachimi_enterprise::EnterpriseCredential::parse(&raw).ok())
        {
            Some(credential) => credential,
            None => return write_response(&mut stream, 401, "unauthenticated", None).await,
        };
        let echo = match hachimi_enterprise::verify_wecom_callback_echo(
            &credential,
            &query.timestamp,
            &query.nonce,
            &query.signature,
            query.echo.as_deref().unwrap_or_default(),
            now_ms(),
        ) {
            Ok(echo) => echo,
            Err(_) => return write_response(&mut stream, 401, "unauthenticated", None).await,
        };
        return write_plain_response(&mut stream, 200, &echo).await;
    }
    if request_path == WECOM_CALLBACK_PATH && request.method == "POST" {
        let envelope = if let Some(query) = wecom_query(query, false) {
            match wecom_callback_envelope(&query, &request.body) {
                Some(envelope) => envelope,
                None => {
                    return write_response(&mut stream, 400, "invalid_wecom_callback", None).await;
                }
            }
        } else {
            match serde_json::from_slice::<ChannelEnvelope>(&request.body) {
                Ok(envelope) if envelope.route.channel == "wecom" => envelope,
                _ => return write_response(&mut stream, 400, "invalid_envelope", None).await,
            }
        };
        return match gateway.ingest_provider("wecom", None, envelope).await {
            Ok(_receipt) => {
                notify_desktop(token).await;
                write_plain_response(&mut stream, 200, "success").await
            }
            Err(error) => {
                let (status, code) = match error {
                    hachimi_gateway::GatewayError::Unauthenticated => (401, "unauthenticated"),
                    hachimi_gateway::GatewayError::RouteNotAllowed => (403, "route_not_allowed"),
                    hachimi_gateway::GatewayError::InvalidMessage => (400, "invalid_message"),
                    _ => (503, "gateway_unavailable"),
                };
                write_response(&mut stream, status, code, None).await
            }
        };
    }
    if request.method != "POST" {
        return write_response(&mut stream, 404, "not_found", None).await;
    }
    if request.authorization.as_deref() != Some(&format!("Bearer {token}")) {
        return write_response(&mut stream, 401, "unauthenticated", None).await;
    }
    if request_path == LOOPBACK_OUTBOX_PATH {
        let delivery = match gateway
            .claim_next_delivery_for_channel("loopback-webhook", now_ms())
            .await
        {
            Ok(delivery) => delivery,
            Err(_) => return write_response(&mut stream, 503, "gateway_unavailable", None).await,
        };
        let Some(delivery) = delivery else {
            return write_empty_response(&mut stream, 204).await;
        };
        let written = write_delivery_response(&mut stream, &delivery).await;
        let _ = gateway
            .finish_delivery(
                &delivery.id,
                written.is_ok(),
                true,
                written
                    .as_ref()
                    .err()
                    .map(|_| "loopback_delivery_write_failed"),
                now_ms(),
            )
            .await;
        return written;
    }
    if request_path != LOOPBACK_PATH {
        return write_response(&mut stream, 404, "not_found", None).await;
    }
    let envelope = match serde_json::from_slice::<ChannelEnvelope>(&request.body) {
        Ok(envelope) => envelope,
        Err(_) => return write_response(&mut stream, 400, "invalid_envelope", None).await,
    };
    match gateway
        .ingest_provider("loopback-webhook", Some(token), envelope)
        .await
    {
        Ok(receipt) => {
            notify_desktop(token).await;
            write_response(&mut stream, 202, "accepted", Some(&receipt)).await
        }
        Err(error) => {
            let (status, code) = match error {
                hachimi_gateway::GatewayError::Unauthenticated => (401, "unauthenticated"),
                hachimi_gateway::GatewayError::RouteNotAllowed => (403, "route_not_allowed"),
                hachimi_gateway::GatewayError::BotLoop => (409, "bot_loop"),
                hachimi_gateway::GatewayError::InvalidMessage => (400, "invalid_message"),
                _ => (503, "gateway_unavailable"),
            };
            write_response(&mut stream, status, code, None).await
        }
    }
}

struct WecomQuery {
    account_id: String,
    signature: String,
    timestamp: String,
    nonce: String,
    echo: Option<String>,
}

fn request_target(target: &str) -> (&str, Option<&str>) {
    target
        .split_once('?')
        .map_or((target, None), |(path, query)| (path, Some(query)))
}

fn wecom_query(query: Option<&str>, require_echo: bool) -> Option<WecomQuery> {
    let values = url::form_urlencoded::parse(query?.as_bytes())
        .collect::<std::collections::BTreeMap<_, _>>();
    let account_id = values.get("account_id")?.to_string();
    let signature = values.get("msg_signature")?.to_string();
    let timestamp = values.get("timestamp")?.to_string();
    let nonce = values.get("nonce")?.to_string();
    let echo = values.get("echostr").map(ToString::to_string);
    if account_id.is_empty()
        || account_id.len() > 128
        || !account_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
        || signature.len() != 40
        || !signature.bytes().all(|byte| byte.is_ascii_hexdigit())
        || timestamp.is_empty()
        || timestamp.len() > 20
        || !timestamp.bytes().all(|byte| byte.is_ascii_digit())
        || nonce.is_empty()
        || nonce.len() > 128
        || !nonce.bytes().all(|byte| byte.is_ascii_alphanumeric())
        || (require_echo && echo.as_deref().is_none_or(str::is_empty))
        || echo.as_ref().is_some_and(|value| value.len() > 48 * 1024)
    {
        return None;
    }
    Some(WecomQuery {
        account_id,
        signature,
        timestamp,
        nonce,
        echo,
    })
}

fn wecom_callback_envelope(query: &WecomQuery, body: &[u8]) -> Option<ChannelEnvelope> {
    let xml = std::str::from_utf8(body).ok()?;
    let encrypted = xml_tag(xml, "Encrypt")?;
    if encrypted.is_empty() || encrypted.len() > 48 * 1024 {
        return None;
    }
    Some(ChannelEnvelope {
        message_id: hachimi_protocol::ChannelMessageId::new(format!(
            "wecom:{}:{}",
            query.account_id, query.signature
        )),
        route: hachimi_protocol::ChannelRouteKey {
            channel: "wecom".into(),
            account: query.account_id.clone(),
            peer: "pending-verification".into(),
            thread: "pending-verification".into(),
        },
        sender: "pending-verification".into(),
        text: String::new(),
        metadata: serde_json::json!({
            "timestamp": query.timestamp,
            "nonce": query.nonce,
            "signature": query.signature,
            "encrypted": encrypted,
        }),
        authenticated: false,
        bot_generated: false,
        received_at_ms: now_ms(),
    })
}

fn xml_tag(xml: &str, tag: &str) -> Option<String> {
    let start = format!("<{tag}>");
    let end = format!("</{tag}>");
    let value = xml.split_once(&start)?.1.split_once(&end)?.0.trim();
    Some(
        value
            .strip_prefix("<![CDATA[")
            .and_then(|value| value.strip_suffix("]]>"))
            .unwrap_or(value)
            .to_owned(),
    )
}

fn executable_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

async fn notify_desktop(token: &str) {
    let notified = if let Ok(mut stream) =
        tokio::net::TcpStream::connect(DESKTOP_WAKE_ADDRESS).await
    {
        let request = format!(
            "POST /v1/gateway/wake HTTP/1.1\r\nHost: {DESKTOP_WAKE_ADDRESS}\r\nAuthorization: Bearer {token}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).await.is_ok()
    } else {
        false
    };
    #[cfg(test)]
    let _ = notified;
    #[cfg(not(test))]
    {
        if notified {
            return;
        }
        static LAST_WAKE: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
        let mut last = LAST_WAKE
            .get_or_init(|| Mutex::new(None))
            .lock()
            .expect("wake lock");
        if last.is_some_and(|instant| instant.elapsed() < Duration::from_secs(10)) {
            return;
        }
        *last = Some(Instant::now());
        let Ok(executable) = std::env::current_exe() else {
            return;
        };
        let mut command = std::process::Command::new(executable);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            command.creation_flags(0x0800_0000);
        }
        let _ = command.spawn();
    }
}

struct HttpRequest {
    method: String,
    path: String,
    authorization: Option<String>,
    body: Vec<u8>,
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> Result<HttpRequest, u16> {
    let mut bytes = Vec::with_capacity(4096);
    let header_end = loop {
        if bytes.len() >= MAX_HTTP_REQUEST_BYTES {
            return Err(413);
        }
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).await.map_err(|_| 400_u16)?;
        if read == 0 {
            return Err(400);
        }
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let header = std::str::from_utf8(&bytes[..header_end]).map_err(|_| 400_u16)?;
    let mut lines = header.split("\r\n");
    let request_line = lines.next().ok_or(400_u16)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or(400_u16)?.to_owned();
    let path = parts.next().ok_or(400_u16)?.to_owned();
    if parts.next() != Some("HTTP/1.1") || parts.next().is_some() {
        return Err(400);
    }
    let mut content_length = None;
    let mut authorization = None;
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').ok_or(400_u16)?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            content_length = Some(value.parse::<usize>().map_err(|_| 400_u16)?);
        } else if name.eq_ignore_ascii_case("authorization") {
            authorization = Some(value.to_owned());
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(400);
        }
    }
    let content_length = content_length.ok_or(411_u16)?;
    if header_end.saturating_add(content_length) > MAX_HTTP_REQUEST_BYTES {
        return Err(413);
    }
    while bytes.len() < header_end + content_length {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).await.map_err(|_| 400_u16)?;
        if read == 0 {
            return Err(400);
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    Ok(HttpRequest {
        method,
        path,
        authorization,
        body: bytes[header_end..header_end + content_length].to_vec(),
    })
}

async fn write_response(
    stream: &mut tokio::net::TcpStream,
    status: u16,
    code: &str,
    receipt: Option<&IngressReceipt>,
) -> Result<(), std::io::Error> {
    let reason = match status {
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Content Too Large",
        _ => "Service Unavailable",
    };
    let body = serde_json::to_vec(&serde_json::json!({ "code": code, "receipt": receipt }))
        .unwrap_or_else(|_| b"{\"code\":\"serialization_failed\"}".to_vec());
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(&body).await?;
    stream.shutdown().await
}

async fn write_plain_response(
    stream: &mut tokio::net::TcpStream,
    status: u16,
    body: &str,
) -> Result<(), std::io::Error> {
    let reason = if status == 200 { "OK" } else { "Accepted" };
    let bytes = body.as_bytes();
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        bytes.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(bytes).await?;
    stream.shutdown().await
}

async fn write_delivery_response(
    stream: &mut tokio::net::TcpStream,
    delivery: &hachimi_protocol::DeliveryAttempt,
) -> Result<(), std::io::Error> {
    let body = serde_json::to_vec(delivery).unwrap_or_else(|_| b"{}".to_vec());
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(&body).await?;
    stream.shutdown().await
}

async fn write_empty_response(
    stream: &mut tokio::net::TcpStream,
    status: u16,
) -> Result<(), std::io::Error> {
    let reason = if status == 204 { "No Content" } else { "OK" };
    stream
        .write_all(
            format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n"
            )
            .as_bytes(),
        )
        .await?;
    stream.shutdown().await
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hachimi_protocol::{ChannelMessageId, IngressStatus};
    use serde_json::{Value, json};

    fn route(thread: &str) -> ChannelRouteKey {
        ChannelRouteKey {
            channel: "loopback-webhook".into(),
            account: "local".into(),
            peer: "local-user".into(),
            thread: thread.into(),
        }
    }

    fn envelope(id: &str, route: ChannelRouteKey) -> ChannelEnvelope {
        ChannelEnvelope {
            message_id: ChannelMessageId::new(id),
            route,
            sender: "local-user".into(),
            text: "hello".into(),
            metadata: json!({}),
            authenticated: false,
            bot_generated: false,
            received_at_ms: 1,
        }
    }

    fn request(path: &str, token: &str, body: &[u8]) -> Vec<u8> {
        format!(
            "POST {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes()
        .into_iter()
        .chain(body.iter().copied())
        .collect()
    }

    async fn exchange(
        gateway: hachimi_gateway::GatewayHost,
        _channel: hachimi_gateway::LoopbackWebhookChannel,
        token: &str,
        request: Vec<u8>,
    ) -> Vec<u8> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let address = listener.local_addr().expect("address");
        let token = token.to_owned();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            serve_loopback(stream, &gateway, &token)
                .await
                .expect("serve");
        });
        let mut stream = tokio::net::TcpStream::connect(address)
            .await
            .expect("connect");
        stream.write_all(&request).await.expect("write");
        stream.shutdown().await.expect("shutdown request");
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.expect("read");
        server.await.expect("server");
        response
    }

    async fn configured_gateway(
        token: &str,
    ) -> (
        hachimi_gateway::GatewayHost,
        hachimi_gateway::LoopbackWebhookChannel,
    ) {
        let store = hachimi_storage::AgentStore::connect_in_memory()
            .await
            .expect("store");
        let builtins =
            hachimi_gateway::local_builtin_providers(store.clone(), token).expect("builtins");
        let channel = builtins.loopback.clone();
        let gateway = hachimi_gateway::GatewayHost::with_registry(store, builtins.registry.clone());
        gateway
            .bootstrap_provider_accounts(&builtins.accounts)
            .await
            .expect("accounts");
        (gateway, channel)
    }

    fn response_status(response: &[u8]) -> u16 {
        std::str::from_utf8(response)
            .expect("utf8 response")
            .split_whitespace()
            .nth(1)
            .expect("status")
            .parse()
            .expect("numeric status")
    }

    fn response_json(response: &[u8]) -> Value {
        let body = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| &response[index + 4..])
            .expect("response body");
        serde_json::from_slice(body).expect("json response")
    }

    #[tokio::test]
    async fn bounded_http_parser_rejects_chunked_requests() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let address = listener.local_addr().expect("address");
        let client = tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(address)
                .await
                .expect("connect");
            stream
                .write_all(b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n")
                .await
                .expect("write");
        });
        let (mut stream, _) = listener.accept().await.expect("accept");
        assert_eq!(read_request(&mut stream).await.err(), Some(400));
        client.await.expect("client");
    }

    #[test]
    fn wecom_callback_target_is_bounded_decoded_and_normalized() {
        let query = wecom_query(
            Some("account_id=release-wecom&msg_signature=0123456789abcdef0123456789abcdef01234567&timestamp=2000&nonce=abc123&echostr=echo%2Bvalue%3D"),
            true,
        )
        .expect("query");
        assert_eq!(query.account_id, "release-wecom");
        assert_eq!(query.echo.as_deref(), Some("echo+value="));
        let envelope = wecom_callback_envelope(
            &query,
            b"<xml><Encrypt><![CDATA[encrypted-value]]></Encrypt></xml>",
        )
        .expect("envelope");
        assert_eq!(envelope.route.channel, "wecom");
        assert_eq!(envelope.route.account, "release-wecom");
        assert_eq!(envelope.metadata["encrypted"], "encrypted-value");
        assert!(!envelope.authenticated);
    }

    #[tokio::test]
    async fn loopback_http_enforces_bearer_route_loop_and_dedup() {
        let token = "0123456789abcdef0123456789abcdef";
        let (gateway, channel) = configured_gateway(token).await;
        let allowed_route = route("main");
        let body =
            serde_json::to_vec(&envelope("message-1", allowed_route.clone())).expect("envelope");
        let unauthorized = exchange(
            gateway.clone(),
            channel.clone(),
            token,
            request(LOOPBACK_PATH, "wrong", &body),
        )
        .await;
        assert_eq!(response_status(&unauthorized), 401);

        let accepted = exchange(
            gateway.clone(),
            channel.clone(),
            token,
            request(LOOPBACK_PATH, token, &body),
        )
        .await;
        assert_eq!(response_status(&accepted), 202);
        assert_eq!(
            response_json(&accepted)["receipt"]["status"],
            json!(IngressStatus::Accepted)
        );
        let duplicate = exchange(
            gateway.clone(),
            channel.clone(),
            token,
            request(LOOPBACK_PATH, token, &body),
        )
        .await;
        assert_eq!(response_status(&duplicate), 202);
        assert_eq!(
            response_json(&duplicate)["receipt"]["status"],
            json!(IngressStatus::Duplicate)
        );

        let rejected_route =
            serde_json::to_vec(&envelope("message-2", route("other"))).expect("envelope");
        assert_eq!(
            response_status(
                &exchange(
                    gateway.clone(),
                    channel.clone(),
                    token,
                    request(LOOPBACK_PATH, token, &rejected_route),
                )
                .await
            ),
            403
        );
        let mut loop_message = envelope("message-3", allowed_route);
        loop_message.bot_generated = true;
        let loop_body = serde_json::to_vec(&loop_message).expect("envelope");
        assert_eq!(
            response_status(
                &exchange(
                    gateway,
                    channel,
                    token,
                    request(LOOPBACK_PATH, token, &loop_body),
                )
                .await
            ),
            409
        );
    }

    #[tokio::test]
    async fn loopback_outbox_claim_is_durable_and_not_repeated() {
        let token = "0123456789abcdef0123456789abcdef";
        let (gateway, channel) = configured_gateway(token).await;
        let empty = exchange(
            gateway.clone(),
            channel.clone(),
            token,
            request(LOOPBACK_OUTBOX_PATH, token, &[]),
        )
        .await;
        assert_eq!(response_status(&empty), 204);

        let queued = gateway
            .enqueue_delivery(route("main"), "reply-1", "done", now_ms())
            .await
            .expect("enqueue");
        let delivered = exchange(
            gateway.clone(),
            channel.clone(),
            token,
            request(LOOPBACK_OUTBOX_PATH, token, &[]),
        )
        .await;
        assert_eq!(response_status(&delivered), 200);
        assert_eq!(response_json(&delivered)["id"], json!(queued.id));
        assert_eq!(
            gateway.health().await.expect("health").pending_deliveries,
            0
        );
        let replay = exchange(
            gateway,
            channel,
            token,
            request(LOOPBACK_OUTBOX_PATH, token, &[]),
        )
        .await;
        assert_eq!(response_status(&replay), 204);
    }
}
