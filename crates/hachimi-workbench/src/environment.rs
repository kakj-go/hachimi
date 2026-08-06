use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use hachimi_protocol::{
    AgentWorkspaceStatus, CheckoutKind, ComputerControlStatus, EnvironmentActivity,
    EnvironmentChangeSummary, EnvironmentGitSummary, EnvironmentHandoffState, ForgeKind,
    GitRefRecord, GitRemoteRecord, PlanStepStatus, SessionContextBinding, SessionId,
    WorkbenchEnvironmentSnapshot,
};
use sha2::{Digest, Sha256};

use crate::{WorkbenchError, WorkbenchService, git_optional, git_required, now_ms};

const MAX_CHANGE_PATHS: usize = 20_000;
const MAX_UNTRACKED_TEXT_BYTES: u64 = 8 * 1024 * 1024;

impl WorkbenchService {
    pub async fn environment_snapshot(
        &self,
        session_id: &SessionId,
    ) -> Result<WorkbenchEnvironmentSnapshot, WorkbenchError> {
        let session = self
            .store
            .get_session(session_id)
            .await?
            .ok_or_else(|| WorkbenchError::SessionNotFound(session_id.clone()))?;
        let (
            checkout,
            workspace,
            root,
            baseline_revision,
            binding_revision,
            environment_revision,
            handoff,
        ) = match &session.context {
            SessionContextBinding::Project {
                project_id,
                checkout_id,
            } => {
                let checkout = self
                    .store
                    .get_checkout(checkout_id)
                    .await?
                    .ok_or_else(|| WorkbenchError::CheckoutNotFound(checkout_id.clone()))?;
                let state = self
                    .store
                    .ensure_session_environment_state(
                        session_id,
                        checkout_id,
                        checkout.kind,
                        checkout.head_revision.as_deref(),
                    )
                    .await?;
                let local_checkout_id = self
                    .store
                    .list_checkouts(project_id)
                    .await?
                    .into_iter()
                    .find(|candidate| candidate.kind == CheckoutKind::Local)
                    .map(|candidate| candidate.id);
                let active = self.store.checkout_has_active_runs(checkout_id).await?;
                let leased = self.store.checkout_has_write_lease(checkout_id).await?;
                let blocked_reason = if active {
                    Some("active_run".to_owned())
                } else if leased {
                    Some("write_lease".to_owned())
                } else {
                    None
                };
                let handoff = EnvironmentHandoffState {
                    local_checkout_id,
                    managed_checkout_id: state.managed_checkout_id.clone(),
                    can_handoff: blocked_reason.is_none(),
                    blocked_reason,
                };
                (
                    Some(checkout.clone()),
                    None,
                    PathBuf::from(&checkout.path),
                    state.baseline_revision,
                    state.binding_revision,
                    state.revision,
                    handoff,
                )
            }
            SessionContextBinding::Workspace { workspace_id } => {
                let workspace = self.store.workspace(workspace_id).await?.ok_or_else(|| {
                    WorkbenchError::WorkspaceUnavailable(format!("not found: {workspace_id}"))
                })?;
                if workspace.status != AgentWorkspaceStatus::Ready {
                    return Err(WorkbenchError::WorkspaceUnavailable(
                        workspace
                            .status_reason
                            .clone()
                            .unwrap_or_else(|| workspace.root_path.clone()),
                    ));
                }
                let root = PathBuf::from(&workspace.root_path);
                if !root.is_dir() {
                    return Err(WorkbenchError::WorkspaceUnavailable(
                        workspace.root_path.clone(),
                    ));
                }
                (
                    None,
                    Some(workspace.clone()),
                    root,
                    None,
                    0,
                    u64::try_from(workspace.updated_at_ms.max(0)).unwrap_or(u64::MAX),
                    EnvironmentHandoffState {
                        local_checkout_id: None,
                        managed_checkout_id: None,
                        can_handoff: false,
                        blocked_reason: Some("workspace_context".to_owned()),
                    },
                )
            }
        };
        let (changes, git) = git_environment(&root, baseline_revision.as_deref()).await?;
        let browser_lease = self
            .store
            .active_browser_automation_lease_for_session(session_id)
            .await?;
        let runs = self.store.list_runs(session_id).await?;
        let plans = self.store.list_proposed_plans(session_id).await?;
        let computer_controls = self
            .store
            .list_session_computer_control_sessions(session_id)
            .await?;
        let activity = if let Some(lease) = browser_lease {
            if let (Some(workspace_id), Some(browser_tab_id)) =
                (lease.workspace_id.as_ref(), lease.tab_id.as_ref())
            {
                self.store
                    .browser_workspace(workspace_id)
                    .await?
                    .tabs
                    .into_iter()
                    .find(|tab| tab.id == *browser_tab_id)
                    .map(|tab| EnvironmentActivity::Browser {
                        lease_id: lease.id.clone(),
                        surface: lease.surface,
                        browser_tab_id: Some(browser_tab_id.clone()),
                        browser_session_id: None,
                        run_id: lease.owner_run_id,
                        domain: display_domain(&tab.url),
                    })
            } else {
                let external_session_id = self
                    .store
                    .external_browser_session_for_lease(&lease.id)
                    .await?;
                self.store
                    .list_session_browser_sessions(session_id)
                    .await?
                    .into_iter()
                    .find(|session| Some(&session.id) == external_session_id.as_ref())
                    .map(|session| EnvironmentActivity::Browser {
                        lease_id: lease.id,
                        surface: lease.surface,
                        browser_tab_id: None,
                        browser_session_id: Some(session.id),
                        run_id: lease.owner_run_id,
                        domain: display_domain(
                            session
                                .current_url
                                .as_deref()
                                .or(session.origin.as_deref())
                                .unwrap_or(""),
                        ),
                    })
            }
        } else {
            None
        }
        .or_else(|| {
            computer_controls
                .iter()
                .find(|control| {
                    matches!(
                        control.status,
                        ComputerControlStatus::Active | ComputerControlStatus::Suspended
                    )
                })
                .and_then(|control| {
                    control
                        .app
                        .as_ref()
                        .map(|app| EnvironmentActivity::Computer {
                            control_session_id: control.id.clone(),
                            run_id: control.owner_run_id.clone(),
                            app_id: app.app_id.clone(),
                            app_name: app.display_name.clone(),
                        })
                })
        })
        .or_else(|| {
            runs.iter()
                .rev()
                .find(|run| !run.status.is_terminal())
                .and_then(|run| {
                    plans
                        .iter()
                        .rev()
                        .find(|plan| plan.accepted_run_id.as_ref() == Some(&run.id))
                })
                .and_then(|plan| {
                    plan.steps
                        .iter()
                        .find(|step| step.status == PlanStepStatus::InProgress)
                        .map(|step| EnvironmentActivity::Plan {
                            plan_id: plan.id.clone(),
                            step_id: step.id.clone(),
                            description: step.description.clone(),
                        })
                })
        });
        let sources = self.store.list_session_sources(session_id).await?;
        Ok(WorkbenchEnvironmentSnapshot {
            session_id: session.id,
            checkout,
            workspace,
            binding_revision,
            baseline_revision,
            changes,
            git,
            handoff,
            activity,
            sources,
            revision: environment_revision,
            generated_at_ms: now_ms(),
        })
    }
}

