use hachimi_gateway::{CLAIM_TTL_MS, ChannelDeliveryOutcome, GatewayHost, ReactiveDeliverySource};
use hachimi_protocol::{
    ChannelAccessPolicyUpsert, ChannelAccountState, ChannelActor, ChannelAuthorizationTarget,
    ChannelChatKind, ChannelConversationAddress, ChannelDmPolicy, ChannelEventKey, ChannelGrant,
    ChannelMentionPolicy, ChannelMessagePart, ChannelOutboundPayload, ChannelPairingCodeRequest,
    ChannelProviderAccount, ChannelProviderHealthState, ChannelTopicPolicy, DeliveryAttemptStatus,
    IngressStatus, IntegrationProviderId, RemoteMediaDescriptor, RunId, SessionId,
    VerifiedChannelMessage,
};
use hachimi_storage::AgentStore;
use serde_json::json;

fn address() -> ChannelConversationAddress {
    ChannelConversationAddress {
        provider_id: "dingtalk".into(),
        account_id: "account-1".into(),
        tenant_key: "tenant-1".into(),
        chat_kind: ChannelChatKind::Dm,
        chat_id: "actor-1".into(),
        topic_id: None,
    }
}

fn message(
    external_message_id: &str,
    text: impl Into<String>,
    received_at_ms: i64,
) -> VerifiedChannelMessage {
    VerifiedChannelMessage {
        event_key: ChannelEventKey {
            provider_id: "dingtalk".into(),
            account_id: "account-1".into(),
            external_message_id: external_message_id.into(),
        },
        address: address(),
        actor: ChannelActor {
            external_id: "actor-1".into(),
            display_name: Some("Actor One".into()),
            is_bot: false,
        },
        parts: vec![ChannelMessagePart::Text { text: text.into() }],
        mentions: Vec::new(),
        quote: None,
        provider_context: json!({}),
        received_at_ms,
    }
}

async fn formal_gateway(dm_policy: &str) -> (AgentStore, GatewayHost) {
    let store = AgentStore::connect_in_memory().await.expect("store");
    sqlx::query("INSERT INTO integration_provider_accounts(id, provider_id, display_name, tenant_key, tenant_identity_hash, transport, state, diagnostic, connector_account_id, credential_ref, credential_fingerprint, api_access_enabled, messaging_enabled, config_json, credential_revision, config_revision, last_event_at_ms, last_delivery_at_ms, next_reconnect_at_ms, consecutive_failures, created_at_ms, updated_at_ms) VALUES('account-1', 'dingtalk', 'DingTalk', 'tenant-1', 'tenant-hash', 'stream', 'healthy', NULL, NULL, NULL, NULL, 0, 1, '{}', 1, 1, NULL, NULL, NULL, 0, 1, 1)")
        .execute(store.pool())
        .await
        .expect("account");
    sqlx::query("INSERT INTO channel_access_policies(account_id, dm_policy, allowlist_actor_ids_json, grant_ceiling_json, revision, updated_at_ms) VALUES('account-1', ?, '[]', ?, 1, 1)")
        .bind(dm_policy)
        .bind(serde_json::to_string(&ChannelGrant::default()).expect("grant"))
        .execute(store.pool())
        .await
        .expect("policy");
    let gateway = GatewayHost::new(store.clone(), vec!["dingtalk".into()]);
    (store, gateway)
}

#[tokio::test]
async fn gateway_health_exposes_the_latest_persisted_heartbeat() {
    let store = AgentStore::connect_in_memory().await.expect("store");
    let gateway = GatewayHost::new(store, vec!["dingtalk".into()]);
    let now = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("wall clock")
            .as_millis(),
    )
    .expect("timestamp");
    gateway.heartbeat(42, now).await.expect("persist heartbeat");
    let health = gateway.health().await.expect("gateway health");
    assert!(health.running);
    assert_eq!(health.last_heartbeat_ms, Some(now));

    gateway
        .heartbeat(42, now - 15_001)
        .await
        .expect("persist stale heartbeat");
    assert!(!gateway.health().await.expect("stale health").running);
}

