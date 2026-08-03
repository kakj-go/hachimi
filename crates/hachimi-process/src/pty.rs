// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/utils/pty/src/{pty,process}.rs
// and app-server/src/request_processors/process_exec_processor.rs.
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// Modified for Hachimi: optional restricted launcher, typed terminal size, and Tokio controls.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use hachimi_process_policy::ProcessPolicy;
use hachimi_protocol::{ProcessOutputStream, ProcessTerminalSize};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::{mpsc, oneshot},
};

use crate::{ProcessError, RuntimeControl, RuntimeOutput, SpawnedRuntime};

#[cfg(windows)]
mod conpty;

const PIPE_READ_CHUNK: usize = 8 * 1024;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn spawn_runtime(
    launcher: Option<PathBuf>,
    command: Vec<String>,
    cwd: PathBuf,
    environment: BTreeMap<String, String>,
    tty: bool,
    stream_stdin: bool,
    size: ProcessTerminalSize,
    timeout: Option<Duration>,
) -> Result<SpawnedRuntime, ProcessError> {
    if tty {
        spawn_pty(launcher, command, cwd, environment, size, timeout).await
    } else {
        spawn_pipe(launcher, command, cwd, environment, stream_stdin, timeout).await
    }
}

fn command_program_and_args(
    launcher: Option<&Path>,
    command: &[String],
) -> Result<(String, Vec<String>), ProcessError> {
    let Some(program) = command.first().filter(|value| !value.trim().is_empty()) else {
        return Err(ProcessError::InvalidRequest("command must not be empty"));
    };
    if let Some(launcher) = launcher {
        let mut args = Vec::with_capacity(command.len() + 1);
        args.push("--".into());
        args.extend(command.iter().cloned());
        Ok((launcher.to_string_lossy().into_owned(), args))
    } else {
        Ok((program.clone(), command[1..].to_vec()))
    }
}

