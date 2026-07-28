use std::{ffi::OsString, path::Path};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RestrictedProcessError {
    #[error("restricted process is only available on Windows")]
    Unsupported,
    #[error("restricted process argument contains an embedded NUL")]
    EmbeddedNul,
    #[error("Windows restricted process operation failed at {operation}: {source}")]
    Windows {
        operation: &'static str,
        source: std::io::Error,
    },
    #[error("Windows AppContainer identity is unavailable: {0}")]
    Identity(String),
}

#[cfg(windows)]
pub fn run_restricted_process(
    executable: &Path,
    args: &[OsString],
    cwd: &Path,
) -> Result<u32, RestrictedProcessError> {
    use std::mem::size_of;

    use windows_sys::Win32::{
        Foundation::HANDLE,
        Security::{
            TOKEN_ADJUST_DEFAULT, TOKEN_ADJUST_SESSIONID, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE,
            TOKEN_QUERY,
        },
        System::{
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject,
            },
            Threading::{
                CREATE_NO_WINDOW, CREATE_SUSPENDED, CreateProcessAsUserW,
                EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, GetExitCodeProcess, INFINITE,
                OpenProcessToken, PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES,
                STARTUPINFOEXW, STARTUPINFOW, TerminateProcess, WaitForSingleObject,
            },
        },
    };

    let mut current_token: HANDLE = std::ptr::null_mut();
    win_bool(
        unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ASSIGN_PRIMARY
                    | TOKEN_DUPLICATE
                    | TOKEN_QUERY
                    | TOKEN_ADJUST_DEFAULT
                    | TOKEN_ADJUST_SESSIONID,
                &mut current_token,
            )
        },
        "OpenProcessToken",
    )?;
    let current_token = OwnedHandle(current_token);
    let app_container = crate::appcontainer::AppContainerSid::resolve()
        .map_err(RestrictedProcessError::Identity)?;
    let standard_handles = BridgedStdio::new()?;
    let inherited_handles = standard_handles.child_handles();
    let attributes = SecurityAttributeList::app_container_with_handles(
        app_container.as_ptr(),
        inherited_handles,
    )?;

    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() {
        return Err(last_error("CreateJobObjectW"));
    }
    let job = OwnedHandle(job);
    let mut job_limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    job_limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    win_bool(
        unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                std::ptr::addr_of!(job_limits).cast(),
                u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                    .unwrap_or(u32::MAX),
            )
        },
        "SetInformationJobObject",
    )?;

    let application = wide_nul(executable.as_os_str())?;
    let mut command_line = command_line(executable, args)?;
    let cwd = wide_nul(cwd.as_os_str())?;
    let startup = STARTUPINFOEXW {
        StartupInfo: STARTUPINFOW {
            cb: u32::try_from(size_of::<STARTUPINFOEXW>()).unwrap_or(u32::MAX),
            dwFlags: STARTF_USESTDHANDLES,
            hStdInput: standard_handles.child_stdin.0,
            hStdOutput: standard_handles.child_stdout.0,
            hStdError: standard_handles.child_stderr.0,
            ..STARTUPINFOW::default()
        },
        lpAttributeList: attributes.as_ptr(),
    };
    let mut process = PROCESS_INFORMATION::default();
    win_bool(
        unsafe {
            CreateProcessAsUserW(
                current_token.0,
                application.as_ptr(),
                command_line.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                // Required by PROC_THREAD_ATTRIBUTE_HANDLE_LIST. The attribute
                // list, rather than the process-wide inheritable bit, is the
                // authority for which handles enter the AppContainer child.
                1,
                CREATE_SUSPENDED | CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT,
                std::ptr::null(),
                cwd.as_ptr(),
                &startup.StartupInfo,
                &mut process,
            )
        },
        "CreateProcessAsUserW",
    )?;
    let process_handle = OwnedHandle(process.hProcess);
    let thread_handle = OwnedHandle(process.hThread);
    let bridges = standard_handles.start();
    if unsafe { AssignProcessToJobObject(job.0, process_handle.0) } == 0 {
        unsafe {
            TerminateProcess(process_handle.0, 0xC000_0001);
        }
        return Err(last_error("AssignProcessToJobObject"));
    }
    if unsafe { ResumeThread(thread_handle.0) } == u32::MAX {
        unsafe {
            TerminateProcess(process_handle.0, 0xC000_0001);
        }
        return Err(last_error("ResumeThread"));
    }
    unsafe {
        WaitForSingleObject(process_handle.0, INFINITE);
    }
    let mut exit_code = 0_u32;
    win_bool(
        unsafe { GetExitCodeProcess(process_handle.0, &mut exit_code) },
        "GetExitCodeProcess",
    )?;
    drop(job);
    bridges.finish_output();
    Ok(exit_code)
}

