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