#[tokio::test]
async fn provider_account_reconciliation_is_revision_driven_and_removes_stale_runtimes() {
    let store = AgentStore::connect_in_memory().await.expect("store");
    let builtins = hachimi_gateway::local_builtin_providers(store.clone(), &"x".repeat(32))
        .expect("built-in providers");
    let gateway = GatewayHost::with_registry(store.clone(), builtins.registry);
    gateway
        .bootstrap_provider_accounts(&[ChannelProviderAccount {
            id: "ding-account".into(),
            provider_id: "dingtalk".into(),
            display_name: "DingTalk".into(),
            tenant_key: "tenant-1".into(),
            credential_ref: Some("keyring:integration:dingtalk:ding-account:primary".into()),
            enabled: true,
            state: ChannelAccountState::Healthy,
            config: json!({}),
            credential_revision: 1,
            config_revision: 1,
        }])
        .await
        .expect("account");
    assert!(!gateway.reconcile_provider_accounts().await.expect("no-op"));

    sqlx::query("UPDATE channel_provider_accounts SET display_name = 'DingTalk 2', config_revision = 2 WHERE id = 'ding-account'")
        .execute(store.pool())
        .await
        .expect("revision update");
    assert!(gateway.reconcile_provider_accounts().await.expect("reload"));
    let health = gateway.provider_health().await.expect("health");
    let dingtalk = health
        .iter()
        .find(|health| health.provider_id == "dingtalk")
        .expect("dingtalk health");
    assert_eq!(dingtalk.config_revision, 2);

    sqlx::query("DELETE FROM channel_provider_accounts WHERE id = 'ding-account'")
        .execute(store.pool())
        .await
        .expect("remove account");
    assert!(gateway.reconcile_provider_accounts().await.expect("remove"));
    let health = gateway
        .provider_health()
        .await
        .expect("health after removal");
    let dingtalk = health
        .iter()
        .find(|health| health.provider_id == "dingtalk")
        .expect("dingtalk health after removal");
    assert_eq!(dingtalk.state, ChannelProviderHealthState::Disabled);
}

#[tokio::test]
async fn provider_runtime_diagnostics_survive_gateway_restart() {
    let store = AgentStore::connect_in_memory().await.expect("store");
    let builtins = hachimi_gateway::local_builtin_providers(store.clone(), &"x".repeat(32))
        .expect("built-in providers");
    let gateway = GatewayHost::with_registry(store.clone(), builtins.registry);
    gateway
        .bootstrap_provider_accounts(&[ChannelProviderAccount {
            id: "diagnostic-account".into(),
            provider_id: "dingtalk".into(),
            display_name: "DingTalk diagnostics".into(),
            tenant_key: "tenant-diagnostic".into(),
            credential_ref: Some("keyring:integration:dingtalk:diagnostic-account:primary".into()),
            enabled: true,
            state: ChannelAccountState::Healthy,
            config: json!({}),
            credential_revision: 1,
            config_revision: 3,
        }])
        .await
        .expect("account");
    gateway
        .persist_provider_health(1_000)
        .await
        .expect("persist health");

    let persisted = gateway
        .persisted_provider_health()
        .await
        .expect("restore health");
    let health = persisted
        .iter()
        .find(|health| health.account_id.as_deref() == Some("diagnostic-account"))
        .expect("diagnostic account health");
    assert!(health.last_handshake_at_ms.is_some());
    assert_eq!(health.last_frame_at_ms, None);
    assert_eq!(health.last_error_code, None);
    assert_eq!(health.config_revision, 3);
}

