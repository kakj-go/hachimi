use hachimi_protocol::{
    BehaviorMode, EntryProfile, LlmSettings, PermissionProfile, PlanAcceptanceRequest, PlanId,
    ProposedPlan, ProposedPlanStatus, RunOrigin, RunStatus, SessionContextBinding,
    WorkbenchTaskStartRequest,
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
    let plan = service
        .store()
        .create_proposed_plan(ProposedPlan {
            id: PlanId::from("workspace-plan-proposal"),
            session_id: planned.session.id.clone(),
            run_id: planned.run.id,
            revision: 0,
            goal: "Apply the workspace change".into(),
            assumptions: Vec::new(),
            steps: Vec::new(),
            affected_resources: Vec::new(),
            verification: Vec::new(),
            risks: Vec::new(),
            open_questions: Vec::new(),
            content_markdown: "Apply the workspace change".into(),
            status: ProposedPlanStatus::Proposed,
            accepted_run_id: None,
            created_at_ms: now_ms(),
            accepted_at_ms: None,
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
