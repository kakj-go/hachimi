//! Sandbox readiness and trustworthy capability reporting.

mod appcontainer;
mod path_security;
mod process_backend;
mod restricted_process;
mod runtime_attestation;
mod runtime_manager;
mod setup;

pub use appcontainer::APP_CONTAINER_NAME;
pub use path_security::{
    PathAccess, PathSecurityError, WindowsFileIdentity, path_file_identity, resolve_checkout_path,
    validate_checkout_alias_root, validate_checkout_root,
};
pub use process_backend::{
    SandboxError, SandboxLaunchSpec, SandboxNetworkPolicy, SandboxSpawnFuture, SandboxedChild,
    SandboxedOutput,
};
pub use restricted_process::{RestrictedProcessError, run_restricted_process};
pub use runtime_attestation::{
    SANDBOX_POLICY_VERSION, attest_windows_runtime, attest_workspace_boundaries,
};
pub use runtime_manager::{SandboxManagerError, SandboxRuntimeManager};
pub use setup::{
    GitMutationAcl, SandboxSetupMarker, deny_restricted_code_read, deny_restricted_code_write,
    grant_restricted_code_access, install_sandbox_marker, prepare_git_mutation_acl,
    prepare_workspace_acl, restore_git_mutation_acl, revoke_restricted_code_access,
    set_managed_git_executable, trusted_git_executable, uninstall_sandbox,
};

use std::path::Path;

use hachimi_protocol::{SandboxCapabilityReport, SandboxReadiness, ToolEffect};
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum SandboxStatus {
    #[default]
    Disabled,
    SetupRequired,
    Degraded,
    Enforced,
}

impl SandboxStatus {
    #[must_use]
    pub const fn is_os_enforced(self) -> bool {
        matches!(self, Self::Enforced)
    }

    #[must_use]
    pub const fn permits(self, effect: ToolEffect) -> bool {
        matches!(
            effect,
            ToolEffect::ReadOnly | ToolEffect::BrowserObserve | ToolEffect::ComputerObserve
        ) || self.is_os_enforced()
    }

    #[must_use]
    pub fn from_report(report: &SandboxCapabilityReport) -> Self {
        if report.os_enforced
            && report.filesystem_enforced
            && report.process_enforced
            && report.network_enforced
            && report.readiness == SandboxReadiness::Ready
        {
            Self::Enforced
        } else {
            match report.readiness {
                SandboxReadiness::Unavailable => Self::Disabled,
                SandboxReadiness::SetupRequired => Self::SetupRequired,
                SandboxReadiness::Degraded | SandboxReadiness::Ready => Self::Degraded,
            }
        }
    }
}

pub trait SandboxBackend: Send + Sync {
    fn capability_report(&self) -> SandboxCapabilityReport;

    fn spawn_restricted(
        &self,
        spec: SandboxLaunchSpec,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> SandboxSpawnFuture<'_>;
}

#[derive(Debug, Clone)]
pub struct WindowsSandboxReadinessProbe {
    setup_marker: std::path::PathBuf,
    runtime: Option<RuntimeProbePaths>,
}

#[derive(Debug, Clone)]
struct RuntimeProbePaths {
    launcher: std::path::PathBuf,
    canary: std::path::PathBuf,
    attestation_root: std::path::PathBuf,
    expected_integrity: Vec<(std::path::PathBuf, String)>,
}

impl WindowsSandboxReadinessProbe {
    #[must_use]
    pub fn new(setup_marker: impl Into<std::path::PathBuf>) -> Self {
        Self {
            setup_marker: setup_marker.into(),
            runtime: None,
        }
    }

    #[must_use]
    pub fn with_runtime(
        mut self,
        launcher: impl Into<std::path::PathBuf>,
        canary: impl Into<std::path::PathBuf>,
        attestation_root: impl Into<std::path::PathBuf>,
    ) -> Self {
        self.runtime = Some(RuntimeProbePaths {
            launcher: launcher.into(),
            canary: canary.into(),
            attestation_root: attestation_root.into(),
            expected_integrity: Vec::new(),
        });
        self
    }

