use hachimi_protocol::{
    AgentTaskId, AgentTaskMessageId, AgentTaskMessageRecord, AgentTaskRecord, AgentTaskStatus,
    EntryProfile, ItemId, ItemPayload, ItemRelations, ItemStatus, RunBudget, RunId, RunRecord,
    RunStatus, SessionId, SessionRecord, TranscriptItem, TranscriptItemKind,
};

use super::tests::{run, seeded_store};
use super::{AgentStore, AgentStoreError, now_ms};

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

async fn agent_child_run(
    store: &AgentStore,
    parent_session: &SessionRecord,
    parent_run: &RunRecord,
    suffix: &str,
) -> (SessionRecord, RunRecord) {
    let timestamp = now_ms();
    let session = SessionRecord {
        id: SessionId::new(format!("agent-session-{suffix}")),
        context: parent_session.context.clone(),
        entry_profile: EntryProfile::Workbench,
        title: format!("Agent {suffix}"),
        archived: false,
        pinned: false,
        parent_session_id: Some(parent_session.id.clone()),
        source_run_id: Some(parent_run.id.clone()),
        created_at_ms: timestamp,
        updated_at_ms: timestamp,
    };
    store.create_session(&session).await.expect("child session");
    let child = run(&session, &format!("agent-run-{suffix}"));
    store
        .create_run_idempotent("agent-test", &format!("agent-run-{suffix}"), &child)
        .await
        .expect("child run");
    (session, child)
}

#[allow(clippy::too_many_arguments)]
fn agent_task(
    id: &str,
    root_task_id: &str,
    root_run_id: &RunId,
    parent_task_id: Option<AgentTaskId>,
    parent_session: &SessionRecord,
    parent_run: &RunRecord,
    child_session: &SessionRecord,
    child_run: &RunRecord,
    depth: u8,
    model_budget: u32,
) -> AgentTaskRecord {
    let timestamp = now_ms();
    AgentTaskRecord {
        id: AgentTaskId::from(id),
        root_task_id: AgentTaskId::from(root_task_id),
        root_run_id: root_run_id.clone(),
        parent_task_id,
        parent_session_id: parent_session.id.clone(),
        parent_run_id: parent_run.id.clone(),
        child_session_id: child_session.id.clone(),
        child_run_id: child_run.id.clone(),
        title: format!("Task {id}"),
        depth,
        status: AgentTaskStatus::Queued,
        reserved_budget: RunBudget {
            max_model_requests: model_budget,
            max_tool_calls: model_budget,
            max_parallel_read_tools: 1,
            model_timeout_ms: 1_000,
            tool_timeout_ms: 1_000,
        },
        usage: Default::default(),
        artifact_ids: Vec::new(),
        result_summary: None,
        error_code: None,
        created_at_ms: timestamp,
        started_at_ms: None,
        finished_at_ms: None,
        updated_at_ms: timestamp,
    }
}

