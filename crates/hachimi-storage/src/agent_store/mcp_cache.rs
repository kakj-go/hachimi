use std::collections::BTreeMap;

use hachimi_protocol::{
    McpCallSummaryListRequest, McpCallSummaryRecord, McpInventorySnapshot, McpPrompt, McpResource,
    McpResourceTemplate, McpServerId, McpToolProgressRecord, RunId, SessionId, ToolCallId,
};
use serde::Serialize;
use sqlx::{Row, Sqlite, Transaction};

use super::{AgentStore, AgentStoreError, append_event_tx, get_run_tx};

impl AgentStore {
    /// Returns the opaque OS-keyring reference for an MCP OAuth credential.
    /// The credential itself must never be stored in SQLite.
    pub async fn get_mcp_auth_reference(
        &self,
        server_id: &McpServerId,
    ) -> Result<Option<String>, AgentStoreError> {
        let row = sqlx::query("SELECT auth_reference FROM mcp_servers WHERE id = ?")
            .bind(server_id.as_str())
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| AgentStoreError::McpServerNotFound(server_id.clone()))?;
        Ok(row.get("auth_reference"))
    }

    /// Atomically replaces the opaque keyring reference and returns the old
    /// reference so callers can remove the superseded secret after commit.
    pub async fn replace_mcp_auth_reference(
        &self,
        server_id: &McpServerId,
        auth_reference: Option<&str>,
    ) -> Result<Option<String>, AgentStoreError> {
        if auth_reference.is_some_and(|reference| {
            reference.trim().is_empty() || reference.len() > 512 || reference.contains('\0')
        }) {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "MCP auth reference",
                value: "invalid opaque reference".into(),
            });
        }
        let mut transaction = self.pool.begin().await?;
        let previous = sqlx::query("SELECT auth_reference FROM mcp_servers WHERE id = ?")
            .bind(server_id.as_str())
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| AgentStoreError::McpServerNotFound(server_id.clone()))?
            .get("auth_reference");
        sqlx::query("UPDATE mcp_servers SET auth_reference = ? WHERE id = ?")
            .bind(auth_reference)
            .bind(server_id.as_str())
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(previous)
    }

    /// Appends progress only while its exact Run generation is active. The check and event append
    /// share one transaction so cancellation or recovery cannot admit a late server notification.
    pub async fn append_mcp_tool_progress(
        &self,
        progress: &McpToolProgressRecord,
    ) -> Result<bool, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let Some(run) = get_run_tx(&mut transaction, &progress.run_id).await? else {
            transaction.commit().await?;
            return Ok(false);
        };
        if run.session_id != progress.session_id
            || run.generation != progress.run_generation
            || run.status.is_terminal()
        {
            transaction.commit().await?;
            return Ok(false);
        }
        append_event_tx(
            &mut transaction,
            &progress.session_id,
            Some(&progress.run_id),
            "mcp.tool.progress",
            serde_json::to_value(progress)?,
            super::now_ms(),
        )
        .await?;
        transaction.commit().await?;
        Ok(true)
    }

    /// Persists only MCP call metadata. Tool arguments, server messages, and returned content are
    /// deliberately excluded from this table and must remain in their bounded runtime projections.
    pub async fn record_mcp_call_summary(
        &self,
        summary: &McpCallSummaryRecord,
    ) -> Result<McpCallSummaryRecord, AgentStoreError> {
        let duration_ms = i64::try_from(summary.duration_ms).map_err(|_| {
            AgentStoreError::InvalidPersistedValue {
                kind: "MCP call duration",
                value: summary.duration_ms.to_string(),
            }
        })?;
        sqlx::query(
            "INSERT INTO mcp_call_summaries (id, server_id, session_id, run_id, tool_name, outcome, duration_ms, created_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET server_id = excluded.server_id, session_id = excluded.session_id, run_id = excluded.run_id, tool_name = excluded.tool_name, outcome = excluded.outcome, duration_ms = excluded.duration_ms",
        )
        .bind(summary.id.as_str())
        .bind(summary.server_id.as_str())
        .bind(summary.session_id.as_str())
        .bind(summary.run_id.as_str())
        .bind(&summary.tool_name)
        .bind(summary.outcome.as_str())
        .bind(duration_ms)
        .bind(summary.created_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(summary.clone())
    }

    pub async fn list_mcp_call_summaries(
        &self,
        request: &McpCallSummaryListRequest,
    ) -> Result<Vec<McpCallSummaryRecord>, AgentStoreError> {
        let limit = i64::from(request.limit.clamp(1, 200));
        let rows = sqlx::query(
            "SELECT id, server_id, session_id, run_id, tool_name, outcome, duration_ms, created_at_ms FROM mcp_call_summaries WHERE (? IS NULL OR server_id = ?) AND (? IS NULL OR session_id = ?) AND session_id IS NOT NULL AND run_id IS NOT NULL ORDER BY created_at_ms DESC, id DESC LIMIT ?",
        )
        .bind(request.server_id.as_ref().map(McpServerId::as_str))
        .bind(request.server_id.as_ref().map(McpServerId::as_str))
        .bind(request.session_id.as_ref().map(SessionId::as_str))
        .bind(request.session_id.as_ref().map(SessionId::as_str))
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let outcome = row.get::<String, _>("outcome");
                let duration_ms = row.get::<i64, _>("duration_ms");
                Ok(McpCallSummaryRecord {
                    id: ToolCallId::new(row.get::<String, _>("id")),
                    server_id: McpServerId::new(row.get::<String, _>("server_id")),
                    session_id: SessionId::new(row.get::<String, _>("session_id")),
                    run_id: RunId::new(row.get::<String, _>("run_id")),
                    tool_name: row.get("tool_name"),
                    outcome: hachimi_protocol::McpCallOutcome::parse(&outcome).ok_or_else(
                        || AgentStoreError::InvalidPersistedValue {
                            kind: "MCP call outcome",
                            value: outcome,
                        },
                    )?,
                    duration_ms: u64::try_from(duration_ms).map_err(|_| {
                        AgentStoreError::InvalidPersistedValue {
                            kind: "MCP call duration",
                            value: duration_ms.to_string(),
                        }
                    })?,
                    created_at_ms: row.get("created_at_ms"),
                })
            })
            .collect()
    }

    /// Replaces only inventory kinds that refreshed successfully. A failed kind keeps its last
    /// verified cache and the snapshot is marked stale instead of erasing useful metadata.
    pub async fn update_mcp_inventory(
        &self,
        server_id: &McpServerId,
        resources: &[McpResource],
        resource_templates: &[McpResourceTemplate],
        prompts: &[McpPrompt],
        errors: &BTreeMap<String, String>,
        refreshed_at_ms: i64,
    ) -> Result<McpInventorySnapshot, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        if !errors.contains_key("resources") {
            replace_kind(
                &mut transaction,
                server_id,
                "resource",
                resources
                    .iter()
                    .map(|resource| (resource.uri.as_str(), resource)),
                refreshed_at_ms,
            )
            .await?;
        }
        if !errors.contains_key("resource_templates") {
            replace_kind(
                &mut transaction,
                server_id,
                "resource_template",
                resource_templates
                    .iter()
                    .map(|template| (template.uri_template.as_str(), template)),
                refreshed_at_ms,
            )
            .await?;
        }
        if !errors.contains_key("prompts") {
            replace_kind(
                &mut transaction,
                server_id,
                "prompt",
                prompts.iter().map(|prompt| (prompt.name.as_str(), prompt)),
                refreshed_at_ms,
            )
            .await?;
        }
        sqlx::query(
            "INSERT INTO mcp_inventory_status (server_id, errors_json, stale, refreshed_at_ms) VALUES (?, ?, ?, ?) ON CONFLICT(server_id) DO UPDATE SET errors_json = excluded.errors_json, stale = excluded.stale, refreshed_at_ms = excluded.refreshed_at_ms",
        )
        .bind(server_id.as_str())
        .bind(serde_json::to_string(errors)?)
        .bind(!errors.is_empty())
        .bind(refreshed_at_ms)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.get_mcp_inventory(server_id)
            .await?
            .ok_or_else(|| AgentStoreError::McpServerNotFound(server_id.clone()))
    }

    pub async fn get_mcp_inventory(
        &self,
        server_id: &McpServerId,
    ) -> Result<Option<McpInventorySnapshot>, AgentStoreError> {
        let Some(status) = sqlx::query(
            "SELECT errors_json, stale, refreshed_at_ms FROM mcp_inventory_status WHERE server_id = ?",
        )
        .bind(server_id.as_str())
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        let rows = sqlx::query(
            "SELECT content_kind, metadata_json FROM mcp_content_cache WHERE server_id = ? ORDER BY content_kind, content_key",
        )
        .bind(server_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        let mut resources = Vec::new();
        let mut resource_templates = Vec::new();
        let mut prompts = Vec::new();
        for row in rows {
            let kind: String = row.get("content_kind");
            let metadata: String = row.get("metadata_json");
            match kind.as_str() {
                "resource" => resources.push(serde_json::from_str(&metadata)?),
                "resource_template" => {
                    resource_templates.push(serde_json::from_str(&metadata)?);
                }
                "prompt" => prompts.push(serde_json::from_str(&metadata)?),
                _ => {
                    return Err(AgentStoreError::InvalidPersistedValue {
                        kind: "MCP content kind",
                        value: kind,
                    });
                }
            }
        }
        Ok(Some(McpInventorySnapshot {
            server_id: server_id.clone(),
            resources,
            resource_templates,
            prompts,
            errors: serde_json::from_str(status.get("errors_json"))?,
            stale: status.get("stale"),
            refreshed_at_ms: status.get("refreshed_at_ms"),
        }))
    }
}

