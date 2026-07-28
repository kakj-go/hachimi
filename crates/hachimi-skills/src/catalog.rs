// SPDX-License-Identifier: Apache-2.0
// Adapted from OpenAI Codex commit 4c43465133428898aa84f0bfc02c306ed65fb66a:
// codex-rs/core-skills/src/{loader,root_loader,service,injection,mention_counts}.rs,
// codex-rs/core-skills/src/loader/{discovery,namespace}.rs, and
// codex-rs/app-server/src/skills_watcher.rs.
// Modified for Hachimi: use a root-confined SkillHost, SQLite path identities,
// Checkout-aware Repo roots, and explicit Skill IDs instead of Codex config/plugin types.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use hachimi_protocol::{
    SkillActivationSource, SkillDiagnosticSeverity, SkillId, SkillRecord, SkillScope,
    SkillToolDependency,
};
use hachimi_storage::StoredSkillRecord;

use crate::{
    ENTRY_FILE, MAX_DEPTH, MAX_FILES, ScanBudget, SkillHost, SkillHostError, SkillRunSelection,
    diagnostic, flatten_tree, hash_tree, now_ms, parse_frontmatter, reject_reparse,
    reject_reparse_chain, scan_tree,
};

const AGENTS_SKILLS_PATH: [&str; 2] = [".agents", "skills"];
const METADATA_PATH: [&str; 2] = ["agents", "openai.yaml"];
const MAX_ROOT_DIRECTORIES: usize = 2_000;
const MAX_ROOT_ENTRIES: usize = 20_000;
const MAX_DEPENDENCIES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillCatalogRoot {
    pub path: PathBuf,
    pub scope: SkillScope,
    pub namespace: Option<String>,
}

impl SkillCatalogRoot {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, scope: SkillScope) -> Self {
        Self {
            path: path.into(),
            scope,
            namespace: None,
        }
    }

    #[must_use]
    pub fn with_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = Some(namespace.into());
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillCatalogContext {
    /// Original Project root. Repo Skills here remain visible from a managed worktree.
    pub project_root: Option<PathBuf>,
    /// Active Local/Managed Worktree root. It has precedence over the original root.
    pub checkout_root: Option<PathBuf>,
}

impl SkillHost {
    /// Replaces app-owned Built-in/User-extra/System/Admin roots and invalidates
    /// the discovered-root snapshot. Missing roots are allowed and simply scan empty.
    pub fn set_catalog_roots(&self, roots: Vec<SkillCatalogRoot>) -> Result<(), SkillHostError> {
        for root in &roots {
            validate_namespace(root.namespace.as_deref())?;
            if root.scope == SkillScope::Repo {
                return Err(SkillHostError::InvalidContent(
                    "Repo Skill roots must be supplied through SkillCatalogContext".into(),
                ));
            }
        }
        *self
            .catalog_roots
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = roots;
        *self
            .discovered_roots
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            BTreeSet::from([self.root.clone()]);
        Ok(())
    }

    pub async fn list_for_context(
        &self,
        context: &SkillCatalogContext,
    ) -> Result<Vec<SkillRecord>, SkillHostError> {
        self.reconcile().await?;
        let roots = self.roots_for_context(context)?;
        self.reconcile_catalog_roots(&roots).await?;

        self.records_for_roots(&roots).await
    }

    /// Reindexes every static or context-bound root registered so far. This is
    /// used by the watcher bridge to emit one global invalidation while each
    /// caller still reloads only the Skills visible in its own context.
    pub async fn list_discovered(&self) -> Result<Vec<SkillRecord>, SkillHostError> {
        self.reconcile().await?;
        let mut roots = self
            .catalog_roots
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        roots.extend(
            self.known_context_roots
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .cloned(),
        );
        dedupe_roots(&mut roots)?;
        self.reconcile_catalog_roots(&roots).await?;
        self.records_for_roots(&roots).await
    }

