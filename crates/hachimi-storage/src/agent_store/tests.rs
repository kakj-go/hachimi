use hachimi_protocol::{
    ApprovalGrantScope, ApprovalPolicy, BehaviorMode, CapabilityGrantSet, CheckoutId, CheckoutKind,
    CheckoutStatus, ClientId, ComputerGrant, DeliveryPolicy, DeliveryStatus, EntryProfile,
    ExecutionTarget, FileSystemAccess, FileSystemGrant, ItemId, LlmSettings, MisfirePolicy,
    NetworkGrant, PermissionGrantScope, PermissionProfile, ProcessGrant, ProcessSessionId,
    ProcessSessionRecord, ProcessStatus, ProjectId, ProviderCapabilities, ReviewDelivery,
    ReviewFinding, ReviewFindingId, ReviewFindingStatus, ReviewId, ReviewOutput, ReviewRecord,
    ReviewSeverity, ReviewTarget, RunBudget, RunConfiguration, RunDriverKind, RunOrigin,
    RunPurpose, SandboxCapabilityReport, SandboxReadiness, ScheduleContextTemplate,
    ScheduleDefinition, ScheduleHealth, ScheduleId, SchedulePermissionConfig, ScheduleSpec,
    SessionContextBinding, SideEffectExecutionId, SideEffectExecutionRecord,
    SideEffectExecutionStatus, SkillId, SkillRecord, TaskRunId, TaskRunRecord, TaskRunStatus,
    TaskRunTrigger, UserInputQuestion, UserInputRequestId, UserInputRequestRecord, UserInputStatus,
    WorkloadKind,
};

use super::*;

async fn seeded_store() -> (AgentStore, SessionRecord) {
    let store = AgentStore::connect_in_memory().await.expect("store");
    let now = now_ms();
    let project = ProjectRecord {
        id: ProjectId::from("project-1"),
        display_name: "Demo".into(),
        root_path: "C:\\demo".into(),
        git_root: Some("C:\\demo".into()),
        trusted: true,
        created_at_ms: now,
        updated_at_ms: now,
    };
    store.create_project(&project).await.expect("project");
    let checkout = CheckoutRecord {
        id: CheckoutId::from("checkout-1"),
        project_id: project.id.clone(),
        kind: CheckoutKind::Local,
        path: project.root_path.clone(),
        base_revision: Some("main".into()),
        head_revision: None,
        status: CheckoutStatus::Ready,
        pinned: false,
        created_at_ms: now,
        updated_at_ms: now,
    };
    store.create_checkout(&checkout).await.expect("checkout");
    let session = SessionRecord {
        id: SessionId::from("session-1"),
        context: SessionContextBinding::Project {
            project_id: project.id,
            checkout_id: checkout.id,
        },
        entry_profile: EntryProfile::Workbench,
        title: "Task".into(),
        archived: false,
        pinned: false,
        parent_session_id: None,
        source_run_id: None,
        created_at_ms: now,
        updated_at_ms: now,
    };
    store.create_session(&session).await.expect("session");
    (store, session)
}

#[tokio::test]
async fn fresh_database_uses_only_the_three_kernel_baselines() {
    let store = AgentStore::connect_in_memory().await.expect("fresh store");
    let migration_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(store.pool())
        .await
        .expect("migration count");
    assert_eq!(migration_count, 3);

    let payload_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('transcript_items') WHERE name = 'payload_json'",
    )
    .fetch_one(store.pool())
    .await
    .expect("typed payload column");
    let legacy_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('transcript_items') WHERE name IN ('content_json', 'item_type_v2', 'payload_version', 'typed_payload')",
    )
    .fetch_one(store.pool())
    .await
    .expect("legacy columns");
    assert_eq!(payload_columns, 1);
    assert_eq!(legacy_columns, 0);
}

#[tokio::test]
async fn skill_extension_fields_round_trip_without_bind_order_shift() {
    let store = AgentStore::connect_in_memory().await.expect("store");
    let stored = StoredSkillRecord {
        stable_path: "C:\\skills\\release-helper".into(),
        record: SkillRecord {
            id: SkillId::from("skill-roundtrip"),
            scope: hachimi_protocol::SkillScope::Repo,
            namespace: Some("example".into()),
            name: "release-helper".into(),
            qualified_name: "example:release-helper".into(),
            description: "Prepare a release".into(),
            interface: Some(hachimi_protocol::SkillInterface {
                display_name: Some("Release Helper".into()),
                short_description: Some("Prepare releases".into()),
                brand_color: Some("#7A6FF0".into()),
                ..hachimi_protocol::SkillInterface::default()
            }),
            policy: hachimi_protocol::SkillPolicy {
                allow_implicit_invocation: Some(false),
                workload: None,
            },
            dependencies: vec![hachimi_protocol::SkillToolDependency {
                kind: "mcp".into(),
                value: "calendar".into(),
                description: None,
                transport: None,
                command: None,
                url: None,
            }],
            editable: false,
            enabled: true,
            content_hash: "entry-hash".into(),
            tree_revision: "tree-hash".into(),
            diagnostics: Vec::new(),
            updated_at_ms: 42,
        },
    };

    let saved = store.upsert_skill(&stored).await.expect("save Skill");

    assert_eq!(saved.record.description, "Prepare a release");
    assert_eq!(saved.record.content_hash, "entry-hash");
    assert_eq!(saved.record.tree_revision, "tree-hash");
    assert_eq!(saved.record.scope, hachimi_protocol::SkillScope::Repo);
    assert_eq!(saved.record.qualified_name, "example:release-helper");
    assert_eq!(
        saved
            .record
            .interface
            .as_ref()
            .and_then(|interface| interface.display_name.as_deref()),
        Some("Release Helper")
    );
    assert!(!saved.record.policy.allows_implicit_invocation());
    assert_eq!(saved.record.dependencies[0].value, "calendar");
    assert_eq!(saved.record.updated_at_ms, 42);
}

