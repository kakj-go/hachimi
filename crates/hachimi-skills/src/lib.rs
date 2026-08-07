//! App-managed User Skills with a root-confined file host.

mod catalog;
mod file_import;
mod metadata;
mod watcher;

pub use catalog::{SkillCatalogContext, SkillCatalogRoot};
pub use watcher::SkillChangeWatch;

use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

use atomic_write_file::AtomicWriteFile;
use hachimi_protocol::{
    SkillDiagnostic, SkillDiagnosticSeverity, SkillEditorKind, SkillEntryCreateRequest,
    SkillEntryKind, SkillEntryRenameRequest, SkillFileSnapshot, SkillFileWriteRequest, SkillId,
    SkillPreviewResource, SkillPreviewResourceRequest, SkillRecord, SkillScope, SkillTreeNode,
};
use hachimi_storage::{AgentStore, AgentStoreError, SkillFileIndexRecord, StoredSkillRecord};
use sha2::{Digest, Sha256};
use thiserror::Error;

const ENTRY_FILE: &str = "SKILL.md";
const TRASH_DIRECTORY: &str = ".hachimi-trash";
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_SKILL_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_FILES: usize = 512;
const MAX_DEPTH: usize = 16;
const SUPPORTED_EXTENSIONS: [&str; 3] = ["md", "js", "py"];

#[derive(Debug, Error)]
pub enum SkillHostError {
    #[error("Skill storage failed: {0}")]
    Store(#[from] AgentStoreError),
    #[error("Skill file operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("native Skill file watching is unavailable on this platform")]
    NativeWatchUnsupported,
    #[error("Skill does not exist: {0}")]
    NotFound(SkillId),
    #[error("invalid Skill name")]
    InvalidName,
    #[error("invalid or out-of-root Skill path")]
    InvalidPath,
    #[error("Skill entry already exists")]
    AlreadyExists,
    #[error("Skill file changed since it was opened")]
    RevisionConflict,
    #[error("Skill content is invalid: {0}")]
    InvalidContent(String),
    #[error("Skill archive is invalid: {0}")]
    InvalidArchive(String),
    #[error("Skill resource limit exceeded: {0}")]
    Limit(&'static str),
}

#[derive(Debug, Clone)]
pub struct SkillHost {
    root: PathBuf,
    store: AgentStore,
    catalog_roots: Arc<RwLock<Vec<SkillCatalogRoot>>>,
    known_context_roots: Arc<RwLock<Vec<SkillCatalogRoot>>>,
    discovered_roots: Arc<RwLock<BTreeSet<PathBuf>>>,
}

#[derive(Debug, Clone)]
pub struct SkillRunSelection {
    pub record: SkillRecord,
    pub instructions: String,
    pub revision: String,
    pub source: hachimi_protocol::SkillActivationSource,
}

impl SkillHost {
    pub fn new(root: impl Into<PathBuf>, store: AgentStore) -> Result<Self, SkillHostError> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        fs::create_dir_all(root.join(TRASH_DIRECTORY))?;
        let root = fs::canonicalize(root)?;
        Ok(Self {
            discovered_roots: Arc::new(RwLock::new(BTreeSet::from([root.clone()]))),
            root,
            store,
            catalog_roots: Arc::new(RwLock::new(Vec::new())),
            known_context_roots: Arc::new(RwLock::new(Vec::new())),
        })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub async fn list(&self) -> Result<Vec<SkillRecord>, SkillHostError> {
        self.list_for_context(&SkillCatalogContext::default()).await
    }

    pub async fn create(&self, name: &str) -> Result<SkillRecord, SkillHostError> {
        validate_skill_name(name)?;
        let directory = self.root.join(name);
        if directory.exists() {
            return Err(SkillHostError::AlreadyExists);
        }
        fs::create_dir(&directory)?;
        let entry = directory.join(ENTRY_FILE);
        let title = title_from_name(name);
        let template = format!(
            "---\nname: {name}\ndescription: 请描述该 Skill 的用途和触发条件\n---\n\n# {title}\n\n在这里编写 Skill 指令。\n"
        );
        if let Err(error) = write_atomic(&entry, template.as_bytes()) {
            let _ = fs::remove_dir(&directory);
            return Err(error.into());
        }
        let stored = StoredSkillRecord {
            stable_path: directory.to_string_lossy().into_owned(),
            record: SkillRecord {
                id: SkillId::random(),
                scope: SkillScope::User,
                namespace: None,
                name: name.to_owned(),
                qualified_name: name.to_owned(),
                description: "请描述该 Skill 的用途和触发条件".into(),
                interface: None,
                policy: hachimi_protocol::SkillPolicy::default(),
                dependencies: Vec::new(),
                editable: true,
                enabled: true,
                content_hash: String::new(),
                tree_revision: String::new(),
                diagnostics: Vec::new(),
                updated_at_ms: now_ms(),
            },
        };
        if let Err(error) = self.store.upsert_skill(&stored).await {
            let _ = fs::remove_dir_all(&directory);
            return Err(error.into());
        }
        match self.reindex(&stored.record.id).await {
            Ok(record) => Ok(record),
            Err(error) => {
                let _ = self.store.remove_skill(&stored.record.id).await;
                let _ = fs::remove_dir_all(&directory);
                Err(error)
            }
        }
    }

    /// Imports one complete User Skill from a bounded ZIP archive. The archive may contain the
    /// Skill files at its root or inside one wrapper directory. Validation is completed in the
    /// internal trash/staging area before a same-volume rename makes the Skill visible.
    pub async fn import_archive(&self, archive_path: &Path) -> Result<SkillRecord, SkillHostError> {
        if archive_path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("zip"))
        {
            return Err(SkillHostError::InvalidArchive(
                "only .zip archives are supported".into(),
            ));
        }
        let metadata = fs::metadata(archive_path)?;
        if !metadata.is_file() || metadata.len() > MAX_ARCHIVE_BYTES {
            return Err(SkillHostError::Limit("archive size"));
        }
        let entries = read_archive_entries(archive_path)?;
        let staging = self
            .root
            .join(TRASH_DIRECTORY)
            .join(format!("import-{}", uuid::Uuid::now_v7()));
        fs::create_dir(&staging)?;
        let imported = (|| -> Result<(String, String), SkillHostError> {
            for entry in &entries {
                let target = staging.join(&entry.relative_path);
                if let Some(bytes) = &entry.bytes {
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    write_atomic(&target, bytes)?;
                } else {
                    fs::create_dir_all(&target)?;
                }
            }
            validate_imported_directory(&staging)
        })();
        let (name, description) = match imported {
            Ok(imported) => imported,
            Err(error) => {
                let _ = fs::remove_dir_all(&staging);
                return Err(error);
            }
        };
        let destination = self.root.join(&name);
        if destination.exists() {
            let _ = fs::remove_dir_all(&staging);
            return Err(SkillHostError::AlreadyExists);
        }
        fs::rename(&staging, &destination)?;
        let stored = StoredSkillRecord {
            stable_path: destination.to_string_lossy().into_owned(),
            record: SkillRecord {
                id: SkillId::random(),
                scope: SkillScope::User,
                namespace: None,
                qualified_name: name.clone(),
                name,
                description,
                interface: None,
                policy: hachimi_protocol::SkillPolicy::default(),
                dependencies: Vec::new(),
                editable: true,
                enabled: true,
                content_hash: String::new(),
                tree_revision: String::new(),
                diagnostics: Vec::new(),
                updated_at_ms: now_ms(),
            },
        };
        if let Err(error) = self.store.upsert_skill(&stored).await {
            let _ = fs::remove_dir_all(&destination);
            return Err(error.into());
        }
        match self.reindex(&stored.record.id).await {
            Ok(record) => Ok(record),
            Err(error) => {
                let _ = self.store.remove_skill(&stored.record.id).await;
                let _ = fs::remove_dir_all(&destination);
                Err(error)
            }
        }
    }

    pub async fn rename(
        &self,
        skill_id: &SkillId,
        new_name: &str,
    ) -> Result<SkillRecord, SkillHostError> {
        validate_skill_name(new_name)?;
        let stored = self.stored(skill_id).await?;
        let source = self.checked_directory_root(&stored)?;
        let destination = source
            .parent()
            .ok_or(SkillHostError::InvalidPath)?
            .join(new_name);
        if destination.exists() {
            return Err(SkillHostError::AlreadyExists);
        }
        fs::rename(&source, &destination)?;
        let entry_path = destination.join(ENTRY_FILE);
        if let Ok(content) = fs::read_to_string(&entry_path) {
            let old_line = format!("name: {}", stored.record.name);
            let new_line = format!("name: {new_name}");
            write_atomic(
                &entry_path,
                content.replacen(&old_line, &new_line, 1).as_bytes(),
            )?;
        }
        let mut renamed = stored;
        renamed.stable_path = destination.to_string_lossy().into_owned();
        renamed.record.name = new_name.to_owned();
        renamed.record.updated_at_ms = now_ms();
        self.store.upsert_skill(&renamed).await?;
        self.reindex(skill_id).await
    }

    pub async fn remove(&self, skill_id: &SkillId) -> Result<bool, SkillHostError> {
        let stored = self.stored(skill_id).await?;
        let (source, owner_root) = self.checked_write_directory_root(&stored)?;
        let trash = owner_root.join(TRASH_DIRECTORY);
        fs::create_dir_all(&trash)?;
        reject_reparse(&trash)?;
        let trash_name = format!("{}-{}", skill_id.as_str(), now_ms());
        fs::rename(source, trash.join(trash_name))?;
        Ok(self.store.remove_skill(skill_id).await?)
    }

    pub async fn set_enabled(
        &self,
        skill_id: &SkillId,
        enabled: bool,
    ) -> Result<SkillRecord, SkillHostError> {
        Ok(self
            .store
            .set_skill_enabled(skill_id, enabled, now_ms())
            .await?)
    }

    /// Resolves `$name` mentions against the enabled Skill snapshot for one Run.
    /// Only the selected entry documents are loaded; nested resources remain deferred.
    pub async fn select_for_run(
        &self,
        prompt: &str,
    ) -> Result<(Vec<SkillRecord>, Vec<SkillRunSelection>), SkillHostError> {
        self.select_for_run_in_context(prompt, &[], &SkillCatalogContext::default())
            .await
    }

    /// Loads the exact enabled Skill snapshot selected by a persisted task definition.
    /// The caller remains responsible for comparing the records with its authorization hash.
    pub async fn select_ids_for_run(
        &self,
        skill_ids: &[SkillId],
    ) -> Result<(Vec<SkillRecord>, Vec<SkillRunSelection>), SkillHostError> {
        self.select_for_run_in_context("", skill_ids, &SkillCatalogContext::default())
            .await
    }

    pub async fn tree(&self, skill_id: &SkillId) -> Result<SkillTreeNode, SkillHostError> {
        let stored = self.stored(skill_id).await?;
        let root = self.checked_read_directory_root(&stored)?;
        let mut budget = ScanBudget::default();
        scan_tree(&root, &root, 0, &mut budget)
    }

    pub async fn read_file(
        &self,
        skill_id: &SkillId,
        relative_path: &str,
    ) -> Result<SkillFileSnapshot, SkillHostError> {
        let stored = self.stored(skill_id).await?;
        let root = self.checked_read_directory_root(&stored)?;
        let path = resolve_existing(&root, relative_path)?;
        let metadata = fs::metadata(&path)?;
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            return Err(SkillHostError::Limit("file size"));
        }
        let bytes = fs::read(&path)?;
        let editor_kind = editor_kind(&path, &bytes);
        // Unsupported extensions remain read-only, but UTF-8 content is still
        // useful to inspect. Binary or invalid UTF-8 files stay unavailable.
        let content = String::from_utf8(bytes.clone()).ok();
        let diagnostics = if editor_kind == SkillEditorKind::Markdown {
            validate_markdown(content.as_deref().unwrap_or_default(), &path, &root)
        } else {
            Vec::new()
        };
        Ok(SkillFileSnapshot {
            skill_id: skill_id.clone(),
            relative_path: normalize_relative(relative_path)?,
            editor_kind,
            content,
            size_bytes: metadata.len(),
            revision: hash_bytes(&bytes),
            diagnostics,
        })
    }

