use hachimi_protocol::{
    BrowserAutomationLeaseStatus, BrowserAutomationSurfaceKind, BrowserCapability,
    BrowserSessionId, BrowserSitePolicyUpdate, ComputerControlStatus, HostAccessDecision,
    HostAccessRequestStatus, HostPolicyDecision, RunStatus,
};

use super::{
    AgentStoreError,
    tests::{run, seeded_store},
};

#[tokio::test]
async fn browser_policy_is_exact_and_shared_across_surfaces() {
    let (store, session) = seeded_store().await;
    let run = run(&session, "host-policy-run");
    store
        .create_run_idempotent("test", "host-policy-run", &run)
        .await
        .expect("run");
    let policy = store
        .upsert_browser_site_policy(
            &BrowserSitePolicyUpdate {
                origin: "https://example.com".into(),
                decision: HostPolicyDecision::Allow,
                capabilities: vec![BrowserCapability::Observe, BrowserCapability::Act],
                expected_revision: None,
            },
            "https://example.com",
            false,
            false,
        )
        .await
        .expect("policy");
    assert_eq!(policy.revision, 1);
    for _surface in [
        BrowserAutomationSurfaceKind::Embedded,
        BrowserAutomationSurfaceKind::ExternalChrome,
    ] {
        assert_eq!(
            store
                .browser_host_policy_decision(
                    "https://example.com",
                    &session.id,
                    &run.id,
                    &[BrowserCapability::Observe, BrowserCapability::Act],
                    false,
                )
                .await
                .expect("decision"),
            HostPolicyDecision::Allow
        );
    }
    assert_eq!(
        store
            .embedded_browser_allowed_origins(&session.id, &run.id)
            .await
            .expect("CEF allowlist"),
        ["https://example.com".to_owned()]
    );
    assert_eq!(
        store
            .browser_host_policy_decision(
                "https://sub.example.com",
                &session.id,
                &run.id,
                &[BrowserCapability::Observe],
                false,
            )
            .await
            .expect("exact decision"),
        HostPolicyDecision::Ask
    );
}

#[tokio::test]
async fn browser_policy_revocation_changes_the_next_decision_immediately() {
    let (store, session) = seeded_store().await;
    let run = run(&session, "host-revoke-run");
    store
        .create_run_idempotent("test", "host-revoke-run", &run)
        .await
        .expect("run");
    store
        .upsert_browser_site_policy(
            &BrowserSitePolicyUpdate {
                origin: "https://revoked.example".into(),
                decision: HostPolicyDecision::Allow,
                capabilities: vec![BrowserCapability::Observe, BrowserCapability::Act],
                expected_revision: None,
            },
            "https://revoked.example",
            false,
            false,
        )
        .await
        .expect("allow policy");
    assert_eq!(
        store
            .browser_host_policy_decision(
                "https://revoked.example",
                &session.id,
                &run.id,
                &[BrowserCapability::Observe, BrowserCapability::Act],
                false,
            )
            .await
            .expect("allow decision"),
        HostPolicyDecision::Allow
    );
    assert!(
        store
            .embedded_browser_allowed_origins(&session.id, &run.id)
            .await
            .expect("allowed CEF origins")
            .contains(&"https://revoked.example".to_owned())
    );
    store
        .upsert_browser_site_policy(
            &BrowserSitePolicyUpdate {
                origin: "https://revoked.example".into(),
                decision: HostPolicyDecision::Block,
                capabilities: vec![BrowserCapability::Observe, BrowserCapability::Act],
                expected_revision: Some(1),
            },
            "https://revoked.example",
            false,
            false,
        )
        .await
        .expect("block policy");
    assert_eq!(
        store
            .browser_host_policy_decision(
                "https://revoked.example",
                &session.id,
                &run.id,
                &[BrowserCapability::Observe, BrowserCapability::Act],
                false,
            )
            .await
            .expect("block decision"),
        HostPolicyDecision::Block
    );
    assert!(
        !store
            .embedded_browser_allowed_origins(&session.id, &run.id)
            .await
            .expect("blocked CEF origins")
            .contains(&"https://revoked.example".to_owned())
    );
    assert!(
        store
            .remove_browser_site_policy("https://revoked.example")
            .await
            .expect("remove policy")
    );
    assert_eq!(
        store
            .browser_host_policy_decision(
                "https://revoked.example",
                &session.id,
                &run.id,
                &[BrowserCapability::Observe],
                false,
            )
            .await
            .expect("ask decision"),
        HostPolicyDecision::Ask
    );
}

#[tokio::test]
async fn session_grant_resolves_request_without_becoming_persistent() {
    let (store, session) = seeded_store().await;
    let run = run(&session, "host-request-run");
    store
        .create_run_idempotent("test", "host-request-run", &run)
        .await
        .expect("run");
    let request = store
        .create_browser_host_access_request(
            &session.id,
            &run.id,
            run.generation,
            "https://example.net",
            BrowserAutomationSurfaceKind::ExternalChrome,
            &[BrowserCapability::Observe, BrowserCapability::Act],
            false,
        )
        .await
        .expect("request");
    store
        .resolve_host_access_request(&request.id, HostAccessDecision::AllowSession, false)
        .await
        .expect("resolution");
    assert!(
        store
            .browser_site_policy("https://example.net")
            .await
            .expect("policy lookup")
            .is_none()
    );
    assert_eq!(
        store
            .browser_host_policy_decision(
                "https://example.net",
                &session.id,
                &run.id,
                &[BrowserCapability::Observe, BrowserCapability::Act],
                false,
            )
            .await
            .expect("decision"),
        HostPolicyDecision::Allow
    );
    assert!(
        store
            .embedded_browser_allowed_origins(&session.id, &run.id)
            .await
            .expect("session CEF origins")
            .contains(&"https://example.net".to_owned())
    );
}