    #[must_use]
    pub fn with_runtime_integrity(
        mut self,
        expected_integrity: Vec<(std::path::PathBuf, String)>,
    ) -> Self {
        if let Some(runtime) = &mut self.runtime {
            runtime.expected_integrity = expected_integrity;
        }
        self
    }
}

impl SandboxBackend for WindowsSandboxReadinessProbe {
    fn capability_report(&self) -> SandboxCapabilityReport {
        self.runtime.as_ref().map_or_else(
            || probe_windows_readiness(&self.setup_marker),
            |runtime| {
                runtime_attestation::attest_windows_runtime_with_integrity(
                    &self.setup_marker,
                    &runtime.launcher,
                    &runtime.canary,
                    &runtime.attestation_root,
                    &runtime.expected_integrity,
                )
            },
        )
    }

    fn spawn_restricted(
        &self,
        spec: SandboxLaunchSpec,
        cancellation: tokio_util::sync::CancellationToken,
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
        let Some(runtime) = self.runtime.as_ref() else {
            return Box::pin(async { Err(SandboxError::RuntimeUnavailable) });
        };
        process_backend::spawn_with_launcher(runtime.launcher.clone(), spec, cancellation)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetupMarker {
    version: String,
    acl_component: bool,
    token_component: bool,
    network_component: bool,
}

#[must_use]
pub fn probe_windows_readiness(setup_marker: &Path) -> SandboxCapabilityReport {
    if !cfg!(windows) {
        return report(
            SandboxReadiness::Unavailable,
            Some("unsupported_os"),
            vec!["the first sandbox backend is Windows-only".into()],
        );
    }
    let bytes = match std::fs::read(setup_marker) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return report(
                SandboxReadiness::SetupRequired,
                Some("setup_marker_missing"),
                vec!["per-user Windows sandbox setup has not completed".into()],
            );
        }
        Err(_) => {
            return report(
                SandboxReadiness::Degraded,
                Some("setup_marker_unreadable"),
                vec!["Windows sandbox setup marker could not be read".into()],
            );
        }
    };
    let marker = match serde_json::from_slice::<SetupMarker>(&bytes) {
        Ok(marker) => marker,
        Err(_) => {
            return report(
                SandboxReadiness::Degraded,
                Some("setup_marker_invalid"),
                vec!["Windows sandbox setup marker is invalid".into()],
            );
        }
    };
    let mut diagnostics = Vec::new();
    if !marker.acl_component {
        diagnostics.push("filesystem ACL component is not ready".into());
    }
    if !marker.token_component {
        diagnostics.push("restricted token component is not ready".into());
    }
    if !marker.network_component {
        diagnostics.push("network policy component is not ready".into());
    }
    if diagnostics.is_empty() {
        diagnostics.push(
            "setup components are present; runtime attestation is still required before execution"
                .into(),
        );
    }
    SandboxCapabilityReport {
        backend: "windows_sandbox_v1".into(),
        readiness: SandboxReadiness::Degraded,
        os_enforced: false,
        filesystem_enforced: false,
        process_enforced: false,
        network_enforced: false,
        version: Some(marker.version),
        stable_error_code: Some("runtime_attestation_missing".into()),
        diagnostics,
    }
}

#[must_use]
pub fn unavailable_report() -> SandboxCapabilityReport {
    report(
        SandboxReadiness::Unavailable,
        Some("sandbox_unavailable"),
        vec!["no OS-enforced sandbox backend is active".into()],
    )
}

fn report(
    readiness: SandboxReadiness,
    code: Option<&str>,
    diagnostics: Vec<String>,
) -> SandboxCapabilityReport {
    SandboxCapabilityReport {
        backend: "none".into(),
        readiness,
        os_enforced: false,
        filesystem_enforced: false,
        process_enforced: false,
        network_enforced: false,
        version: None,
        stable_error_code: code.map(str::to_owned),
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_backend_never_claims_os_enforcement() {
        assert!(!SandboxStatus::Disabled.is_os_enforced());
        assert!(!unavailable_report().os_enforced);
    }

    #[test]
    fn side_effects_fail_closed_without_enforcement() {
        assert!(SandboxStatus::Disabled.permits(ToolEffect::ReadOnly));
        assert!(!SandboxStatus::Degraded.permits(ToolEffect::WorkspaceWrite));
        assert!(SandboxStatus::Enforced.permits(ToolEffect::Process));
    }
}
