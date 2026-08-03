use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    time::{SystemTime, UNIX_EPOCH},
};

use hachimi_protocol::{
    BrowserAction, BrowserFileToken, BrowserImportedDownload, BrowserNetworkPolicy,
    BrowserNetworkRuleKind, BrowserProfileKind, BrowserSessionId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::BrowserHostError;

pub type BrowserBrokerFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, BrowserHostError>> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq)]
pub struct BrokerObservation {
    pub url: String,
    pub title: String,
    pub text: String,
    pub screenshot_png: Option<Vec<u8>>,
    pub viewport_width: Option<u32>,
    pub viewport_height: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BrokerActionResult {
    pub result_code: String,
    pub output: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerNetworkDenial {
    pub origin: String,
    pub network_kind: BrowserNetworkRuleKind,
    pub private_network: bool,
    pub observed_at_ms: i64,
}

pub trait BrowserBroker: Send + Sync {
    fn attest_profile<'a>(
        &'a self,
        profile_kind: BrowserProfileKind,
    ) -> BrowserBrokerFuture<'a, ()> {
        let _ = profile_kind;
        Box::pin(async { Err(BrowserHostError::BrokerUnavailable) })
    }

    fn start<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
        profile_kind: BrowserProfileKind,
        initial_url: Option<&'a str>,
        initial_network_policy: BrowserNetworkPolicy,
        extension_identity: Option<&'a str>,
    ) -> BrowserBrokerFuture<'a, ()>;

    fn observe<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
    ) -> BrowserBrokerFuture<'a, BrokerObservation>;

    fn act<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
        expected_origin: &'a str,
        action: &'a BrowserAction,
    ) -> BrowserBrokerFuture<'a, BrokerActionResult>;

    fn stage_upload<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
        source: &'a Path,
    ) -> BrowserBrokerFuture<'a, BrowserFileToken> {
        let _ = (session_id, source);
        Box::pin(async { Err(BrowserHostError::BrokerUnavailable) })
    }

    fn import_download<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
        download_token: &'a str,
        destination: &'a Path,
    ) -> BrowserBrokerFuture<'a, BrowserImportedDownload> {
        let _ = (session_id, download_token, destination);
        Box::pin(async { Err(BrowserHostError::BrokerUnavailable) })
    }

    fn set_network_policy<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
        policy: BrowserNetworkPolicy,
    ) -> BrowserBrokerFuture<'a, ()> {
        let _ = (session_id, policy);
        Box::pin(async { Err(BrowserHostError::BrokerUnavailable) })
    }

    fn drain_network_denials<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
    ) -> BrowserBrokerFuture<'a, Vec<BrokerNetworkDenial>> {
        let _ = session_id;
        Box::pin(async { Ok(Vec::new()) })
    }

    fn take_over<'a>(&'a self, session_id: &'a BrowserSessionId) -> BrowserBrokerFuture<'a, ()>;

    fn stop<'a>(&'a self, session_id: &'a BrowserSessionId) -> BrowserBrokerFuture<'a, ()>;
}

#[derive(Debug, Default)]
pub struct UnavailableBrowserBroker;

impl BrowserBroker for UnavailableBrowserBroker {
    fn start<'a>(
        &'a self,
        _session_id: &'a BrowserSessionId,
        _profile_kind: BrowserProfileKind,
        _initial_url: Option<&'a str>,
        _initial_network_policy: BrowserNetworkPolicy,
        _extension_identity: Option<&'a str>,
    ) -> BrowserBrokerFuture<'a, ()> {
        Box::pin(async { Err(BrowserHostError::BrokerUnavailable) })
    }

    fn observe<'a>(
        &'a self,
        _session_id: &'a BrowserSessionId,
    ) -> BrowserBrokerFuture<'a, BrokerObservation> {
        Box::pin(async { Err(BrowserHostError::BrokerUnavailable) })
    }

    fn act<'a>(
        &'a self,
        _session_id: &'a BrowserSessionId,
        _expected_origin: &'a str,
        _action: &'a BrowserAction,
    ) -> BrowserBrokerFuture<'a, BrokerActionResult> {
        Box::pin(async { Err(BrowserHostError::BrokerUnavailable) })
    }

    fn stage_upload<'a>(
        &'a self,
        _session_id: &'a BrowserSessionId,
        _source: &'a Path,
    ) -> BrowserBrokerFuture<'a, BrowserFileToken> {
        Box::pin(async { Err(BrowserHostError::BrokerUnavailable) })
    }

    fn import_download<'a>(
        &'a self,
        _session_id: &'a BrowserSessionId,
        _download_token: &'a str,
        _destination: &'a Path,
    ) -> BrowserBrokerFuture<'a, BrowserImportedDownload> {
        Box::pin(async { Err(BrowserHostError::BrokerUnavailable) })
    }

    fn take_over<'a>(&'a self, _session_id: &'a BrowserSessionId) -> BrowserBrokerFuture<'a, ()> {
        Box::pin(async { Err(BrowserHostError::BrokerUnavailable) })
    }

    fn stop<'a>(&'a self, _session_id: &'a BrowserSessionId) -> BrowserBrokerFuture<'a, ()> {
        Box::pin(async { Err(BrowserHostError::BrokerUnavailable) })
    }
}

