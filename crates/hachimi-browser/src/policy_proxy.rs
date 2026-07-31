use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use hachimi_protocol::BrowserNetworkPolicy;
use parking_lot::{Mutex, RwLock};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, lookup_host},
    sync::{mpsc, watch},
    task::JoinHandle,
};
use url::Url;

use crate::BrowserHostError;

const MAX_PROXY_HEADER_BYTES: usize = 64 * 1024;

/// A session-local forward proxy. The listening socket is never exposed outside
/// loopback and every outbound socket is opened only after an exact-origin and
/// resolved-address policy check.
pub(crate) struct PolicyProxy {
    address: SocketAddr,
    policy: Arc<RwLock<BrowserNetworkPolicy>>,
    shutdown: watch::Sender<bool>,
    task: JoinHandle<()>,
    denials: Arc<Mutex<mpsc::UnboundedReceiver<ProxyNetworkDenial>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProxyNetworkDenial {
    pub origin: String,
    pub private_network: bool,
    pub observed_at_ms: i64,
}

impl std::fmt::Debug for PolicyProxy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PolicyProxy")
            .field("address", &self.address)
            .field("revision", &self.policy.read().revision)
            .finish_non_exhaustive()
    }
}

impl PolicyProxy {
    pub(crate) async fn start(policy: BrowserNetworkPolicy) -> Result<Self, BrowserHostError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(proxy_error)?;
        let address = listener.local_addr().map_err(proxy_error)?;
        let policy = Arc::new(RwLock::new(policy));
        let (shutdown, mut shutdown_rx) = watch::channel(false);
        let (denial_tx, denial_rx) = mpsc::unbounded_channel();
        let task_policy = Arc::clone(&policy);
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    changed = shutdown_rx.changed() => {
                        if changed.is_err() || *shutdown_rx.borrow() {
                            break;
                        }
                    }
                    accepted = listener.accept() => {
                        let Ok((stream, peer)) = accepted else { break };
                        if !peer.ip().is_loopback() {
                            continue;
                        }
                        let connection_policy = Arc::clone(&task_policy);
                        let denial_tx = denial_tx.clone();
                        tokio::spawn(async move {
                            let _ = serve_connection(stream, connection_policy, denial_tx).await;
                        });
                    }
                }
            }
        });
        Ok(Self {
            address,
            policy,
            shutdown,
            task,
            denials: Arc::new(Mutex::new(denial_rx)),
        })
    }

    pub(crate) fn endpoint(&self) -> String {
        format!("http://{}", self.address)
    }

    pub(crate) fn update(&self, policy: BrowserNetworkPolicy) {
        *self.policy.write() = policy;
    }

    pub(crate) fn stop(&self) {
        let _ = self.shutdown.send(true);
        self.task.abort();
    }

    pub(crate) fn drain_denials(&self) -> Vec<ProxyNetworkDenial> {
        let mut receiver = self.denials.lock();
        let mut values = Vec::new();
        while let Ok(value) = receiver.try_recv() {
            if !values.iter().any(|existing: &ProxyNetworkDenial| {
                existing.origin == value.origin && existing.private_network == value.private_network
            }) {
                values.push(value);
            }
        }
        values
    }
}

