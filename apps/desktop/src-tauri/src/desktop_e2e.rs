pub(crate) fn reject_release_e2e_environment() -> Result<(), std::io::Error> {
    if !cfg!(debug_assertions)
        && [
            "HACHIMI_DESKTOP_E2E_PROJECT_PATH",
            "HACHIMI_DESKTOP_E2E_ATTACHMENT_PATH",
            "HACHIMI_DESKTOP_E2E_SANDBOX",
            "HACHIMI_DESKTOP_E2E_PROVIDER",
            "HACHIMI_DESKTOP_E2E_ARTIFACTS",
            "HACHIMI_DESKTOP_E2E_MCP_URL",
            "HACHIMI_DESKTOP_E2E_APP",
        ]
        .iter()
        .any(|name| std::env::var_os(name).is_some())
    {
        return Err(std::io::Error::other(
            "desktop E2E environment variables are forbidden in release builds",
        ));
    }
    Ok(())
}

#[cfg(all(debug_assertions, feature = "desktop-e2e"))]
pub(crate) fn deterministic_e2e_sandbox_report() -> Option<hachimi_protocol::SandboxCapabilityReport>
{
    (std::env::var("HACHIMI_DESKTOP_E2E_SANDBOX").as_deref() == Ok("deterministic")).then(|| {
        hachimi_protocol::SandboxCapabilityReport {
            backend: "desktop-e2e-deterministic".into(),
            readiness: hachimi_protocol::SandboxReadiness::Ready,
            os_enforced: true,
            filesystem_enforced: true,
            process_enforced: true,
            network_enforced: true,
            version: Some("test-only-v1".into()),
            stable_error_code: None,
            diagnostics: vec!["debug-only deterministic UI test backend".into()],
        }
    })
}

#[cfg(all(debug_assertions, feature = "desktop-e2e"))]
#[derive(Debug, Default)]
struct DeterministicE2eSandboxBackend;

#[cfg(all(debug_assertions, feature = "desktop-e2e"))]
impl hachimi_sandbox::SandboxBackend for DeterministicE2eSandboxBackend {
    fn capability_report(&self) -> hachimi_protocol::SandboxCapabilityReport {
        deterministic_e2e_sandbox_report().expect("deterministic E2E backend is enabled")
    }

    fn spawn_restricted(
        &self,
        spec: hachimi_sandbox::SandboxLaunchSpec,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> hachimi_sandbox::SandboxSpawnFuture<'_> {
        Box::pin(async move {
            use tokio::io::AsyncWriteExt;

            let mut command = tokio::process::Command::new(&spec.executable);
            hachimi_process_policy::ProcessPolicy::HiddenCaptured.apply_tokio(&mut command);
            command
                .args(&spec.args)
                .current_dir(&spec.cwd)
                .env_clear()
                .envs(spec.environment)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);
            let mut child = command
                .spawn()
                .map_err(hachimi_sandbox::SandboxError::Spawn)?;
            if let Some(input) = spec.stdin
                && let Some(mut stdin) = child.stdin.take()
            {
                stdin
                    .write_all(&input)
                    .await
                    .map_err(hachimi_sandbox::SandboxError::Spawn)?;
            }
            Ok(hachimi_sandbox::SandboxedChild::new(
                child,
                cancellation,
                spec.timeout,
                spec.output_limit,
            ))
        })
    }
}

#[cfg(all(debug_assertions, feature = "desktop-e2e"))]
pub(crate) fn deterministic_e2e_sandbox_backend()
-> Option<std::sync::Arc<dyn hachimi_sandbox::SandboxBackend>> {
    deterministic_e2e_sandbox_report().map(|_| {
        std::sync::Arc::new(DeterministicE2eSandboxBackend)
            as std::sync::Arc<dyn hachimi_sandbox::SandboxBackend>
    })
}

#[cfg(all(debug_assertions, feature = "desktop-e2e"))]
pub(crate) fn deterministic_e2e_provider_enabled() -> bool {
    std::env::var("HACHIMI_DESKTOP_E2E_PROVIDER").as_deref() == Ok("deterministic")
}

#[cfg(not(all(debug_assertions, feature = "desktop-e2e")))]
pub(crate) fn deterministic_e2e_provider_enabled() -> bool {
    false
}

#[cfg(not(all(debug_assertions, feature = "desktop-e2e")))]
pub(crate) fn deterministic_e2e_sandbox_report() -> Option<hachimi_protocol::SandboxCapabilityReport>
{
    None
}

#[cfg(not(all(debug_assertions, feature = "desktop-e2e")))]
pub(crate) fn deterministic_e2e_sandbox_backend()
-> Option<std::sync::Arc<dyn hachimi_sandbox::SandboxBackend>> {
    None
}
