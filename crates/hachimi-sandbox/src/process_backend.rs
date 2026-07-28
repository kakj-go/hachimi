//! Transport-neutral restricted process launch contract.
// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/windows-sandbox-rs/src/{wrapper,spawn_prep,process,resolved_permissions}.rs
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// Modified for Hachimi: typed Run/Checkout grants, AppContainer launcher, and bounded one-shot I/O.

use std::{ffi::OsString, future::Future, path::PathBuf, pin::Pin, process::Stdio, time::Duration};

use hachimi_protocol::{
    CapabilityGrantSet, CheckoutId, FileSystemAccess, PermissionGrantScope, RunId, SessionId,
    ToolEffect,
};
use thiserror::Error;
use tokio::process::{Child, Command};
use tokio_util::sync::CancellationToken;

const MAX_OUTPUT_LIMIT: usize = 16 * 1024 * 1024;
const ALLOWED_ENVIRONMENT: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
    "COMSPEC",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "LOCALAPPDATA",
    "APPDATA",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "LANG",
    "LC_ALL",
    "HACHIMI_WORKSPACE_WORKER_TOKEN",
    "HACHIMI_GIT_EXECUTABLE",
    "HACHIMI_GIT_DIR_ALIAS",
    "HACHIMI_GIT_WORK_TREE_ALIAS",
    "HACHIMI_GIT_WORK_TREE_REAL",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxNetworkPolicy {
    DenyAll,
}

#[derive(Debug, Clone)]
pub struct SandboxLaunchSpec {
    pub session_id: SessionId,
    pub run_id: RunId,
    pub run_generation: u64,
    pub checkout_id: CheckoutId,
    pub checkout_root: PathBuf,
    pub grants: CapabilityGrantSet,
    pub required_effect: ToolEffect,
    pub executable: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub environment: Vec<(OsString, OsString)>,
    pub stdin: Option<Vec<u8>>,
    /// Keeps stdin open for a long-lived protocol host. It cannot be combined
    /// with a one-shot `stdin` payload.
    pub interactive_stdin: bool,
    pub timeout: Duration,
    pub output_limit: usize,
    pub network_policy: SandboxNetworkPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxedOutput {
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub truncated: bool,
}

#[derive(Debug)]
pub struct SandboxedChild {
    child: Option<Child>,
    cancellation: CancellationToken,
    timeout: Duration,
    output_limit: usize,
}

impl SandboxedChild {
    pub(crate) fn new(
        child: Child,
        cancellation: CancellationToken,
        timeout: Duration,
        output_limit: usize,
    ) -> Self {
        Self {
            child: Some(child),
            cancellation,
            timeout,
            output_limit: output_limit.clamp(1, MAX_OUTPUT_LIMIT),
        }
    }

    pub async fn wait(mut self) -> Result<SandboxedOutput, SandboxError> {
        let child = self.child.take().ok_or(SandboxError::AlreadyWaited)?;
        let output = child.wait_with_output();
        tokio::pin!(output);
        let output = tokio::select! {
            biased;
            () = self.cancellation.cancelled() => return Err(SandboxError::Cancelled),
            () = tokio::time::sleep(self.timeout) => return Err(SandboxError::TimedOut),
            result = &mut output => result.map_err(SandboxError::Spawn)?,
        };
        let (stdout, stdout_truncated) = bounded(output.stdout, self.output_limit);
        let (stderr, stderr_truncated) = bounded(output.stderr, self.output_limit);
        Ok(SandboxedOutput {
            exit_code: output.status.code(),
            stdout,
            stderr,
            truncated: stdout_truncated || stderr_truncated,
        })
    }

    /// Transfers the attested launcher process to a long-lived protocol host.
    /// The caller becomes responsible for cancellation and process-tree shutdown.
    pub fn into_child(mut self) -> Result<Child, SandboxError> {
        self.child.take().ok_or(SandboxError::AlreadyWaited)
    }
}

impl Drop for SandboxedChild {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.start_kill();
        }
    }
}

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("sandbox runtime is not enforced: {0}")]
    NotEnforced(String),
    #[error("sandbox runtime paths are unavailable")]
    RuntimeUnavailable,
    #[error("sandbox launch environment contains a forbidden variable: {0}")]
    ForbiddenEnvironment(String),
    #[error("sandbox launch binding is invalid: {0}")]
    InvalidBinding(String),
    #[error("sandbox capability grant does not authorize the requested effect")]
    GrantDenied,
    #[error("sandbox process could not be spawned: {0}")]
    Spawn(std::io::Error),
    #[error("sandbox process was cancelled")]
    Cancelled,
    #[error("sandbox process timed out")]
    TimedOut,
    #[error("sandbox process has already been waited")]
    AlreadyWaited,
    #[error("sandbox launch cannot combine one-shot and interactive stdin")]
    ConflictingStdinMode,
}

pub type SandboxSpawnFuture<'a> =
    Pin<Box<dyn Future<Output = Result<SandboxedChild, SandboxError>> + Send + 'a>>;