    pub async fn read_preview_resource(
        &self,
        request: &SkillPreviewResourceRequest,
    ) -> Result<SkillPreviewResource, SkillHostError> {
        let stored = self.stored(&request.skill_id).await?;
        let root = self.checked_read_directory_root(&stored)?;
        let source = resolve_existing(&root, &request.source_path)?;
        if source
            .extension()
            .and_then(|extension| extension.to_str())
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("md"))
        {
            return Err(SkillHostError::InvalidPath);
        }
        let (path, relative_path) =
            resolve_markdown_reference(&root, &source, &request.destination)?;
        if !supported_skill_file(&path) {
            return Err(SkillHostError::InvalidContent(
                "Skill files must use .md, .js, or .py".into(),
            ));
        }
        let metadata = fs::metadata(&path)?;
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            return Err(SkillHostError::Limit("preview resource size"));
        }
        revalidate_existing(&root, &path)?;
        let bytes = fs::read(&path)?;
        let revision = hash_bytes(&bytes);
        let kind = editor_kind(&path, &bytes);
        let text = String::from_utf8(bytes).map_err(|_| {
            SkillHostError::InvalidContent("Skill files must contain UTF-8 text".into())
        })?;
        Ok(SkillPreviewResource {
            skill_id: request.skill_id.clone(),
            source_path: normalize_relative(&request.source_path)?,
            relative_path,
            editor_kind: kind,
            text: Some(text),
            media_type: None,
            data_base64: None,
            size_bytes: metadata.len(),
            revision,
        })
    }

    pub async fn write_file(
        &self,
        request: &SkillFileWriteRequest,
    ) -> Result<SkillFileSnapshot, SkillHostError> {
        if request.content.len() as u64 > MAX_FILE_BYTES {
            return Err(SkillHostError::Limit("file size"));
        }
        let stored = self.stored(&request.skill_id).await?;
        let root = self.checked_directory_root(&stored)?;
        let path = resolve_existing(&root, &request.relative_path)?;
        if !supported_skill_file(&path) {
            return Err(SkillHostError::InvalidContent(
                "Skill files must use .md, .js, or .py".into(),
            ));
        }
        if !fs::metadata(&path)?.is_file() {
            return Err(SkillHostError::InvalidPath);
        }
        let existing = fs::read(&path)?;
        if request
            .expected_revision
            .as_ref()
            .is_some_and(|revision| revision != &hash_bytes(&existing))
        {
            return Err(SkillHostError::RevisionConflict);
        }
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        {
            let diagnostics = validate_markdown(&request.content, &path, &root);
            if let Some(error) = diagnostics
                .iter()
                .find(|diagnostic| diagnostic.severity == SkillDiagnosticSeverity::Error)
            {
                return Err(SkillHostError::InvalidContent(error.message.clone()));
            }
            if path.file_name().and_then(|name| name.to_str()) == Some(ENTRY_FILE) {
                let (name, _) = parse_frontmatter(&request.content)?;
                if name != stored.record.name {
                    return Err(SkillHostError::InvalidContent(
                        "SKILL.md name must match the Skill directory".into(),
                    ));
                }
            }
        }
        revalidate_existing(&root, &path)?;
        write_atomic(&path, request.content.as_bytes())?;
        self.reindex(&request.skill_id).await?;
        self.read_file(&request.skill_id, &request.relative_path)
            .await
    }

    pub async fn create_entry(
        &self,
        request: &SkillEntryCreateRequest,
    ) -> Result<SkillTreeNode, SkillHostError> {
        validate_entry_name(&request.name)?;
        if request.name.eq_ignore_ascii_case(ENTRY_FILE) {
            return Err(SkillHostError::InvalidContent(
                "only the Skill root may contain SKILL.md".into(),
            ));
        }
        if request.kind == SkillEntryKind::File && !supported_skill_file(Path::new(&request.name)) {
            return Err(SkillHostError::InvalidContent(
                "Skill files must use .md, .js, or .py".into(),
            ));
        }
        let stored = self.stored(&request.skill_id).await?;
        let root = self.checked_directory_root(&stored)?;
        let parent = if request.parent_path.is_empty() {
            root.clone()
        } else {
            resolve_existing(&root, &request.parent_path)?
        };
        if !parent.is_dir() {
            return Err(SkillHostError::InvalidPath);
        }
        let target = parent.join(&request.name);
        if target.exists() {
            return Err(SkillHostError::AlreadyExists);
        }
        match request.kind {
            SkillEntryKind::File => write_atomic(&target, b"")?,
            SkillEntryKind::Directory => fs::create_dir(&target)?,
        }
        self.reindex(&request.skill_id).await?;
        self.tree(&request.skill_id).await
    }

    pub async fn rename_entry(
        &self,
        request: &SkillEntryRenameRequest,
    ) -> Result<SkillTreeNode, SkillHostError> {
        validate_entry_name(&request.new_name)?;
        if request.relative_path == ENTRY_FILE {
            return Err(SkillHostError::InvalidPath);
        }
        if request.new_name.eq_ignore_ascii_case(ENTRY_FILE) {
            return Err(SkillHostError::InvalidContent(
                "only the Skill root may contain SKILL.md".into(),
            ));
        }
        let stored = self.stored(&request.skill_id).await?;
        let root = self.checked_directory_root(&stored)?;
        let source = resolve_existing(&root, &request.relative_path)?;
        if source.is_file() && !supported_skill_file(Path::new(&request.new_name)) {
            return Err(SkillHostError::InvalidContent(
                "Skill files must use .md, .js, or .py".into(),
            ));
        }
        let parent = source.parent().ok_or(SkillHostError::InvalidPath)?;
        let destination = parent.join(&request.new_name);
        if destination.exists() {
            return Err(SkillHostError::AlreadyExists);
        }
        fs::rename(source, destination)?;
        self.reindex(&request.skill_id).await?;
        self.tree(&request.skill_id).await
    }

    pub async fn remove_entry(
        &self,
        skill_id: &SkillId,
        relative_path: &str,
    ) -> Result<SkillTreeNode, SkillHostError> {
        if relative_path == ENTRY_FILE {
            return Err(SkillHostError::InvalidPath);
        }
        let stored = self.stored(skill_id).await?;
        let root = self.checked_directory_root(&stored)?;
        let source = resolve_existing(&root, relative_path)?;
        let trash = root.join(TRASH_DIRECTORY);
        fs::create_dir_all(&trash)?;
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(SkillHostError::InvalidPath)?
            .to_owned();
        fs::rename(source, trash.join(format!("{}-{name}", now_ms())))?;
        self.reindex(skill_id).await?;
        self.tree(skill_id).await
    }

    pub async fn validate(&self, skill_id: &SkillId) -> Result<SkillRecord, SkillHostError> {
        self.reindex(skill_id).await
    }

    async fn reconcile(&self) -> Result<(), SkillHostError> {
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == TRASH_DIRECTORY || !entry.file_type()?.is_dir() {
                continue;
            }
            let path = entry.path();
            reject_reparse(&path)?;
            let stable_path = fs::canonicalize(&path)?.to_string_lossy().into_owned();
            let stored = self.store.get_skill_by_path(&stable_path).await?;
            let skill_id = if let Some(stored) = stored {
                stored.record.id
            } else {
                let record = StoredSkillRecord {
                    stable_path,
                    record: SkillRecord {
                        id: SkillId::random(),
                        scope: SkillScope::User,
                        namespace: None,
                        qualified_name: name.clone(),
                        name,
                        description: String::new(),
                        interface: None,
                        policy: hachimi_protocol::SkillPolicy::default(),
                        dependencies: Vec::new(),
                        editable: true,
                        enabled: true,
                        content_hash: String::new(),
                        tree_revision: String::new(),
                        diagnostics: Vec::new(),
                        updated_at_ms: now_ms(),
                    },
                };
                self.store.upsert_skill(&record).await?.record.id
            };
            self.reindex(&skill_id).await?;
        }
        Ok(())
    }

    async fn reindex(&self, skill_id: &SkillId) -> Result<SkillRecord, SkillHostError> {
        let mut stored = self.stored(skill_id).await?;
        let root = self.checked_directory_root(&stored)?;
        let tree = {
            let mut budget = ScanBudget::default();
            scan_tree(&root, &root, 0, &mut budget)?
        };
        let mut index = Vec::new();
        flatten_tree(&tree, &mut index);
        let content_hash = hash_tree(&index);
        let entry_path = root.join(ENTRY_FILE);
        let mut diagnostics = Vec::new();
        let mut description = stored.record.description.clone();
        let mut frontmatter_display_name = None;
        if entry_path.is_file() {
            match fs::read_to_string(&entry_path) {
                Ok(entry_content) => {
                    diagnostics.extend(validate_markdown(&entry_content, &entry_path, &root));
                    match parse_frontmatter(&entry_content) {
                        Ok((frontmatter_name, parsed_description)) => {
                            description = parsed_description;
                            frontmatter_display_name =
                                parse_frontmatter_display_name(&entry_content);
                            if frontmatter_name != stored.record.name {
                                diagnostics.push(diagnostic(
                                    "skill_name_mismatch",
                                    "SKILL.md name must match the Skill directory",
                                    Some(ENTRY_FILE),
                                    SkillDiagnosticSeverity::Error,
                                ));
                            }
                        }
                        Err(error) => diagnostics.push(diagnostic(
                            "skill_frontmatter_invalid",
                            &error.to_string(),
                            Some(ENTRY_FILE),
                            SkillDiagnosticSeverity::Error,
                        )),
                    }
                }
                Err(error) => diagnostics.push(diagnostic(
                    "skill_entry_not_utf8",
                    &format!("SKILL.md must be UTF-8 text: {error}"),
                    Some(ENTRY_FILE),
                    SkillDiagnosticSeverity::Error,
                )),
            }
        } else {
            diagnostics.push(diagnostic(
                "skill_entry_missing",
                "Skill directory is missing SKILL.md",
                Some(ENTRY_FILE),
                SkillDiagnosticSeverity::Error,
            ));
        }
        for entry in &index {
            if entry.kind != SkillEntryKind::File
                || entry.editor_kind != SkillEditorKind::Unsupported
            {
                continue;
            }
            let supported = supported_skill_file(Path::new(&entry.relative_path));
            diagnostics.push(diagnostic(
                if supported {
                    "skill_file_not_utf8"
                } else {
                    "skill_file_type_unsupported"
                },
                if supported {
                    "Skill files must contain UTF-8 text"
                } else {
                    "Skill files must use .md, .js, or .py"
                },
                Some(&entry.relative_path),
                SkillDiagnosticSeverity::Error,
            ));
        }
        for entry in &index {
            if entry.editor_kind != SkillEditorKind::Markdown || entry.relative_path == ENTRY_FILE {
                continue;
            }
            let path = root.join(
                entry
                    .relative_path
                    .replace('/', std::path::MAIN_SEPARATOR_STR),
            );
            if let Ok(content) = fs::read_to_string(&path) {
                diagnostics.extend(validate_markdown(&content, &path, &root));
            }
        }
        match metadata::read_interface_and_policy(&root) {
            Ok((mut interface, policy, metadata_diagnostics)) => {
                diagnostics.extend(metadata_diagnostics);
                if let Some(display_name) = frontmatter_display_name {
                    interface.get_or_insert_default().display_name = Some(display_name);
                }
                stored.record.interface = interface;
                stored.record.policy = policy;
            }
            Err(error) => diagnostics.push(diagnostic(
                "skill_metadata_invalid",
                &error.to_string(),
                Some("agents/openai.yaml"),
                SkillDiagnosticSeverity::Error,
            )),
        }
        stored.record.description = description;
        stored.record.content_hash = content_hash.clone();
        stored.record.tree_revision = content_hash;
        stored.record.diagnostics = diagnostics;
        stored.record.updated_at_ms = now_ms();
        let saved = self.store.upsert_skill(&stored).await?;
        self.store
            .replace_skill_file_index(skill_id, &index)
            .await?;
        Ok(saved.record)
    }

    async fn stored(&self, skill_id: &SkillId) -> Result<StoredSkillRecord, SkillHostError> {
        self.store
            .get_skill(skill_id)
            .await?
            .ok_or_else(|| SkillHostError::NotFound(skill_id.clone()))
    }

    fn checked_directory_root(
        &self,
        stored: &StoredSkillRecord,
    ) -> Result<PathBuf, SkillHostError> {
        self.checked_write_directory_root(stored)
            .map(|(directory, _)| directory)
    }
}

