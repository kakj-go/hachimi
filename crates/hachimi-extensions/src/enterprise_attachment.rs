use std::{fs, io::Read as _, path::Path};

use hachimi_enterprise::{EnterpriseApiClient, EnterpriseApiError, EnterpriseCredential};
use hachimi_protocol::{
    ArtifactId, ArtifactKind, ArtifactRecord, AttachmentId, AttachmentRecord, ConnectorAccountId,
    EnterpriseAttachmentDownloadRequest, EnterpriseAttachmentDownloadResult,
};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::Row;
use zeroize::Zeroize;
use zip::ZipArchive;

use crate::{ExtensionHostError, PluginHost, now_ms};

const MAX_ATTACHMENT_BYTES: u64 = 25 * 1024 * 1024;

impl PluginHost {
    /// Overrides only the enterprise attachment transport while retaining the
    /// same Plugin, account, revision and credential checks. The supplied
    /// client is expected to have enforced its own endpoint policy.
    #[must_use]
    pub fn with_enterprise_api_client(mut self, client: EnterpriseApiClient) -> Self {
        self.enterprise_api = client;
        self
    }

    /// Resolves an enterprise integration identity to its persisted Connector
    /// account binding. Callers must still validate the Connector contribution
    /// revision and allowed action; the model cannot provide this binding.
    pub async fn enterprise_attachment_connector_account(
        &self,
        integration_account_id: &str,
    ) -> Result<Option<ConnectorAccountId>, ExtensionHostError> {
        let row = sqlx::query(
            "SELECT connector_account_id FROM enterprise_integration_accounts WHERE id = ? AND state = 'healthy'",
        )
        .bind(integration_account_id)
        .fetch_optional(self.store.pool())
        .await?;
        Ok(row
            .and_then(|row| row.get::<Option<String>, _>("connector_account_id"))
            .map(ConnectorAccountId::new))
    }

    pub async fn download_enterprise_attachment(
        &self,
        request: &EnterpriseAttachmentDownloadRequest,
    ) -> Result<EnterpriseAttachmentDownloadResult, ExtensionHostError> {
        validate_request(self, request).await?;
        if let Some(result) = completed_result(self, request).await? {
            return Ok(result);
        }
        let metadata = attachment_metadata(self, request).await?;
        claim_download(self, request, &metadata.input_hash).await?;
        let mut raw_credential = load_enterprise_credential(self, request).await?;
        let credential = EnterpriseCredential::parse(&raw_credential)
            .map_err(|_| ExtensionHostError::ConnectorNotHealthy)?;
        raw_credential.zeroize();
        if credential.platform() != request.platform {
            fail_download(
                self,
                request,
                "failed",
                "enterprise_credential_platform_drift",
            )
            .await?;
            return Err(ExtensionHostError::EnterpriseAttachmentDrift);
        }

        let root = self
            .store
            .managed_artifact_root()
            .join("enterprise-attachments");
        let staging_root = root.join(".staging");
        fs::create_dir_all(&staging_root)?;
        let staging = staging_root.join(format!("{}.part", metadata.input_hash));
        if staging.is_file() {
            fs::remove_file(&staging)?;
        }
        let receipt = self
            .enterprise_api
            .download_attachment_to(
                &request.account_id,
                &credential,
                &request.event_id,
                &request.remote_id,
                &staging,
                MAX_ATTACHMENT_BYTES,
            )
            .await;
        let receipt = match receipt {
            Ok(receipt) => receipt,
            Err(error) => {
                let _ = fs::remove_file(&staging);
                let (status, code) = download_error_state(&error);
                fail_download(self, request, status, code).await?;
                return Err(map_download_error(error));
            }
        };
        let mime_type = match detect_allowed_type(
            &staging,
            metadata.file_name.as_deref(),
            metadata.declared_mime.as_deref(),
            receipt.content_type.as_deref(),
        ) {
            Ok(value) => value,
            Err(error) => {
                let _ = fs::remove_file(&staging);
                fail_download(self, request, "failed", "enterprise_attachment_type_denied").await?;
                return Err(error);
            }
        };
        fs::create_dir_all(&root)?;
        let destination = root.join(&receipt.content_hash);
        if destination.is_file() {
            fs::remove_file(&staging)?;
        } else {
            fs::rename(&staging, &destination)?;
        }

        let attachment = self
            .store
            .upsert_attachment(
                &AttachmentRecord {
                    id: AttachmentId::random(),
                    content_hash: receipt.content_hash.clone(),
                    original_name: metadata
                        .file_name
                        .clone()
                        .unwrap_or_else(|| format!("enterprise-{}", request.remote_id)),
                    mime_type: mime_type.clone(),
                    byte_size: receipt.byte_size,
                    created_at_ms: now_ms(),
                },
                &destination,
            )
            .await?;
        self.store
            .attach_to_run(&request.run_id, std::slice::from_ref(&attachment.id))
            .await?;
        let artifact = self
            .store
            .create_artifact(&ArtifactRecord {
                id: ArtifactId::random(),
                run_id: Some(request.run_id.clone()),
                kind: ArtifactKind::EnterpriseAttachment,
                display_name: attachment.original_name.clone(),
                content_hash: Some(receipt.content_hash.clone()),
                metadata: json!({
                    "attachmentId": attachment.id,
                    "platform": request.platform,
                    "accountIdHash": digest_hex(request.account_id.as_bytes()),
                    "eventIdHash": digest_hex(request.event_id.as_bytes()),
                    "remoteIdHash": digest_hex(request.remote_id.as_bytes()),
                    "mimeType": mime_type,
                    "byteSize": receipt.byte_size,
                }),
                created_at_ms: now_ms(),
            })
            .await?;
        let result = EnterpriseAttachmentDownloadResult {
            artifact_id: artifact.id,
            content_hash: receipt.content_hash,
            mime_type,
            byte_size: receipt.byte_size,
            duplicate: false,
        };
        complete_download(self, request, &attachment.id, &result).await?;
        Ok(result)
    }
}