#[tokio::test]
async fn delivery_is_typed_idempotent_and_retryable() {
    let (_store, gateway) = formal_gateway("open").await;
    let payload = ChannelOutboundPayload {
        parts: vec![ChannelMessagePart::Text {
            text: "done".into(),
        }],
        reply_to_external_message_id: Some("event-1".into()),
    };
    assert!(matches!(
        gateway
            .enqueue_delivery(address(), "unscoped-proactive", payload.clone(), 8)
            .await,
        Err(hachimi_gateway::GatewayError::RouteNotAllowed)
    ));
    let incoming = message("event-delivery-1", "request", 9);
    gateway
        .ingest_verified(&incoming)
        .await
        .expect("reactive ingress");
    let first = gateway
        .enqueue_reactive_delivery(
            ReactiveDeliverySource {
                event_key: &incoming.event_key,
                run_id: None,
                final_item_id: "assistant-1",
            },
            address(),
            0,
            payload.clone(),
            10,
        )
        .await
        .expect("enqueue");
    let replay = gateway
        .enqueue_reactive_delivery(
            ReactiveDeliverySource {
                event_key: &incoming.event_key,
                run_id: None,
                final_item_id: "assistant-1",
            },
            address(),
            0,
            payload,
            10,
        )
        .await
        .expect("replay");
    assert_eq!(first.id, replay.id);
    let claimed = gateway
        .claim_next_delivery(10)
        .await
        .expect("claim")
        .expect("delivery");
    let retry = gateway
        .finish_delivery(
            &claimed.id,
            ChannelDeliveryOutcome {
                delivered: false,
                retryable: true,
                indeterminate: false,
                result_code: "offline".into(),
                provider_receipt: None,
            },
            10,
        )
        .await
        .expect("retry");
    assert_eq!(retry.status, DeliveryAttemptStatus::RetryScheduled);
    assert_eq!(retry.next_attempt_at_ms, Some(1_010));
}

#[tokio::test]
async fn queued_reply_is_blocked_when_its_authorization_revision_changes() {
    let (store, gateway) = formal_gateway("pairing").await;
    let pairing = gateway
        .create_pairing_code(
            ChannelPairingCodeRequest {
                account_id: "account-1".into(),
                target: ChannelAuthorizationTarget::DmIdentity,
                group_history_policy: None,
                topic_policy: ChannelTopicPolicy::InheritGroup,
                mention_policy: ChannelMentionPolicy::Disabled,
                grant: ChannelGrant::default(),
            },
            10,
        )
        .await
        .expect("pairing");
    let connected = message("authorized-reply", format!("/connect {}", pairing.code), 20);
    gateway
        .ingest_verified(&connected)
        .await
        .expect("authorized ingress");
    let delivery = gateway
        .enqueue_reactive_text_delivery(
            ReactiveDeliverySource {
                event_key: &connected.event_key,
                run_id: None,
                final_item_id: "control-response",
            },
            address(),
            "connected",
            None,
            21,
        )
        .await
        .expect("reply");
    sqlx::query(
        "UPDATE channel_authorizations SET revision = revision + 1 WHERE account_id = 'account-1'",
    )
    .execute(store.pool())
    .await
    .expect("authorization update");
    assert!(
        gateway
            .claim_next_delivery(21)
            .await
            .expect("claim")
            .is_none()
    );
    let status: (String, String) =
        sqlx::query_as("SELECT status, error_code FROM channel_outbox WHERE id = ?")
            .bind(delivery.id.as_str())
            .fetch_one(store.pool())
            .await
            .expect("delivery status");
    assert_eq!(
        status,
        (
            "permanent_failure".into(),
            "delivery_authorization_stale".into()
        )
    );
}

