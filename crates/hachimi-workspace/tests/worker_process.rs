use std::time::Duration;

use hachimi_protocol::{DiffScope, FsSearchId, SessionId};
use hachimi_workspace::{WorkspaceHostClient, WorkspaceOperation, WorkspaceOutput};
use tokio_util::sync::CancellationToken;

#[cfg(windows)]
use hachimi_agent::{
    AuthorizedToolContext, ToolCall, ToolInvocation, ToolResultStatus, authorized_tool,
    workspace_tool_executors,
};
#[cfg(windows)]
use hachimi_approvals::NonInteractiveApproval;
#[cfg(windows)]
use hachimi_audit::NoopAudit;
#[cfg(windows)]
use hachimi_core::WindowKind;
#[cfg(windows)]
use hachimi_policy::DefaultPolicy;
#[cfg(windows)]
use hachimi_protocol::{
    ApprovalPolicy, BehaviorMode, CapabilityGrantSet, CheckoutId, ClientContext, EntryProfile,
    FileSystemAccess, FileSystemGrant, NetworkGrant, PermissionGrantScope, PermissionProfile,
    ProcessGrant, RunId, Scope, ToolCallId, WorkloadKind,
};
#[cfg(windows)]
use hachimi_sandbox::{
    SandboxBackend, SandboxStatus, WindowsSandboxReadinessProbe, attest_workspace_boundaries,
    prepare_git_mutation_acl, prepare_workspace_acl, restore_git_mutation_acl,
};
#[cfg(windows)]
use hachimi_workspace::{
    WorkspaceLaunchCheck, WorkspaceLaunchGuard, WorkspaceLaunchValidationFuture,
    WorkspaceSandboxContext,
};
#[cfg(windows)]
use std::sync::Arc;

#[cfg(windows)]
struct AllowReleaseSmokeLaunch;

#[cfg(windows)]
fn release_worker_binary() -> std::path::PathBuf {
    std::env::var_os("HACHIMI_RELEASE_WORKSPACE_WORKER")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_BIN_EXE_hachimi-workspace-worker")))
}