#[tokio::test]
async fn agent_tasks_enforce_root_concurrency_budget_lineage_and_messages() {
    let (store, session) = seeded_store().await;
    let parent = run(&session, "agent-parent");
    store
        .create_run_idempotent("agent-test", "agent-parent", &parent)
        .await
        .expect("parent run");

    let mut first_task = None;
    for index in 0..4 {
        let suffix = format!("concurrent-{index}");
        let (child_session, child_run) = agent_child_run(&store, &session, &parent, &suffix).await;
        let id = format!("agent-task-{suffix}");
        let task = agent_task(
            &id,
            &id,
            &parent.id,
            None,
            &session,
            &parent,
            &child_session,
            &child_run,
            1,
            1,
        );
        let created = store.create_agent_task(&task).await.expect("active task");
        first_task.get_or_insert(created);
    }
    let (fifth_session, fifth_run) =
        agent_child_run(&store, &session, &parent, "concurrent-4").await;
    let fifth = agent_task(
        "agent-task-concurrent-4",
        "agent-task-concurrent-4",
        &parent.id,
        None,
        &session,
        &parent,
        &fifth_session,
        &fifth_run,
        1,
        1,
    );
    assert!(matches!(
        store.create_agent_task(&fifth).await,
        Err(AgentStoreError::AgentTaskLimitExceeded)
    ));

    let first = first_task.expect("first task");
    let message = AgentTaskMessageRecord {
        id: AgentTaskMessageId::from("agent-message-1"),
        task_id: first.id.clone(),
        sender_run_id: parent.id.clone(),
        recipient_run_id: first.child_run_id.clone(),
        content: "Report status only; do not widen permissions.".into(),
        created_at_ms: now_ms(),
        delivered_at_ms: Some(now_ms()),
    };
    store
        .append_agent_task_message(&message)
        .await
        .expect("message");
    let collected = store
        .collect_agent_tasks(&parent.id)
        .await
        .expect("collection");
    assert_eq!(collected.tasks.len(), 4);
    assert_eq!(collected.messages, vec![message]);

    let (budget_store, budget_session) = seeded_store().await;
    let budget_parent = run(&budget_session, "agent-budget-parent");
    budget_store
        .create_run_idempotent("agent-test", "agent-budget-parent", &budget_parent)
        .await
        .expect("budget parent");
    for index in 0..2 {
        let suffix = format!("budget-{index}");
        let (child_session, child_run) =
            agent_child_run(&budget_store, &budget_session, &budget_parent, &suffix).await;
        let id = format!("agent-task-{suffix}");
        let task = agent_task(
            &id,
            &id,
            &budget_parent.id,
            None,
            &budget_session,
            &budget_parent,
            &child_session,
            &child_run,
            1,
            17,
        );
        if index == 0 {
            budget_store
                .create_agent_task(&task)
                .await
                .expect("first budget reservation");
        } else {
            assert!(matches!(
                budget_store.create_agent_task(&task).await,
                Err(AgentStoreError::AgentTaskLimitExceeded)
            ));
        }
    }
}

#[tokio::test]
async fn agent_tasks_limit_total_children_and_depth() {
    let (store, session) = seeded_store().await;
    let parent = run(&session, "agent-limit-parent");
    store
        .create_run_idempotent("agent-test", "agent-limit-parent", &parent)
        .await
        .expect("parent run");
    for index in 0..17 {
        let suffix = format!("total-{index}");
        let (child_session, child_run) = agent_child_run(&store, &session, &parent, &suffix).await;
        let id = format!("agent-task-{suffix}");
        let task = agent_task(
            &id,
            &id,
            &parent.id,
            None,
            &session,
            &parent,
            &child_session,
            &child_run,
            if index == 16 { 4 } else { 1 },
            1,
        );
        if index < 16 {
            let created = store.create_agent_task(&task).await.expect("bounded child");
            store
                .transition_agent_task(
                    &created.id,
                    AgentTaskStatus::Cancelled,
                    None,
                    Some("test_complete"),
                    now_ms(),
                )
                .await
                .expect("release concurrency and budget");
        } else {
            assert!(matches!(
                store.create_agent_task(&task).await,
                Err(AgentStoreError::AgentTaskLimitExceeded)
            ));
        }
    }
}