pub(crate) fn spawn_with_launcher(
    launcher: PathBuf,
    spec: SandboxLaunchSpec,
    cancellation: CancellationToken,
) -> SandboxSpawnFuture<'static> {
    Box::pin(async move {
        validate_launch_spec(&spec)?;
        for (name, _) in &spec.environment {
            let normalized = name.to_string_lossy().to_ascii_uppercase();
            if !ALLOWED_ENVIRONMENT.contains(&normalized.as_str()) {
                return Err(SandboxError::ForbiddenEnvironment(
                    name.to_string_lossy().into_owned(),
                ));
            }
        }
        let mut command = Command::new(launcher);
        command
            .arg("--")
            .arg(&spec.executable)
            .args(&spec.args)
            .current_dir(&spec.cwd)
            .env_clear()
            .envs(spec.environment)
            .stdin(if spec.stdin.is_some() || spec.interactive_stdin {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(SandboxError::Spawn)?;
        if let Some(input) = spec.stdin {
            use tokio::io::AsyncWriteExt as _;

            let mut stdin = child.stdin.take().ok_or_else(|| {
                SandboxError::Spawn(std::io::Error::other(
                    "restricted process stdin is unavailable",
                ))
            })?;
            stdin.write_all(&input).await.map_err(SandboxError::Spawn)?;
            stdin.shutdown().await.map_err(SandboxError::Spawn)?;
        }
        Ok(SandboxedChild::new(
            child,
            cancellation,
            spec.timeout,
            spec.output_limit,
        ))
    })
}

fn validate_launch_spec(spec: &SandboxLaunchSpec) -> Result<(), SandboxError> {
    if spec.stdin.is_some() && spec.interactive_stdin {
        return Err(SandboxError::ConflictingStdinMode);
    }
    if spec.grants.scope != PermissionGrantScope::Run
        || spec.grants.session_id != spec.session_id
        || spec.grants.run_id.as_ref() != Some(&spec.run_id)
    {
        return Err(SandboxError::InvalidBinding(
            "grant Session/Run scope does not match the launch".into(),
        ));
    }
    if spec
        .grants
        .expires_at_ms
        .is_some_and(|expires| expires <= now_ms())
    {
        return Err(SandboxError::InvalidBinding(
            "capability grant expired before dispatch".into(),
        ));
    }
    if spec.network_policy != SandboxNetworkPolicy::DenyAll || spec.grants.network.enabled {
        return Err(SandboxError::InvalidBinding(
            "C2.1 only permits a deny-all network grant".into(),
        ));
    }
    let root = std::fs::canonicalize(&spec.checkout_root).map_err(SandboxError::Spawn)?;
    let cwd = std::fs::canonicalize(&spec.cwd).map_err(SandboxError::Spawn)?;
    if !component_prefix(&root, &cwd) {
        return Err(SandboxError::InvalidBinding(
            "process cwd is outside the bound Checkout".into(),
        ));
    }
    let allowed = match spec.required_effect {
        ToolEffect::ReadOnly => grant_has_root(&spec.grants, FileSystemAccess::Read, &root),
        ToolEffect::WorkspaceWrite => {
            grant_has_root(&spec.grants, FileSystemAccess::Write, &root)
                && spec.grants.process.spawn
        }
        ToolEffect::Process => {
            spec.grants.process.spawn
                && grant_has_root(&spec.grants, FileSystemAccess::Write, &root)
        }
        ToolEffect::ExternalSideEffect | ToolEffect::ComputerObserve | ToolEffect::ComputerAct => {
            false
        }
    };
    allowed.then_some(()).ok_or(SandboxError::GrantDenied)
}

fn grant_has_root(
    grants: &CapabilityGrantSet,
    access: FileSystemAccess,
    checkout_root: &std::path::Path,
) -> bool {
    grants.file_system.iter().any(|grant| {
        grant.access == access
            && grant.roots.iter().any(|root| {
                std::fs::canonicalize(root)
                    .ok()
                    .is_some_and(|root| component_prefix(&root, checkout_root))
            })
    })
}

fn component_prefix(root: &std::path::Path, candidate: &std::path::Path) -> bool {
    let root = root.components().collect::<Vec<_>>();
    let candidate = candidate.components().collect::<Vec<_>>();
    candidate.len() >= root.len()
        && root.iter().zip(candidate.iter()).all(|(left, right)| {
            #[cfg(windows)]
            {
                left.as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
            }
            #[cfg(not(windows))]
            {
                left == right
            }
        })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or_default()
}

fn bounded(mut bytes: Vec<u8>, limit: usize) -> (Vec<u8>, bool) {
    let truncated = bytes.len() > limit;
    bytes.truncate(limit);
    (bytes, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hachimi_protocol::{
        ComputerGrant, FileSystemGrant, NetworkGrant, PermissionProfile, ProcessGrant,
    };

    fn launch_spec(root: &std::path::Path) -> SandboxLaunchSpec {
        let session_id = SessionId::from("session-sandbox");
        let run_id = RunId::from("run-sandbox");
        SandboxLaunchSpec {
            session_id: session_id.clone(),
            run_id: run_id.clone(),
            run_generation: 7,
            checkout_id: CheckoutId::from("checkout-sandbox"),
            checkout_root: root.to_owned(),
            grants: CapabilityGrantSet {
                profile: PermissionProfile::WorkspaceWrite,
                scope: PermissionGrantScope::Run,
                session_id,
                run_id: Some(run_id),
                source: "test".into(),
                file_system: vec![
                    FileSystemGrant {
                        access: FileSystemAccess::Read,
                        roots: vec![root.to_string_lossy().into_owned()],
                        globs: Vec::new(),
                        special_roots: Vec::new(),
                    },
                    FileSystemGrant {
                        access: FileSystemAccess::Write,
                        roots: vec![root.to_string_lossy().into_owned()],
                        globs: Vec::new(),
                        special_roots: Vec::new(),
                    },
                ],
                network: NetworkGrant::default(),
                process: ProcessGrant {
                    spawn: true,
                    interactive: false,
                    allowed_commands: Vec::new(),
                },
                computer: ComputerGrant::default(),
                review_each_command: true,
                expires_at_ms: None,
            },
            required_effect: ToolEffect::WorkspaceWrite,
            executable: root.join("worker.exe"),
            args: Vec::new(),
            cwd: root.to_owned(),
            environment: Vec::new(),
            stdin: None,
            interactive_stdin: false,
            timeout: Duration::from_secs(5),
            output_limit: 1024,
            network_policy: SandboxNetworkPolicy::DenyAll,
        }
    }

    #[test]
    fn output_is_bounded_by_bytes() {
        assert_eq!(bounded(vec![1, 2, 3], 2), (vec![1, 2], true));
    }

    #[test]
    fn final_spawn_rejects_stale_scope_expiry_network_and_plan_grants() {
        let root = tempfile::tempdir().expect("root");
        let valid = launch_spec(root.path());
        validate_launch_spec(&valid).expect("valid launch");

        let mut conflicting_stdin = valid.clone();
        conflicting_stdin.stdin = Some(Vec::new());
        conflicting_stdin.interactive_stdin = true;
        assert!(matches!(
            validate_launch_spec(&conflicting_stdin),
            Err(SandboxError::ConflictingStdinMode)
        ));

        let mut mismatched_session = valid.clone();
        mismatched_session.grants.session_id = SessionId::from("other-session");
        assert!(matches!(
            validate_launch_spec(&mismatched_session),
            Err(SandboxError::InvalidBinding(_))
        ));

        let mut mismatched_run = valid.clone();
        mismatched_run.grants.run_id = Some(RunId::from("other-run"));
        assert!(matches!(
            validate_launch_spec(&mismatched_run),
            Err(SandboxError::InvalidBinding(_))
        ));

        let mut session_scope = valid.clone();
        session_scope.grants.scope = PermissionGrantScope::Session;
        assert!(matches!(
            validate_launch_spec(&session_scope),
            Err(SandboxError::InvalidBinding(_))
        ));

        let mut expired = valid.clone();
        expired.grants.expires_at_ms = Some(now_ms().saturating_sub(1));
        assert!(matches!(
            validate_launch_spec(&expired),
            Err(SandboxError::InvalidBinding(_))
        ));

        let mut network = valid.clone();
        network.grants.network.enabled = true;
        assert!(matches!(
            validate_launch_spec(&network),
            Err(SandboxError::InvalidBinding(_))
        ));

        let mut plan = valid.clone();
        plan.grants.profile = PermissionProfile::ReadOnly;
        plan.grants.process.spawn = false;
        plan.grants
            .file_system
            .retain(|grant| grant.access == FileSystemAccess::Read);
        assert!(matches!(
            validate_launch_spec(&plan),
            Err(SandboxError::GrantDenied)
        ));
    }

    #[test]
    fn final_spawn_rejects_a_cwd_outside_the_checkout_component_boundary() {
        let root = tempfile::tempdir().expect("root");
        let outside = tempfile::tempdir().expect("outside");
        let mut spec = launch_spec(root.path());
        spec.cwd = outside.path().to_owned();
        assert!(matches!(
            validate_launch_spec(&spec),
            Err(SandboxError::InvalidBinding(_))
        ));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn cancellation_drops_and_kills_the_spawned_process() {
        let directory = tempfile::tempdir().expect("directory");
        let marker = directory.path().join("late-marker.txt");
        let mut command = Command::new("powershell.exe");
        command
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 3; Set-Content -LiteralPath $args[0] -Value escaped",
            ])
            .arg(&marker)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let child = command.spawn().expect("child");
        let cancellation = CancellationToken::new();
        let sandboxed =
            SandboxedChild::new(child, cancellation.clone(), Duration::from_secs(10), 1024);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            cancellation.cancel();
        });
        assert!(matches!(
            sandboxed.wait().await,
            Err(SandboxError::Cancelled)
        ));
        tokio::time::sleep(Duration::from_secs(4)).await;
        assert!(!marker.exists(), "cancelled process wrote a late result");
    }
}
