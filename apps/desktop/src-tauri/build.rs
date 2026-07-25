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

fn verify_motion_catalog(root: &Path) {
    let manifest_path = root.join("catalog.json");
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    let bytes = std::fs::read(&manifest_path).unwrap_or_else(|error| {
        panic!(
            "Avatar Motion Runtime V4 catalog is missing: {}: {error}",
            manifest_path.display()
        )
    });
    let manifest: serde_json::Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("V4 motion catalog is invalid: {error}"));
    assert_eq!(manifest["schemaVersion"].as_u64(), Some(1));
    let entries = manifest["entries"]
        .as_array()
        .expect("V4 catalog must contain entries");
    assert!(!entries.is_empty(), "V4 catalog must not be empty");
    let mut hashes = std::collections::HashSet::new();
    for entry in entries {
        let id = entry["id"].as_str().expect("motion id");
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
        assert!(
            hashes.insert(expected_hash),
            "duplicate catalog SHA-256: {expected_hash}"
        );
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
    verify_motion_catalog(Path::new("../../../assets/avatar-motions-v4"));
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
    }
    tauri_build::build();
}