#[tokio::test]
async fn embedded_session_grant_reaches_cef_and_global_block_overrides_it() {
    let (store, session) = seeded_store().await;
    let run = run(&session, "embedded-session-grant");
    store
        .create_run_idempotent("test", "embedded-session-grant", &run)
        .await
        .expect("run");
    sqlx::query(
        "INSERT INTO embedded_browser_site_permissions(id, origin, scope, scope_key, owner_session_id, owner_run_id, capabilities_json, allow_private_network, created_at_ms, expires_at_ms, updated_at_ms) VALUES('embedded-session-test', 'https://session.example', 'session', ?, ?, NULL, '[\"observe\",\"act\"]', 0, 1, NULL, 1)",
    )
    .bind(format!("session:{}", session.id))
    .bind(session.id.as_str())
    .execute(store.pool())
    .await
    .expect("embedded session permission");
    assert!(
        store
            .embedded_browser_allowed_origins(&session.id, &run.id)
            .await
            .expect("CEF allowlist")
            .contains(&"https://session.example".to_owned())
    );

    store
        .upsert_browser_site_policy(
            &BrowserSitePolicyUpdate {
                origin: "https://session.example".into(),
                decision: HostPolicyDecision::Block,
                capabilities: vec![BrowserCapability::Observe, BrowserCapability::Act],
                expected_revision: None,
            },
            "https://session.example",
            false,
            false,
        )
        .await
        .expect("global block");
    assert!(
        !store
            .embedded_browser_allowed_origins(&session.id, &run.id)
            .await
            .expect("blocked CEF allowlist")
            .contains(&"https://session.example".to_owned())
    );
}

#[tokio::test]
async fn private_network_allow_cannot_be_persisted_by_normal_settings() {
    let (store, _) = seeded_store().await;
    let error = store
        .upsert_browser_site_policy(
            &BrowserSitePolicyUpdate {
                origin: "http://127.0.0.1".into(),
                decision: HostPolicyDecision::Allow,
                capabilities: vec![BrowserCapability::Observe],
                expected_revision: None,
            },
            "http://127.0.0.1",
            true,
            false,
        )
        .await
        .expect_err("private allow must fail");
    assert!(matches!(
        error,
        AgentStoreError::PersistentPrivateHostPolicyDenied
    ));
}

#[tokio::test]
async fn terminal_run_expires_all_interactive_host_control() {
    let (store, session) = seeded_store().await;
    let run = run(&session, "terminal-host-cleanup");
    store
        .create_run_idempotent("test", "terminal-host-cleanup", &run)
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
    let lease = store
        .create_external_browser_automation_lease(
            &session.id,
            &run.id,
            run.generation,
            &BrowserSessionId::from("external-session-cleanup"),
            &[BrowserCapability::Observe, BrowserCapability::Act],
            super::now_ms().saturating_add(60_000),
        )
        .await
        .expect("lease");
    let request = store
        .create_browser_host_access_request(
            &session.id,
            &run.id,
            run.generation,
            "https://cleanup.example",
            BrowserAutomationSurfaceKind::ExternalChrome,
            &[BrowserCapability::Observe],
            false,
        )
        .await
        .expect("host request");
    store
        .set_computer_control_observation(
            &session.id,
            Some("app.cleanup"),
            Some("window-cleanup"),
            1,
            "controlling",
            Some(super::now_ms()),
            super::now_ms(),
        )
        .await
        .expect("computer projection");
    sqlx::query(
        "UPDATE computer_control_sessions SET owner_run_id = ?, owner_run_generation = ? WHERE session_id = ?",
    )
    .bind(run.id.as_str())
    .bind(i64::try_from(run.generation).unwrap_or(i64::MAX))
    .bind(session.id.as_str())
    .execute(store.pool())
    .await
    .expect("computer ownership");

    store
        .transition_run(&run.id, RunStatus::Succeeded, None)
        .await
        .expect("terminal run");

    assert_eq!(
        store
            .browser_automation_lease(&lease.id)
            .await
            .expect("expired lease")
            .status,
        BrowserAutomationLeaseStatus::Expired
    );
    assert_eq!(
        store
            .list_host_access_requests(Some(&session.id))
            .await
            .expect("expired requests")
            .into_iter()
            .find(|candidate| candidate.id == request.id)
            .expect("request")
            .status,
        HostAccessRequestStatus::Expired
    );
    assert_eq!(
        store
            .list_session_computer_control_sessions(&session.id)
            .await
            .expect("computer controls")[0]
            .status,
        ComputerControlStatus::Stopped
    );
}
