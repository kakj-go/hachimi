use super::*;

/// Metadata-only audit row. Keeping the values together prevents callers
/// from accidentally omitting Run fencing fields or persisting raw payloads.
#[derive(Debug, Clone)]
pub struct AuditMetadataRecord {
    pub principal: String,
    pub session_id: Option<SessionId>,
    pub run_id: Option<RunId>,
    pub run_generation: Option<u64>,
    pub operation: String,
    pub target_summary: String,
    pub decision: String,
    pub result_code: String,
    pub created_at_ms: i64,
}

impl AgentStore {
    pub async fn append_audit_metadata(
        &self,
        record: AuditMetadataRecord,
    ) -> Result<(), AgentStoreError> {
        sqlx::query(
            "INSERT INTO audit_events (principal, session_id, run_id, run_generation, operation, target_summary, decision, result_code, created_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(record.principal)
        .bind(record.session_id.as_ref().map(SessionId::as_str))
        .bind(record.run_id.as_ref().map(RunId::as_str))
        .bind(
            record
                .run_generation
                .map(|value| i64::try_from(value).unwrap_or(i64::MAX)),
        )
        .bind(record.operation)
        .bind(record.target_summary)
        .bind(record.decision)
        .bind(record.result_code)
        .bind(record.created_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
