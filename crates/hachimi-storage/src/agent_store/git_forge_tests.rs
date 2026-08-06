use hachimi_protocol::{
    ApprovalPolicy, BehaviorMode, CheckoutId, CheckoutKind, CheckoutRecord, CheckoutStatus,
    EntryProfile, ForgeChangeRecord, ForgeChangeState, ForgeKind, ForgeOperationId,
    ForgeOperationRecord, ForgeOperationStatus, ForgeRepositoryIdentity, LlmSettings,
    PermissionProfile, ProjectId, ProjectRecord, ProviderCapabilities, RunBudget, RunConfiguration,
    RunDriverKind, RunId, RunOrigin, RunPurpose, RunRecord, RunStatus, SessionContextBinding,
    SessionId, SessionRecord,
};

use super::{AgentStore, AgentStoreError};

async fn store_with_run() -> (AgentStore, SessionRecord, RunRecord) {
    let store = AgentStore::connect_in_memory().await.expect("store");
    let project = ProjectRecord {
        id: ProjectId::from("forge-project"),
        display_name: "Forge".into(),
        root_path: "C:\\forge".into(),
        git_root: Some("C:\\forge".into()),
        trusted: true,
        created_at_ms: 1,
        updated_at_ms: 1,
    };
    store.create_project(&project).await.expect("project");
    let checkout = CheckoutRecord {
        id: CheckoutId::from("forge-checkout"),
        project_id: project.id.clone(),
        kind: CheckoutKind::Local,
        path: project.root_path,
        base_revision: Some("main".into()),
        head_revision: Some("a".repeat(40)),
        status: CheckoutStatus::Ready,
        pinned: false,
        created_at_ms: 1,
        updated_at_ms: 1,
    };
    store.create_checkout(&checkout).await.expect("checkout");
    let session = SessionRecord {
        id: SessionId::from("forge-session"),
        context: SessionContextBinding::Project {
            project_id: project.id,
            checkout_id: checkout.id,
        },
        entry_profile: EntryProfile::Workbench,
        title: "Forge".into(),
        archived: false,
        pinned: false,
        parent_session_id: None,
        source_run_id: None,
        created_at_ms: 1,
        updated_at_ms: 1,
    };
    store.create_session(&session).await.expect("session");
    let run = RunRecord {
        id: RunId::from("forge-run"),
        session_id: session.id.clone(),
        status: RunStatus::Queued,
        purpose: RunPurpose::Task,
        origin: RunOrigin::Manual,
        generation: 1,
        configuration: RunConfiguration {
            model_snapshot: LlmSettings::default(),
            driver: RunDriverKind::ToolLoop,
            entry_profile: EntryProfile::Workbench,
            workload_override: None,
            behavior_mode: BehaviorMode::Default,
            execution_target: None,
            approval_policy: ApprovalPolicy::OnlyWhenNeeded,
            permission_profile: PermissionProfile::Writable,
            budget: RunBudget::default(),
            accepted_plan_id: None,
            accepted_plan_revision: None,
        },
        requested_capabilities: ProviderCapabilities::default(),
        negotiated_capabilities: ProviderCapabilities::default(),
        provider_capability_probe: None,
        capability_degradations: Vec::new(),
        failure_code: None,
        created_at_ms: 1,
        updated_at_ms: 1,
    };
    store
        .create_run_idempotent("test", "forge-run", &run)
        .await
        .expect("run");
    (store, session, run)
}

fn operation(session: &SessionRecord, run: &RunRecord) -> ForgeOperationRecord {
    ForgeOperationRecord {
        id: ForgeOperationId::from("forge-operation"),
        session_id: session.id.clone(),
        run_id: Some(run.id.clone()),
        run_generation: Some(run.generation),
        operation_kind: "forge.change.create".into(),
        repository: ForgeRepositoryIdentity {
            forge_kind: ForgeKind::GitHub,
            api_base_url: "https://api.github.com/".into(),
            owner: "team".into(),
            repository: "repo".into(),
            remote_url_hash: "a".repeat(64),
            secret_ref: Some("forge:test".into()),
        },
        source_ref: Some("feature".into()),
        target_ref: Some("main".into()),
        commit_oid: "b".repeat(40),
        expected_revision: None,
        approval_id: None,
        idempotency_key: "forge-create-1".into(),
        request_hash: "c".repeat(64),
        status: ForgeOperationStatus::Claimed,
        result: None,
        error_code: None,
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

#[tokio::test]
async fn forge_ledger_is_idempotent_and_reconciliation_safe() {
    let (store, session, run) = store_with_run().await;
    let operation = operation(&session, &run);
    assert_eq!(
        store
            .claim_forge_operation(&operation)
            .await
            .expect("claim"),
        operation
    );
    assert_eq!(
        store
            .claim_forge_operation(&operation)
            .await
            .expect("replay")
            .id,
        operation.id
    );
    let mut conflict = operation.clone();
    conflict.request_hash = "d".repeat(64);
    assert!(matches!(
        store.claim_forge_operation(&conflict).await,
        Err(AgentStoreError::IdempotencyConflict)
    ));
    store
        .update_forge_operation(
            &operation.id,
            ForgeOperationStatus::Claimed,
            ForgeOperationStatus::Dispatched,
            None,
            None,
            2,
        )
        .await
        .expect("dispatched");
    let result = ForgeChangeRecord {
        forge_kind: ForgeKind::GitHub,
        number: 7,
        title: "Change".into(),
        body: String::new(),
        source_ref: "feature".into(),
        target_ref: "main".into(),
        source_commit_oid: Some("b".repeat(40)),
        state: ForgeChangeState::Open,
        web_url: Some("https://github.com/team/repo/pull/7".into()),
        revision: "e".repeat(64),
    };
    let confirmed = store
        .update_forge_operation(
            &operation.id,
            ForgeOperationStatus::Dispatched,
            ForgeOperationStatus::Confirmed,
            Some(&result),
            None,
            3,
        )
        .await
        .expect("confirmed");
    assert_eq!(confirmed.result, Some(result));
    assert!(
        store
            .list_forge_operations_for_reconciliation()
            .await
            .expect("reconcile")
            .is_empty()
    );
}
