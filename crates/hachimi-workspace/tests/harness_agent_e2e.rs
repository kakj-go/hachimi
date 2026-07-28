use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::Command,
    sync::Arc,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use hachimi_agent::{
    AuthorizedToolContext, PersistedToolLoop, ToolLoopDriver, ToolRegistry, ToolRuntime,
    authorized_tool, workspace_tool_executors_with_diff_tracking,
};
use hachimi_approvals::{
    ApprovalBroker, ApprovalCancelFuture, ApprovalError, ApprovalFuture, ApprovalResolveFuture,
};
use hachimi_audit::NoopAudit;
use hachimi_core::WindowKind;
use hachimi_llm::OpenAiCompatibleRuntime;
use hachimi_policy::{DefaultPolicy, expand_permission_profile};
use hachimi_protocol::{
    ApprovalPolicy, ApprovalRequestRecord, ApprovalResolution, ApprovalStatus, ArtifactKind,
    BehaviorMode, ClientContext, ExecutionTarget, LlmSettings, ModelMessage, ModelRole,
    PermissionProfile, RunId, RunStatus, Scope, WorkbenchTaskStartRequest,
};
use hachimi_sandbox::SandboxStatus;
use hachimi_storage::AgentStore;
use hachimi_workbench::WorkbenchService;
use hachimi_workspace::WorkspaceHostClient;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
struct ApproveExactParameters {
    store: AgentStore,
}

impl ApprovalBroker for ApproveExactParameters {
    fn request(
        &self,
        request: ApprovalRequestRecord,
        _cancellation: CancellationToken,
    ) -> ApprovalFuture {
        let store = self.store.clone();
        Box::pin(async move {
            store
                .create_approval(&request)
                .await
                .map_err(|error| ApprovalError::Store(error.to_string()))?;
            store
                .resolve_approval(&ApprovalResolution {
                    approval_id: request.id,
                    decision: ApprovalStatus::Approved,
                    parameter_hash: request.parameter_hash,
                    run_generation: request.run_generation,
                    resolved_by: "test:approver".into(),
                    resolved_at_ms: now_ms(),
                })
                .await
                .map_err(|error| ApprovalError::Store(error.to_string()))
        })
    }

    fn resolve(&self, _resolution: ApprovalResolution) -> ApprovalResolveFuture {
        Box::pin(async { Err(ApprovalError::Unavailable) })
    }

    fn cancel_run(&self, _run_id: RunId) -> ApprovalCancelFuture {
        Box::pin(async { Ok(0) })
    }
}

