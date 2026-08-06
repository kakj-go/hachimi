use hachimi_protocol::{
    BrowserAutomationLease, BrowserAutomationLeaseId, BrowserAutomationLeaseStatus,
    BrowserAutomationSurfaceKind, BrowserCapability, BrowserDownloadSnapshot,
    BrowserDownloadStatus, BrowserHistoryEntry, BrowserNavigationError, BrowserObservation,
    BrowserSessionId, BrowserTabId, BrowserTabSnapshot, BrowserWorkspace, BrowserWorkspaceId,
    BrowserWorkspaceRuntimeState, EmbeddedBrowserSettings, EmbeddedBrowserSettingsUpdate,
    ExternalBrowserLeaseObservation, RunId, SessionId,
};
use sha2::{Digest, Sha256};
use sqlx::{Row, Sqlite, Transaction};
use url::Url;

use super::{AgentStore, AgentStoreError, now_ms};

const EMBEDDED_PROFILE_ID: &str = "embedded-default";
const NEW_TAB_URL: &str = "about:blank";

#[derive(Debug, Clone, Default)]
pub struct BrowserTabRuntimeUpdate {
    pub url: Option<String>,
    pub title: Option<String>,
    pub favicon_token: Option<Option<String>>,
    pub loading: Option<bool>,
    pub can_go_back: Option<bool>,
    pub can_go_forward: Option<bool>,
    pub runtime_loaded: Option<bool>,
    pub navigation_error: Option<Option<BrowserNavigationError>>,
    pub user_input: bool,
}

#[derive(Debug, Clone)]
pub struct BrowserDownloadRuntimeUpdate {
    pub runtime_id: u32,
    pub tab_id: BrowserTabId,
    pub source_url: String,
    pub suggested_name: String,
    pub destination: Option<String>,
    pub received_bytes: u64,
    pub total_bytes: Option<u64>,
    pub complete: bool,
    pub cancelled: bool,
    pub interrupted: bool,
}

