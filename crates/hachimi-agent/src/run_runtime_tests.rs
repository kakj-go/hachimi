use std::sync::{Arc, Mutex};

use futures_util::stream;
use hachimi_protocol::{
    AgentPermissionPolicy, ApprovalPolicy, AuthorityMode, BehaviorMode, DeliveryStatus,
    EntryProfile, LlmSettings, ModelEvent, ModelFinishReason, ModelMessage, ModelRequest,
    PermissionProfile, ProviderCapabilities, RunBudget, RunOrigin, RunPurpose, RunStatus,
    SandboxCapabilityReport, SandboxReadiness, ScopedPermissionRules, SessionContextBinding,
    TaskRunRecord, TaskRunStatus, TaskRunTrigger, TokenUsage, WorkloadKind, WorkloadResolution,
    WorkloadResolutionSource,
};
use tokio_util::sync::CancellationToken;

use crate::{
    AgentInstructionLayer, AgentPreparationFuture, AgentRunCreateRequest, AgentRunExecutor,
    AgentRunLaunchRequest, AgentRunLauncher, AgentRunPreparer, AgentRunPriority, AgentRunRequest,
    ModelClientFuture, ModelEventStream, ModelRuntime, ModelRuntimeError, ModelRuntimeFactory,
    PreparedAgentRun, StepRuntimeState, StepWorldState, UserInputAvailability,
};

#[derive(Debug, Default)]
struct WindowlessModel {
    requests: Mutex<Vec<ModelRequest>>,
}

impl ModelRuntime for WindowlessModel {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            text_input: true,
            streaming_usage: true,
            context_window: Some(16_384),
            max_output_tokens: Some(1_024),
            ..ProviderCapabilities::default()
        }
    }

    fn stream(&self, request: ModelRequest, cancellation: CancellationToken) -> ModelEventStream {
        self.requests.lock().expect("request lock").push(request);
        if cancellation.is_cancelled() {
            return Box::pin(stream::iter([Err(ModelRuntimeError::Cancelled)]));
        }
        Box::pin(stream::iter([
            Ok(ModelEvent::TextDelta {
                delta: "windowless scheduled Run completed".into(),
            }),
            Ok(ModelEvent::Usage {
                usage: TokenUsage {
                    input_tokens: 12,
                    output_tokens: 5,
                },
            }),
            Ok(ModelEvent::Completed {
                finish_reason: ModelFinishReason::Stop,
            }),
        ]))
    }
}

#[derive(Debug, Clone)]
struct WindowlessFactory(Arc<WindowlessModel>);

impl ModelRuntimeFactory for WindowlessFactory {
    fn create_session(
        &self,
        _configuration: &hachimi_protocol::RunConfiguration,
    ) -> ModelClientFuture {
        let model = Arc::clone(&self.0);
        Box::pin(async move { Ok(model as Arc<dyn crate::ModelClientSession>) })
    }
}

#[derive(Debug, Default)]
struct WindowlessPreparer {
    principals: Mutex<Vec<String>>,
}

impl AgentRunPreparer for WindowlessPreparer {
    fn prepare(
        &self,
        request: AgentRunRequest,
        _checkpoint: Option<hachimi_protocol::CompactionCheckpoint>,
        _model: Arc<dyn ModelRuntime>,
        _cancellation: CancellationToken,
    ) -> AgentPreparationFuture {
        self.principals
            .lock()
            .expect("principal lock")
            .push(request.principal);
        Box::pin(async move {
            Ok(PreparedAgentRun {
                initial_messages: vec![ModelMessage::user("run the scheduled task")],
                tool_executors: Vec::new(),
                host_context: Some("service=scheduler;window=none".into()),
                host_revision_snapshot: None,
                state: StepRuntimeState::new(
                    StepWorldState {
                        context_revision: 1,
                        profile_revision: 1,
                        agents_revision: "none".into(),
                        skills_revision: "none".into(),
                        mcp_revision: "none".into(),
                        host_revision: "windowless-test".into(),
                        instructions: Vec::<AgentInstructionLayer>::new().into(),
                        skill_activations: Vec::new().into(),
                        mcp_bindings: Vec::new().into(),
                        disabled_tool_names: Vec::new().into(),
                        diagnostics: Vec::new().into(),
                        sandbox: SandboxCapabilityReport {
                            backend: "test".into(),
                            readiness: SandboxReadiness::Unavailable,
                            os_enforced: false,
                            filesystem_enforced: false,
                            process_enforced: false,
                            network_enforced: false,
                            version: None,
                            stable_error_code: Some("read_only_test".into()),
                            diagnostics: Vec::new(),
                        },
                        host_ready: true,
                    },
                    WorkloadResolution {
                        workload: WorkloadKind::General,
                        source: WorkloadResolutionSource::GeneralFallback,
                        activated_skill_ids: Vec::new(),
                        reason: "windowless General task".into(),
                        classifier_revision: None,
                    },
                ),
                world_refresher: None,
                diff_tracker: None,
            })
        })
    }
}

