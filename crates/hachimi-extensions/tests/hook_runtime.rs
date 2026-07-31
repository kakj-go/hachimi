use std::sync::{Arc, Mutex};

use hachimi_extensions::PluginHost;
use hachimi_protocol::{
    ContributionRuntimeState, PluginContribution, PluginContributionKind, PluginId, PluginManifest,
    SandboxCapabilityReport, SandboxReadiness,
};
use hachimi_sandbox::{
    SandboxBackend, SandboxError, SandboxLaunchSpec, SandboxNetworkPolicy, SandboxSpawnFuture,
    SandboxedChild,
};
use hachimi_storage::{AgentStore, PluginHookEventRecord};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

type FixtureLaunch = (SandboxNetworkPolicy, Vec<String>, Vec<String>);

#[derive(Debug, Default)]
struct FixtureSandbox {
    launches: Mutex<Vec<FixtureLaunch>>,
}

impl SandboxBackend for FixtureSandbox {
    fn capability_report(&self) -> SandboxCapabilityReport {
        SandboxCapabilityReport {
            backend: "fixture".into(),
            readiness: SandboxReadiness::Ready,
            os_enforced: true,
            filesystem_enforced: true,
            process_enforced: true,
            network_enforced: true,
            version: Some("fixture-v1".into()),
            stable_error_code: None,
            diagnostics: Vec::new(),
        }
    }

