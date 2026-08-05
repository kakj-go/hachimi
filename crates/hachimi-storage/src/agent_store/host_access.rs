use hachimi_protocol::{
    BrowserAutomationSurfaceKind, BrowserCapability, BrowserSitePolicy, BrowserSitePolicyUpdate,
    ComputerAppDescriptor, ComputerAppPolicy, HostAccessDecision, HostAccessRequestRecord,
    HostAccessRequestStatus, HostAccessTarget, HostPolicyDecision, ItemId, RunId, RunStatus,
    SessionId,
};
use sqlx::{Row, Sqlite, Transaction};

use super::{AgentStore, AgentStoreError, now_ms};

const ACCESS_REQUEST_LIFETIME_MS: i64 = 10 * 60 * 1_000;

impl AgentStore {
    pub async fn list_computer_app_policies(
        &self,
    ) -> Result<Vec<ComputerAppPolicy>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM computer_app_policies ORDER BY updated_at_ms DESC, app_id ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(decode_computer_app_policy).collect()
    }

    pub async fn upsert_computer_app_policy(
        &self,
        app: &ComputerAppDescriptor,
        decision: HostPolicyDecision,
        expected_revision: Option<u64>,
    ) -> Result<ComputerAppPolicy, AgentStoreError> {
        let now = now_ms();
        let current =
            sqlx::query("SELECT revision FROM computer_app_policies WHERE identity_hash = ?")
                .bind(&app.identity_hash)
                .fetch_optional(&self.pool)
                .await?;
        if let Some(expected) = expected_revision
            && current.as_ref().map(|row| row.get::<i64, _>("revision"))
                != Some(i64::try_from(expected).unwrap_or(i64::MAX))
        {
            return Err(AgentStoreError::HostPolicyRevisionConflict);
        }
        let revision = current
            .as_ref()
            .map_or(1, |row| row.get::<i64, _>("revision").saturating_add(1));
        sqlx::query(
            "INSERT INTO computer_app_policies(identity_hash, app_id, descriptor_json, decision, revision, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?) ON CONFLICT(identity_hash) DO UPDATE SET app_id = excluded.app_id, descriptor_json = excluded.descriptor_json, decision = excluded.decision, revision = excluded.revision, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(&app.identity_hash)
        .bind(&app.app_id)
        .bind(serde_json::to_string(app)?)
        .bind(policy_decision_text(decision))
        .bind(revision)
        .bind(now)
        .execute(&self.pool)
        .await?;
        sqlx::query("SELECT * FROM computer_app_policies WHERE identity_hash = ?")
            .bind(&app.identity_hash)
            .fetch_one(&self.pool)
            .await
            .map_err(AgentStoreError::from)
            .and_then(|row| decode_computer_app_policy(&row))
    }

    pub async fn computer_host_policy_decision(
        &self,
        app: &ComputerAppDescriptor,
        session_id: &SessionId,
        run_id: &RunId,
    ) -> Result<HostPolicyDecision, AgentStoreError> {
        if let Some(row) =
            sqlx::query("SELECT decision FROM computer_app_policies WHERE identity_hash = ?")
                .bind(&app.identity_hash)
                .fetch_optional(&self.pool)
                .await?
        {
            let decision = parse_policy_decision(row.get::<String, _>("decision").as_str())?;
            if decision != HostPolicyDecision::Ask {
                return Ok(decision);
            }
        }
        let allowed = sqlx::query(
            "SELECT 1 FROM host_access_grants WHERE target_kind = 'computer' AND target_key = ? AND ((scope = 'run' AND owner_run_id = ?) OR (scope = 'session' AND owner_session_id = ?)) AND (expires_at_ms IS NULL OR expires_at_ms > ?) LIMIT 1",
        )
        .bind(&app.identity_hash)
        .bind(run_id.as_str())
        .bind(session_id.as_str())
        .bind(now_ms())
        .fetch_optional(&self.pool)
        .await?
        .is_some();
        Ok(if allowed {
            HostPolicyDecision::Allow
        } else {
            HostPolicyDecision::Ask
        })
    }

    pub async fn create_computer_host_access_request(
        &self,
        owner_session_id: &SessionId,
        owner_run_id: &RunId,
        run_generation: u64,
        app: &ComputerAppDescriptor,
    ) -> Result<HostAccessRequestRecord, AgentStoreError> {
        let now = now_ms();
        let expires = now.saturating_add(ACCESS_REQUEST_LIFETIME_MS);
        let target = HostAccessTarget::Computer { app: app.clone() };
        let capabilities = vec!["observe".to_owned(), "act".to_owned()];
        let mut transaction = self.pool.begin().await?;
        expire_host_access_requests(&mut transaction, now).await?;
        require_active_run(
            &mut transaction,
            owner_session_id,
            owner_run_id,
            run_generation,
        )
        .await?;
        sqlx::query(
            "INSERT OR IGNORE INTO host_access_requests(id, owner_session_id, owner_run_id, run_generation, target_kind, target_key, surface, target_json, capabilities_json, private_network, status, created_at_ms, expires_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, 'computer', ?, NULL, ?, ?, 0, 'pending', ?, ?, ?)",
        )
        .bind(ItemId::random().as_str())
        .bind(owner_session_id.as_str())
        .bind(owner_run_id.as_str())
        .bind(i64::try_from(run_generation).unwrap_or(i64::MAX))
        .bind(&app.identity_hash)
        .bind(serde_json::to_string(&target)?)
        .bind(serde_json::to_string(&capabilities)?)
        .bind(now)
        .bind(expires)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        let row = sqlx::query(
            "SELECT * FROM host_access_requests WHERE owner_run_id = ? AND target_kind = 'computer' AND target_key = ? AND status = 'pending' ORDER BY created_at_ms DESC LIMIT 1",
        )
        .bind(owner_run_id.as_str())
        .bind(&app.identity_hash)
        .fetch_one(&mut *transaction)
        .await?;
        let request = decode_host_access_request(&row)?;
        transaction.commit().await?;
        Ok(request)
    }

    pub async fn list_browser_site_policies(
        &self,
    ) -> Result<Vec<BrowserSitePolicy>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM browser_site_policies ORDER BY updated_at_ms DESC, origin ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(decode_browser_site_policy).collect()
    }

    pub async fn browser_site_policy(
        &self,
        origin: &str,
    ) -> Result<Option<BrowserSitePolicy>, AgentStoreError> {
        sqlx::query("SELECT * FROM browser_site_policies WHERE origin = ?")
            .bind(origin)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(decode_browser_site_policy)
            .transpose()
    }

    pub async fn upsert_browser_site_policy(
        &self,
        update: &BrowserSitePolicyUpdate,
        normalized_origin: &str,
        private_network: bool,
        allow_persistent_private_network: bool,
    ) -> Result<BrowserSitePolicy, AgentStoreError> {
        if private_network
            && update.decision == HostPolicyDecision::Allow
            && !allow_persistent_private_network
        {
            return Err(AgentStoreError::PersistentPrivateHostPolicyDenied);
        }
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;
        let current = sqlx::query("SELECT revision FROM browser_site_policies WHERE origin = ?")
            .bind(normalized_origin)
            .fetch_optional(&mut *transaction)
            .await?;
        if let Some(expected) = update.expected_revision
            && current.as_ref().map(|row| row.get::<i64, _>("revision"))
                != Some(i64::try_from(expected).unwrap_or(i64::MAX))
        {
            return Err(AgentStoreError::HostPolicyRevisionConflict);
        }
        let next_revision = current
            .as_ref()
            .map_or(1, |row| row.get::<i64, _>("revision").saturating_add(1));
        sqlx::query(
            "INSERT INTO browser_site_policies(origin, decision, capabilities_json, private_network, revision, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?) ON CONFLICT(origin) DO UPDATE SET decision = excluded.decision, capabilities_json = excluded.capabilities_json, private_network = excluded.private_network, revision = excluded.revision, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(normalized_origin)
        .bind(policy_decision_text(update.decision))
        .bind(serde_json::to_string(&update.capabilities)?)
        .bind(i64::from(private_network))
        .bind(next_revision)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        let row = sqlx::query("SELECT * FROM browser_site_policies WHERE origin = ?")
            .bind(normalized_origin)
            .fetch_one(&mut *transaction)
            .await?;
        let policy = decode_browser_site_policy(&row)?;
        transaction.commit().await?;
        Ok(policy)
    }

    pub async fn remove_browser_site_policy(&self, origin: &str) -> Result<bool, AgentStoreError> {
        Ok(
            sqlx::query("DELETE FROM browser_site_policies WHERE origin = ?")
                .bind(origin)
                .execute(&self.pool)
                .await?
                .rows_affected()
                == 1,
        )
    }

    pub async fn browser_host_policy_decision(
        &self,
        origin: &str,
        session_id: &SessionId,
        run_id: &RunId,
        capabilities: &[BrowserCapability],
        private_network: bool,
    ) -> Result<HostPolicyDecision, AgentStoreError> {
        if let Some(policy) = self.browser_site_policy(origin).await? {
            if policy.decision == HostPolicyDecision::Block {
                return Ok(HostPolicyDecision::Block);
            }
            if policy.decision == HostPolicyDecision::Allow
                && (!private_network || policy.private_network)
                && capabilities
                    .iter()
                    .all(|capability| policy.capabilities.contains(capability))
            {
                return Ok(HostPolicyDecision::Allow);
            }
        }
        let now = now_ms();
        let capability_keys = capabilities
            .iter()
            .map(|capability| capability_text(*capability))
            .collect::<Vec<_>>();
        let rows = sqlx::query(
            "SELECT capabilities_json, allow_private_network FROM host_access_grants WHERE target_kind = 'browser' AND target_key = ? AND ((scope = 'run' AND owner_run_id = ?) OR (scope = 'session' AND owner_session_id = ?)) AND (expires_at_ms IS NULL OR expires_at_ms > ?) ORDER BY CASE scope WHEN 'run' THEN 0 ELSE 1 END",
        )
        .bind(origin)
        .bind(run_id.as_str())
        .bind(session_id.as_str())
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        for row in rows {
            let allowed: Vec<String> =
                serde_json::from_str(&row.get::<String, _>("capabilities_json"))?;
            if (!private_network || row.get::<i64, _>("allow_private_network") != 0)
                && capability_keys
                    .iter()
                    .all(|value| allowed.iter().any(|allowed| allowed == value))
            {
                return Ok(HostPolicyDecision::Allow);
            }
        }
        Ok(HostPolicyDecision::Ask)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_browser_host_access_request(
        &self,
        owner_session_id: &SessionId,
        owner_run_id: &RunId,
        run_generation: u64,
        origin: &str,
        surface: BrowserAutomationSurfaceKind,
        capabilities: &[BrowserCapability],
        private_network: bool,
    ) -> Result<HostAccessRequestRecord, AgentStoreError> {
        let now = now_ms();
        let expires = now.saturating_add(ACCESS_REQUEST_LIFETIME_MS);
        let target = HostAccessTarget::Browser {
            origin: origin.to_owned(),
            surface,
            private_network,
        };
        let capability_keys = capabilities
            .iter()
            .map(|capability| capability_text(*capability).to_owned())
            .collect::<Vec<_>>();
        let mut transaction = self.pool.begin().await?;
        expire_host_access_requests(&mut transaction, now).await?;
        require_active_run(
            &mut transaction,
            owner_session_id,
            owner_run_id,
            run_generation,
        )
        .await?;
        let request_id = ItemId::random();
        sqlx::query(
            "INSERT OR IGNORE INTO host_access_requests(id, owner_session_id, owner_run_id, run_generation, target_kind, target_key, surface, target_json, capabilities_json, private_network, status, created_at_ms, expires_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, 'browser', ?, ?, ?, ?, ?, 'pending', ?, ?, ?)",
        )
        .bind(request_id.as_str())
        .bind(owner_session_id.as_str())
        .bind(owner_run_id.as_str())
        .bind(i64::try_from(run_generation).unwrap_or(i64::MAX))
        .bind(origin)
        .bind(surface_text(surface))
        .bind(serde_json::to_string(&target)?)
        .bind(serde_json::to_string(&capability_keys)?)
        .bind(i64::from(private_network))
        .bind(now)
        .bind(expires)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        let row = sqlx::query(
            "SELECT * FROM host_access_requests WHERE owner_run_id = ? AND target_kind = 'browser' AND target_key = ? AND surface = ? AND status = 'pending' ORDER BY created_at_ms DESC LIMIT 1",
        )
        .bind(owner_run_id.as_str())
        .bind(origin)
        .bind(surface_text(surface))
        .fetch_one(&mut *transaction)
        .await?;
        let request = decode_host_access_request(&row)?;
        transaction.commit().await?;
        Ok(request)
    }

    pub async fn list_host_access_requests(
        &self,
        session_id: Option<&SessionId>,
    ) -> Result<Vec<HostAccessRequestRecord>, AgentStoreError> {
        let now = now_ms();
        sqlx::query(
            "UPDATE host_access_requests SET status = 'expired', updated_at_ms = ? WHERE status = 'pending' AND expires_at_ms <= ?",
        )
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        let rows = if let Some(session_id) = session_id {
            sqlx::query(
                "SELECT * FROM host_access_requests WHERE owner_session_id = ? ORDER BY created_at_ms DESC LIMIT 500",
            )
            .bind(session_id.as_str())
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query("SELECT * FROM host_access_requests ORDER BY created_at_ms DESC LIMIT 500")
                .fetch_all(&self.pool)
                .await?
        };
        rows.iter().map(decode_host_access_request).collect()
    }

    pub async fn resolve_host_access_request(
        &self,
        request_id: &ItemId,
        decision: HostAccessDecision,
        allow_persistent_private_network: bool,
    ) -> Result<HostAccessRequestRecord, AgentStoreError> {
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;
        expire_host_access_requests(&mut transaction, now).await?;
        let row = sqlx::query(
            "SELECT requests.*, runs.generation AS current_generation, runs.status AS run_status FROM host_access_requests AS requests INNER JOIN runs ON runs.id = requests.owner_run_id WHERE requests.id = ?",
        )
        .bind(request_id.as_str())
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| AgentStoreError::HostAccessRequestNotFound(request_id.clone()))?;
        let request = decode_host_access_request(&row)?;
        if request.status != HostAccessRequestStatus::Pending
            || request.expires_at_ms <= now
            || row.get::<i64, _>("current_generation")
                != i64::try_from(request.run_generation).unwrap_or(i64::MAX)
            || RunStatus::parse(row.get::<String, _>("run_status").as_str())
                .is_none_or(RunStatus::is_terminal)
        {
            return Err(AgentStoreError::HostAccessRequestStale);
        }
        let (target_kind, target_key, private_network) = match &request.target {
            HostAccessTarget::Browser {
                origin,
                private_network,
                ..
            } => ("browser", origin.as_str(), *private_network),
            HostAccessTarget::Computer { app } => ("computer", app.identity_hash.as_str(), false),
        };
        if private_network
            && decision == HostAccessDecision::AlwaysAllow
            && !allow_persistent_private_network
        {
            return Err(AgentStoreError::PersistentPrivateHostPolicyDenied);
        }
        match decision {
            HostAccessDecision::AllowOnce | HostAccessDecision::AllowSession => {
                let (scope, scope_key, owner_run_id) = if decision == HostAccessDecision::AllowOnce
                {
                    (
                        "run",
                        format!("run:{}", request.owner_run_id),
                        Some(request.owner_run_id.as_str()),
                    )
                } else {
                    (
                        "session",
                        format!("session:{}", request.owner_session_id),
                        None,
                    )
                };
                sqlx::query(
                    "INSERT INTO host_access_grants(id, target_kind, target_key, scope, scope_key, owner_session_id, owner_run_id, capabilities_json, allow_private_network, expires_at_ms, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(target_kind, target_key, scope_key) DO UPDATE SET capabilities_json = excluded.capabilities_json, allow_private_network = excluded.allow_private_network, expires_at_ms = excluded.expires_at_ms, updated_at_ms = excluded.updated_at_ms",
                )
                .bind(ItemId::random().as_str())
                .bind(target_kind)
                .bind(target_key)
                .bind(scope)
                .bind(scope_key)
                .bind(request.owner_session_id.as_str())
                .bind(owner_run_id)
                .bind(serde_json::to_string(&request.capabilities)?)
                .bind(i64::from(private_network))
                .bind((scope == "run").then_some(request.expires_at_ms))
                .bind(now)
                .bind(now)
                .execute(&mut *transaction)
                .await?;
            }
            HostAccessDecision::AlwaysAllow | HostAccessDecision::AlwaysBlock => {
                let policy_decision = if decision == HostAccessDecision::AlwaysAllow {
                    HostPolicyDecision::Allow
                } else {
                    HostPolicyDecision::Block
                };
                match &request.target {
                    HostAccessTarget::Browser { origin, .. } => {
                        let capabilities = request
                            .capabilities
                            .iter()
                            .filter_map(|value| parse_capability(value))
                            .collect::<Vec<_>>();
                        sqlx::query(
                            "INSERT INTO browser_site_policies(origin, decision, capabilities_json, private_network, revision, updated_at_ms) VALUES(?, ?, ?, ?, 1, ?) ON CONFLICT(origin) DO UPDATE SET decision = excluded.decision, capabilities_json = excluded.capabilities_json, private_network = excluded.private_network, revision = browser_site_policies.revision + 1, updated_at_ms = excluded.updated_at_ms",
                        )
                        .bind(origin)
                        .bind(policy_decision_text(policy_decision))
                        .bind(serde_json::to_string(&capabilities)?)
                        .bind(i64::from(private_network))
                        .bind(now)
                        .execute(&mut *transaction)
                        .await?;
                    }
                    HostAccessTarget::Computer { app } => {
                        sqlx::query(
                            "INSERT INTO computer_app_policies(identity_hash, app_id, descriptor_json, decision, revision, updated_at_ms) VALUES(?, ?, ?, ?, 1, ?) ON CONFLICT(identity_hash) DO UPDATE SET app_id = excluded.app_id, descriptor_json = excluded.descriptor_json, decision = excluded.decision, revision = computer_app_policies.revision + 1, updated_at_ms = excluded.updated_at_ms",
                        )
                        .bind(&app.identity_hash)
                        .bind(&app.app_id)
                        .bind(serde_json::to_string(app)?)
                        .bind(policy_decision_text(policy_decision))
                        .bind(now)
                        .execute(&mut *transaction)
                        .await?;
                    }
                }
            }
            HostAccessDecision::Deny => {}
        }
        let status = if decision == HostAccessDecision::Deny {
            "denied"
        } else {
            "allowed"
        };
        sqlx::query(
            "UPDATE host_access_requests SET status = ?, updated_at_ms = ? WHERE id = ? AND status = 'pending'",
        )
        .bind(status)
        .bind(now)
        .bind(request_id.as_str())
        .execute(&mut *transaction)
        .await?;
        let row = sqlx::query("SELECT * FROM host_access_requests WHERE id = ?")
            .bind(request_id.as_str())
            .fetch_one(&mut *transaction)
            .await?;
        let resolved = decode_host_access_request(&row)?;
        transaction.commit().await?;
        Ok(resolved)
    }
}

