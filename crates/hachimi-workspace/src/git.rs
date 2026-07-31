// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex/codex-rs/git-utils/src/{info,operations}.rs
// @ 4c43465133428898aa84f0bfc02c306ed65fb66a.
// Modified for Hachimi: Checkout-bound fixed operations, structured status,
// restricted-process dispatch, path validation, and bounded local-only commits.

use std::{
    ffi::{OsStr, OsString},
    process::Stdio,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use hachimi_protocol::{
    ForgeKind, GitCommitSummary, GitFileStatus, GitMutationResponse, GitPushResponse,
    GitRemoteRecord, GitWorkspaceSnapshot, ProjectGitSnapshot, ProjectGitState, ProjectId,
};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::{WorkerContext, WorkspaceError, WorkspaceErrorCode, WorkspaceOutput, relative_display};

const GIT_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_GIT_OUTPUT: usize = 2 * 1024 * 1024;
const MAX_PATHS: usize = 500;
const MAX_COMMIT_MESSAGE: usize = 4_096;
const DISABLED_HOOKS_PATH: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

impl WorkerContext {
    pub(crate) async fn git_project_inspect(
        &self,
        project_id: ProjectId,
    ) -> Result<WorkspaceOutput, WorkspaceError> {
        let root = match self.git_raw(["rev-parse", "--show-toplevel"], true).await {
            Ok(output) if output.status.success() => {
                String::from_utf8_lossy(&output.stdout).trim().to_owned()
            }
            Ok(_) => {
                return Ok(WorkspaceOutput::ProjectGitSnapshot {
                    snapshot: ProjectGitSnapshot {
                        project_id,
                        git_root: None,
                        state: ProjectGitState::NotRepository,
                        observed_at_ms: now_ms(),
                    },
                });
            }
            Err(_) => {
                return Ok(WorkspaceOutput::ProjectGitSnapshot {
                    snapshot: ProjectGitSnapshot {
                        project_id,
                        git_root: None,
                        state: ProjectGitState::Unavailable {
                            error_code: "git_inspection_failed".into(),
                        },
                        observed_at_ms: now_ms(),
                    },
                });
            }
        };
        let branch_output = self
            .git_raw(["symbolic-ref", "--quiet", "--short", "HEAD"], true)
            .await?;
        let branch = branch_output
            .status
            .success()
            .then(|| {
                String::from_utf8_lossy(&branch_output.stdout)
                    .trim()
                    .to_owned()
            })
            .filter(|value| !value.is_empty());
        let head_output = self
            .git_raw(["rev-parse", "--verify", "HEAD"], true)
            .await?;
        let head = head_output
            .status
            .success()
            .then(|| {
                String::from_utf8_lossy(&head_output.stdout)
                    .trim()
                    .to_owned()
            })
            .filter(|value| !value.is_empty());
        let state = match (branch, head) {
            (Some(branch), None) => ProjectGitState::Unborn { branch },
            (Some(branch), Some(head)) => ProjectGitState::Ready {
                branch: Some(branch),
                head,
            },
            (None, Some(head)) => ProjectGitState::Detached { head },
            (None, None) => ProjectGitState::Unavailable {
                error_code: "git_head_unavailable".into(),
            },
        };
        Ok(WorkspaceOutput::ProjectGitSnapshot {
            snapshot: ProjectGitSnapshot {
                project_id,
                git_root: Some(root),
                state,
                observed_at_ms: now_ms(),
            },
        })
    }

    pub(crate) async fn git_workspace_snapshot(
        &self,
        history_limit: u16,
    ) -> Result<WorkspaceOutput, WorkspaceError> {
        Ok(WorkspaceOutput::GitWorkspaceSnapshot {
            snapshot: self.git_snapshot(history_limit).await?,
        })
    }

    pub(crate) async fn git_stage(
        &self,
        paths: &[String],
        history_limit: u16,
    ) -> Result<WorkspaceOutput, WorkspaceError> {
        let paths = self.validated_git_paths(paths)?;
        let mut arguments = vec![OsString::from("add"), OsString::from("--")];
        arguments.extend(paths.into_iter().map(OsString::from));
        self.git_checked(arguments, false).await?;
        Ok(WorkspaceOutput::GitMutation {
            response: GitMutationResponse {
                snapshot: self.git_snapshot(history_limit).await?,
                commit_sha: None,
            },
        })
    }

    pub(crate) async fn git_unstage(
        &self,
        paths: &[String],
        history_limit: u16,
    ) -> Result<WorkspaceOutput, WorkspaceError> {
        let paths = self.validated_git_paths(paths)?;
        let head = self
            .git_raw(["rev-parse", "--verify", "HEAD"], true)
            .await?;
        let mut arguments = if head.status.success() {
            vec![
                OsString::from("restore"),
                OsString::from("--staged"),
                OsString::from("--"),
            ]
        } else {
            vec![
                OsString::from("rm"),
                OsString::from("--cached"),
                OsString::from("--ignore-unmatch"),
                OsString::from("--"),
            ]
        };
        arguments.extend(paths.into_iter().map(OsString::from));
        self.git_checked(arguments, false).await?;
        Ok(WorkspaceOutput::GitMutation {
            response: GitMutationResponse {
                snapshot: self.git_snapshot(history_limit).await?,
                commit_sha: None,
            },
        })
    }

    pub(crate) async fn git_commit(
        &self,
        message: &str,
        history_limit: u16,
    ) -> Result<WorkspaceOutput, WorkspaceError> {
        let message = validate_commit_message(message)?;
        let staged = self
            .git_raw(["diff", "--cached", "--quiet", "--exit-code"], true)
            .await?;
        match staged.status.code() {
            Some(1) => {}
            Some(0) => {
                return Err(WorkspaceError::new(
                    WorkspaceErrorCode::Conflict,
                    "Git commit requires at least one staged change",
                ));
            }
            _ => return Err(git_failure("inspect staged changes", &staged)),
        }
        self.git_checked(
            [
                OsString::from("-c"),
                OsString::from("commit.gpgSign=false"),
                OsString::from("commit"),
                OsString::from("--no-verify"),
                OsString::from("-m"),
                OsString::from(message),
            ],
            false,
        )
        .await?;
        let head = self
            .git_checked([OsString::from("rev-parse"), OsString::from("HEAD")], true)
            .await?;
        let commit_sha = String::from_utf8(head.stdout)
            .map_err(|_| {
                WorkspaceError::new(
                    WorkspaceErrorCode::NotText,
                    "Git returned a non-UTF-8 commit identifier",
                )
            })?
            .trim()
            .to_owned();
        Ok(WorkspaceOutput::GitMutation {
            response: GitMutationResponse {
                snapshot: self.git_snapshot(history_limit).await?,
                commit_sha: Some(commit_sha),
            },
        })
    }

    pub(crate) async fn git_create_empty_initial_commit(
        &self,
        author_name: &str,
        author_email: &str,
        history_limit: u16,
    ) -> Result<WorkspaceOutput, WorkspaceError> {
        let (author_name, author_email) = validate_identity(author_name, author_email)?;
        let head = self
            .git_raw(["rev-parse", "--verify", "HEAD"], true)
            .await?;
        if head.status.success() {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::Conflict,
                "Git repository already has an initial commit",
            ));
        }
        let symbolic = self
            .git_checked(
                [OsString::from("symbolic-ref"), OsString::from("HEAD")],
                true,
            )
            .await?;
        let reference = String::from_utf8(symbolic.stdout)
            .map_err(|_| {
                WorkspaceError::new(WorkspaceErrorCode::NotText, "Git branch is not UTF-8")
            })?
            .trim()
            .to_owned();
        if !reference.starts_with("refs/heads/") || reference.len() > 1_024 {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::Conflict,
                "initial commit requires an unborn local branch",
            ));
        }
        let tree = self
            .git_raw_with_input(
                ["hash-object", "-t", "tree", "-w", "--stdin"],
                true,
                &[],
                &[],
            )
            .await?;
        if !tree.status.success() {
            return Err(git_failure("create the empty tree", &tree));
        }
        let tree = String::from_utf8_lossy(&tree.stdout).trim().to_owned();
        let identity = [
            ("GIT_AUTHOR_NAME", author_name),
            ("GIT_AUTHOR_EMAIL", author_email),
            ("GIT_COMMITTER_NAME", author_name),
            ("GIT_COMMITTER_EMAIL", author_email),
        ];
        let commit = self
            .git_raw_with_input(
                ["commit-tree", tree.as_str(), "-m", "Initial commit"],
                false,
                &[],
                &identity,
            )
            .await?;
        if !commit.status.success() {
            return Err(git_failure("create the initial commit", &commit));
        }
        let commit_sha = String::from_utf8_lossy(&commit.stdout).trim().to_owned();
        let update = self
            .git_raw(
                [
                    OsString::from("update-ref"),
                    OsString::from(&reference),
                    OsString::from(&commit_sha),
                    OsString::new(),
                ],
                false,
            )
            .await?;
        if !update.status.success() {
            return Err(git_failure("install the initial branch reference", &update));
        }
        Ok(WorkspaceOutput::GitMutation {
            response: GitMutationResponse {
                snapshot: self.git_snapshot(history_limit).await?,
                commit_sha: Some(commit_sha),
            },
        })
    }

    pub(crate) async fn git_remotes(&self) -> Result<WorkspaceOutput, WorkspaceError> {
        let names = self.git_checked(["remote"], true).await?;
        let names = String::from_utf8(names.stdout).map_err(|_| {
            WorkspaceError::new(
                WorkspaceErrorCode::NotText,
                "Git remote names are not UTF-8",
            )
        })?;
        let mut remotes = Vec::new();
        for name in names.lines().map(str::trim).filter(|name| !name.is_empty()) {
            validate_remote_name(name)?;
            let url = self.git_checked(["remote", "get-url", name], true).await?;
            let url = String::from_utf8(url.stdout).map_err(|_| {
                WorkspaceError::new(WorkspaceErrorCode::NotText, "Git remote URL is not UTF-8")
            })?;
            let url = url.trim();
            if url.is_empty() || url.len() > 4_096 {
                return Err(WorkspaceError::new(
                    WorkspaceErrorCode::InvalidRequest,
                    "Git remote URL is empty or too large",
                ));
            }
            remotes.push(GitRemoteRecord {
                name: name.to_owned(),
                display_url: redact_remote_url(url),
                remote_url_hash: hash_remote_url(url),
                forge_kind: infer_forge_kind(url),
            });
        }
        Ok(WorkspaceOutput::GitRemotes { remotes })
    }

    pub(crate) async fn git_push(
        &self,
        remote_name: &str,
        expected_remote_url_hash: &str,
        source_ref: &str,
        target_ref: &str,
        expected_commit_oid: &str,
    ) -> Result<WorkspaceOutput, WorkspaceError> {
        validate_remote_name(remote_name)?;
        validate_git_ref(source_ref, false)?;
        validate_git_ref(target_ref, true)?;
        validate_oid(expected_commit_oid)?;
        if expected_remote_url_hash.len() != 64
            || !expected_remote_url_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::InvalidRequest,
                "Git remote URL hash must be a SHA-256 value",
            ));
        }
        let remote_url = self
            .git_checked(["remote", "get-url", remote_name], true)
            .await?;
        let remote_url = String::from_utf8(remote_url.stdout).map_err(|_| {
            WorkspaceError::new(WorkspaceErrorCode::NotText, "Git remote URL is not UTF-8")
        })?;
        if hash_remote_url(remote_url.trim()) != expected_remote_url_hash {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::Conflict,
                "Git remote URL changed after approval",
            ));
        }
        let resolved = self
            .git_checked(
                ["rev-parse", "--verify", &format!("{source_ref}^{{commit}}")],
                true,
            )
            .await?;
        let resolved = String::from_utf8_lossy(&resolved.stdout)
            .trim()
            .to_ascii_lowercase();
        if resolved != expected_commit_oid.to_ascii_lowercase() {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::Conflict,
                "Git source ref changed after approval",
            ));
        }
        self.git_checked(
            [
                "push".to_owned(),
                "--porcelain".to_owned(),
                "--".to_owned(),
                remote_name.to_owned(),
                format!("{source_ref}:{target_ref}"),
            ],
            false,
        )
        .await?;
        let receipt = self
            .git_checked(["ls-remote", "--refs", remote_name, target_ref], true)
            .await?;
        let confirmed = String::from_utf8_lossy(&receipt.stdout)
            .lines()
            .filter_map(|line| line.split_whitespace().next())
            .any(|oid| oid.eq_ignore_ascii_case(expected_commit_oid));
        if !confirmed {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::Conflict,
                "Git push returned without a matching remote ref receipt",
            ));
        }
        Ok(WorkspaceOutput::GitPush {
            response: GitPushResponse {
                remote_name: remote_name.to_owned(),
                remote_url_hash: expected_remote_url_hash.to_ascii_lowercase(),
                source_ref: source_ref.to_owned(),
                target_ref: target_ref.to_owned(),
                commit_oid: expected_commit_oid.to_ascii_lowercase(),
                confirmed: true,
                result_code: "git_push_confirmed".into(),
            },
        })
    }

    async fn git_snapshot(
        &self,
        history_limit: u16,
    ) -> Result<GitWorkspaceSnapshot, WorkspaceError> {
        let status = self.git_status_snapshot().await?;
        let WorkspaceOutput::GitStatusSnapshot { entries } = status else {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::ProcessFailed,
                "Git status returned an unexpected worker result",
            ));
        };
        let branch_output = self
            .git_raw(["symbolic-ref", "--quiet", "--short", "HEAD"], true)
            .await?;
        let branch = branch_output
            .status
            .success()
            .then(|| {
                String::from_utf8_lossy(&branch_output.stdout)
                    .trim()
                    .to_owned()
            })
            .filter(|branch| !branch.is_empty());
        let head_output = self
            .git_raw(["rev-parse", "--verify", "HEAD"], true)
            .await?;
        let head_sha = head_output
            .status
            .success()
            .then(|| {
                String::from_utf8_lossy(&head_output.stdout)
                    .trim()
                    .to_owned()
            })
            .filter(|sha| !sha.is_empty());
        let recent_commits = self.recent_commits(history_limit.clamp(1, 50)).await?;
        Ok(GitWorkspaceSnapshot {
            detached: head_sha.is_some() && branch.is_none(),
            branch,
            head_sha,
            status: entries
                .into_iter()
                .map(|entry| GitFileStatus {
                    index_status: entry.index_status.to_string(),
                    worktree_status: entry.worktree_status.to_string(),
                    path: entry.path,
                    previous_path: entry.previous_path,
                })
                .collect(),
            recent_commits,
        })
    }

    async fn recent_commits(&self, limit: u16) -> Result<Vec<GitCommitSummary>, WorkspaceError> {
        let format = "%H%x1f%h%x1f%ct%x1f%an%x1f%s%x1e";
        let output = self
            .git_raw(
                [
                    OsString::from("log"),
                    OsString::from("-n"),
                    OsString::from(limit.to_string()),
                    OsString::from(format!("--pretty=format:{format}")),
                ],
                true,
            )
            .await?;
        if !output.status.success() {
            // An unborn repository has no history but is still a valid local workflow.
            return Ok(Vec::new());
        }
        let text = String::from_utf8_lossy(&output.stdout);
        Ok(text
            .split('\u{001e}')
            .filter_map(|record| {
                let mut fields = record.trim().split('\u{001f}');
                let sha = fields.next()?.trim();
                let abbreviated_sha = fields.next()?.trim();
                let seconds = fields.next()?.trim().parse::<i64>().ok()?;
                let author_name = fields.next()?.trim();
                let subject = fields.next()?.trim();
                (!sha.is_empty()).then(|| GitCommitSummary {
                    sha: sha.to_owned(),
                    abbreviated_sha: abbreviated_sha.to_owned(),
                    subject: subject.to_owned(),
                    author_name: author_name.to_owned(),
                    committed_at_ms: seconds.saturating_mul(1_000),
                })
            })
            .collect())
    }

    fn validated_git_paths(&self, paths: &[String]) -> Result<Vec<String>, WorkspaceError> {
        if paths.is_empty() || paths.len() > MAX_PATHS {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::InvalidRequest,
                "Git mutation requires 1-500 paths",
            ));
        }
        paths
            .iter()
            .map(|path| {
                if path.is_empty() || path.len() > 4_096 || path.contains('\0') {
                    return Err(WorkspaceError::new(
                        WorkspaceErrorCode::InvalidRequest,
                        "Git path is empty or exceeds the protocol limit",
                    ));
                }
                self.resolve_write(path)
                    .map(|resolved| relative_display(&self.root, &resolved))
            })
            .collect()
    }

    async fn git_checked<I, S>(
        &self,
        arguments: I,
        optional_locks: bool,
    ) -> Result<std::process::Output, WorkspaceError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.git_raw(arguments, optional_locks).await?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(git_failure("run local operation", &output))
        }
    }

    async fn git_raw<I, S>(
        &self,
        arguments: I,
        optional_locks: bool,
    ) -> Result<std::process::Output, WorkspaceError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let mut command = Command::new(crate::git_program());
        command
            .arg("-c")
            .arg(format!("core.hooksPath={DISABLED_HOOKS_PATH}"))
            .args(arguments)
            .current_dir(crate::restricted_process_cwd(&self.root))
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if optional_locks {
            command.env("GIT_OPTIONAL_LOCKS", "0");
        }
        crate::copy_process_environment(&mut command);
        let output = tokio::time::timeout(GIT_TIMEOUT, command.output())
            .await
            .map_err(|_| {
                WorkspaceError::new(WorkspaceErrorCode::TimedOut, "Git operation timed out")
            })?
            .map_err(|error| {
                WorkspaceError::new(WorkspaceErrorCode::ProcessFailed, error.to_string())
            })?;
        if output.stdout.len() > MAX_GIT_OUTPUT || output.stderr.len() > MAX_GIT_OUTPUT {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::TooLarge,
                "Git output exceeded the 2 MiB safety limit",
            ));
        }
        Ok(output)
    }

    async fn git_raw_with_input<I, S>(
        &self,
        arguments: I,
        optional_locks: bool,
        input: &[u8],
        environment: &[(&str, &str)],
    ) -> Result<std::process::Output, WorkspaceError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = Command::new(crate::git_program());
        command
            .arg("-c")
            .arg(format!("core.hooksPath={DISABLED_HOOKS_PATH}"))
            .args(arguments)
            .current_dir(crate::restricted_process_cwd(&self.root))
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if optional_locks {
            command.env("GIT_OPTIONAL_LOCKS", "0");
        }
        for (name, value) in environment {
            command.env(name, value);
        }
        crate::copy_process_environment(&mut command);
        let mut child = command.spawn().map_err(|error| {
            WorkspaceError::new(WorkspaceErrorCode::ProcessFailed, error.to_string())
        })?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(input).await.map_err(|error| {
                WorkspaceError::new(WorkspaceErrorCode::ProcessFailed, error.to_string())
            })?;
            stdin.shutdown().await.map_err(|error| {
                WorkspaceError::new(WorkspaceErrorCode::ProcessFailed, error.to_string())
            })?;
        }
        let output = tokio::time::timeout(GIT_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| {
                WorkspaceError::new(WorkspaceErrorCode::TimedOut, "Git operation timed out")
            })?
            .map_err(|error| {
                WorkspaceError::new(WorkspaceErrorCode::ProcessFailed, error.to_string())
            })?;
        if output.stdout.len() > MAX_GIT_OUTPUT || output.stderr.len() > MAX_GIT_OUTPUT {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::TooLarge,
                "Git output exceeded the 2 MiB safety limit",
            ));
        }
        Ok(output)
    }
}

