use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc};

use futures_util::StreamExt;
use hachimi_protocol::{
    ApprovalPolicy, AttachmentId, BehaviorMode, CapabilityGrantSet, CompactionCheckpoint,
    EntryProfile, ItemId, ItemPayload, ItemRelations, ItemStatus, LlmSettings, McpToolSelection,
    ModelEvent, ModelMessage, ModelRequest, ModelRole, PermissionProfile, ProviderCapabilities,
    RunBudget, RunConfiguration, RunDriverKind, RunId, RunOrigin, RunPurpose, RunRecord, RunStatus,
    SandboxCapabilityReport, SessionContextBinding, SessionId, SessionRecord, SkillId,
    TranscriptItem, TranscriptItemKind, WorkloadKind,
};
use hachimi_storage::{AgentStore, AgentStoreError, CreatedAgentRun};
use parking_lot::Mutex;
use thiserror::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::{
    CompactionError, LaneError, ModelRuntime, ModelRuntimeError, ModelRuntimeFactory,
    RunStepContext, SemanticCompactor, SessionLanes, StepRuntimeState, ToolExecutor, ToolRuntime,
    TurnRuntime,
};

#[derive(Debug, Clone)]
pub struct AgentRunCreateRequest {
    pub principal: String,
    pub idempotency_key: String,
    pub context: SessionContextBinding,
    pub origin: RunOrigin,
    pub title: String,
    pub prompt: String,
    pub attachment_ids: Vec<AttachmentId>,
    pub parent_session_id: Option<SessionId>,
    pub source_run_id: Option<RunId>,
    pub purpose: RunPurpose,
    pub model_snapshot: LlmSettings,
    pub entry_profile: EntryProfile,
    pub workload_override: Option<WorkloadKind>,
    pub behavior_mode: BehaviorMode,
    pub execution_target: Option<hachimi_protocol::ExecutionTarget>,
    pub approval_policy: ApprovalPolicy,
    pub permission_profile: PermissionProfile,
    pub budget: RunBudget,
    pub requested_capabilities: ProviderCapabilities,
    pub created_at_ms: i64,
}

#[derive(Debug, Error)]
pub enum AgentRunFactoryError {
    #[error("agent storage failed: {0}")]
    Store(#[from] AgentStoreError),
    #[error("prompt must contain 1-32000 characters")]
    InvalidPrompt,
    #[error("title must contain 1-200 characters")]
    InvalidTitle,
    #[error("coding Runs require a Project context and matching execution target")]
    CodingProjectRequired,
    #[error("General and Avatar contexts cannot carry a workspace execution target")]
    UnexpectedExecutionTarget,
    #[error("Plan mode must use a read-only permission profile")]
    PlanMustBeReadOnly,
    #[error("entry profile is incompatible with the requested Session context")]
    EntryContextMismatch,
    #[error("existing Session does not match the continuation request")]
    ExistingSessionMismatch,
}

#[derive(Debug, Clone)]
pub struct AgentRunFactory {
    store: AgentStore,
}

impl AgentRunFactory {
    #[must_use]
    pub const fn new(store: AgentStore) -> Self {
        Self { store }
    }

