use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};

use hachimi_motion::{MotionCatalog, MotionError};
use hachimi_protocol::RuntimeComponentId;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use tauri::{AppHandle, Runtime};

use crate::runtime_supervisor::RuntimeSupervisor;

const RESOURCE_VALIDATION_SCHEMA: u32 = 1;
const RESOURCE_VALIDATION_FILE: &str = ".hachimi-resource-validation-v1.json";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResourceFileStamp {
    path: String,
    size: u64,
    modified_nanos: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResourceValidationStamp {
    schema_version: u32,
    manifest_sha256: String,
    source_files: Vec<ResourceFileStamp>,
    target_files: Vec<ResourceFileStamp>,
}

pub(super) fn load_motion_catalog(
    builtin_catalog: &Path,
    data_dir: &Path,
    supervisor: &RuntimeSupervisor,
) -> Result<MotionCatalog, MotionError> {
    let motion_root = data_dir.join("motions-v3");
    MotionCatalog::load(&motion_root, builtin_catalog).inspect(|_| {
        supervisor.replace_internal_resource_issues("motion_catalog", std::iter::empty::<String>());
    }).or_else(|error| {
        tracing::error!(code = "motion_catalog_unavailable", %error, "Bundled motion catalog is unavailable");
        supervisor.replace_internal_resource_issues("motion_catalog", ["motion_catalog_unavailable"]);
        MotionCatalog::load_degraded(motion_root)
    })
}

pub(super) struct OptionalResourceLayout {
    pub motion_catalog: PathBuf,
    pub speech_model: PathBuf,
    pub voice_model: PathBuf,
}

pub(super) fn stage_optional_resources<R: Runtime>(
    app: &AppHandle<R>,
    data_dir: &Path,
    supervisor: RuntimeSupervisor,
) -> OptionalResourceLayout {
    let motion_source = resolve_or_development(
        app,
        "resources/avatar-motions-v4/catalog.json",
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../assets/avatar-motions-v4/catalog.json"),
    );
    let speech_source = resolve_or_development(
        app,
        "resources/ai-models/speech-to-text/sensevoice-small/manifest.json",
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources/ai-models/speech-to-text/sensevoice-small/manifest.json"),
    );
    let voice_source = resolve_or_development(
        app,
        "resources/ai-models/text-to-speech/vits-melo-zh-en/manifest.json",
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources/ai-models/text-to-speech/vits-melo-zh-en/manifest.json"),
    );
    let root = data_dir.join("runtime-assets");
    let motion_target = root.join("avatar-motions-v4/catalog.json");
    let speech_target = root.join("speech-to-text/sensevoice-small/manifest.json");
    let voice_target = root.join("text-to-speech/vits-melo-zh-en/manifest.json");
    let mut issues = Vec::new();
    if let Err(code) = stage_motion(&motion_source, &motion_target) {
        issues.push(code);
    }
    if let Err(error) = stage_model(&speech_source, &speech_target) {
        tracing::error!(code = "speech_model_invalid", %error, "Speech model staging failed");
        issues.push("speech_model_invalid");
    }
    if let Err(error) = stage_model(&voice_source, &voice_target) {
        tracing::error!(code = "voice_model_invalid", %error, "Voice model staging failed");
        issues.push("voice_model_invalid");
    }
    supervisor.replace_internal_resource_issues("motion_voice", issues.iter().copied());
    if !issues.is_empty() {
        let retry = supervisor.retry_signal(RuntimeComponentId::InternalResources);
        let supervisor_retry = supervisor.clone();
        let motion_source_retry = motion_source.clone();
        let speech_source_retry = speech_source.clone();
        let voice_source_retry = voice_source.clone();
        let motion_target_retry = motion_target.clone();
        let speech_target_retry = speech_target.clone();
        let voice_target_retry = voice_target.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                retry.notified().await;
                let mut retry_issues = Vec::new();
                if let Err(code) = stage_motion(&motion_source_retry, &motion_target_retry) {
                    retry_issues.push(code);
                }
                if stage_model(&speech_source_retry, &speech_target_retry).is_err() {
                    retry_issues.push("speech_model_invalid");
                }
                if stage_model(&voice_source_retry, &voice_target_retry).is_err() {
                    retry_issues.push("voice_model_invalid");
                }
                supervisor_retry
                    .replace_internal_resource_issues("motion_voice", retry_issues.iter().copied());
                if retry_issues.is_empty() {
                    break;
                }
            }
        });
    }
    OptionalResourceLayout {
        motion_catalog: if motion_target.is_file() {
            motion_target
        } else {
            motion_source
        },
        speech_model: speech_target.parent().map_or_else(
            || speech_source.parent().unwrap_or(Path::new(".")).to_owned(),
            Path::to_owned,
        ),
        voice_model: voice_target.parent().map_or_else(
            || voice_source.parent().unwrap_or(Path::new(".")).to_owned(),
            Path::to_owned,
        ),
    }
}