async fn git_environment(
    root: &Path,
    baseline: Option<&str>,
) -> Result<(EnvironmentChangeSummary, EnvironmentGitSummary), WorkbenchError> {
    let inside_work_tree = git_optional(root, &["rev-parse", "--is-inside-work-tree"])
        .await?
        .is_some_and(|value| value == "true");
    if !inside_work_tree {
        return Ok((
            EnvironmentChangeSummary::default(),
            EnvironmentGitSummary {
                branch: None,
                head_sha: None,
                detached: false,
                status_fingerprint: String::new(),
                uncommitted_files: 0,
                upstream: None,
                ahead: 0,
                behind: 0,
                default_comparison_ref: None,
                refs: Vec::new(),
                remotes: Vec::new(),
            },
        ));
    }
    let head = git_optional(root, &["rev-parse", "HEAD"]).await?;
    let branch = git_optional(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .await?
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let status = git_required(
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignored=no",
        ],
        None,
    )
    .await?;
    let refs = git_refs(root, branch.as_deref()).await?;
    let remotes = git_remotes(root).await?;
    let upstream = git_optional(root, &["rev-parse", "--abbrev-ref", "@{upstream}"])
        .await?
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let (ahead, behind) = ahead_behind(root, upstream.as_deref()).await?;
    let default_comparison_ref = default_comparison_ref(root, branch.as_deref(), &refs).await?;
    let detached = branch.is_none();
    Ok((
        change_summary(root, baseline).await?,
        EnvironmentGitSummary {
            branch,
            head_sha: head,
            detached,
            status_fingerprint: digest(status.as_bytes()),
            uncommitted_files: u32::try_from(status_records(&status)).unwrap_or(u32::MAX),
            upstream,
            ahead,
            behind,
            default_comparison_ref,
            refs,
            remotes,
        },
    ))
}

