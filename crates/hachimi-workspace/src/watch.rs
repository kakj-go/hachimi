// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/file-watcher/src/{lib.rs,file_watcher_tests.rs}
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// Modified for Hachimi: Checkout-bound sidecar, native Windows backend, JSONL events, and generation fencing.

use std::{
    collections::BTreeSet,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use hachimi_protocol::{FsChangeEvent, FsChangeKind, FsWatchId};
use serde::{Deserialize, Serialize};

use crate::{
    WorkerContext, WorkspaceError, WorkspaceErrorCode, path_security_error, relative_display,
};

const MAX_EVENT_PATHS: usize = 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchServerRequest {
    pub checkout_id: String,
    pub run_generation: u64,
    pub worker_token: String,
    pub watch_id: FsWatchId,
    pub watch_generation: u64,
    pub path: String,
    pub recursive: bool,
    pub interval_ms: u64,
}

pub fn run_watch_server(
    context: &WorkerContext,
    request: &WatchServerRequest,
    mut output: impl Write,
) -> Result<(), WorkspaceError> {
    validate_request(context, request)?;
    let requested = resolve_requested_path(context, &request.path)?;
    let debounce = Duration::from_millis(request.interval_ms.clamp(100, 250));
    let mut registration = NativeWatchRegistration::new(&requested, request.recursive)?;

    loop {
        let first = registration.recv().map_err(|_| {
            WorkspaceError::new(
                WorkspaceErrorCode::HostDisconnected,
                "workspace watcher backend disconnected",
            )
        })?;
        let mut paths = BTreeSet::new();
        let mut kinds = BTreeSet::new();
        let mut invalidated = first.invalidated;
        collect_backend_event(first, &requested, request.recursive, &mut paths, &mut kinds);
        let deadline = Instant::now() + debounce;
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            match registration.recv_timeout(remaining) {
                Ok(event) => {
                    invalidated |= event.invalidated;
                    collect_backend_event(
                        event,
                        &requested,
                        request.recursive,
                        &mut paths,
                        &mut kinds,
                    );
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    invalidated = true;
                    break;
                }
            }
        }

        if registration.should_move_closer(&requested, request.recursive) {
            registration = NativeWatchRegistration::new(&requested, request.recursive)?;
        }
        if paths.is_empty() && !invalidated {
            continue;
        }
        let overflowed = invalidated || paths.len() > MAX_EVENT_PATHS;
        let paths = paths
            .into_iter()
            .take(MAX_EVENT_PATHS)
            .filter(|path| path.starts_with(&context.root))
            .map(|path| relative_display(&context.root, &path))
            .collect();
        write_event(
            &mut output,
            &FsChangeEvent {
                watch_id: request.watch_id.clone(),
                generation: request.watch_generation,
                kind: if overflowed {
                    FsChangeKind::Invalidated
                } else {
                    coalesced_kind(&kinds)
                },
                paths,
                overflowed,
            },
        )?;
        if invalidated {
            return Ok(());
        }
    }
}

fn validate_request(
    context: &WorkerContext,
    request: &WatchServerRequest,
) -> Result<(), WorkspaceError> {
    if request.worker_token != context.worker_token || request.checkout_id != context.checkout_id {
        return Err(WorkspaceError::new(
            WorkspaceErrorCode::Unauthorized,
            "workspace watch token or checkout binding is invalid",
        ));
    }
    if request.run_generation != context.run_generation {
        return Err(WorkspaceError::new(
            WorkspaceErrorCode::StaleGeneration,
            "workspace watch belongs to a stale generation",
        ));
    }
    Ok(())
}

fn resolve_requested_path(
    context: &WorkerContext,
    relative: &str,
) -> Result<PathBuf, WorkspaceError> {
    match context.resolve_existing(relative) {
        Ok(path) => Ok(path),
        Err(error) if error.code == WorkspaceErrorCode::NotFound => {
            hachimi_sandbox::resolve_checkout_path(
                &context.root,
                relative,
                hachimi_sandbox::PathAccess::Read,
                true,
            )
            .map_err(path_security_error)
        }
        Err(error) => Err(error),
    }
}

