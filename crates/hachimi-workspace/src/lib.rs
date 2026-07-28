// SPDX-License-Identifier: Apache-2.0
// Adapted in part from openai/codex at commit
// 4c43465133428898aa84f0bfc02c306ed65fb66a; see provenance for source paths.

//! Short-lived, process-isolated Workspace Host protocol and implementation.
//!
//! The Agent kernel never receives a filesystem handle. Each operation is sent to a fresh
//! worker process with a checkout-bound token and generation; cancellation drops and kills that
//! process. This is a process boundary, not an OS sandbox, and callers must still apply Policy,
//! Approval, and Sandbox checks before dispatching side effects.

mod browser;
mod diff;
mod file_search;
mod git;
mod git_alias;
mod operation;
mod patch;
mod review_diff;
mod watch;

pub use file_search::{SearchServerCommand, SearchServerRequest, run_search_server};
pub use patch::{ApplyPatchPlan, WorkspacePatchChange, WorkspacePatchStatus, parse_apply_patch};
pub use watch::{WatchServerRequest, run_watch_server};

use std::{
    ffi::OsStr,
    future::Future,
    io::{Read, Write},
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use atomic_write_file::AtomicWriteFile;
use hachimi_sandbox::{
    PathAccess, PathSecurityError, SandboxBackend, SandboxLaunchSpec, SandboxNetworkPolicy,
    resolve_checkout_path, validate_checkout_root,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::{Mutex as AsyncMutex, mpsc, watch as tokio_watch},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use operation::workspace_operation_effect;

pub const WORKER_TOKEN_ENV: &str = "HACHIMI_WORKSPACE_WORKER_TOKEN";
pub const GIT_EXECUTABLE_ENV: &str = "HACHIMI_GIT_EXECUTABLE";
const MAX_TEXT_BYTES: u64 = 2 * 1024 * 1024;
const MAX_PROCESS_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_SEARCH_RESULTS: usize = 200;
const MAX_SEARCHED_FILES: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRequestEnvelope {
    pub request_id: String,
    pub checkout_id: String,
    pub run_generation: u64,
    pub worker_token: String,
    pub operation: WorkspaceOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkspaceOperation {
    ReadFile {
        path: String,
    },
    ListDirectory {
        path: String,
    },
    ListDirectoryPage {
        path: String,
        cursor: Option<String>,
        limit: u16,
    },
    ReadFileChunk {
        path: String,
        offset: u64,
        limit: u32,
        if_match: Option<String>,
    },
    FuzzyFileSearch {
        query: String,
        max_results: u16,
        search_id: hachimi_protocol::FsSearchId,
        generation: u64,
    },
    SearchText {
        path: String,
        query: String,
        case_sensitive: bool,
        max_results: usize,
    },
    WriteFile {
        path: String,
        content: String,
        expected_sha256: Option<String>,
    },
    ReplaceText {
        path: String,
        old_text: String,
        new_text: String,
        expected_sha256: String,
        replace_all: bool,
    },
    ApplyPatch {
        patch: String,
    },
    GitStatus,
    GitDiff,
    GitReviewDiff {
        target: hachimi_protocol::ReviewTarget,
    },
    GitDiffStructured {
        scope: hachimi_protocol::DiffScope,
        base_revision: Option<String>,
    },
    GitDiffFileChunk {
        scope: hachimi_protocol::DiffScope,
        path: String,
        base_revision: Option<String>,
        offset: u64,
        limit: u32,
        if_match: Option<String>,
    },
    GitStatusSnapshot,
    GitWorkspaceSnapshot {
        history_limit: u16,
    },
    GitProjectInspect {
        project_id: hachimi_protocol::ProjectId,
    },
    GitStage {
        paths: Vec<String>,
        history_limit: u16,
    },
    GitUnstage {
        paths: Vec<String>,
        history_limit: u16,
    },
    GitCommit {
        message: String,
        history_limit: u16,
    },
    GitCreateEmptyInitialCommit {
        author_name: String,
        author_email: String,
        history_limit: u16,
    },
    ReadGitBlob {
        path: String,
    },
    Exec {
        program: String,
        args: Vec<String>,
        cwd: String,
        timeout_ms: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkspaceOutput {
    File {
        path: String,
        content: String,
        sha256: String,
        byte_size: u64,
    },
    Directory {
        path: String,
        entries: Vec<WorkspaceEntry>,
    },
    DirectoryPage {
        page: hachimi_protocol::FsListPage,
    },
    FileChunk {
        chunk: hachimi_protocol::FsFileChunk,
    },
    FileSearch {
        snapshot: hachimi_protocol::FsSearchSnapshot,
    },
    Diff {
        snapshot: hachimi_protocol::RunDiffSnapshot,
    },
    DiffFileChunk {
        chunk: hachimi_protocol::DiffReadFileResponse,
    },
    GitStatusSnapshot {
        entries: Vec<GitStatusEntry>,
    },
    GitWorkspaceSnapshot {
        snapshot: hachimi_protocol::GitWorkspaceSnapshot,
    },
    ProjectGitSnapshot {
        snapshot: hachimi_protocol::ProjectGitSnapshot,
    },
    GitMutation {
        response: hachimi_protocol::GitMutationResponse,
    },
    GitBlob {
        blob: GitBlob,
    },
    Search {
        matches: Vec<SearchMatch>,
        truncated: bool,
    },
    Write {
        path: String,
        sha256: String,
        byte_size: u64,
        replacements: usize,
    },
    Patch {
        changes: Vec<WorkspacePatchChange>,
    },
    Process {
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
        truncated: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEntry {
    pub path: String,
    pub kind: WorkspaceEntryKind,
    pub byte_size: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceEntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchMatch {
    pub path: String,
    pub line: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusEntry {
    pub index_status: char,
    pub worktree_status: char,
    pub path: String,
    pub previous_path: Option<String>,
    pub current_hash: Option<String>,
    pub current_size: Option<u64>,
    pub current_binary: bool,
    pub current_mode: Option<String>,
    pub current_kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitBlob {
    pub path: String,
    pub data_base64: String,
    pub sha256: String,
    pub byte_size: u64,
    pub binary: bool,
    pub mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceResponseEnvelope {
    pub request_id: String,
    pub output: Option<WorkspaceOutput>,
    pub error: Option<WorkspaceErrorRecord>,
}

impl WorkspaceResponseEnvelope {
    #[must_use]
    pub fn success(request_id: String, output: WorkspaceOutput) -> Self {
        Self {
            request_id,
            output: Some(output),
            error: None,
        }
    }

    #[must_use]
    pub fn failure(request_id: String, error: WorkspaceError) -> Self {
        Self {
            request_id,
            output: None,
            error: Some(error.into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceErrorCode {
    InvalidRequest,
    Unauthorized,
    StaleGeneration,
    PathOutsideCheckout,
    NotFound,
    NotText,
    TooLarge,
    Conflict,
    ProcessFailed,
    TimedOut,
    Cancelled,
    HostDisconnected,
    Io,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceErrorRecord {
    pub code: WorkspaceErrorCode,
    pub message: String,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{message}")]
pub struct WorkspaceError {
    pub code: WorkspaceErrorCode,
    pub message: String,
}

impl WorkspaceError {
    pub fn new(code: WorkspaceErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<WorkspaceError> for WorkspaceErrorRecord {
    fn from(value: WorkspaceError) -> Self {
        Self {
            code: value.code,
            message: value.message,
        }
    }
}

#[derive(Clone)]
pub struct WorkspaceHostClient {
    worker_program: PathBuf,
    restricted_launcher: Option<PathBuf>,
    checkout_root: PathBuf,
    checkout_id: String,
    run_generation: u64,
    worker_token: String,
    run_temp: Arc<RunTempDirectory>,
    sandbox_backend: Option<Arc<dyn SandboxBackend>>,
    sandbox_context: Option<WorkspaceSandboxContext>,
    launch_guard: Option<Arc<dyn WorkspaceLaunchGuard>>,
}

impl std::fmt::Debug for WorkspaceHostClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkspaceHostClient")
            .field("worker_program", &self.worker_program)
            .field("checkout_root", &self.checkout_root)
            .field("checkout_id", &self.checkout_id)
            .field("run_generation", &self.run_generation)
            .field("sandbox_configured", &self.sandbox_backend.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct WorkspaceSandboxContext {
    pub session_id: hachimi_protocol::SessionId,
    pub run_id: hachimi_protocol::RunId,
    pub grants: hachimi_protocol::CapabilityGrantSet,
}

#[derive(Debug, Clone)]
pub struct WorkspaceLaunchCheck {
    pub session_id: hachimi_protocol::SessionId,
    pub run_id: hachimi_protocol::RunId,
    pub run_generation: u64,
    pub checkout_id: hachimi_protocol::CheckoutId,
    pub effect: hachimi_protocol::ToolEffect,
}

pub type WorkspaceLaunchValidationFuture =
    Pin<Box<dyn Future<Output = Result<(), WorkspaceError>> + Send + 'static>>;

pub trait WorkspaceLaunchGuard: Send + Sync {
    fn validate(&self, check: WorkspaceLaunchCheck) -> WorkspaceLaunchValidationFuture;
}

impl WorkspaceHostClient {
    #[must_use]
    pub fn new(
        worker_program: impl Into<PathBuf>,
        checkout_root: impl Into<PathBuf>,
        checkout_id: impl Into<String>,
        run_generation: u64,
    ) -> Self {
        Self {
            worker_program: worker_program.into(),
            restricted_launcher: None,
            checkout_root: checkout_root.into(),
            checkout_id: checkout_id.into(),
            run_generation,
            worker_token: Uuid::new_v4().to_string(),
            run_temp: Arc::new(RunTempDirectory::new()),
            sandbox_backend: None,
            sandbox_context: None,
            launch_guard: None,
        }
    }

    #[must_use]
    pub const fn run_generation(&self) -> u64 {
        self.run_generation
    }

    #[must_use]
    pub fn with_restricted_launcher(mut self, launcher: impl Into<PathBuf>) -> Self {
        self.restricted_launcher = Some(launcher.into());
        self
    }

    #[must_use]
    pub fn with_sandbox(
        mut self,
        backend: Arc<dyn SandboxBackend>,
        context: WorkspaceSandboxContext,
        launch_guard: Arc<dyn WorkspaceLaunchGuard>,
    ) -> Self {
        self.sandbox_backend = Some(backend);
        self.sandbox_context = Some(context);
        self.launch_guard = Some(launch_guard);
        self
    }

    #[must_use]
    pub fn run_temp_dir(&self) -> &Path {
        &self.run_temp.path
    }

    pub async fn start_watch(
        &self,
        session_id: hachimi_protocol::SessionId,
        path: String,
        recursive: bool,
        generation: u64,
        cancellation: CancellationToken,
    ) -> Result<WorkspaceWatchSession, WorkspaceError> {
        if cancellation.is_cancelled() {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::Cancelled,
                "workspace watch was cancelled before dispatch",
            ));
        }
        let watch_id = hachimi_protocol::FsWatchId::random();
        let registration = hachimi_protocol::FsWatchRegistration {
            id: watch_id.clone(),
            session_id,
            checkout_id: hachimi_protocol::CheckoutId::new(self.checkout_id.clone()),
            path: path.clone(),
            generation,
        };
        let request = WatchServerRequest {
            checkout_id: self.checkout_id.clone(),
            run_generation: self.run_generation,
            worker_token: self.worker_token.clone(),
            watch_id,
            watch_generation: generation,
            path,
            recursive,
            interval_ms: 150,
        };
        let encoded = serde_json::to_vec(&request).map_err(|error| {
            WorkspaceError::new(WorkspaceErrorCode::InvalidRequest, error.to_string())
        })?;
        let mut command = self.worker_command();
        self.prepare_run_temp()?;
        command
            .arg("--watch-server")
            .arg("--root")
            .arg(&self.checkout_root)
            .arg("--checkout-id")
            .arg(&self.checkout_id)
            .arg("--generation")
            .arg(self.run_generation.to_string())
            .current_dir(&self.checkout_root)
            .env(WORKER_TOKEN_ENV, &self.worker_token)
            .env("TEMP", &self.run_temp.path)
            .env("TMP", &self.run_temp.path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|error| {
            WorkspaceError::new(WorkspaceErrorCode::HostDisconnected, error.to_string())
        })?;
        let mut stdin = child.stdin.take().ok_or_else(|| {
            WorkspaceError::new(
                WorkspaceErrorCode::HostDisconnected,
                "workspace watch stdin is unavailable",
            )
        })?;
        stdin.write_all(&encoded).await.map_err(|error| {
            WorkspaceError::new(WorkspaceErrorCode::HostDisconnected, error.to_string())
        })?;
        stdin.write_all(b"\n").await.map_err(|error| {
            WorkspaceError::new(WorkspaceErrorCode::HostDisconnected, error.to_string())
        })?;
        stdin.shutdown().await.map_err(|error| {
            WorkspaceError::new(WorkspaceErrorCode::HostDisconnected, error.to_string())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            WorkspaceError::new(
                WorkspaceErrorCode::HostDisconnected,
                "workspace watch stdout is unavailable",
            )
        })?;
        let local_cancellation = CancellationToken::new();
        let task_cancellation = local_cancellation.clone();
        let (events, receiver) = mpsc::channel(64);
        let task = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            loop {
                tokio::select! {
                    biased;
                    () = cancellation.cancelled() => {
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                        break;
                    }
                    () = task_cancellation.cancelled() => {
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                        break;
                    }
                    line = lines.next_line() => match line {
                        Ok(Some(line)) => match serde_json::from_str::<hachimi_protocol::FsChangeEvent>(&line) {
                            Ok(event) => {
                                if events.send(Ok(event)).await.is_err() {
                                    let _ = child.kill().await;
                                    let _ = child.wait().await;
                                    break;
                                }
                            }
                            Err(error) => {
                                let _ = events.send(Err(WorkspaceError::new(
                                    WorkspaceErrorCode::HostDisconnected,
                                    format!("invalid workspace watch event: {error}"),
                                ))).await;
                                let _ = child.kill().await;
                                let _ = child.wait().await;
                                break;
                            }
                        },
                        Ok(None) => {
                            let status = child.wait().await;
                            if !status.is_ok_and(|status| status.success()) {
                                let _ = events.send(Err(WorkspaceError::new(
                                    WorkspaceErrorCode::HostDisconnected,
                                    "workspace watch worker disconnected",
                                ))).await;
                            }
                            break;
                        }
                        Err(error) => {
                            let _ = events.send(Err(WorkspaceError::new(
                                WorkspaceErrorCode::HostDisconnected,
                                error.to_string(),
                            ))).await;
                            let _ = child.kill().await;
                            let _ = child.wait().await;
                            break;
                        }
                    }
                }
            }
        });
        Ok(WorkspaceWatchSession {
            registration,
            receiver,
            cancellation: local_cancellation,
            task: Some(task),
        })
    }

    pub async fn start_file_search(
        &self,
        search_id: hachimi_protocol::FsSearchId,
        query: String,
        max_results: u16,
        generation: u64,
        cancellation: CancellationToken,
    ) -> Result<WorkspaceSearchSession, WorkspaceError> {
        if cancellation.is_cancelled() {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::Cancelled,
                "workspace search was cancelled before dispatch",
            ));
        }
        let request = SearchServerRequest {
            checkout_id: self.checkout_id.clone(),
            run_generation: self.run_generation,
            worker_token: self.worker_token.clone(),
            search_id: search_id.clone(),
            search_generation: generation,
            query,
            max_results,
        };
        let encoded = serde_json::to_vec(&request).map_err(|error| {
            WorkspaceError::new(WorkspaceErrorCode::InvalidRequest, error.to_string())
        })?;
        let mut command = self.worker_command();
        self.prepare_run_temp()?;
        command
            .arg("--search-server")
            .arg("--root")
            .arg(&self.checkout_root)
            .arg("--checkout-id")
            .arg(&self.checkout_id)
            .arg("--generation")
            .arg(self.run_generation.to_string())
            .current_dir(&self.checkout_root)
            .env(WORKER_TOKEN_ENV, &self.worker_token)
            .env("TEMP", &self.run_temp.path)
            .env("TMP", &self.run_temp.path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|error| {
            WorkspaceError::new(WorkspaceErrorCode::HostDisconnected, error.to_string())
        })?;
        let mut stdin = child.stdin.take().ok_or_else(|| {
            WorkspaceError::new(
                WorkspaceErrorCode::HostDisconnected,
                "workspace search stdin is unavailable",
            )
        })?;
        stdin.write_all(&encoded).await.map_err(|error| {
            WorkspaceError::new(WorkspaceErrorCode::HostDisconnected, error.to_string())
        })?;
        stdin.write_all(b"\n").await.map_err(|error| {
            WorkspaceError::new(WorkspaceErrorCode::HostDisconnected, error.to_string())
        })?;
        stdin.flush().await.map_err(|error| {
            WorkspaceError::new(WorkspaceErrorCode::HostDisconnected, error.to_string())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            WorkspaceError::new(
                WorkspaceErrorCode::HostDisconnected,
                "workspace search stdout is unavailable",
            )
        })?;
        let local_cancellation = CancellationToken::new();
        let task_cancellation = local_cancellation.clone();
        let outer_cancellation = cancellation.clone();
        let (updates, receiver) = tokio_watch::channel(None);
        let task = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            loop {
                tokio::select! {
                    biased;
                    () = outer_cancellation.cancelled() => {
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                        break;
                    }
                    () = task_cancellation.cancelled() => {
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                        break;
                    }
                    line = lines.next_line() => match line {
                        Ok(Some(line)) => match serde_json::from_str::<hachimi_protocol::FsSearchSnapshot>(&line) {
                            Ok(snapshot) => {
                                updates.send_replace(Some(Ok(snapshot)));
                            }
                            Err(error) => {
                                updates.send_replace(Some(Err(WorkspaceError::new(
                                    WorkspaceErrorCode::HostDisconnected,
                                    format!("invalid workspace search snapshot: {error}"),
                                ))));
                                let _ = child.kill().await;
                                let _ = child.wait().await;
                                break;
                            }
                        },
                        Ok(None) => {
                            let status = child.wait().await;
                            if !task_cancellation.is_cancelled() && !outer_cancellation.is_cancelled() {
                                let detail = status.map_or_else(
                                    |error| error.to_string(),
                                    |status| format!("exit status {status}"),
                                );
                                updates.send_replace(Some(Err(WorkspaceError::new(
                                    WorkspaceErrorCode::HostDisconnected,
                                    format!("workspace search worker disconnected: {detail}"),
                                ))));
                            }
                            break;
                        }
                        Err(error) => {
                            updates.send_replace(Some(Err(WorkspaceError::new(
                                WorkspaceErrorCode::HostDisconnected,
                                error.to_string(),
                            ))));
                            let _ = child.kill().await;
                            let _ = child.wait().await;
                            break;
                        }
                    }
                }
            }
        });
        Ok(WorkspaceSearchSession {
            search_id,
            control: Arc::new(WorkspaceSearchControl {
                stdin: AsyncMutex::new(Some(stdin)),
                cancellation: local_cancellation,
            }),
            updates: receiver,
            task: Arc::new(AsyncMutex::new(Some(task))),
        })
    }

    pub async fn execute(
        &self,
        operation: WorkspaceOperation,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<WorkspaceOutput, WorkspaceError> {
        if cancellation.is_cancelled() {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::Cancelled,
                "workspace operation was cancelled before dispatch",
            ));
        }
        let effect = workspace_operation_effect(&operation);
        if effect != hachimi_protocol::ToolEffect::ReadOnly && self.sandbox_backend.is_some() {
            return self
                .execute_sandboxed(operation, effect, timeout, cancellation)
                .await;
        }
        let request = WorkspaceRequestEnvelope {
            request_id: Uuid::new_v4().to_string(),
            checkout_id: self.checkout_id.clone(),
            run_generation: self.run_generation,
            worker_token: self.worker_token.clone(),
            operation,
        };
        let encoded = serde_json::to_vec(&request).map_err(|error| {
            WorkspaceError::new(WorkspaceErrorCode::InvalidRequest, error.to_string())
        })?;
        let mut command = self.worker_command();
        self.prepare_run_temp()?;
        command
            .arg("--root")
            .arg(&self.checkout_root)
            .arg("--checkout-id")
            .arg(&self.checkout_id)
            .arg("--generation")
            .arg(self.run_generation.to_string())
            .current_dir(&self.checkout_root)
            .env(WORKER_TOKEN_ENV, &self.worker_token)
            .env("TEMP", &self.run_temp.path)
            .env("TMP", &self.run_temp.path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|error| {
            WorkspaceError::new(WorkspaceErrorCode::HostDisconnected, error.to_string())
        })?;
        let mut stdin = child.stdin.take().ok_or_else(|| {
            WorkspaceError::new(
                WorkspaceErrorCode::HostDisconnected,
                "workspace worker stdin is unavailable",
            )
        })?;
        let exchange = async move {
            stdin.write_all(&encoded).await.map_err(|error| {
                WorkspaceError::new(WorkspaceErrorCode::HostDisconnected, error.to_string())
            })?;
            stdin.write_all(b"\n").await.map_err(|error| {
                WorkspaceError::new(WorkspaceErrorCode::HostDisconnected, error.to_string())
            })?;
            stdin.shutdown().await.map_err(|error| {
                WorkspaceError::new(WorkspaceErrorCode::HostDisconnected, error.to_string())
            })?;
            drop(stdin);
            let output = child.wait_with_output().await.map_err(|error| {
                WorkspaceError::new(WorkspaceErrorCode::HostDisconnected, error.to_string())
            })?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(WorkspaceError::new(
                    WorkspaceErrorCode::HostDisconnected,
                    format!(
                        "workspace worker exited unsuccessfully: {}",
                        bounded(&stderr, 512)
                    ),
                ));
            }
            let response: WorkspaceResponseEnvelope = serde_json::from_slice(&output.stdout)
                .map_err(|error| {
                    WorkspaceError::new(
                        WorkspaceErrorCode::HostDisconnected,
                        format!("invalid workspace worker response: {error}"),
                    )
                })?;
            if response.request_id != request.request_id {
                return Err(WorkspaceError::new(
                    WorkspaceErrorCode::HostDisconnected,
                    "workspace worker response ID mismatch",
                ));
            }
            if let Some(error) = response.error {
                return Err(WorkspaceError::new(error.code, error.message));
            }
            response.output.ok_or_else(|| {
                WorkspaceError::new(
                    WorkspaceErrorCode::HostDisconnected,
                    "workspace worker returned neither output nor error",
                )
            })
        };
        tokio::select! {
            () = cancellation.cancelled() => Err(WorkspaceError::new(
                WorkspaceErrorCode::Cancelled,
                "workspace operation was cancelled",
            )),
            result = tokio::time::timeout(timeout, exchange) => match result {
                Ok(result) => result,
                Err(_) => Err(WorkspaceError::new(
                    WorkspaceErrorCode::TimedOut,
                    "workspace host request timed out",
                )),
            }
        }
    }

    async fn execute_sandboxed(
        &self,
        operation: WorkspaceOperation,
        effect: hachimi_protocol::ToolEffect,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<WorkspaceOutput, WorkspaceError> {
        let backend = self.sandbox_backend.as_ref().ok_or_else(|| {
            WorkspaceError::new(
                WorkspaceErrorCode::Unauthorized,
                "restricted Workspace dispatch has no Sandbox backend",
            )
        })?;
        let context = self.sandbox_context.as_ref().ok_or_else(|| {
            WorkspaceError::new(
                WorkspaceErrorCode::Unauthorized,
                "restricted Workspace dispatch has no Run grant context",
            )
        })?;
        let guard = self.launch_guard.as_ref().ok_or_else(|| {
            WorkspaceError::new(
                WorkspaceErrorCode::Unauthorized,
                "restricted Workspace dispatch has no generation guard",
            )
        })?;
        guard
            .validate(WorkspaceLaunchCheck {
                session_id: context.session_id.clone(),
                run_id: context.run_id.clone(),
                run_generation: self.run_generation,
                checkout_id: hachimi_protocol::CheckoutId::new(self.checkout_id.clone()),
                effect,
            })
            .await?;
        self.prepare_run_temp()?;
        let request = WorkspaceRequestEnvelope {
            request_id: Uuid::new_v4().to_string(),
            checkout_id: self.checkout_id.clone(),
            run_generation: self.run_generation,
            worker_token: self.worker_token.clone(),
            operation,
        };
        let mut input = serde_json::to_vec(&request).map_err(|error| {
            WorkspaceError::new(WorkspaceErrorCode::InvalidRequest, error.to_string())
        })?;
        input.push(b'\n');
        let git_aliases = git_alias::RestrictedGitAliases::for_checkout(&self.checkout_root)?;
        let worker_root = git_aliases
            .as_ref()
            .map_or(self.checkout_root.as_path(), |aliases| {
                aliases.workspace_root()
            });
        let root_identity = git_aliases
            .as_ref()
            .map(|aliases| hachimi_sandbox::path_file_identity(aliases.real_workspace_root()))
            .transpose()
            .map_err(path_security_error)?;
        let mut worker_args = vec![
            "--root".into(),
            worker_root.as_os_str().to_owned(),
            "--checkout-id".into(),
            self.checkout_id.clone().into(),
            "--generation".into(),
            self.run_generation.to_string().into(),
        ];
        if let Some(identity) = root_identity {
            worker_args.extend([
                "--root-volume-serial".into(),
                identity.volume_serial_number.to_string().into(),
                "--root-file-id".into(),
                identity.file_index.to_string().into(),
            ]);
        }
        let spec = SandboxLaunchSpec {
            session_id: context.session_id.clone(),
            run_id: context.run_id.clone(),
            run_generation: self.run_generation,
            checkout_id: hachimi_protocol::CheckoutId::new(self.checkout_id.clone()),
            checkout_root: self.checkout_root.clone(),
            grants: context.grants.clone(),
            required_effect: effect,
            executable: self.worker_program.clone(),
            args: worker_args,
            cwd: self.checkout_root.clone(),
            environment: restricted_worker_environment(
                &self.run_temp.path,
                &self.worker_token,
                git_aliases.as_ref(),
            )?,
            stdin: Some(input),
            interactive_stdin: false,
            timeout,
            output_limit: 8 * 1024 * 1024,
            network_policy: SandboxNetworkPolicy::DenyAll,
        };
        let child = backend
            .spawn_restricted(spec, cancellation)
            .await
            .map_err(sandbox_error)?;
        let output = child.wait().await.map_err(sandbox_error)?;
        if output.exit_code != Some(0) {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::HostDisconnected,
                format!(
                    "restricted workspace worker exited unsuccessfully: {}",
                    bounded(&String::from_utf8_lossy(&output.stderr), 512)
                ),
            ));
        }
        let response: WorkspaceResponseEnvelope =
            serde_json::from_slice(&output.stdout).map_err(|error| {
                WorkspaceError::new(
                    WorkspaceErrorCode::HostDisconnected,
                    format!("invalid restricted workspace response: {error}"),
                )
            })?;
        if response.request_id != request.request_id {
            let detail = response
                .error
                .as_ref()
                .map(|error| bounded(&error.message, 256))
                .unwrap_or_else(|| "response did not include a worker error".into());
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::HostDisconnected,
                format!("restricted workspace response ID mismatch: {detail}"),
            ));
        }
        if let Some(error) = response.error {
            return Err(WorkspaceError::new(error.code, error.message));
        }
        response.output.ok_or_else(|| {
            WorkspaceError::new(
                WorkspaceErrorCode::HostDisconnected,
                "restricted workspace worker returned neither output nor error",
            )
        })
    }

    fn worker_command(&self) -> Command {
        let mut command = if let Some(launcher) = &self.restricted_launcher {
            let mut command = Command::new(launcher);
            command.arg("--").arg(&self.worker_program);
            command
        } else {
            Command::new(&self.worker_program)
        };
        hide_background_window(&mut command);
        command.env_clear();
        copy_process_environment(&mut command);
        command
    }

    fn prepare_run_temp(&self) -> Result<(), WorkspaceError> {
        std::fs::create_dir_all(&self.run_temp.path).map_err(|error| {
            WorkspaceError::new(
                WorkspaceErrorCode::HostDisconnected,
                format!("run TEMP could not be prepared: {error}"),
            )
        })
    }
}

#[cfg(windows)]
fn hide_background_window(command: &mut Command) {
    use std::os::windows::process::CommandExt as _;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.as_std_mut().creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_background_window(_command: &mut Command) {}

#[derive(Debug)]
struct RunTempDirectory {
    base: PathBuf,
    path: PathBuf,
}

impl RunTempDirectory {
    fn new() -> Self {
        let base = std::env::temp_dir().join("hachimi-agent-runs");
        let path = base.join(format!("run-{}", Uuid::now_v7()));
        Self { base, path }
    }
}

impl Drop for RunTempDirectory {
    fn drop(&mut self) {
        let safe_name = self
            .path
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with("run-") && name.len() <= 64);
        if safe_name && self.path.parent() == Some(self.base.as_path()) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

fn restricted_worker_environment(
    run_temp: &Path,
    worker_token: &str,
    git_aliases: Option<&git_alias::RestrictedGitAliases>,
) -> Result<Vec<(std::ffi::OsString, std::ffi::OsString)>, WorkspaceError> {
    let mut environment = Vec::new();
    for name in [
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "SYSTEMDRIVE",
        "WINDIR",
        "COMSPEC",
        "CARGO_HOME",
        "RUSTUP_HOME",
    ] {
        if let Some(value) = std::env::var_os(name) {
            environment.push((name.into(), value));
        }
    }
    environment.push(("TEMP".into(), run_temp.as_os_str().to_owned()));
    environment.push(("TMP".into(), run_temp.as_os_str().to_owned()));
    environment.push(("USERPROFILE".into(), run_temp.as_os_str().to_owned()));
    environment.push(("LOCALAPPDATA".into(), run_temp.as_os_str().to_owned()));
    environment.push(("APPDATA".into(), run_temp.as_os_str().to_owned()));
    environment.push((WORKER_TOKEN_ENV.into(), worker_token.into()));
    let git = hachimi_sandbox::trusted_git_executable().map_err(|error| {
        WorkspaceError::new(
            WorkspaceErrorCode::HostDisconnected,
            format!("trusted Git runtime is unavailable: {error}"),
        )
    })?;
    environment.push((GIT_EXECUTABLE_ENV.into(), git.into_os_string()));
    if let Some(git_aliases) = git_aliases {
        git_aliases.append_environment(&mut environment);
    }
    Ok(environment)
}

pub(crate) fn git_program() -> std::ffi::OsString {
    std::env::var_os(GIT_EXECUTABLE_ENV).unwrap_or_else(|| "git".into())
}

pub(crate) fn restricted_process_cwd(fallback: &Path) -> PathBuf {
    let Some(alias_root) = std::env::var_os(git_alias::GIT_WORK_TREE_ALIAS_ENV).map(PathBuf::from)
    else {
        return fallback.to_owned();
    };
    let Some(real_root) = std::env::var_os(git_alias::GIT_WORK_TREE_REAL_ENV).map(PathBuf::from)
    else {
        return fallback.to_owned();
    };
    let Ok(relative) = fallback.strip_prefix(real_root) else {
        return fallback.to_owned();
    };
    alias_root.join(relative)
}

fn configure_restricted_git_environment(command: &mut Command) {
    if let Some(value) = std::env::var_os(git_alias::GIT_DIR_ALIAS_ENV) {
        command.env("GIT_DIR", value);
    }
    if let Some(value) = std::env::var_os(git_alias::GIT_WORK_TREE_ALIAS_ENV) {
        command.env("GIT_WORK_TREE", value);
    }
}

fn configure_restricted_std_git_environment(command: &mut std::process::Command) {
    if let Some(value) = std::env::var_os(git_alias::GIT_DIR_ALIAS_ENV) {
        command.env("GIT_DIR", value);
    }
    if let Some(value) = std::env::var_os(git_alias::GIT_WORK_TREE_ALIAS_ENV) {
        command.env("GIT_WORK_TREE", value);
    }
}

fn sandbox_error(error: hachimi_sandbox::SandboxError) -> WorkspaceError {
    let code = match error {
        hachimi_sandbox::SandboxError::Cancelled => WorkspaceErrorCode::Cancelled,
        hachimi_sandbox::SandboxError::TimedOut => WorkspaceErrorCode::TimedOut,
        hachimi_sandbox::SandboxError::GrantDenied
        | hachimi_sandbox::SandboxError::InvalidBinding(_)
        | hachimi_sandbox::SandboxError::NotEnforced(_)
        | hachimi_sandbox::SandboxError::RuntimeUnavailable
        | hachimi_sandbox::SandboxError::ForbiddenEnvironment(_)
        | hachimi_sandbox::SandboxError::ConflictingStdinMode => WorkspaceErrorCode::Unauthorized,
        hachimi_sandbox::SandboxError::Spawn(_) | hachimi_sandbox::SandboxError::AlreadyWaited => {
            WorkspaceErrorCode::HostDisconnected
        }
    };
    WorkspaceError::new(code, error.to_string())
}

#[derive(Debug)]
pub struct WorkspaceWatchSession {
    pub registration: hachimi_protocol::FsWatchRegistration,
    receiver: mpsc::Receiver<Result<hachimi_protocol::FsChangeEvent, WorkspaceError>>,
    cancellation: CancellationToken,
    task: Option<JoinHandle<()>>,
}

struct WorkspaceSearchControl {
    stdin: AsyncMutex<Option<tokio::process::ChildStdin>>,
    cancellation: CancellationToken,
}

impl Drop for WorkspaceSearchControl {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

#[derive(Clone)]
pub struct WorkspaceSearchSession {
    pub search_id: hachimi_protocol::FsSearchId,
    control: Arc<WorkspaceSearchControl>,
    updates:
        tokio_watch::Receiver<Option<Result<hachimi_protocol::FsSearchSnapshot, WorkspaceError>>>,
    task: Arc<AsyncMutex<Option<JoinHandle<()>>>>,
}

impl std::fmt::Debug for WorkspaceSearchSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkspaceSearchSession")
            .field("search_id", &self.search_id)
            .finish_non_exhaustive()
    }
}

impl WorkspaceSearchSession {
    pub async fn update(&self, generation: u64, query: String) -> Result<(), WorkspaceError> {
        let command = SearchServerCommand::Update { generation, query };
        let mut encoded = serde_json::to_vec(&command).map_err(|error| {
            WorkspaceError::new(WorkspaceErrorCode::InvalidRequest, error.to_string())
        })?;
        encoded.push(b'\n');
        let mut stdin = self.control.stdin.lock().await;
        let stdin = stdin.as_mut().ok_or_else(|| {
            WorkspaceError::new(
                WorkspaceErrorCode::HostDisconnected,
                "workspace search command channel is closed",
            )
        })?;
        stdin.write_all(&encoded).await.map_err(|error| {
            WorkspaceError::new(WorkspaceErrorCode::HostDisconnected, error.to_string())
        })?;
        stdin.flush().await.map_err(|error| {
            WorkspaceError::new(WorkspaceErrorCode::HostDisconnected, error.to_string())
        })
    }

    pub async fn wait_for_snapshot(
        &self,
        generation: u64,
        query: &str,
        timeout: Duration,
    ) -> Result<hachimi_protocol::FsSearchSnapshot, WorkspaceError> {
        let mut receiver = self.updates.clone();
        let expected_query = query.trim().to_owned();
        let wait = async move {
            loop {
                if let Some(result) = receiver.borrow_and_update().clone() {
                    let snapshot = result?;
                    if snapshot.generation == generation && snapshot.query == expected_query {
                        return Ok(snapshot);
                    }
                }
                receiver.changed().await.map_err(|_| {
                    WorkspaceError::new(
                        WorkspaceErrorCode::HostDisconnected,
                        "workspace search update channel closed",
                    )
                })?;
            }
        };
        tokio::time::timeout(timeout, wait).await.map_err(|_| {
            WorkspaceError::new(WorkspaceErrorCode::TimedOut, "workspace search timed out")
        })?
    }

    #[must_use]
    pub fn subscribe(
        &self,
    ) -> tokio_watch::Receiver<Option<Result<hachimi_protocol::FsSearchSnapshot, WorkspaceError>>>
    {
        self.updates.clone()
    }

    pub fn cancel(&self) {
        self.control.cancellation.cancel();
    }

    pub async fn join(&self) {
        if let Some(task) = self.task.lock().await.take() {
            let _ = task.await;
        }
    }
}

impl WorkspaceWatchSession {
    pub async fn recv(
        &mut self,
    ) -> Option<Result<hachimi_protocol::FsChangeEvent, WorkspaceError>> {
        self.receiver.recv().await
    }

    pub fn cancel(&self) {
        self.cancellation.cancel();
    }
}

impl Drop for WorkspaceWatchSession {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkerContext {
    root: PathBuf,
    checkout_id: String,
    run_generation: u64,
    worker_token: String,
}

impl WorkerContext {
    pub fn new(
        root: impl AsRef<Path>,
        checkout_id: impl Into<String>,
        run_generation: u64,
        worker_token: impl Into<String>,
    ) -> Result<Self, WorkspaceError> {
        Self::new_with_alias(root, None, checkout_id, run_generation, worker_token)
    }

    pub fn new_with_alias(
        root: impl AsRef<Path>,
        expected_identity: Option<hachimi_sandbox::WindowsFileIdentity>,
        checkout_id: impl Into<String>,
        run_generation: u64,
        worker_token: impl Into<String>,
    ) -> Result<Self, WorkspaceError> {
        let root = match expected_identity {
            Some(expected) => {
                hachimi_sandbox::validate_checkout_alias_root(root.as_ref(), expected)
            }
            None => validate_checkout_root(root.as_ref()),
        }
        .map_err(path_security_error)?;
        Ok(Self {
            root,
            checkout_id: checkout_id.into(),
            run_generation,
            worker_token: worker_token.into(),
        })
    }

    pub async fn handle(&self, request: WorkspaceRequestEnvelope) -> WorkspaceResponseEnvelope {
        let request_id = request.request_id.clone();
        if request.worker_token != self.worker_token || request.checkout_id != self.checkout_id {
            return WorkspaceResponseEnvelope::failure(
                request_id,
                WorkspaceError::new(
                    WorkspaceErrorCode::Unauthorized,
                    "workspace worker token or checkout binding is invalid",
                ),
            );
        }
        if request.run_generation != self.run_generation {
            return WorkspaceResponseEnvelope::failure(
                request_id,
                WorkspaceError::new(
                    WorkspaceErrorCode::StaleGeneration,
                    "workspace request belongs to a stale run generation",
                ),
            );
        }
        // Keep the operation dispatcher future off the worker's small Windows main-thread
        // stack. Some bounded streaming operations intentionally carry sizeable byte buffers;
        // inlining their state into the top-level worker future can otherwise overflow before a
        // lightweight operation (for example a Git snapshot) is polled.
        match Box::pin(self.execute(request.operation)).await {
            Ok(output) => WorkspaceResponseEnvelope::success(request_id, output),
            Err(error) => WorkspaceResponseEnvelope::failure(request_id, error),
        }
    }

    async fn execute(
        &self,
        operation: WorkspaceOperation,
    ) -> Result<WorkspaceOutput, WorkspaceError> {
        match operation {
            WorkspaceOperation::ReadFile { path } => self.read_file(&path),
            WorkspaceOperation::ListDirectory { path } => self.list_directory(&path),
            WorkspaceOperation::ListDirectoryPage {
                path,
                cursor,
                limit,
            } => self.list_directory_page(&path, cursor.as_deref(), limit),
            WorkspaceOperation::ReadFileChunk {
                path,
                offset,
                limit,
                if_match,
            } => self.read_file_chunk(&path, offset, limit, if_match.as_deref()),
            WorkspaceOperation::FuzzyFileSearch {
                query,
                max_results,
                search_id,
                generation,
            } => self.fuzzy_file_search(&query, max_results, search_id, generation),
            WorkspaceOperation::SearchText {
                path,
                query,
                case_sensitive,
                max_results,
            } => self.search_text(&path, &query, case_sensitive, max_results),
            WorkspaceOperation::WriteFile {
                path,
                content,
                expected_sha256,
            } => self.write_file(&path, &content, expected_sha256.as_deref()),
            WorkspaceOperation::ReplaceText {
                path,
                old_text,
                new_text,
                expected_sha256,
                replace_all,
            } => self.replace_text(&path, &old_text, &new_text, &expected_sha256, replace_all),
            WorkspaceOperation::ApplyPatch { patch } => self.apply_patch(&patch),
            WorkspaceOperation::GitStatus => {
                self.run_process("git", &["status", "--short"], &self.root, 30_000)
                    .await
            }
            WorkspaceOperation::GitDiff => {
                self.run_process(
                    "git",
                    &["diff", "--no-textconv", "--no-ext-diff"],
                    &self.root,
                    30_000,
                )
                .await
            }
            WorkspaceOperation::GitReviewDiff { target } => self.git_review_diff(&target).await,
            WorkspaceOperation::GitDiffStructured {
                scope,
                base_revision,
            } => {
                self.git_diff_structured(scope, base_revision.as_deref())
                    .await
            }
            WorkspaceOperation::GitDiffFileChunk {
                scope,
                path,
                base_revision,
                offset,
                limit,
                if_match,
            } => {
                self.git_diff_file_chunk(
                    scope,
                    &path,
                    base_revision.as_deref(),
                    offset,
                    limit,
                    if_match.as_deref(),
                )
                .await
            }
            WorkspaceOperation::GitStatusSnapshot => self.git_status_snapshot().await,
            WorkspaceOperation::GitWorkspaceSnapshot { history_limit } => {
                self.git_workspace_snapshot(history_limit).await
            }
            WorkspaceOperation::GitProjectInspect { project_id } => {
                self.git_project_inspect(project_id).await
            }
            WorkspaceOperation::GitStage {
                paths,
                history_limit,
            } => self.git_stage(&paths, history_limit).await,
            WorkspaceOperation::GitUnstage {
                paths,
                history_limit,
            } => self.git_unstage(&paths, history_limit).await,
            WorkspaceOperation::GitCommit {
                message,
                history_limit,
            } => self.git_commit(&message, history_limit).await,
            WorkspaceOperation::GitCreateEmptyInitialCommit {
                author_name,
                author_email,
                history_limit,
            } => {
                self.git_create_empty_initial_commit(&author_name, &author_email, history_limit)
                    .await
            }
            WorkspaceOperation::ReadGitBlob { path } => self.read_git_blob(&path).await,
            WorkspaceOperation::Exec {
                program,
                args,
                cwd,
                timeout_ms,
            } => {
                let cwd = self.resolve_existing(&cwd)?;
                if !cwd.is_dir() {
                    return Err(WorkspaceError::new(
                        WorkspaceErrorCode::NotFound,
                        "execution cwd is not a directory",
                    ));
                }
                let borrowed = args.iter().map(String::as_str).collect::<Vec<_>>();
                self.run_process(&program, &borrowed, &cwd, timeout_ms.clamp(1, 120_000))
                    .await
            }
        }
    }

    fn read_file(&self, path: &str) -> Result<WorkspaceOutput, WorkspaceError> {
        let resolved = self.resolve_existing(path)?;
        let bytes = read_bounded(&resolved)?;
        let content = String::from_utf8(bytes.clone()).map_err(|_| {
            WorkspaceError::new(WorkspaceErrorCode::NotText, "file is not valid UTF-8 text")
        })?;
        Ok(WorkspaceOutput::File {
            path: relative_display(&self.root, &resolved),
            content,
            sha256: sha256(&bytes),
            byte_size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        })
    }

    fn list_directory(&self, path: &str) -> Result<WorkspaceOutput, WorkspaceError> {
        let resolved = self.resolve_existing(path)?;
        if !resolved.is_dir() {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::NotFound,
                "path is not a directory",
            ));
        }
        let mut entries = std::fs::read_dir(&resolved)
            .map_err(io_error)?
            .take(1_000)
            .map(|entry| {
                let entry = entry.map_err(io_error)?;
                let metadata = std::fs::symlink_metadata(entry.path()).map_err(io_error)?;
                let kind = if metadata.file_type().is_symlink() {
                    WorkspaceEntryKind::Symlink
                } else if metadata.is_file() {
                    WorkspaceEntryKind::File
                } else if metadata.is_dir() {
                    WorkspaceEntryKind::Directory
                } else {
                    WorkspaceEntryKind::Other
                };
                Ok(WorkspaceEntry {
                    path: relative_display(&self.root, &entry.path()),
                    kind,
                    byte_size: metadata.is_file().then_some(metadata.len()),
                })
            })
            .collect::<Result<Vec<_>, WorkspaceError>>()?;
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(WorkspaceOutput::Directory {
            path: relative_display(&self.root, &resolved),
            entries,
        })
    }

    fn search_text(
        &self,
        path: &str,
        query: &str,
        case_sensitive: bool,
        max_results: usize,
    ) -> Result<WorkspaceOutput, WorkspaceError> {
        if query.is_empty() || query.chars().count() > 512 {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::InvalidRequest,
                "search query must contain 1-512 characters",
            ));
        }
        let resolved = self.resolve_existing(path)?;
        let mut state = SearchState {
            root: &self.root,
            query,
            folded_query: (!case_sensitive).then(|| query.to_lowercase()),
            max_results: max_results.clamp(1, MAX_SEARCH_RESULTS),
            visited_files: 0,
            matches: Vec::new(),
            truncated: false,
        };
        search_path(&resolved, &mut state)?;
        Ok(WorkspaceOutput::Search {
            matches: state.matches,
            truncated: state.truncated,
        })
    }

    fn write_file(
        &self,
        path: &str,
        content: &str,
        expected_sha256: Option<&str>,
    ) -> Result<WorkspaceOutput, WorkspaceError> {
        ensure_content_size(content)?;
        let resolved = self.resolve_write(path)?;
        if resolved.exists() {
            let current = read_bounded(&resolved)?;
            let current_hash = sha256(&current);
            match expected_sha256 {
                Some(expected) if constant_time_eq(expected, &current_hash) => {}
                Some(_) => {
                    return Err(WorkspaceError::new(
                        WorkspaceErrorCode::Conflict,
                        "file changed after it was read",
                    ));
                }
                None => {
                    return Err(WorkspaceError::new(
                        WorkspaceErrorCode::Conflict,
                        "overwriting an existing file requires its expected SHA-256",
                    ));
                }
            }
        } else if expected_sha256.is_some() {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::Conflict,
                "expected file does not exist",
            ));
        }
        atomic_write(&resolved, content.as_bytes())?;
        Ok(write_output(&self.root, &resolved, content.as_bytes(), 0))
    }

    fn replace_text(
        &self,
        path: &str,
        old_text: &str,
        new_text: &str,
        expected_sha256: &str,
        replace_all: bool,
    ) -> Result<WorkspaceOutput, WorkspaceError> {
        if old_text.is_empty() {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::InvalidRequest,
                "replacement source text must not be empty",
            ));
        }
        let resolved = self.resolve_existing(path)?;
        let bytes = read_bounded(&resolved)?;
        if !constant_time_eq(expected_sha256, &sha256(&bytes)) {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::Conflict,
                "file changed after it was read",
            ));
        }
        let content = String::from_utf8(bytes).map_err(|_| {
            WorkspaceError::new(WorkspaceErrorCode::NotText, "file is not valid UTF-8 text")
        })?;
        let occurrences = content.matches(old_text).count();
        if occurrences == 0 {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::Conflict,
                "replacement source text was not found",
            ));
        }
        if occurrences > 1 && !replace_all {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::Conflict,
                "replacement source text is not unique",
            ));
        }
        let updated = if replace_all {
            content.replace(old_text, new_text)
        } else {
            content.replacen(old_text, new_text, 1)
        };
        ensure_content_size(&updated)?;
        atomic_write(&resolved, updated.as_bytes())?;
        Ok(write_output(
            &self.root,
            &resolved,
            updated.as_bytes(),
            if replace_all { occurrences } else { 1 },
        ))
    }

    async fn run_process(
        &self,
        program: &str,
        args: &[&str],
        cwd: &Path,
        timeout_ms: u64,
    ) -> Result<WorkspaceOutput, WorkspaceError> {
        if program.trim().is_empty() || program.chars().count() > 260 || args.len() > 128 {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::InvalidRequest,
                "invalid process program or argument count",
            ));
        }
        let restricted_git =
            program.eq_ignore_ascii_case("git") || program.eq_ignore_ascii_case("git.exe");
        let mut command = Command::new(if restricted_git {
            git_program()
        } else {
            std::ffi::OsString::from(program)
        });
        command
            .args(args)
            .current_dir(restricted_process_cwd(cwd))
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        copy_process_environment(&mut command);
        let output = tokio::time::timeout(Duration::from_millis(timeout_ms), command.output())
            .await
            .map_err(|_| WorkspaceError::new(WorkspaceErrorCode::TimedOut, "process timed out"))?
            .map_err(|error| {
                WorkspaceError::new(WorkspaceErrorCode::ProcessFailed, error.to_string())
            })?;
        let (stdout, stdout_truncated) = bounded_bytes(&output.stdout);
        let (stderr, stderr_truncated) = bounded_bytes(&output.stderr);
        Ok(WorkspaceOutput::Process {
            exit_code: output.status.code(),
            stdout,
            stderr,
            truncated: stdout_truncated || stderr_truncated,
        })
    }

    fn resolve_existing(&self, relative: &str) -> Result<PathBuf, WorkspaceError> {
        resolve_checkout_path(&self.root, relative, PathAccess::Read, false)
            .map_err(path_security_error)
    }

    fn resolve_write(&self, relative: &str) -> Result<PathBuf, WorkspaceError> {
        resolve_checkout_path(&self.root, relative, PathAccess::Write, true)
            .map_err(path_security_error)
    }
}

