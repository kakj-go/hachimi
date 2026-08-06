// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex/codex-rs/core/src/turn_diff_tracker.rs and
// openai/codex/codex-rs/app-server-protocol/src/protocol/v2/fs.rs
// @ 4c43465133428898aa84f0bfc02c306ed65fb66a.
// Modified for Hachimi: Checkout-bound Git baselines and byte-bounded per-file Diff reads.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    process::Stdio,
    time::UNIX_EPOCH,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use hachimi_process_policy::{ProcessPolicy, tokio_command};
use hachimi_protocol::{
    DiffHunk, DiffLine, DiffReadFileResponse, DiffScope, FileDiffStatus, FileDiffSummary,
    RunDiffSnapshot,
};
use tokio::{io::AsyncReadExt, process::Command};

use sha2::{Digest, Sha256};

use crate::{
    GitBlob, GitStatusEntry, WorkerContext, WorkspaceError, WorkspaceErrorCode, WorkspaceOutput,
};

const MAX_DIFF_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_DIFF_CHUNK_BYTES: u32 = 256 * 1024;
const MAX_DIFF_CHUNK_BYTES: u32 = 1024 * 1024;

impl WorkerContext {
    pub(crate) async fn git_status_snapshot(&self) -> Result<WorkspaceOutput, WorkspaceError> {
        let output = self
            .git_output(&[
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--ignored=no",
            ])
            .await?;
        let records = output.split(|byte| *byte == 0).collect::<Vec<_>>();
        let mut index = 0_usize;
        let mut entries = Vec::new();
        while index < records.len() {
            let record = records[index];
            index += 1;
            if record.is_empty() {
                continue;
            }
            if record.len() < 4 || record[2] != b' ' {
                return Err(WorkspaceError::new(
                    WorkspaceErrorCode::ProcessFailed,
                    "Git returned an invalid porcelain status record",
                ));
            }
            let index_status = char::from(record[0]);
            let worktree_status = char::from(record[1]);
            let path = String::from_utf8(record[3..].to_vec()).map_err(|_| {
                WorkspaceError::new(
                    WorkspaceErrorCode::NotText,
                    "Git status path is not valid UTF-8",
                )
            })?;
            let previous_path =
                if matches!(index_status, 'R' | 'C') || matches!(worktree_status, 'R' | 'C') {
                    let previous = records.get(index).ok_or_else(|| {
                        WorkspaceError::new(
                            WorkspaceErrorCode::ProcessFailed,
                            "Git rename status omitted its previous path",
                        )
                    })?;
                    index += 1;
                    Some(String::from_utf8((*previous).to_vec()).map_err(|_| {
                        WorkspaceError::new(
                            WorkspaceErrorCode::NotText,
                            "Git previous path is not valid UTF-8",
                        )
                    })?)
                } else {
                    None
                };
            let fingerprint = fingerprint_path(&self.root.join(&path))?;
            entries.push(GitStatusEntry {
                index_status,
                worktree_status,
                path,
                previous_path,
                current_hash: fingerprint.as_ref().map(|value| value.hash.clone()),
                current_size: fingerprint.as_ref().map(|value| value.size),
                current_binary: fingerprint.as_ref().is_some_and(|value| value.binary),
                current_mode: fingerprint.as_ref().map(|value| value.mode.clone()),
                current_kind: fingerprint.map(|value| value.kind),
            });
        }
        Ok(WorkspaceOutput::GitStatusSnapshot { entries })
    }

