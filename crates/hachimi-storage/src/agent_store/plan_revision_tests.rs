use hachimi_protocol::{
    AgentPermissionPolicy, BehaviorMode, ItemId, ItemPayload, ItemRelations, ItemStatus,
    PermissionProfile, PlanConfirmationStatus, PlanDocument, PlanId, PlanStep, PlanStepId,
    PlanStepStatus, RunRecord, RunStatus, TranscriptItem, TranscriptItemKind,
};

use super::{AgentStore, AtomicRunLaunchInput, now_ms};
use crate::agent_store::tests::{run, seeded_store};

async fn ensure_environment(store: &AgentStore, session: &hachimi_protocol::SessionRecord) {
    let checkout_id = session.context.checkout_id().expect("checkout");
    let checkout = store
        .get_checkout(checkout_id)
        .await
        .expect("checkout lookup")
        .expect("checkout");
    store
        .ensure_session_environment_state(
            &session.id,
            &checkout.id,
            checkout.kind,
            checkout.head_revision.as_deref(),
        )
        .await
        .expect("environment");
}

async fn pending_plan(
    store: &AgentStore,
    session: &hachimi_protocol::SessionRecord,
    suffix: &str,
) -> PlanDocument {
    let mut source = run(session, &format!("plan-source-{suffix}"));
    source.configuration.behavior_mode = BehaviorMode::Plan;
    source.configuration.permission_profile = PermissionProfile::ReadOnly;
    store
        .create_run_idempotent("test", &format!("plan-source-{suffix}"), &source)
        .await
        .expect("source run");
    store
        .transition_run(&source.id, RunStatus::Preparing, None)
        .await
        .expect("preparing");
    store
        .transition_run(&source.id, RunStatus::Running, None)
        .await
        .expect("running");
    store
        .transition_run(&source.id, RunStatus::Succeeded, None)
        .await
        .expect("succeeded");
    let item = store
        .append_transcript_item(TranscriptItem {
            id: ItemId::random(),
            session_id: session.id.clone(),
            run_id: Some(source.id.clone()),
            sequence: 0,
            kind: TranscriptItemKind::Plan,
            status: ItemStatus::Completed,
            payload: ItemPayload::Plan {
                text: format!("# Plan {suffix}"),
            },
            relations: ItemRelations::default(),
            created_at_ms: now_ms(),
        })
        .await
        .expect("plan item");
    store
        .create_plan_document(PlanDocument {
            id: PlanId::new(format!("plan-{suffix}")),
            session_id: session.id.clone(),
            source_run_id: source.id,
            source_item_id: item.id,
            revision: 0,
            title: format!("Plan {suffix}"),
            goal: format!("Plan {suffix}"),
            content_markdown: format!("# Plan {suffix}"),
            created_at_ms: now_ms(),
        })
        .await
        .expect("plan document")
        .0
}

fn revision_candidate(
    session: &hachimi_protocol::SessionRecord,
    id: &str,
) -> (RunRecord, TranscriptItem) {
    let mut candidate = run(session, id);
    candidate.configuration.behavior_mode = BehaviorMode::Plan;
    candidate.configuration.permission_profile = PermissionProfile::ReadOnly;
    let item = TranscriptItem {
        id: ItemId::new(format!("item-{id}")),
        session_id: session.id.clone(),
        run_id: Some(candidate.id.clone()),
        sequence: 0,
        kind: TranscriptItemKind::User,
        status: ItemStatus::Completed,
        payload: ItemPayload::User {
            text: "Revise the plan".into(),
            attachment_ids: Vec::new(),
        },
        relations: ItemRelations::default(),
        created_at_ms: candidate.created_at_ms,
    };
    (candidate, item)
}

