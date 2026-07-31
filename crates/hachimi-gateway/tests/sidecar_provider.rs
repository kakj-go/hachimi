use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use hachimi_gateway::{ChannelProvider, SandboxedStdioChannelProvider};
use hachimi_protocol::{
    ChannelDeliveryId, ChannelEnvelope, ChannelMessageId, ChannelProviderAccount,
    ChannelProviderHealthState, ChannelProviderManifest, ChannelProviderRuntimeKind,
    ChannelRouteKey, DeliveryAttempt, DeliveryAttemptStatus, PluginId, SandboxCapabilityReport,
    SandboxReadiness,
};
use hachimi_sandbox::{
    SandboxBackend, SandboxError, SandboxLaunchSpec, SandboxNetworkPolicy, SandboxSpawnFuture,
    SandboxedChild,
};
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
async fn channel_sidecar_fails_closed_when_the_sandbox_cannot_launch() {
    let bundle = tempfile::tempdir().expect("bundle");
    let executable = bundle.path().join(if cfg!(windows) {
        "channel.exe"
    } else {
        "channel"
    });
    std::fs::write(&executable, b"fixture").expect("fixture");
    let manifest = ChannelProviderManifest {
        id: "plugin.fixture.local".into(),
        plugin_id: Some(PluginId::from("fixture")),
        runtime_kind: ChannelProviderRuntimeKind::SandboxedStdioJsonRpc,
        entrypoint: Some("channels/local.json".into()),
        content_hash: "content-v1".into(),
        required_scopes: vec!["channels.receive".into()],
    };
    let provider = SandboxedStdioChannelProvider::new(
        Arc::new(RejectingSandbox),
        manifest,
        PathBuf::from(bundle.path()),
        executable,
        Vec::new(),
    )
    .expect("provider");
    let error = provider
        .configure(&ChannelProviderAccount {
            id: "local".into(),
            provider_id: "plugin.fixture.local".into(),
            display_name: "Fixture".into(),
            secret_ref: None,
            enabled: true,
            route_allowlist: vec![ChannelRouteKey {
                channel: "plugin.fixture.local".into(),
                account: "local".into(),
                peer: "user".into(),
                thread: "main".into(),
            }],
            config_revision: 1,
        })
        .await
        .expect_err("configure must fail closed");
    assert!(error.to_string().contains("sandbox_spawn_failed"));
}

#[tokio::test]
async fn channel_sidecar_executes_full_lifecycle_and_passes_transport_secret_only_on_stdin() {
    let fixture = std::path::Path::new(env!("CARGO_BIN_EXE_hachimi-channel-sidecar-fixture"));
    let bundle = tempfile::tempdir().expect("bundle");
    let executable = bundle.path().join("channel.exe");
    std::fs::copy(fixture, &executable).expect("copy fixture");
    let sandbox = Arc::new(FixtureSandbox::default());
    let provider = SandboxedStdioChannelProvider::new(
        sandbox.clone(),
        ChannelProviderManifest {
            id: "plugin.fixture.local".into(),
            plugin_id: Some(PluginId::from("fixture")),
            runtime_kind: ChannelProviderRuntimeKind::SandboxedStdioJsonRpc,
            entrypoint: Some("channels/local.json".into()),
            content_hash: "content-v1".into(),
            required_scopes: vec!["channels.receive".into(), "channels.deliver".into()],
        },
        bundle.path().to_path_buf(),
        executable,
        Vec::new(),
    )
    .expect("provider");
    let route = ChannelRouteKey {
        channel: "plugin.fixture.local".into(),
        account: "local".into(),
        peer: "user".into(),
        thread: "main".into(),
    };
    let account = ChannelProviderAccount {
        id: "local".into(),
        provider_id: "plugin.fixture.local".into(),
        display_name: "Fixture".into(),
        secret_ref: None,
        enabled: true,
        route_allowlist: vec![route.clone()],
        config_revision: 1,
    };
    provider.configure(&account).await.expect("configure");
    provider.start().await.expect("start");
    assert_eq!(
        provider.health().await.expect("health").state,
        ChannelProviderHealthState::Healthy
    );
    let envelope = ChannelEnvelope {
        message_id: ChannelMessageId::from("message-1"),
        route: route.clone(),
        sender: "user".into(),
        text: "hello".into(),
        metadata: serde_json::json!({"fixture":true}),
        authenticated: true,
        bot_generated: false,
        received_at_ms: 1,
    };
    assert_eq!(
        provider
            .receive(
                Some("channel-secret-never-in-argv-or-env"),
                envelope.clone(),
            )
            .await
            .expect("receive"),
        envelope
    );
    let delivery = DeliveryAttempt {
        id: ChannelDeliveryId::from("delivery-1"),
        route,
        idempotency_key: "delivery-key-1".into(),
        text: "reply".into(),
        status: DeliveryAttemptStatus::Claimed,
        attempt: 1,
        next_attempt_at_ms: None,
        error_code: None,
    };
    let delivered = provider.deliver(&delivery).await.expect("deliver");
    assert!(delivered.delivered);
    assert!(!delivered.retryable);
    assert_eq!(delivered.result_code, "fixture_delivered");
    provider.ack(&delivery).await.expect("ack");
    let mut reloaded = account;
    reloaded.config_revision = 2;
    provider.reload(&reloaded).await.expect("reload");
    provider.stop().await.expect("stop");

    let launches = sandbox.launches.lock().expect("launches");
    assert_eq!(launches.len(), 8);
    assert!(launches.iter().all(|launch| {
        launch
            .args
            .iter()
            .all(|value| !value.contains("channel-secret"))
            && launch.environment.iter().all(|(key, value)| {
                !key.contains("channel-secret") && !value.contains("channel-secret")
            })
    }));
    assert_eq!(
        launches
            .iter()
            .filter(|launch| launch.stdin.contains("channel-secret-never-in-argv-or-env"))
            .count(),
        1
    );
}
