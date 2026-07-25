//! Validated GLB/VRM catalog, capability assessment, and content-addressed storage.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as FmtWrite,
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use atomic_write_file::AtomicWriteFile;
use gltf::{Gltf, Semantic, mesh::Mode};
use hachimi_core::FeatureAvailability;
use hachimi_protocol::{
    AvatarAdaptationProfile, AvatarAssessment, AvatarBodyProportions, AvatarCapability,
    AvatarCatalogSnapshot, AvatarCollisionCapsule, AvatarCompatibility, AvatarContactPoint,
    AvatarEntry, AvatarExpressionBinding, AvatarFormat, AvatarImportInspection, AvatarIssue,
    AvatarIssueSeverity, AvatarJointLimit, AvatarLookAtProfile, AvatarRequirementResult,
    AvatarRestBone, AvatarStatistics, LipSyncCapability,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

const MAX_GLB_BYTES: u64 = 200 * 1024 * 1024;
const CATALOG_SCHEMA_VERSION: u32 = 4;
const DETECTOR_VERSION: u32 = 4;
const CATALOG_FILE_NAME: &str = "catalog-v4.json";
const SOURCE_FILE_NAME: &str = "source.glb";
const DEFAULT_AVATAR_ID: &str = "hachimi-default-2639776812528692620";
const REMOVED_DEFAULT_AVATAR_IDS: [&str; 2] = [
    "hachimi-default-sendagaya-shino",
    "hachimi-default-3800386813668044008",
];
const DEFAULT_AVATAR_NAME: &str = "VRoid 2639776812528692620";

#[must_use]
pub const fn availability() -> FeatureAvailability {
    FeatureAvailability::Available
}

#[derive(Debug, Error)]
pub enum AvatarError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("catalog error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("模型名称长度必须为 1–64 个字符")]
    InvalidName,
    #[error("同名 3D 模型已存在")]
    DuplicateName,
    #[error("请选择 .vrm 文件；普通 GLB 与旧版 L0/L1 模型不再支持")]
    InvalidExtension,
    #[error("3D 模型文件不能超过 200MB")]
    TooLarge,
    #[error("模型不满足 Hachimi Runtime Ready 要求")]
    Incompatible,
    #[error("找不到指定的 3D 模型")]
    NotFound,
    #[error("内置默认角色不能删除")]
    Protected,
    #[error("不兼容模型不能设为当前桌宠")]
    IncompatibleSelection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogDocument {
    schema_version: u32,
    detector_version: u32,
    entries: Vec<AvatarEntry>,
    current_id: Option<String>,
    profiles: BTreeMap<String, AvatarAdaptationProfile>,
}

impl Default for CatalogDocument {
    fn default() -> Self {
        Self {
            schema_version: CATALOG_SCHEMA_VERSION,
            detector_version: DETECTOR_VERSION,
            entries: Vec::new(),
            current_id: None,
            profiles: BTreeMap::new(),
        }
    }
}

#[derive(Debug)]
pub struct AvatarCatalog {
    root: PathBuf,
    default_asset_path: Option<PathBuf>,
    document: CatalogDocument,
}

#[derive(Debug, Clone)]
pub struct AvatarAsset {
    pub entry: AvatarEntry,
    pub path: PathBuf,
    pub profile: AvatarAdaptationProfile,
}

#[derive(Debug, Clone)]
pub struct InspectedAvatar {
    pub original_file_name: String,
    pub size_bytes: u32,
    pub sha256: String,
    pub format: AvatarFormat,
    pub assessment: AvatarAssessment,
    pub profile: AvatarAdaptationProfile,
    pub modified_millis: u128,
}

impl InspectedAvatar {
    #[must_use]
    pub fn is_compatible(&self) -> bool {
        self.assessment.compatibility == AvatarCompatibility::RuntimeReady
    }

    #[must_use]
    pub fn view(&self, token: Option<String>) -> AvatarImportInspection {
        AvatarImportInspection {
            token: token.filter(|_| self.is_compatible()),
            original_file_name: self.original_file_name.clone(),
            size_bytes: self.size_bytes,
            sha256: self.sha256.clone(),
            format: self.format,
            assessment: self.assessment.clone(),
        }
    }
}

impl AvatarCatalog {
    pub fn load(root: impl Into<PathBuf>) -> Result<Self, AvatarError> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        let path = root.join(CATALOG_FILE_NAME);
        let document = if path.exists() {
            serde_json::from_slice(&fs::read(path)?)?
        } else {
            CatalogDocument::default()
        };
        let catalog = Self {
            root,
            default_asset_path: None,
            document,
        };
        Ok(catalog)
    }

    pub fn load_with_default(
        root: impl Into<PathBuf>,
        default_asset_path: impl Into<PathBuf>,
    ) -> Result<Self, AvatarError> {
        let default_asset_path = default_asset_path.into();
        let mut catalog = Self::load(root)?;
        let inspection = inspect_avatar(&default_asset_path)?;
        if !inspection.is_compatible() {
            return Err(AvatarError::Incompatible);
        }
        catalog.default_asset_path = Some(default_asset_path);
        let removed_default_was_current = catalog
            .document
            .current_id
            .as_deref()
            .is_some_and(|id| REMOVED_DEFAULT_AVATAR_IDS.contains(&id));
        catalog
            .document
            .entries
            .retain(|entry| !REMOVED_DEFAULT_AVATAR_IDS.contains(&entry.id.as_str()));
        for removed_id in REMOVED_DEFAULT_AVATAR_IDS {
            catalog.document.profiles.remove(removed_id);
        }
        if let Some(entry) = catalog
            .document
            .entries
            .iter_mut()
            .find(|entry| entry.id == DEFAULT_AVATAR_ID)
        {
            entry.name = DEFAULT_AVATAR_NAME.into();
            entry.original_file_name = inspection.original_file_name.clone();
            entry.size_bytes = inspection.size_bytes;
            entry.sha256 = inspection.sha256.clone();
            entry.format = inspection.format;
            entry.assessment = inspection.assessment.clone();
            entry.protected = true;
        } else {
            catalog.document.entries.push(AvatarEntry {
                id: DEFAULT_AVATAR_ID.into(),
                name: DEFAULT_AVATAR_NAME.into(),
                original_file_name: inspection.original_file_name.clone(),
                size_bytes: inspection.size_bytes,
                sha256: inspection.sha256.clone(),
                imported_at: "0".into(),
                is_current: false,
                protected: true,
                format: inspection.format,
                assessment: inspection.assessment.clone(),
            });
        }
        catalog
            .document
            .profiles
            .insert(DEFAULT_AVATAR_ID.into(), inspection.profile);
        let current_ready = catalog.document.current_id.as_deref().is_some_and(|id| {
            catalog.document.entries.iter().any(|entry| {
                entry.id == id
                    && entry.assessment.compatibility == AvatarCompatibility::RuntimeReady
            })
        });
        if removed_default_was_current || !current_ready {
            catalog.document.current_id = Some(DEFAULT_AVATAR_ID.into());
        }
        catalog.document.schema_version = CATALOG_SCHEMA_VERSION;
        catalog.document.detector_version = DETECTOR_VERSION;
        catalog.save()?;
        Ok(catalog)
    }

    #[must_use]
    pub fn snapshot(&self) -> AvatarCatalogSnapshot {
        let current_id = self.document.current_id.clone();
        AvatarCatalogSnapshot {
            entries: self
                .document
                .entries
                .iter()
                .cloned()
                .map(|mut entry| {
                    entry.is_current = entry.assessment.compatibility
                        == AvatarCompatibility::RuntimeReady
                        && current_id.as_deref() == Some(entry.id.as_str());
                    entry
                })
                .collect(),
            current_id,
        }
    }

    #[must_use]
    pub fn current_asset(&self) -> Option<AvatarAsset> {
        let current_id = self.document.current_id.as_deref()?;
        self.asset_for(current_id)
    }

    /// Resolves a Runtime Ready catalog entry without changing the user's current avatar.
    /// This is used by the Workbench Motion Library Lab for cross-model QA.
    #[must_use]
    pub fn asset_for(&self, id: &str) -> Option<AvatarAsset> {
        let entry = self
            .document
            .entries
            .iter()
            .find(|entry| {
                entry.id == id
                    && entry.assessment.compatibility == AvatarCompatibility::RuntimeReady
            })?
            .clone();
        let path = if entry.id == DEFAULT_AVATAR_ID {
            self.default_asset_path.clone()?
        } else {
            self.root.join(&entry.sha256).join(SOURCE_FILE_NAME)
        };
        let profile = self.document.profiles.get(id).cloned().unwrap_or_default();
        path.is_file().then_some(AvatarAsset {
            entry,
            path,
            profile,
        })
    }

    #[must_use]
    pub fn current_asset_for(&self, id: &str) -> Option<AvatarAsset> {
        self.current_asset().filter(|asset| asset.entry.id == id)
    }

    pub fn import(
        &mut self,
        name: &str,
        source: &Path,
    ) -> Result<AvatarCatalogSnapshot, AvatarError> {
        let inspection = inspect_avatar(source)?;
        self.import_inspected(name, source, &inspection)
    }

    pub fn import_inspected(
        &mut self,
        name: &str,
        source: &Path,
        inspection: &InspectedAvatar,
    ) -> Result<AvatarCatalogSnapshot, AvatarError> {
        let name = validate_name(name)?;
        if !inspection.is_compatible() {
            return Err(AvatarError::Incompatible);
        }
        let normalized_name = name.to_lowercase();
        if self
            .document
            .entries
            .iter()
            .any(|entry| entry.name.to_lowercase() == normalized_name)
        {
            return Err(AvatarError::DuplicateName);
        }
        let metadata = fs::metadata(source)?;
        if metadata.len() != u64::from(inspection.size_bytes) {
            return Err(AvatarError::Incompatible);
        }
        let id = Uuid::new_v4().to_string();
        let created_blob = self.install_blob(source, &inspection.sha256)?;
        let entry = AvatarEntry {
            id: id.clone(),
            name,
            original_file_name: inspection.original_file_name.clone(),
            size_bytes: inspection.size_bytes,
            sha256: inspection.sha256.clone(),
            imported_at: now_millis().to_string(),
            is_current: false,
            protected: false,
            format: inspection.format,
            assessment: inspection.assessment.clone(),
        };
        self.document.entries.push(entry);
        self.document
            .profiles
            .insert(id.clone(), inspection.profile.clone());
        if self.document.current_id.is_none() {
            self.document.current_id = Some(id.clone());
        }
        if let Err(error) = self.save() {
            self.document.entries.retain(|entry| entry.id != id);
            self.document.profiles.remove(&id);
            if created_blob {
                let _ = fs::remove_dir_all(self.root.join(&inspection.sha256));
            }
            return Err(error);
        }
        Ok(self.snapshot())
    }

    pub fn select(&mut self, id: &str) -> Result<AvatarCatalogSnapshot, AvatarError> {
        let entry = self
            .document
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or(AvatarError::NotFound)?;
        if entry.assessment.compatibility != AvatarCompatibility::RuntimeReady {
            return Err(AvatarError::IncompatibleSelection);
        }
        let previous = self.document.current_id.replace(id.to_owned());
        if let Err(error) = self.save() {
            self.document.current_id = previous;
            return Err(error);
        }
        Ok(self.snapshot())
    }

    pub fn delete(&mut self, id: &str) -> Result<AvatarCatalogSnapshot, AvatarError> {
        let index = self
            .document
            .entries
            .iter()
            .position(|entry| entry.id == id)
            .ok_or(AvatarError::NotFound)?;
        if self.document.entries[index].protected {
            return Err(AvatarError::Protected);
        }
        let previous = self.document.clone();
        let removed = self.document.entries.remove(index);
        self.document.profiles.remove(id);
        if self.document.current_id.as_deref() == Some(id) {
            self.document.current_id = newest_compatible_id(&self.document.entries);
        }
        if let Err(error) = self.save() {
            self.document = previous;
            return Err(error);
        }
        if !self
            .document
            .entries
            .iter()
            .any(|entry| entry.sha256 == removed.sha256)
        {
            let _ = fs::remove_dir_all(self.root.join(&removed.sha256));
        }
        Ok(self.snapshot())
    }

    fn install_blob(&self, source: &Path, sha256: &str) -> Result<bool, AvatarError> {
        let destination_dir = self.root.join(sha256);
        let destination = destination_dir.join(SOURCE_FILE_NAME);
        if destination.exists() {
            return Ok(false);
        }
        let temporary = self.root.join(format!(".import-{}.tmp", Uuid::new_v4()));
        fs::copy(source, &temporary)?;
        if let Err(error) = fs::create_dir(&destination_dir)
            && error.kind() != std::io::ErrorKind::AlreadyExists
        {
            let _ = fs::remove_file(&temporary);
            return Err(error.into());
        }
        if destination.exists() {
            fs::remove_file(temporary)?;
            return Ok(false);
        }
        if let Err(error) = fs::rename(&temporary, &destination) {
            let _ = fs::remove_file(&temporary);
            let _ = fs::remove_dir(&destination_dir);
            return Err(error.into());
        }
        Ok(true)
    }

    fn save(&self) -> Result<(), AvatarError> {
        let bytes = serde_json::to_vec_pretty(&self.document)?;
        let mut file = AtomicWriteFile::open(self.root.join(CATALOG_FILE_NAME))?;
        file.write_all(&bytes)?;
        file.flush()?;
        file.commit()?;
        Ok(())
    }
}

pub fn inspect_avatar(source: &Path) -> Result<InspectedAvatar, AvatarError> {
    inspect_avatar_impl(source, true)
}

fn inspect_avatar_impl(
    source: &Path,
    require_vrm_extension: bool,
) -> Result<InspectedAvatar, AvatarError> {
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if require_vrm_extension && !extension.eq_ignore_ascii_case("vrm") {
        return Err(AvatarError::InvalidExtension);
    }
    let metadata = fs::metadata(source)?;
    if metadata.len() > MAX_GLB_BYTES || metadata.len() > u64::from(u32::MAX) {
        return Err(AvatarError::TooLarge);
    }
    let bytes = fs::read(source)?;
    let sha256 = hash_bytes(&bytes);
    let modified_millis = metadata
        .modified()
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let (format, assessment, profile) = match analyze_glb(&bytes) {
        Ok(value) => value,
        Err(code) => (
            AvatarFormat::Glb,
            incompatible_assessment(code),
            AvatarAdaptationProfile::default(),
        ),
    };
    Ok(InspectedAvatar {
        original_file_name: source
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("model.vrm")
            .to_owned(),
        size_bytes: metadata.len() as u32,
        sha256,
        format,
        assessment,
        profile,
        modified_millis,
    })
}

fn analyze_glb(
    bytes: &[u8],
) -> Result<(AvatarFormat, AvatarAssessment, AvatarAdaptationProfile), &'static str> {
    let json = read_json_chunk(bytes)?;
    reject_external_resources(&json)?;
    reject_unsupported_extensions(&json)?;
    validate_embedded_resource_bounds(bytes, &json)?;
    let gltf = Gltf::from_slice(bytes).map_err(|_| "invalid_glb")?;
    let (reachable_nodes, reachable_meshes) = reachable_scene_content(&gltf);
    if reachable_nodes.is_empty() || reachable_meshes.is_empty() {
        return Err("valid_scene_missing");
    }
    let format = detect_format(&json);
    let mut statistics = AvatarStatistics {
        node_count: to_u32(gltf.nodes().count()),
        mesh_count: to_u32(gltf.meshes().count()),
        material_count: to_u32(gltf.materials().count()),
        texture_count: to_u32(gltf.textures().count()),
        animation_count: to_u32(gltf.animations().count()),
        ..AvatarStatistics::default()
    };
    let mut capabilities = BTreeSet::new();
    let mut renderable = false;
    let mut bounds_min = [f64::INFINITY; 3];
    let mut bounds_max = [f64::NEG_INFINITY; 3];
    for mesh in gltf
        .meshes()
        .filter(|mesh| reachable_meshes.contains(&mesh.index()))
    {
        for primitive in mesh.primitives() {
            statistics.primitive_count = statistics.primitive_count.saturating_add(1);
            statistics.morph_target_count = statistics
                .morph_target_count
                .saturating_add(to_u32(primitive.morph_targets().count()));
            if primitive.mode() != Mode::Triangles {
                continue;
            }
            let Some(position) = primitive.get(&Semantic::Positions) else {
                continue;
            };
            if position.count() < 3 {
                continue;
            }
            let (minimum, maximum) =
                accessor_bounds(&json, position.index()).ok_or("position_bounds_missing")?;
            for axis in 0..3 {
                bounds_min[axis] = bounds_min[axis].min(minimum[axis]);
                bounds_max[axis] = bounds_max[axis].max(maximum[axis]);
            }
            let index_count = primitive
                .indices()
                .map_or(position.count(), |indices| indices.count());
            statistics.triangle_count = statistics
                .triangle_count
                .saturating_add(to_u32(index_count / 3));
            renderable = true;
        }
    }
    if !renderable {
        return Err("renderable_mesh_missing");
    }
    if !(0..3).any(|axis| bounds_max[axis] > bounds_min[axis]) {
        return Err("model_bounds_empty");
    }
    capabilities.insert(AvatarCapability::RenderableMesh);

    let reachable_skin_indices: BTreeSet<_> = gltf
        .nodes()
        .filter(|node| reachable_nodes.contains(&node.index()) && node.mesh().is_some())
        .filter_map(|node| node.skin().map(|skin| skin.index()))
        .collect();
    let skinned = !reachable_skin_indices.is_empty();
    let joints: BTreeSet<_> = gltf
        .skins()
        .filter(|skin| reachable_skin_indices.contains(&skin.index()))
        .flat_map(|skin| skin.joints().map(|node| node.index()))
        .collect();
    statistics.bone_count = to_u32(joints.len());
    if skinned {
        capabilities.insert(AvatarCapability::SkinnedMesh);
    }
    if statistics.animation_count > 0 {
        capabilities.insert(AvatarCapability::BuiltInAnimations);
    }

    let mut issues = Vec::new();
    let mut requirements = Vec::new();
    let parents = parent_indices(&json, statistics.node_count as usize);
    let explicit_bones = explicit_humanoid_bones(&json, format);
    let bones = explicit_bones;
    let world_positions = node_world_positions(&gltf, &parents);
    let humanoid = skinned && validate_runtime_ready_humanoid(&bones, &parents, &joints);
    if humanoid {
        capabilities.insert(AvatarCapability::HumanoidSkeleton);
        capabilities.insert(AvatarCapability::StandardMotionRetarget);
    }

    let mut expressions = expression_bindings(&json, format);
    let standard_expression_names = declared_standard_expression_names(&json, format);
    expressions.sort_by(|left, right| left.expression.cmp(&right.expression));
    expressions.dedup_by(|left, right| {
        left.expression == right.expression
            && left.node_index == right.node_index
            && left.morph_index == right.morph_index
    });

    let expression_names: BTreeSet<_> = expressions
        .iter()
        .map(|binding| binding.expression.as_str())
        .collect();
    let blink = expression_names.contains("blink")
        || (expression_names.contains("blink_left") && expression_names.contains("blink_right"));
    let viseme = ["aa", "ih", "ou", "ee", "oh"]
        .iter()
        .any(|name| expression_names.contains(name));
    if blink {
        capabilities.insert(AvatarCapability::Blink);
    }
    if viseme {
        capabilities.insert(AvatarCapability::Viseme);
    }
    if expression_names.contains("happy") {
        capabilities.insert(AvatarCapability::HappyExpression);
    }
    if expression_names.contains("sad") {
        capabilities.insert(AvatarCapability::SadExpression);
    }
    if expression_names.contains("angry") {
        capabilities.insert(AvatarCapability::AngryExpression);
    }
    let look_at = has_path(&json, &["extensions", "VRMC_vrm", "lookAt"])
        || has_path(
            &json,
            &["extensions", "VRM", "firstPerson", "lookAtTypeName"],
        );
    if look_at {
        capabilities.insert(AvatarCapability::LookAt);
    }
    let (spring_bone_groups, collider_count) = spring_bone_counts(&json, format);
    let spring_bone = spring_bone_groups > 0;
    let spring_collider = collider_count > 0;
    if spring_bone {
        capabilities.insert(AvatarCapability::SpringBone);
    }
    if spring_collider {
        capabilities.insert(AvatarCapability::SpringBoneCollider);
    }
    let mtoon = has_mtoon(&json, format);
    if mtoon {
        capabilities.insert(AvatarCapability::MToon);
    }
    let complete_blinks = ["neutral", "blink", "blink_left", "blink_right"]
        .iter()
        .all(|name| standard_expression_names.contains(*name));
    let complete_visemes = ["aa", "ih", "ou", "ee", "oh"]
        .iter()
        .all(|name| standard_expression_names.contains(*name));
    let complete_emotions = ["happy", "relaxed", "sad", "angry", "surprised"]
        .iter()
        .all(|name| standard_expression_names.contains(*name));
    if complete_visemes {
        capabilities.insert(AvatarCapability::FiveVisemes);
        capabilities.insert(AvatarCapability::LipSyncFiveViseme);
    } else if viseme {
        capabilities.insert(AvatarCapability::LipSyncJaw);
    }
    if complete_emotions && complete_blinks {
        capabilities.insert(AvatarCapability::StandardExpressions);
    }
    if required_finger_bones()
        .iter()
        .all(|bone| bones.contains_key(*bone))
    {
        capabilities.insert(AvatarCapability::FiveFingerHands);
    }

    let texture_budget = inspect_texture_budget(bytes, &json)?;
    statistics.max_texture_dimension = texture_budget.max_dimension;
    statistics.estimated_texture_memory_bytes =
        u32::try_from(texture_budget.decoded_bytes).unwrap_or(u32::MAX);
    let skin_weights_valid = validate_skin_weights(&gltf, &reachable_nodes, &reachable_meshes);
    let required_format = matches!(format, AvatarFormat::Vrm0 | AvatarFormat::Vrm1);
    let budgets_valid = statistics.triangle_count <= 150_000
        && statistics.node_count <= 512
        && statistics.bone_count <= 256
        && statistics.material_count <= 64
        && statistics.texture_count <= 64
        && statistics.max_texture_dimension <= 4_096
        && statistics.estimated_texture_memory_bytes <= 512 * 1024 * 1024;
    for (id, passed, detail) in [
        ("vrm_format", required_format, format!("{format:?}")),
        ("skinned_mesh", skinned, statistics.mesh_count.to_string()),
        (
            "complete_humanoid",
            humanoid,
            missing_bones(&bones).join(", "),
        ),
        ("skin_weights", skin_weights_valid, String::new()),
        (
            "resource_budget",
            budgets_valid,
            format!(
                "triangles={}, nodes={}, joints={}, materials={}, textures={}, maxTexture={}, decodedTextureBytes={}",
                statistics.triangle_count,
                statistics.node_count,
                statistics.bone_count,
                statistics.material_count,
                statistics.texture_count,
                statistics.max_texture_dimension,
                statistics.estimated_texture_memory_bytes
            ),
        ),
    ] {
        requirements.push(AvatarRequirementResult {
            requirement: id.to_owned(),
            passed,
            detail,
        });
        if !passed {
            issues.push(issue(id, AvatarIssueSeverity::Error));
        }
    }
    for (id, passed, detail) in [
        (
            "chest_bone",
            bones.contains_key("chest") || bones.contains_key("upper_chest"),
            String::new(),
        ),
        (
            "toe_bones",
            bones.contains_key("left_toes") && bones.contains_key("right_toes"),
            String::new(),
        ),
        (
            "finger_bones",
            required_finger_bones()
                .iter()
                .all(|bone| bones.contains_key(*bone)),
            String::new(),
        ),
        (
            "standard_blinks",
            complete_blinks,
            missing_expressions(
                &standard_expression_names,
                &["neutral", "blink", "blink_left", "blink_right"],
            )
            .join(", "),
        ),
        (
            "jaw_lip_sync",
            viseme,
            missing_expressions(&standard_expression_names, &["aa", "ih", "ou", "ee", "oh"])
                .join(", "),
        ),
        (
            "five_visemes",
            complete_visemes,
            missing_expressions(&standard_expression_names, &["aa", "ih", "ou", "ee", "oh"])
                .join(", "),
        ),
        (
            "standard_emotions",
            complete_emotions,
            missing_expressions(
                &standard_expression_names,
                &["happy", "relaxed", "sad", "angry", "surprised"],
            )
            .join(", "),
        ),
        ("look_at", look_at, String::new()),
        ("mtoon", mtoon, String::new()),
        ("spring_bone", spring_bone, spring_bone_groups.to_string()),
        (
            "spring_collider",
            spring_collider,
            collider_count.to_string(),
        ),
    ] {
        requirements.push(AvatarRequirementResult {
            requirement: id.to_owned(),
            passed,
            detail,
        });
        if !passed {
            issues.push(issue(id, AvatarIssueSeverity::Warning));
        }
    }
    let runtime_ready =
        required_format && skinned && humanoid && skin_weights_valid && budgets_valid;
    if runtime_ready {
        capabilities.insert(AvatarCapability::RuntimeReady);
    }
    let profile = build_adaptation_profile(
        format,
        &json,
        &bones,
        &parents,
        &world_positions,
        expressions,
        spring_bone_groups,
        collider_count,
        bounds_min,
        bounds_max,
        if complete_visemes {
            LipSyncCapability::FiveViseme
        } else if viseme {
            LipSyncCapability::Jaw
        } else {
            LipSyncCapability::None
        },
    );
    let assessment = AvatarAssessment {
        compatibility: if runtime_ready {
            AvatarCompatibility::RuntimeReady
        } else {
            AvatarCompatibility::Incompatible
        },
        detector_version: DETECTOR_VERSION,
        capabilities: capabilities.into_iter().collect(),
        statistics,
        requirements,
        issues,
    };
    Ok((format, assessment, profile))
}

