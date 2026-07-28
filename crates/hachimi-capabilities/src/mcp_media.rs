// SPDX-License-Identifier: Apache-2.0
// Adapted from OpenAI Codex commit 4c43465133428898aa84f0bfc02c306ed65fb66a:
// codex-rs/app-server/src/image_url.rs and app-server-protocol item media boundaries.
//! Host-side MCP media normalization.
//!
//! MCP servers are untrusted. A model or a WebView never receives a remote
//! media URL or inline binary payload directly. The host validates the URL,
//! downloads through a no-redirect bounded client, checks the declared type,
//! hashes the bytes, and exposes only a content reference.

use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use hachimi_protocol::McpMediaReference;
use reqwest::{Client, Url, header::CONTENT_TYPE};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

const MAX_MEDIA_BYTES: usize = 8 * 1024 * 1024;
const MAX_MEDIA_URL_CHARS: usize = 2_048;
const MAX_MEDIA_MIME_CHARS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum McpMediaError {
    #[error("MCP media URL is invalid")]
    InvalidUrl,
    #[error("MCP media URL uses an unsupported scheme")]
    UnsupportedScheme,
    #[error("MCP media response was redirected")]
    Redirect,
    #[error("MCP media response failed")]
    Response,
    #[error("MCP media response exceeded the byte budget")]
    TooLarge,
    #[error("MCP media type is missing or unsupported")]
    InvalidType,
    #[error("MCP media bytes do not match the declared type")]
    InvalidBytes,
    #[error("MCP media base64 is invalid")]
    InvalidBase64,
    #[error("MCP media download was cancelled")]
    Cancelled,
}

impl McpMediaError {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::InvalidUrl => "mcp_media_invalid_url",
            Self::UnsupportedScheme => "mcp_media_unsupported_scheme",
            Self::Redirect => "mcp_media_redirect_rejected",
            Self::Response => "mcp_media_response_failed",
            Self::TooLarge => "mcp_media_too_large",
            Self::InvalidType => "mcp_media_invalid_type",
            Self::InvalidBytes => "mcp_media_invalid_bytes",
            Self::InvalidBase64 => "mcp_media_invalid_base64",
            Self::Cancelled => "mcp_media_cancelled",
        }
    }
}

#[derive(Clone)]
pub struct McpMediaHost {
    client: Client,
}

impl Default for McpMediaHost {
    fn default() -> Self {
        Self::new()
    }
}

impl McpMediaHost {
    #[must_use]
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("bounded MCP media HTTP client must build");
        Self { client }
    }

    #[must_use]
    pub fn with_client(client: Client) -> Self {
        Self { client }
    }

    pub async fn fetch_remote(
        &self,
        url: &str,
        expected_kind: Option<&str>,
        cancellation: CancellationToken,
    ) -> Result<McpMediaReference, McpMediaError> {
        let url = validate_media_url(url)?;
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(McpMediaError::Cancelled),
            result = self.client.get(url).send() => result.map_err(|_| McpMediaError::Response)?,
        };
        if response.status().is_redirection() {
            return Err(McpMediaError::Redirect);
        }
        if !response.status().is_success() {
            return Err(McpMediaError::Response);
        }
        let mime = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(normalize_mime)
            .ok_or(McpMediaError::InvalidType)?;
        validate_mime(&mime, expected_kind)?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_MEDIA_BYTES as u64)
        {
            return Err(McpMediaError::TooLarge);
        }
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = tokio::select! {
            _ = cancellation.cancelled() => return Err(McpMediaError::Cancelled),
            chunk = stream.next() => chunk,
        } {
            let chunk = chunk.map_err(|_| McpMediaError::Response)?;
            if bytes.len().saturating_add(chunk.len()) > MAX_MEDIA_BYTES {
                return Err(McpMediaError::TooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        self.reference_for_bytes(&mime, expected_kind, &bytes)
    }

    pub fn materialize_inline(
        &self,
        mime: &str,
        encoded: &str,
        expected_kind: Option<&str>,
    ) -> Result<McpMediaReference, McpMediaError> {
        if encoded.len()
            > MAX_MEDIA_BYTES
                .saturating_mul(4)
                .div_ceil(3)
                .saturating_add(4)
        {
            return Err(McpMediaError::TooLarge);
        }
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|_| McpMediaError::InvalidBase64)?;
        if bytes.len() > MAX_MEDIA_BYTES {
            return Err(McpMediaError::TooLarge);
        }
        let mime = normalize_mime(mime).ok_or(McpMediaError::InvalidType)?;
        self.reference_for_bytes(&mime, expected_kind, &bytes)
    }

    pub async fn normalize_content_item(
        &self,
        item: &Value,
        cancellation: CancellationToken,
    ) -> Value {
        let Some(kind) = item.get("type").and_then(Value::as_str) else {
            return json!({"type": "unknown", "errorCode": "mcp_media_invalid_type"});
        };
        if !matches!(kind, "image" | "audio" | "video") {
            return item.clone();
        }
        let mime = item
            .get("mimeType")
            .or_else(|| item.get("mime_type"))
            .and_then(Value::as_str);
        let data = item.get("data").and_then(Value::as_str);
        let url = item
            .get("url")
            .and_then(Value::as_str)
            .or(data.filter(|value| value.starts_with("https://") || value.starts_with("http://")));
        let result = if let Some(url) = url {
            self.fetch_remote(url, Some(kind), cancellation).await
        } else if let (Some(mime), Some(data)) = (mime, data) {
            self.materialize_inline(mime, data, Some(kind))
        } else {
            Err(McpMediaError::InvalidType)
        };
        match result {
            Ok(reference) => json!({
                "type": kind,
                "mimeType": reference.mime_type,
                "contentReference": reference,
            }),
            Err(error) => json!({ "type": kind, "errorCode": error.stable_code() }),
        }
    }

    fn reference_for_bytes(
        &self,
        mime: &str,
        expected_kind: Option<&str>,
        bytes: &[u8],
    ) -> Result<McpMediaReference, McpMediaError> {
        validate_mime(mime, expected_kind)?;
        validate_signature(mime, bytes)?;
        let digest = Sha256::digest(bytes);
        let sha256 = hex(&digest);
        Ok(McpMediaReference {
            id: format!("mcp-media:{sha256}"),
            mime_type: mime.to_owned(),
            byte_length: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            sha256,
        })
    }
}