pub(crate) fn resolve_upload_token(
    session_id: &BrowserSessionId,
    root: &Path,
    token: &str,
) -> Result<PathBuf, BrowserHostError> {
    validate_staged_file(session_id, root, token)
        .map(|(path, _)| path)
        .map_err(|_| BrowserHostError::UploadTokenInvalid)
}

const MAX_BROWSER_FILE_BYTES: u64 = 100 * 1024 * 1024;
const BROWSER_FILE_TTL_MS: i64 = 60 * 60 * 1_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserFileMetadata {
    version: u32,
    session_id: String,
    token: String,
    file_name: String,
    size: u64,
    sha256: String,
    created_at_ms: i64,
    expires_at_ms: i64,
}

pub(crate) fn stage_upload_file(
    session_id: &BrowserSessionId,
    root: &Path,
    source: &Path,
) -> Result<BrowserFileToken, BrowserHostError> {
    let metadata =
        std::fs::symlink_metadata(source).map_err(|_| BrowserHostError::UploadTokenInvalid)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_BROWSER_FILE_BYTES
    {
        return Err(BrowserHostError::UploadTokenInvalid);
    }
    let file_name = safe_file_name(source)?;
    let token = opaque_file_token(&file_name);
    std::fs::create_dir_all(root).map_err(|error| BrowserHostError::Broker(error.to_string()))?;
    cleanup_expired_file_tokens(root);
    let destination = root.join(&token);
    copy_new(source, &destination)?;
    let bytes =
        std::fs::read(&destination).map_err(|error| BrowserHostError::Broker(error.to_string()))?;
    let sha256 = hex_digest(&Sha256::digest(bytes));
    let expires_at_ms = write_file_token_metadata(
        session_id,
        root,
        &token,
        &file_name,
        metadata.len(),
        &sha256,
    )?;
    Ok(BrowserFileToken {
        browser_session_id: session_id.clone(),
        token,
        file_name,
        size: metadata.len(),
        sha256,
        expires_at_ms,
    })
}

pub(crate) fn stage_download_file(
    session_id: &BrowserSessionId,
    root: &Path,
    source: &Path,
    declared_mime: Option<&str>,
    allow_unknown_type: bool,
) -> Result<(BrowserFileToken, String), BrowserHostError> {
    let mime = validate_download_file(source, declared_mime, allow_unknown_type)?;
    stage_upload_file(session_id, root, source).map(|token| (token, mime))
}

pub(crate) fn import_download_file(
    session_id: &BrowserSessionId,
    root: &Path,
    token: &str,
    destination: &Path,
) -> Result<BrowserImportedDownload, BrowserHostError> {
    let (canonical_source, token_metadata) = validate_staged_file(session_id, root, token)
        .map_err(|_| BrowserHostError::DownloadFailed)?;
    copy_new(&canonical_source, destination)?;
    let bytes =
        std::fs::read(destination).map_err(|error| BrowserHostError::Broker(error.to_string()))?;
    Ok(BrowserImportedDownload {
        browser_session_id: session_id.clone(),
        download_token: token.to_owned(),
        destination: destination.to_string_lossy().into_owned(),
        size: token_metadata.size,
        sha256: hex_digest(&Sha256::digest(bytes)),
    })
}