fn resolve_or_development<R: Runtime>(
    app: &AppHandle<R>,
    resource: &str,
    development: PathBuf,
) -> PathBuf {
    let bundled = crate::app_shell::resolve_resource(app, resource);
    if bundled.is_file() || bundled.is_dir() {
        bundled
    } else {
        development
    }
}

fn stage_motion(source_catalog: &Path, target_catalog: &Path) -> Result<(), &'static str> {
    let source_root = source_catalog
        .parent()
        .ok_or("motion_catalog_unavailable")?;
    let target_root = target_catalog
        .parent()
        .ok_or("motion_catalog_unavailable")?;
    let bytes = fs::read(source_catalog).map_err(|_| "motion_catalog_unavailable")?;
    let document: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "motion_catalog_invalid")?;
    let entries = document["entries"]
        .as_array()
        .ok_or("motion_catalog_invalid")?;
    let mut files = Vec::new();
    for entry in entries {
        let name = entry["fileName"].as_str().ok_or("motion_catalog_invalid")?;
        let size = entry["sizeBytes"]
            .as_u64()
            .ok_or("motion_catalog_invalid")?;
        let hash = entry["sha256"].as_str().ok_or("motion_catalog_invalid")?;
        files.push((PathBuf::from("builtin").join(name), size, hash));
    }
    stage_resource(
        source_root,
        target_root,
        source_catalog,
        target_catalog,
        &bytes,
        &files,
        "motion_catalog_unavailable",
    )
}

fn stage_model(source_manifest: &Path, target_manifest: &Path) -> Result<(), &'static str> {
    let source_root = source_manifest.parent().ok_or("model_manifest_invalid")?;
    let target_root = target_manifest.parent().ok_or("model_manifest_invalid")?;
    let bytes = fs::read(source_manifest).map_err(|_| "model_manifest_missing")?;
    let document: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "model_manifest_invalid")?;
    let files = document["files"]
        .as_object()
        .ok_or("model_manifest_invalid")?;
    let entries = files
        .iter()
        .map(|(name, value)| {
            Ok((
                PathBuf::from(name),
                value["size"].as_u64().ok_or("model_manifest_invalid")?,
                value["sha256"].as_str().ok_or("model_manifest_invalid")?,
            ))
        })
        .collect::<Result<Vec<_>, &'static str>>()?;
    stage_resource(
        source_root,
        target_root,
        source_manifest,
        target_manifest,
        &bytes,
        &entries,
        "model_resource_unavailable",
    )
}

fn stage_resource(
    source_root: &Path,
    target_root: &Path,
    source_manifest: &Path,
    target_manifest: &Path,
    manifest_bytes: &[u8],
    files: &[(PathBuf, u64, &str)],
    error_code: &'static str,
) -> Result<(), &'static str> {
    if files
        .iter()
        .any(|(relative, _, _)| !resource_path_is_safe(relative))
    {
        return Err(error_code);
    }
    let manifest_sha256 = digest_hex(manifest_bytes);
    if validation_stamp_matches(
        source_root,
        target_root,
        target_manifest,
        manifest_bytes,
        files,
        &manifest_sha256,
    ) {
        return Ok(());
    }

    stage_files(source_root, target_root, files, error_code)?;
    copy_file(source_manifest, target_manifest).map_err(|_| error_code)?;
    write_validation_stamp(source_root, target_root, files, manifest_sha256).map_err(|_| error_code)
}