fn validate_identity<'a>(
    name: &'a str,
    email: &'a str,
) -> Result<(&'a str, &'a str), WorkspaceError> {
    let name = name.trim();
    let email = email.trim();
    let invalid = |value: &str| {
        value.is_empty()
            || value.len() > 320
            || value
                .chars()
                .any(|character| matches!(character, '\0' | '\r' | '\n'))
    };
    if invalid(name) || invalid(email) || !email.contains('@') {
        return Err(WorkspaceError::new(
            WorkspaceErrorCode::InvalidRequest,
            "Git author name/email are invalid",
        ));
    }
    Ok((name, email))
}

fn validate_remote_name(value: &str) -> Result<(), WorkspaceError> {
    let valid = !value.is_empty()
        && value.len() <= 256
        && !value.starts_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if !valid {
        return Err(WorkspaceError::new(
            WorkspaceErrorCode::InvalidRequest,
            "Git remote name is invalid",
        ));
    }
    Ok(())
}

fn validate_git_ref(value: &str, require_branch: bool) -> Result<(), WorkspaceError> {
    let valid = !value.is_empty()
        && value.len() <= 1_024
        && !value.starts_with('-')
        && !value.ends_with('/')
        && !value.ends_with('.')
        && !value.contains("..")
        && !value.contains("@{")
        && !value.contains("//")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-'))
        && (!require_branch || value.starts_with("refs/heads/"));
    if !valid {
        return Err(WorkspaceError::new(
            WorkspaceErrorCode::InvalidRequest,
            "Git ref is invalid or target is not a branch ref",
        ));
    }
    Ok(())
}