    async fn records_for_roots(
        &self,
        roots: &[SkillCatalogRoot],
    ) -> Result<Vec<SkillRecord>, SkillHostError> {
        let mut records = self
            .store
            .list_skills()
            .await?
            .into_iter()
            .filter(|stored| skill_is_visible(stored, &self.root, roots))
            .map(|mut stored| {
                stored.record.editable = managed_user_skill(&stored, &self.root);
                stored.record
            })
            .collect::<Vec<_>>();
        records.sort_by(|left, right| {
            scope_rank(left.scope)
                .cmp(&scope_rank(right.scope))
                .then_with(|| left.qualified_name.cmp(&right.qualified_name))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(records)
    }

    /// Codex-style selection: structured IDs resolve first and remain exact;
    /// textual `$name` shortcuts are accepted only when unambiguous.
    pub async fn select_for_run_in_context(
        &self,
        prompt: &str,
        explicit_skill_ids: &[SkillId],
        context: &SkillCatalogContext,
    ) -> Result<(Vec<SkillRecord>, Vec<SkillRunSelection>), SkillHostError> {
        let enabled = self
            .list_for_context(context)
            .await?
            .into_iter()
            .filter(|record| {
                record.enabled
                    && !record
                        .diagnostics
                        .iter()
                        .any(|entry| entry.severity == SkillDiagnosticSeverity::Error)
            })
            .collect::<Vec<_>>();

        let mut selected_ids = BTreeMap::new();
        for skill_id in explicit_skill_ids {
            let record = enabled
                .iter()
                .find(|record| &record.id == skill_id)
                .ok_or_else(|| SkillHostError::NotFound(skill_id.clone()))?;
            selected_ids.insert(record.id.clone(), SkillActivationSource::ExplicitSelection);
        }

        let mut name_counts = BTreeMap::<&str, usize>::new();
        for record in &enabled {
            *name_counts
                .entry(record.qualified_name.as_str())
                .or_default() += 1;
        }
        for record in &enabled {
            if name_counts
                .get(record.qualified_name.as_str())
                .copied()
                .unwrap_or_default()
                == 1
                && record.policy.allows_implicit_invocation()
                && crate::contains_skill_mention(prompt, &record.qualified_name)
            {
                selected_ids
                    .entry(record.id.clone())
                    .or_insert(SkillActivationSource::Mention);
            }
        }

        let mut selected = Vec::with_capacity(selected_ids.len());
        // Preserve catalog precedence, not caller/set ordering.
        for record in &enabled {
            let Some(source) = selected_ids.get(&record.id).copied() else {
                continue;
            };
            let entry = self.read_file(&record.id, ENTRY_FILE).await?;
            let instructions = entry.content.ok_or_else(|| {
                SkillHostError::InvalidContent("SKILL.md must be UTF-8 text".into())
            })?;
            selected.push(SkillRunSelection {
                record: record.clone(),
                instructions,
                revision: entry.revision,
                source,
            });
        }
        Ok((enabled, selected))
    }

    pub(crate) fn checked_read_directory_root(
        &self,
        stored: &StoredSkillRecord,
    ) -> Result<PathBuf, SkillHostError> {
        let path = PathBuf::from(&stored.stable_path);
        reject_reparse(&path)?;
        let canonical = fs::canonicalize(path)?;
        if !canonical.is_dir() {
            return Err(SkillHostError::InvalidPath);
        }
        let roots = self
            .discovered_roots
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for root in roots.iter() {
            if canonical != *root && canonical.starts_with(root) {
                reject_reparse_chain(root, &canonical)?;
                return Ok(canonical);
            }
        }
        Err(SkillHostError::InvalidPath)
    }

    fn roots_for_context(
        &self,
        context: &SkillCatalogContext,
    ) -> Result<Vec<SkillCatalogRoot>, SkillHostError> {
        let mut roots = self
            .catalog_roots
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        // Checkout first: Codex restores root precedence after concurrent scans.
        for path in [
            context.checkout_root.as_ref(),
            context.project_root.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            let root = AGENTS_SKILLS_PATH
                .iter()
                .fold(path.clone(), |path, segment| path.join(segment));
            roots.insert(0, SkillCatalogRoot::new(root, SkillScope::Repo));
        }
        dedupe_roots(&mut roots)?;
        let mut known = self
            .known_context_roots
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        known.extend(
            roots
                .iter()
                .filter(|root| root.scope == SkillScope::Repo)
                .cloned(),
        );
        dedupe_roots(&mut known)?;
        Ok(roots)
    }

    async fn reconcile_catalog_roots(
        &self,
        roots: &[SkillCatalogRoot],
    ) -> Result<(), SkillHostError> {
        for root in roots {
            let Some(canonical_root) = canonical_root_if_directory(&root.path)? else {
                continue;
            };
            self.discovered_roots
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(canonical_root.clone());
            for skill_dir in discover_skill_directories(&canonical_root)? {
                self.index_catalog_skill(root, &skill_dir).await?;
            }
        }
        Ok(())
    }

    async fn index_catalog_skill(
        &self,
        root: &SkillCatalogRoot,
        skill_dir: &Path,
    ) -> Result<(), SkillHostError> {
        let stable_path = skill_dir.to_string_lossy().into_owned();
        let existing = self.store.get_skill_by_path(&stable_path).await?;
        let entry_path = skill_dir.join(ENTRY_FILE);
        let entry = fs::read_to_string(&entry_path)?;
        let (name, description) = parse_frontmatter(&entry)?;
        let namespace = root.namespace.clone();
        let qualified_name = namespace
            .as_ref()
            .map_or_else(|| name.clone(), |namespace| format!("{namespace}:{name}"));
        let mut diagnostics = crate::validate_markdown(&entry, &entry_path, skill_dir);
        if let Err(error) = crate::validate_skill_name(&name) {
            diagnostics.push(diagnostic(
                "skill_name_invalid",
                &error.to_string(),
                Some(ENTRY_FILE),
                SkillDiagnosticSeverity::Error,
            ));
        }
        let (dependencies, dependency_diagnostics) = read_dependencies(skill_dir)?;
        diagnostics.extend(dependency_diagnostics);
        let (interface, policy, metadata_diagnostics) =
            crate::metadata::read_interface_and_policy(skill_dir)?;
        diagnostics.extend(metadata_diagnostics);
        let tree = scan_tree(skill_dir, skill_dir, 0, &mut ScanBudget::default())?;
        let mut index = Vec::new();
        flatten_tree(&tree, &mut index);
        let content_hash = hash_tree(&index);
        let enabled = existing.as_ref().is_none_or(|stored| stored.record.enabled);
        let stored = StoredSkillRecord {
            stable_path,
            record: SkillRecord {
                id: existing
                    .as_ref()
                    .map_or_else(SkillId::random, |stored| stored.record.id.clone()),
                scope: root.scope,
                namespace,
                name,
                qualified_name,
                description,
                interface,
                policy,
                dependencies,
                editable: false,
                enabled,
                content_hash: content_hash.clone(),
                tree_revision: content_hash,
                diagnostics,
                updated_at_ms: now_ms(),
            },
        };
        let saved = self.store.upsert_skill(&stored).await?;
        self.store
            .replace_skill_file_index(&saved.record.id, &index)
            .await?;
        Ok(())
    }
}

fn managed_user_skill(stored: &StoredSkillRecord, user_root: &Path) -> bool {
    stored.record.scope == SkillScope::User
        && Path::new(&stored.stable_path).parent() == Some(user_root)
}

fn skill_is_visible(
    stored: &StoredSkillRecord,
    user_root: &Path,
    roots: &[SkillCatalogRoot],
) -> bool {
    let path = Path::new(&stored.stable_path);
    if !path.is_dir() {
        return false;
    }
    if managed_user_skill(stored, user_root) {
        return true;
    }
    roots.iter().any(|root| {
        fs::canonicalize(&root.path)
            .ok()
            .is_some_and(|root| path.starts_with(&root) && path != root)
    })
}

fn scope_rank(scope: SkillScope) -> u8 {
    match scope {
        SkillScope::Repo => 0,
        SkillScope::User => 1,
        SkillScope::BuiltIn => 2,
        SkillScope::System => 3,
        SkillScope::Admin => 4,
    }
}

fn dedupe_roots(roots: &mut Vec<SkillCatalogRoot>) -> Result<(), SkillHostError> {
    let mut seen = BTreeSet::new();
    roots.retain(|root| {
        let identity = fs::canonicalize(&root.path).unwrap_or_else(|_| root.path.clone());
        seen.insert(identity)
    });
    for root in roots.iter() {
        validate_namespace(root.namespace.as_deref())?;
    }
    Ok(())
}

fn validate_namespace(namespace: Option<&str>) -> Result<(), SkillHostError> {
    let Some(namespace) = namespace else {
        return Ok(());
    };
    if namespace.is_empty()
        || namespace.len() > 63
        || namespace.starts_with('-')
        || namespace.ends_with('-')
        || !namespace
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(SkillHostError::InvalidContent(
            "Skill namespace must be a lowercase slug".into(),
        ));
    }
    Ok(())
}

fn canonical_root_if_directory(path: &Path) -> Result<Option<PathBuf>, SkillHostError> {
    if !path.exists() {
        return Ok(None);
    }
    reject_reparse(path)?;
    let canonical = fs::canonicalize(path)?;
    if !canonical.is_dir() {
        return Err(SkillHostError::InvalidPath);
    }
    Ok(Some(canonical))
}

fn discover_skill_directories(root: &Path) -> Result<Vec<PathBuf>, SkillHostError> {
    let mut pending = vec![(root.to_path_buf(), 0_usize)];
    let mut directories = 0_usize;
    let mut entries = 0_usize;
    let mut skills = Vec::new();
    while let Some((directory, depth)) = pending.pop() {
        directories += 1;
        if directories > MAX_ROOT_DIRECTORIES {
            return Err(SkillHostError::Limit("Skill root directories"));
        }
        reject_reparse(&directory)?;
        if directory.join(ENTRY_FILE).is_file() {
            skills.push(directory.clone());
        }
        if depth >= MAX_DEPTH {
            continue;
        }
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            entries += 1;
            if entries > MAX_ROOT_ENTRIES {
                return Err(SkillHostError::Limit("Skill root entries"));
            }
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            reject_reparse(&entry.path())?;
            pending.push((entry.path(), depth + 1));
        }
    }
    skills.sort();
    skills.truncate(MAX_FILES);
    Ok(skills)
}

