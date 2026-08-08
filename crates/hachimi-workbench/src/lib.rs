//! Project, Checkout and task-draft services for the coding Workbench.

mod attachment_host;
mod environment;
mod git_mutation;
mod handoff;
mod plan_acceptance;
#[cfg(test)]
mod workspace_plan_tests;

pub use attachment_host::AttachmentModelContext;

use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use hachimi_agent::{
    AgentRunCreateRequest, AgentRunFactoryError, AgentRunLaunchRequest, AgentRunLauncher,
};
use hachimi_process_policy::{ProcessPolicy, tokio_command};
use hachimi_protocol::{
    AgentPermissionPolicy, AttachmentId, AttachmentRecord, AuthorityMode, CheckoutId, CheckoutKind,
    CheckoutRecord, CheckoutStatus, ExecutionTarget, GitRefRecord, ItemPayload, LlmSettings,
    PlanId, PlanRevisionRequest, PlanSkipRequest, ProjectId, ProjectRecord, ProviderCapabilities,
    RunBudget, RunId, RunOrigin, RunPurpose, ScopedPermissionRules, SessionContextBinding,
    SessionId, WorkbenchAttachmentPreview, WorkbenchGitAction, WorkbenchGitPhaseResult,
    WorkbenchGitPhaseStatus, WorkbenchGitRequest, WorkbenchGitResponse, WorkbenchPlanSkipSnapshot,
    WorkbenchSessionListItem, WorkbenchSessionSnapshot, WorkbenchTaskSnapshot,
    WorkbenchTaskStartRequest, WorkloadKind, WorkspaceId,
};
use hachimi_storage::{AgentStore, AgentStoreError, IdempotentMutationClaim};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

