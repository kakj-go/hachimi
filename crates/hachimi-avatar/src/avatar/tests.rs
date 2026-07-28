use super::*;

const DEFAULT_AVATAR: &str =
    "../../assets/avatar-default/2639776812528692620/2639776812528692620.vrm";

fn make_legacy_glb() -> Vec<u8> {
    let mut json = serde_json::to_vec(&serde_json::json!({
        "asset": { "version": "2.0" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "mesh": 0 }],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0 } }] }],
        "buffers": [{ "byteLength": 36 }],
        "bufferViews": [{ "buffer": 0, "byteOffset": 0, "byteLength": 36 }],
        "accessors": [{
            "bufferView": 0,
            "componentType": 5126,
            "count": 3,
            "type": "VEC3",
            "min": [-1.0, 0.0, 0.0],
            "max": [1.0, 2.0, 0.0]
        }]
    }))
    .expect("serialize fixture");
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let binary = [0_u8; 36];
    let total = 12 + 8 + json.len() + 8 + binary.len();
    let mut bytes = Vec::with_capacity(total);
    bytes.extend_from_slice(b"glTF");
    bytes.extend_from_slice(&2_u32.to_le_bytes());
    bytes.extend_from_slice(&(total as u32).to_le_bytes());
    bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&0x4E4F_534A_u32.to_le_bytes());
    bytes.extend_from_slice(&json);
    bytes.extend_from_slice(&(binary.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&0x004E_4942_u32.to_le_bytes());
    bytes.extend_from_slice(&binary);
    bytes
}

fn mutate_default_vrm(mutate: impl FnOnce(&mut Value)) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_AVATAR);
    let source = fs::read(path).expect("default VRM");
    let mut document = read_json_chunk(&source).expect("default VRM JSON");
    mutate(&mut document);
    let binary = glb_binary_chunk(&source).expect("default VRM binary");
    let mut json = serde_json::to_vec(&document).expect("mutated JSON");
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut padded_binary = binary.to_vec();
    while !padded_binary.len().is_multiple_of(4) {
        padded_binary.push(0);
    }
    let total = 12 + 8 + json.len() + 8 + padded_binary.len();
    let mut bytes = Vec::with_capacity(total);
    bytes.extend_from_slice(b"glTF");
    bytes.extend_from_slice(&2_u32.to_le_bytes());
    bytes.extend_from_slice(&(total as u32).to_le_bytes());
    bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&0x4E4F_534A_u32.to_le_bytes());
    bytes.extend_from_slice(&json);
    bytes.extend_from_slice(&(padded_binary.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&0x004E_4942_u32.to_le_bytes());
    bytes.extend_from_slice(&padded_binary);
    bytes
}

fn inspect_mutated(bytes: &[u8]) -> InspectedAvatar {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("mutated.vrm");
    fs::write(&path, bytes).expect("mutated fixture");
    inspect_avatar(&path).expect("inspect mutated VRM")
}

#[test]
fn bundled_default_avatar_is_runtime_ready() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_AVATAR);
    let inspection = inspect_avatar(&path).expect("inspect bundled default VRM");
    assert_eq!(inspection.format, AvatarFormat::Vrm0);
    assert_eq!(
        inspection.assessment.compatibility,
        AvatarCompatibility::RuntimeReady,
        "issues: {:?}; requirements: {:?}",
        inspection.assessment.issues,
        inspection.assessment.requirements
    );
    assert!(inspection.is_compatible());
    assert!(inspection.profile.bones.len() >= REQUIRED_RUNTIME_BONES.len());
    assert!(inspection.profile.spring_bone_group_count > 0);
    assert!(inspection.profile.collider_count > 0);
}

#[test]
fn ordinary_glb_extension_is_not_an_import_candidate() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_AVATAR);
    let directory = tempfile::tempdir().expect("tempdir");
    let glb = directory.path().join("avatar.glb");
    fs::copy(source, &glb).expect("copy fixture");
    assert!(matches!(
        inspect_avatar(&glb),
        Err(AvatarError::InvalidExtension)
    ));
}