#[tokio::test]
async fn outbox_recovery_retries_only_before_external_dispatch() {
    let (store, gateway) = formal_gateway("open").await;
    let first_message = message("outbox-safe", "request", 10);
    gateway
        .ingest_verified(&first_message)
        .await
        .expect("first ingress");
    let first = gateway
        .enqueue_reactive_text_delivery(
            ReactiveDeliverySource {
                event_key: &first_message.event_key,
                run_id: None,
                final_item_id: "assistant-safe",
            },
            address(),
            "reply",
            None,
            11,
        )
        .await
        .expect("first enqueue");
    gateway
        .claim_next_delivery(11)
        .await
        .expect("claim")
        .expect("first delivery");
    gateway
        .reconcile_startup(11 + CLAIM_TTL_MS)
        .await
        .expect("safe recovery");
    let first_status: String = sqlx::query_scalar("SELECT status FROM channel_outbox WHERE id = ?")
        .bind(first.id.as_str())
        .fetch_one(store.pool())
        .await
        .expect("first status");
    assert_eq!(first_status, "retry_scheduled");

    sqlx::query(
        "UPDATE channel_outbox SET status = 'delivered', next_attempt_at_ms = NULL WHERE id = ?",
    )
    .bind(first.id.as_str())
    .execute(store.pool())
    .await
    .expect("finish first fixture");
    let second_message = message("outbox-unknown", "request", 20);
    gateway
        .ingest_verified(&second_message)
        .await
        .expect("second ingress");
    let second = gateway
        .enqueue_reactive_text_delivery(
            ReactiveDeliverySource {
                event_key: &second_message.event_key,
                run_id: None,
                final_item_id: "assistant-unknown",
            },
            address(),
            "reply",
            None,
            21,
        )
        .await
        .expect("second enqueue");
    let claimed = gateway
        .claim_next_delivery(21)
        .await
        .expect("claim")
        .expect("second delivery");
    assert_eq!(claimed.id, second.id);
    gateway
        .mark_delivery_dispatched(&second.id, 22)
        .await
        .expect("dispatch marker");
    gateway
        .reconcile_startup(21 + CLAIM_TTL_MS)
        .await
        .expect("indeterminate recovery");
    let second_status: String =
        sqlx::query_scalar("SELECT status FROM channel_outbox WHERE id = ?")
            .bind(second.id.as_str())
            .fetch_one(store.pool())
            .await
            .expect("second status");
    assert_eq!(second_status, "indeterminate");
}

#[tokio::test]
async fn changed_payload_for_existing_event_is_audited_without_message_text() {
    let (store, gateway) = formal_gateway("open").await;
    gateway
        .ingest_verified(&message("audit-event", "original secret text", 10))
        .await
        .expect("first ingress");
    assert!(matches!(
        gateway
            .ingest_verified(&message("audit-event", "changed secret text", 11))
            .await,
        Err(hachimi_gateway::GatewayError::PayloadConflict)
    ));
    let summary: String = sqlx::query_scalar("SELECT target_summary FROM audit_events WHERE operation = 'channel.ingress_payload_conflict'")
        .fetch_one(store.pool())
        .await
        .expect("security audit");
    assert!(!summary.contains("secret text"));
    assert!(summary.contains("storedPayloadHash"));
}

#[tokio::test]
async fn pairing_cooldown_resets_and_ilink_group_scope_is_rejected() {
    let (store, gateway) = formal_gateway("pairing").await;
    for timestamp in 10..15 {
        assert!(
            gateway
                .consume_pairing_code(
                    &message("attempt", "ignored", timestamp),
                    "BADCODE",
                    timestamp
                )
                .await
                .is_err()
        );
    }
    let cooldown_until: i64 = sqlx::query_scalar("SELECT cooldown_until_ms FROM channel_pairing_attempts WHERE account_id = 'account-1' AND actor_id = 'actor-1'")
        .fetch_one(store.pool())
        .await
        .expect("cooldown");
    assert!(
        gateway
            .consume_pairing_code(
                &message("after-cooldown", "ignored", cooldown_until + 1),
                "BADCODE",
                cooldown_until + 1,
            )
            .await
            .is_err()
    );
    let attempt: (i64, Option<i64>) = sqlx::query_as("SELECT failure_count, cooldown_until_ms FROM channel_pairing_attempts WHERE account_id = 'account-1' AND actor_id = 'actor-1'")
        .fetch_one(store.pool())
        .await
        .expect("reset attempt");
    assert_eq!(attempt, (1, None));

    sqlx::query("UPDATE integration_provider_accounts SET provider_id = 'wechat_ilink', transport = 'qr_long_poll' WHERE id = 'account-1'")
        .execute(store.pool())
        .await
        .expect("ilink account");
    let group_pairing = gateway
        .create_pairing_code(
            ChannelPairingCodeRequest {
                account_id: "account-1".into(),
                target: ChannelAuthorizationTarget::GroupConversation,
                group_history_policy: Some(hachimi_protocol::ChannelGroupHistoryPolicy::Shared),
                topic_policy: ChannelTopicPolicy::InheritGroup,
                mention_policy: ChannelMentionPolicy::Required,
                grant: ChannelGrant::default(),
            },
            cooldown_until + 2,
        )
        .await;
    assert!(matches!(
        group_pairing,
        Err(hachimi_gateway::GatewayError::InvalidMessage)
    ));
}