const REQUIRED_RUNTIME_BONES: &[&str] = &[
    "hips",
    "spine",
    "head",
    "left_upper_arm",
    "left_lower_arm",
    "left_hand",
    "right_upper_arm",
    "right_lower_arm",
    "right_hand",
    "left_upper_leg",
    "left_lower_leg",
    "left_foot",
    "right_upper_leg",
    "right_lower_leg",
    "right_foot",
];

fn required_finger_bones() -> &'static [&'static str] {
    &[
        "left_thumb_proximal",
        "left_thumb_intermediate",
        "left_thumb_distal",
        "left_index_proximal",
        "left_index_intermediate",
        "left_index_distal",
        "left_middle_proximal",
        "left_middle_intermediate",
        "left_middle_distal",
        "left_ring_proximal",
        "left_ring_intermediate",
        "left_ring_distal",
        "left_little_proximal",
        "left_little_intermediate",
        "left_little_distal",
        "right_thumb_proximal",
        "right_thumb_intermediate",
        "right_thumb_distal",
        "right_index_proximal",
        "right_index_intermediate",
        "right_index_distal",
        "right_middle_proximal",
        "right_middle_intermediate",
        "right_middle_distal",
        "right_ring_proximal",
        "right_ring_intermediate",
        "right_ring_distal",
        "right_little_proximal",
        "right_little_intermediate",
        "right_little_distal",
    ]
}