async fn serve_connection(
    mut downstream: TcpStream,
    policy: Arc<RwLock<BrowserNetworkPolicy>>,
    denial_tx: mpsc::UnboundedSender<ProxyNetworkDenial>,
) -> Result<(), BrowserHostError> {
    let request = read_proxy_header(&mut downstream).await?;
    let header_end = find_header_end(&request).ok_or(BrowserHostError::InvalidInput)?;
    let header =
        std::str::from_utf8(&request[..header_end]).map_err(|_| BrowserHostError::InvalidInput)?;
    let first_line = header
        .split("\r\n")
        .next()
        .ok_or(BrowserHostError::InvalidInput)?;
    let mut pieces = first_line.split_whitespace();
    let method = pieces.next().ok_or(BrowserHostError::InvalidInput)?;
    let target = pieces.next().ok_or(BrowserHostError::InvalidInput)?;
    let version = pieces.next().ok_or(BrowserHostError::InvalidInput)?;
    if pieces.next().is_some() || !matches!(version, "HTTP/1.0" | "HTTP/1.1") {
        return reject(&mut downstream, "browser_proxy_invalid_request").await;
    }

    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = parse_authority(target, 443)?;
        let current_policy = policy.read().clone();
        let mut upstream = match connect_allowed(&current_policy, "https", &host, port).await {
            Ok(upstream) => upstream,
            Err(error) => {
                report_denial(&denial_tx, "https", &host, port, &error);
                let _ = reject(&mut downstream, error_code(&error)).await;
                return Err(error);
            }
        };
        downstream
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .map_err(proxy_error)?;
        let _ = tokio::io::copy_bidirectional(&mut downstream, &mut upstream)
            .await
            .map_err(proxy_error)?;
        return Ok(());
    }

    let url = Url::parse(target).map_err(|_| BrowserHostError::InvalidOrigin)?;
    if url.scheme() != "http" || url.username() != "" || url.password().is_some() {
        return reject(&mut downstream, "browser_proxy_scheme_denied").await;
    }
    let host = url.host_str().ok_or(BrowserHostError::InvalidOrigin)?;
    let port = url
        .port_or_known_default()
        .ok_or(BrowserHostError::InvalidOrigin)?;
    let body_length = match request_body_length(header) {
        Ok(value) => value,
        Err(error) => {
            let _ = reject(&mut downstream, "browser_proxy_request_body_denied").await;
            return Err(error);
        }
    };
    let buffered_body = &request[header_end..];
    if buffered_body.len() > body_length {
        let _ = reject(&mut downstream, "browser_proxy_pipelining_denied").await;
        return Err(BrowserHostError::InvalidInput);
    }
    let current_policy = policy.read().clone();
    let mut upstream = match connect_allowed(&current_policy, "http", host, port).await {
        Ok(upstream) => upstream,
        Err(error) => {
            report_denial(&denial_tx, "http", host, port, &error);
            let _ = reject(&mut downstream, error_code(&error)).await;
            return Err(error);
        }
    };
    let path = match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_owned(),
    };
    tracing::debug!(
        method,
        absolute_target = target.starts_with("http://"),
        "managed Browser proxy allowed request"
    );
    let rewritten = rewrite_http_request(header, method, &path, version)?;
    upstream.write_all(&rewritten).await.map_err(proxy_error)?;
    upstream
        .write_all(buffered_body)
        .await
        .map_err(proxy_error)?;
    let remaining = body_length.saturating_sub(buffered_body.len());
    if remaining > 0 {
        let copied = tokio::io::copy(&mut (&mut downstream).take(remaining as u64), &mut upstream)
            .await
            .map_err(proxy_error)?;
        if copied != remaining as u64 {
            return Err(BrowserHostError::InvalidInput);
        }
    }
    upstream.shutdown().await.map_err(proxy_error)?;
    let _ = tokio::io::copy(&mut upstream, &mut downstream)
        .await
        .map_err(proxy_error)?;
    downstream.shutdown().await.map_err(proxy_error)?;
    Ok(())
}

fn report_denial(
    sender: &mpsc::UnboundedSender<ProxyNetworkDenial>,
    scheme: &str,
    host: &str,
    port: u16,
    error: &BrowserHostError,
) {
    if !matches!(
        error,
        BrowserHostError::NetworkOriginDenied | BrowserHostError::PrivateNetworkDenied
    ) {
        return;
    }
    let Ok(origin) = normalized_origin(scheme, host, port) else {
        return;
    };
    let _ = sender.send(ProxyNetworkDenial {
        origin,
        private_network: *error == BrowserHostError::PrivateNetworkDenied,
        observed_at_ms: epoch_ms(),
    });
}

