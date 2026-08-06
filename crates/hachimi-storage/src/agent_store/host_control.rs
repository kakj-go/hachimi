use hachimi_protocol::{
    ComputerAppDescriptor, ComputerControlSession, ComputerControlSessionId, ComputerControlStatus,
    ComputerFrame, RunId, SessionId,
};
use sqlx::Row;

use super::{AgentStore, AgentStoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostActionLedgerInput {
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
    pub async fn list_session_computer_control_sessions(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<ComputerControlSession>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM computer_control_sessions WHERE session_id = ? ORDER BY updated_at_ms DESC",
        )
        .bind(session_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                let app_id = row.get::<Option<String>, _>("selected_app_id");
                let input_epoch =
                    u64::try_from(row.get::<i64, _>("input_epoch")).map_err(|_| {
                        AgentStoreError::InvalidPersistedValue {
                            kind: "computer control input epoch",
                            value: row.get::<i64, _>("input_epoch").to_string(),
                        }
                    })?;
                Ok(ComputerControlSession {
                    id: ComputerControlSessionId::new(row.get::<String, _>("session_id")),
                    owner_session_id: SessionId::new(row.get::<String, _>("session_id")),
                    owner_run_id: row.get::<Option<String>, _>("owner_run_id").map(RunId::new),
                    run_generation: row
                        .get::<Option<i64>, _>("owner_run_generation")
                        .map(|value| u64::try_from(value).unwrap_or_default()),
                    app: row
                        .get::<Option<String>, _>("app_descriptor_json")
                        .map(|value| serde_json::from_str(&value))
                        .transpose()?
                        .or_else(|| {
                            app_id.map(|app_id| ComputerAppDescriptor {
                                display_name: app_id.clone(),
                                identity_hash: app_id.clone(),
                                executable_name: app_id.clone(),
                                app_id,
                                executable_path: None,
                                publisher: None,
                                publisher_verified: false,
                                package_family_name: None,
                                app_user_model_id: None,
                                file_identity: None,
                            })
                        }),
                    window: row
                        .get::<Option<String>, _>("window_json")
                        .map(|value| serde_json::from_str(&value))
                        .transpose()?,
                    latest_frame: row
                        .get::<Option<String>, _>("latest_frame_json")
                        .map(|value| serde_json::from_str(&value))
                        .transpose()?,
                    status: match row.get::<String, _>("control_state").as_str() {
                        "observing" | "controlling" => ComputerControlStatus::Active,
                        "taken_over" | "needs_attention" => ComputerControlStatus::Suspended,
                        "stopped" => ComputerControlStatus::Stopped,
                        value => {
                            return Err(AgentStoreError::InvalidPersistedValue {
                                kind: "computer control status",
                                value: value.to_owned(),
                            });
                        }
                    },
                    revision: u64::try_from(row.get::<i64, _>("revision"))
                        .unwrap_or_else(|_| input_epoch.saturating_add(1)),
                    updated_at_ms: row.get("updated_at_ms"),
                })
            })
            .collect()
    }

    pub async fn store_computer_control_frame(
        &self,
        frame: &ComputerFrame,
        app: &ComputerAppDescriptor,
        now_ms: i64,
    ) -> Result<(), AgentStoreError> {
        let mut persisted_frame = frame.clone();
        persisted_frame.image_token.clear();
        sqlx::query(
            "UPDATE computer_control_sessions SET owner_run_id = ?, owner_run_generation = ?, app_descriptor_json = ?, window_json = ?, latest_frame_json = ?, revision = revision + 1, updated_at_ms = ? WHERE session_id = ?",
        )
        .bind(frame.run_id.as_str())
        .bind(i64::try_from(frame.run_generation).unwrap_or(i64::MAX))
        .bind(serde_json::to_string(app)?)
        .bind(serde_json::to_string(&frame.target)?)
        .bind(serde_json::to_string(&persisted_frame)?)
        .bind(now_ms)
        .bind(frame.session_id.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn set_computer_control_observation(
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
            r#"INSERT INTO computer_control_sessions(
                session_id, selected_app_id, selected_window_fingerprint, input_epoch,
                control_state, last_observation_at_ms, updated_at_ms
             ) VALUES(?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(session_id) DO UPDATE SET
                selected_app_id = excluded.selected_app_id,
                selected_window_fingerprint = excluded.selected_window_fingerprint,
                input_epoch = excluded.input_epoch,
                control_state = excluded.control_state,
                last_observation_at_ms = excluded.last_observation_at_ms,
                revision = computer_control_sessions.revision + 1,
                updated_at_ms = excluded.updated_at_ms"#,
        )
        .bind(session_id.as_str())
        .bind(app_id)
        .bind(window_fingerprint)
        .bind(i64::try_from(input_epoch).unwrap_or(i64::MAX))
        .bind(control_state)
        .bind(observed_at_ms)
        .bind(now_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn prepare_host_action(
        &self,
        input: &HostActionLedgerInput,
    ) -> Result<bool, AgentStoreError> {
        let affected = sqlx::query(
            r#"INSERT INTO host_action_ledger(
                session_id, run_id, generation, action_id, action_kind,
                target_fingerprint_hash, observation_revision, status, result_code,
                created_at_ms, updated_at_ms
             ) SELECT ?, ?, ?, ?, ?, ?, ?, 'prepared', NULL, ?, ?
             WHERE EXISTS(SELECT 1 FROM sessions WHERE id = ?)
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

    pub async fn update_host_action(
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
            "UPDATE host_action_ledger SET status = ?, result_code = ?, \
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
