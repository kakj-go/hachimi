use hachimi_protocol::{BrowserSessionId, RunId, SessionId};

use super::{AgentStore, AgentStoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopControlActionLedgerInput {
    pub session_id: SessionId,
    pub run_id: RunId,
    pub generation: u64,
    pub action_id: String,
    pub action_kind: String,
    pub target_fingerprint_hash: String,
    pub observation_revision: String,
    pub now_ms: i64,
}

impl AgentStore {
    pub async fn desktop_control_session_exists(
        &self,
        session_id: &SessionId,
    ) -> Result<bool, AgentStoreError> {
        let exists = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM desktop_control_sessions WHERE session_id = ?)",
        )
        .bind(session_id.as_str())
        .fetch_one(&self.pool)
        .await?;
        Ok(exists != 0)
    }

    pub async fn upsert_desktop_control_session(
        &self,
        session_id: &SessionId,
        now_ms: i64,
    ) -> Result<(), AgentStoreError> {
        sqlx::query(
            r#"INSERT INTO desktop_control_sessions(
                session_id, active_browser_session_id, selected_app_id,
                selected_window_fingerprint, input_epoch, control_state,
                last_observation_at_ms, updated_at_ms
             ) VALUES(?, NULL, NULL, NULL, 0, 'observing', NULL, ?)
             ON CONFLICT(session_id) DO UPDATE SET
                control_state = CASE
                    WHEN desktop_control_sessions.control_state = 'stopped' THEN 'observing'
                    ELSE desktop_control_sessions.control_state
                END,
                updated_at_ms = excluded.updated_at_ms"#,
        )
        .bind(session_id.as_str())
        .bind(now_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_desktop_control_browser_session(
        &self,
        session_id: &SessionId,
        browser_session_id: Option<&BrowserSessionId>,
        control_state: &str,
        now_ms: i64,
    ) -> Result<(), AgentStoreError> {
        sqlx::query(
            "UPDATE desktop_control_sessions SET active_browser_session_id = ?, \
             control_state = ?, updated_at_ms = ? WHERE session_id = ?",
        )
        .bind(browser_session_id.map(BrowserSessionId::as_str))
        .bind(control_state)
        .bind(now_ms)
        .bind(session_id.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn set_desktop_control_computer_observation(
        &self,
        session_id: &SessionId,
        app_id: Option<&str>,
        window_fingerprint: Option<&str>,
        input_epoch: u64,
        control_state: &str,
        observed_at_ms: Option<i64>,
        now_ms: i64,
    ) -> Result<(), AgentStoreError> {
        sqlx::query(
            "UPDATE desktop_control_sessions SET selected_app_id = ?, \
             selected_window_fingerprint = ?, input_epoch = ?, control_state = ?, \
             last_observation_at_ms = ?, updated_at_ms = ? WHERE session_id = ?",
        )
        .bind(app_id)
        .bind(window_fingerprint)
        .bind(i64::try_from(input_epoch).unwrap_or(i64::MAX))
        .bind(control_state)
        .bind(observed_at_ms)
        .bind(now_ms)
        .bind(session_id.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn touch_desktop_control_observation(
        &self,
        session_id: &SessionId,
        control_state: &str,
        observed_at_ms: i64,
    ) -> Result<(), AgentStoreError> {
        sqlx::query(
            "UPDATE desktop_control_sessions SET control_state = ?, \
             last_observation_at_ms = ?, updated_at_ms = ? WHERE session_id = ?",
        )
        .bind(control_state)
        .bind(observed_at_ms)
        .bind(observed_at_ms)
        .bind(session_id.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn prepare_desktop_control_action(
        &self,
        input: &DesktopControlActionLedgerInput,
    ) -> Result<bool, AgentStoreError> {
        let affected = sqlx::query(
            r#"INSERT INTO desktop_control_action_ledger(
                session_id, run_id, generation, action_id, action_kind,
                target_fingerprint_hash, observation_revision, status, result_code,
                created_at_ms, updated_at_ms
             ) SELECT ?, ?, ?, ?, ?, ?, ?, 'prepared', NULL, ?, ?
             WHERE EXISTS(SELECT 1 FROM desktop_control_sessions WHERE session_id = ?)
             ON CONFLICT(session_id, action_id) DO NOTHING"#,
        )
        .bind(input.session_id.as_str())
        .bind(input.run_id.as_str())
        .bind(i64::try_from(input.generation).unwrap_or(i64::MAX))
        .bind(&input.action_id)
        .bind(&input.action_kind)
        .bind(&input.target_fingerprint_hash)
        .bind(&input.observation_revision)
        .bind(input.now_ms)
        .bind(input.now_ms)
        .bind(input.session_id.as_str())
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(affected > 0)
    }

    pub async fn update_desktop_control_action(
        &self,
        session_id: &SessionId,
        action_id: &str,
        status: &str,
        result_code: Option<&str>,
        now_ms: i64,
    ) -> Result<(), AgentStoreError> {
        debug_assert!(matches!(
            status,
            "prepared" | "approved" | "dispatched" | "completed" | "denied" | "indeterminate"
        ));
        sqlx::query(
            "UPDATE desktop_control_action_ledger SET status = ?, result_code = ?, \
             updated_at_ms = ? WHERE session_id = ? AND action_id = ?",
        )
        .bind(status)
        .bind(result_code)
        .bind(now_ms)
        .bind(session_id.as_str())
        .bind(action_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
