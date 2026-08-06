use super::*;
use hachimi_protocol::{ConnectorAccountUpsert, PluginContribution, PluginId};
use std::io::Write as _;
use zip::{ZipWriter, write::SimpleFileOptions};

fn write_sample_plugin(root: &Path) {
    fs::create_dir_all(root.join(".codex-plugin")).expect("manifest directory");
    fs::create_dir_all(root.join("connectors")).expect("connectors");
    fs::write(
        root.join("connectors/sample-crm.json"),
        br#"{"hostIdentity":"hachimi.sample-crm.local.v1","transport":"local","actions":["get","search","create","update","webhook_emit","webhook_next","poll"],"webhook":true,"poll":true,"externalNetwork":false}"#,
    )
    .expect("connector");
    let manifest = PluginManifest {
        manifest_version: 1,
        id: PluginId::from("sample-crm"),
        name: "Sample CRM".into(),
        version: "1.0.0".into(),
        description: "Deterministic local fixture".into(),
        contributions: vec![PluginContribution {
            kind: hachimi_protocol::PluginContributionKind::Connector,
            id: "sample-crm".into(),
            relative_path: "connectors/sample-crm.json".into(),
            required_scopes: vec!["connectors.invoke".into()],
        }],
    };
    fs::write(
        root.join(MANIFEST_RELATIVE_PATH),
        serde_json::to_vec_pretty(&manifest).expect("manifest"),
    )
    .expect("manifest file");
}

fn write_builtin_wecom_app_channel_plugin(root: &Path) {
    fs::create_dir_all(root.join(".codex-plugin")).expect("manifest directory");
    fs::create_dir_all(root.join("channels")).expect("channels directory");
    fs::write(
        root.join("channels/wecom_app.json"),
        br#"{"protocolVersion":1,"providerId":"wecom_app","transport":"builtin_enterprise"}"#,
    )
    .expect("channel descriptor");
    let manifest = PluginManifest {
        manifest_version: 1,
        id: PluginId::from("wecom_app"),
        name: "WeCom App".into(),
        version: "1.0.0".into(),
        description: "Built-in enterprise channel fixture".into(),
        contributions: vec![PluginContribution {
            kind: hachimi_protocol::PluginContributionKind::Channel,
            id: "wecom-app-channel".into(),
            relative_path: "channels/wecom_app.json".into(),
            required_scopes: vec!["channel.receive".into(), "channel.send".into()],
        }],
    };
    fs::write(
        root.join(MANIFEST_RELATIVE_PATH),
        serde_json::to_vec_pretty(&manifest).expect("manifest"),
    )
    .expect("manifest file");
}

#[tokio::test]
async fn builtin_enterprise_channel_follows_plugin_lifecycle_without_sidecar_discovery() {
    let source = tempfile::tempdir().expect("source");
    let installs = tempfile::tempdir().expect("installs");
    write_builtin_wecom_app_channel_plugin(source.path());
    let store = AgentStore::connect_in_memory().await.expect("store");
    let host = PluginHost::new(store, installs.path());

    let installed = host.install_local(source.path()).await.expect("install");
    let installed_contribution = host
        .list_contributions(Some(&installed.manifest.id))
        .await
        .expect("installed contributions")
        .into_iter()
        .next()
        .expect("channel contribution");
    assert_eq!(installed.status, PluginStatus::Disabled);
    assert_eq!(
        installed_contribution.state,
        ContributionRuntimeState::Disabled
    );
    assert_eq!(
        host.builtin_enterprise_channel_provider_id(&installed, "wecom-app-channel")
            .expect("built-in descriptor"),
        Some("wecom_app".into())
    );

    let enabled = host
        .set_enabled(&installed.manifest.id, true)
        .await
        .expect("enable");
    assert_eq!(enabled.status, PluginStatus::Enabled);
    assert!(
        host.enabled_channel_sidecars()
            .await
            .expect("sidecars")
            .is_empty()
    );
    assert_eq!(
        host.list_contributions(Some(&installed.manifest.id))
            .await
            .expect("enabled contributions")[0]
            .state,
        ContributionRuntimeState::Active
    );

    let disabled = host
        .set_enabled(&installed.manifest.id, false)
        .await
        .expect("disable");
    assert_eq!(disabled.status, PluginStatus::Disabled);
    assert_eq!(
        host.list_contributions(Some(&installed.manifest.id))
            .await
            .expect("disabled contributions")[0]
            .state,
        ContributionRuntimeState::Disabled
    );

    assert!(
        host.uninstall(&installed.manifest.id)
            .await
            .expect("uninstall")
    );
    assert!(
        host.get(&installed.manifest.id)
            .await
            .expect("plugin lookup")
            .is_none()
    );
    assert!(
        host.list_contributions(Some(&installed.manifest.id))
            .await
            .expect("removed contributions")
            .is_empty()
    );
    let operations = host
        .lifecycle_journal(Some(&installed.manifest.id))
        .await
        .expect("lifecycle journal");
    for operation in [
        PluginLifecycleOperation::Install,
        PluginLifecycleOperation::Enable,
        PluginLifecycleOperation::Disable,
        PluginLifecycleOperation::Uninstall,
    ] {
        assert!(operations.iter().any(|entry| {
            entry.operation == operation && entry.status == PluginLifecycleJournalStatus::Committed
        }));
    }
}