async fn replace_kind<'a, T: Serialize + 'a>(
    transaction: &mut Transaction<'_, Sqlite>,
    server_id: &McpServerId,
    kind: &str,
    values: impl Iterator<Item = (&'a str, &'a T)>,
    refreshed_at_ms: i64,
) -> Result<(), AgentStoreError> {
    sqlx::query("DELETE FROM mcp_content_cache WHERE server_id = ? AND content_kind = ?")
        .bind(server_id.as_str())
        .bind(kind)
        .execute(&mut **transaction)
        .await?;
    for (key, value) in values {
        sqlx::query(
            "INSERT INTO mcp_content_cache (server_id, content_kind, content_key, metadata_json, refreshed_at_ms) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(server_id.as_str())
        .bind(kind)
        .bind(key)
        .bind(serde_json::to_string(value)?)
        .bind(refreshed_at_ms)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use hachimi_protocol::{
        ApprovalPolicy, BehaviorMode, EntryProfile, LlmSettings, McpCallOutcome,
        McpCallSummaryListRequest, McpCallSummaryRecord, McpServerRecord, McpServerTransport,
        McpToolProgressRecord, PermissionProfile, ProviderCapabilities, RunBudget,
        RunConfiguration, RunDriverKind, RunOrigin, RunPurpose, RunRecord, RunStatus,
        SessionContextBinding, SessionRecord, ToolCallId, WorkloadKind,
    };

    async fn store_with_server() -> (AgentStore, McpServerId) {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let id = McpServerId::from("inventory");
        store
            .upsert_mcp_server(&McpServerRecord {
                id: id.clone(),
                display_name: "Inventory".into(),
                enabled: false,
                transport: McpServerTransport::Stdio {
                    command: "fixture".into(),
                    args: Vec::new(),
                    cwd: None,
                },
                headers: Vec::new(),
                read_only_tools: Vec::new(),
                startup_timeout_ms: 1_000,
                request_timeout_ms: 1_000,
                max_message_bytes: 4_096,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .await
            .expect("server");
        (store, id)
    }

    #[tokio::test]
    async fn oauth_reference_is_opaque_and_atomically_replaceable() {
        let (store, id) = store_with_server().await;
        assert_eq!(store.get_mcp_auth_reference(&id).await.expect("get"), None);
        assert_eq!(
            store
                .replace_mcp_auth_reference(&id, Some("oauth:keyring:v1"))
                .await
                .expect("set"),
            None
        );
        assert_eq!(
            store.get_mcp_auth_reference(&id).await.expect("get"),
            Some("oauth:keyring:v1".into())
        );
        assert_eq!(
            store
                .replace_mcp_auth_reference(&id, Some("oauth:keyring:v2"))
                .await
                .expect("replace"),
            Some("oauth:keyring:v1".into())
        );
        assert_eq!(
            store
                .replace_mcp_auth_reference(&id, None)
                .await
                .expect("clear"),
            Some("oauth:keyring:v2".into())
        );
        assert_eq!(store.get_mcp_auth_reference(&id).await.expect("get"), None);
    }

    #[tokio::test]
    async fn partial_refresh_preserves_last_verified_kind_as_stale() {
        let (store, id) = store_with_server().await;
        let resource = McpResource {
            uri: "memo://one".into(),
            name: "one".into(),
            title: None,
            description: None,
            mime_type: Some("text/plain".into()),
            size: None,
            annotations: Some(json!({ "audience": ["user"] })),
            meta: None,
        };
        store
            .update_mcp_inventory(
                &id,
                std::slice::from_ref(&resource),
                &[],
                &[],
                &BTreeMap::new(),
                2,
            )
            .await
            .expect("initial cache");
        let stale = store
            .update_mcp_inventory(
                &id,
                &[],
                &[],
                &[],
                &BTreeMap::from([("resources".into(), "timeout".into())]),
                3,
            )
            .await
            .expect("stale cache");
        assert!(stale.stale);
        assert_eq!(stale.resources, vec![resource]);
        assert_eq!(
            stale.errors.get("resources").map(String::as_str),
            Some("timeout")
        );
    }

    #[tokio::test]
    async fn call_summaries_and_progress_keep_only_bounded_run_metadata() {
        let (store, server_id) = store_with_server().await;
        let session = SessionRecord {
            id: SessionId::from("mcp-session"),
            context: SessionContextBinding::Workspace {
                workspace_id: hachimi_protocol::WorkspaceId::random(),
            },
            entry_profile: EntryProfile::Workbench,
            title: "MCP".into(),
            archived: false,
            pinned: false,
            parent_session_id: None,
            source_run_id: None,
            created_at_ms: 10,
            updated_at_ms: 10,
        };
        store.create_session(&session).await.expect("session");
        let run = RunRecord {
            id: RunId::from("mcp-run"),
            session_id: session.id.clone(),
            status: RunStatus::Running,
            purpose: RunPurpose::Task,
            origin: RunOrigin::Manual,
            generation: 4,
            configuration: RunConfiguration {
                model_snapshot: LlmSettings::default(),
                driver: RunDriverKind::ToolLoop,
                entry_profile: EntryProfile::Workbench,
                workload_override: Some(WorkloadKind::Office),
                behavior_mode: BehaviorMode::Default,
                execution_target: None,
                approval_policy: ApprovalPolicy::OnlyWhenNeeded,
                permission_profile: PermissionProfile::ReadOnly,
                budget: RunBudget::default(),
                accepted_plan_id: None,
                accepted_plan_revision: None,
            },
            requested_capabilities: ProviderCapabilities::default(),
            negotiated_capabilities: ProviderCapabilities::default(),
            provider_capability_probe: None,
            capability_degradations: Vec::new(),
            failure_code: None,
            created_at_ms: 11,
            updated_at_ms: 11,
        };
        store
            .create_run_idempotent("test", "mcp-run", &run)
            .await
            .expect("run");
        let summary = McpCallSummaryRecord {
            id: ToolCallId::from("mcp-call"),
            server_id: server_id.clone(),
            session_id: session.id.clone(),
            run_id: run.id.clone(),
            tool_name: "search".into(),
            outcome: McpCallOutcome::Succeeded,
            duration_ms: 23,
            created_at_ms: 12,
        };
        store
            .record_mcp_call_summary(&summary)
            .await
            .expect("summary");
        assert_eq!(
            store
                .list_mcp_call_summaries(&McpCallSummaryListRequest {
                    server_id: Some(server_id.clone()),
                    session_id: None,
                    limit: 25,
                })
                .await
                .expect("list"),
            vec![summary]
        );

        let progress = McpToolProgressRecord {
            server_id,
            session_id: session.id.clone(),
            run_id: run.id.clone(),
            run_generation: 4,
            tool_call_id: ToolCallId::from("mcp-call"),
            progress: 1.0,
            total: Some(2.0),
            message: Some("working".into()),
        };
        assert!(
            store
                .append_mcp_tool_progress(&progress)
                .await
                .expect("progress")
        );
        assert!(
            !store
                .append_mcp_tool_progress(&McpToolProgressRecord {
                    run_generation: 3,
                    ..progress.clone()
                })
                .await
                .expect("stale progress")
        );
        store
            .transition_run(&run.id, RunStatus::Succeeded, None)
            .await
            .expect("complete");
        assert!(
            !store
                .append_mcp_tool_progress(&progress)
                .await
                .expect("late progress")
        );
        let events = store.list_events(&session.id, 0).await.expect("events");
        let progress_events = events
            .iter()
            .filter(|event| event.event_name() == "mcp.tool.progress")
            .collect::<Vec<_>>();
        assert_eq!(progress_events.len(), 1);
        let hachimi_protocol::RunEventPayload::Generic { data, .. } = &progress_events[0].payload
        else {
            panic!("progress must remain a typed generic metadata event");
        };
        assert_eq!(data["message"], "working");
        assert!(data.get("arguments").is_none());
    }
}