struct AttachmentMetadata {
    file_name: Option<String>,
    declared_mime: Option<String>,
    input_hash: String,
}

async fn validate_request(
    host: &PluginHost,
    request: &EnterpriseAttachmentDownloadRequest,
) -> Result<(), ExtensionHostError> {
    if request.context.protocol_version != hachimi_protocol::CONTROL_PROTOCOL_VERSION
        || request.context.idempotency_key != request.idempotency_key
        || request.context.expected_run_id.as_ref() != Some(&request.run_id)
        || request.context.expected_generation != Some(request.run_generation)
        || request.idempotency_key.is_empty()
        || request.idempotency_key.len() > 128
        || request.account_id.is_empty()
        || request.event_id.is_empty()
        || request.remote_id.is_empty()
        || request.metadata_hash.len() != 64
    {
        return Err(ExtensionHostError::InvalidInvocation);
    }
    let run = host
        .store
        .get_run(&request.run_id)
        .await?
        .ok_or(ExtensionHostError::EnterpriseAttachmentDrift)?;
    if run.generation != request.run_generation {
        return Err(ExtensionHostError::StaleRunGeneration);
    }
    Ok(())
}

async fn attachment_metadata(
    host: &PluginHost,
    request: &EnterpriseAttachmentDownloadRequest,
) -> Result<AttachmentMetadata, ExtensionHostError> {
    let row = sqlx::query("SELECT metadata.file_name, metadata.mime_type, metadata.declared_size_bytes, metadata.metadata_hash, integration.platform FROM enterprise_attachment_metadata AS metadata INNER JOIN enterprise_integration_accounts AS integration ON integration.id = metadata.account_id WHERE metadata.platform = ? AND metadata.account_id = ? AND metadata.event_id = ? AND metadata.remote_id = ?")
        .bind(request.platform.as_str())
        .bind(&request.account_id)
        .bind(&request.event_id)
        .bind(&request.remote_id)
        .fetch_optional(host.store.pool())
        .await?
        .ok_or(ExtensionHostError::EnterpriseAttachmentDrift)?;
    let metadata_hash = row.get::<String, _>("metadata_hash");
    let declared_size = row.get::<Option<i64>, _>("declared_size_bytes");
    let size_error = validate_declared_size(declared_size);
    if metadata_hash != request.metadata_hash
        || row.get::<String, _>("platform") != request.platform.as_str()
        || size_error.is_some()
    {
        return Err(size_error.unwrap_or(ExtensionHostError::EnterpriseAttachmentDrift));
    }
    let input_hash = request_input_hash(request)?;
    Ok(AttachmentMetadata {
        file_name: row.get("file_name"),
        declared_mime: row.get("mime_type"),
        input_hash,
    })
}

