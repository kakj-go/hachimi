use std::fs::OpenOptions;

use super::*;

impl SkillHost {
    /// Imports files selected by a trusted desktop drag operation into an
    /// existing Skill directory. Sources are read-only inputs; every target is
    /// validated against the managed Skill root before any file is created.
    pub async fn import_files(
        &self,
        skill_id: &SkillId,
        parent_path: &str,
        sources: &[PathBuf],
    ) -> Result<SkillTreeNode, SkillHostError> {
        if sources.is_empty() || sources.len() > MAX_FILES {
            return Err(SkillHostError::Limit("import file count"));
        }
        let stored = self.stored(skill_id).await?;
        let root = self.checked_directory_root(&stored)?;
        let parent = if parent_path.is_empty() {
            root.clone()
        } else {
            resolve_existing(&root, parent_path)?
        };
        if !parent.is_dir() {
            return Err(SkillHostError::InvalidPath);
        }

        let current_tree = self.tree(skill_id).await?;
        let (current_files, current_bytes) = tree_usage(&current_tree);
        let mut incoming_names = BTreeSet::new();
        let mut imports = Vec::with_capacity(sources.len());
        let mut incoming_bytes = 0_u64;
        for source in sources {
            reject_reparse(source)?;
            let metadata = fs::symlink_metadata(source)?;
            if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
                return Err(SkillHostError::Limit("file size"));
            }
            let name = source
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or(SkillHostError::InvalidName)?;
            validate_entry_name(name)?;
            if name.eq_ignore_ascii_case(ENTRY_FILE) {
                return Err(SkillHostError::InvalidContent(
                    "only the Skill root may contain SKILL.md".into(),
                ));
            }
            if !supported_skill_file(Path::new(name)) {
                return Err(SkillHostError::InvalidContent(
                    "Skill files must use .md, .js, or .py".into(),
                ));
            }
            if !incoming_names.insert(name.to_ascii_lowercase()) || parent.join(name).exists() {
                return Err(SkillHostError::AlreadyExists);
            }
            let bytes = fs::read(source)?;
            reject_reparse(source)?;
            let content = std::str::from_utf8(&bytes).map_err(|_| {
                SkillHostError::InvalidContent("Skill files must be UTF-8 text".into())
            })?;
            let target = parent.join(name);
            if target
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
                && let Some(error) = validate_markdown(content, &target, &root)
                    .into_iter()
                    .find(|diagnostic| diagnostic.severity == SkillDiagnosticSeverity::Error)
            {
                return Err(SkillHostError::InvalidContent(error.message));
            }
            incoming_bytes = incoming_bytes.saturating_add(metadata.len());
            imports.push((name.to_owned(), bytes));
        }
        if current_files.saturating_add(imports.len()) > MAX_FILES {
            return Err(SkillHostError::Limit("file count"));
        }
        if current_bytes.saturating_add(incoming_bytes) > MAX_SKILL_BYTES {
            return Err(SkillHostError::Limit("Skill byte budget"));
        }

        revalidate_existing(&root, &parent)?;
        let mut created = Vec::with_capacity(imports.len());
        for (name, bytes) in imports {
            if let Err(error) = revalidate_existing(&root, &parent) {
                rollback_imported_files(&created);
                return Err(error);
            }
            let target = parent.join(name);
            if target.exists() {
                rollback_imported_files(&created);
                return Err(SkillHostError::AlreadyExists);
            }
            if let Err(error) = write_new_file(&target, &bytes) {
                let _ = fs::remove_file(&target);
                rollback_imported_files(&created);
                return Err(error.into());
            }
            created.push(target);
        }
        if let Err(error) = self.reindex(skill_id).await {
            rollback_imported_files(&created);
            let _ = self.reindex(skill_id).await;
            return Err(error);
        }
        self.tree(skill_id).await
    }
}

fn tree_usage(node: &SkillTreeNode) -> (usize, u64) {
    let own = if node.kind == SkillEntryKind::File {
        (1, node.size_bytes)
    } else {
        (0, 0)
    };
    node.children.iter().fold(own, |(files, bytes), child| {
        let (child_files, child_bytes) = tree_usage(child);
        (
            files.saturating_add(child_files),
            bytes.saturating_add(child_bytes),
        )
    })
}

fn rollback_imported_files(paths: &[PathBuf]) {
    for path in paths.iter().rev() {
        let _ = fs::remove_file(path);
    }
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.flush()?;
    file.sync_all()
}
