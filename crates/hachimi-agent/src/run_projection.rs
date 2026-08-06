//! Transaction-backed projection of Tool Loop events into Run, Event, and Transcript storage.

use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use hachimi_protocol::{
    AgentMessagePhase, AgentTaskRecord, ArtifactId, ArtifactKind, ArtifactRecord, BehaviorMode,
    CapabilityGrantSet, ItemId, ItemPayload, ItemRelations, ItemStatus, McpServerId, ModelEvent,
    ModelFinishReason, ModelMessage, ModelRole, PlanId, PlanStep, PlanStepId, PlanStepStatus,
    ProcessSessionId, ProposedPlan, ProposedPlanStatus, RunRecord, RunStatus, RunStepCheckpoint,
    RunStepCheckpointId, RunUsageSnapshot, SandboxCapabilityReport, SandboxReadiness,
    ToolExecutionResult, TranscriptItem, TranscriptItemKind, WorkloadKind, WorkloadResolution,
    WorkloadResolutionSource,
};
use hachimi_storage::{AgentStore, AgentStoreError};
use serde_json::json;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    LoopEvent, ModelRuntimeError, RunCheckpointDraft, RunCheckpointFuture, RunCheckpointReporter,
    SteeringFuture, SteeringSource, StepRuntimeState, StepWorldState, ToolLoopDriver,
    ToolLoopOutcome, ToolLoopRunOptions, ToolResultStatus, negotiate_provider_capabilities,
};

pub const AUTHORITY_NEEDS_ATTENTION_EVENT: &str = "agent.authority.needs_attention";

#[derive(Clone)]
struct StoreSteeringSource {
    store: AgentStore,
    run_id: hachimi_protocol::RunId,
}

impl SteeringSource for StoreSteeringSource {
    fn take_pending(&self, run_generation: u64) -> SteeringFuture {
        let store = self.store.clone();
        let run_id = self.run_id.clone();
        Box::pin(async move {
            store
                .drain_run_steers(&run_id, run_generation, now_ms())
                .await
                .map(|records| records.into_iter().map(|record| record.input).collect())
                .map_err(|error| ModelRuntimeError::Provider(error.to_string()))
        })
    }
}

#[derive(Clone)]
struct StoreRunCheckpointReporter {
    store: AgentStore,
    session_id: hachimi_protocol::SessionId,
    run_id: hachimi_protocol::RunId,
    run_generation: u64,
}

impl RunCheckpointReporter for StoreRunCheckpointReporter {
    fn report(&self, draft: RunCheckpointDraft) -> RunCheckpointFuture {
        let store = self.store.clone();
        let session_id = self.session_id.clone();
        let run_id = self.run_id.clone();
        let run_generation = self.run_generation;
        Box::pin(async move {
            let world_revision = draft.revision_snapshot.host_revision.clone();
            let provider_revision = draft.revision_snapshot.provider_revision.clone();
            store
                .record_run_step_checkpoint(&RunStepCheckpoint {
                    id: RunStepCheckpointId::random(),
                    session_id,
                    run_id,
                    run_generation,
                    step_index: draft.step_index,
                    phase: draft.phase,
                    tool_call_id: draft.tool_call_id,
                    tool_name: draft.tool_name,
                    side_effect_execution_id: draft.side_effect_execution_id,
                    recovery_policy: draft.recovery_policy,
                    parameter_hash: draft.parameter_hash,
                    world_revision,
                    provider_revision,
                    revision_snapshot: draft.revision_snapshot,
                    created_at_ms: now_ms(),
                })
                .await
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
    }
}

#[derive(Debug, Error)]
pub enum PersistedRunError {
    #[error("run storage failed: {0}")]
    Store(#[from] AgentStoreError),
    #[error("model runtime failed: {0}")]
    Runtime(#[from] ModelRuntimeError),
    #[error("run event projector stopped unexpectedly")]
    ProjectorStopped,
}

#[derive(Clone)]
pub struct PersistedToolLoop {
    store: AgentStore,
    driver: Arc<ToolLoopDriver>,
}

#[derive(Clone)]
pub struct RunStepContext {
    pub host_context: Option<String>,
    pub state: StepRuntimeState,
    pub run_tool_allowlist: Option<Vec<String>>,
    pub capability_grants: Option<CapabilityGrantSet>,
    pub world_refresher: Option<Arc<dyn crate::StepWorldStateRefresher>>,
    pub diff_tracker: Option<Arc<crate::RunDiffTracker>>,
}

impl std::fmt::Debug for RunStepContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RunStepContext")
            .field("host_context", &self.host_context)
            .field("state", &self.state)
            .field("run_tool_allowlist", &self.run_tool_allowlist)
            .field(
                "world_refresher",
                &self.world_refresher.as_ref().map(|_| "configured"),
            )
            .field(
                "diff_tracker",
                &self.diff_tracker.as_ref().map(|_| "configured"),
            )
            .finish()
    }
}

impl std::fmt::Debug for PersistedToolLoop {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PersistedToolLoop")
            .field("driver", &self.driver)
            .finish_non_exhaustive()
    }
}

impl PersistedToolLoop {
    #[must_use]
    pub fn new(store: AgentStore, driver: Arc<ToolLoopDriver>) -> Self {
        Self { store, driver }
    }

    pub async fn execute(
        &self,
        run: RunRecord,
        initial_messages: Vec<ModelMessage>,
        cancellation: CancellationToken,
    ) -> Result<ToolLoopOutcome, PersistedRunError> {
        self.execute_with_context(run, initial_messages, None, cancellation)
            .await
    }

