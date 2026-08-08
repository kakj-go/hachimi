use hachimi_protocol::{
    BehaviorMode, EntryProfile, ItemId, ItemPayload, ItemRelations, ItemStatus, LlmSettings,
    PermissionProfile, PlanAcceptanceRequest, PlanDocument, PlanId, PlanRevisionRequest,
    PlanSkipRequest, RunOrigin, RunStatus, SessionContextBinding, TranscriptItem,
    TranscriptItemKind, WorkbenchTaskStartRequest,
};
use hachimi_storage::AgentStore;
use tokio_util::sync::CancellationToken;

use crate::{WorkbenchService, now_ms};

#[tokio::test]
async fn workspace_plan_acceptance_creates_writable_authorized_run() {
    let store = AgentStore::connect_in_memory().await.expect("store");
    let worktrees = tempfile::tempdir().expect("worktrees");
    let attachments = tempfile::tempdir().expect("attachments");
    let service = WorkbenchService::new(store, worktrees.path(), attachments.path());
    let planned = service
        .create_task(
            &WorkbenchTaskStartRequest {
                idempotency_key: "workspace-plan".into(),
                entry_profile: EntryProfile::Workbench,
                session_id: None,
                project_id: None,
                prompt: "Plan a change in the default workspace".into(),
                execution_target: None,
                behavior_mode: BehaviorMode::Plan,
                permission_profile: PermissionProfile::Writable,
                attachment_ids: Vec::new(),
                skill_ids: Vec::new(),
            },
            LlmSettings::default(),
            "test-user",
            "workspace-plan",
            &CancellationToken::new(),
        )
        .await
        .expect("planned task");
    let workspace_id = match &planned.session.context {
        SessionContextBinding::Workspace { workspace_id } => workspace_id.clone(),
        SessionContextBinding::Project { .. } => panic!("expected workspace session"),
    };
    assert_eq!(
        planned.run.configuration.permission_profile,
        PermissionProfile::ReadOnly
    );
    assert_eq!(
        service
            .store()
            .permission_policy(&format!("session:{}", planned.session.id))
            .await
            .expect("policy lookup")
            .expect("session policy")
            .level,
        PermissionProfile::Writable
    );
    let environment = service
        .environment_snapshot(&planned.session.id)
        .await
        .expect("workspace environment");
    assert!(environment.checkout.is_none());
    assert_eq!(
        environment
            .workspace
            .as_ref()
            .map(|workspace| &workspace.id),
        Some(&workspace_id)
    );
    assert!(environment.git.head_sha.is_none());
    assert!(!environment.handoff.can_handoff);
    service
        .store()
        .transition_run(&planned.run.id, RunStatus::Preparing, None)
        .await
        .expect("preparing");
    service
        .store()
        .transition_run(&planned.run.id, RunStatus::Running, None)
        .await
        .expect("running");
    service
        .store()
        .transition_run(&planned.run.id, RunStatus::Succeeded, None)
        .await
        .expect("succeeded");
    let source_item_id = ItemId::random();
    service
        .store()
        .append_transcript_item(TranscriptItem {
            id: source_item_id.clone(),
            session_id: planned.session.id.clone(),
            run_id: Some(planned.run.id.clone()),
            sequence: 0,
            kind: TranscriptItemKind::Plan,
            status: ItemStatus::Completed,
            payload: ItemPayload::Plan {
                text: "# Apply the workspace change".into(),
            },
            relations: ItemRelations::default(),
            created_at_ms: now_ms(),
        })
        .await
        .expect("plan item");
    let (plan, _) = service
        .store()
        .create_plan_document(PlanDocument {
            id: PlanId::from("workspace-plan-proposal"),
            session_id: planned.session.id.clone(),
            source_run_id: planned.run.id,
            source_item_id,
            revision: 0,
            title: "Apply the workspace change".into(),
            goal: "Apply the workspace change".into(),
            content_markdown: "# Apply the workspace change".into(),
            created_at_ms: now_ms(),
        })
        .await
        .expect("plan");

    let accepted = service
        .accept_plan(
            &PlanAcceptanceRequest {
                idempotency_key: "workspace-plan-accept".into(),
                plan_id: plan.id,
                expected_revision: plan.revision,
                user_message: "Implement it".into(),
            },
            LlmSettings::default(),
            "test-user",
        )
        .await
        .expect("accepted workspace plan");

    assert!(accepted.task.project.is_none());
    assert!(accepted.task.checkout.is_none());
    assert_eq!(accepted.task.run.origin, RunOrigin::Manual);
    assert_eq!(
        accepted.task.run.configuration.permission_profile,
        PermissionProfile::Writable
    );
    assert_eq!(
        accepted.task.session.context,
        SessionContextBinding::Workspace {
            workspace_id: workspace_id.clone()
        }
    );
    let workspace = service
        .store()
        .workspace(&workspace_id)
        .await
        .expect("workspace lookup")
        .expect("workspace");
    let authority = service
        .store()
        .authority_snapshot(&accepted.task.run.id)
        .await
        .expect("authority lookup")
        .expect("authority");
    assert_eq!(authority.policy.level, PermissionProfile::Writable);
    assert_eq!(authority.workspace_root, workspace.root_path);
}

