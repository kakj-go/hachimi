// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex app-server/src/request_processors/process_exec_processor.rs,
// exec-server/src/local_process.rs, and exec-server/src/process.rs.
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// Modified for Hachimi: ProcessSession IDs, Checkout/Run bindings, bounded replay, and TTL detach.

mod pty;

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex as StdMutex, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use hachimi_protocol::{
    ClientId, ProcessEvent, ProcessOutputChunk, ProcessOutputStream, ProcessReadSnapshot,
    ProcessSessionId, ProcessSessionRecord, ProcessStatus, ProcessTerminalSize,
};
use thiserror::Error;
use tokio::sync::{broadcast, mpsc, oneshot};

const OUTPUT_EVENT_CAPACITY: usize = 50_000;
const RETAINED_OUTPUT_BYTES: usize = 1024 * 1024;
const DEFAULT_READ_BYTES: usize = 256 * 1024;
const MAX_READ_BYTES: usize = 1024 * 1024;
const MAX_ACCEPTED_WRITE_IDS: usize = 4_096;

#[derive(Debug, Clone)]
pub struct ProcessLaunchSpec {
    pub record: ProcessSessionRecord,
    pub restricted_launcher: Option<PathBuf>,
    pub command: Vec<String>,
    pub cwd: PathBuf,
    pub environment: BTreeMap<String, String>,
    pub tty: bool,
    pub stream_stdin: bool,
    pub output_bytes_cap: usize,
    pub timeout: Option<Duration>,
    pub size: ProcessTerminalSize,
    pub reconnect_ttl: Duration,
}

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("invalid process request: {0}")]
    InvalidRequest(&'static str),
    #[error("process session already exists: {0}")]
    AlreadyExists(ProcessSessionId),
    #[error("process session does not exist: {0}")]
    NotFound(ProcessSessionId),
    #[error("process session is no longer running")]
    NotRunning,
    #[error("process session is not interactive")]
    NotInteractive,
    #[error("process stdin is not enabled")]
    StdinDisabled,
    #[error("process stdin is closed")]
    StdinClosed,
    #[error("process owner does not match the active client")]
    OwnerMismatch,
    #[error("process output encoding is invalid")]
    InvalidBase64,
    #[error("process spawn failed: {0}")]
    Spawn(std::io::Error),
    #[error("PTY operation failed: {0}")]
    Pty(String),
    #[error("spawned process stdio is unavailable")]
    MissingStdio,
    #[error("process control channel is closed")]
    ControlClosed,
}

#[derive(Debug)]
struct RuntimeOutput {
    stream: ProcessOutputStream,
    bytes: Vec<u8>,
}

#[derive(Debug)]
enum RuntimeControl {
    Write {
        bytes: Vec<u8>,
        close: bool,
        response: oneshot::Sender<Result<(), ProcessError>>,
    },
    Resize {
        size: ProcessTerminalSize,
        response: oneshot::Sender<Result<(), ProcessError>>,
    },
    Terminate {
        response: oneshot::Sender<Result<(), ProcessError>>,
    },
}

#[derive(Debug)]
struct SpawnedRuntime {
    control_tx: mpsc::Sender<RuntimeControl>,
    output_rx: mpsc::Receiver<RuntimeOutput>,
    exit_rx: oneshot::Receiver<i32>,
}

#[derive(Debug, Clone)]
struct RetainedEvent {
    event: ProcessEvent,
    output_bytes: usize,
}

#[derive(Debug)]
struct ProcessEventLog {
    history: StdMutex<VecDeque<RetainedEvent>>,
    retained_bytes: StdMutex<usize>,
    live: broadcast::Sender<ProcessEvent>,
}

impl Default for ProcessEventLog {
    fn default() -> Self {
        let (live, _) = broadcast::channel(256);
        Self {
            history: StdMutex::new(VecDeque::new()),
            retained_bytes: StdMutex::new(0),
            live,
        }
    }
}

