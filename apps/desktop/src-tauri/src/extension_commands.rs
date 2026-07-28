use std::collections::{BTreeMap, BTreeSet};

use hachimi_control_plane::McpSecretResolver;
use hachimi_protocol::{
    McpHeaderInput, McpHeaderView, McpServerId, ProjectId, SkillEntryCreateRequest,
    SkillEntryRenameRequest, SkillFileSnapshot, SkillFileWriteRequest, SkillId,
    SkillPreviewResource, SkillPreviewResourceRequest, SkillRecord, SkillSubscriptionId,
    SkillTreeNode,
};

use super::*;

const MCP_HEADER_SERVICE: &str = "com.hachimi.mcp-header";
const SKILL_TRASH_DIRECTORY: &str = ".hachimi-trash";
pub(super) const SKILL_CHANGE_EVENT: &str = "skills:changed";

pub(super) fn start_skill_change_bridge(app: AppHandle, host: hachimi_skills::SkillHost) {
    tauri::async_runtime::spawn(async move {
        let mut watch = match host.watch_changes() {
            Ok(watch) => watch,
            Err(error) => {
                tracing::error!(%error, "native Skill watcher failed to start");
                return;
            }
        };
        let mut known = match host.list_discovered().await {
            Ok(records) => records
                .into_iter()
                .map(|record| (record.id.to_string(), record))
                .collect::<BTreeMap<_, _>>(),
            Err(error) => {
                tracing::warn!(%error, "initial Skill watcher index failed");
                BTreeMap::new()
            }
        };
        while let Some(mut changed_paths) = watch.recv().await {
            tokio::time::sleep(Duration::from_millis(120)).await;
            while let Some(paths) = watch.try_recv() {
                changed_paths.extend(paths);
            }
            changed_paths.sort();
            changed_paths.dedup();
            let current = match host.list_discovered().await {
                Ok(records) => records
                    .into_iter()
                    .map(|record| (record.id.to_string(), record))
                    .collect::<BTreeMap<_, _>>(),
                Err(error) => {
                    tracing::warn!(%error, "Skill watcher reindex failed");
                    continue;
                }
            };
            let mut events = Vec::new();
            for (id, record) in &current {
                let kind = match known.get(id) {
                    None => hachimi_protocol::SkillChangeKind::Created,
                    Some(previous) if previous.name != record.name => {
                        hachimi_protocol::SkillChangeKind::Renamed
                    }
                    Some(previous) if previous.tree_revision != record.tree_revision => {
                        hachimi_protocol::SkillChangeKind::Reindexed
                    }
                    Some(_) => continue,
                };
                let mut relative_paths =
                    skill_relative_paths(host.root(), &record.name, changed_paths.as_slice());
                if let Some(previous) = known.get(id)
                    && previous.name != record.name
                {
                    relative_paths.extend(skill_relative_paths(
                        host.root(),
                        &previous.name,
                        changed_paths.as_slice(),
                    ));
                    relative_paths.sort();
                    relative_paths.dedup();
                }
                events.push(hachimi_protocol::SkillChangeEvent {
                    skill_id: record.id.clone(),
                    relative_paths,
                    kind,
                    tree_revision: record.tree_revision.clone(),
                });
            }
            for (id, previous) in &known {
                if !current.contains_key(id) {
                    events.push(hachimi_protocol::SkillChangeEvent {
                        skill_id: previous.id.clone(),
                        relative_paths: skill_relative_paths(
                            host.root(),
                            &previous.name,
                            changed_paths.as_slice(),
                        ),
                        kind: hachimi_protocol::SkillChangeKind::Removed,
                        tree_revision: previous.tree_revision.clone(),
                    });
                }
            }
            known = current;
            if !events.is_empty() {
                let labels = app
                    .state::<DesktopState>()
                    .skill_subscriptions
                    .lock()
                    .values()
                    .cloned()
                    .collect::<BTreeSet<_>>();
                for label in labels {
                    let _ = app.emit_to(&label, SKILL_CHANGE_EVENT, events.clone());
                }
            }
        }
    });
}