const MAX_ATTACHMENT_BYTES: u64 = 25 * 1024 * 1024;
const MAX_ATTACHMENT_PREVIEW_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum WorkbenchError {
    #[error("agent Run creation failed: {0}")]
    Agent(#[from] AgentRunFactoryError),
    #[error("workbench storage failed: {0}")]
    Store(#[from] AgentStoreError),
    #[error("project path does not exist or is not a directory")]
    InvalidProjectPath,
    #[error("project does not exist: {0}")]
    ProjectNotFound(ProjectId),
    #[error("session does not exist: {0}")]
    SessionNotFound(SessionId),
    #[error("this operation requires a Project-bound Session")]
    ProjectContextRequired,
    #[error("workspace is unavailable: {0}")]
    WorkspaceUnavailable(String),
    #[error("archived Sessions must be restored before they can continue")]
    SessionArchived,
    #[error("the selected Session context does not match the requested task context")]
    SessionContextMismatch,
    #[error("checkout does not exist: {0}")]
    CheckoutNotFound(CheckoutId),
    #[error("execution target belongs to a different project")]
    ProjectTargetMismatch,
    #[error("managed worktrees require a Git project")]
    GitRequired,
    #[error("Git revision must not be empty")]
    EmptyRevision,
    #[error("Git command failed: {0}")]
    Git(String),
    #[error("Git HEAD changed since the operation was prepared")]
    GitHeadChanged,
    #[error("Git status changed since the operation was prepared")]
    GitStatusChanged,
    #[error("cannot switch branches while the checkout has uncommitted changes")]
    GitCheckoutDirty,
    #[error("Git action idempotency key must contain 1-128 bytes")]
    InvalidGitIdempotencyKey,
    #[error("the matching Git action is still in progress; refresh before retrying")]
    GitActionIndeterminate,
    #[error("workbench operation was cancelled")]
    Cancelled,
    #[error("task prompt must contain 1-32000 characters")]
    InvalidPrompt,
    #[error("the proposed plan revision changed; refresh before continuing")]
    StalePlanRevision,
    #[error("attachment must be a regular file with a usable name")]
    InvalidAttachmentFile,
    #[error("attachment exceeds the {MAX_ATTACHMENT_BYTES} byte limit")]
    AttachmentTooLarge,
    #[error("attachment does not exist: {0}")]
    AttachmentNotFound(AttachmentId),
    #[error("attachment I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("attachment worker failed: {0}")]
    Worker(String),
    #[error("worktree path has no parent")]
    InvalidWorktreeRoot,
    #[error("only managed worktrees can be cleaned up")]
    CleanupRequiresManagedWorktree,
    #[error("pinned worktree must be unpinned before cleanup")]
    CheckoutPinned,
    #[error("worktree has an active Run or write lease")]
    CheckoutInUse,
    #[error("worktree has uncommitted changes; Diff is preserved for review")]
    CheckoutDirty,
    #[error("managed worktree path is outside the configured worktree root")]
    CheckoutOutsideManagedRoot,
    #[error("the requested Handoff target does not differ from the active checkout")]
    InvalidHandoffTarget,
    #[error("Handoff preconditions changed; refresh the environment before retrying")]
    HandoffPreconditionFailed,
    #[error("the Handoff target contains changes that were not created by this Session")]
    HandoffTargetChanged,
    #[error("the Handoff Git histories cannot be moved with a fast-forward")]
    HandoffHistoryDiverged,
    #[error("Handoff failed and the original checkout was restored: {0}")]
    HandoffFailed(String),
    #[error("Handoff snapshot serialization failed: {0}")]
    HandoffSnapshotJson(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct WorkbenchService {
    store: AgentStore,
    worktree_root: PathBuf,
    attachment_root: PathBuf,
}

impl WorkbenchService {
    #[must_use]
    pub fn new(
        store: AgentStore,
        worktree_root: impl Into<PathBuf>,
        attachment_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            store,
            worktree_root: worktree_root.into(),
            attachment_root: attachment_root.into(),
        }
    }

    #[must_use]
    pub const fn store(&self) -> &AgentStore {
        &self.store
    }

    pub async fn import_attachment(
        &self,
        source: impl AsRef<Path>,
    ) -> Result<AttachmentRecord, WorkbenchError> {
        let source = source.as_ref().to_owned();
        let attachment_root = self.attachment_root.clone();
        let (attachment, managed_path) =
            tokio::task::spawn_blocking(move || stage_attachment(&source, &attachment_root))
                .await
                .map_err(|error| WorkbenchError::Worker(error.to_string()))??;
        Ok(self
            .store
            .upsert_attachment(&attachment, &managed_path)
            .await?)
    }

    pub async fn attachment_model_context(
        &self,
        run_id: &RunId,
    ) -> Result<Option<AttachmentModelContext>, WorkbenchError> {
        let attachments = self.store.list_run_managed_attachments(run_id).await?;
        attachment_host::load_attachment_model_context(self.attachment_root.clone(), attachments)
            .await
            .map_err(WorkbenchError::Worker)
    }

    pub async fn attachment_preview(
        &self,
        attachment_id: &AttachmentId,
    ) -> Result<WorkbenchAttachmentPreview, WorkbenchError> {
        let attachment = self
            .store
            .get_attachment(attachment_id)
            .await?
            .ok_or_else(|| AgentStoreError::AttachmentNotFound(attachment_id.clone()))?;
        let path = self.attachment_root.join(&attachment.content_hash);
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|error| WorkbenchError::Worker(error.to_string()))?;
        let truncated = bytes.len() > MAX_ATTACHMENT_PREVIEW_BYTES;
        let preview = &bytes[..bytes.len().min(MAX_ATTACHMENT_PREVIEW_BYTES)];
        let utf8_text = is_text_mime(&attachment.mime_type)
            .then(|| std::str::from_utf8(preview).ok().map(str::to_owned))
            .flatten();
        let data_url = attachment.mime_type.starts_with("image/").then(|| {
            format!(
                "data:{};base64,{}",
                attachment.mime_type,
                STANDARD.encode(preview)
            )
        });
        Ok(WorkbenchAttachmentPreview {
            attachment,
            utf8_text,
            data_url,
            truncated,
        })
    }

    pub async fn add_project(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<ProjectRecord, WorkbenchError> {
        let path = path.as_ref();
        if !path.is_dir() {
            return Err(WorkbenchError::InvalidProjectPath);
        }
        let canonical =
            std::fs::canonicalize(path).map_err(|_| WorkbenchError::InvalidProjectPath)?;
        let canonical_path = canonical.to_string_lossy();
        if let Some(existing) = self
            .store
            .list_projects()
            .await?
            .into_iter()
            .find(|project| project.root_path.eq_ignore_ascii_case(&canonical_path))
        {
            return Ok(existing);
        }
        let git_root = git_optional(&canonical, &["rev-parse", "--show-toplevel"]).await?;
        let now = now_ms();
        let project = ProjectRecord {
            id: ProjectId::random(),
            display_name: canonical
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Project")
                .to_owned(),
            root_path: canonical.to_string_lossy().into_owned(),
            git_root: git_root.map(|value| value.trim().to_owned()),
            trusted: false,
            created_at_ms: now,
            updated_at_ms: now,
        };
        self.store
            .create_project(&project)
            .await
            .map_err(WorkbenchError::from)
    }

    pub async fn projects(&self) -> Result<Vec<ProjectRecord>, WorkbenchError> {
        Ok(self.store.list_projects().await?)
    }

    pub async fn rename_project(
        &self,
        project_id: &ProjectId,
        display_name: &str,
    ) -> Result<ProjectRecord, WorkbenchError> {
        self.project(project_id).await?;
        Ok(self
            .store
            .update_project_display_name(project_id, display_name, now_ms())
            .await?)
    }

    pub async fn sessions(
        &self,
        project_id: Option<&ProjectId>,
    ) -> Result<Vec<WorkbenchSessionListItem>, WorkbenchError> {
        Ok(self.store.list_workbench_session_items(project_id).await?)
    }

    pub async fn session_snapshot(
        &self,
        session_id: &SessionId,
    ) -> Result<WorkbenchSessionSnapshot, WorkbenchError> {
        let session = self
            .store
            .get_session(session_id)
            .await?
            .ok_or_else(|| WorkbenchError::SessionNotFound(session_id.clone()))?;
        let checkout = match session.context.checkout_id() {
            Some(checkout_id) => self.store.get_checkout(checkout_id).await?,
            None => None,
        };
        let runs = self.store.list_runs(session_id).await?;
        let events = self.store.list_events(session_id, 0).await?;
        let transcript = self.store.list_transcript(session_id).await?;
        let attachment_ids = transcript
            .iter()
            .filter_map(|item| match &item.payload {
                ItemPayload::User { attachment_ids, .. } => Some(attachment_ids),
                _ => None,
            })
            .flatten()
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut attachments = Vec::with_capacity(attachment_ids.len());
        for attachment_id in attachment_ids {
            if let Some(attachment) = self.store.get_attachment(&attachment_id).await? {
                attachments.push(attachment);
            }
        }
        let pending_approvals = self
            .store
            .list_pending_approvals()
            .await?
            .into_iter()
            .filter(|approval| approval.session_id == *session_id)
            .collect();
        let plan_documents = self.store.list_plan_documents(session_id).await?;
        let plan_confirmations = self.store.list_plan_confirmations(session_id).await?;
        let execution_plans = self.store.list_execution_plans(session_id).await?;
        let artifacts = self.store.list_session_artifacts(session_id).await?;
        let agent_tasks = self.store.list_agent_tasks_for_session(session_id).await?;
        let run_summaries = self.store.list_run_summaries(session_id).await?;
        let browser_sessions = self.store.list_session_browser_sessions(session_id).await?;
        let browser_automation_leases = self
            .store
            .list_session_browser_automation_leases(session_id)
            .await?;
        let external_browser_observations = self
            .store
            .list_session_external_browser_observations(session_id)
            .await?;
        let host_access_requests = self
            .store
            .list_host_access_requests(Some(session_id))
            .await?;
        let computer_control_sessions = self
            .store
            .list_session_computer_control_sessions(session_id)
            .await?;
        let sources = self.store.list_session_sources(session_id).await?;
        Ok(WorkbenchSessionSnapshot {
            session,
            checkout,
            runs,
            events,
            transcript,
            attachments,
            pending_approvals,
            plan_documents,
            plan_confirmations,
            execution_plans,
            artifacts,
            agent_tasks,
            run_summaries,
            browser_sessions,
            browser_automation_leases,
            external_browser_observations,
            host_access_requests,
            computer_control_sessions,
            sources,
        })
    }

    pub async fn revise_plan(
        &self,
        request: &PlanRevisionRequest,
        model_snapshot: LlmSettings,
        principal: &str,
        cancellation: &CancellationToken,
    ) -> Result<WorkbenchTaskSnapshot, WorkbenchError> {
        let plan = self
            .store
            .get_plan_document(&request.plan_id)
            .await?
            .ok_or_else(|| AgentStoreError::ProposedPlanNotFound(request.plan_id.clone()))?;
        let confirmation = self
            .store
            .get_plan_confirmation(&request.plan_id)
            .await?
            .ok_or_else(|| AgentStoreError::ProposedPlanNotFound(request.plan_id.clone()))?;
        if plan.revision != request.expected_revision
            || matches!(
                confirmation.status,
                hachimi_protocol::PlanConfirmationStatus::Accepted
                    | hachimi_protocol::PlanConfirmationStatus::Skipped
            )
        {
            return Err(WorkbenchError::StalePlanRevision);
        }
        let instructions = request.instructions.trim();
        if instructions.is_empty() || instructions.chars().count() > 32_000 {
            return Err(WorkbenchError::InvalidPrompt);
        }
        let session = self
            .store
            .get_session(&plan.session_id)
            .await?
            .ok_or_else(|| WorkbenchError::SessionNotFound(plan.session_id.clone()))?;
        let source_run = self
            .store
            .get_run(&plan.source_run_id)
            .await?
            .ok_or_else(|| AgentStoreError::RunNotFound(plan.source_run_id.clone()))?;
        let (project_id, execution_target) = match &session.context {
            SessionContextBinding::Project {
                project_id,
                checkout_id,
            } => {
                let checkout = self
                    .store
                    .get_checkout(checkout_id)
                    .await?
                    .ok_or_else(|| WorkbenchError::CheckoutNotFound(checkout_id.clone()))?;
                let target = match checkout.kind {
                    CheckoutKind::Local => ExecutionTarget::Local {
                        project_id: project_id.clone(),
                    },
                    CheckoutKind::ManagedWorktree => ExecutionTarget::ManagedWorktree {
                        project_id: project_id.clone(),
                        base_revision: checkout.base_revision.unwrap_or_default(),
                    },
                };
                (Some(project_id.clone()), Some(target))
            }
            SessionContextBinding::Workspace { .. } => (None, None),
        };
        let attachment_ids = self
            .store
            .list_run_managed_attachments(&plan.source_run_id)
            .await?
            .into_iter()
            .map(|record| record.attachment.id)
            .collect();
        let task = self
            .create_task_inner(
                &WorkbenchTaskStartRequest {
                    idempotency_key: request.idempotency_key.clone(),
                    entry_profile: session.entry_profile,
                    session_id: Some(session.id),
                    project_id,
                    prompt: instructions.to_owned(),
                    execution_target,
                    behavior_mode: hachimi_protocol::BehaviorMode::Plan,
                    permission_profile: source_run.configuration.permission_profile,
                    attachment_ids,
                    skill_ids: Vec::new(),
                },
                model_snapshot,
                principal,
                &request.idempotency_key,
                cancellation,
                Some((&plan.id, plan.revision)),
            )
            .await?;
        Ok(task)
    }

    pub async fn skip_plan(
        &self,
        request: &PlanSkipRequest,
    ) -> Result<WorkbenchPlanSkipSnapshot, WorkbenchError> {
        let plan = self
            .store
            .get_plan_document(&request.plan_id)
            .await?
            .ok_or_else(|| AgentStoreError::ProposedPlanNotFound(request.plan_id.clone()))?;
        if plan.revision != request.expected_revision {
            return Err(WorkbenchError::StalePlanRevision);
        }
        let confirmation = self
            .store
            .get_plan_confirmation(&request.plan_id)
            .await?
            .ok_or_else(|| AgentStoreError::ProposedPlanNotFound(request.plan_id.clone()))?;
        let confirmation = match confirmation.status {
            hachimi_protocol::PlanConfirmationStatus::Pending => {
                self.store
                    .resolve_plan_confirmation(
                        &plan.id,
                        hachimi_protocol::PlanConfirmationStatus::Pending,
                        hachimi_protocol::PlanConfirmationStatus::Skipped,
                        now_ms(),
                    )
                    .await?
            }
            hachimi_protocol::PlanConfirmationStatus::Skipped => confirmation,
            _ => {
                return Err(WorkbenchError::Store(
                    AgentStoreError::ProposedPlanNotAcceptable(plan.id),
                ));
            }
        };
        Ok(WorkbenchPlanSkipSnapshot { plan, confirmation })
    }

    pub async fn git_refs(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<GitRefRecord>, WorkbenchError> {
        let project = self.project(project_id).await?;
        let git_root = project
            .git_root
            .as_deref()
            .ok_or(WorkbenchError::GitRequired)?;
        let current = git_optional(
            Path::new(git_root),
            &["symbolic-ref", "--quiet", "--short", "HEAD"],
        )
        .await?
        .unwrap_or_default();
        let output = git_required(
            Path::new(git_root),
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
                    name: name.into(),
                    revision: revision.into(),
                    remote: full_name.starts_with("refs/remotes/"),
                    current: name == current.trim(),
                })
            })
            .collect::<Vec<_>>();
        refs.sort_by_key(|reference| {
            (!reference.current, reference.remote, reference.name.clone())
        });
        Ok(refs)
    }

    pub async fn execute_git(
        &self,
        request: &WorkbenchGitRequest,
        principal: &str,
        generated_message: Option<&str>,
    ) -> Result<WorkbenchGitResponse, WorkbenchError> {
        if request.idempotency_key.trim().is_empty() || request.idempotency_key.len() > 128 {
            return Err(WorkbenchError::InvalidGitIdempotencyKey);
        }
        let request_hash = sha256_text(
            &serde_json::to_string(request).expect("Workbench Git requests are serializable"),
        );
        match self
            .store
            .claim_idempotent_mutation::<WorkbenchGitResponse>(
                principal,
                "workbench.git.execute",
                &request.idempotency_key,
                &request_hash,
                now_ms(),
            )
            .await?
        {
            IdempotentMutationClaim::Completed(response) => return Ok(response),
            IdempotentMutationClaim::Indeterminate => {
                return Err(WorkbenchError::GitActionIndeterminate);
            }
            IdempotentMutationClaim::Claimed => {}
        }
        let result = self.execute_git_claimed(request, generated_message).await;
        match &result {
            Ok(response) => {
                self.store
                    .complete_idempotent_mutation(
                        principal,
                        "workbench.git.execute",
                        &request.idempotency_key,
                        response,
                    )
                    .await?;
            }
            Err(_) => {
                self.store
                    .abandon_idempotent_mutation(
                        principal,
                        "workbench.git.execute",
                        &request.idempotency_key,
                    )
                    .await?;
            }
        }
        result
    }

    async fn execute_git_claimed(
        &self,
        request: &WorkbenchGitRequest,
        generated_message: Option<&str>,
    ) -> Result<WorkbenchGitResponse, WorkbenchError> {
        let session = self
            .store
            .get_session(&request.session_id)
            .await?
            .ok_or_else(|| WorkbenchError::SessionNotFound(request.session_id.clone()))?;
        if session.context.checkout_id() != Some(&request.checkout_id) {
            return Err(WorkbenchError::SessionContextMismatch);
        }
        let checkout = self
            .store
            .get_checkout(&request.checkout_id)
            .await?
            .ok_or_else(|| WorkbenchError::CheckoutNotFound(request.checkout_id.clone()))?;
        let root = Path::new(&checkout.path);
        let head = git_optional(root, &["rev-parse", "HEAD"]).await?;
        if request.expected_head.as_deref() != head.as_deref() {
            return Err(WorkbenchError::GitHeadChanged);
        }
        let status = git_required(root, &["status", "--porcelain=v1", "-z"], None).await?;
        let fingerprint = sha256_text(&status);
        if request.status_fingerprint != fingerprint {
            return Err(WorkbenchError::GitStatusChanged);
        }
        let latest_run = self
            .store
            .list_runs(&request.session_id)
            .await?
            .into_iter()
            .last()
            .ok_or_else(|| WorkbenchError::SessionNotFound(request.session_id.clone()))?;
        if request.context.expected_run_id.as_ref() != Some(&latest_run.id)
            || request.context.expected_generation != Some(latest_run.generation)
        {
            return Err(WorkbenchError::SessionContextMismatch);
        }
        let skipped = || WorkbenchGitPhaseResult {
            status: WorkbenchGitPhaseStatus::Skipped,
            message: None,
        };
        let mut stage = skipped();
        let mut commit = skipped();

        match &request.action {
            WorkbenchGitAction::Commit { message } => {
                if request.include_unstaged {
                    match git_required(root, &["add", "-A"], None).await {
                        Ok(_) => stage = git_phase_ok("staged working tree changes"),
                        Err(error) => stage = git_phase_failed(error.to_string()),
                    }
                }
                if stage.status != WorkbenchGitPhaseStatus::Failed {
                    let message = message
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned)
                        .or_else(|| {
                            generated_message
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                                .map(str::to_owned)
                        })
                        .unwrap_or_else(|| deterministic_commit_message(&status));
                    match git_required(root, &["commit", "-m", &message], None).await {
                        Ok(output) => commit = git_phase_ok(output.trim()),
                        Err(error) => commit = git_phase_failed(error.to_string()),
                    }
                }
            }
            WorkbenchGitAction::SwitchBranch { branch, remote } => {
                if !status.is_empty() {
                    return Err(WorkbenchError::GitCheckoutDirty);
                }
                stage = git_mutation::switch_branch(root, branch, *remote).await?;
                if stage.status == WorkbenchGitPhaseStatus::Succeeded {
                    let new_head = git_optional(root, &["rev-parse", "HEAD"]).await?;
                    self.store
                        .update_session_environment_baseline(
                            &request.session_id,
                            new_head.as_deref(),
                        )
                        .await?;
                }
            }
            WorkbenchGitAction::CreateBranch { branch } => {
                stage = git_mutation::create_branch(root, branch).await?;
            }
        }
        let final_head = git_optional(root, &["rev-parse", "HEAD"]).await?;
        let final_status = git_required(root, &["status", "--porcelain=v1", "-z"], None).await?;
        let branch = git_optional(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
            .await?
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        self.store
            .bump_session_environment_revision(&request.session_id)
            .await?;
        Ok(WorkbenchGitResponse {
            stage,
            commit,
            head: final_head,
            status_fingerprint: sha256_text(&final_status),
            branch,
        })
    }

    pub async fn prepare_checkout(
        &self,
        target: &ExecutionTarget,
        cancellation: &CancellationToken,
    ) -> Result<CheckoutRecord, WorkbenchError> {
        let project = self.project(target.project_id()).await?;
        match target {
            ExecutionTarget::Local { .. } => {
                if let Some(existing) = self
                    .store
                    .list_checkouts(&project.id)
                    .await?
                    .into_iter()
                    .find(|checkout| checkout.kind == CheckoutKind::Local)
                {
                    return Ok(existing);
                }
                let now = now_ms();
                let head_revision = match project.git_root.as_deref() {
                    Some(git_root) => {
                        git_optional(Path::new(git_root), &["rev-parse", "HEAD"]).await?
                    }
                    None => None,
                };
                let checkout = CheckoutRecord {
                    id: CheckoutId::random(),
                    project_id: project.id,
                    kind: CheckoutKind::Local,
                    path: project.root_path,
                    base_revision: head_revision.clone(),
                    head_revision,
                    status: CheckoutStatus::Ready,
                    pinned: true,
                    created_at_ms: now,
                    updated_at_ms: now,
                };
                Ok(self.store.create_checkout(&checkout).await?)
            }
            ExecutionTarget::ManagedWorktree { base_revision, .. } => {
                let git_root = project
                    .git_root
                    .as_deref()
                    .ok_or(WorkbenchError::GitRequired)?;
                if base_revision.trim().is_empty() {
                    return Err(WorkbenchError::EmptyRevision);
                }
                let checkout_id = CheckoutId::random();
                let checkout_path = self.worktree_root.join(checkout_id.as_str());
                let parent = checkout_path
                    .parent()
                    .ok_or(WorkbenchError::InvalidWorktreeRoot)?;
                std::fs::create_dir_all(parent)
                    .map_err(|error| WorkbenchError::Git(error.to_string()))?;
                git_required(
                    Path::new(git_root),
                    &[
                        "worktree",
                        "add",
                        "--detach",
                        checkout_path.to_string_lossy().as_ref(),
                        base_revision,
                    ],
                    Some(cancellation),
                )
                .await?;
                let head_revision = git_optional(&checkout_path, &["rev-parse", "HEAD"]).await?;
                let now = now_ms();
                let checkout = CheckoutRecord {
                    id: checkout_id,
                    project_id: project.id,
                    kind: CheckoutKind::ManagedWorktree,
                    path: checkout_path.to_string_lossy().into_owned(),
                    base_revision: Some(base_revision.clone()),
                    head_revision,
                    status: CheckoutStatus::Ready,
                    pinned: false,
                    created_at_ms: now,
                    updated_at_ms: now,
                };
                Ok(self.store.create_checkout(&checkout).await?)
            }
        }
    }

    pub async fn pin_checkout(
        &self,
        checkout_id: &CheckoutId,
        pinned: bool,
    ) -> Result<CheckoutRecord, WorkbenchError> {
        let checkout = self
            .store
            .get_checkout(checkout_id)
            .await?
            .ok_or_else(|| WorkbenchError::CheckoutNotFound(checkout_id.clone()))?;
        Ok(self
            .store
            .update_checkout_lifecycle(checkout_id, checkout.status, pinned)
            .await?)
    }

    pub async fn cleanup_checkout(
        &self,
        checkout_id: &CheckoutId,
    ) -> Result<CheckoutRecord, WorkbenchError> {
        let checkout = self
            .store
            .get_checkout(checkout_id)
            .await?
            .ok_or_else(|| WorkbenchError::CheckoutNotFound(checkout_id.clone()))?;
        if checkout.kind != CheckoutKind::ManagedWorktree {
            return Err(WorkbenchError::CleanupRequiresManagedWorktree);
        }
        if checkout.pinned {
            return Err(WorkbenchError::CheckoutPinned);
        }
        if self.store.checkout_has_active_runs(checkout_id).await?
            || self.store.checkout_has_write_lease(checkout_id).await?
        {
            return Err(WorkbenchError::CheckoutInUse);
        }
        let root = std::fs::canonicalize(&self.worktree_root)
            .map_err(|_| WorkbenchError::CheckoutOutsideManagedRoot)?;
        let checkout_path = Path::new(&checkout.path);
        if checkout_path.exists() {
            let canonical = std::fs::canonicalize(checkout_path)
                .map_err(|_| WorkbenchError::CheckoutOutsideManagedRoot)?;
            if !canonical.starts_with(&root)
                || canonical.file_name().and_then(|name| name.to_str())
                    != Some(checkout.id.as_str())
            {
                return Err(WorkbenchError::CheckoutOutsideManagedRoot);
            }
            let dirty = git_required(&canonical, &["status", "--porcelain=v1"], None).await?;
            if !dirty.is_empty() {
                self.store
                    .update_checkout_lifecycle(checkout_id, CheckoutStatus::CleanupBlocked, false)
                    .await?;
                return Err(WorkbenchError::CheckoutDirty);
            }
            let project = self.project(&checkout.project_id).await?;
            let git_root = project
                .git_root
                .as_deref()
                .ok_or(WorkbenchError::GitRequired)?;
            let canonical_argument = canonical.to_string_lossy().into_owned();
            git_required(
                Path::new(git_root),
                &["worktree", "remove", &canonical_argument],
                None,
            )
            .await?;
            let _ = git_required(Path::new(git_root), &["worktree", "prune"], None).await?;
        }
        Ok(self
            .store
            .update_checkout_lifecycle(checkout_id, CheckoutStatus::Removed, false)
            .await?)
    }

    pub async fn create_task(
        &self,
        request: &WorkbenchTaskStartRequest,
        model_snapshot: LlmSettings,
        principal: &str,
        idempotency_key: &str,
        cancellation: &CancellationToken,
    ) -> Result<WorkbenchTaskSnapshot, WorkbenchError> {
        self.create_task_inner(
            request,
            model_snapshot,
            principal,
            idempotency_key,
            cancellation,
            None,
        )
        .await
    }

    async fn create_task_inner(
        &self,
        request: &WorkbenchTaskStartRequest,
        model_snapshot: LlmSettings,
        principal: &str,
        idempotency_key: &str,
        cancellation: &CancellationToken,
        revised_plan: Option<(&PlanId, u32)>,
    ) -> Result<WorkbenchTaskSnapshot, WorkbenchError> {
        let prompt = request.prompt.trim();
        if prompt.is_empty() || prompt.chars().count() > 32_000 {
            return Err(WorkbenchError::InvalidPrompt);
        }
        for attachment_id in &request.attachment_ids {
            if self.store.get_attachment(attachment_id).await?.is_none() {
                return Err(WorkbenchError::AttachmentNotFound(attachment_id.clone()));
            }
        }
        let mut existing_session = None;
        let (project, checkout, context, execution_target, workload_override) =
            if let Some(session_id) = &request.session_id {
                let session = self
                    .store
                    .get_session(session_id)
                    .await?
                    .ok_or_else(|| WorkbenchError::SessionNotFound(session_id.clone()))?;
                if session.archived {
                    return Err(WorkbenchError::SessionArchived);
                }
                if session.entry_profile != request.entry_profile {
                    return Err(WorkbenchError::SessionContextMismatch);
                }
                let resolved = match &session.context {
                    SessionContextBinding::Project {
                        project_id,
                        checkout_id,
                    } => {
                        if request
                            .project_id
                            .as_ref()
                            .is_some_and(|requested| requested != project_id)
                        {
                            return Err(WorkbenchError::SessionContextMismatch);
                        }
                        let project = self.project(project_id).await?;
                        let checkout =
                            self.store.get_checkout(checkout_id).await?.ok_or_else(|| {
                                WorkbenchError::CheckoutNotFound(checkout_id.clone())
                            })?;
                        let target = match checkout.kind {
                            CheckoutKind::Local => ExecutionTarget::Local {
                                project_id: project_id.clone(),
                            },
                            CheckoutKind::ManagedWorktree => ExecutionTarget::ManagedWorktree {
                                project_id: project_id.clone(),
                                base_revision: checkout.base_revision.clone().unwrap_or_default(),
                            },
                        };
                        if request
                            .execution_target
                            .as_ref()
                            .is_some_and(|requested| requested != &target)
                        {
                            return Err(WorkbenchError::SessionContextMismatch);
                        }
                        (
                            Some(project),
                            Some(checkout),
                            session.context.clone(),
                            Some(target),
                            Some(WorkloadKind::Coding),
                        )
                    }
                    SessionContextBinding::Workspace { .. } => {
                        if request.project_id.is_some() || request.execution_target.is_some() {
                            return Err(WorkbenchError::SessionContextMismatch);
                        }
                        (None, None, session.context.clone(), None, None)
                    }
                };
                existing_session = Some(session);
                resolved
            } else if let Some(project_id) = &request.project_id {
                let target = request
                    .execution_target
                    .as_ref()
                    .ok_or(WorkbenchError::ProjectTargetMismatch)?;
                if target.project_id() != project_id {
                    return Err(WorkbenchError::ProjectTargetMismatch);
                }
                let project = self.project(project_id).await?;
                let checkout = self.prepare_checkout(target, cancellation).await?;
                (
                    Some(project.clone()),
                    Some(checkout.clone()),
                    SessionContextBinding::Project {
                        project_id: project.id,
                        checkout_id: checkout.id,
                    },
                    Some(target.clone()),
                    Some(WorkloadKind::Coding),
                )
            } else {
                if request.execution_target.is_some() {
                    return Err(WorkbenchError::ProjectTargetMismatch);
                }
                (
                    None,
                    None,
                    SessionContextBinding::Workspace {
                        workspace_id: WorkspaceId::random(),
                    },
                    None,
                    None,
                )
            };
        let now = now_ms();
        let requested_capabilities = requested_provider_capabilities(&model_snapshot);
        let create_request = AgentRunCreateRequest {
            principal: principal.to_owned(),
            idempotency_key: idempotency_key.to_owned(),
            context,
            origin: RunOrigin::Manual,
            title: existing_session.as_ref().map_or_else(
                || prompt.chars().take(80).collect(),
                |session| session.title.clone(),
            ),
            prompt: prompt.to_owned(),
            attachment_ids: request.attachment_ids.clone(),
            parent_session_id: None,
            source_run_id: None,
            purpose: RunPurpose::Task,
            model_snapshot,
            entry_profile: request.entry_profile,
            workload_override,
            behavior_mode: request.behavior_mode,
            execution_target,
            approval_policy: hachimi_protocol::ApprovalPolicy::OnlyWhenNeeded,
            permission_profile: request.permission_profile,
            budget: RunBudget::default(),
            requested_capabilities,
            created_at_ms: now,
        };
        let selected_level = create_request.permission_profile;
        let mut policy = if let Some(session) = existing_session.as_ref() {
            self.store
                .permission_policy(&format!("session:{}", session.id))
                .await?
                .unwrap_or_default()
        } else {
            AgentPermissionPolicy {
                level: selected_level,
                rules: ScopedPermissionRules::default(),
                revision: 0,
            }
        };
        if policy.level != selected_level {
            policy.level = selected_level;
            policy.rules = ScopedPermissionRules::default();
            policy.revision = policy.revision.saturating_add(1);
        }
        let launcher = AgentRunLauncher::new(self.store.clone());
        let launched = if let Some(session) = existing_session {
            if let Some((plan_id, expected_revision)) = revised_plan {
                launcher
                    .launch_plan_revision_in_session(
                        AgentRunLaunchRequest {
                            create: create_request,
                            policy,
                            authority_mode: AuthorityMode::Interactive,
                        },
                        session,
                        plan_id,
                        expected_revision,
                    )
                    .await?
            } else {
                launcher
                    .launch_in_session(
                        AgentRunLaunchRequest {
                            create: create_request,
                            policy,
                            authority_mode: AuthorityMode::Interactive,
                        },
                        session,
                    )
                    .await?
            }
        } else {
            if revised_plan.is_some() {
                return Err(WorkbenchError::SessionContextMismatch);
            }
            launcher
                .launch_new(AgentRunLaunchRequest {
                    create: create_request,
                    policy,
                    authority_mode: AuthorityMode::Interactive,
                })
                .await?
        };
        let created = launched.created;
        if let Some(checkout) = checkout.as_ref() {
            self.store
                .ensure_session_environment_state(
                    &created.session.id,
                    &checkout.id,
                    checkout.kind,
                    checkout.head_revision.as_deref(),
                )
                .await?;
        }
        for attachment_id in &request.attachment_ids {
            if let Some(attachment) = self.store.get_attachment(attachment_id).await? {
                self.store
                    .upsert_session_upload_source(
                        &created.session.id,
                        Some(&created.run.id),
                        attachment_id,
                        &attachment.original_name,
                    )
                    .await?;
            }
        }
        Ok(WorkbenchTaskSnapshot {
            project,
            checkout,
            session: created.session,
            run: created.run,
        })
    }

    async fn project(&self, project_id: &ProjectId) -> Result<ProjectRecord, WorkbenchError> {
        self.store
            .get_project(project_id)
            .await?
            .ok_or_else(|| WorkbenchError::ProjectNotFound(project_id.clone()))
    }
}

fn stage_attachment(
    source: &Path,
    attachment_root: &Path,
) -> Result<(AttachmentRecord, PathBuf), WorkbenchError> {
    let canonical = std::fs::canonicalize(source)?;
    let metadata = std::fs::metadata(&canonical)?;
    if !metadata.is_file() {
        return Err(WorkbenchError::InvalidAttachmentFile);
    }
    if metadata.len() > MAX_ATTACHMENT_BYTES {
        return Err(WorkbenchError::AttachmentTooLarge);
    }
    let original_name = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty() && name.chars().count() <= 255)
        .ok_or(WorkbenchError::InvalidAttachmentFile)?
        .to_owned();
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or_default());
    File::open(&canonical)?
        .take(MAX_ATTACHMENT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_ATTACHMENT_BYTES {
        return Err(WorkbenchError::AttachmentTooLarge);
    }
    let content_hash = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    std::fs::create_dir_all(attachment_root)?;
    let managed_path = attachment_root.join(&content_hash);
    if !managed_path.is_file() {
        let temporary_path = attachment_root.join(format!(
            ".{content_hash}.{}.tmp",
            AttachmentId::random().as_str()
        ));
        let mut temporary = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)?;
        temporary.write_all(&bytes)?;
        temporary.sync_all()?;
        drop(temporary);
        if let Err(error) = std::fs::rename(&temporary_path, &managed_path) {
            if managed_path.is_file() {
                let _ = std::fs::remove_file(&temporary_path);
            } else {
                return Err(WorkbenchError::Io(error));
            }
        }
    }
    let attachment = AttachmentRecord {
        id: AttachmentId::random(),
        content_hash,
        original_name: original_name.clone(),
        mime_type: attachment_mime_type(&original_name).into(),
        byte_size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        created_at_ms: now_ms(),
    };
    Ok((attachment, managed_path))
}