#[cfg(windows)]
impl WorkspaceLaunchGuard for AllowReleaseSmokeLaunch {
    fn validate(&self, _check: WorkspaceLaunchCheck) -> WorkspaceLaunchValidationFuture {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn client_reads_through_the_worker_process() {
    let directory = tempfile::tempdir().expect("directory");
    std::fs::write(directory.path().join("demo.txt"), "hello worker").expect("seed");
    let client = WorkspaceHostClient::new(
        env!("CARGO_BIN_EXE_hachimi-workspace-worker"),
        directory.path(),
        "checkout-1",
        1,
    );
    let output = client
        .execute(
            WorkspaceOperation::ReadFile {
                path: "demo.txt".into(),
            },
            Duration::from_secs(10),
            CancellationToken::new(),
        )
        .await
        .expect("worker read");
    assert!(matches!(
        output,
        WorkspaceOutput::File { content, .. } if content == "hello worker"
    ));
}

#[cfg(windows)]
#[tokio::test]
#[ignore = "requires the elevated Windows sandbox release environment"]
async fn restricted_workspace_worker_executes_a_checkout_bound_write() {
    let marker = std::env::var_os("HACHIMI_SANDBOX_MARKER").expect("sandbox marker env");
    let launcher = std::path::PathBuf::from(
        std::env::var_os("HACHIMI_SANDBOX_LAUNCHER").expect("launcher env"),
    );
    let canary =
        std::path::PathBuf::from(std::env::var_os("HACHIMI_SANDBOX_CANARY").expect("canary env"));
    let attestation_root =
        std::env::var_os("HACHIMI_SANDBOX_ATTESTATION_ROOT").expect("attestation root env");
    let backend: Arc<dyn SandboxBackend> =
        Arc::new(WindowsSandboxReadinessProbe::new(marker).with_runtime(
            &launcher,
            &canary,
            attestation_root,
        ));
    assert_eq!(
        SandboxStatus::from_report(&backend.capability_report()),
        SandboxStatus::Enforced
    );

    let directory = tempfile::tempdir().expect("checkout");
    let worker = release_worker_binary();
    let session_id = SessionId::random();
    let run_id = RunId::random();
    let checkout_id = CheckoutId::random();
    let mut client = WorkspaceHostClient::new(&worker, directory.path(), checkout_id.as_str(), 1);
    let read_only_roots = prepare_workspace_acl(directory.path(), client.run_temp_dir(), &worker)
        .expect("workspace ACL");
    attest_workspace_boundaries(
        &launcher,
        &canary,
        directory.path(),
        client.run_temp_dir(),
        &worker,
        &read_only_roots,
    )
    .expect("workspace boundary attestation");
    let root = directory.path().to_string_lossy().into_owned();
    let temp = client.run_temp_dir().to_string_lossy().into_owned();
    let grants = CapabilityGrantSet {
        profile: PermissionProfile::WorkspaceWrite,
        scope: PermissionGrantScope::Run,
        session_id: session_id.clone(),
        run_id: Some(run_id.clone()),
        source: "windows_release_smoke".into(),
        file_system: vec![FileSystemGrant {
            access: FileSystemAccess::Write,
            roots: vec![root, temp],
            globs: Vec::new(),
            special_roots: Vec::new(),
        }],
        network: NetworkGrant::default(),
        process: ProcessGrant {
            spawn: true,
            interactive: false,
            allowed_commands: vec![worker.to_string_lossy().into_owned()],
        },
        computer: Default::default(),
        review_each_command: false,
        expires_at_ms: None,
    };
    client = client.with_sandbox(
        backend,
        WorkspaceSandboxContext {
            session_id,
            run_id,
            grants,
        },
        Arc::new(AllowReleaseSmokeLaunch),
    );
    let output = client
        .execute(
            WorkspaceOperation::WriteFile {
                path: "restricted.txt".into(),
                content: "restricted worker".into(),
                expected_sha256: None,
            },
            Duration::from_secs(20),
            CancellationToken::new(),
        )
        .await
        .expect("restricted write");
    assert!(matches!(output, WorkspaceOutput::Write { .. }));
    assert_eq!(
        std::fs::read_to_string(directory.path().join("restricted.txt")).expect("written file"),
        "restricted worker"
    );
}

#[cfg(windows)]
#[tokio::test]
#[ignore = "requires the elevated Windows sandbox release environment"]
async fn restricted_workspace_worker_creates_an_empty_initial_commit_without_touching_index() {
    let marker = std::env::var_os("HACHIMI_SANDBOX_MARKER").expect("sandbox marker env");
    let launcher = std::path::PathBuf::from(
        std::env::var_os("HACHIMI_SANDBOX_LAUNCHER").expect("launcher env"),
    );
    let canary =
        std::path::PathBuf::from(std::env::var_os("HACHIMI_SANDBOX_CANARY").expect("canary env"));
    let attestation_root =
        std::env::var_os("HACHIMI_SANDBOX_ATTESTATION_ROOT").expect("attestation root env");
    let backend: Arc<dyn SandboxBackend> =
        Arc::new(WindowsSandboxReadinessProbe::new(marker).with_runtime(
            &launcher,
            &canary,
            attestation_root,
        ));
    assert_eq!(
        SandboxStatus::from_report(&backend.capability_report()),
        SandboxStatus::Enforced
    );

    let directory = tempfile::tempdir().expect("unborn Git checkout");
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(directory.path())
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()
            .expect("run Git fixture command");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    };
    git(&["init", "--initial-branch=main"]);
    std::fs::write(directory.path().join("staged.txt"), "staged\n").expect("staged fixture");
    std::fs::write(directory.path().join("untracked.txt"), "untracked\n")
        .expect("untracked fixture");
    git(&["add", "staged.txt"]);
    let index_path = directory.path().join(".git/index");
    let index_before = std::fs::read(&index_path).expect("read index before mutation");

    let worker = release_worker_binary();
    let session_id = SessionId::random();
    let run_id = RunId::random();
    let checkout_id = CheckoutId::random();
    let mut client = WorkspaceHostClient::new(&worker, directory.path(), checkout_id.as_str(), 1);
    let read_only_roots = prepare_workspace_acl(directory.path(), client.run_temp_dir(), &worker)
        .expect("workspace ACL");
    attest_workspace_boundaries(
        &launcher,
        &canary,
        directory.path(),
        client.run_temp_dir(),
        &worker,
        &read_only_roots,
    )
    .expect("workspace boundary attestation");
    let mutation_acl = prepare_git_mutation_acl(directory.path()).expect("temporary Git write ACL");
    let root = directory.path().to_string_lossy().into_owned();
    let temp = client.run_temp_dir().to_string_lossy().into_owned();
    let grants = CapabilityGrantSet {
        profile: PermissionProfile::WorkspaceWrite,
        scope: PermissionGrantScope::Run,
        session_id: session_id.clone(),
        run_id: Some(run_id.clone()),
        source: "windows_release_initial_commit_smoke".into(),
        file_system: vec![FileSystemGrant {
            access: FileSystemAccess::Write,
            roots: vec![root, temp],
            globs: Vec::new(),
            special_roots: Vec::new(),
        }],
        network: NetworkGrant::default(),
        process: ProcessGrant {
            spawn: true,
            interactive: false,
            allowed_commands: vec![worker.to_string_lossy().into_owned()],
        },
        computer: Default::default(),
        review_each_command: false,
        expires_at_ms: None,
    };
    client = client.with_sandbox(
        backend,
        WorkspaceSandboxContext {
            session_id,
            run_id,
            grants,
        },
        Arc::new(AllowReleaseSmokeLaunch),
    );
    let result = client
        .execute(
            WorkspaceOperation::GitCreateEmptyInitialCommit {
                author_name: "Hachimi Release Test".into(),
                author_email: "release-test@hachimi.invalid".into(),
                history_limit: 5,
            },
            Duration::from_secs(30),
            CancellationToken::new(),
        )
        .await;
    restore_git_mutation_acl(&mutation_acl).expect("restore normal Git ACL");
    let output = result.expect("restricted empty initial commit");
    assert!(matches!(
        output,
        WorkspaceOutput::GitMutation { response }
            if response.commit_sha.as_deref().is_some_and(|sha| !sha.is_empty())
    ));
    assert_eq!(
        std::fs::read(index_path).expect("read index after mutation"),
        index_before,
        "empty initial commit changed the index"
    );
    assert_eq!(
        String::from_utf8_lossy(&git(&["diff", "--cached", "--name-only"])).trim(),
        "staged.txt"
    );
    assert!(
        String::from_utf8_lossy(&git(&["ls-tree", "--name-only", "HEAD"]))
            .trim()
            .is_empty()
    );
}

#[cfg(windows)]
#[tokio::test]
#[ignore = "requires the elevated Windows sandbox release environment"]
async fn restricted_agent_exec_tool_runs_through_policy_and_workspace_sandbox() {
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
        SandboxStatus::Enforced
    );

