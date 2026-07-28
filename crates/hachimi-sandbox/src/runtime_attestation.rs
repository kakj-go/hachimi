// SPDX-License-Identifier: Apache-2.0
// Security boundary reviewed against openai/codex windows-sandbox-rs ACL/spawn smoke tests.
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// Modified for Hachimi: AppContainer canaries and per-Run Checkout/Git/TEMP attestation.

use std::{path::Path, process::Stdio};

use hachimi_protocol::{SandboxCapabilityReport, SandboxReadiness};

use crate::{
    SandboxSetupMarker,
    appcontainer::{APP_CONTAINER_NAME, AppContainerSid},
    deny_restricted_code_read, grant_restricted_code_access,
};

pub const SANDBOX_POLICY_VERSION: &str = "hachimi-windows-appcontainer-v3";

pub fn attest_workspace_boundaries(
    launcher: &Path,
    canary: &Path,
    checkout: &Path,
    run_temp: &Path,
    worker_program: &Path,
    read_only_roots: &[std::path::PathBuf],
) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("workspace boundary attestation is Windows-only".into());
    }
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let checkout_probe = checkout.join(format!(".hachimi-sandbox-write-{nonce}"));
    let temp_probe = run_temp.join(format!("hachimi-sandbox-temp-{nonce}"));
    let result = (|| {
        require_canary_success(
            launcher,
            canary,
            checkout,
            &["--touch", &checkout_probe.to_string_lossy()],
            "sandbox_checkout_write_attestation_failed",
        )?;
        require_canary_success(
            launcher,
            canary,
            run_temp,
            &["--touch", &temp_probe.to_string_lossy()],
            "sandbox_temp_write_attestation_failed",
        )?;
        require_canary_success(
            launcher,
            canary,
            checkout,
            &["--read", &worker_program.to_string_lossy()],
            "sandbox_worker_read_attestation_failed",
        )?;
        for root in read_only_roots {
            let denied = root.join(format!("hachimi-sandbox-denied-{nonce}"));
            if run_canary(
                launcher,
                canary,
                checkout,
                &["--touch", &denied.to_string_lossy()],
            )
            .is_ok_and(|status| status.success())
            {
                let _ = std::fs::remove_file(&denied);
                return Err("sandbox_read_only_root_write_succeeded".into());
            }
        }
        Ok(())
    })();
    let _ = std::fs::remove_file(checkout_probe);
    let _ = std::fs::remove_file(temp_probe);
    result
}

fn require_canary_success(
    launcher: &Path,
    canary: &Path,
    cwd: &Path,
    arguments: &[&str],
    code: &str,
) -> Result<(), String> {
    if run_canary(launcher, canary, cwd, arguments).is_ok_and(|status| status.success()) {
        Ok(())
    } else {
        Err(code.into())
    }
}

pub fn attest_windows_runtime(
    marker_path: &Path,
    launcher: &Path,
    canary: &Path,
    attestation_root: &Path,
) -> SandboxCapabilityReport {
    if !cfg!(windows) {
        return degraded(
            SandboxReadiness::Unavailable,
            "unsupported_os",
            "the enforced sandbox backend is Windows-only",
            None,
        );
    }
    let marker = match read_marker(marker_path) {
        Ok(marker) => marker,
        Err(report) => return report,
    };
    if marker.version != SANDBOX_POLICY_VERSION {
        return degraded(
            SandboxReadiness::SetupRequired,
            "sandbox_policy_version_mismatch",
            "the installed sandbox policy must be repaired",
            Some(marker.version),
        );
    }
    let resolved_identity = AppContainerSid::resolve()
        .and_then(|identity| identity.to_string_sid())
        .ok();
    if marker.app_container_name.as_deref() != Some(APP_CONTAINER_NAME)
        || marker.app_container_sid.as_deref() != resolved_identity.as_deref()
    {
        return degraded(
            SandboxReadiness::SetupRequired,
            "sandbox_identity_mismatch",
            "the installed AppContainer identity must be repaired",
            Some(marker.version),
        );
    }
    let mut missing = Vec::new();
    if !marker.acl_component {
        missing.push("filesystem ACL component");
    }
    if !marker.token_component {
        missing.push("restricted token component");
    }
    if !marker.network_component {
        missing.push("deny-all network policy component");
    }
    if !missing.is_empty() {
        return degraded(
            SandboxReadiness::Degraded,
            "sandbox_setup_incomplete",
            &format!("missing {}", missing.join(", ")),
            Some(marker.version),
        );
    }
    if !launcher.is_file() || !canary.is_file() {
        return degraded(
            SandboxReadiness::SetupRequired,
            "sandbox_runtime_binary_missing",
            "the sandbox launcher or canary binary is missing",
            Some(marker.version),
        );
    }
    let canary_root = attestation_root.join(format!("runtime-{}", std::process::id()));
    let allowed = canary_root.join("allowed");
    let forbidden = canary_root.join("forbidden");
    if std::fs::create_dir_all(&allowed).is_err() || std::fs::create_dir_all(&forbidden).is_err() {
        return degraded(
            SandboxReadiness::Degraded,
            "sandbox_canary_prepare_failed",
            "runtime attestation directories could not be created",
            Some(marker.version),
        );
    }
    let result = run_canaries(launcher, canary, &allowed, &forbidden);
    let _ = std::fs::remove_dir_all(&canary_root);
    match result {
        Ok(()) => SandboxCapabilityReport {
            backend: "windows_restricted_process_v1".into(),
            readiness: SandboxReadiness::Ready,
            os_enforced: true,
            filesystem_enforced: true,
            process_enforced: true,
            network_enforced: true,
            version: Some(marker.version),
            stable_error_code: None,
            diagnostics: vec![
                "restricted token, Job Object, filesystem boundary, and deny-all network canaries passed"
                    .into(),
            ],
        },
        Err((code, message)) => degraded(
            SandboxReadiness::Degraded,
            code,
            message,
            Some(marker.version),
        ),
    }
}

