use std::{
    io::Write,
    path::{Component, Path, PathBuf},
};

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::workbench_commands::set_managed_workspace_runtime;

#[derive(Debug, Clone)]
pub(super) struct ManagedSandboxRuntime {
    pub root: PathBuf,
    pub setup: PathBuf,
    pub launcher: PathBuf,
    pub canary: PathBuf,
    pub worker: PathBuf,
    pub managed_git: Option<PathBuf>,
    pub expected_integrity: Vec<(PathBuf, String)>,
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

#[derive(Debug, Deserialize, Serialize)]
struct ManagedGitManifest {
    version: String,
    files: std::collections::BTreeMap<String, String>,
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
    for (name, expected) in definitions {
        stage_file(
            &packaged_sidecar_path(name)?,
            &root.join(executable_name(name)),
            expected,
        )?;
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
        .find(|candidate| candidate.join("manifest.json").is_file())
        .ok_or_else(|| {
            format!(
                "managed Git manifest is missing from packaged resources: {}",
                resource_root.display()
            )
        })?;
    let managed_git = stage_managed_git(&managed_git_source, &root.join("managed-git"))?;
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
    };
    write_manifest(&runtime, &definitions)?;
    set_managed_workspace_runtime(runtime.root.clone(), runtime.worker.clone())?;
    if let Some(git) = &runtime.managed_git {
        hachimi_sandbox::set_managed_git_executable(git.clone())?;
    }
    Ok(runtime)
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
    let manifest: ManagedGitManifest =
        serde_json::from_slice(&std::fs::read(&manifest_path).map_err(|error| error.to_string())?)
            .map_err(|error| format!("managed Git manifest: {error}"))?;
    if manifest.version.trim().is_empty() || manifest.files.is_empty() {
        return Err("managed Git manifest is incomplete".into());
    }
    remove_unlisted_files(destination, manifest.files.keys().map(String::as_str))?;
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
    let git = destination.join("cmd/git.exe");
    if !git.is_file() {
        return Err("managed Git manifest does not contain cmd/git.exe".into());
    }
    Ok(Some(git))
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
            } else if path.file_name().is_some_and(|name| name == "manifest.json")
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