    let directory = tempfile::tempdir().expect("agent Exec checkout");
    let worker = release_worker_binary();
    let powershell = std::path::PathBuf::from(
        std::env::var_os("SystemRoot").expect("SystemRoot must be available"),
    )
    .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let session_id = SessionId::random();
    let run_id = RunId::random();
    let checkout_id = CheckoutId::random();
    let mut client = WorkspaceHostClient::new(&worker, directory.path(), checkout_id.as_str(), 1);
    prepare_workspace_acl(directory.path(), client.run_temp_dir(), &worker).expect("workspace ACL");
    let root = directory.path().to_string_lossy().into_owned();
    let temp = client.run_temp_dir().to_string_lossy().into_owned();
    let grants = CapabilityGrantSet {
        profile: PermissionProfile::WorkspaceWrite,
        scope: PermissionGrantScope::Run,
        session_id: session_id.clone(),
        run_id: Some(run_id.clone()),
        source: "windows_release_agent_exec_smoke".into(),
        file_system: vec![FileSystemGrant {
            access: FileSystemAccess::Write,
            roots: vec![root, temp],
            globs: Vec::new(),
            special_roots: Vec::new(),
        }],
        network: NetworkGrant::default(),
        process: ProcessGrant {
            spawn: true,
            interactive: false,
            allowed_commands: vec![
                worker.to_string_lossy().into_owned(),
                powershell.to_string_lossy().into_owned(),
            ],
        },
        computer: Default::default(),
        review_each_command: false,
        expires_at_ms: None,
    };
    client = client.with_sandbox(
        backend,
        WorkspaceSandboxContext {
            session_id: session_id.clone(),
            run_id: run_id.clone(),
            grants: grants.clone(),
        },
        Arc::new(AllowReleaseSmokeLaunch),
    );
    let inner = workspace_tool_executors(Arc::new(client))
        .into_iter()
        .find(|tool| tool.descriptor().name == "workspace_exec")
        .expect("Agent Exec tool");
    let mut control_client = ClientContext::for_window(WindowKind::Workbench);
    control_client.scopes.insert(Scope::WorkspaceExec);
    let tool = authorized_tool(
        inner,
        AuthorizedToolContext {
            client: control_client,
            principal: "test:windows-release".into(),
            session_id,
            run_id,
            run_generation: 1,
            approval_policy: ApprovalPolicy::NeverPrompt,
            permission_profile: PermissionProfile::WorkspaceWrite,
            capability_grants: grants,
            capability_host: "workspace-worker".into(),
            run_tool_allowlist: Some(vec!["workspace_exec".into()]),
            schedule_grant_hash: Some("windows-release-smoke".into()),
            sandbox_status: SandboxStatus::Enforced,
            run_store: None,
            policy: Arc::new(DefaultPolicy),
            approvals: Arc::new(NonInteractiveApproval),
            audit: Arc::new(NoopAudit),
        },
    );
    let result = tool
        .execute(ToolInvocation {
            call: ToolCall {
                id: ToolCallId::from("windows-release-agent-exec"),
                name: "workspace_exec".into(),
                arguments: serde_json::json!({
                    "program": powershell.to_string_lossy(),
                    "args": [
                        "-NoProfile",
                        "-NonInteractive",
                        "-Command",
                        "[Console]::Out.Write('agent-exec-restricted')"
                    ],
                    "cwd": "",
                    "timeoutMs": 20_000
                }),
                step_revision: 1,
                tool_plan_hash: "windows-release-plan".into(),
                registry_revision: "windows-release-registry".into(),
            },
            entry_profile: EntryProfile::Workbench,
            workload: WorkloadKind::Coding,
            behavior_mode: BehaviorMode::Default,
            run_generation: 1,
            step_revision: 1,
            tool_plan_hash: "windows-release-plan".into(),
            registry_revision: "windows-release-registry".into(),
            cancellation: CancellationToken::new(),
        })
        .await
        .expect("Agent Exec Tool execution");
    assert_eq!(result.status, ToolResultStatus::Succeeded);
    assert!(result.model_content.contains("agent-exec-restricted"));
}

