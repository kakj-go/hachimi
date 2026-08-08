use super::*;
use hachimi_protocol::{PluginContributionKind, PluginId};

const KINDS: [PluginContributionKind; 10] = [
    PluginContributionKind::Skill,
    PluginContributionKind::Hook,
    PluginContributionKind::EventSource,
    PluginContributionKind::Mcp,
    PluginContributionKind::Connector,
    PluginContributionKind::BrowserExtension,
    PluginContributionKind::ScheduledTaskTemplate,
    PluginContributionKind::Asset,
    PluginContributionKind::CustomUi,
    PluginContributionKind::Channel,
];

#[tokio::test]
async fn every_contribution_kind_completes_the_plugin_lifecycle_matrix() {
    for kind in KINDS {
        exercise_lifecycle(kind).await;
    }
}

#[tokio::test]
async fn every_contribution_kind_rolls_back_failed_health_and_interrupted_updates() {
    for kind in KINDS {
        exercise_failed_update(kind).await;
        exercise_interrupted_update(kind).await;
    }
}

async fn exercise_lifecycle(kind: PluginContributionKind) {
    let source = tempfile::tempdir().expect("source");
    let installs = tempfile::tempdir().expect("installs");
    let plugin_id = fixture_id(kind);
    write_fixture(source.path(), kind, &plugin_id, "1.0.0");
    let store = AgentStore::connect_in_memory().await.expect("store");
    let host = PluginHost::new(store.clone(), installs.path());

    let installed = host.install_local(source.path()).await.expect("install");
    assert_eq!(installed.status, PluginStatus::Disabled, "{kind:?}");
    assert_runtime(&host, &plugin_id, ContributionRuntimeState::Disabled).await;

    host.set_enabled(&plugin_id, true).await.expect("enable");
    assert_runtime(&host, &plugin_id, ContributionRuntimeState::Active).await;
    install_owned_runtime_rows(&store, kind, &installed).await;

    host.set_enabled(&plugin_id, false).await.expect("disable");
    assert_runtime(&host, &plugin_id, ContributionRuntimeState::Disabled).await;
    let hooks_enabled: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM plugin_hook_subscriptions WHERE plugin_id = ? AND enabled = 1",
    )
    .bind(plugin_id.as_str())
    .fetch_one(store.pool())
    .await
    .expect("hook state");
    assert_eq!(hooks_enabled, 0, "{kind:?}");

    host.set_enabled(&plugin_id, true).await.expect("re-enable");
    write_fixture(source.path(), kind, &plugin_id, "2.0.0");
    let candidate = host.install_local(source.path()).await.expect("update");
    assert_eq!(candidate.status, PluginStatus::NeedsAttention, "{kind:?}");
    assert_ne!(candidate.content_hash, installed.content_hash, "{kind:?}");

    let restored = host.rollback(&plugin_id, None).await.expect("rollback");
    assert_eq!(restored.content_hash, installed.content_hash, "{kind:?}");
    assert_eq!(restored.status, PluginStatus::Enabled, "{kind:?}");

    assert!(host.uninstall(&plugin_id).await.expect("uninstall"));
    assert_owned_runtime_rows_removed(&store, &plugin_id).await;
    assert!(host.get(&plugin_id).await.expect("lookup").is_none());
}

async fn exercise_failed_update(kind: PluginContributionKind) {
    let source = tempfile::tempdir().expect("source");
    let installs = tempfile::tempdir().expect("installs");
    let plugin_id = fixture_id(kind);
    write_fixture(source.path(), kind, &plugin_id, "1.0.0");
    let store = AgentStore::connect_in_memory().await.expect("store");
    let host = PluginHost::new(store, installs.path());
    let installed = host.install_local(source.path()).await.expect("install");
    host.set_enabled(&plugin_id, true).await.expect("enable");

    corrupt_fixture(source.path(), kind);
    assert!(host.install_local(source.path()).await.is_err(), "{kind:?}");
    let restored = host.get(&plugin_id).await.expect("lookup").expect("plugin");
    assert_eq!(restored.content_hash, installed.content_hash, "{kind:?}");
    assert_eq!(restored.status, PluginStatus::Enabled, "{kind:?}");
    assert!(
        host.lifecycle_journal(Some(&plugin_id))
            .await
            .expect("journal")
            .iter()
            .any(|entry| entry.operation == PluginLifecycleOperation::Update
                && entry.status == PluginLifecycleJournalStatus::RolledBack),
        "{kind:?}"
    );
}

