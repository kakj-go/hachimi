// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/apply-patch/src/{parser,streaming_parser,
// seek_sequence,lib}.rs
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// Modified for Hachimi: checkout-relative paths, bounded inputs, preflight of every
// target, and a Workspace Worker transaction with rollback on commit failure.

use std::{collections::BTreeSet, fs, path::PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    WorkerContext, WorkspaceError, WorkspaceErrorCode, WorkspaceOutput, atomic_write,
    ensure_content_size, read_bounded, sha256,
};

const BEGIN_PATCH: &str = "*** Begin Patch";
const END_PATCH: &str = "*** End Patch";
const ADD_FILE: &str = "*** Add File: ";
const DELETE_FILE: &str = "*** Delete File: ";
const UPDATE_FILE: &str = "*** Update File: ";
const MOVE_TO: &str = "*** Move to: ";
const END_OF_FILE: &str = "*** End of File";
const MAX_PATCH_BYTES: usize = 4 * 1024 * 1024;
const MAX_PATCH_FILES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyPatchPlan {
    hunks: Vec<PatchHunk>,
    targets: Vec<String>,
}

impl ApplyPatchPlan {
    #[must_use]
    pub fn targets(&self) -> &[String] {
        &self.targets
    }

    #[must_use]
    pub fn move_pairs(&self) -> Vec<(&str, &str)> {
        self.hunks
            .iter()
            .filter_map(|hunk| match hunk {
                PatchHunk::Update {
                    path,
                    move_to: Some(destination),
                    ..
                } => Some((path.as_str(), destination.as_str())),
                _ => None,
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PatchHunk {
    Add {
        path: String,
        content: String,
    },
    Delete {
        path: String,
    },
    Update {
        path: String,
        move_to: Option<String>,
        chunks: Vec<PatchChunk>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct PatchChunk {
    context: Option<String>,
    old_lines: Vec<String>,
    new_lines: Vec<String>,
    end_of_file: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePatchChange {
    pub path: String,
    pub previous_path: Option<String>,
    pub status: WorkspacePatchStatus,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspacePatchStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

pub fn parse_apply_patch(patch: &str) -> Result<ApplyPatchPlan, WorkspaceError> {
    if patch.len() > MAX_PATCH_BYTES {
        return Err(invalid("patch exceeds the 4 MiB limit"));
    }
    let normalized = patch.replace("\r\n", "\n");
    let mut lines = normalized.trim().split('\n').collect::<Vec<_>>();
    if lines.first().is_some_and(|line| line.trim() != BEGIN_PATCH)
        && lines.len() >= 4
        && matches!(
            lines.first().map(|line| line.trim()),
            Some("<<EOF" | "<<'EOF'" | "<<\"EOF\"")
        )
        && lines
            .last()
            .is_some_and(|line| line.trim_end().ends_with("EOF"))
    {
        lines = lines[1..lines.len() - 1].to_vec();
    }
    if lines.first().map(|line| line.trim()) != Some(BEGIN_PATCH) {
        return Err(invalid("the first line must be '*** Begin Patch'"));
    }
    if lines.last().map(|line| line.trim()) != Some(END_PATCH) {
        return Err(invalid("the last line must be '*** End Patch'"));
    }

    let mut hunks = Vec::new();
    let mut index = 1;
    while index + 1 < lines.len() {
        let marker = lines[index].trim_end();
        if let Some(path) = marker.strip_prefix(ADD_FILE) {
            let path = validate_patch_path(path)?;
            index += 1;
            let mut content = Vec::new();
            while index + 1 < lines.len() && !is_top_level_marker(lines[index]) {
                let line = lines[index].strip_prefix('+').ok_or_else(|| {
                    invalid(format!("add-file line {} must start with '+'", index + 1))
                })?;
                content.push(line);
                index += 1;
            }
            if content.is_empty() {
                return Err(invalid(format!("add-file hunk for '{path}' is empty")));
            }
            hunks.push(PatchHunk::Add {
                path,
                content: format!("{}\n", content.join("\n")),
            });
            continue;
        }
        if let Some(path) = marker.strip_prefix(DELETE_FILE) {
            hunks.push(PatchHunk::Delete {
                path: validate_patch_path(path)?,
            });
            index += 1;
            continue;
        }
        if let Some(path) = marker.strip_prefix(UPDATE_FILE) {
            let path = validate_patch_path(path)?;
            index += 1;
            let move_to = if index + 1 < lines.len() {
                lines[index]
                    .trim_end()
                    .strip_prefix(MOVE_TO)
                    .map(validate_patch_path)
                    .transpose()?
            } else {
                None
            };
            if move_to.is_some() {
                index += 1;
            }
            let mut chunks = Vec::new();
            let mut current: Option<PatchChunk> = None;
            while index + 1 < lines.len() && !is_top_level_marker(lines[index]) {
                let line = lines[index];
                if line == "@@" || line.starts_with("@@ ") {
                    finish_chunk(&mut chunks, current.take(), index + 1)?;
                    current = Some(PatchChunk {
                        context: line.strip_prefix("@@ ").map(str::to_owned),
                        ..PatchChunk::default()
                    });
                } else if line == END_OF_FILE {
                    current.get_or_insert_with(PatchChunk::default).end_of_file = true;
                } else {
                    let chunk = current.get_or_insert_with(PatchChunk::default);
                    let (prefix, value) = line.split_at(line.len().min(1));
                    match prefix {
                        " " => {
                            chunk.old_lines.push(value.to_owned());
                            chunk.new_lines.push(value.to_owned());
                        }
                        "+" => chunk.new_lines.push(value.to_owned()),
                        "-" => chunk.old_lines.push(value.to_owned()),
                        _ => {
                            return Err(invalid(format!(
                                "update line {} must start with ' ', '+' or '-'",
                                index + 1
                            )));
                        }
                    }
                }
                index += 1;
            }
            finish_chunk(&mut chunks, current.take(), index + 1)?;
            if chunks.is_empty() && move_to.is_none() {
                return Err(invalid(format!("update hunk for '{path}' is empty")));
            }
            hunks.push(PatchHunk::Update {
                path,
                move_to,
                chunks,
            });
            continue;
        }
        return Err(invalid(format!(
            "unexpected patch marker on line {}: {marker}",
            index + 1
        )));
    }
    if hunks.is_empty() {
        return Err(invalid("patch does not modify any files"));
    }
    if hunks.len() > MAX_PATCH_FILES {
        return Err(invalid("patch exceeds the 128 file limit"));
    }
    let mut unique = BTreeSet::new();
    let mut targets = Vec::new();
    for hunk in &hunks {
        let source = match hunk {
            PatchHunk::Add { path, .. }
            | PatchHunk::Delete { path }
            | PatchHunk::Update { path, .. } => path,
        };
        if !unique.insert(source.to_ascii_lowercase()) {
            return Err(invalid(format!(
                "patch contains duplicate target '{source}'"
            )));
        }
        targets.push(source.clone());
        if let PatchHunk::Update {
            move_to: Some(destination),
            ..
        } = hunk
        {
            if !unique.insert(destination.to_ascii_lowercase()) {
                return Err(invalid(format!(
                    "patch contains conflicting move target '{destination}'"
                )));
            }
            targets.push(destination.clone());
        }
    }
    Ok(ApplyPatchPlan { hunks, targets })
}

fn finish_chunk(
    chunks: &mut Vec<PatchChunk>,
    chunk: Option<PatchChunk>,
    line: usize,
) -> Result<(), WorkspaceError> {
    if let Some(chunk) = chunk {
        if chunk.old_lines.is_empty() && chunk.new_lines.is_empty() {
            return Err(invalid(format!("empty update chunk near line {line}")));
        }
        chunks.push(chunk);
    }
    Ok(())
}

fn is_top_level_marker(line: &str) -> bool {
    let line = line.trim_end();
    line == END_PATCH
        || line.starts_with(ADD_FILE)
        || line.starts_with(DELETE_FILE)
        || line.starts_with(UPDATE_FILE)
}

fn validate_patch_path(path: &str) -> Result<String, WorkspaceError> {
    let normalized = path.trim().replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains(':')
        || normalized.contains('\0')
        || normalized
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(invalid(format!(
            "patch path must be checkout-relative: '{path}'"
        )));
    }
    Ok(normalized)
}

#[derive(Debug)]
struct PreparedChange {
    source_relative: String,
    source: PathBuf,
    destination_relative: String,
    destination: PathBuf,
    old_source: Option<Vec<u8>>,
    old_destination: Option<Vec<u8>>,
    new_content: Option<Vec<u8>>,
    status: WorkspacePatchStatus,
}

impl WorkerContext {
    pub(crate) fn apply_patch(&self, patch: &str) -> Result<WorkspaceOutput, WorkspaceError> {
        let plan = parse_apply_patch(patch)?;
        let prepared = self.prepare_patch(&plan)?;
        let transaction_root = std::env::temp_dir().join(format!(
            "hachimi-patch-{}-{}",
            self.checkout_id,
            Uuid::new_v4()
        ));
        fs::create_dir_all(&transaction_root).map_err(io_error)?;
        let result = self.commit_patch(&prepared, &transaction_root);
        let _ = fs::remove_dir_all(&transaction_root);
        result
    }

    fn prepare_patch(&self, plan: &ApplyPatchPlan) -> Result<Vec<PreparedChange>, WorkspaceError> {
        let mut prepared = Vec::with_capacity(plan.hunks.len());
        for hunk in &plan.hunks {
            let (source_relative, destination_relative, content, status) = match hunk {
                PatchHunk::Add { path, content } => {
                    ensure_content_size(content)?;
                    (
                        path.clone(),
                        path.clone(),
                        Some(content.as_bytes().to_vec()),
                        WorkspacePatchStatus::Added,
                    )
                }
                PatchHunk::Delete { path } => (
                    path.clone(),
                    path.clone(),
                    None,
                    WorkspacePatchStatus::Deleted,
                ),
                PatchHunk::Update {
                    path,
                    move_to,
                    chunks,
                } => {
                    let source = self.resolve_existing(path)?;
                    let original = String::from_utf8(read_bounded(&source)?).map_err(|_| {
                        WorkspaceError::new(
                            WorkspaceErrorCode::NotText,
                            "patch target is not UTF-8 text",
                        )
                    })?;
                    let updated = apply_chunks(&original, path, chunks)?;
                    ensure_content_size(&updated)?;
                    let destination = move_to.as_ref().unwrap_or(path).clone();
                    (
                        path.clone(),
                        destination,
                        Some(updated.into_bytes()),
                        if move_to.is_some() {
                            WorkspacePatchStatus::Renamed
                        } else {
                            WorkspacePatchStatus::Modified
                        },
                    )
                }
            };
            let source = if matches!(hunk, PatchHunk::Add { .. }) {
                self.resolve_patch_write(&source_relative)?
            } else {
                self.resolve_existing(&source_relative)?
            };
            let destination = self.resolve_patch_write(&destination_relative)?;
            let old_source = source.exists().then(|| read_bounded(&source)).transpose()?;
            let old_destination = if destination != source && destination.exists() {
                Some(read_bounded(&destination)?)
            } else {
                None
            };
            match hunk {
                PatchHunk::Add { .. } if old_source.is_some() => {
                    return Err(conflict(format!(
                        "add target already exists: '{source_relative}'"
                    )));
                }
                PatchHunk::Update {
                    move_to: Some(_), ..
                } if old_destination.is_some() => {
                    return Err(conflict(format!(
                        "move target already exists: '{destination_relative}'"
                    )));
                }
                PatchHunk::Delete { .. } | PatchHunk::Update { .. } if old_source.is_none() => {
                    return Err(conflict(format!(
                        "patch target does not exist: '{source_relative}'"
                    )));
                }
                _ => {}
            }
            prepared.push(PreparedChange {
                source_relative,
                source,
                destination_relative,
                destination,
                old_source,
                old_destination,
                new_content: content,
                status,
            });
        }
        Ok(prepared)
    }

    fn commit_patch(
        &self,
        changes: &[PreparedChange],
        transaction_root: &std::path::Path,
    ) -> Result<WorkspaceOutput, WorkspaceError> {
        for (index, change) in changes.iter().enumerate() {
            if let Some(content) = &change.new_content {
                let staged = transaction_root.join(format!("{index}.new"));
                atomic_write(&staged, content)?;
            }
        }
        for (committed, (index, change)) in changes.iter().enumerate().enumerate() {
            let result: Result<(), WorkspaceError> = (|| {
                if let Some(content) = &change.new_content {
                    if let Some(parent) = change.destination.parent() {
                        fs::create_dir_all(parent).map_err(io_error)?;
                    }
                    let destination = self.resolve_write(&change.destination_relative)?;
                    let staged = transaction_root.join(format!("{index}.new"));
                    let bytes = fs::read(staged).map_err(io_error)?;
                    debug_assert_eq!(&bytes, content);
                    atomic_write(&destination, &bytes)?;
                    if change.source != destination && change.source.exists() {
                        fs::remove_file(&change.source).map_err(io_error)?;
                    }
                    Ok(())
                } else {
                    fs::remove_file(&change.source).map_err(io_error)
                }
            })();
            if let Err(error) = result {
                rollback(&self.root, &changes[..=committed]);
                return Err(WorkspaceError::new(
                    WorkspaceErrorCode::Io,
                    format!("patch transaction rolled back: {}", error.message),
                ));
            }
        }
        let output = changes
            .iter()
            .map(|change| WorkspacePatchChange {
                path: change.destination_relative.clone(),
                previous_path: (change.source_relative != change.destination_relative)
                    .then(|| change.source_relative.clone()),
                status: change.status,
                sha256: change.new_content.as_deref().map(sha256),
            })
            .collect();
        Ok(WorkspaceOutput::Patch { changes: output })
    }

    fn resolve_patch_write(&self, relative: &str) -> Result<PathBuf, WorkspaceError> {
        match self.resolve_write(relative) {
            Ok(path) => Ok(path),
            Err(error) if error.code == WorkspaceErrorCode::NotFound => {
                let parts = relative.split('/').collect::<Vec<_>>();
                let mut prefix = String::new();
                for part in parts.iter().take(parts.len().saturating_sub(1)) {
                    if !prefix.is_empty() {
                        prefix.push('/');
                    }
                    prefix.push_str(part);
                    let candidate = self
                        .root
                        .join(prefix.replace('/', std::path::MAIN_SEPARATOR_STR));
                    if candidate.exists() {
                        let verified = self.resolve_existing(&prefix)?;
                        if !verified.is_dir() {
                            return Err(conflict(format!(
                                "patch parent is not a directory: '{prefix}'"
                            )));
                        }
                    }
                }
                Ok(self
                    .root
                    .join(relative.replace('/', std::path::MAIN_SEPARATOR_STR)))
            }
            Err(error) => Err(error),
        }
    }
}

fn rollback(root: &std::path::Path, changes: &[PreparedChange]) {
    for change in changes.iter().rev() {
        if change.source != change.destination {
            restore(&change.destination, change.old_destination.as_deref());
        }
        restore(&change.source, change.old_source.as_deref());
        cleanup_empty_parents(change.destination.parent(), root);
    }
}

fn cleanup_empty_parents(mut directory: Option<&std::path::Path>, root: &std::path::Path) {
    while let Some(path) = directory {
        if path == root || !path.starts_with(root) || fs::remove_dir(path).is_err() {
            break;
        }
        directory = path.parent();
    }
}

fn restore(path: &std::path::Path, content: Option<&[u8]>) {
    match content {
        Some(content) => {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = atomic_write(path, content);
        }
        None => {
            let _ = fs::remove_file(path);
        }
    }
}

fn apply_chunks(
    original: &str,
    path: &str,
    chunks: &[PatchChunk],
) -> Result<String, WorkspaceError> {
    if chunks.is_empty() {
        return Ok(original.to_owned());
    }
    let mut lines = original.split('\n').map(str::to_owned).collect::<Vec<_>>();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    let mut replacements = Vec::new();
    let mut cursor = 0;
    for chunk in chunks {
        if let Some(context) = &chunk.context {
            cursor = seek_sequence(&lines, std::slice::from_ref(context), cursor, false)
                .map(|index| index + 1)
                .ok_or_else(|| conflict(format!("failed to find context '{context}' in {path}")))?;
        }
        if chunk.old_lines.is_empty() {
            replacements.push((lines.len(), 0, chunk.new_lines.clone()));
            continue;
        }
        let position = seek_sequence(&lines, &chunk.old_lines, cursor, chunk.end_of_file)
            .ok_or_else(|| conflict(format!("failed to find patch context in {path}")))?;
        replacements.push((position, chunk.old_lines.len(), chunk.new_lines.clone()));
        cursor = position + chunk.old_lines.len();
    }
    for (position, old_len, replacement) in replacements.into_iter().rev() {
        lines.splice(position..position + old_len, replacement);
    }
    Ok(format!("{}\n", lines.join("\n")))
}

fn seek_sequence(lines: &[String], pattern: &[String], start: usize, eof: bool) -> Option<usize> {
    if pattern.is_empty() {
        return Some(start.min(lines.len()));
    }
    if pattern.len() > lines.len() {
        return None;
    }
    let first = if eof {
        lines.len() - pattern.len()
    } else {
        start.min(lines.len() - pattern.len())
    };
    for comparison in [
        |left: &str, right: &str| left == right,
        |left: &str, right: &str| left.trim_end() == right.trim_end(),
        |left: &str, right: &str| left.trim() == right.trim(),
    ] {
        for index in first..=lines.len() - pattern.len() {
            if lines[index..index + pattern.len()]
                .iter()
                .zip(pattern)
                .all(|(left, right)| comparison(left, right))
            {
                return Some(index);
            }
        }
    }
    None
}

fn invalid(message: impl Into<String>) -> WorkspaceError {
    WorkspaceError::new(WorkspaceErrorCode::InvalidRequest, message)
}

fn conflict(message: impl Into<String>) -> WorkspaceError {
    WorkspaceError::new(WorkspaceErrorCode::Conflict, message)
}

fn io_error(error: std::io::Error) -> WorkspaceError {
    WorkspaceError::new(WorkspaceErrorCode::Io, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_add_update_delete_and_move_targets() {
        let plan = parse_apply_patch(
            "*** Begin Patch\n*** Add File: a.txt\n+hello\n*** Update File: b.txt\n*** Move to: c.txt\n@@\n-old\n+new\n*** Delete File: d.txt\n*** End Patch",
        )
        .expect("patch");
        assert_eq!(plan.targets(), ["a.txt", "b.txt", "c.txt", "d.txt"]);
    }

    #[test]
    fn rejects_escape_and_duplicate_targets() {
        assert!(
            parse_apply_patch("*** Begin Patch\n*** Delete File: ../x\n*** End Patch").is_err()
        );
        assert!(
            parse_apply_patch(
                "*** Begin Patch\n*** Delete File: A\n*** Delete File: a\n*** End Patch"
            )
            .is_err()
        );
    }

    #[test]
    fn update_uses_codex_style_context_matching() {
        let plan = parse_apply_patch(
            "*** Begin Patch\n*** Update File: a.txt\n@@ section\n-old  \n+new\n*** End Patch",
        )
        .expect("patch");
        let PatchHunk::Update { chunks, .. } = &plan.hunks[0] else {
            panic!("update")
        };
        let updated = apply_chunks("header\nsection\nold\n", "a.txt", chunks).expect("apply");
        assert_eq!(updated, "header\nsection\nnew\n");
    }
}
