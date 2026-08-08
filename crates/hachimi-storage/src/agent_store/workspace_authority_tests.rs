use std::path::Path;

use super::*;

#[tokio::test]
async fn permission_config_persists_policy_and_skill_ids_together() {
    let (store, _) = seeded_store().await;
    let policy = AgentPermissionPolicy {
        revision: 3,
        ..AgentPermissionPolicy::default()
    };
    let skills = vec![
        SkillId::from("office-documents"),
        SkillId::from("office-pdf"),
    ];

    store
        .store_permission_config("profile:pet_conversation", &policy, &skills, now_ms())
        .await
        .expect("store permission config");

    assert_eq!(
        store
            .permission_policy("profile:pet_conversation")
            .await
            .expect("policy"),
        Some(policy)
    );
    assert_eq!(
        store
            .permission_skill_ids("profile:pet_conversation")
            .await
            .expect("skills"),
        skills
    );
}

#[tokio::test]
async fn permission_owners_resolve_only_matching_active_runs() {
    let (store, session) = seeded_store().await;
    let mut scheduled = run(&session, "permission-owner-run");
    scheduled.origin = RunOrigin::Scheduled {
        schedule_id: ScheduleId::from("permission-owner-schedule"),
        task_run_id: TaskRunId::from("permission-owner-task"),
        scheduled_for_ms: now_ms(),
        event_context: None,
    };
    store
        .create_run_idempotent("test", "permission-owner-run", &scheduled)
        .await
        .expect("run");

    let mut channel_session = session.clone();
    channel_session.id = SessionId::from("permission-owner-channel-session");
    store
        .create_session(&channel_session)
        .await
        .expect("channel session");
    let mut channel = run(&channel_session, "permission-owner-channel-run");
    channel.origin = RunOrigin::Channel {
        channel: "fixture".into(),
        account: "account-1".into(),
        peer: "peer-1".into(),
        thread: "thread-1".into(),
        message_id: hachimi_protocol::ChannelMessageId::from("message-1"),
    };
    store
        .create_run_idempotent("test", "permission-owner-channel-run", &channel)
        .await
        .expect("channel run");
    sqlx::query("INSERT INTO channel_session_bindings(binding_key_hash, binding_key_json, account_id, authorization_id, authorization_revision, identity_group_id, session_id, created_at_ms, updated_at_ms) VALUES('binding-1', '{}', NULL, NULL, 1, NULL, ?, ?, ?)")
        .bind(channel_session.id.as_str())
        .bind(now_ms())
        .bind(now_ms())
        .execute(store.pool())
        .await
        .expect("channel binding");

    let mut pet_session = session.clone();
    pet_session.id = SessionId::from("permission-owner-pet-session");
    pet_session.entry_profile = EntryProfile::PetConversation;
    store
        .create_session(&pet_session)
        .await
        .expect("pet session");
    let mut pet = run(&pet_session, "permission-owner-pet-run");
    pet.origin = RunOrigin::Pet;
    store
        .create_run_idempotent("test", "permission-owner-pet-run", &pet)
        .await
        .expect("pet run");

    for owner in [
        format!("session:{}", session.id),
        "schedule:permission-owner-schedule".into(),
    ] {
        let active = store
            .active_runs_for_permission_owner(&owner)
            .await
            .expect("active runs");
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, scheduled.id);
    }
    assert!(
        store
            .active_runs_for_permission_owner("schedule:other")
            .await
            .expect("unrelated owner")
            .is_empty()
    );
    assert_eq!(
        store
            .active_runs_for_permission_owner("channel_binding:binding-1")
            .await
            .expect("channel owner"),
        vec![channel]
    );
    assert_eq!(
        store
            .active_runs_for_permission_owner("profile:pet_conversation")
            .await
            .expect("pet owner"),
        vec![pet]
    );

    store
        .transition_run(&scheduled.id, RunStatus::Cancelled, None)
        .await
        .expect("cancelled");
    assert!(
        store
            .active_runs_for_permission_owner(&format!("session:{}", session.id))
            .await
            .expect("terminal runs")
            .is_empty()
    );
}

