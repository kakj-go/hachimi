use hachimi_protocol::{
    BrowserAction, BrowserCapability, CapabilityGrantSet, PermissionProfile, ScheduleHostGrant,
};

#[allow(clippy::too_many_arguments)]
pub(super) async fn require_embedded_navigation_permission(
    embedded: &crate::embedded_browser_agent::EmbeddedAgentBrowser,
    schedule_host_grant: Option<&ScheduleHostGrant>,
    session_id: &hachimi_protocol::SessionId,
    run_id: &hachimi_protocol::RunId,
    run_generation: u64,
    url: &str,
    allow_private_network: bool,
    lease_id: Option<&hachimi_protocol::BrowserAutomationLeaseId>,
) -> Result<(), crate::embedded_browser_agent::EmbeddedAgentBrowserError> {
    if schedule_host_grant.is_some() {
        return Ok(());
    }
    embedded
        .require_site_permission(
            session_id,
            run_id,
            run_generation,
            url,
            allow_private_network,
            lease_id,
        )
        .await
}

pub(super) fn embedded_origin_policy<'a>(
    grants: &'a CapabilityGrantSet,
    schedule_host_grant: Option<&'a ScheduleHostGrant>,
) -> crate::embedded_browser_agent::EmbeddedBrowserOriginPolicy<'a> {
    if let Some(browser) = schedule_host_grant.and_then(|grant| grant.browser.as_ref()) {
        crate::embedded_browser_agent::EmbeddedBrowserOriginPolicy {
            allowed_origins: &browser.document_origins,
            allow_unlisted_origin: false,
            allow_private_network: browser.allow_private_network,
            require_site_permission: false,
        }
    } else {
        crate::embedded_browser_agent::EmbeddedBrowserOriginPolicy {
            allowed_origins: &grants.browser.origins,
            allow_unlisted_origin: grants.profile == PermissionProfile::ExternalSandbox
                && grants.browser.origins.is_empty(),
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
