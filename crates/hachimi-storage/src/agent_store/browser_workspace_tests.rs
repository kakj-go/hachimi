use hachimi_protocol::{
    BrowserAutomationLeaseStatus, BrowserAutomationSurfaceKind, BrowserCapability,
    BrowserPermissionDecision, BrowserPermissionRequestStatus, BrowserSessionId,
    BrowserWorkspaceRuntimeState, EmbeddedBrowserPermissionScope, EmbeddedBrowserSettingsUpdate,
};

use super::{
    AgentStoreError, BrowserDownloadRuntimeUpdate, BrowserTabRuntimeUpdate,
    tests::{run, seeded_store},
};

#[tokio::test]
async fn workspace_reuses_the_session_and_manages_real_tabs() {
    let (store, session) = seeded_store().await;
    let workspace = store
        .get_or_create_browser_workspace(&session.id, Some("https://example.com/start#fragment"))
        .await
        .expect("create browser workspace");
    assert_eq!(workspace.owner_session_id, session.id);
    assert_eq!(workspace.tabs.len(), 1);
    assert_eq!(workspace.tabs[0].url, "https://example.com/start");
    assert_eq!(workspace.active_tab_id, workspace.tabs[0].id);

    let reused = store
        .get_or_create_browser_workspace(&workspace.owner_session_id, Some("https://ignored.test"))
        .await
        .expect("reuse browser workspace");
    assert_eq!(reused.id, workspace.id);
    assert_eq!(reused.tabs[0].url, "https://example.com/start");

    let with_second_tab = store
        .create_browser_tab(
            &workspace.id,
            workspace.revision,
            Some("https://www.rust-lang.org/"),
        )
        .await
        .expect("create second tab");
    assert_eq!(with_second_tab.tabs.len(), 2);
    assert_ne!(with_second_tab.active_tab_id, workspace.active_tab_id);

    let stale = store
        .activate_browser_tab(&workspace.id, &workspace.active_tab_id, workspace.revision)
        .await
        .expect_err("stale workspace revision must fail");
    assert!(matches!(
        stale,
        AgentStoreError::BrowserWorkspaceRevisionConflict
    ));

    let activated = store
        .activate_browser_tab(
            &workspace.id,
            &workspace.active_tab_id,
            with_second_tab.revision,
        )
        .await
        .expect("activate original tab");
    assert_eq!(activated.active_tab_id, workspace.active_tab_id);

    let closed = store
        .close_browser_tab(&workspace.id, &workspace.active_tab_id, activated.revision)
        .await
        .expect("close active tab");
    assert_eq!(closed.tabs.len(), 1);
    let last_tab_id = closed.tabs[0].id.clone();
    let replacement = store
        .close_browser_tab(&workspace.id, &last_tab_id, closed.revision)
        .await
        .expect("closing the last tab creates a blank replacement");
    assert_eq!(replacement.tabs.len(), 1);
    assert_ne!(replacement.tabs[0].id, last_tab_id);
    assert_eq!(replacement.tabs[0].url, "about:blank");
}