#[derive(Debug)]
struct ArchiveEntry {
    relative_path: PathBuf,
    bytes: Option<Vec<u8>>,
}

#[derive(Debug)]
struct RawArchiveEntry {
    segments: Vec<String>,
    bytes: Option<Vec<u8>>,
}

fn read_archive_entries(archive_path: &Path) -> Result<Vec<ArchiveEntry>, SkillHostError> {
    let file = File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|_| SkillHostError::InvalidArchive("the ZIP directory cannot be read".into()))?;
    if archive.is_empty() || archive.len() > MAX_FILES.saturating_mul(2) {
        return Err(SkillHostError::Limit("archive entry count"));
    }
    let mut raw_entries = Vec::with_capacity(archive.len());
    let mut file_count = 0_usize;
    let mut total_bytes = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|_| SkillHostError::InvalidArchive("a ZIP entry cannot be opened".into()))?;
        if entry.unix_mode().is_some_and(|mode| {
            let file_type = mode & 0o170000;
            file_type != 0 && file_type != 0o040000 && file_type != 0o100000
        }) {
            return Err(SkillHostError::InvalidArchive(
                "links and special files are not allowed".into(),
            ));
        }
        let path = entry.enclosed_name().ok_or_else(|| {
            SkillHostError::InvalidArchive("an entry escapes the archive root".into())
        })?;
        let segments = archive_segments(&path)?;
        let bytes = if entry.is_dir() {
            None
        } else {
            file_count = file_count.saturating_add(1);
            if file_count > MAX_FILES {
                return Err(SkillHostError::Limit("file count"));
            }
            if entry.size() > MAX_FILE_BYTES {
                return Err(SkillHostError::Limit("Skill byte budget"));
            }
            let mut bytes = Vec::with_capacity(usize::try_from(entry.size()).unwrap_or(0));
            (&mut entry)
                .take(MAX_FILE_BYTES.saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(SkillHostError::Io)?;
            if bytes.len() as u64 > MAX_FILE_BYTES {
                return Err(SkillHostError::Limit("file size"));
            }
            total_bytes = total_bytes.saturating_add(bytes.len() as u64);
            if total_bytes > MAX_SKILL_BYTES {
                return Err(SkillHostError::Limit("Skill byte budget"));
            }
            Some(bytes)
        };
        raw_entries.push(RawArchiveEntry { segments, bytes });
    }

    let entry_paths = raw_entries
        .iter()
        .filter(|entry| {
            entry.bytes.is_some() && entry.segments.last().is_some_and(|name| name == ENTRY_FILE)
        })
        .map(|entry| entry.segments.clone())
        .collect::<Vec<_>>();
    if entry_paths.len() != 1 {
        return Err(SkillHostError::InvalidArchive(
            "the archive must contain exactly one root SKILL.md".into(),
        ));
    }
    let entry_path = &entry_paths[0];
    let wrapper = match entry_path.as_slice() {
        [entry] if entry == ENTRY_FILE => None,
        [directory, entry] if entry == ENTRY_FILE => Some(directory.as_str()),
        _ => {
            return Err(SkillHostError::InvalidArchive(
                "SKILL.md must be at the archive root or inside one wrapper directory".into(),
            ));
        }
    };

    let mut entries = Vec::with_capacity(raw_entries.len());
    let mut seen = BTreeSet::new();
    for raw in raw_entries {
        let segments = if let Some(wrapper) = wrapper {
            if raw.segments.first().map(String::as_str) != Some(wrapper) {
                return Err(SkillHostError::InvalidArchive(
                    "all entries must stay inside the Skill wrapper directory".into(),
                ));
            }
            &raw.segments[1..]
        } else {
            raw.segments.as_slice()
        };
        if segments.is_empty() {
            continue;
        }
        if segments.len() > MAX_DEPTH {
            return Err(SkillHostError::Limit("directory depth"));
        }
        let relative_path = segments.iter().collect::<PathBuf>();
        if raw.bytes.is_some() {
            if !supported_skill_file(&relative_path) {
                return Err(SkillHostError::InvalidArchive(
                    "Skill files must use .md, .js, or .py".into(),
                ));
            }
            if raw
                .bytes
                .as_deref()
                .is_some_and(|bytes| std::str::from_utf8(bytes).is_err())
            {
                return Err(SkillHostError::InvalidArchive(
                    "Skill files must contain UTF-8 text".into(),
                ));
            }
        }
        let collision_key = segments.join("/").to_ascii_lowercase();
        if !seen.insert(collision_key) {
            return Err(SkillHostError::InvalidArchive(
                "the archive contains duplicate paths".into(),
            ));
        }
        entries.push(ArchiveEntry {
            relative_path,
            bytes: raw.bytes,
        });
    }
    if !entries
        .iter()
        .any(|entry| entry.relative_path == Path::new(ENTRY_FILE) && entry.bytes.is_some())
    {
        return Err(SkillHostError::InvalidArchive(
            "the archive is missing SKILL.md".into(),
        ));
    }
    Ok(entries)
}

