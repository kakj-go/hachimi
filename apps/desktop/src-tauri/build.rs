use std::{
    fs::File,
    io::Read,
    path::{Component, Path},
};

use sha2::{Digest, Sha256};

fn sha256_file(path: &Path) -> String {
    let mut file = File::open(path)
        .unwrap_or_else(|error| panic!("failed to open {} for hashing: {error}", path.display()));
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .unwrap_or_else(|error| panic!("failed to hash {}: {error}", path.display()));
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn verify_speech_model_manifest(manifest_path: &Path) {
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    let bytes = std::fs::read(manifest_path).unwrap_or_else(|error| {
        panic!(
            "bundled speech model manifest is missing: {}: {error}",
            manifest_path.display()
        )
    });
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        panic!(
            "bundled speech model manifest is invalid: {}: {error}",
            manifest_path.display()
        )
    });
    let files = manifest["files"].as_object().unwrap_or_else(|| {
        panic!(
            "bundled speech model manifest has no verified files: {}",
            manifest_path.display()
        )
    });
    let root = manifest_path
        .parent()
        .expect("speech model manifest must have a parent directory");
    for (relative, expected) in files {
        let relative_path = Path::new(relative);
        if relative_path.as_os_str().is_empty()
            || relative_path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            panic!("invalid bundled speech model path: {relative}");
        }
        let path = root.join(relative_path);
        println!("cargo:rerun-if-changed={}", path.display());
        let expected_size = expected["size"]
            .as_u64()
            .unwrap_or_else(|| panic!("missing size for bundled speech model file: {relative}"));
        let expected_sha = expected["sha256"]
            .as_str()
            .unwrap_or_else(|| panic!("missing SHA-256 for bundled speech model file: {relative}"));
        let metadata = std::fs::metadata(&path).unwrap_or_else(|error| {
            panic!(
                "bundled speech model file is missing: {}: {error}. Run `corepack pnpm models:prepare` first",
                path.display()
            )
        });
        if !metadata.is_file() || metadata.len() != expected_size {
            panic!(
                "bundled speech model file has an invalid size: {}. Run `corepack pnpm models:prepare`",
                path.display()
            );
        }
        let actual_sha = sha256_file(&path);
        if !actual_sha.eq_ignore_ascii_case(expected_sha) {
            panic!(
                "bundled speech model SHA-256 mismatch: {}. Run `corepack pnpm models:prepare`",
                path.display()
            );
        }
    }
}

fn verify_default_avatar_manifest(manifest_path: &Path) {
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(manifest_path).unwrap_or_else(|error| {
            panic!(
                "bundled default avatar manifest is missing: {}: {error}",
                manifest_path.display()
            )
        }))
        .unwrap_or_else(|error| panic!("bundled default avatar manifest is invalid: {error}"));
    let root = manifest_path
        .parent()
        .expect("default avatar manifest must have a parent directory");
    let file_name = manifest["file"]
        .as_str()
        .expect("default avatar manifest must contain a file name");
    assert_eq!(
        Path::new(file_name).components().count(),
        1,
        "default avatar file name must not contain a path"
    );
    let path = root.join(file_name);
    println!("cargo:rerun-if-changed={}", path.display());
    let expected_size = manifest["sizeBytes"]
        .as_u64()
        .expect("default avatar manifest must contain sizeBytes");
    let expected_sha = manifest["sha256"]
        .as_str()
        .expect("default avatar manifest must contain sha256");
    let actual_size = std::fs::metadata(&path)
        .unwrap_or_else(|error| {
            panic!(
                "bundled default avatar is missing: {}: {error}",
                path.display()
            )
        })
        .len();
    assert_eq!(
        actual_size, expected_size,
        "bundled default avatar size mismatch"
    );
    assert_eq!(
        sha256_file(&path),
        expected_sha,
        "bundled default avatar SHA-256 mismatch"
    );
}