#[tokio::test]
async fn session_extra_authority_clear_revokes_tool_and_host_grants() {
    let (store, session) = seeded_store().await;
    let other_session = SessionId::from("host-authority-other-session");
    let mut other = session.clone();
    other.id = other_session.clone();
    store.create_session(&other).await.expect("other session");
    let run = run(&session, "host-authority-run");
    store
        .create_run_idempotent("test", "host-authority-run", &run)
        .await
        .expect("run");
    let timestamp = now_ms();
    sqlx::query(
        "INSERT INTO host_access_grants(id, target_kind, target_key, scope, scope_key, owner_session_id, owner_run_id, capabilities_json, allow_private_network, expires_at_ms, created_at_ms, updated_at_ms) VALUES('session-computer-grant', 'computer', 'notepad', 'session', ?, ?, NULL, '[\"observe\",\"act\"]', 0, NULL, ?, ?)",
    )
    .bind(format!("session:{}", session.id))
    .bind(session.id.as_str())
    .bind(timestamp)
    .bind(timestamp)
    .execute(store.pool())
    .await
    .expect("computer grant");
    sqlx::query(
        "INSERT INTO embedded_browser_site_permissions(id, origin, scope, scope_key, owner_session_id, owner_run_id, capabilities_json, allow_private_network, created_at_ms, expires_at_ms, updated_at_ms) VALUES('session-browser-grant', 'https://session.example', 'session', ?, ?, NULL, '[\"observe\",\"act\"]', 0, ?, NULL, ?)",
    )
    .bind(format!("session:{}", session.id))
    .bind(session.id.as_str())
    .bind(timestamp)
    .bind(timestamp)
    .execute(store.pool())
    .await
    .expect("browser grant");

    let summaries = store
        .list_session_host_authorities(&session.id)
        .await
        .expect("host summary");
    assert_eq!(summaries.len(), 2);
    assert_eq!(
        store
            .clear_session_extra_authorities(&session.id, timestamp + 1)
            .await
            .expect("clear authorities"),
        2
    );
    assert!(
        store
            .list_session_host_authorities(&session.id)
            .await
            .expect("cleared host summary")
            .is_empty()
    );
    assert_eq!(
        sqlx::query("SELECT COUNT(*) FROM host_access_grants WHERE owner_session_id = ?")
            .bind(session.id.as_str())
            .fetch_one(store.pool())
            .await
            .expect("host grant count")
            .get::<i64, _>(0),
        0
    );
    assert_eq!(
        sqlx::query(
            "SELECT COUNT(*) FROM embedded_browser_site_permissions WHERE owner_session_id = ?",
        )
        .bind(session.id.as_str())
        .fetch_one(store.pool())
        .await
        .expect("browser permission count")
        .get::<i64, _>(0),
        0
    );
}

#[tokio::test]
async fn selected_workspace_is_normalized_and_marked_unavailable_when_removed() {
    let (store, session) = seeded_store().await;
    let directory = tempfile::tempdir().expect("selected directory");
    let workspace_id = hachimi_protocol::WorkspaceId::from("selected-workspace");
    let owner = super::WorkspaceOwnerRef::Session(&session.id);
    let workspace = store
        .ensure_selected_workspace(workspace_id, owner, directory.path(), now_ms())
        .await
        .expect("selected workspace");
    assert_eq!(workspace.kind, AgentWorkspaceKind::SelectedDirectory);
    assert_eq!(workspace.status, AgentWorkspaceStatus::Ready);
    assert_eq!(
        std::fs::canonicalize(directory.path())
            .expect("canonical directory")
            .to_string_lossy(),
        workspace.root_path
    );

    drop(directory);
    let error = store
        .ensure_selected_workspace(
            workspace.id.clone(),
            owner,
            Path::new(&workspace.root_path),
            now_ms(),
        )
        .await
        .expect_err("removed directory must not fall back");
    assert!(error.to_string().contains("workspace root"));
    let persisted = store
        .workspace(&workspace.id)
        .await
        .expect("workspace lookup")
        .expect("persisted workspace");
    assert_eq!(persisted.status, AgentWorkspaceStatus::Unavailable);
    assert!(
        persisted
            .status_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("unavailable"))
    );
}