fn archive_segments(path: &Path) -> Result<Vec<String>, SkillHostError> {
    let mut segments = Vec::new();
    for component in path.components() {
        let Component::Normal(segment) = component else {
            return Err(SkillHostError::InvalidArchive(
                "archive paths must be relative".into(),
            ));
        };
        let segment = segment
            .to_str()
            .ok_or_else(|| SkillHostError::InvalidArchive("archive paths must be UTF-8".into()))?;
        validate_entry_name(segment).map_err(|_| {
            SkillHostError::InvalidArchive("an archive entry name is invalid".into())
        })?;
        segments.push(segment.to_owned());
    }
    if segments.is_empty() {
        return Err(SkillHostError::InvalidArchive(
            "the archive contains an empty path".into(),
        ));
    }
    Ok(segments)
}

fn validate_imported_directory(root: &Path) -> Result<(String, String), SkillHostError> {
    let entry_path = root.join(ENTRY_FILE);
    let content = fs::read_to_string(&entry_path).map_err(|_| {
        SkillHostError::InvalidArchive("SKILL.md is missing or is not UTF-8".into())
    })?;
    let (name, description) = parse_frontmatter(&content)
        .map_err(|error| SkillHostError::InvalidArchive(error.to_string()))?;
    let mut budget = ScanBudget::default();
    let tree = scan_tree(root, root, 0, &mut budget)?;
    let mut index = Vec::new();
    flatten_tree(&tree, &mut index);
    for entry in index {
        if entry.kind != SkillEntryKind::File {
            continue;
        }
        if entry.editor_kind == SkillEditorKind::Unsupported {
            return Err(SkillHostError::InvalidArchive(
                "every Skill file must be UTF-8 .md, .js, or .py".into(),
            ));
        }
        if entry.editor_kind == SkillEditorKind::Markdown {
            let path = root.join(
                entry
                    .relative_path
                    .replace('/', std::path::MAIN_SEPARATOR_STR),
            );
            let markdown = fs::read_to_string(&path)?;
            if let Some(error) = validate_markdown(&markdown, &path, root)
                .into_iter()
                .find(|diagnostic| diagnostic.severity == SkillDiagnosticSeverity::Error)
            {
                return Err(SkillHostError::InvalidArchive(error.message));
            }
        }
    }
    Ok((name, description))
}

