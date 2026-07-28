// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex/codex-rs/core/src/turn_diff_tracker.rs
// @ 4c43465133428898aa84f0bfc02c306ed65fb66a.
// Modified for Hachimi: persisted Run baselines and indexed, chunk-readable Diff artifacts.

use std::io::{Read, Seek, SeekFrom};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use hachimi_protocol::{
    ArtifactId, CheckoutId, DiffReadFileResponse, DiffScope, RunDiffSnapshot, RunId, SessionId,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use super::{AgentStore, AgentStoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunFileBaselineRecord {
    pub run_id: RunId,
    pub path_key: String,
    pub display_path: String,
    pub baseline_hash: Option<String>,
    pub baseline_artifact_id: Option<ArtifactId>,
    pub previous_path: Option<String>,
    pub baseline_mode: Option<String>,
    pub baseline_size: Option<u64>,
    pub baseline_binary: bool,
    pub current_hash: Option<String>,
    pub current_mode: Option<String>,
    pub current_size: Option<u64>,
    pub current_binary: bool,
    pub change_kind: Option<String>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Copy)]
pub struct ManagedRunDiffFile<'a> {
    pub path: &'a str,
    pub content: &'a [u8],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagedRunDiffIndex {
    format: String,
    byte_size: u64,
    files: Vec<ManagedRunDiffIndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagedRunDiffIndexEntry {
    path: String,
    offset: u64,
    byte_size: u64,
    sha256: String,
}

const RUN_DIFF_INDEX_FORMAT: &str = "hachimi-run-diff-index-v1";
const DEFAULT_DIFF_CHUNK_BYTES: u32 = 256 * 1024;
const MAX_DIFF_CHUNK_BYTES: u32 = 1024 * 1024;

impl AgentStore {
    // The baseline mirrors the persisted row and keeps content/hash metadata explicit at the
    // storage boundary; callers must not be able to omit one field through a partial map.
    #[allow(clippy::too_many_arguments)]
    pub async fn capture_run_file_baseline(
        &self,
        run_id: &RunId,
        path_key: &str,
        display_path: &str,
        content: Option<&[u8]>,
        baseline_hash: Option<&str>,
        previous_path: Option<&str>,
        baseline_mode: Option<&str>,
        baseline_size: Option<u64>,
        baseline_binary: bool,
        updated_at_ms: i64,
    ) -> Result<RunFileBaselineRecord, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        if let Some(row) =
            sqlx::query("SELECT * FROM run_file_baselines WHERE run_id = ? AND path_key = ?")
                .bind(run_id.as_str())
                .bind(path_key)
                .fetch_optional(&mut *transaction)
                .await?
        {
            transaction.commit().await?;
            return baseline_from_row(&row);
        }

        let artifact_id = content.map(|_| ArtifactId::new(Uuid::now_v7().to_string()));
        let managed_path = artifact_id.as_ref().map(|artifact_id| {
            self.managed_artifacts
                .path
                .join("file-baselines")
                .join(format!("{}.bin", artifact_id.as_str()))
        });
        if let (Some(content), Some(path), Some(artifact_id)) =
            (content, managed_path.as_ref(), artifact_id.as_ref())
        {
            let parent = path.parent().ok_or(AgentStoreError::InvalidPath)?;
            std::fs::create_dir_all(parent)?;
            let temporary = path.with_extension("bin.tmp");
            std::fs::write(&temporary, content)?;
            std::fs::rename(&temporary, path)?;
            sqlx::query(
                "INSERT INTO artifacts (id, run_id, kind, display_name, content_hash, managed_path, metadata_json, created_at_ms) VALUES (?, ?, 'file_baseline', ?, ?, ?, '{}', ?)",
            )
            .bind(artifact_id.as_str())
            .bind(run_id.as_str())
            .bind(format!("Run baseline: {display_path}"))
            .bind(baseline_hash)
            .bind(path.to_string_lossy().as_ref())
            .bind(updated_at_ms)
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query(
            "INSERT INTO run_file_baselines (run_id, path_key, display_path, baseline_hash, baseline_artifact_id, previous_path, baseline_mode, baseline_size, baseline_binary, current_hash, current_mode, current_size, current_binary, change_kind, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(run_id.as_str())
        .bind(path_key)
        .bind(display_path)
        .bind(baseline_hash)
        .bind(artifact_id.as_ref().map(ArtifactId::as_str))
        .bind(previous_path)
        .bind(baseline_mode)
        .bind(baseline_size.and_then(|value| i64::try_from(value).ok()))
        .bind(baseline_binary)
        .bind(baseline_hash)
        .bind(baseline_mode)
        .bind(baseline_size.and_then(|value| i64::try_from(value).ok()))
        .bind(baseline_binary)
        .bind("unchanged")
        .bind(updated_at_ms)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(RunFileBaselineRecord {
            run_id: run_id.clone(),
            path_key: path_key.to_owned(),
            display_path: display_path.to_owned(),
            baseline_hash: baseline_hash.map(str::to_owned),
            baseline_artifact_id: artifact_id,
            previous_path: previous_path.map(str::to_owned),
            baseline_mode: baseline_mode.map(str::to_owned),
            baseline_size,
            baseline_binary,
            current_hash: baseline_hash.map(str::to_owned),
            current_mode: baseline_mode.map(str::to_owned),
            current_size: baseline_size,
            current_binary: baseline_binary,
            change_kind: Some("unchanged".into()),
            updated_at_ms,
        })
    }

    pub async fn read_run_file_baseline(
        &self,
        artifact_id: &ArtifactId,
    ) -> Result<Vec<u8>, AgentStoreError> {
        let path = sqlx::query_scalar::<_, String>(
            "SELECT managed_path FROM artifacts WHERE id = ? AND kind = 'file_baseline'",
        )
        .bind(artifact_id.as_str())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AgentStoreError::InvalidPath)?;
        let path = std::path::PathBuf::from(path);
        let root = self.managed_artifacts.path.join("file-baselines");
        let expected = root.join(format!("{}.bin", artifact_id.as_str()));
        if path != expected || path.parent() != Some(root.as_path()) {
            return Err(AgentStoreError::InvalidPath);
        }
        Ok(std::fs::read(path)?)
    }

    pub async fn create_managed_run_diff_artifact(
        &self,
        run_id: &RunId,
        files: &[ManagedRunDiffFile<'_>],
        created_at_ms: i64,
    ) -> Result<ArtifactId, AgentStoreError> {
        let mut content = Vec::new();
        let mut entries = Vec::with_capacity(files.len());
        for file in files {
            if file.path.is_empty()
                || entries
                    .iter()
                    .any(|entry: &ManagedRunDiffIndexEntry| entry.path == file.path)
            {
                return Err(AgentStoreError::InvalidPath);
            }
            let offset = u64::try_from(content.len()).map_err(|_| AgentStoreError::InvalidPath)?;
            content.extend_from_slice(file.content);
            entries.push(ManagedRunDiffIndexEntry {
                path: file.path.to_owned(),
                offset,
                byte_size: u64::try_from(file.content.len())
                    .map_err(|_| AgentStoreError::InvalidPath)?,
                sha256: hex_digest(file.content),
            });
        }
        let byte_size = u64::try_from(content.len()).map_err(|_| AgentStoreError::InvalidPath)?;
        let content_hash = hex_digest(&content);
        let index = ManagedRunDiffIndex {
            format: RUN_DIFF_INDEX_FORMAT.into(),
            byte_size,
            files: entries,
        };
        let artifact_id = ArtifactId::new(Uuid::now_v7().to_string());
        let root = self.managed_artifacts.path.join("run-diffs");
        std::fs::create_dir_all(&root)?;
        let path = root.join(format!("{}.diff", artifact_id.as_str()));
        let temporary = path.with_extension("diff.tmp");
        std::fs::write(&temporary, &content)?;
        std::fs::rename(&temporary, &path)?;
        sqlx::query(
            "INSERT INTO artifacts (id, run_id, kind, display_name, content_hash, managed_path, metadata_json, created_at_ms) VALUES (?, ?, 'diff_evidence', 'Full run diff', ?, ?, ?, ?)",
        )
        .bind(artifact_id.as_str())
        .bind(run_id.as_str())
        .bind(&content_hash)
        .bind(path.to_string_lossy().as_ref())
        .bind(serde_json::to_string(&index)?)
        .bind(created_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(artifact_id)
    }

    pub async fn read_managed_run_diff_file_chunk(
        &self,
        run_id: &RunId,
        artifact_id: &ArtifactId,
        path: &str,
        offset: u64,
        limit: u32,
        if_match: Option<&str>,
    ) -> Result<DiffReadFileResponse, AgentStoreError> {
        let row = sqlx::query(
            "SELECT run_id, content_hash, managed_path, metadata_json FROM artifacts WHERE id = ? AND kind = 'diff_evidence'",
        )
        .bind(artifact_id.as_str())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AgentStoreError::InvalidPath)?;
        if row.get::<Option<String>, _>("run_id").as_deref() != Some(run_id.as_str()) {
            return Err(AgentStoreError::InvalidPath);
        }
        let stored_hash = row
            .get::<Option<String>, _>("content_hash")
            .ok_or(AgentStoreError::InvalidPath)?;
        let index: ManagedRunDiffIndex =
            serde_json::from_str(&row.get::<String, _>("metadata_json"))?;
        if index.format != RUN_DIFF_INDEX_FORMAT {
            return Err(AgentStoreError::InvalidPath);
        }
        let entry = index
            .files
            .iter()
            .find(|entry| entry.path == path)
            .ok_or(AgentStoreError::InvalidPath)?;
        if offset > entry.byte_size {
            return Err(AgentStoreError::InvalidPath);
        }
        let root = self.managed_artifacts.path.join("run-diffs");
        let expected = root.join(format!("{}.diff", artifact_id.as_str()));
        let stored_path = std::path::PathBuf::from(row.get::<String, _>("managed_path"));
        if stored_path != expected || stored_path.parent() != Some(root.as_path()) {
            return Err(AgentStoreError::InvalidPath);
        }
        let mut file = std::fs::File::open(&stored_path)?;
        let metadata = file.metadata()?;
        if metadata.len() != index.byte_size
            || entry
                .offset
                .checked_add(entry.byte_size)
                .is_none_or(|end| end > metadata.len())
        {
            return Err(AgentStoreError::InvalidPath);
        }
        let actual_hash = hash_open_file(&mut file)?;
        if actual_hash != stored_hash {
            return Err(AgentStoreError::InvalidPath);
        }
        let etag = format!("sha256:{}", entry.sha256);
        if if_match.is_some_and(|expected| expected != etag) {
            return Err(AgentStoreError::InvalidPath);
        }
        let limit = if limit == 0 {
            DEFAULT_DIFF_CHUNK_BYTES
        } else {
            limit.clamp(1, MAX_DIFF_CHUNK_BYTES)
        };
        file.seek(SeekFrom::Start(entry.offset.saturating_add(offset)))?;
        let remaining = entry.byte_size.saturating_sub(offset);
        let requested = usize::try_from(remaining.min(u64::from(limit)))
            .map_err(|_| AgentStoreError::InvalidPath)?;
        let mut bytes = vec![0_u8; requested];
        file.read_exact(&mut bytes)?;
        while !bytes.is_empty() && std::str::from_utf8(&bytes).is_err() {
            bytes.pop();
        }
        if requested > 0 && bytes.is_empty() {
            return Err(AgentStoreError::InvalidPath);
        }
        let next_offset = offset
            .saturating_add(u64::try_from(bytes.len()).map_err(|_| AgentStoreError::InvalidPath)?);
        Ok(DiffReadFileResponse {
            scope: DiffScope::Run {
                run_id: run_id.clone(),
            },
            path: path.to_owned(),
            offset,
            next_offset,
            byte_size: entry.byte_size,
            eof: next_offset >= entry.byte_size,
            data_base64: STANDARD.encode(&bytes),
            utf8_text: String::from_utf8(bytes).ok(),
            etag,
        })
    }

    pub async fn remove_managed_run_diff_artifact(
        &self,
        artifact_id: &ArtifactId,
    ) -> Result<(), AgentStoreError> {
        let path = sqlx::query_scalar::<_, String>(
            "SELECT managed_path FROM artifacts WHERE id = ? AND kind = 'diff_evidence'",
        )
        .bind(artifact_id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        sqlx::query("DELETE FROM artifacts WHERE id = ? AND kind = 'diff_evidence'")
            .bind(artifact_id.as_str())
            .execute(&self.pool)
            .await?;
        if let Some(path) = path {
            let path = std::path::PathBuf::from(path);
            let root = self.managed_artifacts.path.join("run-diffs");
            let expected = root.join(format!("{}.diff", artifact_id.as_str()));
            if path == expected && path.parent() == Some(root.as_path()) {
                match std::fs::remove_file(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(())
    }

    pub async fn upsert_run_file_baseline(
        &self,
        baseline: &RunFileBaselineRecord,
    ) -> Result<(), AgentStoreError> {
        sqlx::query(
            "INSERT INTO run_file_baselines (run_id, path_key, display_path, baseline_hash, baseline_artifact_id, previous_path, baseline_mode, baseline_size, baseline_binary, current_hash, current_mode, current_size, current_binary, change_kind, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(run_id, path_key) DO UPDATE SET display_path = excluded.display_path, previous_path = COALESCE(excluded.previous_path, run_file_baselines.previous_path), current_hash = excluded.current_hash, current_mode = excluded.current_mode, current_size = excluded.current_size, current_binary = excluded.current_binary, change_kind = excluded.change_kind, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(baseline.run_id.as_str())
        .bind(&baseline.path_key)
        .bind(&baseline.display_path)
        .bind(&baseline.baseline_hash)
        .bind(
            baseline
                .baseline_artifact_id
                .as_ref()
                .map(ArtifactId::as_str),
        )
        .bind(&baseline.previous_path)
        .bind(&baseline.baseline_mode)
        .bind(baseline.baseline_size.and_then(|value| i64::try_from(value).ok()))
        .bind(baseline.baseline_binary)
        .bind(&baseline.current_hash)
        .bind(&baseline.current_mode)
        .bind(baseline.current_size.and_then(|value| i64::try_from(value).ok()))
        .bind(baseline.current_binary)
        .bind(&baseline.change_kind)
        .bind(baseline.updated_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_run_file_baselines(
        &self,
        run_id: &RunId,
    ) -> Result<Vec<RunFileBaselineRecord>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM run_file_baselines WHERE run_id = ? ORDER BY display_path ASC",
        )
        .bind(run_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(baseline_from_row).collect()
    }

    pub async fn move_run_file_baseline(
        &self,
        run_id: &RunId,
        old_path_key: &str,
        new_path_key: &str,
        new_display_path: &str,
        previous_path: &str,
        updated_at_ms: i64,
    ) -> Result<bool, AgentStoreError> {
        let result = sqlx::query(
            "UPDATE run_file_baselines SET path_key = ?, display_path = ?, previous_path = COALESCE(previous_path, ?), updated_at_ms = ? WHERE run_id = ? AND path_key = ? AND NOT EXISTS (SELECT 1 FROM run_file_baselines existing WHERE existing.run_id = ? AND existing.path_key = ?)",
        )
        .bind(new_path_key)
        .bind(new_display_path)
        .bind(previous_path)
        .bind(updated_at_ms)
        .bind(run_id.as_str())
        .bind(old_path_key)
        .bind(run_id.as_str())
        .bind(new_path_key)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn put_run_diff_manifest(
        &self,
        run_id: &RunId,
        checkout_id: &CheckoutId,
        snapshot: &RunDiffSnapshot,
    ) -> Result<(), AgentStoreError> {
        sqlx::query(
            "INSERT INTO run_diff_manifests (run_id, checkout_id, snapshot_json, artifact_id, generated_at_ms) VALUES (?, ?, ?, ?, ?) ON CONFLICT(run_id) DO UPDATE SET checkout_id = excluded.checkout_id, snapshot_json = excluded.snapshot_json, artifact_id = excluded.artifact_id, generated_at_ms = excluded.generated_at_ms",
        )
        .bind(run_id.as_str())
        .bind(checkout_id.as_str())
        .bind(serde_json::to_string(snapshot)?)
        .bind(snapshot.artifact_id.as_ref().map(ArtifactId::as_str))
        .bind(snapshot.generated_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn put_run_diff_manifest_if_current(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        expected_generation: u64,
        checkout_id: &CheckoutId,
        snapshot: &RunDiffSnapshot,
    ) -> Result<(), AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query("SELECT session_id, generation, status FROM runs WHERE id = ?")
            .bind(run_id.as_str())
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| AgentStoreError::RunNotFound(run_id.clone()))?;
        let generation = u64::try_from(row.get::<i64, _>("generation")).unwrap_or(u64::MAX);
        let status = row.get::<String, _>("status");
        let persisted_session = row.get::<String, _>("session_id");
        if generation != expected_generation
            || persisted_session != session_id.as_str()
            || matches!(
                status.as_str(),
                "cancelling" | "succeeded" | "failed" | "cancelled" | "interrupted" | "lost"
            )
        {
            return Err(AgentStoreError::RunPreconditionFailed);
        }
        sqlx::query(
            "INSERT INTO run_diff_manifests (run_id, checkout_id, snapshot_json, artifact_id, generated_at_ms) VALUES (?, ?, ?, ?, ?) ON CONFLICT(run_id) DO UPDATE SET checkout_id = excluded.checkout_id, snapshot_json = excluded.snapshot_json, artifact_id = excluded.artifact_id, generated_at_ms = excluded.generated_at_ms",
        )
        .bind(run_id.as_str())
        .bind(checkout_id.as_str())
        .bind(serde_json::to_string(snapshot)?)
        .bind(snapshot.artifact_id.as_ref().map(ArtifactId::as_str))
        .bind(snapshot.generated_at_ms)
        .execute(&mut *transaction)
        .await?;
        super::append_event_tx(
            &mut transaction,
            session_id,
            Some(run_id),
            "run.diff.updated",
            serde_json::json!({
                "fileCount": snapshot.files.len(),
                "truncated": snapshot.truncated,
                "artifactId": snapshot.artifact_id,
            }),
            snapshot.generated_at_ms,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn get_run_diff_manifest(
        &self,
        run_id: &RunId,
    ) -> Result<Option<RunDiffSnapshot>, AgentStoreError> {
        let value = sqlx::query_scalar::<_, String>(
            "SELECT snapshot_json FROM run_diff_manifests WHERE run_id = ?",
        )
        .bind(run_id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        value
            .map(|encoded| serde_json::from_str(&encoded))
            .transpose()
            .map_err(AgentStoreError::from)
    }
}

fn baseline_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<RunFileBaselineRecord, AgentStoreError> {
    Ok(RunFileBaselineRecord {
        run_id: RunId::new(row.get::<String, _>("run_id")),
        path_key: row.get("path_key"),
        display_path: row.get("display_path"),
        baseline_hash: row.get("baseline_hash"),
        baseline_artifact_id: row
            .get::<Option<String>, _>("baseline_artifact_id")
            .map(ArtifactId::new),
        previous_path: row.get("previous_path"),
        baseline_mode: row.get("baseline_mode"),
        baseline_size: row
            .get::<Option<i64>, _>("baseline_size")
            .and_then(|value| u64::try_from(value).ok()),
        baseline_binary: row.get("baseline_binary"),
        current_hash: row.get("current_hash"),
        current_mode: row.get("current_mode"),
        current_size: row
            .get::<Option<i64>, _>("current_size")
            .and_then(|value| u64::try_from(value).ok()),
        current_binary: row.get("current_binary"),
        change_kind: row.get("change_kind"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

fn hash_open_file(file: &mut std::fs::File) -> Result<String, AgentStoreError> {
    file.seek(SeekFrom::Start(0))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(encode_hex(&digest.finalize()))
}

fn hex_digest(bytes: &[u8]) -> String {
    encode_hex(&Sha256::digest(bytes))
}

fn encode_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}
