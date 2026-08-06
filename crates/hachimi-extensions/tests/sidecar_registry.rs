use std::sync::{Arc, Mutex};

use hachimi_extensions::{
    ConnectorDriver, ConnectorDriverContext, ExtensionHostError, PluginHost,
    SandboxedStdioConnectorDriver,
};
use hachimi_protocol::{
    ConnectorAccount, ConnectorAccountId, ConnectorAccountUpsert, ConnectorHealth,
    ConnectorInvocationRequest, ConnectorRevision, ConnectorRuntimeKind, PluginContribution,
    PluginContributionKind, PluginId, PluginManifest, SandboxCapabilityReport, SandboxReadiness,
};
use hachimi_sandbox::{
    SandboxBackend, SandboxError, SandboxLaunchSpec, SandboxNetworkPolicy, SandboxSpawnFuture,
    SandboxedChild,
};
use hachimi_storage::AgentStore;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
struct RejectingSandbox;

#[derive(Debug, Clone)]
struct CapturedLaunch {
    args: Vec<String>,
    environment: Vec<(String, String)>,
    stdin: String,
}

#[derive(Debug, Default)]
struct FixtureSandbox {
    launches: Mutex<Vec<CapturedLaunch>>,
}

impl SandboxBackend for FixtureSandbox {
    fn capability_report(&self) -> SandboxCapabilityReport {
        RejectingSandbox.capability_report()
    }

    fn spawn_restricted(
        &self,
        spec: SandboxLaunchSpec,
        cancellation: CancellationToken,
    ) -> SandboxSpawnFuture<'_> {
        assert_eq!(spec.network_policy, SandboxNetworkPolicy::DenyAll);
        let args = spec
            .args
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let environment = spec
            .environment
            .iter()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
            .collect::<Vec<_>>();
        let input = spec.stdin.unwrap_or_default();
        self.launches
            .lock()
            .expect("launches")
            .push(CapturedLaunch {
                args: args.clone(),
                environment,
                stdin: String::from_utf8_lossy(&input).into_owned(),
            });
        Box::pin(async move {
            let mut command = tokio::process::Command::new(&spec.executable);
            command
                .args(args)
                .current_dir(&spec.cwd)
                .env_clear()
                .envs(spec.environment)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);
            let mut child = command.spawn().map_err(SandboxError::Spawn)?;
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(&input).await.map_err(SandboxError::Spawn)?;
            }
            Ok(SandboxedChild::new(
                child,
                cancellation,
                spec.timeout,
                spec.output_limit,
            ))
        })
    }
}

impl SandboxBackend for RejectingSandbox {
    fn capability_report(&self) -> SandboxCapabilityReport {
        SandboxCapabilityReport {
            backend: "test".into(),
            readiness: SandboxReadiness::Ready,
            os_enforced: true,
            filesystem_enforced: true,
            process_enforced: true,
            network_enforced: true,
            version: Some("test".into()),
            stable_error_code: None,
            diagnostics: Vec::new(),
        }
    }

    fn spawn_restricted(
        &self,
        _spec: SandboxLaunchSpec,
        _cancellation: CancellationToken,
    ) -> SandboxSpawnFuture<'_> {
        Box::pin(async { Err(SandboxError::RuntimeUnavailable) })
    }
}

#[tokio::test]
async fn channel_contribution_loads_as_a_namespaced_sidecar_definition() {
    let source = tempfile::tempdir().expect("source");
    std::fs::create_dir_all(source.path().join(".codex-plugin")).expect("manifest directory");
    std::fs::create_dir_all(source.path().join("channels")).expect("channels");
    std::fs::create_dir_all(source.path().join("bin")).expect("bin");
    std::fs::write(source.path().join("bin/channel.exe"), b"fixture").expect("sidecar fixture");
    std::fs::write(
        source.path().join("channels/local.json"),
        br#"{"protocolVersion":1,"providerId":"plugin.channel-fixture.local","transport":"stdio_json_rpc","entrypoint":"bin/channel.exe","args":[]}"#,
    )
    .expect("channel descriptor");
    let manifest = PluginManifest {
        manifest_version: 1,
        id: PluginId::from("channel-fixture"),
        name: "Channel fixture".into(),
        version: "1.0.0".into(),
        description: "Sandboxed Channel JSON-RPC fixture".into(),
        contributions: vec![PluginContribution {
            kind: PluginContributionKind::Channel,
            id: "local".into(),
            relative_path: "channels/local.json".into(),
            required_scopes: vec!["channels.receive".into(), "channels.deliver".into()],
        }],
    };
    std::fs::write(
        source.path().join(".codex-plugin/plugin.json"),
        serde_json::to_vec_pretty(&manifest).expect("manifest"),
    )
    .expect("manifest file");
    let store = AgentStore::connect_in_memory().await.expect("store");
    let installs = tempfile::tempdir().expect("installs");
    let host = PluginHost::new(store, installs.path());
    let installed = host.install_local(source.path()).await.expect("install");
    host.set_enabled(&installed.manifest.id, true)
        .await
        .expect("enable");
    let definitions = host.enabled_channel_sidecars().await.expect("definitions");
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].manifest.id, "plugin.channel-fixture.local");
    assert!(
        definitions[0]
            .executable
            .starts_with(&definitions[0].bundle_root)
    );
    assert_eq!(
        definitions[0].manifest.required_scopes,
        ["channels.receive", "channels.deliver"]
    );
}