    pub async fn execute_with_context(
        &self,
        run: RunRecord,
        initial_messages: Vec<ModelMessage>,
        host_context: Option<&str>,
        cancellation: CancellationToken,
    ) -> Result<ToolLoopOutcome, PersistedRunError> {
        let workload = run
            .configuration
            .workload_override
            .unwrap_or(WorkloadKind::General);
        self.execute_with_step_context(
            run,
            initial_messages,
            RunStepContext {
                host_context: host_context.map(str::to_owned),
                state: StepRuntimeState::new(
                    unavailable_world_state(host_context),
                    WorkloadResolution {
                        workload,
                        source: if workload == WorkloadKind::General {
                            WorkloadResolutionSource::GeneralFallback
                        } else {
                            WorkloadResolutionSource::UserOverride
                        },
                        activated_skill_ids: Vec::new(),
                        reason: "initial runtime workload".into(),
                        classifier_revision: None,
                    },
                ),
                run_tool_allowlist: None,
                capability_grants: None,
                world_refresher: None,
                diff_tracker: None,
            },
            cancellation,
        )
        .await
    }

    pub async fn execute_with_step_context(
        &self,
        mut run: RunRecord,
        initial_messages: Vec<ModelMessage>,
        step_context: RunStepContext,
        cancellation: CancellationToken,
    ) -> Result<ToolLoopOutcome, PersistedRunError> {
        let proposed_goal = initial_messages
            .iter()
            .rev()
            .find(|message| message.role == ModelRole::User)
            .map(|message| message.content.clone())
            .unwrap_or_default();
        self.store
            .transition_run(&run.id, RunStatus::Preparing, None)
            .await?;
        let (negotiated, degradations) = negotiate_provider_capabilities(
            run.requested_capabilities,
            self.driver.provider_capabilities(),
        );
        let capability_probe = self.driver.provider_capability_probe();
        run = self
            .store
            .update_run_capabilities(
                &run,
                negotiated,
                capability_probe.as_ref(),
                &degradations,
                now_ms(),
            )
            .await?;
        if !run.negotiated_capabilities.text_input {
            self.store
                .transition_run(
                    &run.id,
                    RunStatus::Failed,
                    Some("provider_text_input_unsupported"),
                )
                .await?;
            return Err(PersistedRunError::Runtime(
                ModelRuntimeError::UnsupportedCapability("text_input"),
            ));
        }
        if cancellation.is_cancelled() {
            self.store
                .transition_run(&run.id, RunStatus::Cancelled, None)
                .await?;
            return Err(PersistedRunError::Runtime(ModelRuntimeError::Cancelled));
        }
        self.store
            .transition_run(&run.id, RunStatus::Running, None)
            .await?;
        let (event_sender, event_receiver) = mpsc::unbounded_channel();
        let projector_store = self.store.clone();
        let projector_run = run.clone();
        let projector = tokio::spawn(async move {
            project_loop_events(projector_store, projector_run, event_receiver).await
        });
        let request_context = format!(
            "run_id={}; execution_target={:?}; permission_profile={:?}; approval_policy={:?}; accepted_plan_id={:?}; accepted_plan_revision={:?}; host_context={}.",
            run.id,
            run.configuration.execution_target,
            run.configuration.permission_profile,
            run.configuration.approval_policy,
            run.configuration.accepted_plan_id,
            run.configuration.accepted_plan_revision,
            step_context
                .host_context
                .as_deref()
                .unwrap_or("not supplied"),
        );
        let outcome = self
            .driver
            .run(
                initial_messages,
                ToolLoopRunOptions {
                    session_id: run.session_id.clone(),
                    run_id: run.id.clone(),
                    entry_profile: run.configuration.entry_profile,
                    state: step_context.state.clone(),
                    mode: run.configuration.behavior_mode,
                    origin: run.origin.clone(),
                    context: self
                        .store
                        .get_session(&run.session_id)
                        .await?
                        .ok_or_else(|| AgentStoreError::SessionNotFound(run.session_id.clone()))?
                        .context,
                    run_generation: run.generation,
                    budget: &run.configuration.budget,
                    run_tool_allowlist: step_context.run_tool_allowlist.clone(),
                    capability_grants: step_context.capability_grants.clone(),
                    request_context: Some(&request_context),
                    world_refresher: step_context.world_refresher.clone(),
                    steering: Some(Arc::new(StoreSteeringSource {
                        store: self.store.clone(),
                        run_id: run.id.clone(),
                    })),
                    checkpoint_reporter: Some(Arc::new(StoreRunCheckpointReporter {
                        store: self.store.clone(),
                        session_id: run.session_id.clone(),
                        run_id: run.id.clone(),
                        run_generation: run.generation,
                    })),
                    cancellation: cancellation.clone(),
                },
                |event| {
                    let _ = event_sender.send(event);
                },
            )
            .await;
        drop(event_sender);
        let projection = projector
            .await
            .map_err(|_| PersistedRunError::ProjectorStopped)??;
        let outcome = if cancellation.is_cancelled() {
            Err(ModelRuntimeError::Cancelled)
        } else {
            outcome
        };
        if let Some(tracker) = &step_context.diff_tracker {
            // A final Diff failure degrades the summary only; it never changes the Run result.
            let _ = tracker.refresh(&CancellationToken::new()).await;
        }
        if let Some(open) = projection.open_assistant {
            let status = if matches!(outcome, Err(ModelRuntimeError::Cancelled)) {
                ItemStatus::Interrupted
            } else {
                ItemStatus::Failed
            };
            self.store
                .complete_transcript_item(
                    &open.item_id,
                    status,
                    ItemPayload::Assistant {
                        text: open.text,
                        phase: open.phase,
                    },
                )
                .await?;
        }
        if let Some(open) = projection.open_reasoning {
            let status = if matches!(outcome, Err(ModelRuntimeError::Cancelled)) {
                ItemStatus::Interrupted
            } else {
                ItemStatus::Failed
            };
            self.store
                .complete_transcript_item(&open.item_id, status, reasoning_payload(&run, open.text))
                .await?;
        }
        match outcome {
            Ok(outcome) => {
                let completed_at_ms = now_ms();
                let (kind, payload) = if run.configuration.behavior_mode == BehaviorMode::Plan {
                    let plan = self
                        .store
                        .create_proposed_plan(ProposedPlan {
                            id: PlanId::random(),
                            session_id: run.session_id.clone(),
                            run_id: run.id.clone(),
                            revision: 0,
                            goal: proposed_goal,
                            assumptions: Vec::new(),
                            steps: plan_steps(&outcome.final_text),
                            affected_resources: Vec::new(),
                            verification: Vec::new(),
                            risks: Vec::new(),
                            open_questions: Vec::new(),
                            content_markdown: outcome.final_text.clone(),
                            status: ProposedPlanStatus::Proposed,
                            accepted_run_id: None,
                            created_at_ms: completed_at_ms,
                            accepted_at_ms: None,
                        })
                        .await?;
                    (
                        TranscriptItemKind::Plan,
                        ItemPayload::Plan {
                            plan_id: plan.id,
                            revision: plan.revision,
                            text: plan.content_markdown,
                            steps: plan.steps,
                        },
                    )
                } else {
                    (
                        TranscriptItemKind::Assistant,
                        ItemPayload::Assistant {
                            text: outcome.final_text.clone(),
                            phase: AgentMessagePhase::FinalAnswer,
                        },
                    )
                };
                let already_projected = kind == TranscriptItemKind::Assistant
                    && projection.last_assistant.as_ref().is_some_and(|assistant| {
                        assistant.text == outcome.final_text
                            && assistant.phase == AgentMessagePhase::FinalAnswer
                    });
                if !already_projected {
                    let item_id = ItemId::random();
                    self.store
                        .append_transcript_item(TranscriptItem {
                            id: item_id.clone(),
                            session_id: run.session_id.clone(),
                            run_id: Some(run.id.clone()),
                            sequence: 0,
                            kind,
                            status: ItemStatus::InProgress,
                            payload: payload.clone(),
                            relations: ItemRelations::default(),
                            created_at_ms: completed_at_ms,
                        })
                        .await?;
                    self.store
                        .complete_transcript_item(&item_id, ItemStatus::Completed, payload)
                        .await?;
                }
                if let Some(plan_id) = run.configuration.accepted_plan_id.as_ref()
                    && let Some(plan) = self.store.get_proposed_plan(plan_id).await?
                {
                    let completed_steps = plan
                        .steps
                        .into_iter()
                        .map(|mut step| {
                            step.status = PlanStepStatus::Completed;
                            step
                        })
                        .collect::<Vec<_>>();
                    self.store
                        .update_execution_plan(
                            plan_id,
                            &run.id,
                            Some("Execution completed successfully."),
                            &completed_steps,
                        )
                        .await?;
                }
                self.store
                    .transition_run(&run.id, RunStatus::Succeeded, None)
                    .await?;
                Ok(outcome)
            }
            Err(ModelRuntimeError::Cancelled) => {
                let current = self
                    .store
                    .get_run(&run.id)
                    .await?
                    .ok_or_else(|| AgentStoreError::RunNotFound(run.id.clone()))?;
                if current.status != RunStatus::Cancelling
                    && current.status.can_transition_to(RunStatus::Cancelling)
                {
                    self.store
                        .transition_run(&run.id, RunStatus::Cancelling, None)
                        .await?;
                }
                self.store
                    .transition_run(&run.id, RunStatus::Cancelled, None)
                    .await?;
                Err(PersistedRunError::Runtime(ModelRuntimeError::Cancelled))
            }
            Err(ModelRuntimeError::NeedsAttention(code)) => {
                self.store
                    .append_transcript_item(TranscriptItem {
                        id: ItemId::random(),
                        session_id: run.session_id.clone(),
                        run_id: Some(run.id.clone()),
                        sequence: 0,
                        kind: TranscriptItemKind::SystemContext,
                        status: ItemStatus::Failed,
                        payload: ItemPayload::SystemContext {
                            code: "agent_authority_needs_attention".into(),
                            message: format!(
                                "This Run stopped because its unattended authority was exceeded: {code}"
                            ),
                        },
                        relations: ItemRelations::default(),
                        created_at_ms: now_ms(),
                    })
                    .await?;
                self.store
                    .transition_run(
                        &run.id,
                        RunStatus::Failed,
                        Some("authority_needs_attention"),
                    )
                    .await?;
                Err(PersistedRunError::Runtime(
                    ModelRuntimeError::NeedsAttention(code),
                ))
            }
            Err(error) => {
                self.store
                    .append_transcript_item(TranscriptItem {
                        id: ItemId::random(),
                        session_id: run.session_id.clone(),
                        run_id: Some(run.id.clone()),
                        sequence: 0,
                        kind: TranscriptItemKind::SystemContext,
                        status: ItemStatus::Failed,
                        payload: ItemPayload::SystemContext {
                            code: "agent_runtime_failed".into(),
                            message: error.to_string(),
                        },
                        relations: ItemRelations::default(),
                        created_at_ms: now_ms(),
                    })
                    .await?;
                self.store
                    .transition_run(&run.id, RunStatus::Failed, Some("agent_runtime_failed"))
                    .await?;
                Err(PersistedRunError::Runtime(error))
            }
        }
    }
}

#[derive(Debug)]
struct AssistantProjection {
    item_id: ItemId,
    message_id: String,
    text: String,
    phase: AgentMessagePhase,
}

#[derive(Debug)]
struct TextProjection {
    item_id: ItemId,
    text: String,
}

#[derive(Debug, Default)]
struct LoopProjection {
    open_assistant: Option<AssistantProjection>,
    open_reasoning: Option<TextProjection>,
    last_assistant: Option<AssistantProjection>,
    saw_tool_call: bool,
}

async fn start_assistant_projection(
    store: &AgentStore,
    run: &RunRecord,
    projection: &mut LoopProjection,
    message_id: String,
    phase: AgentMessagePhase,
) -> Result<(), AgentStoreError> {
    if let Some(open) = projection.open_assistant.as_mut()
        && open.message_id == message_id
    {
        if open.phase == AgentMessagePhase::Unknown && phase != AgentMessagePhase::Unknown {
            open.phase = phase;
        }
        return Ok(());
    }
    if let Some(mut previous) = projection.open_assistant.take() {
        if previous.phase == AgentMessagePhase::Unknown {
            previous.phase = AgentMessagePhase::Commentary;
        }
        store
            .complete_transcript_item(
                &previous.item_id,
                ItemStatus::Completed,
                ItemPayload::Assistant {
                    text: previous.text.clone(),
                    phase: previous.phase,
                },
            )
            .await?;
        projection.last_assistant = Some(previous);
    }
    let item_id = ItemId::random();
    store
        .append_transcript_item(TranscriptItem {
            id: item_id.clone(),
            session_id: run.session_id.clone(),
            run_id: Some(run.id.clone()),
            sequence: 0,
            kind: TranscriptItemKind::Assistant,
            status: ItemStatus::InProgress,
            payload: ItemPayload::Assistant {
                text: String::new(),
                phase,
            },
            relations: ItemRelations::default(),
            created_at_ms: now_ms(),
        })
        .await?;
    projection.open_assistant = Some(AssistantProjection {
        item_id,
        message_id,
        text: String::new(),
        phase,
    });
    Ok(())
}

async fn complete_assistant_projection(
    store: &AgentStore,
    projection: &mut LoopProjection,
    inferred_phase: AgentMessagePhase,
) -> Result<(), AgentStoreError> {
    let Some(mut assistant) = projection.open_assistant.take() else {
        return Ok(());
    };
    if assistant.phase == AgentMessagePhase::Unknown {
        assistant.phase = inferred_phase;
    }
    store
        .complete_transcript_item(
            &assistant.item_id,
            ItemStatus::Completed,
            ItemPayload::Assistant {
                text: assistant.text.clone(),
                phase: assistant.phase,
            },
        )
        .await?;
    projection.last_assistant = Some(assistant);
    Ok(())
}

async fn project_loop_events(
    store: AgentStore,
    run: RunRecord,
    mut receiver: mpsc::UnboundedReceiver<LoopEvent>,
) -> Result<LoopProjection, AgentStoreError> {
    let mut calls = BTreeMap::new();
    let mut projection = LoopProjection::default();
    while let Some(event) = receiver.recv().await {
        match event {
            LoopEvent::UsageReconciled {
                billed_usage,
                active_context_tokens,
                remaining_context_tokens,
                source,
            } => {
                store
                    .upsert_run_usage_snapshot(&RunUsageSnapshot {
                        run_id: run.id.clone(),
                        billed_usage,
                        active_context_tokens,
                        remaining_context_tokens,
                        source,
                        updated_at_ms: now_ms(),
                    })
                    .await?;
            }
            LoopEvent::ToolStarted(call) => {
                let item_id = ItemId::random();
                calls.insert(call.id.clone(), (call.clone(), item_id.clone()));
                let (kind, payload) = projected_tool_start(&run, &call);
                store
                    .append_transcript_item(TranscriptItem {
                        id: item_id,
                        session_id: run.session_id.clone(),
                        run_id: Some(run.id.clone()),
                        sequence: 0,
                        kind,
                        status: ItemStatus::InProgress,
                        payload,
                        relations: ItemRelations {
                            tool_call_id: Some(call.id.clone()),
                            process_session_id: (call.name == "workspace_exec")
                                .then(|| ProcessSessionId::new(call.id.to_string())),
                            ..ItemRelations::default()
                        },
                        created_at_ms: now_ms(),
                    })
                    .await?;
            }
            LoopEvent::ToolCompleted(result) => {
                let status = tool_status(result.status);
                let (persisted_model_content, persisted_structured_content) =
                    persisted_tool_result(&result);
                let call = calls.remove(&result.call_id);
                if let Some(artifact) =
                    evidence_artifact(&run, &result, call.as_ref().map(|(call, _)| call))
                {
                    store.create_artifact(&artifact).await?;
                }
                if result
                    .structured_content
                    .get("needsAttention")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
                {
                    store
                        .append_event(
                            &run.session_id,
                            Some(&run.id),
                            AUTHORITY_NEEDS_ATTENTION_EVENT,
                            serde_json::json!({
                                "code": result
                                    .structured_content
                                    .get("code")
                                    .and_then(serde_json::Value::as_str)
                                    .unwrap_or("authority_needs_attention"),
                                "tool": result.tool_name,
                            }),
                        )
                        .await?;
                }
                if let Some((call, item_id)) = call {
                    let payload = projected_tool_completed(
                        &run,
                        &call,
                        &result,
                        status,
                        persisted_model_content,
                        persisted_structured_content,
                    );
                    if let ItemPayload::CollabToolCall {
                        agent_task_id: Some(task_id),
                        ..
                    } = &payload
                    {
                        store
                            .link_agent_task_transcript_item(task_id, &item_id)
                            .await?;
                    }
                    store
                        .complete_transcript_item(
                            &item_id,
                            match result.status {
                                ToolResultStatus::Succeeded => ItemStatus::Completed,
                                ToolResultStatus::Aborted => ItemStatus::Interrupted,
                                ToolResultStatus::Failed
                                | ToolResultStatus::Rejected
                                | ToolResultStatus::TimedOut => ItemStatus::Failed,
                            },
                            payload,
                        )
                        .await?;
                    if let Some(checkpoint) = store
                        .latest_run_step_checkpoint_for_tool(
                            &run.id,
                            run.generation,
                            &result.call_id,
                        )
                        .await?
                    {
                        store
                            .record_run_step_checkpoint(&RunStepCheckpoint {
                                id: RunStepCheckpointId::random(),
                                phase: hachimi_protocol::RunStepPhase::ProjectionCommitted,
                                created_at_ms: now_ms(),
                                ..checkpoint
                            })
                            .await?;
                    }
                }
            }
            // Codex keeps usage as connection/run state instead of replayable
            // transcript history. `UsageReconciled` below persists the
            // authoritative snapshot after active-context reconciliation.
            LoopEvent::Model(ModelEvent::Usage { .. }) => {}
            LoopEvent::Model(ModelEvent::AgentMessageStarted { message_id, phase })
                if run.configuration.behavior_mode != BehaviorMode::Plan =>
            {
                start_assistant_projection(&store, &run, &mut projection, message_id, phase)
                    .await?;
            }
            LoopEvent::Model(ModelEvent::AgentMessageDelta { message_id, delta })
                if run.configuration.behavior_mode != BehaviorMode::Plan =>
            {
                start_assistant_projection(
                    &store,
                    &run,
                    &mut projection,
                    message_id,
                    AgentMessagePhase::Unknown,
                )
                .await?;
                let assistant = projection
                    .open_assistant
                    .as_mut()
                    .expect("assistant projection initialized");
                assistant.text.push_str(&delta);
                store
                    .append_live_item_delta(&run.session_id, &run.id, &assistant.item_id, &delta)
                    .await?;
            }
            LoopEvent::Model(ModelEvent::AgentMessageCompleted { message_id })
                if run.configuration.behavior_mode != BehaviorMode::Plan =>
            {
                let should_complete = projection.open_assistant.as_ref().is_some_and(|assistant| {
                    assistant.message_id == message_id
                        && assistant.phase != AgentMessagePhase::Unknown
                });
                if should_complete {
                    complete_assistant_projection(
                        &store,
                        &mut projection,
                        AgentMessagePhase::Unknown,
                    )
                    .await?;
                }
            }
            LoopEvent::Model(ModelEvent::TextDelta { delta })
                if run.configuration.behavior_mode != BehaviorMode::Plan =>
            {
                start_assistant_projection(
                    &store,
                    &run,
                    &mut projection,
                    "legacy-message-0".into(),
                    AgentMessagePhase::Unknown,
                )
                .await?;
                let assistant = projection
                    .open_assistant
                    .as_mut()
                    .expect("assistant projection initialized");
                assistant.text.push_str(&delta);
                store
                    .append_live_item_delta(&run.session_id, &run.id, &assistant.item_id, &delta)
                    .await?;
            }
            LoopEvent::Model(ModelEvent::ReasoningDelta { delta }) => {
                if projection.open_reasoning.is_none() {
                    let item_id = ItemId::random();
                    store
                        .append_transcript_item(TranscriptItem {
                            id: item_id.clone(),
                            session_id: run.session_id.clone(),
                            run_id: Some(run.id.clone()),
                            sequence: 0,
                            kind: TranscriptItemKind::Reasoning,
                            status: ItemStatus::InProgress,
                            payload: reasoning_payload(&run, String::new()),
                            relations: ItemRelations::default(),
                            created_at_ms: now_ms(),
                        })
                        .await?;
                    projection.open_reasoning = Some(TextProjection {
                        item_id,
                        text: String::new(),
                    });
                }
                let reasoning = projection
                    .open_reasoning
                    .as_mut()
                    .expect("reasoning projection initialized");
                reasoning.text.push_str(&delta);
                store
                    .append_live_item_delta(&run.session_id, &run.id, &reasoning.item_id, &delta)
                    .await?;
            }
            LoopEvent::Model(
                ModelEvent::ToolCallDelta { .. } | ModelEvent::ToolCallCompleted { .. },
            ) => projection.saw_tool_call = true,
            LoopEvent::Model(ModelEvent::Completed { finish_reason }) => {
                let inferred_phase =
                    if projection.saw_tool_call || finish_reason == ModelFinishReason::ToolCalls {
                        AgentMessagePhase::Commentary
                    } else {
                        AgentMessagePhase::FinalAnswer
                    };
                complete_assistant_projection(&store, &mut projection, inferred_phase).await?;
                projection.saw_tool_call = false;
                if let Some(reasoning) = projection.open_reasoning.take() {
                    store
                        .complete_transcript_item(
                            &reasoning.item_id,
                            ItemStatus::Completed,
                            reasoning_payload(&run, reasoning.text),
                        )
                        .await?;
                }
            }
            LoopEvent::Model(_) => {}
            LoopEvent::ContextCompacted {
                before_chars,
                after_chars,
                reason,
            } => {
                store
                    .append_event(
                        &run.session_id,
                        Some(&run.id),
                        "context.reactive_compacted",
                        json!({
                            "beforeChars": before_chars,
                            "afterChars": after_chars,
                            "reason": reason,
                        }),
                    )
                    .await?;
            }
        }
    }
    Ok(projection)
}

fn tool_status(status: ToolResultStatus) -> &'static str {
    match status {
        ToolResultStatus::Succeeded => "succeeded",
        ToolResultStatus::Failed => "failed",
        ToolResultStatus::Rejected => "rejected",
        ToolResultStatus::Aborted => "aborted",
        ToolResultStatus::TimedOut => "timed_out",
    }
}

