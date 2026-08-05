//! Validated VRMA 1.0 catalog with immutable bundled motions and content-addressed user imports.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use atomic_write_file::AtomicWriteFile;
use hachimi_core::FeatureAvailability;
use hachimi_protocol::{
    BehaviorChannel, InteractionMotionBinding, InteractionMotionBindingUpdateRequest,
    InteractionRegion, MotionAssetBindingsClearRequest, MotionCatalogEntry, MotionCatalogSnapshot,
    MotionCategory, MotionEnabledUpdateRequest, MotionImportCommitRequest, MotionImportInspection,
    MotionMetadataUpdateRequest, MotionSource,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

const MAX_VRMA_BYTES: u64 = 64 * 1024 * 1024;
const STATE_FILE: &str = "catalog.json";
const USER_BLOB_FILE: &str = "source.vrma";
const BUILTIN_SCHEMA_VERSION: u32 = 1;
const STATE_SCHEMA_VERSION: u32 = 3;

#[must_use]
pub const fn availability() -> FeatureAvailability {
    FeatureAvailability::Available
}

#[derive(Debug, Error)]
pub enum MotionError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("motion catalog error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("请选择正式 VRMA 1.0 文件")]
    InvalidExtension,
    #[error("VRMA 文件不能超过 64MB")]
    TooLarge,
    #[error("VRMA 文件不是有效的 glTF 2.0 binary")]
    InvalidGlb,
    #[error("VRMA 文件缺少 VRMC_vrm_animation 1.0")]
    InvalidVrma,
    #[error("VRMA 必须包含且只包含一个动画")]
    InvalidAnimationCount,
    #[error("VRMA 没有可用的 Humanoid 动画轨道")]
    MissingHumanoidMotion,
    #[error("动作名称长度必须为 1–80 个字符")]
    InvalidName,
    #[error("动作描述不能超过 500 个字符")]
    InvalidDescription,
    #[error("找不到指定动作")]
    NotFound,
    #[error("内置动作不能修改或删除")]
    Protected,
    #[error("互动动作绑定无效")]
    InvalidBinding,
    #[error("内置动作目录无效: {0}")]
    InvalidBuiltin(String),
}

#[derive(Debug, Clone)]
pub struct InspectedMotion {
    pub original_file_name: String,
    pub size_bytes: u32,
    pub sha256: String,
    pub duration_ms: u32,
    pub animated_bones: Vec<String>,
    pub channels: Vec<BehaviorChannel>,
    pub finger_bone_count: u16,
    pub has_expression: bool,
    pub has_look_at: bool,
    pub warnings: Vec<String>,
    modified_millis: u128,
}

impl InspectedMotion {
    #[must_use]
    pub fn view(&self, token: Option<String>) -> MotionImportInspection {
        MotionImportInspection {
            token,
            original_file_name: self.original_file_name.clone(),
            size_bytes: self.size_bytes,
            sha256: self.sha256.clone(),
            duration_ms: self.duration_ms,
            animated_bones: self.animated_bones.clone(),
            finger_bone_count: self.finger_bone_count,
            has_expression: self.has_expression,
            has_look_at: self.has_look_at,
            warnings: self.warnings.clone(),
        }
    }

    #[must_use]
    pub fn source_is_unchanged(&self, other: &Self) -> bool {
        self.size_bytes == other.size_bytes
            && self.sha256 == other.sha256
            && self.modified_millis == other.modified_millis
    }
}

#[derive(Debug, Clone)]
pub struct MotionAsset {
    pub entry: MotionCatalogEntry,
    pub path: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BuiltinCatalogDocument {
    schema_version: u32,
    spec_version: String,
    entries: Vec<MotionCatalogEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MotionStateDocument {
    schema_version: u32,
    entries: Vec<MotionCatalogEntry>,
    bindings: Vec<InteractionMotionBinding>,
    #[serde(default)]
    disabled_motion_ids: BTreeSet<String>,
}

#[derive(Debug)]
pub struct MotionCatalog {
    root: PathBuf,
    builtin_root: PathBuf,
    builtin_entries: Vec<MotionCatalogEntry>,
    state: MotionStateDocument,
}

impl MotionCatalog {
    pub fn load(
        root: impl Into<PathBuf>,
        builtin_catalog: impl AsRef<Path>,
    ) -> Result<Self, MotionError> {
        let root = root.into();
        fs::create_dir_all(root.join("blobs"))?;
        let builtin_catalog = builtin_catalog.as_ref();
        let builtin_root = builtin_catalog
            .parent()
            .ok_or_else(|| MotionError::InvalidBuiltin("catalog has no parent".into()))?
            .join("builtin");
        let builtins: BuiltinCatalogDocument = serde_json::from_slice(&fs::read(builtin_catalog)?)?;
        if builtins.schema_version != BUILTIN_SCHEMA_VERSION || builtins.spec_version != "1.0" {
            return Err(MotionError::InvalidBuiltin(
                "unsupported catalog version".into(),
            ));
        }
        validate_builtin_entries(&builtin_root, &builtins.entries)?;
        let state_path = root.join(STATE_FILE);
        let state_exists = state_path.is_file();
        let state = if state_exists {
            let state: MotionStateDocument = serde_json::from_slice(&fs::read(&state_path)?)?;
            if state.schema_version != STATE_SCHEMA_VERSION {
                return Err(MotionError::InvalidBinding);
            }
            state
        } else {
            MotionStateDocument::default()
        };
        let mut catalog = Self {
            root,
            builtin_root,
            builtin_entries: builtins.entries,
            state,
        };
        catalog.state.schema_version = STATE_SCHEMA_VERSION;
        catalog.state.entries.retain(|entry| {
            entry.source == MotionSource::User
                && catalog
                    .root
                    .join("blobs")
                    .join(&entry.sha256)
                    .join(USER_BLOB_FILE)
                    .is_file()
        });
        if !state_exists {
            catalog.state.bindings = default_bindings(&catalog.all_entries());
        }
        catalog.sanitize_bindings();
        catalog.save_state()?;
        Ok(catalog)
    }

    /// Opens the user catalog without bundled motions so the desktop can keep
    /// running when a packaged catalog is missing or invalid.
    pub fn load_degraded(root: impl Into<PathBuf>) -> Result<Self, MotionError> {
        let root = root.into();
        fs::create_dir_all(root.join("blobs"))?;
        let state_path = root.join(STATE_FILE);
        let state_exists = state_path.is_file();
        let state = if state_exists {
            let state: MotionStateDocument = serde_json::from_slice(&fs::read(&state_path)?)?;
            if state.schema_version != STATE_SCHEMA_VERSION {
                return Err(MotionError::InvalidBinding);
            }
            state
        } else {
            MotionStateDocument::default()
        };
        let mut catalog = Self {
            root,
            builtin_root: PathBuf::new(),
            builtin_entries: Vec::new(),
            state,
        };
        catalog.state.schema_version = STATE_SCHEMA_VERSION;
        catalog.state.entries.retain(|entry| {
            entry.source == MotionSource::User
                && catalog
                    .root
                    .join("blobs")
                    .join(&entry.sha256)
                    .join(USER_BLOB_FILE)
                    .is_file()
        });
        catalog.sanitize_bindings();
        catalog.save_state()?;
        Ok(catalog)
    }

    #[must_use]
    pub fn snapshot(&self) -> MotionCatalogSnapshot {
        MotionCatalogSnapshot {
            entries: self.all_entries(),
            bindings: self.state.bindings.clone(),
            disabled_motion_ids: self.state.disabled_motion_ids.iter().cloned().collect(),
        }
    }

    #[must_use]
    pub fn asset_for(&self, id: &str) -> Option<MotionAsset> {
        if let Some(entry) = self.builtin_entries.iter().find(|entry| entry.id == id) {
            let path = self.builtin_root.join(&entry.file_name);
            return path.is_file().then(|| MotionAsset {
                entry: entry.clone(),
                path,
            });
        }
        let entry = self
            .state
            .entries
            .iter()
            .find(|entry| entry.id == id)?
            .clone();
        let path = self
            .root
            .join("blobs")
            .join(&entry.sha256)
            .join(USER_BLOB_FILE);
        path.is_file().then_some(MotionAsset { entry, path })
    }

    pub fn import_inspected(
        &mut self,
        source: &Path,
        inspection: &InspectedMotion,
        request: &MotionImportCommitRequest,
    ) -> Result<MotionCatalogSnapshot, MotionError> {
        let name = validate_name(&request.name)?;
        let description = validate_description(&request.description)?;
        let refreshed = inspect_motion(source)?;
        if !inspection.source_is_unchanged(&refreshed) {
            return Err(MotionError::InvalidVrma);
        }
        let destination_dir = self.root.join("blobs").join(&inspection.sha256);
        let destination = destination_dir.join(USER_BLOB_FILE);
        let mut created_blob = false;
        if !destination.exists() {
            fs::create_dir_all(&destination_dir)?;
            let temporary = self.root.join(format!(".import-{}.tmp", Uuid::new_v4()));
            fs::copy(source, &temporary)?;
            fs::rename(temporary, &destination)?;
            created_blob = true;
        }
        let id = format!("user.{}", Uuid::new_v4());
        let entry = MotionCatalogEntry {
            id,
            source: MotionSource::User,
            protected: false,
            name: name.clone(),
            name_zh: name,
            description: description.clone(),
            description_zh: description,
            file_name: USER_BLOB_FILE.into(),
            sha256: inspection.sha256.clone(),
            size_bytes: inspection.size_bytes,
            duration_ms: inspection.duration_ms,
            category: request.category,
            tags: vec![category_tag(request.category).into(), "user".into()],
            playback_mode: request.playback_mode,
            root_mode: request.root_mode,
            channels: inspection.channels.clone(),
            animated_bones: inspection.animated_bones.clone(),
            finger_bone_count: inspection.finger_bone_count,
            has_finger_motion: inspection.finger_bone_count > 0,
            has_expression: inspection.has_expression,
            has_look_at: inspection.has_look_at,
            mirrorable: true,
            transition_in_ms: 220,
            transition_out_ms: 260,
            source_project: "User".into(),
            source_paths: vec![inspection.original_file_name.clone()],
            warnings: inspection.warnings.clone(),
        };
        let mut next = self.state.clone();
        let new_id = entry.id.clone();
        next.entries.push(entry);
        if let Some(region) = request.interaction_region {
            replace_binding(&mut next.bindings, default_binding_values(region, new_id));
        }
        if let Err(error) = self.commit_state(next) {
            if created_blob {
                let _ = fs::remove_dir_all(destination_dir);
            }
            return Err(error);
        }
        Ok(self.snapshot())
    }

    pub fn update_metadata(
        &mut self,
        request: &MotionMetadataUpdateRequest,
    ) -> Result<MotionCatalogSnapshot, MotionError> {
        if self
            .builtin_entries
            .iter()
            .any(|entry| entry.id == request.id)
        {
            return Err(MotionError::Protected);
        }
        let mut next = self.state.clone();
        let entry = next
            .entries
            .iter_mut()
            .find(|entry| entry.id == request.id)
            .ok_or(MotionError::NotFound)?;
        entry.name = validate_name(&request.name)?;
        entry.name_zh = entry.name.clone();
        entry.description = validate_description(&request.description)?;
        entry.description_zh = entry.description.clone();
        entry.category = request.category;
        entry.playback_mode = request.playback_mode;
        entry.root_mode = request.root_mode;
        entry.tags.retain(|tag| !is_category_tag(tag));
        entry.tags.insert(0, category_tag(request.category).into());
        self.commit_state(next)?;
        Ok(self.snapshot())
    }

    pub fn delete_user(&mut self, id: &str) -> Result<MotionCatalogSnapshot, MotionError> {
        if self.builtin_entries.iter().any(|entry| entry.id == id) {
            return Err(MotionError::Protected);
        }
        let index = self
            .state
            .entries
            .iter()
            .position(|entry| entry.id == id)
            .ok_or(MotionError::NotFound)?;
        let mut next = self.state.clone();
        let removed = next.entries.remove(index);
        next.disabled_motion_ids.remove(id);
        let affected_regions: Vec<_> = next
            .bindings
            .iter()
            .filter(|binding| binding.motion_id == id)
            .map(|binding| binding.region)
            .collect();
        next.bindings.retain(|binding| binding.motion_id != id);
        let defaults = default_bindings(&self.builtin_entries);
        for region in affected_regions {
            if let Some(binding) = defaults.iter().find(|binding| binding.region == region) {
                replace_binding(&mut next.bindings, binding.clone());
            }
        }
        self.commit_state(next)?;
        if !self
            .state
            .entries
            .iter()
            .any(|entry| entry.sha256 == removed.sha256)
        {
            let _ = fs::remove_dir_all(self.root.join("blobs").join(removed.sha256));
        }
        Ok(self.snapshot())
    }

    pub fn update_binding(
        &mut self,
        request: &InteractionMotionBindingUpdateRequest,
    ) -> Result<MotionCatalogSnapshot, MotionError> {
        let mut next = self.state.clone();
        if let Some(motion_id) = request.motion_id.as_ref() {
            if !self
                .all_entries()
                .iter()
                .any(|entry| entry.id == *motion_id)
            {
                return Err(MotionError::InvalidBinding);
            }
            let already_bound_to_region = next
                .bindings
                .iter()
                .any(|binding| binding.region == request.region && binding.motion_id == *motion_id);
            if next.disabled_motion_ids.contains(motion_id) && !already_bound_to_region {
                return Err(MotionError::InvalidBinding);
            }
            let current = next
                .bindings
                .iter()
                .find(|binding| binding.region == request.region)
                .map(|binding| (binding.cooldown_ms, binding.mirror_by_side));
            let defaults = default_binding_values(request.region, motion_id.clone());
            replace_binding(
                &mut next.bindings,
                InteractionMotionBinding {
                    region: request.region,
                    motion_id: motion_id.clone(),
                    cooldown_ms: request
                        .cooldown_ms
                        .or(current.map(|binding| binding.0))
                        .unwrap_or(defaults.cooldown_ms),
                    mirror_by_side: request
                        .mirror_by_side
                        .or(current.map(|binding| binding.1))
                        .unwrap_or(defaults.mirror_by_side),
                },
            );
        } else {
            next.bindings
                .retain(|binding| binding.region != request.region);
        }
        validate_bindings(&next.bindings, &self.all_entries())?;
        self.commit_state(next)?;
        Ok(self.snapshot())
    }

    pub fn clear_motion_bindings(
        &mut self,
        request: &MotionAssetBindingsClearRequest,
    ) -> Result<MotionCatalogSnapshot, MotionError> {
        if !self
            .all_entries()
            .iter()
            .any(|entry| entry.id == request.motion_id)
        {
            return Err(MotionError::NotFound);
        }
        let mut next = self.state.clone();
        next.bindings
            .retain(|binding| binding.motion_id != request.motion_id);
        self.commit_state(next)?;
        Ok(self.snapshot())
    }

    pub fn set_motion_enabled(
        &mut self,
        request: &MotionEnabledUpdateRequest,
    ) -> Result<MotionCatalogSnapshot, MotionError> {
        if !self
            .all_entries()
            .iter()
            .any(|entry| entry.id == request.id)
        {
            return Err(MotionError::NotFound);
        }
        let mut next = self.state.clone();
        if request.enabled {
            next.disabled_motion_ids.remove(&request.id);
        } else {
            next.disabled_motion_ids.insert(request.id.clone());
        }
        self.commit_state(next)?;
        Ok(self.snapshot())
    }

    pub fn reset_bindings(&mut self) -> Result<MotionCatalogSnapshot, MotionError> {
        let mut next = self.state.clone();
        next.bindings = default_bindings(&self.all_entries());
        self.commit_state(next)?;
        Ok(self.snapshot())
    }

    pub fn reset_binding(
        &mut self,
        region: InteractionRegion,
    ) -> Result<MotionCatalogSnapshot, MotionError> {
        let mut next = self.state.clone();
        next.bindings.retain(|binding| binding.region != region);
        self.commit_state(next)?;
        Ok(self.snapshot())
    }

    fn all_entries(&self) -> Vec<MotionCatalogEntry> {
        let mut entries = self.builtin_entries.clone();
        entries.extend(self.state.entries.clone());
        entries
    }

    fn sanitize_bindings(&mut self) {
        let ids: BTreeSet<_> = self
            .all_entries()
            .into_iter()
            .map(|entry| entry.id)
            .collect();
        self.state
            .bindings
            .retain(|binding| ids.contains(&binding.motion_id));
        let mut regions = BTreeSet::new();
        self.state
            .bindings
            .retain(|binding| regions.insert(binding.region));
        self.state
            .disabled_motion_ids
            .retain(|motion_id| ids.contains(motion_id));
    }

    fn commit_state(&mut self, next: MotionStateDocument) -> Result<(), MotionError> {
        atomic_json(&self.root.join(STATE_FILE), &next)?;
        self.state = next;
        Ok(())
    }

    fn save_state(&self) -> Result<(), MotionError> {
        atomic_json(&self.root.join(STATE_FILE), &self.state)
    }
}

pub fn inspect_motion(source: &Path) -> Result<InspectedMotion, MotionError> {
    if !source
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("vrma"))
    {
        return Err(MotionError::InvalidExtension);
    }
    let metadata = fs::metadata(source)?;
    if metadata.len() > MAX_VRMA_BYTES {
        return Err(MotionError::TooLarge);
    }
    let bytes = fs::read(source)?;
    let parsed = ParsedGlb::parse(&bytes)?;
    let extension = parsed
        .json
        .pointer("/extensions/VRMC_vrm_animation")
        .ok_or(MotionError::InvalidVrma)?;
    if extension.get("specVersion").and_then(Value::as_str) != Some("1.0") {
        return Err(MotionError::InvalidVrma);
    }
    let animations = parsed
        .json
        .get("animations")
        .and_then(Value::as_array)
        .ok_or(MotionError::InvalidAnimationCount)?;
    if animations.len() != 1 {
        return Err(MotionError::InvalidAnimationCount);
    }
    let human_bones = extension
        .pointer("/humanoid/humanBones")
        .and_then(Value::as_object)
        .ok_or(MotionError::MissingHumanoidMotion)?;
    let node_to_bone: BTreeMap<u64, String> = human_bones
        .iter()
        .filter_map(|(bone, value)| Some((value.get("node")?.as_u64()?, bone.clone())))
        .collect();
    let animation = &animations[0];
    let channels = animation
        .get("channels")
        .and_then(Value::as_array)
        .ok_or(MotionError::MissingHumanoidMotion)?;
    let mut animated = BTreeSet::new();
    let mut ignored_translations = 0_u32;
    let mut scale_tracks = 0_u32;
    for channel in channels {
        let Some(target) = channel.get("target") else {
            continue;
        };
        let Some(node) = target.get("node").and_then(Value::as_u64) else {
            continue;
        };
        let Some(bone) = node_to_bone.get(&node) else {
            continue;
        };
        animated.insert(bone.clone());
        match target.get("path").and_then(Value::as_str) {
            Some("scale") => scale_tracks += 1,
            Some("translation") if bone != "hips" => ignored_translations += 1,
            _ => {}
        }
    }
    if animated.is_empty() {
        return Err(MotionError::MissingHumanoidMotion);
    }
    let animated_bones: Vec<_> = animated.into_iter().collect();
    let finger_bone_count = animated_bones
        .iter()
        .filter(|bone| is_finger_bone(bone))
        .count()
        .try_into()
        .unwrap_or(u16::MAX);
    let mut warnings = Vec::new();
    if scale_tracks > 0 {
        warnings.push(format!("ignored_scale_tracks:{scale_tracks}"));
    }
    if ignored_translations > 0 {
        warnings.push(format!(
            "ignored_non_hips_translations:{ignored_translations}"
        ));
    }
    let duration_ms = parsed.duration_ms(animation)?;
    Ok(InspectedMotion {
        original_file_name: source
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("motion.vrma")
            .to_owned(),
        size_bytes: metadata
            .len()
            .try_into()
            .map_err(|_| MotionError::TooLarge)?,
        sha256: Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<Vec<_>>()
            .join(""),
        duration_ms,
        channels: infer_channels(&animated_bones),
        animated_bones,
        finger_bone_count,
        has_expression: extension.get("expressions").is_some(),
        has_look_at: extension.get("lookAt").is_some(),
        warnings,
        modified_millis: metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |value| value.as_millis()),
    })
}

struct ParsedGlb<'a> {
    json: Value,
    bin: &'a [u8],
}