#[tokio::test]
async fn agent_task_execution_lease_fences_duplicate_spawn_and_expired_owners() {
    let (store, session) = seeded_store().await;
    let parent = run(&session, "agent-lease-parent");
    store
        .create_run_idempotent("agent-test", "agent-lease-parent", &parent)
        .await
        .expect("parent run");
    let (child_session, child_run) = agent_child_run(&store, &session, &parent, "lease").await;
    let task = agent_task(
        "agent-task-lease",
        "agent-task-lease",
        &parent.id,
        None,
        &session,
        &parent,
        &child_session,
        &child_run,
        1,
        1,
    );
    store.create_agent_task(&task).await.expect("task");
    let first = store
        .claim_agent_task_execution(&task.id, "process-a", 100, 50)
        .await
        .expect("claim")
        .expect("first owner");
    assert_eq!(first.execution_generation, 1);
    assert!(
        store
            .claim_agent_task_execution(&task.id, "process-b", 120, 50)
            .await
            .expect("duplicate claim")
            .is_none()
    );
    assert!(
        store
            .renew_agent_task_execution_lease(
                &task.id,
                first.execution_generation,
                "process-a",
                130,
                50,
            )
            .await
            .expect("renew")
    );
    assert!(
        store
            .claim_agent_task_execution(&task.id, "process-b", 170, 50)
            .await
            .expect("still fenced")
            .is_none()
    );
    let second = store
        .claim_agent_task_execution(&task.id, "process-b", 181, 50)
        .await
        .expect("expired claim")
        .expect("replacement owner");
    assert_eq!(second.execution_generation, 2);
    assert!(
        !store
            .release_agent_task_execution_lease(
                &task.id,
                first.execution_generation,
                "process-a",
                182,
            )
            .await
            .expect("stale release")
    );
    assert!(
        store
            .release_agent_task_execution_lease(
                &task.id,
                second.execution_generation,
                "process-b",
                183,
            )
            .await
            .expect("release")
    );
}

#[tokio::test]
async fn agent_task_transitions_refresh_the_parent_collab_item() {
    let (store, session) = seeded_store().await;
    let parent = run(&session, "agent-collab-parent");
    store
        .create_run_idempotent("agent-test", "agent-collab-parent", &parent)
        .await
        .expect("parent run");
    let (child_session, child_run) = agent_child_run(&store, &session, &parent, "collab").await;
    let task = agent_task(
        "agent-task-collab",
        "agent-task-collab",
        &parent.id,
        None,
        &session,
        &parent,
        &child_session,
        &child_run,
        1,
        1,
    );
    store.create_agent_task(&task).await.expect("task");

    let item_id = ItemId::from("collab-item");
    store
        .append_transcript_item(TranscriptItem {
            id: item_id.clone(),
            session_id: session.id.clone(),
            run_id: Some(parent.id.clone()),
            sequence: 0,
            kind: TranscriptItemKind::CollabToolCall,
            status: ItemStatus::InProgress,
            payload: ItemPayload::CollabToolCall {
                tool_name: "agent.spawn".into(),
                agent_task_id: None,
                parent_run_id: parent.id.clone(),
                child_run_id: None,
                title: task.title.clone(),
                status: "running".into(),
                summary: None,
                usage: Default::default(),
            },
            relations: ItemRelations::default(),
            created_at_ms: now_ms(),
        })
        .await
        .expect("collab item");
    store
        .link_agent_task_transcript_item(&task.id, &item_id)
        .await
        .expect("link");
    store
        .complete_transcript_item(
            &item_id,
            ItemStatus::Completed,
            ItemPayload::CollabToolCall {
                tool_name: "agent.spawn".into(),
                agent_task_id: Some(task.id.clone()),
                parent_run_id: parent.id.clone(),
                child_run_id: Some(child_run.id.clone()),
                title: task.title.clone(),
                status: "queued".into(),
                summary: None,
                usage: Default::default(),
            },
        )
        .await
        .expect("complete");
    store
        .transition_agent_task(
            &task.id,
            AgentTaskStatus::Running,
            Some("working"),
            None,
            now_ms(),
        )
        .await
        .expect("running");

    let transcript = store
        .list_transcript(&session.id)
        .await
        .expect("transcript");
    let item = transcript
        .iter()
        .find(|item| item.id == item_id)
        .expect("updated collab item");
    assert_eq!(item.relations.agent_task_id.as_ref(), Some(&task.id));
    assert!(matches!(
        &item.payload,
        ItemPayload::CollabToolCall { status, summary, .. }
            if status == "running" && summary.as_deref() == Some("working")
    ));
    assert!(store
        .list_events(&session.id, 0)
        .await
        .expect("events")
        .iter()
        .any(|event| matches!(
            &event.payload,
            hachimi_protocol::RunEventPayload::ItemCompleted { item }
                if item.id == item_id
                    && matches!(&item.payload, ItemPayload::CollabToolCall { status, .. } if status == "running")
        )));
}
