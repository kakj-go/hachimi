use std::{
    io::Write,
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use hachimi_protocol::RuntimeComponentId;

use crate::{
    runtime_supervisor::RuntimeSupervisor, workbench_commands::set_managed_workspace_runtime,
};

const MANAGED_GIT_VALIDATION_SCHEMA: u32 = 1;
const MANAGED_GIT_VALIDATION_FILE: &str = ".hachimi-validation-v1.json";

#[derive(Debug, Clone)]
pub(super) struct ManagedSandboxRuntime {
    pub root: PathBuf,
    pub setup: PathBuf,
    pub launcher: PathBuf,
    pub canary: PathBuf,
    pub worker: PathBuf,
    pub managed_git: Option<PathBuf>,
    pub expected_integrity: Vec<(PathBuf, String)>,
    pub issue_codes: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeManifest<'a> {
    policy_version: &'a str,
    files: Vec<RuntimeManifestFile<'a>>,
    managed_git: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeManifestFile<'a> {
    name: &'a str,
    sha256: &'a str,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ManagedGitManifest {
    version: String,
    files: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagedGitFileStamp {
    path: String,
    size: u64,
    modified_nanos: u64,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagedGitValidationStamp {
    schema_version: u32,
    manifest_sha256: String,
    source_files: Vec<ManagedGitFileStamp>,
    destination_files: Vec<ManagedGitFileStamp>,
}

pub(super) fn stage(
    data_root: &Path,
    resource_root: &Path,
) -> Result<ManagedSandboxRuntime, String> {
    let root = data_root
        .join("sandbox/windows/runtime")
        .join(hachimi_sandbox::SANDBOX_POLICY_VERSION);
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let definitions = [
        (
            "hachimi-sandbox-setup",
            env!("HACHIMI_SANDBOX_SETUP_SHA256"),
        ),
        (
            "hachimi-sandbox-launcher",
            env!("HACHIMI_SANDBOX_LAUNCHER_SHA256"),
        ),
        (
            "hachimi-sandbox-canary",
            env!("HACHIMI_SANDBOX_CANARY_SHA256"),
        ),
        (
            "hachimi-sandbox-attest",
            env!("HACHIMI_SANDBOX_ATTEST_SHA256"),
        ),
        (
            "hachimi-workspace-worker",
            env!("HACHIMI_WORKSPACE_WORKER_SHA256"),
        ),
    ];
    let mut issues = Vec::new();
    for (name, expected) in definitions {
        let result = packaged_sidecar_path(name)
            .and_then(|source| stage_file(&source, &root.join(executable_name(name)), expected));
        if let Err(error) = result {
            let code = sidecar_error_code(name);
            tracing::error!(code, %error, resource = name, "Packaged runtime resource staging failed");
            issues.push(code);
        }
    }
    let mut managed_git_candidates = vec![
        resource_root.join("managed-git"),
        resource_root.join("resources/managed-git"),
    ];
    if cfg!(debug_assertions) {
        managed_git_candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("managed-git"));
    }
    let managed_git_source = managed_git_candidates
        .into_iter()
        .find(|candidate| candidate.join("manifest.json").is_file());
    let managed_git = match managed_git_source {
        Some(source) => match stage_managed_git(&source, &root.join("managed-git")) {
            Ok(git) => git,
            Err(error) => {
                tracing::error!(code = "managed_git_invalid", %error, "Managed Git staging failed");
                issues.push("managed_git_invalid");
                None
            }
        },
        None => {
            tracing::error!(code = "managed_git_missing", root = %resource_root.display(), "Managed Git resource is missing");
            issues.push("managed_git_missing");
            None
        }
    };
    let runtime = ManagedSandboxRuntime {
        setup: root.join(executable_name("hachimi-sandbox-setup")),
        launcher: root.join(executable_name("hachimi-sandbox-launcher")),
        canary: root.join(executable_name("hachimi-sandbox-canary")),
        worker: root.join(executable_name("hachimi-workspace-worker")),
        root,
        managed_git,
        expected_integrity: definitions
            .iter()
            .map(|(name, expected)| {
                (
                    data_root
                        .join("sandbox/windows/runtime")
                        .join(hachimi_sandbox::SANDBOX_POLICY_VERSION)
                        .join(executable_name(name)),
                    (*expected).to_owned(),
                )
            })
            .collect(),
        issue_codes: Vec::new(),
    };
    if let Err(error) = write_manifest(&runtime, &definitions) {
        tracing::error!(code = "runtime_manifest_write_failed", %error, "Runtime manifest write failed");
        issues.push("runtime_manifest_write_failed");
    }
    if runtime.worker.is_file()
        && let Err(error) =
            set_managed_workspace_runtime(runtime.root.clone(), runtime.worker.clone())
    {
        tracing::error!(code = "workspace_worker_registration_failed", %error, "Workspace Worker registration failed");
        issues.push("workspace_worker_registration_failed");
    }
    if let Some(git) = &runtime.managed_git
        && let Err(error) = hachimi_sandbox::set_managed_git_executable(git.clone())
    {
        tracing::error!(code = "managed_git_registration_failed", %error, "Managed Git registration failed");
        issues.push("managed_git_registration_failed");
    }
    let mut runtime = runtime;
    issues.sort_unstable();
    issues.dedup();
    runtime.issue_codes = issues;
    Ok(runtime)
}

pub(super) fn stage_or_degrade(
    data_root: &Path,
    resource_root: &Path,
    supervisor: RuntimeSupervisor,
) -> ManagedSandboxRuntime {
    let runtime = stage(data_root, resource_root).unwrap_or_else(|error| {
        tracing::error!(code = "internal_resource_storage_failed", %error, "Internal runtime staging root is unavailable");
        let mut runtime = layout(data_root);
        runtime.issue_codes.push("internal_resource_storage_failed");
        runtime
    });
    publish_health(&supervisor, &runtime);
    if !runtime.issue_codes.is_empty() {
        let data_root = data_root.to_owned();
        let resource_root = resource_root.to_owned();
        let retry = supervisor.retry_signal(RuntimeComponentId::InternalResources);
        tauri::async_runtime::spawn(async move {
            loop {
                retry.notified().await;
                let next = stage(&data_root, &resource_root).unwrap_or_else(|error| {
                    tracing::error!(code = "internal_resource_storage_failed", %error, "Internal runtime restaging failed");
                    let mut runtime = layout(&data_root);
                    runtime.issue_codes.push("internal_resource_storage_failed");
                    runtime
                });
                publish_health(&supervisor, &next);
                if next.issue_codes.is_empty() {
                    break;
                }
            }
        });
    }
    runtime
}

fn layout(data_root: &Path) -> ManagedSandboxRuntime {
    let root = data_root
        .join("sandbox/windows/runtime")
        .join(hachimi_sandbox::SANDBOX_POLICY_VERSION);
    let definitions = [
        (
            "hachimi-sandbox-setup",
            env!("HACHIMI_SANDBOX_SETUP_SHA256"),
        ),
        (
            "hachimi-sandbox-launcher",
            env!("HACHIMI_SANDBOX_LAUNCHER_SHA256"),
        ),
        (
            "hachimi-sandbox-canary",
            env!("HACHIMI_SANDBOX_CANARY_SHA256"),
        ),
        (
            "hachimi-sandbox-attest",
            env!("HACHIMI_SANDBOX_ATTEST_SHA256"),
        ),
        (
            "hachimi-workspace-worker",
            env!("HACHIMI_WORKSPACE_WORKER_SHA256"),
        ),
    ];
    ManagedSandboxRuntime {
        setup: root.join(executable_name("hachimi-sandbox-setup")),
        launcher: root.join(executable_name("hachimi-sandbox-launcher")),
        canary: root.join(executable_name("hachimi-sandbox-canary")),
        worker: root.join(executable_name("hachimi-workspace-worker")),
        root,
        managed_git: None,
        expected_integrity: definitions
            .into_iter()
            .map(|(name, expected)| {
                (
                    data_root
                        .join("sandbox/windows/runtime")
                        .join(hachimi_sandbox::SANDBOX_POLICY_VERSION)
                        .join(executable_name(name)),
                    expected.to_owned(),
                )
            })
            .collect(),
        issue_codes: Vec::new(),
    }
}

fn publish_health(supervisor: &RuntimeSupervisor, runtime: &ManagedSandboxRuntime) {
    supervisor.replace_internal_resource_issues(
        "sandbox_workspace_git",
        runtime.issue_codes.iter().copied(),
    );
}

fn sidecar_error_code(name: &str) -> &'static str {
    match name {
        "hachimi-workspace-worker" => "workspace_worker_invalid",
        "hachimi-sandbox-setup" => "sandbox_setup_invalid",
        "hachimi-sandbox-launcher" => "sandbox_launcher_invalid",
        "hachimi-sandbox-canary" | "hachimi-sandbox-attest" => "sandbox_attestation_invalid",
        _ => "internal_resource_invalid",
    }
}

fn stage_file(source: &Path, destination: &Path, expected_hash: &str) -> Result<(), String> {
    let source_bytes = std::fs::read(source)
        .map_err(|error| format!("managed Runtime source {}: {error}", source.display()))?;
    if hash(&source_bytes) != expected_hash {
        return Err(format!(
            "managed Runtime source hash mismatch: {}",
            source.display()
        ));
    }
    if std::fs::read(destination)
        .ok()
        .is_some_and(|bytes| hash(&bytes) == expected_hash)
    {
        return Ok(());
    }
    atomic_write(destination, &source_bytes)?;
    let installed = std::fs::read(destination).map_err(|error| error.to_string())?;
    if hash(&installed) != expected_hash {
        return Err(format!(
            "managed Runtime attestation failed: {}",
            destination.display()
        ));
    }
    Ok(())
}

fn stage_managed_git(source: &Path, destination: &Path) -> Result<Option<PathBuf>, String> {
    let manifest_path = source.join("manifest.json");
    if !manifest_path.is_file() {
        return Ok(None);
    }
    let manifest_bytes = std::fs::read(&manifest_path).map_err(|error| error.to_string())?;
    let manifest: ManagedGitManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("managed Git manifest: {error}"))?;
    if manifest.version.trim().is_empty() || manifest.files.is_empty() {
        return Err("managed Git manifest is incomplete".into());
    }
    let git = destination.join("cmd/git.exe");
    remove_unlisted_files(destination, manifest.files.keys().map(String::as_str))?;
    if managed_git_validation_matches(source, destination, &manifest, &manifest_bytes)
        && git.is_file()
    {
        return Ok(Some(git));
    }
    for (relative, expected) in &manifest.files {
        let relative = safe_relative(relative)?;
        stage_file(
            &source.join(&relative),
            &destination.join(&relative),
            expected,
        )?;
    }
    atomic_write(
        &destination.join("manifest.json"),
        &serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
    )?;
    write_managed_git_validation(source, destination, &manifest, &manifest_bytes)?;
    if !git.is_file() {
        return Err("managed Git manifest does not contain cmd/git.exe".into());
    }
    Ok(Some(git))
}

