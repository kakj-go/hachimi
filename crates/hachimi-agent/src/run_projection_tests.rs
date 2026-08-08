use futures_util::stream;
use hachimi_protocol::{
    AgentMessagePhase, ApprovalPolicy, BehaviorMode, CheckoutId, CheckoutKind, CheckoutRecord,
    CheckoutStatus, EntryProfile, ExecutionTarget, LlmSettings, ModelFinishReason, ModelRequest,
    ModelToolCall, PermissionProfile, PlanConfirmationStatus, ProjectId, ProjectRecord,
    ProviderCapabilities, ProviderCapabilityProbe, ProviderCapabilityProbeSource, RunBudget,
    RunConfiguration, RunDriverKind, RunEventPayload, RunId, RunOrigin, RunPurpose,
    SessionContextBinding, SessionId, SessionRecord, ToolCallId, ToolDescriptor, ToolEffect,
    WorkloadKind,
};

use super::*;
use crate::{
    ModelEventStream, ModelRuntime, ToolExecutor, ToolFuture, ToolInvocation, ToolRegistry,
    ToolResult, ToolRuntime,
};

struct FinalModel;

struct ChunkedPlanModel {
    chunks: &'static [&'static str],
}

struct SplitPlanModel {
    output: &'static str,
    split: usize,
}

impl ModelRuntime for FinalModel {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            tool_calls: true,
            parallel_tool_calls: true,
            streaming_usage: true,
            realtime: false,
            text_input: true,
            strict_json_schema: true,
            output_schema: true,
            ..ProviderCapabilities::default()
        }
    }

    fn capability_probe(&self) -> Option<ProviderCapabilityProbe> {
        Some(ProviderCapabilityProbe {
            strict_json_schema: true,
            output_schema: true,
            source: ProviderCapabilityProbeSource::Probe,
            stable_error_code: None,
        })
    }

    fn stream(&self, _request: ModelRequest, _cancellation: CancellationToken) -> ModelEventStream {
        Box::pin(stream::iter([
            Ok(ModelEvent::TextDelta {
                delta: "done".into(),
            }),
            Ok(ModelEvent::Usage {
                usage: hachimi_protocol::TokenUsage {
                    input_tokens: 7,
                    output_tokens: 3,
                },
            }),
            Ok(ModelEvent::Completed {
                finish_reason: ModelFinishReason::Stop,
            }),
        ]))
    }
}

impl ModelRuntime for ChunkedPlanModel {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            tool_calls: true,
            parallel_tool_calls: true,
            streaming_usage: true,
            text_input: true,
            ..ProviderCapabilities::default()
        }
    }

    fn stream(&self, _request: ModelRequest, _cancellation: CancellationToken) -> ModelEventStream {
        let events = self
            .chunks
            .iter()
            .map(|chunk| {
                Ok(ModelEvent::TextDelta {
                    delta: (*chunk).to_owned(),
                })
            })
            .chain(std::iter::once(Ok(ModelEvent::Completed {
                finish_reason: ModelFinishReason::Stop,
            })))
            .collect::<Vec<_>>();
        Box::pin(stream::iter(events))
    }
}

impl ModelRuntime for SplitPlanModel {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            tool_calls: true,
            parallel_tool_calls: true,
            streaming_usage: true,
            text_input: true,
            ..ProviderCapabilities::default()
        }
    }

    fn stream(&self, _request: ModelRequest, _cancellation: CancellationToken) -> ModelEventStream {
        let (before, after) = self.output.split_at(self.split);
        Box::pin(stream::iter([
            Ok(ModelEvent::TextDelta {
                delta: before.to_owned(),
            }),
            Ok(ModelEvent::TextDelta {
                delta: after.to_owned(),
            }),
            Ok(ModelEvent::Completed {
                finish_reason: ModelFinishReason::Stop,
            }),
        ]))
    }
}

struct PendingModel;

struct NeedsAttentionModel;

struct NeedsAttentionTool;

impl ModelRuntime for PendingModel {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            tool_calls: true,
            parallel_tool_calls: true,
            streaming_usage: true,
            realtime: false,
            text_input: true,
            ..ProviderCapabilities::default()
        }
    }

    fn stream(&self, _request: ModelRequest, _cancellation: CancellationToken) -> ModelEventStream {
        Box::pin(stream::pending())
    }
}