async fn change_summary(
    root: &Path,
    baseline: Option<&str>,
) -> Result<EnvironmentChangeSummary, WorkbenchError> {
    let baseline = baseline.or(Some("HEAD"));
    let mut args = vec!["diff", "--numstat", "--find-renames"];
    if let Some(value) = baseline {
        args.push(value);
    }
    args.push("--");
    let numstat = git_required(root, &args, None).await?;
    let mut paths = BTreeSet::new();
    let mut additions = 0_u64;
    let mut deletions = 0_u64;
    let mut truncated = false;
    for line in numstat.lines() {
        if paths.len() >= MAX_CHANGE_PATHS {
            truncated = true;
            break;
        }
        let mut fields = line.splitn(3, '\t');
        let added = fields.next().unwrap_or_default();
        let deleted = fields.next().unwrap_or_default();
        let path = fields.next().unwrap_or_default();
        if path.is_empty() {
            continue;
        }
        additions = additions.saturating_add(added.parse::<u64>().unwrap_or_default());
        deletions = deletions.saturating_add(deleted.parse::<u64>().unwrap_or_default());
        paths.insert(path.to_owned());
    }
    let untracked = git_required(
        root,
        &["ls-files", "--others", "--exclude-standard", "-z"],
        None,
    )
    .await?;
    for path in untracked.split('\0').filter(|value| !value.is_empty()) {
        if paths.len() >= MAX_CHANGE_PATHS {
            truncated = true;
            break;
        }
        let file = root.join(path);
        if let Ok(metadata) = std::fs::symlink_metadata(&file)
            && metadata.file_type().is_file()
        {
            if metadata.len() <= MAX_UNTRACKED_TEXT_BYTES
                && let Ok(bytes) = std::fs::read(&file)
                && !bytes.contains(&0)
                && std::str::from_utf8(&bytes).is_ok()
            {
                additions = additions.saturating_add(line_count(&bytes));
            }
            paths.insert(path.to_owned());
        }
    }
    Ok(EnvironmentChangeSummary {
        changed_files: u32::try_from(paths.len()).unwrap_or(u32::MAX),
        additions: u32::try_from(additions).unwrap_or(u32::MAX),
        deletions: u32::try_from(deletions).unwrap_or(u32::MAX),
        truncated,
    })
}

async fn git_refs(root: &Path, current: Option<&str>) -> Result<Vec<GitRefRecord>, WorkbenchError> {
    let output = git_required(
        root,
        &[
            "for-each-ref",
            "--format=%(refname:short)%09%(objectname)%09%(refname)",
            "refs/heads",
            "refs/remotes",
        ],
        None,
    )
    .await?;
    let mut refs = output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let name = fields.next()?.trim();
            let revision = fields.next()?.trim();
            let full_name = fields.next()?.trim();
            (!name.is_empty() && !revision.is_empty()).then(|| GitRefRecord {
                name: name.to_owned(),
                revision: revision.to_owned(),
                remote: full_name.starts_with("refs/remotes/"),
                current: current == Some(name),
            })
        })
        .collect::<Vec<_>>();
    refs.retain(|reference| !reference.name.ends_with("/HEAD"));
    refs.sort_by_key(|reference| (!reference.current, reference.remote, reference.name.clone()));
    Ok(refs)
}

