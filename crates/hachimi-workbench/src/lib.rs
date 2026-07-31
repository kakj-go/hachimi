//! Project, Checkout and task-draft services for the coding Workbench.

mod attachment_host;

pub use attachment_host::AttachmentModelContext;

use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::{SystemTime, UNIX_EPOCH},
};

use hachimi_agent::{AgentRunCreateRequest, AgentRunFactory, AgentRunFactoryError};
use hachimi_protocol::{
    AttachmentId, AttachmentRecord, CheckoutId, CheckoutKind, CheckoutRecord, CheckoutStatus,
    EntryProfile, ExecutionTarget, GitRefRecord, ItemId, ItemPayload, ItemRelations, ItemStatus,
    LlmSettings, PermissionProfile, PlanAcceptanceRequest, ProjectId, ProjectRecord,
    ProviderCapabilities, RunBudget, RunId, RunOrigin, RunPurpose, RunRecord, RunStatus,
    SessionContextBinding, SessionId, SessionRecord, TranscriptItem, TranscriptItemKind,
    WorkbenchPlanAcceptanceSnapshot, WorkbenchSessionSnapshot, WorkbenchTaskSnapshot,
    WorkbenchTaskStartRequest, WorkloadKind,
};
use hachimi_storage::{AgentStore, AgentStoreError};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