    pub(crate) async fn read_git_blob(
        &self,
        path: &str,
    ) -> Result<WorkspaceOutput, WorkspaceError> {
        let resolved = self.resolve_write(path)?;
        let relative = crate::relative_display(&self.root, &resolved);
        let tree = self
            .git_output(&["ls-tree", "-z", "HEAD", "--", relative.as_str()])
            .await?;
        let record = tree.split(|byte| *byte == 0).next().unwrap_or_default();
        let separator = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| {
                WorkspaceError::new(
                    WorkspaceErrorCode::NotFound,
                    "path has no blob in the Run baseline revision",
                )
            })?;
        let metadata = &record[..separator];
        let metadata = std::str::from_utf8(metadata).map_err(|_| {
            WorkspaceError::new(
                WorkspaceErrorCode::ProcessFailed,
                "invalid Git tree metadata",
            )
        })?;
        let mut fields = metadata.split_whitespace();
        let mode = fields.next().unwrap_or_default();
        let kind = fields.next().unwrap_or_default();
        let object_id = fields.next().unwrap_or_default();
        if kind != "blob" || object_id.len() < 40 {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::NotFound,
                "path has no file blob in the Run baseline revision",
            ));
        }
        let bytes = self.git_output(&["cat-file", "blob", object_id]).await?;
        let binary = bytes.contains(&0) || std::str::from_utf8(&bytes).is_err();
        Ok(WorkspaceOutput::GitBlob {
            blob: GitBlob {
                path: relative,
                data_base64: STANDARD.encode(&bytes),
                sha256: hex_digest(&bytes),
                byte_size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                binary,
                mode: mode.to_owned(),
            },
        })
    }

    async fn git_output(&self, arguments: &[&str]) -> Result<Vec<u8>, WorkspaceError> {
        let mut command = tokio_command(crate::git_program(), ProcessPolicy::HiddenCaptured);
        command
            .args(arguments)
            .current_dir(crate::restricted_process_cwd(&self.root))
            .env_clear()
            .env("GIT_OPTIONAL_LOCKS", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        crate::copy_process_environment(&mut command);
        let output = command.output().await.map_err(|error| {
            WorkspaceError::new(WorkspaceErrorCode::ProcessFailed, error.to_string())
        })?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(WorkspaceError::new(
                WorkspaceErrorCode::ProcessFailed,
                String::from_utf8_lossy(&output.stderr)
                    .chars()
                    .take(1_000)
                    .collect::<String>(),
            ))
        }
    }

    pub(crate) async fn git_diff_structured(
        &self,
        scope: DiffScope,
        base_revision: Option<&str>,
    ) -> Result<WorkspaceOutput, WorkspaceError> {
        let mut command = tokio_command(crate::git_program(), ProcessPolicy::HiddenCaptured);
        command
            .args([
                "diff",
                "--no-textconv",
                "--no-ext-diff",
                "--no-color",
                "--find-renames",
                "--unified=3",
            ])
            .current_dir(crate::restricted_process_cwd(&self.root))
            .env_clear()
            .env("GIT_OPTIONAL_LOCKS", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        crate::copy_process_environment(&mut command);
        if let Some(base_revision) = base_revision {
            if base_revision.is_empty()
                || base_revision.starts_with('-')
                || base_revision.chars().count() > 512
            {
                return Err(WorkspaceError::new(
                    WorkspaceErrorCode::InvalidRequest,
                    "invalid Git diff base revision",
                ));
            }
            command.arg(base_revision);
        }
        command.arg("--");
        let output = command.output().await.map_err(|error| {
            WorkspaceError::new(WorkspaceErrorCode::ProcessFailed, error.to_string())
        })?;
        if !output.status.success() {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::ProcessFailed,
                String::from_utf8_lossy(&output.stderr)
                    .chars()
                    .take(1_000)
                    .collect::<String>(),
            ));
        }
        let mut truncated = output.stdout.len() > MAX_DIFF_BYTES;
        let bytes = &output.stdout[..output.stdout.len().min(MAX_DIFF_BYTES)];
        let text = String::from_utf8_lossy(bytes);
        let mut files = parse_unified_diff(&text, truncated);
        if !matches!(&scope, DiffScope::Branch { .. }) {
            let mut inline_budget = MAX_DIFF_BYTES.saturating_sub(bytes.len());
            for path in self.untracked_paths().await? {
                let summary = summarize_untracked(&self.root, &path, &mut inline_budget)?;
                truncated |= summary.too_large;
                files.push(summary);
            }
        }
        Ok(WorkspaceOutput::Diff {
            snapshot: RunDiffSnapshot {
                scope,
                files,
                artifact_id: None,
                truncated,
                generated_at_ms: now_ms(),
            },
        })
    }

    pub(crate) async fn git_diff_file_chunk(
        &self,
        scope: DiffScope,
        path: &str,
        base_revision: Option<&str>,
        offset: u64,
        limit: u32,
        if_match: Option<&str>,
    ) -> Result<WorkspaceOutput, WorkspaceError> {
        let checkout_id = match &scope {
            DiffScope::Checkout { checkout_id }
            | DiffScope::Session { checkout_id, .. }
            | DiffScope::Branch { checkout_id, .. } => checkout_id,
            DiffScope::Run { .. } => {
                return Err(WorkspaceError::new(
                    WorkspaceErrorCode::InvalidRequest,
                    "Workspace Host cannot materialize Run Diff files",
                ));
            }
        };
        if checkout_id.as_str() != self.checkout_id {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::Unauthorized,
                "Diff checkout does not match the Worker checkout binding",
            ));
        }
        let resolved = self.resolve_write(path)?;
        let relative = crate::relative_display(&self.root, &resolved);
        validate_base_revision(base_revision)?;
        let limit = if limit == 0 {
            DEFAULT_DIFF_CHUNK_BYTES
        } else {
            limit.clamp(1, MAX_DIFF_CHUNK_BYTES)
        };

        if !matches!(&scope, DiffScope::Branch { .. }) && self.is_untracked(&relative).await? {
            let chunk =
                untracked_diff_file_chunk(scope, &resolved, &relative, offset, limit, if_match)?;
            return Ok(WorkspaceOutput::DiffFileChunk { chunk });
        }

        let mut command = self.git_diff_command(base_revision);
        command.arg("--").arg(&relative);
        let mut child = command.spawn().map_err(|error| {
            WorkspaceError::new(WorkspaceErrorCode::ProcessFailed, error.to_string())
        })?;
        let mut stdout = child.stdout.take().ok_or_else(|| {
            WorkspaceError::new(
                WorkspaceErrorCode::ProcessFailed,
                "Git stdout is unavailable",
            )
        })?;
        let mut stderr = child.stderr.take().ok_or_else(|| {
            WorkspaceError::new(
                WorkspaceErrorCode::ProcessFailed,
                "Git stderr is unavailable",
            )
        })?;
        let stderr_task = tokio::spawn(async move {
            let mut kept = Vec::new();
            let mut buffer = [0_u8; 8 * 1024];
            loop {
                let read = stderr.read(&mut buffer).await?;
                if read == 0 {
                    break;
                }
                let remaining = 64 * 1024_usize - kept.len().min(64 * 1024);
                kept.extend_from_slice(&buffer[..read.min(remaining)]);
            }
            Ok::<Vec<u8>, std::io::Error>(kept)
        });
        let requested_end = offset.saturating_add(u64::from(limit));
        let mut digest = Sha256::new();
        let mut total = 0_u64;
        let mut bytes = Vec::with_capacity(usize::try_from(limit).unwrap_or_default());
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = stdout.read(&mut buffer).await.map_err(crate::io_error)?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
            let start = total;
            let end = total.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
            if end > offset && start < requested_end {
                let local_start = usize::try_from(offset.saturating_sub(start)).unwrap_or_default();
                let local_end = usize::try_from(requested_end.min(end).saturating_sub(start))
                    .unwrap_or(read)
                    .min(read);
                bytes.extend_from_slice(&buffer[local_start.min(read)..local_end]);
            }
            total = end;
        }
        let status = child.wait().await.map_err(crate::io_error)?;
        let stderr = stderr_task
            .await
            .map_err(|error| {
                WorkspaceError::new(WorkspaceErrorCode::ProcessFailed, error.to_string())
            })?
            .map_err(crate::io_error)?;
        if !status.success() {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::ProcessFailed,
                String::from_utf8_lossy(&stderr)
                    .chars()
                    .take(1_000)
                    .collect::<String>(),
            ));
        }
        if offset > total {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::InvalidRequest,
                "Diff chunk offset exceeds the file Diff size",
            ));
        }
        let etag = format!("sha256:{}", encode_hex(&digest.finalize()));
        if if_match.is_some_and(|expected| expected != etag) {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::Conflict,
                "file Diff changed while it was being read",
            ));
        }
        let next_offset = offset.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        let utf8_text = String::from_utf8(bytes.clone()).ok();
        Ok(WorkspaceOutput::DiffFileChunk {
            chunk: DiffReadFileResponse {
                scope,
                path: relative,
                offset,
                next_offset,
                byte_size: total,
                eof: next_offset >= total,
                data_base64: STANDARD.encode(bytes),
                utf8_text,
                etag,
            },
        })
    }

    fn git_diff_command(&self, base_revision: Option<&str>) -> Command {
        let mut command = tokio_command(crate::git_program(), ProcessPolicy::HiddenCaptured);
        command
            .args([
                "diff",
                "--no-textconv",
                "--no-ext-diff",
                "--no-color",
                "--find-renames",
                "--unified=3",
            ])
            .current_dir(crate::restricted_process_cwd(&self.root))
            .env_clear()
            .env("GIT_OPTIONAL_LOCKS", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        crate::copy_process_environment(&mut command);
        if let Some(base_revision) = base_revision {
            command.arg(base_revision);
        }
        command
    }

    async fn untracked_paths(&self) -> Result<Vec<String>, WorkspaceError> {
        let output = self
            .git_output(&["ls-files", "--others", "--exclude-standard", "-z"])
            .await?;
        output
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| {
                String::from_utf8(path.to_vec()).map_err(|_| {
                    WorkspaceError::new(
                        WorkspaceErrorCode::NotText,
                        "Git untracked path is not valid UTF-8",
                    )
                })
            })
            .collect()
    }

    async fn is_untracked(&self, path: &str) -> Result<bool, WorkspaceError> {
        let output = self
            .git_output(&[
                "ls-files",
                "--others",
                "--exclude-standard",
                "-z",
                "--",
                path,
            ])
            .await?;
        Ok(output
            .split(|byte| *byte == 0)
            .any(|candidate| candidate == path.as_bytes()))
    }
}