async fn read_proxy_header(stream: &mut TcpStream) -> Result<Vec<u8>, BrowserHostError> {
    let mut request = Vec::with_capacity(4096);
    while request.len() < MAX_PROXY_HEADER_BYTES {
        let mut chunk = [0_u8; 2048];
        let count = stream.read(&mut chunk).await.map_err(proxy_error)?;
        if count == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..count]);
        if find_header_end(&request).is_some() {
            return Ok(request);
        }
    }
    Err(BrowserHostError::InvalidInput)
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn rewrite_http_request(
    header: &str,
    method: &str,
    path: &str,
    version: &str,
) -> Result<Vec<u8>, BrowserHostError> {
    if path.contains(['\r', '\n']) {
        return Err(BrowserHostError::InvalidInput);
    }
    let mut output = format!("{method} {path} {version}\r\n");
    for line in header.split("\r\n").skip(1) {
        let lower = line.to_ascii_lowercase();
        if line.is_empty()
            || lower.starts_with("proxy-connection:")
            || lower.starts_with("connection:")
        {
            continue;
        }
        output.push_str(line);
        output.push_str("\r\n");
    }
    output.push_str("Connection: close\r\n\r\n");
    Ok(output.into_bytes())
}

fn request_body_length(header: &str) -> Result<usize, BrowserHostError> {
    let mut content_length = None;
    for line in header.split("\r\n").skip(1) {
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(BrowserHostError::InvalidInput);
        };
        let name = name.trim();
        let value = value.trim();
        if name.eq_ignore_ascii_case("transfer-encoding") || name.eq_ignore_ascii_case("expect") {
            return Err(BrowserHostError::InvalidInput);
        }
        if name.eq_ignore_ascii_case("content-length") {
            let parsed = value
                .parse::<usize>()
                .map_err(|_| BrowserHostError::InvalidInput)?;
            if content_length.is_some_and(|existing| existing != parsed) {
                return Err(BrowserHostError::InvalidInput);
            }
            content_length = Some(parsed);
        }
    }
    Ok(content_length.unwrap_or_default())
}

async fn connect_allowed(
    policy: &BrowserNetworkPolicy,
    scheme: &str,
    host: &str,
    port: u16,
) -> Result<TcpStream, BrowserHostError> {
    let origin = normalized_origin(scheme, host, port)?;
    let now = epoch_ms();
    let rules = policy
        .rules
        .iter()
        .filter(|rule| rule.origin == origin && rule.expires_at_ms.is_none_or(|value| value > now))
        .collect::<Vec<_>>();
    if rules.is_empty() {
        return Err(BrowserHostError::NetworkOriginDenied);
    }
    let addresses = lookup_host((host, port))
        .await
        .map_err(proxy_error)?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(BrowserHostError::NetworkResolutionDenied);
    }
    let private_allowed = rules.iter().any(|rule| rule.allow_private_network);
    let permitted = addresses
        .into_iter()
        .filter(|address| private_allowed || !is_non_public(address.ip()))
        .collect::<Vec<_>>();
    if permitted.is_empty() {
        return Err(BrowserHostError::PrivateNetworkDenied);
    }
    let mut last_error = None;
    for address in permitted {
        match TcpStream::connect(address).await {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }
    Err(proxy_error(last_error.unwrap_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            "no permitted address connected",
        )
    })))
}

fn normalized_origin(scheme: &str, host: &str, port: u16) -> Result<String, BrowserHostError> {
    if !matches!(scheme, "http" | "https") || host.trim().is_empty() {
        return Err(BrowserHostError::InvalidOrigin);
    }
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    let rendered_host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host
    };
    let default_port = (scheme == "http" && port == 80) || (scheme == "https" && port == 443);
    Ok(if default_port {
        format!("{scheme}://{rendered_host}")
    } else {
        format!("{scheme}://{rendered_host}:{port}")
    })
}

fn parse_authority(authority: &str, default_port: u16) -> Result<(String, u16), BrowserHostError> {
    let url =
        Url::parse(&format!("https://{authority}")).map_err(|_| BrowserHostError::InvalidOrigin)?;
    if url.username() != "" || url.password().is_some() || url.path() != "/" {
        return Err(BrowserHostError::InvalidOrigin);
    }
    Ok((
        url.host_str()
            .ok_or(BrowserHostError::InvalidOrigin)?
            .to_owned(),
        url.port().unwrap_or(default_port),
    ))
}