fn stage_files(
    source_root: &Path,
    target_root: &Path,
    files: &[(PathBuf, u64, &str)],
    error_code: &'static str,
) -> Result<(), &'static str> {
    for (relative, size, hash) in files {
        if !resource_path_is_safe(relative) {
            return Err(error_code);
        }
        let source = source_root.join(relative);
        if !verify_file(&source, *size, hash) {
            return Err(error_code);
        }
        let target = target_root.join(relative);
        if !verify_file(&target, *size, hash) {
            copy_file(&source, &target).map_err(|_| error_code)?;
            if !verify_file(&target, *size, hash) {
                return Err(error_code);
            }
        }
    }
    Ok(())
}

fn resource_path_is_safe(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

fn validation_stamp_matches(
    source_root: &Path,
    target_root: &Path,
    target_manifest: &Path,
    manifest_bytes: &[u8],
    files: &[(PathBuf, u64, &str)],
    manifest_sha256: &str,
) -> bool {
    let stamp_path = target_root.join(RESOURCE_VALIDATION_FILE);
    let Ok(stamp_bytes) = fs::read(stamp_path) else {
        return false;
    };
    let Ok(stamp) = serde_json::from_slice::<ResourceValidationStamp>(&stamp_bytes) else {
        return false;
    };
    if stamp.schema_version != RESOURCE_VALIDATION_SCHEMA
        || stamp.manifest_sha256 != manifest_sha256
        || !fs::read(target_manifest).is_ok_and(|target| target == manifest_bytes)
    {
        return false;
    }
    let Some(source_files) = collect_file_stamps(source_root, files) else {
        return false;
    };
    let Some(target_files) = collect_file_stamps(target_root, files) else {
        return false;
    };
    stamp.source_files == source_files && stamp.target_files == target_files
}

fn write_validation_stamp(
    source_root: &Path,
    target_root: &Path,
    files: &[(PathBuf, u64, &str)],
    manifest_sha256: String,
) -> std::io::Result<()> {
    let source_files = collect_file_stamps(source_root, files)
        .ok_or_else(|| std::io::Error::other("source resource metadata is unavailable"))?;
    let target_files = collect_file_stamps(target_root, files)
        .ok_or_else(|| std::io::Error::other("target resource metadata is unavailable"))?;
    let stamp = ResourceValidationStamp {
        schema_version: RESOURCE_VALIDATION_SCHEMA,
        manifest_sha256,
        source_files,
        target_files,
    };
    let path = target_root.join(RESOURCE_VALIDATION_FILE);
    let temporary = target_root.join(".hachimi-resource-validation-v1.tmp");
    let bytes = serde_json::to_vec(&stamp).map_err(std::io::Error::other)?;
    fs::write(&temporary, bytes)?;
    let _ = fs::remove_file(&path);
    fs::rename(temporary, path)
}

fn collect_file_stamps(
    root: &Path,
    files: &[(PathBuf, u64, &str)],
) -> Option<Vec<ResourceFileStamp>> {
    files
        .iter()
        .map(|(relative, expected_size, _)| {
            let metadata = fs::metadata(root.join(relative)).ok()?;
            if !metadata.is_file() || metadata.len() != *expected_size {
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
            Some(ResourceFileStamp {
                path: relative.to_string_lossy().replace('\\', "/"),
                size: metadata.len(),
                modified_nanos,
            })
        })
        .collect()
}

fn copy_file(source: &Path, target: &Path) -> std::io::Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = target.with_extension("stage.tmp");
    let mut input = fs::File::open(source)?;
    let mut output = fs::File::create(&temporary)?;
    std::io::copy(&mut input, &mut output)?;
    output.flush()?;
    drop(output);
    let _ = fs::remove_file(target);
    fs::rename(temporary, target)
}

fn verify_file(path: &Path, size: u64, expected_hash: &str) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() || metadata.len() != size {
        return false;
    }
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut hasher = sha2::Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let Ok(read) = file.read(&mut buffer) else {
            return false;
        };
        if read == 0 {
            break;
        }
        sha2::Digest::update(&mut hasher, &buffer[..read]);
    }
    let actual = sha2::Digest::finalize(hasher);
    actual
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
        .eq_ignore_ascii_case(expected_hash)
}

