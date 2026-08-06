// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/windows-sandbox-rs/src/bin/command_runner/win/cwd_junction.rs.
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// Modified for Hachimi: short-lived DOS drive aliases for an AppContainer-bound
// Checkout and linked-worktree common directory, with deterministic cleanup.

use std::path::{Path, PathBuf};

use crate::{WorkspaceError, WorkspaceErrorCode};

pub(crate) const GIT_DIR_ALIAS_ENV: &str = "HACHIMI_GIT_DIR_ALIAS";
pub(crate) const GIT_WORK_TREE_ALIAS_ENV: &str = "HACHIMI_GIT_WORK_TREE_ALIAS";
pub(crate) const GIT_WORK_TREE_REAL_ENV: &str = "HACHIMI_GIT_WORK_TREE_REAL";

#[derive(Debug)]
pub(crate) struct RestrictedGitAliases {
    git_dir: Option<PathBuf>,
    work_tree: PathBuf,
    real_work_tree: PathBuf,
    _common_drive: Option<SubstDrive>,
    _checkout_drive: SubstDrive,
}

impl RestrictedGitAliases {
    #[cfg(windows)]
    pub(crate) fn for_checkout(checkout: &Path) -> Result<Option<Self>, WorkspaceError> {
        let checkout = hachimi_sandbox::validate_checkout_root(checkout).map_err(|error| {
            WorkspaceError::new(WorkspaceErrorCode::PathOutsideCheckout, error.to_string())
        })?;
        let checkout_drive = SubstDrive::new(&checkout)?;
        let work_tree = checkout_drive.root();
        if !checkout.join(".git").exists() {
            return Ok(Some(Self {
                git_dir: None,
                work_tree,
                real_work_tree: checkout,
                _common_drive: None,
                _checkout_drive: checkout_drive,
            }));
        }
        let git = hachimi_sandbox::trusted_git_executable()
            .map_err(|error| WorkspaceError::new(WorkspaceErrorCode::HostDisconnected, error))?;
        let git_dir = git_stdout(&git, &checkout, &["rev-parse", "--absolute-git-dir"])?;
        let git_dir = PathBuf::from(git_dir).canonicalize().map_err(io_error)?;
        let common_dir = git_stdout(&git, &checkout, &["rev-parse", "--git-common-dir"])?;
        let common_dir = PathBuf::from(common_dir);
        let common_dir = if common_dir.is_absolute() {
            common_dir
        } else {
            checkout.join(common_dir)
        };
        let common_dir = hachimi_sandbox::validate_checkout_root(&common_dir).map_err(|error| {
            WorkspaceError::new(WorkspaceErrorCode::PathOutsideCheckout, error.to_string())
        })?;
        let git_dir_relative = git_dir.strip_prefix(&common_dir).map_err(|_| {
            WorkspaceError::new(
                WorkspaceErrorCode::PathOutsideCheckout,
                "linked-worktree Git directory is outside its common directory",
            )
        })?;
        let common_drive = SubstDrive::new(&common_dir)?;
        let git_dir = common_drive.root().join(git_dir_relative);
        Ok(Some(Self {
            git_dir: Some(git_dir),
            work_tree,
            real_work_tree: checkout,
            _common_drive: Some(common_drive),
            _checkout_drive: checkout_drive,
        }))
    }

    #[cfg(not(windows))]
    pub(crate) fn for_checkout(_checkout: &Path) -> Result<Option<Self>, WorkspaceError> {
        Ok(None)
    }

    pub(crate) fn append_environment(
        &self,
        environment: &mut Vec<(std::ffi::OsString, std::ffi::OsString)>,
    ) {
        if let Some(git_dir) = &self.git_dir {
            environment.push((GIT_DIR_ALIAS_ENV.into(), git_dir.as_os_str().to_owned()));
        }
        environment.push((
            GIT_WORK_TREE_ALIAS_ENV.into(),
            self.work_tree.as_os_str().to_owned(),
        ));
        environment.push((
            GIT_WORK_TREE_REAL_ENV.into(),
            self.real_work_tree.as_os_str().to_owned(),
        ));
    }

    pub(crate) fn workspace_root(&self) -> &Path {
        &self.work_tree
    }

    pub(crate) fn real_workspace_root(&self) -> &Path {
        &self.real_work_tree
    }
}

#[derive(Debug)]
struct SubstDrive {
    drive: String,
}

impl SubstDrive {
    #[cfg(windows)]
    fn new(target: &Path) -> Result<Self, WorkspaceError> {
        static DRIVE_ASSIGNMENT: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = DRIVE_ASSIGNMENT.lock().map_err(|_| {
            WorkspaceError::new(
                WorkspaceErrorCode::HostDisconnected,
                "restricted Git alias registry is poisoned",
            )
        })?;
        for letter in (b'P'..=b'Z').rev() {
            let drive = format!("{}:", char::from(letter));
            if Path::new(&format!("{drive}\\")).exists() {
                continue;
            }
            let output = hachimi_process_policy::std_command(
                "subst.exe",
                hachimi_process_policy::ProcessPolicy::HiddenCaptured,
            )
            .arg(&drive)
            .arg(target)
            .output()
            .map_err(io_error)?;
            if output.status.success() {
                return Ok(Self { drive });
            }
        }
        Err(WorkspaceError::new(
            WorkspaceErrorCode::HostDisconnected,
            "no temporary drive alias is available for restricted Git",
        ))
    }

    #[cfg(not(windows))]
    fn new(_target: &Path) -> Result<Self, WorkspaceError> {
        Err(WorkspaceError::new(
            WorkspaceErrorCode::HostDisconnected,
            "restricted Git aliases are Windows-only",
        ))
    }

    fn root(&self) -> PathBuf {
        PathBuf::from(format!("{}\\", self.drive))
    }
}

impl Drop for SubstDrive {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            let _ = hachimi_process_policy::std_command(
                "subst.exe",
                hachimi_process_policy::ProcessPolicy::HiddenCaptured,
            )
            .args([self.drive.as_str(), "/D"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        }
    }
}

#[cfg(windows)]
fn git_stdout(git: &Path, checkout: &Path, arguments: &[&str]) -> Result<String, WorkspaceError> {
    let output = hachimi_process_policy::std_command(
        git,
        hachimi_process_policy::ProcessPolicy::HiddenCaptured,
    )
    .arg("-C")
    .arg(checkout)
    .args(arguments)
    .env("GIT_OPTIONAL_LOCKS", "0")
    .stdin(std::process::Stdio::null())
    .output()
    .map_err(io_error)?;
    if !output.status.success() {
        return Err(WorkspaceError::new(
            WorkspaceErrorCode::ProcessFailed,
            format!(
                "Git metadata discovery failed: {}",
                String::from_utf8_lossy(&output.stderr)
                    .chars()
                    .take(512)
                    .collect::<String>()
            ),
        ));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|_| {
            WorkspaceError::new(
                WorkspaceErrorCode::ProcessFailed,
                "Git metadata path is not UTF-8",
            )
        })
}

fn io_error(error: std::io::Error) -> WorkspaceError {
    WorkspaceError::new(WorkspaceErrorCode::HostDisconnected, error.to_string())
}
