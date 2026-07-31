//! Deterministic, resource-aware policy decisions. Models and tool results never grant authority.

use hachimi_protocol::{
    ApprovalPolicy, BehaviorMode, CapabilityGrantSet, ClientContext, ComputerGrant, ControlMethod,
    EntryProfile, FileSystemAccess, FileSystemGrant, NetworkGrant, PermissionGrantScope,
    PermissionProfile, ProcessGrant, RunId, Scope, SessionId, ToolEffect, WorkloadKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Deny { code: &'static str },
    RequireApproval { code: &'static str },
}

#[derive(Debug)]
pub struct PolicyContext<'a> {
    pub client: &'a ClientContext,
    pub method: Option<ControlMethod>,
    pub required_scope: Scope,
    pub entry_profile: EntryProfile,
    pub workload: WorkloadKind,
    pub behavior_mode: BehaviorMode,
    pub approval_policy: ApprovalPolicy,
    pub permission_profile: PermissionProfile,
    pub effect: ToolEffect,
    pub action: &'a str,
    pub resource: &'a str,
    pub capability_host: Option<&'a str>,
    pub schedule_grant_hash: Option<&'a str>,
}

impl<'a> PolicyContext<'a> {
    #[must_use]
    pub fn control(
        client: &'a ClientContext,
        method: ControlMethod,
        required_scope: Scope,
    ) -> Self {
        Self {
            client,
            method: Some(method),
            required_scope,
            entry_profile: EntryProfile::Workbench,
            workload: WorkloadKind::General,
            behavior_mode: BehaviorMode::Default,
            approval_policy: ApprovalPolicy::OnlyWhenNeeded,
            permission_profile: PermissionProfile::WorkspaceWrite,
            effect: ToolEffect::ReadOnly,
            action: method.as_str(),
            resource: "control-plane",
            capability_host: None,
            schedule_grant_hash: None,
        }
    }
}

pub trait PolicyEngine: Send + Sync {
    fn evaluate(&self, context: &PolicyContext<'_>) -> PolicyDecision;
}

#[must_use]
pub fn expand_permission_profile(
    profile: PermissionProfile,
    mode: BehaviorMode,
    session_id: SessionId,
    run_id: RunId,
    workspace_root: String,
) -> CapabilityGrantSet {
    let mut file_system = vec![FileSystemGrant {
        access: FileSystemAccess::Read,
        roots: vec![workspace_root.clone()],
        globs: Vec::new(),
        special_roots: Vec::new(),
    }];
    let mut network = NetworkGrant::default();
    let mut process = ProcessGrant::default();
    let mut browser = hachimi_protocol::BrowserGrant::default();
    let mut computer = ComputerGrant::default();
    if profile != PermissionProfile::ReadOnly {
        file_system.push(FileSystemGrant {
            access: FileSystemAccess::Write,
            roots: vec![workspace_root],
            globs: Vec::new(),
            special_roots: Vec::new(),
        });
        process.spawn = true;
        if profile == PermissionProfile::ExternalSandbox {
            network.enabled = true;
            browser.observe = true;
            browser.act = true;
            browser.upload = true;
            browser.download = true;
            computer.observe = true;
            computer.act = true;
        }
    }
    if mode == BehaviorMode::Plan {
        file_system.retain(|grant| grant.access == FileSystemAccess::Read);
        network = NetworkGrant::default();
        process = ProcessGrant::default();
        browser.act = false;
        browser.upload = false;
        browser.download = false;
        browser.cookie_storage = false;
        browser.cdp = false;
        computer.act = false;
    }
    CapabilityGrantSet {
        profile: if mode == BehaviorMode::Plan {
            PermissionProfile::ReadOnly
        } else {
            profile
        },
        scope: PermissionGrantScope::Run,
        session_id,
        run_id: Some(run_id),
        source: "permission_profile".into(),
        file_system,
        network,
        process,
        browser,
        computer,
        review_each_command: true,
        expires_at_ms: None,
    }
}

#[must_use]
pub fn capability_grant_allows(grants: &CapabilityGrantSet, effect: ToolEffect) -> bool {
    match effect {
        ToolEffect::ReadOnly => grants
            .file_system
            .iter()
            .any(|grant| grant.access == FileSystemAccess::Read),
        ToolEffect::WorkspaceWrite => grants
            .file_system
            .iter()
            .any(|grant| grant.access == FileSystemAccess::Write),
        ToolEffect::Process => grants.process.spawn,
        ToolEffect::ExternalSideEffect => grants.network.enabled,
        ToolEffect::BrowserObserve => grants.browser.observe,
        // `browser_act` is the typed dispatcher for navigation as well as
        // separately granted upload/download/storage/CDP actions. The tool
        // executor performs the exact variant check; this coarse policy gate
        // must therefore admit any explicitly granted Browser action without
        // widening `BrowserGrant::act` to navigation/click/type.
        ToolEffect::BrowserAct => {
            grants.browser.act
                || grants.browser.upload
                || grants.browser.download
                || grants.browser.cookie_storage
                || grants.browser.cdp
        }
        ToolEffect::ComputerObserve => grants.computer.observe,
        ToolEffect::ComputerAct => grants.computer.act,
    }
}

#[derive(Debug, Default)]
pub struct DefaultPolicy;

impl PolicyEngine for DefaultPolicy {
    fn evaluate(&self, context: &PolicyContext<'_>) -> PolicyDecision {
        if !context.client.scopes.contains(&context.required_scope) {
            return PolicyDecision::Deny {
                code: "missing_scope",
            };
        }
        let side_effect = !matches!(
            context.effect,
            ToolEffect::ReadOnly | ToolEffect::BrowserObserve | ToolEffect::ComputerObserve
        );
        if context.behavior_mode == BehaviorMode::Plan && side_effect {
            return PolicyDecision::Deny {
                code: "plan_mode_read_only",
            };
        }
        if context.permission_profile == PermissionProfile::ReadOnly && side_effect {
            return PolicyDecision::Deny {
                code: "permission_profile_read_only",
            };
        }
        let needs_approval = match context.approval_policy {
            ApprovalPolicy::AlwaysAskSideEffects => side_effect,
            ApprovalPolicy::OnlyWhenNeeded => matches!(
                context.effect,
                ToolEffect::Process
                    | ToolEffect::ExternalSideEffect
                    | ToolEffect::BrowserAct
                    | ToolEffect::ComputerAct
            ),
            ApprovalPolicy::NeverPrompt => false,
        };
        if context.approval_policy == ApprovalPolicy::NeverPrompt
            && matches!(
                context.effect,
                ToolEffect::Process
                    | ToolEffect::ExternalSideEffect
                    | ToolEffect::BrowserAct
                    | ToolEffect::ComputerAct
            )
            && context.schedule_grant_hash.is_none()
        {
            return PolicyDecision::Deny {
                code: "approval_escalation_disabled",
            };
        }
        if needs_approval {
            return PolicyDecision::RequireApproval {
                code: "side_effect_requires_approval",
            };
        }
        PolicyDecision::Allow
    }
}

#[cfg(test)]
mod tests {
    use hachimi_core::WindowKind;

    use super::*;

    #[test]
    fn exact_scope_is_required() {
        let policy = DefaultPolicy;
        let pet = ClientContext::for_window(WindowKind::Pet);
        let context =
            PolicyContext::control(&pet, ControlMethod::SettingsRead, Scope::SettingsRead);
        assert_eq!(
            policy.evaluate(&context),
            PolicyDecision::Deny {
                code: "missing_scope"
            }
        );
    }

    #[test]
    fn plan_mode_rejects_write_independent_of_prompt() {
        let policy = DefaultPolicy;
        let mut client = ClientContext::for_window(WindowKind::Workbench);
        client.scopes.insert(Scope::WorkspaceWrite);
        let context = PolicyContext {
            client: &client,
            method: None,
            required_scope: Scope::WorkspaceWrite,
            entry_profile: EntryProfile::Workbench,
            workload: WorkloadKind::Coding,
            behavior_mode: BehaviorMode::Plan,
            approval_policy: ApprovalPolicy::OnlyWhenNeeded,
            permission_profile: PermissionProfile::WorkspaceWrite,
            effect: ToolEffect::WorkspaceWrite,
            action: "workspace.edit",
            resource: "README.md",
            capability_host: Some("workspace-worker"),
            schedule_grant_hash: None,
        };
        assert_eq!(
            policy.evaluate(&context),
            PolicyDecision::Deny {
                code: "plan_mode_read_only"
            }
        );
    }

    #[test]
    fn plan_mode_permission_expansion_removes_every_side_effect_grant() {
        let grants = expand_permission_profile(
            PermissionProfile::ExternalSandbox,
            BehaviorMode::Plan,
            SessionId::from("session"),
            RunId::from("run"),
            "C:\\workspace".into(),
        );
        assert_eq!(grants.profile, PermissionProfile::ReadOnly);
        assert!(capability_grant_allows(&grants, ToolEffect::ReadOnly));
        for effect in [
            ToolEffect::WorkspaceWrite,
            ToolEffect::Process,
            ToolEffect::ExternalSideEffect,
            ToolEffect::ComputerAct,
        ] {
            assert!(!capability_grant_allows(&grants, effect));
        }
    }

    #[test]
    fn never_prompt_denies_instead_of_granting_escalation() {
        let policy = DefaultPolicy;
        let mut client = ClientContext::for_window(WindowKind::Workbench);
        client.scopes.insert(Scope::WorkspaceExec);
        let context = PolicyContext {
            client: &client,
            method: None,
            required_scope: Scope::WorkspaceExec,
            entry_profile: EntryProfile::Workbench,
            workload: WorkloadKind::Coding,
            behavior_mode: BehaviorMode::Default,
            approval_policy: ApprovalPolicy::NeverPrompt,
            permission_profile: PermissionProfile::WorkspaceWrite,
            effect: ToolEffect::Process,
            action: "workspace.exec",
            resource: "cargo test",
            capability_host: Some("workspace-worker"),
            schedule_grant_hash: None,
        };
        assert_eq!(
            policy.evaluate(&context),
            PolicyDecision::Deny {
                code: "approval_escalation_disabled"
            }
        );
    }
}
