use std::{io, sync::Arc};

use hachimi_browser::{BrowserHost, ChromeExtensionBroker, ExtensionCommandResult};
use serde::Deserialize;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const EXTENSION_ADDRESS: &str = "127.0.0.1:42372";
const MAX_REQUEST_BYTES: usize = 256 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairRequest {
    nonce: String,
    extension_identity: String,
}

pub(super) async fn run(
    browser: Arc<BrowserHost>,
    broker: Arc<ChromeExtensionBroker>,
) -> Result<(), io::Error> {
    let listener = tokio::net::TcpListener::bind(EXTENSION_ADDRESS).await?;
    tracing::info!(
        address = EXTENSION_ADDRESS,
        "Chrome extension broker listening"
    );
    loop {
        let (stream, peer) = listener.accept().await?;
        if !peer.ip().is_loopback() {
            continue;
        }
        let browser = Arc::clone(&browser);
        let broker = Arc::clone(&broker);
        tokio::spawn(async move {
            if let Err(error) = handle(stream, browser, broker).await {
                tracing::debug!(%error, "Chrome extension request failed");
            }
        });
    }
}

async fn handle(
    mut stream: tokio::net::TcpStream,
    browser: Arc<BrowserHost>,
    broker: Arc<ChromeExtensionBroker>,
) -> Result<(), io::Error> {
    let request = match read_request(&mut stream).await {
        Ok(request) => request,
        Err(status) => {
            return write_json(&mut stream, status, &json!({"error":"invalid_request"})).await;
        }
    };
    if request.method == "OPTIONS" {
        return write_json(&mut stream, 204, &json!({})).await;
    }
    if request.method != "POST" {
        return write_json(&mut stream, 405, &json!({"error":"method_not_allowed"})).await;
    }
    match request.path.as_str() {
        "/v1/pair" => {
            let input: PairRequest = match serde_json::from_slice(&request.body) {
                Ok(input) => input,
                Err(_) => {
                    return write_json(&mut stream, 400, &json!({"error":"invalid_json"})).await;
                }
            };
            if !extension_origin_matches_identity(
                request.origin.as_deref(),
                &input.extension_identity,
            ) {
                return write_json(
                    &mut stream,
                    403,
                    &json!({"error":"pairing_transport_rejected"}),
                )
                .await;
            }
            let pairing =
                match browser.confirm_extension_pairing(&input.nonce, &input.extension_identity) {
                    Ok(pairing) => pairing,
                    Err(error) => {
                        return write_json(
                            &mut stream,
                            403,
                            &json!({"error":"pairing_rejected","message":error.to_string()}),
                        )
                        .await;
                    }
                };
            let token = match broker.register_identity(&input.extension_identity) {
                Ok(token) => token,
                Err(error) => {
                    return write_json(
                        &mut stream,
                        403,
                        &json!({"error":"identity_rejected","message":error.to_string()}),
                    )
                    .await;
                }
            };
            write_json(
                &mut stream,
                200,
                &json!({
                    "token": token,
                    "pairingId": pairing.id,
                    "expiresAtMs": pairing.expires_at_ms,
                }),
            )
            .await
        }
        "/v1/commands/claim" => {
            let Some(token) = bearer_token(request.authorization.as_deref()) else {
                return write_json(&mut stream, 401, &json!({"error":"unauthorized"})).await;
            };
            match broker.claim(token) {
                Ok(Some(command)) => write_json(&mut stream, 200, &json!(command)).await,
                Ok(None) => write_json(&mut stream, 204, &json!({})).await,
                Err(_) => write_json(&mut stream, 401, &json!({"error":"unauthorized"})).await,
            }
        }
        "/v1/commands/complete" => {
            let Some(token) = bearer_token(request.authorization.as_deref()) else {
                return write_json(&mut stream, 401, &json!({"error":"unauthorized"})).await;
            };
            let result: ExtensionCommandResult = match serde_json::from_slice(&request.body) {
                Ok(result) => result,
                Err(_) => {
                    return write_json(&mut stream, 400, &json!({"error":"invalid_json"})).await;
                }
            };
            match broker.complete(token, result) {
                Ok(()) => write_json(&mut stream, 202, &json!({"accepted":true})).await,
                Err(error) => {
                    write_json(
                        &mut stream,
                        409,
                        &json!({"error":"completion_rejected","message":error.to_string()}),
                    )
                    .await
                }
            }
        }
        _ => write_json(&mut stream, 404, &json!({"error":"not_found"})).await,
    }
}

struct HttpRequest {
    method: String,
    path: String,
    authorization: Option<String>,
    origin: Option<String>,
    body: Vec<u8>,
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> Result<HttpRequest, u16> {
    let mut bytes = Vec::with_capacity(4096);
    let header_end = loop {
        if bytes.len() >= MAX_REQUEST_BYTES {
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
    let mut origin = None;
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').ok_or(400_u16)?;
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(400);
        }
        if name.eq_ignore_ascii_case("content-length") {
            content_length = Some(value.trim().parse::<usize>().map_err(|_| 400_u16)?);
        } else if name.eq_ignore_ascii_case("authorization") {
            authorization = Some(value.trim().to_owned());
        } else if name.eq_ignore_ascii_case("origin") {
            origin = Some(value.trim().to_owned());
        }
    }
    let content_length = content_length.ok_or(411_u16)?;
    if header_end.saturating_add(content_length) > MAX_REQUEST_BYTES {
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
        origin,
        body: bytes[header_end..header_end + content_length].to_vec(),
    })
}

fn extension_origin_matches_identity(origin: Option<&str>, extension_identity: &str) -> bool {
    let Some(extension_id) = origin.and_then(|value| value.strip_prefix("chrome-extension://"))
    else {
        return false;
    };
    extension_id.len() == 32
        && extension_id.bytes().all(|byte| matches!(byte, b'a'..=b'p'))
        && extension_identity
            .strip_prefix(extension_id)
            .and_then(|suffix| suffix.strip_prefix(':'))
            .is_some_and(|install_id| {
                (1..=128).contains(&install_id.len()) && !install_id.contains(['\0', '\r', '\n'])
            })
}

fn bearer_token(value: Option<&str>) -> Option<&str> {
    value?
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty())
}

async fn write_json(
    stream: &mut tokio::net::TcpStream,
    status: u16,
    body: &serde_json::Value,
) -> Result<(), io::Error> {
    let body = if status == 204 {
        Vec::new()
    } else {
        serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec())
    };
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Payload Too Large",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: Authorization, Content-Type\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(&body).await?;
    stream.shutdown().await
}

#[cfg(test)]
mod tests {
    use super::extension_origin_matches_identity;

    #[test]
    fn pairing_is_bound_to_the_real_chrome_extension_origin() {
        let id = "abcdefghijklmnopabcdefghijklmnop";
        assert!(extension_origin_matches_identity(
            Some(&format!("chrome-extension://{id}")),
            &format!("{id}:install-identity"),
        ));
        assert!(!extension_origin_matches_identity(
            Some("https://attacker.example"),
            &format!("{id}:install-identity"),
        ));
        assert!(!extension_origin_matches_identity(
            Some(&format!("chrome-extension://{id}")),
            "ponmlkjihgfedcbaponmlkjihgfedcba:install-identity",
        ));
        assert!(!extension_origin_matches_identity(
            None,
            &format!("{id}:install-identity")
        ));
    }
}
