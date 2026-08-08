// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/utils/pty/src/win/{conpty,psuedocon,mod}.rs.
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// The ConPTY command-line quoting and pseudo-console lifetime control below
// retain the WezTerm-derived MIT notice from the fixed Codex source.
//
// Copyright (c) 2018-Present Wez Furlong
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString, c_void},
    io::{Error as IoError, ErrorKind},
    mem::size_of,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
    ptr,
    sync::Arc,
    time::Duration,
};

use hachimi_protocol::{ProcessOutputStream, ProcessTerminalSize};
use tokio::sync::{mpsc, oneshot};
use windows_sys::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0},
    Security::SECURITY_ATTRIBUTES,
    System::{
        Console::{COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole},
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectExtendedLimitInformation, SetInformationJobObject,
        },
        Pipes::CreatePipe,
        Threading::{
            CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
            EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, InitializeProcThreadAttributeList,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
            STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
        },
    },
};

use super::PIPE_READ_CHUNK;
use crate::{ProcessError, RuntimeControl, RuntimeOutput, SpawnedRuntime};

const PSEUDOCONSOLE_RESIZE_QUIRK: u32 = 0x2;
const PROC_THREAD_ATTRIBUTE_JOB_LIST: usize = 0x0002_000D;
const INFINITE: u32 = u32::MAX;

#[derive(Debug)]
struct PipeHandle(OwnedHandle);

impl PipeHandle {
    fn raw(&self) -> HANDLE {
        self.0.as_raw_handle() as HANDLE
    }