impl<'a> ParsedGlb<'a> {
    fn parse(bytes: &'a [u8]) -> Result<Self, MotionError> {
        if bytes.len() < 20 || &bytes[0..4] != b"glTF" || read_u32(bytes, 4)? != 2 {
            return Err(MotionError::InvalidGlb);
        }
        let mut offset = 12_usize;
        let mut json = None;
        let mut bin = &bytes[0..0];
        while offset.saturating_add(8) <= bytes.len() {
            let length =
                usize::try_from(read_u32(bytes, offset)?).map_err(|_| MotionError::InvalidGlb)?;
            let kind = read_u32(bytes, offset + 4)?;
            let start = offset + 8;
            let end = start
                .checked_add(length)
                .filter(|end| *end <= bytes.len())
                .ok_or(MotionError::InvalidGlb)?;
            if kind == 0x4e4f_534a {
                json = Some(serde_json::from_slice::<Value>(&bytes[start..end])?);
            } else if kind == 0x004e_4942 {
                bin = &bytes[start..end];
            }
            offset = end;
        }
        Ok(Self {
            json: json.ok_or(MotionError::InvalidGlb)?,
            bin,
        })
    }

    fn duration_ms(&self, animation: &Value) -> Result<u32, MotionError> {
        let samplers = animation
            .get("samplers")
            .and_then(Value::as_array)
            .ok_or(MotionError::InvalidVrma)?;
        let accessors = self
            .json
            .get("accessors")
            .and_then(Value::as_array)
            .ok_or(MotionError::InvalidVrma)?;
        let views = self
            .json
            .get("bufferViews")
            .and_then(Value::as_array)
            .ok_or(MotionError::InvalidVrma)?;
        let mut duration = 0.0_f32;
        for sampler in samplers {
            let Some(index) = sampler.get("input").and_then(Value::as_u64) else {
                continue;
            };
            let Some(accessor) = accessors.get(index as usize) else {
                continue;
            };
            if let Some(maximum) = accessor
                .get("max")
                .and_then(Value::as_array)
                .and_then(|values| values.first())
                .and_then(Value::as_f64)
            {
                duration = duration.max(maximum as f32);
                continue;
            }
            if accessor.get("componentType").and_then(Value::as_u64) != Some(5126)
                || accessor.get("type").and_then(Value::as_str) != Some("SCALAR")
            {
                continue;
            }
            let Some(view_index) = accessor.get("bufferView").and_then(Value::as_u64) else {
                continue;
            };
            let Some(view) = views.get(view_index as usize) else {
                continue;
            };
            let start = view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0)
                + accessor
                    .get("byteOffset")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
            let stride = view.get("byteStride").and_then(Value::as_u64).unwrap_or(4);
            let count = accessor.get("count").and_then(Value::as_u64).unwrap_or(0);
            for index in 0..count {
                let offset = usize::try_from(start + index * stride)
                    .map_err(|_| MotionError::InvalidVrma)?;
                let end = offset
                    .checked_add(4)
                    .filter(|end| *end <= self.bin.len())
                    .ok_or(MotionError::InvalidVrma)?;
                duration = duration.max(f32::from_le_bytes(
                    self.bin[offset..end]
                        .try_into()
                        .map_err(|_| MotionError::InvalidVrma)?,
                ));
            }
        }
        if !duration.is_finite() || duration <= 0.0 {
            return Err(MotionError::InvalidVrma);
        }
        Ok((duration * 1000.0).round().clamp(1.0, u32::MAX as f32) as u32)
    }
}