fn is_non_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ipv4_non_public(ip),
        IpAddr::V6(ip) => ipv6_non_public(ip),
    }
}

fn ipv4_non_public(ip: Ipv4Addr) -> bool {
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.octets()[0] == 0
        || ip.octets()[0] >= 240
        || matches!(ip.octets(), [100, second, _, _] if (64..=127).contains(&second))
        || matches!(
            ip.octets(),
            [192, 0, 0, _] | [192, 0, 2, _] | [198, 51, 100, _] | [203, 0, 113, _]
        )
        || matches!(ip.octets(), [198, second, _, _] if (18..=19).contains(&second))
}

fn ipv6_non_public(ip: Ipv6Addr) -> bool {
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
        || ip.is_multicast()
        || ip.to_ipv4_mapped().is_some_and(ipv4_non_public)
        || (ip.segments()[0] == 0x2001 && ip.segments()[1] == 0x0db8)
}

async fn reject(stream: &mut TcpStream, code: &str) -> Result<(), BrowserHostError> {
    let body = format!("{code}\n");
    let response = format!(
        "HTTP/1.1 403 Forbidden\r\nConnection: close\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(proxy_error)
}

fn error_code(error: &BrowserHostError) -> &'static str {
    match error {
        BrowserHostError::NetworkOriginDenied => "browser_proxy_origin_denied",
        BrowserHostError::PrivateNetworkDenied => "browser_proxy_private_network_denied",
        BrowserHostError::NetworkResolutionDenied => "browser_proxy_resolution_denied",
        _ => "browser_proxy_connection_failed",
    }
}

fn proxy_error(error: std::io::Error) -> BrowserHostError {
    BrowserHostError::Broker(format!("browser_policy_proxy: {error}"))
}

