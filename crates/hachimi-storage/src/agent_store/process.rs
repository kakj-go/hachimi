//! Persistent process-session metadata.
//!
//! Output bytes remain in the bounded live/replay registry. SQLite stores only
//! lifecycle metadata so a restart can report a process as lost without
//! persisting terminal output or secrets.

use hachimi_protocol::{
    CheckoutId, ProcessSessionId, ProcessSessionRecord, ProcessStatus, RunId, SessionId,
};
use sqlx::{QueryBuilder, Row, Sqlite};

use super::{AgentStore, AgentStoreError};

impl AgentStore {
    pub async fn upsert_process_session(
        &self,
        record: &ProcessSessionRecord,
    ) -> Result<ProcessSessionRecord, AgentStoreError> {
        sqlx::query(
            "INSERT INTO process_sessions (id, session_id, run_id, checkout_id, run_generation, owner_client_id, command_summary, interactive, status, exit_code, output_limit_bytes, reconnect_expires_at_ms, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET status = excluded.status, exit_code = excluded.exit_code, reconnect_expires_at_ms = excluded.reconnect_expires_at_ms, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(record.id.as_str())
        .bind(record.session_id.as_str())
        .bind(record.run_id.as_ref().map(RunId::as_str))
        .bind(record.checkout_id.as_str())
        .bind(record.run_generation.map(|value| i64::try_from(value).unwrap_or(i64::MAX)))
        .bind(record.owner_client_id.0.as_str())
        .bind(&record.command_summary)
        .bind(record.interactive)
        .bind(record.status.as_str())
        .bind(record.exit_code)
        .bind(i64::try_from(record.output_limit_bytes).unwrap_or(i64::MAX))
        .bind(record.reconnect_expires_at_ms)
        .bind(record.created_at_ms)
        .bind(record.updated_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(record.clone())
    }

    pub async fn get_process_session(
        &self,
        id: &ProcessSessionId,
    ) -> Result<Option<ProcessSessionRecord>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM process_sessions WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.map(|row| process_session_from_row(&row)).transpose()
    }

    pub async fn list_process_sessions(
        &self,
        session_id: Option<&SessionId>,
        run_id: Option<&RunId>,
        include_terminal: bool,
    ) -> Result<Vec<ProcessSessionRecord>, AgentStoreError> {
        let mut query = QueryBuilder::<Sqlite>::new("SELECT * FROM process_sessions WHERE 1 = 1");
        if let Some(session_id) = session_id {
            query
                .push(" AND session_id = ")
                .push_bind(session_id.as_str());
        }
        if let Some(run_id) = run_id {
            query.push(" AND run_id = ").push_bind(run_id.as_str());
        }
        if !include_terminal {
            query.push(" AND status IN ('starting', 'running')");
        }
        query.push(" ORDER BY updated_at_ms DESC, id DESC");
        let rows = query.build().fetch_all(&self.pool).await?;
        rows.iter().map(process_session_from_row).collect()
    }
}

fn process_session_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<ProcessSessionRecord, AgentStoreError> {
    let status_value: String = row.get("status");
    let status = ProcessStatus::parse(&status_value).ok_or_else(|| {
        AgentStoreError::InvalidPersistedValue {
            kind: "process status",
            value: status_value,
        }
    })?;
    Ok(ProcessSessionRecord {
        id: ProcessSessionId::new(row.get::<String, _>("id")),
        session_id: SessionId::new(row.get::<String, _>("session_id")),
        run_id: row.get::<Option<String>, _>("run_id").map(RunId::new),
        checkout_id: CheckoutId::new(row.get::<String, _>("checkout_id")),
        run_generation: row
            .get::<Option<i64>, _>("run_generation")
            .map(|value| u64::try_from(value).unwrap_or_default()),
        owner_client_id: hachimi_protocol::ClientId(row.get("owner_client_id")),
        command_summary: row.get("command_summary"),
        interactive: row.get("interactive"),
        status,
        exit_code: row.get("exit_code"),
        output_limit_bytes: u64::try_from(row.get::<i64, _>("output_limit_bytes"))
            .unwrap_or_default(),
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
        reconnect_expires_at_ms: row.get("reconnect_expires_at_ms"),
    })
}