#[tokio::test]
async fn mock_provider_drives_real_worker_and_persists_evidence_across_restart() {
    let repository = tempfile::tempdir().expect("repository");
    git(repository.path(), &["init", "-b", "main"]);
    git(
        repository.path(),
        &["config", "user.email", "test@example.com"],
    );
    git(repository.path(), &["config", "user.name", "Hachimi E2E"]);
    std::fs::write(repository.path().join("README.md"), "# E2E\n").expect("seed file");
    git(repository.path(), &["add", "README.md"]);
    git(repository.path(), &["commit", "-m", "initial"]);
    std::fs::write(
        repository.path().join("README.md"),
        "# E2E\nuser dirty line\n",
    )
    .expect("pre-existing dirty change");

    let responses = vec![
        tool_response(
            "call-write",
            "workspace_write_file",
            json!({
                "path": "README.md",
                "content": "# E2E\nverified by harness agent\n",
                "expectedSha256": sha256(b"# E2E\nuser dirty line\n")
            }),
        ),
        tool_response(
            "call-rename",
            "workspace_exec",
            json!({
                "program": "git",
                "args": ["mv", "README.md", "RENAMED.md"],
                "cwd": "",
                "timeoutMs": 10_000
            }),
        ),
        tool_response("call-diff", "workspace_git_diff", json!({})),
        tool_response(
            "call-exec",
            "workspace_exec",
            json!({
                "program": "git",
                "args": ["diff", "--check"],
                "cwd": "",
                "timeoutMs": 10_000
            }),
        ),
        final_response("Updated README.md and verified the diff."),
    ];
    let (base_url, provider) = spawn_provider(responses);

    let state = tempfile::tempdir().expect("state");
    let database = state.path().join("agent.sqlite3");
    let store = AgentStore::connect(&database).await.expect("store");
    let workbench = WorkbenchService::new(
        store.clone(),
        state.path().join("worktrees"),
        state.path().join("attachments"),
    );
    let project = workbench
        .add_project(repository.path())
        .await
        .expect("project");
    let request = WorkbenchTaskStartRequest {
        idempotency_key: "e2e-task".into(),
        project_id: project.id.clone(),
        prompt: "Update README.md, inspect the diff, and run git diff --check.".into(),
        execution_target: ExecutionTarget::Local {
            project_id: project.id.clone(),
        },
        behavior_mode: BehaviorMode::Default,
        approval_policy: ApprovalPolicy::OnlyWhenNeeded,
        attachment_ids: Vec::new(),
        skill_ids: Vec::new(),
    };
    let snapshot = workbench
        .create_task(
            &request,
            LlmSettings {
                base_url,
                model_name: "mock-harness".into(),
                max_input_tokens: 128_000,
                max_output_tokens: 4_096,
                structured_output_mode: hachimi_protocol::StructuredOutputMode::Disabled,
            },
            "test:user",
            &request.idempotency_key,
            &CancellationToken::new(),
        )
        .await
        .expect("task");
    store
        .acquire_checkout_write_lease(
            &snapshot.checkout.id,
            &snapshot.run.id,
            snapshot.run.generation,
        )
        .await
        .expect("lease");

    let model = Arc::new(
        OpenAiCompatibleRuntime::tool_calling(
            snapshot.run.configuration.model_snapshot.clone(),
            None,
        )
        .expect("model"),
    );
    let host = Arc::new(WorkspaceHostClient::new(
        env!("CARGO_BIN_EXE_hachimi-workspace-worker"),
        &snapshot.checkout.path,
        snapshot.checkout.id.as_str(),
        snapshot.run.generation,
    ));
    let mut client = ClientContext::for_window(WindowKind::Workbench);
    client.scopes.extend([
        Scope::AgentRun,
        Scope::WorkspaceRead,
        Scope::WorkspaceWrite,
        Scope::WorkspaceExec,
    ]);
    let authorization = AuthorizedToolContext {
        client,
        principal: "test:user".into(),
        session_id: snapshot.session.id.clone(),
        run_id: snapshot.run.id.clone(),
        run_generation: snapshot.run.generation,
        approval_policy: ApprovalPolicy::OnlyWhenNeeded,
        permission_profile: PermissionProfile::WorkspaceWrite,
        capability_grants: expand_permission_profile(
            PermissionProfile::WorkspaceWrite,
            BehaviorMode::Default,
            snapshot.session.id.clone(),
            snapshot.run.id.clone(),
            snapshot.checkout.path.clone(),
        ),
        capability_host: "workspace-worker".into(),
        run_tool_allowlist: None,
        schedule_grant_hash: None,
        sandbox_status: SandboxStatus::Enforced,
        run_store: Some(store.clone()),
        policy: Arc::new(DefaultPolicy),
        approvals: Arc::new(ApproveExactParameters {
            store: store.clone(),
        }),
        audit: Arc::new(NoopAudit),
    };
    let mut registry = ToolRegistry::new();
    for tool in workspace_tool_executors_with_diff_tracking(
        host,
        store.clone(),
        snapshot.session.id.clone(),
        snapshot.run.id.clone(),
        snapshot.checkout.id.clone(),
    ) {
        registry
            .register(authorized_tool(tool, authorization.clone()))
            .expect("register tool");
    }
    let driver = Arc::new(ToolLoopDriver::new(
        model,
        Arc::new(ToolRuntime::new(Arc::new(registry))),
    ));
    let outcome = PersistedToolLoop::new(store.clone(), driver)
        .execute_with_context(
            snapshot.run.clone(),
            vec![
                ModelMessage {
                    role: ModelRole::System,
                    content: "Use workspace tools and verify the requested change.".into(),
                    name: None,
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                },
                ModelMessage::user(request.prompt.clone()),
            ],
            Some("e2e=true"),
            CancellationToken::new(),
        )
        .await
        .expect("agent run");
    store
        .release_checkout_write_lease(
            &snapshot.checkout.id,
            &snapshot.run.id,
            snapshot.run.generation,
        )
        .await
        .expect("release lease");

    assert_eq!(outcome.tool_calls, 4);
    assert_eq!(
        outcome.final_text,
        "Updated README.md and verified the diff."
    );
    assert_eq!(
        std::fs::read_to_string(repository.path().join("RENAMED.md")).expect("result"),
        "# E2E\nverified by harness agent\n"
    );
    assert_eq!(
        store
            .get_run(&snapshot.run.id)
            .await
            .expect("run")
            .expect("persisted run")
            .status,
        RunStatus::Succeeded
    );
    let artifacts = store
        .list_session_artifacts(&snapshot.session.id)
        .await
        .expect("artifacts");
    let diff = artifacts
        .iter()
        .find(|artifact| artifact.kind == ArtifactKind::DiffEvidence)
        .expect("diff evidence");
    assert!(
        diff.metadata["byteSize"]
            .as_u64()
            .is_some_and(|size| size > 0)
    );
    assert!(
        diff.metadata["lineCount"]
            .as_u64()
            .is_some_and(|lines| lines > 0)
    );
    let command = artifacts
        .iter()
        .find(|artifact| artifact.kind == ArtifactKind::CommandEvidence)
        .expect("command evidence");
    assert_eq!(command.metadata["status"], "succeeded");
    assert_eq!(command.metadata["exitCode"], 0);
    assert_eq!(command.metadata["program"], "git");
    assert_eq!(command.metadata["argumentCount"], 3);
    let run_diff = store
        .get_run_diff_manifest(&snapshot.run.id)
        .await
        .expect("run diff")
        .expect("run diff manifest");
    assert_eq!(run_diff.files.len(), 1);
    assert_eq!(run_diff.files[0].path, "RENAMED.md");
    assert_eq!(
        run_diff.files[0].previous_path.as_deref(),
        Some("README.md")
    );
    assert_eq!(
        run_diff.files[0].status,
        hachimi_protocol::FileDiffStatus::Renamed
    );
    assert_eq!(run_diff.files[0].additions, 1);
    assert_eq!(run_diff.files[0].deletions, 1);
    assert!(
        run_diff.files[0].hunks[0]
            .lines
            .iter()
            .any(|line| line.text == "user dirty line" && line.kind == "deletion")
    );

    drop(workbench);
    drop(store);
    let reopened = AgentStore::connect(&database).await.expect("reopen");
    assert_eq!(
        reopened
            .get_run(&snapshot.run.id)
            .await
            .expect("reopened run")
            .expect("run after restart")
            .status,
        RunStatus::Succeeded
    );
    assert!(
        reopened
            .list_transcript(&snapshot.session.id)
            .await
            .expect("reopened transcript")
            .iter()
            .any(|item| matches!(
                &item.payload,
                hachimi_protocol::ItemPayload::Assistant { text }
                    if text == &outcome.final_text
            ))
    );
    assert_eq!(
        reopened
            .list_session_artifacts(&snapshot.session.id)
            .await
            .expect("reopened artifacts")
            .len(),
        3
    );
    assert_eq!(
        reopened
            .get_run_diff_manifest(&snapshot.run.id)
            .await
            .expect("reopened run diff")
            .expect("run diff after restart")
            .files[0]
            .path,
        "RENAMED.md"
    );
    provider.join().expect("provider thread");
}