fn validate_declared_size(size: Option<i64>) -> Option<ExtensionHostError> {
    match size {
        Some(value) if value < 0 => Some(ExtensionHostError::EnterpriseAttachmentDrift),
        Some(value) if value > MAX_ATTACHMENT_BYTES as i64 => {
            Some(ExtensionHostError::EnterpriseAttachmentTooLarge)
        }
        _ => None,
    }
}

fn request_input_hash(
    request: &EnterpriseAttachmentDownloadRequest,
) -> Result<String, ExtensionHostError> {
    Ok(digest_hex(
        serde_json::to_vec(&json!({
            "platform": request.platform,
            "accountId": request.account_id,
            "eventId": request.event_id,
            "remoteId": request.remote_id,
            "metadataHash": request.metadata_hash,
            "runId": request.run_id,
            "runGeneration": request.run_generation,
        }))?
        .as_slice(),
    ))
}

async fn claim_download(
    host: &PluginHost,
    request: &EnterpriseAttachmentDownloadRequest,
    input_hash: &str,
) -> Result<(), ExtensionHostError> {
    let now = now_ms();
    let mut transaction = host.store.pool().begin().await?;
    let inserted = sqlx::query("INSERT OR IGNORE INTO enterprise_operation_ledger(account_id, idempotency_key, operation, input_hash, status, provider_request_id, provider_result_id, result_json, error_code, created_at_ms, updated_at_ms) VALUES(?, ?, 'download_attachment', ?, 'claimed', NULL, NULL, NULL, NULL, ?, ?)")
        .bind(&request.account_id)
        .bind(&request.idempotency_key)
        .bind(input_hash)
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
    if inserted.rows_affected() == 0 {
        let row = sqlx::query("SELECT input_hash, status FROM enterprise_operation_ledger WHERE account_id = ? AND idempotency_key = ?")
            .bind(&request.account_id)
            .bind(&request.idempotency_key)
            .fetch_one(&mut *transaction)
            .await?;
        if row.get::<String, _>("input_hash") != input_hash {
            return Err(ExtensionHostError::IdempotencyConflict);
        }
        match row.get::<String, _>("status").as_str() {
            "failed" => {
                sqlx::query("UPDATE enterprise_operation_ledger SET status = 'claimed', error_code = NULL, updated_at_ms = ? WHERE account_id = ? AND idempotency_key = ? AND status = 'failed'")
                    .bind(now)
                    .bind(&request.account_id)
                    .bind(&request.idempotency_key)
                    .execute(&mut *transaction)
                    .await?;
            }
            "completed" => return Err(ExtensionHostError::EnterpriseAttachmentDrift),
            "claimed" | "indeterminate" => {
                return Err(ExtensionHostError::EnterpriseIndeterminate);
            }
            _ => return Err(ExtensionHostError::EnterpriseTransport),
        }
    }
    sqlx::query("UPDATE enterprise_attachment_metadata SET download_status = 'downloading' WHERE platform = ? AND account_id = ? AND event_id = ? AND remote_id = ?")
        .bind(request.platform.as_str())
        .bind(&request.account_id)
        .bind(&request.event_id)
        .bind(&request.remote_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

async fn completed_result(
    host: &PluginHost,
    request: &EnterpriseAttachmentDownloadRequest,
) -> Result<Option<EnterpriseAttachmentDownloadResult>, ExtensionHostError> {
    let row = sqlx::query("SELECT ledger.input_hash, ledger.result_json, metadata.metadata_hash FROM enterprise_operation_ledger AS ledger INNER JOIN enterprise_attachment_metadata AS metadata ON metadata.account_id = ledger.account_id WHERE ledger.account_id = ? AND ledger.idempotency_key = ? AND ledger.operation = 'download_attachment' AND ledger.status = 'completed' AND metadata.platform = ? AND metadata.event_id = ? AND metadata.remote_id = ?")
        .bind(&request.account_id)
        .bind(&request.idempotency_key)
        .bind(request.platform.as_str())
        .bind(&request.event_id)
        .bind(&request.remote_id)
        .fetch_optional(host.store.pool())
        .await?;
    let Some(row) = row else { return Ok(None) };
    if row.get::<String, _>("metadata_hash") != request.metadata_hash
        || row.get::<String, _>("input_hash") != request_input_hash(request)?
    {
        return Err(ExtensionHostError::IdempotencyConflict);
    }
    let mut result: EnterpriseAttachmentDownloadResult =
        serde_json::from_str(row.get("result_json"))?;
    result.duplicate = true;
    Ok(Some(result))
}

async fn load_enterprise_credential(
    host: &PluginHost,
    request: &EnterpriseAttachmentDownloadRequest,
) -> Result<String, ExtensionHostError> {
    let row = sqlx::query("SELECT connector_account_id, channel_account_id FROM enterprise_integration_accounts WHERE id = ? AND platform = ?")
        .bind(&request.account_id)
        .bind(request.platform.as_str())
        .fetch_optional(host.store.pool())
        .await?
        .ok_or(ExtensionHostError::ConnectorNotHealthy)?;
    let entry = if let Some(account_id) = row.get::<Option<String>, _>("connector_account_id") {
        keyring::Entry::new("com.hachimi.connector", &account_id)
    } else if let Some(account_id) = row.get::<Option<String>, _>("channel_account_id") {
        keyring::Entry::new(
            "com.hachimi.channel",
            &format!("{}:{account_id}", request.platform.channel_provider_id()),
        )
    } else {
        return Err(ExtensionHostError::ConnectorNotHealthy);
    }
    .map_err(|_| ExtensionHostError::SecretStore)?;
    entry
        .get_password()
        .map_err(|_| ExtensionHostError::SecretStore)
}

async fn complete_download(
    host: &PluginHost,
    request: &EnterpriseAttachmentDownloadRequest,
    attachment_id: &AttachmentId,
    result: &EnterpriseAttachmentDownloadResult,
) -> Result<(), ExtensionHostError> {
    let mut transaction = host.store.pool().begin().await?;
    sqlx::query("UPDATE enterprise_attachment_metadata SET download_status = 'completed', content_hash = ?, detected_mime_type = ?, downloaded_size_bytes = ?, managed_attachment_id = ?, artifact_id = ? WHERE platform = ? AND account_id = ? AND event_id = ? AND remote_id = ? AND download_status = 'downloading'")
        .bind(&result.content_hash)
        .bind(&result.mime_type)
        .bind(i64::try_from(result.byte_size).unwrap_or(i64::MAX))
        .bind(attachment_id.as_str())
        .bind(result.artifact_id.as_str())
        .bind(request.platform.as_str())
        .bind(&request.account_id)
        .bind(&request.event_id)
        .bind(&request.remote_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("UPDATE enterprise_operation_ledger SET status = 'completed', provider_result_id = ?, result_json = ?, error_code = NULL, updated_at_ms = ? WHERE account_id = ? AND idempotency_key = ? AND status = 'claimed'")
        .bind(result.artifact_id.as_str())
        .bind(serde_json::to_string(result)?)
        .bind(now_ms())
        .bind(&request.account_id)
        .bind(&request.idempotency_key)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

async fn fail_download(
    host: &PluginHost,
    request: &EnterpriseAttachmentDownloadRequest,
    status: &str,
    code: &str,
) -> Result<(), ExtensionHostError> {
    let mut transaction = host.store.pool().begin().await?;
    sqlx::query("UPDATE enterprise_attachment_metadata SET download_status = ? WHERE platform = ? AND account_id = ? AND event_id = ? AND remote_id = ? AND download_status = 'downloading'")
        .bind(status)
        .bind(request.platform.as_str())
        .bind(&request.account_id)
        .bind(&request.event_id)
        .bind(&request.remote_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("UPDATE enterprise_operation_ledger SET status = ?, error_code = ?, updated_at_ms = ? WHERE account_id = ? AND idempotency_key = ? AND status = 'claimed'")
        .bind(status)
        .bind(code)
        .bind(now_ms())
        .bind(&request.account_id)
        .bind(&request.idempotency_key)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

fn detect_allowed_type(
    path: &Path,
    file_name: Option<&str>,
    declared_mime: Option<&str>,
    response_mime: Option<&str>,
) -> Result<String, ExtensionHostError> {
    let mut file = fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let detected = if bytes.starts_with(b"%PDF-") {
        "application/pdf"
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else if bytes.starts_with(b"PK\x03\x04") {
        detect_office_zip(path)?
    } else {
        detect_text(&bytes, file_name, declared_mime)?
    };
    let extension_mime = file_name.and_then(extension_mime);
    let has_extension = file_name.is_some_and(|value| Path::new(value).extension().is_some());
    if (has_extension && extension_mime.is_none())
        || extension_mime.is_some_and(|value| value != detected)
        || declared_mime
            .and_then(normalize_mime)
            .is_some_and(|value| value != detected)
        || response_mime
            .and_then(normalize_mime)
            .is_some_and(|value| value != detected)
    {
        return Err(ExtensionHostError::EnterpriseAttachmentTypeDenied);
    }
    Ok(detected.into())
}

fn detect_office_zip(path: &Path) -> Result<&'static str, ExtensionHostError> {
    let file = fs::File::open(path)?;
    let mut archive =
        ZipArchive::new(file).map_err(|_| ExtensionHostError::EnterpriseAttachmentTypeDenied)?;
    if archive.len() > 10_000 {
        return Err(ExtensionHostError::EnterpriseAttachmentTypeDenied);
    }
    let mut word = false;
    let mut sheet = false;
    let mut slides = false;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|_| ExtensionHostError::EnterpriseAttachmentTypeDenied)?;
        let name = entry.name();
        word |= name.starts_with("word/");
        sheet |= name.starts_with("xl/");
        slides |= name.starts_with("ppt/");
    }
    match (word, sheet, slides) {
        (true, false, false) => {
            Ok("application/vnd.openxmlformats-officedocument.wordprocessingml.document")
        }
        (false, true, false) => {
            Ok("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet")
        }
        (false, false, true) => {
            Ok("application/vnd.openxmlformats-officedocument.presentationml.presentation")
        }
        _ => Err(ExtensionHostError::EnterpriseAttachmentTypeDenied),
    }
}

fn detect_text(
    bytes: &[u8],
    file_name: Option<&str>,
    declared_mime: Option<&str>,
) -> Result<&'static str, ExtensionHostError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| ExtensionHostError::EnterpriseAttachmentTypeDenied)?;
    let trimmed = text.trim_start().to_ascii_lowercase();
    if trimmed.starts_with("<!doctype html")
        || trimmed.starts_with("<html")
        || trimmed.starts_with("<script")
        || trimmed.contains("<object")
        || trimmed.contains("<embed")
    {
        return Err(ExtensionHostError::EnterpriseAttachmentTypeDenied);
    }
    let json_expected = file_name
        .and_then(|value| {
            Path::new(value)
                .extension()
                .and_then(|value| value.to_str())
        })
        .is_some_and(|value| value.eq_ignore_ascii_case("json"))
        || declared_mime.and_then(normalize_mime) == Some("application/json");
    if json_expected {
        serde_json::from_str::<Value>(text)
            .map_err(|_| ExtensionHostError::EnterpriseAttachmentTypeDenied)?;
        Ok("application/json")
    } else {
        Ok("text/plain")
    }
}

fn extension_mime(file_name: &str) -> Option<&'static str> {
    match Path::new(file_name)
        .extension()?
        .to_str()?
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => Some("application/pdf"),
        "txt" | "md" | "csv" | "tsv" | "log" => Some("text/plain"),
        "json" => Some("application/json"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "docx" => Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
        "xlsx" => Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
        "pptx" => Some("application/vnd.openxmlformats-officedocument.presentationml.presentation"),
        _ => None,
    }
}

fn normalize_mime(value: &str) -> Option<&str> {
    match value
        .split(';')
        .next()?
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "application/octet-stream" => None,
        "image/jpg" => Some("image/jpeg"),
        "application/pdf" => Some("application/pdf"),
        "text/plain" | "text/csv" | "text/markdown" => Some("text/plain"),
        "application/json" | "text/json" => Some("application/json"),
        "image/png" => Some("image/png"),
        "image/jpeg" => Some("image/jpeg"),
        "image/gif" => Some("image/gif"),
        "image/webp" => Some("image/webp"),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
            Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document")
        }
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => {
            Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet")
        }
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => {
            Some("application/vnd.openxmlformats-officedocument.presentationml.presentation")
        }
        _ => Some("__denied__"),
    }
}