fn validate_oid(value: &str) -> Result<(), WorkspaceError> {
    if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(WorkspaceError::new(
            WorkspaceErrorCode::InvalidRequest,
            "Git commit OID must contain 40 hexadecimal characters",
        ));
    }
    Ok(())
}

fn hash_remote_url(value: &str) -> String {
    crate::worker_io::sha256(value.trim().as_bytes())
}

fn infer_forge_kind(value: &str) -> ForgeKind {
    let normalized = value.to_ascii_lowercase();
    if normalized.contains("github.com") {
        ForgeKind::GitHub
    } else if normalized.contains("gitlab") {
        ForgeKind::GitLab
    } else if normalized.contains("gitee.com") {
        ForgeKind::Gitee
    } else if normalized.contains("gitea") || normalized.contains("forgejo") {
        ForgeKind::GiteaForgejo
    } else {
        ForgeKind::Unknown
    }
}

fn redact_remote_url(value: &str) -> String {
    let trimmed = value.trim();
    let Some(scheme) = trimmed.find("://") else {
        return trimmed.to_owned();
    };
    let authority_start = scheme + 3;
    let authority_end = trimmed[authority_start..]
        .find('/')
        .map_or(trimmed.len(), |index| authority_start + index);
    let authority = &trimmed[authority_start..authority_end];
    let Some(at) = authority.rfind('@') else {
        return trimmed.to_owned();
    };
    format!(
        "{}{}",
        &trimmed[..authority_start],
        &trimmed[authority_start + at + 1..]
    )
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}