fn missing_bones(bones: &BTreeMap<String, u32>) -> Vec<String> {
    REQUIRED_RUNTIME_BONES
        .iter()
        .filter(|bone| !bones.contains_key(**bone))
        .map(|bone| (*bone).to_owned())
        .collect()
}

fn missing_expressions(names: &BTreeSet<String>, required: &[&str]) -> Vec<String> {
    required
        .iter()
        .filter(|name| !names.contains(**name))
        .map(|name| (*name).to_owned())
        .collect()
}

fn validate_runtime_ready_humanoid(
    bones: &BTreeMap<String, u32>,
    parents: &[Option<usize>],
    joints: &BTreeSet<usize>,
) -> bool {
    if !missing_bones(bones).is_empty() {
        return false;
    }
    let mut unique = BTreeSet::new();
    if !REQUIRED_RUNTIME_BONES.iter().all(|bone| {
        bones.get(*bone).is_some_and(|node| {
            let node = *node as usize;
            node < parents.len() && joints.contains(&node) && unique.insert(node)
        })
    }) {
        return false;
    }
    let chains: &[&[&str]] = &[
        &["hips", "spine", "head"],
        &["spine", "left_upper_arm", "left_lower_arm", "left_hand"],
        &["spine", "right_upper_arm", "right_lower_arm", "right_hand"],
        &["hips", "left_upper_leg", "left_lower_leg", "left_foot"],
        &["hips", "right_upper_leg", "right_lower_leg", "right_foot"],
    ];
    if !chains
        .iter()
        .all(|chain| hierarchy_chain_is_valid(chain, bones, parents))
    {
        return false;
    }
    true
}