#[tokio::test]
async fn plan_revision_creation_is_atomic_idempotent_and_rolls_back_on_launch_failure() {
    let (store, mut session) = seeded_store().await;
    ensure_environment(&store, &session).await;
    let plan = pending_plan(&store, &session, "atomic").await;
    session = store
        .get_session(&session.id)
        .await
        .expect("session lookup")
        .expect("session");
    let (candidate, item) = revision_candidate(&session, "revision-run");
    let policy = AgentPermissionPolicy {
        level: PermissionProfile::ReadOnly,
        ..AgentPermissionPolicy::default()
    };
    let owner_key = format!("session:{}", session.id);
    let launch = AtomicRunLaunchInput {
        proposed_workspace: None,
        workspace_owner: None,
        stored_policy_owner_key: Some(&owner_key),
        stored_policy: Some(&policy),
        effective_policy: &policy,
        authority_mode: hachimi_protocol::AuthorityMode::Interactive,
        workspace_root: "C:\\demo",
        task_run_id: None,
    };
    let created = store
        .create_plan_revision_run_authorized_idempotent(
            "test",
            "revise-atomic",
            &session,
            &candidate,
            &item,
            &[],
            &plan.id,
            plan.revision,
            launch,
        )
        .await
        .expect("revision run");
    let duplicate = store
        .create_plan_revision_run_authorized_idempotent(
            "test",
            "revise-atomic",
            &session,
            &candidate,
            &item,
            &[],
            &plan.id,
            plan.revision,
            launch,
        )
        .await
        .expect("idempotent revision run");
    assert_eq!(duplicate.run.id, created.run.id);
    assert_eq!(
        store
            .get_plan_confirmation(&plan.id)
            .await
            .expect("confirmation")
            .expect("confirmation")
            .status,
        PlanConfirmationStatus::Superseded
    );

    store
        .transition_run(&created.run.id, RunStatus::Cancelled, None)
        .await
        .expect("cancel revision run");
    let rollback_plan = pending_plan(&store, &session, "rollback").await;
    session = store
        .get_session(&session.id)
        .await
        .expect("session lookup")
        .expect("session");
    let (rollback_run, rollback_item) = revision_candidate(&session, "rollback-run");
    let failed = store
        .create_plan_revision_run_authorized_idempotent(
            "test",
            "revise-rollback",
            &session,
            &rollback_run,
            &rollback_item,
            &[],
            &rollback_plan.id,
            rollback_plan.revision,
            AtomicRunLaunchInput {
                workspace_root: "",
                ..launch
            },
        )
        .await;
    assert!(failed.is_err());
    assert!(
        store
            .get_run(&rollback_run.id)
            .await
            .expect("rollback run lookup")
            .is_none()
    );
    assert_eq!(
        store
            .get_plan_confirmation(&rollback_plan.id)
            .await
            .expect("confirmation")
            .expect("confirmation")
            .status,
        PlanConfirmationStatus::Pending
    );

    let (race_run_a, race_item_a) = revision_candidate(&session, "race-run-a");
    let (race_run_b, race_item_b) = revision_candidate(&session, "race-run-b");
    let first = store.create_plan_revision_run_authorized_idempotent(
        "test",
        "revise-race-a",
        &session,
        &race_run_a,
        &race_item_a,
        &[],
        &rollback_plan.id,
        rollback_plan.revision,
        launch,
    );
    let second = store.create_plan_revision_run_authorized_idempotent(
        "test",
        "revise-race-b",
        &session,
        &race_run_b,
        &race_item_b,
        &[],
        &rollback_plan.id,
        rollback_plan.revision,
        launch,
    );
    let (first, second) = tokio::join!(first, second);
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    assert_eq!(
        store
            .get_plan_confirmation(&rollback_plan.id)
            .await
            .expect("confirmation")
            .expect("confirmation")
            .status,
        PlanConfirmationStatus::Superseded
    );
    let persisted_race_runs = usize::from(
        store
            .get_run(&race_run_a.id)
            .await
            .expect("race run lookup")
            .is_some(),
    ) + usize::from(
        store
            .get_run(&race_run_b.id)
            .await
            .expect("race run lookup")
            .is_some(),
    );
    assert_eq!(persisted_race_runs, 1);
}

#[tokio::test]
async fn accepted_execution_terminal_bumps_environment_without_rewriting_checklist() {
    let (store, session) = seeded_store().await;
    ensure_environment(&store, &session).await;
    let plan = pending_plan(&store, &session, "execution").await;
    let mut execution = run(&session, "accepted-execution");
    execution.configuration.accepted_plan_id = Some(plan.id.clone());
    execution.configuration.accepted_plan_revision = Some(plan.revision);
    let (_, _, execution) = store
        .accept_proposed_plan_idempotent("test", "accept-execution", &plan.id, &execution)
        .await
        .expect("accept plan");
    let steps = vec![
        PlanStep {
            id: PlanStepId::from("done"),
            description: "Already done".into(),
            status: PlanStepStatus::Completed,
        },
        PlanStep {
            id: PlanStepId::from("active"),
            description: "Still active".into(),
            status: PlanStepStatus::InProgress,
        },
    ];
    store
        .update_execution_plan(&execution.id, Some("Keep model state"), &steps)
        .await
        .expect("execution plan");
    let before = store
        .get_session_environment_state(&session.id)
        .await
        .expect("environment")
        .expect("environment")
        .revision;
    store
        .transition_run(&execution.id, RunStatus::Preparing, None)
        .await
        .expect("preparing");
    store
        .transition_run(&execution.id, RunStatus::Running, None)
        .await
        .expect("running");
    store
        .transition_run(&execution.id, RunStatus::Succeeded, None)
        .await
        .expect("succeeded");
    let after = store
        .get_session_environment_state(&session.id)
        .await
        .expect("environment")
        .expect("environment")
        .revision;
    assert_eq!(after, before + 1);
    assert_eq!(
        store
            .get_execution_plan(&execution.id)
            .await
            .expect("execution plan")
            .expect("execution plan")
            .steps,
        steps
    );

    let unrelated = run(&session, "unrelated-terminal");
    store
        .create_run_idempotent("test", "unrelated-terminal", &unrelated)
        .await
        .expect("unrelated run");
    store
        .transition_run(&unrelated.id, RunStatus::Preparing, None)
        .await
        .expect("preparing");
    store
        .transition_run(&unrelated.id, RunStatus::Running, None)
        .await
        .expect("running");
    store
        .transition_run(&unrelated.id, RunStatus::Succeeded, None)
        .await
        .expect("succeeded");
    let unrelated_after = store
        .get_session_environment_state(&session.id)
        .await
        .expect("environment")
        .expect("environment")
        .revision;
    assert_eq!(unrelated_after, after);
}