fn read_dependencies(
    skill_dir: &Path,
) -> Result<
    (
        Vec<SkillToolDependency>,
        Vec<hachimi_protocol::SkillDiagnostic>,
    ),
    SkillHostError,
> {
    let metadata_path = METADATA_PATH
        .iter()
        .fold(skill_dir.to_path_buf(), |path, segment| path.join(segment));
    if !metadata_path.is_file() {
        return Ok((Vec::new(), Vec::new()));
    }
    let content = fs::read_to_string(&metadata_path)?;
    if content.len() as u64 > crate::MAX_FILE_BYTES {
        return Ok((
            Vec::new(),
            vec![dependency_diagnostic("skill_dependency_metadata_too_large")],
        ));
    }
    let dependencies = parse_dependency_yaml_subset(&content);
    let mut diagnostics = Vec::new();
    if dependencies.len() > MAX_DEPENDENCIES {
        diagnostics.push(dependency_diagnostic("skill_dependency_limit"));
    }
    let dependencies = dependencies
        .into_iter()
        .take(MAX_DEPENDENCIES)
        .filter_map(|dependency| match validate_dependency(&dependency) {
            Ok(()) => Some(dependency),
            Err(code) => {
                diagnostics.push(dependency_diagnostic(code));
                None
            }
        })
        .collect();
    Ok((dependencies, diagnostics))
}