fn summarize_untracked(
    root: &std::path::Path,
    path: &str,
    inline_budget: &mut usize,
) -> Result<FileDiffSummary, WorkspaceError> {
    let resolved = root.join(path);
    let fingerprint = fingerprint_path(&resolved)?.ok_or_else(|| {
        WorkspaceError::new(
            WorkspaceErrorCode::NotFound,
            "untracked file disappeared while building Diff",
        )
    })?;
    if fingerprint.kind != "file" && fingerprint.kind != "symlink" {
        return Ok(FileDiffSummary {
            path: path.to_owned(),
            previous_path: None,
            status: FileDiffStatus::Added,
            additions: 0,
            deletions: 0,
            binary: false,
            too_large: false,
            hunks: Vec::new(),
        });
    }
    if fingerprint.binary {
        return Ok(FileDiffSummary {
            path: path.to_owned(),
            previous_path: None,
            status: FileDiffStatus::Binary,
            additions: 0,
            deletions: 0,
            binary: true,
            too_large: false,
            hunks: Vec::new(),
        });
    }
    let byte_size = usize::try_from(fingerprint.size).unwrap_or(usize::MAX);
    if byte_size > *inline_budget {
        return Ok(FileDiffSummary {
            path: path.to_owned(),
            previous_path: None,
            status: FileDiffStatus::Added,
            additions: 0,
            deletions: 0,
            binary: false,
            too_large: true,
            hunks: Vec::new(),
        });
    }
    let bytes = if fingerprint.kind == "symlink" {
        std::fs::read_link(&resolved)
            .map_err(crate::io_error)?
            .to_string_lossy()
            .into_owned()
            .into_bytes()
    } else {
        std::fs::read(&resolved).map_err(crate::io_error)?
    };
    *inline_budget = (*inline_budget).saturating_sub(bytes.len());
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        WorkspaceError::new(
            WorkspaceErrorCode::NotText,
            "untracked file changed to binary while building Diff",
        )
    })?;
    let lines = text
        .lines()
        .enumerate()
        .map(|(index, text)| DiffLine {
            kind: "addition".into(),
            old_line: None,
            new_line: Some(u32::try_from(index + 1).unwrap_or(u32::MAX)),
            text: text.to_owned(),
        })
        .collect::<Vec<_>>();
    let additions = u32::try_from(lines.len()).unwrap_or(u32::MAX);
    let hunks = (!lines.is_empty())
        .then(|| DiffHunk {
            header: format!("@@ -0,0 +1,{additions} @@"),
            lines,
        })
        .into_iter()
        .collect();
    Ok(FileDiffSummary {
        path: path.to_owned(),
        previous_path: None,
        status: FileDiffStatus::Added,
        additions,
        deletions: 0,
        binary: false,
        too_large: false,
        hunks,
    })
}