    fn spawn_restricted(
        &self,
        spec: SandboxLaunchSpec,
        cancellation: CancellationToken,
    ) -> SandboxSpawnFuture<'_> {
        let args = spec
            .args
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let environment = spec
            .environment
            .iter()
            .map(|(key, _)| key.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        self.launches.lock().expect("launches").push((
            spec.network_policy,
            args.clone(),
            environment,
        ));
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
            if let Some(input) = spec.stdin
                && let Some(mut stdin) = child.stdin.take()
            {
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

#[tokio::test]
async fn legacy_event_only_hook_requires_runtime_upgrade() {
    let (source, manifest) = hook_bundle(br#"{"events":["run.before"]}"#, false);
    let store = AgentStore::connect_in_memory().await.expect("store");
    let installs = tempfile::tempdir().expect("installs");
    let host = PluginHost::new(store, installs.path());
    let installed = host.install_local(source.path()).await.expect("install");
    let runtime = host
        .list_contributions(Some(&manifest.id))
        .await
        .expect("contributions")
        .remove(0);
    assert_eq!(
        runtime.diagnostic.as_deref(),
        Some("plugin_hook_runtime_upgrade_required")
    );
    assert!(
        host.set_enabled(&installed.manifest.id, true)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn hook_sidecar_executes_all_lifecycle_events_in_a_deny_network_sandbox() {
    let fixture = std::path::Path::new(env!("CARGO_BIN_EXE_hachimi-sidecar-fixture"));
    let descriptor = br#"{
        "protocolVersion":1,
        "runtime":"sandboxed_stdio_json_rpc",
        "entrypoint":"bin/hook.exe",
        "args":[],
        "events":["run.before","run.after","tool.before","tool.after","schedule.before","schedule.after"]
    }"#;
    let (source, _manifest) = hook_bundle(descriptor, true);
    std::fs::copy(fixture, source.path().join("bin/hook.exe")).expect("copy fixture");
    let store = AgentStore::connect_in_memory().await.expect("store");
    let installs = tempfile::tempdir().expect("installs");
    let host = PluginHost::new(store.clone(), installs.path());
    let sandbox = Arc::new(FixtureSandbox::default());
    host.register_sidecar_drivers(sandbox.clone())
        .await
        .expect("attach hook runtime");
    let installed = host.install_local(source.path()).await.expect("install");
    host.set_enabled(&installed.manifest.id, true)
        .await
        .expect("enable");

    for (index, event) in [
        "run.before",
        "run.after",
        "tool.before",
        "tool.after",
        "schedule.before",
        "schedule.after",
    ]
    .into_iter()
    .enumerate()
    {
        let executed = store
            .dispatch_plugin_hook_event(
                &PluginHookEventRecord {
                    event: event.into(),
                    session_id: None,
                    run_id: None,
                    run_generation: None,
                    subject: format!("fixture-{index}"),
                    result_code: "started".into(),
                    created_at_ms: i64::try_from(index).unwrap_or_default() + 1,
                },
                CancellationToken::new(),
            )
            .await
            .expect("hook dispatch");
        assert_eq!(executed, 1);
    }
    let outcomes: Vec<String> =
        sqlx::query_scalar("SELECT result_code FROM plugin_hook_executions ORDER BY id")
            .fetch_all(store.pool())
            .await
            .expect("outcomes");
    assert_eq!(outcomes.len(), 6);
    assert!(outcomes.iter().all(|code| code.starts_with("hook_")));
    let launches = sandbox.launches.lock().expect("launches");
    assert_eq!(launches.len(), 6);
    assert!(launches.iter().all(|(network, args, environment)| {
        *network == SandboxNetworkPolicy::DenyAll
            && args.is_empty()
            && environment.iter().all(|key| !key.contains("SECRET"))
    }));
}

#[tokio::test]
async fn hooks_execute_in_plugin_and_contribution_id_order() {
    let fixture = std::path::Path::new(env!("CARGO_BIN_EXE_hachimi-sidecar-fixture"));
    let descriptor = success_descriptor(&["run.before"]);
    let store = AgentStore::connect_in_memory().await.expect("store");
    let installs = tempfile::tempdir().expect("installs");
    let host = PluginHost::new(store.clone(), installs.path());
    host.register_sidecar_drivers(Arc::new(FixtureSandbox::default()))
        .await
        .expect("attach hook runtime");
    for plugin_id in ["zzz-hook", "aaa-hook"] {
        let (source, manifest) = hook_bundle_named(plugin_id, descriptor.as_bytes(), true);
        std::fs::copy(fixture, source.path().join("bin/hook.exe")).expect("copy fixture");
        host.install_local(source.path()).await.expect("install");
        host.set_enabled(&manifest.id, true).await.expect("enable");
    }

    store
        .dispatch_plugin_hook_event(&hook_record("run.before"), CancellationToken::new())
        .await
        .expect("dispatch");
    let order: Vec<String> = sqlx::query_scalar(
        "SELECT plugin_id FROM plugin_hook_executions WHERE event = 'run.before' ORDER BY id",
    )
    .fetch_all(store.pool())
    .await
    .expect("execution order");
    assert_eq!(order, ["aaa-hook", "zzz-hook"]);
}

#[tokio::test]
async fn hook_error_and_malformed_response_fail_closed_and_disable_subscription() {
    for (mode, expected) in [
        ("error", "plugin_hook_response_rejected"),
        ("malformed", "plugin_hook_response_invalid"),
    ] {
        assert_hook_failure(mode, expected, CancellationToken::new()).await;
    }
}

#[tokio::test]
async fn hook_timeout_and_cancellation_kill_the_sidecar_and_fail_closed() {
    assert_hook_failure(
        "timeout",
        "plugin_hook_process_failed",
        CancellationToken::new(),
    )
    .await;
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert_hook_failure("success", "plugin_hook_process_failed", cancelled).await;
}

#[tokio::test]
async fn hook_content_drift_is_detected_before_launch_and_disables_future_execution() {
    let fixture = std::path::Path::new(env!("CARGO_BIN_EXE_hachimi-sidecar-fixture"));
    let descriptor = success_descriptor(&["run.before"]);
    let (source, manifest) = hook_bundle_named("drift-hook", descriptor.as_bytes(), true);
    std::fs::copy(fixture, source.path().join("bin/hook.exe")).expect("copy fixture");
    let store = AgentStore::connect_in_memory().await.expect("store");
    let installs = tempfile::tempdir().expect("installs");
    let sandbox = Arc::new(FixtureSandbox::default());
    let host = PluginHost::new(store.clone(), installs.path());
    host.register_sidecar_drivers(sandbox.clone())
        .await
        .expect("attach hook runtime");
    let installed = host.install_local(source.path()).await.expect("install");
    host.set_enabled(&manifest.id, true).await.expect("enable");
    std::fs::write(
        std::path::Path::new(&installed.root_path).join("hooks/lifecycle.json"),
        format!("{descriptor}\n"),
    )
    .expect("tamper installed descriptor");

    let error = store
        .dispatch_plugin_hook_event(&hook_record("run.before"), CancellationToken::new())
        .await
        .expect_err("content drift must fail closed");
    assert!(error.to_string().contains("plugin_hook_content_drift"));
    assert_eq!(sandbox.launches.lock().expect("launches").len(), 0);
    assert_failed_contribution(&host, &store, &manifest.id, "plugin_hook_content_drift").await;
}

async fn assert_hook_failure(mode: &str, expected: &str, cancellation: CancellationToken) {
    let fixture = std::path::Path::new(env!("CARGO_BIN_EXE_hachimi-sidecar-fixture"));
    let descriptor = format!(
        r#"{{
            "protocolVersion":1,
            "runtime":"sandboxed_stdio_json_rpc",
            "entrypoint":"bin/hook.exe",
            "args":["--mode={mode}"],
            "events":["run.before"]
        }}"#
    );
    let plugin_id = PluginId::new(format!("failure-{mode}"));
    let (source, manifest) = hook_bundle_named(plugin_id.as_str(), descriptor.as_bytes(), true);
    std::fs::copy(fixture, source.path().join("bin/hook.exe")).expect("copy fixture");
    let store = AgentStore::connect_in_memory().await.expect("store");
    let installs = tempfile::tempdir().expect("installs");
    let host = PluginHost::new(store.clone(), installs.path());
    host.register_sidecar_drivers(Arc::new(FixtureSandbox::default()))
        .await
        .expect("attach hook runtime");
    host.install_local(source.path()).await.expect("install");
    host.set_enabled(&manifest.id, true).await.expect("enable");

    let error = store
        .dispatch_plugin_hook_event(&hook_record("run.before"), cancellation)
        .await
        .expect_err("Hook failure must fail closed");
    assert!(error.to_string().contains(expected), "{error}");
    assert_failed_contribution(&host, &store, &manifest.id, expected).await;
    assert_eq!(
        store
            .dispatch_plugin_hook_event(&hook_record("run.before"), CancellationToken::new())
            .await
            .expect("disabled subscription dispatch"),
        0
    );
}

async fn assert_failed_contribution(
    host: &PluginHost,
    store: &AgentStore,
    plugin_id: &PluginId,
    expected: &str,
) {
    let runtime = host
        .list_contributions(Some(plugin_id))
        .await
        .expect("contributions")
        .remove(0);
    assert_eq!(runtime.state, ContributionRuntimeState::Failed);
    assert_eq!(runtime.diagnostic.as_deref(), Some(expected));
    let enabled: i64 = sqlx::query_scalar(
        "SELECT enabled FROM plugin_hook_subscriptions WHERE plugin_id = ? AND contribution_id = 'lifecycle'",
    )
    .bind(plugin_id.as_str())
    .fetch_one(store.pool())
    .await
    .expect("subscription state");
    assert_eq!(enabled, 0);
}

fn hook_record(event: &str) -> PluginHookEventRecord {
    PluginHookEventRecord {
        event: event.into(),
        session_id: None,
        run_id: None,
        run_generation: None,
        subject: "fixture-subject".into(),
        result_code: "started".into(),
        created_at_ms: 1,
    }
}

fn success_descriptor(events: &[&str]) -> String {
    serde_json::json!({
        "protocolVersion": 1,
        "runtime": "sandboxed_stdio_json_rpc",
        "entrypoint": "bin/hook.exe",
        "args": [],
        "events": events
    })
    .to_string()
}

fn hook_bundle(descriptor: &[u8], with_bin: bool) -> (tempfile::TempDir, PluginManifest) {
    hook_bundle_named("lifecycle-fixture", descriptor, with_bin)
}

fn hook_bundle_named(
    plugin_id: &str,
    descriptor: &[u8],
    with_bin: bool,
) -> (tempfile::TempDir, PluginManifest) {
    let source = tempfile::tempdir().expect("source");
    std::fs::create_dir_all(source.path().join(".codex-plugin")).expect("manifest directory");
    std::fs::create_dir_all(source.path().join("hooks")).expect("hooks");
    if with_bin {
        std::fs::create_dir_all(source.path().join("bin")).expect("bin");
    }
    std::fs::write(source.path().join("hooks/lifecycle.json"), descriptor)
        .expect("hook descriptor");
    let manifest = PluginManifest {
        manifest_version: 1,
        id: PluginId::new(plugin_id.to_owned()),
        name: "Lifecycle fixture".into(),
        version: "1.0.0".into(),
        description: "Sandboxed lifecycle fixture".into(),
        contributions: vec![PluginContribution {
            kind: PluginContributionKind::Hook,
            id: "lifecycle".into(),
            relative_path: "hooks/lifecycle.json".into(),
            required_scopes: Vec::new(),
        }],
    };
    std::fs::write(
        source.path().join(".codex-plugin/plugin.json"),
        serde_json::to_vec_pretty(&manifest).expect("manifest"),
    )
    .expect("manifest file");
    (source, manifest)
}