fn managed_git_validation_matches(
    source: &Path,
    destination: &Path,
    manifest: &ManagedGitManifest,
    manifest_bytes: &[u8],
) -> bool {
    let Ok(bytes) = std::fs::read(destination.join(MANAGED_GIT_VALIDATION_FILE)) else {
        return false;
    };
    let Ok(stored) = serde_json::from_slice::<ManagedGitValidationStamp>(&bytes) else {
        return false;
    };
    let Ok(destination_manifest) =
        std::fs::read(destination.join("manifest.json")).and_then(|bytes| {
            serde_json::from_slice::<ManagedGitManifest>(&bytes).map_err(std::io::Error::other)
        })
    else {
        return false;
    };
    let Some(current) = managed_git_validation_stamp(source, destination, manifest, manifest_bytes)
    else {
        return false;
    };
    destination_manifest == *manifest && stored == current
}

fn write_managed_git_validation(
    source: &Path,
    destination: &Path,
    manifest: &ManagedGitManifest,
    manifest_bytes: &[u8],
) -> Result<(), String> {
    let stamp = managed_git_validation_stamp(source, destination, manifest, manifest_bytes)
        .ok_or_else(|| "managed Git validation metadata is unavailable".to_owned())?;
    atomic_write(
        &destination.join(MANAGED_GIT_VALIDATION_FILE),
        &serde_json::to_vec(&stamp).map_err(|error| error.to_string())?,
    )
}