#[tokio::test]
async fn skipped_plan_is_persistent_idempotent_and_remains_in_the_environment() {
    let store = AgentStore::connect_in_memory().await.expect("store");
    let worktrees = tempfile::tempdir().expect("worktrees");
    let attachments = tempfile::tempdir().expect("attachments");
    let service = WorkbenchService::new(store, worktrees.path(), attachments.path());
    let planned = service
        .create_task(
            &WorkbenchTaskStartRequest {
                idempotency_key: "workspace-plan-skip".into(),
                entry_profile: EntryProfile::Workbench,
                session_id: None,
                project_id: None,
                prompt: "Plan a skipped change".into(),
                execution_target: None,
                behavior_mode: BehaviorMode::Plan,
                permission_profile: PermissionProfile::Writable,
                attachment_ids: Vec::new(),
                skill_ids: Vec::new(),
            },
            LlmSettings::default(),
            "test-user",
            "workspace-plan-skip",
            &CancellationToken::new(),
        )
        .await
        .expect("planned task");
    service
        .store()
        .transition_run(&planned.run.id, RunStatus::Preparing, None)
        .await
        .expect("preparing");
    service
        .store()
        .transition_run(&planned.run.id, RunStatus::Running, None)
        .await
        .expect("running");
    service
        .store()
        .transition_run(&planned.run.id, RunStatus::Succeeded, None)
        .await
        .expect("succeeded");
    let source_item_id = ItemId::random();
    service
        .store()
        .append_transcript_item(TranscriptItem {
            id: source_item_id.clone(),
            session_id: planned.session.id.clone(),
            run_id: Some(planned.run.id.clone()),
            sequence: 0,
            kind: TranscriptItemKind::Plan,
            status: ItemStatus::Completed,
            payload: ItemPayload::Plan {
                text: "# Skipped change".into(),
            },
            relations: ItemRelations::default(),
            created_at_ms: now_ms(),
        })
        .await
        .expect("plan item");
    let (plan, _) = service
        .store()
        .create_plan_document(PlanDocument {
            id: PlanId::from("workspace-plan-skipped"),
            session_id: planned.session.id.clone(),
            source_run_id: planned.run.id,
            source_item_id,
            revision: 0,
            title: "Skipped change".into(),
            goal: "Skipped change".into(),
            content_markdown: "# Skipped change".into(),
            created_at_ms: now_ms(),
        })
        .await
        .expect("plan");
    let pending_environment = service
        .environment_snapshot(&planned.session.id)
        .await
        .expect("pending plan environment");
    assert!(
        pending_environment
            .activities
            .iter()
            .any(|activity| matches!(
                activity,
                hachimi_protocol::EnvironmentActivity::Plan {
                    plan_id,
                    confirmation_status: hachimi_protocol::PlanConfirmationStatus::Pending,
                    ..
                } if plan_id == &plan.id
            ))
    );
    let request = PlanSkipRequest {
        idempotency_key: "skip-plan".into(),
        plan_id: plan.id.clone(),
        expected_revision: plan.revision,
    };
    let skipped = service.skip_plan(&request).await.expect("skip plan");
    assert_eq!(
        skipped.confirmation.status,
        hachimi_protocol::PlanConfirmationStatus::Skipped
    );
    let duplicate = service.skip_plan(&request).await.expect("idempotent skip");
    assert_eq!(duplicate.confirmation, skipped.confirmation);
    let environment = service
        .environment_snapshot(&planned.session.id)
        .await
        .expect("environment");
    assert!(environment.revision > pending_environment.revision);
    assert!(environment.activities.iter().any(|activity| matches!(
        activity,
        hachimi_protocol::EnvironmentActivity::Plan {
            plan_id,
            confirmation_status: hachimi_protocol::PlanConfirmationStatus::Skipped,
            ..
        } if plan_id == &plan.id
    )));
}