#[test]
fn core_requirements_reject_invalid_models_while_optional_features_degrade() {
    let missing_core_bone = inspect_mutated(&mutate_default_vrm(|document| {
        document
            .pointer_mut("/extensions/VRM/humanoid/humanBones")
            .and_then(Value::as_array_mut)
            .expect("VRM0 bones")
            .retain(|bone| bone.get("bone").and_then(Value::as_str) != Some("leftFoot"));
    }));
    assert_eq!(
        missing_core_bone.assessment.compatibility,
        AvatarCompatibility::Incompatible
    );
    assert!(
        missing_core_bone
            .assessment
            .requirements
            .iter()
            .any(
                |requirement| requirement.requirement == "complete_humanoid" && !requirement.passed
            )
    );
    assert!(
        missing_core_bone
            .view(Some("forbidden".into()))
            .token
            .is_none()
    );

    let missing_chest = inspect_mutated(&mutate_default_vrm(|document| {
        document
            .pointer_mut("/extensions/VRM/humanoid/humanBones")
            .and_then(Value::as_array_mut)
            .expect("VRM0 bones")
            .retain(|bone| {
                !matches!(
                    bone.get("bone").and_then(Value::as_str),
                    Some("chest" | "upperChest")
                )
            });
    }));
    assert_eq!(
        missing_chest.assessment.compatibility,
        AvatarCompatibility::RuntimeReady
    );
    assert!(
        missing_chest
            .assessment
            .requirements
            .iter()
            .any(|requirement| requirement.requirement == "chest_bone" && !requirement.passed)
    );

    let missing_face = inspect_mutated(&mutate_default_vrm(|document| {
        document["extensions"]["VRM"]["blendShapeMaster"]["blendShapeGroups"] =
            serde_json::json!([]);
    }));
    assert_eq!(
        missing_face.assessment.compatibility,
        AvatarCompatibility::RuntimeReady
    );
    assert_eq!(missing_face.profile.lip_sync, LipSyncCapability::None);
    for requirement in ["standard_blinks", "five_visemes", "standard_emotions"] {
        assert!(
            missing_face
                .assessment
                .requirements
                .iter()
                .any(|value| { value.requirement == requirement && !value.passed })
        );
    }

    let missing_physics = inspect_mutated(&mutate_default_vrm(|document| {
        document["extensions"]["VRM"]["secondaryAnimation"] = serde_json::json!({});
    }));
    assert_eq!(
        missing_physics.assessment.compatibility,
        AvatarCompatibility::RuntimeReady
    );
    for requirement in ["spring_bone", "spring_collider"] {
        assert!(
            missing_physics
                .assessment
                .requirements
                .iter()
                .any(|value| { value.requirement == requirement && !value.passed })
        );
    }

    let over_budget = inspect_mutated(&mutate_default_vrm(|document| {
        document["materials"] = Value::Array((0..65).map(|_| serde_json::json!({})).collect());
    }));
    assert!(
        over_budget
            .assessment
            .requirements
            .iter()
            .any(|value| { value.requirement == "resource_budget" && !value.passed })
    );
}

#[test]
fn unknown_required_extension_is_rejected_before_runtime_profile_creation() {
    let inspection = inspect_mutated(&mutate_default_vrm(|document| {
        document["extensionsRequired"] = serde_json::json!(["VENDOR_unknown_runtime"]);
    }));
    assert_eq!(
        inspection.assessment.compatibility,
        AvatarCompatibility::Incompatible
    );
    assert!(
        inspection
            .assessment
            .issues
            .iter()
            .any(|issue| issue.code == "unsupported_required_extension")
    );
    assert!(inspection.profile.bones.is_empty());
}