#[cfg(not(windows))]
pub fn run_restricted_process(
    _executable: &Path,
    _args: &[OsString],
    _cwd: &Path,
) -> Result<u32, RestrictedProcessError> {
    Err(RestrictedProcessError::Unsupported)
}

#[cfg(windows)]
struct SecurityAttributeList {
    pointer: windows_sys::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST,
    _buffer: Vec<usize>,
    _capabilities: Box<windows_sys::Win32::Security::SECURITY_CAPABILITIES>,
    _handles: Vec<windows_sys::Win32::Foundation::HANDLE>,
}

#[cfg(windows)]
impl SecurityAttributeList {
    fn app_container_with_handles(
        sid: windows_sys::Win32::Security::PSID,
        handles: Vec<windows_sys::Win32::Foundation::HANDLE>,
    ) -> Result<Self, RestrictedProcessError> {
        use std::mem::size_of;

        use windows_sys::Win32::{
            Security::SECURITY_CAPABILITIES,
            System::Threading::{
                InitializeProcThreadAttributeList, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, UpdateProcThreadAttribute,
            },
        };

        let mut bytes = 0_usize;
        unsafe {
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 2, 0, &mut bytes);
        }
        if bytes == 0 {
            return Err(last_error("InitializeProcThreadAttributeList(size)"));
        }
        let words = bytes.div_ceil(size_of::<usize>());
        let mut buffer = vec![0_usize; words];
        let pointer = buffer.as_mut_ptr().cast();
        win_bool(
            unsafe { InitializeProcThreadAttributeList(pointer, 2, 0, &mut bytes) },
            "InitializeProcThreadAttributeList",
        )?;
        let capabilities = Box::new(SECURITY_CAPABILITIES {
            AppContainerSid: sid,
            Capabilities: std::ptr::null_mut(),
            CapabilityCount: 0,
            Reserved: 0,
        });
        if let Err(error) = win_bool(
            unsafe {
                UpdateProcThreadAttribute(
                    pointer,
                    0,
                    usize::try_from(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES)
                        .unwrap_or(usize::MAX),
                    std::ptr::from_ref(capabilities.as_ref()).cast(),
                    size_of::<SECURITY_CAPABILITIES>(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            },
            "UpdateProcThreadAttribute(SecurityCapabilities)",
        ) {
            unsafe {
                windows_sys::Win32::System::Threading::DeleteProcThreadAttributeList(pointer);
            }
            return Err(error);
        }
        if let Err(error) = win_bool(
            unsafe {
                UpdateProcThreadAttribute(
                    pointer,
                    0,
                    usize::try_from(PROC_THREAD_ATTRIBUTE_HANDLE_LIST).unwrap_or(usize::MAX),
                    handles.as_ptr().cast(),
                    handles
                        .len()
                        .saturating_mul(size_of::<windows_sys::Win32::Foundation::HANDLE>()),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            },
            "UpdateProcThreadAttribute(HandleList)",
        ) {
            unsafe {
                windows_sys::Win32::System::Threading::DeleteProcThreadAttributeList(pointer);
            }
            return Err(error);
        }
        Ok(Self {
            pointer,
            _buffer: buffer,
            _capabilities: capabilities,
            _handles: handles,
        })
    }

    fn as_ptr(&self) -> windows_sys::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST {
        self.pointer
    }
}

#[cfg(windows)]
impl Drop for SecurityAttributeList {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Threading::DeleteProcThreadAttributeList(self.pointer);
        }
    }
}

