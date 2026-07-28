use hachimi_protocol::{RunId, RunUsageSnapshot, TokenCountSource, TokenUsage};
use sqlx::Row;

use super::{AgentStore, AgentStoreError, enum_from_db, enum_to_db};

impl AgentStore {
    pub async fn upsert_run_usage_snapshot(
        &self,
        snapshot: &RunUsageSnapshot,
    ) -> Result<RunUsageSnapshot, AgentStoreError> {
        sqlx::query(
            "INSERT INTO run_usage_snapshots (run_id, billed_input_tokens, billed_output_tokens, active_context_tokens, remaining_context_tokens, count_source, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(run_id) DO UPDATE SET billed_input_tokens = MAX(run_usage_snapshots.billed_input_tokens, excluded.billed_input_tokens), billed_output_tokens = MAX(run_usage_snapshots.billed_output_tokens, excluded.billed_output_tokens), active_context_tokens = excluded.active_context_tokens, remaining_context_tokens = excluded.remaining_context_tokens, count_source = excluded.count_source, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(snapshot.run_id.as_str())
        .bind(i64::try_from(snapshot.billed_usage.input_tokens).unwrap_or(i64::MAX))
        .bind(i64::try_from(snapshot.billed_usage.output_tokens).unwrap_or(i64::MAX))
        .bind(i64::try_from(snapshot.active_context_tokens).unwrap_or(i64::MAX))
        .bind(i64::try_from(snapshot.remaining_context_tokens).unwrap_or(i64::MAX))
        .bind(enum_to_db(&snapshot.source)?)
        .bind(snapshot.updated_at_ms)
        .execute(&self.pool)
        .await?;
        self.get_run_usage_snapshot(&snapshot.run_id)
            .await?
            .ok_or_else(|| AgentStoreError::RunNotFound(snapshot.run_id.clone()))
    }

    pub async fn get_run_usage_snapshot(
        &self,
        run_id: &RunId,
    ) -> Result<Option<RunUsageSnapshot>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM run_usage_snapshots WHERE run_id = ?")
            .bind(run_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(usage_from_row).transpose()
    }
}

pub(super) fn usage_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<RunUsageSnapshot, AgentStoreError> {
    Ok(RunUsageSnapshot {
        run_id: RunId::new(row.get::<String, _>("run_id")),
        billed_usage: TokenUsage {
            input_tokens: u64::try_from(row.get::<i64, _>("billed_input_tokens"))
                .unwrap_or_default(),
            output_tokens: u64::try_from(row.get::<i64, _>("billed_output_tokens"))
                .unwrap_or_default(),
        },
        active_context_tokens: u64::try_from(row.get::<i64, _>("active_context_tokens"))
            .unwrap_or_default(),
        remaining_context_tokens: u64::try_from(row.get::<i64, _>("remaining_context_tokens"))
            .unwrap_or_default(),
        source: enum_from_db::<TokenCountSource>(row.get("count_source"), "token count source")?,
        updated_at_ms: row.get("updated_at_ms"),
    })
}
