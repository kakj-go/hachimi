use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

use hachimi_protocol::{
    CapabilityGrantSet, ComputerGrant, FileSystemAccess, FileSystemGrant, NetworkGrant,
    PermissionGrantScope, PermissionProfile, ProcessGrant, SandboxCapabilityReport,
    SandboxReadiness,
};

fn request(operation: WorkspaceOperation) -> WorkspaceRequestEnvelope {
    WorkspaceRequestEnvelope {
        request_id: "request".into(),
        checkout_id: "checkout".into(),
        run_generation: 7,
        worker_token: "token".into(),
        operation,
    }
}

#[test]
fn workspace_root_rejections_have_stable_migration_codes() {
    assert_eq!(
        path_security_error(PathSecurityError::UnsupportedRoot).code,
        WorkspaceErrorCode::UnsupportedWorkspaceRoot
    );
    assert_eq!(
        path_security_error(PathSecurityError::OwnershipMismatch).code,
        WorkspaceErrorCode::WorkspaceOwnershipMismatch
    );
    assert_eq!(
        path_security_error(PathSecurityError::ProtectedRoot).code,
        WorkspaceErrorCode::ProtectedWorkspaceRoot
    );
}

#[tokio::test]
async fn reads_replaces_and_rejects_stale_writes() {
    let directory = tempfile::tempdir().expect("directory");
    std::fs::write(directory.path().join("demo.txt"), "alpha\nbeta\n").expect("seed");
    let context = WorkerContext::new(directory.path(), "checkout", 7, "token").expect("context");
    let response = context
        .handle(request(WorkspaceOperation::ReadFile {
            path: "demo.txt".into(),
        }))
        .await;
    let WorkspaceOutput::File { sha256, .. } = response.output.expect("read") else {
        panic!("file output");
    };
    let response = context
        .handle(request(WorkspaceOperation::ReplaceText {
            path: "demo.txt".into(),
            old_text: "beta".into(),
            new_text: "gamma".into(),
            expected_sha256: sha256.clone(),
            replace_all: false,
        }))
        .await;
    assert!(response.error.is_none());
    let stale = context
        .handle(request(WorkspaceOperation::WriteFile {
            path: "demo.txt".into(),
            content: "stale".into(),
            expected_sha256: Some(sha256),
        }))
        .await;
    assert_eq!(
        stale.error.expect("conflict").code,
        WorkspaceErrorCode::Conflict
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("demo.txt")).expect("updated"),
        "alpha\ngamma\n"
    );
}

#[tokio::test]
async fn apply_patch_preflights_every_target_before_committing() {
    let directory = tempfile::tempdir().expect("directory");
    std::fs::write(directory.path().join("move.txt"), "section\nold\n").expect("seed move");
    std::fs::write(directory.path().join("delete.txt"), "remove\n").expect("seed delete");
    let context = WorkerContext::new(directory.path(), "checkout", 7, "token").expect("context");
    let patch = "*** Begin Patch\n*** Add File: nested/new.txt\n+created\n*** Update File: move.txt\n*** Move to: moved.txt\n@@ section\n-old\n+new\n*** Delete File: delete.txt\n*** End Patch";
    let response = context
        .handle(request(WorkspaceOperation::ApplyPatch {
            patch: patch.into(),
        }))
        .await;
    let WorkspaceOutput::Patch { changes } = response.output.expect("patch") else {
        panic!("patch output")
    };
    assert_eq!(changes.len(), 3);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("nested/new.txt")).expect("add"),
        "created\n"
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("moved.txt")).expect("move"),
        "section\nnew\n"
    );
    assert!(!directory.path().join("move.txt").exists());
    assert!(!directory.path().join("delete.txt").exists());

    let rejected = context
        .handle(request(WorkspaceOperation::ApplyPatch {
            patch: "*** Begin Patch\n*** Add File: should-not-exist.txt\n+new\n*** Update File: missing.txt\n@@\n-old\n+new\n*** End Patch".into(),
        }))
        .await;
    assert_eq!(
        rejected.error.expect("preflight conflict").code,
        WorkspaceErrorCode::NotFound
    );
    assert!(!directory.path().join("should-not-exist.txt").exists());
}