fn validate_builtin_entries(
    root: &Path,
    entries: &[MotionCatalogEntry],
) -> Result<(), MotionError> {
    let mut ids = BTreeSet::new();
    let mut hashes = BTreeSet::new();
    for entry in entries {
        if entry.source != MotionSource::Builtin || !entry.protected {
            return Err(MotionError::InvalidBuiltin(format!(
                "{} is not protected",
                entry.id
            )));
        }
        if entry.name.trim().is_empty()
            || entry.name_zh.trim().is_empty()
            || entry.description.trim().is_empty()
            || entry.description_zh.trim().is_empty()
        {
            return Err(MotionError::InvalidBuiltin(format!(
                "{} has incomplete localization",
                entry.id
            )));
        }
        if !ids.insert(&entry.id) || !hashes.insert(&entry.sha256) {
            return Err(MotionError::InvalidBuiltin(format!(
                "duplicate {}",
                entry.id
            )));
        }
        let path = root.join(&entry.file_name);
        if !path.is_file() {
            return Err(MotionError::InvalidBuiltin(format!(
                "missing {}",
                entry.file_name
            )));
        }
    }
    Ok(())
}

fn validate_bindings(
    bindings: &[InteractionMotionBinding],
    entries: &[MotionCatalogEntry],
) -> Result<(), MotionError> {
    let ids: BTreeSet<_> = entries.iter().map(|entry| entry.id.as_str()).collect();
    let mut regions = BTreeSet::new();
    for binding in bindings {
        if !regions.insert(binding.region)
            || binding.cooldown_ms > 60_000
            || !ids.contains(binding.motion_id.as_str())
        {
            return Err(MotionError::InvalidBinding);
        }
    }
    Ok(())
}