fn untracked_diff_file_chunk(
    scope: DiffScope,
    resolved: &std::path::Path,
    relative: &str,
    offset: u64,
    limit: u32,
    if_match: Option<&str>,
) -> Result<DiffReadFileResponse, WorkspaceError> {
    let fingerprint = fingerprint_path(resolved)?.ok_or_else(|| {
        WorkspaceError::new(
            WorkspaceErrorCode::NotFound,
            "untracked file no longer exists",
        )
    })?;
    if fingerprint.binary || (fingerprint.kind != "file" && fingerprint.kind != "symlink") {
        return Err(WorkspaceError::new(
            WorkspaceErrorCode::NotText,
            "untracked Diff is not a regular text file",
        ));
    }
    let mut accumulator = DiffChunkAccumulator::new(offset, limit);
    accumulator.feed(
        format!(
            "diff --git a/{relative} b/{relative}\nnew file mode {}\n--- /dev/null\n+++ b/{relative}\n@@ -0,0 +1 @@\n",
            fingerprint.mode
        )
        .as_bytes(),
    );
    if fingerprint.kind == "symlink" {
        let bytes = std::fs::read_link(resolved)
            .map_err(crate::io_error)?
            .to_string_lossy()
            .into_owned()
            .into_bytes();
        feed_added_text(&mut accumulator, &bytes);
    } else {
        let mut file = File::open(resolved).map_err(crate::io_error)?;
        file.seek(SeekFrom::Start(0)).map_err(crate::io_error)?;
        let mut buffer = [0_u8; 64 * 1024];
        let mut line_start = true;
        loop {
            let read = file.read(&mut buffer).map_err(crate::io_error)?;
            if read == 0 {
                break;
            }
            feed_added_bytes(&mut accumulator, &buffer[..read], &mut line_start);
        }
    }
    accumulator.finish(scope, relative, if_match)
}

