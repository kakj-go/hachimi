// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex/codex-rs/core/src/turn_diff_tracker.rs
// @ 4c43465133428898aa84f0bfc02c306ed65fb66a.
// Modified for Hachimi: Checkout Worker reads, SQLite baselines, and indexed Diff artifacts.
//! Run-scoped Diff tracking based on immutable pre-side-effect file baselines.

use std::{collections::HashMap, sync::Arc, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use hachimi_protocol::{
    CheckoutId, DiffHunk, DiffLine, DiffScope, FileDiffStatus, FileDiffSummary, RunDiffSnapshot,
    RunId, SessionId,
};
use hachimi_storage::{AgentStore, ManagedRunDiffFile, RunFileBaselineRecord};
use hachimi_workspace::{
    GitBlob, GitStatusEntry, WorkspaceErrorCode, WorkspaceHostClient, WorkspaceOperation,
    WorkspaceOutput,
};
use tokio_util::sync::CancellationToken;

const BASELINE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RENDERED_DIFF_LINES: usize = 10_000;
const MAX_DIFF_MATRIX_CELLS: usize = 5_000_000;
const MAX_BASELINE_BYTES: u64 = 8 * 1024 * 1024;
const BASELINE_CHUNK_BYTES: u32 = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct RunDiffTracker {
    store: AgentStore,
    client: Arc<WorkspaceHostClient>,
    session_id: SessionId,
    run_id: RunId,
    run_generation: u64,
    checkout_id: CheckoutId,
}

#[derive(Debug, Clone, Default)]
struct FileMaterialization {
    exists: bool,
    content: Option<Vec<u8>>,
    hash: Option<String>,
    mode: Option<String>,
    size: Option<u64>,
    binary: bool,
}

impl RunDiffTracker {
    #[must_use]
    pub fn new(
        store: AgentStore,
        client: Arc<WorkspaceHostClient>,
        session_id: SessionId,
        run_id: RunId,
        checkout_id: CheckoutId,
    ) -> Self {
        let run_generation = client.run_generation();
        Self {
            store,
            client,
            session_id,
            run_id,
            run_generation,
            checkout_id,
        }
    }

    pub async fn capture_before_write(
        &self,
        path: &str,
        cancellation: CancellationToken,
    ) -> Result<(), String> {
        if self.has_baseline(path).await? {
            return Ok(());
        }
        let baseline = self.read_current(path, None, cancellation).await?;
        self.capture_materialized_baseline(path, None, baseline)
            .await
    }

    pub async fn capture_before_move(
        &self,
        source: &str,
        destination: &str,
        cancellation: CancellationToken,
    ) -> Result<(), String> {
        if self.has_baseline(destination).await? {
            return Err(format!(
                "run baseline already exists for move destination '{destination}'"
            ));
        }
        if self
            .store
            .move_run_file_baseline(
                &self.run_id,
                &normalized_path_key(source),
                &normalized_path_key(destination),
                destination,
                source,
                now_ms(),
            )
            .await
            .map_err(|error| error.to_string())?
        {
            return Ok(());
        }
        let baseline = self.read_current(source, None, cancellation).await?;
        if !baseline.exists {
            return Err(format!("move source '{source}' does not exist"));
        }
        self.capture_materialized_baseline(destination, Some(source), baseline)
            .await
    }

    /// Captures files that were already dirty before an Exec. Files that were clean are restored
    /// from HEAD after Exec, so an existing user change never becomes part of the Run Diff.
    pub async fn capture_before_exec(&self, cancellation: CancellationToken) -> Result<(), String> {
        for entry in self.git_status(cancellation.child_token()).await? {
            if cancellation.is_cancelled() {
                return Err("run baseline capture was cancelled".into());
            }
            self.capture_before_write(&entry.path, cancellation.child_token())
                .await?;
        }
        Ok(())
    }

    pub async fn record_write_and_refresh(
        &self,
        path: &str,
        _current_hash: &str,
        cancellation: &CancellationToken,
    ) -> Result<(), String> {
        if cancellation.is_cancelled() {
            return Err("run diff update was cancelled".into());
        }
        if !self.has_baseline(path).await? {
            return Err("run baseline was not captured before the write".into());
        }
        self.refresh(cancellation).await
    }

    pub async fn record_exec_and_refresh(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<(), String> {
        let status = self.git_status(cancellation.child_token()).await?;
        for entry in &status {
            if cancellation.is_cancelled() {
                return Err("run diff update was cancelled".into());
            }
            if self.has_baseline(&entry.path).await? {
                continue;
            }
            if let Some(previous_path) = entry.previous_path.as_deref()
                && self.has_baseline(previous_path).await?
                && self
                    .store
                    .move_run_file_baseline(
                        &self.run_id,
                        &normalized_path_key(previous_path),
                        &normalized_path_key(&entry.path),
                        &entry.path,
                        previous_path,
                        now_ms(),
                    )
                    .await
                    .map_err(|error| error.to_string())?
            {
                continue;
            }
            let previous_path = entry.previous_path.as_deref();
            let baseline = if status_is_added(entry) {
                FileMaterialization::default()
            } else {
                self.read_head_blob(
                    previous_path.unwrap_or(&entry.path),
                    cancellation.child_token(),
                )
                .await?
            };
            self.capture_materialized_baseline(&entry.path, previous_path, baseline)
                .await?;
        }
        self.refresh_with_status(&status, cancellation).await
    }

    pub async fn refresh(&self, cancellation: &CancellationToken) -> Result<(), String> {
        let status = self.git_status(cancellation.child_token()).await?;
        self.refresh_with_status(&status, cancellation).await
    }

    async fn refresh_with_status(
        &self,
        status: &[GitStatusEntry],
        cancellation: &CancellationToken,
    ) -> Result<(), String> {
        let status_by_path = status
            .iter()
            .map(|entry| (normalized_path_key(&entry.path), entry))
            .collect::<HashMap<_, _>>();
        let baselines = self
            .store
            .list_run_file_baselines(&self.run_id)
            .await
            .map_err(|error| error.to_string())?;
        let mut files = Vec::new();
        let mut full_diff_files = Vec::<(String, Vec<u8>)>::new();
        for baseline in baselines {
            if cancellation.is_cancelled() {
                return Err("run diff refresh was cancelled".into());
            }
            let previous = self.materialize_baseline(&baseline).await?;
            let status = status_by_path.get(&baseline.path_key).copied();
            let current = self
                .read_current(&baseline.display_path, status, cancellation.child_token())
                .await?;
            if same_file(&previous, &current) {
                continue;
            }
            let file = build_file_diff(
                &baseline.display_path,
                baseline.previous_path.as_deref(),
                &previous,
                &current,
            );
            if file.too_large && !file.binary {
                let mut rendered = String::new();
                append_full_replacement_diff(
                    &mut rendered,
                    &baseline.display_path,
                    previous.content.as_deref(),
                    current.content.as_deref(),
                );
                full_diff_files.push((baseline.display_path.clone(), rendered.into_bytes()));
            }
            files.push(file);
        }
        if cancellation.is_cancelled() {
            return Err("run diff persistence was cancelled".into());
        }
        let artifact_id = if full_diff_files.is_empty() {
            None
        } else {
            let indexed_files = full_diff_files
                .iter()
                .map(|(path, content)| ManagedRunDiffFile { path, content })
                .collect::<Vec<_>>();
            Some(
                self.store
                    .create_managed_run_diff_artifact(&self.run_id, &indexed_files, now_ms())
                    .await
                    .map_err(|error| error.to_string())?,
            )
        };
        if cancellation.is_cancelled() {
            if let Some(artifact_id) = &artifact_id {
                let _ = self
                    .store
                    .remove_managed_run_diff_artifact(artifact_id)
                    .await;
            }
            return Err("run diff persistence was cancelled".into());
        }
        let snapshot = RunDiffSnapshot {
            scope: DiffScope::Run {
                run_id: self.run_id.clone(),
            },
            truncated: files.iter().any(|file| file.too_large),
            files,
            artifact_id,
            generated_at_ms: now_ms(),
        };
        let persistence = self.store.put_run_diff_manifest_if_current(
            &self.session_id,
            &self.run_id,
            self.run_generation,
            &self.checkout_id,
            &snapshot,
        );
        tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                if let Some(artifact_id) = &snapshot.artifact_id {
                    let _ = self.store.remove_managed_run_diff_artifact(artifact_id).await;
                }
                Err("run diff persistence was cancelled".into())
            }
            result = persistence => result.map_err(|error| error.to_string()),
        }
    }

    async fn has_baseline(&self, path: &str) -> Result<bool, String> {
        let key = normalized_path_key(path);
        Ok(self
            .store
            .list_run_file_baselines(&self.run_id)
            .await
            .map_err(|error| error.to_string())?
            .iter()
            .any(|baseline| baseline.path_key == key))
    }

    async fn capture_materialized_baseline(
        &self,
        path: &str,
        previous_path: Option<&str>,
        baseline: FileMaterialization,
    ) -> Result<(), String> {
        self.store
            .capture_run_file_baseline(
                &self.run_id,
                &normalized_path_key(path),
                path,
                baseline.content.as_deref(),
                baseline.hash.as_deref(),
                previous_path,
                baseline.mode.as_deref(),
                baseline.size,
                baseline.binary,
                now_ms(),
            )
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    async fn materialize_baseline(
        &self,
        baseline: &RunFileBaselineRecord,
    ) -> Result<FileMaterialization, String> {
        let content = match baseline.baseline_artifact_id.as_ref() {
            Some(artifact_id) => Some(
                self.store
                    .read_run_file_baseline(artifact_id)
                    .await
                    .map_err(|error| error.to_string())?,
            ),
            None => None,
        };
        Ok(FileMaterialization {
            exists: baseline.baseline_hash.is_some(),
            content,
            hash: baseline.baseline_hash.clone(),
            mode: baseline.baseline_mode.clone(),
            size: baseline.baseline_size,
            binary: baseline.baseline_binary,
        })
    }

    async fn git_status(
        &self,
        cancellation: CancellationToken,
    ) -> Result<Vec<GitStatusEntry>, String> {
        match self
            .client
            .execute(
                WorkspaceOperation::GitStatusSnapshot,
                BASELINE_TIMEOUT,
                cancellation,
            )
            .await
        {
            Ok(WorkspaceOutput::GitStatusSnapshot { entries }) => Ok(entries),
            Ok(_) => Err("Git status host returned an unexpected output".into()),
            Err(error) => Err(format!("Git status snapshot failed: {}", error.message)),
        }
    }

    async fn read_head_blob(
        &self,
        path: &str,
        cancellation: CancellationToken,
    ) -> Result<FileMaterialization, String> {
        match self
            .client
            .execute(
                WorkspaceOperation::ReadGitBlob {
                    path: path.to_owned(),
                },
                BASELINE_TIMEOUT,
                cancellation,
            )
            .await
        {
            Ok(WorkspaceOutput::GitBlob { blob }) => materialize_git_blob(blob),
            Err(error) if error.code == WorkspaceErrorCode::NotFound => {
                Ok(FileMaterialization::default())
            }
            Ok(_) => Err("Git blob host returned an unexpected output".into()),
            Err(error) => Err(format!("Git baseline blob failed: {}", error.message)),
        }
    }

    async fn read_current(
        &self,
        path: &str,
        status: Option<&GitStatusEntry>,
        cancellation: CancellationToken,
    ) -> Result<FileMaterialization, String> {
        let first = self
            .client
            .execute(
                WorkspaceOperation::ReadFileChunk {
                    path: path.to_owned(),
                    offset: 0,
                    limit: BASELINE_CHUNK_BYTES,
                    if_match: None,
                },
                BASELINE_TIMEOUT,
                cancellation.child_token(),
            )
            .await;
        let chunk = match first {
            Ok(WorkspaceOutput::FileChunk { chunk }) => chunk,
            Err(error) if error.code == WorkspaceErrorCode::NotFound => {
                return Ok(FileMaterialization::default());
            }
            Err(error) if error.code == WorkspaceErrorCode::PathOutsideCheckout => {
                return status.map_or_else(
                    || {
                        Err(format!(
                            "run diff path inspection failed: {}",
                            error.message
                        ))
                    },
                    |entry| {
                        Ok(FileMaterialization {
                            exists: entry.current_hash.is_some(),
                            content: None,
                            hash: entry.current_hash.clone(),
                            mode: entry.current_mode.clone(),
                            size: entry.current_size,
                            binary: entry.current_binary,
                        })
                    },
                );
            }
            Ok(_) => return Err("run diff host returned an unexpected output".into()),
            Err(error) => return Err(format!("run diff read failed: {}", error.message)),
        };
        let hash = chunk
            .etag
            .strip_prefix("sha256:")
            .unwrap_or(&chunk.etag)
            .to_owned();
        if chunk.byte_size > MAX_BASELINE_BYTES {
            return Ok(FileMaterialization {
                exists: true,
                content: None,
                hash: Some(hash),
                mode: status.and_then(|entry| entry.current_mode.clone()),
                size: Some(chunk.byte_size),
                binary: chunk.binary,
            });
        }
        let mut bytes = STANDARD
            .decode(&chunk.data_base64)
            .map_err(|error| format!("invalid baseline chunk: {error}"))?;
        let mut next_offset = chunk.next_offset;
        let etag = chunk.etag;
        let mut binary = chunk.binary;
        while next_offset < chunk.byte_size {
            if cancellation.is_cancelled() {
                return Err("run diff read was cancelled".into());
            }
            let output = self
                .client
                .execute(
                    WorkspaceOperation::ReadFileChunk {
                        path: path.to_owned(),
                        offset: next_offset,
                        limit: BASELINE_CHUNK_BYTES,
                        if_match: Some(etag.clone()),
                    },
                    BASELINE_TIMEOUT,
                    cancellation.child_token(),
                )
                .await
                .map_err(|error| format!("run diff read failed: {}", error.message))?;
            let WorkspaceOutput::FileChunk { chunk } = output else {
                return Err("run diff host returned an unexpected output".into());
            };
            binary |= chunk.binary;
            bytes.extend(
                STANDARD
                    .decode(&chunk.data_base64)
                    .map_err(|error| format!("invalid baseline chunk: {error}"))?,
            );
            next_offset = chunk.next_offset;
        }
        Ok(FileMaterialization {
            exists: true,
            content: Some(bytes),
            hash: Some(hash),
            mode: status.and_then(|entry| entry.current_mode.clone()),
            size: Some(chunk.byte_size),
            binary,
        })
    }
}

fn materialize_git_blob(blob: GitBlob) -> Result<FileMaterialization, String> {
    let content = if blob.byte_size > MAX_BASELINE_BYTES {
        None
    } else {
        Some(
            STANDARD
                .decode(blob.data_base64)
                .map_err(|error| format!("invalid Git baseline blob: {error}"))?,
        )
    };
    Ok(FileMaterialization {
        exists: true,
        content,
        hash: Some(blob.sha256),
        mode: Some(blob.mode),
        size: Some(blob.byte_size),
        binary: blob.binary,
    })
}

fn same_file(previous: &FileMaterialization, current: &FileMaterialization) -> bool {
    previous.exists == current.exists
        && previous.hash == current.hash
        && (previous.mode.is_none() || current.mode.is_none() || previous.mode == current.mode)
}

fn status_is_added(entry: &GitStatusEntry) -> bool {
    matches!(entry.index_status, '?' | 'A') || matches!(entry.worktree_status, '?' | 'A')
}

fn append_full_replacement_diff(
    output: &mut String,
    path: &str,
    previous: Option<&[u8]>,
    current: Option<&[u8]>,
) {
    let old = previous
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .unwrap_or_default();
    let new = current
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .unwrap_or_default();
    output.push_str(&format!(
        "--- a/{path}\n+++ b/{path}\n@@ -1,{} +1,{} @@\n",
        old.lines().count(),
        new.lines().count()
    ));
    for line in old.lines() {
        output.push('-');
        output.push_str(line);
        output.push('\n');
    }
    for line in new.lines() {
        output.push('+');
        output.push_str(line);
        output.push('\n');
    }
}

fn build_file_diff(
    path: &str,
    previous_path: Option<&str>,
    previous: &FileMaterialization,
    current: &FileMaterialization,
) -> FileDiffSummary {
    let binary = previous.binary || current.binary;
    let type_changed = previous.exists
        && current.exists
        && previous.mode.is_some()
        && current.mode.is_some()
        && previous.mode != current.mode;
    let status = if previous_path.is_some() {
        FileDiffStatus::Renamed
    } else if !previous.exists && current.exists {
        FileDiffStatus::Added
    } else if previous.exists && !current.exists {
        FileDiffStatus::Deleted
    } else if type_changed {
        FileDiffStatus::TypeChanged
    } else if binary {
        FileDiffStatus::Binary
    } else {
        FileDiffStatus::Modified
    };
    if binary
        || previous.content.is_none() && previous.exists
        || current.content.is_none() && current.exists
    {
        return FileDiffSummary {
            path: path.to_owned(),
            previous_path: previous_path.map(str::to_owned),
            status,
            additions: 0,
            deletions: 0,
            binary,
            too_large: !binary,
            hunks: Vec::new(),
        };
    }
    let old = previous
        .content
        .as_deref()
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .unwrap_or_default();
    let new = current
        .content
        .as_deref()
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .unwrap_or_default();
    let mut rendered = build_text_diff(path, old, new, status);
    rendered.previous_path = previous_path.map(str::to_owned);
    rendered
}

fn build_text_diff(path: &str, old: &str, new: &str, status: FileDiffStatus) -> FileDiffSummary {
    let old_lines = old.lines().count();
    let new_lines = new.lines().count();
    let too_large = old_lines.saturating_mul(new_lines) > MAX_DIFF_MATRIX_CELLS
        || old_lines.saturating_add(new_lines) > MAX_RENDERED_DIFF_LINES;
    if too_large {
        return FileDiffSummary {
            path: path.to_owned(),
            previous_path: None,
            status,
            additions: u32::try_from(new_lines).unwrap_or(u32::MAX),
            deletions: u32::try_from(old_lines).unwrap_or(u32::MAX),
            binary: false,
            too_large: true,
            hunks: Vec::new(),
        };
    }
    let mut old_line = 1_u32;
    let mut new_line = 1_u32;
    let mut additions = 0_u32;
    let mut deletions = 0_u32;
    let mut lines = Vec::new();
    for change in diff::lines(old, new) {
        match change {
            diff::Result::Left(text) => {
                deletions = deletions.saturating_add(1);
                lines.push(DiffLine {
                    kind: "deletion".into(),
                    old_line: Some(old_line),
                    new_line: None,
                    text: text.to_owned(),
                });
                old_line = old_line.saturating_add(1);
            }
            diff::Result::Right(text) => {
                additions = additions.saturating_add(1);
                lines.push(DiffLine {
                    kind: "addition".into(),
                    old_line: None,
                    new_line: Some(new_line),
                    text: text.to_owned(),
                });
                new_line = new_line.saturating_add(1);
            }
            diff::Result::Both(text, _) => {
                lines.push(DiffLine {
                    kind: "context".into(),
                    old_line: Some(old_line),
                    new_line: Some(new_line),
                    text: text.to_owned(),
                });
                old_line = old_line.saturating_add(1);
                new_line = new_line.saturating_add(1);
            }
        }
    }
    FileDiffSummary {
        path: path.to_owned(),
        previous_path: None,
        status,
        additions,
        deletions,
        binary: false,
        too_large: false,
        hunks: vec![DiffHunk {
            header: format!("@@ -1,{old_lines} +1,{new_lines} @@"),
            lines,
        }],
    }
}

fn normalized_path_key(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
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

    fn materialized(value: Option<&[u8]>, mode: &str) -> FileMaterialization {
        FileMaterialization {
            exists: value.is_some(),
            content: value.map(<[u8]>::to_vec),
            hash: value.map(|value| format!("{}", value.len())),
            mode: Some(mode.into()),
            size: value.map(|value| value.len() as u64),
            binary: value.is_some_and(|value| value.contains(&0)),
        }
    }

    #[test]
    fn run_diff_is_relative_to_the_pre_write_content() {
        let file = build_file_diff(
            "demo.txt",
            None,
            &materialized(Some(b"dirty before\nkeep\n"), "100644"),
            &materialized(Some(b"agent\nkeep\n"), "100644"),
        );
        assert_eq!(file.status, FileDiffStatus::Modified);
        assert_eq!(file.additions, 1);
        assert_eq!(file.deletions, 1);
        assert!(file.hunks[0].lines.iter().any(|line| line.text == "agent"));
    }

    #[test]
    fn binary_rename_and_mode_change_are_typed() {
        let binary = build_file_diff(
            "new.bin",
            Some("old.bin"),
            &materialized(Some(b"\0old"), "100644"),
            &materialized(Some(b"\0new"), "100644"),
        );
        assert_eq!(binary.status, FileDiffStatus::Renamed);
        assert!(binary.binary);
        let mode = build_file_diff(
            "script.sh",
            None,
            &materialized(Some(b"echo ok\n"), "100644"),
            &materialized(Some(b"echo ok\n"), "100755"),
        );
        assert_eq!(mode.status, FileDiffStatus::TypeChanged);
    }
}