#[tokio::test]
async fn event_source_contribution_requires_a_bounded_typed_descriptor() {
    let source = tempfile::tempdir().expect("source");
    let installs = tempfile::tempdir().expect("installs");
    write_sample_plugin(source.path());
    fs::create_dir_all(source.path().join("events")).expect("events directory");
    fs::write(
        source.path().join("events/sample-crm.json"),
        br#"{"sourceId":"sample-crm","eventTypes":["record.changed","sync.completed"]}"#,
    )
    .expect("event source descriptor");
    let manifest_path = source.path().join(MANIFEST_RELATIVE_PATH);
    let mut manifest: PluginManifest =
        serde_json::from_slice(&fs::read(&manifest_path).expect("manifest bytes"))
            .expect("manifest");
    manifest.contributions.push(PluginContribution {
        kind: hachimi_protocol::PluginContributionKind::EventSource,
        id: "sample-crm-events".into(),
        relative_path: "events/sample-crm.json".into(),
        required_scopes: Vec::new(),
    });
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("event manifest"),
    )
    .expect("manifest file");

    let store = AgentStore::connect_in_memory().await.expect("store");
    let host = PluginHost::new(store, installs.path());
    let installed = host.install_local(source.path()).await.expect("install");
    host.set_enabled(&installed.manifest.id, true)
        .await
        .expect("enable");
    let event_source = host
        .list_contributions(Some(&installed.manifest.id))
        .await
        .expect("contributions")
        .into_iter()
        .find(|contribution| contribution.contribution_id == "sample-crm-events")
        .expect("event source runtime");
    assert_eq!(event_source.state, ContributionRuntimeState::Active);
    assert_eq!(
        event_source.diagnostic.as_deref(),
        Some("event_source_uses_authenticated_scheduler_ingress")
    );
}