fn attachment_mime_type(name: &str) -> &'static str {
    let extension = Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "txt" | "log" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "json" => "application/json",
        "csv" => "text/csv",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        "yaml" | "yml" => "application/yaml",
        _ => "application/octet-stream",
    }
}

fn sha256_text(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn deterministic_commit_message(status: &str) -> String {
    let files = status.split('\0').filter(|entry| !entry.is_empty()).count();
    if files == 1 {
        "Update 1 file".into()
    } else {
        format!("Update {files} files")
    }
}

fn git_phase_ok(message: impl Into<String>) -> WorkbenchGitPhaseResult {
    let message = message.into();
    WorkbenchGitPhaseResult {
        status: WorkbenchGitPhaseStatus::Succeeded,
        message: (!message.is_empty()).then_some(message),
    }
}

fn git_phase_failed(message: impl Into<String>) -> WorkbenchGitPhaseResult {
    WorkbenchGitPhaseResult {
        status: WorkbenchGitPhaseStatus::Failed,
        message: Some(message.into()),
    }
}

async fn git_optional(root: &Path, args: &[&str]) -> Result<Option<String>, WorkbenchError> {
    let output = tokio_command("git", ProcessPolicy::HiddenCaptured)
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await
        .map_err(|error| WorkbenchError::Git(error.to_string()))?;
    Ok(output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned()))
}

