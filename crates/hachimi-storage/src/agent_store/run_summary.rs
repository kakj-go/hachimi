use hachimi_protocol::{
    ArtifactId, ProjectId, RunId, RunStatus, RunSummaryFile, RunSummaryRecord, SessionId,
    SessionRunActivity, WorkbenchSessionListItem,
};
use sqlx::Row;

use super::{AgentStore, AgentStoreError, session_from_row};

impl AgentStore {
    pub async fn finalize_run_summary(
        &self,
        run_id: &RunId,
        status: RunStatus,
        completed_at_ms: i64,
    ) -> Result<RunSummaryRecord, AgentStoreError> {
        let snapshot = self.get_run_diff_manifest(run_id).await?;
        let files = snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .files
                    .iter()
                    .map(|file| RunSummaryFile {
                        path: file.path.clone(),
                        previous_path: file.previous_path.clone(),
                        status: file.status,
                        additions: file.additions,
                        deletions: file.deletions,
                        binary: file.binary,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let additions = files
            .iter()
            .fold(0_u32, |total, file| total.saturating_add(file.additions));
        let deletions = files
            .iter()
            .fold(0_u32, |total, file| total.saturating_add(file.deletions));
        let summary = RunSummaryRecord {
            run_id: run_id.clone(),
            status,
            changed_files: u32::try_from(files.len()).unwrap_or(u32::MAX),
            additions,
            deletions,
            diff_artifact_id: snapshot
                .as_ref()
                .and_then(|value| value.artifact_id.clone()),
            diff_unavailable: snapshot.is_none(),
            files,
            completed_at_ms,
        };
        self.put_run_summary(&summary).await
    }

    pub async fn list_workbench_session_items(
        &self,
        project_id: Option<&ProjectId>,
    ) -> Result<Vec<WorkbenchSessionListItem>, AgentStoreError> {
        let rows = if let Some(project_id) = project_id {
            sqlx::query(
                "SELECT sessions.*, latest.id AS latest_run_id, latest.status AS latest_run_status, latest.updated_at_ms AS latest_run_updated_at_ms, terminal.id AS terminal_run_id, terminal.status AS terminal_run_status, terminal.updated_at_ms AS terminal_run_updated_at_ms FROM sessions LEFT JOIN runs latest ON latest.id = (SELECT id FROM runs WHERE session_id = sessions.id ORDER BY created_at_ms DESC, id DESC LIMIT 1) LEFT JOIN runs terminal ON terminal.id = (SELECT id FROM runs WHERE session_id = sessions.id AND status IN ('succeeded', 'failed', 'timed_out', 'cancelled', 'interrupted', 'lost') ORDER BY updated_at_ms DESC, id DESC LIMIT 1) WHERE sessions.context_kind = 'project' AND json_extract(sessions.context_json, '$.project_id') = ? AND NOT EXISTS (SELECT 1 FROM project_tool_contexts tools WHERE tools.session_id = sessions.id) ORDER BY sessions.updated_at_ms DESC, sessions.id ASC",
            )
            .bind(project_id.as_str())
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT sessions.*, latest.id AS latest_run_id, latest.status AS latest_run_status, latest.updated_at_ms AS latest_run_updated_at_ms, terminal.id AS terminal_run_id, terminal.status AS terminal_run_status, terminal.updated_at_ms AS terminal_run_updated_at_ms FROM sessions LEFT JOIN runs latest ON latest.id = (SELECT id FROM runs WHERE session_id = sessions.id ORDER BY created_at_ms DESC, id DESC LIMIT 1) LEFT JOIN runs terminal ON terminal.id = (SELECT id FROM runs WHERE session_id = sessions.id AND status IN ('succeeded', 'failed', 'timed_out', 'cancelled', 'interrupted', 'lost') ORDER BY updated_at_ms DESC, id DESC LIMIT 1) WHERE NOT EXISTS (SELECT 1 FROM project_tool_contexts tools WHERE tools.session_id = sessions.id) ORDER BY sessions.updated_at_ms DESC, sessions.id ASC",
            )
            .fetch_all(&self.pool)
            .await?
        };
        rows.iter().map(session_list_item_from_row).collect()
    }

    pub async fn put_run_summary(
        &self,
        summary: &RunSummaryRecord,
    ) -> Result<RunSummaryRecord, AgentStoreError> {
        if !summary.status.is_terminal() {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "run summary status",
                value: summary.status.as_str().into(),
            });
        }
        sqlx::query(
            "INSERT INTO run_summaries (run_id, status, changed_files, additions, deletions, files_json, diff_artifact_id, diff_unavailable, completed_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(run_id) DO UPDATE SET status = excluded.status, changed_files = excluded.changed_files, additions = excluded.additions, deletions = excluded.deletions, files_json = excluded.files_json, diff_artifact_id = excluded.diff_artifact_id, diff_unavailable = excluded.diff_unavailable, completed_at_ms = excluded.completed_at_ms",
        )
        .bind(summary.run_id.as_str())
        .bind(summary.status.as_str())
        .bind(i64::from(summary.changed_files))
        .bind(i64::from(summary.additions))
        .bind(i64::from(summary.deletions))
        .bind(serde_json::to_string(&summary.files)?)
        .bind(summary.diff_artifact_id.as_ref().map(ArtifactId::as_str))
        .bind(summary.diff_unavailable)
        .bind(summary.completed_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(summary.clone())
    }