fn hierarchy_chain_is_valid(
    chain: &[&str],
    bones: &BTreeMap<String, u32>,
    parents: &[Option<usize>],
) -> bool {
    chain.windows(2).all(|pair| {
        let Some(parent) = bones.get(pair[0]).map(|value| *value as usize) else {
            return false;
        };
        let Some(child) = bones.get(pair[1]).map(|value| *value as usize) else {
            return false;
        };
        child != parent && is_descendant(child, parent, parents)
    })
}

fn has_mtoon(json: &Value, format: AvatarFormat) -> bool {
    match format {
        AvatarFormat::Vrm1 => json
            .get("materials")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|material| {
                material
                    .pointer("/extensions/VRMC_materials_mtoon")
                    .is_some()
            }),
        AvatarFormat::Vrm0 => json
            .pointer("/extensions/VRM/materialProperties")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|material| {
                material
                    .get("shader")
                    .and_then(Value::as_str)
                    .is_some_and(|shader| shader.to_ascii_lowercase().contains("mtoon"))
            }),
        AvatarFormat::Glb => false,
    }
}

fn spring_bone_counts(json: &Value, format: AvatarFormat) -> (u32, u32) {
    match format {
        AvatarFormat::Vrm1 => (
            json.pointer("/extensions/VRMC_springBone/springs")
                .and_then(Value::as_array)
                .map_or(0, |values| to_u32(values.len())),
            json.pointer("/extensions/VRMC_springBone/colliders")
                .and_then(Value::as_array)
                .map_or(0, |values| to_u32(values.len())),
        ),
        AvatarFormat::Vrm0 => (
            json.pointer("/extensions/VRM/secondaryAnimation/boneGroups")
                .and_then(Value::as_array)
                .map_or(0, |values| {
                    to_u32(
                        values
                            .iter()
                            .filter(|value| {
                                value
                                    .get("bones")
                                    .and_then(Value::as_array)
                                    .is_some_and(|bones| !bones.is_empty())
                            })
                            .count(),
                    )
                }),
            json.pointer("/extensions/VRM/secondaryAnimation/colliderGroups")
                .and_then(Value::as_array)
                .map_or(0, |values| {
                    to_u32(
                        values
                            .iter()
                            .filter(|value| {
                                value
                                    .get("colliders")
                                    .and_then(Value::as_array)
                                    .is_some_and(|colliders| !colliders.is_empty())
                            })
                            .count(),
                    )
                }),
        ),
        AvatarFormat::Glb => (0, 0),
    }
}

fn validate_skin_weights(
    gltf: &Gltf,
    reachable_nodes: &BTreeSet<usize>,
    reachable_meshes: &BTreeSet<usize>,
) -> bool {
    let blob = gltf.blob.as_deref();
    for node in gltf.nodes().filter(|node| {
        reachable_nodes.contains(&node.index())
            && node
                .mesh()
                .is_some_and(|mesh| reachable_meshes.contains(&mesh.index()))
            && node.skin().is_some()
    }) {
        let Some(mesh) = node.mesh() else { continue };
        let Some(skin) = node.skin() else { continue };
        let joint_count = skin.joints().count();
        for primitive in mesh.primitives() {
            let reader = primitive.reader(|buffer| match buffer.source() {
                gltf::buffer::Source::Bin => blob,
                gltf::buffer::Source::Uri(_) => None,
            });
            if reader.read_weights(1).is_some() || reader.read_joints(1).is_some() {
                return false;
            }
            let Some(weights) = reader.read_weights(0).map(|values| values.into_f32()) else {
                return false;
            };
            let Some(joints_values) = reader.read_joints(0).map(|values| values.into_u16()) else {
                return false;
            };
            for (weights, joints) in weights.zip(joints_values) {
                let sum = weights.iter().copied().sum::<f32>();
                if !weights
                    .iter()
                    .all(|weight| weight.is_finite() && *weight >= 0.0)
                    || !sum.is_finite()
                    || sum <= f32::EPSILON
                    || weights
                        .iter()
                        .zip(joints)
                        .any(|(weight, joint)| *weight > 0.0 && usize::from(joint) >= joint_count)
                {
                    return false;
                }
            }
        }
    }
    true
}

#[derive(Debug, Clone, Copy)]
struct TextureBudget {
    max_dimension: u32,
    decoded_bytes: u64,
}

