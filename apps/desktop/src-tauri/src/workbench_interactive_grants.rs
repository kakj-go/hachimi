pub(super) fn for_workbench_run(
    mut grants: hachimi_protocol::CapabilityGrantSet,
    mode: hachimi_protocol::BehaviorMode,
) -> hachimi_protocol::CapabilityGrantSet {
    if mode == hachimi_protocol::BehaviorMode::Plan {
        return grants;
    }

    grants.source = "workbench_interactive".into();
    grants.browser.observe = true;
    grants.computer.observe = true;
    if grants.profile == hachimi_protocol::PermissionProfile::ReadOnly {
        return grants;
    }
    grants.network.enabled = true;
    grants.network.protocols.extend([
        "http".into(),
        "https".into(),
        "managed-connector".into(),
        "mcp".into(),
    ]);
    grants.network.protocols.sort();
    grants.network.protocols.dedup();
    grants.browser = hachimi_protocol::BrowserGrant {
        observe: true,
        act: true,
        upload: true,
        download: true,
        cookie_storage: true,
        cdp: false,
        origins: Vec::new(),
    };
    grants.computer = hachimi_protocol::ComputerGrant {
        observe: true,
        act: true,
        target_windows: Vec::new(),
        max_actions: Some(100),
    };
    grants
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interactive_host_capabilities_are_available_without_pregranting_origins() {
        let grants = for_workbench_run(
            hachimi_protocol::CapabilityGrantSet {
                profile: hachimi_protocol::PermissionProfile::FullAccess,
                ..hachimi_protocol::CapabilityGrantSet::default()
            },
            hachimi_protocol::BehaviorMode::Default,
        );
        assert!(grants.browser.observe);
        assert!(grants.browser.act);
        assert!(grants.browser.origins.is_empty());
        assert!(grants.computer.observe);
        assert!(grants.computer.act);
        assert!(!grants.browser.cdp);
    }

    #[test]
    fn plan_mode_keeps_host_control_disabled() {
        let grants = for_workbench_run(
            hachimi_protocol::CapabilityGrantSet::default(),
            hachimi_protocol::BehaviorMode::Plan,
        );
        assert!(!grants.browser.act);
        assert!(!grants.computer.act);
    }

    #[test]
    fn read_only_interactive_runs_can_request_observation_but_not_actions() {
        let grants = for_workbench_run(
            hachimi_protocol::CapabilityGrantSet {
                profile: hachimi_protocol::PermissionProfile::ReadOnly,
                ..hachimi_protocol::CapabilityGrantSet::default()
            },
            hachimi_protocol::BehaviorMode::Default,
        );
        assert!(grants.browser.observe);
        assert!(grants.computer.observe);
        assert!(!grants.browser.act);
        assert!(!grants.computer.act);
        assert!(!grants.network.enabled);
    }
}