fn default_bindings(_entries: &[MotionCatalogEntry]) -> Vec<InteractionMotionBinding> {
    Vec::new()
}

fn default_binding_values(
    region: InteractionRegion,
    motion_id: String,
) -> InteractionMotionBinding {
    InteractionMotionBinding {
        region,
        motion_id,
        cooldown_ms: if matches!(region, InteractionRegion::HeadTop | InteractionRegion::Face) {
            1_800
        } else {
            2_200
        },
        mirror_by_side: matches!(
            region,
            InteractionRegion::LeftHand
                | InteractionRegion::RightHand
                | InteractionRegion::LeftArm
                | InteractionRegion::RightArm
        ),
    }
}

fn replace_binding(
    bindings: &mut Vec<InteractionMotionBinding>,
    replacement: InteractionMotionBinding,
) {
    bindings.retain(|binding| binding.region != replacement.region);
    bindings.push(replacement);
    bindings.sort_by_key(|binding| binding.region);
}

fn infer_channels(bones: &[String]) -> Vec<BehaviorChannel> {
    let has_lower = bones.iter().any(|bone| {
        bone == "hips"
            || bone.contains("UpperLeg")
            || bone.contains("LowerLeg")
            || bone.contains("Foot")
            || bone.contains("Toes")
    });
    if has_lower {
        return vec![BehaviorChannel::FullBody];
    }
    let mut channels = vec![BehaviorChannel::UpperBody];
    if bones
        .iter()
        .any(|bone| bone.starts_with("left") && (bone.contains("Arm") || bone.contains("Hand")))
    {
        channels.push(BehaviorChannel::LeftArm);
    }
    if bones
        .iter()
        .any(|bone| bone.starts_with("right") && (bone.contains("Arm") || bone.contains("Hand")))
    {
        channels.push(BehaviorChannel::RightArm);
    }
    if bones.iter().any(|bone| is_finger_bone(bone)) {
        channels.push(BehaviorChannel::Fingers);
    }
    channels
}

