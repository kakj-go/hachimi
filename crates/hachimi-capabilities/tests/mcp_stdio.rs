use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use futures_util::FutureExt;
use hachimi_agent::{McpToolPolicy, ToolCall, ToolInvocation, mcp_tool_executors};
use hachimi_capabilities::{
    McpClientError, McpClientHandle, McpProgressFuture, McpProgressHandler,
    McpProgressNotification, McpRunCorrelation, McpServerRequest, McpServerRequestFuture,
    McpServerRequestHandler, McpServerRequestResponse, McpStdioClient, McpStdioSandboxHost,
    McpStdioServerConfig, McpSupervisor,
};
use hachimi_protocol::{
    BehaviorMode, EntryProfile, McpServerHealthState, McpServerId, McpServerRecord,
    McpServerTransport, RunId, SessionId, ToolCallId, ToolEffect, WorkloadKind,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[cfg(windows)]
use hachimi_sandbox::{
    SandboxBackend, SandboxStatus, WindowsSandboxReadinessProbe, prepare_workspace_acl,
};

fn config(id: &str) -> McpStdioServerConfig {
    let mut config = McpStdioServerConfig::new(id, env!("CARGO_BIN_EXE_hachimi-mcp-test-server"));
    config.startup_timeout = Duration::from_secs(5);
    config.request_timeout = Duration::from_secs(5);
    config
}

fn persisted_config(id: &str, enabled: bool) -> McpServerRecord {
    McpServerRecord {
        id: McpServerId::from(id),
        display_name: "Fixture".into(),
        enabled,
        transport: McpServerTransport::Stdio {
            command: env!("CARGO_BIN_EXE_hachimi-mcp-test-server").into(),
            args: Vec::new(),
            cwd: None,
        },
        headers: Vec::new(),
        read_only_tools: vec!["echo".into()],
        startup_timeout_ms: 5_000,
        request_timeout_ms: 5_000,
        max_message_bytes: 1024 * 1024,
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

#[tokio::test]
async fn initializes_discovers_and_calls_stdio_tools() {
    let client =
        McpStdioClient::connect_unrestricted_for_tests(config("fixture"), CancellationToken::new())
            .await
            .expect("connect");
    assert_eq!(client.server_info().name, "hachimi-mcp-fixture");
    assert!(client.server_info().tools_supported);
    assert!(client.server_info().resources_supported);
    assert!(client.server_info().resource_templates_supported);
    assert!(client.server_info().prompts_supported);
    let tools = client
        .list_tools(CancellationToken::new())
        .await
        .expect("tools");
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        vec!["echo", "wait", "elicit"]
    );
    let result = client
        .call_tool(
            "echo",
            json!({ "text": "hello MCP" }),
            CancellationToken::new(),
        )
        .await
        .expect("call");
    assert!(!result.is_error);
    assert_eq!(result.content[0]["text"], "hello MCP");
    assert_eq!(
        result.structured_content.expect("structured")["echoed"],
        true
    );
    let resources = client
        .list_resources(CancellationToken::new())
        .await
        .expect("resources");
    assert_eq!(
        resources
            .iter()
            .map(|resource| resource.uri.as_str())
            .collect::<Vec<_>>(),
        vec!["fixture://notes/one", "fixture://notes/two"]
    );
    let templates = client
        .list_resource_templates(CancellationToken::new())
        .await
        .expect("resource templates");
    assert_eq!(templates[0].uri_template, "fixture://notes/{id}");
    let contents = client
        .read_resource("fixture://notes/one", CancellationToken::new())
        .await
        .expect("resource contents");
    assert_eq!(contents[0].uri, "fixture://notes/one");
    assert!(
        contents[0]
            .text
            .as_deref()
            .is_some_and(|text| text.contains("fixture content"))
    );
    let prompts = client
        .list_prompts(CancellationToken::new())
        .await
        .expect("prompts");
    assert_eq!(prompts[0].name, "summarize-note");
    let prompt = client
        .get_prompt(
            "summarize-note",
            BTreeMap::from([("topic".into(), "safety".into())]),
            CancellationToken::new(),
        )
        .await
        .expect("prompt");
    assert_eq!(prompt.messages.len(), 1);
    assert_eq!(prompt.messages[0].content["type"], "text");
    let client = Arc::new(McpClientHandle::Stdio(Box::new(client)));

    let mut policy = McpToolPolicy::default();
    policy.set_effect("echo", ToolEffect::ReadOnly);
    let executors = mcp_tool_executors(Arc::clone(&client), tools, &policy);
    let echo = executors
        .iter()
        .find(|executor| executor.descriptor().effect == ToolEffect::ReadOnly)
        .expect("echo adapter");
    let descriptor = echo.descriptor();
    assert!(descriptor.name.starts_with("mcp_fixture_echo_"));
    assert_eq!(descriptor.required_scopes, vec!["connectors.invoke"]);
    let adapted = echo
        .execute(ToolInvocation {
            call: ToolCall {
                id: ToolCallId::from("adapted-call"),
                name: descriptor.name,
                arguments: json!({ "text": "through ToolRuntime" }),
                step_revision: 1,
                tool_plan_hash: "fixture-plan".into(),
                registry_revision: "fixture-registry".into(),
            },
            entry_profile: EntryProfile::Workbench,
            workload: WorkloadKind::Office,
            behavior_mode: BehaviorMode::Default,
            run_generation: 1,
            step_revision: 1,
            tool_plan_hash: "fixture-plan".into(),
            registry_revision: "fixture-registry".into(),
            cancellation: CancellationToken::new(),
        })
        .await
        .expect("adapted call");
    assert!(adapted.model_content.contains("through ToolRuntime"));
    client.shutdown().await.expect("shutdown");
}

struct AcceptFixtureElicitation;

#[derive(Default)]
struct CaptureProgress(Mutex<Vec<McpProgressNotification>>);

impl McpProgressHandler for CaptureProgress {
    fn progress(&self, notification: McpProgressNotification) -> McpProgressFuture {
        self.0.lock().expect("progress capture").push(notification);
        async {}.boxed()
    }
}

#[tokio::test]
async fn stdio_progress_notification_keeps_tool_call_correlation() {
    let client = McpStdioClient::connect_unrestricted_for_tests(
        config("progress"),
        CancellationToken::new(),
    )
    .await
    .expect("connect");
    let progress = Arc::new(CaptureProgress::default());
    let result = client
        .call_tool_with_handlers(
            "echo",
            json!({ "text": "progress" }),
            Some(McpRunCorrelation {
                session_id: SessionId::from("session"),
                run_id: RunId::from("run"),
                run_generation: 7,
                tool_call_id: ToolCallId::from("progress-call"),
            }),
            None,
            Some(progress.clone()),
            CancellationToken::new(),
        )
        .await
        .expect("tool call");
    assert!(!result.is_error);
    {
        let captured = progress.0.lock().expect("capture");
        assert_eq!(captured.len(), 1);
        assert_eq!(
            captured[0].correlation.tool_call_id.as_str(),
            "progress-call"
        );
        assert_eq!(captured[0].message.as_deref(), Some("fixture working"));
    }
    client.shutdown().await.expect("shutdown");
}

impl McpServerRequestHandler for AcceptFixtureElicitation {
    fn handle(
        &self,
        request: McpServerRequest,
        _cancellation: CancellationToken,
    ) -> McpServerRequestFuture {
        async move {
            assert_eq!(request.method, "elicitation/create");
            let correlation = request.correlation.expect("Run correlation");
            assert_eq!(correlation.run_generation, 9);
            McpServerRequestResponse::result(json!({
                "action": "accept",
                "content": { "confirmed": true }
            }))
        }
        .boxed()
    }
}

#[tokio::test]
async fn stdio_elicitation_round_trip_preserves_request_and_run_correlation() {
    let client = McpStdioClient::connect_unrestricted_for_tests(
        config("elicitation"),
        CancellationToken::new(),
    )
    .await
    .expect("connect");
    let result = client
        .call_tool_with_handler(
            "elicit",
            json!({}),
            Some(McpRunCorrelation {
                session_id: SessionId::from("session"),
                run_id: RunId::from("run"),
                run_generation: 9,
                tool_call_id: ToolCallId::from("tool-call"),
            }),
            Some(Arc::new(AcceptFixtureElicitation)),
            CancellationToken::new(),
        )
        .await
        .expect("elicitation tool call");
    assert!(!result.is_error);
    assert_eq!(
        result.structured_content.expect("structured")["confirmed"],
        true
    );
    client.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn timeout_kills_the_server_and_fences_later_calls() {
    let mut server = config("timeout");
    server.request_timeout = Duration::from_millis(30);
    let client = McpStdioClient::connect_unrestricted_for_tests(server, CancellationToken::new())
        .await
        .expect("connect");
    let error = client
        .call_tool(
            "wait",
            json!({ "milliseconds": 5_000 }),
            CancellationToken::new(),
        )
        .await
        .expect_err("timeout");
    assert!(matches!(error, McpClientError::TimedOut));
    assert!(matches!(
        client
            .call_tool("echo", json!({ "text": "late" }), CancellationToken::new())
            .await,
        Err(McpClientError::Disconnected)
    ));
}

#[tokio::test]
async fn cancellation_kills_the_server_and_rejects_late_results() {
    let client =
        McpStdioClient::connect_unrestricted_for_tests(config("cancel"), CancellationToken::new())
            .await
            .expect("connect");
    let cancellation = CancellationToken::new();
    let trigger = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        trigger.cancel();
    });
    let error = client
        .call_tool("wait", json!({ "milliseconds": 5_000 }), cancellation)
        .await
        .expect_err("cancelled");
    assert!(matches!(error, McpClientError::Cancelled));
    assert!(matches!(
        client
            .call_tool("echo", json!({ "text": "late" }), CancellationToken::new())
            .await,
        Err(McpClientError::Disconnected)
    ));
}

#[tokio::test]
async fn supervisor_applies_health_checks_and_stops_persisted_definitions() {
    let supervisor = McpSupervisor::allow_unrestricted_stdio_for_tests();
    let enabled = persisted_config("supervised", true);
    let ready = supervisor.apply(&enabled).await;
    assert_eq!(ready.health.state, McpServerHealthState::Ready);
    assert_eq!(ready.health.tool_count, 3);
    assert_eq!(ready.tools.len(), 3);
    assert_eq!(ready.resources.len(), 2);
    assert_eq!(ready.resource_templates.len(), 1);
    assert_eq!(ready.prompts.len(), 1);
    assert!(ready.inventory_errors.is_empty());
    assert!(supervisor.client_and_tools(&enabled.id).await.is_some());

    let refreshed = supervisor
        .refresh_health(&enabled.id)
        .await
        .expect("refresh");
    assert_eq!(refreshed.health.state, McpServerHealthState::Ready);
    assert!(refreshed.health.error_code.is_none());

    let stopped = supervisor.stop(&enabled.id).await;
    assert_eq!(stopped.health.state, McpServerHealthState::Stopped);
    assert!(supervisor.client_and_tools(&enabled.id).await.is_none());

    let disabled = persisted_config("supervised", false);
    let disabled = supervisor.apply(&disabled).await;
    assert_eq!(disabled.health.state, McpServerHealthState::Disabled);
    assert!(disabled.tools.is_empty());
    assert!(supervisor.remove(&enabled.id).await);
    assert!(supervisor.get(&enabled.id).await.is_none());
}

#[tokio::test]
async fn supervisor_stop_cancels_an_in_flight_stdio_call() {
    let supervisor = McpSupervisor::allow_unrestricted_stdio_for_tests();
    let enabled = persisted_config("stop-in-flight", true);
    let ready = supervisor.apply(&enabled).await;
    assert_eq!(ready.health.state, McpServerHealthState::Ready);
    let (client, _) = supervisor
        .client_and_tools(&enabled.id)
        .await
        .expect("ready client");
    let call = tokio::spawn(async move {
        client
            .call_tool(
                "wait",
                json!({ "milliseconds": 5_000 }),
                CancellationToken::new(),
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(30)).await;

    let stopped = tokio::time::timeout(Duration::from_secs(1), supervisor.stop(&enabled.id))
        .await
        .expect("stop must not wait for the request timeout");
    assert_eq!(stopped.health.state, McpServerHealthState::Stopped);
    assert!(matches!(
        call.await.expect("call task"),
        Err(McpClientError::Cancelled)
    ));
}

#[tokio::test]
async fn repeated_resource_cursor_is_rejected_without_unbounded_paging() {
    let mut server = config("duplicate-cursor");
    server.args.push("--duplicate-resource-cursor".into());
    let client = McpStdioClient::connect_unrestricted_for_tests(server, CancellationToken::new())
        .await
        .expect("connect");
    assert!(matches!(
        client.list_resources(CancellationToken::new()).await,
        Err(McpClientError::InvalidResponse(
            "pagination returned a duplicate cursor"
        ))
    ));
    client.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn supervisor_reports_stable_failure_codes_without_server_output() {
    let supervisor = McpSupervisor::allow_unrestricted_stdio_for_tests();
    let mut missing = persisted_config("missing", true);
    missing.transport = McpServerTransport::Stdio {
        command: "definitely-not-a-real-hachimi-mcp-server".into(),
        args: Vec::new(),
        cwd: None,
    };
    let failed = supervisor.apply(&missing).await;
    assert_eq!(failed.health.state, McpServerHealthState::Failed);
    assert_eq!(failed.health.error_code.as_deref(), Some("spawn_failed"));
    assert!(failed.tools.is_empty());
}

#[tokio::test]
async fn production_supervisor_fails_closed_without_stdio_host_sandbox() {
    let supervisor = McpSupervisor::default();
    let enabled = persisted_config("sandbox-required", true);
    let failed = supervisor.apply(&enabled).await;
    assert_eq!(failed.health.state, McpServerHealthState::Failed);
    assert_eq!(
        failed.health.error_code.as_deref(),
        Some("mcp_host_sandbox_not_configured")
    );
    assert!(failed.tools.is_empty());
    assert!(supervisor.client_and_tools(&enabled.id).await.is_none());
}

#[cfg(windows)]
#[tokio::test]
#[ignore = "requires the standard-user Windows sandbox release environment"]
async fn production_stdio_mcp_runs_restricted_and_cannot_connect_to_loopback() {
    let marker = std::env::var_os("HACHIMI_SANDBOX_MARKER").expect("sandbox marker env");
    let launcher = std::env::var_os("HACHIMI_SANDBOX_LAUNCHER").expect("launcher env");
    let canary = std::env::var_os("HACHIMI_SANDBOX_CANARY").expect("canary env");
    let attestation_root =
        std::env::var_os("HACHIMI_SANDBOX_ATTESTATION_ROOT").expect("attestation root env");
    let backend: Arc<dyn SandboxBackend> = Arc::new(
        WindowsSandboxReadinessProbe::new(marker).with_runtime(launcher, canary, attestation_root),
    );
    assert_eq!(
        SandboxStatus::from_report(&backend.capability_report()),
        SandboxStatus::Enforced,
        "restricted stdio MCP smoke requires a fully attested Sandbox"
    );

    let host_root = tempfile::tempdir().expect("MCP host root");
    let supervisor =
        McpSupervisor::with_stdio_sandbox(McpStdioSandboxHost::new(backend, host_root.path()));
    let listener =
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).expect("loopback listener");
    let address = listener.local_addr().expect("listener address");
    let mcp_server = std::env::var("HACHIMI_RELEASE_MCP_TEST_SERVER")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_hachimi-mcp-test-server").into());
    let preflight_temp = host_root.path().join("preflight-temp");
    std::fs::create_dir_all(&preflight_temp).expect("preflight TEMP");
    prepare_workspace_acl(
        host_root.path(),
        &preflight_temp,
        std::path::Path::new(&mcp_server),
    )
    .expect("restricted MCP preflight ACL");
    let mut preflight = std::process::Command::new(
        std::env::var_os("HACHIMI_SANDBOX_LAUNCHER").expect("launcher env"),
    )
    .arg("--")
    .arg(&mcp_server)
    .args(["--network-probe-address", &address.to_string()])
    .current_dir(host_root.path())
    .stdin(std::process::Stdio::piped())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped())
    .spawn()
    .expect("restricted MCP preflight spawn");
    use std::io::Write as _;
    preflight
        .stdin
        .take()
        .expect("preflight stdin")
        .write_all(
            br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"release-preflight","version":"1"}}}
"#,
        )
        .expect("preflight initialize request");
    let preflight = preflight.wait_with_output().expect("preflight output");
    assert!(
        preflight.status.success()
            && String::from_utf8_lossy(&preflight.stdout).contains("hachimi-mcp-fixture"),
        "restricted MCP preflight failed with {}: {}",
        preflight.status,
        String::from_utf8_lossy(&preflight.stderr)
    );
    let mut async_preflight = tokio::process::Command::new(
        std::env::var_os("HACHIMI_SANDBOX_LAUNCHER").expect("launcher env"),
    );
    async_preflight
        .arg("--")
        .arg(&mcp_server)
        .args(["--network-probe-address", &address.to_string()])
        .current_dir(host_root.path())
        .env_clear()
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    for name in [
        "PATH",
        "PATHEXT",
        "SystemRoot",
        "SystemDrive",
        "WINDIR",
        "ComSpec",
        "TEMP",
        "TMP",
        "LANG",
        "LC_ALL",
    ] {
        if let Some(value) = std::env::var_os(name) {
            async_preflight.env(name, value);
        }
    }
    for name in ["TEMP", "TMP", "USERPROFILE", "LOCALAPPDATA", "APPDATA"] {
        async_preflight.env(name, &preflight_temp);
    }
    let mut async_preflight = async_preflight
        .spawn()
        .expect("async restricted MCP preflight spawn");
    use tokio::io::AsyncWriteExt as _;
    let mut async_stdin = async_preflight.stdin.take().expect("async preflight stdin");
    async_stdin
        .write_all(
            br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"release-async-preflight","version":"1"}}}