#[tokio::test]
async fn cef_download_progress_is_durable_and_hashes_completed_files() {
    let (store, session) = seeded_store().await;
    let workspace = store
        .get_or_create_browser_workspace(&session.id, None)
        .await
        .expect("workspace");
    let tab_id = workspace.active_tab_id.clone();
    let pending = store
        .upsert_browser_download(BrowserDownloadRuntimeUpdate {
            runtime_id: 7,
            tab_id: tab_id.clone(),
            source_url: "https://example.com/report.txt".into(),
            suggested_name: "report.txt".into(),
            destination: None,
            received_bytes: 128,
            total_bytes: Some(256),
            complete: false,
            cancelled: false,
            interrupted: false,
        })
        .await
        .expect("pending download");
    assert_eq!(
        pending.status,
        hachimi_protocol::BrowserDownloadStatus::InProgress
    );

    let directory = tempfile::tempdir().expect("download directory");
    let path = directory.path().join("report.txt");
    std::fs::write(&path, b"downloaded report\n").expect("download file");
    let completed = store
        .upsert_browser_download(BrowserDownloadRuntimeUpdate {
            runtime_id: 7,
            tab_id,
            source_url: "https://example.com/report.txt".into(),
            suggested_name: "report.txt".into(),
            destination: Some(path.to_string_lossy().into_owned()),
            received_bytes: 18,
            total_bytes: Some(18),
            complete: true,
            cancelled: false,
            interrupted: false,
        })
        .await
        .expect("completed download");
    assert_eq!(completed.id, pending.id);
    assert_eq!(
        completed.status,
        hachimi_protocol::BrowserDownloadStatus::Completed
    );
    assert_eq!(completed.sha256.as_deref().map(str::len), Some(64));
    let restored = store
        .browser_downloads(&workspace.id, 10)
        .await
        .expect("restored downloads");
    assert_eq!(restored, vec![completed]);
}

#[tokio::test]
async fn runtime_updates_history_and_reconciles_after_restart() {
    let (store, session) = seeded_store().await;
    let workspace = store
        .get_or_create_browser_workspace(&session.id, None)
        .await
        .expect("browser workspace");
    let tab_id = workspace.active_tab_id.clone();

    let ready = store
        .set_browser_workspace_runtime(&workspace.id, BrowserWorkspaceRuntimeState::Ready)
        .await
        .expect("mark runtime ready");
    let updated = store
        .update_browser_tab_runtime(
            &workspace.id,
            &tab_id,
            BrowserTabRuntimeUpdate {
                url: Some("https://example.com/docs#one".into()),
                title: Some("Example docs".into()),
                loading: Some(true),
                can_go_back: Some(true),
                runtime_loaded: Some(true),
                user_input: true,
                ..BrowserTabRuntimeUpdate::default()
            },
        )
        .await
        .expect("update browser tab");
    assert!(updated.revision > ready.revision);
    assert!(updated.tabs[0].loading);
    assert!(updated.tabs[0].runtime_loaded);
    assert_eq!(updated.tabs[0].input_epoch, 2);

    store
        .update_browser_tab_runtime(
            &workspace.id,
            &tab_id,
            BrowserTabRuntimeUpdate {
                url: Some("https://example.com/docs#two".into()),
                title: Some("Updated docs".into()),
                ..BrowserTabRuntimeUpdate::default()
            },
        )
        .await
        .expect("visit canonical URL again");
    let history = store.browser_history("example", 10).await.expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].url, "https://example.com/docs");
    assert_eq!(history[0].title, "Updated docs");
    assert_eq!(history[0].visit_count, 2);

    store
        .reconcile_browser_startup()
        .await
        .expect("startup reconciliation");
    let reconciled = store
        .browser_workspace(&workspace.id)
        .await
        .expect("reconciled workspace");
    assert_eq!(
        reconciled.runtime_state,
        BrowserWorkspaceRuntimeState::Dormant
    );
    assert!(!reconciled.tabs[0].loading);
    assert!(!reconciled.tabs[0].runtime_loaded);
}