#[cfg(windows)]
struct OwnedHandle(windows_sys::Win32::Foundation::HANDLE);

// SAFETY: a Windows kernel HANDLE is process-scoped and may be used and closed
// from a thread other than the one that created it. `OwnedHandle` has unique
// ownership, is not cloneable, and closes the handle exactly once on drop.
#[cfg(windows)]
unsafe impl Send for OwnedHandle {}

#[cfg(windows)]
struct BridgedStdio {
    child_stdin: OwnedHandle,
    child_stdout: OwnedHandle,
    child_stderr: OwnedHandle,
    bridge_stdin: OwnedHandle,
    bridge_stdout: OwnedHandle,
    bridge_stderr: OwnedHandle,
}

#[cfg(windows)]
impl BridgedStdio {
    fn new() -> Result<Self, RestrictedProcessError> {
        let (child_stdin, bridge_stdin) = anonymous_pipe()?;
        let (bridge_stdout, child_stdout) = anonymous_pipe()?;
        let (bridge_stderr, child_stderr) = anonymous_pipe()?;
        Ok(Self {
            child_stdin,
            child_stdout,
            child_stderr,
            bridge_stdin,
            bridge_stdout,
            bridge_stderr,
        })
    }

    fn child_handles(&self) -> Vec<windows_sys::Win32::Foundation::HANDLE> {
        vec![self.child_stdin.0, self.child_stdout.0, self.child_stderr.0]
    }

    fn start(self) -> StdioBridgeThreads {
        use std::io::{Read as _, Write as _};

        let Self {
            child_stdin,
            child_stdout,
            child_stderr,
            bridge_stdin,
            bridge_stdout,
            bridge_stderr,
        } = self;
        drop(child_stdin);
        drop(child_stdout);
        drop(child_stderr);
        let stdin = std::thread::spawn(move || {
            let mut input = std::io::stdin().lock();
            let mut buffer = [0_u8; 16 * 1024];
            while let Ok(read) = input.read(&mut buffer) {
                if read == 0 || write_handle(&bridge_stdin, &buffer[..read]).is_err() {
                    break;
                }
            }
        });
        let stdout = std::thread::spawn(move || {
            let mut output = std::io::stdout().lock();
            copy_from_handle(&bridge_stdout, &mut output);
            let _ = output.flush();
        });
        let stderr = std::thread::spawn(move || {
            let mut output = std::io::stderr().lock();
            copy_from_handle(&bridge_stderr, &mut output);
            let _ = output.flush();
        });
        StdioBridgeThreads {
            _stdin: stdin,
            stdout,
            stderr,
        }
    }
}

#[cfg(windows)]
struct StdioBridgeThreads {
    _stdin: std::thread::JoinHandle<()>,
    stdout: std::thread::JoinHandle<()>,
    stderr: std::thread::JoinHandle<()>,
}

#[cfg(windows)]
impl StdioBridgeThreads {
    fn finish_output(self) {
        let _ = self.stdout.join();
        let _ = self.stderr.join();
    }
}

#[cfg(windows)]
fn anonymous_pipe() -> Result<(OwnedHandle, OwnedHandle), RestrictedProcessError> {
    use std::mem::size_of;

    use windows_sys::Win32::{Security::SECURITY_ATTRIBUTES, System::Pipes::CreatePipe};

    let attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(u32::MAX),
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    let mut read = std::ptr::null_mut();
    let mut write = std::ptr::null_mut();
    win_bool(
        unsafe { CreatePipe(&mut read, &mut write, &attributes, 0) },
        "CreatePipe(stdio)",
    )?;
    let read = OwnedHandle(read);
    let write = OwnedHandle(write);
    Ok((read, write))
}