fn dependency_diagnostic(code: &str) -> hachimi_protocol::SkillDiagnostic {
    diagnostic(
        code,
        "Optional Skill dependency metadata is unavailable or invalid",
        Some("agents/openai.yaml"),
        SkillDiagnosticSeverity::Warning,
    )
}

fn validate_dependency(dependency: &SkillToolDependency) -> Result<(), &'static str> {
    if dependency.kind.is_empty()
        || dependency.kind.len() > 64
        || dependency.value.is_empty()
        || dependency.value.len() > 1_024
    {
        return Err("skill_dependency_invalid");
    }
    if !dependency.kind.eq_ignore_ascii_case("mcp") {
        return Ok(());
    }
    match dependency.transport.as_deref().unwrap_or("streamable_http") {
        value if value.eq_ignore_ascii_case("streamable_http") => dependency
            .url
            .as_ref()
            .filter(|value| !value.is_empty())
            .map(|_| ())
            .ok_or("skill_dependency_mcp_url_missing"),
        value if value.eq_ignore_ascii_case("stdio") => dependency
            .command
            .as_ref()
            .filter(|value| !value.is_empty())
            .map(|_| ())
            .ok_or("skill_dependency_mcp_command_missing"),
        _ => Err("skill_dependency_mcp_transport_unsupported"),
    }
}