#[derive(Debug)]
struct BackendEvent {
    kind: FsChangeKind,
    paths: Vec<PathBuf>,
    invalidated: bool,
}

struct NativeWatchRegistration {
    receiver: mpsc::Receiver<BackendEvent>,
    stop: Arc<AtomicBool>,
    actual: PathBuf,
    recursive: bool,
    #[cfg(windows)]
    raw_handle: usize,
    task: Option<thread::JoinHandle<()>>,
}

impl NativeWatchRegistration {
    fn new(requested: &Path, recursive: bool) -> Result<Self, WorkspaceError> {
        let (actual, effective_recursive) = actual_watch_path(requested, recursive);
        let (sender, receiver) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        #[cfg(windows)]
        let (task, raw_handle) = spawn_windows_watch(
            actual.clone(),
            effective_recursive,
            Arc::clone(&stop),
            sender,
        )?;
        #[cfg(not(windows))]
        let task = spawn_portable_watch(
            actual.clone(),
            effective_recursive,
            Arc::clone(&stop),
            sender,
        );
        Ok(Self {
            receiver,
            stop,
            actual,
            recursive: effective_recursive,
            #[cfg(windows)]
            raw_handle,
            task: Some(task),
        })
    }

    fn recv(&self) -> Result<BackendEvent, mpsc::RecvError> {
        self.receiver.recv()
    }

    fn recv_timeout(&self, timeout: Duration) -> Result<BackendEvent, mpsc::RecvTimeoutError> {
        self.receiver.recv_timeout(timeout)
    }

    fn should_move_closer(&self, requested: &Path, recursive: bool) -> bool {
        let (actual, effective_recursive) = actual_watch_path(requested, recursive);
        actual != self.actual || effective_recursive != self.recursive
    }
}

impl Drop for NativeWatchRegistration {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        #[cfg(windows)]
        unsafe {
            let _ = windows_sys::Win32::System::IO::CancelIoEx(
                self.raw_handle as windows_sys::Win32::Foundation::HANDLE,
                std::ptr::null(),
            );
        }
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
    }
}

fn actual_watch_path(requested: &Path, recursive: bool) -> (PathBuf, bool) {
    if requested.exists() {
        return (requested.to_path_buf(), recursive);
    }
    let mut ancestor = requested.parent();
    while let Some(path) = ancestor {
        if path.is_dir() {
            return (path.to_path_buf(), false);
        }
        ancestor = path.parent();
    }
    (requested.to_path_buf(), false)
}

#[cfg(windows)]
fn spawn_windows_watch(
    actual: PathBuf,
    recursive: bool,
    stop: Arc<AtomicBool>,
    sender: mpsc::Sender<BackendEvent>,
) -> Result<(thread::JoinHandle<()>, usize), WorkspaceError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::INVALID_HANDLE_VALUE,
        Storage::FileSystem::{
            CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY, FILE_SHARE_DELETE,
            FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        },
    };

    let path = actual
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(WorkspaceError::new(
            WorkspaceErrorCode::HostDisconnected,
            std::io::Error::last_os_error().to_string(),
        ));
    }
    let raw_handle = handle as usize;
    let task = thread::spawn(move || {
        let handle = raw_handle as windows_sys::Win32::Foundation::HANDLE;
        windows_watch_loop(handle, &actual, recursive, &stop, &sender);
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(handle);
        }
    });
    Ok((task, raw_handle))
}