    fn read(&mut self, buffer: &mut [u8]) -> Result<usize, IoError> {
        let mut read = 0_u32;
        // SAFETY: the handle is a synchronous pipe and the buffer is valid for
        // the requested byte count.
        let ok = unsafe {
            ReadFile(
                self.raw(),
                buffer.as_mut_ptr(),
                u32::try_from(buffer.len()).unwrap_or(u32::MAX),
                &mut read,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            let error = IoError::last_os_error();
            if error.kind() == ErrorKind::BrokenPipe {
                Ok(0)
            } else {
                Err(error)
            }
        } else {
            Ok(read as usize)
        }
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), IoError> {
        let mut offset = 0_usize;
        while offset < bytes.len() {
            let remaining = &bytes[offset..];
            let mut written = 0_u32;
            // SAFETY: the handle is a synchronous pipe and the slice is valid
            // for the requested byte count.
            let ok = unsafe {
                WriteFile(
                    self.raw(),
                    remaining.as_ptr(),
                    u32::try_from(remaining.len()).unwrap_or(u32::MAX),
                    &mut written,
                    ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(IoError::last_os_error());
            }
            if written == 0 {
                return Err(IoError::new(
                    ErrorKind::WriteZero,
                    "pipe write made no progress",
                ));
            }
            offset = offset.saturating_add(written as usize);
        }
        Ok(())
    }
}

#[derive(Debug)]
struct PseudoConsole {
    handle: HPCON,
    // CreatePseudoConsole borrows the pseudo-side pipe handles until the
    // console is closed. They must outlive the process and all I/O drains.
    _input: OwnedHandle,
    _output: OwnedHandle,
    _job: OwnedHandle,
}

unsafe impl Send for PseudoConsole {}
unsafe impl Sync for PseudoConsole {}

impl Drop for PseudoConsole {
    fn drop(&mut self) {
        // SAFETY: `handle` was returned by CreatePseudoConsole and is closed
        // exactly once here after all process I/O users have been dropped.
        unsafe { ClosePseudoConsole(self.handle) };
    }
}

impl PseudoConsole {
    fn create(size: ProcessTerminalSize) -> Result<(Self, PipeHandle, PipeHandle), ProcessError> {
        validate_size(size)?;
        let (input_read, input_write) = create_pipe()?;
        let (output_read, output_write) = create_pipe()?;
        let coordinate = COORD {
            X: i16::try_from(size.cols)
                .map_err(|_| ProcessError::InvalidRequest("terminal columns are too large"))?,
            Y: i16::try_from(size.rows)
                .map_err(|_| ProcessError::InvalidRequest("terminal rows are too large"))?,
        };
        let mut handle: HPCON = 0;
        // SAFETY: all handles are valid anonymous pipe handles and `handle`
        // points to writable storage owned by this function.
        let result = unsafe {
            CreatePseudoConsole(
                coordinate,
                input_read.as_raw_handle() as HANDLE,
                output_write.as_raw_handle() as HANDLE,
                PSEUDOCONSOLE_RESIZE_QUIRK,
                &mut handle,
            )
        };
        if result < 0 || handle == 0 {
            return Err(ProcessError::Pty(format!(
                "CreatePseudoConsole failed: HRESULT {result:#x}; {}",
                IoError::last_os_error()
            )));
        }
        let input = PipeHandle(input_write);
        let output = PipeHandle(output_read);
        Ok((
            Self {
                handle,
                _input: input_read,
                _output: output_write,
                _job: create_job()?,
            },
            input,
            output,
        ))
    }

    fn resize(&self, size: ProcessTerminalSize) -> Result<(), ProcessError> {
        validate_size(size)?;
        let coordinate = COORD {
            X: i16::try_from(size.cols)
                .map_err(|_| ProcessError::InvalidRequest("terminal columns are too large"))?,
            Y: i16::try_from(size.rows)
                .map_err(|_| ProcessError::InvalidRequest("terminal rows are too large"))?,
        };
        // SAFETY: this handle remains valid for the lifetime of `self`.
        let result = unsafe { ResizePseudoConsole(self.handle, coordinate) };
        if result < 0 {
            return Err(ProcessError::Pty(format!(
                "ResizePseudoConsole failed: HRESULT {result:#x}; {}",
                IoError::last_os_error()
            )));
        }
        Ok(())
    }
}

#[derive(Debug)]
struct ProcThreadAttributes {
    pointer: *mut c_void,
    _storage: Vec<u8>,
    _job_list: Vec<HANDLE>,
    job_attribute_installed: bool,
}

unsafe impl Send for ProcThreadAttributes {}
unsafe impl Sync for ProcThreadAttributes {}

impl ProcThreadAttributes {
    fn with_pseudoconsole(handle: HPCON, job: HANDLE) -> Result<Self, ProcessError> {
        let mut bytes = 0_usize;
        // SAFETY: the first call only asks Windows for the required size.
        unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 2, 0, &mut bytes) };
        if bytes == 0 {
            return Err(ProcessError::Pty(format!(
                "InitializeProcThreadAttributeList(size) failed: {}",
                IoError::last_os_error()
            )));
        }
        let mut storage = vec![0_u8; bytes];
        let pointer = storage.as_mut_ptr().cast::<c_void>();
        // SAFETY: `storage` has the exact size requested by Windows and stays
        // alive until the process has been created.
        if unsafe { InitializeProcThreadAttributeList(pointer, 2, 0, &mut bytes) } == 0 {
            return Err(ProcessError::Pty(format!(
                "InitializeProcThreadAttributeList failed: {}",
                IoError::last_os_error()
            )));
        }
        // The Win32 API expects the HPCON value itself as lpValue, not a
        // pointer to a nested structure. This is the same representation used
        // by Codex's winapi-backed ConPTY adapter.
        // SAFETY: the HPCON is valid while the attribute list is installed.
        let updated = unsafe {
            UpdateProcThreadAttribute(
                pointer,
                0,
                usize::try_from(PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE).unwrap_or(usize::MAX),
                handle as *const c_void,
                size_of::<HPCON>(),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        if updated == 0 {
            // SAFETY: the list was initialized successfully above.
            unsafe { DeleteProcThreadAttributeList(pointer) };
            return Err(ProcessError::Pty(format!(
                "UpdateProcThreadAttribute(PseudoConsole) failed: {}",
                IoError::last_os_error()
            )));
        }
        let job_list = vec![job];
        let updated = unsafe {
            UpdateProcThreadAttribute(
                pointer,
                0,
                PROC_THREAD_ATTRIBUTE_JOB_LIST,
                job_list.as_ptr().cast::<c_void>(),
                size_of::<HANDLE>(),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        let job_attribute_installed = if updated == 0 {
            let code = IoError::last_os_error().raw_os_error().unwrap_or_default();
            if code != 31 {
                unsafe { DeleteProcThreadAttributeList(pointer) };
                return Err(ProcessError::Pty(format!(
                    "UpdateProcThreadAttribute(JobList) failed: {}",
                    IoError::from_raw_os_error(code)
                )));
            }
            false
        } else {
            true
        };
        Ok(Self {
            pointer,
            _storage: storage,
            _job_list: job_list,
            job_attribute_installed,
        })
    }
}

impl Drop for ProcThreadAttributes {
    fn drop(&mut self) {
        // SAFETY: `pointer` is initialized exactly once by Windows and storage
        // outlives this call.
        unsafe { DeleteProcThreadAttributeList(self.pointer) };
    }
}

#[derive(Debug)]
struct SharedProcessHandle(OwnedHandle);

unsafe impl Send for SharedProcessHandle {}
unsafe impl Sync for SharedProcessHandle {}

impl SharedProcessHandle {
    fn raw(&self) -> HANDLE {
        self.0.as_raw_handle() as HANDLE
    }
}

pub(crate) async fn spawn_conpty(
    launcher: Option<PathBuf>,
    command: Vec<String>,
    cwd: PathBuf,
    environment: BTreeMap<String, String>,
    size: ProcessTerminalSize,
    timeout: Option<Duration>,
) -> Result<SpawnedRuntime, ProcessError> {
    let spawned = tokio::task::spawn_blocking(move || {
        spawn_native(launcher.as_deref(), &command, &cwd, &environment, size)
    })
    .await
    .map_err(|error| ProcessError::Pty(error.to_string()))??;
    let (console, reader, writer, process) = spawned;
    let process = Arc::new(SharedProcessHandle(process));
    let (output_tx, output_rx) = mpsc::channel(256);
    let reader_task = tokio::task::spawn_blocking(move || read_conpty(reader, output_tx));
    let (write_tx, mut write_rx) = mpsc::channel::<PtyWrite>(128);
    let writer_task = tokio::task::spawn_blocking(move || write_conpty(writer, &mut write_rx));
    let (control_tx, mut control_rx) = mpsc::channel(128);
    let (exit_tx, exit_rx) = oneshot::channel();
    tokio::spawn(async move {
        let console = Arc::new(console);
        let mut write_tx = Some(write_tx);
        let wait_process = Arc::clone(&process);
        let (wait_tx, mut wait_rx) = oneshot::channel();
        tokio::task::spawn_blocking(move || {
            let _ = wait_tx.send(wait_for_process(&wait_process));
        });
        let expiration = super::wait_timeout(timeout);
        tokio::pin!(expiration);
        let mut timed_out = false;
        let exit_code = loop {
            tokio::select! {
                code = &mut wait_rx => break code.unwrap_or(-1),
                control = control_rx.recv() => match control {
                    Some(RuntimeControl::Write { bytes, close, response }) => {
                        let result = if bytes.is_empty() && !close {
                            Ok(())
                        } else if let Some(sender) = write_tx.as_ref() {
                            let (written_tx, written_rx) = oneshot::channel();
                            if sender.send(PtyWrite { bytes, response: written_tx }).await.is_err() {
                                Err(ProcessError::StdinClosed)
                            } else {
                                async_result(written_rx).await
                            }
                        } else {
                            Err(ProcessError::StdinClosed)
                        };
                        if close { write_tx.take(); }
                        let _ = response.send(result);
                    }
                    Some(RuntimeControl::Resize { size, response }) => {
                        let _ = response.send(console.resize(size));
                    }
                    Some(RuntimeControl::Terminate { response }) => {
                        let result = terminate_process(process.raw());
                        let _ = response.send(result);
                    }
                    None => {
                        let _ = terminate_process(process.raw());
                        break -1;
                    }
                },
                () = &mut expiration, if timeout.is_some() && !timed_out => {
                    timed_out = true;
                    let _ = terminate_process(process.raw());
                }
            }
        };
        write_tx.take();
        // Closing the pseudo console releases the pseudo-side output handle so
        // the reader can observe EOF after the child has exited.
        drop(console);
        let _ = tokio::time::timeout(Duration::from_secs(2), reader_task).await;
        let _ = tokio::time::timeout(Duration::from_secs(2), writer_task).await;
        let _ = exit_tx.send(if timed_out { 124 } else { exit_code });
    });
    Ok(SpawnedRuntime {
        control_tx,
        output_rx,
        exit_rx,
    })
}

#[derive(Debug)]
struct PtyWrite {
    bytes: Vec<u8>,
    response: oneshot::Sender<Result<(), ProcessError>>,
}

async fn async_result(
    receiver: oneshot::Receiver<Result<(), ProcessError>>,
) -> Result<(), ProcessError> {
    receiver.await.unwrap_or(Err(ProcessError::ControlClosed))
}

fn read_conpty(mut reader: PipeHandle, output: mpsc::Sender<RuntimeOutput>) {
    let mut buffer = [0_u8; PIPE_READ_CHUNK];
    let mut access_denied_retries = 0_u32;
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => {
                break;
            }
            Ok(read) => {
                access_denied_retries = 0;
                if output
                    .blocking_send(RuntimeOutput {
                        stream: ProcessOutputStream::Stdout,
                        bytes: buffer[..read].to_vec(),
                    })
                    .is_err()
                {
                    break;
                }
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) if error.raw_os_error() == Some(5) && access_denied_retries < 100 => {
                // ConPTY can briefly report ERROR_ACCESS_DENIED while the
                // child is attaching its pseudoconsole during CreateProcess.
                // Codex's reader keeps draining during this hand-off instead
                // of closing the host pipe, which would make the child exit
                // with STATUS_CONTROL_C_EXIT.
                access_denied_retries += 1;
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_error) => {
                break;
            }
        }
    }
}

fn write_conpty(mut writer: PipeHandle, receiver: &mut mpsc::Receiver<PtyWrite>) {
    while let Some(message) = receiver.blocking_recv() {
        let result = (|| {
            if !message.bytes.is_empty() {
                let mut retries = 0_u32;
                loop {
                    match writer.write_all(&message.bytes) {
                        Ok(()) => break,
                        Err(error) if error.raw_os_error() == Some(5) && retries < 100 => {
                            retries += 1;
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => {
                            return Err(ProcessError::Pty(format!(
                                "ConPTY input write failed: {error}"
                            )));
                        }
                    }
                }
            }
            Ok(())
        })();
        let _ = message.response.send(result);
    }
}

fn spawn_native(
    launcher: Option<&Path>,
    command: &[String],
    cwd: &Path,
    environment: &BTreeMap<String, String>,
    size: ProcessTerminalSize,
) -> Result<(PseudoConsole, PipeHandle, PipeHandle, OwnedHandle), ProcessError> {
    let (program, args) = super::command_program_and_args(launcher, command)?;
    let mut env = environment.clone();
    let program_os = search_path(&env, OsStr::new(&program));
    let commandline = build_command_line(&program_os, &args)?;
    let mut commandline_wide = commandline
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut program_wide = program_os
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let cwd = win32_compatible_cwd(cwd);
    let cwd_wide = cwd
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut env_block = build_environment_block(&mut env)?;
    let (console, input, output) = PseudoConsole::create(size)?;
    let attributes = ProcThreadAttributes::with_pseudoconsole(
        console.handle,
        console._job.as_raw_handle() as HANDLE,
    )?;
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
    startup.lpAttributeList = attributes.pointer;
    let mut process_info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: all wide strings are NUL terminated and mutable as required by
    // CreateProcessW; attribute storage and pseudo console outlive the call.
    let created = unsafe {
        CreateProcessW(
            program_wide.as_mut_ptr(),
            commandline_wide.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            env_block.as_mut_ptr().cast(),
            cwd_wide.as_ptr(),
            std::ptr::from_ref(&startup.StartupInfo),
            &mut process_info,
        )
    };
    if created == 0 {
        return Err(ProcessError::Pty(format!(
            "CreateProcessW failed: {}",
            IoError::last_os_error()
        )));
    }
    if !attributes.job_attribute_installed {
        // Best-effort fallback for hosts that disallow nested Job Objects.
        // The assignment is performed before the process handle is exposed to
        // the async runtime; failure terminates the just-created process and
        // fails closed instead of returning an uncontained process.
        let assigned = unsafe {
            AssignProcessToJobObject(
                console._job.as_raw_handle() as HANDLE,
                process_info.hProcess,
            )
        };
        if assigned == 0 {
            let error = IoError::last_os_error();
            unsafe {
                TerminateProcess(process_info.hProcess, 1);
                CloseHandle(process_info.hThread);
                CloseHandle(process_info.hProcess);
            }
            return Err(ProcessError::Pty(format!(
                "AssignProcessToJobObject fallback failed: {error}"
            )));
        }
    }
    // SAFETY: CreateProcessW returned owned handles, each closed exactly once.
    unsafe { CloseHandle(process_info.hThread) };
    // Keep the pseudo console alive in the returned runtime, and pass only the
    // host-side pipe ends to the reader and writer tasks.
    let process = unsafe { OwnedHandle::from_raw_handle(process_info.hProcess as _) };
    Ok((console, output, input, process))
}

fn win32_compatible_cwd(cwd: &Path) -> OsString {
    // PowerShell 5 accepts a verbatim cwd but cannot resolve relative provider paths from it.
    let value = cwd.as_os_str().encode_wide().collect::<Vec<_>>();
    let verbatim = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    if !value.starts_with(&verbatim) {
        return cwd.as_os_str().to_owned();
    }

    let is_drive_path = value.len() >= 7
        && u8::try_from(value[4]).is_ok_and(|letter| letter.is_ascii_alphabetic())
        && value[5] == b':' as u16
        && matches!(value[6], value if value == b'\\' as u16 || value == b'/' as u16);
    if is_drive_path {
        return OsString::from_wide(&value[4..]);
    }

    let unc = [
        (b'U' as u16, b'u' as u16),
        (b'N' as u16, b'n' as u16),
        (b'C' as u16, b'c' as u16),
        (b'\\' as u16, b'\\' as u16),
    ];
    if value.get(4..8).is_some_and(|prefix| {
        prefix
            .iter()
            .zip(unc)
            .all(|(&actual, (upper, lower))| actual == upper || actual == lower)
    }) {
        let mut compatible = vec![b'\\' as u16, b'\\' as u16];
        compatible.extend_from_slice(&value[8..]);
        return OsString::from_wide(&compatible);
    }

    cwd.as_os_str().to_owned()
}

fn create_pipe() -> Result<(OwnedHandle, OwnedHandle), ProcessError> {
    let mut read: HANDLE = ptr::null_mut();
    let mut write: HANDLE = ptr::null_mut();
    let mut security_attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: 0,
    };
    // SAFETY: output pointers and security attributes are valid for the call;
    // Windows creates both handles and does not retain the descriptor pointer.
    if unsafe {
        CreatePipe(
            &mut read,
            &mut write,
            std::ptr::from_mut(&mut security_attributes),
            0,
        )
    } == 0
    {
        return Err(ProcessError::Pty(format!(
            "CreatePipe failed: {}",
            IoError::last_os_error()
        )));
    }
    // SAFETY: handles were returned by CreatePipe and ownership is transferred
    // to these RAII wrappers.
    Ok(
        (unsafe { OwnedHandle::from_raw_handle(read as _) }, unsafe {
            OwnedHandle::from_raw_handle(write as _)
        }),
    )
}

fn create_job() -> Result<OwnedHandle, ProcessError> {
    // SAFETY: null security attributes and name create a private unnamed job.
    let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if handle.is_null() {
        return Err(ProcessError::Pty(format!(
            "CreateJobObjectW failed: {}",
            IoError::last_os_error()
        )));
    }
    // SAFETY: handle is valid and limits has the documented ABI layout.
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags =
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK;
    let configured = unsafe {
        SetInformationJobObject(
            handle,
            JobObjectExtendedLimitInformation,
            std::ptr::from_ref(&limits).cast::<c_void>(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if configured == 0 {
        // SAFETY: handle was returned by CreateJobObjectW and is not wrapped
        // yet because configuration failed.
        unsafe { CloseHandle(handle) };
        return Err(ProcessError::Pty(format!(
            "SetInformationJobObject failed: {}",
            IoError::last_os_error()
        )));
    }
    // SAFETY: ownership is transferred to this RAII wrapper.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle as _) })
}

fn wait_for_process(process: &SharedProcessHandle) -> i32 {
    // SAFETY: process handle is valid and remains alive through Arc ownership.
    let result = unsafe { WaitForSingleObject(process.raw(), INFINITE) };
    if result != WAIT_OBJECT_0 {
        return -1;
    }
    let mut code = 0_u32;
    // SAFETY: output pointer is valid and process handle is signaled.
    if unsafe { GetExitCodeProcess(process.raw(), &mut code) } == 0 {
        -1
    } else {
        i32::try_from(code).unwrap_or(-1)
    }
}

fn terminate_process(handle: HANDLE) -> Result<(), ProcessError> {
    // SAFETY: handle is owned by the running ProcessRegistry entry.
    if unsafe { TerminateProcess(handle, 1) } == 0 {
        return Err(ProcessError::Spawn(IoError::last_os_error()));
    }
    Ok(())
}

fn validate_size(size: ProcessTerminalSize) -> Result<(), ProcessError> {
    if size.rows == 0 || size.cols == 0 {
        return Err(ProcessError::InvalidRequest(
            "terminal rows and columns must be greater than zero",
        ));
    }
    if size.rows > i16::MAX as u16 || size.cols > i16::MAX as u16 {
        return Err(ProcessError::InvalidRequest("terminal size is too large"));
    }
    Ok(())
}

fn build_environment_block(
    environment: &mut BTreeMap<String, String>,
) -> Result<Vec<u16>, ProcessError> {
    let mut block = Vec::new();
    for (key, value) in environment.iter() {
        if key.is_empty() || key.contains('=') || key.contains('\0') || value.contains('\0') {
            return Err(ProcessError::InvalidRequest(
                "environment contains an invalid key or value",
            ));
        }
        block.extend(OsStr::new(key).encode_wide());
        block.push('=' as u16);
        block.extend(OsStr::new(value).encode_wide());
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

fn search_path(environment: &BTreeMap<String, String>, executable: &OsStr) -> OsString {
    if executable.is_empty() || Path::new(executable).components().count() > 1 {
        return executable.to_os_string();
    }
    let path = environment
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("PATH"))
        .map(|(_, value)| value.as_str());
    let Some(path) = path else {
        return executable.to_os_string();
    };
    for directory in std::env::split_paths(OsStr::new(path)) {
        let candidate = directory.join(executable);
        if candidate.is_file() {
            return candidate.into_os_string();
        }
        let pathext = environment
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("PATHEXT"))
            .map(|(_, value)| value.as_str())
            .unwrap_or(".COM;.EXE;.BAT;.CMD");
        for extension in pathext.split(';').filter(|value| !value.is_empty()) {
            let suffix = extension.strip_prefix('.').unwrap_or(extension);
            let candidate = directory.join(executable).with_extension(suffix);
            if candidate.is_file() {
                return candidate.into_os_string();
            }
        }
    }
    executable.to_os_string()
}

fn build_command_line(program: &OsStr, args: &[String]) -> Result<OsString, ProcessError> {
    let mut wide = Vec::new();
    append_quoted(program, &mut wide)?;
    for argument in args {
        if argument.contains('\0') {
            return Err(ProcessError::InvalidRequest(
                "command argument contains NUL",
            ));
        }
        wide.push(' ' as u16);
        append_quoted(OsStr::new(argument), &mut wide)?;
    }
    Ok(OsString::from_wide(&wide))
}

fn append_quoted(argument: &OsStr, commandline: &mut Vec<u16>) -> Result<(), ProcessError> {
    if argument.encode_wide().any(|value| value == 0) {
        return Err(ProcessError::InvalidRequest("command contains NUL"));
    }
    let values = argument.encode_wide().collect::<Vec<_>>();
    if !values.is_empty()
        && !values.iter().any(|value| {
            *value == b' ' as u16
                || *value == b'\t' as u16
                || *value == b'\n' as u16
                || *value == 0x0b
                || *value == b'"' as u16
        })
    {
        commandline.extend(values);
        return Ok(());
    }
    commandline.push('"' as u16);
    let mut index = 0;
    while index < values.len() {
        let mut slashes = 0;
        while index < values.len() && values[index] == '\\' as u16 {
            slashes += 1;
            index += 1;
        }
        if index == values.len() {
            commandline.extend(std::iter::repeat_n('\\' as u16, slashes * 2));
            break;
        }
        if values[index] == '"' as u16 {
            commandline.extend(std::iter::repeat_n('\\' as u16, slashes * 2 + 1));
            commandline.push('"' as u16);
        } else {
            commandline.extend(std::iter::repeat_n('\\' as u16, slashes));
            commandline.push(values[index]);
        }
        index += 1;
    }
    commandline.push('"' as u16);
    Ok(())
}

#[cfg(test)]
mod conpty_tests {
    use super::*;

    #[test]
    fn anonymous_pipe_file_write() {
        let (read, write) = create_pipe().expect("pipe");
        let mut file = PipeHandle(write);
        file.write_all(b"x").expect("write");
        let mut read = PipeHandle(read);
        let mut byte = [0_u8; 1];
        assert_eq!(read.read(&mut byte).expect("read"), 1);
        assert_eq!(byte, [b'x']);
    }

    #[test]
    fn process_cwd_strips_only_win32_compatible_verbatim_prefixes() {
        assert_eq!(
            win32_compatible_cwd(Path::new(r"\\?\C:\Hachimi\project")),
            OsString::from(r"C:\Hachimi\project")
        );
        assert_eq!(
            win32_compatible_cwd(Path::new(r"\\?\unc\server\share\project")),
            OsString::from(r"\\server\share\project")
        );
        assert_eq!(
            win32_compatible_cwd(Path::new(
                r"\\?\Volume{00000000-0000-0000-0000-000000000000}\"
            )),
            OsString::from(r"\\?\Volume{00000000-0000-0000-0000-000000000000}\")
        );
    }
}
