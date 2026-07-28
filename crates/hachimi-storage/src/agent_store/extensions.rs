use hachimi_protocol::{
    McpServerId, McpToolOverride, McpToolView, RunId, SkillActivation, SkillActivationId,
    SkillActivationSource, SkillClassification, SkillEditorKind, SkillEntryKind, SkillId,
    SkillRecord, SkillScope, WorkloadKind,
};
use serde_json::Value;
use sqlx::Row;

use super::{AgentStore, AgentStoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSkillRecord {
    pub record: SkillRecord,
    pub stable_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillFileIndexRecord {
    pub relative_path: String,
    pub kind: SkillEntryKind,
    pub editor_kind: SkillEditorKind,
    pub size_bytes: u64,
    pub sha256: Option<String>,
    pub modified_at_ms: i64,
}

impl AgentStore {
    /// Records only an opaque Keyring reference, never the credential value.
    pub async fn defer_mcp_keyring_cleanup(
        &self,
        credential_reference: &str,
        attempted_at_ms: i64,
    ) -> Result<(), AgentStoreError> {
        sqlx::query(
            "INSERT INTO mcp_keyring_cleanup_queue (credential_reference, attempt_count, created_at_ms, last_attempt_at_ms) VALUES (?, 1, ?, ?) ON CONFLICT(credential_reference) DO UPDATE SET attempt_count = attempt_count + 1, last_attempt_at_ms = excluded.last_attempt_at_ms",
        )
        .bind(credential_reference)
        .bind(attempted_at_ms)
        .bind(attempted_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_pending_mcp_keyring_cleanup(
        &self,
        limit: u32,
    ) -> Result<Vec<String>, AgentStoreError> {
        let limit = i64::from(limit.clamp(1, 256));
        Ok(sqlx::query_scalar::<_, String>(
            "SELECT credential_reference FROM mcp_keyring_cleanup_queue ORDER BY created_at_ms, credential_reference LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn complete_mcp_keyring_cleanup(
        &self,
        credential_reference: &str,
    ) -> Result<bool, AgentStoreError> {
        Ok(
            sqlx::query("DELETE FROM mcp_keyring_cleanup_queue WHERE credential_reference = ?")
                .bind(credential_reference)
                .execute(&self.pool)
                .await?
                .rows_affected()
                == 1,
        )
    }

    pub async fn upsert_skill(
        &self,
        stored: &StoredSkillRecord,
    ) -> Result<StoredSkillRecord, AgentStoreError> {
        let record = &stored.record;
        sqlx::query(
            "INSERT INTO skills (id, stable_path, namespace, name, qualified_name, source_scope, enabled, content_hash, dependencies_json, diagnostics_json, updated_at_ms, entry_hash, tree_revision, indexed_at_ms, description, interface_json, policy_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET stable_path = excluded.stable_path, namespace = excluded.namespace, name = excluded.name, qualified_name = excluded.qualified_name, source_scope = excluded.source_scope, enabled = excluded.enabled, content_hash = excluded.content_hash, dependencies_json = excluded.dependencies_json, diagnostics_json = excluded.diagnostics_json, updated_at_ms = excluded.updated_at_ms, entry_hash = excluded.entry_hash, tree_revision = excluded.tree_revision, indexed_at_ms = excluded.indexed_at_ms, description = excluded.description, interface_json = excluded.interface_json, policy_json = excluded.policy_json",
        )
        .bind(record.id.as_str())
        .bind(&stored.stable_path)
        .bind(record.namespace.as_deref().unwrap_or_default())
        .bind(&record.name)
        .bind(&record.qualified_name)
        .bind(skill_scope_db(record.scope))
        .bind(record.enabled)
        .bind(&record.content_hash)
        .bind(serde_json::to_string(&record.dependencies)?)
        .bind(serde_json::to_string(&record.diagnostics)?)
        .bind(record.updated_at_ms)
        .bind(&record.content_hash)
        .bind(&record.tree_revision)
        .bind(record.updated_at_ms)
        .bind(&record.description)
        .bind(serde_json::to_string(&record.interface)?)
        .bind(serde_json::to_string(&record.policy)?)
        .execute(&self.pool)
        .await?;
        self.get_skill(&record.id)
            .await?
            .ok_or_else(|| AgentStoreError::SkillNotFound(record.id.clone()))
    }

    pub async fn get_skill(
        &self,
        skill_id: &SkillId,
    ) -> Result<Option<StoredSkillRecord>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM skills WHERE id = ?")
            .bind(skill_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(stored_skill_from_row).transpose()
    }

    pub async fn get_skill_by_path(
        &self,
        stable_path: &str,
    ) -> Result<Option<StoredSkillRecord>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM skills WHERE stable_path = ? LIMIT 1")
            .bind(stable_path)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(stored_skill_from_row).transpose()
    }

    pub async fn list_skills(&self) -> Result<Vec<StoredSkillRecord>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM skills ORDER BY CASE source_scope WHEN 'repo' THEN 0 WHEN 'user' THEN 1 WHEN 'built_in' THEN 2 WHEN 'system' THEN 3 ELSE 4 END, qualified_name COLLATE NOCASE, stable_path",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(stored_skill_from_row).collect()
    }

    pub async fn set_skill_enabled(
        &self,
        skill_id: &SkillId,
        enabled: bool,
        updated_at_ms: i64,
    ) -> Result<SkillRecord, AgentStoreError> {
        let changed = sqlx::query("UPDATE skills SET enabled = ?, updated_at_ms = ? WHERE id = ?")
            .bind(enabled)
            .bind(updated_at_ms)
            .bind(skill_id.as_str())
            .execute(&self.pool)
            .await?
            .rows_affected();
        if changed != 1 {
            return Err(AgentStoreError::SkillNotFound(skill_id.clone()));
        }
        Ok(self
            .get_skill(skill_id)
            .await?
            .ok_or_else(|| AgentStoreError::SkillNotFound(skill_id.clone()))?
            .record)
    }

    pub async fn remove_skill(&self, skill_id: &SkillId) -> Result<bool, AgentStoreError> {
        Ok(sqlx::query("DELETE FROM skills WHERE id = ?")
            .bind(skill_id.as_str())
            .execute(&self.pool)
            .await?
            .rows_affected()
            == 1)
    }

    pub async fn replace_skill_file_index(
        &self,
        skill_id: &SkillId,
        entries: &[SkillFileIndexRecord],
    ) -> Result<(), AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM skill_file_index WHERE skill_id = ?")
            .bind(skill_id.as_str())
            .execute(&mut *transaction)
            .await?;
        for entry in entries {
            sqlx::query(
                "INSERT INTO skill_file_index (skill_id, relative_path, entry_kind, editor_kind, size_bytes, sha256, modified_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(skill_id.as_str())
            .bind(&entry.relative_path)
            .bind(entry_kind_db(entry.kind))
            .bind(editor_kind_db(entry.editor_kind))
            .bind(i64::try_from(entry.size_bytes).unwrap_or(i64::MAX))
            .bind(&entry.sha256)
            .bind(entry.modified_at_ms)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn put_skill_classification(
        &self,
        classification: &SkillClassification,
    ) -> Result<(), AgentStoreError> {
        sqlx::query(
            "INSERT INTO skill_classifications (skill_id, content_revision, workload, confidence_basis_points, reason, classifier_revision, classified_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(skill_id, content_revision) DO UPDATE SET workload = excluded.workload, confidence_basis_points = excluded.confidence_basis_points, reason = excluded.reason, classifier_revision = excluded.classifier_revision, classified_at_ms = excluded.classified_at_ms",
        )
        .bind(classification.skill_id.as_str())
        .bind(&classification.content_revision)
        .bind(workload_db(classification.workload))
        .bind(i64::from(classification.confidence_basis_points))
        .bind(&classification.reason)
        .bind(&classification.classifier_revision)
        .bind(classification.classified_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_skill_classification(
        &self,
        skill_id: &SkillId,
        content_revision: &str,
    ) -> Result<Option<SkillClassification>, AgentStoreError> {
        let row = sqlx::query(
            "SELECT * FROM skill_classifications WHERE skill_id = ? AND content_revision = ?",
        )
        .bind(skill_id.as_str())
        .bind(content_revision)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(skill_classification_from_row).transpose()
    }

    pub async fn record_skill_activation(
        &self,
        run_id: &RunId,
        activation: &SkillActivation,
        created_at_ms: i64,
    ) -> Result<bool, AgentStoreError> {
        Ok(sqlx::query(
            "INSERT OR IGNORE INTO skill_activations (id, run_id, skill_id, content_revision, source, activated_at_step_revision, classified_workload, created_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(activation.id.as_str())
        .bind(run_id.as_str())
        .bind(activation.skill_id.as_str())
        .bind(&activation.content_revision)
        .bind(skill_activation_source_db(activation.source))
        .bind(i64::try_from(activation.activated_at_step_revision).unwrap_or(i64::MAX))
        .bind(workload_db(activation.classified_workload))
        .bind(created_at_ms)
        .execute(&self.pool)
        .await?
        .rows_affected()
            == 1)
    }

    pub async fn list_run_skill_activations(
        &self,
        run_id: &RunId,
    ) -> Result<Vec<SkillActivation>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM skill_activations WHERE run_id = ? ORDER BY activated_at_step_revision, created_at_ms, id",
        )
        .bind(run_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(skill_activation_from_row).collect()
    }

    pub async fn replace_mcp_discovered_tools(
        &self,
        server_id: &McpServerId,
        tools: &[McpToolView],
    ) -> Result<(), AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM mcp_discovered_tools WHERE server_id = ?")
            .bind(server_id.as_str())
            .execute(&mut *transaction)
            .await?;
        for tool in tools {
            sqlx::query(
                "INSERT INTO mcp_discovered_tools (server_id, tool_name, exposed_name, description, input_schema_json, schema_hash, host_identity_hash, validation_error, discovered_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(server_id.as_str())
            .bind(&tool.name)
            .bind(&tool.exposed_name)
            .bind(&tool.description)
            .bind(serde_json::to_string(&tool.input_schema)?)
            .bind(&tool.schema_hash)
            .bind(&tool.host_identity_hash)
            .bind(&tool.validation_error)
            .bind(tool.discovered_at_ms)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn list_mcp_tools(
        &self,
        server_id: &McpServerId,
        stale: bool,
    ) -> Result<Vec<McpToolView>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT tools.*, COALESCE(overrides.enabled, 1) AS enabled FROM mcp_discovered_tools tools LEFT JOIN mcp_tool_overrides overrides ON overrides.server_id = tools.server_id AND overrides.tool_name = tools.tool_name WHERE tools.server_id = ? ORDER BY tools.tool_name",
        )
        .bind(server_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                let schema: Value = serde_json::from_str(row.get("input_schema_json"))?;
                let required_parameters = schema
                    .get("required")
                    .and_then(Value::as_array)
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                Ok(McpToolView {
                    server_id: McpServerId::new(row.get::<String, _>("server_id")),
                    name: row.get("tool_name"),
                    exposed_name: row.get("exposed_name"),
                    description: row.get("description"),
                    input_schema: schema,
                    required_parameters,
                    enabled: row.get("enabled"),
                    stale,
                    validation_error: row.get("validation_error"),
                    schema_hash: row.get("schema_hash"),
                    host_identity_hash: row.get("host_identity_hash"),
                    discovered_at_ms: row.get("discovered_at_ms"),
                })
            })
            .collect()
    }

    pub async fn set_mcp_tool_enabled(
        &self,
        server_id: &McpServerId,
        tool_name: &str,
        enabled: bool,
        updated_at_ms: i64,
    ) -> Result<McpToolOverride, AgentStoreError> {
        sqlx::query(
            "INSERT INTO mcp_tool_overrides (server_id, tool_name, enabled, updated_at_ms) VALUES (?, ?, ?, ?) ON CONFLICT(server_id, tool_name) DO UPDATE SET enabled = excluded.enabled, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(server_id.as_str())
        .bind(tool_name)
        .bind(enabled)
        .bind(updated_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(McpToolOverride {
            server_id: server_id.clone(),
            tool_name: tool_name.to_owned(),
            enabled,
            updated_at_ms,
        })
    }

    pub async fn mcp_tool_enabled(
        &self,
        server_id: &McpServerId,
        tool_name: &str,
    ) -> Result<bool, AgentStoreError> {
        Ok(sqlx::query_scalar::<_, bool>(
            "SELECT enabled FROM mcp_tool_overrides WHERE server_id = ? AND tool_name = ?",
        )
        .bind(server_id.as_str())
        .bind(tool_name)
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or(true))
    }
}

fn stored_skill_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<StoredSkillRecord, AgentStoreError> {
    Ok(StoredSkillRecord {
        stable_path: row.get("stable_path"),
        record: SkillRecord {
            id: SkillId::new(row.get::<String, _>("id")),
            scope: skill_scope_from_db(row.get("source_scope"))?,
            namespace: {
                let namespace: String = row.get("namespace");
                (!namespace.is_empty()).then_some(namespace)
            },
            name: row.get("name"),
            qualified_name: row.get("qualified_name"),
            description: row.get("description"),
            interface: serde_json::from_str(row.get("interface_json"))?,
            policy: serde_json::from_str(row.get("policy_json"))?,
            dependencies: serde_json::from_str(row.get("dependencies_json"))?,
            editable: row.get::<String, _>("source_scope") == "user",
            enabled: row.get("enabled"),
            content_hash: row.get("content_hash"),
            tree_revision: row.get("tree_revision"),
            diagnostics: serde_json::from_str(row.get("diagnostics_json"))?,
            updated_at_ms: row.get("updated_at_ms"),
        },
    })
}

fn skill_classification_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<SkillClassification, AgentStoreError> {
    Ok(SkillClassification {
        skill_id: SkillId::new(row.get::<String, _>("skill_id")),
        content_revision: row.get("content_revision"),
        workload: workload_from_db(row.get("workload"))?,
        confidence_basis_points: u16::try_from(row.get::<i64, _>("confidence_basis_points"))
            .unwrap_or_default(),
        reason: row.get("reason"),
        classifier_revision: row.get("classifier_revision"),
        classified_at_ms: row.get("classified_at_ms"),
    })
}

fn skill_activation_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<SkillActivation, AgentStoreError> {
    Ok(SkillActivation {
        id: SkillActivationId::new(row.get::<String, _>("id")),
        skill_id: SkillId::new(row.get::<String, _>("skill_id")),
        content_revision: row.get("content_revision"),
        source: skill_activation_source_from_db(row.get("source"))?,
        activated_at_step_revision: u64::try_from(row.get::<i64, _>("activated_at_step_revision"))
            .unwrap_or_default(),
        classified_workload: workload_from_db(row.get("classified_workload"))?,
    })
}

const fn workload_db(workload: WorkloadKind) -> &'static str {
    match workload {
        WorkloadKind::General => "general",
        WorkloadKind::Coding => "coding",
        WorkloadKind::Office => "office",
    }
}

fn workload_from_db(value: &str) -> Result<WorkloadKind, AgentStoreError> {
    match value {
        "general" => Ok(WorkloadKind::General),
        "coding" => Ok(WorkloadKind::Coding),
        "office" => Ok(WorkloadKind::Office),
        other => Err(AgentStoreError::InvalidPersistedValue {
            kind: "Skill workload",
            value: other.to_owned(),
        }),
    }
}

const fn skill_activation_source_db(source: SkillActivationSource) -> &'static str {
    match source {
        SkillActivationSource::ExplicitSelection => "explicit_selection",
        SkillActivationSource::Mention => "mention",
        SkillActivationSource::ModelRead => "model_read",
        SkillActivationSource::BuiltInDiscovery => "built_in_discovery",
    }
}

fn skill_activation_source_from_db(value: &str) -> Result<SkillActivationSource, AgentStoreError> {
    match value {
        "explicit_selection" => Ok(SkillActivationSource::ExplicitSelection),
        "mention" => Ok(SkillActivationSource::Mention),
        "model_read" => Ok(SkillActivationSource::ModelRead),
        "built_in_discovery" => Ok(SkillActivationSource::BuiltInDiscovery),
        other => Err(AgentStoreError::InvalidPersistedValue {
            kind: "Skill activation source",
            value: other.to_owned(),
        }),
    }
}

const fn skill_scope_db(scope: SkillScope) -> &'static str {
    match scope {
        SkillScope::BuiltIn => "built_in",
        SkillScope::User => "user",
        SkillScope::Repo => "repo",
        SkillScope::System => "system",
        SkillScope::Admin => "admin",
    }
}

fn skill_scope_from_db(value: &str) -> Result<SkillScope, AgentStoreError> {
    match value {
        "built_in" => Ok(SkillScope::BuiltIn),
        "user" => Ok(SkillScope::User),
        "repo" => Ok(SkillScope::Repo),
        "system" => Ok(SkillScope::System),
        "admin" => Ok(SkillScope::Admin),
        other => Err(AgentStoreError::InvalidPersistedValue {
            kind: "Skill scope",
            value: other.to_owned(),
        }),
    }
}

const fn entry_kind_db(kind: SkillEntryKind) -> &'static str {
    match kind {
        SkillEntryKind::File => "file",
        SkillEntryKind::Directory => "directory",
    }
}

const fn editor_kind_db(kind: SkillEditorKind) -> &'static str {
    match kind {
        SkillEditorKind::Markdown => "markdown",
        SkillEditorKind::Text => "text",
        SkillEditorKind::Unsupported => "unsupported",
    }
}