#[tokio::test]
async fn process_session_metadata_round_trips_without_output_payloads() {
    let (store, session) = seeded_store().await;
    let now = now_ms();
    let record = ProcessSessionRecord {
        id: ProcessSessionId::from("process-roundtrip"),
        session_id: session.id.clone(),
        run_id: None,
        checkout_id: session.context.checkout_id().expect("checkout").clone(),
        run_generation: None,
        owner_client_id: ClientId("window:workbench".into()),
        command_summary: "powershell -NoProfile".into(),
        interactive: true,
        status: ProcessStatus::Running,
        exit_code: None,
        output_limit_bytes: 1024,
        created_at_ms: now,
        updated_at_ms: now,
        reconnect_expires_at_ms: None,
    };
    store
        .upsert_process_session(&record)
        .await
        .expect("persist process");
    let loaded = store
        .get_process_session(&record.id)
        .await
        .expect("load process")
        .expect("process");
    assert_eq!(loaded, record);
    assert_eq!(
        store
            .list_process_sessions(Some(&session.id), None, false)
            .await
            .expect("list process"),
        vec![record]
    );
}

#[tokio::test]
async fn review_output_and_findings_are_completed_once_and_remain_updateable() {
    let (store, session) = seeded_store().await;
    let mut review_run = run(&session, "review-run");
    review_run.purpose = RunPurpose::Review;
    review_run.configuration.permission_profile = PermissionProfile::ReadOnly;
    review_run.configuration.approval_policy = ApprovalPolicy::NeverPrompt;
    store
        .create_run_idempotent("user", "review-run", &review_run)
        .await
        .expect("review Run");
    let review = store
        .create_review_record(&ReviewRecord {
            id: ReviewId::from("review-1"),
            session_id: session.id.clone(),
            run_id: review_run.id.clone(),
            target: ReviewTarget::UncommittedChanges,
            delivery: ReviewDelivery::Inline,
            created_at_ms: now_ms(),
        })
        .await
        .expect("Review record");
    let finding = ReviewFinding {
        id: ReviewFindingId::from("finding-1"),
        review_id: review.id.clone(),
        severity: ReviewSeverity::Error,
        file: Some("src/lib.rs".into()),
        line: Some(12),
        message: "Unchecked optional value".into(),
        evidence: "The None branch is reachable.".into(),
        status: ReviewFindingStatus::Open,
    };
    let output = ReviewOutput {
        findings: Vec::new(),
        overall_correctness: "incorrect".into(),
        overall_explanation: "One actionable defect.".into(),
        overall_confidence_score: 0.9,
    };

    let first = store
        .complete_review(
            &review,
            &output,
            std::slice::from_ref(&finding),
            false,
            now_ms(),
        )
        .await
        .expect("complete Review");
    let second = store
        .complete_review(
            &review,
            &output,
            std::slice::from_ref(&finding),
            false,
            now_ms(),
        )
        .await
        .expect("idempotent completion");
    assert_eq!(first.findings, second.findings);
    assert_eq!(first.findings.len(), 1);
    assert_eq!(first.summary.as_deref(), Some("One actionable defect."));
    let updated = store
        .update_review_finding_status(&review.id, &finding.id, ReviewFindingStatus::Acknowledged)
        .await
        .expect("update finding");
    assert_eq!(updated.status, ReviewFindingStatus::Acknowledged);
    assert_eq!(
        store.list_reviews(&session.id).await.expect("reviews"),
        vec![review]
    );
}

fn run(session: &SessionRecord, id: &str) -> RunRecord {
    let now = now_ms();
    RunRecord {
        id: RunId::from(id),
        session_id: session.id.clone(),
        status: RunStatus::Queued,
        purpose: RunPurpose::Task,
        origin: RunOrigin::Interactive,
        generation: 1,
        configuration: RunConfiguration {
            model_snapshot: LlmSettings::default(),
            driver: RunDriverKind::ToolLoop,
            entry_profile: EntryProfile::Workbench,
            workload_override: Some(WorkloadKind::Coding),
            behavior_mode: BehaviorMode::Default,
            execution_target: Some(ExecutionTarget::Local {
                project_id: session.context.project_id().expect("project").clone(),
            }),
            approval_policy: ApprovalPolicy::OnlyWhenNeeded,
            permission_profile: PermissionProfile::WorkspaceWrite,
            budget: RunBudget::default(),
            accepted_plan_id: None,
            accepted_plan_revision: None,
        },
        requested_capabilities: ProviderCapabilities {
            tool_calls: true,
            text_input: true,
            ..ProviderCapabilities::default()
        },
        negotiated_capabilities: ProviderCapabilities::default(),
        provider_capability_probe: None,
        capability_degradations: Vec::new(),
        failure_code: None,
        created_at_ms: now,
        updated_at_ms: now,
    }
}