async fn git_required(
    root: &Path,
    args: &[&str],
    cancellation: Option<&CancellationToken>,
) -> Result<String, WorkbenchError> {
    let mut command = tokio_command("git", ProcessPolicy::HiddenCaptured);
    command
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = if let Some(cancellation) = cancellation {
        tokio::select! {
            () = cancellation.cancelled() => return Err(WorkbenchError::Cancelled),
            output = command.output() => output,
        }
    } else {
        command.output().await
    }
    .map_err(|error| WorkbenchError::Git(error.to_string()))?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr)
            .trim()
            .chars()
            .take(1_024)
            .collect::<String>();
        return Err(WorkbenchError::Git(message));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn now_ms() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}

fn is_text_mime(mime: &str) -> bool {
    mime.starts_with("text/")
        || matches!(
            mime,
            "application/json"
                | "application/javascript"
                | "application/xml"
                | "application/yaml"
                | "application/toml"
        )
}

fn requested_provider_capabilities(settings: &LlmSettings) -> ProviderCapabilities {
    let structured =
        settings.structured_output_mode != hachimi_protocol::StructuredOutputMode::Disabled;
    ProviderCapabilities {
        tool_calls: true,
        parallel_tool_calls: true,
        strict_json_schema: structured,
        output_schema: structured,
        text_input: true,
        streaming_usage: true,
        http_transport: true,
        context_window: (settings.max_input_tokens > 0)
            .then_some(u64::from(settings.max_input_tokens)),
        max_output_tokens: (settings.max_output_tokens > 0)
            .then_some(u64::from(settings.max_output_tokens)),
        ..ProviderCapabilities::default()
    }
}