#[derive(Default)]
struct ScanBudget {
    files: usize,
    bytes: u64,
}

fn scan_tree(
    root: &Path,
    path: &Path,
    depth: usize,
    budget: &mut ScanBudget,
) -> Result<SkillTreeNode, SkillHostError> {
    if depth > MAX_DEPTH {
        return Err(SkillHostError::Limit("directory depth"));
    }
    reject_reparse(path)?;
    let metadata = fs::metadata(path)?;
    let relative_path = path
        .strip_prefix(root)
        .map_err(|_| SkillHostError::InvalidPath)?
        .to_string_lossy()
        .replace('\\', "/");
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    if metadata.is_dir() {
        let mut children = Vec::new();
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if entry.file_name().to_string_lossy() == TRASH_DIRECTORY {
                continue;
            }
            children.push(scan_tree(root, &entry.path(), depth + 1, budget)?);
        }
        children.sort_by(|left, right| {
            (
                left.kind != SkillEntryKind::Directory,
                left.name.to_lowercase(),
            )
                .cmp(&(
                    right.kind != SkillEntryKind::Directory,
                    right.name.to_lowercase(),
                ))
        });
        Ok(SkillTreeNode {
            name,
            relative_path,
            kind: SkillEntryKind::Directory,
            editor_kind: SkillEditorKind::Unsupported,
            size_bytes: 0,
            revision: None,
            children,
        })
    } else if metadata.is_file() {
        budget.files += 1;
        budget.bytes = budget.bytes.saturating_add(metadata.len());
        if budget.files > MAX_FILES {
            return Err(SkillHostError::Limit("file count"));
        }
        if budget.bytes > MAX_SKILL_BYTES || metadata.len() > MAX_FILE_BYTES {
            return Err(SkillHostError::Limit("Skill byte budget"));
        }
        let bytes = fs::read(path)?;
        Ok(SkillTreeNode {
            name,
            relative_path,
            kind: SkillEntryKind::File,
            editor_kind: editor_kind(path, &bytes),
            size_bytes: metadata.len(),
            revision: Some(hash_bytes(&bytes)),
            children: Vec::new(),
        })
    } else {
        Err(SkillHostError::InvalidPath)
    }
}

fn flatten_tree(node: &SkillTreeNode, output: &mut Vec<SkillFileIndexRecord>) {
    if !node.relative_path.is_empty() {
        output.push(SkillFileIndexRecord {
            relative_path: node.relative_path.clone(),
            kind: node.kind,
            editor_kind: node.editor_kind,
            size_bytes: node.size_bytes,
            sha256: node.revision.clone(),
            modified_at_ms: now_ms(),
        });
    }
    for child in &node.children {
        flatten_tree(child, output);
    }
}

fn hash_tree(entries: &[SkillFileIndexRecord]) -> String {
    let mut digest = Sha256::new();
    for entry in entries {
        digest.update(entry.relative_path.as_bytes());
        digest.update([0]);
        if let Some(hash) = &entry.sha256 {
            digest.update(hash.as_bytes());
        }
        digest.update([0]);
    }
    hex_digest(&digest.finalize())
}

fn contains_skill_mention(prompt: &str, name: &str) -> bool {
    let needle = format!("${name}");
    prompt.match_indices(&needle).any(|(start, _)| {
        let after = start.saturating_add(needle.len());
        prompt[after..].chars().next().is_none_or(|character| {
            !character.is_ascii_lowercase() && !character.is_ascii_digit() && character != '-'
        })
    })
}

fn validate_skill_name(name: &str) -> Result<(), SkillHostError> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || name.starts_with('-')
        || name.ends_with('-')
    {
        return Err(SkillHostError::InvalidName);
    }
    Ok(())
}

fn validate_entry_name(name: &str) -> Result<(), SkillHostError> {
    let upper = name.trim_end_matches([' ', '.']).to_ascii_uppercase();
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if name.is_empty()
        || name.len() > 128
        || name == "."
        || name == ".."
        || name == TRASH_DIRECTORY
        || name.ends_with([' ', '.'])
        || name
            .chars()
            .any(|character| character.is_control() || "<>:\"/\\|?*".contains(character))
        || reserved.contains(&upper.as_str())
    {
        return Err(SkillHostError::InvalidName);
    }
    Ok(())
}

fn normalize_relative(value: &str) -> Result<String, SkillHostError> {
    if value.is_empty()
        || value.contains('\0')
        || value.contains(':')
        || value.starts_with(['/', '\\'])
    {
        return Err(SkillHostError::InvalidPath);
    }
    let path = Path::new(value);
    let mut segments = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => {
                let segment = segment.to_str().ok_or(SkillHostError::InvalidPath)?;
                validate_entry_name(segment)?;
                segments.push(segment);
            }
            _ => return Err(SkillHostError::InvalidPath),
        }
    }
    if segments.is_empty() || segments.len() > MAX_DEPTH {
        return Err(SkillHostError::InvalidPath);
    }
    Ok(segments.join("/"))
}