fn is_finger_bone(bone: &str) -> bool {
    ["Thumb", "Index", "Middle", "Ring", "Little"]
        .iter()
        .any(|finger| bone.contains(finger))
}

fn validate_name(value: &str) -> Result<String, MotionError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 80 {
        return Err(MotionError::InvalidName);
    }
    Ok(value.to_owned())
}

fn validate_description(value: &str) -> Result<String, MotionError> {
    let value = value.trim();
    if value.chars().count() > 500 {
        return Err(MotionError::InvalidDescription);
    }
    Ok(value.to_owned())
}

fn category_tag(value: MotionCategory) -> &'static str {
    match value {
        MotionCategory::Idle => "idle",
        MotionCategory::Reaction => "reaction",
        MotionCategory::Gesture => "gesture",
        MotionCategory::Speech => "speech",
        MotionCategory::Locomotion => "locomotion",
        MotionCategory::Performance => "performance",
    }
}

fn is_category_tag(value: &str) -> bool {
    [
        "idle",
        "reaction",
        "gesture",
        "speech",
        "locomotion",
        "performance",
    ]
    .contains(&value)
}

fn atomic_json(path: &Path, value: &impl Serialize) -> Result<(), MotionError> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut file = AtomicWriteFile::open(path)?;
    file.write_all(&bytes)?;
    file.flush()?;
    file.commit()?;
    Ok(())
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, MotionError> {
    let end = offset
        .checked_add(4)
        .filter(|end| *end <= bytes.len())
        .ok_or(MotionError::InvalidGlb)?;
    Ok(u32::from_le_bytes(
        bytes[offset..end]
            .try_into()
            .map_err(|_| MotionError::InvalidGlb)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hachimi_protocol::{MotionPlaybackMode, MotionRootMode};

    #[test]
    fn category_tags_are_complete() {
        for category in [
            MotionCategory::Idle,
            MotionCategory::Reaction,
            MotionCategory::Gesture,
            MotionCategory::Speech,
            MotionCategory::Locomotion,
            MotionCategory::Performance,
        ] {
            assert!(is_category_tag(category_tag(category)));
        }
    }

    #[test]
    fn finger_detection_covers_vrm_names() {
        assert!(is_finger_bone("leftThumbMetacarpal"));
        assert!(is_finger_bone("rightLittleDistal"));
        assert!(!is_finger_bone("leftHand"));
    }

    #[test]
    fn bundled_catalog_loads_and_keeps_finger_metadata() {
        let root = tempfile::tempdir().expect("temporary catalog");
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/avatar-motions-v4/catalog.json");
        let catalog = MotionCatalog::load(root.path(), manifest).expect("V4 catalog");
        let snapshot = catalog.snapshot();
        assert_eq!(snapshot.entries.len(), 156);
        assert!(
            snapshot
                .entries
                .iter()
                .any(|entry| entry.finger_bone_count == 30)
        );
        assert!(
            snapshot
                .entries
                .iter()
                .any(|entry| !entry.has_finger_motion)
        );
        assert!(snapshot.bindings.is_empty());
        for name in ["standard waiting", "photobooth peace sign"] {
            let entry = snapshot
                .entries
                .iter()
                .find(|entry| entry.name.eq_ignore_ascii_case(name))
                .unwrap_or_else(|| panic!("missing {name}"));
            assert!(entry.finger_bone_count >= 28, "{name} lost finger tracks");
        }
    }

    #[test]
    fn degraded_catalog_starts_without_bundled_resources() {
        let root = tempfile::tempdir().expect("temporary catalog");
        let catalog = MotionCatalog::load_degraded(root.path()).expect("degraded catalog");
        let snapshot = catalog.snapshot();
        assert!(snapshot.entries.is_empty());
        assert!(snapshot.bindings.is_empty());
        assert!(root.path().join(STATE_FILE).is_file());
    }

    #[test]
    fn builtins_are_protected_and_user_delete_cleans_bindings() {
        let root = tempfile::tempdir().expect("temporary catalog");
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/avatar-motions-v4/catalog.json");
        let mut catalog = MotionCatalog::load(root.path(), manifest).expect("V4 catalog");
        let builtin_id = catalog.snapshot().entries[0].id.clone();
        assert!(matches!(
            catalog.delete_user(&builtin_id),
            Err(MotionError::Protected)
        ));

        let source = catalog.asset_for(&builtin_id).expect("builtin asset").path;
        let inspection = inspect_motion(&source).expect("inspect VRMA");
        let imported = catalog
            .import_inspected(
                &source,
                &inspection,
                &MotionImportCommitRequest {
                    token: "test-token".into(),
                    name: "Imported Test".into(),
                    description: "delete cleanup".into(),
                    category: MotionCategory::Gesture,
                    playback_mode: MotionPlaybackMode::Once,
                    root_mode: MotionRootMode::InPlace,
                    interaction_region: Some(InteractionRegion::Generic),
                },
            )
            .expect("import");
        let user_id = imported
            .entries
            .iter()
            .find(|entry| entry.source == MotionSource::User)
            .expect("user motion")
            .id
            .clone();
        catalog
            .update_binding(&InteractionMotionBindingUpdateRequest {
                region: InteractionRegion::Generic,
                motion_id: Some(user_id.clone()),
                cooldown_ms: Some(0),
                mirror_by_side: Some(false),
            })
            .expect("bind user motion");
        catalog
            .set_motion_enabled(&MotionEnabledUpdateRequest {
                id: user_id.clone(),
                enabled: false,
            })
            .expect("disable user motion");
        let deleted = catalog.delete_user(&user_id).expect("delete user motion");
        assert!(deleted.entries.iter().all(|entry| entry.id != user_id));
        assert!(!deleted.disabled_motion_ids.contains(&user_id));
        assert!(
            deleted
                .bindings
                .iter()
                .all(|binding| binding.region != InteractionRegion::Generic)
        );
    }

    #[test]
    fn optional_import_binding_is_committed_with_the_user_motion() {
        let root = tempfile::tempdir().expect("temporary catalog");
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/avatar-motions-v4/catalog.json");
        let mut catalog = MotionCatalog::load(root.path(), manifest).expect("V4 catalog");
        let source = catalog
            .asset_for(&catalog.snapshot().entries[0].id)
            .expect("builtin asset")
            .path;
        let inspection = inspect_motion(&source).expect("inspect VRMA");
        let imported = catalog
            .import_inspected(
                &source,
                &inspection,
                &MotionImportCommitRequest {
                    token: "test-token".into(),
                    name: "用户填写名称".into(),
                    description: "user supplied description".into(),
                    category: MotionCategory::Reaction,
                    playback_mode: MotionPlaybackMode::Once,
                    root_mode: MotionRootMode::InPlace,
                    interaction_region: Some(InteractionRegion::Face),
                },
            )
            .expect("import");
        let user = imported
            .entries
            .iter()
            .find(|entry| entry.source == MotionSource::User)
            .expect("user entry");
        assert_eq!(user.name_zh, user.name);
        assert_eq!(user.description_zh, user.description);
        assert_eq!(
            imported
                .bindings
                .iter()
                .find(|binding| binding.region == InteractionRegion::Face)
                .map(|binding| binding.motion_id.as_str()),
            Some(user.id.as_str())
        );
    }

    #[test]
    fn import_without_binding_only_creates_the_user_motion() {
        let root = tempfile::tempdir().expect("temporary catalog");
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/avatar-motions-v4/catalog.json");
        let mut catalog = MotionCatalog::load(root.path(), manifest).expect("V4 catalog");
        let before = catalog.snapshot();
        let source = catalog
            .asset_for(&before.entries[0].id)
            .expect("builtin asset")
            .path;
        let inspection = inspect_motion(&source).expect("inspect VRMA");
        let imported = catalog
            .import_inspected(
                &source,
                &inspection,
                &MotionImportCommitRequest {
                    token: "test-token".into(),
                    name: "Unbound user motion".into(),
                    description: String::new(),
                    category: MotionCategory::Gesture,
                    playback_mode: MotionPlaybackMode::Once,
                    root_mode: MotionRootMode::InPlace,
                    interaction_region: None,
                },
            )
            .expect("unbound import");

        assert_eq!(imported.entries.len(), before.entries.len() + 1);
        assert_eq!(imported.bindings, before.bindings);
    }

    #[test]
    fn region_updates_are_atomic_and_invalid_motion_ids_are_rejected() {
        let root = tempfile::tempdir().expect("temporary catalog");
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/avatar-motions-v4/catalog.json");
        let mut catalog = MotionCatalog::load(root.path(), manifest).expect("V4 catalog");
        let before = catalog.snapshot();
        let invalid_id = catalog.update_binding(&InteractionMotionBindingUpdateRequest {
            region: InteractionRegion::Face,
            motion_id: Some("missing.motion".into()),
            cooldown_ms: Some(0),
            mirror_by_side: Some(false),
        });
        assert!(matches!(invalid_id, Err(MotionError::InvalidBinding)));
        assert_eq!(catalog.snapshot(), before);

        let replacement_id = before.entries[0].id.clone();
        let updated = catalog
            .update_binding(&InteractionMotionBindingUpdateRequest {
                region: InteractionRegion::Face,
                motion_id: Some(replacement_id.clone()),
                cooldown_ms: Some(400),
                mirror_by_side: Some(true),
            })
            .expect("replace face binding");
        assert_eq!(updated.bindings.len(), before.bindings.len() + 1);
        assert!(updated.bindings.iter().any(|binding| {
            binding.region == InteractionRegion::Face
                && binding.motion_id == replacement_id
                && binding.cooldown_ms == 400
                && binding.mirror_by_side
        }));
        for binding in before
            .bindings
            .iter()
            .filter(|binding| binding.region != InteractionRegion::Face)
        {
            assert!(updated.bindings.contains(binding));
        }
    }

    #[test]
    fn an_intentionally_empty_binding_document_stays_unbound_after_reload() {
        let root = tempfile::tempdir().expect("temporary catalog");
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/avatar-motions-v4/catalog.json");
        let mut catalog = MotionCatalog::load(root.path(), &manifest).expect("V4 catalog");
        let regions: Vec<_> = catalog
            .snapshot()
            .bindings
            .iter()
            .map(|binding| binding.region)
            .collect();
        for region in regions {
            catalog
                .update_binding(&InteractionMotionBindingUpdateRequest {
                    region,
                    motion_id: None,
                    cooldown_ms: None,
                    mirror_by_side: None,
                })
                .expect("clear region binding");
        }
        assert!(catalog.snapshot().bindings.is_empty());
        drop(catalog);

        let reloaded = MotionCatalog::load(root.path(), manifest).expect("reload V4 catalog");
        assert!(reloaded.snapshot().bindings.is_empty());
    }

    #[test]
    fn disabled_motion_ids_persist_and_clear_all_related_bindings() {
        let root = tempfile::tempdir().expect("temporary catalog");
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/avatar-motions-v4/catalog.json");
        let mut catalog = MotionCatalog::load(root.path(), &manifest).expect("V4 catalog");
        let motion_id = catalog.snapshot().entries[0].id.clone();
        catalog
            .update_binding(&InteractionMotionBindingUpdateRequest {
                region: InteractionRegion::Face,
                motion_id: Some(motion_id.clone()),
                cooldown_ms: None,
                mirror_by_side: None,
            })
            .expect("bind face");
        catalog
            .update_binding(&InteractionMotionBindingUpdateRequest {
                region: InteractionRegion::Chest,
                motion_id: Some(motion_id.clone()),
                cooldown_ms: None,
                mirror_by_side: None,
            })
            .expect("bind chest");
        catalog
            .set_motion_enabled(&MotionEnabledUpdateRequest {
                id: motion_id.clone(),
                enabled: false,
            })
            .expect("disable motion");
        assert!(matches!(
            catalog.update_binding(&InteractionMotionBindingUpdateRequest {
                region: InteractionRegion::Belly,
                motion_id: Some(motion_id.clone()),
                cooldown_ms: None,
                mirror_by_side: None,
            }),
            Err(MotionError::InvalidBinding)
        ));
        drop(catalog);

        let mut reloaded = MotionCatalog::load(root.path(), manifest).expect("reload V4 catalog");
        assert_eq!(
            reloaded.snapshot().disabled_motion_ids.as_slice(),
            std::slice::from_ref(&motion_id)
        );
        let cleared = reloaded
            .clear_motion_bindings(&MotionAssetBindingsClearRequest {
                motion_id: motion_id.clone(),
            })
            .expect("clear asset bindings");
        assert!(
            cleared
                .bindings
                .iter()
                .all(|binding| binding.motion_id != motion_id)
        );
    }

    #[test]
    fn failed_state_write_rolls_back_import_and_new_blob() {
        let root = tempfile::tempdir().expect("temporary catalog");
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/avatar-motions-v4/catalog.json");
        let mut catalog = MotionCatalog::load(root.path(), manifest).expect("V4 catalog");
        let before = catalog.snapshot();
        let source = catalog
            .asset_for(&before.entries[0].id)
            .expect("builtin asset")
            .path;
        let inspection = inspect_motion(&source).expect("inspect VRMA");
        fs::remove_file(root.path().join(STATE_FILE)).expect("remove writable state");
        fs::create_dir(root.path().join(STATE_FILE)).expect("block atomic state write");

        let result = catalog.import_inspected(
            &source,
            &inspection,
            &MotionImportCommitRequest {
                token: "test-token".into(),
                name: "Rollback".into(),
                description: String::new(),
                category: MotionCategory::Gesture,
                playback_mode: MotionPlaybackMode::Once,
                root_mode: MotionRootMode::InPlace,
                interaction_region: None,
            },
        );

        assert!(result.is_err());
        assert_eq!(catalog.snapshot(), before);
        assert!(!root.path().join("blobs").join(inspection.sha256).exists());
    }

    #[test]
    fn v3_storage_does_not_read_or_delete_v2_motion_data() {
        let directory = tempfile::tempdir().expect("temporary data root");
        let legacy = directory.path().join("motions-v2");
        fs::create_dir_all(&legacy).expect("legacy directory");
        let sentinel = legacy.join("catalog.json");
        fs::write(&sentinel, b"legacy motion data").expect("legacy sentinel");
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/avatar-motions-v4/catalog.json");

        let catalog = MotionCatalog::load(directory.path().join("motions-v3"), manifest)
            .expect("fresh V3 catalog");

        assert!(catalog.snapshot().bindings.is_empty());
        assert_eq!(
            fs::read(&sentinel).expect("legacy data remains"),
            b"legacy motion data"
        );
    }
}