fn reasoning_payload(run: &RunRecord, summary: String) -> ItemPayload {
    let mut hasher = Sha256::new();
    hasher.update(
        serde_json::to_vec(&(
            &run.configuration.model_snapshot,
            run.negotiated_capabilities,
        ))
        .unwrap_or_default(),
    );
    let capability_revision = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    ItemPayload::Reasoning {
        summary,
        source: hachimi_protocol::ReasoningSummarySource::ProviderPublic,
        provider_endpoint_id: run
            .configuration
            .model_snapshot
            .provider_endpoint_id
            .clone(),
        provider_account_id: run.configuration.model_snapshot.provider_account_id.clone(),
        protocol: run.configuration.model_snapshot.protocol,
        capability_revision,
    }
}

fn unavailable_world_state(host_context: Option<&str>) -> StepWorldState {
    StepWorldState {
        context_revision: 1,
        profile_revision: 1,
        agents_revision: "not_loaded".into(),
        skills_revision: "not_loaded".into(),
        mcp_revision: "not_loaded".into(),
        host_revision: host_context.unwrap_or("not_supplied").into(),
        instructions: Arc::from([]),
        skill_activations: Arc::from([]),
        mcp_bindings: Arc::from([]),
        disabled_tool_names: Arc::from([]),
        diagnostics: Arc::from([]),
        sandbox: SandboxCapabilityReport {
            backend: "runtime_snapshot_unavailable".into(),
            readiness: SandboxReadiness::Unavailable,
            os_enforced: false,
            filesystem_enforced: false,
            process_enforced: false,
            network_enforced: false,
            version: None,
            stable_error_code: Some("runtime_snapshot_unavailable".into()),
            diagnostics: Vec::new(),
        },
        host_ready: true,
    }
}