impl AgentStore {
    pub async fn reconcile_browser_startup(&self) -> Result<(), AgentStoreError> {
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "UPDATE browser_automation_leases SET status = 'expired', revision = revision + 1, updated_at_ms = ? WHERE status IN ('pending', 'active', 'suspended')",
        )
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE browser_tabs SET loading = 0, runtime_loaded = 0, revision = revision + 1, updated_at_ms = ? WHERE loading = 1 OR runtime_loaded = 1",
        )
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE browser_workspaces SET runtime_state = 'dormant', revision = revision + 1, updated_at_ms = ? WHERE runtime_state != 'dormant'",
        )
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE embedded_browser_permission_requests SET status = 'expired', updated_at_ms = ? WHERE status = 'pending'",
        )
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("DELETE FROM embedded_browser_site_permissions WHERE scope = 'once'")
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_browser_automation_lease(
        &self,
        surface: BrowserAutomationSurfaceKind,
        workspace_id: Option<&BrowserWorkspaceId>,
        tab_id: Option<&BrowserTabId>,
        owner_session_id: &SessionId,
        owner_run_id: &RunId,
        run_generation: u64,
        capabilities: &[BrowserCapability],
        expires_at_ms: i64,
    ) -> Result<BrowserAutomationLease, AgentStoreError> {
        let now = now_ms();
        let id = BrowserAutomationLeaseId::random();
        sqlx::query(
            "INSERT INTO browser_automation_leases(id, surface, workspace_id, tab_id, owner_session_id, owner_run_id, run_generation, capabilities_json, status, revision, expires_at_ms, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?, 'active', 1, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(surface_text(surface))
        .bind(workspace_id.map(BrowserWorkspaceId::as_str))
        .bind(tab_id.map(BrowserTabId::as_str))
        .bind(owner_session_id.as_str())
        .bind(owner_run_id.as_str())
        .bind(i64::try_from(run_generation).unwrap_or(i64::MAX))
        .bind(serde_json::to_string(capabilities)?)
        .bind(expires_at_ms)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        self.browser_automation_lease(&id).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_external_browser_automation_lease(
        &self,
        owner_session_id: &SessionId,
        owner_run_id: &RunId,
        run_generation: u64,
        browser_session_id: &BrowserSessionId,
        capabilities: &[BrowserCapability],
        expires_at_ms: i64,
    ) -> Result<BrowserAutomationLease, AgentStoreError> {
        let lease = self
            .create_browser_automation_lease(
                BrowserAutomationSurfaceKind::ExternalChrome,
                None,
                None,
                owner_session_id,
                owner_run_id,
                run_generation,
                capabilities,
                expires_at_ms,
            )
            .await?;
        sqlx::query("UPDATE browser_automation_leases SET external_session_id = ? WHERE id = ?")
            .bind(browser_session_id.as_str())
            .bind(lease.id.as_str())
            .execute(&self.pool)
            .await?;
        Ok(lease)
    }

    pub async fn external_browser_session_for_lease(
        &self,
        lease_id: &BrowserAutomationLeaseId,
    ) -> Result<Option<BrowserSessionId>, AgentStoreError> {
        Ok(sqlx::query_scalar::<_, Option<String>>(
            "SELECT external_session_id FROM browser_automation_leases WHERE id = ?",
        )
        .bind(lease_id.as_str())
        .fetch_optional(&self.pool)
        .await?
        .flatten()
        .map(BrowserSessionId::new))
    }

    pub async fn browser_automation_lease(
        &self,
        lease_id: &BrowserAutomationLeaseId,
    ) -> Result<BrowserAutomationLease, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM browser_automation_leases WHERE id = ?")
            .bind(lease_id.as_str())
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| AgentStoreError::BrowserAutomationLeaseNotFound(lease_id.clone()))?;
        decode_lease(&row)
    }

    pub async fn active_browser_automation_lease_for_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<BrowserAutomationLease>, AgentStoreError> {
        let row = sqlx::query(
            "SELECT * FROM browser_automation_leases WHERE owner_session_id = ? AND status = 'active' AND expires_at_ms > ? ORDER BY updated_at_ms DESC, id ASC LIMIT 1",
        )
        .bind(session_id.as_str())
        .bind(now_ms())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(decode_lease).transpose()
    }

    pub async fn list_session_browser_automation_leases(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<BrowserAutomationLease>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM browser_automation_leases WHERE owner_session_id = ? ORDER BY updated_at_ms DESC, id ASC",
        )
        .bind(session_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(decode_lease).collect()
    }

    pub async fn store_external_browser_lease_observation(
        &self,
        lease_id: &BrowserAutomationLeaseId,
        owner_session_id: &SessionId,
        observation: &BrowserObservation,
    ) -> Result<(), AgentStoreError> {
        sqlx::query(
            "INSERT INTO external_browser_lease_observations(lease_id, owner_session_id, observation_json, updated_at_ms) VALUES(?, ?, ?, ?) ON CONFLICT(lease_id) DO UPDATE SET observation_json = excluded.observation_json, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(lease_id.as_str())
        .bind(owner_session_id.as_str())
        .bind(serde_json::to_string(observation)?)
        .bind(now_ms())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_session_external_browser_observations(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<ExternalBrowserLeaseObservation>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT lease_id, observation_json FROM external_browser_lease_observations WHERE owner_session_id = ? ORDER BY updated_at_ms DESC",
        )
        .bind(session_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ExternalBrowserLeaseObservation {
                    lease_id: BrowserAutomationLeaseId::new(row.get::<String, _>("lease_id")),
                    observation: serde_json::from_str(&row.get::<String, _>("observation_json"))?,
                })
            })
            .collect()
    }

    pub async fn set_browser_automation_lease_status(
        &self,
        lease_id: &BrowserAutomationLeaseId,
        expected_revision: u64,
        status: BrowserAutomationLeaseStatus,
    ) -> Result<BrowserAutomationLease, AgentStoreError> {
        let result = sqlx::query(
            "UPDATE browser_automation_leases SET status = ?, revision = revision + 1, updated_at_ms = ? WHERE id = ? AND revision = ?",
        )
        .bind(lease_status_text(status))
        .bind(now_ms())
        .bind(lease_id.as_str())
        .bind(i64::try_from(expected_revision).unwrap_or(i64::MAX))
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            let exists = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM browser_automation_leases WHERE id = ?",
            )
            .bind(lease_id.as_str())
            .fetch_one(&self.pool)
            .await?
                > 0;
            return Err(if exists {
                AgentStoreError::BrowserAutomationLeaseRevisionConflict
            } else {
                AgentStoreError::BrowserAutomationLeaseNotFound(lease_id.clone())
            });
        }
        self.browser_automation_lease(lease_id).await
    }

    pub async fn transition_browser_workspace_automation(
        &self,
        workspace_id: &BrowserWorkspaceId,
        expected_workspace_revision: u64,
        from: BrowserAutomationLeaseStatus,
        to: BrowserAutomationLeaseStatus,
    ) -> Result<BrowserWorkspace, AgentStoreError> {
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;
        require_workspace_revision(&mut transaction, workspace_id, expected_workspace_revision)
            .await?;
        let lease_id = sqlx::query_scalar::<_, String>(
            "SELECT id FROM browser_automation_leases WHERE workspace_id = ? AND status = ? AND expires_at_ms > ? ORDER BY updated_at_ms DESC, id ASC LIMIT 1",
        )
        .bind(workspace_id.as_str())
        .bind(lease_status_text(from))
        .bind(now)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(AgentStoreError::BrowserAutomationLeaseUnavailable)?;
        let updated = sqlx::query(
            "UPDATE browser_automation_leases SET status = ?, revision = revision + 1, updated_at_ms = ? WHERE id = ? AND status = ? AND expires_at_ms > ?",
        )
        .bind(lease_status_text(to))
        .bind(now)
        .bind(lease_id)
        .bind(lease_status_text(from))
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(AgentStoreError::BrowserAutomationLeaseRevisionConflict);
        }
        bump_workspace(
            &mut transaction,
            workspace_id,
            expected_workspace_revision,
            None,
            now,
        )
        .await?;
        transaction.commit().await?;
        self.browser_workspace(workspace_id).await
    }

    pub async fn suspend_active_browser_automation_for_tab(
        &self,
        tab_id: &BrowserTabId,
    ) -> Result<Option<BrowserWorkspace>, AgentStoreError> {
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT workspace_id, id FROM browser_automation_leases WHERE tab_id = ? AND status = 'active' AND expires_at_ms > ? ORDER BY updated_at_ms DESC, id ASC LIMIT 1",
        )
        .bind(tab_id.as_str())
        .bind(now)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else {
            transaction.rollback().await?;
            return Ok(None);
        };
        let workspace_id = BrowserWorkspaceId::new(row.get::<String, _>("workspace_id"));
        let lease_id = row.get::<String, _>("id");
        let updated = sqlx::query(
            "UPDATE browser_automation_leases SET status = 'suspended', revision = revision + 1, updated_at_ms = ? WHERE id = ? AND status = 'active' AND expires_at_ms > ?",
        )
        .bind(now)
        .bind(lease_id)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            transaction.rollback().await?;
            return Ok(None);
        }
        sqlx::query(
            "UPDATE browser_workspaces SET revision = revision + 1, updated_at_ms = ? WHERE id = ?",
        )
        .bind(now)
        .bind(workspace_id.as_str())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.browser_workspace(&workspace_id).await.map(Some)
    }

    pub async fn update_browser_automation_lease_target(
        &self,
        lease_id: &BrowserAutomationLeaseId,
        expected_revision: u64,
        workspace_id: &BrowserWorkspaceId,
        tab_id: &BrowserTabId,
    ) -> Result<BrowserAutomationLease, AgentStoreError> {
        let result = sqlx::query(
            "UPDATE browser_automation_leases SET workspace_id = ?, tab_id = ?, revision = revision + 1, updated_at_ms = ? WHERE id = ? AND revision = ? AND status = 'active'",
        )
        .bind(workspace_id.as_str())
        .bind(tab_id.as_str())
        .bind(now_ms())
        .bind(lease_id.as_str())
        .bind(i64::try_from(expected_revision).unwrap_or(i64::MAX))
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            return Err(AgentStoreError::BrowserAutomationLeaseRevisionConflict);
        }
        self.browser_automation_lease(lease_id).await
    }

    pub async fn get_or_create_browser_workspace(
        &self,
        session_id: &SessionId,
        initial_url: Option<&str>,
    ) -> Result<BrowserWorkspace, AgentStoreError> {
        if let Some(workspace) = self.browser_workspace_for_session(session_id).await? {
            return Ok(workspace);
        }

        let now = now_ms();
        let workspace_id = BrowserWorkspaceId::random();
        let tab_id = BrowserTabId::random();
        let initial_url = normalized_persisted_url(initial_url.unwrap_or(NEW_TAB_URL));
        let mut transaction = self.pool.begin().await?;
        let inserted = sqlx::query(
            "INSERT OR IGNORE INTO browser_workspaces(id, owner_session_id, profile_id, active_tab_id, runtime_state, revision, created_at_ms, updated_at_ms) VALUES(?, ?, ?, NULL, 'dormant', 1, ?, ?)",
        )
        .bind(workspace_id.as_str())
        .bind(session_id.as_str())
        .bind(EMBEDDED_PROFILE_ID)
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await?
        .rows_affected()
            == 1;
        if inserted {
            sqlx::query(
                "INSERT INTO browser_tabs(id, workspace_id, url, title, loading, can_go_back, can_go_forward, runtime_loaded, revision, input_epoch, created_at_ms, updated_at_ms) VALUES(?, ?, ?, '', 0, 0, 0, 0, 1, 1, ?, ?)",
            )
            .bind(tab_id.as_str())
            .bind(workspace_id.as_str())
            .bind(&initial_url)
            .bind(now)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
            sqlx::query("UPDATE browser_workspaces SET active_tab_id = ? WHERE id = ?")
                .bind(tab_id.as_str())
                .bind(workspace_id.as_str())
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        self.browser_workspace_for_session(session_id)
            .await?
            .ok_or(AgentStoreError::BrowserWorkspaceNotFound(workspace_id))
    }

    pub async fn browser_workspace_for_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<BrowserWorkspace>, AgentStoreError> {
        let row = sqlx::query("SELECT id FROM browser_workspaces WHERE owner_session_id = ?")
            .bind(session_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        self.browser_workspace(&BrowserWorkspaceId::new(row.get::<String, _>("id")))
            .await
            .map(Some)
    }

    pub async fn browser_workspace(
        &self,
        workspace_id: &BrowserWorkspaceId,
    ) -> Result<BrowserWorkspace, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM browser_workspaces WHERE id = ?")
            .bind(workspace_id.as_str())
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| AgentStoreError::BrowserWorkspaceNotFound(workspace_id.clone()))?;
        let tabs = sqlx::query(
            "SELECT * FROM browser_tabs WHERE workspace_id = ? ORDER BY created_at_ms ASC, id ASC",
        )
        .bind(workspace_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        let lease = sqlx::query(
            "SELECT * FROM browser_automation_leases WHERE workspace_id = ? AND status IN ('active', 'suspended') AND expires_at_ms > ? ORDER BY updated_at_ms DESC, id ASC LIMIT 1",
        )
        .bind(workspace_id.as_str())
        .bind(now_ms())
        .fetch_optional(&self.pool)
        .await?
        .as_ref()
        .map(decode_lease)
        .transpose()?;
        decode_workspace(&row, &tabs, lease)
    }

    pub async fn create_browser_tab(
        &self,
        workspace_id: &BrowserWorkspaceId,
        expected_revision: u64,
        url: Option<&str>,
    ) -> Result<BrowserWorkspace, AgentStoreError> {
        let now = now_ms();
        let tab_id = BrowserTabId::random();
        let url = normalized_persisted_url(url.unwrap_or(NEW_TAB_URL));
        let mut transaction = self.pool.begin().await?;
        require_workspace_revision(&mut transaction, workspace_id, expected_revision).await?;
        sqlx::query(
            "INSERT INTO browser_tabs(id, workspace_id, url, title, loading, can_go_back, can_go_forward, runtime_loaded, revision, input_epoch, created_at_ms, updated_at_ms) VALUES(?, ?, ?, '', 0, 0, 0, 0, 1, 1, ?, ?)",
        )
        .bind(tab_id.as_str())
        .bind(workspace_id.as_str())
        .bind(url)
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        bump_workspace(
            &mut transaction,
            workspace_id,
            expected_revision,
            Some(&tab_id),
            now,
        )
        .await?;
        transaction.commit().await?;
        self.browser_workspace(workspace_id).await
    }

    pub async fn activate_browser_tab(
        &self,
        workspace_id: &BrowserWorkspaceId,
        tab_id: &BrowserTabId,
        expected_revision: u64,
    ) -> Result<BrowserWorkspace, AgentStoreError> {
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;
        require_workspace_revision(&mut transaction, workspace_id, expected_revision).await?;
        require_tab(&mut transaction, workspace_id, tab_id).await?;
        bump_workspace(
            &mut transaction,
            workspace_id,
            expected_revision,
            Some(tab_id),
            now,
        )
        .await?;
        transaction.commit().await?;
        self.browser_workspace(workspace_id).await
    }

    pub async fn close_browser_tab(
        &self,
        workspace_id: &BrowserWorkspaceId,
        tab_id: &BrowserTabId,
        expected_revision: u64,
    ) -> Result<BrowserWorkspace, AgentStoreError> {
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;
        require_workspace_revision(&mut transaction, workspace_id, expected_revision).await?;
        require_tab(&mut transaction, workspace_id, tab_id).await?;
        sqlx::query("DELETE FROM browser_tabs WHERE id = ? AND workspace_id = ?")
            .bind(tab_id.as_str())
            .bind(workspace_id.as_str())
            .execute(&mut *transaction)
            .await?;

        let remaining = sqlx::query(
            "SELECT id FROM browser_tabs WHERE workspace_id = ? ORDER BY updated_at_ms DESC, id ASC LIMIT 1",
        )
        .bind(workspace_id.as_str())
        .fetch_optional(&mut *transaction)
        .await?;
        let next_tab = if let Some(row) = remaining {
            BrowserTabId::new(row.get::<String, _>("id"))
        } else {
            let next = BrowserTabId::random();
            sqlx::query(
                "INSERT INTO browser_tabs(id, workspace_id, url, title, loading, can_go_back, can_go_forward, runtime_loaded, revision, input_epoch, created_at_ms, updated_at_ms) VALUES(?, ?, ?, '', 0, 0, 0, 0, 1, 1, ?, ?)",
            )
            .bind(next.as_str())
            .bind(workspace_id.as_str())
            .bind(NEW_TAB_URL)
            .bind(now)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
            next
        };
        bump_workspace(
            &mut transaction,
            workspace_id,
            expected_revision,
            Some(&next_tab),
            now,
        )
        .await?;
        transaction.commit().await?;
        self.browser_workspace(workspace_id).await
    }

    pub async fn set_browser_workspace_runtime(
        &self,
        workspace_id: &BrowserWorkspaceId,
        runtime_state: BrowserWorkspaceRuntimeState,
    ) -> Result<BrowserWorkspace, AgentStoreError> {
        let now = now_ms();
        let result = sqlx::query(
            "UPDATE browser_workspaces SET runtime_state = ?, revision = revision + 1, updated_at_ms = ? WHERE id = ?",
        )
        .bind(runtime_state_text(runtime_state))
        .bind(now)
        .bind(workspace_id.as_str())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            return Err(AgentStoreError::BrowserWorkspaceNotFound(
                workspace_id.clone(),
            ));
        }
        self.browser_workspace(workspace_id).await
    }

    pub async fn update_browser_tab_runtime(
        &self,
        workspace_id: &BrowserWorkspaceId,
        tab_id: &BrowserTabId,
        update: BrowserTabRuntimeUpdate,
    ) -> Result<BrowserWorkspace, AgentStoreError> {
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query("SELECT * FROM browser_tabs WHERE id = ? AND workspace_id = ?")
            .bind(tab_id.as_str())
            .bind(workspace_id.as_str())
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| AgentStoreError::BrowserTabNotFound(tab_id.clone()))?;
        let current_error = row.get::<Option<String>, _>("navigation_error_json");
        let navigation_error = match update.navigation_error {
            Some(value) => value
                .map(|error| serde_json::to_string(&error))
                .transpose()?,
            None => current_error,
        };
        let current_favicon = row.get::<Option<String>, _>("favicon_token");
        let favicon = update.favicon_token.unwrap_or(current_favicon);
        let url_changed = update.url.is_some();
        let url = update
            .url
            .map(|value| normalized_persisted_url(&value))
            .unwrap_or_else(|| row.get::<String, _>("url"));
        let title = update
            .title
            .unwrap_or_else(|| row.get::<String, _>("title"));
        let loading = update
            .loading
            .unwrap_or_else(|| row.get::<i64, _>("loading") != 0);
        let can_go_back = update
            .can_go_back
            .unwrap_or_else(|| row.get::<i64, _>("can_go_back") != 0);
        let can_go_forward = update
            .can_go_forward
            .unwrap_or_else(|| row.get::<i64, _>("can_go_forward") != 0);
        let runtime_loaded = update
            .runtime_loaded
            .unwrap_or_else(|| row.get::<i64, _>("runtime_loaded") != 0);
        sqlx::query(
            "UPDATE browser_tabs SET url = ?, title = ?, favicon_token = ?, loading = ?, can_go_back = ?, can_go_forward = ?, runtime_loaded = ?, navigation_error_json = ?, revision = revision + 1, input_epoch = input_epoch + ?, updated_at_ms = ? WHERE id = ? AND workspace_id = ?",
        )
        .bind(&url)
        .bind(title.chars().take(1_000).collect::<String>())
        .bind(favicon)
        .bind(i64::from(loading))
        .bind(i64::from(can_go_back))
        .bind(i64::from(can_go_forward))
        .bind(i64::from(runtime_loaded))
        .bind(navigation_error)
        .bind(i64::from(update.user_input))
        .bind(now)
        .bind(tab_id.as_str())
        .bind(workspace_id.as_str())
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE browser_workspaces SET revision = revision + 1, updated_at_ms = ? WHERE id = ?",
        )
        .bind(now)
        .bind(workspace_id.as_str())
        .execute(&mut *transaction)
        .await?;
        if url_changed && url != NEW_TAB_URL {
            upsert_history(&mut transaction, &url, &title, now).await?;
        }
        transaction.commit().await?;
        self.browser_workspace(workspace_id).await
    }

    pub async fn browser_history(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<BrowserHistoryEntry>, AgentStoreError> {
        let pattern = format!("%{}%", query.trim());
        let rows = sqlx::query(
            "SELECT url, title, visit_count, last_visited_at_ms FROM browser_history WHERE profile_id = ? AND (? = '%%' OR url LIKE ? OR title LIKE ?) ORDER BY last_visited_at_ms DESC, canonical_url ASC LIMIT ?",
        )
        .bind(EMBEDDED_PROFILE_ID)
        .bind(&pattern)
        .bind(&pattern)
        .bind(&pattern)
        .bind(i64::from(limit.clamp(1, 100)))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(BrowserHistoryEntry {
                    url: row.get("url"),
                    title: row.get("title"),
                    visit_count: u64::try_from(row.get::<i64, _>("visit_count")).map_err(|_| {
                        AgentStoreError::InvalidPersistedValue {
                            kind: "browser history visit count",
                            value: row.get::<i64, _>("visit_count").to_string(),
                        }
                    })?,
                    last_visited_at_ms: row.get("last_visited_at_ms"),
                })
            })
            .collect()
    }

    pub async fn upsert_browser_download(
        &self,
        update: BrowserDownloadRuntimeUpdate,
    ) -> Result<BrowserDownloadSnapshot, AgentStoreError> {
        let workspace_id =
            sqlx::query_scalar::<_, String>("SELECT workspace_id FROM browser_tabs WHERE id = ?")
                .bind(update.tab_id.as_str())
                .fetch_optional(&self.pool)
                .await?
                .ok_or_else(|| AgentStoreError::BrowserTabNotFound(update.tab_id.clone()))?;
        let workspace_id = BrowserWorkspaceId::new(workspace_id);
        let id = format!("cef:{}:{}", update.tab_id, update.runtime_id);
        let status = if update.complete {
            BrowserDownloadStatus::Completed
        } else if update.cancelled {
            BrowserDownloadStatus::Cancelled
        } else if update.interrupted {
            BrowserDownloadStatus::Failed
        } else if update.received_bytes > 0 {
            BrowserDownloadStatus::InProgress
        } else {
            BrowserDownloadStatus::Pending
        };
        let destination = update
            .destination
            .filter(|value| !value.trim().is_empty())
            .map(|value| value.chars().take(32_768).collect::<String>());
        let sha256 = if status == BrowserDownloadStatus::Completed {
            let hash_destination = destination.clone();
            tokio::task::spawn_blocking(move || {
                hash_destination.as_deref().and_then(download_sha256)
            })
            .await
            .ok()
            .flatten()
        } else {
            None
        };
        let now = now_ms();
        sqlx::query(
            "INSERT INTO browser_downloads(id, workspace_id, tab_id, source_url, suggested_name, destination, status, received_bytes, total_bytes, sha256, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET source_url = excluded.source_url, suggested_name = excluded.suggested_name, destination = COALESCE(excluded.destination, browser_downloads.destination), status = excluded.status, received_bytes = excluded.received_bytes, total_bytes = excluded.total_bytes, sha256 = COALESCE(excluded.sha256, browser_downloads.sha256), updated_at_ms = excluded.updated_at_ms",
        )
        .bind(&id)
        .bind(workspace_id.as_str())
        .bind(update.tab_id.as_str())
        .bind(update.source_url.chars().take(32_768).collect::<String>())
        .bind(update.suggested_name.chars().take(1_000).collect::<String>())
        .bind(destination)
        .bind(download_status_text(status))
        .bind(i64::try_from(update.received_bytes).unwrap_or(i64::MAX))
        .bind(update.total_bytes.map(|value| i64::try_from(value).unwrap_or(i64::MAX)))
        .bind(sha256)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        self.browser_download(&id).await
    }

    pub async fn browser_downloads(
        &self,
        workspace_id: &BrowserWorkspaceId,
        limit: u32,
    ) -> Result<Vec<BrowserDownloadSnapshot>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM browser_downloads WHERE workspace_id = ? ORDER BY updated_at_ms DESC, id ASC LIMIT ?",
        )
        .bind(workspace_id.as_str())
        .bind(i64::from(limit.clamp(1, 100)))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(decode_download).collect()
    }

    pub async fn browser_download(
        &self,
        id: &str,
    ) -> Result<BrowserDownloadSnapshot, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM browser_downloads WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        decode_download(&row)
    }

    pub async fn embedded_browser_settings(
        &self,
        full_cdp_access_allowed: bool,
    ) -> Result<EmbeddedBrowserSettings, AgentStoreError> {
        let row = sqlx::query(
            "SELECT download_directory, ask_where_to_save_downloads, full_cdp_access, settings_revision FROM browser_profiles WHERE id = ?",
        )
        .bind(EMBEDDED_PROFILE_ID)
        .fetch_one(&self.pool)
        .await?;
        Ok(EmbeddedBrowserSettings {
            download_directory: row.get("download_directory"),
            ask_where_to_save_downloads: row.get::<i64, _>("ask_where_to_save_downloads") != 0,
            full_cdp_access: full_cdp_access_allowed && row.get::<i64, _>("full_cdp_access") != 0,
            full_cdp_access_allowed,
            revision: persisted_u64(&row, "settings_revision", "browser settings revision")?,
        })
    }

    pub async fn update_embedded_browser_settings(
        &self,
        update: &EmbeddedBrowserSettingsUpdate,
        full_cdp_access_allowed: bool,
    ) -> Result<EmbeddedBrowserSettings, AgentStoreError> {
        let result = sqlx::query(
            "UPDATE browser_profiles SET download_directory = ?, ask_where_to_save_downloads = ?, full_cdp_access = ?, settings_revision = settings_revision + 1, updated_at_ms = ? WHERE id = ? AND settings_revision = ?",
        )
        .bind(update.download_directory.as_deref())
        .bind(i64::from(update.ask_where_to_save_downloads))
        .bind(i64::from(update.full_cdp_access && full_cdp_access_allowed))
        .bind(now_ms())
        .bind(EMBEDDED_PROFILE_ID)
        .bind(i64::try_from(update.expected_revision).unwrap_or(i64::MAX))
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            return Err(AgentStoreError::EmbeddedBrowserSettingsRevisionConflict);
        }
        self.embedded_browser_settings(full_cdp_access_allowed)
            .await
    }

    pub async fn clear_embedded_browser_history(&self) -> Result<u64, AgentStoreError> {
        let result = sqlx::query("DELETE FROM browser_history WHERE profile_id = ?")
            .bind(EMBEDDED_PROFILE_ID)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}