#[tokio::test]
async fn ingress_persists_bounded_enterprise_media_metadata_atomically() {
    let (store, gateway) = formal_gateway("open").await;
    let media = RemoteMediaDescriptor {
        provider_id: IntegrationProviderId::DingTalk,
        remote_id: "download-code-1".into(),
        resource_key: Some("file".into()),
        file_name: Some("report.pdf".into()),
        mime_type: Some("application/pdf".into()),
        declared_size_bytes: Some(1024),
        content_hash: None,
        download_required: true,
    };
    let mut incoming = message("event-media-1", "review", 20);
    incoming.parts.push(ChannelMessagePart::File {
        media: media.clone(),
    });
    gateway
        .ingest_verified(&incoming)
        .await
        .expect("media ingress");
    let row = sqlx::query("SELECT resource_key, file_name, mime_type, declared_size_bytes, metadata_hash FROM channel_attachment_metadata WHERE platform = 'dingtalk' AND account_id = 'account-1' AND event_id = 'event-media-1' AND remote_id = 'download-code-1'")
        .fetch_one(store.pool())
        .await
        .expect("media metadata");
    use sqlx::Row as _;
    assert_eq!(row.get::<String, _>("resource_key"), "file");
    assert_eq!(row.get::<String, _>("file_name"), "report.pdf");
    assert_eq!(row.get::<i64, _>("declared_size_bytes"), 1024);
    assert_eq!(
        row.get::<String, _>("metadata_hash"),
        hachimi_gateway::remote_media_metadata_hash(&media).expect("metadata hash")
    );

    let mut oversized = message("event-media-large", "too large", 30);
    oversized.parts.push(ChannelMessagePart::File {
        media: RemoteMediaDescriptor {
            declared_size_bytes: Some(25 * 1024 * 1024 + 1),
            ..media
        },
    });
    assert!(matches!(
        gateway.ingest_verified(&oversized).await,
        Err(hachimi_gateway::GatewayError::InvalidMessage)
    ));
    let rejected: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM channel_ingress WHERE external_message_id = 'event-media-large'",
    )
    .fetch_one(store.pool())
    .await
    .expect("rejected ingress count");
    assert_eq!(rejected, 0);
}

#[tokio::test]
async fn connect_precedes_pairing_policy_and_replay_does_not_consume_twice() {
    let (store, gateway) = formal_gateway("pairing").await;
    let pairing = gateway
        .create_pairing_code(
            ChannelPairingCodeRequest {
                account_id: "account-1".into(),
                target: ChannelAuthorizationTarget::DmIdentity,
                group_history_policy: None,
                topic_policy: ChannelTopicPolicy::InheritGroup,
                mention_policy: ChannelMentionPolicy::Disabled,
                grant: ChannelGrant::default(),
            },
            10,
        )
        .await
        .expect("pairing code");
    let incoming = message("connect-1", format!("/connect {}", pairing.code), 20);
    let accepted = gateway
        .ingest_verified(&incoming)
        .await
        .expect("connect accepted");
    assert_eq!(accepted.status, IngressStatus::Accepted);
    let replay = gateway
        .ingest_verified(&incoming)
        .await
        .expect("connect replay");
    assert_eq!(replay.status, IngressStatus::Duplicate);
    let consumed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM channel_pairing_codes WHERE consumed_at_ms IS NOT NULL",
    )
    .fetch_one(store.pool())
    .await
    .expect("consumed count");
    let authorizations: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM channel_authorizations WHERE actor_id = 'actor-1'",
    )
    .fetch_one(store.pool())
    .await
    .expect("authorization count");
    assert_eq!(consumed, 1);
    assert_eq!(authorizations, 1);
    assert!(
        gateway.resolve_binding(&incoming).await.is_ok(),
        "DM control authorization must not require a mention"
    );
}