fn tool_response(call_id: &str, name: &str, arguments: Value) -> Value {
    json!({
        "id": format!("response-{call_id}"),
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "id": call_id,
                    "type": "function",
                    "function": { "name": name, "arguments": arguments.to_string() }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": { "prompt_tokens": 32, "completion_tokens": 8 }
    })
}

fn final_response(content: &str) -> Value {
    json!({
        "id": "response-final",
        "choices": [{
            "message": { "role": "assistant", "content": content },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 32, "completion_tokens": 8 }
    })
}

fn spawn_provider(responses: Vec<Value>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock provider");
    let address = listener.local_addr().expect("provider address");
    let handle = thread::spawn(move || {
        for response in responses {
            let (mut stream, _) = listener.accept().expect("provider connection");
            read_request(&mut stream);
            let body = response.to_string();
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(headers.as_bytes()).expect("headers");
            stream.write_all(body.as_bytes()).expect("body");
            stream.flush().expect("flush");
        }
    });
    (format!("http://{address}/v1"), handle)
}

fn read_request(stream: &mut TcpStream) {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("read timeout");
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4_096];
    let mut expected_len = None;
    loop {
        let read = stream.read(&mut buffer).expect("read request");
        assert_ne!(read, 0, "provider request ended before its body");
        request.extend_from_slice(&buffer[..read]);
        if expected_len.is_none()
            && let Some(header_end) = find_bytes(&request, b"\r\n\r\n")
        {
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().expect("content length"))
                })
                .unwrap_or_default();
            expected_len = Some(header_end + 4 + content_length);
        }
        if expected_len.is_some_and(|length| request.len() >= length) {
            break;
        }
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn git(root: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .status()
        .expect("git");
    assert!(status.success(), "git command failed: {args:?}");
}

fn now_ms() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