async fn require_workspace_revision(
    transaction: &mut Transaction<'_, Sqlite>,
    workspace_id: &BrowserWorkspaceId,
    expected_revision: u64,
) -> Result<(), AgentStoreError> {
    let row = sqlx::query("SELECT revision FROM browser_workspaces WHERE id = ?")
        .bind(workspace_id.as_str())
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or_else(|| AgentStoreError::BrowserWorkspaceNotFound(workspace_id.clone()))?;
    if row.get::<i64, _>("revision") != i64::try_from(expected_revision).unwrap_or(i64::MAX) {
        return Err(AgentStoreError::BrowserWorkspaceRevisionConflict);
    }
    Ok(())
}

async fn require_tab(
    transaction: &mut Transaction<'_, Sqlite>,
    workspace_id: &BrowserWorkspaceId,
    tab_id: &BrowserTabId,
) -> Result<(), AgentStoreError> {
    let exists = sqlx::query("SELECT 1 FROM browser_tabs WHERE id = ? AND workspace_id = ?")
        .bind(tab_id.as_str())
        .bind(workspace_id.as_str())
        .fetch_optional(&mut **transaction)
        .await?
        .is_some();
    if !exists {
        return Err(AgentStoreError::BrowserTabNotFound(tab_id.clone()));
    }
    Ok(())
}