fn projected_tool_start(
    run: &RunRecord,
    call: &crate::ToolCall,
) -> (TranscriptItemKind, ItemPayload) {
    if call.name == "workspace_exec" {
        let command = command_text(&call.arguments);
        return (
            TranscriptItemKind::CommandExecution,
            ItemPayload::CommandExecution {
                process_session_id: ProcessSessionId::new(call.id.to_string()),
                command_summary: command.chars().take(240).collect(),
                command,
                cwd: call
                    .arguments
                    .get("cwd")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                status: "running".into(),
                aggregated_output: String::new(),
                exit_code: None,
                duration_ms: None,
                output_artifact_id: None,
            },
        );
    }
    if matches!(
        call.name.as_str(),
        "workspace_write_file" | "workspace_replace_text"
    ) {
        let path = call
            .arguments
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("workspace")
            .to_owned();
        return (
            TranscriptItemKind::FileChange,
            ItemPayload::FileChange {
                path,
                change_kind: "modifying".into(),
                artifact_id: None,
            },
        );
    }
    if is_collab_tool(&call.name) {
        return (
            TranscriptItemKind::CollabToolCall,
            ItemPayload::CollabToolCall {
                tool_name: call.name.clone(),
                agent_task_id: None,
                parent_run_id: run.id.clone(),
                child_run_id: None,
                title: call
                    .arguments
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(&call.name)
                    .to_owned(),
                status: "running".into(),
                summary: None,
                usage: Default::default(),
            },
        );
    }
    if call.name.starts_with("mcp_") {
        return (
            TranscriptItemKind::McpCall,
            ItemPayload::McpCall {
                server_id: McpServerId::new(mcp_server_label(&call.name)),
                tool_name: call.name.clone(),
                status: "running".into(),
                arguments: transcript_tool_arguments(call),
                result: None,
                error: None,
            },
        );
    }
    if let Some((namespace, name)) = dynamic_tool_parts(&call.name) {
        return (
            TranscriptItemKind::DynamicToolCall,
            ItemPayload::DynamicToolCall {
                namespace,
                name,
                status: "running".into(),
                arguments: transcript_tool_arguments(call),
                result: None,
                error: None,
            },
        );
    }
    (
        TranscriptItemKind::ToolExecution,
        ItemPayload::ToolExecution {
            tool_call_id: call.id.clone(),
            name: call.name.clone(),
            arguments: transcript_tool_arguments(call),
            step_revision: call.step_revision,
            tool_plan_hash: call.tool_plan_hash.clone(),
            registry_revision: call.registry_revision.clone(),
            result: None,
        },
    )
}