impl ProcessEventLog {
    fn publish(&self, event: ProcessEvent, output_bytes: usize) {
        let mut history = self
            .history
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut retained_bytes = self
            .retained_bytes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *retained_bytes = retained_bytes.saturating_add(output_bytes);
        history.push_back(RetainedEvent {
            event: event.clone(),
            output_bytes,
        });
        while history.len() > OUTPUT_EVENT_CAPACITY || *retained_bytes > RETAINED_OUTPUT_BYTES {
            let Some(removed) = history.pop_front() else {
                break;
            };
            *retained_bytes = retained_bytes.saturating_sub(removed.output_bytes);
        }
        drop(retained_bytes);
        drop(history);
        let _ = self.live.send(event);
    }

    fn subscribe(&self) -> broadcast::Receiver<ProcessEvent> {
        self.live.subscribe()
    }

    fn read_after(&self, after: u64, max_bytes: usize) -> (Vec<ProcessOutputChunk>, u64) {
        let history = self
            .history
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut chunks = Vec::new();
        let mut bytes = 0_usize;
        let mut next_sequence = after;
        for retained in history
            .iter()
            .filter(|event| event_sequence(&event.event) > after)
        {
            let sequence = event_sequence(&retained.event);
            if retained.output_bytes > 0
                && !chunks.is_empty()
                && bytes.saturating_add(retained.output_bytes) > max_bytes
            {
                break;
            }
            next_sequence = sequence;
            if let ProcessEvent::Output { chunk, .. } = &retained.event {
                bytes = bytes.saturating_add(retained.output_bytes);
                chunks.push(chunk.clone());
            }
        }
        (chunks, next_sequence)
    }
}

#[derive(Debug, Default)]
struct AcceptedWrites {
    ids: BTreeSet<String>,
    order: VecDeque<String>,
}

#[derive(Debug)]
struct RestrictedProcessTemp {
    base: PathBuf,
    path: PathBuf,
}