async fn exercise_interrupted_update(kind: PluginContributionKind) {
    let source = tempfile::tempdir().expect("source");
    let installs = tempfile::tempdir().expect("installs");
    let plugin_id = fixture_id(kind);
    write_fixture(source.path(), kind, &plugin_id, "1.0.0");
    let store = AgentStore::connect_in_memory().await.expect("store");
    let host = PluginHost::new(store, installs.path());
    let installed = host.install_local(source.path()).await.expect("install");
    host.set_enabled(&plugin_id, true).await.expect("enable");
    let journal = host
        .begin_lifecycle(
            &plugin_id,
            PluginLifecycleOperation::Update,
            Some(&installed.content_hash),
            Some("interrupted-candidate"),
        )
        .await
        .expect("journal");

    let report = host.reconcile_lifecycle().await.expect("reconcile");
    assert_eq!(report.rolled_back, vec![journal], "{kind:?}");
    assert_runtime(&host, &plugin_id, ContributionRuntimeState::Active).await;
}

async fn assert_runtime(
    host: &PluginHost,
    plugin_id: &PluginId,
    expected: ContributionRuntimeState,
) {
    let contributions = host
        .list_contributions(Some(plugin_id))
        .await
        .expect("contributions");
    assert_eq!(contributions.len(), 1);
    assert_eq!(contributions[0].state, expected);
}

async fn install_owned_runtime_rows(
    store: &AgentStore,
    kind: PluginContributionKind,
    plugin: &InstalledPlugin,
) {
    let plugin_id = plugin.manifest.id.as_str();
    if matches!(
        kind,
        PluginContributionKind::Mcp
            | PluginContributionKind::ScheduledTaskTemplate
            | PluginContributionKind::BrowserExtension
            | PluginContributionKind::Asset
            | PluginContributionKind::CustomUi
            | PluginContributionKind::Channel
    ) {
        sqlx::query("INSERT INTO plugin_runtime_bindings(plugin_id, contribution_id, resource_kind, resource_id, runtime_revision, metadata_json, enabled, updated_at_ms) VALUES(?, 'fixture', ?, 'owned-resource', 'revision', '{}', 1, 1)")
            .bind(plugin_id)
            .bind(kind.as_str())
            .execute(store.pool())
            .await
            .expect("runtime binding");
    }
    if kind == PluginContributionKind::Channel {
        sqlx::query("INSERT INTO channel_provider_manifests(provider_id, plugin_id, manifest_json, content_hash, enabled, contribution_enabled, config_revision, health, diagnostic, updated_at_ms) VALUES(?, ?, '{}', 'hash', 0, 0, 1, 'disabled', NULL, 1)")
            .bind(format!("provider-{plugin_id}"))
            .bind(plugin_id)
            .execute(store.pool())
            .await
            .expect("channel manifest");
    }
}

async fn assert_owned_runtime_rows_removed(store: &AgentStore, plugin_id: &PluginId) {
    for (table, query) in [
        (
            "plugin_contribution_runtime",
            "SELECT COUNT(*) FROM plugin_contribution_runtime WHERE plugin_id = ?",
        ),
        (
            "plugin_hook_subscriptions",
            "SELECT COUNT(*) FROM plugin_hook_subscriptions WHERE plugin_id = ?",
        ),
        (
            "plugin_runtime_bindings",
            "SELECT COUNT(*) FROM plugin_runtime_bindings WHERE plugin_id = ?",
        ),
        (
            "connector_accounts",
            "SELECT COUNT(*) FROM connector_accounts WHERE plugin_id = ?",
        ),
        (
            "channel_provider_manifests",
            "SELECT COUNT(*) FROM channel_provider_manifests WHERE plugin_id = ?",
        ),
    ] {
        let count: i64 = sqlx::query_scalar(query)
            .bind(plugin_id.as_str())
            .fetch_one(store.pool())
            .await
            .expect("residue query");
        assert_eq!(count, 0, "residue in {table}");
    }
}

fn fixture_id(kind: PluginContributionKind) -> PluginId {
    PluginId::new(format!("lifecycle-{}", kind.as_str().replace('_', "-")))
}