#[tokio::test]
async fn new_selected_workspace_failure_persists_unavailable_state() {
    let (store, session) = seeded_store().await;
    let parent = tempfile::tempdir().expect("workspace parent");
    let missing = parent.path().join("missing");
    let workspace_id = hachimi_protocol::WorkspaceId::from("missing-selected-workspace");
    let owner = super::WorkspaceOwnerRef::Session(&session.id);

    let error = store
        .ensure_selected_workspace(workspace_id.clone(), owner, &missing, now_ms())
        .await
        .expect_err("missing selected directory");
    assert!(error.to_string().contains("workspace root"));

    let persisted = store
        .workspace(&workspace_id)
        .await
        .expect("workspace lookup")
        .expect("unavailable workspace row");
    assert_eq!(persisted.kind, AgentWorkspaceKind::SelectedDirectory);
    assert_eq!(persisted.status, AgentWorkspaceStatus::Unavailable);
    assert_eq!(persisted.root_path, missing.to_string_lossy());
    assert!(persisted.status_reason.is_some());
}

#[tokio::test]
async fn workspace_cleanup_deletes_only_exact_managed_directories() {
    let (store, session) = seeded_store().await;
    let owner = super::WorkspaceOwnerRef::Session(&session.id);
    let managed = store
        .ensure_managed_workspace(
            hachimi_protocol::WorkspaceId::from("managed-cleanup"),
            owner,
            now_ms(),
        )
        .await
        .expect("managed workspace");
    let managed_root = Path::new(&managed.root_path).to_path_buf();
    std::fs::write(managed_root.join("artifact.txt"), "managed").expect("managed file");
    assert!(
        store
            .remove_workspace_for_owner(owner)
            .await
            .expect("remove row")
    );
    assert!(!managed_root.exists());

    let selected_directory = tempfile::tempdir().expect("selected directory");
    store
        .ensure_selected_workspace(
            hachimi_protocol::WorkspaceId::from("selected-cleanup"),
            owner,
            selected_directory.path(),
            now_ms(),
        )
        .await
        .expect("selected workspace");
    assert!(
        store
            .remove_workspace_for_owner(owner)
            .await
            .expect("remove row")
    );
    assert!(selected_directory.path().is_dir());
}

#[tokio::test]
async fn startup_reconciliation_removes_orphaned_managed_rows_and_directories() {
    let store = AgentStore::connect_in_memory().await.expect("store");
    let orphan_session = SessionId::from("missing-session");
    let workspace = store
        .ensure_managed_workspace(
            hachimi_protocol::WorkspaceId::from("orphan-managed"),
            super::WorkspaceOwnerRef::Session(&orphan_session),
            now_ms(),
        )
        .await
        .expect("orphan workspace");
    let root = workspace.root_path.clone();
    let report = store
        .reconcile_managed_workspaces()
        .await
        .expect("reconcile workspaces");
    assert_eq!(report.removed_rows, 1);
    assert_eq!(report.removed_directories, 1);
    assert!(!Path::new(&root).exists());
}

#[cfg(unix)]
#[tokio::test]
async fn selected_workspace_rejects_symbolic_link_roots() {
    let (store, session) = seeded_store().await;
    let directory = tempfile::tempdir().expect("selected directory");
    let target = directory.path().join("target");
    let link = directory.path().join("link");
    std::fs::create_dir(&target).expect("target");
    std::os::unix::fs::symlink(&target, &link).expect("symlink");
    let error = store
        .ensure_selected_workspace(
            hachimi_protocol::WorkspaceId::from("symlink-workspace"),
            super::WorkspaceOwnerRef::Session(&session.id),
            &link,
            now_ms(),
        )
        .await
        .expect_err("symlink root must be rejected");
    assert!(error.to_string().contains("workspace root"));
}

