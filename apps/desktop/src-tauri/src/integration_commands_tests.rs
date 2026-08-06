use super::*;

fn test_dimension(ok: bool, code: &str) -> IntegrationProbeDimension {
    IntegrationProbeDimension {
        ok,
        result_code: code.into(),
        diagnostic: (!ok).then(|| code.into()),
    }
}

#[test]
fn probe_dimensions_keep_ingress_independent_from_api_authentication() {
    let ingress = ingress_probe_dimension(true, true, true);
    let egress = egress_probe_dimension(true, true, false, true);
    let api = api_probe_dimension(true, false);
    assert_eq!(ingress.result_code, "ingress_healthy");
    assert!(ingress.ok);
    assert_eq!(egress.result_code, "egress_authentication_failed");
    assert!(!egress.ok);
    assert_eq!(api.result_code, "api_authentication_failed");
    assert!(!api.ok);
}

#[tokio::test]
async fn probe_snapshot_survives_store_reconnect() {
    let root = tempfile::tempdir().expect("temporary database root");
    let database = root.path().join("agent.db");
    let store = hachimi_storage::AgentStore::connect(&database)
        .await
        .expect("store");
    sqlx::query("INSERT INTO integration_provider_accounts(id, provider_id, display_name, tenant_key, tenant_identity_hash, transport, state, api_access_enabled, messaging_enabled, config_json, credential_revision, config_revision, consecutive_failures, created_at_ms, updated_at_ms) VALUES('account-1', 'wecom_app', 'Account', 'tenant', 'tenant-hash', 'encrypted_callback', 'needs_attention', 1, 1, '{}', 1, 1, 0, 10, 10)")
        .execute(store.pool())
        .await
        .expect("account");
    let snapshot = IntegrationAccountProbeSnapshot {
        credential: test_dimension(false, "credential_authentication_failed"),
        ingress: test_dimension(true, "ingress_healthy"),
        egress: test_dimension(false, "egress_authentication_failed"),
        api: test_dimension(false, "api_authentication_failed"),
        probed_at_ms: 42,
    };
    store_probe_snapshot(store.pool(), "account-1", &snapshot)
        .await
        .expect("snapshot");
    drop(store);

    let reopened = hachimi_storage::AgentStore::connect(&database)
        .await
        .expect("reopened store");
    let loaded = load_probe_snapshot(reopened.pool(), "account-1")
        .await
        .expect("loaded snapshot");
    assert_eq!(loaded, Some(snapshot));
}

#[test]
fn cleanup_reference_accepts_scoped_conversation_tokens() {
    let digest = "a".repeat(64);
    let secret_ref = format!("keyring:integration:wechat_ilink:account-1:conversation:{digest}");
    let expected = format!("wechat_ilink:account-1:conversation:{digest}");
    assert_eq!(
        cleanup_keyring_username(&secret_ref, "account-1").expect("valid reference"),
        Some(expected.as_str())
    );
    assert_eq!(
        cleanup_keyring_username(&secret_ref, "another-account").expect("scoped reference"),
        None
    );
}

#[tokio::test]
async fn account_removal_queues_all_keyring_references_before_cascade() {
    let store = hachimi_storage::AgentStore::connect_in_memory()
        .await
        .expect("store");
    sqlx::query("INSERT INTO integration_provider_accounts(id, provider_id, display_name, tenant_key, tenant_identity_hash, transport, state, credential_ref, messaging_enabled, created_at_ms, updated_at_ms) VALUES('account-1', 'wechat_ilink', 'iLink', 'tenant', 'tenant-hash', 'qr_long_poll', 'healthy', 'keyring:integration:wechat_ilink:account-1:primary', 1, 1, 1)")
        .execute(store.pool())
        .await
        .expect("account");
    let conversation_ref = format!(
        "keyring:integration:wechat_ilink:account-1:conversation:{}",
        "b".repeat(64)
    );
    let media_ref = format!(
        "keyring:integration:wechat_ilink:account-1:media:{}",
        "c".repeat(64)
    );
    sqlx::query("INSERT INTO channel_route_secrets(account_id, conversation_hash, secret_ref, updated_at_ms) VALUES('account-1', 'conversation', ?, 1)")
        .bind(&conversation_ref)
        .execute(store.pool())
        .await
        .expect("route secret");
    sqlx::query("INSERT INTO channel_media_secrets(platform, account_id, event_id, remote_id, secret_ref, secret_fingerprint, created_at_ms) VALUES('wechat_ilink', 'account-1', 'event', 'media', ?, 'fingerprint', 1)")
        .bind(&media_ref)
        .execute(store.pool())
        .await
        .expect("media secret");
    stage_account_removal(
        store.pool(),
        "account-1",
        Some("keyring:integration:wechat_ilink:account-1:primary"),
    )
    .await
    .expect("stage removal");
    let queued: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM integration_secret_cleanup_queue WHERE account_id = 'account-1'",
    )
    .fetch_one(store.pool())
    .await
    .expect("cleanup count");
    let account_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM integration_provider_accounts WHERE id = 'account-1')",
    )
    .fetch_one(store.pool())
    .await
    .expect("account state");
    assert_eq!(queued, 3);
    assert!(!account_exists);
}