"#,
        )
        .await
        .expect("async preflight initialize request");
    async_stdin
        .shutdown()
        .await
        .expect("async preflight stdin shutdown");
    drop(async_stdin);
    let async_preflight = async_preflight
        .wait_with_output()
        .await
        .expect("async preflight output");
    assert!(
        async_preflight.status.success()
            && String::from_utf8_lossy(&async_preflight.stdout).contains("hachimi-mcp-fixture"),
        "async restricted MCP preflight failed with {}: {}",
        async_preflight.status,
        String::from_utf8_lossy(&async_preflight.stderr)
    );
    let mut server = persisted_config("restricted-network", true);
    server.transport = McpServerTransport::Stdio {
        command: mcp_server,
        args: vec!["--network-probe-address".into(), address.to_string()],
        cwd: None,
    };
    let snapshot = supervisor.apply(&server).await;
    assert_eq!(
        snapshot.health.state,
        McpServerHealthState::Ready,
        "restricted MCP startup failed with {:?}",
        snapshot.health.error_code
    );
    let (client, tools) = supervisor
        .client_and_tools(&server.id)
        .await
        .expect("restricted MCP runtime");
    assert!(tools.iter().any(|tool| tool.name == "network_probe"));
    let result = client
        .call_tool("network_probe", json!({}), CancellationToken::new())
        .await
        .expect("network probe call");
    assert_eq!(
        result
            .structured_content
            .as_ref()
            .and_then(|value| value.get("connected"))
            .and_then(serde_json::Value::as_bool),
        Some(false),
        "restricted stdio MCP connected despite deny-all network policy"
    );
    supervisor.stop(&server.id).await;
}
