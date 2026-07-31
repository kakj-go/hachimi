// SPDX-License-Identifier: Apache-2.0

use std::sync::OnceLock;
use std::{io::Write, path::Path};

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};

use crate::appcontainer::{
    APP_CONTAINER_NAME, AppContainerSid, deny_appcontainer_read, deny_appcontainer_write,
    grant_appcontainer_access, revoke_appcontainer_access,
};
use crate::runtime_attestation::SANDBOX_POLICY_VERSION;

static MANAGED_GIT_EXECUTABLE: OnceLock<std::path::PathBuf> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxSetupMarker {
    pub version: String,
    pub acl_component: bool,
    pub token_component: bool,
    pub network_component: bool,
    #[serde(default)]
    pub app_container_name: Option<String>,
    #[serde(default)]
    pub app_container_sid: Option<String>,
    #[serde(default)]
    pub acl_paths: Vec<String>,
    #[serde(default)]
    pub git_executable: Option<String>,
    pub installed_at_ms: i64,
}

pub fn install_sandbox_marker(
    marker_path: &Path,
    launcher: &Path,
) -> Result<SandboxSetupMarker, String> {
    if !cfg!(windows) {
        return Err("Windows sandbox setup is only supported on Windows".into());
    }
    if !launcher.is_file() {
        return Err("sandbox launcher binary is missing".into());
    }
    let parent = marker_path
        .parent()
        .ok_or_else(|| "sandbox marker path has no parent".to_owned())?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    if !restricted_sid_available() {
        return Err("the Windows Restricted Code SID could not be resolved".into());
    }
    let (app_container, profile_created) = AppContainerSid::ensure_profile_with_state()?;
    let app_container_sid = app_container.to_string_sid()?;
    let launcher_parent = launcher
        .parent()
        .ok_or_else(|| "sandbox launcher has no parent directory".to_owned())?;
    if let Err(error) = grant_restricted_code_access(launcher_parent, false) {
        if profile_created {
            let _ = revoke_restricted_code_access(launcher_parent);
            let _ = AppContainerSid::delete_profile();
        }
        return Err(format!("sandbox launcher ACL setup failed: {error}"));
    }
    let git_runtime = match trusted_git_runtime() {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = revoke_restricted_code_access(launcher_parent);
            if profile_created {
                let _ = AppContainerSid::delete_profile();
            }
            return Err(format!("trusted Git runtime discovery failed: {error}"));
        }
    };
    let acl_paths = vec![launcher_parent.to_string_lossy().into_owned()];
    let marker = SandboxSetupMarker {
        version: SANDBOX_POLICY_VERSION.into(),
        acl_component: true,
        token_component: true,
        // An AppContainer launched without network capabilities is the persistent C2.1 deny-all
        // identity. Runtime attestation must still prove the boundary before it is trusted.
        network_component: true,
        app_container_name: Some(APP_CONTAINER_NAME.to_owned()),
        app_container_sid: Some(app_container_sid),
        acl_paths,
        git_executable: git_runtime
            .as_ref()
            .map(|runtime| runtime.executable.to_string_lossy().into_owned()),
        installed_at_ms: now_ms(),
    };
    let encoded = serde_json::to_vec_pretty(&marker).map_err(|error| error.to_string())?;
    let marker_commit = (|| {
        let mut file = AtomicWriteFile::open(marker_path)?;
        file.write_all(&encoded)?;
        file.flush()?;
        file.commit()
    })();
    if let Err(error) = marker_commit {
        if profile_created {
            let _ = revoke_restricted_code_access(launcher_parent);
            let _ = AppContainerSid::delete_profile();
        }
        return Err(format!("sandbox marker commit failed: {error}"));
    }
    Ok(marker)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrustedGitRuntime {
    executable: std::path::PathBuf,
    root: std::path::PathBuf,
}

/// Resolves only the pinned Git executable staged in Hachimi's per-user
/// managed Runtime. System PATH is deliberately ignored.
fn trusted_git_runtime() -> Result<Option<TrustedGitRuntime>, String> {
    let candidate = MANAGED_GIT_EXECUTABLE
        .get()
        .cloned()
        .or_else(|| std::env::var_os("HACHIMI_MANAGED_GIT_EXECUTABLE").map(Into::into));
    let Some(candidate) = candidate else {
        return Ok(None);
    };
    if !candidate.is_absolute() || !candidate.is_file() {
        return Err("managed Git executable is missing".into());
    }
    let executable = candidate
        .canonicalize()
        .map_err(|error| format!("could not canonicalize {}: {error}", candidate.display()))?;
    let cmd = executable
        .parent()
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("cmd"))
        })
        .ok_or_else(|| "managed Git must use the cmd/git.exe layout".to_owned())?;
    let root = cmd
        .parent()
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("managed-git"))
        })
        .ok_or_else(|| "Git executable is outside Hachimi managed-git".to_owned())?
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !root.join("manifest.json").is_file()
        || !root
            .parent()
            .is_some_and(|runtime| runtime.join("runtime-manifest.json").is_file())
    {
        return Err("managed Git attestation manifests are missing".into());
    }
    Ok(Some(TrustedGitRuntime { executable, root }))
}