#[tokio::test]
async fn rejects_parent_traversal_and_stale_generation() {
    let directory = tempfile::tempdir().expect("directory");
    let context = WorkerContext::new(directory.path(), "checkout", 7, "token").expect("context");
    let traversal = context
        .handle(request(WorkspaceOperation::ReadFile {
            path: "../outside.txt".into(),
        }))
        .await;
    assert_eq!(
        traversal.error.expect("traversal").code,
        WorkspaceErrorCode::PathOutsideCheckout
    );
    let mut stale = request(WorkspaceOperation::ListDirectory { path: "".into() });
    stale.run_generation = 8;
    assert_eq!(
        context.handle(stale).await.error.expect("stale").code,
        WorkspaceErrorCode::StaleGeneration
    );
}

struct CountingSandbox {
    dispatches: Arc<AtomicUsize>,
}

impl SandboxBackend for CountingSandbox {
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
    ) -> hachimi_sandbox::SandboxSpawnFuture<'_> {
        self.dispatches.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Err(hachimi_sandbox::SandboxError::NotEnforced(
                "test dispatch should not occur".into(),
            ))
        })
    }
}

struct RejectingLaunchGuard {
    validations: Arc<AtomicUsize>,
}

impl WorkspaceLaunchGuard for RejectingLaunchGuard {
    fn validate(&self, _check: WorkspaceLaunchCheck) -> WorkspaceLaunchValidationFuture {
        self.validations.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Err(WorkspaceError::new(
                WorkspaceErrorCode::StaleGeneration,
                "Run generation changed before final dispatch",
            ))
        })
    }
}

#[tokio::test]
async fn stale_generation_guard_fails_before_restricted_worker_dispatch() {
    let directory = tempfile::tempdir().expect("directory");
    let session_id = hachimi_protocol::SessionId::from("session-final-guard");
    let run_id = hachimi_protocol::RunId::from("run-final-guard");
    let root = directory.path().to_string_lossy().into_owned();
    let grants = CapabilityGrantSet {
        profile: PermissionProfile::Writable,
        scope: PermissionGrantScope::Run,
        session_id: session_id.clone(),
        run_id: Some(run_id.clone()),
        source: "test".into(),
        file_system: vec![FileSystemGrant {
            access: FileSystemAccess::Write,
            roots: vec![root],
            globs: Vec::new(),
            special_roots: Vec::new(),
        }],
        network: NetworkGrant::default(),
        process: ProcessGrant {
            spawn: true,
            interactive: false,
            allowed_commands: Vec::new(),
        },
        browser: Default::default(),
        computer: ComputerGrant::default(),
        review_each_command: true,
        expires_at_ms: None,
    };
    let dispatches = Arc::new(AtomicUsize::new(0));
    let validations = Arc::new(AtomicUsize::new(0));
    let client = WorkspaceHostClient::new(
        directory.path().join("worker-must-not-launch.exe"),
        directory.path(),
        "checkout-final-guard",
        7,
    )
    .with_sandbox(
        Arc::new(CountingSandbox {
            dispatches: dispatches.clone(),
        }),
        WorkspaceSandboxContext {
            session_id,
            run_id,
            grants,
        },
        Arc::new(RejectingLaunchGuard {
            validations: validations.clone(),
        }),
    );
    let error = client
        .execute(
            WorkspaceOperation::WriteFile {
                path: "blocked.txt".into(),
                content: "blocked".into(),
                expected_sha256: None,
            },
            Duration::from_secs(1),
            CancellationToken::new(),
        )
        .await
        .expect_err("stale generation");
    assert_eq!(error.code, WorkspaceErrorCode::StaleGeneration);
    assert_eq!(validations.load(Ordering::SeqCst), 1);
    assert_eq!(dispatches.load(Ordering::SeqCst), 0);
    assert!(!directory.path().join("blocked.txt").exists());
}