#[tokio::test]
async fn cancelled_request_does_not_launch_a_worker() {
    let directory = tempfile::tempdir().expect("directory");
    let client = WorkspaceHostClient::new(
        env!("CARGO_BIN_EXE_hachimi-workspace-worker"),
        directory.path(),
        "checkout-1",
        1,
    );
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let error = client
        .execute(
            WorkspaceOperation::ListDirectory { path: "".into() },
            Duration::from_secs(10),
            cancellation,
        )
        .await
        .expect_err("cancelled");
    assert_eq!(error.code, hachimi_workspace::WorkspaceErrorCode::Cancelled);
}

#[tokio::test]
async fn watch_server_reports_changes_and_stops_on_cancel() {
    let directory = tempfile::tempdir().expect("directory");
    std::fs::write(directory.path().join("demo.txt"), "before").expect("seed");
    let client = WorkspaceHostClient::new(
        env!("CARGO_BIN_EXE_hachimi-workspace-worker"),
        directory.path(),
        "checkout-watch",
        3,
    );
    let cancellation = CancellationToken::new();
    let mut watch = client
        .start_watch(
            SessionId::random(),
            "".into(),
            true,
            9,
            cancellation.clone(),
        )
        .await
        .expect("watch");
    tokio::time::sleep(Duration::from_millis(250)).await;
    std::fs::write(directory.path().join("demo.txt"), "after change").expect("change");
    let event = tokio::time::timeout(Duration::from_secs(5), watch.recv())
        .await
        .expect("event timeout")
        .expect("event stream")
        .expect("watch event");
    assert_eq!(event.generation, 9);
    assert!(event.paths.iter().any(|path| path == "demo.txt"));
    cancellation.cancel();
}

#[tokio::test]
async fn search_session_updates_generation_without_spawning_a_second_session() {
    let directory = tempfile::tempdir().expect("directory");
    for index in 0..600 {
        std::fs::write(
            directory.path().join(format!("alpha-beta-{index:04}.rs")),
            "fixture",
        )
        .expect("seed");
    }
    let client = WorkspaceHostClient::new(
        env!("CARGO_BIN_EXE_hachimi-workspace-worker"),
        directory.path(),
        "checkout-search",
        11,
    );
    let search_id = FsSearchId::random();
    let session = client
        .start_file_search(
            search_id.clone(),
            "alpha".into(),
            20,
            1,
            CancellationToken::new(),
        )
        .await
        .expect("search");
    let first = session
        .wait_for_snapshot(1, "alpha", Duration::from_secs(10))
        .await
        .expect("first snapshot");
    assert_eq!(first.search_id, search_id);
    session
        .update(2, "beta".into())
        .await
        .expect("update query");
    let second = session
        .wait_for_snapshot(2, "beta", Duration::from_secs(10))
        .await
        .expect("second snapshot");
    assert_eq!(second.search_id, search_id);
    assert_eq!(second.generation, 2);
    assert!(
        second
            .results
            .iter()
            .all(|result| result.path.contains("beta"))
    );
    session.cancel();
    session.join().await;
}

