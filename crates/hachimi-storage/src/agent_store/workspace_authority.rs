use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use hachimi_protocol::{
    AgentPermissionPolicy, AgentWorkspace, AgentWorkspaceKind, AgentWorkspaceOwner,
    AgentWorkspaceStatus, ApprovalRequestRecord, RunAuthoritySnapshot, RunId, RunOrigin, RunRecord,
    ScheduleId, SessionId, SkillId, WorkspaceId,
};
use sqlx::Row;

use super::{AgentStore, AgentStoreError};

#[derive(Debug, Clone, Copy)]
pub enum WorkspaceOwnerRef<'a> {
    Session(&'a SessionId),
    Schedule(&'a ScheduleId),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkspaceReconciliationReport {
    pub removed_rows: u64,
    pub removed_directories: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHostAuthorizationSummary {
    pub id: String,
    pub action: String,
    pub resource: String,
    pub target_host: String,
    pub granted_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedAgentWorkspace {
    pub workspace: AgentWorkspace,
    pub created_directory: bool,
}

impl WorkspaceOwnerRef<'_> {
    fn kind(self) -> &'static str {
        match self {
            Self::Session(_) => "session",
            Self::Schedule(_) => "schedule",
        }
    }

    fn id(&self) -> &str {
        match self {
            Self::Session(id) => id.as_str(),
            Self::Schedule(id) => id.as_str(),
        }
    }

    fn owned(self) -> AgentWorkspaceOwner {
        match self {
            Self::Session(id) => AgentWorkspaceOwner::Session {
                session_id: id.clone(),
            },
            Self::Schedule(id) => AgentWorkspaceOwner::Schedule {
                schedule_id: id.clone(),
            },
        }
    }
}

impl AgentStore {
    #[must_use]
    pub fn managed_workspace_path(&self, workspace_id: &WorkspaceId) -> PathBuf {
        let root = if self.managed_artifacts.transient {
            self.managed_artifact_root().join("agent-workspaces")
        } else {
            self.managed_artifact_root()
                .parent()
                .unwrap_or_else(|| self.managed_artifact_root())
                .join("agent-workspaces")
        };
        root.join(workspace_id.as_str())
    }

    pub async fn ensure_managed_workspace(
        &self,
        workspace_id: WorkspaceId,
        owner: WorkspaceOwnerRef<'_>,
        timestamp_ms: i64,
    ) -> Result<AgentWorkspace, AgentStoreError> {
        let workspace_id = self
            .workspace_for_owner(owner)
            .await?
            .map_or(workspace_id, |workspace| workspace.id);
        let root = self.managed_workspace_path(&workspace_id);
        validate_existing_path_chain(root.parent().ok_or_else(|| {
            AgentStoreError::InvalidPersistedValue {
                kind: "managed workspace parent",
                value: root.display().to_string(),
            }
        })?)
        .map_err(|value| AgentStoreError::InvalidPersistedValue {
            kind: "managed workspace root",
            value,
        })?;
        let created_directory = match std::fs::symlink_metadata(&root) {
            Ok(metadata) => {
                validate_workspace_metadata(&root, &metadata).map_err(|value| {
                    AgentStoreError::InvalidPersistedValue {
                        kind: "managed workspace root",
                        value,
                    }
                })?;
                false
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(error) => return Err(AgentStoreError::Io(error)),
        };
        std::fs::create_dir_all(&root)?;
        validate_existing_path_chain(&root).map_err(|value| {
            AgentStoreError::InvalidPersistedValue {
                kind: "managed workspace root",
                value,
            }
        })?;
        let result = self
            .upsert_workspace(
                workspace_id,
                AgentWorkspaceKind::Managed,
                owner,
                &root,
                timestamp_ms,
            )
            .await;
        if result.is_err() && created_directory {
            let _ = std::fs::remove_dir(&root);
        }
        result
    }

    pub fn prepare_managed_workspace(
        &self,
        workspace_id: WorkspaceId,
        owner: WorkspaceOwnerRef<'_>,
        timestamp_ms: i64,
    ) -> Result<PreparedAgentWorkspace, AgentStoreError> {
        let root = self.managed_workspace_path(&workspace_id);
        validate_existing_path_chain(root.parent().ok_or_else(|| {
            AgentStoreError::InvalidPersistedValue {
                kind: "managed workspace parent",
                value: root.display().to_string(),
            }
        })?)
        .map_err(|value| AgentStoreError::InvalidPersistedValue {
            kind: "managed workspace root",
            value,
        })?;
        let created_directory = match std::fs::symlink_metadata(&root) {
            Ok(metadata) => {
                validate_workspace_metadata(&root, &metadata).map_err(|value| {
                    AgentStoreError::InvalidPersistedValue {
                        kind: "managed workspace root",
                        value,
                    }
                })?;
                false
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(error) => return Err(AgentStoreError::Io(error)),
        };
        std::fs::create_dir_all(&root)?;
        validate_existing_path_chain(&root).map_err(|value| {
            AgentStoreError::InvalidPersistedValue {
                kind: "managed workspace root",
                value,
            }
        })?;
        Ok(PreparedAgentWorkspace {
            workspace: AgentWorkspace {
                id: workspace_id,
                kind: AgentWorkspaceKind::Managed,
                owner: owner.owned(),
                root_path: root.to_string_lossy().into_owned(),
                status: AgentWorkspaceStatus::Ready,
                status_reason: None,
                created_at_ms: timestamp_ms,
                updated_at_ms: timestamp_ms,
            },
            created_directory,
        })
    }

    pub fn discard_prepared_workspace(
        &self,
        prepared: &PreparedAgentWorkspace,
    ) -> Result<bool, AgentStoreError> {
        if !prepared.created_directory {
            return Ok(false);
        }
        let expected = self.managed_workspace_path(&prepared.workspace.id);
        if Path::new(&prepared.workspace.root_path) != expected {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "prepared managed workspace root",
                value: prepared.workspace.root_path.clone(),
            });
        }
        if expected.is_dir() {
            std::fs::remove_dir(&expected)?;
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn ensure_selected_workspace(
        &self,
        workspace_id: WorkspaceId,
        owner: WorkspaceOwnerRef<'_>,
        root: &Path,
        timestamp_ms: i64,
    ) -> Result<AgentWorkspace, AgentStoreError> {
        let previous = self.workspace_for_owner(owner).await?;
        let workspace_id = previous
            .as_ref()
            .map_or(workspace_id, |workspace| workspace.id.clone());
        let canonical = match validate_selected_workspace_root(root) {
            Ok(canonical) => canonical,
            Err(error) => {
                self.persist_unavailable_workspace(workspace_id, owner, root, &error, timestamp_ms)
                    .await?;
                return Err(AgentStoreError::InvalidPersistedValue {
                    kind: "workspace root",
                    value: error,
                });
            }
        };
        if !canonical.is_dir() {
            let reason = format!("selected directory is unavailable: {}", root.display());
            self.persist_unavailable_workspace(workspace_id, owner, root, &reason, timestamp_ms)
                .await?;
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "workspace root",
                value: root.display().to_string(),
            });
        }
        let selected = self
            .upsert_workspace(
                workspace_id,
                AgentWorkspaceKind::SelectedDirectory,
                owner,
                &canonical,
                timestamp_ms,
            )
            .await?;
        if let Some(previous) = previous
            && previous.kind == AgentWorkspaceKind::Managed
        {
            self.remove_managed_workspace_directory(&previous)?;
        }
        Ok(selected)
    }

    async fn upsert_workspace(
        &self,
        workspace_id: WorkspaceId,
        kind: AgentWorkspaceKind,
        owner: WorkspaceOwnerRef<'_>,
        root: &Path,
        timestamp_ms: i64,
    ) -> Result<AgentWorkspace, AgentStoreError> {
        let root_path = root.to_string_lossy().into_owned();
        let kind_db = match kind {
            AgentWorkspaceKind::Managed => "managed",
            AgentWorkspaceKind::SelectedDirectory => "selected_directory",
        };
        sqlx::query("INSERT INTO agent_workspaces(id, kind, owner_kind, owner_id, root_path, status, status_reason, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, 'ready', NULL, ?, ?) ON CONFLICT(owner_kind, owner_id) DO UPDATE SET root_path = excluded.root_path, kind = excluded.kind, status = 'ready', status_reason = NULL, updated_at_ms = excluded.updated_at_ms")
            .bind(workspace_id.as_str())
            .bind(kind_db)
            .bind(owner.kind())
            .bind(owner.id())
            .bind(&root_path)
            .bind(timestamp_ms)
            .bind(timestamp_ms)
            .execute(self.pool())
            .await?;
        Ok(AgentWorkspace {
            id: workspace_id,
            kind,
            owner: owner.owned(),
            root_path,
            status: AgentWorkspaceStatus::Ready,
            status_reason: None,
            created_at_ms: timestamp_ms,
            updated_at_ms: timestamp_ms,
        })
    }

    pub async fn workspace_for_owner(
        &self,
        owner: WorkspaceOwnerRef<'_>,
    ) -> Result<Option<AgentWorkspace>, AgentStoreError> {
        let row =
            sqlx::query("SELECT * FROM agent_workspaces WHERE owner_kind = ? AND owner_id = ?")
                .bind(owner.kind())
                .bind(owner.id())
                .fetch_optional(self.pool())
                .await?;
        row.map(|row| {
            let kind = match row.get::<String, _>("kind").as_str() {
                "managed" => AgentWorkspaceKind::Managed,
                "selected_directory" => AgentWorkspaceKind::SelectedDirectory,
                value => {
                    return Err(AgentStoreError::InvalidPersistedValue {
                        kind: "workspace kind",
                        value: value.into(),
                    });
                }
            };
            let status = match row.get::<String, _>("status").as_str() {
                "ready" => AgentWorkspaceStatus::Ready,
                "unavailable" => AgentWorkspaceStatus::Unavailable,
                value => {
                    return Err(AgentStoreError::InvalidPersistedValue {
                        kind: "workspace status",
                        value: value.into(),
                    });
                }
            };
            Ok(AgentWorkspace {
                id: WorkspaceId::new(row.get::<String, _>("id")),
                kind,
                owner: owner.owned(),
                root_path: row.get("root_path"),
                status,
                status_reason: row.get("status_reason"),
                created_at_ms: row.get("created_at_ms"),
                updated_at_ms: row.get("updated_at_ms"),
            })
        })
        .transpose()
    }

    pub async fn workspace(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Option<AgentWorkspace>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM agent_workspaces WHERE id = ?")
            .bind(workspace_id.as_str())
            .fetch_optional(self.pool())
            .await?;
        row.map(|row| {
            let kind = match row.get::<String, _>("kind").as_str() {
                "managed" => AgentWorkspaceKind::Managed,
                "selected_directory" => AgentWorkspaceKind::SelectedDirectory,
                value => {
                    return Err(AgentStoreError::InvalidPersistedValue {
                        kind: "workspace kind",
                        value: value.into(),
                    });
                }
            };
            let owner = match row.get::<String, _>("owner_kind").as_str() {
                "session" => AgentWorkspaceOwner::Session {
                    session_id: SessionId::new(row.get::<String, _>("owner_id")),
                },
                "schedule" => AgentWorkspaceOwner::Schedule {
                    schedule_id: ScheduleId::new(row.get::<String, _>("owner_id")),
                },
                value => {
                    return Err(AgentStoreError::InvalidPersistedValue {
                        kind: "workspace owner kind",
                        value: value.into(),
                    });
                }
            };
            let status = match row.get::<String, _>("status").as_str() {
                "ready" => AgentWorkspaceStatus::Ready,
                "unavailable" => AgentWorkspaceStatus::Unavailable,
                value => {
                    return Err(AgentStoreError::InvalidPersistedValue {
                        kind: "workspace status",
                        value: value.into(),
                    });
                }
            };
            Ok(AgentWorkspace {
                id: WorkspaceId::new(row.get::<String, _>("id")),
                kind,
                owner,
                root_path: row.get("root_path"),
                status,
                status_reason: row.get("status_reason"),
                created_at_ms: row.get("created_at_ms"),
                updated_at_ms: row.get("updated_at_ms"),
            })
        })
        .transpose()
    }

    pub async fn mark_workspace_unavailable(
        &self,
        owner: WorkspaceOwnerRef<'_>,
        reason: &str,
        timestamp_ms: i64,
    ) -> Result<(), AgentStoreError> {
        sqlx::query("UPDATE agent_workspaces SET status = 'unavailable', status_reason = ?, updated_at_ms = ? WHERE owner_kind = ? AND owner_id = ?")
            .bind(reason)
            .bind(timestamp_ms)
            .bind(owner.kind())
            .bind(owner.id())
            .execute(self.pool())
            .await?;
        Ok(())
    }

    async fn persist_unavailable_workspace(
        &self,
        workspace_id: WorkspaceId,
        owner: WorkspaceOwnerRef<'_>,
        root: &Path,
        reason: &str,
        timestamp_ms: i64,
    ) -> Result<(), AgentStoreError> {
        let root_path = root.to_string_lossy().into_owned();
        sqlx::query("INSERT INTO agent_workspaces(id, kind, owner_kind, owner_id, root_path, status, status_reason, created_at_ms, updated_at_ms) VALUES(?, 'selected_directory', ?, ?, ?, 'unavailable', ?, ?, ?) ON CONFLICT(owner_kind, owner_id) DO UPDATE SET kind = excluded.kind, root_path = excluded.root_path, status = 'unavailable', status_reason = excluded.status_reason, updated_at_ms = excluded.updated_at_ms")
            .bind(workspace_id.as_str())
            .bind(owner.kind())
            .bind(owner.id())
            .bind(root_path)
            .bind(reason)
            .bind(timestamp_ms)
            .bind(timestamp_ms)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn remove_workspace_for_owner(
        &self,
        owner: WorkspaceOwnerRef<'_>,
    ) -> Result<bool, AgentStoreError> {
        let Some(workspace) = self.workspace_for_owner(owner).await? else {
            return Ok(false);
        };
        if workspace.kind == AgentWorkspaceKind::Managed {
            self.remove_managed_workspace_directory(&workspace)?;
        }
        sqlx::query("DELETE FROM agent_workspaces WHERE id = ?")
            .bind(workspace.id.as_str())
            .execute(self.pool())
            .await?;
        Ok(true)
    }

    pub async fn reconcile_managed_workspaces(
        &self,
    ) -> Result<WorkspaceReconciliationReport, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT id, kind, owner_kind, owner_id, root_path FROM agent_workspaces ORDER BY id",
        )
        .fetch_all(self.pool())
        .await?;
        let mut report = WorkspaceReconciliationReport::default();
        let mut referenced = HashSet::new();
        for row in rows {
            let id = WorkspaceId::new(row.get::<String, _>("id"));
            let kind = row.get::<String, _>("kind");
            let owner_kind = row.get::<String, _>("owner_kind");
            let owner_id = row.get::<String, _>("owner_id");
            let owner_exists = match owner_kind.as_str() {
                "session" => {
                    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sessions WHERE id = ?")
                        .bind(&owner_id)
                        .fetch_one(self.pool())
                        .await?
                        > 0
                }
                "schedule" => {
                    sqlx::query_scalar::<_, i64>(
                        "SELECT COUNT(*) FROM schedule_definitions WHERE id = ?",
                    )
                    .bind(&owner_id)
                    .fetch_one(self.pool())
                    .await?
                        > 0
                }
                value => {
                    return Err(AgentStoreError::InvalidPersistedValue {
                        kind: "workspace owner kind",
                        value: value.into(),
                    });
                }
            };
            if owner_exists {
                if kind == "managed" {
                    referenced.insert(id.as_str().to_owned());
                }
                continue;
            }
            if kind == "managed" {
                let workspace = AgentWorkspace {
                    id: id.clone(),
                    kind: AgentWorkspaceKind::Managed,
                    owner: if owner_kind == "session" {
                        AgentWorkspaceOwner::Session {
                            session_id: SessionId::new(owner_id),
                        }
                    } else {
                        AgentWorkspaceOwner::Schedule {
                            schedule_id: ScheduleId::new(owner_id),
                        }
                    },
                    root_path: row.get("root_path"),
                    status: AgentWorkspaceStatus::Unavailable,
                    status_reason: None,
                    created_at_ms: 0,
                    updated_at_ms: 0,
                };
                report.removed_directories = report.removed_directories.saturating_add(u64::from(
                    self.remove_managed_workspace_directory(&workspace)?,
                ));
            }
            sqlx::query("DELETE FROM agent_workspaces WHERE id = ?")
                .bind(id.as_str())
                .execute(self.pool())
                .await?;
            report.removed_rows = report.removed_rows.saturating_add(1);
        }

        let root = self.managed_workspace_path(&WorkspaceId::new("placeholder"));
        let Some(root) = root.parent() else {
            return Ok(report);
        };
        if root.is_dir() {
            for entry in std::fs::read_dir(root)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if referenced.contains(&name) {
                    continue;
                }
                let workspace = AgentWorkspace {
                    id: WorkspaceId::new(name),
                    kind: AgentWorkspaceKind::Managed,
                    owner: AgentWorkspaceOwner::Session {
                        session_id: SessionId::new("orphan"),
                    },
                    root_path: entry.path().to_string_lossy().into_owned(),
                    status: AgentWorkspaceStatus::Unavailable,
                    status_reason: None,
                    created_at_ms: 0,
                    updated_at_ms: 0,
                };
                report.removed_directories = report.removed_directories.saturating_add(u64::from(
                    self.remove_managed_workspace_directory(&workspace)?,
                ));
            }
        }
        Ok(report)
    }

    fn remove_managed_workspace_directory(
        &self,
        workspace: &AgentWorkspace,
    ) -> Result<bool, AgentStoreError> {
        let expected = self.managed_workspace_path(&workspace.id);
        if Path::new(&workspace.root_path) != expected {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "managed workspace root",
                value: workspace.root_path.clone(),
            });
        }
        let metadata = match std::fs::symlink_metadata(&expected) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "managed workspace root",
                value: workspace.root_path.clone(),
            });
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(AgentStoreError::InvalidPersistedValue {
                    kind: "managed workspace root",
                    value: workspace.root_path.clone(),
                });
            }
        }
        if !metadata.is_dir() {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "managed workspace root",
                value: workspace.root_path.clone(),
            });
        }
        std::fs::remove_dir_all(expected)?;
        Ok(true)
    }

    pub async fn session_for_workspace(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Option<hachimi_protocol::SessionRecord>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM sessions WHERE context_kind = 'workspace' AND json_extract(context_json, '$.workspace_id') = ? AND archived = 0 ORDER BY created_at_ms ASC LIMIT 1")
            .bind(workspace_id.as_str())
            .fetch_optional(self.pool())
            .await?;
        row.as_ref().map(super::session_from_row).transpose()
    }

    pub async fn session_for_channel_binding(
        &self,
        binding_key_hash: &str,
    ) -> Result<Option<hachimi_protocol::SessionRecord>, AgentStoreError> {
        let row = sqlx::query("SELECT session.* FROM channel_session_bindings AS binding INNER JOIN sessions AS session ON session.id = binding.session_id WHERE binding.binding_key_hash = ? AND session.archived = 0")
            .bind(binding_key_hash)
            .fetch_optional(self.pool())
            .await?;
        row.as_ref().map(super::session_from_row).transpose()
    }

    pub async fn store_permission_policy(
        &self,
        owner_key: &str,
        policy: &AgentPermissionPolicy,
        timestamp_ms: i64,
    ) -> Result<(), AgentStoreError> {
        sqlx::query("INSERT INTO agent_permission_policies(owner_key, policy_json, revision, updated_at_ms) VALUES(?, ?, ?, ?) ON CONFLICT(owner_key) DO UPDATE SET policy_json = excluded.policy_json, revision = excluded.revision, updated_at_ms = excluded.updated_at_ms")
            .bind(owner_key)
            .bind(serde_json::to_string(policy)?)
            .bind(i64::try_from(policy.revision).unwrap_or(i64::MAX))
            .bind(timestamp_ms)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn permission_policy(
        &self,
        owner_key: &str,
    ) -> Result<Option<AgentPermissionPolicy>, AgentStoreError> {
        let value = sqlx::query_scalar::<_, String>(
            "SELECT policy_json FROM agent_permission_policies WHERE owner_key = ?",
        )
        .bind(owner_key)
        .fetch_optional(self.pool())
        .await?;
        value
            .map(|value| serde_json::from_str(&value))
            .transpose()
            .map_err(Into::into)
    }

    pub async fn store_permission_config(
        &self,
        owner_key: &str,
        policy: &AgentPermissionPolicy,
        skill_ids: &[SkillId],
        timestamp_ms: i64,
    ) -> Result<(), AgentStoreError> {
        let mut transaction = self.pool().begin().await?;
        sqlx::query("INSERT INTO agent_permission_policies(owner_key, policy_json, skill_allowlist_json, revision, updated_at_ms) VALUES(?, ?, ?, ?, ?) ON CONFLICT(owner_key) DO UPDATE SET policy_json = excluded.policy_json, skill_allowlist_json = excluded.skill_allowlist_json, revision = excluded.revision, updated_at_ms = excluded.updated_at_ms")
            .bind(owner_key)
            .bind(serde_json::to_string(policy)?)
            .bind(serde_json::to_string(skill_ids)?)
            .bind(i64::try_from(policy.revision).unwrap_or(i64::MAX))
            .bind(timestamp_ms)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn permission_skill_ids(
        &self,
        owner_key: &str,
    ) -> Result<Vec<SkillId>, AgentStoreError> {
        let value = sqlx::query_scalar::<_, String>(
            "SELECT skill_allowlist_json FROM agent_permission_policies WHERE owner_key = ?",
        )
        .bind(owner_key)
        .fetch_optional(self.pool())
        .await?;
        value
            .map_or_else(|| Ok(Vec::new()), |value| serde_json::from_str(&value))
            .map_err(Into::into)
    }

    pub async fn approved_session_tool_authority(
        &self,
        session_id: &SessionId,
        action: &str,
        resource: &str,
        target_host: &str,
    ) -> Result<Option<ApprovalRequestRecord>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM approval_requests WHERE session_id = ? AND status = 'approved' AND grant_scope = 'session' AND uses_remaining > 0 AND action = ? AND resource = ? AND target_host = ? ORDER BY resolved_at_ms DESC, id DESC LIMIT 1")
            .bind(session_id.as_str())
            .bind(action)
            .bind(resource)
            .bind(target_host)
            .fetch_optional(self.pool())
            .await?;
        row.as_ref().map(super::approval_from_row).transpose()
    }

    pub async fn list_session_tool_authorities(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<ApprovalRequestRecord>, AgentStoreError> {
        let rows = sqlx::query("SELECT * FROM approval_requests WHERE session_id = ? AND status = 'approved' AND grant_scope = 'session' ORDER BY resolved_at_ms DESC, id DESC")
            .bind(session_id.as_str())
            .fetch_all(self.pool())
            .await?;
        rows.iter().map(super::approval_from_row).collect()
    }

    pub async fn clear_session_tool_authorities(
        &self,
        session_id: &SessionId,
        timestamp_ms: i64,
    ) -> Result<u64, AgentStoreError> {
        let result = sqlx::query("UPDATE approval_requests SET status = 'cancelled', resolved_at_ms = ? WHERE session_id = ? AND status = 'approved' AND grant_scope = 'session'")
            .bind(timestamp_ms)
            .bind(session_id.as_str())
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn list_session_host_authorities(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<SessionHostAuthorizationSummary>, AgentStoreError> {
        let host_rows = sqlx::query(
            "SELECT id, target_kind, target_key, updated_at_ms FROM host_access_grants WHERE scope = 'session' AND owner_session_id = ?",
        )
        .bind(session_id.as_str())
        .fetch_all(self.pool())
        .await?;
        let browser_rows = sqlx::query(
            "SELECT id, origin, updated_at_ms FROM embedded_browser_site_permissions WHERE scope = 'session' AND owner_session_id = ?",
        )
        .bind(session_id.as_str())
        .fetch_all(self.pool())
        .await?;
        let mut summaries = host_rows
            .iter()
            .map(|row| {
                let target_kind = row.get::<String, _>("target_kind");
                SessionHostAuthorizationSummary {
                    id: row.get("id"),
                    action: format!("{target_kind}.access"),
                    resource: row.get("target_key"),
                    target_host: target_kind,
                    granted_at_ms: row.get("updated_at_ms"),
                }
            })
            .chain(
                browser_rows
                    .iter()
                    .map(|row| SessionHostAuthorizationSummary {
                        id: row.get("id"),
                        action: "browser.site".into(),
                        resource: row.get("origin"),
                        target_host: "browser:embedded".into(),
                        granted_at_ms: row.get("updated_at_ms"),
                    }),
            )
            .collect::<Vec<_>>();
        summaries.sort_by(|left, right| {
            right
                .granted_at_ms
                .cmp(&left.granted_at_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(summaries)
    }

    pub async fn clear_session_extra_authorities(
        &self,
        session_id: &SessionId,
        timestamp_ms: i64,
    ) -> Result<u64, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let approvals = sqlx::query("UPDATE approval_requests SET status = 'cancelled', resolved_at_ms = ? WHERE session_id = ? AND status = 'approved' AND grant_scope = 'session'")
            .bind(timestamp_ms)
            .bind(session_id.as_str())
            .execute(&mut *transaction)
            .await?
            .rows_affected();
        let host_grants = sqlx::query(
            "DELETE FROM host_access_grants WHERE scope = 'session' AND owner_session_id = ?",
        )
        .bind(session_id.as_str())
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        let browser_permissions = sqlx::query(
            "DELETE FROM embedded_browser_site_permissions WHERE scope = 'session' AND owner_session_id = ?",
        )
        .bind(session_id.as_str())
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        transaction.commit().await?;
        Ok(approvals + host_grants + browser_permissions)
    }

    pub async fn active_runs_for_permission_owner(
        &self,
        owner_key: &str,
    ) -> Result<Vec<RunRecord>, AgentStoreError> {
        let rows = sqlx::query("SELECT * FROM runs WHERE status NOT IN ('succeeded', 'failed', 'timed_out', 'cancelled', 'interrupted', 'lost') ORDER BY created_at_ms ASC, id ASC")
            .fetch_all(self.pool())
            .await?;
        let runs = rows
            .iter()
            .map(super::run_from_row)
            .collect::<Result<Vec<_>, _>>()?;

        if let Some(session_id) = owner_key.strip_prefix("session:") {
            return Ok(runs
                .into_iter()
                .filter(|run| run.session_id.as_str() == session_id)
                .collect());
        }
        if let Some(schedule_id) = owner_key.strip_prefix("schedule:") {
            return Ok(runs
                .into_iter()
                .filter(|run| {
                    matches!(
                        &run.origin,
                        RunOrigin::Scheduled { schedule_id: current, .. }
                            if current.as_str() == schedule_id
                    )
                })
                .collect());
        }
        if let Some(binding_key_hash) = owner_key.strip_prefix("channel_binding:") {
            let session_id = sqlx::query_scalar::<_, String>(
                "SELECT session_id FROM channel_session_bindings WHERE binding_key_hash = ?",
            )
            .bind(binding_key_hash)
            .fetch_optional(self.pool())
            .await?;
            return Ok(session_id.map_or_else(Vec::new, |session_id| {
                runs.into_iter()
                    .filter(|run| run.session_id.as_str() == session_id)
                    .collect()
            }));
        }
        if owner_key == "profile:pet_conversation" {
            let mut pet_runs = Vec::new();
            for run in runs {
                if self
                    .get_session(&run.session_id)
                    .await?
                    .is_some_and(|session| {
                        session.entry_profile == hachimi_protocol::EntryProfile::PetConversation
                    })
                {
                    pet_runs.push(run);
                }
            }
            return Ok(pet_runs);
        }
        Ok(Vec::new())
    }

    pub async fn channel_binding_permission_owners_for_account(
        &self,
        account_id: &str,
    ) -> Result<Vec<String>, AgentStoreError> {
        let hashes = sqlx::query_scalar::<_, String>(
            "SELECT binding_key_hash FROM channel_session_bindings WHERE account_id = ? ORDER BY binding_key_hash",
        )
        .bind(account_id)
        .fetch_all(self.pool())
        .await?;
        Ok(hashes
            .into_iter()
            .map(|hash| format!("channel_binding:{hash}"))
            .collect())
    }

    pub async fn channel_binding_permission_owners_for_authorization(
        &self,
        authorization_id: &str,
    ) -> Result<Vec<String>, AgentStoreError> {
        let hashes = sqlx::query_scalar::<_, String>(
            "SELECT binding_key_hash FROM channel_session_bindings WHERE authorization_id = ? ORDER BY binding_key_hash",
        )
        .bind(authorization_id)
        .fetch_all(self.pool())
        .await?;
        Ok(hashes
            .into_iter()
            .map(|hash| format!("channel_binding:{hash}"))
            .collect())
    }

    pub async fn persist_authority_snapshot(
        &self,
        snapshot: &RunAuthoritySnapshot,
    ) -> Result<(), AgentStoreError> {
        sqlx::query("INSERT OR REPLACE INTO run_authority_snapshots(id, run_id, session_id, snapshot_json, created_at_ms) VALUES(?, ?, ?, ?, ?)")
            .bind(snapshot.id.as_str())
            .bind(snapshot.run_id.as_str())
            .bind(snapshot.session_id.as_str())
            .bind(serde_json::to_string(snapshot)?)
            .bind(snapshot.created_at_ms)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn authority_snapshot(
        &self,
        run_id: &RunId,
    ) -> Result<Option<RunAuthoritySnapshot>, AgentStoreError> {
        let value = sqlx::query_scalar::<_, String>(
            "SELECT snapshot_json FROM run_authority_snapshots WHERE run_id = ?",
        )
        .bind(run_id.as_str())
        .fetch_optional(self.pool())
        .await?;
        value
            .map(|value| serde_json::from_str(&value))
            .transpose()
            .map_err(Into::into)
    }
}

fn validate_existing_path_chain(path: &Path) -> Result<(), String> {
    let mut cursor = path;
    loop {
        match std::fs::symlink_metadata(cursor) {
            Ok(metadata) => {
                validate_workspace_metadata(cursor, &metadata)?;
                if !metadata.is_dir() {
                    return Err(format!(
                        "workspace path is not a directory: {}",
                        cursor.display()
                    ));
                }
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                cursor = cursor.parent().ok_or_else(|| {
                    format!("workspace path has no existing parent: {}", path.display())
                })?;
            }
            Err(error) => {
                return Err(format!(
                    "workspace path cannot be inspected: {}: {error}",
                    cursor.display()
                ));
            }
        }
    }

    for ancestor in cursor.ancestors().skip(1) {
        let metadata = std::fs::symlink_metadata(ancestor).map_err(|error| {
            format!(
                "workspace path cannot inspect ancestor {}: {error}",
                ancestor.display()
            )
        })?;
        validate_workspace_metadata(ancestor, &metadata)?;
        if !metadata.is_dir() {
            return Err(format!(
                "workspace path is not a directory: {}",
                ancestor.display()
            ));
        }
    }
    Ok(())
}

fn validate_workspace_metadata(path: &Path, metadata: &std::fs::Metadata) -> Result<(), String> {
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "managed workspace traverses a symbolic link: {}",
            path.display()
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(format!(
                "managed workspace traverses a reparse point: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn validate_selected_workspace_root(root: &Path) -> Result<PathBuf, String> {
    if !root.is_absolute() {
        return Err("selected directory must be an absolute path".into());
    }
    let mut ancestors = root.ancestors().collect::<Vec<_>>();
    ancestors.reverse();
    for ancestor in ancestors {
        let metadata = match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(format!(
                    "selected directory is unavailable: {}",
                    root.display()
                ));
            }
            Err(error) => return Err(format!("selected directory cannot be inspected: {error}")),
        };
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "selected directory traverses a symbolic link: {}",
                ancestor.display()
            ));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(format!(
                    "selected directory traverses a reparse point: {}",
                    ancestor.display()
                ));
            }
        }
    }
    std::fs::canonicalize(root)
        .map_err(|error| format!("selected directory is unavailable: {error}"))
}