#[tokio::test]
async fn local_bundle_upgrade_needs_attention_and_crm_is_idempotent() {
    let source = tempfile::tempdir().expect("source");
    let installs = tempfile::tempdir().expect("installs");
    write_sample_plugin(source.path());
    let store = AgentStore::connect_in_memory().await.expect("store");
    let host = PluginHost::new(store, installs.path());
    let installed = host.install_local(source.path()).await.expect("install");
    assert_eq!(installed.status, PluginStatus::Disabled);
    let enabled = host
        .set_enabled(&installed.manifest.id, true)
        .await
        .expect("enable");
    assert_eq!(enabled.status, PluginStatus::Enabled);
    let revision = connector_revision(&enabled, "sample-crm").expect("revision");
    let contribution_revision = ContributionRevision {
        plugin_id: installed.manifest.id.clone(),
        contribution_id: "sample-crm".into(),
        account_id: Some(ConnectorAccountId::from("account-1")),
        content_hash: installed.content_hash.clone(),
        host_identity_hash: Some(revision.host_identity_hash.clone()),
        schema_hash: Some(revision.schema_hash.clone()),
        action_hash: Some(revision.action_hash.clone()),
    };
    let account = host
        .upsert_connector_account(ConnectorAccountUpsert {
            id: ConnectorAccountId::from("account-1"),
            plugin_id: installed.manifest.id.clone(),
            connector_id: "sample-crm".into(),
            display_name: "Fixture".into(),
            secret: None,
        })
        .await
        .expect("account");
    host.verify_contribution_revisions(std::slice::from_ref(&contribution_revision))
        .await
        .expect("pinned contribution");
    let request = ConnectorInvocationRequest {
        account_id: account.id.clone(),
        action: "create".into(),
        arguments: json!({"id": "customer-1", "data": {"name": "Ada"}}),
        idempotency_key: "create-customer-1".into(),
        expected_revision: account.revision,
    };
    let created = host.invoke_connector(&request).await.expect("create");
    assert!(!created.replayed);
    assert!(
        host.invoke_connector(&request)
            .await
            .expect("replay")
            .replayed
    );
    let manifest_path = source.path().join(MANIFEST_RELATIVE_PATH);
    let mut upgraded_manifest: PluginManifest =
        serde_json::from_slice(&fs::read(&manifest_path).expect("manifest bytes"))
            .expect("manifest");
    upgraded_manifest.contributions[0]
        .required_scopes
        .push("connectors.webhook".into());
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&upgraded_manifest).expect("upgraded manifest"),
    )
    .expect("expand plugin scope");
    assert_eq!(
        host.install_local(source.path())
            .await
            .expect("upgrade")
            .status,
        PluginStatus::NeedsAttention
    );
    let permission_diff = host
        .permission_diff(&installed.manifest.id)
        .await
        .expect("permission diff")
        .expect("stored permission diff");
    assert!(permission_diff.requires_confirmation);
    assert_eq!(permission_diff.added_scopes, vec!["connectors.webhook"]);
    assert!(matches!(
        host.verify_contribution_revisions(&[contribution_revision])
            .await,
        Err(ExtensionHostError::ContributionDrift)
    ));
}

#[tokio::test]
async fn contribution_escape_is_rejected() {
    let source = tempfile::tempdir().expect("source");
    write_sample_plugin(source.path());
    let manifest_path = source.path().join(MANIFEST_RELATIVE_PATH);
    let mut manifest: PluginManifest =
        serde_json::from_slice(&fs::read(&manifest_path).expect("read")).expect("parse");
    manifest.contributions[0].relative_path = "../outside".into();
    fs::write(
        manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("manifest"),
    )
    .expect("write");
    let store = AgentStore::connect_in_memory().await.expect("store");
    let installs = tempfile::tempdir().expect("installs");
    let host = PluginHost::new(store, installs.path());
    assert!(matches!(
        host.install_local(source.path()).await,
        Err(ExtensionHostError::ContributionEscape)
    ));
}

#[tokio::test]
async fn zip_bundle_with_one_wrapper_directory_installs_safely() {
    let source = tempfile::tempdir().expect("source");
    write_sample_plugin(source.path());
    let archive_path = source.path().join("sample-crm.zip");
    let archive_file = fs::File::create(&archive_path).expect("archive");
    let mut archive = ZipWriter::new(archive_file);
    let (_, files) = hash_bundle(source.path()).expect("bundle files");
    for relative in files {
        if relative == Path::new("sample-crm.zip") {
            continue;
        }
        archive
            .start_file(
                format!(
                    "sample-crm/{}",
                    relative.to_string_lossy().replace('\\', "/")
                ),
                SimpleFileOptions::default(),
            )
            .expect("entry");
        archive
            .write_all(&fs::read(source.path().join(relative)).expect("source file"))
            .expect("entry body");
    }
    archive.finish().expect("finish archive");
    let store = AgentStore::connect_in_memory().await.expect("store");
    let installs = tempfile::tempdir().expect("installs");
    let installed = PluginHost::new(store, installs.path())
        .install_local(&archive_path)
        .await
        .expect("install archive");
    assert_eq!(installed.manifest.id.as_str(), "sample-crm");
    assert_eq!(installed.status, PluginStatus::Disabled);
}