#[cfg(test)]
mod tests {
    use hachimi_protocol::{
        ClientId, ItemId, ItemRelations, ItemStatus, MutationContext, RequestId, TranscriptItem,
        TranscriptItemKind,
    };

    use std::process::Command as StdCommand;

    use hachimi_protocol::{
        BehaviorMode, EntryProfile, PermissionProfile, PlanAcceptanceRequest,
        PlanConfirmationStatus, PlanDocument, PlanId, RunRecord, RunStatus,
        WorkbenchTaskStartRequest,
    };

    use super::*;

    fn git(root: &Path, args: &[&str]) {
        let status = StdCommand::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .expect("git");
        assert!(status.success());
    }

    fn mutation_context(run: &RunRecord, idempotency_key: &str) -> MutationContext {
        MutationContext {
            request_id: RequestId(format!("request-{idempotency_key}")),
            client_id: ClientId("test-user".into()),
            protocol_version: 1,
            idempotency_key: idempotency_key.into(),
            expected_run_id: Some(run.id.clone()),
            expected_generation: Some(run.generation),
        }
    }

    #[tokio::test]
    async fn creates_real_project_refs_and_local_task() {
        let directory = tempfile::tempdir().expect("tempdir");
        git(directory.path(), &["init", "-b", "main"]);
        git(
            directory.path(),
            &["config", "user.email", "test@example.com"],
        );
        git(directory.path(), &["config", "user.name", "Hachimi Test"]);
        std::fs::write(directory.path().join("README.md"), "demo").expect("file");
        git(directory.path(), &["add", "README.md"]);
        git(directory.path(), &["commit", "-m", "initial"]);

        let store = AgentStore::connect_in_memory().await.expect("store");
        let worktrees = tempfile::tempdir().expect("worktrees");
        let attachments = tempfile::tempdir().expect("attachments");
        let service = WorkbenchService::new(store, worktrees.path(), attachments.path());
        let project = service
            .add_project(directory.path())
            .await
            .expect("project");
        let attachment = service
            .import_attachment(directory.path().join("README.md"))
            .await
            .expect("attachment");
        assert_eq!(attachment.mime_type, "text/markdown");
        assert!(attachments.path().join(&attachment.content_hash).is_file());
        let refs = service.git_refs(&project.id).await.expect("refs");
        assert!(
            refs.iter()
                .any(|reference| reference.name == "main" && reference.current)
        );
        let target = ExecutionTarget::Local {
            project_id: project.id.clone(),
        };
        let snapshot = service
            .create_task(
                &WorkbenchTaskStartRequest {
                    idempotency_key: "request-1".into(),
                    entry_profile: EntryProfile::Workbench,
                    session_id: None,
                    project_id: Some(project.id),
                    prompt: "Inspect the project".into(),
                    execution_target: Some(target),
                    behavior_mode: BehaviorMode::Plan,
                    permission_profile: PermissionProfile::ReadOnly,
                    attachment_ids: vec![attachment.id.clone()],
                    skill_ids: Vec::new(),
                },
                LlmSettings::default(),
                "test-user",
                "request-1",
                &CancellationToken::new(),
            )
            .await
            .expect("task");
        assert_eq!(
            snapshot.checkout.as_ref().expect("checkout").kind,
            CheckoutKind::Local
        );
        assert_eq!(
            snapshot.run.configuration.permission_profile,
            PermissionProfile::ReadOnly
        );
        assert_eq!(
            service
                .store()
                .get_attachment(&attachment.id)
                .await
                .expect("stored attachment"),
            Some(attachment)
        );

        service
            .store()
            .transition_run(&snapshot.run.id, RunStatus::Preparing, None)
            .await
            .expect("preparing");
        service
            .store()
            .transition_run(&snapshot.run.id, RunStatus::Running, None)
            .await
            .expect("running");
        service
            .store()
            .transition_run(&snapshot.run.id, RunStatus::Succeeded, None)
            .await
            .expect("succeeded");
        let source_item_id = ItemId::random();
        service
            .store()
            .append_transcript_item(TranscriptItem {
                id: source_item_id.clone(),
                session_id: snapshot.session.id.clone(),
                run_id: Some(snapshot.run.id.clone()),
                sequence: 0,
                kind: TranscriptItemKind::Plan,
                status: ItemStatus::Completed,
                payload: ItemPayload::Plan {
                    text: "# Inspect the project".into(),
                },
                relations: ItemRelations::default(),
                created_at_ms: now_ms(),
            })
            .await
            .expect("plan item");
        let (plan, _) = service
            .store()
            .create_plan_document(PlanDocument {
                id: PlanId::from("plan-1"),
                session_id: snapshot.session.id.clone(),
                source_run_id: snapshot.run.id.clone(),
                source_item_id,
                revision: 0,
                title: "Inspect the project".into(),
                goal: "Inspect the project".into(),
                content_markdown: "# Inspect the project\n1. Update README\n2. Review diff".into(),
                created_at_ms: now_ms(),
            })
            .await
            .expect("plan");
        let acceptance_request = PlanAcceptanceRequest {
            idempotency_key: "accept-1".into(),
            plan_id: plan.id,
            expected_revision: plan.revision,
            user_message: "Yes, implement this plan".into(),
        };
        let accepted = service
            .accept_plan(&acceptance_request, LlmSettings::default(), "test-user")
            .await
            .expect("accept");
        assert_eq!(
            accepted.confirmation.status,
            PlanConfirmationStatus::Accepted
        );
        assert_eq!(
            accepted.task.run.configuration.behavior_mode,
            BehaviorMode::Default
        );
        assert_eq!(
            accepted.task.run.configuration.accepted_plan_id,
            Some(accepted.plan.id.clone())
        );
        assert!(
            service
                .attachment_model_context(&accepted.task.run.id)
                .await
                .expect("accepted attachment context")
                .is_some()
        );
        let duplicate = service
            .accept_plan(&acceptance_request, LlmSettings::default(), "test-user")
            .await
            .expect("idempotent accept");
        assert_eq!(duplicate.task.run.id, accepted.task.run.id);
        let confirmation_messages = service
            .store()
            .list_transcript(&accepted.task.session.id)
            .await
            .expect("accepted plan transcript")
            .into_iter()
            .filter(|item| item.run_id.as_ref() == Some(&accepted.task.run.id))
            .filter_map(|item| match item.payload {
                ItemPayload::User { text, .. } => Some(text),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(confirmation_messages, ["Yes, implement this plan"]);
    }

    #[tokio::test]
    async fn high_level_git_rejects_stale_state_and_replays_local_commit() {
        let directory = tempfile::tempdir().expect("tempdir");
        git(directory.path(), &["init", "-b", "main"]);
        git(
            directory.path(),
            &["config", "user.email", "test@example.com"],
        );
        git(directory.path(), &["config", "user.name", "Hachimi Test"]);
        std::fs::write(directory.path().join("README.md"), "initial\n").expect("file");
        git(directory.path(), &["add", "README.md"]);
        git(directory.path(), &["commit", "-m", "initial"]);

        let store = AgentStore::connect_in_memory().await.expect("store");
        let worktrees = tempfile::tempdir().expect("worktrees");
        let attachments = tempfile::tempdir().expect("attachments");
        let service = WorkbenchService::new(store, worktrees.path(), attachments.path());
        let project = service
            .add_project(directory.path())
            .await
            .expect("project");
        let task = service
            .create_task(
                &WorkbenchTaskStartRequest {
                    idempotency_key: "git-task".into(),
                    entry_profile: EntryProfile::Workbench,
                    session_id: None,
                    project_id: Some(project.id.clone()),
                    prompt: "Prepare Git changes".into(),
                    execution_target: Some(ExecutionTarget::Local {
                        project_id: project.id,
                    }),
                    behavior_mode: BehaviorMode::Default,
                    permission_profile: PermissionProfile::Writable,
                    attachment_ids: Vec::new(),
                    skill_ids: Vec::new(),
                },
                LlmSettings::default(),
                "test-user",
                "git-task",
                &CancellationToken::new(),
            )
            .await
            .expect("task");
        let checkout = task.checkout.expect("checkout");
        std::fs::write(directory.path().join("README.md"), "changed\n").expect("change");
        let head = git_optional(directory.path(), &["rev-parse", "HEAD"])
            .await
            .expect("head");
        let status = git_required(directory.path(), &["status", "--porcelain=v1", "-z"], None)
            .await
            .expect("status");

        let stale = service
            .execute_git(
                &WorkbenchGitRequest {
                    context: mutation_context(&task.run, "stale-git-action"),
                    idempotency_key: "stale-git-action".into(),
                    session_id: task.session.id.clone(),
                    checkout_id: checkout.id.clone(),
                    expected_head: head.clone(),
                    status_fingerprint: "stale".into(),
                    include_unstaged: true,
                    action: WorkbenchGitAction::Commit {
                        message: Some("should not commit".into()),
                    },
                },
                "test-user",
                None,
            )
            .await;
        assert!(matches!(stale, Err(WorkbenchError::GitStatusChanged)));

        let action = WorkbenchGitRequest {
            context: mutation_context(&task.run, "commit-action"),
            idempotency_key: "commit-action".into(),
            session_id: task.session.id,
            checkout_id: checkout.id,
            expected_head: head,
            status_fingerprint: sha256_text(&status),
            include_unstaged: true,
            action: WorkbenchGitAction::Commit {
                message: Some("update readme".into()),
            },
        };
        let response = service
            .execute_git(&action, "test-user", None)
            .await
            .expect("commit result");
        assert_eq!(response.stage.status, WorkbenchGitPhaseStatus::Succeeded);
        assert_eq!(response.commit.status, WorkbenchGitPhaseStatus::Succeeded);
        let replay = service
            .execute_git(&action, "test-user", None)
            .await
            .expect("idempotent replay");
        assert_eq!(replay, response);
    }

    #[tokio::test]
    async fn high_level_git_respects_staging_head_dirty_and_remote_tracking_fences() {
        let directory = tempfile::tempdir().expect("tempdir");
        git(directory.path(), &["init", "-b", "main"]);
        git(
            directory.path(),
            &["config", "user.email", "test@example.com"],
        );
        git(directory.path(), &["config", "user.name", "Hachimi Test"]);
        std::fs::write(directory.path().join("README.md"), "initial\n").expect("file");
        git(directory.path(), &["add", "README.md"]);
        git(directory.path(), &["commit", "-m", "initial"]);

        let store = AgentStore::connect_in_memory().await.expect("store");
        let worktrees = tempfile::tempdir().expect("worktrees");
        let attachments = tempfile::tempdir().expect("attachments");
        let service = WorkbenchService::new(store, worktrees.path(), attachments.path());
        let project = service
            .add_project(directory.path())
            .await
            .expect("project");
        let task = service
            .create_task(
                &WorkbenchTaskStartRequest {
                    idempotency_key: "git-boundary-task".into(),
                    entry_profile: EntryProfile::Workbench,
                    session_id: None,
                    project_id: Some(project.id.clone()),
                    prompt: "Exercise Git boundaries".into(),
                    execution_target: Some(ExecutionTarget::Local {
                        project_id: project.id,
                    }),
                    behavior_mode: BehaviorMode::Default,
                    permission_profile: PermissionProfile::Writable,
                    attachment_ids: Vec::new(),
                    skill_ids: Vec::new(),
                },
                LlmSettings::default(),
                "test-user",
                "git-boundary-task",
                &CancellationToken::new(),
            )
            .await
            .expect("task");
        let checkout = task.checkout.expect("checkout");

        std::fs::write(directory.path().join("staged.txt"), "staged\n").expect("staged");
        std::fs::write(directory.path().join("unstaged.txt"), "unstaged\n").expect("unstaged");
        git(directory.path(), &["add", "staged.txt"]);
        let head = git_optional(directory.path(), &["rev-parse", "HEAD"])
            .await
            .expect("head");
        let status = git_required(directory.path(), &["status", "--porcelain=v1", "-z"], None)
            .await
            .expect("status");
        let staged_only = service
            .execute_git(
                &WorkbenchGitRequest {
                    context: mutation_context(&task.run, "staged-only"),
                    idempotency_key: "staged-only".into(),
                    session_id: task.session.id.clone(),
                    checkout_id: checkout.id.clone(),
                    expected_head: head,
                    status_fingerprint: sha256_text(&status),
                    include_unstaged: false,
                    action: WorkbenchGitAction::Commit {
                        message: Some("commit staged only".into()),
                    },
                },
                "test-user",
                None,
            )
            .await
            .expect("staged-only commit");
        assert_eq!(staged_only.stage.status, WorkbenchGitPhaseStatus::Skipped);
        assert_eq!(
            staged_only.commit.status,
            WorkbenchGitPhaseStatus::Succeeded
        );
        let committed = git_required(
            directory.path(),
            &["show", "--name-only", "--format="],
            None,
        )
        .await
        .expect("committed files");
        assert!(committed.contains("staged.txt"));
        assert!(!committed.contains("unstaged.txt"));
        assert!(directory.path().join("unstaged.txt").exists());

        let status = git_required(directory.path(), &["status", "--porcelain=v1", "-z"], None)
            .await
            .expect("status");
        let include_unstaged = service
            .execute_git(
                &WorkbenchGitRequest {
                    context: mutation_context(&task.run, "include-unstaged"),
                    idempotency_key: "include-unstaged".into(),
                    session_id: task.session.id.clone(),
                    checkout_id: checkout.id.clone(),
                    expected_head: staged_only.head,
                    status_fingerprint: sha256_text(&status),
                    include_unstaged: true,
                    action: WorkbenchGitAction::Commit {
                        message: Some("commit unstaged".into()),
                    },
                },
                "test-user",
                None,
            )
            .await
            .expect("include unstaged commit");
        assert_eq!(
            include_unstaged.stage.status,
            WorkbenchGitPhaseStatus::Succeeded
        );
        assert_eq!(
            include_unstaged.commit.status,
            WorkbenchGitPhaseStatus::Succeeded
        );

        std::fs::write(directory.path().join("dirty.txt"), "dirty\n").expect("dirty");
        let current_head = git_optional(directory.path(), &["rev-parse", "HEAD"])
            .await
            .expect("head");
        let dirty_status =
            git_required(directory.path(), &["status", "--porcelain=v1", "-z"], None)
                .await
                .expect("dirty status");
        let dirty_checkout = service
            .execute_git(
                &WorkbenchGitRequest {
                    context: mutation_context(&task.run, "dirty-checkout"),
                    idempotency_key: "dirty-checkout".into(),
                    session_id: task.session.id.clone(),
                    checkout_id: checkout.id.clone(),
                    expected_head: current_head,
                    status_fingerprint: sha256_text(&dirty_status),
                    include_unstaged: true,
                    action: WorkbenchGitAction::SwitchBranch {
                        branch: "main".into(),
                        remote: false,
                    },
                },
                "test-user",
                None,
            )
            .await;
        assert!(matches!(
            dirty_checkout,
            Err(WorkbenchError::GitCheckoutDirty)
        ));
        std::fs::remove_file(directory.path().join("dirty.txt")).expect("remove dirty");

        let remote = tempfile::tempdir().expect("remote");
        git(remote.path(), &["init", "--bare"]);
        let remote_path = remote.path().to_string_lossy();
        git(directory.path(), &["remote", "add", "origin", &remote_path]);
        git(directory.path(), &["branch", "feature"]);
        git(directory.path(), &["push", "origin", "feature"]);
        git(directory.path(), &["branch", "-D", "feature"]);
        let current_head = git_optional(directory.path(), &["rev-parse", "HEAD"])
            .await
            .expect("head");
        let status = git_required(directory.path(), &["status", "--porcelain=v1", "-z"], None)
            .await
            .expect("status");
        let tracking = service
            .execute_git(
                &WorkbenchGitRequest {
                    context: mutation_context(&task.run, "remote-tracking"),
                    idempotency_key: "remote-tracking".into(),
                    session_id: task.session.id,
                    checkout_id: checkout.id,
                    expected_head: current_head,
                    status_fingerprint: sha256_text(&status),
                    include_unstaged: true,
                    action: WorkbenchGitAction::SwitchBranch {
                        branch: "origin/feature".into(),
                        remote: true,
                    },
                },
                "test-user",
                None,
            )
            .await
            .expect("tracking checkout");
        assert_eq!(tracking.stage.status, WorkbenchGitPhaseStatus::Succeeded);
        assert_eq!(
            git_required(directory.path(), &["branch", "--show-current"], None)
                .await
                .expect("current branch")
                .trim(),
            "feature"
        );
    }

    #[tokio::test]
    async fn general_and_project_sessions_continue_in_place_without_parallel_runs() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let worktrees = tempfile::tempdir().expect("worktrees");
        let attachments = tempfile::tempdir().expect("attachments");
        let service = WorkbenchService::new(store, worktrees.path(), attachments.path());
        let first = service
            .create_task(
                &WorkbenchTaskStartRequest {
                    idempotency_key: "general-1".into(),
                    entry_profile: EntryProfile::Workbench,
                    session_id: None,
                    project_id: None,
                    prompt: "Start a general conversation".into(),
                    execution_target: None,
                    behavior_mode: BehaviorMode::Default,
                    permission_profile: PermissionProfile::Writable,
                    attachment_ids: Vec::new(),
                    skill_ids: Vec::new(),
                },
                LlmSettings::default(),
                "test-user",
                "general-1",
                &CancellationToken::new(),
            )
            .await
            .expect("general task");
        assert!(first.project.is_none());
        assert!(first.checkout.is_none());
        assert!(matches!(
            first.session.context,
            SessionContextBinding::Workspace { .. }
        ));

        let continuation = WorkbenchTaskStartRequest {
            idempotency_key: "general-2".into(),
            entry_profile: EntryProfile::Workbench,
            session_id: Some(first.session.id.clone()),
            project_id: None,
            prompt: "Continue in the same session".into(),
            execution_target: None,
            behavior_mode: BehaviorMode::Default,
            permission_profile: PermissionProfile::Writable,
            attachment_ids: Vec::new(),
            skill_ids: Vec::new(),
        };
        assert!(matches!(
            service
                .create_task(
                    &continuation,
                    LlmSettings::default(),
                    "test-user",
                    "general-2",
                    &CancellationToken::new(),
                )
                .await,
            Err(WorkbenchError::Agent(AgentRunFactoryError::Store(
                AgentStoreError::RunPreconditionFailed
            )))
        ));
        service
            .store()
            .transition_run(&first.run.id, RunStatus::Preparing, None)
            .await
            .expect("preparing");
        service
            .store()
            .transition_run(&first.run.id, RunStatus::Running, None)
            .await
            .expect("running");
        service
            .store()
            .transition_run(&first.run.id, RunStatus::Succeeded, None)
            .await
            .expect("succeeded");
        let next = service
            .create_task(
                &continuation,
                LlmSettings::default(),
                "test-user",
                "general-2",
                &CancellationToken::new(),
            )
            .await
            .expect("continuation");
        assert_eq!(next.session.id, first.session.id);
        assert_ne!(next.run.id, first.run.id);
        assert_eq!(next.run.session_id, first.session.id);
    }

    #[tokio::test]
    async fn managed_worktree_cleanup_preserves_dirty_or_pinned_checkouts() {
        let directory = tempfile::tempdir().expect("repo");
        git(directory.path(), &["init", "-b", "main"]);
        git(
            directory.path(),
            &["config", "user.email", "test@example.com"],
        );
        git(directory.path(), &["config", "user.name", "Hachimi Test"]);
        std::fs::write(directory.path().join("README.md"), "demo").expect("file");
        git(directory.path(), &["add", "README.md"]);
        git(directory.path(), &["commit", "-m", "initial"]);

        let store = AgentStore::connect_in_memory().await.expect("store");
        let worktrees = tempfile::tempdir().expect("worktrees");
        let attachments = tempfile::tempdir().expect("attachments");
        let service = WorkbenchService::new(store, worktrees.path(), attachments.path());
        let project = service
            .add_project(directory.path())
            .await
            .expect("project");
        let checkout = service
            .prepare_checkout(
                &ExecutionTarget::ManagedWorktree {
                    project_id: project.id,
                    base_revision: "main".into(),
                },
                &CancellationToken::new(),
            )
            .await
            .expect("worktree");
        let dirty_file = Path::new(&checkout.path).join("scratch.txt");
        std::fs::write(&dirty_file, "dirty").expect("dirty file");
        let dirty_result = service.cleanup_checkout(&checkout.id).await;
        assert!(
            matches!(dirty_result, Err(WorkbenchError::CheckoutDirty)),
            "unexpected cleanup result: {dirty_result:?}"
        );
        assert!(Path::new(&checkout.path).is_dir());
        std::fs::remove_file(&dirty_file).expect("remove test file");
        service.pin_checkout(&checkout.id, true).await.expect("pin");
        assert!(matches!(
            service.cleanup_checkout(&checkout.id).await,
            Err(WorkbenchError::CheckoutPinned)
        ));
        service
            .pin_checkout(&checkout.id, false)
            .await
            .expect("unpin");
        let removed = service
            .cleanup_checkout(&checkout.id)
            .await
            .expect("cleanup");
        assert_eq!(removed.status, CheckoutStatus::Removed);
        assert!(!Path::new(&checkout.path).exists());
    }
}