fn skill_relative_paths(root: &Path, skill_name: &str, paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .filter_map(|path| path.strip_prefix(root).ok())
        .filter_map(|relative| {
            let mut components = relative.components();
            let first = components.next()?.as_os_str().to_str()?;
            if first != skill_name {
                return None;
            }
            let value = components
                .map(|component| component.as_os_str().to_str())
                .collect::<Option<Vec<_>>>()?
                .join("/");
            (!value.is_empty() && !value.starts_with(SKILL_TRASH_DIRECTORY)).then_some(value)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct McpKeyring;

impl McpKeyring {
    fn entry(reference: &str) -> Result<keyring::Entry, CommandError> {
        keyring::Entry::new(MCP_HEADER_SERVICE, reference)
            .map_err(|error| CommandError::operation("mcp_secret_store_failed", error))
    }

    pub(super) fn get(&self, reference: &str) -> Result<Option<String>, CommandError> {
        match Self::entry(reference)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(CommandError::operation("mcp_secret_store_failed", error)),
        }
    }

    pub(super) fn set(&self, reference: &str, value: &str) -> Result<(), CommandError> {
        Self::entry(reference)?
            .set_password(value)
            .map_err(|error| CommandError::operation("mcp_secret_store_failed", error))
    }

    pub(super) fn clear(&self, reference: &str) -> Result<(), CommandError> {
        match Self::entry(reference)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(CommandError::operation("mcp_secret_store_failed", error)),
        }
    }

    pub(super) fn prepare_headers(
        &self,
        server_id: &McpServerId,
        inputs: &[McpHeaderInput],
        existing: &[McpHeaderView],
    ) -> Result<(Vec<McpHeaderView>, Vec<String>), CommandError> {
        let mut names = BTreeSet::new();
        let mut views = Vec::new();
        let mut created_references = Vec::new();
        for input in inputs {
            let name = input.name.trim();
            let normalized = name.to_ascii_lowercase();
            if name.is_empty() || !names.insert(normalized.clone()) {
                return Err(CommandError::new(
                    "mcp_header_invalid",
                    "MCP header names must be non-empty and unique",
                ));
            }
            let secret = input.secret || sensitive_header_name(name);
            if secret {
                let previous = existing
                    .iter()
                    .find(|header| header.name.eq_ignore_ascii_case(name) && header.secret);
                let (reference, configured) = if let Some(value) = input.value.as_deref() {
                    if value.is_empty() {
                        return Err(CommandError::new(
                            "mcp_header_invalid",
                            "Secret header values cannot be empty; remove the row to clear it",
                        ));
                    }
                    let reference = format!(
                        "{}:{}:{}",
                        server_id.as_str(),
                        normalized,
                        uuid::Uuid::now_v7()
                    );
                    self.set(&reference, value)?;
                    created_references.push(reference.clone());
                    (reference, true)
                } else if let Some(previous) = previous {
                    (
                        previous.credential_reference.clone().ok_or_else(|| {
                            CommandError::new(
                                "mcp_header_secret_missing",
                                "The saved secret header reference is missing",
                            )
                        })?,
                        previous.configured,
                    )
                } else {
                    return Err(CommandError::new(
                        "mcp_header_secret_missing",
                        "A new secret header requires a value",
                    ));
                };
                views.push(McpHeaderView {
                    name: name.into(),
                    value: None,
                    secret: true,
                    configured,
                    credential_reference: Some(reference),
                });
            } else {
                let value = input.value.clone().or_else(|| {
                    existing
                        .iter()
                        .find(|header| header.name.eq_ignore_ascii_case(name) && !header.secret)
                        .and_then(|header| header.value.clone())
                });
                let value = value.ok_or_else(|| {
                    CommandError::new("mcp_header_invalid", "Header value is required")
                })?;
                if value.contains(['\r', '\n', '\0']) {
                    return Err(CommandError::new(
                        "mcp_header_invalid",
                        "Header values cannot contain line breaks",
                    ));
                }
                views.push(McpHeaderView {
                    name: name.into(),
                    value: Some(value),
                    secret: false,
                    configured: true,
                    credential_reference: None,
                });
            }
        }
        Ok((views, created_references))
    }

    pub(super) fn resolve_inputs(
        &self,
        inputs: &[McpHeaderInput],
        existing: &[McpHeaderView],
    ) -> Result<BTreeMap<String, String>, CommandError> {
        let mut resolved = BTreeMap::new();
        for input in inputs {
            let name = input.name.trim();
            let secret = input.secret || sensitive_header_name(name);
            let value = if let Some(value) = input.value.clone() {
                value
            } else if let Some(previous) = existing
                .iter()
                .find(|header| header.name.eq_ignore_ascii_case(name))
            {
                if secret {
                    let reference = previous.credential_reference.as_deref().ok_or_else(|| {
                        CommandError::new(
                            "mcp_header_secret_missing",
                            "The saved secret header reference is missing",
                        )
                    })?;
                    self.get(reference)?.ok_or_else(|| {
                        CommandError::new(
                            "mcp_header_secret_missing",
                            "The saved secret header is unavailable",
                        )
                    })?
                } else {
                    previous.value.clone().unwrap_or_default()
                }
            } else {
                return Err(CommandError::new(
                    "mcp_header_invalid",
                    "Header value is required",
                ));
            };
            resolved.insert(name.into(), value);
        }
        Ok(resolved)
    }

    pub(super) fn cleanup_replaced(
        &self,
        previous: &[McpHeaderView],
        current: &[McpHeaderView],
    ) -> Vec<String> {
        let retained = current
            .iter()
            .filter_map(|header| header.credential_reference.as_deref())
            .collect::<BTreeSet<_>>();
        previous
            .iter()
            .filter_map(|header| header.credential_reference.as_deref())
            .filter(|reference| !retained.contains(*reference))
            .filter_map(|reference| self.clear(reference).err().map(|_| reference.to_owned()))
            .collect()
    }
}

pub(super) async fn defer_mcp_secret_cleanup_failures(store: &AgentStore, references: Vec<String>) {
    let attempted_at_ms = i64::try_from(epoch_millis()).unwrap_or(i64::MAX);
    for reference in references {
        if let Err(error) = store
            .defer_mcp_keyring_cleanup(&reference, attempted_at_ms)
            .await
        {
            tracing::warn!(%error, "failed to persist deferred MCP credential cleanup");
        }
    }
}

pub(super) async fn retry_deferred_mcp_secret_cleanup(store: &AgentStore, keyring: McpKeyring) {
    let references = match store.list_pending_mcp_keyring_cleanup(256).await {
        Ok(references) => references,
        Err(error) => {
            tracing::warn!(%error, "failed to load deferred MCP credential cleanup");
            return;
        }
    };
    for reference in references {
        if keyring.clear(&reference).is_ok() {
            if let Err(error) = store.complete_mcp_keyring_cleanup(&reference).await {
                tracing::warn!(%error, "failed to complete deferred MCP credential cleanup");
            }
        } else {
            defer_mcp_secret_cleanup_failures(store, vec![reference]).await;
        }
    }
}

impl McpSecretResolver for McpKeyring {
    fn resolve(&self, credential_reference: &str) -> Result<Option<String>, String> {
        self.get(credential_reference)
            .map_err(|error| error.message)
    }

    fn persist(&self, credential_reference: &str, value: &str) -> Result<(), String> {
        self.set(credential_reference, value)
            .map_err(|error| error.message)
    }

    fn delete(&self, credential_reference: &str) -> Result<(), String> {
        self.clear(credential_reference)
            .map_err(|error| error.message)
    }
}

fn sensitive_header_name(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    matches!(
        lowered.as_str(),
        "authorization" | "proxy-authorization" | "cookie" | "x-api-key"
    ) || lowered.contains("token")
        || lowered.contains("secret")
        || lowered.ends_with("-key")
}

fn require_skills_window(window: &WebviewWindow, state: &DesktopState) -> Result<(), CommandError> {
    state.authorize(window, ControlMethod::SkillsManage)?;
    require_window(window, "workbench")
}

async fn dispatch_skills(
    window: &WebviewWindow,
    state: &DesktopState,
    request: hachimi_control_plane::SkillsAppRequest,
) -> Result<hachimi_control_plane::SkillsAppResponse, CommandError> {
    let client = state.authorize(window, ControlMethod::SkillsManage)?;
    require_window(window, "workbench")?;
    let context = hachimi_control_plane::AppServerContext {
        principal: client.client_id.0.clone(),
        client,
    };
    match state
        .app_server
        .dispatch(
            &context,
            hachimi_control_plane::AppServerRequest::Domain(Box::new(
                hachimi_control_plane::AppServerDomainRequest::Skills(request),
            )),
        )
        .await
        .map_err(|error| CommandError::operation("skills_app_server_failed", error))?
    {
        hachimi_control_plane::AppServerResponse::Domain(response) => match *response {
            hachimi_control_plane::AppServerDomainResponse::Skills(response) => Ok(response),
            _ => Err(CommandError::new(
                "skills_app_server_protocol_mismatch",
                "App Server returned a response for a different domain",
            )),
        },
        _ => Err(CommandError::new(
            "skills_app_server_protocol_mismatch",
            "App Server returned a response for a different domain",
        )),
    }
}

#[tauri::command]
pub(super) async fn list_skills(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    project_id: Option<ProjectId>,
) -> Result<Vec<SkillRecord>, CommandError> {
    match dispatch_skills(
        &window,
        &state,
        hachimi_control_plane::SkillsAppRequest::List(project_id),
    )
    .await?
    {
        hachimi_control_plane::SkillsAppResponse::Skills(skills) => Ok(skills),
        _ => Err(CommandError::new(
            "skills_response_mismatch",
            "expected Skill list",
        )),
    }
}

#[tauri::command]
pub(super) async fn create_skill(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    name: String,
) -> Result<SkillRecord, CommandError> {
    match dispatch_skills(
        &window,
        &state,
        hachimi_control_plane::SkillsAppRequest::Create { name },
    )
    .await?
    {
        hachimi_control_plane::SkillsAppResponse::Skill(skill) => Ok(skill),
        _ => Err(CommandError::new(
            "skills_response_mismatch",
            "expected Skill",
        )),
    }
}

#[tauri::command]
pub(super) async fn import_skill_archive(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Option<SkillRecord>, CommandError> {
    require_skills_window(&window, &state)?;
    let selected = match desktop_e2e_path("HACHIMI_DESKTOP_E2E_SKILL_ARCHIVE_PATH") {
        Some(path) => Some(path),
        None => rfd::AsyncFileDialog::new()
            .add_filter("Skill ZIP", &["zip"])
            .pick_file()
            .await
            .map(|handle| handle.path().to_path_buf()),
    };
    let Some(archive) = selected else {
        return Ok(None);
    };
    state
        .skill_host
        .import_archive(&archive)
        .await
        .map(Some)
        .map_err(|error| CommandError::operation("skill_import_failed", error))
}

#[tauri::command]
pub(super) async fn import_skill_dropped_files(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    drop_token: String,
    skill_id: SkillId,
    parent_path: String,
) -> Result<SkillTreeNode, CommandError> {
    require_skills_window(&window, &state)?;
    if drop_token.len() > 64 {
        return Err(CommandError::new(
            "skill_drop_invalid",
            "The native file drop token is invalid",
        ));
    }
    let pending = {
        let now = Instant::now();
        let mut drops = state.pending_skill_drops.lock();
        drops.retain(|_, drop| drop.expires_at > now);
        drops.remove(&drop_token)
    }
    .ok_or_else(|| {
        CommandError::new(
            "skill_drop_expired",
            "The native file drop has expired; drop the files again",
        )
    })?;
    if pending.owner != window.label() || pending.expires_at <= Instant::now() {
        return Err(CommandError::new(
            "skill_drop_invalid",
            "The native file drop does not belong to this window",
        ));
    }
    state
        .skill_host
        .import_files(&skill_id, &parent_path, &pending.paths)
        .await
        .map_err(|error| CommandError::operation("skill_files_import_failed", error))
}

#[tauri::command]
pub(super) async fn rename_skill(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    skill_id: SkillId,
    name: String,
) -> Result<SkillRecord, CommandError> {
    match dispatch_skills(
        &window,
        &state,
        hachimi_control_plane::SkillsAppRequest::Rename { skill_id, name },
    )
    .await?
    {
        hachimi_control_plane::SkillsAppResponse::Skill(skill) => Ok(skill),
        _ => Err(CommandError::new(
            "skills_response_mismatch",
            "expected Skill",
        )),
    }
}

#[tauri::command]
pub(super) async fn remove_skill(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    skill_id: SkillId,
) -> Result<bool, CommandError> {
    match dispatch_skills(
        &window,
        &state,
        hachimi_control_plane::SkillsAppRequest::Remove(skill_id),
    )
    .await?
    {
        hachimi_control_plane::SkillsAppResponse::Removed(removed) => Ok(removed),
        _ => Err(CommandError::new(
            "skills_response_mismatch",
            "expected removal result",
        )),
    }
}

#[tauri::command]
pub(super) async fn set_skill_enabled(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    skill_id: SkillId,
    enabled: bool,
) -> Result<SkillRecord, CommandError> {
    match dispatch_skills(
        &window,
        &state,
        hachimi_control_plane::SkillsAppRequest::SetEnabled { skill_id, enabled },
    )
    .await?
    {
        hachimi_control_plane::SkillsAppResponse::Skill(skill) => Ok(skill),
        _ => Err(CommandError::new(
            "skills_response_mismatch",
            "expected Skill",
        )),
    }
}

#[tauri::command]
pub(super) async fn get_skill_tree(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    skill_id: SkillId,
) -> Result<SkillTreeNode, CommandError> {
    match dispatch_skills(
        &window,
        &state,
        hachimi_control_plane::SkillsAppRequest::Tree(skill_id),
    )
    .await?
    {
        hachimi_control_plane::SkillsAppResponse::Tree(tree) => Ok(tree),
        _ => Err(CommandError::new(
            "skills_response_mismatch",
            "expected Skill tree",
        )),
    }
}

#[tauri::command]
pub(super) async fn read_skill_file(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    skill_id: SkillId,
    relative_path: String,
) -> Result<SkillFileSnapshot, CommandError> {
    match dispatch_skills(
        &window,
        &state,
        hachimi_control_plane::SkillsAppRequest::ReadFile {
            skill_id,
            relative_path,
        },
    )
    .await?
    {
        hachimi_control_plane::SkillsAppResponse::File(file) => Ok(file),
        _ => Err(CommandError::new(
            "skills_response_mismatch",
            "expected Skill file",
        )),
    }
}

#[tauri::command]
pub(super) async fn read_skill_preview_resource(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: SkillPreviewResourceRequest,
) -> Result<SkillPreviewResource, CommandError> {
    match dispatch_skills(
        &window,
        &state,
        hachimi_control_plane::SkillsAppRequest::ReadPreviewResource(request),
    )
    .await?
    {
        hachimi_control_plane::SkillsAppResponse::PreviewResource(resource) => Ok(resource),
        _ => Err(CommandError::new(
            "skills_response_mismatch",
            "expected preview resource",
        )),
    }
}

#[tauri::command]
pub(super) async fn write_skill_file(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: SkillFileWriteRequest,
) -> Result<SkillFileSnapshot, CommandError> {
    match dispatch_skills(
        &window,
        &state,
        hachimi_control_plane::SkillsAppRequest::WriteFile(request),
    )
    .await?
    {
        hachimi_control_plane::SkillsAppResponse::File(file) => Ok(file),
        _ => Err(CommandError::new(
            "skills_response_mismatch",
            "expected Skill file",
        )),
    }
}

#[tauri::command]
pub(super) async fn create_skill_entry(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: SkillEntryCreateRequest,
) -> Result<SkillTreeNode, CommandError> {
    match dispatch_skills(
        &window,
        &state,
        hachimi_control_plane::SkillsAppRequest::CreateEntry(request),
    )
    .await?
    {
        hachimi_control_plane::SkillsAppResponse::Tree(tree) => Ok(tree),
        _ => Err(CommandError::new(
            "skills_response_mismatch",
            "expected Skill tree",
        )),
    }
}

#[tauri::command]
pub(super) async fn rename_skill_entry(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: SkillEntryRenameRequest,
) -> Result<SkillTreeNode, CommandError> {
    match dispatch_skills(
        &window,
        &state,
        hachimi_control_plane::SkillsAppRequest::RenameEntry(request),
    )
    .await?
    {
        hachimi_control_plane::SkillsAppResponse::Tree(tree) => Ok(tree),
        _ => Err(CommandError::new(
            "skills_response_mismatch",
            "expected Skill tree",
        )),
    }
}

#[tauri::command]
pub(super) async fn remove_skill_entry(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    skill_id: SkillId,
    relative_path: String,
) -> Result<SkillTreeNode, CommandError> {
    match dispatch_skills(
        &window,
        &state,
        hachimi_control_plane::SkillsAppRequest::RemoveEntry {
            skill_id,
            relative_path,
        },
    )
    .await?
    {
        hachimi_control_plane::SkillsAppResponse::Tree(tree) => Ok(tree),
        _ => Err(CommandError::new(
            "skills_response_mismatch",
            "expected Skill tree",
        )),
    }
}

#[tauri::command]
pub(super) async fn validate_skill(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    skill_id: SkillId,
) -> Result<SkillRecord, CommandError> {
    match dispatch_skills(
        &window,
        &state,
        hachimi_control_plane::SkillsAppRequest::Validate(skill_id),
    )
    .await?
    {
        hachimi_control_plane::SkillsAppResponse::Skill(skill) => Ok(skill),
        _ => Err(CommandError::new(
            "skills_response_mismatch",
            "expected Skill",
        )),
    }
}

#[tauri::command]
pub(super) fn subscribe_skills(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<SkillSubscriptionId, CommandError> {
    require_skills_window(&window, &state)?;
    let id = SkillSubscriptionId::random();
    state
        .skill_subscriptions
        .lock()
        .insert(id.clone(), window.label().to_owned());
    Ok(id)
}

#[tauri::command]
pub(super) fn unsubscribe_skills(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    subscription_id: SkillSubscriptionId,
) -> Result<bool, CommandError> {
    require_skills_window(&window, &state)?;
    let mut subscriptions = state.skill_subscriptions.lock();
    if subscriptions
        .get(&subscription_id)
        .is_some_and(|label| label == window.label())
    {
        subscriptions.remove(&subscription_id);
        Ok(true)
    } else {
        Ok(false)
    }
}