fn write_file_token_metadata(
    session_id: &BrowserSessionId,
    root: &Path,
    token: &str,
    file_name: &str,
    size: u64,
    sha256: &str,
) -> Result<i64, BrowserHostError> {
    let created_at_ms = epoch_ms();
    let expires_at_ms = created_at_ms.saturating_add(BROWSER_FILE_TTL_MS);
    let metadata = BrowserFileMetadata {
        version: 1,
        session_id: session_id.as_str().to_owned(),
        token: token.to_owned(),
        file_name: file_name.to_owned(),
        size,
        sha256: sha256.to_owned(),
        created_at_ms,
        expires_at_ms,
    };
    let encoded = serde_json::to_vec(&metadata)
        .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
    let path = file_metadata_path(root, token);
    let write = (|| {
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
        std::io::Write::write_all(&mut output, &encoded)
            .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
        output
            .sync_all()
            .map_err(|error| BrowserHostError::Broker(error.to_string()))
    })();
    if let Err(error) = write {
        let _ = std::fs::remove_file(root.join(token));
        return Err(error);
    }
    Ok(expires_at_ms)
}

fn validate_staged_file(
    session_id: &BrowserSessionId,
    root: &Path,
    token: &str,
) -> Result<(PathBuf, BrowserFileMetadata), BrowserHostError> {
    validate_file_token_text(token)?;
    let canonical_root = root
        .canonicalize()
        .map_err(|_| BrowserHostError::UploadTokenInvalid)?;
    let canonical_file = root
        .join(token)
        .canonicalize()
        .map_err(|_| BrowserHostError::UploadTokenInvalid)?;
    let canonical_metadata = file_metadata_path(root, token)
        .canonicalize()
        .map_err(|_| BrowserHostError::UploadTokenInvalid)?;
    if !canonical_file.starts_with(&canonical_root)
        || !canonical_metadata.starts_with(&canonical_root)
    {
        return Err(BrowserHostError::UploadTokenInvalid);
    }
    let file_metadata = std::fs::symlink_metadata(&canonical_file)
        .map_err(|_| BrowserHostError::UploadTokenInvalid)?;
    let sidecar_metadata = std::fs::symlink_metadata(&canonical_metadata)
        .map_err(|_| BrowserHostError::UploadTokenInvalid)?;
    if !file_metadata.is_file()
        || file_metadata.file_type().is_symlink()
        || file_metadata.len() == 0
        || file_metadata.len() > MAX_BROWSER_FILE_BYTES
        || !sidecar_metadata.is_file()
        || sidecar_metadata.file_type().is_symlink()
        || sidecar_metadata.len() > 16 * 1024
    {
        return Err(BrowserHostError::UploadTokenInvalid);
    }
    let encoded =
        std::fs::read(&canonical_metadata).map_err(|_| BrowserHostError::UploadTokenInvalid)?;
    let metadata: BrowserFileMetadata =
        serde_json::from_slice(&encoded).map_err(|_| BrowserHostError::UploadTokenInvalid)?;
    let now = epoch_ms();
    if metadata.version != 1
        || metadata.session_id != session_id.as_str()
        || metadata.token != token
        || metadata.size != file_metadata.len()
        || metadata.created_at_ms > now.saturating_add(5 * 60 * 1_000)
        || metadata.expires_at_ms <= now
        || metadata.expires_at_ms != metadata.created_at_ms.saturating_add(BROWSER_FILE_TTL_MS)
    {
        return Err(BrowserHostError::UploadTokenInvalid);
    }
    let bytes = std::fs::read(&canonical_file).map_err(|_| BrowserHostError::UploadTokenInvalid)?;
    if metadata.sha256 != hex_digest(&Sha256::digest(bytes)) {
        return Err(BrowserHostError::UploadTokenInvalid);
    }
    Ok((canonical_file, metadata))
}

fn validate_file_token_text(token: &str) -> Result<(), BrowserHostError> {
    (!token.is_empty()
        && token.len() <= 160
        && !token.contains(['/', '\\', ':'])
        && token != "."
        && token != "..")
        .then_some(())
        .ok_or(BrowserHostError::UploadTokenInvalid)
}

fn file_metadata_path(root: &Path, token: &str) -> PathBuf {
    root.join(format!("{token}.hachimi.json"))
}