#[cfg(windows)]
fn write_handle(handle: &OwnedHandle, mut bytes: &[u8]) -> Result<(), ()> {
    use windows_sys::Win32::Storage::FileSystem::WriteFile;

    while !bytes.is_empty() {
        let mut written = 0_u32;
        if unsafe {
            WriteFile(
                handle.0,
                bytes.as_ptr(),
                u32::try_from(bytes.len()).unwrap_or(u32::MAX),
                &mut written,
                std::ptr::null_mut(),
            )
        } == 0
            || written == 0
        {
            return Err(());
        }
        bytes = &bytes[usize::try_from(written).unwrap_or(bytes.len())..];
    }
    Ok(())
}

#[cfg(windows)]
fn copy_from_handle(handle: &OwnedHandle, output: &mut impl std::io::Write) {
    use windows_sys::Win32::Storage::FileSystem::ReadFile;

    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let mut read = 0_u32;
        if unsafe {
            ReadFile(
                handle.0,
                buffer.as_mut_ptr(),
                u32::try_from(buffer.len()).unwrap_or(u32::MAX),
                &mut read,
                std::ptr::null_mut(),
            )
        } == 0
            || read == 0
        {
            break;
        }
        if output
            .write_all(&buffer[..usize::try_from(read).unwrap_or_default()])
            .is_err()
        {
            break;
        }
        // The launcher is a transparent protocol transport. Rust may block-
        // buffer redirected stdout/stderr, so every bridge chunk must become
        // visible before the AppContainer child exits (long-lived MCP and
        // Workspace hosts keep running after their first response).
        if output.flush().is_err() {
            break;
        }
    }
}

#[cfg(windows)]
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }
}

#[cfg(windows)]
fn win_bool(value: i32, operation: &'static str) -> Result<(), RestrictedProcessError> {
    if value == 0 {
        Err(last_error(operation))
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn last_error(operation: &'static str) -> RestrictedProcessError {
    RestrictedProcessError::Windows {
        operation,
        source: std::io::Error::last_os_error(),
    }
}

#[cfg(windows)]
fn wide_nul(value: &std::ffi::OsStr) -> Result<Vec<u16>, RestrictedProcessError> {
    use std::os::windows::ffi::OsStrExt;

    let encoded = value.encode_wide().collect::<Vec<_>>();
    if encoded.contains(&0) {
        return Err(RestrictedProcessError::EmbeddedNul);
    }
    Ok(encoded.into_iter().chain(Some(0)).collect())
}

#[cfg(windows)]
fn command_line(executable: &Path, args: &[OsString]) -> Result<Vec<u16>, RestrictedProcessError> {
    use std::os::windows::ffi::OsStrExt;

    let mut command =
        quote_windows_argument(&executable.as_os_str().encode_wide().collect::<Vec<_>>())?;
    for argument in args {
        command.push(' ' as u16);
        command.extend(quote_windows_argument(
            &argument.encode_wide().collect::<Vec<_>>(),
        )?);
    }
    command.push(0);
    Ok(command)
}

#[cfg(windows)]
fn quote_windows_argument(argument: &[u16]) -> Result<Vec<u16>, RestrictedProcessError> {
    if argument.contains(&0) {
        return Err(RestrictedProcessError::EmbeddedNul);
    }
    let needs_quotes = argument.is_empty()
        || argument
            .iter()
            .any(|value| [b' ' as u16, b'\t' as u16, b'"' as u16].contains(value));
    if !needs_quotes {
        return Ok(argument.to_vec());
    }
    let mut output = vec![b'"' as u16];
    let mut slashes = 0_usize;
    for value in argument {
        if *value == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        if *value == b'"' as u16 {
            output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
        } else {
            output.extend(std::iter::repeat_n(b'\\' as u16, slashes));
        }
        slashes = 0;
        output.push(*value);
    }
    output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    output.push(b'"' as u16);
    Ok(output)
}