fn inspect_texture_budget(bytes: &[u8], json: &Value) -> Result<TextureBudget, &'static str> {
    use base64::Engine as _;
    let binary = glb_binary_chunk(bytes)?;
    let buffer_views = json.get("bufferViews").and_then(Value::as_array);
    let mut budget = TextureBudget {
        max_dimension: 0,
        decoded_bytes: 0,
    };
    for image in json
        .get("images")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let owned;
        let image_bytes = if let Some(index) = image
            .get("bufferView")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
        {
            let view = buffer_views
                .and_then(|views| views.get(index))
                .ok_or("image_buffer_view_invalid")?;
            if view.get("buffer").and_then(Value::as_u64).unwrap_or(0) != 0 {
                return Err("image_buffer_invalid");
            }
            let offset = view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0) as usize;
            let length = view
                .get("byteLength")
                .and_then(Value::as_u64)
                .ok_or("image_buffer_view_invalid")? as usize;
            binary
                .get(
                    offset
                        ..offset
                            .checked_add(length)
                            .ok_or("image_buffer_view_invalid")?,
                )
                .ok_or("image_buffer_view_invalid")?
        } else if let Some(uri) = image.get("uri").and_then(Value::as_str) {
            let encoded = uri
                .split_once(',')
                .map(|(_, data)| data)
                .ok_or("image_data_uri_invalid")?;
            owned = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| "image_data_uri_invalid")?;
            &owned
        } else {
            return Err("image_source_missing");
        };
        let (width, height) = image_dimensions(image_bytes).ok_or("image_dimensions_unreadable")?;
        budget.max_dimension = budget.max_dimension.max(width).max(height);
        budget.decoded_bytes = budget.decoded_bytes.saturating_add(
            u64::from(width)
                .saturating_mul(u64::from(height))
                .saturating_mul(4),
        );
    }
    Ok(budget)
}

fn glb_binary_chunk(bytes: &[u8]) -> Result<&[u8], &'static str> {
    let json_length = usize::try_from(u32::from_le_bytes(
        bytes
            .get(12..16)
            .ok_or("invalid_glb")?
            .try_into()
            .map_err(|_| "invalid_glb")?,
    ))
    .map_err(|_| "invalid_glb")?;
    let header = 20_usize.checked_add(json_length).ok_or("invalid_glb")?;
    if header == bytes.len() {
        return Ok(&[]);
    }
    let chunk_header = bytes.get(header..header + 8).ok_or("invalid_glb")?;
    let length = usize::try_from(u32::from_le_bytes(
        chunk_header[..4].try_into().map_err(|_| "invalid_glb")?,
    ))
    .map_err(|_| "invalid_glb")?;
    if &chunk_header[4..8] != b"BIN\0" {
        return Err("invalid_glb_binary_chunk");
    }
    bytes
        .get(header + 8..header + 8 + length)
        .ok_or("invalid_glb_binary_chunk")
}

fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() >= 24 {
        return Some((
            u32::from_be_bytes(bytes[16..20].try_into().ok()?),
            u32::from_be_bytes(bytes[20..24].try_into().ok()?),
        ));
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        let mut cursor = 2;
        while cursor + 9 < bytes.len() {
            if bytes[cursor] != 0xff {
                cursor += 1;
                continue;
            }
            let marker = bytes[cursor + 1];
            cursor += 2;
            if marker == 0xd8 || marker == 0xd9 || marker == 0x01 {
                continue;
            }
            let length = usize::from(u16::from_be_bytes(
                bytes.get(cursor..cursor + 2)?.try_into().ok()?,
            ));
            if length < 2 || cursor + length > bytes.len() {
                return None;
            }
            if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
                let height = u32::from(u16::from_be_bytes(
                    bytes.get(cursor + 3..cursor + 5)?.try_into().ok()?,
                ));
                let width = u32::from(u16::from_be_bytes(
                    bytes.get(cursor + 5..cursor + 7)?.try_into().ok()?,
                ));
                return Some((width, height));
            }
            cursor += length;
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn build_adaptation_profile(
    vrm_version: AvatarFormat,
    json: &Value,
    bones: &BTreeMap<String, u32>,
    parents: &[Option<usize>],
    world_positions: &[Option<[f32; 3]>],
    expressions: Vec<AvatarExpressionBinding>,
    spring_bone_group_count: u32,
    collider_count: u32,
    bounds_min: [f64; 3],
    bounds_max: [f64; 3],
    lip_sync: LipSyncCapability,
) -> AvatarAdaptationProfile {
    let by_node: BTreeMap<usize, &str> = bones
        .iter()
        .map(|(bone, node)| (*node as usize, bone.as_str()))
        .collect();
    let rest_bones = bones
        .iter()
        .map(|(bone, node_index)| {
            let node = json
                .get("nodes")
                .and_then(Value::as_array)
                .and_then(|nodes| nodes.get(*node_index as usize));
            let local_translation =
                json_vec3(node.and_then(|node| node.get("translation")), [0.0; 3]);
            let local_rotation = json_vec4(
                node.and_then(|node| node.get("rotation")),
                [0.0, 0.0, 0.0, 1.0],
            );
            let node = *node_index as usize;
            let parent_bone = nearest_parent_bone(node, parents, &by_node).map(str::to_owned);
            let length = bones
                .iter()
                .filter(|(_, child)| {
                    nearest_parent_bone(**child as usize, parents, &by_node) == Some(bone.as_str())
                })
                .filter_map(|(_, child)| {
                    distance(
                        world_positions.get(node)?.as_ref()?,
                        world_positions.get(*child as usize)?.as_ref()?,
                    )
                })
                .fold(f32::INFINITY, f32::min);
            AvatarRestBone {
                bone: bone.clone(),
                node_index: *node_index,
                parent_bone,
                local_translation,
                local_rotation,
                length: if length.is_finite() { length } else { 0.0 },
            }
        })
        .collect();
    let height = (bounds_max[1] - bounds_min[1]).max(0.001) as f32;
    let shoulder_width = bone_distance(bones, world_positions, "left_shoulder", "right_shoulder")
        .unwrap_or(height * 0.22);
    let hip_width = bone_distance(bones, world_positions, "left_upper_leg", "right_upper_leg")
        .unwrap_or(height * 0.16);
    let segment = |from: &str, to: &str, fallback: f32| {
        bone_distance(bones, world_positions, from, to).unwrap_or(height * fallback)
    };
    let left_upper_arm_length = segment("left_upper_arm", "left_lower_arm", 0.16);
    let left_lower_arm_length = segment("left_lower_arm", "left_hand", 0.15);
    let right_upper_arm_length = segment("right_upper_arm", "right_lower_arm", 0.16);
    let right_lower_arm_length = segment("right_lower_arm", "right_hand", 0.15);
    let left_upper_leg_length = segment("left_upper_leg", "left_lower_leg", 0.24);
    let left_lower_leg_length = segment("left_lower_leg", "left_foot", 0.24);
    let right_upper_leg_length = segment("right_upper_leg", "right_lower_leg", 0.24);
    let right_lower_leg_length = segment("right_lower_leg", "right_foot", 0.24);
    let spine_length = available_chain_length(
        bones,
        world_positions,
        &["hips", "spine", "chest", "upper_chest", "neck", "head"],
    )
    .unwrap_or(height * 0.3);
    let left_hand_length = segment("left_hand", "left_middle_distal", 0.1);
    let right_hand_length = segment("right_hand", "right_middle_distal", 0.1);
    let left_foot_length = segment("left_foot", "left_toes", 0.13);
    let right_foot_length = segment("right_foot", "right_toes", 0.13);
    let proportions = AvatarBodyProportions {
        height,
        shoulder_width,
        hip_width,
        spine_length,
        left_upper_arm_length,
        left_lower_arm_length,
        right_upper_arm_length,
        right_lower_arm_length,
        left_upper_leg_length,
        left_lower_leg_length,
        right_upper_leg_length,
        right_lower_leg_length,
        left_hand_length,
        right_hand_length,
        left_foot_length,
        right_foot_length,
        foot_height: bounds_min[1] as f32,
    };
    let mut collision_capsules = vec![
        AvatarCollisionCapsule {
            bone: "hips".into(),
            radius: hip_width * 0.55,
            half_height: height * 0.08,
        },
        AvatarCollisionCapsule {
            bone: if bones.contains_key("chest") {
                "chest".into()
            } else {
                "spine".into()
            },
            radius: shoulder_width * 0.48,
            half_height: height * 0.12,
        },
        AvatarCollisionCapsule {
            bone: "head".into(),
            radius: height * 0.075,
            half_height: height * 0.06,
        },
    ];
    for (bone, length) in [
        ("left_upper_arm", left_upper_arm_length),
        ("left_lower_arm", left_lower_arm_length),
        ("right_upper_arm", right_upper_arm_length),
        ("right_lower_arm", right_lower_arm_length),
        ("left_upper_leg", left_upper_leg_length),
        ("left_lower_leg", left_lower_leg_length),
        ("right_upper_leg", right_upper_leg_length),
        ("right_lower_leg", right_lower_leg_length),
    ] {
        collision_capsules.push(AvatarCollisionCapsule {
            bone: bone.into(),
            radius: length * 0.14,
            half_height: length * 0.42,
        });
    }
    let contacts = vec![
        AvatarContactPoint {
            id: "left_sole".into(),
            bone: "left_foot".into(),
            local_position: [0.0, -height * 0.015, left_foot_length * 0.38],
            local_normal: [0.0, 1.0, 0.0],
            radius: left_foot_length * 0.28,
        },
        AvatarContactPoint {
            id: "right_sole".into(),
            bone: "right_foot".into(),
            local_position: [0.0, -height * 0.015, right_foot_length * 0.38],
            local_normal: [0.0, 1.0, 0.0],
            radius: right_foot_length * 0.28,
        },
        AvatarContactPoint {
            id: "left_palm".into(),
            bone: "left_hand".into(),
            local_position: [0.0, 0.0, left_hand_length * 0.25],
            local_normal: [0.0, 0.0, 1.0],
            radius: left_hand_length * 0.22,
        },
        AvatarContactPoint {
            id: "right_palm".into(),
            bone: "right_hand".into(),
            local_position: [0.0, 0.0, right_hand_length * 0.25],
            local_normal: [0.0, 0.0, 1.0],
            radius: right_hand_length * 0.22,
        },
        AvatarContactPoint {
            id: "head_top".into(),
            bone: "head".into(),
            local_position: [0.0, height * 0.09, 0.0],
            local_normal: [0.0, 1.0, 0.0],
            radius: height * 0.065,
        },
    ];
    AvatarAdaptationProfile {
        vrm_version,
        bones: rest_bones,
        expressions,
        look_at: AvatarLookAtProfile::default(),
        spring_bone_group_count,
        collider_count,
        joint_limits: default_joint_limits(),
        proportions,
        contacts,
        collision_capsules,
        left_knee_pole: [0.0, 0.0, 1.0],
        right_knee_pole: [0.0, 0.0, 1.0],
        left_elbow_pole: [0.0, 0.0, 1.0],
        right_elbow_pole: [0.0, 0.0, 1.0],
        lip_sync,
        has_finger_bones: required_finger_bones()
            .iter()
            .all(|bone| bones.contains_key(*bone)),
        has_toe_bones: bones.contains_key("left_toes") && bones.contains_key("right_toes"),
    }
}

fn json_vec3(value: Option<&Value>, fallback: [f32; 3]) -> [f32; 3] {
    let Some(values) = value
        .and_then(Value::as_array)
        .filter(|values| values.len() == 3)
    else {
        return fallback;
    };
    std::array::from_fn(|index| {
        values[index]
            .as_f64()
            .map_or(fallback[index], |value| value as f32)
    })
}

fn json_vec4(value: Option<&Value>, fallback: [f32; 4]) -> [f32; 4] {
    let Some(values) = value
        .and_then(Value::as_array)
        .filter(|values| values.len() == 4)
    else {
        return fallback;
    };
    std::array::from_fn(|index| {
        values[index]
            .as_f64()
            .map_or(fallback[index], |value| value as f32)
    })
}

fn nearest_parent_bone<'a>(
    mut node: usize,
    parents: &[Option<usize>],
    by_node: &'a BTreeMap<usize, &str>,
) -> Option<&'a str> {
    let mut remaining = parents.len();
    while remaining > 0 {
        node = parents.get(node).copied().flatten()?;
        if let Some(bone) = by_node.get(&node) {
            return Some(*bone);
        }
        remaining -= 1;
    }
    None
}