const MAX_ATTACHMENT_BYTES: u64 = 25 * 1024 * 1024;

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
    #[error("coding Workbench requires a Project-bound Session")]
    ProjectContextRequired,
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
    #[error("workbench operation was cancelled")]
    Cancelled,
    #[error("task prompt must contain 1-32000 characters")]
    InvalidPrompt,
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
    ) -> Result<Vec<SessionRecord>, WorkbenchError> {
        Ok(self.store.list_sessions(project_id).await?)
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
        let runs = self.store.list_runs(session_id).await?;
        let events = self.store.list_events(session_id, 0).await?;
        let transcript = self.store.list_transcript(session_id).await?;
        let pending_approvals = self
            .store
            .list_pending_approvals()
            .await?
            .into_iter()
            .filter(|approval| approval.session_id == *session_id)
            .collect();
        let proposed_plans = self.store.list_proposed_plans(session_id).await?;
        let artifacts = self.store.list_session_artifacts(session_id).await?;
        let agent_tasks = self.store.list_agent_tasks_for_session(session_id).await?;
        Ok(WorkbenchSessionSnapshot {
            session,
            runs,
            events,
            transcript,
            pending_approvals,
            proposed_plans,
            artifacts,
            agent_tasks,
        })
    }

    pub async fn accept_plan(
        &self,
        request: &PlanAcceptanceRequest,
        model_snapshot: LlmSettings,
        principal: &str,
    ) -> Result<WorkbenchPlanAcceptanceSnapshot, WorkbenchError> {
        let plan = self
            .store
            .get_proposed_plan(&request.plan_id)
            .await?
            .ok_or_else(|| AgentStoreError::ProposedPlanNotFound(request.plan_id.clone()))?;
        let source_run = self
            .store
            .get_run(&plan.run_id)
            .await?
            .ok_or_else(|| AgentStoreError::RunNotFound(plan.run_id.clone()))?;
        if source_run.status != RunStatus::Succeeded
            || source_run.configuration.behavior_mode != hachimi_protocol::BehaviorMode::Plan
        {
            return Err(WorkbenchError::Store(
                AgentStoreError::ProposedPlanNotAcceptable(plan.id),
            ));
        }
        let session = self
            .store
            .get_session(&plan.session_id)
            .await?
            .ok_or_else(|| WorkbenchError::SessionNotFound(plan.session_id.clone()))?;
        let checkout_id = session
            .context
            .checkout_id()
            .ok_or(WorkbenchError::ProjectContextRequired)?;
        let project_id = session
            .context
            .project_id()
            .ok_or(WorkbenchError::ProjectContextRequired)?;
        let checkout = self
            .store
            .get_checkout(checkout_id)
            .await?
            .ok_or_else(|| WorkbenchError::CheckoutNotFound(checkout_id.clone()))?;
        let project = self.project(project_id).await?;
        let now = now_ms();
        let requested_capabilities = source_run.requested_capabilities;
        let mut configuration = source_run.configuration;
        configuration.model_snapshot = model_snapshot;
        configuration.behavior_mode = hachimi_protocol::BehaviorMode::Default;
        configuration.permission_profile = PermissionProfile::WorkspaceWrite;
        configuration.accepted_plan_id = Some(plan.id.clone());
        configuration.accepted_plan_revision = Some(plan.revision);
        let candidate = RunRecord {
            id: RunId::random(),
            session_id: session.id.clone(),
            status: RunStatus::Queued,
            purpose: RunPurpose::Task,
            origin: RunOrigin::Interactive,
            generation: 1,
            configuration,
            requested_capabilities,
            negotiated_capabilities: ProviderCapabilities::default(),
            provider_capability_probe: None,
            capability_degradations: Vec::new(),
            failure_code: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let (accepted_plan, run) = self
            .store
            .accept_proposed_plan_idempotent(
                principal,
                &request.idempotency_key,
                &plan.id,
                &candidate,
            )
            .await?;
        if run.id == candidate.id {
            self.store
                .append_transcript_item(TranscriptItem {
                    id: ItemId::random(),
                    session_id: session.id.clone(),
                    run_id: Some(run.id.clone()),
                    sequence: 0,
                    kind: TranscriptItemKind::User,
                    status: ItemStatus::Completed,
                    payload: ItemPayload::User {
                        text: format!(
                            "Accepted proposed plan revision {} for execution.",
                            accepted_plan.revision
                        ),
                        attachment_ids: Vec::new(),
                    },
                    relations: ItemRelations::default(),
                    created_at_ms: now,
                })
                .await?;
        }
        Ok(WorkbenchPlanAcceptanceSnapshot {
            plan: accepted_plan,
            task: WorkbenchTaskSnapshot {
                project: Some(project),
                checkout: Some(checkout),
                session,
                run,
            },
        })
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
        let prompt = request.prompt.trim();
        if prompt.is_empty() || prompt.chars().count() > 32_000 {
            return Err(WorkbenchError::InvalidPrompt);
        }
        if request.entry_profile == EntryProfile::DesktopControl
            && (request.project_id.is_some() || request.execution_target.is_some())
        {
            return Err(WorkbenchError::SessionContextMismatch);
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
                    SessionContextBinding::General => {
                        if request.project_id.is_some() || request.execution_target.is_some() {
                            return Err(WorkbenchError::SessionContextMismatch);
                        }
                        (
                            None,
                            None,
                            SessionContextBinding::General,
                            None,
                            Some(WorkloadKind::General),
                        )
                    }
                    SessionContextBinding::Avatar { .. } => {
                        return Err(WorkbenchError::SessionContextMismatch);
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
                    SessionContextBinding::General,
                    None,
                    Some(WorkloadKind::General),
                )
            };
        let now = now_ms();
        let requested_capabilities = requested_provider_capabilities(&model_snapshot);
        let create_request = AgentRunCreateRequest {
            principal: principal.to_owned(),
            idempotency_key: idempotency_key.to_owned(),
            context,
            origin: RunOrigin::Interactive,
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
            approval_policy: request.approval_policy,
            permission_profile: if request.entry_profile == EntryProfile::DesktopControl {
                PermissionProfile::ExternalSandbox
            } else if project.is_none()
                || request.behavior_mode == hachimi_protocol::BehaviorMode::Plan
            {
                PermissionProfile::ReadOnly
            } else {
                PermissionProfile::WorkspaceWrite
            },
            budget: RunBudget::default(),
            requested_capabilities,
            created_at_ms: now,
        };
        let factory = AgentRunFactory::new(self.store.clone());
        let created = if let Some(session) = existing_session {
            factory.create_in_session(create_request, session).await?
        } else {
            factory.create(create_request).await?
        };
        if request.entry_profile == EntryProfile::DesktopControl {
            self.store
                .upsert_desktop_control_session(&created.session.id, now)
                .await?;
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

async fn git_optional(root: &Path, args: &[&str]) -> Result<Option<String>, WorkbenchError> {
    let output = Command::new("git")
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
    let mut command = Command::new("git");
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
    use hachimi_protocol::{PlanStep, PlanStepId, PlanStepStatus};

    use std::process::Command as StdCommand;

    use hachimi_protocol::{
        ApprovalPolicy, BehaviorMode, PlanAcceptanceRequest, PlanId, ProposedPlan,
        ProposedPlanStatus, WorkbenchTaskStartRequest,
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
                    approval_policy: ApprovalPolicy::NeverPrompt,
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
        let plan = service
            .store()
            .create_proposed_plan(ProposedPlan {
                id: PlanId::from("plan-1"),
                session_id: snapshot.session.id.clone(),
                run_id: snapshot.run.id.clone(),
                revision: 0,
                goal: "Inspect the project".into(),
                assumptions: Vec::new(),
                steps: vec![PlanStep {
                    id: PlanStepId::from("step-1"),
                    description: "Update README".into(),
                    status: PlanStepStatus::Pending,
                }],
                affected_resources: vec!["README.md".into()],
                verification: vec!["Review diff".into()],
                risks: Vec::new(),
                open_questions: Vec::new(),
                content_markdown: "1. Update README\n2. Review diff".into(),
                status: ProposedPlanStatus::Proposed,
                accepted_run_id: None,
                created_at_ms: now_ms(),
                accepted_at_ms: None,
            })
            .await
            .expect("plan");
        let acceptance_request = PlanAcceptanceRequest {
            idempotency_key: "accept-1".into(),
            plan_id: plan.id,
        };
        let accepted = service
            .accept_plan(&acceptance_request, LlmSettings::default(), "test-user")
            .await
            .expect("accept");
        assert_eq!(accepted.plan.status, ProposedPlanStatus::Accepted);
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
                    approval_policy: ApprovalPolicy::OnlyWhenNeeded,
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
        assert_eq!(first.session.context, SessionContextBinding::General);

        let continuation = WorkbenchTaskStartRequest {
            idempotency_key: "general-2".into(),
            entry_profile: EntryProfile::Workbench,
            session_id: Some(first.session.id.clone()),
            project_id: None,
            prompt: "Continue in the same session".into(),
            execution_target: None,
            behavior_mode: BehaviorMode::Default,
            approval_policy: ApprovalPolicy::OnlyWhenNeeded,
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