fn managed_git_validation_stamp(
    source: &Path,
    destination: &Path,
    manifest: &ManagedGitManifest,
    manifest_bytes: &[u8],
) -> Option<ManagedGitValidationStamp> {
    Some(ManagedGitValidationStamp {
        schema_version: MANAGED_GIT_VALIDATION_SCHEMA,
        manifest_sha256: hash(manifest_bytes),
        source_files: managed_git_file_stamps(source, manifest)?,
        destination_files: managed_git_file_stamps(destination, manifest)?,
    })
}

fn managed_git_file_stamps(
    root: &Path,
    manifest: &ManagedGitManifest,
) -> Option<Vec<ManagedGitFileStamp>> {
    manifest
        .files
        .keys()
        .map(|value| {
            let relative = safe_relative(value).ok()?;
            let metadata = std::fs::metadata(root.join(&relative)).ok()?;
            if !metadata.is_file() {
                return None;
            }
            let modified_nanos = metadata
                .modified()
                .ok()?
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_nanos()
                .try_into()
                .unwrap_or(u64::MAX);
            Some(ManagedGitFileStamp {
                path: relative.to_string_lossy().replace('\\', "/"),
                size: metadata.len(),
                modified_nanos,
            })
        })
        .collect()
}

fn remove_unlisted_files<'a>(
    destination: &Path,
    listed: impl Iterator<Item = &'a str>,
) -> Result<(), String> {
    if !destination.is_dir() {
        return Ok(());
    }
    let allowed = listed
        .map(|value| safe_relative(value).map(|path| destination.join(path)))
        .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
    let mut directories = Vec::new();
    let mut pending = vec![destination.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                directories.push(path.clone());
                pending.push(path);
            } else if path
                .file_name()
                .is_some_and(|name| name == "manifest.json" || name == MANAGED_GIT_VALIDATION_FILE)
                || allowed.contains(&path)
            {
                continue;
            } else {
                std::fs::remove_file(&path).map_err(|error| error.to_string())?;
            }
        }
    }
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in directories {
        if std::fs::read_dir(&directory)
            .map_err(|error| error.to_string())?
            .next()
            .is_none()
        {
            std::fs::remove_dir(directory).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn safe_relative(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if path.as_os_str().is_empty()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("managed Runtime manifest path escapes its root".into());
    }
    Ok(path.to_owned())
}

fn write_manifest(
    runtime: &ManagedSandboxRuntime,
    definitions: &[(&str, &str)],
) -> Result<(), String> {
    let manifest = RuntimeManifest {
        policy_version: hachimi_sandbox::SANDBOX_POLICY_VERSION,
        files: definitions
            .iter()
            .map(|(name, sha256)| RuntimeManifestFile { name, sha256 })
            .collect(),
        managed_git: runtime
            .managed_git
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
    };
    atomic_write(
        &runtime.root.join("runtime-manifest.json"),
        &serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
    )
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut file = AtomicWriteFile::open(path).map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.flush().map_err(|error| error.to_string())?;
    file.commit().map_err(|error| error.to_string())
}

fn executable_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

fn packaged_sidecar_path(name: &str) -> Result<PathBuf, String> {
    std::env::current_exe()
        .map_err(|error| error.to_string())?
        .parent()
        .map(|parent| parent.join(executable_name(name)))
        .ok_or_else(|| "Hachimi executable has no parent directory".into())
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_managed_git_validation_repairs_same_size_changes() {
        let source = tempfile::tempdir().expect("source");
        let destination = tempfile::tempdir().expect("destination");
        let bytes = b"git-runtime";
        std::fs::create_dir_all(source.path().join("cmd")).expect("source cmd");
        std::fs::write(source.path().join("cmd/git.exe"), bytes).expect("source git");
        let manifest = ManagedGitManifest {
            version: "test-v1".into(),
            files: [("cmd/git.exe".into(), hash(bytes))].into_iter().collect(),
        };
        std::fs::write(
            source.path().join("manifest.json"),
            serde_json::to_vec(&manifest).expect("manifest"),
        )
        .expect("source manifest");

        stage_managed_git(source.path(), destination.path()).expect("initial stage");
        assert!(
            destination
                .path()
                .join(MANAGED_GIT_VALIDATION_FILE)
                .is_file()
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(destination.path().join("cmd/git.exe"), b"bad-runtime")
            .expect("modified git");
        stage_managed_git(source.path(), destination.path()).expect("repair stage");

        assert_eq!(
            std::fs::read(destination.path().join("cmd/git.exe")).expect("installed git"),
            bytes
        );
    }
}