impl ModelRuntime for NeedsAttentionModel {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            tool_calls: true,
            text_input: true,
            ..ProviderCapabilities::default()
        }
    }

    fn stream(&self, _request: ModelRequest, _cancellation: CancellationToken) -> ModelEventStream {
        Box::pin(stream::iter([
            Ok(ModelEvent::ToolCallCompleted {
                call: ModelToolCall {
                    id: ToolCallId::from("authority-call"),
                    name: "authority_probe".into(),
                    arguments: json!({}),
                },
            }),
            Ok(ModelEvent::Completed {
                finish_reason: ModelFinishReason::ToolCalls,
            }),
        ]))
    }
}

impl ToolExecutor for NeedsAttentionTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "authority_probe".into(),
            description: "returns an unattended authority blocker".into(),
            input_schema: json!({ "type": "object" }),
            effect: ToolEffect::ExternalSideEffect,
            parallel_safe: false,
            required_scopes: Vec::new(),
        }
    }

    fn execute(&self, invocation: ToolInvocation) -> ToolFuture {
        Box::pin(std::future::ready(Ok(ToolResult::needs_attention(
            &invocation.call,
            "authority_test",
            "background authority requires attention",
        ))))
    }
}

async fn seeded_run(id: &str, mode: BehaviorMode) -> (AgentStore, RunRecord) {
    let store = AgentStore::connect_in_memory().await.expect("store");
    let timestamp = now_ms();
    let project = ProjectRecord {
        id: ProjectId::from("project"),
        display_name: "Demo".into(),
        root_path: "C:\\demo".into(),
        git_root: Some("C:\\demo".into()),
        trusted: true,
        created_at_ms: timestamp,
        updated_at_ms: timestamp,
    };
    store.create_project(&project).await.expect("project");
    let checkout = CheckoutRecord {
        id: CheckoutId::from("checkout"),
        project_id: project.id.clone(),
        kind: CheckoutKind::Local,
        path: project.root_path.clone(),
        base_revision: Some("main".into()),
        head_revision: None,
        status: CheckoutStatus::Ready,
        pinned: false,
        created_at_ms: timestamp,
        updated_at_ms: timestamp,
    };
    store.create_checkout(&checkout).await.expect("checkout");
    let session = SessionRecord {
        id: SessionId::from("session"),
        context: SessionContextBinding::Project {
            project_id: project.id.clone(),
            checkout_id: checkout.id,
        },
        entry_profile: EntryProfile::Workbench,
        title: "Task".into(),
        archived: false,
        pinned: false,
        parent_session_id: None,
        source_run_id: None,
        created_at_ms: timestamp,
        updated_at_ms: timestamp,
    };
    store.create_session(&session).await.expect("session");
    let run = RunRecord {
        id: RunId::from(id),
        session_id: session.id,
        status: RunStatus::Queued,
        purpose: RunPurpose::Task,
        origin: RunOrigin::Manual,
        generation: 1,
        configuration: RunConfiguration {
            model_snapshot: LlmSettings::default(),
            driver: RunDriverKind::ToolLoop,
            entry_profile: EntryProfile::Workbench,
            workload_override: Some(WorkloadKind::Coding),
            behavior_mode: mode,
            execution_target: Some(ExecutionTarget::Local {
                project_id: project.id,
            }),
            approval_policy: ApprovalPolicy::OnlyWhenNeeded,
            permission_profile: PermissionProfile::Writable,
            budget: RunBudget::default(),
            accepted_plan_id: None,
            accepted_plan_revision: None,
        },
        requested_capabilities: ProviderCapabilities {
            tool_calls: true,
            parallel_tool_calls: true,
            text_input: true,
            streaming_usage: true,
            ..ProviderCapabilities::default()
        },
        negotiated_capabilities: ProviderCapabilities::default(),
        provider_capability_probe: None,
        capability_degradations: Vec::new(),
        failure_code: None,
        created_at_ms: timestamp,
        updated_at_ms: timestamp,
    };
    store
        .create_run_idempotent("user", id, &run)
        .await
        .expect("run");
    (store, run)
}

fn persisted_loop(store: &AgentStore, model: Arc<dyn ModelRuntime>) -> PersistedToolLoop {
    let tools = Arc::new(ToolRuntime::new(Arc::new(ToolRegistry::new())));
    PersistedToolLoop::new(store.clone(), Arc::new(ToolLoopDriver::new(model, tools)))
}