async fn spawn_pipe(
    launcher: Option<PathBuf>,
    command: Vec<String>,
    cwd: PathBuf,
    environment: BTreeMap<String, String>,
    stream_stdin: bool,
    timeout: Option<Duration>,
) -> Result<SpawnedRuntime, ProcessError> {
    let command = resolve_restricted_command(launcher.as_deref(), &command, &environment)?;
    let (program, args) = command_program_and_args(launcher.as_deref(), &command)?;
    let mut process = Command::new(program);
    ProcessPolicy::HiddenCaptured.apply_tokio(&mut process);
    process
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .envs(environment)
        .stdin(if stream_stdin {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = process.spawn().map_err(ProcessError::Spawn)?;
    let stdin = Arc::new(tokio::sync::Mutex::new(child.stdin.take()));
    let stdout = child.stdout.take().ok_or(ProcessError::MissingStdio)?;
    let stderr = child.stderr.take().ok_or(ProcessError::MissingStdio)?;
    let (output_tx, output_rx) = mpsc::channel(256);
    let stdout_task = tokio::spawn(read_pipe(
        stdout,
        ProcessOutputStream::Stdout,
        output_tx.clone(),
    ));
    let stderr_task = tokio::spawn(read_pipe(
        stderr,
        ProcessOutputStream::Stderr,
        output_tx.clone(),
    ));
    drop(output_tx);
    let (control_tx, mut control_rx) = mpsc::channel(128);
    let (exit_tx, exit_rx) = oneshot::channel();
    tokio::spawn(async move {
        let expiration = wait_timeout(timeout);
        tokio::pin!(expiration);
        let mut timed_out = false;
        let exit_code = loop {
            tokio::select! {
                status = child.wait() => {
                    break status.ok().and_then(|status| status.code()).unwrap_or(-1);
                }
                control = control_rx.recv() => match control {
                    Some(RuntimeControl::Write { bytes, close, response }) => {
                        let result = write_pipe_stdin(&stdin, bytes, close).await;
                        let _ = response.send(result);
                    }
                    Some(RuntimeControl::Resize { response, .. }) => {
                        let _ = response.send(Err(ProcessError::NotInteractive));
                    }
                    Some(RuntimeControl::Terminate { response }) => {
                        let result = child.start_kill().map_err(ProcessError::Spawn);
                        let _ = response.send(result);
                    }
                    None => {
                        let _ = child.start_kill();
                    }
                },
                () = &mut expiration, if timeout.is_some() && !timed_out => {
                    timed_out = true;
                    let _ = child.start_kill();
                }
            }
        };
        drop(stdin);
        let _ = tokio::time::timeout(Duration::from_secs(2), stdout_task).await;
        let _ = tokio::time::timeout(Duration::from_secs(2), stderr_task).await;
        let _ = exit_tx.send(if timed_out { 124 } else { exit_code });
    });
    Ok(SpawnedRuntime {
        control_tx,
        output_rx,
        exit_rx,
    })
}

fn resolve_restricted_command(
    launcher: Option<&Path>,
    command: &[String],
    environment: &BTreeMap<String, String>,
) -> Result<Vec<String>, ProcessError> {
    if launcher.is_none() {
        return Ok(command.to_vec());
    }
    let Some(program) = command.first().filter(|value| !value.trim().is_empty()) else {
        return Err(ProcessError::InvalidRequest("command must not be empty"));
    };
    let program_path = Path::new(program);
    let resolved = if program_path.is_absolute() && program_path.is_file() {
        program_path.to_owned()
    } else if program_path.components().count() == 1 {
        search_environment_path(environment, program_path).ok_or(ProcessError::InvalidRequest(
            "restricted command executable could not be resolved",
        ))?
    } else {
        return Err(ProcessError::InvalidRequest(
            "restricted command executable must be absolute",
        ));
    };
    let mut resolved_command = command.to_vec();
    resolved_command[0] = resolved.to_string_lossy().into_owned();
    Ok(resolved_command)
}

fn search_environment_path(
    environment: &BTreeMap<String, String>,
    executable: &Path,
) -> Option<PathBuf> {
    let path = environment
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("PATH"))
        .map(|(_, value)| value)?;
    for directory in std::env::split_paths(std::ffi::OsStr::new(path)) {
        let candidate = directory.join(executable);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let extensions = environment
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("PATHEXT"))
                .map(|(_, value)| value.as_str())
                .unwrap_or(".COM;.EXE;.BAT;.CMD");
            for extension in extensions.split(';').filter(|value| !value.is_empty()) {
                let extension = extension.strip_prefix('.').unwrap_or(extension);
                let candidate = directory.join(executable).with_extension(extension);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

async fn read_pipe(
    mut reader: impl AsyncRead + Unpin,
    stream: ProcessOutputStream,
    output: mpsc::Sender<RuntimeOutput>,
) {
    let mut buffer = [0_u8; PIPE_READ_CHUNK];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                if output
                    .send(RuntimeOutput {
                        stream,
                        bytes: buffer[..read].to_vec(),
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
    }
}

async fn write_pipe_stdin(
    stdin: &tokio::sync::Mutex<Option<tokio::process::ChildStdin>>,
    bytes: Vec<u8>,
    close: bool,
) -> Result<(), ProcessError> {
    let mut stdin = stdin.lock().await;
    let writer = stdin.as_mut().ok_or(ProcessError::StdinClosed)?;
    if !bytes.is_empty() {
        writer
            .write_all(&bytes)
            .await
            .map_err(ProcessError::Spawn)?;
        writer.flush().await.map_err(ProcessError::Spawn)?;
    }
    if close && let Some(mut writer) = stdin.take() {
        writer.shutdown().await.map_err(ProcessError::Spawn)?;
    }
    Ok(())
}

async fn spawn_pty(
    launcher: Option<PathBuf>,
    command: Vec<String>,
    cwd: PathBuf,
    environment: BTreeMap<String, String>,
    size: ProcessTerminalSize,
    timeout: Option<Duration>,
) -> Result<SpawnedRuntime, ProcessError> {
    #[cfg(windows)]
    {
        let command = resolve_restricted_command(launcher.as_deref(), &command, &environment)?;
        conpty::spawn_conpty(launcher, command, cwd, environment, size, timeout).await
    }
    #[cfg(not(windows))]
    {
        let _ = (launcher, command, cwd, environment, size, timeout);
        Err(ProcessError::Pty(
            "the first interactive process backend is Windows ConPTY".into(),
        ))
    }
}

async fn wait_timeout(timeout: Option<Duration>) {
    match timeout {
        Some(timeout) => tokio::time::sleep(timeout).await,
        None => std::future::pending().await,
    }
}