fn verify_native_runtime(runtime: &Path) {
    let manifest_path = runtime.join("manifest.json");
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    let bytes = std::fs::read(&manifest_path).unwrap_or_else(|error| {
        panic!(
            "Windows DirectML runtime manifest is missing: {}: {error}",
            manifest_path.display()
        )
    });
    let manifest: serde_json::Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("Windows DirectML manifest is invalid: {error}"));
    let files = manifest["files"]
        .as_object()
        .expect("Windows DirectML manifest must contain file hashes");
    for (name, expected) in files {
        let relative = Path::new(name);
        if relative.components().count() != 1 {
            panic!("invalid Windows DirectML runtime file name: {name}");
        }
        let path = runtime.join(relative);
        println!("cargo:rerun-if-changed={}", path.display());
        if !path.is_file() {
            panic!(
                "Windows DirectML runtime is incomplete: missing {}",
                path.display()
            );
        }
        let expected_sha = expected
            .as_str()
            .unwrap_or_else(|| panic!("missing DirectML runtime SHA-256 for {name}"));
        if !sha256_file(&path).eq_ignore_ascii_case(expected_sha) {
            panic!(
                "Windows DirectML runtime SHA-256 mismatch: {}",
                path.display()
            );
        }
    }
}

fn verify_managed_git(root: &Path) {
    const VERSION: &str = "2.50.1.windows.1";
    const ARCHIVE_SHA256: &str = "6f672aebe9e488a246efd6875f9197dbc0d9a40100e218acc3877cba2b206c45";
    let manifest_path = root.join("manifest.json");
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    if !manifest_path.is_file() {
        if std::env::var("PROFILE").as_deref() == Ok("release") {
            panic!(
                "pinned Managed Git is missing. Run `corepack pnpm runtime:prepare` before a release build"
            );
        }
        println!(
            "cargo:warning=pinned Managed Git is absent; Workspace Git is unavailable until `corepack pnpm runtime:prepare` runs"
        );
        return;
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).expect("read Managed Git manifest"))
            .expect("Managed Git manifest is invalid");
    assert_eq!(manifest["version"].as_str(), Some(VERSION));
    assert_eq!(
        manifest["sourceArchiveSha256"].as_str(),
        Some(ARCHIVE_SHA256)
    );
    let files = manifest["files"]
        .as_object()
        .expect("Managed Git manifest has no file hashes");
    assert!(files.contains_key("cmd/git.exe"));
    for (relative, expected) in files {
        let relative_path = Path::new(relative);
        assert!(
            !relative_path.as_os_str().is_empty()
                && !relative_path.components().any(|component| matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )),
            "invalid Managed Git path: {relative}"
        );
        let path = root.join(relative_path);
        println!("cargo:rerun-if-changed={}", path.display());
        let metadata = std::fs::symlink_metadata(&path).unwrap_or_else(|error| {
            panic!("Managed Git file is missing: {}: {error}", path.display())
        });
        assert!(metadata.is_file() && !metadata.file_type().is_symlink());
        assert_eq!(
            sha256_file(&path),
            expected.as_str().expect("Managed Git SHA-256 must be text"),
            "Managed Git SHA-256 mismatch: {}",
            path.display()
        );
    }
}