#[tokio::test]
async fn ingress_freezes_grant_while_new_messages_observe_the_latest_ceiling() {
    let (_store, gateway) = formal_gateway("pairing").await;
    let allowed = ChannelGrant {
        skill_ids: vec!["skill-a".into()],
        ..ChannelGrant::default()
    };
    gateway
        .upsert_access_policy(
            ChannelAccessPolicyUpsert {
                account_id: "account-1".into(),
                dm_policy: ChannelDmPolicy::Pairing,
                allowlist_actor_ids: Vec::new(),
                grant_ceiling: allowed.clone(),
                expected_revision: 1,
            },
            10,
        )
        .await
        .expect("raise grant ceiling");
    let pairing = gateway
        .create_pairing_code(
            ChannelPairingCodeRequest {
                account_id: "account-1".into(),
                target: ChannelAuthorizationTarget::DmIdentity,
                group_history_policy: None,
                topic_policy: ChannelTopicPolicy::InheritGroup,
                mention_policy: ChannelMentionPolicy::Disabled,
                grant: allowed.clone(),
            },
            20,
        )
        .await
        .expect("pairing code");
    let connected = message("connect-grant", format!("/connect {}", pairing.code), 30);
    gateway
        .ingest_verified(&connected)
        .await
        .expect("connect accepted");

    gateway
        .upsert_access_policy(
            ChannelAccessPolicyUpsert {
                account_id: "account-1".into(),
                dm_policy: ChannelDmPolicy::Pairing,
                allowlist_actor_ids: Vec::new(),
                grant_ceiling: ChannelGrant::default(),
                expected_revision: 2,
            },
            40,
        )
        .await
        .expect("lower grant ceiling");
    assert_eq!(
        gateway
            .ingress_grant_snapshot(&connected.event_key)
            .await
            .expect("frozen grant"),
        allowed
    );
    assert!(matches!(
        gateway
            .ingest_verified(&message("after-grant-change", "hello", 50))
            .await,
        Err(hachimi_gateway::GatewayError::AuthorizationConflict)
    ));
}

#[tokio::test]
async fn expired_run_created_claim_recovers_the_existing_run() {
    let (store, gateway) = formal_gateway("open").await;
    let incoming = message("event-1", "hello", 10);
    gateway.ingest_verified(&incoming).await.expect("ingest");
    gateway
        .claim_next_ingress(10)
        .await
        .expect("claim")
        .expect("message");
    let session_id = SessionId::new("session-1");
    let run_id = RunId::new("run-1");
    sqlx::query("INSERT INTO sessions(id, context_kind, context_json, entry_profile, title, archived, pinned, parent_session_id, source_run_id, next_sequence, created_at_ms, updated_at_ms) VALUES(?, 'workspace', '{\"kind\":\"workspace\",\"workspace_id\":\"workspace-1\"}', 'workbench', 'Channel', 0, 0, NULL, NULL, 1, 10, 10)")
        .bind(session_id.as_str()).execute(store.pool()).await.expect("session");
    sqlx::query("INSERT INTO runs(id, session_id, status, purpose, origin_json, generation, configuration_json, requested_capabilities_json, negotiated_capabilities_json, provider_capability_probe_json, capability_degradations_json, failure_code, created_at_ms, updated_at_ms) VALUES(?, ?, 'queued', 'task', '{\"kind\":\"default\"}', 1, '{}', '{}', '{}', 'null', '[]', NULL, 10, 10)")
        .bind(run_id.as_str()).bind(session_id.as_str()).execute(store.pool()).await.expect("run");
    gateway
        .record_ingress_run(&incoming.event_key, &session_id, &run_id, 11)
        .await
        .expect("record run");
    assert!(
        gateway
            .claim_next_ingress(11)
            .await
            .expect("early claim")
            .is_none()
    );
    let recovered = gateway
        .claim_next_ingress(10 + CLAIM_TTL_MS)
        .await
        .expect("recovery claim")
        .expect("recovered message");
    assert_eq!(recovered.event_key, incoming.event_key);
    assert_eq!(
        gateway
            .ingress_run(&incoming.event_key)
            .await
            .expect("ingress run"),
        Some((session_id, run_id))
    );
}