fn validate_commit_message(message: &str) -> Result<&str, WorkspaceError> {
    let message = message.trim();
    if message.is_empty() || message.len() > MAX_COMMIT_MESSAGE || message.contains('\0') {
        return Err(WorkspaceError::new(
            WorkspaceErrorCode::InvalidRequest,
            "commit message must contain 1-4096 UTF-8 bytes",
        ));
    }
    Ok(message)
}

fn git_failure(operation: &str, output: &std::process::Output) -> WorkspaceError {
    WorkspaceError::new(
        WorkspaceErrorCode::ProcessFailed,
        format!(
            "Git could not {operation}: {}",
            String::from_utf8_lossy(&output.stderr)
                .trim()
                .chars()
                .take(1_000)
                .collect::<String>()
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_stage_unstage_commit_and_history_are_structured() {
        let fixture = tempfile::tempdir().expect("fixture");
        run_git(fixture.path(), &["init"]);
        run_git(fixture.path(), &["config", "user.name", "Hachimi Test"]);
        run_git(
            fixture.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        std::fs::write(fixture.path().join("tracked.txt"), "one\n").expect("write fixture");
        run_git(fixture.path(), &["add", "tracked.txt"]);
        run_git(fixture.path(), &["commit", "-m", "initial"]);
        std::fs::write(fixture.path().join("tracked.txt"), "two\n").expect("update fixture");
        let worker = WorkerContext::new(fixture.path(), "checkout", 1, "token").expect("worker");

        let WorkspaceOutput::GitWorkspaceSnapshot { snapshot } =
            worker.git_workspace_snapshot(10).await.expect("snapshot")
        else {
            panic!("unexpected output")
        };
        assert_eq!(snapshot.status[0].worktree_status, "M");
        assert_eq!(snapshot.recent_commits[0].subject, "initial");

        let WorkspaceOutput::GitMutation { response } = worker
            .git_stage(&["tracked.txt".into()], 10)
            .await
            .expect("stage")
        else {
            panic!("unexpected stage output")
        };
        assert_eq!(response.snapshot.status[0].index_status, "M");
        let WorkspaceOutput::GitMutation { response } = worker
            .git_unstage(&["tracked.txt".into()], 10)
            .await
            .expect("unstage")
        else {
            panic!("unexpected unstage output")
        };
        assert_eq!(response.snapshot.status[0].worktree_status, "M");
        worker
            .git_stage(&["tracked.txt".into()], 10)
            .await
            .expect("restage");
        let WorkspaceOutput::GitMutation { response } = worker
            .git_commit("update tracked", 10)
            .await
            .expect("commit")
        else {
            panic!("unexpected commit output")
        };
        assert!(response.commit_sha.is_some());
        assert_eq!(
            response.snapshot.recent_commits[0].subject,
            "update tracked"
        );
        assert!(response.snapshot.status.is_empty());
    }

    #[tokio::test]
    async fn git_mutations_reject_checkout_escape_and_empty_commits() {
        let fixture = tempfile::tempdir().expect("fixture");
        run_git(fixture.path(), &["init"]);
        let worker = WorkerContext::new(fixture.path(), "checkout", 1, "token").expect("worker");
        assert!(worker.git_stage(&["../escape".into()], 5).await.is_err());
        assert_eq!(
            worker
                .git_commit("nothing staged", 5)
                .await
                .unwrap_err()
                .code,
            WorkspaceErrorCode::Conflict
        );
    }

    #[tokio::test]
    async fn project_inspection_reconciles_unborn_and_empty_initial_commit_preserves_index() {
        let fixture = tempfile::tempdir().expect("fixture");
        let worker = WorkerContext::new(fixture.path(), "checkout", 0, "token").expect("worker");
        let WorkspaceOutput::ProjectGitSnapshot { snapshot } = worker
            .git_project_inspect(ProjectId::from("project"))
            .await
            .expect("not repository")
        else {
            panic!("unexpected output")
        };
        assert_eq!(snapshot.state, ProjectGitState::NotRepository);

        run_git(fixture.path(), &["init", "-b", "main"]);
        std::fs::write(fixture.path().join("staged.txt"), "staged\n").expect("staged file");
        std::fs::write(fixture.path().join("untracked.txt"), "untracked\n")
            .expect("untracked file");
        run_git(fixture.path(), &["add", "staged.txt"]);
        let index_path = fixture.path().join(".git/index");
        let index_before = std::fs::read(&index_path).expect("index before");

        let WorkspaceOutput::ProjectGitSnapshot { snapshot } = worker
            .git_project_inspect(ProjectId::from("project"))
            .await
            .expect("unborn")
        else {
            panic!("unexpected output")
        };
        assert_eq!(
            snapshot.state,
            ProjectGitState::Unborn {
                branch: "main".into()
            }
        );

        let WorkspaceOutput::GitMutation { response } = worker
            .git_create_empty_initial_commit("Hachimi Test", "test@example.invalid", 5)
            .await
            .expect("empty initial commit")
        else {
            panic!("unexpected output")
        };
        assert_eq!(
            std::fs::read(&index_path).expect("index after"),
            index_before
        );
        assert!(fixture.path().join("untracked.txt").is_file());
        assert!(
            response
                .snapshot
                .status
                .iter()
                .any(|entry| entry.path == "staged.txt" && entry.index_status == "A")
        );
        let tree = std::process::Command::new(crate::git_program())
            .args(["show", "--pretty=", "--name-only", "HEAD"])
            .current_dir(fixture.path())
            .output()
            .expect("show commit");
        assert!(tree.status.success());
        assert!(
            tree.stdout.is_empty(),
            "initial commit must contain an empty tree"
        );
    }

    #[tokio::test]
    async fn standard_remote_push_is_oid_and_url_hash_fenced() {
        let fixture = tempfile::tempdir().expect("fixture");
        let remote = tempfile::tempdir().expect("remote");
        run_git(remote.path(), &["init", "--bare"]);
        run_git(fixture.path(), &["init", "-b", "main"]);
        run_git(fixture.path(), &["config", "user.name", "Hachimi Test"]);
        run_git(
            fixture.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        std::fs::write(fixture.path().join("README.md"), "push test\n").expect("fixture");
        run_git(fixture.path(), &["add", "README.md"]);
        run_git(fixture.path(), &["commit", "-m", "initial"]);
        let remote_path = remote.path().to_string_lossy().into_owned();
        run_git(fixture.path(), &["remote", "add", "origin", &remote_path]);
        let worker = WorkerContext::new(fixture.path(), "checkout", 1, "token").expect("worker");
        let WorkspaceOutput::GitRemotes { remotes } = worker.git_remotes().await.expect("remotes")
        else {
            panic!("unexpected remote output")
        };
        assert_eq!(remotes.len(), 1);
        assert_eq!(remotes[0].forge_kind, ForgeKind::Unknown);
        let head = std::process::Command::new(crate::git_program())
            .args(["rev-parse", "HEAD"])
            .current_dir(fixture.path())
            .output()
            .expect("head");
        let head = String::from_utf8_lossy(&head.stdout).trim().to_owned();
        let WorkspaceOutput::GitPush { response } = worker
            .git_push(
                "origin",
                &remotes[0].remote_url_hash,
                "HEAD",
                "refs/heads/main",
                &head,
            )
            .await
            .expect("push")
        else {
            panic!("unexpected push output")
        };
        assert!(response.confirmed);
        assert_eq!(response.commit_oid, head);
        let stale = worker
            .git_push("origin", &"0".repeat(64), "HEAD", "refs/heads/main", &head)
            .await
            .expect_err("stale remote hash");
        assert_eq!(stale.code, WorkspaceErrorCode::Conflict);
        assert_eq!(
            redact_remote_url("https://user:secret@example.test/owner/repo.git"),
            "https://example.test/owner/repo.git"
        );
    }

    fn run_git(cwd: &std::path::Path, arguments: &[&str]) {
        let output = std::process::Command::new(crate::git_program())
            .args(arguments)
            .current_dir(crate::restricted_process_cwd(cwd))
            .output()
            .expect("git command");
        assert!(
            output.status.success(),
            "git {arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