#[tokio::test]
async fn lease_revision_fencing_stop_and_restart_expiry_are_durable() {
    let (store, session) = seeded_store().await;
    let run = run(&session, "browser-lease-run");
    store
        .create_run_idempotent("user", "browser-lease-run", &run)
        .await
        .expect("run");
    let workspace = store
        .get_or_create_browser_workspace(&session.id, None)
        .await
        .expect("workspace");
    let lease = store
        .create_browser_automation_lease(
            BrowserAutomationSurfaceKind::Embedded,
            Some(&workspace.id),
            Some(&workspace.active_tab_id),
            &session.id,
            &run.id,
            run.generation,
            &[BrowserCapability::Observe, BrowserCapability::Act],
            super::now_ms() + 60_000,
        )
        .await
        .expect("create lease");
    assert_eq!(lease.owner_session_id, session.id);
    assert_eq!(lease.owner_run_id, run.id);
    assert_eq!(lease.run_generation, run.generation);

    let visible = store
        .browser_workspace(&workspace.id)
        .await
        .expect("workspace exposes active lease");
    assert_eq!(
        visible.automation_lease.as_ref().map(|value| value.status),
        Some(BrowserAutomationLeaseStatus::Active)
    );
    let taken_over = store
        .suspend_active_browser_automation_for_tab(&workspace.active_tab_id)
        .await
        .expect("suspend from user input")
        .expect("active lease must be suspended");
    assert_eq!(
        taken_over
            .automation_lease
            .as_ref()
            .map(|value| value.status),
        Some(BrowserAutomationLeaseStatus::Suspended)
    );
    let resumed = store
        .transition_browser_workspace_automation(
            &workspace.id,
            taken_over.revision,
            BrowserAutomationLeaseStatus::Suspended,
            BrowserAutomationLeaseStatus::Active,
        )
        .await
        .expect("resume Agent control");
    assert_eq!(
        resumed.automation_lease.as_ref().map(|value| value.status),
        Some(BrowserAutomationLeaseStatus::Active)
    );

    let with_task_tab = store
        .create_browser_tab(
            &workspace.id,
            resumed.revision,
            Some("https://example.com/task"),
        )
        .await
        .expect("task tab");
    let rebound = store
        .update_browser_automation_lease_target(
            &lease.id,
            resumed
                .automation_lease
                .as_ref()
                .expect("resumed lease")
                .revision,
            &workspace.id,
            &with_task_tab.active_tab_id,
        )
        .await
        .expect("rebind task tab");
    assert_eq!(rebound.tab_id, Some(with_task_tab.active_tab_id));

    let suspended = store
        .set_browser_automation_lease_status(
            &rebound.id,
            rebound.revision,
            BrowserAutomationLeaseStatus::Suspended,
        )
        .await
        .expect("suspend lease");
    let stale = store
        .set_browser_automation_lease_status(
            &rebound.id,
            rebound.revision,
            BrowserAutomationLeaseStatus::Expired,
        )
        .await
        .expect_err("stale lease mutation must fail");
    assert!(matches!(
        stale,
        AgentStoreError::BrowserAutomationLeaseRevisionConflict
    ));

    store
        .reconcile_browser_startup()
        .await
        .expect("reconcile browser state");
    let expired = store
        .browser_automation_lease(&suspended.id)
        .await
        .expect("read expired lease");
    assert_eq!(expired.status, BrowserAutomationLeaseStatus::Expired);
    assert!(expired.revision > suspended.revision);
}

#[tokio::test]
async fn embedded_site_permissions_are_scoped_and_pending_requests_are_deduplicated() {
    let (store, session) = seeded_store().await;
    let run = run(&session, "embedded-site-permission-run");
    store
        .create_run_idempotent("user", "embedded-site-permission-run", &run)
        .await
        .expect("run");
    let workspace = store
        .get_or_create_browser_workspace(&session.id, None)
        .await
        .expect("workspace");
    let first = store
        .create_embedded_browser_permission_request(
            &workspace.id,
            &workspace.active_tab_id,
            None,
            &session.id,
            &run.id,
            run.generation,
            "https://example.com",
            false,
            workspace.tabs[0].revision,
        )
        .await
        .expect("permission request");
    let duplicate = store
        .create_embedded_browser_permission_request(
            &workspace.id,
            &workspace.active_tab_id,
            None,
            &session.id,
            &run.id,
            run.generation,
            "https://example.com",
            false,
            workspace.tabs[0].revision,
        )
        .await
        .expect("deduplicated request");
    assert_eq!(duplicate.id, first.id);

    let resolved = store
        .resolve_embedded_browser_permission_request(
            &first.id,
            BrowserPermissionDecision::AllowSession,
        )
        .await
        .expect("resolve permission");
    assert_eq!(resolved.status, BrowserPermissionRequestStatus::Allowed);
    let permission = store
        .embedded_browser_site_permission("https://example.com", &session.id, &run.id, false)
        .await
        .expect("permission lookup")
        .expect("session permission");
    assert_eq!(permission.scope, EmbeddedBrowserPermissionScope::Session);
    assert!(
        store
            .revoke_embedded_browser_site_permission(&permission.id)
            .await
            .expect("revoke permission")
    );
    assert!(
        store
            .embedded_browser_site_permission("https://example.com", &session.id, &run.id, false,)
            .await
            .expect("revoked lookup")
            .is_none()
    );
}

