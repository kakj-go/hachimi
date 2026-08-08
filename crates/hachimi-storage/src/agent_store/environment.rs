use hachimi_protocol::{
    AttachmentId, BrowserSession, BrowserSessionId, BrowserTabId, CheckoutId, CheckoutKind,
    ProjectId, RunId, SessionContextBinding, SessionId, SessionRecord, SessionSourceId,
    SessionSourceKind, SessionSourceOrigin, SessionSourceRecord,
};
use serde_json::json;
use sqlx::Row;
use sqlx::{Sqlite, Transaction};

use super::{AgentStore, AgentStoreError, append_event_tx, now_ms, session_from_row};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEnvironmentState {
    pub session_id: SessionId,
    pub baseline_revision: Option<String>,
    pub managed_checkout_id: Option<CheckoutId>,
    pub binding_revision: u64,
    pub revision: u64,
    pub inactive_head: Option<String>,
    pub inactive_status_fingerprint: Option<String>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCheckoutBindingUpdate {
    pub session_id: SessionId,
    pub project_id: ProjectId,
    pub target_checkout_id: CheckoutId,
    pub target_kind: CheckoutKind,
    pub expected_binding_revision: u64,
    pub inactive_head: Option<String>,
    pub inactive_status_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchHandoffJournalRecord {
    pub id: String,
    pub session_id: SessionId,
    pub source_checkout_id: CheckoutId,
    pub target_checkout_id: CheckoutId,
    pub phase: String,
    pub source_head: Option<String>,
    pub source_branch: Option<String>,
    pub source_status_fingerprint: String,
    pub target_head: Option<String>,
    pub target_branch: Option<String>,
    pub target_status_fingerprint: String,
    pub expected_binding_revision: u64,
    pub snapshot_path: String,
    pub snapshot_hash: String,
}

impl AgentStore {
    #[allow(clippy::too_many_arguments)]
    pub async fn start_workbench_handoff_journal(
        &self,
        id: &str,
        idempotency_key: &str,
        session_id: &SessionId,
        source_checkout_id: &CheckoutId,
        target_checkout_id: &CheckoutId,
        source_head: Option<&str>,
        source_branch: Option<&str>,
        source_status_fingerprint: &str,
        target_head: Option<&str>,
        target_branch: Option<&str>,
        target_status_fingerprint: &str,
        expected_binding_revision: u64,
        snapshot_path: &str,
        snapshot_hash: &str,
    ) -> Result<(), AgentStoreError> {
        let now = now_ms();
        sqlx::query(
            "INSERT INTO workbench_handoff_journal(id, idempotency_key, session_id, source_checkout_id, target_checkout_id, phase, source_head, source_branch, source_status_fingerprint, target_head, target_branch, target_status_fingerprint, expected_binding_revision, snapshot_path, snapshot_hash, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, 'prepared', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(idempotency_key)
        .bind(session_id.as_str())
        .bind(source_checkout_id.as_str())
        .bind(target_checkout_id.as_str())
        .bind(source_head)
        .bind(source_branch)
        .bind(source_status_fingerprint)
        .bind(target_head)
        .bind(target_branch)
        .bind(target_status_fingerprint)
        .bind(i64::try_from(expected_binding_revision).unwrap_or(i64::MAX))
        .bind(snapshot_path)
        .bind(snapshot_hash)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_unfinished_workbench_handoffs(
        &self,
    ) -> Result<Vec<WorkbenchHandoffJournalRecord>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM workbench_handoff_journal WHERE phase IN ('prepared', 'destination_applied', 'source_cleaned') ORDER BY created_at_ms ASC, id ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(handoff_journal_from_row).collect()
    }

    pub async fn update_workbench_handoff_phase(
        &self,
        id: &str,
        phase: &str,
        error_code: Option<&str>,
    ) -> Result<(), AgentStoreError> {
        sqlx::query(
            "UPDATE workbench_handoff_journal SET phase = ?, error_code = ?, updated_at_ms = ? WHERE id = ?",
        )
        .bind(phase)
        .bind(error_code)
        .bind(now_ms())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn ensure_session_environment_state(
        &self,
        session_id: &SessionId,
        checkout_id: &CheckoutId,
        checkout_kind: CheckoutKind,
        baseline_revision: Option<&str>,
    ) -> Result<SessionEnvironmentState, AgentStoreError> {
        let now = now_ms();
        let managed =
            (checkout_kind == CheckoutKind::ManagedWorktree).then_some(checkout_id.as_str());
        sqlx::query(
            "INSERT OR IGNORE INTO session_environment_state(session_id, baseline_revision, managed_checkout_id, binding_revision, revision, updated_at_ms) VALUES(?, ?, ?, 1, 1, ?)",
        )
        .bind(session_id.as_str())
        .bind(baseline_revision)
        .bind(managed)
        .bind(now)
        .execute(&self.pool)
        .await?;
        self.get_session_environment_state(session_id)
            .await?
            .ok_or_else(|| AgentStoreError::SessionNotFound(session_id.clone()))
    }

    pub async fn get_session_environment_state(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<SessionEnvironmentState>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM session_environment_state WHERE session_id = ?")
            .bind(session_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(environment_state_from_row).transpose()
    }

    pub async fn bump_session_environment_revision(
        &self,
        session_id: &SessionId,
    ) -> Result<u64, AgentStoreError> {
        let revision = sqlx::query_scalar::<_, i64>(
            "INSERT INTO session_environment_state(session_id, baseline_revision, managed_checkout_id, binding_revision, revision, updated_at_ms) SELECT id, NULL, NULL, 1, 1, ? FROM sessions WHERE id = ? ON CONFLICT(session_id) DO UPDATE SET revision = session_environment_state.revision + 1, updated_at_ms = excluded.updated_at_ms RETURNING revision",
        )
        .bind(now_ms())
        .bind(session_id.as_str())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| AgentStoreError::SessionNotFound(session_id.clone()))?;
        Ok(u64::try_from(revision).unwrap_or(u64::MAX))
    }

    pub async fn update_session_environment_baseline(
        &self,
        session_id: &SessionId,
        baseline_revision: Option<&str>,
    ) -> Result<SessionEnvironmentState, AgentStoreError> {
        let changed = sqlx::query(
            "UPDATE session_environment_state SET baseline_revision = ?, revision = revision + 1, updated_at_ms = ? WHERE session_id = ?",
        )
        .bind(baseline_revision)
        .bind(now_ms())
        .bind(session_id.as_str())
        .execute(&self.pool)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(AgentStoreError::SessionNotFound(session_id.clone()));
        }
        self.get_session_environment_state(session_id)
            .await?
            .ok_or_else(|| AgentStoreError::SessionNotFound(session_id.clone()))
    }

    pub async fn bind_session_checkout(
        &self,
        update: &SessionCheckoutBindingUpdate,
    ) -> Result<(SessionRecord, SessionEnvironmentState), AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let current_revision = sqlx::query_scalar::<_, i64>(
            "SELECT binding_revision FROM session_environment_state WHERE session_id = ?",
        )
        .bind(update.session_id.as_str())
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| AgentStoreError::SessionNotFound(update.session_id.clone()))?;
        if u64::try_from(current_revision).unwrap_or_default() != update.expected_binding_revision {
            return Err(AgentStoreError::RunPreconditionFailed);
        }
        let context = SessionContextBinding::Project {
            project_id: update.project_id.clone(),
            checkout_id: update.target_checkout_id.clone(),
        };
        let now = now_ms();
        sqlx::query("UPDATE sessions SET context_kind = 'project', context_json = ?, updated_at_ms = ? WHERE id = ?")
            .bind(serde_json::to_string(&context)?)
            .bind(now)
            .bind(update.session_id.as_str())
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "UPDATE session_environment_state SET managed_checkout_id = CASE WHEN ? = 'managed_worktree' THEN ? ELSE managed_checkout_id END, binding_revision = binding_revision + 1, revision = revision + 1, inactive_head = ?, inactive_status_fingerprint = ?, updated_at_ms = ? WHERE session_id = ?",
        )
        .bind(if update.target_kind == CheckoutKind::ManagedWorktree { "managed_worktree" } else { "local" })
        .bind(update.target_checkout_id.as_str())
        .bind(update.inactive_head.as_deref())
        .bind(&update.inactive_status_fingerprint)
        .bind(now)
        .bind(update.session_id.as_str())
        .execute(&mut *transaction)
        .await?;
        append_event_tx(
            &mut transaction,
            &update.session_id,
            None,
            "session.checkout_handed_off",
            json!({ "checkoutId": update.target_checkout_id, "kind": update.target_kind }),
            now,
        )
        .await?;
        let session_row = sqlx::query("SELECT * FROM sessions WHERE id = ?")
            .bind(update.session_id.as_str())
            .fetch_one(&mut *transaction)
            .await?;
        let state_row = sqlx::query("SELECT * FROM session_environment_state WHERE session_id = ?")
            .bind(update.session_id.as_str())
            .fetch_one(&mut *transaction)
            .await?;
        let session = session_from_row(&session_row)?;
        let state = environment_state_from_row(&state_row)?;
        transaction.commit().await?;
        Ok((session, state))
    }

    pub async fn upsert_session_upload_source(
        &self,
        session_id: &SessionId,
        run_id: Option<&RunId>,
        attachment_id: &AttachmentId,
        title: &str,
    ) -> Result<SessionSourceRecord, AgentStoreError> {
        self.upsert_session_source(
            session_id,
            run_id,
            SessionSourceKind::Upload,
            SessionSourceOrigin::Upload,
            &format!("attachment:{}", attachment_id.as_str()),
            Some(attachment_id),
            None,
            Some(title),
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_session_web_source(
        &self,
        session_id: &SessionId,
        run_id: Option<&RunId>,
        origin: SessionSourceOrigin,
        canonical_url: &str,
        title: Option<&str>,
        browser_tab_id: Option<&BrowserTabId>,
    ) -> Result<SessionSourceRecord, AgentStoreError> {
        let canonical_url = canonical_session_source_url(canonical_url).ok_or_else(|| {
            AgentStoreError::InvalidPersistedValue {
                kind: "session source URL",
                value: "invalid".into(),
            }
        })?;
        self.upsert_session_source(
            session_id,
            run_id,
            SessionSourceKind::Web,
            origin,
            &format!("url:{canonical_url}"),
            None,
            Some(&canonical_url),
            title,
            browser_tab_id,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn upsert_session_source(
        &self,
        session_id: &SessionId,
        run_id: Option<&RunId>,
        kind: SessionSourceKind,
        origin: SessionSourceOrigin,
        canonical_key: &str,
        attachment_id: Option<&AttachmentId>,
        url: Option<&str>,
        title: Option<&str>,
        browser_tab_id: Option<&BrowserTabId>,
    ) -> Result<SessionSourceRecord, AgentStoreError> {
        let now = now_ms();
        let id = SessionSourceId::random();
        sqlx::query(
            "INSERT INTO session_sources(id, session_id, run_id, kind, origin, canonical_key, attachment_id, url, title, browser_tab_id, created_at_ms, last_used_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(session_id, canonical_key) DO UPDATE SET run_id = COALESCE(excluded.run_id, session_sources.run_id), origin = excluded.origin, title = COALESCE(excluded.title, session_sources.title), browser_tab_id = COALESCE(excluded.browser_tab_id, session_sources.browser_tab_id), last_used_at_ms = excluded.last_used_at_ms",
        )
        .bind(id.as_str())
        .bind(session_id.as_str())
        .bind(run_id.map(RunId::as_str))
        .bind(source_kind_db(&kind))
        .bind(source_origin_db(&origin))
        .bind(canonical_key)
        .bind(attachment_id.map(AttachmentId::as_str))
        .bind(url)
        .bind(title)
        .bind(browser_tab_id.map(BrowserTabId::as_str))
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        self.bump_session_environment_revision(session_id).await?;
        let row =
            sqlx::query("SELECT * FROM session_sources WHERE session_id = ? AND canonical_key = ?")
                .bind(session_id.as_str())
                .bind(canonical_key)
                .fetch_one(&self.pool)
                .await?;
        session_source_from_row(&row)
    }

    pub async fn list_session_sources(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<SessionSourceRecord>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM session_sources WHERE session_id = ? ORDER BY last_used_at_ms DESC, id ASC",
        )
        .bind(session_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(session_source_from_row).collect()
    }

    pub async fn list_session_browser_sessions(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<BrowserSession>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT record_json FROM browser_sessions WHERE owner_session_id = ? ORDER BY updated_at_ms DESC, id ASC",
        )
        .bind(session_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                serde_json::from_str(row.get::<&str, _>("record_json"))
                    .map_err(AgentStoreError::from)
            })
            .collect()
    }

    pub async fn upsert_session_browser(
        &self,
        session: &BrowserSession,
    ) -> Result<(), AgentStoreError> {
        sqlx::query(
            "INSERT INTO browser_sessions(id, owner_session_id, owner_run_id, record_json, updated_at_ms, owner_run_generation) VALUES(?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET record_json = excluded.record_json, owner_run_generation = excluded.owner_run_generation, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(session.id.as_str())
        .bind(session.owner_session_id.as_str())
        .bind(session.owner_run_id.as_str())
        .bind(serde_json::to_string(session)?)
        .bind(super::now_ms())
        .bind(i64::try_from(session.run_generation).unwrap_or(i64::MAX))
        .execute(&self.pool)
        .await?;
        self.bump_session_environment_revision(&session.owner_session_id)
            .await?;
        Ok(())
    }

    pub async fn get_browser_session(
        &self,
        browser_session_id: &BrowserSessionId,
    ) -> Result<Option<BrowserSession>, AgentStoreError> {
        let row = sqlx::query("SELECT record_json FROM browser_sessions WHERE id = ?")
            .bind(browser_session_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.map(|row| {
            serde_json::from_str(row.get::<&str, _>("record_json")).map_err(AgentStoreError::from)
        })
        .transpose()
    }
}

pub(super) async fn bump_session_environment_revision_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    session_id: &SessionId,
    updated_at_ms: i64,
) -> Result<u64, AgentStoreError> {
    let revision = sqlx::query_scalar::<_, i64>(
        "INSERT INTO session_environment_state(session_id, baseline_revision, managed_checkout_id, binding_revision, revision, updated_at_ms) SELECT id, NULL, NULL, 1, 1, ? FROM sessions WHERE id = ? ON CONFLICT(session_id) DO UPDATE SET revision = session_environment_state.revision + 1, updated_at_ms = excluded.updated_at_ms RETURNING revision",
    )
    .bind(updated_at_ms)
    .bind(session_id.as_str())
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| AgentStoreError::SessionNotFound(session_id.clone()))?;
    Ok(u64::try_from(revision).unwrap_or(u64::MAX))
}

#[must_use]
pub fn canonical_session_source_url(value: &str) -> Option<String> {
    if value.trim().is_empty() || value.chars().count() > 4_096 {
        return None;
    }
    let mut url = url::Url::parse(value.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
    {
        return None;
    }
    url.set_fragment(None);
    Some(url.into())
}

fn environment_state_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<SessionEnvironmentState, AgentStoreError> {
    Ok(SessionEnvironmentState {
        session_id: SessionId::new(row.get::<String, _>("session_id")),
        baseline_revision: row.get("baseline_revision"),
        managed_checkout_id: row
            .get::<Option<String>, _>("managed_checkout_id")
            .map(CheckoutId::new),
        binding_revision: u64::try_from(row.get::<i64, _>("binding_revision")).unwrap_or_default(),
        revision: u64::try_from(row.get::<i64, _>("revision")).unwrap_or_default(),
        inactive_head: row.get("inactive_head"),
        inactive_status_fingerprint: row.get("inactive_status_fingerprint"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

fn handoff_journal_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<WorkbenchHandoffJournalRecord, AgentStoreError> {
    Ok(WorkbenchHandoffJournalRecord {
        id: row.get("id"),
        session_id: SessionId::new(row.get::<String, _>("session_id")),
        source_checkout_id: CheckoutId::new(row.get::<String, _>("source_checkout_id")),
        target_checkout_id: CheckoutId::new(row.get::<String, _>("target_checkout_id")),
        phase: row.get("phase"),
        source_head: row.get("source_head"),
        source_branch: row.get("source_branch"),
        source_status_fingerprint: row.get("source_status_fingerprint"),
        target_head: row.get("target_head"),
        target_branch: row.get("target_branch"),
        target_status_fingerprint: row.get("target_status_fingerprint"),
        expected_binding_revision: u64::try_from(row.get::<i64, _>("expected_binding_revision"))
            .unwrap_or_default(),
        snapshot_path: row.get("snapshot_path"),
        snapshot_hash: row.get("snapshot_hash"),
    })
}

fn session_source_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<SessionSourceRecord, AgentStoreError> {
    let kind = match row.get::<&str, _>("kind") {
        "upload" => SessionSourceKind::Upload,
        "web" => SessionSourceKind::Web,
        value => {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "session source kind",
                value: value.into(),
            });
        }
    };
    let origin = match row.get::<&str, _>("origin") {
        "upload" => SessionSourceOrigin::Upload,
        "browser" => SessionSourceOrigin::Browser,
        "mcp" => SessionSourceOrigin::Mcp,
        "connector" => SessionSourceOrigin::Connector,
        value => {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "session source origin",
                value: value.into(),
            });
        }
    };
    Ok(SessionSourceRecord {
        id: SessionSourceId::new(row.get::<String, _>("id")),
        session_id: SessionId::new(row.get::<String, _>("session_id")),
        run_id: row.get::<Option<String>, _>("run_id").map(RunId::new),
        kind,
        origin,
        attachment_id: row
            .get::<Option<String>, _>("attachment_id")
            .map(AttachmentId::new),
        url: row.get("url"),
        title: row.get("title"),
        browser_tab_id: row
            .get::<Option<String>, _>("browser_tab_id")
            .map(BrowserTabId::new),
        created_at_ms: row.get("created_at_ms"),
        last_used_at_ms: row.get("last_used_at_ms"),
    })
}

fn source_kind_db(kind: &SessionSourceKind) -> &'static str {
    match kind {
        SessionSourceKind::Upload => "upload",
        SessionSourceKind::Web => "web",
    }
}

fn source_origin_db(origin: &SessionSourceOrigin) -> &'static str {
    match origin {
        SessionSourceOrigin::Upload => "upload",
        SessionSourceOrigin::Browser => "browser",
        SessionSourceOrigin::Mcp => "mcp",
        SessionSourceOrigin::Connector => "connector",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_source_urls_reject_credentials_and_normalize_equivalent_urls() {
        assert_eq!(
            canonical_session_source_url(" HTTPS://Example.COM:443/docs?q=1#section "),
            Some("https://example.com/docs?q=1".into())
        );
        assert!(canonical_session_source_url("file:///tmp/source").is_none());
        assert!(canonical_session_source_url("https://user:secret@example.com/").is_none());
    }
}