fn cleanup_expired_file_tokens(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let now = epoch_ms();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.ends_with(".hachimi.json"))
        {
            continue;
        }
        let Ok(encoded) = std::fs::read(&path) else {
            continue;
        };
        let Ok(metadata) = serde_json::from_slice::<BrowserFileMetadata>(&encoded) else {
            continue;
        };
        if metadata.expires_at_ms <= now && validate_file_token_text(&metadata.token).is_ok() {
            let _ = std::fs::remove_file(root.join(metadata.token));
            let _ = std::fs::remove_file(path);
        }
    }
}

fn copy_new(source: &Path, destination: &Path) -> Result<(), BrowserHostError> {
    let mut input =
        std::fs::File::open(source).map_err(|error| BrowserHostError::Broker(error.to_string()))?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
    std::io::copy(&mut input, &mut output)
        .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
    output
        .sync_all()
        .map_err(|error| BrowserHostError::Broker(error.to_string()))
}

fn safe_file_name(path: &Path) -> Result<String, BrowserHostError> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty() && value.chars().count() <= 240)
        .ok_or(BrowserHostError::InvalidInput)?;
    if name.contains(['/', '\\', ':']) || matches!(name, "." | "..") {
        return Err(BrowserHostError::InvalidInput);
    }
    Ok(name.to_owned())
}

fn opaque_file_token(file_name: &str) -> String {
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| {
            value.len() <= 16
                && value
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())
        })
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    format!("{}{}", uuid::Uuid::new_v4(), extension)
}

fn epoch_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|value| i64::try_from(value.as_millis()).ok())
        .unwrap_or_default()
}

fn validate_download_file(
    path: &Path,
    declared_mime: Option<&str>,
    allow_unknown_type: bool,
) -> Result<String, BrowserHostError> {
    let file_name = safe_file_name(path)?;
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    if extension.as_deref().is_some_and(|value| {
        matches!(
            value,
            "exe"
                | "dll"
                | "com"
                | "scr"
                | "msi"
                | "msp"
                | "bat"
                | "cmd"
                | "ps1"
                | "vbs"
                | "vbe"
                | "js"
                | "jse"
                | "wsf"
                | "wsh"
                | "hta"
                | "lnk"
        )
    }) {
        return Err(BrowserHostError::DownloadFailed);
    }
    let bytes = std::fs::read(path).map_err(|error| BrowserHostError::Broker(error.to_string()))?;
    if bytes.is_empty()
        || bytes.starts_with(b"MZ")
        || bytes.starts_with(b"\x7fELF")
        || bytes.starts_with(&[0xfe, 0xed, 0xfa, 0xce])
        || bytes.starts_with(&[0xcf, 0xfa, 0xed, 0xfe])
        || bytes.starts_with(b"#!")
    {
        return Err(BrowserHostError::DownloadFailed);
    }
    let magic_mime = if bytes.starts_with(b"%PDF-") {
        Some("application/pdf")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if std::str::from_utf8(&bytes).is_ok() && !bytes.contains(&0) {
        let trimmed = bytes
            .iter()
            .copied()
            .skip_while(u8::is_ascii_whitespace)
            .collect::<Vec<_>>();
        if matches!(trimmed.first(), Some(b'{') | Some(b'['))
            && serde_json::from_slice::<Value>(&bytes).is_ok()
        {
            Some("application/json")
        } else {
            Some("text/plain")
        }
    } else {
        None
    };
    let extension_mime = match extension.as_deref() {
        Some("pdf") => Some("application/pdf"),
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("json") => Some("application/json"),
        Some("txt" | "md" | "csv") => Some("text/plain"),
        _ => None,
    };
    if let (Some(expected), Some(actual)) = (extension_mime, magic_mime)
        && expected != actual
    {
        return Err(BrowserHostError::DownloadFailed);
    }
    let declared_mime = declared_mime
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or(value)
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|value| !value.is_empty());
    if declared_mime.as_deref().is_some_and(|declared| {
        declared != "application/octet-stream"
            && magic_mime.is_some_and(|actual| actual != declared)
    }) {
        return Err(BrowserHostError::DownloadFailed);
    }
    let Some(mime) = magic_mime else {
        return if allow_unknown_type {
            Ok("application/octet-stream".to_owned())
        } else {
            Err(BrowserHostError::DownloadConfirmationRequired)
        };
    };
    if declared_mime.as_deref() == Some("application/octet-stream") && !allow_unknown_type {
        return Err(BrowserHostError::DownloadConfirmationRequired);
    }
    let _ = file_name;
    Ok(mime.to_owned())
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
#[path = "broker_tests.rs"]
mod tests;