fn verify_cef_runtime(root: &Path) {
    const CEF_CRATE_VERSION: &str = "151.2.0+151.3.14";
    const CHROMIUM_VERSION: &str = "151.0.7922.72";
    const ARCHIVE_SHA256: &str = "c63a18909fea077b5c3b5f9a3194f05781cd909efa8a6d7a543cad99c4183a55";
    let manifest_path = root.join("runtime-manifest.json");
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    if !manifest_path.is_file() {
        if std::env::var("PROFILE").as_deref() == Ok("release") {
            panic!("CEF Runtime is missing; run scripts/build-cef-host.ps1 -Release");
        }
        println!(
            "cargo:warning=CEF Runtime is absent; the embedded browser is unavailable until scripts/build-cef-host.ps1 runs"
        );
        return;
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).expect("read CEF Runtime manifest"))
            .expect("CEF Runtime manifest is invalid");
    assert_eq!(manifest["schemaVersion"].as_u64(), Some(1));
    assert_eq!(
        manifest["cefCrateVersion"].as_str(),
        Some(CEF_CRATE_VERSION)
    );
    assert_eq!(manifest["chromiumVersion"].as_str(), Some(CHROMIUM_VERSION));
    assert_eq!(manifest["platform"].as_str(), Some("windows-x64"));
    assert_eq!(manifest["archiveSha256"].as_str(), Some(ARCHIVE_SHA256));
    let files = manifest["files"]
        .as_array()
        .expect("CEF Runtime manifest must contain files");
    assert!(!files.is_empty(), "CEF Runtime manifest is empty");
    let mut has_host = false;
    let mut has_libcef = false;
    for entry in files {
        let relative = entry["path"].as_str().expect("CEF Runtime path");
        let relative_path = Path::new(relative);
        assert!(
            !relative_path.as_os_str().is_empty()
                && !relative_path.components().any(|component| matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )),
            "invalid CEF Runtime path: {relative}"
        );
        has_host |= relative.eq_ignore_ascii_case("hachimi-cef-host.exe");
        has_libcef |= relative.eq_ignore_ascii_case("libcef.dll");
        let path = root.join(relative_path);
        println!("cargo:rerun-if-changed={}", path.display());
        let metadata = std::fs::symlink_metadata(&path).unwrap_or_else(|error| {
            panic!("CEF Runtime file is missing: {}: {error}", path.display())
        });
        assert!(metadata.is_file() && !metadata.file_type().is_symlink());
        assert_eq!(
            metadata.len(),
            entry["size"].as_u64().expect("CEF Runtime size")
        );
        assert_eq!(
            sha256_file(&path),
            entry["sha256"].as_str().expect("CEF Runtime SHA-256"),
            "CEF Runtime SHA-256 mismatch: {}",
            path.display()
        );
    }
    assert!(has_host && has_libcef, "CEF Runtime is incomplete");
}

fn verify_motion_catalog(root: &Path) {
    let manifest_path = root.join("catalog.json");
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    let bytes = std::fs::read(&manifest_path).unwrap_or_else(|error| {
        panic!(
            "Avatar Motion Runtime V5 catalog is missing: {}: {error}",
            manifest_path.display()
        )
    });
    let manifest: serde_json::Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("V5 motion catalog is invalid: {error}"));
    assert_eq!(manifest["schemaVersion"].as_u64(), Some(2));
    assert!(
        manifest["transitionProfiles"]
            .as_array()
            .is_some_and(|profiles| !profiles.is_empty()),
        "V5 catalog must contain transition profiles"
    );
    let entries = manifest["entries"]
        .as_array()
        .expect("V5 catalog must contain entries");
    assert!(!entries.is_empty(), "V5 catalog must not be empty");
    let mut ids = std::collections::HashSet::new();
    let mut hash_entries =
        std::collections::HashMap::<String, Vec<(String, String, Option<String>)>>::new();
    for entry in entries {
        let id = entry["id"].as_str().expect("motion id");
        assert!(ids.insert(id.to_owned()), "duplicate motion id: {id}");
        assert!(
            id.starts_with("builtin."),
            "invalid built-in motion id: {id}"
        );
        let file_name = entry["fileName"].as_str().expect("motion file");
        let relative = Path::new(file_name);
        assert!(
            relative.components().count() == 1
                && relative.extension().is_some_and(|value| value == "vrma"),
            "invalid motion file path: {file_name}"
        );
        let path = root.join("builtin").join(relative);
        println!("cargo:rerun-if-changed={}", path.display());
        let expected_hash = entry["sha256"].as_str().expect("motion SHA-256");
        assert_eq!(expected_hash.len(), 64, "motion SHA-256 length");
        hash_entries
            .entry(expected_hash.to_owned())
            .or_default()
            .push((
                id.to_owned(),
                file_name.to_owned(),
                entry["derivedFromMotionId"].as_str().map(str::to_owned),
            ));
        assert!(
            sha256_file(&path).eq_ignore_ascii_case(expected_hash),
            "motion SHA-256 mismatch: {}",
            path.display()
        );
        let document = read_glb_json(&path);
        assert_eq!(
            document
                .pointer("/extensions/VRMC_vrm_animation/specVersion")
                .and_then(serde_json::Value::as_str),
            Some("1.0"),
            "motion {id} is not formal VRMA 1.0"
        );
        let animations = document["animations"].as_array().expect("VRMA animations");
        assert_eq!(
            animations.len(),
            1,
            "motion {id} must contain one animation"
        );
        assert!(entry["durationMs"].as_u64().is_some_and(|value| value > 0));
        assert!(
            entry["sourceProject"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
        assert!(
            entry["sourcePaths"]
                .as_array()
                .is_some_and(|value| !value.is_empty())
        );
    }
    for (hash, shared_entries) in hash_entries {
        if shared_entries.len() < 2 {
            continue;
        }
        let canonical_entries: Vec<_> = shared_entries
            .iter()
            .filter(|(_, _, source_id)| source_id.is_none())
            .collect();
        assert_eq!(
            canonical_entries.len(),
            1,
            "motion blob {hash} must have exactly one canonical entry"
        );
        let (canonical_id, canonical_file, _) = canonical_entries[0];
        for (id, file_name, source_id) in &shared_entries {
            if id == canonical_id {
                continue;
            }
            assert_eq!(
                source_id.as_deref(),
                Some(canonical_id.as_str()),
                "motion {id} shares blob {hash} without deriving from {canonical_id}"
            );
            assert_eq!(
                file_name, canonical_file,
                "derived motion {id} maps blob {hash} to a different file"
            );
        }
    }
}

fn register_sandbox_sidecar_hashes() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    for (name, environment) in [
        ("hachimi-sandbox-setup", "HACHIMI_SANDBOX_SETUP_SHA256"),
        (
            "hachimi-sandbox-launcher",
            "HACHIMI_SANDBOX_LAUNCHER_SHA256",
        ),
        ("hachimi-sandbox-canary", "HACHIMI_SANDBOX_CANARY_SHA256"),
        ("hachimi-sandbox-attest", "HACHIMI_SANDBOX_ATTEST_SHA256"),
        (
            "hachimi-workspace-worker",
            "HACHIMI_WORKSPACE_WORKER_SHA256",
        ),
    ] {
        let path = Path::new("resources/internal-runtime").join(format!("{name}.exe"));
        println!("cargo:rerun-if-changed={}", path.display());
        if !path.is_file() {
            panic!(
                "bundled Sandbox sidecar is missing: {}. Run the sidecar preparation script first",
                path.display()
            );
        }
        println!("cargo:rustc-env={environment}={}", sha256_file(&path));
    }
}