async fn git_remotes(root: &Path) -> Result<Vec<GitRemoteRecord>, WorkbenchError> {
    let names = git_required(root, &["remote"], None).await?;
    let mut remotes = Vec::new();
    for name in names
        .lines()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let Some(url) = git_optional(root, &["remote", "get-url", name]).await? else {
            continue;
        };
        let url = url.trim();
        remotes.push(GitRemoteRecord {
            name: name.to_owned(),
            display_url: redact_remote_url(url),
            remote_url_hash: digest(url.as_bytes()),
            forge_kind: infer_forge_kind(url),
        });
    }
    Ok(remotes)
}

async fn ahead_behind(root: &Path, upstream: Option<&str>) -> Result<(u32, u32), WorkbenchError> {
    let Some(upstream) = upstream else {
        return Ok((0, 0));
    };
    let output = git_required(
        root,
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("HEAD...{upstream}"),
        ],
        None,
    )
    .await?;
    let mut fields = output.split_whitespace();
    Ok((
        fields
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0),
        fields
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0),
    ))
}

async fn default_comparison_ref(
    root: &Path,
    current: Option<&str>,
    refs: &[GitRefRecord],
) -> Result<Option<String>, WorkbenchError> {
    for remote in git_required(root, &["remote"], None).await?.lines() {
        if let Some(reference) = git_optional(
            root,
            &[
                "symbolic-ref",
                "--quiet",
                "--short",
                &format!("refs/remotes/{remote}/HEAD"),
            ],
        )
        .await?
        {
            return Ok(Some(reference.trim().to_owned()));
        }
    }
    Ok(["main", "master"]
        .into_iter()
        .find(|name| {
            refs.iter()
                .any(|reference| reference.name == *name && current != Some(*name))
        })
        .map(str::to_owned)
        .or_else(|| {
            refs.iter()
                .find(|reference| !reference.remote && !reference.current)
                .map(|reference| reference.name.clone())
        }))
}

fn status_records(status: &str) -> usize {
    let mut records = status.split('\0').filter(|record| !record.is_empty());
    let mut count = 0;
    while let Some(record) = records.next() {
        count += 1;
        let status_code = record.as_bytes().get(..2).unwrap_or_default();
        if status_code.iter().any(|code| matches!(code, b'R' | b'C')) {
            // In porcelain v1 -z output, rename/copy records are followed by
            // the original path as a second NUL-delimited field.
            let _ = records.next();
        }
    }
    count
}

fn line_count(bytes: &[u8]) -> u64 {
    if bytes.is_empty() {
        0
    } else {
        u64::try_from(bytes.iter().filter(|byte| **byte == b'\n').count()).unwrap_or(u64::MAX)
            + u64::from(bytes.last() != Some(&b'\n'))
    }
}

fn display_domain(origin: &str) -> String {
    url::Url::parse(origin)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| origin.trim_end_matches('/').to_owned())
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn infer_forge_kind(value: &str) -> ForgeKind {
    let value = value.to_ascii_lowercase();
    if value.contains("github.com") {
        ForgeKind::GitHub
    } else if value.contains("gitlab") {
        ForgeKind::GitLab
    } else if value.contains("gitee.com") {
        ForgeKind::Gitee
    } else if value.contains("gitea") || value.contains("forgejo") {
        ForgeKind::GiteaForgejo
    } else {
        ForgeKind::Unknown
    }
}