fn validate_media_url(value: &str) -> Result<Url, McpMediaError> {
    if value.trim().is_empty() || value.len() > MAX_MEDIA_URL_CHARS {
        return Err(McpMediaError::InvalidUrl);
    }
    let url = Url::parse(value).map_err(|_| McpMediaError::InvalidUrl)?;
    let loopback = url.scheme() == "http"
        && url
            .host_str()
            .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1"));
    if url.scheme() != "https" && !loopback {
        return Err(McpMediaError::UnsupportedScheme);
    }
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err(McpMediaError::InvalidUrl);
    }
    Ok(url)
}

fn normalize_mime(value: &str) -> Option<String> {
    let mime = value.split(';').next()?.trim().to_ascii_lowercase();
    (!mime.is_empty() && mime.len() <= MAX_MEDIA_MIME_CHARS).then_some(mime)
}

fn validate_mime(mime: &str, expected_kind: Option<&str>) -> Result<(), McpMediaError> {
    let allowed = matches!(
        mime,
        "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "audio/mpeg"
            | "audio/ogg"
            | "audio/wav"
            | "audio/wave"
            | "video/mp4"
            | "video/webm"
            | "application/pdf"
    );
    if !allowed {
        return Err(McpMediaError::InvalidType);
    }
    if expected_kind.is_some_and(|kind| !mime.starts_with(&format!("{kind}/"))) {
        return Err(McpMediaError::InvalidType);
    }
    Ok(())
}

fn validate_signature(mime: &str, bytes: &[u8]) -> Result<(), McpMediaError> {
    let valid = match mime {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/webp" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
        "application/pdf" => bytes.starts_with(b"%PDF-"),
        "audio/ogg" => bytes.starts_with(b"OggS"),
        "audio/wav" | "audio/wave" => {
            bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE")
        }
        // MPEG streams have several valid sync/header forms; type and size
        // validation still apply, while the decoder remains the authority.
        "audio/mpeg" | "video/mp4" | "video/webm" => true,
        _ => false,
    };
    valid.then_some(()).ok_or(McpMediaError::InvalidBytes)
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        value.push(char::from(HEX[usize::from(byte >> 4)]));
        value.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_media_is_reduced_to_hashed_reference() {
        let host = McpMediaHost::new();
        let reference = host
            .materialize_inline(
                "image/png",
                &STANDARD.encode(b"\x89PNG\r\n\x1a\nfixture"),
                Some("image"),
            )
            .expect("reference");
        assert!(reference.id.starts_with("mcp-media:"));
        assert_eq!(reference.mime_type, "image/png");
        assert_eq!(reference.byte_length, 15);
        assert_eq!(reference.sha256.len(), 64);
    }

    #[test]
    fn media_validation_rejects_remote_credentials_redirect_types_and_bad_magic() {
        assert!(matches!(
            validate_media_url("http://example.test/image.png"),
            Err(McpMediaError::UnsupportedScheme)
        ));
        assert!(matches!(
            validate_media_url("https://user@example.test/image.png"),
            Err(McpMediaError::InvalidUrl)
        ));
        assert!(matches!(
            validate_mime("text/html", Some("image")),
            Err(McpMediaError::InvalidType)
        ));
        assert!(matches!(
            validate_signature("image/png", b"not-png"),
            Err(McpMediaError::InvalidBytes)
        ));
    }

    #[tokio::test]
    async fn normalize_item_never_returns_original_url_or_bytes() {
        let host = McpMediaHost::new();
        let value = host
            .normalize_content_item(
                &json!({
                    "type": "image",
                    "mimeType": "image/png",
                    "data": STANDARD.encode(b"\x89PNG\r\n\x1a\nfixture")
                }),
                CancellationToken::new(),
            )
            .await;
        assert!(value.get("contentReference").is_some());
        assert!(value.get("data").is_none());
    }
}
