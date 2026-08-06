use std::{
    fs,
    path::{Component, Path, PathBuf},
    process::Stdio,
};

use hachimi_process_policy::{ProcessPolicy, tokio_command};
use hachimi_protocol::{
    CheckoutKind, ExecutionTarget, SessionContextBinding, WorkbenchHandoffRequest,
    WorkbenchHandoffResponse,
};
use hachimi_storage::{
    IdempotentMutationClaim, SessionCheckoutBindingUpdate, SessionEnvironmentState,
    WorkbenchHandoffJournalRecord,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::{WorkbenchError, WorkbenchService, git_optional, git_required, now_ms, sha256_text};

#[derive(Debug)]
struct CheckoutSnapshot {
    root: PathBuf,
    head: Option<String>,
    branch: Option<String>,
    status: String,
    staged_patch: PathBuf,
    unstaged_patch: PathBuf,
    untracked_root: PathBuf,
    untracked: Vec<String>,
    included_ignored: Vec<String>,
    digest: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckoutSnapshotManifest {
    head: Option<String>,
    branch: Option<String>,
    status: String,
    untracked: Vec<String>,
    included_ignored: Vec<String>,
}

const SNAPSHOT_MANIFEST: &str = "manifest.json";
const COPIED_IGNORED_MANIFEST: &str = "copied-ignored.json";

impl WorkbenchService {
    pub async fn reconcile_handoffs(&self) -> Result<u32, WorkbenchError> {
        let journals = self.store.list_unfinished_workbench_handoffs().await?;
        let mut reconciled = 0_u32;
        for journal in journals {
            match self.reconcile_handoff(&journal).await {
                Ok(()) => reconciled = reconciled.saturating_add(1),
                Err(_) => {
                    self.store
                        .update_workbench_handoff_phase(
                            &journal.id,
                            "failed",
                            Some("startup_reconciliation_required"),
                        )
                        .await?;
                }
            }
        }
        Ok(reconciled)
    }

    async fn reconcile_handoff(
        &self,
        journal: &WorkbenchHandoffJournalRecord,
    ) -> Result<(), WorkbenchError> {
        let session = self
            .store
            .get_session(&journal.session_id)
            .await?
            .ok_or_else(|| WorkbenchError::SessionNotFound(journal.session_id.clone()))?;
        if session.context.checkout_id() == Some(&journal.target_checkout_id) {
            self.store
                .update_workbench_handoff_phase(&journal.id, "committed", None)
                .await?;
            return Ok(());
        }
        if session.context.checkout_id() != Some(&journal.source_checkout_id) {
            return Err(WorkbenchError::HandoffPreconditionFailed);
        }
        let source = self
            .store
            .get_checkout(&journal.source_checkout_id)
            .await?
            .ok_or_else(|| WorkbenchError::CheckoutNotFound(journal.source_checkout_id.clone()))?;
        let target = self
            .store
            .get_checkout(&journal.target_checkout_id)
            .await?
            .ok_or_else(|| WorkbenchError::CheckoutNotFound(journal.target_checkout_id.clone()))?;
        let snapshot =
            load_checkout_snapshot(journal, Path::new(&source.path), &self.worktree_root)?;
        let source_matches = checkout_matches_snapshot(Path::new(&source.path), &snapshot).await?;
        let source_is_clean =
            checkout_is_clean_at(Path::new(&source.path), snapshot.head.as_deref()).await?;
        if !source_matches && !source_is_clean {
            return Err(WorkbenchError::HandoffTargetChanged);
        }
        let target_root = Path::new(&target.path);
        let target_is_original = checkout_is_original(
            target_root,
            journal.target_head.as_deref(),
            journal.target_branch.as_deref(),
            &journal.target_status_fingerprint,
        )
        .await?;
        let target_is_applied = checkout_matches_snapshot(target_root, &snapshot).await?;
        if !target_is_original && !target_is_applied {
            return Err(WorkbenchError::HandoffTargetChanged);
        }
        if journal.phase == "prepared" && !target_is_original {
            return Err(WorkbenchError::HandoffTargetChanged);
        }
        if target_is_applied {
            rollback_target(
                &snapshot,
                target_root,
                journal.target_head.as_deref(),
                journal.target_branch.as_deref(),
            )
            .await?;
        }
        restore_source(&snapshot).await?;
        if !checkout_matches_snapshot(Path::new(&source.path), &snapshot).await?
            || !checkout_is_original(
                target_root,
                journal.target_head.as_deref(),
                journal.target_branch.as_deref(),
                &journal.target_status_fingerprint,
            )
            .await?
        {
            return Err(WorkbenchError::HandoffFailed(
                "startup rollback verification failed".into(),
            ));
        }
        self.store
            .update_workbench_handoff_phase(&journal.id, "rolled_back", None)
            .await?;
        Ok(())
    }

    pub async fn handoff_session(
        &self,
        request: &WorkbenchHandoffRequest,
        principal: &str,
    ) -> Result<WorkbenchHandoffResponse, WorkbenchError> {
        if request.idempotency_key.trim().is_empty() || request.idempotency_key.len() > 128 {
            return Err(WorkbenchError::InvalidGitIdempotencyKey);
        }
        let request_hash = sha256_text(
            &serde_json::to_string(request).expect("Handoff requests are serializable"),
        );
        match self
            .store
            .claim_idempotent_mutation::<WorkbenchHandoffResponse>(
                principal,
                "workbench.environment.handoff",
                &request.idempotency_key,
                &request_hash,
                now_ms(),
            )
            .await?
        {
            IdempotentMutationClaim::Completed(response) => return Ok(response),
            IdempotentMutationClaim::Indeterminate => {
                return Err(WorkbenchError::GitActionIndeterminate);
            }
            IdempotentMutationClaim::Claimed => {}
        }
        let result = self.handoff_session_claimed(request, &request_hash).await;
        match &result {
            Ok(response) => {
                self.store
                    .complete_idempotent_mutation(
                        principal,
                        "workbench.environment.handoff",
                        &request.idempotency_key,
                        response,
                    )
                    .await?;
            }
            Err(_) => {
                self.store
                    .abandon_idempotent_mutation(
                        principal,
                        "workbench.environment.handoff",
                        &request.idempotency_key,
                    )
                    .await?;
            }
        }
        result
    }

    async fn handoff_session_claimed(
        &self,
        request: &WorkbenchHandoffRequest,
        request_hash: &str,
    ) -> Result<WorkbenchHandoffResponse, WorkbenchError> {
        let session = self
            .store
            .get_session(&request.session_id)
            .await?
            .ok_or_else(|| WorkbenchError::SessionNotFound(request.session_id.clone()))?;
        let (project_id, bound_checkout_id) = match &session.context {
            SessionContextBinding::Project {
                project_id,
                checkout_id,
            } => (project_id.clone(), checkout_id.clone()),
            _ => return Err(WorkbenchError::ProjectContextRequired),
        };
        if bound_checkout_id != request.source_checkout_id {
            return Err(WorkbenchError::HandoffPreconditionFailed);
        }
        let source = self
            .store
            .get_checkout(&request.source_checkout_id)
            .await?
            .ok_or_else(|| WorkbenchError::CheckoutNotFound(request.source_checkout_id.clone()))?;
        if source.kind == request.target_kind {
            return Err(WorkbenchError::InvalidHandoffTarget);
        }
        let state = self
            .store
            .ensure_session_environment_state(
                &session.id,
                &source.id,
                source.kind,
                source.head_revision.as_deref(),
            )
            .await?;
        if state.binding_revision != request.expected_binding_revision
            || self.store.checkout_has_active_runs(&source.id).await?
            || self.store.checkout_has_write_lease(&source.id).await?
        {
            return Err(WorkbenchError::HandoffPreconditionFailed);
        }
        let source_root = Path::new(&source.path);
        let source_head = git_optional(source_root, &["rev-parse", "HEAD"]).await?;
        let source_status = status(source_root).await?;
        if source_head != request.expected_head
            || sha256_text(&source_status) != request.status_fingerprint
        {
            return Err(WorkbenchError::HandoffPreconditionFailed);
        }
        let target = self
            .resolve_handoff_target(
                &project_id,
                request.target_kind,
                &state,
                source_head.as_deref(),
            )
            .await?;
        if self.store.checkout_has_active_runs(&target.id).await?
            || self.store.checkout_has_write_lease(&target.id).await?
        {
            return Err(WorkbenchError::HandoffPreconditionFailed);
        }
        let target_root = Path::new(&target.path);
        let target_status = status(target_root).await?;
        if !target_status.is_empty() {
            return Err(WorkbenchError::HandoffTargetChanged);
        }
        let target_head = git_optional(target_root, &["rev-parse", "HEAD"]).await?;
        let target_status_fingerprint = sha256_text(&target_status);
        if state.inactive_head.is_some()
            && (state.inactive_head != target_head
                || state.inactive_status_fingerprint.as_deref()
                    != Some(target_status_fingerprint.as_str()))
        {
            return Err(WorkbenchError::HandoffTargetChanged);
        }
        let target_branch = branch(target_root).await?;
        let snapshot_root = self
            .worktree_root
            .join("handoffs")
            .join(&session.id.0)
            .join(&request_hash[..24]);
        if snapshot_root.exists() {
            fs::remove_dir_all(&snapshot_root).map_err(WorkbenchError::Io)?;
        }
        fs::create_dir_all(&snapshot_root).map_err(WorkbenchError::Io)?;
        let snapshot = capture_checkout(source_root, &snapshot_root, source.kind).await?;
        verify_snapshot_rebuild(source_root, &snapshot).await?;
        let journal_id = format!("handoff-{}", &request_hash[..32]);
        self.store
            .start_workbench_handoff_journal(
                &journal_id,
                &request.idempotency_key,
                &session.id,
                &source.id,
                &target.id,
                snapshot.head.as_deref(),
                snapshot.branch.as_deref(),
                &request.status_fingerprint,
                target_head.as_deref(),
                target_branch.as_deref(),
                &sha256_text(&target_status),
                state.binding_revision,
                snapshot_root.to_string_lossy().as_ref(),
                &snapshot.digest,
            )
            .await?;

        let released_source_branch = if source.kind == CheckoutKind::ManagedWorktree
            && target.kind == CheckoutKind::Local
            && snapshot.branch.is_some()
        {
            git_required(source_root, &["switch", "--detach"], None).await?;
            true
        } else {
            false
        };
        let apply_result = self
            .apply_snapshot_to_target(
                &snapshot,
                &source,
                target_root,
                target_head.as_deref(),
                target_branch.as_deref(),
            )
            .await;
        if let Err(error) = apply_result {
            let _ = rollback_target(
                &snapshot,
                target_root,
                target_head.as_deref(),
                target_branch.as_deref(),
            )
            .await;
            if released_source_branch {
                let _ = restore_source(&snapshot).await;
            }
            self.store
                .update_workbench_handoff_phase(
                    &journal_id,
                    "failed",
                    Some("destination_apply_failed"),
                )
                .await?;
            return Err(error);
        }
        self.store
            .update_workbench_handoff_phase(&journal_id, "destination_applied", None)
            .await?;
        if status(target_root).await? != snapshot.status {
            let _ = rollback_target(
                &snapshot,
                target_root,
                target_head.as_deref(),
                target_branch.as_deref(),
            )
            .await;
            if released_source_branch {
                let _ = restore_source(&snapshot).await;
            }
            self.store
                .update_workbench_handoff_phase(
                    &journal_id,
                    "failed",
                    Some("destination_verify_failed"),
                )
                .await?;
            return Err(WorkbenchError::HandoffFailed(
                "destination verification failed".into(),
            ));
        }
        if let Err(error) = clean_source(source_root).await {
            let _ = rollback_target(
                &snapshot,
                target_root,
                target_head.as_deref(),
                target_branch.as_deref(),
            )
            .await;
            let _ = restore_source(&snapshot).await;
            self.store
                .update_workbench_handoff_phase(
                    &journal_id,
                    "rolled_back",
                    Some("source_cleanup_failed"),
                )
                .await?;
            return Err(error);
        }
        self.store
            .update_workbench_handoff_phase(&journal_id, "source_cleaned", None)
            .await?;
        let clean_source_status = status(source_root).await?;
        let binding_update = SessionCheckoutBindingUpdate {
            session_id: session.id.clone(),
            project_id,
            target_checkout_id: target.id.clone(),
            target_kind: target.kind,
            expected_binding_revision: state.binding_revision,
            inactive_head: snapshot.head.clone(),
            inactive_status_fingerprint: sha256_text(&clean_source_status),
        };
        let binding = self.store.bind_session_checkout(&binding_update).await;
        let (next_session, _) = match binding {
            Ok(value) => value,
            Err(error) => {
                let _ = rollback_target(
                    &snapshot,
                    target_root,
                    target_head.as_deref(),
                    target_branch.as_deref(),
                )
                .await;
                let _ = restore_source(&snapshot).await;
                self.store
                    .update_workbench_handoff_phase(
                        &journal_id,
                        "rolled_back",
                        Some("binding_failed"),
                    )
                    .await?;
                return Err(WorkbenchError::Store(error));
            }
        };
        self.store
            .update_workbench_handoff_phase(&journal_id, "committed", None)
            .await?;
        let environment = self.environment_snapshot(&session.id).await?;
        Ok(WorkbenchHandoffResponse {
            session: next_session,
            checkout: target,
            environment,
        })
    }

    async fn resolve_handoff_target(
        &self,
        project_id: &hachimi_protocol::ProjectId,
        target_kind: CheckoutKind,
        state: &SessionEnvironmentState,
        source_head: Option<&str>,
    ) -> Result<hachimi_protocol::CheckoutRecord, WorkbenchError> {
        if target_kind == CheckoutKind::ManagedWorktree {
            if let Some(id) = state.managed_checkout_id.as_ref()
                && let Some(checkout) = self.store.get_checkout(id).await?
            {
                return Ok(checkout);
            }
            let revision = source_head.ok_or(WorkbenchError::GitRequired)?.to_owned();
            return self
                .prepare_checkout(
                    &ExecutionTarget::ManagedWorktree {
                        project_id: project_id.clone(),
                        base_revision: revision,
                    },
                    &CancellationToken::new(),
                )
                .await;
        }
        self.prepare_checkout(
            &ExecutionTarget::Local {
                project_id: project_id.clone(),
            },
            &CancellationToken::new(),
        )
        .await
    }

    async fn apply_snapshot_to_target(
        &self,
        snapshot: &CheckoutSnapshot,
        source: &hachimi_protocol::CheckoutRecord,
        target_root: &Path,
        target_head: Option<&str>,
        _target_branch: Option<&str>,
    ) -> Result<(), WorkbenchError> {
        let source_head = snapshot
            .head
            .as_deref()
            .ok_or(WorkbenchError::GitRequired)?;
        let copied_ignored = snapshot
            .included_ignored
            .iter()
            .filter(|path| !target_root.join(path).exists())
            .cloned()
            .collect::<Vec<_>>();
        let snapshot_root = snapshot
            .staged_patch
            .parent()
            .ok_or(WorkbenchError::InvalidWorktreeRoot)?;
        fs::write(
            snapshot_root.join(COPIED_IGNORED_MANIFEST),
            serde_json::to_vec(&copied_ignored)?,
        )?;
        if source.kind == CheckoutKind::Local {
            git_required(target_root, &["switch", "--detach", source_head], None).await?;
        } else if let Some(source_branch) = snapshot.branch.as_deref() {
            git_required(target_root, &["switch", source_branch], None).await?;
        } else {
            let target_head = target_head.ok_or(WorkbenchError::HandoffHistoryDiverged)?;
            if git_required(
                target_root,
                &["merge-base", "--is-ancestor", target_head, source_head],
                None,
            )
            .await
            .is_err()
            {
                return Err(WorkbenchError::HandoffHistoryDiverged);
            }
            git_required(target_root, &["merge", "--ff-only", source_head], None).await?;
        }
        git_required(target_root, &["reset", "--hard", source_head], None).await?;
        git_required(target_root, &["clean", "-fd"], None).await?;
        apply_patch_file(target_root, &snapshot.staged_patch, true).await?;
        apply_patch_file(target_root, &snapshot.unstaged_patch, false).await?;
        copy_snapshot_files(
            &snapshot.untracked_root,
            target_root,
            &snapshot.untracked,
            false,
        )?;
        if source.kind == CheckoutKind::Local {
            copy_snapshot_files(
                &snapshot.untracked_root,
                target_root,
                &snapshot.included_ignored,
                true,
            )?;
        }
        Ok(())
    }
}

async fn capture_checkout(
    root: &Path,
    snapshot_root: &Path,
    kind: CheckoutKind,
) -> Result<CheckoutSnapshot, WorkbenchError> {
    let head = git_optional(root, &["rev-parse", "HEAD"]).await?;
    let branch = branch(root).await?;
    let status = status(root).await?;
    let staged_patch = snapshot_root.join("staged.patch");
    let unstaged_patch = snapshot_root.join("unstaged.patch");
    fs::write(
        &staged_patch,
        git_patch(root, &["diff", "--cached", "--binary", "HEAD", "--"]).await?,
    )?;
    fs::write(
        &unstaged_patch,
        git_patch(root, &["diff", "--binary", "--"]).await?,
    )?;
    let untracked = nul_paths(
        &git_required(
            root,
            &["ls-files", "--others", "--exclude-standard", "-z"],
            None,
        )
        .await?,
    )?;
    let included_ignored = if kind == CheckoutKind::Local {
        included_ignored_paths(root).await?
    } else {
        Vec::new()
    };
    let untracked_root = snapshot_root.join("files");
    fs::create_dir_all(&untracked_root)?;
    copy_snapshot_files(root, &untracked_root, &untracked, false)?;
    copy_snapshot_files(root, &untracked_root, &included_ignored, false)?;
    let digest = snapshot_digest(
        &staged_patch,
        &unstaged_patch,
        &untracked_root,
        &untracked,
        &included_ignored,
    )?;
    fs::write(
        snapshot_root.join(SNAPSHOT_MANIFEST),
        serde_json::to_vec(&CheckoutSnapshotManifest {
            head: head.clone(),
            branch: branch.clone(),
            status: status.clone(),
            untracked: untracked.clone(),
            included_ignored: included_ignored.clone(),
        })?,
    )?;
    Ok(CheckoutSnapshot {
        root: root.to_owned(),
        head,
        branch,
        status,
        staged_patch,
        unstaged_patch,
        untracked_root,
        untracked,
        included_ignored,
        digest,
    })
}

async fn verify_snapshot_rebuild(
    repository_root: &Path,
    snapshot: &CheckoutSnapshot,
) -> Result<(), WorkbenchError> {
    let head = snapshot
        .head
        .as_deref()
        .ok_or(WorkbenchError::GitRequired)?;
    let snapshot_root = snapshot
        .staged_patch
        .parent()
        .ok_or(WorkbenchError::InvalidWorktreeRoot)?;
    let verification_root = snapshot_root.join("verification-worktree");
    let verification_path = verification_root.to_string_lossy().into_owned();
    git_required(
        repository_root,
        &[
            "worktree",
            "add",
            "--detach",
            verification_path.as_str(),
            head,
        ],
        None,
    )
    .await?;

    let verification = async {
        apply_patch_file(&verification_root, &snapshot.staged_patch, true).await?;
        apply_patch_file(&verification_root, &snapshot.unstaged_patch, false).await?;
        copy_snapshot_files(
            &snapshot.untracked_root,
            &verification_root,
            &snapshot.untracked,
            false,
        )?;
        copy_snapshot_files(
            &snapshot.untracked_root,
            &verification_root,
            &snapshot.included_ignored,
            false,
        )?;
        if !checkout_matches_snapshot(&verification_root, snapshot).await? {
            return Err(WorkbenchError::HandoffFailed(
                "temporary worktree could not reproduce the checkout snapshot".into(),
            ));
        }
        Ok(())
    }
    .await;
    let cleanup = git_required(
        repository_root,
        &["worktree", "remove", "--force", verification_path.as_str()],
        None,
    )
    .await;
    match (verification, cleanup) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(WorkbenchError::HandoffFailed(format!(
            "temporary worktree cleanup failed: {error}"
        ))),
        (Ok(()), Ok(_)) => Ok(()),
    }
}

fn load_checkout_snapshot(
    journal: &WorkbenchHandoffJournalRecord,
    source_root: &Path,
    allowed_root: &Path,
) -> Result<CheckoutSnapshot, WorkbenchError> {
    let snapshot_root = PathBuf::from(&journal.snapshot_path);
    let canonical_allowed = fs::canonicalize(allowed_root)?;
    let canonical_snapshot = fs::canonicalize(&snapshot_root)?;
    if !canonical_snapshot.starts_with(canonical_allowed) {
        return Err(WorkbenchError::CheckoutOutsideManagedRoot);
    }
    let manifest: CheckoutSnapshotManifest =
        serde_json::from_slice(&fs::read(snapshot_root.join(SNAPSHOT_MANIFEST))?)?;
    for path in manifest.untracked.iter().chain(&manifest.included_ignored) {
        validate_relative(path)?;
    }
    let snapshot = CheckoutSnapshot {
        root: source_root.to_owned(),
        head: manifest.head,
        branch: manifest.branch,
        status: manifest.status,
        staged_patch: snapshot_root.join("staged.patch"),
        unstaged_patch: snapshot_root.join("unstaged.patch"),
        untracked_root: snapshot_root.join("files"),
        untracked: manifest.untracked,
        included_ignored: manifest.included_ignored,
        digest: journal.snapshot_hash.clone(),
    };
    let actual = snapshot_digest(
        &snapshot.staged_patch,
        &snapshot.unstaged_patch,
        &snapshot.untracked_root,
        &snapshot.untracked,
        &snapshot.included_ignored,
    )?;
    if actual != journal.snapshot_hash {
        return Err(WorkbenchError::HandoffFailed(
            "Handoff snapshot checksum changed".into(),
        ));
    }
    Ok(snapshot)
}

async fn checkout_matches_snapshot(
    root: &Path,
    snapshot: &CheckoutSnapshot,
) -> Result<bool, WorkbenchError> {
    if git_optional(root, &["rev-parse", "HEAD"]).await? != snapshot.head
        || status(root).await? != snapshot.status
        || git_patch(root, &["diff", "--cached", "--binary", "HEAD", "--"]).await?
            != fs::read(&snapshot.staged_patch)?
        || git_patch(root, &["diff", "--binary", "--"]).await?
            != fs::read(&snapshot.unstaged_patch)?
    {
        return Ok(false);
    }
    let current_untracked = nul_paths(
        &git_required(
            root,
            &["ls-files", "--others", "--exclude-standard", "-z"],
            None,
        )
        .await?,
    )?;
    if current_untracked != snapshot.untracked {
        return Ok(false);
    }
    for relative in snapshot.untracked.iter().chain(&snapshot.included_ignored) {
        let expected = snapshot.untracked_root.join(relative);
        let actual = root.join(relative);
        if !actual.is_file() || fs::read(expected)? != fs::read(actual)? {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn checkout_is_original(
    root: &Path,
    expected_head: Option<&str>,
    expected_branch: Option<&str>,
    expected_status_fingerprint: &str,
) -> Result<bool, WorkbenchError> {
    Ok(
        git_optional(root, &["rev-parse", "HEAD"]).await?.as_deref() == expected_head
            && branch(root).await?.as_deref() == expected_branch
            && sha256_text(&status(root).await?) == expected_status_fingerprint,
    )
}

async fn checkout_is_clean_at(
    root: &Path,
    expected_head: Option<&str>,
) -> Result<bool, WorkbenchError> {
    Ok(
        git_optional(root, &["rev-parse", "HEAD"]).await?.as_deref() == expected_head
            && status(root).await?.is_empty(),
    )
}

async fn included_ignored_paths(root: &Path) -> Result<Vec<String>, WorkbenchError> {
    let include = root.join(".worktreeinclude");
    let ignored = nul_paths(
        &git_required(
            root,
            &[
                "ls-files",
                "--others",
                "--ignored",
                "--exclude-standard",
                "-z",
            ],
            None,
        )
        .await?,
    )?;
    let ignored = ignored
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    let mut included = if include.is_file() {
        nul_paths(
            &git_required(
                root,
                &[
                    "ls-files",
                    "--others",
                    "--ignored",
                    "--exclude-from=.worktreeinclude",
                    "-z",
                ],
                None,
            )
            .await?,
        )?
    } else {
        Vec::new()
    };
    if ignored.contains("AGENTS.override.md") {
        included.push("AGENTS.override.md".to_owned());
    }
    included.retain(|path| ignored.contains(path));
    included.sort();
    included.dedup();
    Ok(included)
}

async fn restore_source(snapshot: &CheckoutSnapshot) -> Result<(), WorkbenchError> {
    if let Some(branch) = snapshot.branch.as_deref() {
        git_required(&snapshot.root, &["switch", branch], None).await?;
    } else if let Some(head) = snapshot.head.as_deref() {
        git_required(&snapshot.root, &["switch", "--detach", head], None).await?;
    }
    if let Some(head) = snapshot.head.as_deref() {
        git_required(&snapshot.root, &["reset", "--hard", head], None).await?;
    }
    apply_patch_file(&snapshot.root, &snapshot.staged_patch, true).await?;
    apply_patch_file(&snapshot.root, &snapshot.unstaged_patch, false).await?;
    copy_snapshot_files(
        &snapshot.untracked_root,
        &snapshot.root,
        &snapshot.untracked,
        false,
    )?;
    Ok(())
}

async fn restore_target(
    root: &Path,
    head: Option<&str>,
    branch: Option<&str>,
) -> Result<(), WorkbenchError> {
    git_required(root, &["clean", "-fd"], None).await?;
    if let Some(branch) = branch {
        git_required(root, &["switch", branch], None).await?;
    } else if let Some(head) = head {
        git_required(root, &["switch", "--detach", head], None).await?;
    }
    if let Some(head) = head {
        git_required(root, &["reset", "--hard", head], None).await?;
    }
    Ok(())
}

async fn rollback_target(
    snapshot: &CheckoutSnapshot,
    root: &Path,
    head: Option<&str>,
    branch: Option<&str>,
) -> Result<(), WorkbenchError> {
    remove_copied_ignored(snapshot, root)?;
    restore_target(root, head, branch).await
}

fn remove_copied_ignored(
    snapshot: &CheckoutSnapshot,
    target_root: &Path,
) -> Result<(), WorkbenchError> {
    let snapshot_root = snapshot
        .staged_patch
        .parent()
        .ok_or(WorkbenchError::InvalidWorktreeRoot)?;
    let manifest = snapshot_root.join(COPIED_IGNORED_MANIFEST);
    if !manifest.is_file() {
        return Ok(());
    }
    let paths: Vec<String> = serde_json::from_slice(&fs::read(manifest)?)?;
    for relative in paths {
        validate_relative(&relative)?;
        let source = snapshot.untracked_root.join(&relative);
        let target = target_root.join(&relative);
        if target.is_file() && fs::read(&source)? == fs::read(&target)? {
            fs::remove_file(target)?;
        }
    }
    Ok(())
}

async fn clean_source(root: &Path) -> Result<(), WorkbenchError> {
    git_required(root, &["reset", "--hard", "HEAD"], None).await?;
    git_required(root, &["clean", "-fd"], None).await?;
    Ok(())
}

async fn apply_patch_file(root: &Path, patch: &Path, index: bool) -> Result<(), WorkbenchError> {
    if fs::metadata(patch).map_or(true, |metadata| metadata.len() == 0) {
        return Ok(());
    }
    let patch = patch.to_string_lossy();
    if index {
        git_required(
            root,
            &["apply", "--index", "--binary", patch.as_ref()],
            None,
        )
        .await?;
    } else {
        git_required(root, &["apply", "--binary", patch.as_ref()], None).await?;
    }
    Ok(())
}

async fn status(root: &Path) -> Result<String, WorkbenchError> {
    git_required(
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignored=no",
        ],
        None,
    )
    .await
}

async fn git_patch(root: &Path, args: &[&str]) -> Result<Vec<u8>, WorkbenchError> {
    let output = tokio_command("git", ProcessPolicy::HiddenCaptured)
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|error| WorkbenchError::Git(error.to_string()))?;
    if !output.status.success() {
        return Err(WorkbenchError::Git(
            String::from_utf8_lossy(&output.stderr)
                .trim()
                .chars()
                .take(1_024)
                .collect(),
        ));
    }
    Ok(output.stdout)
}

async fn branch(root: &Path) -> Result<Option<String>, WorkbenchError> {
    Ok(
        git_optional(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
            .await?
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty()),
    )
}

fn nul_paths(value: &str) -> Result<Vec<String>, WorkbenchError> {
    value
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(|path| {
            validate_relative(path)?;
            Ok(path.to_owned())
        })
        .collect()
}

fn validate_relative(path: &str) -> Result<(), WorkbenchError> {
    let path = Path::new(path);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(WorkbenchError::HandoffFailed("unsafe snapshot path".into()));
    }
    Ok(())
}

fn copy_snapshot_files(
    source_root: &Path,
    target_root: &Path,
    paths: &[String],
    skip_existing: bool,
) -> Result<(), WorkbenchError> {
    for relative in paths {
        validate_relative(relative)?;
        let source = source_root.join(relative);
        let metadata = fs::symlink_metadata(&source)?;
        if !metadata.file_type().is_file() {
            continue;
        }
        let target = target_root.join(relative);
        if skip_existing && target.exists() {
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, target)?;
    }
    Ok(())
}

fn snapshot_digest(
    staged: &Path,
    unstaged: &Path,
    files_root: &Path,
    untracked: &[String],
    ignored: &[String],
) -> Result<String, WorkbenchError> {
    let mut hasher = Sha256::new();
    hasher.update(fs::read(staged)?);
    hasher.update(fs::read(unstaged)?);
    for path in untracked.iter().chain(ignored) {
        hasher.update(path.as_bytes());
        hasher.update(fs::read(files_root.join(path))?);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use hachimi_protocol::{
        BehaviorMode, CheckoutId, EntryProfile, LlmSettings, PermissionProfile, RunStatus,
        SessionId, WorkbenchEnvironmentSnapshot, WorkbenchTaskStartRequest,
    };
    use hachimi_storage::AgentStore;

    use super::*;

    struct HandoffFixture {
        _repository: tempfile::TempDir,
        _worktrees: tempfile::TempDir,
        _attachments: tempfile::TempDir,
        service: WorkbenchService,
        session_id: SessionId,
        local_checkout_id: CheckoutId,
    }

    fn git(root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .expect("git");
        assert!(
            output.status.success(),
            "git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    async fn fixture() -> HandoffFixture {
        let repository = tempfile::tempdir().expect("repository");
        git(repository.path(), &["init", "-b", "main"]);
        git(
            repository.path(),
            &["config", "user.email", "test@example.com"],
        );
        git(repository.path(), &["config", "user.name", "Hachimi Test"]);
        fs::write(repository.path().join("README.md"), "base\n").expect("readme");
        fs::write(repository.path().join(".gitignore"), "cache/**\n").expect("gitignore");
        fs::write(repository.path().join(".worktreeinclude"), "cache/**\n")
            .expect("worktreeinclude");
        git(repository.path(), &["add", "."]);
        git(repository.path(), &["commit", "-m", "initial"]);

        let store = AgentStore::connect_in_memory().await.expect("store");
        let worktrees = tempfile::tempdir().expect("worktrees");
        let attachments = tempfile::tempdir().expect("attachments");
        let service = WorkbenchService::new(store, worktrees.path(), attachments.path());
        let project = service
            .add_project(repository.path())
            .await
            .expect("project");
        let task = service
            .create_task(
                &WorkbenchTaskStartRequest {
                    idempotency_key: "handoff-task".into(),
                    entry_profile: EntryProfile::Workbench,
                    session_id: None,
                    project_id: Some(project.id.clone()),
                    prompt: "Test Handoff".into(),
                    execution_target: Some(ExecutionTarget::Local {
                        project_id: project.id,
                    }),
                    behavior_mode: BehaviorMode::Default,
                    permission_profile: PermissionProfile::Writable,
                    attachment_ids: Vec::new(),
                    skill_ids: Vec::new(),
                },
                LlmSettings::default(),
                "test-user",
                "handoff-task",
                &CancellationToken::new(),
            )
            .await
            .expect("task");
        service
            .store()
            .transition_run(&task.run.id, RunStatus::Preparing, None)
            .await
            .expect("preparing");
        service
            .store()
            .transition_run(&task.run.id, RunStatus::Running, None)
            .await
            .expect("running");
        service
            .store()
            .transition_run(&task.run.id, RunStatus::Succeeded, None)
            .await
            .expect("succeeded");
        HandoffFixture {
            _repository: repository,
            _worktrees: worktrees,
            _attachments: attachments,
            service,
            session_id: task.session.id,
            local_checkout_id: task.checkout.expect("checkout").id,
        }
    }

    fn handoff_request(
        environment: &WorkbenchEnvironmentSnapshot,
        target_kind: CheckoutKind,
        idempotency_key: &str,
    ) -> WorkbenchHandoffRequest {
        WorkbenchHandoffRequest {
            idempotency_key: idempotency_key.into(),
            session_id: environment.session_id.clone(),
            source_checkout_id: environment
                .checkout
                .as_ref()
                .expect("Project environment checkout")
                .id
                .clone(),
            target_kind,
            expected_head: environment.git.head_sha.clone(),
            status_fingerprint: environment.git.status_fingerprint.clone(),
            expected_binding_revision: environment.binding_revision,
        }
    }

    #[test]
    fn rejects_snapshot_path_traversal() {
        assert!(validate_relative("config/local.json").is_ok());
        assert!(validate_relative("../secrets.txt").is_err());
    }

    #[tokio::test]
    async fn handoff_round_trip_preserves_state_reuses_worktree_and_fences_external_changes() {
        let fixture = fixture().await;
        let local_root = fixture._repository.path();
        fs::write(local_root.join("README.md"), "base\nunstaged\n").expect("unstaged");
        fs::write(local_root.join("staged.txt"), "staged\n").expect("staged");
        git(local_root, &["add", "staged.txt"]);
        fs::write(local_root.join("untracked.txt"), "untracked\n").expect("untracked");
        fs::create_dir_all(local_root.join("cache")).expect("cache directory");
        fs::write(local_root.join("cache/settings.json"), "{\"local\":true}\n")
            .expect("ignored config");

        let local_environment = fixture
            .service
            .environment_snapshot(&fixture.session_id)
            .await
            .expect("local environment");
        let to_worktree = fixture
            .service
            .handoff_session(
                &handoff_request(
                    &local_environment,
                    CheckoutKind::ManagedWorktree,
                    "to-worktree",
                ),
                "test-user",
            )
            .await
            .expect("Local to Worktree");
        let managed_id = to_worktree.checkout.id.clone();
        let managed_root = Path::new(&to_worktree.checkout.path);
        assert_eq!(
            git(managed_root, &["diff", "--cached", "--name-only"]).trim(),
            "staged.txt"
        );
        assert!(git(managed_root, &["status", "--porcelain=v1"]).contains("README.md"));
        assert!(managed_root.join("untracked.txt").is_file());
        assert_eq!(
            fs::read_to_string(managed_root.join("cache/settings.json")).expect("copied ignored"),
            "{\"local\":true}\n"
        );
        assert!(status(local_root).await.expect("local status").is_empty());
        assert!(local_root.join("cache/settings.json").is_file());

        let to_local = fixture
            .service
            .handoff_session(
                &handoff_request(&to_worktree.environment, CheckoutKind::Local, "to-local"),
                "test-user",
            )
            .await
            .expect("Worktree to Local");
        assert_eq!(to_local.checkout.id, fixture.local_checkout_id);
        assert_eq!(
            git(local_root, &["diff", "--cached", "--name-only"]).trim(),
            "staged.txt"
        );
        assert!(local_root.join("untracked.txt").is_file());

        fs::write(managed_root.join("external.txt"), "outside\n").expect("external change");
        let changed_target = fixture
            .service
            .handoff_session(
                &handoff_request(
                    &to_local.environment,
                    CheckoutKind::ManagedWorktree,
                    "changed-target",
                ),
                "test-user",
            )
            .await;
        assert!(matches!(
            changed_target,
            Err(WorkbenchError::HandoffTargetChanged)
        ));
        let session = fixture
            .service
            .store()
            .get_session(&fixture.session_id)
            .await
            .expect("session lookup")
            .expect("session");
        assert_eq!(
            session.context.checkout_id(),
            Some(&fixture.local_checkout_id)
        );

        fs::remove_file(managed_root.join("external.txt")).expect("remove external change");
        let refreshed = fixture
            .service
            .environment_snapshot(&fixture.session_id)
            .await
            .expect("refreshed environment");
        let reused = fixture
            .service
            .handoff_session(
                &handoff_request(&refreshed, CheckoutKind::ManagedWorktree, "reuse-worktree"),
                "test-user",
            )
            .await
            .expect("reuse managed Worktree");
        assert_eq!(reused.checkout.id, managed_id);
    }

    #[tokio::test]
    async fn startup_reconciliation_rolls_back_destination_applied_state() {
        let fixture = fixture().await;
        let source = fixture
            .service
            .store()
            .get_checkout(&fixture.local_checkout_id)
            .await
            .expect("checkout lookup")
            .expect("source checkout");
        let source_root = Path::new(&source.path);
        fs::write(source_root.join("README.md"), "base\ninterrupted\n").expect("unstaged");
        fs::write(source_root.join("untracked.txt"), "pending\n").expect("untracked");
        let environment = fixture
            .service
            .environment_snapshot(&fixture.session_id)
            .await
            .expect("environment");
        let state = fixture
            .service
            .store()
            .get_session_environment_state(&fixture.session_id)
            .await
            .expect("state lookup")
            .expect("environment state");
        let project_id = source.project_id.clone();
        let target = fixture
            .service
            .resolve_handoff_target(
                &project_id,
                CheckoutKind::ManagedWorktree,
                &state,
                environment.git.head_sha.as_deref(),
            )
            .await
            .expect("target");
        let target_root = Path::new(&target.path);
        let target_head = git_optional(target_root, &["rev-parse", "HEAD"])
            .await
            .expect("target head");
        let target_branch = branch(target_root).await.expect("target branch");
        let target_status = status(target_root).await.expect("target status");
        let snapshot_root = fixture
            .service
            .worktree_root
            .join("handoffs")
            .join(fixture.session_id.as_str())
            .join("reconciliation-test");
        fs::create_dir_all(&snapshot_root).expect("snapshot root");
        let snapshot = capture_checkout(source_root, &snapshot_root, source.kind)
            .await
            .expect("capture");
        verify_snapshot_rebuild(source_root, &snapshot)
            .await
            .expect("verify snapshot");
        fixture
            .service
            .store()
            .start_workbench_handoff_journal(
                "handoff-reconciliation-test",
                "reconciliation-test",
                &fixture.session_id,
                &source.id,
                &target.id,
                snapshot.head.as_deref(),
                snapshot.branch.as_deref(),
                &environment.git.status_fingerprint,
                target_head.as_deref(),
                target_branch.as_deref(),
                &sha256_text(&target_status),
                state.binding_revision,
                snapshot_root.to_string_lossy().as_ref(),
                &snapshot.digest,
            )
            .await
            .expect("journal");
        fixture
            .service
            .apply_snapshot_to_target(
                &snapshot,
                &source,
                target_root,
                target_head.as_deref(),
                target_branch.as_deref(),
            )
            .await
            .expect("apply destination");
        fixture
            .service
            .store()
            .update_workbench_handoff_phase(
                "handoff-reconciliation-test",
                "destination_applied",
                None,
            )
            .await
            .expect("phase");

        assert_eq!(
            fixture
                .service
                .reconcile_handoffs()
                .await
                .expect("reconcile"),
            1
        );
        assert!(
            checkout_matches_snapshot(source_root, &snapshot)
                .await
                .expect("source restored")
        );
        assert!(
            checkout_is_original(
                target_root,
                target_head.as_deref(),
                target_branch.as_deref(),
                &sha256_text(&target_status),
            )
            .await
            .expect("target restored")
        );
    }
}