#[tokio::test]
async fn sample_crm_webhook_poll_and_retry_ledgers_are_durable() {
    let source = tempfile::tempdir().expect("source");
    let installs = tempfile::tempdir().expect("installs");
    write_sample_plugin(source.path());
    let store = AgentStore::connect_in_memory().await.expect("store");
    let host = PluginHost::new(store.clone(), installs.path());
    let installed = host.install_local(source.path()).await.expect("install");
    let enabled = host
        .set_enabled(&installed.manifest.id, true)
        .await
        .expect("enable");
    let account = host
        .upsert_connector_account(ConnectorAccountUpsert {
            id: ConnectorAccountId::from("transport-account"),
            plugin_id: enabled.manifest.id,
            connector_id: "sample-crm".into(),
            display_name: "Transport fixture".into(),
            secret: None,
        })
        .await
        .expect("account");
    let invoke = |action: &str, arguments: Value, key: &str| ConnectorInvocationRequest {
        account_id: account.id.clone(),
        action: action.into(),
        arguments,
        idempotency_key: key.into(),
        expected_revision: account.revision.clone(),
    };

    host.invoke_connector(&invoke(
        "create",
        json!({"id": "customer-1", "data": {"name": "Ada"}}),
        "transport-create",
    ))
    .await
    .expect("create");
    host.invoke_connector(&invoke(
        "webhook_emit",
        json!({"eventId": "event-1", "payload": {"recordId": "customer-1"}}),
        "transport-webhook-emit",
    ))
    .await
    .expect("emit");
    let event = host
        .invoke_connector(&invoke("webhook_next", json!({}), "transport-webhook-next"))
        .await
        .expect("next event");
    assert_eq!(event.result["eventId"], "event-1");
    let poll = host
        .invoke_connector(&invoke("poll", json!({}), "transport-poll"))
        .await
        .expect("poll");
    assert_eq!(poll.result.as_array().map(Vec::len), Some(1));

    let row = sqlx::query("SELECT attempt, next_attempt_at_ms, last_error FROM connector_retry_ledger WHERE account_id = ? AND idempotency_key = ?")
        .bind(account.id.as_str())
        .bind("transport-poll")
        .fetch_one(store.pool())
        .await
        .expect("retry ledger");
    assert_eq!(row.get::<i64, _>("attempt"), 1);
    assert_eq!(row.get::<Option<i64>, _>("next_attempt_at_ms"), None);
    assert_eq!(row.get::<Option<String>, _>("last_error"), None);

    let audit = sqlx::query(
        "SELECT target_summary, decision, result_code FROM audit_events WHERE operation = 'connector.poll'",
    )
    .fetch_one(store.pool())
    .await
    .expect("connector audit");
    let summary = audit.get::<String, _>("target_summary");
    assert!(summary.starts_with("connector:sample-crm:account_sha256:"));
    assert!(!summary.contains(account.id.as_str()));
    assert!(!summary.contains("customer-1"));
    assert_eq!(audit.get::<String, _>("decision"), "allowed");
    assert_eq!(audit.get::<String, _>("result_code"), "completed");
}

#[test]
fn connector_audit_summary_hashes_account_identity() {
    let summary = connector_target_summary("sample-crm", "secret-account-name");
    assert!(summary.starts_with("connector:sample-crm:account_sha256:"));
    assert!(!summary.contains("secret-account-name"));
}