fn projected_tool_completed(
    run: &RunRecord,
    call: &crate::ToolCall,
    result: &crate::ToolResult,
    status: &str,
    model_content: String,
    structured_content: serde_json::Value,
) -> ItemPayload {
    if call.name == "workspace_exec" {
        let stdout = structured_content
            .get("stdout")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let stderr = structured_content
            .get("stderr")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let output = match (stdout.is_empty(), stderr.is_empty()) {
            (false, false) => format!("{stdout}\n{stderr}"),
            (false, true) => stdout.to_owned(),
            (true, false) => stderr.to_owned(),
            (true, true) => model_content,
        };
        let command = command_text(&call.arguments);
        return ItemPayload::CommandExecution {
            process_session_id: ProcessSessionId::new(call.id.to_string()),
            command_summary: command.chars().take(240).collect(),
            command,
            cwd: call
                .arguments
                .get("cwd")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            status: status.into(),
            aggregated_output: bounded_output(&output, 262_144),
            exit_code: structured_value(&structured_content, "exitCode", "exit_code")
                .and_then(serde_json::Value::as_i64)
                .and_then(|value| i32::try_from(value).ok()),
            duration_ms: structured_value(&structured_content, "durationMs", "duration_ms")
                .and_then(serde_json::Value::as_u64),
            output_artifact_id: None,
        };
    }
    if matches!(
        call.name.as_str(),
        "workspace_write_file" | "workspace_replace_text"
    ) {
        return ItemPayload::FileChange {
            path: call
                .arguments
                .get("path")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("workspace")
                .to_owned(),
            change_kind: if status == "succeeded" {
                "modified"
            } else {
                status
            }
            .into(),
            artifact_id: None,
        };
    }
    if is_collab_tool(&call.name) {
        let task = collab_task(&structured_content);
        return ItemPayload::CollabToolCall {
            tool_name: call.name.clone(),
            agent_task_id: task.as_ref().map(|task| task.id.clone()).or_else(|| {
                call.arguments
                    .get("taskId")
                    .and_then(serde_json::Value::as_str)
                    .map(hachimi_protocol::AgentTaskId::from)
            }),
            parent_run_id: run.id.clone(),
            child_run_id: task.as_ref().map(|task| task.child_run_id.clone()),
            title: task
                .as_ref()
                .map(|task| task.title.clone())
                .or_else(|| {
                    call.arguments
                        .get("title")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| call.name.clone()),
            status: task
                .as_ref()
                .map(|task| format!("{:?}", task.status).to_ascii_lowercase())
                .unwrap_or_else(|| status.into()),
            summary: task
                .as_ref()
                .and_then(|task| task.result_summary.clone())
                .or_else(|| (!model_content.is_empty()).then(|| model_content.clone())),
            usage: task.map_or_else(Default::default, |task| task.usage),
        };
    }
    if call.name.starts_with("mcp_") {
        return ItemPayload::McpCall {
            server_id: McpServerId::new(mcp_server_label(&call.name)),
            tool_name: call.name.clone(),
            status: status.into(),
            arguments: transcript_tool_arguments(call),
            result: Some(structured_content),
            error: (status != "succeeded").then_some(model_content),
        };
    }
    if let Some((namespace, name)) = dynamic_tool_parts(&call.name) {
        return ItemPayload::DynamicToolCall {
            namespace,
            name,
            status: status.into(),
            arguments: transcript_tool_arguments(call),
            result: Some(structured_content),
            error: (status != "succeeded").then_some(model_content),
        };
    }
    ItemPayload::ToolExecution {
        tool_call_id: result.call_id.clone(),
        name: result.tool_name.clone(),
        arguments: transcript_tool_arguments(call),
        step_revision: call.step_revision,
        tool_plan_hash: call.tool_plan_hash.clone(),
        registry_revision: call.registry_revision.clone(),
        result: Some(ToolExecutionResult {
            status: status.into(),
            model_content,
            structured_content,
            stable_result_code: status.into(),
        }),
    }
}

fn is_collab_tool(name: &str) -> bool {
    matches!(
        name,
        crate::AGENT_SPAWN_TOOL
            | crate::AGENT_SEND_TOOL
            | crate::AGENT_WAIT_TOOL
            | crate::AGENT_CANCEL_TOOL
            | crate::AGENT_COLLECT_TOOL
    )
}

fn mcp_server_label(name: &str) -> String {
    name.strip_prefix("mcp_")
        .and_then(|value| value.split('_').next())
        .filter(|value| !value.is_empty())
        .unwrap_or("mcp")
        .to_owned()
}

fn dynamic_tool_parts(name: &str) -> Option<(String, String)> {
    let (namespace, tool) = name.split_once('.')?;
    (!namespace.is_empty() && !tool.is_empty()).then(|| (namespace.to_owned(), tool.to_owned()))
}

fn collab_task(value: &serde_json::Value) -> Option<AgentTaskRecord> {
    serde_json::from_value(value.clone()).ok().or_else(|| {
        value
            .get("tasks")
            .and_then(serde_json::Value::as_array)
            .and_then(|tasks| tasks.first())
            .and_then(|task| serde_json::from_value(task.clone()).ok())
    })
}

fn command_text(arguments: &serde_json::Value) -> String {
    let program = arguments
        .get("program")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("command");
    let args = arguments
        .get("args")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .collect::<Vec<_>>()
        .join(" ");
    if args.is_empty() {
        program.into()
    } else {
        format!("{program} {args}")
    }
}

fn bounded_output(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_owned();
    }
    let head = max_chars / 2;
    let tail = max_chars.saturating_sub(head);
    let start = value.chars().take(head).collect::<String>();
    let end = value
        .chars()
        .rev()
        .take(tail)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{start}\n… output truncated …\n{end}")
}