fn resolve_existing(root: &Path, relative: &str) -> Result<PathBuf, SkillHostError> {
    let normalized = normalize_relative(relative)?;
    let candidate = root.join(normalized.replace('/', std::path::MAIN_SEPARATOR_STR));
    reject_reparse_chain(root, &candidate)?;
    let canonical = fs::canonicalize(candidate)?;
    if !canonical.starts_with(root) || canonical == root {
        return Err(SkillHostError::InvalidPath);
    }
    Ok(canonical)
}

fn revalidate_existing(root: &Path, path: &Path) -> Result<(), SkillHostError> {
    reject_reparse_chain(root, path)?;
    let canonical = fs::canonicalize(path)?;
    if !canonical.starts_with(root) {
        return Err(SkillHostError::InvalidPath);
    }
    Ok(())
}

fn reject_reparse_chain(root: &Path, path: &Path) -> Result<(), SkillHostError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| SkillHostError::InvalidPath)?;
    let mut current = root.to_path_buf();
    reject_reparse(&current)?;
    for component in relative.components() {
        if let Component::Normal(segment) = component {
            current.push(segment);
            reject_reparse(&current)?;
        } else {
            return Err(SkillHostError::InvalidPath);
        }
    }
    Ok(())
}

fn reject_reparse(path: &Path) -> Result<(), SkillHostError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || is_windows_reparse_point(&metadata) {
        return Err(SkillHostError::InvalidPath);
    }
    Ok(())
}

#[cfg(windows)]
fn is_windows_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
const fn is_windows_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn editor_kind(path: &Path, bytes: &[u8]) -> SkillEditorKind {
    if std::str::from_utf8(bytes).is_err() || !supported_skill_file(path) {
        SkillEditorKind::Unsupported
    } else if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
    {
        SkillEditorKind::Markdown
    } else {
        SkillEditorKind::Text
    }
}

fn supported_skill_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            SUPPORTED_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

fn parse_frontmatter(content: &str) -> Result<(String, String), SkillHostError> {
    let mut lines = content.lines();
    if lines.next() != Some("---") {
        return Err(SkillHostError::InvalidContent(
            "SKILL.md must begin with YAML frontmatter".into(),
        ));
    }
    let mut name = None;
    let mut description = None;
    let mut closed = false;
    for line in lines {
        if line.trim() == "---" {
            closed = true;
            break;
        }
        if let Some(value) = line.strip_prefix("name:") {
            name = Some(value.trim().trim_matches(['\'', '"']).to_owned());
        }
        if let Some(value) = line.strip_prefix("description:") {
            description = Some(value.trim().trim_matches(['\'', '"']).to_owned());
        }
    }
    if !closed {
        return Err(SkillHostError::InvalidContent(
            "SKILL.md frontmatter is not closed".into(),
        ));
    }
    let name = name.filter(|value| !value.is_empty()).ok_or_else(|| {
        SkillHostError::InvalidContent("SKILL.md frontmatter requires name".into())
    })?;
    validate_skill_name(&name)?;
    let description = description
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            SkillHostError::InvalidContent("SKILL.md frontmatter requires description".into())
        })?;
    Ok((name, description))
}

fn parse_frontmatter_display_name(content: &str) -> Option<String> {
    content
        .lines()
        .skip(1)
        .take_while(|line| line.trim() != "---")
        .find_map(|line| line.strip_prefix("display_name:"))
        .map(|value| value.trim().trim_matches(['\'', '"']).to_owned())
        .filter(|value| !value.is_empty())
}

fn validate_markdown(content: &str, file_path: &Path, root: &Path) -> Vec<SkillDiagnostic> {
    let mut diagnostics = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('<')
            && trimmed.chars().nth(1).is_some_and(|character| {
                character.is_ascii_alphabetic() || matches!(character, '/' | '!')
            })
        {
            diagnostics.push(diagnostic(
                "markdown_raw_html",
                "Raw HTML is disabled in Skill Markdown",
                file_path.strip_prefix(root).ok().and_then(Path::to_str),
                SkillDiagnosticSeverity::Error,
            ));
        }
        if let Some((_, destination)) = trimmed.split_once("]:") {
            validate_destination(destination.trim(), file_path, root, &mut diagnostics);
        }
    }
    let mut rest = content;
    while let Some(index) = rest.find("](") {
        rest = &rest[index + 2..];
        let Some(end) = rest.find(')') else { break };
        validate_destination(&rest[..end], file_path, root, &mut diagnostics);
        rest = &rest[end + 1..];
    }
    diagnostics
}

fn validate_destination(
    raw: &str,
    file_path: &Path,
    root: &Path,
    diagnostics: &mut Vec<SkillDiagnostic>,
) {
    let destination = raw
        .trim()
        .trim_matches(['<', '>'])
        .split_whitespace()
        .next()
        .unwrap_or_default();
    if destination.is_empty() || destination.starts_with('#') {
        return;
    }
    let lowered = destination.to_ascii_lowercase();
    let unsafe_target = destination.starts_with(['/', '\\'])
        || destination.contains(':')
        || destination.contains('%')
        || destination.split(['/', '\\']).any(|part| part == "..")
        || lowered.starts_with("file:")
        || lowered.starts_with("http:")
        || lowered.starts_with("https:")
        || lowered.starts_with("//");
    let relative_file = file_path
        .parent()
        .and_then(|parent| parent.strip_prefix(root).ok());
    let normalized = relative_file
        .unwrap_or_else(|| Path::new(""))
        .join(destination.split('#').next().unwrap_or_default());
    if unsafe_target || normalize_reference(&normalized).is_none() {
        diagnostics.push(diagnostic(
            "markdown_reference_outside_skill",
            "Markdown references must stay inside the current Skill directory",
            file_path.strip_prefix(root).ok().and_then(Path::to_str),
            SkillDiagnosticSeverity::Error,
        ));
        return;
    }
    let target = root.join(normalized);
    if !target.exists() {
        diagnostics.push(diagnostic(
            "markdown_reference_missing",
            "Referenced Skill resource does not exist yet",
            file_path.strip_prefix(root).ok().and_then(Path::to_str),
            SkillDiagnosticSeverity::Warning,
        ));
    }
}

fn resolve_markdown_reference(
    root: &Path,
    source: &Path,
    raw_destination: &str,
) -> Result<(PathBuf, String), SkillHostError> {
    let destination = raw_destination
        .trim()
        .trim_matches(['<', '>'])
        .split_whitespace()
        .next()
        .unwrap_or_default();
    let lowered = destination.to_ascii_lowercase();
    if destination.is_empty()
        || destination.starts_with('#')
        || destination.starts_with(['/', '\\'])
        || destination.contains(':')
        || destination.contains('%')
        || destination.split(['/', '\\']).any(|part| part == "..")
        || lowered.starts_with("file:")
        || lowered.starts_with("http:")
        || lowered.starts_with("https:")
        || lowered.starts_with("//")
    {
        return Err(SkillHostError::InvalidPath);
    }
    let source_parent = source.parent().ok_or(SkillHostError::InvalidPath)?;
    let source_parent = source_parent
        .strip_prefix(root)
        .map_err(|_| SkillHostError::InvalidPath)?;
    let requested = source_parent.join(destination.split('#').next().unwrap_or_default());
    let normalized = normalize_reference(&requested).ok_or(SkillHostError::InvalidPath)?;
    let relative_path = normalized
        .components()
        .map(|component| match component {
            Component::Normal(segment) => segment.to_str().ok_or(SkillHostError::InvalidPath),
            _ => Err(SkillHostError::InvalidPath),
        })
        .collect::<Result<Vec<_>, _>>()?
        .join("/");
    let path = resolve_existing(root, &relative_path)?;
    Ok((path, relative_path))
}