#[tokio::test]
async fn structured_diff_is_returned_per_file() {
    let directory = tempfile::tempdir().expect("directory");
    git(directory.path(), &["init", "-b", "main"]);
    git(
        directory.path(),
        &["config", "user.email", "test@example.com"],
    );
    git(directory.path(), &["config", "user.name", "Hachimi Test"]);
    std::fs::write(directory.path().join("demo.txt"), "before\n").expect("seed");
    git(directory.path(), &["add", "demo.txt"]);
    git(directory.path(), &["commit", "-m", "initial"]);
    std::fs::write(directory.path().join("demo.txt"), "after\n").expect("change");
    let client = WorkspaceHostClient::new(
        env!("CARGO_BIN_EXE_hachimi-workspace-worker"),
        directory.path(),
        "checkout-diff",
        1,
    );
    let output = client
        .execute(
            WorkspaceOperation::GitDiffStructured {
                scope: DiffScope::Checkout {
                    checkout_id: hachimi_protocol::CheckoutId::new("checkout-diff"),
                },
                base_revision: None,
            },
            Duration::from_secs(10),
            CancellationToken::new(),
        )
        .await
        .expect("diff");
    let WorkspaceOutput::Diff { snapshot } = output else {
        panic!("diff snapshot");
    };
    assert_eq!(snapshot.files.len(), 1);
    assert_eq!(snapshot.files[0].path, "demo.txt");
    assert_eq!(snapshot.files[0].additions, 1);
    assert_eq!(snapshot.files[0].deletions, 1);
}

#[tokio::test]
async fn git_workspace_snapshot_is_safe_through_the_worker_process() {
    let directory = tempfile::tempdir().expect("directory");
    git(directory.path(), &["init", "-b", "main"]);
    git(
        directory.path(),
        &["config", "user.email", "test@example.com"],
    );
    git(directory.path(), &["config", "user.name", "Hachimi Test"]);
    std::fs::write(directory.path().join("tracked.txt"), "tracked\n").expect("seed tracked");
    git(directory.path(), &["add", "tracked.txt"]);
    git(directory.path(), &["commit", "-m", "initial"]);
    std::fs::write(directory.path().join("untracked.txt"), "untracked\n").expect("seed untracked");

    let client = WorkspaceHostClient::new(
        env!("CARGO_BIN_EXE_hachimi-workspace-worker"),
        directory.path(),
        "checkout-git-snapshot",
        5,
    );
    let output = client
        .execute(
            WorkspaceOperation::GitWorkspaceSnapshot { history_limit: 10 },
            Duration::from_secs(10),
            CancellationToken::new(),
        )
        .await
        .expect("Git workspace snapshot");
    let WorkspaceOutput::GitWorkspaceSnapshot { snapshot } = output else {
        panic!("Git workspace snapshot output");
    };
    assert_eq!(snapshot.branch.as_deref(), Some("main"));
    assert_eq!(snapshot.recent_commits[0].subject, "initial");
    assert!(
        snapshot
            .status
            .iter()
            .any(|entry| entry.path == "untracked.txt" && entry.index_status == "?")
    );
}

#[tokio::test]
async fn repository_textconv_cannot_execute_during_read_only_diff() {
    let directory = tempfile::tempdir().expect("directory");
    let (textconv_command, textconv_marker) = textconv_canary(directory.path());
    git(directory.path(), &["init", "-b", "main"]);
    git(
        directory.path(),
        &["config", "user.email", "test@example.com"],
    );
    git(directory.path(), &["config", "user.name", "Hachimi Test"]);
    git(
        directory.path(),
        &[
            "config",
            "diff.hachimi-adversarial.textconv",
            &textconv_command,
        ],
    );
    std::fs::write(
        directory.path().join(".gitattributes"),
        "*.txt diff=hachimi-adversarial\n",
    )
    .expect("attributes");
    std::fs::write(directory.path().join("demo.txt"), "before\n").expect("seed");
    git(directory.path(), &["add", ".gitattributes", "demo.txt"]);
    git(directory.path(), &["commit", "-m", "initial"]);
    std::fs::write(directory.path().join("demo.txt"), "after\n").expect("change");

    let client = WorkspaceHostClient::new(
        env!("CARGO_BIN_EXE_hachimi-workspace-worker"),
        directory.path(),
        "checkout-no-textconv",
        13,
    );
    let scope = DiffScope::Checkout {
        checkout_id: hachimi_protocol::CheckoutId::new("checkout-no-textconv"),
    };
    for (name, operation) in [
        ("legacy", WorkspaceOperation::GitDiff),
        (
            "structured",
            WorkspaceOperation::GitDiffStructured {
                scope: scope.clone(),
                base_revision: None,
            },
        ),
        (
            "chunked",
            WorkspaceOperation::GitDiffFileChunk {
                scope,
                path: "demo.txt".into(),
                base_revision: None,
                offset: 0,
                limit: 256 * 1024,
                if_match: None,
            },
        ),
    ] {
        client
            .execute(operation, Duration::from_secs(10), CancellationToken::new())
            .await
            .unwrap_or_else(|error| panic!("{name} Diff failed: {error}"));
        assert!(
            !textconv_marker.exists(),
            "{name} Diff executed repository-controlled textconv configuration"
        );
    }
}