#[test]
fn v4_catalog_does_not_read_or_delete_legacy_catalog_data() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path().join("models");
    let blob = root.join("legacy-sha").join(SOURCE_FILE_NAME);
    fs::create_dir_all(blob.parent().expect("blob parent")).expect("blob directory");
    fs::write(&blob, make_legacy_glb()).expect("legacy blob");
    let old_catalog = serde_json::json!({
        "schemaVersion": 2,
        "detectorVersion": 2,
        "entries": [{
            "id": "legacy-glb",
            "name": "Legacy GLB",
            "originalFileName": "legacy.glb",
            "sizeBytes": fs::metadata(&blob).expect("metadata").len(),
            "sha256": "legacy-sha",
            "importedAt": "1",
            "isCurrent": true,
            "format": "glb",
            "assessment": {
                "status": "compatible",
                "level": "l0_basic",
                "detectorVersion": 2,
                "capabilities": ["renderable_mesh"],
                "statistics": {},
                "issues": []
            }
        }],
        "currentId": "legacy-glb",
        "profiles": {}
    });
    fs::write(
        root.join("catalog.json"),
        serde_json::to_vec(&old_catalog).expect("catalog JSON"),
    )
    .expect("old catalog");
    let default = Path::new(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_AVATAR);

    let catalog = AvatarCatalog::load_with_default(&root, &default).expect("load V4 catalog");
    let snapshot = catalog.snapshot();
    assert!(
        snapshot
            .entries
            .iter()
            .all(|entry| entry.id != "legacy-glb")
    );
    assert_eq!(snapshot.current_id.as_deref(), Some(DEFAULT_AVATAR_ID));
    assert!(blob.is_file(), "V4 startup must not delete old user blobs");
    assert!(root.join("catalog.json").is_file());
    let saved: Value =
        serde_json::from_slice(&fs::read(root.join(CATALOG_FILE_NAME)).expect("saved V4 catalog"))
            .expect("saved catalog JSON");
    assert_eq!(saved["schemaVersion"], CATALOG_SCHEMA_VERSION);
    assert_eq!(saved["detectorVersion"], DETECTOR_VERSION);
}

#[test]
fn removed_bundled_avatar_is_replaced_when_it_was_current() {
    let directory = tempfile::tempdir().expect("temporary avatar catalog");
    let default = Path::new(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_AVATAR);
    drop(
        AvatarCatalog::load_with_default(directory.path(), &default)
            .expect("create current catalog"),
    );
    let catalog_path = directory.path().join(CATALOG_FILE_NAME);
    let mut document: Value =
        serde_json::from_slice(&fs::read(&catalog_path).expect("read current catalog"))
            .expect("catalog JSON");
    let removed_default_id = REMOVED_DEFAULT_AVATAR_IDS[1];
    document["entries"][0]["id"] = Value::String(removed_default_id.into());
    document["currentId"] = Value::String(removed_default_id.into());
    let profile = document["profiles"]
        .as_object_mut()
        .expect("profile map")
        .remove(DEFAULT_AVATAR_ID)
        .expect("default profile");
    document["profiles"]
        .as_object_mut()
        .expect("profile map")
        .insert(removed_default_id.into(), profile);
    fs::write(
        &catalog_path,
        serde_json::to_vec(&document).expect("serialize old default catalog"),
    )
    .expect("write old default catalog");

    let catalog = AvatarCatalog::load_with_default(directory.path(), &default)
        .expect("replace removed default");
    let snapshot = catalog.snapshot();
    assert_eq!(snapshot.current_id.as_deref(), Some(DEFAULT_AVATAR_ID));
    assert!(
        snapshot
            .entries
            .iter()
            .all(|entry| entry.id != removed_default_id)
    );
}

#[test]
fn optional_reference_vrm_remains_runtime_ready() {
    let Ok(path) = std::env::var("HACHIMI_REFERENCE_VRM") else {
        return;
    };
    let inspection = inspect_avatar(Path::new(&path)).expect("inspect reference VRM");
    assert_eq!(
        inspection.assessment.compatibility,
        AvatarCompatibility::RuntimeReady,
        "issues: {:?}; requirements: {:?}",
        inspection.assessment.issues,
        inspection.assessment.requirements
    );
}