/// Bounded parser for the exact `dependencies.tools` scalar surface used by
/// Codex `agents/openai.yaml`. Unknown YAML sections are ignored fail-open.
fn parse_dependency_yaml_subset(content: &str) -> Vec<SkillToolDependency> {
    let mut output = Vec::new();
    let mut current: Option<BTreeMap<String, String>> = None;
    let mut in_dependencies = false;
    let mut in_tools = false;
    for raw in content.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len().saturating_sub(trimmed.len());
        if indent == 0 {
            in_dependencies = trimmed == "dependencies:";
            in_tools = false;
            continue;
        }
        if in_dependencies && indent <= 2 && trimmed == "tools:" {
            in_tools = true;
            continue;
        }
        if !in_tools {
            continue;
        }
        if trimmed.starts_with('-') {
            if let Some(fields) = current.take()
                && let Some(dependency) = dependency_from_fields(fields)
            {
                output.push(dependency);
            }
            current = Some(BTreeMap::new());
            let rest = trimmed.trim_start_matches('-').trim();
            if let Some((key, value)) = yaml_scalar(rest) {
                current
                    .as_mut()
                    .expect("current dependency")
                    .insert(key, value);
            }
            continue;
        }
        if let Some((key, value)) = yaml_scalar(trimmed)
            && let Some(fields) = current.as_mut()
        {
            fields.insert(key, value);
        }
    }
    if let Some(fields) = current
        && let Some(dependency) = dependency_from_fields(fields)
    {
        output.push(dependency);
    }
    output
}

fn yaml_scalar(line: &str) -> Option<(String, String)> {
    let (key, value) = line.split_once(':')?;
    let key = key.trim();
    if !matches!(
        key,
        "type" | "value" | "description" | "transport" | "command" | "url"
    ) {
        return None;
    }
    let value = value.trim().trim_matches(['\'', '"']).to_owned();
    Some((key.to_owned(), value))
}