#[test]
fn user_input_tool_results_are_redacted_before_transcript_projection() {
    let secret = "secret-that-must-not-be-persisted";
    let result = crate::ToolResult {
        call_id: hachimi_protocol::ToolCallId::from("input-call"),
        tool_name: crate::REQUEST_USER_INPUT_TOOL.into(),
        status: ToolResultStatus::Succeeded,
        model_content: format!("{{\"answers\":[{{\"answer\":\"{secret}\"}}]}}"),
        structured_content: json!({
            "requestId": "request-1",
            "answerCount": 1,
            "containsSecret": true,
            "redactForPersistence": true,
        }),
        model_images: Vec::new(),
    };
    let (content, structured) = persisted_tool_result(&result);
    assert!(!content.contains(secret));
    assert!(!structured.to_string().contains(secret));
    assert_eq!(structured["redacted"], true);
}

#[test]
fn ephemeral_model_images_are_never_projected_to_persistence() {
    let marker = "sensitive-image-base64";
    let result = crate::ToolResult {
        call_id: hachimi_protocol::ToolCallId::from("computer-call"),
        tool_name: "computer_observe".into(),
        status: ToolResultStatus::Succeeded,
        model_content: "Computer frame attached ephemerally".into(),
        structured_content: json!({ "frameId": "frame-1" }),
        model_images: vec![hachimi_protocol::ModelInputImage {
            media_type: "image/png".into(),
            data_base64: marker.into(),
            source_label: "computer frame frame-1".into(),
        }],
    };
    let (content, structured) = persisted_tool_result(&result);
    assert!(!content.contains(marker));
    assert!(!structured.to_string().contains(marker));
    assert_eq!(structured["frameId"], "frame-1");
}

#[tokio::test]
async fn persists_successful_run_events_and_assistant_transcript() {
    let (store, run) = seeded_run("run-success", BehaviorMode::Default).await;
    let outcome = persisted_loop(&store, Arc::new(FinalModel))
        .execute(
            run.clone(),
            vec![ModelMessage::user("finish")],
            CancellationToken::new(),
        )
        .await
        .expect("execute");
    assert_eq!(outcome.final_text, "done");
    let persisted = store.get_run(&run.id).await.expect("get").unwrap();
    assert_eq!(persisted.status, RunStatus::Succeeded);
    assert_eq!(
        persisted
            .provider_capability_probe
            .as_ref()
            .map(|probe| probe.source),
        Some(ProviderCapabilityProbeSource::Probe)
    );
    let transcript = store
        .list_transcript(&run.session_id)
        .await
        .expect("transcript");
    assert!(transcript.iter().any(|item| matches!(
        &item.payload,
        ItemPayload::Assistant { text, phase }
            if text == "done" && *phase == AgentMessagePhase::FinalAnswer
    )));
    let events = store.list_events(&run.session_id, 0).await.expect("events");
    let assistant = transcript
        .iter()
        .find(|item| {
            matches!(
                &item.payload,
                ItemPayload::Assistant { text, phase }
                    if text == "done" && *phase == AgentMessagePhase::FinalAnswer
            )
        })
        .expect("assistant item");
    assert!(events.iter().any(|event| matches!(
        &event.payload,
        RunEventPayload::ItemStarted { item }
            if item.id == assistant.id && item.kind == TranscriptItemKind::Assistant
    )));
    assert!(events.iter().any(|event| matches!(
        &event.payload,
        RunEventPayload::ItemCompleted { item }
            if item.id == assistant.id
                && item.status == ItemStatus::Completed
                && matches!(
                    &item.payload,
                    ItemPayload::Assistant { text, phase }
                        if text == "done" && *phase == AgentMessagePhase::FinalAnswer
                )
    )));
    assert!(
        events
            .iter()
            .all(|event| !matches!(event.payload, RunEventPayload::ItemDelta { .. }))
    );
    assert!(
        store
            .list_active_event_replay(&run.session_id, 0)
            .is_empty()
    );
    assert!(events.iter().any(|event| matches!(
        &event.payload,
        RunEventPayload::Generic { event, data }
            if event == "run.status_changed" && data["to"] == "succeeded"
    )));
    let usage = store
        .get_run_usage_snapshot(&run.id)
        .await
        .expect("usage")
        .expect("usage snapshot");
    assert_eq!(usage.billed_usage.input_tokens, 7);
    assert_eq!(usage.billed_usage.output_tokens, 3);
    assert!(
        events
            .iter()
            .all(|event| !event.event_name().contains("usage")),
        "usage snapshots are connection state, not replayable history"
    );
}