#[tokio::test]
async fn stdio_connector_registers_generically_and_fails_closed_in_the_sandbox() {
    let source = tempfile::tempdir().expect("source");
    std::fs::create_dir_all(source.path().join(".codex-plugin")).expect("manifest directory");
    std::fs::create_dir_all(source.path().join("connectors")).expect("connectors");
    std::fs::create_dir_all(source.path().join("bin")).expect("bin");
    std::fs::write(source.path().join("bin/fixture.exe"), b"fixture").expect("sidecar fixture");
    std::fs::write(
        source.path().join("connectors/fixture.json"),
        br#"{"hostIdentity":"hachimi.plugin.sidecar-fixture.fixture.sidecar.v1","transport":"stdio_json_rpc","entrypoint":"bin/fixture.exe","args":[],"actions":["search"],"schema":{"query":"string"}}"#,
    )
    .expect("connector descriptor");
    let manifest = PluginManifest {
        manifest_version: 1,
        id: PluginId::from("sidecar-fixture"),
        name: "Sidecar fixture".into(),
        version: "1.0.0".into(),
        description: "Sandboxed JSON-RPC fixture".into(),
        contributions: vec![PluginContribution {
            kind: PluginContributionKind::Connector,
            id: "fixture".into(),
            relative_path: "connectors/fixture.json".into(),
            required_scopes: vec!["connectors.invoke".into()],
        }],
    };
    std::fs::write(
        source.path().join(".codex-plugin/plugin.json"),
        serde_json::to_vec_pretty(&manifest).expect("manifest"),
    )
    .expect("manifest file");
    let store = AgentStore::connect_in_memory().await.expect("store");
    let installs = tempfile::tempdir().expect("installs");
    let host = PluginHost::new(store, installs.path());
    let installed = host.install_local(source.path()).await.expect("install");
    assert_eq!(
        host.register_sidecar_drivers(Arc::new(RejectingSandbox))
            .await
            .expect("register"),
        1
    );
    host.set_enabled(&installed.manifest.id, true)
        .await
        .expect("enable");
    let descriptor = host
        .connector_driver_descriptor(&installed.manifest.id, "fixture")
        .await
        .expect("descriptor");
    assert_eq!(
        descriptor.runtime_kind,
        ConnectorRuntimeKind::SandboxedStdioJsonRpc
    );
    let account = host
        .upsert_connector_account(ConnectorAccountUpsert {
            id: ConnectorAccountId::from("sidecar-account"),
            plugin_id: installed.manifest.id,
            connector_id: "fixture".into(),
            display_name: "Sidecar".into(),
            secret: None,
        })
        .await
        .expect("account");
    let error = host
        .invoke_connector(&ConnectorInvocationRequest {
            account_id: account.id,
            action: "search".into(),
            arguments: serde_json::json!({"query": "Ada"}),
            idempotency_key: "sidecar-search".into(),
            expected_revision: account.revision,
        })
        .await
        .expect_err("rejecting sandbox must fail closed");
    assert!(matches!(error, ExtensionHostError::Sidecar(_)));
}

#[tokio::test]
async fn stdio_connector_executes_every_method_and_keeps_credentials_out_of_process_metadata() {
    let fixture = std::path::Path::new(env!("CARGO_BIN_EXE_hachimi-sidecar-fixture"));
    let bundle = tempfile::tempdir().expect("bundle");
    let executable = bundle.path().join("fixture.exe");
    std::fs::copy(fixture, &executable).expect("copy fixture");
    let sandbox = Arc::new(FixtureSandbox::default());
    let driver = SandboxedStdioConnectorDriver::new(
        sandbox.clone(),
        bundle.path().to_path_buf(),
        executable,
        Vec::new(),
        vec!["search".into()],
    )
    .expect("driver");
    let store = AgentStore::connect_in_memory().await.expect("store");
    let revision = ConnectorRevision {
        host_identity_hash: "host-v1".into(),
        schema_hash: "schema-v1".into(),
        action_hash: "action-v1".into(),
    };
    let context = ConnectorDriverContext {
        store,
        account: ConnectorAccount {
            id: ConnectorAccountId::from("fixture-account"),
            plugin_id: PluginId::from("fixture-plugin"),
            connector_id: "fixture".into(),
            display_name: "Fixture".into(),
            secret_ref: Some("keyring:test-only".into()),
            revision: revision.clone(),
            health: ConnectorHealth::Healthy,
            updated_at_ms: 1,
        },
        credential: Some("fixture-secret-never-in-argv-or-env".into()),
    };
    let request = ConnectorInvocationRequest {
        account_id: context.account.id.clone(),
        action: "search".into(),
        arguments: serde_json::json!({"query":"Ada"}),
        idempotency_key: "fixture-search".into(),
        expected_revision: revision,
    };
    assert_eq!(
        driver.health(&context).await.expect("health"),
        ConnectorHealth::Healthy
    );
    for result in [
        driver
            .invoke(context.clone(), &request)
            .await
            .expect("invoke"),
        driver
            .webhook(context.clone(), &request)
            .await
            .expect("webhook"),
        driver.poll(context.clone(), &request).await.expect("poll"),
    ] {
        assert_eq!(
            result.get("ok").and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            result
                .get("secretInArgvOrEnvironment")
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );
    }
    driver.revoke(context).await.expect("revoke");
    let launches = sandbox.launches.lock().expect("launches");
    assert_eq!(launches.len(), 5);
    assert!(launches.iter().all(|launch| {
        launch.stdin.contains("fixture-secret-never-in-argv-or-env")
            && launch
                .args
                .iter()
                .all(|value| !value.contains("fixture-secret"))
            && launch.environment.iter().all(|(key, value)| {
                !key.contains("fixture-secret") && !value.contains("fixture-secret")
            })
    }));
}