fn read_glb_json(path: &Path) -> serde_json::Value {
    let bytes = std::fs::read(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    assert!(bytes.len() >= 20, "VRMA is truncated: {}", path.display());
    assert_eq!(&bytes[0..4], b"glTF", "VRMA is not GLB: {}", path.display());
    assert_eq!(
        u32::from_le_bytes(bytes[4..8].try_into().expect("version")),
        2
    );
    assert_eq!(
        u32::from_le_bytes(bytes[8..12].try_into().expect("length")) as usize,
        bytes.len(),
        "VRMA declared length mismatch"
    );
    let json_length = u32::from_le_bytes(bytes[12..16].try_into().expect("JSON length")) as usize;
    assert_eq!(&bytes[16..20], b"JSON", "VRMA first chunk is not JSON");
    serde_json::from_slice(&bytes[20..20 + json_length])
        .unwrap_or_else(|error| panic!("VRMA JSON is invalid: {}: {error}", path.display()))
}

fn main() {
    println!(
        "cargo:rerun-if-changed=resources/native/sherpa-onnx-1.13.4-directml/windows-x64/manifest.json"
    );
    verify_motion_catalog(Path::new("../../../assets/avatar-motions-v5"));
    verify_default_avatar_manifest(Path::new(
        "../../../assets/avatar-default/2639776812528692620/manifest.json",
    ));
    verify_speech_model_manifest(Path::new(
        "resources/ai-models/speech-to-text/sensevoice-small/manifest.json",
    ));
    verify_speech_model_manifest(Path::new(
        "resources/ai-models/text-to-speech/vits-melo-zh-en/manifest.json",
    ));
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let runtime =
            std::path::Path::new("resources/native/sherpa-onnx-1.13.4-directml/windows-x64");
        verify_native_runtime(runtime);
        verify_managed_git(Path::new("managed-git"));
        verify_cef_runtime(Path::new("../../../target/cef-bundle"));
    }
    register_sandbox_sidecar_hashes();
    tauri_build::build();
}
