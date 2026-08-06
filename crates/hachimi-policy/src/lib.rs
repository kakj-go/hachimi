//! Deterministic, resource-aware policy decisions. Models and tool results never grant authority.

use std::path::Path;

use hachimi_protocol::{
    AgentPermissionPolicy, ApprovalPolicy, AuthorityMode, BehaviorMode, CapabilityGrantSet,
    ClientContext, ControlMethod, EntryProfile, FileSystemAccess, FileSystemGrant, NetworkGrant,
    PermissionGrantScope, PermissionProfile, ProcessGrant, RunId, Scope, ScopedPermissionRules,
    SessionId, ToolEffect, WorkloadKind,
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
            permission_profile: PermissionProfile::Writable,
            effect: ToolEffect::ReadOnly,
            action: method.as_str(),
            resource: "control-plane",
            capability_host: None,
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
    expand_permission_policy(
        &AgentPermissionPolicy {
            level: profile,
            rules: ScopedPermissionRules::default(),
            revision: 0,
        },
        AuthorityMode::Interactive,
        mode,
        session_id,
        run_id,
        workspace_root,
    )
}

#[must_use]
pub fn expand_permission_policy(
    policy: &AgentPermissionPolicy,
    authority_mode: AuthorityMode,
    mode: BehaviorMode,
    session_id: SessionId,
    run_id: RunId,
    workspace_root: String,
) -> CapabilityGrantSet {
    let profile = policy.level;
    let mut file_system = vec![FileSystemGrant {
        access: FileSystemAccess::Read,
        roots: vec![workspace_root.clone()],
        globs: Vec::new(),
        special_roots: Vec::new(),
    }];
    let mut network = policy.rules.network.clone();
    let mut process = policy.rules.process.clone();
    let mut browser = policy.rules.browser.clone();
    let mut computer = policy.rules.computer.clone();
    if !policy.rules.connectors.is_empty() {
        // This protocol marker lets ToolPlan expose the typed Connector
        // dispatcher even when every authorized action is read-only. The
        // invocation wrapper still validates account, action and revision.
        network.protocols.push("managed-connector".into());
    }
    file_system.extend(
        policy
            .rules
            .file_system
            .iter()
            .filter(|grant| {
                profile != PermissionProfile::ReadOnly || grant.access != FileSystemAccess::Write
            })
            .cloned(),
    );
    if profile != PermissionProfile::ReadOnly {
        file_system.push(FileSystemGrant {
            access: FileSystemAccess::Write,
            roots: vec![workspace_root],
            globs: Vec::new(),
            special_roots: Vec::new(),
        });
        process.spawn = true;
        if policy.rules.mcp.iter().any(|rule| !rule.read_only) {
            network.enabled = true;
            network.protocols.push("mcp".into());
        }
        if !policy.rules.connectors.is_empty() {
            network.enabled = true;
        }
        network.protocols.sort();
        network.protocols.dedup();
        if profile == PermissionProfile::FullAccess {
            file_system.push(FileSystemGrant {
                access: FileSystemAccess::Write,
                roots: Vec::new(),
                globs: Vec::new(),
                special_roots: vec![":root".into()],
            });
            network.enabled = true;
            network.hosts = vec!["*".into()];
            network.protocols = vec!["http".into(), "https".into(), "ws".into(), "wss".into()];
            process.interactive = true;
            process.allowed_commands.clear();
            browser.observe = true;
            browser.act = true;
            browser.upload = true;
            browser.download = true;
            computer.observe = true;
            computer.act = true;
        }
    } else {
        process = ProcessGrant::default();
        browser.act = false;
        browser.upload = false;
        browser.download = false;
        browser.cookie_storage = false;
        browser.cdp = false;
        computer.act = false;
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
        source: format!(
            "agent_permission_policy:{}:{}",
            policy.revision,
            match authority_mode {
                AuthorityMode::Interactive => "interactive",
                AuthorityMode::Unattended => "unattended",
            }
        ),
        file_system,
        network,
        process,
        browser,
        computer,
        review_each_command: true,
        expires_at_ms: None,
    }
}

