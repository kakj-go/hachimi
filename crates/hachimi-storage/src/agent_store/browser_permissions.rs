use hachimi_protocol::{
    BrowserAutomationLeaseId, BrowserCapability, BrowserPermissionDecision,
    BrowserPermissionRequestStatus, BrowserTabId, BrowserWorkspaceId,
    EmbeddedBrowserPermissionRequest, EmbeddedBrowserPermissionScope,
    EmbeddedBrowserSitePermission, ItemId, RunId, RunStatus, SessionId,
};
use sqlx::{Row, Sqlite, Transaction};

use super::{AgentStore, AgentStoreError, now_ms};

const PERMISSION_REQUEST_LIFETIME_MS: i64 = 10 * 60 * 1_000;

impl AgentStore {
    pub async fn embedded_browser_site_permission(
        &self,
        origin: &str,
        session_id: &SessionId,
        run_id: &RunId,
        require_private_network: bool,
    ) -> Result<Option<EmbeddedBrowserSitePermission>, AgentStoreError> {
        let now = now_ms();
        sqlx::query(
            "DELETE FROM embedded_browser_site_permissions WHERE expires_at_ms IS NOT NULL AND expires_at_ms <= ?",
        )
        .bind(now)
        .execute(&self.pool)
        .await?;
        let rows = sqlx::query(
            "SELECT * FROM embedded_browser_site_permissions WHERE origin = ? AND (scope = 'persisted' OR (scope = 'session' AND owner_session_id = ?) OR (scope = 'once' AND owner_run_id = ?)) ORDER BY CASE scope WHEN 'once' THEN 0 WHEN 'session' THEN 1 ELSE 2 END, updated_at_ms DESC",
        )
        .bind(origin)
        .bind(session_id.as_str())
        .bind(run_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        for row in rows {
            let permission = decode_site_permission(&row)?;
            if !require_private_network || permission.allow_private_network {
                return Ok(Some(permission));
            }
        }
        Ok(None)
    }

    pub async fn embedded_browser_allowed_origins(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
    ) -> Result<Vec<String>, AgentStoreError> {
        let now = now_ms();
        let mut candidates = std::collections::BTreeMap::new();
        let mut embedded_grants = std::collections::BTreeSet::new();
        let legacy_rows = sqlx::query(
            "SELECT origin, capabilities_json, allow_private_network FROM embedded_browser_site_permissions WHERE (scope = 'session' AND owner_session_id = ? OR scope = 'once' AND owner_run_id = ?) AND (expires_at_ms IS NULL OR expires_at_ms > ?)",
        )
        .bind(session_id.as_str())
        .bind(run_id.as_str())
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        for row in legacy_rows {
            let capabilities = serde_json::from_str::<Vec<BrowserCapability>>(
                &row.get::<String, _>("capabilities_json"),
            )?;
            if !capabilities.contains(&BrowserCapability::Observe)
                || !capabilities.contains(&BrowserCapability::Act)
            {
                continue;
            }
            let origin = row.get::<String, _>("origin");
            embedded_grants.insert(origin.clone());
            let private_network = row.get::<i64, _>("allow_private_network") != 0;
            candidates
                .entry(origin)
                .and_modify(|current| *current |= private_network)
                .or_insert(private_network);
        }

        let policy_rows = sqlx::query(
            "SELECT origin, private_network FROM browser_site_policies WHERE decision = 'allow'",
        )
        .fetch_all(&self.pool)
        .await?;
        for row in policy_rows {
            let private_network = row.get::<i64, _>("private_network") != 0;
            candidates
                .entry(row.get::<String, _>("origin"))
                .and_modify(|current| *current |= private_network)
                .or_insert(private_network);
        }

        let grant_rows = sqlx::query(
            "SELECT target_key, MAX(allow_private_network) AS private_network FROM host_access_grants WHERE target_kind = 'browser' AND ((scope = 'run' AND owner_run_id = ?) OR (scope = 'session' AND owner_session_id = ?)) AND (expires_at_ms IS NULL OR expires_at_ms > ?) GROUP BY target_key",
        )
        .bind(run_id.as_str())
        .bind(session_id.as_str())
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        for row in grant_rows {
            let private_network = row.get::<i64, _>("private_network") != 0;
            candidates
                .entry(row.get::<String, _>("target_key"))
                .and_modify(|current| *current |= private_network)
                .or_insert(private_network);
        }

        let mut allowed = Vec::new();
        for (origin, private_network) in candidates {
            let decision = self
                .browser_host_policy_decision(
                    &origin,
                    session_id,
                    run_id,
                    &[
                        hachimi_protocol::BrowserCapability::Observe,
                        hachimi_protocol::BrowserCapability::Act,
                    ],
                    private_network,
                )
                .await?;
            if decision == hachimi_protocol::HostPolicyDecision::Allow
                || (decision != hachimi_protocol::HostPolicyDecision::Block
                    && embedded_grants.contains(&origin))
            {
                allowed.push(origin);
            }
        }
        Ok(allowed)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_embedded_browser_permission_request(
        &self,
        workspace_id: &BrowserWorkspaceId,
        tab_id: &BrowserTabId,
        automation_lease_id: Option<&BrowserAutomationLeaseId>,
        owner_session_id: &SessionId,
        owner_run_id: &RunId,
        run_generation: u64,
        origin: &str,
        private_network: bool,
        expected_tab_revision: u64,
    ) -> Result<EmbeddedBrowserPermissionRequest, AgentStoreError> {
        let now = now_ms();
        let expires = now.saturating_add(PERMISSION_REQUEST_LIFETIME_MS);
        let capabilities = vec![BrowserCapability::Observe, BrowserCapability::Act];
        let mut transaction = self.pool.begin().await?;
        expire_pending_requests(&mut transaction, now).await?;
        let valid_owner = sqlx::query(
            "SELECT 1 FROM runs WHERE id = ? AND session_id = ? AND generation = ? AND status NOT IN ('succeeded', 'failed', 'cancelled', 'interrupted')",
        )
        .bind(owner_run_id.as_str())
        .bind(owner_session_id.as_str())
        .bind(i64::try_from(run_generation).unwrap_or(i64::MAX))
        .fetch_optional(&mut *transaction)
        .await?
        .is_some();
        if !valid_owner {
            return Err(AgentStoreError::EmbeddedBrowserPermissionRequestStale);
        }
        let request_id = ItemId::random();
        sqlx::query(
            "INSERT OR IGNORE INTO embedded_browser_permission_requests(id, workspace_id, tab_id, automation_lease_id, owner_session_id, owner_run_id, run_generation, origin, capabilities_json, private_network, status, expected_tab_revision, created_at_ms, expires_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?, ?, ?)",
        )
        .bind(request_id.as_str())
        .bind(workspace_id.as_str())
        .bind(tab_id.as_str())
        .bind(automation_lease_id.map(BrowserAutomationLeaseId::as_str))
        .bind(owner_session_id.as_str())
        .bind(owner_run_id.as_str())
        .bind(i64::try_from(run_generation).unwrap_or(i64::MAX))
        .bind(origin)
        .bind(serde_json::to_string(&capabilities)?)
        .bind(i64::from(private_network))
        .bind(i64::try_from(expected_tab_revision).unwrap_or(i64::MAX))
        .bind(now)
        .bind(expires)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        let row = sqlx::query(
            "SELECT * FROM embedded_browser_permission_requests WHERE owner_run_id = ? AND origin = ? AND status = 'pending' ORDER BY created_at_ms DESC LIMIT 1",
        )
        .bind(owner_run_id.as_str())
        .bind(origin)
        .fetch_one(&mut *transaction)
        .await?;
        let request = decode_permission_request(&row)?;
        transaction.commit().await?;
        Ok(request)
    }

    pub async fn embedded_browser_permission_requests(
        &self,
        owner_session_id: Option<&SessionId>,
    ) -> Result<Vec<EmbeddedBrowserPermissionRequest>, AgentStoreError> {
        let now = now_ms();
        sqlx::query(
            "UPDATE embedded_browser_permission_requests SET status = 'expired', updated_at_ms = ? WHERE status = 'pending' AND expires_at_ms <= ?",
        )
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        let rows = if let Some(session_id) = owner_session_id {
            sqlx::query(
                "SELECT * FROM embedded_browser_permission_requests WHERE owner_session_id = ? ORDER BY created_at_ms DESC LIMIT 500",
            )
            .bind(session_id.as_str())
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT * FROM embedded_browser_permission_requests ORDER BY created_at_ms DESC LIMIT 500",
            )
            .fetch_all(&self.pool)
            .await?
        };
        rows.iter().map(decode_permission_request).collect()
    }

    pub async fn embedded_browser_site_permissions(
        &self,
    ) -> Result<Vec<EmbeddedBrowserSitePermission>, AgentStoreError> {
        let now = now_ms();
        sqlx::query(
            "DELETE FROM embedded_browser_site_permissions WHERE expires_at_ms IS NOT NULL AND expires_at_ms <= ?",
        )
        .bind(now)
        .execute(&self.pool)
        .await?;
        let rows = sqlx::query(
            "SELECT * FROM embedded_browser_site_permissions ORDER BY updated_at_ms DESC, origin ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(decode_site_permission).collect()
    }

    pub async fn resolve_embedded_browser_permission_request(
        &self,
        request_id: &ItemId,
        decision: BrowserPermissionDecision,
    ) -> Result<EmbeddedBrowserPermissionRequest, AgentStoreError> {
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;
        expire_pending_requests(&mut transaction, now).await?;
        let row = sqlx::query(
            "SELECT requests.*, runs.generation AS current_generation, runs.status AS run_status FROM embedded_browser_permission_requests AS requests INNER JOIN runs ON runs.id = requests.owner_run_id WHERE requests.id = ?",
        )
        .bind(request_id.as_str())
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| {
            AgentStoreError::EmbeddedBrowserPermissionRequestNotFound(request_id.clone())
        })?;
        let request = decode_permission_request(&row)?;
        let generation = row.get::<i64, _>("current_generation");
        let run_status = RunStatus::parse(row.get::<String, _>("run_status").as_str());
        if request.status != BrowserPermissionRequestStatus::Pending
            || request.expires_at_ms <= now
            || generation != i64::try_from(request.run_generation).unwrap_or(i64::MAX)
            || run_status.is_none_or(RunStatus::is_terminal)
        {
            return Err(AgentStoreError::EmbeddedBrowserPermissionRequestStale);
        }
        let status = if decision == BrowserPermissionDecision::Deny {
            BrowserPermissionRequestStatus::Denied
        } else {
            let (scope, scope_key, owner_session_id, owner_run_id, expires_at_ms) = match decision {
                BrowserPermissionDecision::AllowOnce => (
                    EmbeddedBrowserPermissionScope::Once,
                    format!("run:{}", request.owner_run_id),
                    Some(request.owner_session_id.as_str()),
                    Some(request.owner_run_id.as_str()),
                    Some(request.expires_at_ms),
                ),
                BrowserPermissionDecision::AllowSession => (
                    EmbeddedBrowserPermissionScope::Session,
                    format!("session:{}", request.owner_session_id),
                    Some(request.owner_session_id.as_str()),
                    None,
                    None,
                ),
                BrowserPermissionDecision::AllowPersisted => (
                    EmbeddedBrowserPermissionScope::Persisted,
                    "profile:global-policy".to_owned(),
                    None,
                    None,
                    None,
                ),
                BrowserPermissionDecision::Deny => unreachable!(),
            };
            if scope == EmbeddedBrowserPermissionScope::Persisted {
                if request.private_network {
                    return Err(AgentStoreError::PersistentPrivateHostPolicyDenied);
                }
                sqlx::query(
                    "INSERT INTO browser_site_policies(origin, decision, capabilities_json, private_network, revision, updated_at_ms) VALUES(?, 'allow', ?, 0, 1, ?) ON CONFLICT(origin) DO UPDATE SET decision = 'allow', capabilities_json = excluded.capabilities_json, private_network = 0, revision = browser_site_policies.revision + 1, updated_at_ms = excluded.updated_at_ms",
                )
                .bind(&request.origin)
                .bind(serde_json::to_string(&request.capabilities)?)
                .bind(now)
                .execute(&mut *transaction)
                .await?;
            } else {
                let permission_id = ItemId::random().to_string();
                sqlx::query(
                    "INSERT INTO embedded_browser_site_permissions(id, origin, scope, scope_key, owner_session_id, owner_run_id, capabilities_json, allow_private_network, created_at_ms, expires_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(origin, scope_key) DO UPDATE SET capabilities_json = excluded.capabilities_json, allow_private_network = excluded.allow_private_network, expires_at_ms = excluded.expires_at_ms, updated_at_ms = excluded.updated_at_ms",
                )
                .bind(permission_id)
                .bind(&request.origin)
                .bind(permission_scope_text(scope))
                .bind(scope_key)
                .bind(owner_session_id)
                .bind(owner_run_id)
                .bind(serde_json::to_string(&request.capabilities)?)
                .bind(i64::from(request.private_network))
                .bind(now)
                .bind(expires_at_ms)
                .bind(now)
                .execute(&mut *transaction)
                .await?;
            }
            BrowserPermissionRequestStatus::Allowed
        };
        sqlx::query(
            "UPDATE embedded_browser_permission_requests SET status = ?, updated_at_ms = ? WHERE id = ? AND status = 'pending'",
        )
        .bind(permission_request_status_text(status))
        .bind(now)
        .bind(request_id.as_str())
        .execute(&mut *transaction)
        .await?;
        let row = sqlx::query("SELECT * FROM embedded_browser_permission_requests WHERE id = ?")
            .bind(request_id.as_str())
            .fetch_one(&mut *transaction)
            .await?;
        let resolved = decode_permission_request(&row)?;
        transaction.commit().await?;
        Ok(resolved)
    }

    pub async fn revoke_embedded_browser_site_permission(
        &self,
        permission_id: &str,
    ) -> Result<bool, AgentStoreError> {
        Ok(
            sqlx::query("DELETE FROM embedded_browser_site_permissions WHERE id = ?")
                .bind(permission_id)
                .execute(&self.pool)
                .await?
                .rows_affected()
                == 1,
        )
    }
}

async fn expire_pending_requests(
    transaction: &mut Transaction<'_, Sqlite>,
    now: i64,
) -> Result<(), AgentStoreError> {
    sqlx::query(
        "UPDATE embedded_browser_permission_requests SET status = 'expired', updated_at_ms = ? WHERE status = 'pending' AND expires_at_ms <= ?",
    )
    .bind(now)
    .bind(now)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn decode_permission_request(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<EmbeddedBrowserPermissionRequest, AgentStoreError> {
    Ok(EmbeddedBrowserPermissionRequest {
        id: ItemId::new(row.get::<String, _>("id")),
        workspace_id: BrowserWorkspaceId::new(row.get::<String, _>("workspace_id")),
        tab_id: BrowserTabId::new(row.get::<String, _>("tab_id")),
        automation_lease_id: row
            .get::<Option<String>, _>("automation_lease_id")
            .map(BrowserAutomationLeaseId::new),
        owner_session_id: SessionId::new(row.get::<String, _>("owner_session_id")),
        owner_run_id: RunId::new(row.get::<String, _>("owner_run_id")),
        run_generation: persisted_u64(row, "run_generation", "browser permission generation")?,
        origin: row.get("origin"),
        capabilities: serde_json::from_str(&row.get::<String, _>("capabilities_json"))?,
        private_network: row.get::<i64, _>("private_network") != 0,
        status: parse_permission_request_status(row.get::<String, _>("status").as_str())?,
        expected_tab_revision: persisted_u64(
            row,
            "expected_tab_revision",
            "browser permission tab revision",
        )?,
        created_at_ms: row.get("created_at_ms"),
        expires_at_ms: row.get("expires_at_ms"),
    })
}

fn decode_site_permission(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<EmbeddedBrowserSitePermission, AgentStoreError> {
    Ok(EmbeddedBrowserSitePermission {
        id: row.get("id"),
        origin: row.get("origin"),
        scope: parse_permission_scope(row.get::<String, _>("scope").as_str())?,
        owner_session_id: row
            .get::<Option<String>, _>("owner_session_id")
            .map(SessionId::new),
        owner_run_id: row.get::<Option<String>, _>("owner_run_id").map(RunId::new),
        capabilities: serde_json::from_str(&row.get::<String, _>("capabilities_json"))?,
        allow_private_network: row.get::<i64, _>("allow_private_network") != 0,
        created_at_ms: row.get("created_at_ms"),
        expires_at_ms: row.get("expires_at_ms"),
    })
}

fn persisted_u64(
    row: &sqlx::sqlite::SqliteRow,
    column: &str,
    kind: &'static str,
) -> Result<u64, AgentStoreError> {
    u64::try_from(row.get::<i64, _>(column)).map_err(|_| AgentStoreError::InvalidPersistedValue {
        kind,
        value: row.get::<i64, _>(column).to_string(),
    })
}

const fn permission_scope_text(scope: EmbeddedBrowserPermissionScope) -> &'static str {
    match scope {
        EmbeddedBrowserPermissionScope::Once => "once",
        EmbeddedBrowserPermissionScope::Session => "session",
        EmbeddedBrowserPermissionScope::Persisted => "persisted",
    }
}

fn parse_permission_scope(value: &str) -> Result<EmbeddedBrowserPermissionScope, AgentStoreError> {
    match value {
        "once" => Ok(EmbeddedBrowserPermissionScope::Once),
        "session" => Ok(EmbeddedBrowserPermissionScope::Session),
        "persisted" => Ok(EmbeddedBrowserPermissionScope::Persisted),
        value => Err(AgentStoreError::InvalidPersistedValue {
            kind: "embedded browser permission scope",
            value: value.to_owned(),
        }),
    }
}

const fn permission_request_status_text(status: BrowserPermissionRequestStatus) -> &'static str {
    match status {
        BrowserPermissionRequestStatus::Pending => "pending",
        BrowserPermissionRequestStatus::Allowed => "allowed",
        BrowserPermissionRequestStatus::Denied => "denied",
        BrowserPermissionRequestStatus::Expired => "expired",
    }
}

fn parse_permission_request_status(
    value: &str,
) -> Result<BrowserPermissionRequestStatus, AgentStoreError> {
    match value {
        "pending" => Ok(BrowserPermissionRequestStatus::Pending),
        "allowed" => Ok(BrowserPermissionRequestStatus::Allowed),
        "denied" => Ok(BrowserPermissionRequestStatus::Denied),
        "expired" => Ok(BrowserPermissionRequestStatus::Expired),
        value => Err(AgentStoreError::InvalidPersistedValue {
            kind: "embedded browser permission request status",
            value: value.to_owned(),
        }),
    }
}