fn dependency_from_fields(mut fields: BTreeMap<String, String>) -> Option<SkillToolDependency> {
    Some(SkillToolDependency {
        kind: fields.remove("type")?,
        value: fields.remove("value")?,
        description: fields
            .remove("description")
            .filter(|value| !value.is_empty()),
        transport: fields.remove("transport").filter(|value| !value.is_empty()),
        command: fields.remove("command").filter(|value| !value.is_empty()),
        url: fields.remove("url").filter(|value| !value.is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hachimi_storage::AgentStore;

    fn write_skill(root: &Path, directory: &str, name: &str) {
        let path = root.join(directory);
        fs::create_dir_all(&path).expect("skill directory");
        fs::write(
            path.join(ENTRY_FILE),
            format!("---\nname: {name}\ndescription: {name} description\n---\n"),
        )
        .expect("skill entry");
    }

    #[tokio::test]
    async fn merges_scopes_and_requires_structured_selection_for_collisions() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = AgentStore::connect_in_memory().await.expect("store");
        let host = SkillHost::new(temp.path().join("user"), store).expect("host");
        let builtin = temp.path().join("builtin");
        let repo = temp.path().join("repo");
        write_skill(&builtin, "shared", "shared");
        write_skill(&repo.join(".agents/skills"), "shared", "shared");
        host.set_catalog_roots(vec![SkillCatalogRoot::new(&builtin, SkillScope::BuiltIn)])
            .expect("roots");
        let context = SkillCatalogContext {
            project_root: Some(repo),
            checkout_root: None,
        };
        let records = host.list_for_context(&context).await.expect("catalog");
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].scope, SkillScope::Repo);
        let (_, text_selected) = host
            .select_for_run_in_context("Use $shared", &[], &context)
            .await
            .expect("text selection");
        assert!(text_selected.is_empty());
        let (_, structured) = host
            .select_for_run_in_context("Use $shared", &[records[0].id.clone()], &context)
            .await
            .expect("structured selection");
        assert_eq!(structured.len(), 1);
        assert_eq!(structured[0].record.id, records[0].id);
    }

    #[tokio::test]
    async fn worktree_context_inherits_original_repo_skills_and_parses_dependencies() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = AgentStore::connect_in_memory().await.expect("store");
        let host = SkillHost::new(temp.path().join("user"), store).expect("host");
        let repo = temp.path().join("repo");
        let checkout = temp.path().join("worktree");
        write_skill(&repo.join(".agents/skills"), "office", "office");
        let metadata = repo.join(".agents/skills/office/agents");
        fs::create_dir_all(&metadata).expect("metadata directory");
        fs::write(
            metadata.join("openai.yaml"),
            "dependencies:\n  tools:\n    - type: mcp\n      value: calendar\n      transport: streamable_http\n      url: https://calendar.example/mcp\n",
        )
        .expect("metadata");
        fs::create_dir_all(&checkout).expect("checkout");
        let records = host
            .list_for_context(&SkillCatalogContext {
                project_root: Some(repo),
                checkout_root: Some(checkout),
            })
            .await
            .expect("catalog");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].dependencies[0].value, "calendar");
        assert!(!records[0].editable);
        let entry = host
            .read_file(&records[0].id, ENTRY_FILE)
            .await
            .expect("external Skill remains readable");
        assert!(matches!(
            host.write_file(&hachimi_protocol::SkillFileWriteRequest {
                skill_id: records[0].id.clone(),
                relative_path: ENTRY_FILE.into(),
                content: entry.content.unwrap_or_default(),
                expected_revision: Some(entry.revision),
            })
            .await,
            Err(SkillHostError::InvalidPath)
        ));
    }

    #[tokio::test]
    async fn bundled_office_skills_are_read_only_disableable_and_implicitly_discoverable() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = AgentStore::connect_in_memory().await.expect("store");
        let host = SkillHost::new(temp.path().join("user"), store).expect("host");
        let bundled = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/desktop/src-tauri/resources/skills/builtin");
        host.set_catalog_roots(vec![SkillCatalogRoot::new(bundled, SkillScope::BuiltIn)])
            .expect("roots");

        let records = host.list().await.expect("catalog");
        let office_names = [
            "office-documents",
            "office-spreadsheets",
            "office-presentations",
            "office-pdf",
            "office-file-organizer",
        ];
        let office = records
            .iter()
            .find(|record| record.name == office_names[0])
            .expect("bundled Office Skill");
        for name in office_names {
            let record = records
                .iter()
                .find(|record| record.name == name)
                .expect("bundled Office Skill");
            assert_eq!(record.scope, SkillScope::BuiltIn);
            assert!(!record.editable);
            assert!(record.enabled);
            assert!(!record.content_hash.is_empty());
            assert_eq!(record.content_hash, record.tree_revision);
            assert!(record.policy.allows_implicit_invocation());
            assert_eq!(
                record.policy.workload,
                Some(hachimi_protocol::WorkloadKind::Office)
            );
        }

        let (_, implicit) = host
            .select_for_run("Use $office-documents")
            .await
            .expect("implicit selection");
        assert_eq!(implicit.len(), 1);
        let (_, explicit) = host
            .select_ids_for_run(std::slice::from_ref(&office.id))
            .await
            .expect("explicit selection");
        assert_eq!(explicit.len(), 1);

        let disabled = host
            .set_enabled(&office.id, false)
            .await
            .expect("disable bundled Skill");
        assert!(!disabled.enabled);
        assert!(matches!(
            host.select_ids_for_run(std::slice::from_ref(&office.id))
                .await,
            Err(SkillHostError::NotFound(_))
        ));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn watcher_registers_context_bound_repo_roots_after_startup() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = AgentStore::connect_in_memory().await.expect("store");
        let host = SkillHost::new(temp.path().join("user"), store).expect("host");
        let repo = temp.path().join("repo");
        let repo_skills = repo.join(".agents/skills");
        write_skill(&repo_skills, "watch-repo", "watch-repo");
        let mut watch = host.watch_changes().expect("watch");
        host.list_for_context(&SkillCatalogContext {
            project_root: Some(repo),
            checkout_root: None,
        })
        .await
        .expect("register Repo context");

        tokio::time::sleep(std::time::Duration::from_millis(750)).await;
        fs::write(
            repo_skills.join("watch-repo/SKILL.md"),
            "---\nname: watch-repo\ndescription: changed\n---\n",
        )
        .expect("external Repo Skill update");
        let changed = tokio::time::timeout(std::time::Duration::from_secs(5), watch.recv())
            .await
            .expect("watch timeout")
            .expect("watch channel");
        assert_eq!(
            changed,
            [fs::canonicalize(repo_skills).expect("canonical root")]
        );
    }

    #[test]
    fn invalid_mcp_dependency_is_diagnostic_not_authorization() {
        let parsed = parse_dependency_yaml_subset(
            "dependencies:\n  tools:\n    - type: mcp\n      value: mail\n      transport: stdio\n",
        );
        assert_eq!(
            validate_dependency(&parsed[0]),
            Err("skill_dependency_mcp_command_missing")
        );
    }
}