fn distance(left: &[f32; 3], right: &[f32; 3]) -> Option<f32> {
    let value = ((left[0] - right[0]).powi(2)
        + (left[1] - right[1]).powi(2)
        + (left[2] - right[2]).powi(2))
    .sqrt();
    value.is_finite().then_some(value)
}

fn bone_distance(
    bones: &BTreeMap<String, u32>,
    world: &[Option<[f32; 3]>],
    left: &str,
    right: &str,
) -> Option<f32> {
    distance(
        world.get(*bones.get(left)? as usize)?.as_ref()?,
        world.get(*bones.get(right)? as usize)?.as_ref()?,
    )
}

fn available_chain_length(
    bones: &BTreeMap<String, u32>,
    world: &[Option<[f32; 3]>],
    chain: &[&str],
) -> Option<f32> {
    let present = chain
        .iter()
        .filter(|bone| bones.contains_key(**bone))
        .copied()
        .collect::<Vec<_>>();
    if present.len() < 2 {
        return None;
    }
    present.windows(2).try_fold(0.0, |sum, pair| {
        bone_distance(bones, world, pair[0], pair[1]).map(|length| sum + length)
    })
}

fn default_joint_limits() -> Vec<AvatarJointLimit> {
    [
        ("head", 45.0, -55.0, 55.0),
        ("left_upper_arm", 95.0, -65.0, 85.0),
        ("right_upper_arm", 95.0, -85.0, 65.0),
        ("left_lower_arm", 145.0, -10.0, 80.0),
        ("right_lower_arm", 145.0, -80.0, 10.0),
        ("left_upper_leg", 80.0, -35.0, 45.0),
        ("right_upper_leg", 80.0, -45.0, 35.0),
        ("left_lower_leg", 150.0, 0.0, 5.0),
        ("right_lower_leg", 150.0, -5.0, 0.0),
    ]
    .into_iter()
    .map(
        |(bone, swing_degrees, twist_min_degrees, twist_max_degrees)| AvatarJointLimit {
            bone: bone.to_owned(),
            swing_degrees,
            twist_min_degrees,
            twist_max_degrees,
        },
    )
    .collect()
}

fn read_json_chunk(bytes: &[u8]) -> Result<Value, &'static str> {
    if bytes.len() < 20 || &bytes[0..4] != b"glTF" {
        return Err("invalid_glb");
    }
    if u32::from_le_bytes(bytes[4..8].try_into().map_err(|_| "invalid_glb")?) != 2 {
        return Err("unsupported_glb_version");
    }
    let declared = u32::from_le_bytes(bytes[8..12].try_into().map_err(|_| "invalid_glb")?);
    if usize::try_from(declared).map_err(|_| "invalid_glb")? != bytes.len() {
        return Err("glb_length_mismatch");
    }
    let json_length = u32::from_le_bytes(bytes[12..16].try_into().map_err(|_| "invalid_glb")?);
    let chunk_type = u32::from_le_bytes(bytes[16..20].try_into().map_err(|_| "invalid_glb")?);
    let end = 20_usize
        .checked_add(usize::try_from(json_length).map_err(|_| "invalid_glb")?)
        .ok_or("invalid_glb")?;
    if chunk_type != 0x4E4F_534A || end > bytes.len() {
        return Err("invalid_glb");
    }
    serde_json::from_slice(&bytes[20..end]).map_err(|_| "invalid_glb_json")
}

fn reject_external_resources(json: &Value) -> Result<(), &'static str> {
    for collection in ["buffers", "images"] {
        for item in json
            .get(collection)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if item
                .get("uri")
                .and_then(Value::as_str)
                .is_some_and(|uri| !uri.starts_with("data:"))
            {
                return Err("external_resource");
            }
        }
    }
    Ok(())
}

fn reject_unsupported_extensions(json: &Value) -> Result<(), &'static str> {
    const SUPPORTED_REQUIRED: [&str; 8] = [
        "VRM",
        "VRMC_vrm",
        "VRMC_materials_mtoon",
        "VRMC_springBone",
        "VRMC_node_constraint",
        "KHR_materials_unlit",
        "KHR_texture_transform",
        "KHR_mesh_quantization",
    ];
    if json
        .get("extensionsRequired")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(|extension| !SUPPORTED_REQUIRED.contains(&extension))
    {
        return Err("unsupported_required_extension");
    }
    Ok(())
}

fn validate_embedded_resource_bounds(bytes: &[u8], json: &Value) -> Result<(), &'static str> {
    let binary_length = glb_binary_chunk_length(bytes)?;
    let buffers = json
        .get("buffers")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    for (index, buffer) in buffers.iter().enumerate() {
        let declared = buffer
            .get("byteLength")
            .and_then(Value::as_u64)
            .ok_or("resource_out_of_bounds")?;
        if buffer.get("uri").is_none() && (index != 0 || declared > binary_length as u64) {
            return Err("resource_out_of_bounds");
        }
    }
    for view in json
        .get("bufferViews")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let buffer_index = view
            .get("buffer")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("resource_out_of_bounds")?;
        let buffer_length = buffers
            .get(buffer_index)
            .and_then(|buffer| buffer.get("byteLength"))
            .and_then(Value::as_u64)
            .ok_or("resource_out_of_bounds")?;
        let offset = view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0);
        let length = view
            .get("byteLength")
            .and_then(Value::as_u64)
            .ok_or("resource_out_of_bounds")?;
        if offset
            .checked_add(length)
            .is_none_or(|end| end > buffer_length)
        {
            return Err("resource_out_of_bounds");
        }
    }
    Ok(())
}