    pub async fn create(
        &self,
        request: AgentRunCreateRequest,
    ) -> Result<CreatedAgentRun, AgentRunFactoryError> {
        validate_create_request(&request)?;
        let session_id = SessionId::random();
        let run_id = RunId::random();
        let session = SessionRecord {
            id: session_id.clone(),
            context: request.context,
            entry_profile: request.entry_profile,
            title: request.title.trim().to_owned(),
            archived: false,
            pinned: false,
            parent_session_id: request.parent_session_id,
            source_run_id: request.source_run_id,
            created_at_ms: request.created_at_ms,
            updated_at_ms: request.created_at_ms,
        };
        let run = RunRecord {
            id: run_id.clone(),
            session_id: session_id.clone(),
            status: RunStatus::Queued,
            purpose: request.purpose,
            origin: request.origin,
            generation: 1,
            configuration: RunConfiguration {
                model_snapshot: request.model_snapshot,
                driver: RunDriverKind::ToolLoop,
                entry_profile: request.entry_profile,
                workload_override: request.workload_override,
                behavior_mode: request.behavior_mode,
                execution_target: request.execution_target,
                approval_policy: request.approval_policy,
                permission_profile: request.permission_profile,
                budget: request.budget,
                accepted_plan_id: None,
                accepted_plan_revision: None,
            },
            requested_capabilities: request.requested_capabilities,
            negotiated_capabilities: ProviderCapabilities::default(),
            provider_capability_probe: None,
            capability_degradations: Vec::new(),
            failure_code: None,
            created_at_ms: request.created_at_ms,
            updated_at_ms: request.created_at_ms,
        };
        let prompt = request.prompt.trim().to_owned();
        let user_item = TranscriptItem {
            id: ItemId::random(),
            session_id: session_id.clone(),
            run_id: Some(run_id),
            sequence: 0,
            kind: TranscriptItemKind::User,
            status: ItemStatus::Completed,
            payload: ItemPayload::User {
                text: prompt.clone(),
                attachment_ids: request.attachment_ids.clone(),
            },
            relations: ItemRelations::default(),
            created_at_ms: request.created_at_ms,
        };
        Ok(self
            .store
            .create_agent_run_bundle_idempotent(
                &request.principal,
                &request.idempotency_key,
                &session,
                &run,
                &user_item,
                &request.attachment_ids,
            )
            .await?)
    }