    pub async fn get_run_summary(
        &self,
        run_id: &RunId,
    ) -> Result<Option<RunSummaryRecord>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM run_summaries WHERE run_id = ?")
            .bind(run_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(run_summary_from_row).transpose()
    }

    pub async fn list_run_summaries(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<RunSummaryRecord>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT summary.* FROM run_summaries summary INNER JOIN runs run ON run.id = summary.run_id WHERE run.session_id = ? ORDER BY run.created_at_ms ASC, run.id ASC",
        )
        .bind(session_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(run_summary_from_row).collect()
    }
}

fn session_list_item_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<WorkbenchSessionListItem, AgentStoreError> {
    Ok(WorkbenchSessionListItem {
        session: session_from_row(row)?,
        latest_run: run_activity_from_row(
            row,
            "latest_run_id",
            "latest_run_status",
            "latest_run_updated_at_ms",
        )?,
        latest_terminal_run: run_activity_from_row(
            row,
            "terminal_run_id",
            "terminal_run_status",
            "terminal_run_updated_at_ms",
        )?,
    })
}

fn run_activity_from_row(
    row: &sqlx::sqlite::SqliteRow,
    id_column: &str,
    status_column: &str,
    updated_column: &str,
) -> Result<Option<SessionRunActivity>, AgentStoreError> {
    let Some(id) = row.get::<Option<String>, _>(id_column) else {
        return Ok(None);
    };
    let status_value = row.get::<String, _>(status_column);
    let status =
        RunStatus::parse(&status_value).ok_or_else(|| AgentStoreError::InvalidPersistedValue {
            kind: "session activity run status",
            value: status_value,
        })?;
    Ok(Some(SessionRunActivity {
        id: RunId::new(id),
        status,
        updated_at_ms: row.get(updated_column),
    }))
}

fn run_summary_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<RunSummaryRecord, AgentStoreError> {
    let status_value: String = row.get("status");
    let status =
        RunStatus::parse(&status_value).ok_or_else(|| AgentStoreError::InvalidPersistedValue {
            kind: "run summary status",
            value: status_value,
        })?;
    let files: Vec<RunSummaryFile> = serde_json::from_str(row.get("files_json"))?;
    Ok(RunSummaryRecord {
        run_id: RunId::new(row.get::<String, _>("run_id")),
        status,
        changed_files: u32::try_from(row.get::<i64, _>("changed_files")).unwrap_or(u32::MAX),
        additions: u32::try_from(row.get::<i64, _>("additions")).unwrap_or(u32::MAX),
        deletions: u32::try_from(row.get::<i64, _>("deletions")).unwrap_or(u32::MAX),
        files,
        diff_artifact_id: row
            .get::<Option<String>, _>("diff_artifact_id")
            .map(ArtifactId::new),
        diff_unavailable: row.get("diff_unavailable"),
        completed_at_ms: row.get("completed_at_ms"),
    })
}