/// Evaluates one concrete filesystem target. Deny rules always win, and a
/// write grant also admits reads from the same scope.
#[must_use]
pub fn file_system_grants_allow(
    grants: &[FileSystemGrant],
    required: FileSystemAccess,
    target: &Path,
) -> bool {
    if required == FileSystemAccess::Deny {
        return false;
    }
    if grants.iter().any(|grant| {
        grant.access == FileSystemAccess::Deny && file_system_grant_matches(grant, target)
    }) {
        return false;
    }
    grants.iter().any(|grant| {
        (grant.access == required
            || (required == FileSystemAccess::Read && grant.access == FileSystemAccess::Write))
            && file_system_grant_matches(grant, target)
    })
}

fn file_system_grant_matches(grant: &FileSystemGrant, target: &Path) -> bool {
    let target = normalized_path(target);
    let mut candidates = grant
        .roots
        .iter()
        .filter_map(|root| relative_to_root(&target, &normalized_path(Path::new(root))))
        .collect::<Vec<_>>();
    if grant.special_roots.iter().any(|root| root == ":root") {
        candidates.push(target.trim_start_matches('/').to_owned());
    }
    if grant.roots.is_empty() && grant.special_roots.is_empty() {
        candidates.push(target.trim_start_matches('/').to_owned());
    }
    !candidates.is_empty()
        && (grant.globs.is_empty()
            || candidates.iter().any(|candidate| {
                grant.globs.iter().any(|pattern| {
                    glob_matches(&normalize_case(&pattern.replace('\\', "/")), candidate)
                })
            }))
}

fn normalized_path(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    let trimmed = if value.len() > 1 {
        value.trim_end_matches('/')
    } else {
        &value
    };
    normalize_case(trimmed)
}

fn normalize_case(value: &str) -> String {
    if cfg!(windows) {
        value.to_lowercase()
    } else {
        value.to_owned()
    }
}

fn relative_to_root(target: &str, root: &str) -> Option<String> {
    if root.is_empty() {
        return None;
    }
    if target == root {
        return Some(String::new());
    }
    target
        .strip_prefix(root)
        .and_then(|suffix| suffix.strip_prefix('/'))
        .map(str::to_owned)
}