fn digest_hex(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn publish_voice_health(
    supervisor: &RuntimeSupervisor,
    speech_model_available: bool,
    voice_model_available: bool,
    speech_model_path: &Path,
) {
    let mut issues = Vec::new();
    if !speech_model_available {
        tracing::error!(path = %speech_model_path.display(), code = "speech_model_missing", "Bundled speech recognition model is missing");
        issues.push("speech_model_missing");
    }
    if !voice_model_available {
        tracing::error!(
            code = "voice_model_missing",
            "Bundled VITS model is missing"
        );
        issues.push("voice_model_missing");
    }
    supervisor.replace_internal_resource_issues("voice_runtime", issues);
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn digest(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    #[test]
    fn manifest_staging_repairs_a_corrupt_target() {
        let source = tempfile::tempdir().expect("source");
        let target = tempfile::tempdir().expect("target");
        let bytes = b"verified model";
        fs::write(source.path().join("model.bin"), bytes).expect("source model");
        fs::write(
            source.path().join("manifest.json"),
            serde_json::to_vec(&serde_json::json!({
                "files": {
                    "model.bin": { "size": bytes.len(), "sha256": digest(bytes) }
                }
            }))
            .expect("manifest"),
        )
        .expect("source manifest");
        fs::write(target.path().join("model.bin"), b"corrupt").expect("corrupt target");

        stage_model(
            &source.path().join("manifest.json"),
            &target.path().join("manifest.json"),
        )
        .expect("stage");

        assert_eq!(
            fs::read(target.path().join("model.bin")).expect("target model"),
            bytes
        );
        assert!(target.path().join("manifest.json").is_file());
        assert!(target.path().join(RESOURCE_VALIDATION_FILE).is_file());
    }

    #[test]
    fn cached_validation_detects_and_repairs_same_size_changes() {
        let source = tempfile::tempdir().expect("source");
        let target = tempfile::tempdir().expect("target");
        let bytes = b"verified model";
        let manifest = serde_json::to_vec(&serde_json::json!({
            "files": {
                "model.bin": { "size": bytes.len(), "sha256": digest(bytes) }
            }
        }))
        .expect("manifest");
        fs::write(source.path().join("model.bin"), bytes).expect("source model");
        fs::write(source.path().join("manifest.json"), manifest).expect("source manifest");
        let source_manifest = source.path().join("manifest.json");
        let target_manifest = target.path().join("manifest.json");

        stage_model(&source_manifest, &target_manifest).expect("initial stage");
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(target.path().join("model.bin"), b"modified model").expect("modified target");
        stage_model(&source_manifest, &target_manifest).expect("repair stage");

        assert_eq!(
            fs::read(target.path().join("model.bin")).expect("target model"),
            bytes
        );
    }

    #[test]
    fn manifest_staging_rejects_parent_traversal() {
        let source = tempfile::tempdir().expect("source");
        let target = tempfile::tempdir().expect("target");
        fs::write(
            source.path().join("manifest.json"),
            br#"{"files":{"../outside.bin":{"size":1,"sha256":"00"}}}"#,
        )
        .expect("source manifest");

        assert_eq!(
            stage_model(
                &source.path().join("manifest.json"),
                &target.path().join("manifest.json")
            ),
            Err("model_resource_unavailable")
        );
    }
}