impl RestrictedProcessTemp {
    fn new(process_id: &ProcessSessionId) -> Result<Self, ProcessError> {
        let name = process_id.as_str();
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(ProcessError::InvalidRequest(
                "process ID is not safe for a TEMP directory",
            ));
        }
        let base = std::env::temp_dir().join("hachimi-process-runs");
        let path = base.join(format!("process-{name}"));
        std::fs::create_dir_all(&path).map_err(ProcessError::Spawn)?;
        hachimi_sandbox::grant_restricted_code_access(&path, true).map_err(ProcessError::Pty)?;
        Ok(Self { base, path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RestrictedProcessTemp {
    fn drop(&mut self) {
        let safe_name = self
            .path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|name| name.starts_with("process-") && name.len() <= 72);
        if safe_name && self.path.parent() == Some(self.base.as_path()) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

fn prepare_restricted_environment(environment: &mut BTreeMap<String, String>, temp: &Path) {
    for key in [
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "SYSTEMDRIVE",
        "WINDIR",
        "COMSPEC",
    ] {
        if !environment
            .keys()
            .any(|existing| existing.eq_ignore_ascii_case(key))
            && let Ok(value) = std::env::var(key)
        {
            environment.insert(key.into(), value);
        }
    }
    const ISOLATED: &[&str] = &["TEMP", "TMP", "USERPROFILE", "LOCALAPPDATA", "APPDATA"];
    environment.retain(|key, _| {
        !ISOLATED
            .iter()
            .any(|isolated| key.eq_ignore_ascii_case(isolated))
    });
    let temp = temp.to_string_lossy().into_owned();
    for key in ISOLATED {
        environment.insert((*key).into(), temp.clone());
    }
}

impl AcceptedWrites {
    fn claim(&mut self, id: &str) -> bool {
        if !self.ids.insert(id.to_owned()) {
            return false;
        }
        self.order.push_back(id.to_owned());
        while self.order.len() > MAX_ACCEPTED_WRITE_IDS {
            if let Some(removed) = self.order.pop_front() {
                self.ids.remove(&removed);
            }
        }
        true
    }

    fn release(&mut self, id: &str) {
        self.ids.remove(id);
        self.order.retain(|value| value != id);
    }
}

#[derive(Debug)]
struct ProcessEntry {
    record: RwLock<ProcessSessionRecord>,
    events: ProcessEventLog,
    control: RwLock<Option<mpsc::Sender<RuntimeControl>>>,
    accepted_writes: StdMutex<AcceptedWrites>,
    sequence: AtomicU64,
    attachment_generation: AtomicU64,
    reconnect_ttl: Duration,
    _restricted_temp: Option<RestrictedProcessTemp>,
}

impl ProcessEntry {
    fn new(
        record: ProcessSessionRecord,
        reconnect_ttl: Duration,
        restricted_temp: Option<RestrictedProcessTemp>,
    ) -> Self {
        Self {
            record: RwLock::new(record),
            events: ProcessEventLog::default(),
            control: RwLock::new(None),
            accepted_writes: StdMutex::new(AcceptedWrites::default()),
            sequence: AtomicU64::new(0),
            attachment_generation: AtomicU64::new(0),
            reconnect_ttl,
            _restricted_temp: restricted_temp,
        }
    }

    fn record(&self) -> ProcessSessionRecord {
        self.record
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn next_sequence(&self) -> u64 {
        self.sequence.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn is_terminal(&self) -> bool {
        matches!(
            self.record().status,
            ProcessStatus::Exited
                | ProcessStatus::Terminated
                | ProcessStatus::Failed
                | ProcessStatus::Expired
        )
    }

    fn set_status(&self, status: ProcessStatus, exit_code: Option<i32>) {
        let mut record = self
            .record
            .write()
            .unwrap_or_else(|error| error.into_inner());
        record.status = status;
        record.exit_code = exit_code;
        record.updated_at_ms = now_ms();
    }

    fn control(&self) -> Result<mpsc::Sender<RuntimeControl>, ProcessError> {
        self.control
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .ok_or(ProcessError::NotRunning)
    }
}

#[derive(Debug, Default, Clone)]
pub struct ProcessRegistry {
    entries: Arc<tokio::sync::Mutex<BTreeMap<ProcessSessionId, Arc<ProcessEntry>>>>,
}

impl ProcessRegistry {
    pub async fn spawn(
        &self,
        mut spec: ProcessLaunchSpec,
    ) -> Result<ProcessSessionRecord, ProcessError> {
        validate_launch_spec(&spec)?;
        let id = spec.record.id.clone();
        let restricted_temp = if spec.restricted_launcher.is_some() {
            let temp = RestrictedProcessTemp::new(&id)?;
            prepare_restricted_environment(&mut spec.environment, temp.path());
            Some(temp)
        } else {
            None
        };
        let entry = Arc::new(ProcessEntry::new(
            spec.record.clone(),
            spec.reconnect_ttl,
            restricted_temp,
        ));
        {
            let mut entries = self.entries.lock().await;
            if entries.contains_key(&id) {
                return Err(ProcessError::AlreadyExists(id));
            }
            entries.insert(id.clone(), Arc::clone(&entry));
        }
        let spawned = pty::spawn_runtime(
            spec.restricted_launcher,
            spec.command,
            spec.cwd,
            spec.environment,
            spec.tty,
            spec.stream_stdin,
            spec.size,
            spec.timeout,
        )
        .await;
        let spawned = match spawned {
            Ok(spawned) => spawned,
            Err(error) => {
                entry.set_status(ProcessStatus::Failed, None);
                return Err(error);
            }
        };
        *entry
            .control
            .write()
            .unwrap_or_else(|error| error.into_inner()) = Some(spawned.control_tx);
        entry.set_status(ProcessStatus::Running, None);
        tokio::spawn(collect_process(
            Arc::clone(&entry),
            spawned.output_rx,
            spawned.exit_rx,
            spec.output_bytes_cap,
        ));
        Ok(entry.record())
    }

    pub async fn get(
        &self,
        process_id: &ProcessSessionId,
    ) -> Result<ProcessSessionRecord, ProcessError> {
        Ok(self.entry(process_id).await?.record())
    }

    pub async fn list(
        &self,
        owner: &ClientId,
        include_terminal: bool,
    ) -> Vec<ProcessSessionRecord> {
        let entries = self.entries.lock().await;
        entries
            .values()
            .map(|entry| entry.record())
            .filter(|record| &record.owner_client_id == owner)
            .filter(|record| include_terminal || !terminal_status(record.status))
            .collect()
    }

    pub async fn has_active_processes(&self) -> bool {
        self.entries.lock().await.values().any(|entry| {
            matches!(
                entry.record().status,
                ProcessStatus::Starting | ProcessStatus::Running
            )
        })
    }

    pub async fn read(
        &self,
        process_id: &ProcessSessionId,
        after_sequence: Option<u64>,
        max_bytes: Option<usize>,
        wait: Option<Duration>,
    ) -> Result<ProcessReadSnapshot, ProcessError> {
        let entry = self.entry(process_id).await?;
        let after = after_sequence.unwrap_or_default();
        let max_bytes = max_bytes
            .unwrap_or(DEFAULT_READ_BYTES)
            .clamp(1, MAX_READ_BYTES);
        let (mut chunks, mut next_sequence) = entry.events.read_after(after, max_bytes);
        if chunks.is_empty()
            && !entry.is_terminal()
            && let Some(wait) = wait.filter(|wait| !wait.is_zero())
        {
            let mut live = entry.events.subscribe();
            let _ = tokio::time::timeout(wait.min(Duration::from_secs(30)), live.recv()).await;
            (chunks, next_sequence) = entry.events.read_after(after, max_bytes);
        }
        Ok(ProcessReadSnapshot {
            process: entry.record(),
            chunks,
            next_sequence,
            closed: entry.is_terminal(),
        })
    }

    pub async fn write_base64(
        &self,
        owner: &ClientId,
        process_id: &ProcessSessionId,
        write_id: &str,
        delta_base64: Option<&str>,
        close_stdin: bool,
    ) -> Result<(), ProcessError> {
        if write_id.trim().is_empty() || write_id.len() > 128 {
            return Err(ProcessError::InvalidRequest(
                "write ID must contain 1-128 bytes",
            ));
        }
        let entry = self.entry(process_id).await?;
        verify_owner_and_running(&entry, owner)?;
        if !entry.record().interactive && delta_base64.is_some() {
            return Err(ProcessError::StdinDisabled);
        }
        let bytes = delta_base64
            .map(|value| {
                STANDARD
                    .decode(value)
                    .map_err(|_| ProcessError::InvalidBase64)
            })
            .transpose()?
            .unwrap_or_default();
        {
            let mut accepted = entry
                .accepted_writes
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if !accepted.claim(write_id) {
                return Ok(());
            }
        }
        let (response_tx, response_rx) = oneshot::channel();
        let result = match entry
            .control()?
            .send(RuntimeControl::Write {
                bytes,
                close: close_stdin,
                response: response_tx,
            })
            .await
        {
            Ok(()) => response_rx
                .await
                .unwrap_or(Err(ProcessError::ControlClosed)),
            Err(_) => Err(ProcessError::ControlClosed),
        };
        if result.is_err() {
            entry
                .accepted_writes
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .release(write_id);
        }
        result
    }

    pub async fn resize(
        &self,
        owner: &ClientId,
        process_id: &ProcessSessionId,
        size: ProcessTerminalSize,
    ) -> Result<(), ProcessError> {
        let entry = self.entry(process_id).await?;
        verify_owner_and_running(&entry, owner)?;
        if !entry.record().interactive {
            return Err(ProcessError::NotInteractive);
        }
        send_control(&entry, |response| RuntimeControl::Resize { size, response }).await
    }

    pub async fn terminate(
        &self,
        owner: &ClientId,
        process_id: &ProcessSessionId,
    ) -> Result<(), ProcessError> {
        let entry = self.entry(process_id).await?;
        verify_owner_and_running(&entry, owner)?;
        entry.set_status(ProcessStatus::Terminated, None);
        send_control(&entry, |response| RuntimeControl::Terminate { response }).await
    }

    pub async fn subscribe(
        &self,
        process_id: &ProcessSessionId,
    ) -> Result<broadcast::Receiver<ProcessEvent>, ProcessError> {
        Ok(self.entry(process_id).await?.events.subscribe())
    }

    pub async fn detach_owner(&self, owner: &ClientId) {
        let entries = self
            .entries
            .lock()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for entry in entries {
            if &entry.record().owner_client_id != owner || entry.is_terminal() {
                continue;
            }
            let generation = entry.attachment_generation.fetch_add(1, Ordering::SeqCst) + 1;
            let expires_at = now_ms()
                .saturating_add(i64::try_from(entry.reconnect_ttl.as_millis()).unwrap_or(i64::MAX));
            entry
                .record
                .write()
                .unwrap_or_else(|error| error.into_inner())
                .reconnect_expires_at_ms = Some(expires_at);
            let entry = Arc::clone(&entry);
            tokio::spawn(async move {
                tokio::time::sleep(entry.reconnect_ttl).await;
                if entry.attachment_generation.load(Ordering::SeqCst) == generation
                    && !entry.is_terminal()
                {
                    entry.set_status(ProcessStatus::Expired, None);
                    if let Ok(control) = entry.control() {
                        let (response, _) = oneshot::channel();
                        let _ = control.send(RuntimeControl::Terminate { response }).await;
                    }
                }
            });
        }
    }

    pub async fn attach_owner(&self, owner: &ClientId) {
        let entries = self
            .entries
            .lock()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for entry in entries {
            if &entry.record().owner_client_id == owner && !entry.is_terminal() {
                entry.attachment_generation.fetch_add(1, Ordering::SeqCst);
                entry
                    .record
                    .write()
                    .unwrap_or_else(|error| error.into_inner())
                    .reconnect_expires_at_ms = None;
            }
        }
    }

    pub async fn shutdown(&self) {
        let entries = self
            .entries
            .lock()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for entry in entries {
            if entry.is_terminal() {
                continue;
            }
            entry.set_status(ProcessStatus::Terminated, None);
            if let Ok(control) = entry.control() {
                let (response, _) = oneshot::channel();
                let _ = control.send(RuntimeControl::Terminate { response }).await;
            }
        }
    }

    async fn entry(&self, id: &ProcessSessionId) -> Result<Arc<ProcessEntry>, ProcessError> {
        self.entries
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| ProcessError::NotFound(id.clone()))
    }
}

async fn send_control(
    entry: &ProcessEntry,
    make: impl FnOnce(oneshot::Sender<Result<(), ProcessError>>) -> RuntimeControl,
) -> Result<(), ProcessError> {
    let (response_tx, response_rx) = oneshot::channel();
    entry
        .control()?
        .send(make(response_tx))
        .await
        .map_err(|_| ProcessError::ControlClosed)?;
    response_rx
        .await
        .unwrap_or(Err(ProcessError::ControlClosed))
}

async fn collect_process(
    entry: Arc<ProcessEntry>,
    mut output: mpsc::Receiver<RuntimeOutput>,
    exit: oneshot::Receiver<i32>,
    output_cap: usize,
) {
    let output_entry = Arc::clone(&entry);
    let output_task = tokio::spawn(async move {
        let mut observed = BTreeMap::from([
            (ProcessOutputStream::Stdout, 0_usize),
            (ProcessOutputStream::Stderr, 0_usize),
        ]);
        while let Some(message) = output.recv().await {
            let seen = observed.entry(message.stream).or_default();
            let remaining = output_cap.saturating_sub(*seen);
            let take = remaining.min(message.bytes.len());
            *seen = seen.saturating_add(take);
            let cap_reached = take < message.bytes.len() || *seen == output_cap;
            if take > 0 {
                let sequence = output_entry.next_sequence();
                let chunk = ProcessOutputChunk {
                    sequence,
                    stream: message.stream,
                    delta_base64: STANDARD.encode(&message.bytes[..take]),
                    cap_reached,
                };
                output_entry.events.publish(
                    ProcessEvent::Output {
                        process_session_id: output_entry.record().id,
                        chunk,
                    },
                    take,
                );
            }
        }
    });
    let exit_code = exit.await.unwrap_or(-1);
    let _ = tokio::time::timeout(Duration::from_secs(2), output_task).await;
    let previous = entry.record().status;
    let terminal = match previous {
        ProcessStatus::Terminated | ProcessStatus::Expired => previous,
        _ if exit_code == 124 => ProcessStatus::Failed,
        _ => ProcessStatus::Exited,
    };
    entry.set_status(terminal, Some(exit_code));
    *entry
        .control
        .write()
        .unwrap_or_else(|error| error.into_inner()) = None;
    let exited_sequence = entry.next_sequence();
    entry.events.publish(
        ProcessEvent::Exited {
            process_session_id: entry.record().id,
            sequence: exited_sequence,
            exit_code,
        },
        0,
    );
    let closed_sequence = entry.next_sequence();
    entry.events.publish(
        ProcessEvent::Closed {
            process_session_id: entry.record().id,
            sequence: closed_sequence,
        },
        0,
    );
}

fn validate_launch_spec(spec: &ProcessLaunchSpec) -> Result<(), ProcessError> {
    if spec.command.is_empty()
        || spec.command.len() > 128
        || spec
            .command
            .iter()
            .any(|value| value.contains('\0') || value.len() > 8_192)
    {
        return Err(ProcessError::InvalidRequest("command vector is invalid"));
    }
    if !spec.cwd.is_absolute()
        || spec.output_bytes_cap == 0
        || spec.output_bytes_cap > 16 * 1024 * 1024
    {
        return Err(ProcessError::InvalidRequest(
            "process limits or cwd are invalid",
        ));
    }
    if spec.record.status != ProcessStatus::Starting
        || spec.record.interactive != spec.tty
        || spec.record.output_limit_bytes
            != u64::try_from(spec.output_bytes_cap).unwrap_or(u64::MAX)
    {
        return Err(ProcessError::InvalidRequest(
            "process record does not match launch",
        ));
    }
    if spec.tty && (spec.size.rows == 0 || spec.size.cols == 0) {
        return Err(ProcessError::InvalidRequest("PTY size must be positive"));
    }
    Ok(())
}

fn verify_owner_and_running(entry: &ProcessEntry, owner: &ClientId) -> Result<(), ProcessError> {
    let record = entry.record();
    if &record.owner_client_id != owner {
        return Err(ProcessError::OwnerMismatch);
    }
    if record.status != ProcessStatus::Running {
        return Err(ProcessError::NotRunning);
    }
    Ok(())
}

fn terminal_status(status: ProcessStatus) -> bool {
    matches!(
        status,
        ProcessStatus::Exited
            | ProcessStatus::Terminated
            | ProcessStatus::Failed
            | ProcessStatus::Expired
    )
}

fn event_sequence(event: &ProcessEvent) -> u64 {
    match event {
        ProcessEvent::Output { chunk, .. } => chunk.sequence,
        ProcessEvent::Exited { sequence, .. } | ProcessEvent::Closed { sequence, .. } => *sequence,
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