fn redact_remote_url(value: &str) -> String {
    let Some((scheme, rest)) = value.split_once("://") else {
        return value.to_owned();
    };
    let rest = rest.rsplit_once('@').map_or(rest, |(_, safe)| safe);
    format!("{scheme}://{rest}")
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use hachimi_protocol::{
        BehaviorMode, BrowserAutomationLeaseStatus, BrowserAutomationSurfaceKind,
        BrowserCapability, EntryProfile, ExecutionTarget, LlmSettings, PermissionProfile,
        PlanAcceptanceRequest, PlanId, PlanStep, PlanStepId, ProposedPlan, ProposedPlanStatus,
        RunStatus, WorkbenchTaskStartRequest,
    };
    use hachimi_storage::AgentStore;
    use tokio_util::sync::CancellationToken;

    use super::*;

    fn git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .expect("git");
        assert!(status.success());
    }

    #[test]
    fn counts_unterminated_lines() {
        assert_eq!(line_count(b"a\nb"), 2);
        assert_eq!(line_count(b"a\n"), 1);
        assert_eq!(line_count(b""), 0);
    }

    #[test]
    fn redacts_remote_credentials() {
        assert_eq!(
            redact_remote_url("https://user:secret@example.test/repo"),
            "https://example.test/repo"
        );
    }

    #[tokio::test]
    async fn session_baseline_counts_committed_staged_unstaged_and_untracked_changes() {
        let repository = tempfile::tempdir().expect("repository");
        git(repository.path(), &["init", "-b", "main"]);
        git(
            repository.path(),
            &["config", "user.email", "test@example.com"],
        );
        git(repository.path(), &["config", "user.name", "Hachimi Test"]);
        std::fs::write(repository.path().join("README.md"), "one\n").expect("readme");
        std::fs::write(repository.path().join("old.txt"), "rename\n").expect("rename seed");
        git(repository.path(), &["add", "."]);
        git(repository.path(), &["commit", "-m", "initial"]);

        let store = AgentStore::connect_in_memory().await.expect("store");
        let worktrees = tempfile::tempdir().expect("worktrees");
        let attachments = tempfile::tempdir().expect("attachments");
        let service = WorkbenchService::new(store, worktrees.path(), attachments.path());
        let project = service
            .add_project(repository.path())
            .await
            .expect("project");
        let task = service
            .create_task(
                &WorkbenchTaskStartRequest {
                    idempotency_key: "environment-task".into(),
                    entry_profile: EntryProfile::Workbench,
                    session_id: None,
                    project_id: Some(project.id.clone()),
                    prompt: "Track the environment".into(),
                    execution_target: Some(ExecutionTarget::Local {
                        project_id: project.id,
                    }),
                    behavior_mode: BehaviorMode::Plan,
                    permission_profile: PermissionProfile::ReadOnly,
                    attachment_ids: Vec::new(),
                    skill_ids: Vec::new(),
                },
                LlmSettings::default(),
                "test-user",
                "environment-task",
                &CancellationToken::new(),
            )
            .await
            .expect("task");

        std::fs::write(repository.path().join("committed.txt"), "committed\n")
            .expect("committed file");
        git(repository.path(), &["add", "committed.txt"]);
        git(repository.path(), &["commit", "-m", "after baseline"]);
        std::fs::write(repository.path().join("staged.txt"), "staged\n").expect("staged");
        git(repository.path(), &["add", "staged.txt"]);
        git(repository.path(), &["mv", "old.txt", "renamed.txt"]);
        std::fs::write(repository.path().join("README.md"), "one\ntwo\n").expect("unstaged");
        std::fs::write(repository.path().join("untracked.txt"), "first\nsecond\n")
            .expect("untracked");
        std::fs::write(repository.path().join("binary.bin"), [0_u8, 1, 2, 3]).expect("binary");

        let environment = service
            .environment_snapshot(&task.session.id)
            .await
            .expect("environment");
        assert_eq!(environment.changes.changed_files, 6);
        assert_eq!(environment.changes.additions, 5);
        assert_eq!(environment.changes.deletions, 0);
        assert!(!environment.changes.truncated);
        assert_eq!(environment.git.uncommitted_files, 5);

        let first_source = service
            .store()
            .upsert_session_web_source(
                &task.session.id,
                Some(&task.run.id),
                hachimi_protocol::SessionSourceOrigin::Mcp,
                "HTTPS://Example.COM:443/docs#first",
                Some("Original title"),
                None,
            )
            .await
            .expect("first source");
        let repeated_source = service
            .store()
            .upsert_session_web_source(
                &task.session.id,
                Some(&task.run.id),
                hachimi_protocol::SessionSourceOrigin::Connector,
                "https://example.com/docs#second",
                Some("Updated title"),
                None,
            )
            .await
            .expect("repeated source");
        assert_eq!(first_source.id, repeated_source.id);
        let sources = service
            .store()
            .list_session_sources(&task.session.id)
            .await
            .expect("sources");
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].url.as_deref(), Some("https://example.com/docs"));
        assert_eq!(
            sources[0].origin,
            hachimi_protocol::SessionSourceOrigin::Connector
        );

        service
            .store()
            .transition_run(&task.run.id, RunStatus::Preparing, None)
            .await
            .expect("preparing");
        service
            .store()
            .transition_run(&task.run.id, RunStatus::Running, None)
            .await
            .expect("running");
        service
            .store()
            .transition_run(&task.run.id, RunStatus::Succeeded, None)
            .await
            .expect("succeeded");
        let plan = service
            .store()
            .create_proposed_plan(ProposedPlan {
                id: PlanId::from("environment-plan"),
                session_id: task.session.id.clone(),
                run_id: task.run.id.clone(),
                revision: 0,
                goal: "Finish environment alignment".into(),
                assumptions: Vec::new(),
                steps: vec![PlanStep {
                    id: PlanStepId::from("step-pending"),
                    description: "Prepare changes".into(),
                    status: PlanStepStatus::Pending,
                }],
                affected_resources: Vec::new(),
                verification: Vec::new(),
                risks: Vec::new(),
                open_questions: Vec::new(),
                content_markdown: "Finish environment alignment".into(),
                status: ProposedPlanStatus::Proposed,
                accepted_run_id: None,
                created_at_ms: now_ms(),
                accepted_at_ms: None,
            })
            .await
            .expect("plan");
        let accepted = service
            .accept_plan(
                &PlanAcceptanceRequest {
                    idempotency_key: "accept-environment-plan".into(),
                    plan_id: plan.id,
                    expected_revision: plan.revision,
                    user_message: "Implement the plan".into(),
                },
                LlmSettings::default(),
                "test-user",
            )
            .await
            .expect("accepted plan");
        service
            .store()
            .update_execution_plan(
                &accepted.plan.id,
                &accepted.task.run.id,
                None,
                &[
                    PlanStep {
                        id: PlanStepId::from("step-complete"),
                        description: "Prepare changes".into(),
                        status: PlanStepStatus::Completed,
                    },
                    PlanStep {
                        id: PlanStepId::from("step-active"),
                        description: "Verify the environment summary".into(),
                        status: PlanStepStatus::InProgress,
                    },
                ],
            )
            .await
            .expect("execution plan");
        let plan_environment = service
            .environment_snapshot(&task.session.id)
            .await
            .expect("plan environment");
        assert!(matches!(
            plan_environment.activity,
            Some(EnvironmentActivity::Plan { description, .. })
                if description == "Verify the environment summary"
        ));

        let workspace = service
            .store()
            .get_or_create_browser_workspace(
                &task.session.id,
                Some("https://docs.example.com/guide"),
            )
            .await
            .expect("browser workspace");
        let lease = service
            .store()
            .create_browser_automation_lease(
                BrowserAutomationSurfaceKind::Embedded,
                Some(&workspace.id),
                Some(&workspace.active_tab_id),
                &task.session.id,
                &accepted.task.run.id,
                accepted.task.run.generation,
                &[BrowserCapability::Observe, BrowserCapability::Act],
                now_ms() + 60_000,
            )
            .await
            .expect("browser lease");
        let browser_environment = service
            .environment_snapshot(&task.session.id)
            .await
            .expect("browser environment");
        assert!(matches!(
            browser_environment.activity,
            Some(EnvironmentActivity::Browser { domain, .. }) if domain == "docs.example.com"
        ));

        service
            .store()
            .set_browser_automation_lease_status(
                &lease.id,
                lease.revision,
                BrowserAutomationLeaseStatus::Expired,
            )
            .await
            .expect("stopped browser lease");
        let resumed_plan = service
            .environment_snapshot(&task.session.id)
            .await
            .expect("resumed plan");
        assert!(matches!(
            resumed_plan.activity,
            Some(EnvironmentActivity::Plan { description, .. })
                if description == "Verify the environment summary"
        ));
    }
}
