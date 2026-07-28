// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex/codex-rs/prompts/src/review_request.rs and
// codex-rs/git-utils/src/info.rs
// @ 4c43465133428898aa84f0bfc02c306ed65fb66a.
// Modified for Hachimi: fixed checkout-bound Git commands and bounded output.

use hachimi_protocol::ReviewTarget;

use crate::{WorkerContext, WorkspaceError, WorkspaceErrorCode, WorkspaceOutput};

impl WorkerContext {
    pub(super) async fn git_review_diff(
        &self,
        target: &ReviewTarget,
    ) -> Result<WorkspaceOutput, WorkspaceError> {
        match target {
            ReviewTarget::UncommittedChanges | ReviewTarget::Custom(_) => {
                let diff = self
                    .run_process(
                        "git",
                        &["diff", "HEAD", "--no-textconv", "--no-ext-diff", "--"],
                        &self.root,
                        60_000,
                    )
                    .await?;
                let untracked = self
                    .run_process(
                        "git",
                        &["ls-files", "--others", "--exclude-standard"],
                        &self.root,
                        30_000,
                    )
                    .await?;
                combine_uncommitted_review(diff, untracked)
            }
            ReviewTarget::BaseBranch(branch) => {
                let base = self.resolve_review_commit(branch).await?;
                let merge_base = self
                    .git_stdout(&["merge-base", "HEAD", &base], 30_000)
                    .await?;
                self.run_process(
                    "git",
                    &[
                        "diff",
                        merge_base.trim(),
                        "--no-textconv",
                        "--no-ext-diff",
                        "--",
                    ],
                    &self.root,
                    60_000,
                )
                .await
            }
            ReviewTarget::Commit(revision) => {
                let commit = self.resolve_review_commit(revision).await?;
                self.run_process(
                    "git",
                    &[
                        "show",
                        "--format=",
                        "--no-textconv",
                        "--no-ext-diff",
                        &commit,
                        "--",
                    ],
                    &self.root,
                    60_000,
                )
                .await
            }
        }
    }

    async fn resolve_review_commit(&self, revision: &str) -> Result<String, WorkspaceError> {
        let revision = revision.trim();
        if revision.is_empty() || revision.len() > 512 || revision.chars().any(char::is_control) {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::InvalidRequest,
                "review Git revision is invalid",
            ));
        }
        let commit_expression = format!("{revision}^{{commit}}");
        self.git_stdout(
            &[
                "rev-parse",
                "--verify",
                "--end-of-options",
                &commit_expression,
            ],
            30_000,
        )
        .await
    }

    async fn git_stdout(&self, args: &[&str], timeout_ms: u64) -> Result<String, WorkspaceError> {
        match self
            .run_process("git", args, &self.root, timeout_ms)
            .await?
        {
            WorkspaceOutput::Process {
                exit_code: Some(0),
                stdout,
                ..
            } if !stdout.trim().is_empty() => Ok(stdout.trim().to_owned()),
            WorkspaceOutput::Process {
                exit_code, stderr, ..
            } => Err(WorkspaceError::new(
                WorkspaceErrorCode::ProcessFailed,
                format!(
                    "review Git command failed with exit code {exit_code:?}: {}",
                    stderr.trim()
                ),
            )),
            _ => Err(WorkspaceError::new(
                WorkspaceErrorCode::ProcessFailed,
                "review Git command returned an unexpected result",
            )),
        }
    }
}

fn combine_uncommitted_review(
    diff: WorkspaceOutput,
    untracked: WorkspaceOutput,
) -> Result<WorkspaceOutput, WorkspaceError> {
    let WorkspaceOutput::Process {
        exit_code,
        mut stdout,
        mut stderr,
        mut truncated,
    } = diff
    else {
        return Err(WorkspaceError::new(
            WorkspaceErrorCode::ProcessFailed,
            "review Diff returned an unexpected result",
        ));
    };
    if exit_code != Some(0) {
        return Ok(WorkspaceOutput::Process {
            exit_code,
            stdout,
            stderr,
            truncated,
        });
    }
    if let WorkspaceOutput::Process {
        exit_code: untracked_exit,
        stdout: untracked_stdout,
        stderr: untracked_stderr,
        truncated: untracked_truncated,
    } = untracked
    {
        if untracked_exit == Some(0) && !untracked_stdout.trim().is_empty() {
            stdout.push_str("\n\nHACHIMI_UNTRACKED_FILES\n");
            stdout.push_str(untracked_stdout.trim());
            stdout.push('\n');
        } else if untracked_exit != Some(0) {
            stderr.push_str("\nuntracked file enumeration failed: ");
            stderr.push_str(untracked_stderr.trim());
        }
        truncated |= untracked_truncated;
    }
    Ok(WorkspaceOutput::Process {
        exit_code,
        stdout,
        stderr,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use tempfile::TempDir;

    use super::*;

    fn git(root: &std::path::Path, args: &[&str]) {
        let status = Command::new(crate::git_program())
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .expect("git");
        assert!(status.success());
    }

    #[tokio::test]
    async fn review_diff_covers_staged_and_untracked_changes() {
        let temp = TempDir::new().expect("temp");
        git(temp.path(), &["init"]);
        git(
            temp.path(),
            &["config", "user.email", "review@example.invalid"],
        );
        git(temp.path(), &["config", "user.name", "Review"]);
        std::fs::write(temp.path().join("tracked.txt"), "before\n").expect("write");
        git(temp.path(), &["add", "tracked.txt"]);
        git(temp.path(), &["commit", "-m", "base"]);
        std::fs::write(temp.path().join("tracked.txt"), "after\n").expect("write");
        git(temp.path(), &["add", "tracked.txt"]);
        std::fs::write(temp.path().join("new.txt"), "new\n").expect("write");
        let worker = WorkerContext::new(temp.path(), "checkout", 1, "token").expect("worker");
        let output = worker
            .git_review_diff(&ReviewTarget::UncommittedChanges)
            .await
            .expect("review diff");
        let WorkspaceOutput::Process { stdout, .. } = output else {
            panic!("unexpected output");
        };
        assert!(stdout.contains("+after"));
        assert!(stdout.contains("HACHIMI_UNTRACKED_FILES"));
        assert!(stdout.contains("new.txt"));
    }
}