#[tokio::test]
async fn cancellation_is_persisted_through_cancelling_to_cancelled() {
    let (store, run) = seeded_run("run-cancel", BehaviorMode::Default).await;
    let cancellation = CancellationToken::new();
    let execution = tokio::spawn({
        let loop_driver = persisted_loop(&store, Arc::new(PendingModel));
        let run = run.clone();
        let cancellation = cancellation.clone();
        async move {
            loop_driver
                .execute(run, vec![ModelMessage::user("wait")], cancellation)
                .await
        }
    });
    for _ in 0..100 {
        if store
            .get_run(&run.id)
            .await
            .expect("get")
            .is_some_and(|record| record.status == RunStatus::Running)
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    cancellation.cancel();
    let error = execution.await.expect("join").expect_err("cancelled");
    assert!(matches!(
        error,
        PersistedRunError::Runtime(ModelRuntimeError::Cancelled)
    ));
    assert_eq!(
        store.get_run(&run.id).await.expect("get").unwrap().status,
        RunStatus::Cancelled
    );
    let transitions = store
        .list_events(&run.session_id, 0)
        .await
        .expect("events")
        .into_iter()
        .filter_map(|event| match event.payload {
            RunEventPayload::Generic { event, data } if event == "run.status_changed" => {
                Some(data["to"].clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(transitions.contains(&json!("cancelling")));
    assert!(transitions.contains(&json!("cancelled")));
}

#[tokio::test]
async fn authority_attention_is_persisted_as_an_event_and_stable_failure_code() {
    let (store, run) = seeded_run("run-authority-attention", BehaviorMode::Default).await;
    let mut registry = ToolRegistry::new();
    registry
        .register(Arc::new(NeedsAttentionTool))
        .expect("tool");
    let persisted = PersistedToolLoop::new(
        store.clone(),
        Arc::new(ToolLoopDriver::new(
            Arc::new(NeedsAttentionModel),
            Arc::new(ToolRuntime::new(Arc::new(registry))),
        )),
    );
    let error = persisted
        .execute(
            run.clone(),
            vec![ModelMessage::user("trigger authority blocker")],
            CancellationToken::new(),
        )
        .await
        .expect_err("authority blocker");
    assert!(matches!(
        error,
        PersistedRunError::Runtime(ModelRuntimeError::NeedsAttention(code))
            if code == "authority_test"
    ));
    let persisted_run = store
        .get_run(&run.id)
        .await
        .expect("run")
        .expect("persisted run");
    assert_eq!(persisted_run.status, RunStatus::Failed);
    assert_eq!(
        persisted_run.failure_code.as_deref(),
        Some("authority_needs_attention")
    );
    let events = store.list_events(&run.session_id, 0).await.expect("events");
    assert!(events.iter().any(|event| matches!(
        &event.payload,
        RunEventPayload::Generic { event, data }
            if event == AUTHORITY_NEEDS_ATTENTION_EVENT
                && data["code"] == "authority_test"
    )));
    let transcript = store
        .list_transcript(&run.session_id)
        .await
        .expect("transcript");
    assert!(transcript.iter().any(|item| matches!(
        &item.payload,
        ItemPayload::SystemContext { code, .. }
            if code == "agent_authority_needs_attention"
    )));
}

#[tokio::test]
async fn plan_run_persists_versioned_plan_document_and_plan_transcript() {
    let (store, run) = seeded_run("run-plan", BehaviorMode::Plan).await;
    persisted_loop(
        &store,
        Arc::new(ChunkedPlanModel {
            chunks: &[
                "I checked the request. ",
                "<pro",
                "posed_plan># Implement the change\n\n1. Update the code",
                "</proposed_",
                "plan> Ready for review.",
            ],
        }),
    )
    .execute(
        run.clone(),
        vec![ModelMessage::user("Plan the change")],
        CancellationToken::new(),
    )
    .await
    .expect("execute");
    let plans = store
        .list_plan_documents(&run.session_id)
        .await
        .expect("plans");
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].revision, 1);
    assert_eq!(plans[0].goal, "Plan the change");
    assert_eq!(plans[0].title, "Implement the change");
    assert_eq!(
        plans[0].content_markdown,
        "# Implement the change\n\n1. Update the code"
    );
    let confirmations = store
        .list_plan_confirmations(&run.session_id)
        .await
        .expect("confirmations");
    assert_eq!(confirmations.len(), 1);
    assert_eq!(confirmations[0].status, PlanConfirmationStatus::Pending);
    let transcript = store
        .list_transcript(&run.session_id)
        .await
        .expect("transcript");
    assert!(
        transcript
            .iter()
            .any(|item| item.kind == TranscriptItemKind::Plan)
    );
    let visible_text = transcript
        .iter()
        .filter_map(|item| match &item.payload {
            ItemPayload::Assistant { text, .. } | ItemPayload::Plan { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert!(visible_text.contains("I checked the request."));
    assert!(visible_text.contains("Ready for review."));
    assert!(!visible_text.contains("<proposed_plan>"));
    assert!(!visible_text.contains("</proposed_plan>"));
}

#[tokio::test]
async fn plan_tags_can_split_at_every_byte_boundary_without_leaking_markup() {
    const OUTPUT: &str =
        "Checked. <proposed_plan># Boundary-safe plan\n\nDo the work.</proposed_plan> Done.";
    for split in 1..OUTPUT.len() {
        let run_id = format!("run-plan-split-{split}");
        let (store, run) = seeded_run(&run_id, BehaviorMode::Plan).await;
        persisted_loop(
            &store,
            Arc::new(SplitPlanModel {
                output: OUTPUT,
                split,
            }),
        )
        .execute(
            run.clone(),
            vec![ModelMessage::user("Plan the change")],
            CancellationToken::new(),
        )
        .await
        .expect("execute");
        let plans = store
            .list_plan_documents(&run.session_id)
            .await
            .expect("plans");
        assert_eq!(plans.len(), 1, "split={split}");
        assert_eq!(plans[0].title, "Boundary-safe plan", "split={split}");
        let visible = store
            .list_transcript(&run.session_id)
            .await
            .expect("transcript")
            .into_iter()
            .filter_map(|item| match item.payload {
                ItemPayload::Assistant { text, .. } | ItemPayload::Plan { text } => Some(text),
                _ => None,
            })
            .collect::<String>();
        assert!(!visible.contains("proposed_plan"), "split={split}");
    }
}

#[tokio::test]
async fn incomplete_plan_markup_becomes_commentary_without_a_plan_item_or_gate() {
    for (run_id, chunks) in [
        ("run-plan-partial-open", &["Before <proposed_"][..]),
        (
            "run-plan-partial-close",
            &["Before <proposed_plan># Draft\n\nKeep this</proposed_"][..],
        ),
        (
            "run-plan-missing-close",
            &["Before <proposed_plan># Draft\n\nKeep this"][..],
        ),
    ] {
        let (store, run) = seeded_run(run_id, BehaviorMode::Plan).await;
        persisted_loop(&store, Arc::new(ChunkedPlanModel { chunks }))
            .execute(
                run.clone(),
                vec![ModelMessage::user("Plan the change")],
                CancellationToken::new(),
            )
            .await
            .expect("execute");
        assert!(
            store
                .list_plan_documents(&run.session_id)
                .await
                .expect("plans")
                .is_empty(),
            "run={run_id}"
        );
        let transcript = store
            .list_transcript(&run.session_id)
            .await
            .expect("transcript");
        assert!(
            transcript
                .iter()
                .all(|item| item.kind != TranscriptItemKind::Plan),
            "run={run_id}"
        );
        let visible = transcript
            .iter()
            .filter_map(|item| match &item.payload {
                ItemPayload::Assistant { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert!(!visible.contains("proposed_"), "run={run_id}");
        if run_id != "run-plan-partial-open" {
            assert!(visible.contains("# Draft"), "run={run_id}");
        }
    }
}

#[tokio::test]
async fn empty_plan_blocks_do_not_create_plan_items_or_confirmation_gates() {
    for (run_id, block) in [
        ("run-plan-empty", "<proposed_plan></proposed_plan>"),
        (
            "run-plan-whitespace",
            "<proposed_plan> \n\t </proposed_plan>",
        ),
    ] {
        let (store, run) = seeded_run(run_id, BehaviorMode::Plan).await;
        persisted_loop(
            &store,
            Arc::new(SplitPlanModel {
                output: block,
                split: PROPOSED_PLAN_OPEN.len(),
            }),
        )
        .execute(
            run.clone(),
            vec![ModelMessage::user("Plan the change")],
            CancellationToken::new(),
        )
        .await
        .expect("execute");
        assert!(
            store
                .list_plan_documents(&run.session_id)
                .await
                .expect("plans")
                .is_empty()
        );
        assert!(
            store
                .list_transcript(&run.session_id)
                .await
                .expect("transcript")
                .iter()
                .all(|item| item.kind != TranscriptItemKind::Plan)
        );
    }
}

#[tokio::test]
async fn plan_projection_preserves_commentary_tool_commentary_plan_order() {
    let (store, run) = seeded_run("run-plan-order", BehaviorMode::Plan).await;
    let call = crate::ToolCall {
        id: ToolCallId::from("call-read"),
        name: "workspace_read_file".into(),
        arguments: json!({ "path": "README.md" }),
        step_revision: 1,
        tool_plan_hash: "plan-hash".into(),
        registry_revision: "registry".into(),
    };
    let result = ToolResult::succeeded(&call, "read", json!({ "text": "content" }));
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    for event in [
        LoopEvent::Model(ModelEvent::AgentMessageStarted {
            message_id: "before".into(),
            phase: AgentMessagePhase::Commentary,
        }),
        LoopEvent::Model(ModelEvent::AgentMessageDelta {
            message_id: "before".into(),
            delta: "Checking files.".into(),
        }),
        LoopEvent::Model(ModelEvent::AgentMessageCompleted {
            message_id: "before".into(),
        }),
        LoopEvent::ToolStarted(call),
        LoopEvent::ToolCompleted(result),
        LoopEvent::Model(ModelEvent::AgentMessageStarted {
            message_id: "after".into(),
            phase: AgentMessagePhase::Commentary,
        }),
        LoopEvent::Model(ModelEvent::AgentMessageDelta {
            message_id: "after".into(),
            delta: "The file confirms the approach.".into(),
        }),
        LoopEvent::Model(ModelEvent::AgentMessageCompleted {
            message_id: "after".into(),
        }),
        LoopEvent::Model(ModelEvent::AgentMessageStarted {
            message_id: "plan".into(),
            phase: AgentMessagePhase::FinalAnswer,
        }),
        LoopEvent::Model(ModelEvent::AgentMessageDelta {
            message_id: "plan".into(),
            delta: "<proposed_plan># Ordered plan\n\nImplement it.</proposed_plan>".into(),
        }),
        LoopEvent::Model(ModelEvent::AgentMessageCompleted {
            message_id: "plan".into(),
        }),
    ] {
        sender.send(event).expect("projection event");
    }
    drop(sender);
    project_loop_events(store.clone(), run.clone(), receiver)
        .await
        .expect("projection");
    let kinds = store
        .list_transcript(&run.session_id)
        .await
        .expect("transcript")
        .into_iter()
        .map(|item| item.kind)
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            TranscriptItemKind::Assistant,
            TranscriptItemKind::ToolExecution,
            TranscriptItemKind::Assistant,
            TranscriptItemKind::Plan,
        ]
    );
}

#[tokio::test]
async fn plan_run_without_complete_plan_block_keeps_commentary_without_a_gate() {
    let (store, run) = seeded_run("run-plan-no-block", BehaviorMode::Plan).await;
    persisted_loop(&store, Arc::new(FinalModel))
        .execute(
            run.clone(),
            vec![ModelMessage::user("Plan the change")],
            CancellationToken::new(),
        )
        .await
        .expect("execute");
    assert!(
        store
            .list_plan_documents(&run.session_id)
            .await
            .expect("plans")
            .is_empty()
    );
    let transcript = store
        .list_transcript(&run.session_id)
        .await
        .expect("transcript");
    assert!(transcript.iter().any(|item| matches!(
        &item.payload,
        ItemPayload::Assistant { text, phase }
            if text == "done" && *phase == AgentMessagePhase::Commentary
    )));
}

#[tokio::test]
async fn plan_run_uses_only_the_first_complete_plan_block() {
    let (store, run) = seeded_run("run-plan-multiple", BehaviorMode::Plan).await;
    persisted_loop(
        &store,
        Arc::new(ChunkedPlanModel {
            chunks: &[
                "<proposed_plan># First\n\nDo first.</proposed_plan>",
                "<proposed_plan># Second\n\nDo second.</proposed_plan>",
            ],
        }),
    )
    .execute(
        run.clone(),
        vec![ModelMessage::user("Plan the change")],
        CancellationToken::new(),
    )
    .await
    .expect("execute");
    let plans = store
        .list_plan_documents(&run.session_id)
        .await
        .expect("plans");
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].title, "First");
    assert!(!plans[0].content_markdown.contains("Second"));
}

#[tokio::test]
async fn command_and_diff_results_become_bounded_evidence_artifacts() {
    let (_store, run) = seeded_run("run-evidence", BehaviorMode::Default).await;
    let command = crate::ToolCall {
        id: hachimi_protocol::ToolCallId::from("call-exec"),
        name: "workspace_exec".into(),
        arguments: json!({
            "program": "cargo",
            "args": ["test"],
            "cwd": ""
        }),
        step_revision: 1,
        tool_plan_hash: "fixture-plan".into(),
        registry_revision: "fixture-registry".into(),
    };
    let command_result = crate::ToolResult::succeeded(
        &command,
        "ok",
        json!({
            "type": "process",
            "exitCode": 0,
            "stdout": "tests passed",
            "stderr": "",
            "truncated": false
        }),
    );
    let artifact = evidence_artifact(&run, &command_result, Some(&command)).expect("artifact");
    assert_eq!(artifact.kind, ArtifactKind::CommandEvidence);
    assert_eq!(artifact.display_name, "cargo");
    assert_eq!(artifact.metadata["exitCode"], 0);
    assert!(artifact.metadata.get("stdout").is_none());

    let diff = crate::ToolCall {
        id: hachimi_protocol::ToolCallId::from("call-diff"),
        name: "workspace_git_diff".into(),
        arguments: json!({}),
        step_revision: 1,
        tool_plan_hash: "fixture-plan".into(),
        registry_revision: "fixture-registry".into(),
    };
    let diff_result = crate::ToolResult::succeeded(
        &diff,
        "diff",
        json!({ "stdout": "+changed\n", "exitCode": 0, "truncated": false }),
    );
    let artifact = evidence_artifact(&run, &diff_result, Some(&diff)).expect("artifact");
    assert_eq!(artifact.kind, ArtifactKind::DiffEvidence);
    assert_eq!(artifact.metadata["lineCount"], 1);
    assert!(artifact.content_hash.is_some());

    let write = crate::ToolCall {
        id: hachimi_protocol::ToolCallId::from("call-write"),
        name: "workspace_write_file".into(),
        arguments: json!({
            "path": "secret.txt",
            "content": "sensitive contents",
            "expectedSha256": null
        }),
        step_revision: 1,
        tool_plan_hash: "fixture-plan".into(),
        registry_revision: "fixture-registry".into(),
    };
    let sanitized = transcript_tool_arguments(&write);
    assert_eq!(sanitized["path"], "secret.txt");
    assert_eq!(sanitized["contentBytes"], 18);
    assert!(!sanitized.to_string().contains("sensitive contents"));
}

#[test]
fn continuity_artifact_reference_keeps_revision_without_binary_metadata() {
    let artifact = ArtifactRecord {
        id: ArtifactId::from("artifact-office-continuity"),
        run_id: Some(RunId::from("run-office-continuity")),
        kind: ArtifactKind::CommandEvidence,
        display_name: "report.docx".into(),
        content_hash: Some("sha256:abcd".into()),
        metadata: json!({
            "mimeType": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "revision": "rev-4",
            "changedParts": ["word/document.xml"],
            "dataBase64": "QUJD".repeat(4_000),
        }),
        created_at_ms: 1,
    };
    let reference = artifact_continuity(&artifact);
    let rendered = serde_json::to_string(&reference).expect("serialize continuity artifact");
    assert!(rendered.contains("artifact-office-continuity"));
    assert!(rendered.contains("sha256:abcd"));
    assert!(rendered.contains("mimeType"));
    assert!(rendered.contains("rev-4"));
    assert!(!rendered.contains("dataBase64"));
    assert!(!rendered.contains("QUJDQUJD"));
}