#[tokio::test]
async fn plugin_update_keeps_revision_history_and_rolls_back_to_enabled_known_good() {
    let source = tempfile::tempdir().expect("source");
    let installs = tempfile::tempdir().expect("installs");
    write_sample_plugin(source.path());
    let store = AgentStore::connect_in_memory().await.expect("store");
    let host = PluginHost::new(store, installs.path());
    let first = host.install_local(source.path()).await.expect("install v1");
    host.set_enabled(&first.manifest.id, true)
        .await
        .expect("enable v1");

    let manifest_path = source.path().join(MANIFEST_RELATIVE_PATH);
    let mut manifest: PluginManifest =
        serde_json::from_slice(&fs::read(&manifest_path).expect("manifest")).expect("decode");
    manifest.version = "2.0.0".into();
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("encode"),
    )
    .expect("write v2");
    let second = host.install_local(source.path()).await.expect("install v2");
    assert_ne!(first.content_hash, second.content_hash);
    assert_eq!(second.status, PluginStatus::NeedsAttention);

    let revisions = host
        .list_revisions(&first.manifest.id)
        .await
        .expect("revision history");
    assert_eq!(revisions.len(), 2);
    assert!(revisions.iter().any(|revision| {
        revision.revision == first.content_hash
            && revision.status == PluginRevisionStatus::Superseded
            && revision.plugin_status == PluginStatus::Enabled
    }));

    let restored = host
        .rollback(&first.manifest.id, None)
        .await
        .expect("rollback");
    assert_eq!(restored.content_hash, first.content_hash);
    assert_eq!(restored.status, PluginStatus::Enabled);
    let head = host
        .revision_head(&first.manifest.id)
        .await
        .expect("head")
        .expect("revision head");
    assert_eq!(head.current_revision, first.content_hash);
}

#[tokio::test]
async fn failed_plugin_update_restores_previous_revision_and_records_rollback() {
    let source = tempfile::tempdir().expect("source");
    let installs = tempfile::tempdir().expect("installs");
    write_sample_plugin(source.path());
    let store = AgentStore::connect_in_memory().await.expect("store");
    let host = PluginHost::new(store, installs.path());
    let first = host.install_local(source.path()).await.expect("install v1");
    host.set_enabled(&first.manifest.id, true)
        .await
        .expect("enable v1");
    fs::write(source.path().join("connectors/sample-crm.json"), b"{}")
        .expect("break candidate connector");

    assert!(host.install_local(source.path()).await.is_err());
    let restored = host
        .get(&first.manifest.id)
        .await
        .expect("lookup")
        .expect("restored plugin");
    assert_eq!(restored.content_hash, first.content_hash);
    assert_eq!(restored.status, PluginStatus::Enabled);
    let journals = host
        .lifecycle_journal(Some(&first.manifest.id))
        .await
        .expect("journal");
    assert!(journals.iter().any(|entry| {
        entry.operation == PluginLifecycleOperation::Update
            && entry.status == PluginLifecycleJournalStatus::RolledBack
    }));
    let revisions = host
        .list_revisions(&first.manifest.id)
        .await
        .expect("revisions");
    assert!(revisions.iter().any(|revision| {
        revision.status == PluginRevisionStatus::Failed
            && revision.health_code.as_deref() == Some("plugin_connector_load_failed")
    }));
}

#[tokio::test]
async fn startup_reconciliation_rolls_back_an_uncommitted_candidate() {
    let source = tempfile::tempdir().expect("source");
    let installs = tempfile::tempdir().expect("installs");
    write_sample_plugin(source.path());
    let store = AgentStore::connect_in_memory().await.expect("store");
    let host = PluginHost::new(store, installs.path());
    let installed = host.install_local(source.path()).await.expect("install");
    let journal = host
        .begin_lifecycle(
            &installed.manifest.id,
            PluginLifecycleOperation::Update,
            Some(&installed.content_hash),
            Some("missing-candidate"),
        )
        .await
        .expect("begin interrupted update");

    let report = host
        .reconcile_lifecycle()
        .await
        .expect("startup reconciliation");
    assert_eq!(report.rolled_back, vec![journal]);
    assert_eq!(
        host.get(&installed.manifest.id)
            .await
            .expect("lookup")
            .expect("plugin")
            .content_hash,
        installed.content_hash
    );
}