async fn bump_workspace(
    transaction: &mut Transaction<'_, Sqlite>,
    workspace_id: &BrowserWorkspaceId,
    expected_revision: u64,
    active_tab_id: Option<&BrowserTabId>,
    now: i64,
) -> Result<(), AgentStoreError> {
    let result = sqlx::query(
        "UPDATE browser_workspaces SET active_tab_id = COALESCE(?, active_tab_id), revision = revision + 1, updated_at_ms = ? WHERE id = ? AND revision = ?",
    )
    .bind(active_tab_id.map(BrowserTabId::as_str))
    .bind(now)
    .bind(workspace_id.as_str())
    .bind(i64::try_from(expected_revision).unwrap_or(i64::MAX))
    .execute(&mut **transaction)
    .await?;
    if result.rows_affected() != 1 {
        return Err(AgentStoreError::BrowserWorkspaceRevisionConflict);
    }
    Ok(())
}

async fn upsert_history(
    transaction: &mut Transaction<'_, Sqlite>,
    url: &str,
    title: &str,
    now: i64,
) -> Result<(), AgentStoreError> {
    let canonical = canonical_history_url(url);
    sqlx::query(
        "INSERT INTO browser_history(profile_id, canonical_url, url, title, visit_count, first_visited_at_ms, last_visited_at_ms) VALUES(?, ?, ?, ?, 1, ?, ?) ON CONFLICT(profile_id, canonical_url) DO UPDATE SET url = excluded.url, title = CASE WHEN excluded.title = '' THEN browser_history.title ELSE excluded.title END, visit_count = browser_history.visit_count + 1, last_visited_at_ms = excluded.last_visited_at_ms",
    )
    .bind(EMBEDDED_PROFILE_ID)
    .bind(canonical)
    .bind(url)
    .bind(title)
    .bind(now)
    .bind(now)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn decode_workspace(
    row: &sqlx::sqlite::SqliteRow,
    tab_rows: &[sqlx::sqlite::SqliteRow],
    automation_lease: Option<BrowserAutomationLease>,
) -> Result<BrowserWorkspace, AgentStoreError> {
    let id = BrowserWorkspaceId::new(row.get::<String, _>("id"));
    let active_tab = row
        .get::<Option<String>, _>("active_tab_id")
        .map(BrowserTabId::new)
        .or_else(|| {
            tab_rows
                .first()
                .map(|tab| BrowserTabId::new(tab.get::<String, _>("id")))
        })
        .ok_or_else(|| AgentStoreError::BrowserWorkspaceNotFound(id.clone()))?;
    Ok(BrowserWorkspace {
        id,
        owner_session_id: SessionId::new(row.get::<String, _>("owner_session_id")),
        active_tab_id: active_tab,
        runtime_state: parse_runtime_state(row.get::<String, _>("runtime_state").as_str())?,
        tabs: tab_rows
            .iter()
            .map(decode_tab)
            .collect::<Result<Vec<_>, _>>()?,
        automation_lease,
        revision: persisted_u64(row, "revision", "browser workspace revision")?,
        updated_at_ms: row.get("updated_at_ms"),
    })
}

fn decode_lease(row: &sqlx::sqlite::SqliteRow) -> Result<BrowserAutomationLease, AgentStoreError> {
    Ok(BrowserAutomationLease {
        id: BrowserAutomationLeaseId::new(row.get::<String, _>("id")),
        surface: parse_surface(row.get::<String, _>("surface").as_str())?,
        workspace_id: row
            .get::<Option<String>, _>("workspace_id")
            .map(BrowserWorkspaceId::new),
        tab_id: row
            .get::<Option<String>, _>("tab_id")
            .map(BrowserTabId::new),
        owner_session_id: SessionId::new(row.get::<String, _>("owner_session_id")),
        owner_run_id: RunId::new(row.get::<String, _>("owner_run_id")),
        run_generation: persisted_u64(row, "run_generation", "browser lease generation")?,
        capabilities: serde_json::from_str(&row.get::<String, _>("capabilities_json"))?,
        status: parse_lease_status(row.get::<String, _>("status").as_str())?,
        revision: persisted_u64(row, "revision", "browser lease revision")?,
        expires_at_ms: row.get("expires_at_ms"),
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

fn decode_download(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<BrowserDownloadSnapshot, AgentStoreError> {
    Ok(BrowserDownloadSnapshot {
        id: row.get("id"),
        workspace_id: BrowserWorkspaceId::new(row.get::<String, _>("workspace_id")),
        tab_id: BrowserTabId::new(row.get::<String, _>("tab_id")),
        source_url: row.get("source_url"),
        suggested_name: row.get("suggested_name"),
        destination: row.get("destination"),
        status: parse_download_status(row.get::<String, _>("status").as_str())?,
        received_bytes: persisted_u64(row, "received_bytes", "browser download received bytes")?,
        total_bytes: row
            .get::<Option<i64>, _>("total_bytes")
            .map(|value| {
                u64::try_from(value).map_err(|_| AgentStoreError::InvalidPersistedValue {
                    kind: "browser download total bytes",
                    value: value.to_string(),
                })
            })
            .transpose()?,
        sha256: row.get("sha256"),
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

fn download_sha256(destination: &str) -> Option<String> {
    let metadata = std::fs::symlink_metadata(destination).ok()?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > 2 * 1024 * 1024 * 1024
    {
        return None;
    }
    let mut file = std::fs::File::open(destination).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = std::io::Read::read(&mut file, &mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Some(
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

fn decode_tab(row: &sqlx::sqlite::SqliteRow) -> Result<BrowserTabSnapshot, AgentStoreError> {
    Ok(BrowserTabSnapshot {
        id: BrowserTabId::new(row.get::<String, _>("id")),
        workspace_id: BrowserWorkspaceId::new(row.get::<String, _>("workspace_id")),
        url: row.get("url"),
        title: row.get("title"),
        favicon_token: row.get("favicon_token"),
        loading: row.get::<i64, _>("loading") != 0,
        can_go_back: row.get::<i64, _>("can_go_back") != 0,
        can_go_forward: row.get::<i64, _>("can_go_forward") != 0,
        runtime_loaded: row.get::<i64, _>("runtime_loaded") != 0,
        navigation_error: row
            .get::<Option<String>, _>("navigation_error_json")
            .map(|value| serde_json::from_str(&value))
            .transpose()?,
        revision: persisted_u64(row, "revision", "browser tab revision")?,
        input_epoch: persisted_u64(row, "input_epoch", "browser tab input epoch")?,
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

fn persisted_u64(
    row: &sqlx::sqlite::SqliteRow,
    column: &str,
    kind: &'static str,
) -> Result<u64, AgentStoreError> {
    let value = row.get::<i64, _>(column);
    u64::try_from(value).map_err(|_| AgentStoreError::InvalidPersistedValue {
        kind,
        value: value.to_string(),
    })
}

fn parse_runtime_state(value: &str) -> Result<BrowserWorkspaceRuntimeState, AgentStoreError> {
    match value {
        "dormant" => Ok(BrowserWorkspaceRuntimeState::Dormant),
        "starting" => Ok(BrowserWorkspaceRuntimeState::Starting),
        "ready" => Ok(BrowserWorkspaceRuntimeState::Ready),
        "failed" => Ok(BrowserWorkspaceRuntimeState::Failed),
        value => Err(AgentStoreError::InvalidPersistedValue {
            kind: "browser workspace runtime state",
            value: value.to_owned(),
        }),
    }
}

const fn runtime_state_text(value: BrowserWorkspaceRuntimeState) -> &'static str {
    match value {
        BrowserWorkspaceRuntimeState::Dormant => "dormant",
        BrowserWorkspaceRuntimeState::Starting => "starting",
        BrowserWorkspaceRuntimeState::Ready => "ready",
        BrowserWorkspaceRuntimeState::Failed => "failed",
    }
}

const fn surface_text(value: BrowserAutomationSurfaceKind) -> &'static str {
    match value {
        BrowserAutomationSurfaceKind::Embedded => "embedded",
        BrowserAutomationSurfaceKind::ExternalChrome => "external_chrome",
    }
}

fn parse_surface(value: &str) -> Result<BrowserAutomationSurfaceKind, AgentStoreError> {
    match value {
        "embedded" => Ok(BrowserAutomationSurfaceKind::Embedded),
        "external_chrome" => Ok(BrowserAutomationSurfaceKind::ExternalChrome),
        value => Err(AgentStoreError::InvalidPersistedValue {
            kind: "browser lease surface",
            value: value.into(),
        }),
    }
}

const fn lease_status_text(value: BrowserAutomationLeaseStatus) -> &'static str {
    match value {
        BrowserAutomationLeaseStatus::Pending => "pending",
        BrowserAutomationLeaseStatus::Active => "active",
        BrowserAutomationLeaseStatus::Suspended => "suspended",
        BrowserAutomationLeaseStatus::Expired => "expired",
        BrowserAutomationLeaseStatus::Failed => "failed",
    }
}

const fn download_status_text(value: BrowserDownloadStatus) -> &'static str {
    match value {
        BrowserDownloadStatus::Pending => "pending",
        BrowserDownloadStatus::InProgress => "in_progress",
        BrowserDownloadStatus::Completed => "completed",
        BrowserDownloadStatus::Cancelled => "cancelled",
        BrowserDownloadStatus::Failed => "failed",
    }
}

fn parse_download_status(value: &str) -> Result<BrowserDownloadStatus, AgentStoreError> {
    match value {
        "pending" => Ok(BrowserDownloadStatus::Pending),
        "in_progress" => Ok(BrowserDownloadStatus::InProgress),
        "completed" => Ok(BrowserDownloadStatus::Completed),
        "cancelled" => Ok(BrowserDownloadStatus::Cancelled),
        "failed" => Ok(BrowserDownloadStatus::Failed),
        _ => Err(AgentStoreError::InvalidPersistedValue {
            kind: "browser download status",
            value: value.to_owned(),
        }),
    }
}

fn parse_lease_status(value: &str) -> Result<BrowserAutomationLeaseStatus, AgentStoreError> {
    match value {
        "pending" => Ok(BrowserAutomationLeaseStatus::Pending),
        "active" => Ok(BrowserAutomationLeaseStatus::Active),
        "suspended" => Ok(BrowserAutomationLeaseStatus::Suspended),
        "expired" => Ok(BrowserAutomationLeaseStatus::Expired),
        "failed" => Ok(BrowserAutomationLeaseStatus::Failed),
        value => Err(AgentStoreError::InvalidPersistedValue {
            kind: "browser lease status",
            value: value.into(),
        }),
    }
}

fn normalized_persisted_url(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case(NEW_TAB_URL) {
        return NEW_TAB_URL.to_owned();
    }
    let Ok(mut url) = Url::parse(value) else {
        return NEW_TAB_URL.to_owned();
    };
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
    {
        return NEW_TAB_URL.to_owned();
    }
    url.set_fragment(None);
    url.into()
}

fn canonical_history_url(value: &str) -> String {
    let Ok(mut url) = Url::parse(value) else {
        return value.to_owned();
    };
    url.set_fragment(None);
    if matches!(url.path(), "" | "/") {
        url.set_path("");
    }
    url.into()
}