    /// Creates a fresh Run in an existing Session without restoring any prior
    /// approval, user-input secret, capability grant, lease, or Host snapshot.
    pub async fn create_in_session(
        &self,
        request: AgentRunCreateRequest,
        session: SessionRecord,
    ) -> Result<CreatedAgentRun, AgentRunFactoryError> {
        validate_create_request(&request)?;
        if request.context != session.context
            || request.entry_profile != session.entry_profile
            || request.parent_session_id.is_some()
        {
            return Err(AgentRunFactoryError::ExistingSessionMismatch);
        }
        let run_id = RunId::random();
        let run = RunRecord {
            id: run_id.clone(),
            session_id: session.id.clone(),
            status: RunStatus::Queued,
            purpose: request.purpose,
            origin: request.origin,
            generation: 1,
            configuration: RunConfiguration {
                model_snapshot: request.model_snapshot,
                driver: RunDriverKind::ToolLoop,
                entry_profile: request.entry_profile,
                workload_override: request.workload_override,
                behavior_mode: request.behavior_mode,
                execution_target: request.execution_target,
                approval_policy: request.approval_policy,
                permission_profile: request.permission_profile,
                budget: request.budget,
                accepted_plan_id: None,
                accepted_plan_revision: None,
            },
            requested_capabilities: request.requested_capabilities,
            negotiated_capabilities: ProviderCapabilities::default(),
            provider_capability_probe: None,
            capability_degradations: Vec::new(),
            failure_code: None,
            created_at_ms: request.created_at_ms,
            updated_at_ms: request.created_at_ms,
        };
        let user_item = TranscriptItem {
            id: ItemId::random(),
            session_id: session.id.clone(),
            run_id: Some(run_id),
            sequence: 0,
            kind: TranscriptItemKind::User,
            status: ItemStatus::Completed,
            payload: ItemPayload::User {
                text: request.prompt.trim().to_owned(),
                attachment_ids: request.attachment_ids.clone(),
            },
            relations: ItemRelations::default(),
            created_at_ms: request.created_at_ms,
        };
        Ok(self
            .store
            .create_agent_run_in_session_idempotent(
                &request.principal,
                &request.idempotency_key,
                &session,
                &run,
                &user_item,
                &request.attachment_ids,
            )
            .await?)
    }
}

fn validate_create_request(request: &AgentRunCreateRequest) -> Result<(), AgentRunFactoryError> {
    let prompt = request.prompt.trim();
    if prompt.is_empty() || prompt.chars().count() > 32_000 {
        return Err(AgentRunFactoryError::InvalidPrompt);
    }
    let title = request.title.trim();
    if title.is_empty() || title.chars().count() > 200 {
        return Err(AgentRunFactoryError::InvalidTitle);
    }
    if request.behavior_mode == BehaviorMode::Plan
        && request.permission_profile != PermissionProfile::ReadOnly
    {
        return Err(AgentRunFactoryError::PlanMustBeReadOnly);
    }
    match (
        &request.entry_profile,
        &request.workload_override,
        &request.context,
        &request.execution_target,
    ) {
        (
            EntryProfile::Workbench,
            Some(WorkloadKind::Coding),
            SessionContextBinding::Project { project_id, .. },
            Some(target),
        ) if target.project_id() == project_id => {}
        (EntryProfile::Workbench, Some(WorkloadKind::Coding), _, _) => {
            return Err(AgentRunFactoryError::CodingProjectRequired);
        }
        (
            EntryProfile::PetConversation | EntryProfile::DesktopControl,
            Some(WorkloadKind::Coding | WorkloadKind::Office),
            _,
            _,
        )
        | (
            EntryProfile::PetConversation | EntryProfile::DesktopControl,
            _,
            SessionContextBinding::Project { .. },
            _,
        )
        | (EntryProfile::PetConversation | EntryProfile::DesktopControl, _, _, Some(_)) => {
            return Err(AgentRunFactoryError::EntryContextMismatch);
        }
        (_, _, SessionContextBinding::General | SessionContextBinding::Avatar { .. }, Some(_)) => {
            return Err(AgentRunFactoryError::UnexpectedExecutionTarget);
        }
        _ => {}
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRunPriority {
    Interactive,
    Background,
}

#[derive(Debug, Clone)]
pub struct ActiveAgentRun {
    pub session_id: SessionId,
    pub run_id: RunId,
    pub run_generation: u64,
    pub priority: AgentRunPriority,
    pub parent_run_id: Option<RunId>,
    pub cancellation: CancellationToken,
}

#[derive(Debug, Error)]
pub enum AgentExecutorRegistryError {
    #[error("run is already registered")]
    AlreadyRegistered,
    #[error("run is not registered")]
    NotRegistered,
    #[error("run generation precondition failed")]
    StaleGeneration,
}

#[derive(Debug)]
pub struct AgentExecutorRegistry {
    active: Mutex<BTreeMap<RunId, ActiveAgentRun>>,
    lanes: Arc<SessionLanes>,
    background_slots: Arc<Semaphore>,
}

impl Default for AgentExecutorRegistry {
    fn default() -> Self {
        Self::new(2)
    }
}

impl AgentExecutorRegistry {
    #[must_use]
    pub fn new(max_background_runs: usize) -> Self {
        Self {
            active: Mutex::new(BTreeMap::new()),
            lanes: Arc::new(SessionLanes::default()),
            background_slots: Arc::new(Semaphore::new(max_background_runs.max(1))),
        }
    }

    pub fn register(
        &self,
        run: &RunRecord,
        priority: AgentRunPriority,
        parent_run_id: Option<RunId>,
    ) -> Result<CancellationToken, AgentExecutorRegistryError> {
        let mut active = self.active.lock();
        if active.contains_key(&run.id) {
            return Err(AgentExecutorRegistryError::AlreadyRegistered);
        }
        let cancellation = CancellationToken::new();
        active.insert(
            run.id.clone(),
            ActiveAgentRun {
                session_id: run.session_id.clone(),
                run_id: run.id.clone(),
                run_generation: run.generation,
                priority,
                parent_run_id,
                cancellation: cancellation.clone(),
            },
        );
        Ok(cancellation)
    }

    #[must_use]
    pub fn get(&self, run_id: &RunId) -> Option<ActiveAgentRun> {
        self.active.lock().get(run_id).cloned()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.active.lock().is_empty()
    }

    pub fn cancel(
        &self,
        run_id: &RunId,
        expected_generation: u64,
    ) -> Result<(), AgentExecutorRegistryError> {
        let active = self
            .active
            .lock()
            .get(run_id)
            .cloned()
            .ok_or(AgentExecutorRegistryError::NotRegistered)?;
        if active.run_generation != expected_generation {
            return Err(AgentExecutorRegistryError::StaleGeneration);
        }
        active.cancellation.cancel();
        self.lanes.reset(&active.session_id);
        self.cancel_descendants(run_id);
        Ok(())
    }

    pub fn cancel_descendants(&self, parent_run_id: &RunId) {
        let descendants = self
            .active
            .lock()
            .values()
            .filter(|run| run.parent_run_id.as_ref() == Some(parent_run_id))
            .cloned()
            .collect::<Vec<_>>();
        for child in descendants {
            child.cancellation.cancel();
            self.lanes.reset(&child.session_id);
            self.cancel_descendants(&child.run_id);
        }
    }

    pub fn remove(&self, run_id: &RunId, expected_generation: u64) -> bool {
        let mut active = self.active.lock();
        if active
            .get(run_id)
            .is_some_and(|run| run.run_generation == expected_generation)
        {
            active.remove(run_id);
            true
        } else {
            false
        }
    }

    pub fn reset_session(&self, session_id: &SessionId) {
        self.lanes.reset(session_id);
        for run in self
            .active
            .lock()
            .values()
            .filter(|run| &run.session_id == session_id)
        {
            run.cancellation.cancel();
        }
    }

    async fn background_permit(
        &self,
        priority: AgentRunPriority,
    ) -> Result<Option<OwnedSemaphorePermit>, AgentExecutionError> {
        if priority == AgentRunPriority::Interactive {
            return Ok(None);
        }
        self.background_slots
            .clone()
            .acquire_owned()
            .await
            .map(Some)
            .map_err(|_| AgentExecutionError::RegistryClosed)
    }
}

#[derive(Debug, Error)]
pub enum AgentExecutionError {
    #[error("agent storage failed: {0}")]
    Store(#[from] AgentStoreError),
    #[error("session lane failed: {0}")]
    Lane(#[from] LaneError),
    #[error("run registry is closed")]
    RegistryClosed,
    #[error("run registry rejected execution: {0}")]
    Registry(#[from] AgentExecutorRegistryError),
    #[error("model runtime failed: {0}")]
    Model(#[from] ModelRuntimeError),
    #[error("run preparation failed: {0}")]
    Preparation(String),
    #[error("run is not registered")]
    NotRegistered,
    #[error("run execution failed: {0}")]
    Execution(String),
    #[error("session lane generation changed before completion")]
    StaleLaneGeneration,
    #[error("run recovery context changed: {0}")]
    RecoveryDrift(&'static str),
}

#[derive(Debug, Clone)]
pub struct AgentRunRequest {
    pub principal: String,
    pub session: SessionRecord,
    pub run: RunRecord,
    pub priority: AgentRunPriority,
    pub capability_grants: CapabilityGrantSet,
    pub sandbox_snapshot: SandboxCapabilityReport,
    pub attachment_ids: Vec<AttachmentId>,
    pub skill_allowlist: Vec<SkillId>,
    pub mcp_tool_allowlist: Vec<McpToolSelection>,
    pub run_tool_allowlist: Option<Vec<String>>,
    pub schedule_host_grant: Option<hachimi_protocol::ScheduleHostGrant>,
    pub workload_override: Option<WorkloadKind>,
    pub recovery_checkpoint: Option<hachimi_protocol::RunStepCheckpoint>,
    pub parent_agent_task_id: Option<hachimi_protocol::AgentTaskId>,
    pub parent_run_id: Option<RunId>,
    pub agent_depth: u8,
}

pub struct PreparedAgentRun {
    pub initial_messages: Vec<ModelMessage>,
    pub tool_executors: Vec<Arc<dyn ToolExecutor>>,
    pub host_context: Option<String>,
    pub state: StepRuntimeState,
    pub world_refresher: Option<Arc<dyn crate::StepWorldStateRefresher>>,
    pub diff_tracker: Option<Arc<crate::RunDiffTracker>>,
}

pub type AgentPreparationFuture =
    Pin<Box<dyn Future<Output = Result<PreparedAgentRun, AgentExecutionError>> + Send + 'static>>;

pub trait AgentRunPreparer: Send + Sync {
    fn prepare(
        &self,
        request: AgentRunRequest,
        checkpoint: Option<CompactionCheckpoint>,
        model: Arc<dyn ModelRuntime>,
        cancellation: CancellationToken,
    ) -> AgentPreparationFuture;
}

#[derive(Clone)]
pub struct AgentRunExecutor {
    store: AgentStore,
    registry: Arc<AgentExecutorRegistry>,
    model_factory: Arc<dyn ModelRuntimeFactory>,
    preparer: Arc<dyn AgentRunPreparer>,
}

impl std::fmt::Debug for AgentRunExecutor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentRunExecutor")
            .finish_non_exhaustive()
    }
}

impl AgentRunExecutor {
    #[must_use]
    pub fn new(
        store: AgentStore,
        registry: Arc<AgentExecutorRegistry>,
        model_factory: Arc<dyn ModelRuntimeFactory>,
        preparer: Arc<dyn AgentRunPreparer>,
    ) -> Self {
        Self {
            store,
            registry,
            model_factory,
            preparer,
        }
    }

    #[must_use]
    pub const fn registry(&self) -> &Arc<AgentExecutorRegistry> {
        &self.registry
    }

    /// Runs a bounded, tool-free helper prompt through the same configured
    /// provider as the Session. Product surfaces use this only for optional
    /// suggestions and must retain a deterministic fallback.
    pub async fn generate_auxiliary_text(
        &self,
        configuration: &RunConfiguration,
        system_prompt: &str,
        user_prompt: &str,
        max_output_tokens: u32,
        cancellation: CancellationToken,
    ) -> Result<String, ModelRuntimeError> {
        let model = self.model_factory.create_session(configuration).await?;
        let request = ModelRequest {
            messages: vec![
                ModelMessage {
                    role: ModelRole::System,
                    content: system_prompt.into(),
                    name: None,
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                    input_images: Vec::new(),
                },
                ModelMessage::user(user_prompt),
            ],
            tools: Vec::new(),
            parallel_tool_calls: false,
            max_output_tokens: Some(max_output_tokens.clamp(16, 512)),
        };
        let mut stream = model.stream(request, cancellation.child_token());
        let mut output = String::new();
        while let Some(event) = tokio::select! {
            () = cancellation.cancelled() => return Err(ModelRuntimeError::Cancelled),
            event = stream.next() => event,
        } {
            match event? {
                ModelEvent::AgentMessageDelta { delta, .. } | ModelEvent::TextDelta { delta } => {
                    output.push_str(&delta)
                }
                ModelEvent::Completed { .. } => break,
                ModelEvent::AgentMessageStarted { .. }
                | ModelEvent::AgentMessageCompleted { .. }
                | ModelEvent::ReasoningDelta { .. }
                | ModelEvent::ToolCallDelta { .. }
                | ModelEvent::ToolCallCompleted { .. }
                | ModelEvent::Usage { .. } => {}
            }
        }
        let output = output.trim();
        if output.is_empty() {
            return Err(ModelRuntimeError::InvalidStream(
                "auxiliary model response contained no text".into(),
            ));
        }
        Ok(output.chars().take(1_024).collect())
    }

    pub async fn execute(&self, request: AgentRunRequest) -> Result<(), AgentExecutionError> {
        validate_agent_run_request(&request)?;
        let run = request.run.clone();
        self.registry
            .register(&run, request.priority, request.parent_run_id.clone())?;
        let active = self
            .registry
            .get(&run.id)
            .ok_or(AgentExecutionError::NotRegistered)?;
        if active.run_generation != run.generation || active.session_id != run.session_id {
            return Err(AgentExecutionError::NotRegistered);
        }
        let result = async {
            let _background_permit = self.registry.background_permit(active.priority).await?;
            let permit = self.registry.lanes.enter(&run.session_id).await?;
            self.store
                .assert_run_precondition(&run.id, &run.id, run.generation)
                .await?;
            self.store
                .dispatch_plugin_hook_event(
                    &hachimi_storage::PluginHookEventRecord {
                        event: "run.before".into(),
                        session_id: Some(run.session_id.clone()),
                        run_id: Some(run.id.clone()),
                        run_generation: Some(run.generation),
                        subject: run.id.as_str().into(),
                        result_code: "started".into(),
                        created_at_ms: current_time_ms(),
                    },
                    active.cancellation.child_token(),
                )
                .await?;
            let combined = CancellationToken::new();
            let watcher_stop = CancellationToken::new();
            let watcher = {
                let external = active.cancellation.clone();
                let lane = permit.cancellation();
                let combined = combined.clone();
                let stop = watcher_stop.clone();
                tokio::spawn(async move {
                    tokio::select! {
                        () = external.cancelled() => combined.cancel(),
                        () = lane.cancelled() => combined.cancel(),
                        () = stop.cancelled() => {}
                    }
                })
            };
            let execution_result: Result<(), AgentExecutionError> = async {
                let client = self
                    .model_factory
                    .create_session(&run.configuration)
                    .await?;
                let model: Arc<dyn ModelRuntime> = client;
                let checkpoint = if run.purpose == RunPurpose::Review {
                    None
                } else {
                    let compactor = SemanticCompactor::new(self.store.clone(), Arc::clone(&model))
                        .with_provider_context(&run.configuration.model_snapshot);
                    match compactor
                        .compact_if_needed(&run.session_id, Some(&run.id), combined.child_token())
                        .await
                    {
                        Ok(Some(checkpoint)) => Some(checkpoint),
                        Ok(None) => {
                            self.store
                                .latest_compaction_checkpoint(&run.session_id)
                                .await?
                        }
                        Err(CompactionError::Runtime(ModelRuntimeError::Cancelled)) => {
                            return Err(AgentExecutionError::Model(ModelRuntimeError::Cancelled));
                        }
                        Err(error) => {
                            let code = compaction_error_code(&error);
                            self.store
                                .append_event(
                                    &run.session_id,
                                    Some(&run.id),
                                    "context.compaction_failed",
                                    serde_json::json!({
                                        "code": code,
                                        "fallback": "previous_checkpoint_or_raw_tail"
                                    }),
                                )
                                .await?;
                            self.store
                                .latest_compaction_checkpoint(&run.session_id)
                                .await?
                        }
                    }
                };
                let prepared = self
                    .preparer
                    .prepare(
                        request.clone(),
                        checkpoint,
                        Arc::clone(&model),
                        combined.child_token(),
                    )
                    .await?;
                let tools = Arc::new(
                    ToolRuntime::from_executors(prepared.tool_executors.clone())
                        .map_err(|error| AgentExecutionError::Preparation(error.to_string()))?,
                );
                if let Some(recovery) = request.recovery_checkpoint.as_ref() {
                    if recovery.run_id != run.id
                        || recovery.session_id != run.session_id
                        || recovery.run_generation.saturating_add(1) != run.generation
                    {
                        return Err(AgentExecutionError::RecoveryDrift(
                            "checkpoint lineage no longer matches the Run",
                        ));
                    }
                    let world = prepared.state.snapshot().world;
                    let current = hachimi_protocol::RecoveryRevisionSnapshot {
                        agents_revision: world.agents_revision,
                        skills_revision: world.skills_revision,
                        mcp_revision: world.mcp_revision,
                        plugin_revision: tools.registry().revision().to_owned(),
                        host_revision: world.host_revision,
                        provider_revision: runtime_provider_revision(&model.capabilities()),
                    };
                    if !recovery_revisions_match(&recovery.revision_snapshot, &current)
                        || (recovery.revision_snapshot.host_revision.is_empty()
                            && current.host_revision != recovery.world_revision)
                    {
                        return Err(AgentExecutionError::RecoveryDrift(
                            "Host, Skill, MCP, Plugin, or Sandbox revision changed",
                        ));
                    }
                    if recovery.revision_snapshot.provider_revision.is_empty()
                        && current.provider_revision != recovery.provider_revision
                    {
                        return Err(AgentExecutionError::RecoveryDrift(
                            "Provider capabilities changed",
                        ));
                    }
                }
                prepared
                    .state
                    .narrow_sandbox(request.sandbox_snapshot.clone());
                TurnRuntime::new(self.store.clone(), model, tools)
                    .execute(
                        run.clone(),
                        prepared.initial_messages,
                        RunStepContext {
                            host_context: prepared.host_context,
                            state: prepared.state,
                            run_tool_allowlist: request.run_tool_allowlist.clone(),
                            capability_grants: Some(request.capability_grants.clone()),
                            world_refresher: prepared.world_refresher,
                            diff_tracker: prepared.diff_tracker,
                        },
                        combined,
                    )
                    .await
                    .map_err(|error| AgentExecutionError::Execution(error.to_string()))?;
                Ok(())
            }
            .await;
            watcher_stop.cancel();
            let _ = watcher.await;
            if !self.registry.lanes.is_current(permit.marker()) && execution_result.is_ok() {
                Err(AgentExecutionError::StaleLaneGeneration)
            } else {
                execution_result
            }
        }
        .await;
        let hook_result = self
            .store
            .dispatch_plugin_hook_event(
                &hachimi_storage::PluginHookEventRecord {
                    event: "run.after".into(),
                    session_id: Some(run.session_id.clone()),
                    run_id: Some(run.id.clone()),
                    run_generation: Some(run.generation),
                    subject: run.id.as_str().into(),
                    result_code: if result.is_ok() {
                        "succeeded".into()
                    } else {
                        "failed".into()
                    },
                    created_at_ms: current_time_ms(),
                },
                active.cancellation.child_token(),
            )
            .await;
        self.store
            .finish_run_recovery(&run.id, run.generation, result.is_ok(), current_time_ms())
            .await?;
        let _ = self
            .store
            .invalidate_run_capability_grants(&run.session_id, &run.id, current_time_ms())
            .await;
        if let Some(checkout_id) = request.session.context.checkout_id() {
            let _ = self
                .store
                .release_checkout_write_lease(checkout_id, &run.id, run.generation)
                .await;
        }
        self.registry.remove(&run.id, run.generation);
        self.registry.cancel_descendants(&run.id);
        hook_result?;
        result
    }
}

fn validate_agent_run_request(request: &AgentRunRequest) -> Result<(), AgentExecutionError> {
    let scheduled_origin = matches!(
        request.run.origin,
        hachimi_protocol::RunOrigin::Scheduled { .. }
    );
    if request.run.session_id != request.session.id
        || request.run.configuration.entry_profile != request.session.entry_profile
        || request.run.configuration.workload_override != request.workload_override
        || request.capability_grants.session_id != request.session.id
        || request.capability_grants.run_id.as_ref() != Some(&request.run.id)
        || scheduled_origin != request.schedule_host_grant.is_some()
        || request
            .recovery_checkpoint
            .as_ref()
            .is_some_and(|checkpoint| {
                checkpoint.run_id != request.run.id
                    || checkpoint.session_id != request.session.id
                    || checkpoint.run_generation.saturating_add(1) != request.run.generation
            })
        || request.agent_depth > 3
        || (request.agent_depth == 0
            && (request.parent_agent_task_id.is_some() || request.parent_run_id.is_some()))
        || (request.agent_depth > 0
            && (request.parent_agent_task_id.is_none() || request.parent_run_id.is_none()))
    {
        return Err(AgentExecutionError::Preparation(
            "AgentRunRequest lineage or immutable snapshots do not match".into(),
        ));
    }
    Ok(())
}

fn compaction_error_code(error: &CompactionError) -> &'static str {
    match error {
        CompactionError::QualityRejected(code) => code,
        CompactionError::Runtime(_) => "compaction_model_failed",
        CompactionError::Store(_) => "compaction_store_failed",
        CompactionError::SourceOverflow => "compaction_source_overflow",
    }
}

fn current_time_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX)
}

fn runtime_provider_revision(capabilities: &ProviderCapabilities) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(capabilities).unwrap_or_default());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn recovery_revisions_match(
    expected: &hachimi_protocol::RecoveryRevisionSnapshot,
    current: &hachimi_protocol::RecoveryRevisionSnapshot,
) -> bool {
    [
        (&expected.agents_revision, &current.agents_revision),
        (&expected.skills_revision, &current.skills_revision),
        (&expected.mcp_revision, &current.mcp_revision),
        (&expected.plugin_revision, &current.plugin_revision),
        (&expected.host_revision, &current.host_revision),
        (&expected.provider_revision, &current.provider_revision),
    ]
    .into_iter()
    .all(|(expected, current)| expected.is_empty() || expected == current)
}