#[tokio::test]
async fn plan_revision_supersedes_and_creates_one_idempotent_plan_run() {
    let store = AgentStore::connect_in_memory().await.expect("store");
    let worktrees = tempfile::tempdir().expect("worktrees");
    let attachments = tempfile::tempdir().expect("attachments");
    let service = WorkbenchService::new(store, worktrees.path(), attachments.path());
    let planned = service
        .create_task(
            &WorkbenchTaskStartRequest {
                idempotency_key: "workspace-plan-revise".into(),
                entry_profile: EntryProfile::Workbench,
                session_id: None,
                project_id: None,
                prompt: "Plan a change that needs revision".into(),
                execution_target: None,
                behavior_mode: BehaviorMode::Plan,
                permission_profile: PermissionProfile::Writable,
                attachment_ids: Vec::new(),
                skill_ids: Vec::new(),
            },
            LlmSettings::default(),
            "test-user",
            "workspace-plan-revise",
            &CancellationToken::new(),
        )
        .await
        .expect("planned task");
    service
        .store()
        .transition_run(&planned.run.id, RunStatus::Preparing, None)
        .await
        .expect("preparing");
    service
        .store()
        .transition_run(&planned.run.id, RunStatus::Running, None)
        .await
        .expect("running");
    service
        .store()
        .transition_run(&planned.run.id, RunStatus::Succeeded, None)
        .await
        .expect("succeeded");
    let item = service
        .store()
        .append_transcript_item(TranscriptItem {
            id: ItemId::random(),
            session_id: planned.session.id.clone(),
            run_id: Some(planned.run.id.clone()),
            sequence: 0,
            kind: TranscriptItemKind::Plan,
            status: ItemStatus::Completed,
            payload: ItemPayload::Plan {
                text: "# Original plan".into(),
            },
            relations: ItemRelations::default(),
            created_at_ms: now_ms(),
        })
        .await
        .expect("plan item");
    let (plan, _) = service
        .store()
        .create_plan_document(PlanDocument {
            id: PlanId::from("workspace-plan-revision-source"),
            session_id: planned.session.id.clone(),
            source_run_id: planned.run.id,
            source_item_id: item.id,
            revision: 0,
            title: "Original plan".into(),
            goal: "Original plan".into(),
            content_markdown: "# Original plan".into(),
            created_at_ms: now_ms(),
        })
        .await
        .expect("plan");
    let request = PlanRevisionRequest {
        idempotency_key: "revise-original-plan".into(),
        plan_id: plan.id.clone(),
        expected_revision: plan.revision,
        instructions: "Use a smaller verification step".into(),
    };
    let first = service
        .revise_plan(
            &request,
            LlmSettings::default(),
            "test-user",
            &CancellationToken::new(),
        )
        .await
        .expect("revise plan");
    let duplicate = service
        .revise_plan(
            &request,
            LlmSettings::default(),
            "test-user",
            &CancellationToken::new(),
        )
        .await
        .expect("idempotent revise plan");
    assert_eq!(first.run.id, duplicate.run.id);
    assert_eq!(first.run.configuration.behavior_mode, BehaviorMode::Plan);
    assert_eq!(
        service
            .store()
            .get_plan_confirmation(&plan.id)
            .await
            .expect("confirmation")
            .expect("confirmation")
            .status,
        hachimi_protocol::PlanConfirmationStatus::Superseded
    );
    assert_eq!(
        service
            .store()
            .list_runs(&planned.session.id)
            .await
            .expect("runs")
            .len(),
        2
    );
}