struct SearchState<'a> {
    root: &'a Path,
    query: &'a str,
    folded_query: Option<String>,
    max_results: usize,
    visited_files: usize,
    matches: Vec<SearchMatch>,
    truncated: bool,
}

fn search_path(path: &Path, state: &mut SearchState<'_>) -> Result<(), WorkspaceError> {
    if state.matches.len() >= state.max_results || state.visited_files >= MAX_SEARCHED_FILES {
        state.truncated = true;
        return Ok(());
    }
    let metadata = std::fs::symlink_metadata(path).map_err(io_error)?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_dir() {
        let mut entries = std::fs::read_dir(path)
            .map_err(io_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(io_error)?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            search_path(&entry.path(), state)?;
            if state.truncated {
                break;
            }
        }
        return Ok(());
    }
    if !metadata.is_file() || metadata.len() > MAX_TEXT_BYTES {
        return Ok(());
    }
    state.visited_files += 1;
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or_default());
    std::fs::File::open(path)
        .map_err(io_error)?
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    let Ok(content) = String::from_utf8(bytes) else {
        return Ok(());
    };
    for (index, line) in content.lines().enumerate() {
        let matches = if let Some(query) = &state.folded_query {
            line.to_lowercase().contains(query)
        } else {
            line.contains(state.query)
        };
        if matches {
            state.matches.push(SearchMatch {
                path: relative_display(state.root, path),
                line: index + 1,
                text: bounded(line, 500),
            });
            if state.matches.len() >= state.max_results {
                state.truncated = true;
                break;
            }
        }
    }
    Ok(())
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, WorkspaceError> {
    let metadata = std::fs::metadata(path).map_err(io_error)?;
    if !metadata.is_file() {
        return Err(WorkspaceError::new(
            WorkspaceErrorCode::NotFound,
            "workspace path is not a file",
        ));
    }
    if metadata.len() > MAX_TEXT_BYTES {
        return Err(WorkspaceError::new(
            WorkspaceErrorCode::TooLarge,
            "workspace file exceeds the 2 MiB text limit",
        ));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or_default());
    std::fs::File::open(path)
        .map_err(io_error)?
        .take(MAX_TEXT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_TEXT_BYTES {
        return Err(WorkspaceError::new(
            WorkspaceErrorCode::TooLarge,
            "workspace file exceeds the 2 MiB text limit",
        ));
    }
    Ok(bytes)
}

fn ensure_content_size(content: &str) -> Result<(), WorkspaceError> {
    if u64::try_from(content.len()).unwrap_or(u64::MAX) > MAX_TEXT_BYTES {
        return Err(WorkspaceError::new(
            WorkspaceErrorCode::TooLarge,
            "workspace content exceeds the 2 MiB text limit",
        ));
    }
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), WorkspaceError> {
    let mut file = AtomicWriteFile::open(path).map_err(io_error)?;
    file.write_all(bytes).map_err(io_error)?;
    file.commit().map_err(io_error)
}

fn write_output(root: &Path, path: &Path, bytes: &[u8], replacements: usize) -> WorkspaceOutput {
    WorkspaceOutput::Write {
        path: relative_display(root, path),
        sha256: sha256(bytes),
        byte_size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        replacements,
    }
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn io_error(error: std::io::Error) -> WorkspaceError {
    WorkspaceError::new(WorkspaceErrorCode::Io, error.to_string())
}

fn path_security_error(error: PathSecurityError) -> WorkspaceError {
    match error {
        PathSecurityError::NotFound => {
            WorkspaceError::new(WorkspaceErrorCode::NotFound, error.to_string())
        }
        PathSecurityError::Io(error) => io_error(error),
        PathSecurityError::UnsupportedRoot
        | PathSecurityError::EscapesCheckout
        | PathSecurityError::UnsupportedPathForm
        | PathSecurityError::ReservedDeviceName
        | PathSecurityError::ReparsePoint
        | PathSecurityError::HardLink => {
            WorkspaceError::new(WorkspaceErrorCode::PathOutsideCheckout, error.to_string())
        }
    }
}

fn bounded(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn bounded_bytes(bytes: &[u8]) -> (String, bool) {
    let truncated = bytes.len() > MAX_PROCESS_OUTPUT_BYTES;
    let bytes = &bytes[..bytes.len().min(MAX_PROCESS_OUTPUT_BYTES)];
    (String::from_utf8_lossy(bytes).into_owned(), truncated)
}

fn copy_process_environment(command: &mut Command) {
    const ALLOWED: &[&str] = &[
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "WINDIR",
        "TEMP",
        "TMP",
        "USERPROFILE",
        "LOCALAPPDATA",
        "APPDATA",
        "CARGO_HOME",
        "RUSTUP_HOME",
    ];
    for key in ALLOWED {
        if let Some(value) = std::env::var_os(key) {
            command.env(OsStr::new(key), value);
        }
    }
    configure_restricted_git_environment(command);
}

#[cfg(test)]
mod tests;
