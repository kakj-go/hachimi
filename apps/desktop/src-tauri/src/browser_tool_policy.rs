use hachimi_protocol::{
    AuthorityMode, BrowserAction, BrowserAutomationSurfaceKind, BrowserCapability,
    CapabilityGrantSet, HostPolicyDecision, PermissionProfile, RunId, SessionId,
};

pub(super) fn unattended_browser_target_access(
    grants: &CapabilityGrantSet,
    origin: &str,
) -> Result<bool, String> {
    if grants.profile == PermissionProfile::FullAccess {
        return Ok(true);
    }
    grants
        .browser
        .origins
        .iter()
        .any(|allowed| allowed == origin)
        .then_some(false)
        .ok_or_else(|| "Browser Origin is outside the preconfigured background grant".into())
}

pub(super) async fn require_interactive_browser_access(
    store: &hachimi_storage::AgentStore,
    session_id: &SessionId,
    run_id: &RunId,
    run_generation: u64,
    url: &str,
    surface: BrowserAutomationSurfaceKind,
    capabilities: &[BrowserCapability],
) -> Result<bool, String> {
    let origin = hachimi_browser::normalized_origin(url).map_err(|error| error.to_string())?;
    let private_network = match hachimi_browser::validate_agent_browser_target(url, false).await {
        Ok(()) => false,
        Err(hachimi_browser::BrowserHostError::PrivateNetworkDenied) => true,
        Err(error) => return Err(error.to_string()),
    };
    match store
        .browser_host_policy_decision(&origin, session_id, run_id, capabilities, private_network)
        .await
        .map_err(|error| error.to_string())?
    {
        HostPolicyDecision::Allow => Ok(private_network),
        HostPolicyDecision::Block => Err(format!(
            "Browser site access is blocked by policy for {origin}"
        )),
        HostPolicyDecision::Ask => {
            let request = store
                .create_browser_host_access_request(
                    session_id,
                    run_id,
                    run_generation,
                    &origin,
                    surface,
                    capabilities,
                    private_network,
                )
                .await
                .map_err(|error| error.to_string())?;
            let _ = store
                .append_event(
                    session_id,
                    Some(run_id),
                    "host.access_required",
                    serde_json::to_value(&request).map_err(|error| error.to_string())?,
                )
                .await;
            Err(format!(
                "Browser site access requires user confirmation: {}",
                request.id
            ))
        }
    }
}

pub(super) async fn require_external_session_access(
    store: &hachimi_storage::AgentStore,
    session: &hachimi_protocol::BrowserSession,
    authority_mode: AuthorityMode,
    grants: &CapabilityGrantSet,
    run_generation: u64,
    capabilities: &[BrowserCapability],
) -> Result<(), String> {
    let Some(url) = session.current_url.as_deref().or(session.origin.as_deref()) else {
        return Err("External Chrome has no current Origin".into());
    };
    if url.eq_ignore_ascii_case("about:blank") {
        return Ok(());
    }
    let origin = hachimi_browser::normalized_origin(url).map_err(|error| error.to_string())?;
    if authority_mode == AuthorityMode::Unattended {
        let allow_private_network = unattended_browser_target_access(grants, &origin)?;
        return hachimi_browser::validate_agent_browser_target(url, allow_private_network)
            .await
            .map_err(|error| error.to_string());
    }
    require_interactive_browser_access(
        store,
        &session.owner_session_id,
        &session.owner_run_id,
        run_generation,
        url,
        BrowserAutomationSurfaceKind::ExternalChrome,
        capabilities,
    )
    .await
    .map(|_| ())
}

pub(super) fn embedded_origin_policy<'a>(
    grants: &'a CapabilityGrantSet,
    authority_mode: AuthorityMode,
) -> crate::embedded_browser_agent::EmbeddedBrowserOriginPolicy<'a> {
    if authority_mode == AuthorityMode::Unattended {
        let full_access = grants.profile == PermissionProfile::FullAccess;
        crate::embedded_browser_agent::EmbeddedBrowserOriginPolicy {
            allowed_origins: &grants.browser.origins,
            allow_unlisted_origin: full_access,
            allow_private_network: full_access,
            require_site_permission: false,
        }
    } else {
        crate::embedded_browser_agent::EmbeddedBrowserOriginPolicy {
            allowed_origins: &grants.browser.origins,
            allow_unlisted_origin: true,
            allow_private_network: true,
            require_site_permission: true,
        }
    }
}

pub(super) const fn browser_action_category(action: &BrowserAction) -> &'static str {
    match action {
        BrowserAction::Navigate { .. } => "navigate",
        BrowserAction::Back => "back",
        BrowserAction::Forward => "forward",
        BrowserAction::Reload { .. } => "reload",
        BrowserAction::Stop => "stop",
        BrowserAction::Click { .. } => "click",
        BrowserAction::Hover { .. } => "hover",
        BrowserAction::DoubleClick { .. } => "double_click",
        BrowserAction::Scroll { .. } => "scroll",
        BrowserAction::DragDrop { .. } => "drag_drop",
        BrowserAction::Clear { .. } => "clear",
        BrowserAction::Fill { .. } => "fill",
        BrowserAction::SelectOption { .. } => "select_option",
        BrowserAction::PressKeys { .. } => "press_keys",
        BrowserAction::WaitFor { .. } => "wait_for",
        BrowserAction::TabList => "tab_list",
        BrowserAction::TabNew { .. } => "tab_new",
        BrowserAction::TabSwitch { .. } => "tab_switch",
        BrowserAction::TabClose { .. } => "tab_close",
        BrowserAction::TypeText { .. } => "type_text",
        BrowserAction::Upload { .. } => "upload",
        BrowserAction::Download { .. } => "download",
        BrowserAction::ReadStorage => "read_storage",
        BrowserAction::WriteStorage { .. } => "write_storage",
        BrowserAction::Cdp { .. } => "cdp",
    }
}

pub(super) const fn browser_capability_allowed(
    grants: &CapabilityGrantSet,
    capability: BrowserCapability,
) -> bool {
    match capability {
        BrowserCapability::Observe => grants.browser.observe,
        BrowserCapability::Act => grants.browser.act,
        BrowserCapability::Upload => grants.browser.upload,
        BrowserCapability::Download => grants.browser.download,
        BrowserCapability::CookieStorage => grants.browser.cookie_storage,
        BrowserCapability::Cdp => grants.browser.cdp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unattended_restricted_browser_requires_an_exact_origin() {
        let mut grants = CapabilityGrantSet::default();
        grants.browser.origins = vec!["https://example.com".into()];

        assert_eq!(
            unattended_browser_target_access(&grants, "https://example.com"),
            Ok(false)
        );
        assert!(unattended_browser_target_access(&grants, "https://other.example").is_err());
    }

    #[test]
    fn unattended_full_access_allows_any_validated_origin() {
        let grants = CapabilityGrantSet {
            profile: PermissionProfile::FullAccess,
            ..CapabilityGrantSet::default()
        };

        assert_eq!(
            unattended_browser_target_access(&grants, "https://example.com"),
            Ok(true)
        );
    }
}