fn glob_matches(pattern: &str, value: &str) -> bool {
    fn matches_from(
        pattern: &[u8],
        value: &[u8],
        pattern_index: usize,
        value_index: usize,
        memo: &mut std::collections::HashMap<(usize, usize), bool>,
    ) -> bool {
        if let Some(result) = memo.get(&(pattern_index, value_index)) {
            return *result;
        }
        let result = if pattern_index == pattern.len() {
            value_index == value.len()
        } else if pattern[pattern_index] == b'*' {
            let recursive = pattern.get(pattern_index + 1) == Some(&b'*');
            let next_pattern = pattern_index + if recursive { 2 } else { 1 };
            matches_from(pattern, value, next_pattern, value_index, memo)
                || (recursive
                    && pattern.get(next_pattern) == Some(&b'/')
                    && matches_from(pattern, value, next_pattern + 1, value_index, memo))
                || (value_index < value.len()
                    && (recursive || value[value_index] != b'/')
                    && matches_from(pattern, value, pattern_index, value_index + 1, memo))
        } else if value_index < value.len()
            && (pattern[pattern_index] == value[value_index]
                || (pattern[pattern_index] == b'?' && value[value_index] != b'/'))
        {
            matches_from(pattern, value, pattern_index + 1, value_index + 1, memo)
        } else {
            false
        };
        memo.insert((pattern_index, value_index), result);
        result
    }

    matches_from(
        pattern.as_bytes(),
        value.as_bytes(),
        0,
        0,
        &mut std::collections::HashMap::new(),
    )
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

/// Returns true when every authority represented by `baseline` remains
/// available in `candidate`. Revisions are deliberately ignored.
#[must_use]
pub fn permission_policy_covers(
    candidate: &AgentPermissionPolicy,
    baseline: &AgentPermissionPolicy,
) -> bool {
    if candidate.level == PermissionProfile::FullAccess {
        return true;
    }
    if baseline.level == PermissionProfile::FullAccess
        || permission_rank(candidate.level) < permission_rank(baseline.level)
    {
        return false;
    }
    let candidate_rules = &candidate.rules;
    let baseline_rules = &baseline.rules;

    let deny_rules_unchanged = candidate_rules
        .file_system
        .iter()
        .filter(|grant| grant.access == FileSystemAccess::Deny)
        .all(|grant| baseline_rules.file_system.contains(grant));
    deny_rules_unchanged
        && baseline_rules
            .file_system
            .iter()
            .filter(|grant| grant.access != FileSystemAccess::Deny)
            .all(|grant| {
                candidate_rules.file_system.iter().any(|candidate| {
                    candidate.access == grant.access
                        && string_scope_covers(&candidate.roots, &grant.roots, false)
                        && string_scope_covers(&candidate.globs, &grant.globs, false)
                        && string_scope_covers(
                            &candidate.special_roots,
                            &grant.special_roots,
                            false,
                        )
                })
            })
        && (!baseline_rules.network.enabled
            || candidate_rules.network.enabled
                && string_scope_covers(
                    &candidate_rules.network.hosts,
                    &baseline_rules.network.hosts,
                    true,
                )
                && string_scope_covers(
                    &candidate_rules.network.protocols,
                    &baseline_rules.network.protocols,
                    false,
                ))
        && (!baseline_rules.process.spawn || candidate_rules.process.spawn)
        && (!baseline_rules.process.interactive || candidate_rules.process.interactive)
        && (!baseline_rules.process.spawn
            || string_scope_covers(
                &candidate_rules.process.allowed_commands,
                &baseline_rules.process.allowed_commands,
                true,
            ))
        && browser_covers(&candidate_rules.browser, &baseline_rules.browser)
        && computer_covers(&candidate_rules.computer, &baseline_rules.computer)
        && baseline_rules.mcp.iter().all(|rule| {
            candidate_rules.mcp.iter().any(|candidate| {
                candidate.server_id == rule.server_id
                    && candidate.tool_name == rule.tool_name
                    && candidate.schema_hash == rule.schema_hash
                    && (rule.read_only || !candidate.read_only)
            })
        })
        && baseline_rules.connectors.iter().all(|rule| {
            candidate_rules.connectors.iter().any(|candidate| {
                candidate.account_id == rule.account_id
                    && candidate.contribution_revision == rule.contribution_revision
                    && rule.actions.iter().all(|action| {
                        candidate.actions.contains(action)
                            && (rule.read_only_actions.contains(action)
                                || !candidate.read_only_actions.contains(action))
                    })
            })
        })
}

const fn permission_rank(profile: PermissionProfile) -> u8 {
    match profile {
        PermissionProfile::ReadOnly => 0,
        PermissionProfile::Writable => 1,
        PermissionProfile::FullAccess => 2,
    }
}

fn string_scope_covers(candidate: &[String], baseline: &[String], empty_is_all: bool) -> bool {
    (empty_is_all && candidate.is_empty())
        || baseline.iter().all(|value| {
            candidate
                .iter()
                .any(|allowed| allowed == value || allowed == "*")
        })
}

fn browser_covers(
    candidate: &hachimi_protocol::BrowserGrant,
    baseline: &hachimi_protocol::BrowserGrant,
) -> bool {
    (!baseline.observe || candidate.observe)
        && (!baseline.act || candidate.act)
        && (!baseline.upload || candidate.upload)
        && (!baseline.download || candidate.download)
        && (!baseline.cookie_storage || candidate.cookie_storage)
        && (!baseline.cdp || candidate.cdp)
        && (!(baseline.observe
            || baseline.act
            || baseline.upload
            || baseline.download
            || baseline.cookie_storage
            || baseline.cdp)
            || string_scope_covers(&candidate.origins, &baseline.origins, true))
}

fn computer_covers(
    candidate: &hachimi_protocol::ComputerGrant,
    baseline: &hachimi_protocol::ComputerGrant,
) -> bool {
    (!baseline.observe || candidate.observe)
        && (!baseline.act || candidate.act)
        && (!(baseline.observe || baseline.act)
            || string_scope_covers(&candidate.target_windows, &baseline.target_windows, true))
        && match (candidate.max_actions, baseline.max_actions) {
            (None, _) => true,
            (Some(candidate), Some(baseline)) => candidate >= baseline,
            (Some(_), None) => false,
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
        if context.permission_profile == PermissionProfile::FullAccess {
            return PolicyDecision::Allow;
        }
        let needs_approval = match context.approval_policy {
            ApprovalPolicy::AlwaysAskSideEffects => side_effect,
            ApprovalPolicy::OnlyWhenNeeded => matches!(
                context.effect,
                ToolEffect::ExternalSideEffect | ToolEffect::BrowserAct | ToolEffect::ComputerAct
            ),
            ApprovalPolicy::NeverPrompt => false,
        };
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
    use hachimi_protocol::{BrowserGrant, ComputerGrant};

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
            permission_profile: PermissionProfile::Writable,
            effect: ToolEffect::WorkspaceWrite,
            action: "workspace.edit",
            resource: "README.md",
            capability_host: Some("workspace-worker"),
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
            PermissionProfile::FullAccess,
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
    fn never_prompt_allows_tools_already_admitted_by_the_authority_snapshot() {
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
            permission_profile: PermissionProfile::Writable,
            effect: ToolEffect::Process,
            action: "workspace.exec",
            resource: "cargo test",
            capability_host: Some("workspace-worker"),
        };
        assert_eq!(policy.evaluate(&context), PolicyDecision::Allow);
    }

    #[test]
    fn policy_coverage_distinguishes_additions_from_revocations() {
        let mut baseline = AgentPermissionPolicy {
            level: PermissionProfile::Writable,
            rules: ScopedPermissionRules::default(),
            revision: 1,
        };
        baseline.rules.browser.observe = true;
        baseline.rules.browser.origins = vec!["https://example.com".into()];
        baseline.rules.computer.observe = true;
        baseline.rules.computer.target_windows = vec!["notepad.exe".into()];
        baseline.rules.computer.max_actions = Some(10);

        let mut wider = baseline.clone();
        wider.revision = 2;
        wider
            .rules
            .browser
            .origins
            .push("https://openai.com".into());
        wider.rules.computer.max_actions = Some(20);
        assert!(permission_policy_covers(&wider, &baseline));

        let mut narrower = wider;
        narrower.rules.browser.origins.remove(0);
        assert!(!permission_policy_covers(&narrower, &baseline));
        assert!(!permission_policy_covers(
            &AgentPermissionPolicy::default(),
            &baseline
        ));
    }

    #[test]
    fn filesystem_rules_apply_deny_before_root_and_glob_allows() {
        let grants = vec![
            FileSystemGrant {
                access: FileSystemAccess::Read,
                roots: vec!["C:\\workspace".into()],
                globs: vec!["src/**/*.rs".into()],
                special_roots: Vec::new(),
            },
            FileSystemGrant {
                access: FileSystemAccess::Deny,
                roots: vec!["C:\\workspace".into()],
                globs: vec!["src/private/**".into()],
                special_roots: Vec::new(),
            },
        ];
        assert!(file_system_grants_allow(
            &grants,
            FileSystemAccess::Read,
            Path::new("C:\\workspace\\src\\lib.rs")
        ));
        assert!(!file_system_grants_allow(
            &grants,
            FileSystemAccess::Read,
            Path::new("C:\\workspace\\src\\private\\secret.rs")
        ));
        assert!(!file_system_grants_allow(
            &grants,
            FileSystemAccess::Read,
            Path::new("C:\\workspace\\README.md")
        ));
    }

    #[test]
    fn filesystem_write_grant_also_allows_reads_but_not_parent_prefixes() {
        let grants = vec![FileSystemGrant {
            access: FileSystemAccess::Write,
            roots: vec!["C:\\workspace".into()],
            globs: Vec::new(),
            special_roots: Vec::new(),
        }];
        assert!(file_system_grants_allow(
            &grants,
            FileSystemAccess::Read,
            Path::new("C:\\workspace\\nested\\file.txt")
        ));
        assert!(!file_system_grants_allow(
            &grants,
            FileSystemAccess::Write,
            Path::new("C:\\workspace-other\\file.txt")
        ));
    }

    #[test]
    fn permission_profiles_cover_every_tool_effect_consistently_across_authority_modes() {
        let session = SessionId::from("matrix-session");
        let run = RunId::from("matrix-run");
        let root = "C:\\workspace".to_owned();
        let effects = [
            ToolEffect::ReadOnly,
            ToolEffect::WorkspaceWrite,
            ToolEffect::Process,
            ToolEffect::ExternalSideEffect,
            ToolEffect::BrowserObserve,
            ToolEffect::BrowserAct,
            ToolEffect::ComputerObserve,
            ToolEffect::ComputerAct,
        ];

        for mode in [AuthorityMode::Interactive, AuthorityMode::Unattended] {
            let read_only_policy = AgentPermissionPolicy {
                rules: ScopedPermissionRules {
                    browser: BrowserGrant {
                        observe: true,
                        origins: vec!["https://example.test".into()],
                        ..BrowserGrant::default()
                    },
                    computer: ComputerGrant {
                        observe: true,
                        act: false,
                        target_windows: vec!["example.exe".into()],
                        max_actions: None,
                    },
                    ..ScopedPermissionRules::default()
                },
                ..AgentPermissionPolicy::default()
            };
            let read_only = expand_permission_policy(
                &read_only_policy,
                mode,
                BehaviorMode::Default,
                session.clone(),
                run.clone(),
                root.clone(),
            );
            assert!(capability_grant_allows(&read_only, ToolEffect::ReadOnly));
            assert!(capability_grant_allows(
                &read_only,
                ToolEffect::BrowserObserve
            ));
            assert!(capability_grant_allows(
                &read_only,
                ToolEffect::ComputerObserve
            ));
            for effect in effects {
                if !matches!(
                    effect,
                    ToolEffect::ReadOnly | ToolEffect::BrowserObserve | ToolEffect::ComputerObserve
                ) {
                    assert!(!capability_grant_allows(&read_only, effect), "{effect:?}");
                }
            }

            let writable = AgentPermissionPolicy {
                level: PermissionProfile::Writable,
                rules: ScopedPermissionRules {
                    network: NetworkGrant {
                        enabled: true,
                        hosts: vec!["example.test".into()],
                        protocols: vec!["https".into()],
                    },
                    browser: BrowserGrant {
                        observe: true,
                        act: true,
                        origins: vec!["https://example.test".into()],
                        ..BrowserGrant::default()
                    },
                    computer: ComputerGrant {
                        observe: true,
                        act: true,
                        target_windows: vec!["example.exe".into()],
                        max_actions: Some(10),
                    },
                    ..ScopedPermissionRules::default()
                },
                revision: 1,
            };
            let writable = expand_permission_policy(
                &writable,
                mode,
                BehaviorMode::Default,
                session.clone(),
                run.clone(),
                root.clone(),
            );
            for effect in effects {
                assert!(capability_grant_allows(&writable, effect), "{effect:?}");
            }

            let full = expand_permission_policy(
                &AgentPermissionPolicy {
                    level: PermissionProfile::FullAccess,
                    ..AgentPermissionPolicy::default()
                },
                mode,
                BehaviorMode::Default,
                session.clone(),
                run.clone(),
                root.clone(),
            );
            for effect in effects {
                assert!(capability_grant_allows(&full, effect), "{effect:?}");
            }
        }

        let policy = AgentPermissionPolicy::default();
        let mut interactive = expand_permission_policy(
            &policy,
            AuthorityMode::Interactive,
            BehaviorMode::Default,
            session.clone(),
            run.clone(),
            root.clone(),
        );
        let mut unattended = expand_permission_policy(
            &policy,
            AuthorityMode::Unattended,
            BehaviorMode::Default,
            session,
            run,
            root,
        );
        interactive.source = "normalized".into();
        unattended.source = "normalized".into();
        assert_eq!(interactive, unattended);
    }
}
