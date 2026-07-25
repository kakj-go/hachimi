use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{self, BufReader, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use atomic_write_file::AtomicWriteFile;
use bzip2::read::BzDecoder;
use hachimi_protocol::{
    VoiceCatalogSnapshot, VoiceModelEntry, VoiceModelInspection, VoiceModelOrigin,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tar::{Archive, EntryType};
use tempfile::NamedTempFile;
use thiserror::Error;
use uuid::Uuid;

const CATALOG_SCHEMA_VERSION: u32 = 3;
const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_EXPANDED_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;
const MAX_TEXT_FILE_BYTES: u64 = 256 * 1024;
pub const BUILTIN_VOICE_ID: &str = "builtin-melo-zh-en";
pub const BUILTIN_ARCHIVE_SHA256: &str =
    "e58351ed7149f290a54534538badd4077cdbe6fddc964b24d0bee870415d1514";

#[derive(Debug, Error)]
pub enum VoiceCatalogError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("voice catalog serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("请选择 sherpa-onnx VITS 的 .tar.bz2 模型包")]
    InvalidExtension,
    #[error("语音模型包超过 512MB")]
    ArchiveTooLarge,
    #[error("语音模型包结构不安全：{0}")]
    UnsafeArchive(String),
    #[error("语音模型包不兼容：{0}")]
    Incompatible(String),
    #[error("模型名称必须为 1–64 个字符")]
    InvalidName,
    #[error("模型名称已存在")]
    DuplicateName,
    #[error("找不到语音模型")]
    NotFound,
    #[error("内置语音模型不可删除")]
    Protected,
    #[error("必须确认拥有该模型的使用权")]
    LicenseNotAcknowledged,
}

#[derive(Debug, Clone)]
pub struct InspectedVoiceModel {
    pub original_file_name: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub modified_millis: u128,
    pub model_type: String,
    pub languages: Vec<String>,
    pub sample_rate: u32,
    pub speaker_count: u32,
    pub suggested_speaker_id: u32,
    pub license_summary: String,
    pub license_warning: bool,
    pub compatible: bool,
    pub issues: Vec<String>,
    /// Relative paths inside the inspected archive. This is an internal Rust
    /// boundary and is deliberately omitted from the protocol view.
    pub paths: VoiceAssetPaths,
}

impl InspectedVoiceModel {
    #[must_use]
    pub fn view(&self, token: Option<String>) -> VoiceModelInspection {
        VoiceModelInspection {
            token: token.filter(|_| self.compatible),
            original_file_name: self.original_file_name.clone(),
            size_bytes: u32::try_from(self.size_bytes).unwrap_or(u32::MAX),
            sha256: self.sha256.clone(),
            model_type: self.model_type.clone(),
            languages: self.languages.clone(),
            sample_rate: self.sample_rate,
            speaker_count: self.speaker_count,
            suggested_speaker_id: self.suggested_speaker_id,
            required_files: [
                Some(self.paths.model.clone()),
                Some(self.paths.tokens.clone()),
                self.paths.lexicon.clone(),
                self.paths.data_dir.clone(),
                self.paths.dict_dir.clone(),
            ]
            .into_iter()
            .flatten()
            .chain(self.paths.rule_fsts.iter().cloned())
            .collect(),
            license_summary: self.license_summary.clone(),
            license_warning: self.license_warning,
            compatible: self.compatible,
            issues: self.issues.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceAssetPaths {
    pub model: String,
    pub tokens: String,
    pub lexicon: Option<String>,
    pub data_dir: Option<String>,
    pub dict_dir: Option<String>,
    pub rule_fsts: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct VoiceAsset {
    pub entry: VoiceModelEntry,
    pub root: PathBuf,
    pub paths: VoiceAssetPaths,
}

impl VoiceAsset {
    #[must_use]
    pub fn model_path(&self) -> PathBuf {
        self.root.join(&self.paths.model)
    }

    #[must_use]
    pub fn tokens_path(&self) -> PathBuf {
        self.root.join(&self.paths.tokens)
    }

    #[must_use]
    pub fn lexicon_path(&self) -> Option<PathBuf> {
        self.paths.lexicon.as_ref().map(|path| self.root.join(path))
    }

    #[must_use]
    pub fn data_dir(&self) -> Option<PathBuf> {
        self.paths
            .data_dir
            .as_ref()
            .map(|path| self.root.join(path))
    }

    #[must_use]
    pub fn dict_dir(&self) -> Option<PathBuf> {
        self.paths
            .dict_dir
            .as_ref()
            .map(|path| self.root.join(path))
    }

    #[must_use]
    pub fn rule_fsts(&self) -> Vec<PathBuf> {
        self.paths
            .rule_fsts
            .iter()
            .map(|path| self.root.join(path))
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredVoiceEntry {
    entry: VoiceModelEntry,
    paths: VoiceAssetPaths,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogDocument {
    schema_version: u32,
    current_id: String,
    entries: Vec<StoredVoiceEntry>,
}

impl Default for CatalogDocument {
    fn default() -> Self {
        Self {
            schema_version: CATALOG_SCHEMA_VERSION,
            current_id: BUILTIN_VOICE_ID.into(),
            entries: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct VoiceCatalog {
    root: PathBuf,
    built_in_root: PathBuf,
    built_in: StoredVoiceEntry,
    document: CatalogDocument,
}

impl VoiceCatalog {
    pub fn load(
        root: impl Into<PathBuf>,
        built_in_root: impl Into<PathBuf>,
    ) -> Result<Self, VoiceCatalogError> {
        let root = root.into();
        let built_in_root = built_in_root.into();
        fs::create_dir_all(&root)?;
        let document_path = root.join("catalog.json");
        let mut document = if document_path.is_file() {
            serde_json::from_slice::<CatalogDocument>(&fs::read(&document_path)?)
                .unwrap_or_default()
        } else {
            CatalogDocument::default()
        };
        match document.schema_version {
            CATALOG_SCHEMA_VERSION => {}
            1 => {
                document.schema_version = CATALOG_SCHEMA_VERSION;
                for stored in &mut document.entries {
                    // Catalog v1 only accepted single-speaker models and did
                    // not persist a speaker selection.
                    if stored.entry.speaker_count == 0 {
                        stored.entry.speaker_count = 1;
                    }
                    if stored.entry.speaker_id >= stored.entry.speaker_count {
                        stored.entry.speaker_id = 0;
                    }
                }
            }
            _ => document = CatalogDocument::default(),
        }
        if document.current_id != BUILTIN_VOICE_ID
            && !document
                .entries
                .iter()
                .any(|entry| entry.entry.id == document.current_id)
        {
            document.current_id = BUILTIN_VOICE_ID.into();
        }
        let built_in = built_in_entry(&built_in_root);
        let catalog = Self {
            root,
            built_in_root,
            built_in,
            document,
        };
        catalog.save()?;
        Ok(catalog)
    }

    #[must_use]
    pub fn snapshot(&self) -> VoiceCatalogSnapshot {
        let mut entries = vec![self.built_in.entry.clone()];
        entries.extend(
            self.document
                .entries
                .iter()
                .map(|entry| entry.entry.clone()),
        );
        VoiceCatalogSnapshot {
            entries,
            current_id: self.document.current_id.clone(),
        }
    }

    #[must_use]
    pub fn current_asset(&self) -> Option<VoiceAsset> {
        self.asset(&self.document.current_id)
    }

    #[must_use]
    pub fn asset(&self, id: &str) -> Option<VoiceAsset> {
        if id == BUILTIN_VOICE_ID {
            return asset_if_complete(&self.built_in, &self.built_in_root);
        }
        let stored = self
            .document
            .entries
            .iter()
            .find(|stored| stored.entry.id == id)?;
        asset_if_complete(stored, &self.root.join(&stored.entry.sha256))
    }

    pub fn import_inspected(
        &mut self,
        name: &str,
        source: &Path,
        inspection: &InspectedVoiceModel,
        license_acknowledged: bool,
        speaker_id: u32,
    ) -> Result<VoiceCatalogSnapshot, VoiceCatalogError> {
        let name = validate_name(name)?;
        if !inspection.compatible {
            return Err(VoiceCatalogError::Incompatible(
                inspection.issues.join("；"),
            ));
        }
        if !license_acknowledged {
            return Err(VoiceCatalogError::LicenseNotAcknowledged);
        }
        if inspection.speaker_count == 0 || speaker_id >= inspection.speaker_count {
            return Err(VoiceCatalogError::Incompatible(format!(
                "Speaker ID {speaker_id} 超出有效范围 0–{}",
                inspection.speaker_count.saturating_sub(1)
            )));
        }
        if self
            .snapshot()
            .entries
            .iter()
            .any(|entry| entry.name.eq_ignore_ascii_case(&name))
        {
            return Err(VoiceCatalogError::DuplicateName);
        }
        if fs::metadata(source)?.len() != inspection.size_bytes
            || hash_file(source)? != inspection.sha256
        {
            return Err(VoiceCatalogError::Incompatible("源文件已发生变化".into()));
        }
        self.install_archive(source, inspection)?;
        let id = Uuid::new_v4().to_string();
        let stored = StoredVoiceEntry {
            entry: VoiceModelEntry {
                id: id.clone(),
                name,
                sha256: inspection.sha256.clone(),
                original_file_name: inspection.original_file_name.clone(),
                size_bytes: u32::try_from(inspection.size_bytes).unwrap_or(u32::MAX),
                origin: VoiceModelOrigin::Imported,
                model_type: inspection.model_type.clone(),
                languages: inspection.languages.clone(),
                sample_rate: inspection.sample_rate,
                speaker_count: inspection.speaker_count,
                speaker_id,
                license_summary: inspection.license_summary.clone(),
                license_warning: inspection.license_warning,
                protected: false,
                imported_at: now_millis().to_string(),
            },
            paths: inspection.paths.clone(),
        };
        self.document.entries.push(stored);
        if let Err(error) = self.save() {
            self.document.entries.retain(|entry| entry.entry.id != id);
            return Err(error);
        }
        Ok(self.snapshot())
    }

    pub fn select(&mut self, id: &str) -> Result<VoiceCatalogSnapshot, VoiceCatalogError> {
        if id != BUILTIN_VOICE_ID
            && !self
                .document
                .entries
                .iter()
                .any(|entry| entry.entry.id == id)
        {
            return Err(VoiceCatalogError::NotFound);
        }
        let previous = std::mem::replace(&mut self.document.current_id, id.to_owned());
        if let Err(error) = self.save() {
            self.document.current_id = previous;
            return Err(error);
        }
        Ok(self.snapshot())
    }

    pub fn delete(&mut self, id: &str) -> Result<VoiceCatalogSnapshot, VoiceCatalogError> {
        if id == BUILTIN_VOICE_ID {
            return Err(VoiceCatalogError::Protected);
        }
        let index = self
            .document
            .entries
            .iter()
            .position(|entry| entry.entry.id == id)
            .ok_or(VoiceCatalogError::NotFound)?;
        let previous = self.document.clone();
        let removed = self.document.entries.remove(index);
        if self.document.current_id == id {
            self.document.current_id = BUILTIN_VOICE_ID.into();
        }
        if let Err(error) = self.save() {
            self.document = previous;
            return Err(error);
        }
        if !self
            .document
            .entries
            .iter()
            .any(|entry| entry.entry.sha256 == removed.entry.sha256)
        {
            let _ = fs::remove_dir_all(self.root.join(&removed.entry.sha256));
        }
        Ok(self.snapshot())
    }

    fn install_archive(
        &self,
        source: &Path,
        inspection: &InspectedVoiceModel,
    ) -> Result<(), VoiceCatalogError> {
        let destination = self.root.join(&inspection.sha256);
        if destination.is_dir() {
            return Ok(());
        }
        let temporary = self.root.join(format!(".import-{}", Uuid::new_v4()));
        fs::create_dir(&temporary)?;
        let result = extract_archive(source, &temporary);
        if let Err(error) = result {
            let _ = fs::remove_dir_all(&temporary);
            return Err(error);
        }
        if destination.exists() {
            fs::remove_dir_all(&temporary)?;
            return Ok(());
        }
        if let Err(error) = fs::rename(&temporary, &destination) {
            let _ = fs::remove_dir_all(&temporary);
            return Err(error.into());
        }
        Ok(())
    }

    fn save(&self) -> Result<(), VoiceCatalogError> {
        let bytes = serde_json::to_vec_pretty(&self.document)?;
        let mut file = AtomicWriteFile::open(self.root.join("catalog.json"))?;
        file.write_all(&bytes)?;
        file.flush()?;
        file.commit()?;
        Ok(())
    }
}

pub fn inspect_voice_archive(source: &Path) -> Result<InspectedVoiceModel, VoiceCatalogError> {
    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if !file_name.to_ascii_lowercase().ends_with(".tar.bz2") {
        return Err(VoiceCatalogError::InvalidExtension);
    }
    let metadata = fs::metadata(source)?;
    if metadata.len() > MAX_ARCHIVE_BYTES {
        return Err(VoiceCatalogError::ArchiveTooLarge);
    }
    let mut inspection = inspect_archive_contents(source)?;
    inspection.original_file_name = file_name.to_owned();
    inspection.size_bytes = metadata.len();
    inspection.sha256 = hash_file(source)?;
    inspection.modified_millis = modified_millis(&metadata);
    Ok(inspection)
}

fn inspect_archive_contents(source: &Path) -> Result<InspectedVoiceModel, VoiceCatalogError> {
    let mut archive = Archive::new(BzDecoder::new(File::open(source)?));
    let mut paths = BTreeSet::new();
    let mut expanded = 0_u64;
    let mut count = 0_usize;
    let mut model_path = None;
    let mut tokens_path = None;
    let mut lexicon_path = None;
    let mut data_dir = None;
    let mut dict_dir = None;
    let mut rule_fsts = Vec::new();
    let mut metadata_map = BTreeMap::new();
    let mut model_card = String::new();
    let mut license_file = String::new();
    for item in archive.entries()? {
        let mut entry = item?;
        count += 1;
        if count > MAX_ENTRIES {
            return Err(VoiceCatalogError::UnsafeArchive("条目数量超过 4096".into()));
        }
        let entry_type = entry.header().entry_type();
        if !matches!(entry_type, EntryType::Regular | EntryType::Directory) {
            return Err(VoiceCatalogError::UnsafeArchive(
                "不允许链接或特殊文件".into(),
            ));
        }
        let path = entry.path()?.into_owned();
        validate_archive_path(&path)?;
        let normalized = path.to_string_lossy().replace('\\', "/");
        let key = normalized.to_ascii_lowercase();
        if !paths.insert(key) {
            return Err(VoiceCatalogError::UnsafeArchive("存在重复路径".into()));
        }
        // tar writers are not required to emit directory entries. Discover
        // required resource directories from regular-file ancestors as well,
        // otherwise valid official sherpa-onnx archives can be rejected.
        data_dir = data_dir.or_else(|| archive_ancestor(&normalized, "espeak-ng-data"));
        dict_dir = dict_dir.or_else(|| archive_ancestor(&normalized, "dict"));
        if entry_type == EntryType::Directory {
            continue;
        }
        expanded = expanded.saturating_add(entry.size());
        if expanded > MAX_EXPANDED_BYTES {
            return Err(VoiceCatalogError::UnsafeArchive(
                "总解压大小超过 1GB".into(),
            ));
        }
        let lower = normalized.to_ascii_lowercase();
        if [".tar.bz2", ".tar.gz", ".tgz", ".zip", ".7z", ".rar"]
            .iter()
            .any(|extension| lower.ends_with(extension))
        {
            return Err(VoiceCatalogError::UnsafeArchive("不允许嵌套归档".into()));
        }
        if lower.ends_with(".onnx") {
            let mut temporary = NamedTempFile::new()?;
            io::copy(&mut entry, &mut temporary)?;
            if is_git_lfs_pointer(temporary.path())? {
                // Some official sherpa-onnx release archives accidentally
                // contain an unmaterialized optional INT8 file. It is not an
                // ONNX model and must not count as a second main model.
                continue;
            }
            if model_path.is_some() {
                return Err(VoiceCatalogError::Incompatible(
                    "VITS 包包含多个有效 ONNX 主模型，请只保留一个".into(),
                ));
            }
            metadata_map.extend(read_onnx_metadata(temporary.path())?);
            model_path = Some(normalized);
        } else if lower.ends_with("/tokens.txt") || lower == "tokens.txt" {
            tokens_path = Some(normalized);
        } else if lower.ends_with("/lexicon.txt") || lower == "lexicon.txt" {
            lexicon_path = Some(normalized);
        } else if lower.ends_with(".fst") {
            rule_fsts.push(normalized);
        } else if lower.ends_with("/model_card") || lower == "model_card" {
            if entry.size() <= MAX_TEXT_FILE_BYTES {
                entry.read_to_string(&mut model_card)?;
            }
        } else if is_license_path(&lower) && entry.size() <= MAX_TEXT_FILE_BYTES {
            entry.read_to_string(&mut license_file)?;
        } else if lower.ends_with(".onnx.json") && entry.size() <= MAX_TEXT_FILE_BYTES {
            let mut json = String::new();
            entry.read_to_string(&mut json)?;
            apply_piper_json(&json, &mut metadata_map);
        }
    }

    let mut issues = Vec::new();
    let model = model_path.unwrap_or_else(|| {
        issues.push("缺少 ONNX 主模型".into());
        String::new()
    });
    let tokens = tokens_path.unwrap_or_else(|| {
        issues.push("缺少 tokens.txt".into());
        String::new()
    });
    let model_type = metadata_map.get("model_type").cloned().unwrap_or_default();
    let model_type_lower = model_type.to_ascii_lowercase();
    if !model_type_lower.contains("vits") {
        issues.push("仅支持 VITS/Piper-VITS/Melo-VITS".into());
    }
    let speaker_count =
        metadata_u32(&metadata_map, &["n_speakers", "num_speakers"]).unwrap_or_default();
    if speaker_count == 0 {
        issues.push("模型缺少说话人数元数据".into());
    }
    let suggested_speaker_id = suggested_speaker_id(&metadata_map, &model, speaker_count);
    let sample_rate =
        metadata_u32(&metadata_map, &["sample_rate", "sampling_rate"]).unwrap_or_default();
    if sample_rate == 0 {
        issues.push("模型缺少有效采样率元数据".into());
    }
    let languages = parse_languages(metadata_map.get("language").map(String::as_str));
    if languages.is_empty() {
        issues.push("模型缺少中文或英文语言元数据".into());
    }
    let license_summary = detect_license(&metadata_map, &model_card, &license_file);
    let license_lower = license_summary.to_ascii_lowercase();
    let license_warning = license_lower.contains("unknown")
        || license_lower.contains("non-commercial")
        || license_lower.contains("noncommercial")
        || license_lower.contains("cc-by-nc");
    let parent = Path::new(&model)
        .parent()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .filter(|path| !path.is_empty());
    if data_dir.is_none() {
        data_dir = parent
            .as_ref()
            .map(|path| format!("{path}/espeak-ng-data"))
            .filter(|path| paths.contains(&path.to_ascii_lowercase()));
    }
    if dict_dir.is_none() {
        dict_dir = parent
            .as_ref()
            .map(|path| format!("{path}/dict"))
            .filter(|path| paths.contains(&path.to_ascii_lowercase()));
    }
    if metadata_map
        .get("has_g2pw")
        .is_some_and(|value| value == "1")
        && lexicon_path.is_none()
    {
        issues.push("该中文 VITS 模型需要 lexicon.txt".into());
    }
    if metadata_map
        .get("has_espeak")
        .is_some_and(|value| value == "1")
        && data_dir.is_none()
    {
        issues.push("该 VITS 模型需要 espeak-ng-data 目录".into());
    }
    Ok(InspectedVoiceModel {
        original_file_name: String::new(),
        size_bytes: 0,
        sha256: String::new(),
        modified_millis: 0,
        model_type,
        languages,
        sample_rate,
        speaker_count,
        suggested_speaker_id,
        license_summary,
        license_warning,
        compatible: issues.is_empty(),
        issues,
        paths: VoiceAssetPaths {
            model,
            tokens,
            lexicon: lexicon_path,
            data_dir,
            dict_dir,
            rule_fsts,
        },
    })
}

fn is_git_lfs_pointer(path: &Path) -> Result<bool, VoiceCatalogError> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > 1024 {
        return Ok(false);
    }
    let value = fs::read(path)?;
    let text = String::from_utf8_lossy(&value);
    Ok(
        text.starts_with("version https://git-lfs.github.com/spec/v1\n")
            && text.lines().any(|line| line.starts_with("oid sha256:"))
            && text.lines().any(|line| line.starts_with("size ")),
    )
}

fn is_license_path(path: &str) -> bool {
    matches!(
        path.rsplit('/').next().unwrap_or_default(),
        "license" | "license.txt" | "license.md" | "copying" | "copying.txt"
    )
}

fn suggested_speaker_id(
    metadata: &BTreeMap<String, String>,
    model_path: &str,
    speaker_count: u32,
) -> u32 {
    let from_metadata = metadata_u32(
        metadata,
        &["default_speaker_id", "speaker_id", "speaker", "sid"],
    );
    if let Some(value) = from_metadata.filter(|value| *value < speaker_count) {
        return value;
    }
    let model_name = Path::new(model_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    // Official csukuangfj/vits-zh-hf models share one 804-speaker
    // checkpoint. The archive named keqing.onnx corresponds to sid 115 in
    // that project's published info.json.
    match model_name.as_str() {
        "keqing" if speaker_count > 115 => 115,
        _ => 0,
    }
}

fn detect_license(
    metadata: &BTreeMap<String, String>,
    model_card: &str,
    license_file: &str,
) -> String {
    if let Some(value) = metadata
        .get("license")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        return normalize_license_summary(value);
    }
    if let Some(value) = model_card
        .lines()
        .find(|line| line.to_ascii_lowercase().contains("license:"))
        .map(|line| line.trim().trim_start_matches(['*', '#']).trim())
    {
        return normalize_license_summary(value);
    }
    let lower = license_file.to_ascii_lowercase();
    if lower.contains("permission is hereby granted, free of charge")
        && lower.contains("the software is provided \"as is\"")
    {
        return "License: MIT".into();
    }
    if lower.contains("apache license") && lower.contains("version 2.0") {
        return "License: Apache-2.0".into();
    }
    if lower.contains("bsd 3-clause") {
        return "License: BSD-3-Clause".into();
    }
    "License: Unknown".into()
}

fn normalize_license_summary(value: &str) -> String {
    let value = value.trim();
    if value.to_ascii_lowercase().starts_with("license:") {
        value.to_owned()
    } else if value.eq_ignore_ascii_case("mit license") {
        "License: MIT".into()
    } else {
        format!("License: {value}")
    }
}

fn archive_ancestor(path: &str, directory: &str) -> Option<String> {
    let components = path.split('/').collect::<Vec<_>>();
    components
        .iter()
        .position(|component| component.eq_ignore_ascii_case(directory))
        .map(|index| components[..=index].join("/"))
}

fn extract_archive(source: &Path, destination: &Path) -> Result<(), VoiceCatalogError> {
    let mut archive = Archive::new(BzDecoder::new(File::open(source)?));
    let mut expanded = 0_u64;
    let mut count = 0_usize;
    let mut paths = BTreeSet::new();
    for item in archive.entries()? {
        let mut entry = item?;
        count += 1;
        if count > MAX_ENTRIES {
            return Err(VoiceCatalogError::UnsafeArchive("条目过多".into()));
        }
        let entry_type = entry.header().entry_type();
        if !matches!(entry_type, EntryType::Regular | EntryType::Directory) {
            return Err(VoiceCatalogError::UnsafeArchive(
                "不允许链接或特殊文件".into(),
            ));
        }
        let path = entry.path()?.into_owned();
        validate_archive_path(&path)?;
        let key = path
            .to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase();
        if !paths.insert(key) {
            return Err(VoiceCatalogError::UnsafeArchive("存在重复路径".into()));
        }
        expanded = expanded.saturating_add(entry.size());
        if expanded > MAX_EXPANDED_BYTES {
            return Err(VoiceCatalogError::UnsafeArchive(
                "总解压大小超过 1GB".into(),
            ));
        }
        let target = destination.join(&path);
        if entry_type == EntryType::Directory {
            fs::create_dir_all(target)?;
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut output = File::create(target)?;
            io::copy(&mut entry, &mut output)?;
            output.flush()?;
        }
    }
    Ok(())
}

fn validate_archive_path(path: &Path) -> Result<(), VoiceCatalogError> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(VoiceCatalogError::UnsafeArchive("归档路径越界".into()));
    }
    Ok(())
}

fn read_onnx_metadata(path: &Path) -> Result<BTreeMap<String, String>, VoiceCatalogError> {
    let mut reader = BufReader::new(File::open(path)?);
    let file_len = reader.get_ref().metadata()?.len();
    let mut result = BTreeMap::new();
    while reader.stream_position()? < file_len {
        let key = read_varint(&mut reader)?;
        let field = key >> 3;
        let wire = key & 7;
        if field == 14 && wire == 2 {
            let length = read_varint(&mut reader)?;
            if length > MAX_TEXT_FILE_BYTES {
                return Err(VoiceCatalogError::Incompatible("ONNX 元数据异常".into()));
            }
            let mut bytes = vec![0; length as usize];
            reader.read_exact(&mut bytes)?;
            if let Some((key, value)) = parse_string_pair(&bytes) {
                result.insert(key, value);
            }
        } else {
            skip_wire(&mut reader, wire)?;
        }
    }
    Ok(result)
}

fn parse_string_pair(bytes: &[u8]) -> Option<(String, String)> {
    let mut cursor = io::Cursor::new(bytes);
    let mut key = None;
    let mut value = None;
    while cursor.position() < bytes.len() as u64 {
        let tag = read_varint(&mut cursor).ok()?;
        let field = tag >> 3;
        let wire = tag & 7;
        if wire != 2 {
            skip_wire(&mut cursor, wire).ok()?;
            continue;
        }
        let length = read_varint(&mut cursor).ok()? as usize;
        let mut text = vec![0; length];
        cursor.read_exact(&mut text).ok()?;
        let text = String::from_utf8(text).ok()?;
        if field == 1 {
            key = Some(text);
        } else if field == 2 {
            value = Some(text);
        }
    }
    Some((key?, value?))
}

fn read_varint(reader: &mut impl Read) -> io::Result<u64> {
    let mut value = 0_u64;
    for shift in (0..70).step_by(7) {
        let mut byte = [0_u8; 1];
        reader.read_exact(&mut byte)?;
        value |= u64::from(byte[0] & 0x7f) << shift;
        if byte[0] & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(io::Error::new(io::ErrorKind::InvalidData, "invalid varint"))
}

fn skip_wire(reader: &mut (impl Read + Seek), wire: u64) -> io::Result<()> {
    let bytes = match wire {
        0 => {
            let _ = read_varint(reader)?;
            return Ok(());
        }
        1 => 8,
        2 => i64::try_from(read_varint(reader)?)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "length overflow"))?,
        5 => 4,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported wire type",
            ));
        }
    };
    reader.seek(SeekFrom::Current(bytes))?;
    Ok(())
}

fn apply_piper_json(value: &str, metadata: &mut BTreeMap<String, String>) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(value) else {
        return;
    };
    if let Some(rate) = value
        .pointer("/audio/sample_rate")
        .and_then(|value| value.as_u64())
    {
        metadata.insert("sample_rate".into(), rate.to_string());
    }
    if let Some(language) = value
        .pointer("/language/code")
        .and_then(|value| value.as_str())
    {
        metadata.insert("language".into(), language.to_owned());
    }
    metadata
        .entry("model_type".into())
        .or_insert_with(|| "vits-piper".into());
    metadata
        .entry("n_speakers".into())
        .or_insert_with(|| "1".into());
}

fn metadata_u32(metadata: &BTreeMap<String, String>, keys: &[&str]) -> Option<u32> {
    keys.iter()
        .find_map(|key| metadata.get(*key)?.parse::<u32>().ok())
}

fn parse_languages(value: Option<&str>) -> Vec<String> {
    let value = value.unwrap_or_default().to_ascii_lowercase();
    let mut languages = Vec::new();
    if value.contains("zh") || value.contains("chinese") {
        languages.push("zh-CN".into());
    }
    if value.contains("en") || value.contains("english") {
        languages.push("en-US".into());
    }
    languages
}

fn built_in_entry(root: &Path) -> StoredVoiceEntry {
    let prefix = "vits-melo-tts-zh_en";
    let model_root = root.join(prefix);
    let effective_root = if model_root.is_dir() { prefix } else { "" };
    let relative = |name: &str| {
        if effective_root.is_empty() {
            name.to_owned()
        } else {
            format!("{effective_root}/{name}")
        }
    };
    StoredVoiceEntry {
        entry: VoiceModelEntry {
            id: BUILTIN_VOICE_ID.into(),
            name: "Hachimi 中英双语女声（MeloTTS）".into(),
            sha256: BUILTIN_ARCHIVE_SHA256.into(),
            original_file_name: "vits-melo-tts-zh_en.tar.bz2".into(),
            size_bytes: 167_006_755,
            origin: VoiceModelOrigin::BuiltIn,
            model_type: "melo-vits".into(),
            languages: vec!["zh-CN".into(), "en-US".into()],
            sample_rate: 44_100,
            speaker_count: 1,
            speaker_id: 0,
            license_summary: "MIT".into(),
            license_warning: false,
            protected: true,
            imported_at: "0".into(),
        },
        paths: VoiceAssetPaths {
            model: relative("model.onnx"),
            tokens: relative("tokens.txt"),
            lexicon: Some(relative("lexicon.txt")),
            data_dir: None,
            dict_dir: Some(relative("dict")),
            rule_fsts: ["new_heteronym.fst", "phone.fst", "date.fst", "number.fst"]
                .into_iter()
                .map(relative)
                .collect(),
        },
    }
}

fn asset_if_complete(stored: &StoredVoiceEntry, root: &Path) -> Option<VoiceAsset> {
    let model = root.join(&stored.paths.model);
    let tokens = root.join(&stored.paths.tokens);
    (model.is_file() && tokens.is_file()).then(|| VoiceAsset {
        entry: stored.entry.clone(),
        root: root.to_owned(),
        paths: stored.paths.clone(),
    })
}

fn validate_name(value: &str) -> Result<String, VoiceCatalogError> {
    let value = value.trim();
    let length = value.chars().count();
    if !(1..=64).contains(&length) {
        return Err(VoiceCatalogError::InvalidName);
    }
    Ok(value.to_owned())
}

fn hash_file(path: &Path) -> Result<String, io::Error> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn modified_millis(metadata: &fs::Metadata) -> u128 {
    metadata
        .modified()
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bzip2::{Compression, write::BzEncoder};
    use tar::{Builder, Header};

    fn put_varint(mut value: u64, output: &mut Vec<u8>) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            output.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    fn metadata_pair(key: &str, value: &str) -> Vec<u8> {
        let mut pair = Vec::new();
        pair.push(10);
        put_varint(key.len() as u64, &mut pair);
        pair.extend_from_slice(key.as_bytes());
        pair.push(18);
        put_varint(value.len() as u64, &mut pair);
        pair.extend_from_slice(value.as_bytes());
        pair
    }

    fn fake_onnx(values: &[(&str, &str)]) -> Vec<u8> {
        let mut model = Vec::new();
        for (key, value) in values {
            let pair = metadata_pair(key, value);
            put_varint((14 << 3) | 2, &mut model);
            put_varint(pair.len() as u64, &mut model);
            model.extend_from_slice(&pair);
        }
        model
    }

    fn append_file(builder: &mut Builder<BzEncoder<File>>, path: &str, bytes: &[u8]) {
        let mut header = Header::new_gnu();
        header.set_entry_type(EntryType::Regular);
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, path, bytes)
            .expect("append archive entry");
    }

    fn write_fixture(path: &Path, metadata: &[(&str, &str)]) {
        let encoder = BzEncoder::new(File::create(path).expect("fixture"), Compression::best());
        let mut builder = Builder::new(encoder);
        append_file(&mut builder, "voice/model.onnx", &fake_onnx(metadata));
        append_file(&mut builder, "voice/tokens.txt", b"_ 0\n");
        append_file(&mut builder, "voice/lexicon.txt", "你 n i 3 _\n".as_bytes());
        append_file(&mut builder, "voice/MODEL_CARD", b"License: CC0\n");
        builder.finish().expect("finish tar");
        builder
            .into_inner()
            .expect("encoder")
            .finish()
            .expect("bz2");
    }

    fn compatible_metadata() -> Vec<(&'static str, &'static str)> {
        vec![
            ("model_type", "vits-melo"),
            ("language", "Chinese + English"),
            ("sample_rate", "44100"),
            ("n_speakers", "1"),
            ("has_g2pw", "1"),
        ]
    }

    #[test]
    fn archive_paths_reject_traversal() {
        assert!(validate_archive_path(Path::new("voice/model.onnx")).is_ok());
        assert!(validate_archive_path(Path::new("../model.onnx")).is_err());
        assert!(validate_archive_path(Path::new("/model.onnx")).is_err());
    }

    #[test]
    fn language_metadata_is_normalized() {
        assert_eq!(
            parse_languages(Some("Chinese + English")),
            ["zh-CN", "en-US"]
        );
        assert_eq!(parse_languages(Some("zh_CN")), ["zh-CN"]);
    }

    #[test]
    fn built_in_is_protected_and_default() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let catalog = VoiceCatalog::load(temporary.path().join("catalog"), temporary.path())
            .expect("catalog");
        assert_eq!(catalog.snapshot().current_id, BUILTIN_VOICE_ID);
        assert!(catalog.snapshot().entries[0].protected);
    }

    #[test]
    fn catalog_v1_migrates_to_single_speaker_defaults() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let root = temporary.path().join("catalog");
        fs::create_dir_all(&root).expect("catalog directory");
        fs::write(
            root.join("catalog.json"),
            br#"{
              "schemaVersion": 1,
              "currentId": "legacy",
              "entries": [{
                "entry": {
                  "id": "legacy", "name": "Legacy", "sha256": "sha",
                  "originalFileName": "legacy.tar.bz2", "sizeBytes": 1,
                  "origin": "imported", "modelType": "vits",
                  "languages": ["zh-CN"], "sampleRate": 22050,
                  "licenseSummary": "License: Test", "licenseWarning": false,
                  "protected": false, "importedAt": "1"
                },
                "paths": { "model": "model.onnx", "tokens": "tokens.txt", "ruleFsts": [] }
              }]
            }"#,
        )
        .expect("legacy catalog");
        let catalog = VoiceCatalog::load(&root, temporary.path().join("missing-builtin"))
            .expect("migrated catalog");
        let legacy = catalog
            .snapshot()
            .entries
            .into_iter()
            .find(|entry| entry.id == "legacy")
            .expect("legacy entry");
        assert_eq!(legacy.speaker_count, 1);
        assert_eq!(legacy.speaker_id, 0);
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("catalog.json")).expect("saved catalog"))
                .expect("saved JSON");
        assert_eq!(saved["schemaVersion"], CATALOG_SCHEMA_VERSION);
    }

    #[test]
    fn inspects_single_speaker_bilingual_vits_archive() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let source = temporary.path().join("vits-melo-tts-zh_en.tar.bz2");
        write_fixture(&source, &compatible_metadata());
        let inspection = inspect_voice_archive(&source).expect("inspection");
        assert!(inspection.compatible, "{:?}", inspection.issues);
        assert_eq!(inspection.languages, ["zh-CN", "en-US"]);
        assert_eq!(inspection.sample_rate, 44_100);
        assert_eq!(inspection.speaker_count, 1);
        assert!(!inspection.license_warning);
        assert!(inspection.view(Some("token".into())).token.is_some());
    }

    #[test]
    fn discovers_required_directories_without_explicit_tar_directory_entries() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let source = temporary.path().join("implicit-directories.tar.bz2");
        let encoder = BzEncoder::new(File::create(&source).expect("fixture"), Compression::best());
        let mut builder = Builder::new(encoder);
        let mut metadata = compatible_metadata();
        metadata.push(("has_espeak", "1"));
        append_file(&mut builder, "voice/model.onnx", &fake_onnx(&metadata));
        append_file(&mut builder, "voice/tokens.txt", b"_ 0\n");
        append_file(&mut builder, "voice/lexicon.txt", b"ni 1\n");
        append_file(&mut builder, "voice/espeak-ng-data/en_dict", b"dictionary");
        append_file(&mut builder, "voice/dict/lexicon.txt", b"dictionary");
        append_file(&mut builder, "voice/MODEL_CARD", b"License: CC0\n");
        builder.finish().expect("finish tar");
        builder
            .into_inner()
            .expect("encoder")
            .finish()
            .expect("bz2");

        let inspection = inspect_voice_archive(&source).expect("inspection");
        assert!(inspection.compatible, "{:?}", inspection.issues);
        assert_eq!(
            inspection.paths.data_dir.as_deref(),
            Some("voice/espeak-ng-data")
        );
        assert_eq!(inspection.paths.dict_dir.as_deref(), Some("voice/dict"));
        assert!(
            inspection
                .view(Some("token".into()))
                .required_files
                .iter()
                .any(|path| path == "voice/espeak-ng-data")
        );
    }

    #[test]
    fn accepts_multi_speaker_and_rejects_non_vits_metadata() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let source = temporary.path().join("multi.tar.bz2");
        let mut metadata = compatible_metadata();
        metadata.retain(|(key, _)| *key != "n_speakers");
        metadata.push(("n_speakers", "4"));
        write_fixture(&source, &metadata);
        let inspection = inspect_voice_archive(&source).expect("inspection");
        assert!(inspection.compatible, "{:?}", inspection.issues);
        assert_eq!(inspection.speaker_count, 4);
        assert!(inspection.suggested_speaker_id < inspection.speaker_count);

        let source = temporary.path().join("matcha.tar.bz2");
        let mut metadata = compatible_metadata();
        metadata.retain(|(key, _)| *key != "model_type");
        metadata.push(("model_type", "matcha"));
        write_fixture(&source, &metadata);
        let inspection = inspect_voice_archive(&source).expect("inspection");
        assert!(!inspection.compatible);
    }

    #[test]
    fn keqing_uses_official_speaker_id_and_melo_ignores_lfs_placeholder() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let keqing = temporary.path().join("vits-zh-hf-keqing.tar.bz2");
        let encoder = BzEncoder::new(File::create(&keqing).expect("fixture"), Compression::best());
        let mut builder = Builder::new(encoder);
        let metadata = [
            ("model_type", "vits"),
            ("language", "Chinese"),
            ("sample_rate", "22050"),
            ("n_speakers", "804"),
        ];
        append_file(&mut builder, "voice/keqing.onnx", &fake_onnx(&metadata));
        append_file(&mut builder, "voice/tokens.txt", b"_ 0\n");
        builder.finish().expect("finish tar");
        builder
            .into_inner()
            .expect("encoder")
            .finish()
            .expect("bz2");
        let inspection = inspect_voice_archive(&keqing).expect("inspection");
        assert!(inspection.compatible, "{:?}", inspection.issues);
        assert_eq!(inspection.speaker_count, 804);
        assert_eq!(inspection.suggested_speaker_id, 115);

        let melo = temporary.path().join("vits-melo-tts-zh_en.tar.bz2");
        let encoder = BzEncoder::new(File::create(&melo).expect("fixture"), Compression::best());
        let mut builder = Builder::new(encoder);
        let mut metadata = compatible_metadata();
        metadata.push(("license", "MIT license"));
        append_file(
            &mut builder,
            "voice/model.int8.onnx",
            b"version https://git-lfs.github.com/spec/v1\noid sha256:f085f5079e05f039b800aeb542f5253c26a303211b0c6465d0d9387977855a63\nsize 53517430\n",
        );
        append_file(&mut builder, "voice/model.onnx", &fake_onnx(&metadata));
        append_file(&mut builder, "voice/tokens.txt", b"_ 0\n");
        append_file(&mut builder, "voice/lexicon.txt", b"ni 1\n");
        append_file(
            &mut builder,
            "voice/LICENSE",
            b"Permission is hereby granted, free of charge. The software is provided \"as is\".",
        );
        builder.finish().expect("finish tar");
        builder
            .into_inner()
            .expect("encoder")
            .finish()
            .expect("bz2");
        let inspection = inspect_voice_archive(&melo).expect("inspection");
        assert!(inspection.compatible, "{:?}", inspection.issues);
        assert_eq!(inspection.paths.model, "voice/model.onnx");
        assert_eq!(inspection.license_summary, "License: MIT");
        assert!(!inspection.license_warning);
    }

    #[test]
    fn rejects_links_and_nested_archives() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let source = temporary.path().join("unsafe.tar.bz2");
        let encoder = BzEncoder::new(File::create(&source).expect("fixture"), Compression::best());
        let mut builder = Builder::new(encoder);
        let mut header = Header::new_gnu();
        header.set_entry_type(EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_link_name("../outside").expect("link");
        header.set_cksum();
        builder
            .append_data(&mut header, "voice/link", io::empty())
            .expect("append link");
        builder.finish().expect("finish");
        builder
            .into_inner()
            .expect("encoder")
            .finish()
            .expect("bz2");
        assert!(matches!(
            inspect_voice_archive(&source),
            Err(VoiceCatalogError::UnsafeArchive(_))
        ));

        let source = temporary.path().join("nested.tar.bz2");
        let encoder = BzEncoder::new(File::create(&source).expect("fixture"), Compression::best());
        let mut builder = Builder::new(encoder);
        append_file(&mut builder, "voice/nested.zip", b"PK");
        builder.finish().expect("finish");
        builder
            .into_inner()
            .expect("encoder")
            .finish()
            .expect("bz2");
        assert!(matches!(
            inspect_voice_archive(&source),
            Err(VoiceCatalogError::UnsafeArchive(_))
        ));
    }

    #[test]
    fn catalog_reuses_blobs_and_protects_builtin() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let source = temporary.path().join("voice.tar.bz2");
        write_fixture(&source, &compatible_metadata());
        let inspection = inspect_voice_archive(&source).expect("inspection");
        let mut catalog = VoiceCatalog::load(
            temporary.path().join("catalog"),
            temporary.path().join("missing-builtin"),
        )
        .expect("catalog");
        assert!(matches!(
            catalog.import_inspected("Unconfirmed", &source, &inspection, false, 0),
            Err(VoiceCatalogError::LicenseNotAcknowledged)
        ));
        let snapshot = catalog
            .import_inspected("Melo", &source, &inspection, true, 0)
            .expect("first import");
        let first = snapshot.entries.last().expect("imported").clone();
        assert_eq!(snapshot.current_id, BUILTIN_VOICE_ID);
        assert!(catalog.asset(&first.id).is_some());
        let snapshot = catalog
            .import_inspected("Melo Copy", &source, &inspection, true, 0)
            .expect("deduplicated import");
        let second = snapshot.entries.last().expect("second").clone();
        assert_eq!(first.sha256, second.sha256);
        assert!(matches!(
            catalog.import_inspected("mElO", &source, &inspection, true, 0),
            Err(VoiceCatalogError::DuplicateName)
        ));
        catalog.select(&first.id).expect("select");
        catalog.delete(&first.id).expect("delete first reference");
        assert!(catalog.root.join(&first.sha256).is_dir());
        catalog.delete(&second.id).expect("delete final reference");
        assert!(!catalog.root.join(&first.sha256).exists());
        assert!(matches!(
            catalog.delete(BUILTIN_VOICE_ID),
            Err(VoiceCatalogError::Protected)
        ));
    }

    #[test]
    #[ignore = "requires local .tar.bz2 archives listed in HACHIMI_TEST_VOICE_ARCHIVES"]
    fn inspects_real_archives_from_environment() {
        let archives = std::env::var("HACHIMI_TEST_VOICE_ARCHIVES")
            .expect("set HACHIMI_TEST_VOICE_ARCHIVES to semicolon-separated archive paths");
        for path in archives.split(';').filter(|value| !value.is_empty()) {
            let inspection = inspect_voice_archive(Path::new(path)).expect("inspect real archive");
            eprintln!(
                "{}: type={}, speakers={}, suggested={}, license={}",
                inspection.original_file_name,
                inspection.model_type,
                inspection.speaker_count,
                inspection.suggested_speaker_id,
                inspection.license_summary
            );
            assert!(inspection.compatible, "{:?}", inspection.issues);
        }
    }
}