fn glb_binary_chunk_length(bytes: &[u8]) -> Result<usize, &'static str> {
    let json_length = u32::from_le_bytes(
        bytes
            .get(12..16)
            .ok_or("invalid_glb")?
            .try_into()
            .map_err(|_| "invalid_glb")?,
    );
    let mut cursor = 20_usize
        .checked_add(usize::try_from(json_length).map_err(|_| "invalid_glb")?)
        .ok_or("invalid_glb")?;
    while cursor < bytes.len() {
        let header_end = cursor.checked_add(8).ok_or("invalid_glb")?;
        let header = bytes.get(cursor..header_end).ok_or("invalid_glb")?;
        let length = usize::try_from(u32::from_le_bytes(
            header[0..4].try_into().map_err(|_| "invalid_glb")?,
        ))
        .map_err(|_| "invalid_glb")?;
        let kind = u32::from_le_bytes(header[4..8].try_into().map_err(|_| "invalid_glb")?);
        let end = header_end.checked_add(length).ok_or("invalid_glb")?;
        if end > bytes.len() {
            return Err("resource_out_of_bounds");
        }
        if kind == 0x004E_4942 {
            return Ok(length);
        }
        cursor = end;
    }
    Ok(0)
}

fn reachable_scene_content(gltf: &Gltf) -> (BTreeSet<usize>, BTreeSet<usize>) {
    fn visit(node: gltf::Node<'_>, nodes: &mut BTreeSet<usize>, meshes: &mut BTreeSet<usize>) {
        if !nodes.insert(node.index()) {
            return;
        }
        if let Some(mesh) = node.mesh() {
            meshes.insert(mesh.index());
        }
        for child in node.children() {
            visit(child, nodes, meshes);
        }
    }

    let mut nodes = BTreeSet::new();
    let mut meshes = BTreeSet::new();
    for scene in gltf.scenes() {
        for node in scene.nodes() {
            visit(node, &mut nodes, &mut meshes);
        }
    }
    (nodes, meshes)
}

fn accessor_bounds(json: &Value, index: usize) -> Option<([f64; 3], [f64; 3])> {
    let accessor = json.get("accessors")?.as_array()?.get(index)?;
    let minimum = vec3(accessor.get("min")?)?;
    let maximum = vec3(accessor.get("max")?)?;
    minimum
        .iter()
        .chain(maximum.iter())
        .all(|value| value.is_finite())
        .then_some((minimum, maximum))
}

fn vec3(value: &Value) -> Option<[f64; 3]> {
    let values = value.as_array()?;
    (values.len() == 3).then(|| {
        [
            values[0].as_f64()?,
            values[1].as_f64()?,
            values[2].as_f64()?,
        ]
        .into()
    })?
}

fn detect_format(json: &Value) -> AvatarFormat {
    let extensions = json.get("extensions").and_then(Value::as_object);
    if extensions.is_some_and(|value| value.contains_key("VRMC_vrm")) {
        AvatarFormat::Vrm1
    } else if extensions.is_some_and(|value| value.contains_key("VRM")) {
        AvatarFormat::Vrm0
    } else {
        AvatarFormat::Glb
    }
}

fn explicit_humanoid_bones(json: &Value, format: AvatarFormat) -> BTreeMap<String, u32> {
    let mut bones = BTreeMap::new();
    match format {
        AvatarFormat::Vrm1 => {
            if let Some(values) = json
                .pointer("/extensions/VRMC_vrm/humanoid/humanBones")
                .and_then(Value::as_object)
            {
                for (bone, binding) in values {
                    if let Some(node) = binding.get("node").and_then(Value::as_u64)
                        && let Ok(node) = u32::try_from(node)
                    {
                        bones.insert(canonical_bone_name(bone), node);
                    }
                }
            }
        }
        AvatarFormat::Vrm0 => {
            for binding in json
                .pointer("/extensions/VRM/humanoid/humanBones")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let (Some(bone), Some(node)) = (
                    binding.get("bone").and_then(Value::as_str),
                    binding.get("node").and_then(Value::as_u64),
                ) && let Ok(node) = u32::try_from(node)
                {
                    bones.insert(canonical_bone_name(bone), node);
                }
            }
        }
        AvatarFormat::Glb => {}
    }
    if let Some(values) = json
        .pointer("/asset/extras/hachimiAvatar/humanoid")
        .and_then(Value::as_object)
    {
        for (bone, binding) in values {
            let node = binding.as_u64().or_else(|| binding.get("node")?.as_u64());
            if let Some(node) = node.and_then(|value| u32::try_from(value).ok()) {
                bones.insert(canonical_bone_name(bone), node);
            }
        }
    }
    bones
}

fn node_world_positions(gltf: &Gltf, parents: &[Option<usize>]) -> Vec<Option<[f32; 3]>> {
    let local: Vec<_> = gltf.nodes().map(|node| node.transform().matrix()).collect();
    (0..local.len())
        .map(|index| {
            let mut chain = Vec::new();
            let mut cursor = Some(index);
            let mut remaining = local.len();
            while let Some(node) = cursor {
                if remaining == 0 || chain.contains(&node) {
                    return None;
                }
                chain.push(node);
                cursor = parents.get(node).copied().flatten();
                remaining -= 1;
            }
            let world = chain
                .into_iter()
                .rev()
                .fold(identity_matrix(), |parent, node| {
                    multiply_matrices(parent, local[node])
                });
            let position = [world[3][0], world[3][1], world[3][2]];
            position
                .iter()
                .all(|value| value.is_finite())
                .then_some(position)
        })
        .collect()
}

const fn identity_matrix() -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn multiply_matrices(left: [[f32; 4]; 4], right: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut result = [[0.0; 4]; 4];
    for column in 0..4 {
        for row in 0..4 {
            result[column][row] = (0..4)
                .map(|index| left[index][row] * right[column][index])
                .sum();
        }
    }
    result
}

fn parent_indices(json: &Value, node_count: usize) -> Vec<Option<usize>> {
    let mut parents = vec![None; node_count];
    for (parent, node) in json
        .get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        for child in node
            .get("children")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .filter_map(|value| usize::try_from(value).ok())
        {
            if child < parents.len() {
                parents[child] = Some(parent);
            }
        }
    }
    parents
}

fn is_descendant(mut child: usize, ancestor: usize, parents: &[Option<usize>]) -> bool {
    let mut remaining = parents.len();
    while remaining > 0 {
        if child == ancestor {
            return true;
        }
        let Some(parent) = parents.get(child).copied().flatten() else {
            return false;
        };
        child = parent;
        remaining -= 1;
    }
    false
}

fn expression_bindings(json: &Value, format: AvatarFormat) -> Vec<AvatarExpressionBinding> {
    let mut bindings = standard_expression_bindings(json, format);
    bindings.extend(generic_expression_bindings(json));
    bindings
}

fn standard_expression_bindings(
    json: &Value,
    format: AvatarFormat,
) -> Vec<AvatarExpressionBinding> {
    let mut bindings = Vec::new();
    if format == AvatarFormat::Vrm1 {
        if let Some(presets) = json
            .pointer("/extensions/VRMC_vrm/expressions/preset")
            .and_then(Value::as_object)
        {
            for (name, expression) in presets {
                let expression_name = canonical_expression_name(name);
                for binding in expression
                    .get("morphTargetBinds")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    push_expression_binding(&mut bindings, &expression_name, binding, "node");
                }
            }
        }
    } else if format == AvatarFormat::Vrm0 {
        let mesh_nodes = mesh_node_indices(json);
        for group in json
            .pointer("/extensions/VRM/blendShapeMaster/blendShapeGroups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let preset = group.get("presetName").and_then(Value::as_str);
            let Some(name) = preset
                .filter(|name| !name.eq_ignore_ascii_case("unknown") && !name.is_empty())
                .or_else(|| group.get("name").and_then(Value::as_str))
            else {
                continue;
            };
            let expression_name = canonical_expression_name(name);
            for binding in group
                .get("binds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(mesh) = binding.get("mesh").and_then(Value::as_u64) else {
                    continue;
                };
                let Some(node) = mesh_nodes.get(&(mesh as usize)).copied() else {
                    continue;
                };
                if let Some(index) = binding.get("index").and_then(Value::as_u64)
                    && let (Ok(node_index), Ok(morph_index)) =
                        (u32::try_from(node), u32::try_from(index))
                {
                    bindings.push(AvatarExpressionBinding {
                        expression: expression_name.clone(),
                        node_index,
                        morph_index,
                    });
                }
            }
        }
    }
    bindings
}