fn persisted_tool_result(result: &crate::ToolResult) -> (String, serde_json::Value) {
    if result
        .structured_content
        .get("redactForPersistence")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        let answer_count = result
            .structured_content
            .get("answerCount")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        return (
            "[user input delivered to the active executor; answers were not persisted]".into(),
            json!({
                "requestId": result.structured_content.get("requestId"),
                "answerCount": answer_count,
                "redacted": true,
            }),
        );
    }
    (
        result.model_content.clone(),
        result.structured_content.clone(),
    )
}

fn plan_steps(markdown: &str) -> Vec<PlanStep> {
    markdown
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            line.trim_start_matches(['-', '*'])
                .trim_start_matches(|character: char| {
                    character.is_ascii_digit() || matches!(character, '.' | ')' | ' ')
                })
                .trim()
        })
        .filter(|line| !line.is_empty())
        .take(128)
        .map(|line| PlanStep {
            id: PlanStepId::random(),
            description: line.chars().take(2_000).collect(),
            status: PlanStepStatus::Pending,
        })
        .collect()
}

fn evidence_artifact(
    run: &RunRecord,
    result: &crate::ToolResult,
    call: Option<&crate::ToolCall>,
) -> Option<ArtifactRecord> {
    let created_at_ms = now_ms();
    match result.tool_name.as_str() {
        "workspace_git_diff" => {
            let diff = result
                .structured_content
                .get("stdout")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(&result.model_content);
            Some(ArtifactRecord {
                id: ArtifactId::random(),
                run_id: Some(run.id.clone()),
                kind: ArtifactKind::DiffEvidence,
                display_name: "Workspace diff".into(),
                content_hash: Some(sha256(diff.as_bytes())),
                metadata: json!({
                    "toolCallId": result.call_id,
                    "status": tool_status(result.status),
                    "exitCode": structured_value(&result.structured_content, "exitCode", "exit_code"),
                    "byteSize": diff.len(),
                    "lineCount": diff.lines().count(),
                    "truncated": result.structured_content.get("truncated").and_then(serde_json::Value::as_bool).unwrap_or(false),
                }),
                created_at_ms,
            })
        }
        "workspace_exec" => {
            let stdout = result
                .structured_content
                .get("stdout")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let stderr = result
                .structured_content
                .get("stderr")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let arguments = call.map(|call| &call.arguments);
            let program = arguments
                .and_then(|value| value.get("program"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("command");
            Some(ArtifactRecord {
                id: ArtifactId::random(),
                run_id: Some(run.id.clone()),
                kind: ArtifactKind::CommandEvidence,
                display_name: program.chars().take(160).collect(),
                content_hash: Some(sha256(
                    serde_json::to_string(&result.structured_content)
                        .unwrap_or_default()
                        .as_bytes(),
                )),
                metadata: json!({
                    "toolCallId": result.call_id,
                    "status": tool_status(result.status),
                    "program": program,
                    "argumentCount": arguments.and_then(|value| value.get("args")).and_then(serde_json::Value::as_array).map(Vec::len).unwrap_or_default(),
                    "cwd": arguments.and_then(|value| value.get("cwd")),
                    "exitCode": structured_value(&result.structured_content, "exitCode", "exit_code"),
                    "stdoutBytes": stdout.len(),
                    "stderrBytes": stderr.len(),
                    "truncated": result.structured_content.get("truncated").and_then(serde_json::Value::as_bool).unwrap_or(false),
                }),
                created_at_ms,
            })
        }
        _ => None,
    }
}

fn structured_value<'a>(
    value: &'a serde_json::Value,
    preferred: &str,
    fallback: &str,
) -> Option<&'a serde_json::Value> {
    value.get(preferred).or_else(|| value.get(fallback))
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn transcript_tool_arguments(call: &crate::ToolCall) -> serde_json::Value {
    match call.name.as_str() {
        "workspace_write_file" => json!({
            "path": call.arguments.get("path"),
            "expectedSha256": call.arguments.get("expectedSha256"),
            "contentBytes": call.arguments.get("content").and_then(serde_json::Value::as_str).map(str::len).unwrap_or_default(),
            "contentSha256": call.arguments.get("content").and_then(serde_json::Value::as_str).map(|content| sha256(content.as_bytes())),
        }),
        "workspace_replace_text" => json!({
            "path": call.arguments.get("path"),
            "expectedSha256": call.arguments.get("expectedSha256"),
            "replaceAll": call.arguments.get("replaceAll"),
            "oldTextBytes": call.arguments.get("oldText").and_then(serde_json::Value::as_str).map(str::len).unwrap_or_default(),
            "oldTextSha256": call.arguments.get("oldText").and_then(serde_json::Value::as_str).map(|content| sha256(content.as_bytes())),
            "newTextBytes": call.arguments.get("newText").and_then(serde_json::Value::as_str).map(str::len).unwrap_or_default(),
            "newTextSha256": call.arguments.get("newText").and_then(serde_json::Value::as_str).map(|content| sha256(content.as_bytes())),
        }),
        "workspace_exec" => json!({
            "program": call.arguments.get("program"),
            "cwd": call.arguments.get("cwd"),
            "timeoutMs": call.arguments.get("timeoutMs"),
            "argumentCount": call.arguments.get("args").and_then(serde_json::Value::as_array).map(Vec::len).unwrap_or_default(),
        }),
        _ => call.arguments.clone(),
    }
}

fn now_ms() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}

#[cfg(test)]
#[path = "run_projection_tests.rs"]
mod tests;