fn download_error_state(error: &EnterpriseApiError) -> (&'static str, &'static str) {
    match error {
        EnterpriseApiError::Transport | EnterpriseApiError::Indeterminate => {
            ("indeterminate", error.code())
        }
        _ => ("failed", error.code()),
    }
}

fn map_download_error(error: EnterpriseApiError) -> ExtensionHostError {
    match error {
        EnterpriseApiError::InvalidRequest => ExtensionHostError::EnterpriseAttachmentTooLarge,
        EnterpriseApiError::Authentication | EnterpriseApiError::InvalidCredential => {
            ExtensionHostError::ConnectorNotHealthy
        }
        EnterpriseApiError::RateLimited { .. } => ExtensionHostError::RateLimited,
        EnterpriseApiError::Provider { code, .. } => ExtensionHostError::EnterpriseProvider(code),
        EnterpriseApiError::Transport | EnterpriseApiError::Indeterminate => {
            ExtensionHostError::EnterpriseIndeterminate
        }
        EnterpriseApiError::MalformedResponse => ExtensionHostError::EnterpriseTransport,
    }
}

fn digest_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hachimi_protocol::{ClientId, MutationContext, RequestId, RunId};

    fn attachment_request(generation: u64) -> EnterpriseAttachmentDownloadRequest {
        let run_id = RunId::new("run-attachment");
        EnterpriseAttachmentDownloadRequest {
            context: MutationContext {
                request_id: RequestId("request-download-1".into()),
                client_id: ClientId("client-download-1".into()),
                protocol_version: hachimi_protocol::CONTROL_PROTOCOL_VERSION,
                idempotency_key: "download-1".into(),
                expected_run_id: Some(run_id.clone()),
                expected_generation: Some(generation),
            },
            platform: hachimi_protocol::EnterprisePlatform::DingTalk,
            account_id: "account-1".into(),
            event_id: "event-1".into(),
            remote_id: "remote-1".into(),
            metadata_hash: "a".repeat(64),
            run_id,
            run_generation: generation,
            idempotency_key: "download-1".into(),
        }
    }

    #[test]
    fn attachment_idempotency_hash_fences_run_generation() {
        let first = attachment_request(1);
        let second = attachment_request(2);
        assert_ne!(
            request_input_hash(&first).expect("first hash"),
            request_input_hash(&second).expect("second hash")
        );
        assert_eq!(
            ExtensionHostError::StaleRunGeneration.code(),
            "stale_run_generation"
        );
    }

    #[test]
    fn attachment_size_limit_is_exactly_twenty_five_mib() {
        assert!(validate_declared_size(Some(MAX_ATTACHMENT_BYTES as i64)).is_none());
        assert!(matches!(
            validate_declared_size(Some(MAX_ATTACHMENT_BYTES as i64 + 1)),
            Some(ExtensionHostError::EnterpriseAttachmentTooLarge)
        ));
        assert!(matches!(
            validate_declared_size(Some(-1)),
            Some(ExtensionHostError::EnterpriseAttachmentDrift)
        ));
    }

    #[test]
    fn interrupted_attachment_download_is_indeterminate_and_not_retryable() {
        assert_eq!(
            download_error_state(&EnterpriseApiError::Transport),
            ("indeterminate", "enterprise_transport_failed")
        );
        assert_eq!(
            download_error_state(&EnterpriseApiError::Indeterminate),
            ("indeterminate", "enterprise_outcome_indeterminate")
        );
        assert_eq!(
            map_download_error(EnterpriseApiError::Indeterminate).code(),
            "enterprise_outcome_indeterminate"
        );
    }

    #[test]
    fn attachment_magic_and_metadata_must_agree() {
        let root = tempfile::tempdir().expect("root");
        let pdf = root.path().join("fixture.pdf");
        fs::write(&pdf, b"%PDF-1.7\nfixture").expect("pdf");
        assert_eq!(
            detect_allowed_type(&pdf, Some("fixture.pdf"), Some("application/pdf"), None)
                .expect("allowed"),
            "application/pdf"
        );
        assert!(matches!(
            detect_allowed_type(&pdf, Some("fixture.png"), Some("image/png"), None),
            Err(ExtensionHostError::EnterpriseAttachmentTypeDenied)
        ));
    }

    #[test]
    fn html_executables_and_unknown_binary_are_rejected() {
        let root = tempfile::tempdir().expect("root");
        for (name, body) in [
            ("fixture.html", b"<!doctype html><p>no</p>".as_slice()),
            ("fixture.exe", b"MZ\0\0binary".as_slice()),
        ] {
            let path = root.path().join(name);
            fs::write(&path, body).expect("fixture");
            assert!(detect_allowed_type(&path, Some(name), None, None).is_err());
        }
    }
}
