// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex/codex-rs/app-server-protocol/src/protocol/v2/fs.rs
// @ 4c43465133428898aa84f0bfc02c306ed65fb66a.
// Modified for Hachimi: Checkout-bound paging, generation fencing, and typed Diff chunks.
//! Transport-neutral Workbench filesystem, search, watch, and Diff contracts.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::{ArtifactId, CheckoutId, FsSearchId, FsWatchId, MutationContext, RunId, SessionId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FsEntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FsEntry {
    pub path: String,
    pub name: String,
    pub kind: FsEntryKind,
    #[specta(type = Option<specta_typescript::Number>)]
    pub byte_size: Option<u64>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub modified_at_ms: Option<i64>,
    pub hidden: bool,
    pub has_children: bool,
    pub git_status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FsListRequest {
    pub session_id: SessionId,
    pub checkout_id: CheckoutId,
    pub path: String,
    pub cursor: Option<String>,
    pub limit: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FsListPage {
    pub path: String,
    pub entries: Vec<FsEntry>,
    pub next_cursor: Option<String>,
    pub etag: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FsReadChunkRequest {
    pub session_id: SessionId,
    pub checkout_id: CheckoutId,
    pub path: String,
    #[specta(type = specta_typescript::Number)]
    pub offset: u64,
    pub limit: u32,
    pub if_match: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FsFileChunk {
    pub path: String,
    #[specta(type = specta_typescript::Number)]
    pub offset: u64,
    #[specta(type = specta_typescript::Number)]
    pub next_offset: u64,
    #[specta(type = specta_typescript::Number)]
    pub byte_size: u64,
    pub eof: bool,
    pub binary: bool,
    pub data_base64: String,
    pub utf8_text: Option<String>,
    pub etag: String,
}

/// Explicit user-initiated text save. The ETag must be the SHA-256 returned by
/// the authoritative full-file read and the mutation context fences retries to
/// the selected Session/Run generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FsWriteRequest {
    pub context: MutationContext,
    pub session_id: SessionId,
    pub checkout_id: CheckoutId,
    pub path: String,
    pub content: String,
    pub if_match: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FsWriteResponse {
    pub path: String,
    #[specta(type = specta_typescript::Number)]
    pub byte_size: u64,
    pub etag: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitFileStatus {
    pub index_status: String,
    pub worktree_status: String,
    pub path: String,
    pub previous_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitSummary {
    pub sha: String,
    pub abbreviated_sha: String,
    pub subject: String,
    pub author_name: String,
    #[specta(type = specta_typescript::Number)]
    pub committed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitWorkspaceRequest {
    pub session_id: SessionId,
    pub checkout_id: CheckoutId,
    pub history_limit: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitWorkspaceSnapshot {
    pub branch: Option<String>,
    pub head_sha: Option<String>,
    pub detached: bool,
    pub status: Vec<GitFileStatus>,
    pub recent_commits: Vec<GitCommitSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GitMutation {
    Stage {
        paths: Vec<String>,
    },
    Unstage {
        paths: Vec<String>,
    },
    Commit {
        message: String,
    },
    CreateEmptyInitialCommit {
        author_name: String,
        author_email: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitMutationRequest {
    pub context: MutationContext,
    pub session_id: SessionId,
    pub checkout_id: CheckoutId,
    pub mutation: GitMutation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitMutationResponse {
    pub snapshot: GitWorkspaceSnapshot,
    pub commit_sha: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FsChangeKind {
    Created,
    Modified,
    Removed,
    Renamed,
    Invalidated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FsWatchRequest {
    pub session_id: SessionId,
    pub checkout_id: CheckoutId,
    pub path: String,
    pub recursive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FsWatchRegistration {
    pub id: FsWatchId,
    pub session_id: SessionId,
    pub checkout_id: CheckoutId,
    pub path: String,
    #[specta(type = specta_typescript::Number)]
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FsChangeEvent {
    pub watch_id: FsWatchId,
    #[specta(type = specta_typescript::Number)]
    pub generation: u64,
    pub kind: FsChangeKind,
    pub paths: Vec<String>,
    pub overflowed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FsSearchStartRequest {
    pub session_id: SessionId,
    pub checkout_id: CheckoutId,
    pub query: String,
    pub max_results: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FsSearchUpdateRequest {
    pub search_id: FsSearchId,
    #[specta(type = specta_typescript::Number)]
    pub expected_generation: u64,
    pub query: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FsSearchResult {
    pub path: String,
    #[specta(type = specta_typescript::Number)]
    pub score: i64,
    pub match_indices: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FsSearchSnapshot {
    pub search_id: FsSearchId,
    #[specta(type = specta_typescript::Number)]
    pub generation: u64,
    pub query: String,
    pub results: Vec<FsSearchResult>,
    pub complete: bool,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DiffScope {
    Run { run_id: RunId },
    Checkout { checkout_id: CheckoutId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FileDiffStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    TypeChanged,
    Binary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: String,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiffHunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FileDiffSummary {
    pub path: String,
    pub previous_path: Option<String>,
    pub status: FileDiffStatus,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
    pub too_large: bool,
    pub hunks: Vec<DiffHunk>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunDiffSnapshot {
    pub scope: DiffScope,
    pub files: Vec<FileDiffSummary>,
    pub artifact_id: Option<ArtifactId>,
    pub truncated: bool,
    #[specta(type = specta_typescript::Number)]
    pub generated_at_ms: i64,
}

/// Reads one file's unified Diff without exposing an arbitrary Artifact identifier or host path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiffReadFileRequest {
    pub scope: DiffScope,
    pub path: String,
    #[specta(type = specta_typescript::Number)]
    pub offset: u64,
    pub limit: u32,
    pub if_match: Option<String>,
}

/// A byte-bounded part of one file's unified Diff. The final response remains authoritative.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiffReadFileResponse {
    pub scope: DiffScope,
    pub path: String,
    #[specta(type = specta_typescript::Number)]
    pub offset: u64,
    #[specta(type = specta_typescript::Number)]
    pub next_offset: u64,
    #[specta(type = specta_typescript::Number)]
    pub byte_size: u64,
    pub eof: bool,
    pub data_base64: String,
    pub utf8_text: Option<String>,
    pub etag: String,
}