pub fn set_managed_git_executable(path: std::path::PathBuf) -> Result<(), String> {
    let runtime = validate_managed_git(&path)?;
    if let Some(existing) = MANAGED_GIT_EXECUTABLE.get() {
        return if existing == &runtime.executable {
            Ok(())
        } else {
            Err("managed Git executable was already initialized to another Runtime".into())
        };
    }
    MANAGED_GIT_EXECUTABLE
        .set(runtime.executable)
        .map_err(|_| "managed Git executable was already initialized".into())
}

fn validate_managed_git(path: &Path) -> Result<TrustedGitRuntime, String> {
    let previous = MANAGED_GIT_EXECUTABLE.get().cloned();
    if previous.as_deref() == Some(path) {
        return trusted_git_runtime()?.ok_or_else(|| "managed Git is unavailable".into());
    }
    let executable = path
        .canonicalize()
        .map_err(|error| format!("managed Git canonicalization failed: {error}"))?;
    let cmd = executable
        .parent()
        .ok_or_else(|| "managed Git has no cmd directory".to_owned())?;
    let root = cmd
        .parent()
        .ok_or_else(|| "managed Git has no Runtime root".to_owned())?;
    if !cmd
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("cmd"))
        || !root
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("managed-git"))
        || !root.join("manifest.json").is_file()
        || !root
            .parent()
            .is_some_and(|parent| parent.join("runtime-manifest.json").is_file())
    {
        return Err("managed Git path is outside the attested per-user Runtime".into());
    }
    let root = root.to_owned();
    Ok(TrustedGitRuntime { executable, root })
}

/// Returns the exact Git executable bound during Sandbox setup discovery.
/// Restricted Workspace Workers receive this path explicitly and never resolve
/// Git from their checkout current directory.
pub fn trusted_git_executable() -> Result<std::path::PathBuf, String> {
    trusted_git_runtime()?
        .map(|runtime| runtime.executable)
        .ok_or_else(|| "pinned Hachimi managed Git runtime is unavailable".to_owned())
}

pub fn uninstall_sandbox(marker_path: &Path) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("Windows sandbox uninstall is only supported on Windows".into());
    }
    let marker = std::fs::read(marker_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<SandboxSetupMarker>(&bytes).ok());
    let mut errors = Vec::new();
    if let Some(marker) = marker {
        for path in marker.acl_paths {
            if let Err(error) = revoke_restricted_code_access(Path::new(&path)) {
                errors.push(error);
            }
        }
    }
    if let Err(error) = AppContainerSid::delete_profile() {
        errors.push(error);
    }
    for path in [marker_path.to_path_buf()] {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => errors.push(error.to_string()),
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

pub fn grant_restricted_code_access(path: &Path, write: bool) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("restricted-code ACLs are only available on Windows".into());
    }
    let permission = if write { "(OI)(CI)M" } else { "(OI)(CI)RX" };
    let grant = format!("*S-1-5-12:{permission}");
    let status = run_icacls(path, &["/grant:r", &grant, "/Q"])?;
    if !status.success() {
        return Err(format!("icacls exited with {status}"));
    }
    grant_appcontainer_access(path, write)
}

pub fn deny_restricted_code_read(path: &Path) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("restricted-code ACLs are only available on Windows".into());
    }
    let status = run_icacls(path, &["/deny", "*S-1-5-12:(OI)(CI)(RX)", "/Q"])?;
    if !status.success() {
        return Err(format!("icacls exited with {status}"));
    }
    deny_appcontainer_read(path)
}

pub fn deny_restricted_code_write(path: &Path) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("restricted-code ACLs are only available on Windows".into());
    }
    let status = run_icacls(path, &["/deny", "*S-1-5-12:(OI)(CI)(W,D,DC)", "/Q"])?;
    if !status.success() {
        return Err(format!("icacls exited with {status}"));
    }
    deny_appcontainer_write(path)
}