#[tokio::test]
async fn external_chrome_uses_the_same_durable_lease_contract() {
    let (store, session) = seeded_store().await;
    let run = run(&session, "external-browser-lease-run");
    store
        .create_run_idempotent("user", "external-browser-lease-run", &run)
        .await
        .expect("run");
    let browser_session_id = BrowserSessionId::from("chrome-extension-session");
    let lease = store
        .create_external_browser_automation_lease(
            &session.id,
            &run.id,
            run.generation,
            &browser_session_id,
            &[BrowserCapability::Observe, BrowserCapability::Act],
            super::now_ms() + 60_000,
        )
        .await
        .expect("external lease");
    assert_eq!(lease.surface, BrowserAutomationSurfaceKind::ExternalChrome);
    assert_eq!(lease.workspace_id, None);
    assert_eq!(
        store
            .external_browser_session_for_lease(&lease.id)
            .await
            .expect("external lease target"),
        Some(browser_session_id)
    );
}

#[tokio::test]
async fn embedded_browser_settings_are_revision_fenced_and_history_can_be_cleared() {
    let (store, session) = seeded_store().await;
    let defaults = store
        .embedded_browser_settings(false)
        .await
        .expect("default browser settings");
    assert_eq!(defaults.download_directory, None);
    assert!(!defaults.ask_where_to_save_downloads);
    assert!(!defaults.full_cdp_access);
    assert!(!defaults.full_cdp_access_allowed);

    let directory = tempfile::tempdir().expect("download directory");
    let updated = store
        .update_embedded_browser_settings(
            &EmbeddedBrowserSettingsUpdate {
                download_directory: Some(directory.path().to_string_lossy().into_owned()),
                ask_where_to_save_downloads: true,
                full_cdp_access: true,
                expected_revision: defaults.revision,
            },
            true,
        )
        .await
        .expect("update browser settings");
    assert!(updated.ask_where_to_save_downloads);
    assert!(updated.full_cdp_access);
    assert!(updated.full_cdp_access_allowed);
    assert!(updated.revision > defaults.revision);

    let stale = store
        .update_embedded_browser_settings(
            &EmbeddedBrowserSettingsUpdate {
                download_directory: None,
                ask_where_to_save_downloads: false,
                full_cdp_access: false,
                expected_revision: defaults.revision,
            },
            true,
        )
        .await
        .expect_err("stale browser settings must fail");
    assert!(matches!(
        stale,
        AgentStoreError::EmbeddedBrowserSettingsRevisionConflict
    ));

    let workspace = store
        .get_or_create_browser_workspace(&session.id, None)
        .await
        .expect("workspace");
    store
        .update_browser_tab_runtime(
            &workspace.id,
            &workspace.active_tab_id,
            BrowserTabRuntimeUpdate {
                url: Some("https://example.com/settings-test".into()),
                title: Some("Settings test".into()),
                ..BrowserTabRuntimeUpdate::default()
            },
        )
        .await
        .expect("record browser history");
    assert_eq!(
        store.browser_history("", 10).await.expect("history").len(),
        1
    );
    assert_eq!(
        store
            .clear_embedded_browser_history()
            .await
            .expect("clear history"),
        1
    );
    assert!(
        store
            .browser_history("", 10)
            .await
            .expect("history")
            .is_empty()
    );
}