fn feed_added_text(accumulator: &mut DiffChunkAccumulator, bytes: &[u8]) {
    let mut line_start = true;
    feed_added_bytes(accumulator, bytes, &mut line_start);
}

fn feed_added_bytes(accumulator: &mut DiffChunkAccumulator, bytes: &[u8], line_start: &mut bool) {
    let mut start = 0;
    while start < bytes.len() {
        if *line_start {
            accumulator.feed(b"+");
            *line_start = false;
        }
        let end = bytes[start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |position| start + position + 1);
        accumulator.feed(&bytes[start..end]);
        if end > start && bytes[end - 1] == b'\n' {
            *line_start = true;
        }
        start = end;
    }
}

struct DiffChunkAccumulator {
    offset: u64,
    requested_end: u64,
    total: u64,
    digest: Sha256,
    bytes: Vec<u8>,
}

impl DiffChunkAccumulator {
    fn new(offset: u64, limit: u32) -> Self {
        Self {
            offset,
            requested_end: offset.saturating_add(u64::from(limit)),
            total: 0,
            digest: Sha256::new(),
            bytes: Vec::with_capacity(usize::try_from(limit).unwrap_or_default()),
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        self.digest.update(bytes);
        let start = self.total;
        let end = start.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        if end > self.offset && start < self.requested_end {
            let local_start =
                usize::try_from(self.offset.saturating_sub(start)).unwrap_or_default();
            let local_end = usize::try_from(self.requested_end.min(end).saturating_sub(start))
                .unwrap_or(bytes.len())
                .min(bytes.len());
            self.bytes
                .extend_from_slice(&bytes[local_start.min(bytes.len())..local_end]);
        }
        self.total = end;
    }

    fn finish(
        self,
        scope: DiffScope,
        path: &str,
        if_match: Option<&str>,
    ) -> Result<DiffReadFileResponse, WorkspaceError> {
        if self.offset > self.total {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::InvalidRequest,
                "Diff chunk offset exceeds the file Diff size",
            ));
        }
        let etag = format!("sha256:{}", encode_hex(&self.digest.finalize()));
        if if_match.is_some_and(|expected| expected != etag) {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::Conflict,
                "file Diff changed while it was being read",
            ));
        }
        let next_offset = self
            .offset
            .saturating_add(u64::try_from(self.bytes.len()).unwrap_or(u64::MAX));
        Ok(DiffReadFileResponse {
            scope,
            path: path.to_owned(),
            offset: self.offset,
            next_offset,
            byte_size: self.total,
            eof: next_offset >= self.total,
            data_base64: STANDARD.encode(&self.bytes),
            utf8_text: String::from_utf8(self.bytes).ok(),
            etag,
        })
    }
}

