// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/windows-sandbox-rs/src/{setup_orchestrator,setup_main_win}.rs
// @ 4c43465133428898aa84f0bfc02c306ed65fb66a.
// Modified for Hachimi: refreshable attestation, per-user repair,
// stable diagnostics, and fail-closed delegation to the restricted backend.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use hachimi_process_policy::{ProcessPolicy, std_command};
use hachimi_protocol::{
    SandboxBootstrapPhase, SandboxBootstrapState, SandboxCapabilityReport, SandboxReadiness,
    SandboxRuntimeSnapshot,
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::{
    SandboxBackend, SandboxError, SandboxLaunchSpec, SandboxSpawnFuture, SandboxStatus,
    WindowsSandboxReadinessProbe,
};

#[derive(Debug, Clone)]
pub struct SandboxManagerError {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for SandboxManagerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for SandboxManagerError {}

#[derive(Debug, Clone)]
struct RuntimeState {
    revision: u64,
    report: SandboxCapabilityReport,
    repairing: bool,
}

pub struct SandboxRuntimeManager {
    probe: Arc<WindowsSandboxReadinessProbe>,
    setup_helper: PathBuf,
    setup_marker: PathBuf,
    launcher: PathBuf,
    state: RwLock<RuntimeState>,
    repair_lock: Mutex<()>,
}

impl std::fmt::Debug for SandboxRuntimeManager {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SandboxRuntimeManager")
            .field("setup_helper", &self.setup_helper)
            .field("setup_marker", &self.setup_marker)
            .field("launcher", &self.launcher)
            .field("snapshot", &self.snapshot())
            .finish_non_exhaustive()
    }
}

impl SandboxRuntimeManager {
    #[must_use]
    pub fn new(
        probe: Arc<WindowsSandboxReadinessProbe>,
        setup_helper: impl Into<PathBuf>,
        setup_marker: impl Into<PathBuf>,
        launcher: impl Into<PathBuf>,
    ) -> Self {
        Self::new_with_report(probe, setup_helper, setup_marker, launcher, None)
    }

    #[must_use]
    pub fn new_with_report(
        probe: Arc<WindowsSandboxReadinessProbe>,
        setup_helper: impl Into<PathBuf>,
        setup_marker: impl Into<PathBuf>,
        launcher: impl Into<PathBuf>,
        initial_report: Option<SandboxCapabilityReport>,
    ) -> Self {
        let report = initial_report.unwrap_or_else(|| probe.capability_report());
        Self {
            probe,
            setup_helper: setup_helper.into(),
            setup_marker: setup_marker.into(),
            launcher: launcher.into(),
            state: RwLock::new(RuntimeState {
                revision: 1,
                report,
                repairing: false,
            }),
            repair_lock: Mutex::new(()),
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> SandboxRuntimeSnapshot {
        let state = self
            .state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        SandboxRuntimeSnapshot {
            revision: state.revision,
            report: state.report.clone(),
            repairing: state.repairing,
        }
    }

    #[must_use]
    pub fn bootstrap_state(&self) -> SandboxBootstrapState {
        let snapshot = self.snapshot();
        let marker = std::fs::read(&self.setup_marker)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<crate::SandboxSetupMarker>(&bytes).ok());
        let phase = if snapshot.repairing {
            SandboxBootstrapPhase::InstallingProfile
        } else if SandboxStatus::from_report(&snapshot.report) == SandboxStatus::Enforced {
            SandboxBootstrapPhase::Ready
        } else {
            match snapshot.report.readiness {
                SandboxReadiness::SetupRequired => SandboxBootstrapPhase::RepairRequired,
                SandboxReadiness::Unavailable => SandboxBootstrapPhase::Failed,
                SandboxReadiness::Degraded | SandboxReadiness::Ready => {
                    SandboxBootstrapPhase::RepairRequired
                }
            }
        };
        SandboxBootstrapState {
            phase,
            runtime_root: self
                .launcher
                .parent()
                .unwrap_or(Path::new(""))
                .to_string_lossy()
                .into_owned(),
            profile_sid: marker.and_then(|marker| marker.app_container_sid),
            stable_error_code: snapshot.report.stable_error_code.clone(),
            snapshot,
        }
    }

    pub async fn refresh(&self) -> SandboxRuntimeSnapshot {
        let probe = self.probe.clone();
        let report = tokio::task::spawn_blocking(move || probe.capability_report())
            .await
            .unwrap_or_else(|error| SandboxCapabilityReport {
                backend: "windows_sandbox_v1".into(),
                readiness: hachimi_protocol::SandboxReadiness::Degraded,
                os_enforced: false,
                filesystem_enforced: false,
                process_enforced: false,
                network_enforced: false,
                version: None,
                stable_error_code: Some("sandbox_attestation_task_failed".into()),
                diagnostics: vec![error.to_string()],
            });
        let mut state = self
            .state
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.revision = state.revision.saturating_add(1);
        state.report = report;
        SandboxRuntimeSnapshot {
            revision: state.revision,
            report: state.report.clone(),
            repairing: state.repairing,
        }
    }

    pub async fn repair(&self) -> Result<SandboxRuntimeSnapshot, SandboxManagerError> {
        let _guard = self
            .repair_lock
            .try_lock()
            .map_err(|_| SandboxManagerError {
                code: "sandbox_repair_in_progress",
                message: "Windows sandbox repair is already running".into(),
            })?;
        {
            let mut state = self
                .state
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.revision = state.revision.saturating_add(1);
            state.repairing = true;
            state.report.readiness = hachimi_protocol::SandboxReadiness::Degraded;
            state.report.os_enforced = false;
            state.report.filesystem_enforced = false;
            state.report.process_enforced = false;
            state.report.network_enforced = false;
            state.report.stable_error_code = Some("sandbox_repair_in_progress".into());
            state.report.diagnostics =
                vec!["Windows sandbox repair is in progress; side effects are fail-closed".into()];
        }
        let helper = self.setup_helper.clone();
        let marker = self.setup_marker.clone();
        let launcher = self.launcher.clone();
        let repair_result =
            tokio::task::spawn_blocking(move || run_per_user_setup(&helper, &marker, &launcher))
                .await
                .map_err(|error| SandboxManagerError {
                    code: "sandbox_repair_task_failed",
                    message: error.to_string(),
                })
                .and_then(|result| result);
        if let Err(error) = repair_result {
            let mut state = self
                .state
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.revision = state.revision.saturating_add(1);
            state.repairing = false;
            state.report.os_enforced = false;
            state.report.filesystem_enforced = false;
            state.report.process_enforced = false;
            state.report.network_enforced = false;
            state.report.stable_error_code = Some(error.code.into());
            state.report.diagnostics = vec![error.message.clone()];
            return Err(error);
        }
        self.refresh().await;
        let mut state = self
            .state
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.repairing = false;
        state.revision = state.revision.saturating_add(1);
        let snapshot = SandboxRuntimeSnapshot {
            revision: state.revision,
            report: state.report.clone(),
            repairing: false,
        };
        if SandboxStatus::from_report(&snapshot.report) != SandboxStatus::Enforced {
            return Err(SandboxManagerError {
                code: "sandbox_attestation_failed_after_repair",
                message: snapshot
                    .report
                    .diagnostics
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "Windows sandbox attestation failed after repair".into()),
            });
        }
        Ok(snapshot)
    }
}

impl SandboxBackend for SandboxRuntimeManager {
    fn capability_report(&self) -> SandboxCapabilityReport {
        self.snapshot().report
    }

    fn spawn_restricted(
        &self,
        spec: SandboxLaunchSpec,
        cancellation: CancellationToken,
    ) -> SandboxSpawnFuture<'_> {
        let report = self.capability_report();
        if SandboxStatus::from_report(&report) != SandboxStatus::Enforced {
            return Box::pin(async move {
                Err(SandboxError::NotEnforced(
                    report
                        .stable_error_code
                        .unwrap_or_else(|| "sandbox_not_enforced".into()),
                ))
            });
        }
        self.probe.spawn_restricted(spec, cancellation)
    }
}