fn side_effect(
    session: &SessionRecord,
    run: &RunRecord,
    id: &str,
    parameter_hash: &str,
    approval_id: Option<ApprovalId>,
) -> SideEffectExecutionRecord {
    let timestamp = now_ms();
    SideEffectExecutionRecord {
        id: SideEffectExecutionId::from(id),
        session_id: session.id.clone(),
        run_id: run.id.clone(),
        run_generation: run.generation,
        tool_call_id: ToolCallId::from("call-side-effect"),
        idempotency_key: "side-effect-key".into(),
        parameter_hash: parameter_hash.into(),
        approval_id,
        host_request_id: None,
        status: SideEffectExecutionStatus::Claimed,
        result_code: None,
        result_reference: None,
        created_at_ms: timestamp,
        updated_at_ms: timestamp,
    }
}

async fn create_running_run(store: &AgentStore, session: &SessionRecord, id: &str) -> RunRecord {
    let run = run(session, id);
    store
        .create_run_idempotent("user", id, &run)
        .await
        .expect("run");
    store
        .transition_run(&run.id, RunStatus::Preparing, None)
        .await
        .expect("preparing");
    store
        .transition_run(&run.id, RunStatus::Running, None)
        .await
        .expect("running");
    run
}

#[tokio::test]
async fn managed_run_diff_artifact_is_path_indexed_etagged_and_chunk_bounded() {
    let (store, session) = seeded_store().await;
    let run = create_running_run(&store, &session, "run-diff-chunk").await;
    let first = b"@@ -1 +1 @@\n-old\n+new\n";
    let second = b"@@ -2 +2 @@\n-left\n+right\n";
    let artifact = store
        .create_managed_run_diff_artifact(
            &run.id,
            &[
                ManagedRunDiffFile {
                    path: "src/lib.rs",
                    content: first,
                },
                ManagedRunDiffFile {
                    path: "src/main.rs",
                    content: second,
                },
            ],
            now_ms(),
        )
        .await
        .expect("artifact");

    let first_chunk = store
        .read_managed_run_diff_file_chunk(&run.id, &artifact, "src/main.rs", 0, 12, None)
        .await
        .expect("first chunk");
    assert_eq!(first_chunk.path, "src/main.rs");
    assert_eq!(first_chunk.next_offset, 12);
    assert!(!first_chunk.eof);
    assert_eq!(first_chunk.utf8_text.as_deref(), Some("@@ -2 +2 @@\n"));
    let second_chunk = store
        .read_managed_run_diff_file_chunk(
            &run.id,
            &artifact,
            "src/main.rs",
            first_chunk.next_offset,
            1024,
            Some(&first_chunk.etag),
        )
        .await
        .expect("second chunk");
    assert!(second_chunk.eof);
    assert_eq!(second_chunk.utf8_text.as_deref(), Some("-left\n+right\n"));
    assert!(
        store
            .read_managed_run_diff_file_chunk(&run.id, &artifact, "src/other.rs", 0, 1, None,)
            .await
            .is_err()
    );
    assert!(
        store
            .read_managed_run_diff_file_chunk(
                &run.id,
                &artifact,
                "src/main.rs",
                0,
                1,
                Some("sha256:stale"),
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn run_creation_is_idempotent_and_events_are_monotonic() {
    let (store, session) = seeded_store().await;
    let first = run(&session, "run-1");
    store
        .create_run_idempotent("user", "same-request", &first)
        .await
        .expect("first");
    let duplicate = run(&session, "run-2");
    let restored = store
        .create_run_idempotent("user", "same-request", &duplicate)
        .await
        .expect("duplicate");
    assert_eq!(restored.id, first.id);

    store
        .append_event(&session.id, Some(&first.id), "test.one", json!({}))
        .await
        .expect("event one");
    store
        .append_event(&session.id, Some(&first.id), "test.two", json!({}))
        .await
        .expect("event two");
    let sequences = store
        .list_events(&session.id, 0)
        .await
        .expect("events")
        .into_iter()
        .map(|event| event.sequence)
        .collect::<Vec<_>>();
    assert_eq!(sequences, vec![1, 2, 3, 4]);
}

#[tokio::test]
async fn illegal_transition_is_rejected() {
    let (store, session) = seeded_store().await;
    let run = run(&session, "run-1");
    store
        .create_run_idempotent("user", "run-1", &run)
        .await
        .expect("run");
    assert!(matches!(
        store
            .transition_run(&run.id, RunStatus::Succeeded, None)
            .await,
        Err(AgentStoreError::InvalidRunTransition { .. })
    ));
}

#[tokio::test]
async fn compaction_checkpoint_is_monotonic_and_event_backed() {
    let (store, session) = seeded_store().await;
    let transcript = store
        .append_transcript_item(TranscriptItem {
            id: hachimi_protocol::ItemId::from("item-compact"),
            session_id: session.id.clone(),
            run_id: None,
            sequence: 0,
            kind: TranscriptItemKind::User,
            status: ItemStatus::Completed,
            payload: ItemPayload::User {
                text: "retain src/lib.rs and run-42".into(),
                attachment_ids: Vec::new(),
            },
            relations: hachimi_protocol::ItemRelations::default(),
            created_at_ms: now_ms(),
        })
        .await
        .expect("transcript");
    let checkpoint = CompactionCheckpoint {
        id: CompactionCheckpointId::from("checkpoint-1"),
        session_id: session.id.clone(),
        run_id: None,
        previous_checkpoint_id: None,
        covered_through_sequence: transcript.sequence,
        reason: CompactionReason::Manual,
        lifecycle: hachimi_protocol::CompactionLifecycle {
            trigger: hachimi_protocol::CompactionTrigger::Manual,
            ..hachimi_protocol::CompactionLifecycle::default()
        },
        summary: hachimi_protocol::CompactionSummary {
            semantic_markdown: "## Current goal\nRetain continuity".into(),
            latest_user_goal: Some("retain src/lib.rs and run-42".into()),
            preserved_identifiers: vec!["run-42".into(), "src/lib.rs".into()],
        },
        quality: hachimi_protocol::CompactionQuality {
            accepted: true,
            source_items: 1,
            source_chars: 32,
            summary_chars: 31,
            recent_tail_items: 0,
            preserved_identifier_count: 2,
            warnings: Vec::new(),
        },
        created_at_ms: now_ms(),
    };
    store
        .create_compaction_checkpoint(&checkpoint)
        .await
        .expect("checkpoint");
    assert_eq!(
        store
            .latest_compaction_checkpoint(&session.id)
            .await
            .expect("latest"),
        Some(checkpoint.clone())
    );
    let events = store.list_events(&session.id, 0).await.expect("events");
    assert!(events.iter().any(|event| {
        matches!(
            &event.payload,
            RunEventPayload::Generic { event, data }
                if event == "context.compaction_checkpoint_created"
                    && data.get("checkpointId") == Some(&json!(checkpoint.id))
        )
    }));

    let mut invalid = checkpoint;
    invalid.id = CompactionCheckpointId::from("checkpoint-2");
    invalid.covered_through_sequence = invalid.covered_through_sequence.saturating_add(1);
    assert!(matches!(
        store.create_compaction_checkpoint(&invalid).await,
        Err(AgentStoreError::CompactionPredecessorMismatch)
    ));
}

#[tokio::test]
async fn restart_marks_active_work_lost_without_restoring_authority() {
    let (store, session) = seeded_store().await;
    let run = run(&session, "run-1");
    store
        .create_run_idempotent("user", "run-1", &run)
        .await
        .expect("run");
    store
        .transition_run(&run.id, RunStatus::Preparing, None)
        .await
        .expect("preparing");
    store
        .transition_run(&run.id, RunStatus::Running, None)
        .await
        .expect("running");
    let user_input = UserInputRequestRecord {
        id: UserInputRequestId::from("restart-input"),
        session_id: session.id.clone(),
        run_id: run.id.clone(),
        run_generation: run.generation,
        item_id: ItemId::from("restart-input-item"),
        questions: vec![UserInputQuestion {
            id: "choice".into(),
            header: "Choice".into(),
            prompt: "Continue?".into(),
            options: Vec::new(),
            secret: false,
            auto_resolution_ms: None,
            default_answer: None,
        }],
        status: UserInputStatus::Pending,
        expires_at_ms: None,
        created_at_ms: now_ms(),
        resolved_at_ms: None,
        resolved_by: None,
    };
    store
        .create_user_input_request(&user_input)
        .await
        .expect("user input");
    store
        .create_task_run(&TaskRunRecord {
            id: TaskRunId::from("task-1"),
            schedule_id: None,
            schedule_revision: None,
            trigger: TaskRunTrigger::Manual,
            scheduled_for_ms: None,
            invocation_key: "test:task-1".into(),
            requester_session_id: Some(session.id.clone()),
            execution_session_id: Some(session.id.clone()),
            run_id: Some(run.id.clone()),
            permission_snapshot_hash: None,
            status: TaskRunStatus::Running,
            progress_percent: None,
            result_summary: None,
            error_code: None,
            error_summary: None,
            artifact_ids: Vec::new(),
            delivery_status: DeliveryStatus::NotRequested,
            delivery_error_code: None,
            created_at_ms: now_ms(),
            started_at_ms: Some(now_ms()),
            finished_at_ms: None,
            updated_at_ms: now_ms(),
        })
        .await
        .expect("task");
    let report = store.recover_interrupted().await.expect("recover");
    assert_eq!(report.interrupted_runs, 1);
    assert_eq!(report.lost_tasks, 1);
    assert_eq!(report.interrupted_user_inputs, 1);
    assert_eq!(
        store.get_run(&run.id).await.expect("get").unwrap().status,
        RunStatus::Interrupted
    );
    assert_eq!(
        store
            .get_user_input_request(&user_input.id)
            .await
            .expect("get user input")
            .expect("user input")
            .status,
        UserInputStatus::Interrupted
    );
}

#[tokio::test]
async fn retained_worktree_count_only_includes_dirty_schedule_checkouts() {
    let (store, session) = seeded_store().await;
    sqlx::query(
        "UPDATE workspace_checkouts SET kind = 'managed_worktree', status = 'cleanup_blocked' WHERE id = 'checkout-1'",
    )
    .execute(store.pool())
    .await
    .expect("mark retained Worktree");
    let now = now_ms();
    let schedule = ScheduleDefinition {
        id: ScheduleId::from("schedule-retained"),
        name: "Retained Worktree".into(),
        enabled: false,
        prompt: "Inspect changes".into(),
        schedule: ScheduleSpec::Every {
            interval_ms: 86_400_000,
            anchor_ms: now,
        },
        entry_profile: EntryProfile::Workbench,
        workload_override: Some(WorkloadKind::Coding),
        context_template: ScheduleContextTemplate::General,
        tool_allowlist: Vec::new(),
        skill_allowlist: Vec::new(),
        mcp_tool_allowlist: Vec::new(),
        permission_config: SchedulePermissionConfig::default(),
        permission_revision: 1,
        timeout_ms: 120_000,
        misfire_policy: MisfirePolicy::Skip,
        delivery_policy: DeliveryPolicy::TaskTabOnly,
        config_revision: 1,
        created_by: "user".into(),
        next_run_at_ms: None,
        health: ScheduleHealth::NeedsAuthorization,
        health_reason: Some("test".into()),
        created_at_ms: now,
        updated_at_ms: now,
    };
    store
        .create_schedule_idempotent("user", "retained", &schedule, None)
        .await
        .expect("schedule");
    store
        .create_task_run(&TaskRunRecord {
            id: TaskRunId::from("task-retained"),
            schedule_id: Some(schedule.id.clone()),
            schedule_revision: Some(1),
            trigger: TaskRunTrigger::Manual,
            scheduled_for_ms: Some(now),
            invocation_key: "retained:1".into(),
            requester_session_id: None,
            execution_session_id: Some(session.id),
            run_id: None,
            permission_snapshot_hash: None,
            status: TaskRunStatus::Succeeded,
            progress_percent: Some(100),
            result_summary: None,
            error_code: None,
            error_summary: None,
            artifact_ids: Vec::new(),
            delivery_status: DeliveryStatus::NotRequested,
            delivery_error_code: None,
            created_at_ms: now,
            started_at_ms: Some(now),
            finished_at_ms: Some(now),
            updated_at_ms: now,
        })
        .await
        .expect("task");
    assert_eq!(
        store
            .count_schedule_retained_worktrees(&schedule.id)
            .await
            .expect("count"),
        1
    );
    store
        .update_checkout_lifecycle(
            &CheckoutId::from("checkout-1"),
            CheckoutStatus::Ready,
            false,
        )
        .await
        .expect("clear retained status");
    assert_eq!(
        store
            .count_schedule_retained_worktrees(&schedule.id)
            .await
            .expect("count"),
        0
    );
}

#[tokio::test]
async fn approval_resolution_is_bound_to_hash_and_generation() {
    let (store, session) = seeded_store().await;
    let run = run(&session, "run-approval");
    store
        .create_run_idempotent("user", "run-approval", &run)
        .await
        .expect("run");
    store
        .transition_run(&run.id, RunStatus::Preparing, None)
        .await
        .expect("preparing");
    store
        .transition_run(&run.id, RunStatus::Running, None)
        .await
        .expect("running");
    store
        .transition_run(&run.id, RunStatus::WaitingApproval, None)
        .await
        .expect("waiting approval");
    let created_at_ms = now_ms();
    let approval = ApprovalRequestRecord {
        id: ApprovalId::from("approval-1"),
        session_id: session.id,
        run_id: run.id,
        tool_call_id: ToolCallId::from("call-1"),
        run_generation: 1,
        status: ApprovalStatus::Pending,
        action: "workspace.exec".into(),
        resource: "cargo test".into(),
        parameter_hash: "sha256:expected".into(),
        risk_summary: "execute a test command".into(),
        target_host: "workspace-worker".into(),
        required_scopes: vec!["workspace.exec".into()],
        grant_scope: ApprovalGrantScope::Once,
        uses_remaining: 1,
        requester_principal: "user".into(),
        resolved_by: None,
        expires_at_ms: Some(created_at_ms + 60_000),
        created_at_ms,
        resolved_at_ms: None,
    };
    store.create_approval(&approval).await.expect("approval");
    let mut resolution = ApprovalResolution {
        approval_id: approval.id.clone(),
        decision: ApprovalStatus::Approved,
        parameter_hash: "sha256:changed".into(),
        run_generation: 1,
        resolved_by: "user".into(),
        resolved_at_ms: created_at_ms + 1,
    };
    assert!(matches!(
        store.resolve_approval(&resolution).await,
        Err(AgentStoreError::StaleApprovalResolution)
    ));
    resolution.parameter_hash = approval.parameter_hash.clone();
    resolution.run_generation = 0;
    assert!(matches!(
        store.resolve_approval(&resolution).await,
        Err(AgentStoreError::StaleApprovalResolution)
    ));
    resolution.run_generation = 1;
    let resolved = store.resolve_approval(&resolution).await.expect("resolved");
    assert_eq!(resolved.status, ApprovalStatus::Approved);
    assert!(
        store
            .list_pending_approvals()
            .await
            .expect("pending")
            .is_empty()
    );

    let mut stale_state_approval = approval;
    stale_state_approval.id = ApprovalId::from("approval-stale-state");
    stale_state_approval.tool_call_id = ToolCallId::from("call-stale-state");
    stale_state_approval.status = ApprovalStatus::Pending;
    stale_state_approval.resolved_by = None;
    stale_state_approval.resolved_at_ms = None;
    store
        .create_approval(&stale_state_approval)
        .await
        .expect("stale state approval");
    store
        .transition_run(&stale_state_approval.run_id, RunStatus::Running, None)
        .await
        .expect("resume run");
    let stale_state_resolution = ApprovalResolution {
        approval_id: stale_state_approval.id,
        decision: ApprovalStatus::Approved,
        parameter_hash: stale_state_approval.parameter_hash,
        run_generation: stale_state_approval.run_generation,
        resolved_by: "user".into(),
        resolved_at_ms: created_at_ms + 2,
    };
    assert!(matches!(
        store.resolve_approval(&stale_state_resolution).await,
        Err(AgentStoreError::StaleApprovalResolution)
    ));
}

#[tokio::test]
async fn concurrent_duplicate_side_effect_claims_have_one_dispatch_owner() {
    let (store, session) = seeded_store().await;
    let run = create_running_run(&store, &session, "run-side-effect-concurrent").await;

    let mut tasks = Vec::new();
    for index in 0..20 {
        let store = store.clone();
        let record = side_effect(
            &session,
            &run,
            &format!("execution-{index}"),
            "sha256:same",
            None,
        );
        tasks.push(tokio::spawn(async move {
            store.claim_side_effect(&record).await
        }));
    }
    let mut created = 0;
    let mut canonical_id = None;
    for task in tasks {
        let claim = task.await.expect("join").expect("claim");
        created += usize::from(claim.created);
        canonical_id.get_or_insert(claim.record.id.clone());
        assert_eq!(Some(&claim.record.id), canonical_id.as_ref());
    }
    assert_eq!(created, 1);
}

#[tokio::test]
async fn side_effect_idempotency_conflict_and_persisted_replay_are_explicit() {
    let (store, session) = seeded_store().await;
    let run = create_running_run(&store, &session, "run-side-effect-replay").await;
    let record = side_effect(&session, &run, "execution-first", "sha256:first", None);
    let claim = store.claim_side_effect(&record).await.expect("claim");
    assert!(claim.created);

    let conflict = side_effect(&session, &run, "execution-conflict", "sha256:changed", None);
    assert!(matches!(
        store.claim_side_effect(&conflict).await,
        Err(AgentStoreError::SideEffectIdempotencyConflict)
    ));

    store
        .mark_side_effect_dispatched(&record.id, "host-request", now_ms())
        .await
        .expect("dispatch");
    let result = json!({
        "modelContent": "completed once",
        "structuredContent": { "written": true }
    });
    store
        .finish_side_effect(
            &record.id,
            SideEffectExecutionStatus::Succeeded,
            Some("succeeded"),
            None,
            Some(&result),
            now_ms(),
        )
        .await
        .expect("finish");
    let replay = store
        .claim_side_effect(&side_effect(
            &session,
            &run,
            "execution-retry",
            "sha256:first",
            None,
        ))
        .await
        .expect("replay");
    assert!(!replay.created);
    assert_eq!(replay.record.status, SideEffectExecutionStatus::Succeeded);
    assert_eq!(replay.persisted_result, Some(result));
}

#[tokio::test]
async fn duplicate_claim_consumes_one_approval_use_only_once() {
    let (store, session) = seeded_store().await;
    let run = run(&session, "run-side-effect-approval");
    store
        .create_run_idempotent("user", "run-side-effect-approval", &run)
        .await
        .expect("run");
    store
        .transition_run(&run.id, RunStatus::Preparing, None)
        .await
        .expect("preparing");
    store
        .transition_run(&run.id, RunStatus::Running, None)
        .await
        .expect("running");
    store
        .transition_run(&run.id, RunStatus::WaitingApproval, None)
        .await
        .expect("waiting approval");
    let timestamp = now_ms();
    let approval = ApprovalRequestRecord {
        id: ApprovalId::from("approval-side-effect"),
        session_id: session.id.clone(),
        run_id: run.id.clone(),
        tool_call_id: ToolCallId::from("call-side-effect"),
        run_generation: run.generation,
        status: ApprovalStatus::Pending,
        action: "workspace.write".into(),
        resource: "README.md".into(),
        parameter_hash: "sha256:approved".into(),
        risk_summary: "write a file".into(),
        target_host: "workspace-worker".into(),
        required_scopes: vec!["workspace.write".into()],
        grant_scope: ApprovalGrantScope::Once,
        uses_remaining: 1,
        requester_principal: "user".into(),
        resolved_by: None,
        expires_at_ms: Some(timestamp + 60_000),
        created_at_ms: timestamp,
        resolved_at_ms: None,
    };
    store.create_approval(&approval).await.expect("approval");
    store
        .resolve_approval(&ApprovalResolution {
            approval_id: approval.id.clone(),
            decision: ApprovalStatus::Approved,
            parameter_hash: approval.parameter_hash.clone(),
            run_generation: approval.run_generation,
            resolved_by: "user".into(),
            resolved_at_ms: timestamp + 1,
        })
        .await
        .expect("approve");
    store
        .transition_run(&run.id, RunStatus::Running, None)
        .await
        .expect("resume after approval");

    let first = side_effect(
        &session,
        &run,
        "execution-approved",
        &approval.parameter_hash,
        Some(approval.id.clone()),
    );
    assert!(
        store
            .claim_side_effect(&first)
            .await
            .expect("first claim")
            .created
    );
    let duplicate = side_effect(
        &session,
        &run,
        "execution-approved-retry",
        &approval.parameter_hash,
        Some(approval.id.clone()),
    );
    assert!(
        !store
            .claim_side_effect(&duplicate)
            .await
            .expect("duplicate claim")
            .created
    );
    let uses_remaining =
        sqlx::query_scalar::<_, i64>("SELECT uses_remaining FROM approval_requests WHERE id = ?")
            .bind(approval.id.as_str())
            .fetch_one(store.pool())
            .await
            .expect("uses remaining");
    assert_eq!(uses_remaining, 0);
}

#[tokio::test]
async fn restart_marks_dispatched_side_effect_indeterminate_and_never_replays_it() {
    let (store, session) = seeded_store().await;
    let run = create_running_run(&store, &session, "run-side-effect-restart").await;
    let record = side_effect(&session, &run, "execution-dispatched", "sha256:once", None);
    store.claim_side_effect(&record).await.expect("claim");
    store
        .mark_side_effect_dispatched(&record.id, "host-request-unknown", now_ms())
        .await
        .expect("dispatch");

    let recovery = store.recover_interrupted().await.expect("recovery");
    assert_eq!(recovery.indeterminate_side_effects, 1);
    let retry = store
        .claim_side_effect(&side_effect(
            &session,
            &run,
            "execution-after-restart",
            "sha256:once",
            None,
        ))
        .await
        .expect("retry lookup");
    assert!(!retry.created);
    assert_eq!(
        retry.record.status,
        SideEffectExecutionStatus::Indeterminate
    );
    assert!(retry.persisted_result.is_none());
}

#[tokio::test]
async fn security_snapshot_is_recoverable_and_grants_can_be_invalidated_once() {
    let (store, session) = seeded_store().await;
    let run = run(&session, "run-security");
    store
        .create_run_idempotent("user", "run-security", &run)
        .await
        .expect("run");
    let grants = CapabilityGrantSet {
        profile: PermissionProfile::WorkspaceWrite,
        scope: PermissionGrantScope::Run,
        session_id: session.id.clone(),
        run_id: Some(run.id.clone()),
        source: "test".into(),
        file_system: vec![FileSystemGrant {
            access: FileSystemAccess::Write,
            roots: vec!["C:\\demo".into()],
            globs: Vec::new(),
            special_roots: Vec::new(),
        }],
        network: NetworkGrant::default(),
        process: ProcessGrant::default(),
        computer: ComputerGrant::default(),
        review_each_command: true,
        expires_at_ms: None,
    };
    let report = SandboxCapabilityReport {
        backend: "windows-test".into(),
        readiness: SandboxReadiness::Degraded,
        os_enforced: false,
        filesystem_enforced: false,
        process_enforced: false,
        network_enforced: false,
        version: None,
        stable_error_code: Some("runtime_attestation_missing".into()),
        diagnostics: vec!["test report".into()],
    };
    store
        .persist_run_security_snapshot(&grants, &report, now_ms())
        .await
        .expect("security snapshot");
    assert_eq!(
        store.latest_sandbox_report(&run.id).await.expect("report"),
        Some(report)
    );
    assert_eq!(
        store
            .invalidate_run_capability_grants(&session.id, &run.id, now_ms())
            .await
            .expect("invalidate"),
        1
    );
    assert_eq!(
        store
            .invalidate_run_capability_grants(&session.id, &run.id, now_ms())
            .await
            .expect("invalidate again"),
        0
    );
}

#[tokio::test]
async fn mcp_keyring_cleanup_queue_persists_only_opaque_references() {
    let store = AgentStore::connect_in_memory().await.expect("store");
    store
        .defer_mcp_keyring_cleanup("server:authorization:opaque-id", 10)
        .await
        .expect("defer");
    store
        .defer_mcp_keyring_cleanup("server:authorization:opaque-id", 20)
        .await
        .expect("retry");

    assert_eq!(
        store
            .list_pending_mcp_keyring_cleanup(10)
            .await
            .expect("list"),
        ["server:authorization:opaque-id"]
    );
    let row = sqlx::query(
        "SELECT attempt_count, created_at_ms, last_attempt_at_ms FROM mcp_keyring_cleanup_queue",
    )
    .fetch_one(&store.pool)
    .await
    .expect("queue row");
    assert_eq!(row.get::<i64, _>("attempt_count"), 2);
    assert_eq!(row.get::<i64, _>("created_at_ms"), 10);
    assert_eq!(row.get::<i64, _>("last_attempt_at_ms"), 20);
    assert!(
        store
            .complete_mcp_keyring_cleanup("server:authorization:opaque-id")
            .await
            .expect("complete")
    );
    assert!(
        store
            .list_pending_mcp_keyring_cleanup(10)
            .await
            .expect("empty")
            .is_empty()
    );
}

#[tokio::test]
async fn checkout_write_lease_is_exclusive_and_generation_bound() {
    let (store, session) = seeded_store().await;
    let first = run(&session, "run-lease-1");
    let second = run(&session, "run-lease-2");
    store
        .create_run_idempotent("user", "run-lease-1", &first)
        .await
        .expect("first");
    store
        .create_run_idempotent("user", "run-lease-2", &second)
        .await
        .expect("second");
    let checkout_id = CheckoutId::from("checkout-1");
    store
        .acquire_checkout_write_lease(&checkout_id, &first.id, first.generation)
        .await
        .expect("first lease");
    assert!(matches!(
        store
            .acquire_checkout_write_lease(&checkout_id, &second.id, second.generation)
            .await,
        Err(AgentStoreError::CheckoutWriteLeaseHeld { .. })
    ));
    assert!(
        !store
            .release_checkout_write_lease(&checkout_id, &first.id, first.generation + 1)
            .await
            .expect("stale release")
    );
    assert!(
        store
            .release_checkout_write_lease(&checkout_id, &first.id, first.generation)
            .await
            .expect("release")
    );
    store
        .acquire_checkout_write_lease(&checkout_id, &second.id, second.generation)
        .await
        .expect("second lease");
    store.recover_interrupted().await.expect("recover");
    store
        .acquire_checkout_write_lease(&checkout_id, &first.id, first.generation)
        .await
        .expect("lease after recovery");
}

#[tokio::test]
async fn mcp_configuration_and_health_are_persistent_and_restart_safe() {
    let store = AgentStore::connect_in_memory().await.expect("store");
    let configured_at = now_ms();
    let server = McpServerRecord {
        id: McpServerId::from("local.docs"),
        display_name: "Local documents".into(),
        enabled: true,
        transport: hachimi_protocol::McpServerTransport::Stdio {
            command: "mcp-docs".into(),
            args: vec!["--stdio".into()],
            cwd: Some("C:\\workspace".into()),
        },
        headers: Vec::new(),
        read_only_tools: vec!["documents/read".into()],
        startup_timeout_ms: 15_000,
        request_timeout_ms: 60_000,
        max_message_bytes: 1024 * 1024,
        created_at_ms: configured_at,
        updated_at_ms: configured_at,
    };
    store.upsert_mcp_server(&server).await.expect("upsert");
    assert_eq!(
        store.get_mcp_server(&server.id).await.expect("get"),
        Some(server.clone())
    );
    let initial = store
        .get_mcp_server_health(&server.id)
        .await
        .expect("health")
        .expect("initial health");
    assert_eq!(initial.state, McpServerHealthState::Stopped);

    let ready = McpServerHealthRecord {
        server_id: server.id.clone(),
        state: McpServerHealthState::Ready,
        server_name: Some("Fixture".into()),
        server_version: Some("1.0.0".into()),
        protocol_version: Some("2025-06-18".into()),
        tool_count: 1,
        error_code: None,
        checked_at_ms: configured_at + 1,
    };
    store.set_mcp_server_health(&ready).await.expect("ready");
    let recovery = store.recover_interrupted().await.expect("recover");
    assert_eq!(recovery.stopped_mcp_servers, 1);
    let recovered = store
        .get_mcp_server_health(&server.id)
        .await
        .expect("health")
        .expect("recovered health");
    assert_eq!(recovered.state, McpServerHealthState::Stopped);
    assert_eq!(recovered.error_code.as_deref(), Some("host_restarted"));
    assert_eq!(recovered.tool_count, 0);
    assert!(store.remove_mcp_server(&server.id).await.expect("remove"));
    assert!(
        store
            .get_mcp_server_health(&server.id)
            .await
            .expect("health")
            .is_none()
    );
}

#[tokio::test]
async fn mcp_configuration_rejects_values_outside_transport_limits() {
    let store = AgentStore::connect_in_memory().await.expect("store");
    let invalid = McpServerRecord {
        id: McpServerId::from("bad server"),
        display_name: "Invalid".into(),
        enabled: true,
        transport: hachimi_protocol::McpServerTransport::Stdio {
            command: "server".into(),
            args: Vec::new(),
            cwd: None,
        },
        headers: Vec::new(),
        read_only_tools: Vec::new(),
        startup_timeout_ms: 15_000,
        request_timeout_ms: 60_000,
        max_message_bytes: 1024 * 1024,
        created_at_ms: now_ms(),
        updated_at_ms: now_ms(),
    };
    assert!(matches!(
        store.upsert_mcp_server(&invalid).await,
        Err(AgentStoreError::InvalidMcpServerConfiguration("server ID"))
    ));
}

#[tokio::test]
async fn mutation_idempotency_claims_cache_responses_and_fence_conflicts() {
    let store = AgentStore::connect_in_memory().await.expect("store");
    let first = store
        .claim_idempotent_mutation::<serde_json::Value>(
            "user",
            "schedule.update",
            "key-1",
            "schedule-1",
            now_ms(),
        )
        .await
        .expect("claim");
    assert_eq!(first, IdempotentMutationClaim::Claimed);
    let in_flight = store
        .claim_idempotent_mutation::<serde_json::Value>(
            "user",
            "schedule.update",
            "key-1",
            "schedule-1",
            now_ms(),
        )
        .await
        .expect("repeat claim");
    assert_eq!(in_flight, IdempotentMutationClaim::Indeterminate);

    let response = serde_json::json!({ "revision": 2 });
    store
        .complete_idempotent_mutation("user", "schedule.update", "key-1", &response)
        .await
        .expect("complete");
    let replay = store
        .claim_idempotent_mutation::<serde_json::Value>(
            "user",
            "schedule.update",
            "key-1",
            "schedule-1",
            now_ms(),
        )
        .await
        .expect("replay");
    assert_eq!(replay, IdempotentMutationClaim::Completed(response));
    assert!(matches!(
        store
            .claim_idempotent_mutation::<serde_json::Value>(
                "user",
                "schedule.update",
                "key-1",
                "schedule-2",
                now_ms(),
            )
            .await,
        Err(AgentStoreError::IdempotencyConflict)
    ));

    store
        .claim_idempotent_mutation::<bool>(
            "user",
            "schedule.remove",
            "key-2",
            "schedule-1",
            now_ms(),
        )
        .await
        .expect("removal claim");
    store
        .abandon_idempotent_mutation("user", "schedule.remove", "key-2")
        .await
        .expect("abandon");
    assert_eq!(
        store
            .claim_idempotent_mutation::<bool>(
                "user",
                "schedule.remove",
                "key-2",
                "schedule-1",
                now_ms(),
            )
            .await
            .expect("reclaim"),
        IdempotentMutationClaim::Claimed
    );
}

#[tokio::test]
async fn active_checkout_runs_follow_the_tagged_session_context_after_migration() {
    let (store, session) = seeded_store().await;
    let checkout_id = session.context.checkout_id().expect("checkout").clone();
    assert!(
        !store
            .checkout_has_active_runs(&checkout_id)
            .await
            .expect("empty")
    );
    let run = run(&session, "active-checkout-run");
    store
        .create_run_idempotent("user", "active-checkout-run", &run)
        .await
        .expect("run");
    assert!(
        store
            .checkout_has_active_runs(&checkout_id)
            .await
            .expect("active")
    );
    store
        .transition_run(&run.id, RunStatus::Cancelled, None)
        .await
        .expect("cancel");
    assert!(
        !store
            .checkout_has_active_runs(&checkout_id)
            .await
            .expect("terminal")
    );
}