fn run_canaries(
    launcher: &Path,
    canary: &Path,
    allowed: &Path,
    forbidden: &Path,
) -> Result<(), (&'static str, &'static str)> {
    grant_restricted_code_access(allowed, true).map_err(|_| {
        (
            "sandbox_acl_attestation_failed",
            "restricted-code ACL could not be applied to the canary directory",
        )
    })?;
    deny_restricted_code_read(forbidden).map_err(|_| {
        (
            "sandbox_deny_read_attestation_failed",
            "the restricted-code deny-read ACL could not be applied to the canary directory",
        )
    })?;
    let allowed_file = allowed.join("write-ok.txt");
    if !run_canary(launcher, canary, allowed, &["--assert-job"])
        .is_ok_and(|status| status.success())
    {
        return Err((
            "sandbox_job_attestation_failed",
            "the restricted canary was not assigned to a Job Object",
        ));
    }
    if !run_canary(
        launcher,
        canary,
        allowed,
        &["--touch", &allowed_file.to_string_lossy()],
    )
    .is_ok_and(|status| status.success())
    {
        return Err((
            "sandbox_allowed_write_failed",
            "the restricted canary could not write its allowed directory",
        ));
    }
    let forbidden_file = forbidden.join("write-denied.txt");
    if run_canary(
        launcher,
        canary,
        allowed,
        &["--touch", &forbidden_file.to_string_lossy()],
    )
    .is_ok_and(|status| status.success())
    {
        return Err((
            "sandbox_forbidden_write_succeeded",
            "the restricted canary escaped its filesystem grant",
        ));
    }
    let forbidden_secret = forbidden.join("read-denied.txt");
    std::fs::write(&forbidden_secret, b"secret").map_err(|_| {
        (
            "sandbox_deny_read_attestation_failed",
            "the deny-read canary fixture could not be created",
        )
    })?;
    if run_canary(
        launcher,
        canary,
        allowed,
        &["--read", &forbidden_secret.to_string_lossy()],
    )
    .is_ok_and(|status| status.success())
    {
        return Err((
            "sandbox_forbidden_read_succeeded",
            "the restricted canary read a deny-read path",
        ));
    }
    let listener =
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).map_err(|_| {
            (
                "sandbox_network_canary_prepare_failed",
                "the local network canary listener could not start",
            )
        })?;
    let address = listener.local_addr().map_err(|_| {
        (
            "sandbox_network_canary_prepare_failed",
            "the local network canary address could not be read",
        )
    })?;
    if run_canary(
        launcher,
        canary,
        allowed,
        &["--network", &address.to_string()],
    )
    .is_ok_and(|status| status.success())
    {
        return Err((
            "sandbox_network_not_denied",
            "the restricted canary connected to a local network endpoint",
        ));
    }
    Ok(())
}

fn run_canary(
    launcher: &Path,
    canary: &Path,
    cwd: &Path,
    arguments: &[&str],
) -> std::io::Result<std::process::ExitStatus> {
    std::process::Command::new(launcher)
        .arg("--")
        .arg(canary)
        .args(arguments)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
}

fn read_marker(marker_path: &Path) -> Result<SandboxSetupMarker, SandboxCapabilityReport> {
    let bytes = std::fs::read(marker_path).map_err(|error| {
        let (readiness, code) = if error.kind() == std::io::ErrorKind::NotFound {
            (SandboxReadiness::SetupRequired, "setup_marker_missing")
        } else {
            (SandboxReadiness::Degraded, "setup_marker_unreadable")
        };
        degraded(
            readiness,
            code,
            "elevated Windows sandbox setup has not completed",
            None,
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|_| {
        degraded(
            SandboxReadiness::SetupRequired,
            "setup_marker_invalid",
            "the Windows sandbox setup marker is invalid",
            None,
        )
    })
}

fn degraded(
    readiness: SandboxReadiness,
    code: &str,
    diagnostic: &str,
    version: Option<String>,
) -> SandboxCapabilityReport {
    SandboxCapabilityReport {
        backend: "windows_restricted_process_v1".into(),
        readiness,
        os_enforced: false,
        filesystem_enforced: false,
        process_enforced: false,
        network_enforced: false,
        version,
        stable_error_code: Some(code.into()),
        diagnostics: vec![diagnostic.into()],
    }
}
