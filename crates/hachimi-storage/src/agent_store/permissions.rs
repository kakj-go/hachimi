use hachimi_protocol::{CapabilityGrantSet, ItemId, RunId, SandboxCapabilityReport, SessionId};
use serde_json::json;
use sqlx::Row;

use super::{AgentStore, AgentStoreError, append_event_tx, get_run_tx};

impl AgentStore {
    /// Persists the exact permission expansion and Sandbox readiness used to admit a Run.
    pub async fn persist_run_security_snapshot(
        &self,
        grants: &CapabilityGrantSet,
        report: &SandboxCapabilityReport,
        created_at_ms: i64,
    ) -> Result<(), AgentStoreError> {
        let run_id = grants
            .run_id
            .as_ref()
            .ok_or(AgentStoreError::RunPreconditionFailed)?;
        let mut transaction = self.pool.begin().await?;
        let run = get_run_tx(&mut transaction, run_id)
            .await?
            .ok_or_else(|| AgentStoreError::RunNotFound(run_id.clone()))?;
        if run.session_id != grants.session_id
            || run.status.is_terminal()
            || grants
                .expires_at_ms
                .is_some_and(|expires| expires <= created_at_ms)
        {
            return Err(AgentStoreError::RunPreconditionFailed);
        }
        let grant_id = ItemId::random();
        sqlx::query(
            "INSERT INTO capability_grants (id, session_id, run_id, scope, grant_json, source, expires_at_ms, invalidated_at_ms, created_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, NULL, ?)",
        )
        .bind(grant_id.as_str())
        .bind(grants.session_id.as_str())
        .bind(run_id.as_str())
        .bind(match grants.scope {
            hachimi_protocol::PermissionGrantScope::Session => "session",
            hachimi_protocol::PermissionGrantScope::Run => "run",
        })
        .bind(serde_json::to_string(grants)?)
        .bind(&grants.source)
        .bind(grants.expires_at_ms)
        .bind(created_at_ms)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO sandbox_capability_reports (session_id, run_id, backend, readiness, report_json, created_at_ms) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(grants.session_id.as_str())
        .bind(run_id.as_str())
        .bind(&report.backend)
        .bind(match report.readiness {
            hachimi_protocol::SandboxReadiness::Unavailable => "unavailable",
            hachimi_protocol::SandboxReadiness::SetupRequired => "setup_required",
            hachimi_protocol::SandboxReadiness::Degraded => "degraded",
            hachimi_protocol::SandboxReadiness::Ready => "ready",
        })
        .bind(serde_json::to_string(report)?)
        .bind(created_at_ms)
        .execute(&mut *transaction)
        .await?;
        append_event_tx(
            &mut transaction,
            &grants.session_id,
            Some(run_id),
            "security.snapshot_persisted",
            json!({
                "permissionProfile": grants.profile,
                "sandboxBackend": report.backend,
                "sandboxReadiness": report.readiness,
                "osEnforced": report.os_enforced,
            }),
            created_at_ms,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn latest_sandbox_report(
        &self,
        run_id: &RunId,
    ) -> Result<Option<SandboxCapabilityReport>, AgentStoreError> {
        let row = sqlx::query(
            "SELECT report_json FROM sandbox_capability_reports WHERE run_id = ? ORDER BY created_at_ms DESC, id DESC LIMIT 1",
        )
        .bind(run_id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| serde_json::from_str(row.get("report_json")))
            .transpose()
            .map_err(AgentStoreError::from)
    }

    pub async fn latest_active_capability_grants(
        &self,
        run_id: &RunId,
    ) -> Result<Option<CapabilityGrantSet>, AgentStoreError> {
        let row = sqlx::query(
            "SELECT grant_json FROM capability_grants WHERE run_id = ? AND invalidated_at_ms IS NULL AND (expires_at_ms IS NULL OR expires_at_ms > ?) ORDER BY created_at_ms DESC, id DESC LIMIT 1",
        )
        .bind(run_id.as_str())
        .bind(current_time_ms())
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| serde_json::from_str(row.get("grant_json")))
            .transpose()
            .map_err(AgentStoreError::from)
    }

    /// Returns the last trusted Run-scoped grant as a recovery input. Callers must
    /// issue a fresh grant and re-check expiry; this never reactivates the stored row.
    pub async fn latest_capability_grants_snapshot(
        &self,
        run_id: &RunId,
    ) -> Result<Option<CapabilityGrantSet>, AgentStoreError> {
        let row = sqlx::query(
            "SELECT grant_json FROM capability_grants WHERE run_id = ? ORDER BY created_at_ms DESC, id DESC LIMIT 1",
        )
        .bind(run_id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| serde_json::from_str(row.get("grant_json")))
            .transpose()
            .map_err(AgentStoreError::from)
    }

    pub async fn invalidate_run_capability_grants(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        invalidated_at_ms: i64,
    ) -> Result<u64, AgentStoreError> {
        let result = sqlx::query(
            "UPDATE capability_grants SET invalidated_at_ms = ? WHERE session_id = ? AND run_id = ? AND invalidated_at_ms IS NULL",
        )
        .bind(invalidated_at_ms)
        .bind(session_id.as_str())
        .bind(run_id.as_str())
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }
}

fn current_time_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or_default()
}