async fn require_active_run(
    transaction: &mut Transaction<'_, Sqlite>,
    session_id: &SessionId,
    run_id: &RunId,
    generation: u64,
) -> Result<(), AgentStoreError> {
    let valid = sqlx::query(
        "SELECT 1 FROM runs WHERE id = ? AND session_id = ? AND generation = ? AND status NOT IN ('succeeded', 'failed', 'cancelled', 'interrupted')",
    )
    .bind(run_id.as_str())
    .bind(session_id.as_str())
    .bind(i64::try_from(generation).unwrap_or(i64::MAX))
    .fetch_optional(&mut **transaction)
    .await?
    .is_some();
    if valid {
        Ok(())
    } else {
        Err(AgentStoreError::HostAccessRequestStale)
    }
}

async fn expire_host_access_requests(
    transaction: &mut Transaction<'_, Sqlite>,
    now: i64,
) -> Result<(), AgentStoreError> {
    sqlx::query(
        "UPDATE host_access_requests SET status = 'expired', updated_at_ms = ? WHERE status = 'pending' AND expires_at_ms <= ?",
    )
    .bind(now)
    .bind(now)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn decode_browser_site_policy(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<BrowserSitePolicy, AgentStoreError> {
    Ok(BrowserSitePolicy {
        origin: row.get("origin"),
        decision: parse_policy_decision(row.get::<String, _>("decision").as_str())?,
        capabilities: serde_json::from_str(&row.get::<String, _>("capabilities_json"))?,
        private_network: row.get::<i64, _>("private_network") != 0,
        revision: persisted_u64(row, "revision", "browser site policy revision")?,
        updated_at_ms: row.get("updated_at_ms"),
    })
}

fn decode_computer_app_policy(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<ComputerAppPolicy, AgentStoreError> {
    Ok(ComputerAppPolicy {
        app: serde_json::from_str(&row.get::<String, _>("descriptor_json"))?,
        decision: parse_policy_decision(row.get::<String, _>("decision").as_str())?,
        revision: persisted_u64(row, "revision", "computer app policy revision")?,
        updated_at_ms: row.get("updated_at_ms"),
    })
}

fn decode_host_access_request(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<HostAccessRequestRecord, AgentStoreError> {
    Ok(HostAccessRequestRecord {
        id: ItemId::new(row.get::<String, _>("id")),
        owner_session_id: SessionId::new(row.get::<String, _>("owner_session_id")),
        owner_run_id: RunId::new(row.get::<String, _>("owner_run_id")),
        run_generation: persisted_u64(row, "run_generation", "host access generation")?,
        target: serde_json::from_str(&row.get::<String, _>("target_json"))?,
        capabilities: serde_json::from_str(&row.get::<String, _>("capabilities_json"))?,
        status: parse_request_status(row.get::<String, _>("status").as_str())?,
        created_at_ms: row.get("created_at_ms"),
        expires_at_ms: row.get("expires_at_ms"),
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

const fn surface_text(surface: BrowserAutomationSurfaceKind) -> &'static str {
    match surface {
        BrowserAutomationSurfaceKind::Embedded => "embedded",
        BrowserAutomationSurfaceKind::ExternalChrome => "external_chrome",
    }
}

const fn policy_decision_text(decision: HostPolicyDecision) -> &'static str {
    match decision {
        HostPolicyDecision::Ask => "ask",
        HostPolicyDecision::Allow => "allow",
        HostPolicyDecision::Block => "block",
    }
}

fn parse_policy_decision(value: &str) -> Result<HostPolicyDecision, AgentStoreError> {
    match value {
        "ask" => Ok(HostPolicyDecision::Ask),
        "allow" => Ok(HostPolicyDecision::Allow),
        "block" => Ok(HostPolicyDecision::Block),
        value => Err(AgentStoreError::InvalidPersistedValue {
            kind: "host policy decision",
            value: value.to_owned(),
        }),
    }
}

fn parse_request_status(value: &str) -> Result<HostAccessRequestStatus, AgentStoreError> {
    match value {
        "pending" => Ok(HostAccessRequestStatus::Pending),
        "allowed" => Ok(HostAccessRequestStatus::Allowed),
        "denied" => Ok(HostAccessRequestStatus::Denied),
        "expired" => Ok(HostAccessRequestStatus::Expired),
        value => Err(AgentStoreError::InvalidPersistedValue {
            kind: "host access request status",
            value: value.to_owned(),
        }),
    }
}

const fn capability_text(capability: BrowserCapability) -> &'static str {
    match capability {
        BrowserCapability::Observe => "observe",
        BrowserCapability::Act => "act",
        BrowserCapability::Upload => "upload",
        BrowserCapability::Download => "download",
        BrowserCapability::CookieStorage => "cookie_storage",
        BrowserCapability::Cdp => "cdp",
    }
}

fn parse_capability(value: &str) -> Option<BrowserCapability> {
    match value {
        "observe" => Some(BrowserCapability::Observe),
        "act" => Some(BrowserCapability::Act),
        "upload" => Some(BrowserCapability::Upload),
        "download" => Some(BrowserCapability::Download),
        "cookie_storage" => Some(BrowserCapability::CookieStorage),
        "cdp" => Some(BrowserCapability::Cdp),
        _ => None,
    }
}