#[cfg(windows)]
fn run_per_user_setup(
    helper: &Path,
    marker: &Path,
    launcher: &Path,
) -> Result<(), SandboxManagerError> {
    if !helper.is_absolute() || !marker.is_absolute() || !launcher.is_absolute() {
        return Err(SandboxManagerError {
            code: "sandbox_repair_path_invalid",
            message: "sandbox repair requires fixed absolute paths".into(),
        });
    }
    if !helper.is_file() || !launcher.is_file() {
        return Err(SandboxManagerError {
            code: "sandbox_repair_binary_missing",
            message: "sandbox setup helper or launcher is missing".into(),
        });
    }
    use std::os::windows::process::CommandExt;
    let mut command = std_command(helper, ProcessPolicy::HiddenCaptured);
    command
        .arg("--marker")
        .arg(marker)
        .arg("--launcher")
        .arg(launcher)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .creation_flags(0x0800_0000);
    if let Ok(git) = crate::trusted_git_executable() {
        command.env("HACHIMI_MANAGED_GIT_EXECUTABLE", git);
    }
    let output = command.output().map_err(|error| SandboxManagerError {
        code: "sandbox_setup_launch_failed",
        message: error.to_string(),
    })?;
    if !output.status.success() {
        return Err(SandboxManagerError {
            code: "sandbox_setup_helper_failed",
            message: String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(1_000)
                .collect(),
        });
    }
    Ok(())
}

#[cfg(not(windows))]
fn run_per_user_setup(
    _helper: &Path,
    _marker: &Path,
    _launcher: &Path,
) -> Result<(), SandboxManagerError> {
    Err(SandboxManagerError {
        code: "sandbox_repair_unsupported_os",
        message: "Windows sandbox repair is only supported on Windows".into(),
    })
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_setup_binary_keeps_runtime_fail_closed() {
        let root = tempfile::tempdir().expect("root");
        let marker = root.path().join("setup.json");
        let launcher = root.path().join("missing-launcher.exe");
        let probe = Arc::new(WindowsSandboxReadinessProbe::new(&marker));
        let manager = SandboxRuntimeManager::new(
            probe,
            root.path().join("missing-setup.exe"),
            &marker,
            &launcher,
        );
        let error = manager.repair().await.expect_err("repair must fail");
        assert_eq!(error.code, "sandbox_repair_binary_missing");
        let snapshot = manager.snapshot();
        assert!(!snapshot.report.os_enforced);
        assert_eq!(
            snapshot.report.stable_error_code.as_deref(),
            Some("sandbox_repair_binary_missing")
        );
        assert!(!snapshot.repairing);
    }
}
