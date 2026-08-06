use std::path::Path;

use hachimi_protocol::{WorkbenchGitPhaseResult, WorkbenchGitPhaseStatus};

use super::{WorkbenchError, git_optional, git_required};

pub(super) async fn switch_branch(
    root: &Path,
    branch: &str,
    remote: bool,
) -> Result<WorkbenchGitPhaseResult, WorkbenchError> {
    let branch = validate_branch_name(root, branch).await?;
    let result = if remote {
        let (_, local) = branch
            .split_once('/')
            .filter(|(remote, local)| !remote.is_empty() && !local.is_empty())
            .ok_or_else(|| WorkbenchError::Git("remote branch must include its remote".into()))?;
        let local = validate_branch_name(root, local).await?;
        ensure_ref(root, &format!("refs/remotes/{branch}"), true).await?;
        ensure_ref(root, &format!("refs/heads/{local}"), false).await?;
        git_required(root, &["switch", "-c", local, "--track", branch], None).await
    } else {
        ensure_ref(root, &format!("refs/heads/{branch}"), true).await?;
        git_required(root, &["switch", branch], None).await
    };
    Ok(phase(result))
}

pub(super) async fn create_branch(
    root: &Path,
    branch: &str,
) -> Result<WorkbenchGitPhaseResult, WorkbenchError> {
    let branch = validate_branch_name(root, branch).await?;
    ensure_ref(root, &format!("refs/heads/{branch}"), false).await?;
    Ok(phase(
        git_required(root, &["switch", "-c", branch], None).await,
    ))
}

async fn validate_branch_name<'a>(root: &Path, branch: &'a str) -> Result<&'a str, WorkbenchError> {
    let branch = branch.trim();
    if branch.is_empty() || branch.starts_with('-') || branch.chars().count() > 255 {
        return Err(WorkbenchError::Git("invalid branch name".into()));
    }
    git_required(root, &["check-ref-format", "--branch", branch], None).await?;
    Ok(branch)
}

async fn ensure_ref(
    root: &Path,
    reference: &str,
    should_exist: bool,
) -> Result<(), WorkbenchError> {
    let exists = git_optional(root, &["show-ref", "--verify", "--hash", reference])
        .await?
        .is_some();
    if exists != should_exist {
        let state = if should_exist {
            "does not exist"
        } else {
            "already exists"
        };
        return Err(WorkbenchError::Git(format!("branch {reference} {state}")));
    }
    Ok(())
}

fn phase(result: Result<String, WorkbenchError>) -> WorkbenchGitPhaseResult {
    match result {
        Ok(message) => WorkbenchGitPhaseResult {
            status: WorkbenchGitPhaseStatus::Succeeded,
            message: (!message.is_empty()).then_some(message),
        },
        Err(error) => WorkbenchGitPhaseResult {
            status: WorkbenchGitPhaseStatus::Failed,
            message: Some(error.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    fn git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .expect("git command");
        assert!(status.success(), "git {args:?}");
    }

    #[tokio::test]
    async fn validates_names_and_creates_explicit_nested_tracking_branch() {
        let local = tempfile::tempdir().expect("local");
        let remote = tempfile::tempdir().expect("remote");
        git(remote.path(), &["init", "--bare"]);
        git(local.path(), &["init", "-b", "main"]);
        git(local.path(), &["config", "user.email", "test@example.com"]);
        git(local.path(), &["config", "user.name", "Hachimi Test"]);
        std::fs::write(local.path().join("README.md"), "initial\n").expect("fixture");
        git(local.path(), &["add", "README.md"]);
        git(local.path(), &["commit", "-m", "initial"]);
        git(local.path(), &["branch", "feature/nested"]);
        let remote_path = remote.path().to_string_lossy();
        git(local.path(), &["remote", "add", "origin", &remote_path]);
        git(local.path(), &["push", "origin", "feature/nested"]);
        git(local.path(), &["branch", "-D", "feature/nested"]);
        git(local.path(), &["fetch", "origin"]);

        assert!(
            switch_branch(local.path(), "--upload-pack=x", false)
                .await
                .is_err()
        );
        let result = switch_branch(local.path(), "origin/feature/nested", true)
            .await
            .expect("tracking branch");
        assert_eq!(result.status, WorkbenchGitPhaseStatus::Succeeded);
        assert!(create_branch(local.path(), "feature/nested").await.is_err());
    }
}