#[cfg(windows)]
fn windows_watch_loop(
    handle: windows_sys::Win32::Foundation::HANDLE,
    actual: &Path,
    recursive: bool,
    stop: &AtomicBool,
    sender: &mpsc::Sender<BackendEvent>,
) {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_CREATION, FILE_NOTIFY_CHANGE_DIR_NAME,
        FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE,
        ReadDirectoryChangesW,
    };
    const BUFFER_BYTES: usize = 64 * 1024;
    let filter = FILE_NOTIFY_CHANGE_FILE_NAME
        | FILE_NOTIFY_CHANGE_DIR_NAME
        | FILE_NOTIFY_CHANGE_ATTRIBUTES
        | FILE_NOTIFY_CHANGE_SIZE
        | FILE_NOTIFY_CHANGE_LAST_WRITE
        | FILE_NOTIFY_CHANGE_CREATION;
    while !stop.load(Ordering::Acquire) {
        let mut buffer = vec![0_u8; BUFFER_BYTES];
        let mut returned = 0_u32;
        let ok = unsafe {
            ReadDirectoryChangesW(
                handle,
                buffer.as_mut_ptr().cast(),
                u32::try_from(buffer.len()).unwrap_or(u32::MAX),
                i32::from(recursive),
                filter,
                &mut returned,
                std::ptr::null_mut(),
                None,
            )
        };
        if stop.load(Ordering::Acquire) {
            break;
        }
        if ok == 0 || returned == 0 {
            let _ = sender.send(BackendEvent {
                kind: FsChangeKind::Invalidated,
                paths: Vec::new(),
                invalidated: true,
            });
            break;
        }
        match parse_windows_changes(actual, &buffer[..usize::try_from(returned).unwrap_or(0)]) {
            Some(event) => {
                if sender.send(event).is_err() {
                    break;
                }
            }
            None => {
                let _ = sender.send(BackendEvent {
                    kind: FsChangeKind::Invalidated,
                    paths: Vec::new(),
                    invalidated: true,
                });
                break;
            }
        }
    }
}

#[cfg(windows)]
fn parse_windows_changes(actual: &Path, buffer: &[u8]) -> Option<BackendEvent> {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ACTION_ADDED, FILE_ACTION_MODIFIED, FILE_ACTION_REMOVED, FILE_ACTION_RENAMED_NEW_NAME,
        FILE_ACTION_RENAMED_OLD_NAME,
    };
    let mut offset = 0_usize;
    let mut paths = Vec::new();
    let mut kinds = BTreeSet::new();
    loop {
        let header = buffer.get(offset..offset.saturating_add(12))?;
        let next = u32::from_le_bytes(header[0..4].try_into().ok()?) as usize;
        let action = u32::from_le_bytes(header[4..8].try_into().ok()?);
        let name_bytes = u32::from_le_bytes(header[8..12].try_into().ok()?) as usize;
        if !name_bytes.is_multiple_of(2) {
            return None;
        }
        let name_slice =
            buffer.get(offset.saturating_add(12)..offset.saturating_add(12 + name_bytes))?;
        let name = name_slice
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        paths.push(actual.join(String::from_utf16_lossy(&name)));
        kinds.insert(match action {
            FILE_ACTION_ADDED => 1,
            FILE_ACTION_REMOVED => 2,
            FILE_ACTION_RENAMED_OLD_NAME | FILE_ACTION_RENAMED_NEW_NAME => 4,
            FILE_ACTION_MODIFIED => 3,
            _ => 3,
        });
        if next == 0 {
            break;
        }
        offset = offset.checked_add(next)?;
        if offset >= buffer.len() {
            return None;
        }
    }
    Some(BackendEvent {
        kind: coalesced_kind(&kinds),
        paths,
        invalidated: false,
    })
}