#[tokio::test]
async fn fresh_database_applies_all_registered_kernel_migrations() {
    let store = AgentStore::connect_in_memory().await.expect("fresh store");
    let migration_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(store.pool())
        .await
        .expect("migration count");
    assert_eq!(migration_count, 42);

    let hardened_outbox_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('channel_outbox') WHERE name IN ('authorization_revision', 'account_config_revision', 'run_id', 'final_item_id', 'part_index', 'dispatched_at_ms')",
    )
    .fetch_one(store.pool())
    .await
    .expect("hardened outbox columns");
    assert_eq!(hardened_outbox_columns, 6);

    let provider_diagnostic_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('channel_provider_runtime_health') WHERE name IN ('last_handshake_at_ms', 'last_frame_at_ms', 'last_error_code')",
    )
    .fetch_one(store.pool())
    .await
    .expect("provider diagnostic columns");
    assert_eq!(provider_diagnostic_columns, 3);

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

    let enterprise_secret_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('integration_provider_accounts') WHERE lower(name) LIKE '%secret%' OR lower(name) LIKE '%credential_json%' OR lower(name) LIKE '%credential_body%'",
    )
    .fetch_one(store.pool())
    .await
    .expect("enterprise credential columns");
    assert_eq!(enterprise_secret_columns, 0);

    sqlx::query("INSERT INTO plugin_installations(plugin_id, manifest_json, content_hash, root_path, status, diagnostics_json, installed_at_ms, updated_at_ms) VALUES('wecom_app', '{}', 'hash', 'fixture', 'enabled', '[]', 1, 1)")
        .execute(store.pool())
        .await
        .expect("plugin installation");
    sqlx::query("INSERT INTO plugin_runtime_bindings(plugin_id, contribution_id, resource_kind, resource_id, runtime_revision, metadata_json, enabled, updated_at_ms) VALUES('wecom_app', 'messages', 'builtin_channel', 'wecom_app', 'revision', '{}', 1, 1)")
        .execute(store.pool())
        .await
        .expect("built-in channel binding");
}

#[tokio::test]
async fn task_requester_binding_is_idempotent_for_session_continuations() {
    let (store, session) = seeded_store().await;
    let now = now_ms();
    let task = TaskRunRecord {
        id: TaskRunId::from("task-requester-idempotent"),
        schedule_id: None,
        schedule_revision: None,
        trigger: TaskRunTrigger::Manual,
        scheduled_for_ms: Some(now),
        event_context: None,
        invocation_key: "requester-idempotent".into(),
        requester_session_id: Some(session.id.clone()),
        execution_session_id: None,
        run_id: None,
        status: TaskRunStatus::NeedsAttention,
        progress_percent: None,
        result_summary: None,
        error_code: Some("connector_revision_drift".into()),
        error_summary: Some("Connector revision drifted".into()),
        artifact_ids: Vec::new(),
        delivery_status: DeliveryStatus::NotRequested,
        delivery_error_code: None,
        created_at_ms: now,
        started_at_ms: None,
        finished_at_ms: Some(now),
        updated_at_ms: now,
    };
    store.create_task_run(&task).await.expect("create task");

    let rebound = store
        .bind_task_run_requester(&task.id, &session.id, now + 1)
        .await
        .expect("same requester is idempotent");
    assert_eq!(rebound.requester_session_id, Some(session.id));
    assert_eq!(rebound.updated_at_ms, now + 1);

    let error = store
        .bind_task_run_requester(&task.id, &SessionId::from("different-session"), now + 2)
        .await
        .expect_err("a different requester must remain fail closed");
    assert!(matches!(error, AgentStoreError::InvalidTaskRunTransition));
}