fn normalize_reference(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(segment) => normalized.push(segment),
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(normalized)
}

fn diagnostic(
    code: &str,
    message: &str,
    path: Option<&str>,
    severity: SkillDiagnosticSeverity,
) -> SkillDiagnostic {
    SkillDiagnostic {
        code: code.into(),
        message: message.into(),
        relative_path: path.map(str::to_owned),
        severity,
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    let mut file = AtomicWriteFile::open(path)?;
    file.write_all(bytes)?;
    file.flush()?;
    file.commit()
}

fn hash_bytes(bytes: &[u8]) -> String {
    hex_digest(&Sha256::digest(bytes))
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn title_from_name(name: &str) -> String {
    name.split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_ascii_uppercase().to_string() + chars.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn now_ms() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::{ZipWriter, write::SimpleFileOptions};

    async fn host() -> (tempfile::TempDir, SkillHost) {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = AgentStore::connect_in_memory().await.expect("store");
        let host = SkillHost::new(directory.path().join("skills/user"), store).expect("host");
        (directory, host)
    }

    fn write_zip(path: &Path, entries: &[(&str, &str)]) {
        let file = File::create(path).expect("archive file");
        let mut writer = ZipWriter::new(file);
        for (name, content) in entries {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .expect("archive entry");
            writer
                .write_all(content.as_bytes())
                .expect("archive content");
        }
        writer.finish().expect("finish archive");
    }

    fn find_tree_node<'a>(tree: &'a SkillTreeNode, path: &str) -> Option<&'a SkillTreeNode> {
        if tree.relative_path == path {
            return Some(tree);
        }
        tree.children
            .iter()
            .find_map(|child| find_tree_node(child, path))
    }

    #[test]
    fn reads_optional_user_facing_display_name() {
        assert_eq!(
            parse_frontmatter_display_name(
                "---\nname: reports\ndisplay_name: Report Studio\ndescription: Build reports\n---\n"
            )
            .as_deref(),
            Some("Report Studio")
        );
    }

    #[tokio::test]
    async fn creates_standard_entry_and_indexes_nested_resources() {
        let (_directory, host) = host().await;
        let skill = host.create("release-notes").await.expect("create");
        let snapshot = host.read_file(&skill.id, ENTRY_FILE).await.expect("entry");
        assert!(snapshot.content.unwrap().contains("name: release-notes"));
        host.create_entry(&SkillEntryCreateRequest {
            skill_id: skill.id.clone(),
            parent_path: String::new(),
            name: "references".into(),
            kind: SkillEntryKind::Directory,
        })
        .await
        .expect("directory");
        let tree = host.tree(&skill.id).await.expect("tree");
        assert!(tree.children.iter().any(|node| node.name == "references"));
        assert_eq!(host.list().await.expect("list").len(), 1);
    }

    #[tokio::test]
    async fn imports_dropped_text_files_into_an_existing_directory() {
        let (directory, host) = host().await;
        let skill = host.create("drop-import").await.expect("create");
        host.create_entry(&SkillEntryCreateRequest {
            skill_id: skill.id.clone(),
            parent_path: String::new(),
            name: "references".into(),
            kind: SkillEntryKind::Directory,
        })
        .await
        .expect("target directory");
        let source_root = directory.path().join("windows-drop");
        fs::create_dir(&source_root).expect("source directory");
        let markdown = source_root.join("guide.md");
        let python = source_root.join("check.py");
        fs::write(&markdown, "# Guide\n").expect("markdown");
        fs::write(&python, "print('ok')\n").expect("python");

        let tree = host
            .import_files(&skill.id, "references", &[markdown, python])
            .await
            .expect("import files");

        assert!(find_tree_node(&tree, "references/guide.md").is_some());
        assert!(find_tree_node(&tree, "references/check.py").is_some());
    }

    #[tokio::test]
    async fn dropped_files_reject_unsupported_types_and_nested_skill_entries() {
        let (directory, host) = host().await;
        let skill = host.create("safe-drop").await.expect("create");
        let unsupported = directory.path().join("notes.txt");
        fs::write(&unsupported, "unsupported").expect("source");
        assert!(matches!(
            host.import_files(&skill.id, "", &[unsupported]).await,
            Err(SkillHostError::InvalidContent(_))
        ));

        let nested_entry = directory.path().join("SKILL.md");
        fs::write(
            &nested_entry,
            "---\nname: other\ndescription: nested\n---\n",
        )
        .expect("nested entry source");
        assert!(matches!(
            host.import_files(&skill.id, "", &[nested_entry]).await,
            Err(SkillHostError::InvalidContent(_))
        ));
    }

    #[tokio::test]
    async fn imports_a_valid_wrapped_zip_after_full_validation() {
        let (directory, host) = host().await;
        let archive = directory.path().join("import.zip");
        write_zip(
            &archive,
            &[
                (
                    "release-helper/SKILL.md",
                    "---\nname: release-helper\ndescription: Prepare a release\n---\n\n# Release\n\nSee [guide](docs/guide.md).\n",
                ),
                ("release-helper/docs/guide.md", "# Guide\n"),
                ("release-helper/scripts/check.py", "print('ok')\n"),
                (
                    "release-helper/scripts/render.js",
                    "export const ok = true;\n",
                ),
            ],
        );

        let imported = host.import_archive(&archive).await.expect("import");

        assert_eq!(imported.name, "release-helper");
        assert_eq!(imported.description, "Prepare a release");
        assert!(imported.diagnostics.is_empty());
        let tree = host.tree(&imported.id).await.expect("tree");
        assert!(find_tree_node(&tree, "scripts/check.py").is_some());
        assert_eq!(host.list().await.expect("list").len(), 1);
    }

    #[tokio::test]
    async fn rejects_archives_without_a_valid_entry_or_with_unsupported_files() {
        let (directory, host) = host().await;
        let missing = directory.path().join("missing.zip");
        write_zip(&missing, &[("helper/main.py", "print('missing entry')")]);
        assert!(matches!(
            host.import_archive(&missing).await,
            Err(SkillHostError::InvalidArchive(_))
        ));

        let unsupported = directory.path().join("unsupported.zip");
        write_zip(
            &unsupported,
            &[
                (
                    "SKILL.md",
                    "---\nname: unsafe-skill\ndescription: Invalid file test\n---\n",
                ),
                ("notes.txt", "not supported"),
            ],
        );
        assert!(matches!(
            host.import_archive(&unsupported).await,
            Err(SkillHostError::InvalidArchive(_))
        ));
        assert!(host.list().await.expect("list").is_empty());
    }

    #[tokio::test]
    async fn create_and_rename_reject_unsupported_file_extensions() {
        let (_directory, host) = host().await;
        let skill = host.create("file-types").await.expect("create");
        assert!(matches!(
            host.create_entry(&SkillEntryCreateRequest {
                skill_id: skill.id.clone(),
                parent_path: String::new(),
                name: "notes.txt".into(),
                kind: SkillEntryKind::File,
            })
            .await,
            Err(SkillHostError::InvalidContent(_))
        ));
        host.create_entry(&SkillEntryCreateRequest {
            skill_id: skill.id.clone(),
            parent_path: String::new(),
            name: "notes.py".into(),
            kind: SkillEntryKind::File,
        })
        .await
        .expect("supported file");
        assert!(matches!(
            host.rename_entry(&SkillEntryRenameRequest {
                skill_id: skill.id.clone(),
                relative_path: "notes.py".into(),
                new_name: "notes.txt".into(),
            })
            .await,
            Err(SkillHostError::InvalidContent(_))
        ));
        let root = host.stored(&skill.id).await.expect("stored").stable_path;
        fs::write(Path::new(&root).join("external.txt"), "not exposed").expect("external file");
        let validated = host.validate(&skill.id).await.expect("validate");
        assert!(
            validated
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "skill_file_type_unsupported")
        );
        let unsupported = host
            .read_file(&skill.id, "external.txt")
            .await
            .expect("unsupported metadata");
        assert_eq!(unsupported.editor_kind, SkillEditorKind::Unsupported);
        assert_eq!(unsupported.content.as_deref(), Some("not exposed"));
    }

    #[tokio::test]
    async fn rejects_remote_and_parent_markdown_references() {
        let (_directory, host) = host().await;
        let skill = host.create("safe-skill").await.expect("create");
        let snapshot = host.read_file(&skill.id, ENTRY_FILE).await.expect("entry");
        for reference in [
            "[outside](../secret.txt)",
            "[absolute](/etc/passwd)",
            "[file](file:///C:/secret.txt)",
            "[remote](https://example.test)",
            "[encoded](%2e%2e/secret.txt)",
            "[unc](//server/share.txt)",
        ] {
            let mut content = snapshot.content.clone().unwrap();
            content.push_str(reference);
            let result = host
                .write_file(&SkillFileWriteRequest {
                    skill_id: skill.id.clone(),
                    relative_path: ENTRY_FILE.into(),
                    content,
                    expected_revision: Some(snapshot.revision.clone()),
                })
                .await;
            assert!(matches!(result, Err(SkillHostError::InvalidContent(_))));
        }
    }

    #[tokio::test]
    async fn optimistic_revision_prevents_silent_overwrite() {
        let (_directory, host) = host().await;
        let skill = host.create("revision-check").await.expect("create");
        let snapshot = host.read_file(&skill.id, ENTRY_FILE).await.expect("entry");
        let request = SkillFileWriteRequest {
            skill_id: skill.id,
            relative_path: ENTRY_FILE.into(),
            content: snapshot.content.clone().unwrap() + "\nUpdated\n",
            expected_revision: Some("stale".into()),
        };
        assert!(matches!(
            host.write_file(&request).await,
            Err(SkillHostError::RevisionConflict)
        ));
    }

    #[tokio::test]
    async fn run_selection_loads_only_explicit_enabled_mentions() {
        let (_directory, host) = host().await;
        let selected = host.create("release-notes").await.expect("selected");
        let disabled = host.create("private-notes").await.expect("disabled");
        host.set_enabled(&disabled.id, false)
            .await
            .expect("disable");

        let (enabled, selections) = host
            .select_for_run("Use $release-notes, not $private-notes or $release-notes-extra.")
            .await
            .expect("select");

        assert_eq!(enabled.len(), 1);
        assert_eq!(selections.len(), 1);
        assert_eq!(selections[0].record.id, selected.id);
        assert!(selections[0].instructions.contains("name: release-notes"));
    }

    #[tokio::test]
    async fn scheduled_selection_loads_exact_authorized_ids_without_prompt_mentions() {
        let (_directory, host) = host().await;
        let selected = host.create("daily-office").await.expect("selected");
        let unrelated = host.create("unrelated").await.expect("unrelated");

        let (enabled, selections) = host
            .select_ids_for_run(std::slice::from_ref(&selected.id))
            .await
            .expect("select by persisted ID");

        assert_eq!(enabled.len(), 2);
        assert_eq!(selections.len(), 1);
        assert_eq!(selections[0].record.id, selected.id);
        assert_ne!(selections[0].record.id, unrelated.id);
        assert!(selections[0].instructions.contains("name: daily-office"));
    }

    #[tokio::test]
    async fn external_directory_without_entry_is_diagnostic_only() {
        let (_directory, host) = host().await;
        let external = host.root().join("external-skill");
        fs::create_dir(&external).expect("external directory");

        let skills = host.list().await.expect("list");
        let skill = skills
            .iter()
            .find(|skill| skill.name == "external-skill")
            .expect("diagnostic Skill");
        assert!(!external.join(ENTRY_FILE).exists());
        assert!(skill.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "skill_entry_missing"
                && diagnostic.severity == SkillDiagnosticSeverity::Error
        }));
        let (_, selected) = host
            .select_for_run("Use $external-skill")
            .await
            .expect("selection");
        assert!(
            selected.is_empty(),
            "invalid Skills are never loaded into a Run"
        );
    }

    #[tokio::test]
    async fn frontend_paths_reject_absolute_unc_ads_and_parent_forms() {
        let (_directory, host) = host().await;
        let skill = host.create("path-check").await.expect("create");
        for path in [
            "../SKILL.md",
            "/SKILL.md",
            r"C:\\SKILL.md",
            r"\\server\share\SKILL.md",
            "SKILL.md:secret",
        ] {
            assert!(matches!(
                host.read_file(&skill.id, path).await,
                Err(SkillHostError::InvalidPath)
            ));
        }
    }

    #[tokio::test]
    async fn preview_resources_are_resolved_by_the_host_without_file_urls() {
        let (_temp, host) = host().await;
        let skill = host.create("preview-skill").await.expect("create");
        host.create_entry(&SkillEntryCreateRequest {
            skill_id: skill.id.clone(),
            parent_path: String::new(),
            name: "reference.py".into(),
            kind: SkillEntryKind::File,
        })
        .await
        .expect("text resource");
        let text = host
            .read_file(&skill.id, "reference.py")
            .await
            .expect("read text");
        host.write_file(&SkillFileWriteRequest {
            skill_id: skill.id.clone(),
            relative_path: "reference.py".into(),
            content: "bounded preview".into(),
            expected_revision: Some(text.revision),
        })
        .await
        .expect("write text");

        let preview = host
            .read_preview_resource(&SkillPreviewResourceRequest {
                skill_id: skill.id.clone(),
                source_path: ENTRY_FILE.into(),
                destination: "reference.py".into(),
            })
            .await
            .expect("preview");
        assert_eq!(preview.text.as_deref(), Some("bounded preview"));
        assert_eq!(preview.relative_path, "reference.py");
        assert!(preview.data_base64.is_none());
        assert!(matches!(
            host.read_preview_resource(&SkillPreviewResourceRequest {
                skill_id: skill.id,
                source_path: ENTRY_FILE.into(),
                destination: "file:///C:/secret.txt".into(),
            })
            .await,
            Err(SkillHostError::InvalidPath)
        ));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn native_watcher_reports_external_skill_changes() {
        let (_temp, host) = host().await;
        let skill = host.create("watched-skill").await.expect("create");
        let mut watch = host.watch_changes().expect("watch");
        let root = host.stored(&skill.id).await.expect("stored").stable_path;
        fs::write(Path::new(&root).join("external.py"), "changed").expect("external write");

        let changed = tokio::time::timeout(std::time::Duration::from_secs(5), watch.recv())
            .await
            .expect("watch timeout")
            .expect("watch channel");
        assert_eq!(changed, [host.root().to_path_buf()]);
    }
}