fn validate_base_revision(base_revision: Option<&str>) -> Result<(), WorkspaceError> {
    if base_revision.is_some_and(|base_revision| {
        base_revision.is_empty()
            || base_revision.starts_with('-')
            || base_revision.chars().count() > 512
    }) {
        return Err(WorkspaceError::new(
            WorkspaceErrorCode::InvalidRequest,
            "invalid Git diff base revision",
        ));
    }
    Ok(())
}

struct FileFingerprint {
    hash: String,
    size: u64,
    binary: bool,
    mode: String,
    kind: String,
}

fn fingerprint_path(path: &std::path::Path) -> Result<Option<FileFingerprint>, WorkspaceError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(crate::io_error(error)),
    };
    if metadata.file_type().is_symlink() {
        let target = std::fs::read_link(path).map_err(crate::io_error)?;
        let bytes = target.to_string_lossy().as_bytes().to_vec();
        return Ok(Some(FileFingerprint {
            hash: hex_digest(&bytes),
            size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            binary: false,
            mode: "120000".into(),
            kind: "symlink".into(),
        }));
    }
    if !metadata.is_file() {
        return Ok(Some(FileFingerprint {
            hash: String::new(),
            size: 0,
            binary: false,
            mode: String::new(),
            kind: "other".into(),
        }));
    }
    let mut file = File::open(path).map_err(crate::io_error)?;
    let mut digest = Sha256::new();
    let mut binary = false;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(crate::io_error)?;
        if read == 0 {
            break;
        }
        binary |= buffer[..read].contains(&0) || std::str::from_utf8(&buffer[..read]).is_err();
        digest.update(&buffer[..read]);
    }
    Ok(Some(FileFingerprint {
        hash: encode_hex(&digest.finalize()),
        size: metadata.len(),
        binary,
        mode: file_mode(&metadata),
        kind: "file".into(),
    }))
}

#[cfg(unix)]
fn file_mode(metadata: &std::fs::Metadata) -> String {
    use std::os::unix::fs::PermissionsExt;
    if metadata.permissions().mode() & 0o111 == 0 {
        "100644".into()
    } else {
        "100755".into()
    }
}