fn declared_standard_expression_names(json: &Value, format: AvatarFormat) -> BTreeSet<String> {
    match format {
        AvatarFormat::Vrm1 => json
            .pointer("/extensions/VRMC_vrm/expressions/preset")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|presets| presets.keys())
            .map(|name| canonical_expression_name(name))
            .collect(),
        AvatarFormat::Vrm0 => json
            .pointer("/extensions/VRM/blendShapeMaster/blendShapeGroups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|group| {
                let preset = group.get("presetName").and_then(Value::as_str);
                preset
                    .filter(|name| !name.eq_ignore_ascii_case("unknown") && !name.is_empty())
                    .or_else(|| group.get("name").and_then(Value::as_str))
            })
            .map(canonical_expression_name)
            .collect(),
        AvatarFormat::Glb => BTreeSet::new(),
    }
}

fn generic_expression_bindings(json: &Value) -> Vec<AvatarExpressionBinding> {
    let mut result = Vec::new();
    let mesh_nodes = mesh_node_indices(json);
    for (mesh_index, mesh) in json
        .get("meshes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let names = mesh
            .pointer("/extras/targetNames")
            .and_then(Value::as_array)
            .or_else(|| mesh.get("targetNames").and_then(Value::as_array));
        let Some(node_index) = mesh_nodes.get(&mesh_index).copied() else {
            continue;
        };
        for (morph_index, name) in names.into_iter().flatten().enumerate() {
            let Some(name) = name.as_str() else { continue };
            let expression = canonical_expression_name(name);
            if is_known_expression(&expression) {
                result.push(AvatarExpressionBinding {
                    expression,
                    node_index: to_u32(node_index),
                    morph_index: to_u32(morph_index),
                });
            }
        }
    }
    result
}

fn push_expression_binding(
    output: &mut Vec<AvatarExpressionBinding>,
    expression: &str,
    binding: &Value,
    node_key: &str,
) {
    let Some(node) = binding.get(node_key).and_then(Value::as_u64) else {
        return;
    };
    let Some(index) = binding.get("index").and_then(Value::as_u64) else {
        return;
    };
    if let (Ok(node_index), Ok(morph_index)) = (u32::try_from(node), u32::try_from(index)) {
        output.push(AvatarExpressionBinding {
            expression: expression.to_owned(),
            node_index,
            morph_index,
        });
    }
}

fn mesh_node_indices(json: &Value) -> BTreeMap<usize, usize> {
    json.get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(node_index, node)| {
            Some((
                usize::try_from(node.get("mesh")?.as_u64()?).ok()?,
                node_index,
            ))
        })
        .collect()
}

fn has_path(json: &Value, path: &[&str]) -> bool {
    path.iter()
        .try_fold(json, |value, key| value.get(*key))
        .is_some()
}

fn canonical_bone_name(name: &str) -> String {
    let normalized = normalize_name(name);
    let aliases = [
        ("upperchest", "upper_chest"),
        ("leftshoulder", "left_shoulder"),
        ("leftupperarm", "left_upper_arm"),
        ("leftlowerarm", "left_lower_arm"),
        ("lefthand", "left_hand"),
        ("rightupperarm", "right_upper_arm"),
        ("rightlowerarm", "right_lower_arm"),
        ("righthand", "right_hand"),
        ("leftupperleg", "left_upper_leg"),
        ("leftlowerleg", "left_lower_leg"),
        ("leftfoot", "left_foot"),
        ("lefttoes", "left_toes"),
        ("lefteye", "left_eye"),
        ("rightshoulder", "right_shoulder"),
        ("rightupperleg", "right_upper_leg"),
        ("rightlowerleg", "right_lower_leg"),
        ("rightfoot", "right_foot"),
        ("righttoes", "right_toes"),
        ("righteye", "right_eye"),
        ("leftthumbproximal", "left_thumb_proximal"),
        ("leftthumbintermediate", "left_thumb_intermediate"),
        ("leftthumbdistal", "left_thumb_distal"),
        ("leftindexproximal", "left_index_proximal"),
        ("leftindexintermediate", "left_index_intermediate"),
        ("leftindexdistal", "left_index_distal"),
        ("leftmiddleproximal", "left_middle_proximal"),
        ("leftmiddleintermediate", "left_middle_intermediate"),
        ("leftmiddledistal", "left_middle_distal"),
        ("leftringproximal", "left_ring_proximal"),
        ("leftringintermediate", "left_ring_intermediate"),
        ("leftringdistal", "left_ring_distal"),
        ("leftlittleproximal", "left_little_proximal"),
        ("leftlittleintermediate", "left_little_intermediate"),
        ("leftlittledistal", "left_little_distal"),
        ("rightthumbproximal", "right_thumb_proximal"),
        ("rightthumbintermediate", "right_thumb_intermediate"),
        ("rightthumbdistal", "right_thumb_distal"),
        ("rightindexproximal", "right_index_proximal"),
        ("rightindexintermediate", "right_index_intermediate"),
        ("rightindexdistal", "right_index_distal"),
        ("rightmiddleproximal", "right_middle_proximal"),
        ("rightmiddleintermediate", "right_middle_intermediate"),
        ("rightmiddledistal", "right_middle_distal"),
        ("rightringproximal", "right_ring_proximal"),
        ("rightringintermediate", "right_ring_intermediate"),
        ("rightringdistal", "right_ring_distal"),
        ("rightlittleproximal", "right_little_proximal"),
        ("rightlittleintermediate", "right_little_intermediate"),
        ("rightlittledistal", "right_little_distal"),
    ];
    aliases
        .iter()
        .find_map(|(alias, canonical)| (normalized == *alias).then_some(*canonical))
        .unwrap_or(name)
        .to_owned()
}

fn canonical_expression_name(name: &str) -> String {
    match normalize_name(name).as_str() {
        "a" | "aa" | "visemeaa" | "moutha" => "aa",
        "i" | "ih" | "visemeih" | "mouthi" => "ih",
        "u" | "ou" | "visemeou" | "mouthu" => "ou",
        "e" | "ee" | "visemeee" | "mouthe" => "ee",
        "o" | "oh" | "visemeoh" | "moutho" => "oh",
        "blink" | "blinkboth" => "blink",
        "blinkleft" | "blinkl" => "blink_left",
        "blinkright" | "blinkr" => "blink_right",
        "neutral" => "neutral",
        "joy" | "happy" => "happy",
        "fun" | "relaxed" => "relaxed",
        "sorrow" | "sad" => "sad",
        "angry" => "angry",
        "surprise" | "surprised" => "surprised",
        other => other,
    }
    .to_owned()
}

fn is_known_expression(name: &str) -> bool {
    matches!(
        name,
        "aa" | "ih"
            | "ou"
            | "ee"
            | "oh"
            | "blink"
            | "blink_left"
            | "blink_right"
            | "neutral"
            | "happy"
            | "relaxed"
            | "sad"
            | "angry"
            | "surprised"
    )
}

fn normalize_name(name: &str) -> String {
    name.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn validate_name(name: &str) -> Result<String, AvatarError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(AvatarError::InvalidName);
    }
    Ok(name.to_owned())
}

fn incompatible_assessment(code: &str) -> AvatarAssessment {
    AvatarAssessment {
        compatibility: AvatarCompatibility::Incompatible,
        detector_version: DETECTOR_VERSION,
        capabilities: Vec::new(),
        statistics: AvatarStatistics::default(),
        requirements: Vec::new(),
        issues: vec![issue(code, AvatarIssueSeverity::Error)],
    }
}

fn issue(code: &str, severity: AvatarIssueSeverity) -> AvatarIssue {
    AvatarIssue {
        code: code.to_owned(),
        severity,
    }
}

fn newest_compatible_id(entries: &[AvatarEntry]) -> Option<String> {
    entries
        .iter()
        .filter(|entry| entry.assessment.compatibility == AvatarCompatibility::RuntimeReady)
        .max_by_key(|entry| entry.imported_at.parse::<u128>().unwrap_or_default())
        .map(|entry| entry.id.clone())
}

fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod runtime_ready_tests {
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
                .any(|requirement| requirement.requirement == "complete_humanoid"
                    && !requirement.passed)
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
        let saved: Value = serde_json::from_slice(
            &fs::read(root.join(CATALOG_FILE_NAME)).expect("saved V4 catalog"),
        )
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
}