pub fn revoke_restricted_code_access(path: &Path) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("restricted-code ACLs are only available on Windows".into());
    }
    let mut errors = Vec::new();
    for mode in ["/remove:g", "/remove:d"] {
        let status = run_icacls(path, &[mode, "*S-1-5-12", "/Q"])?;
        if !status.success() {
            errors.push(format!("icacls {mode} exited with {status}"));
        }
    }
    if let Err(error) = revoke_appcontainer_access(path) {
        errors.push(error);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn run_icacls(path: &Path, arguments: &[&str]) -> Result<std::process::ExitStatus, String> {
    let mut child = std::process::Command::new("icacls.exe")
        .arg(path)
        .args(arguments)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            return Ok(status);
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "icacls timed out after 30 seconds for {}",
                path.display()
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

pub fn prepare_workspace_acl(
    checkout: &Path,
    run_temp: &Path,
    worker_program: &Path,
) -> Result<Vec<std::path::PathBuf>, String> {
    std::fs::create_dir_all(run_temp).map_err(|error| {
        format!(
            "restricted Workspace TEMP could not be created at {}: {error}",
            run_temp.display()
        )
    })?;
    grant_restricted_code_access(checkout, true)?;
    grant_restricted_code_access(run_temp, true)?;
    let worker_parent = worker_program
        .parent()
        .ok_or_else(|| "workspace worker has no parent directory".to_owned())?;
    grant_restricted_code_access(worker_parent, false)?;

    let dot_git = checkout.join(".git");
    let mut read_roots = vec![worker_parent.to_path_buf()];
    if let Some(common_dir) = git_common_dir(checkout)? {
        protect_restricted_code_read_only(&common_dir)?;
        read_roots.push(common_dir);
    }
    if dot_git.exists() {
        protect_restricted_code_read_only(&dot_git)?;
    }
    Ok(read_roots)
}

/// Stops a protected Git path from inheriting the Checkout's writable ACE,
/// then installs the Hachimi restricted identities as read/execute only.
///
/// AppContainer capability SIDs participate in the restricted access pass;
/// an inherited writable allow can therefore defeat a deny-only overlay. The
/// Codex ACL implementation avoids that state by removing `FILE_DELETE_CHILD`
/// from writable-root ACEs and replacing stale inherited rights. Hachimi's
/// icacls adapter makes the protected child explicit instead, preserving all
/// unrelated user/admin ACEs while replacing only its two sandbox identities.
fn protect_restricted_code_read_only(path: &Path) -> Result<(), String> {
    let status = run_icacls(path, &["/inheritance:d", "/Q"])?;
    if !status.success() {
        return Err(format!(
            "icacls inheritance protection exited with {status}"
        ));
    }
    revoke_restricted_code_access(path)?;
    grant_restricted_code_access(path, false)
}

/// Temporary ACL upgrade used only around one fixed local Git mutation. The
/// normal Workspace ACL keeps `.git` and the shared common directory read-only.
#[derive(Debug, Clone)]
pub struct GitMutationAcl {
    dot_git: std::path::PathBuf,
    metadata_dirs: Vec<std::path::PathBuf>,
}

pub fn prepare_git_mutation_acl(checkout: &Path) -> Result<GitMutationAcl, String> {
    let dot_git = checkout.join(".git");
    let common_dir = git_common_dir(checkout)?
        .ok_or_else(|| "Git mutation requires a repository common directory".to_owned())?;
    let git_dir = git_absolute_dir(checkout)?
        .ok_or_else(|| "Git mutation requires an absolute repository directory".to_owned())?;
    if git_dir != common_dir && !git_dir.starts_with(&common_dir) {
        return Err("linked-worktree Git directory is outside its common directory".to_owned());
    }
    let mut metadata_dirs = vec![common_dir];
    if !metadata_dirs.iter().any(|path| path == &git_dir) {
        metadata_dirs.push(git_dir);
    }
    if dot_git.is_dir() {
        // Remove any deny ACE left by an older policy before granting the exact
        // repository metadata directories used by this mutation.
        revoke_restricted_code_access(&dot_git)?;
    }
    let mut upgraded: Vec<std::path::PathBuf> = Vec::new();
    for path in &metadata_dirs {
        if let Err(error) = revoke_restricted_code_access(path)
            .and_then(|()| grant_restricted_code_access(path, true))
        {
            for upgraded_path in upgraded.iter().rev() {
                let _ = protect_restricted_code_read_only(upgraded_path);
            }
            return Err(error);
        }
        upgraded.push(path.clone());
    }
    if dot_git.is_file()
        && let Err(error) = deny_restricted_code_write(&dot_git)
    {
        for upgraded_path in upgraded.iter().rev() {
            let _ = protect_restricted_code_read_only(upgraded_path);
        }
        return Err(error);
    }
    Ok(GitMutationAcl {
        dot_git,
        metadata_dirs,
    })
}

pub fn restore_git_mutation_acl(acl: &GitMutationAcl) -> Result<(), String> {
    let mut errors = Vec::new();
    // Restore the per-worktree directory before its common parent. Both paths
    // receive an explicit RX entry so a later, separate mutation lease can
    // safely upgrade them again without inheriting stale write rights.
    for path in acl.metadata_dirs.iter().rev() {
        if let Err(error) = protect_restricted_code_read_only(path) {
            errors.push(error);
        }
    }
    if acl.dot_git.exists()
        && let Err(error) = protect_restricted_code_read_only(&acl.dot_git)
    {
        errors.push(error);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn git_common_dir(checkout: &Path) -> Result<Option<std::path::PathBuf>, String> {
    git_metadata_dir(checkout, "--git-common-dir")
}

fn git_absolute_dir(checkout: &Path) -> Result<Option<std::path::PathBuf>, String> {
    git_metadata_dir(checkout, "--absolute-git-dir")
}

fn git_metadata_dir(
    checkout: &Path,
    revision_argument: &str,
) -> Result<Option<std::path::PathBuf>, String> {
    #[cfg(test)]
    let git = std::path::PathBuf::from("git");
    #[cfg(not(test))]
    let git = trusted_git_executable()?;
    let output = std::process::Command::new(git)
        .args(["-C"])
        .arg(checkout)
        .args(["rev-parse", revision_argument])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Ok(None);
    }
    let value = String::from_utf8(output.stdout)
        .map_err(|_| "Git metadata directory is not UTF-8".to_owned())?;
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let path = std::path::PathBuf::from(value);
    let path = if path.is_absolute() {
        path
    } else {
        checkout.join(path)
    };
    path.canonicalize()
        .map(Some)
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
fn restricted_sid_available() -> bool {
    use windows_sys::Win32::Security::{
        CreateWellKnownSid, SECURITY_MAX_SID_SIZE, WinRestrictedCodeSid,
    };

    let mut sid = vec![0_u8; usize::try_from(SECURITY_MAX_SID_SIZE).unwrap_or(68)];
    let mut length = u32::try_from(sid.len()).unwrap_or(u32::MAX);
    unsafe {
        CreateWellKnownSid(
            WinRestrictedCodeSid,
            std::ptr::null_mut(),
            sid.as_mut_ptr().cast(),
            &mut length,
        ) != 0
    }
}

#[cfg(not(windows))]
fn restricted_sid_available() -> bool {
    false
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::git_common_dir;

    #[test]
    fn linked_worktree_gitfile_resolves_the_shared_common_directory() {
        // Adapted from openai/codex
        // codex-rs/windows-sandbox-rs/src/sandbox_utils.rs at commit
        // 4c43465133428898aa84f0bfc02c306ed65fb66a.
        let fixture = tempfile::tempdir().expect("fixture");
        let repository = fixture.path().join("repository");
        let worktree = fixture.path().join("linked-worktree");
        run_git(
            fixture.path(),
            &["init", repository.to_str().expect("repo path")],
        );
        std::fs::write(repository.join("tracked.txt"), "tracked").expect("tracked fixture");
        run_git(&repository, &["add", "tracked.txt"]);
        run_git(
            &repository,
            &[
                "-c",
                "user.name=Hachimi Test",
                "-c",
                "user.email=hachimi-test@example.invalid",
                "commit",
                "-m",
                "fixture",
            ],
        );
        run_git(
            &repository,
            &[
                "worktree",
                "add",
                "-b",
                "linked-fixture",
                worktree.to_str().expect("worktree path"),
            ],
        );

        assert!(worktree.join(".git").is_file());
        let resolved = git_common_dir(&worktree)
            .expect("git common dir query")
            .expect("linked worktree common dir");
        assert_eq!(
            resolved,
            repository.join(".git").canonicalize().expect("common dir")
        );
    }

    fn run_git(cwd: &std::path::Path, arguments: &[&str]) {
        let output = std::process::Command::new("git")
            .args(arguments)
            .current_dir(cwd)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()
            .expect("git invocation");
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