#[cfg(not(unix))]
fn file_mode(_metadata: &std::fs::Metadata) -> String {
    "100644".into()
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

fn parse_unified_diff(input: &str, truncated: bool) -> Vec<FileDiffSummary> {
    let mut files = Vec::new();
    let mut current: Option<FileDiffSummary> = None;
    let mut current_hunk: Option<DiffHunk> = None;
    let mut old_line = 0_u32;
    let mut new_line = 0_u32;
    for line in input.lines() {
        if let Some(paths) = line.strip_prefix("diff --git a/") {
            finish_hunk(&mut current, &mut current_hunk);
            if let Some(file) = current.take() {
                files.push(file);
            }
            let path = paths
                .split_once(" b/")
                .map(|(_, right)| right)
                .unwrap_or(paths)
                .to_owned();
            current = Some(FileDiffSummary {
                path,
                previous_path: None,
                status: FileDiffStatus::Modified,
                additions: 0,
                deletions: 0,
                binary: false,
                too_large: truncated,
                hunks: Vec::new(),
            });
            continue;
        }
        let Some(file) = current.as_mut() else {
            continue;
        };
        if line.starts_with("new file mode ") {
            file.status = FileDiffStatus::Added;
        } else if line.starts_with("deleted file mode ") {
            file.status = FileDiffStatus::Deleted;
        } else if let Some(previous) = line.strip_prefix("rename from ") {
            file.status = FileDiffStatus::Renamed;
            file.previous_path = Some(previous.to_owned());
        } else if let Some(path) = line.strip_prefix("rename to ") {
            file.path = path.to_owned();
        } else if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            file.status = FileDiffStatus::Binary;
            file.binary = true;
        } else if line.starts_with("@@") {
            finish_hunk(&mut current, &mut current_hunk);
            let (old, new) = parse_hunk_header(line);
            old_line = old;
            new_line = new;
            current_hunk = Some(DiffHunk {
                header: line.to_owned(),
                lines: Vec::new(),
            });
        } else if let Some(hunk) = current_hunk.as_mut() {
            if line.starts_with('+') && !line.starts_with("+++") {
                file.additions = file.additions.saturating_add(1);
                hunk.lines.push(DiffLine {
                    kind: "addition".into(),
                    old_line: None,
                    new_line: Some(new_line),
                    text: line[1..].to_owned(),
                });
                new_line = new_line.saturating_add(1);
            } else if line.starts_with('-') && !line.starts_with("---") {
                file.deletions = file.deletions.saturating_add(1);
                hunk.lines.push(DiffLine {
                    kind: "deletion".into(),
                    old_line: Some(old_line),
                    new_line: None,
                    text: line[1..].to_owned(),
                });
                old_line = old_line.saturating_add(1);
            } else if let Some(text) = line.strip_prefix(' ') {
                hunk.lines.push(DiffLine {
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
    finish_hunk(&mut current, &mut current_hunk);
    if let Some(file) = current {
        files.push(file);
    }
    files
}

fn finish_hunk(current: &mut Option<FileDiffSummary>, hunk: &mut Option<DiffHunk>) {
    if let (Some(file), Some(hunk)) = (current.as_mut(), hunk.take()) {
        file.hunks.push(hunk);
    }
}

fn parse_hunk_header(header: &str) -> (u32, u32) {
    let mut parts = header.split_whitespace();
    let _ = parts.next();
    let old = parts
        .next()
        .and_then(|value| value.strip_prefix('-'))
        .and_then(|value| value.split(',').next())
        .and_then(|value| value.parse().ok())
        .unwrap_or_default();
    let new = parts
        .next()
        .and_then(|value| value.strip_prefix('+'))
        .and_then(|value| value.split(',').next())
        .and_then(|value| value.parse().ok())
        .unwrap_or_default();
    (old, new)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::process::Command as StdCommand;

    use super::*;

    #[test]
    fn parses_per_file_hunks() {
        let files = parse_unified_diff(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@\n-old\n+new\n keep\n",
            false,
        );
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].additions, 1);
        assert_eq!(files[0].deletions, 1);
        assert_eq!(files[0].hunks.len(), 1);
    }

    #[tokio::test]
    async fn branch_scope_diff_uses_the_selected_merge_base() {
        let directory = tempfile::tempdir().expect("tempdir");
        let git = |arguments: &[&str]| {
            let output = StdCommand::new("git")
                .arg("-C")
                .arg(directory.path())
                .args(arguments)
                .output()
                .expect("git");
            assert!(
                output.status.success(),
                "git failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "-b", "main"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "user.name", "Hachimi Test"]);
        std::fs::write(directory.path().join("base.txt"), "base\n").expect("base");
        git(&["add", "base.txt"]);
        git(&["commit", "-m", "base"]);
        git(&["checkout", "-b", "feature"]);
        std::fs::write(directory.path().join("feature.txt"), "feature\n").expect("feature");
        git(&["add", "feature.txt"]);
        git(&["commit", "-m", "feature"]);
        git(&["checkout", "main"]);
        std::fs::write(directory.path().join("main.txt"), "main\n").expect("main");
        git(&["add", "main.txt"]);
        git(&["commit", "-m", "main"]);

        let worker =
            WorkerContext::new(directory.path(), "checkout", 1, "token").expect("worker context");
        let scope = DiffScope::Branch {
            checkout_id: hachimi_protocol::CheckoutId::new("checkout"),
            branch: "feature".into(),
        };
        let output = worker
            .git_diff_structured(scope.clone(), Some("feature...HEAD"))
            .await
            .expect("branch diff");
        let WorkspaceOutput::Diff { snapshot } = output else {
            panic!("expected Diff output");
        };
        assert_eq!(snapshot.scope, scope);
        assert!(snapshot.files.iter().any(|file| file.path == "main.txt"));
        assert!(!snapshot.files.iter().any(|file| file.path == "feature.txt"));
    }

    #[tokio::test]
    async fn checkout_diff_includes_untracked_text_and_binary_files() {
        let directory = tempfile::tempdir().expect("tempdir");
        let git = |arguments: &[&str]| {
            let output = StdCommand::new("git")
                .arg("-C")
                .arg(directory.path())
                .args(arguments)
                .output()
                .expect("git");
            assert!(
                output.status.success(),
                "git failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "-b", "main"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "user.name", "Hachimi Test"]);
        std::fs::write(directory.path().join("tracked.txt"), "tracked\n").expect("tracked");
        git(&["add", "tracked.txt"]);
        git(&["commit", "-m", "base"]);
        std::fs::write(directory.path().join("notes.txt"), "first\nsecond\n").expect("notes");
        std::fs::write(directory.path().join("asset.bin"), [0_u8, 1, 2]).expect("binary");

        let worker =
            WorkerContext::new(directory.path(), "checkout", 1, "token").expect("worker context");
        let scope = DiffScope::Checkout {
            checkout_id: hachimi_protocol::CheckoutId::new("checkout"),
        };
        let output = worker
            .git_diff_structured(scope.clone(), Some("HEAD"))
            .await
            .expect("checkout diff");
        let WorkspaceOutput::Diff { snapshot } = output else {
            panic!("expected Diff output");
        };
        let text = snapshot
            .files
            .iter()
            .find(|file| file.path == "notes.txt")
            .expect("text summary");
        assert_eq!(text.status, FileDiffStatus::Added);
        assert_eq!(text.additions, 2);
        assert_eq!(text.hunks[0].lines[1].text, "second");
        let binary = snapshot
            .files
            .iter()
            .find(|file| file.path == "asset.bin")
            .expect("binary summary");
        assert!(binary.binary);
        assert_eq!(binary.status, FileDiffStatus::Binary);

        let output = worker
            .git_diff_file_chunk(scope.clone(), "notes.txt", Some("HEAD"), 0, 1024, None)
            .await
            .expect("untracked file Diff");
        let WorkspaceOutput::DiffFileChunk { chunk } = output else {
            panic!("expected Diff chunk");
        };
        assert_eq!(chunk.scope, scope);
        assert!(chunk.eof);
        assert!(chunk.utf8_text.expect("text Diff").contains("+second"));
    }
}