fn epoch_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|value| i64::try_from(value.as_millis()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hachimi_protocol::{BrowserNetworkRule, BrowserNetworkRuleKind};

    fn policy(origin: String, allow_private_network: bool) -> BrowserNetworkPolicy {
        BrowserNetworkPolicy {
            rules: vec![BrowserNetworkRule {
                origin,
                kind: BrowserNetworkRuleKind::Document,
                allow_private_network,
                expires_at_ms: None,
            }],
            deny_private_network_by_default: true,
            revision: 1,
        }
    }

    #[tokio::test]
    async fn private_and_unlisted_origins_fail_closed() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("listener");
        let port = listener.local_addr().expect("address").port();
        let origin = format!("http://127.0.0.1:{port}");
        assert_eq!(
            connect_allowed(&policy(origin.clone(), false), "http", "127.0.0.1", port)
                .await
                .expect_err("private must fail"),
            BrowserHostError::PrivateNetworkDenied
        );
        assert_eq!(
            connect_allowed(
                &policy("https://example.com".into(), false),
                "http",
                "127.0.0.1",
                port
            )
            .await
            .expect_err("origin must fail"),
            BrowserHostError::NetworkOriginDenied
        );
        let private_policy = policy(origin, true);
        let connected = connect_allowed(&private_policy, "http", "127.0.0.1", port);
        let accepted = listener.accept();
        let (connected, accepted) = tokio::join!(connected, accepted);
        assert!(connected.is_ok());
        assert!(accepted.is_ok());
    }

    #[tokio::test]
    async fn denied_proxy_origins_are_reported_without_granting_them() {
        let proxy = PolicyProxy::start(policy("https://example.com".into(), false))
            .await
            .expect("proxy");
        let mut stream = TcpStream::connect(proxy.address).await.expect("connect");
        stream
            .write_all(b"CONNECT 127.0.0.1:443 HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .await
            .expect("request");
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.expect("response");
        assert!(String::from_utf8_lossy(&response).contains("403 Forbidden"));
        let denials = proxy.drain_denials();
        assert_eq!(denials.len(), 1);
        assert_eq!(denials[0].origin, "https://127.0.0.1");
        assert!(!denials[0].private_network);
        proxy.stop();
    }

    #[tokio::test]
    async fn upstream_eof_is_forwarded_without_waiting_for_the_browser_to_close() {
        let upstream = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("upstream listener");
        let port = upstream.local_addr().expect("upstream address").port();
        let origin = format!("http://127.0.0.1:{port}");
        let proxy = PolicyProxy::start(policy(origin.clone(), true))
            .await
            .expect("proxy");
        let response = tokio::spawn(async move {
            let (mut stream, _) = upstream.accept().await.expect("accept upstream");
            let mut request = [0_u8; 4096];
            let count = stream.read(&mut request).await.expect("read request");
            assert!(String::from_utf8_lossy(&request[..count]).starts_with("GET /download"));
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 8\r\nConnection: close\r\n\r\ndownload",
                )
                .await
                .expect("write response");
        });
        let mut client = TcpStream::connect(proxy.address)
            .await
            .expect("connect proxy");
        client
            .write_all(
                format!(
                    "GET {origin}/download HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: keep-alive\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .expect("write proxied request");
        let mut body = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            client.read_to_end(&mut body),
        )
        .await
        .expect("proxy did not forward upstream EOF")
        .expect("read proxied response");
        assert!(body.ends_with(b"download"));
        response.await.expect("upstream task");
        proxy.stop();
    }

    #[test]
    fn private_address_classification_covers_local_and_link_local_ranges() {
        for value in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.1.2",
            "100.64.0.1",
            "::1",
            "fe80::1",
            "fd00::1",
        ] {
            assert!(is_non_public(value.parse().expect("address")), "{value}");
        }
        assert!(!is_non_public("8.8.8.8".parse().expect("public")));
        assert!(!is_non_public(
            "2606:4700:4700::1111".parse().expect("public")
        ));
    }

    #[test]
    fn request_rewrite_removes_proxy_connection_and_absolute_target() {
        let rewritten = rewrite_http_request(
            "GET http://example.com/a HTTP/1.1\r\nHost: example.com\r\nProxy-Connection: keep-alive\r\n",
            "GET",
            "/a",
            "HTTP/1.1",
        )
        .expect("rewrite");
        let text = String::from_utf8(rewritten).expect("utf8");
        assert!(text.starts_with("GET /a HTTP/1.1\r\n"));
        assert!(!text.to_ascii_lowercase().contains("proxy-connection"));
        assert!(text.ends_with("Connection: close\r\n\r\n"));
    }

    #[test]
    fn request_body_framing_rejects_ambiguous_streaming_headers() {
        assert_eq!(
            request_body_length("POST / HTTP/1.1\r\nContent-Length: 4\r\n\r\n"),
            Ok(4)
        );
        assert!(
            request_body_length(
                "POST / HTTP/1.1\r\nContent-Length: 4\r\nContent-Length: 5\r\n\r\n"
            )
            .is_err()
        );
        assert!(
            request_body_length("POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n").is_err()
        );
        assert!(request_body_length("POST / HTTP/1.1\r\nExpect: 100-continue\r\n\r\n").is_err());
    }

    #[tokio::test]
    async fn pipelined_http_requests_are_rejected_before_an_upstream_socket_opens() {
        let upstream = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("upstream listener");
        let port = upstream.local_addr().expect("upstream address").port();
        let origin = format!("http://127.0.0.1:{port}");
        let proxy = PolicyProxy::start(policy(origin.clone(), true))
            .await
            .expect("proxy");
        let mut client = TcpStream::connect(proxy.address)
            .await
            .expect("connect proxy");
        client
            .write_all(
                format!(
                    "GET {origin}/first HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\nGET {origin}/second HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .expect("write pipelined requests");
        let mut response = Vec::new();
        client
            .read_to_end(&mut response)
            .await
            .expect("read rejection");
        assert!(String::from_utf8_lossy(&response).contains("browser_proxy_pipelining_denied"));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(200), upstream.accept())
                .await
                .is_err(),
            "proxy opened an upstream socket for a pipelined request"
        );
        proxy.stop();
    }
}