#[cfg(windows)]
fn textconv_canary(root: &std::path::Path) -> (String, std::path::PathBuf) {
    let script = root.join("textconv-canary.cmd");
    let marker = root.join("textconv-invoked.txt");
    std::fs::write(
        &script,
        "@echo off\r\n>\"%~dp0textconv-invoked.txt\" echo invoked\r\ntype \"%~1\"\r\n",
    )
    .expect("textconv canary");
    (script.to_string_lossy().replace('\\', "/"), marker)
}

#[cfg(unix)]
fn textconv_canary(root: &std::path::Path) -> (String, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt as _;

    let script = root.join("textconv-canary.sh");
    let marker = root.join("textconv-invoked.txt");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf invoked > \"$(dirname \"$0\")/textconv-invoked.txt\"\ncat \"$1\"\n",
    )
    .expect("textconv canary");
    let mut permissions = std::fs::metadata(&script)
        .expect("textconv canary metadata")
        .permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&script, permissions).expect("textconv canary executable");
    (script.to_string_lossy().into_owned(), marker)
}

#[tokio::test]
async fn checkout_diff_file_is_streamed_in_etagged_chunks() {
    let directory = tempfile::tempdir().expect("directory");
    git(directory.path(), &["init", "-b", "main"]);
    git(
        directory.path(),
        &["config", "user.email", "test@example.com"],
    );
    git(directory.path(), &["config", "user.name", "Hachimi Test"]);
    std::fs::write(directory.path().join("demo.txt"), "before\n").expect("seed");
    git(directory.path(), &["add", "demo.txt"]);
    git(directory.path(), &["commit", "-m", "initial"]);
    std::fs::write(directory.path().join("demo.txt"), "after\n").expect("change");
    let client = WorkspaceHostClient::new(
        env!("CARGO_BIN_EXE_hachimi-workspace-worker"),
        directory.path(),
        "checkout-diff-chunk",
        7,
    );
    let scope = DiffScope::Checkout {
        checkout_id: hachimi_protocol::CheckoutId::new("checkout-diff-chunk"),
    };
    let output = client
        .execute(
            WorkspaceOperation::GitDiffFileChunk {
                scope: scope.clone(),
                path: "demo.txt".into(),
                base_revision: None,
                offset: 0,
                limit: 32,
                if_match: None,
            },
            Duration::from_secs(10),
            CancellationToken::new(),
        )
        .await
        .expect("first Diff chunk");
    let WorkspaceOutput::DiffFileChunk { chunk } = output else {
        panic!("Diff file chunk");
    };
    assert_eq!(chunk.path, "demo.txt");
    assert!(chunk.byte_size > chunk.next_offset);
    assert!(!chunk.eof);
    let output = client
        .execute(
            WorkspaceOperation::GitDiffFileChunk {
                scope,
                path: "demo.txt".into(),
                base_revision: None,
                offset: chunk.next_offset,
                limit: 1024,
                if_match: Some(chunk.etag.clone()),
            },
            Duration::from_secs(10),
            CancellationToken::new(),
        )
        .await
        .expect("second Diff chunk");
    let WorkspaceOutput::DiffFileChunk { chunk: tail } = output else {
        panic!("Diff file tail");
    };
    assert!(tail.eof);
    assert_eq!(tail.etag, chunk.etag);
}

fn git(root: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .status()
        .expect("git");
    assert!(status.success(), "git command failed: {args:?}");
}