#[cfg(not(windows))]
fn spawn_portable_watch(
    actual: PathBuf,
    recursive: bool,
    stop: Arc<AtomicBool>,
    sender: mpsc::Sender<BackendEvent>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut previous = portable_snapshot(&actual, recursive);
        while !stop.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(150));
            let current = portable_snapshot(&actual, recursive);
            let paths = previous
                .symmetric_difference(&current)
                .cloned()
                .collect::<Vec<_>>();
            previous = current;
            if !paths.is_empty()
                && sender
                    .send(BackendEvent {
                        kind: FsChangeKind::Modified,
                        paths,
                        invalidated: false,
                    })
                    .is_err()
            {
                break;
            }
        }
    })
}

#[cfg(not(windows))]
fn portable_snapshot(root: &Path, recursive: bool) -> BTreeSet<PathBuf> {
    fn collect(path: &Path, recursive: bool, output: &mut BTreeSet<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            output.insert(path.clone());
            if recursive && path.is_dir() {
                collect(&path, recursive, output);
            }
        }
    }
    let mut output = BTreeSet::new();
    collect(root, recursive, &mut output);
    output
}

fn collect_backend_event(
    event: BackendEvent,
    requested: &Path,
    recursive: bool,
    paths: &mut BTreeSet<PathBuf>,
    kinds: &mut BTreeSet<u8>,
) {
    kinds.insert(match event.kind {
        FsChangeKind::Created => 1,
        FsChangeKind::Removed => 2,
        FsChangeKind::Renamed => 4,
        FsChangeKind::Modified | FsChangeKind::Invalidated => 3,
    });
    for event_path in event.paths {
        if let Some(path) = changed_path_for_event(requested, recursive, &event_path) {
            paths.insert(path);
        }
    }
}

fn coalesced_kind(kinds: &BTreeSet<u8>) -> FsChangeKind {
    if kinds.len() != 1 {
        return FsChangeKind::Modified;
    }
    match kinds.first().copied() {
        Some(1) => FsChangeKind::Created,
        Some(2) => FsChangeKind::Removed,
        Some(4) => FsChangeKind::Renamed,
        _ => FsChangeKind::Modified,
    }
}

fn changed_path_for_event(requested: &Path, recursive: bool, event_path: &Path) -> Option<PathBuf> {
    if event_path == requested || requested.starts_with(event_path) {
        return Some(requested.to_path_buf());
    }
    let relative = event_path.strip_prefix(requested).ok()?;
    (recursive || relative.components().count() == 1).then(|| event_path.to_path_buf())
}

fn write_event(output: &mut impl Write, event: &FsChangeEvent) -> Result<(), WorkspaceError> {
    serde_json::to_writer(&mut *output, event).map_err(|error| {
        WorkspaceError::new(WorkspaceErrorCode::HostDisconnected, error.to_string())
    })?;
    output.write_all(b"\n").map_err(|error| {
        WorkspaceError::new(WorkspaceErrorCode::HostDisconnected, error.to_string())
    })?;
    output.flush().map_err(|error| {
        WorkspaceError::new(WorkspaceErrorCode::HostDisconnected, error.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_events_are_mapped_and_non_recursive_depth_is_enforced() {
        let root = Path::new("root");
        assert_eq!(
            changed_path_for_event(root, true, Path::new("root/src/lib.rs")),
            Some(PathBuf::from("root/src/lib.rs"))
        );
        assert_eq!(
            changed_path_for_event(root, false, Path::new("root/src/lib.rs")),
            None
        );
    }

    #[test]
    fn mixed_event_kinds_are_coalesced_to_modified() {
        assert_eq!(
            coalesced_kind(&BTreeSet::from([1, 2])),
            FsChangeKind::Modified
        );
        assert_eq!(coalesced_kind(&BTreeSet::from([4])), FsChangeKind::Renamed);
    }

    #[test]
    fn missing_target_uses_nearest_existing_ancestor() {
        let directory = tempfile::tempdir().expect("directory");
        let requested = directory.path().join("missing").join("file.txt");
        let (actual, recursive) = actual_watch_path(&requested, true);
        assert_eq!(actual, directory.path());
        assert!(!recursive);
    }
}