fn write_fixture(root: &Path, kind: PluginContributionKind, plugin_id: &PluginId, version: &str) {
    fs::create_dir_all(root.join(".codex-plugin")).expect("manifest directory");
    let (relative_path, bytes) = fixture_content(kind, plugin_id);
    let target = root.join(&relative_path);
    if matches!(
        kind,
        PluginContributionKind::Skill
            | PluginContributionKind::BrowserExtension
            | PluginContributionKind::Asset
            | PluginContributionKind::CustomUi
    ) {
        fs::create_dir_all(&target).expect("contribution directory");
        let (name, body) = match kind {
            PluginContributionKind::Skill => ("SKILL.md", b"# Fixture\n".as_slice()),
            PluginContributionKind::BrowserExtension => (
                "manifest.json",
                br#"{"manifest_version":3,"name":"Fixture","version":"1.0.0"}"#.as_slice(),
            ),
            PluginContributionKind::Asset => ("fixture.json", br#"{"fixture":true}"#.as_slice()),
            PluginContributionKind::CustomUi => ("index.html", b"<main>fixture</main>".as_slice()),
            _ => unreachable!(),
        };
        fs::write(target.join(name), body).expect("contribution content");
    } else {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).expect("contribution parent");
        }
        fs::write(&target, bytes).expect("contribution content");
        if kind == PluginContributionKind::Hook {
            fs::create_dir_all(root.join("bin")).expect("hook bin");
            fs::write(root.join("bin/hook.exe"), b"fixture").expect("hook executable");
        }
        if kind == PluginContributionKind::Channel {
            fs::create_dir_all(root.join("bin")).expect("channel bin");
            fs::write(root.join("bin/channel.exe"), b"fixture").expect("channel executable");
        }
    }
    let manifest = PluginManifest {
        manifest_version: 1,
        id: plugin_id.clone(),
        name: format!("Lifecycle {kind:?}"),
        version: version.into(),
        description: "Lifecycle matrix fixture".into(),
        contributions: vec![PluginContribution {
            kind,
            id: "fixture".into(),
            relative_path,
            required_scopes: Vec::new(),
        }],
    };
    fs::write(
        root.join(MANIFEST_RELATIVE_PATH),
        serde_json::to_vec_pretty(&manifest).expect("manifest json"),
    )
    .expect("manifest");
}

fn fixture_content(kind: PluginContributionKind, plugin_id: &PluginId) -> (String, Vec<u8>) {
    let (path, value) = match kind {
        PluginContributionKind::Skill => ("skills/fixture", Value::Null),
        PluginContributionKind::Hook => (
            "hooks/fixture.json",
            json!({"protocolVersion":1,"runtime":"sandboxed_stdio_json_rpc","entrypoint":"bin/hook.exe","args":[],"events":["run.before"]}),
        ),
        PluginContributionKind::EventSource => (
            "events/fixture.json",
            json!({"sourceId":"fixture","eventTypes":["fixture.event"]}),
        ),
        PluginContributionKind::Mcp => (
            "mcp/fixture.json",
            json!({"displayName":"Fixture","transport":{"kind":"streamable_http","url":"http://127.0.0.1:1"}}),
        ),
        PluginContributionKind::Connector => (
            "connectors/fixture.json",
            json!({"hostIdentity":"hachimi.sample-crm.local.v1","transport":"local","actions":[{"name":"get","effect":"read_only"}],"webhook":false,"poll":false,"externalNetwork":false}),
        ),
        PluginContributionKind::BrowserExtension => ("browser-extension", Value::Null),
        PluginContributionKind::ScheduledTaskTemplate => {
            ("schedules/fixture.json", json!({"name":"Fixture"}))
        }
        PluginContributionKind::Asset => ("assets", Value::Null),
        PluginContributionKind::CustomUi => ("ui", Value::Null),
        PluginContributionKind::Channel => (
            "channels/fixture.json",
            json!({"protocolVersion":1,"providerId":format!("plugin.{}.fixture", plugin_id.as_str()),"transport":"stdio_json_rpc","entrypoint":"bin/channel.exe","args":[]}),
        ),
    };
    (
        path.into(),
        serde_json::to_vec(&value).expect("fixture json"),
    )
}

fn corrupt_fixture(root: &Path, kind: PluginContributionKind) {
    let plugin_id = fixture_id(kind);
    let (relative, _) = fixture_content(kind, &plugin_id);
    let target = root.join(relative);
    match kind {
        PluginContributionKind::Skill => {
            fs::remove_dir_all(&target).expect("remove skill");
            fs::create_dir_all(&target).expect("empty skill directory");
        }
        PluginContributionKind::BrowserExtension => {
            fs::write(target.join("manifest.json"), b"not-json").expect("break extension");
        }
        PluginContributionKind::Asset => {
            fs::write(target.join("forbidden.html"), b"<p>active</p>").expect("break asset");
        }
        PluginContributionKind::CustomUi => {
            fs::write(
                target.join("index.html"),
                b"<script>window.__TAURI__</script>",
            )
            .expect("break custom ui");
        }
        _ => fs::write(target, b"not-json").expect("break descriptor"),
    }
}