#[tokio::test]
async fn service_principal_executes_a_background_run_without_a_window_transport() {
    let store = hachimi_storage::AgentStore::connect_in_memory()
        .await
        .expect("store");
    let task_run_id = hachimi_protocol::TaskRunId::from("task-windowless");
    store
        .create_task_run(&TaskRunRecord {
            id: task_run_id.clone(),
            schedule_id: None,
            schedule_revision: None,
            trigger: TaskRunTrigger::Scheduled,
            scheduled_for_ms: Some(1_800_000_000_000),
            event_context: None,
            invocation_key: "test:task-windowless".into(),
            requester_session_id: None,
            execution_session_id: None,
            run_id: None,
            status: TaskRunStatus::Preparing,
            progress_percent: None,
            result_summary: None,
            error_code: None,
            error_summary: None,
            artifact_ids: Vec::new(),
            delivery_status: DeliveryStatus::NotRequested,
            delivery_error_code: None,
            created_at_ms: 1_800_000_000_000,
            started_at_ms: None,
            finished_at_ms: None,
            updated_at_ms: 1_800_000_000_000,
        })
        .await
        .expect("preparing TaskRun");
    let launched = AgentRunLauncher::new(store.clone())
        .launch_new(AgentRunLaunchRequest {
            create: AgentRunCreateRequest {
                principal: "service:scheduler".into(),
                idempotency_key: "windowless-scheduled-run".into(),
                context: SessionContextBinding::Workspace {
                    workspace_id: hachimi_protocol::WorkspaceId::random(),
                },
                origin: RunOrigin::Scheduled {
                    schedule_id: hachimi_protocol::ScheduleId::from("schedule-windowless"),
                    task_run_id,
                    scheduled_for_ms: 1_800_000_000_000,
                    event_context: None,
                },
                title: "Windowless scheduled Run".into(),
                prompt: "run the scheduled task".into(),
                attachment_ids: Vec::new(),
                parent_session_id: None,
                source_run_id: None,
                purpose: RunPurpose::Task,
                model_snapshot: LlmSettings::default(),
                entry_profile: EntryProfile::Workbench,
                workload_override: None,
                behavior_mode: BehaviorMode::Default,
                execution_target: None,
                approval_policy: ApprovalPolicy::NeverPrompt,
                permission_profile: PermissionProfile::ReadOnly,
                budget: RunBudget::default(),
                requested_capabilities: ProviderCapabilities {
                    text_input: true,
                    streaming_usage: true,
                    ..ProviderCapabilities::default()
                },
                created_at_ms: 1_800_000_000_000,
            },
            policy: AgentPermissionPolicy {
                level: PermissionProfile::ReadOnly,
                rules: ScopedPermissionRules::default(),
                revision: 1,
            },
            authority_mode: AuthorityMode::Unattended,
        })
        .await
        .expect("Run bundle");
    let created = launched.created;
    let model = Arc::new(WindowlessModel::default());
    let preparer = Arc::new(WindowlessPreparer::default());
    let registry = Arc::new(crate::AgentExecutorRegistry::new(2));
    let executor = AgentRunExecutor::new(
        store.clone(),
        Arc::clone(&registry),
        Arc::new(WindowlessFactory(Arc::clone(&model))),
        preparer.clone(),
    );
    executor
        .execute(AgentRunRequest {
            principal: "service:scheduler".into(),
            session: created.session.clone(),
            run: created.run.clone(),
            authority: launched.authority,
            priority: AgentRunPriority::Background,
            user_input_availability: UserInputAvailability::Unavailable,
            capability_grants: launched.capability_grants,
            sandbox_snapshot: SandboxCapabilityReport {
                backend: "test".into(),
                readiness: SandboxReadiness::Unavailable,
                os_enforced: false,
                filesystem_enforced: false,
                process_enforced: false,
                network_enforced: false,
                version: None,
                stable_error_code: Some("read_only_test".into()),
                diagnostics: Vec::new(),
            },
            attachment_ids: Vec::new(),
            skill_allowlist: Vec::new(),
            mcp_tool_allowlist: Vec::new(),
            run_tool_allowlist: Some(Vec::new()),
            host_revision_snapshot: Some(hachimi_protocol::HostRevisionSnapshot::default()),
            workload_override: None,
            recovery_checkpoint: None,
            parent_agent_task_id: None,
            parent_run_id: None,
            agent_depth: 0,
        })
        .await
        .expect("windowless execution");

    assert!(registry.is_empty());
    assert_eq!(
        preparer
            .principals
            .lock()
            .expect("principal lock")
            .as_slice(),
        ["service:scheduler"]
    );
    assert_eq!(
        store
            .get_run(&created.run.id)
            .await
            .expect("Run")
            .expect("Run row")
            .status,
        RunStatus::Succeeded
    );
    assert!(
        store
            .list_transcript(&created.session.id)
            .await
            .expect("transcript")
            .iter()
            .any(|item| matches!(
                &item.payload,
                hachimi_protocol::ItemPayload::Assistant { text, .. }
                    if text == "windowless scheduled Run completed"
            ))
    );
    let requests = model.requests.lock().expect("request lock");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].messages.iter().any(|message| {
        message.content.contains("service=scheduler;window=none")
            && message.content.contains("run_generation=1")
    }));
}
