use hachimi_protocol::{
    ApprovalId, ForgeChangeRecord, ForgeKind, ForgeOperationId, ForgeOperationRecord,
    ForgeOperationStatus, ForgeRepositoryIdentity, RunId, SessionId,
};
use sqlx::Row;

use super::{AgentStore, AgentStoreError};

impl AgentStore {
    pub async fn upsert_forge_repository(
        &self,
        repository: &ForgeRepositoryIdentity,
        updated_at_ms: i64,
    ) -> Result<(), AgentStoreError> {
        validate_repository(repository)?;
        sqlx::query(
            "INSERT INTO forge_repositories (remote_url_hash, forge_kind, api_base_url, owner, repository, secret_ref, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(remote_url_hash) DO UPDATE SET forge_kind = excluded.forge_kind, api_base_url = excluded.api_base_url, owner = excluded.owner, repository = excluded.repository, secret_ref = excluded.secret_ref, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(&repository.remote_url_hash)
        .bind(repository.forge_kind.as_str())
        .bind(&repository.api_base_url)
        .bind(&repository.owner)
        .bind(&repository.repository)
        .bind(&repository.secret_ref)
        .bind(updated_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn claim_forge_operation(
        &self,
        operation: &ForgeOperationRecord,
    ) -> Result<ForgeOperationRecord, AgentStoreError> {
        if operation.status != ForgeOperationStatus::Claimed
            || operation.idempotency_key.trim().is_empty()
            || operation.request_hash.len() != 64
            || operation.commit_oid.len() != 40
        {
            return Err(AgentStoreError::InvalidForgeOperation);
        }
        self.upsert_forge_repository(&operation.repository, operation.updated_at_ms)
            .await?;
        let result = sqlx::query(
            "INSERT INTO forge_operations (id, session_id, run_id, run_generation, operation_kind, remote_url_hash, source_ref, target_ref, commit_oid, expected_revision, approval_id, idempotency_key, request_hash, status, result_json, error_code, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(session_id, idempotency_key) DO NOTHING",
        )
        .bind(operation.id.as_str())
        .bind(operation.session_id.as_str())
        .bind(operation.run_id.as_ref().map(RunId::as_str))
        .bind(operation.run_generation.and_then(|value| i64::try_from(value).ok()))
        .bind(&operation.operation_kind)
        .bind(&operation.repository.remote_url_hash)
        .bind(&operation.source_ref)
        .bind(&operation.target_ref)
        .bind(&operation.commit_oid)
        .bind(&operation.expected_revision)
        .bind(operation.approval_id.as_ref().map(ApprovalId::as_str))
        .bind(&operation.idempotency_key)
        .bind(&operation.request_hash)
        .bind(operation.status.as_str())
        .bind(operation.result.as_ref().map(serde_json::to_string).transpose()?)
        .bind(&operation.error_code)
        .bind(operation.created_at_ms)
        .bind(operation.updated_at_ms)
        .execute(&self.pool)
        .await?;
        let persisted = self
            .get_forge_operation_by_key(&operation.session_id, &operation.idempotency_key)
            .await?
            .ok_or(AgentStoreError::InvalidForgeOperation)?;
        if result.rows_affected() == 0 && persisted.request_hash != operation.request_hash {
            return Err(AgentStoreError::IdempotencyConflict);
        }
        Ok(persisted)
    }

    pub async fn get_forge_operation_by_key(
        &self,
        session_id: &SessionId,
        idempotency_key: &str,
    ) -> Result<Option<ForgeOperationRecord>, AgentStoreError> {
        let row = sqlx::query(
            "SELECT o.*, r.forge_kind, r.api_base_url, r.owner, r.repository, r.secret_ref FROM forge_operations o JOIN forge_repositories r ON r.remote_url_hash = o.remote_url_hash WHERE o.session_id = ? AND o.idempotency_key = ?",
        )
        .bind(session_id.as_str())
        .bind(idempotency_key)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(operation_from_row).transpose()
    }

    pub async fn update_forge_operation(
        &self,
        operation_id: &ForgeOperationId,
        expected: ForgeOperationStatus,
        next: ForgeOperationStatus,
        result: Option<&ForgeChangeRecord>,
        error_code: Option<&str>,
        updated_at_ms: i64,
    ) -> Result<ForgeOperationRecord, AgentStoreError> {
        if !valid_transition(expected, next) {
            return Err(AgentStoreError::InvalidForgeOperation);
        }
        let changed = sqlx::query(
            "UPDATE forge_operations SET status = ?, result_json = COALESCE(?, result_json), error_code = ?, updated_at_ms = ? WHERE id = ? AND status = ?",
        )
        .bind(next.as_str())
        .bind(result.map(serde_json::to_string).transpose()?)
        .bind(error_code)
        .bind(updated_at_ms)
        .bind(operation_id.as_str())
        .bind(expected.as_str())
        .execute(&self.pool)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(AgentStoreError::InvalidForgeOperation);
        }
        let row = sqlx::query(
            "SELECT o.*, r.forge_kind, r.api_base_url, r.owner, r.repository, r.secret_ref FROM forge_operations o JOIN forge_repositories r ON r.remote_url_hash = o.remote_url_hash WHERE o.id = ?",
        )
        .bind(operation_id.as_str())
        .fetch_one(&self.pool)
        .await?;
        operation_from_row(&row)
    }

    pub async fn list_forge_operations_for_reconciliation(
        &self,
    ) -> Result<Vec<ForgeOperationRecord>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT o.*, r.forge_kind, r.api_base_url, r.owner, r.repository, r.secret_ref FROM forge_operations o JOIN forge_repositories r ON r.remote_url_hash = o.remote_url_hash WHERE o.status IN ('claimed', 'dispatched', 'indeterminate') ORDER BY o.updated_at_ms, o.id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(operation_from_row).collect()
    }
}

fn validate_repository(repository: &ForgeRepositoryIdentity) -> Result<(), AgentStoreError> {
    if repository.remote_url_hash.len() != 64
        || repository.api_base_url.len() > 2_048
        || repository.owner.trim().is_empty()
        || repository.repository.trim().is_empty()
        || repository.owner.len() > 256
        || repository.repository.len() > 256
        || repository
            .secret_ref
            .as_ref()
            .is_some_and(|value| value.len() > 512)
    {
        return Err(AgentStoreError::InvalidForgeOperation);
    }
    Ok(())
}

fn valid_transition(current: ForgeOperationStatus, next: ForgeOperationStatus) -> bool {
    matches!(
        (current, next),
        (
            ForgeOperationStatus::Claimed,
            ForgeOperationStatus::Dispatched
        ) | (ForgeOperationStatus::Claimed, ForgeOperationStatus::Failed)
            | (
                ForgeOperationStatus::Dispatched,
                ForgeOperationStatus::Confirmed
            )
            | (
                ForgeOperationStatus::Dispatched,
                ForgeOperationStatus::Failed
            )
            | (
                ForgeOperationStatus::Dispatched,
                ForgeOperationStatus::Indeterminate
            )
            | (
                ForgeOperationStatus::Indeterminate,
                ForgeOperationStatus::Confirmed
            )
            | (
                ForgeOperationStatus::Indeterminate,
                ForgeOperationStatus::Failed
            )
    )
}

fn operation_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<ForgeOperationRecord, AgentStoreError> {
    let forge_kind = row.get::<String, _>("forge_kind");
    let status = row.get::<String, _>("status");
    Ok(ForgeOperationRecord {
        id: ForgeOperationId::new(row.get::<String, _>("id")),
        session_id: SessionId::new(row.get::<String, _>("session_id")),
        run_id: row.get::<Option<String>, _>("run_id").map(RunId::new),
        run_generation: row
            .get::<Option<i64>, _>("run_generation")
            .and_then(|value| u64::try_from(value).ok()),
        operation_kind: row.get("operation_kind"),
        repository: ForgeRepositoryIdentity {
            forge_kind: ForgeKind::parse(&forge_kind).ok_or_else(|| {
                AgentStoreError::InvalidPersistedValue {
                    kind: "forge kind",
                    value: forge_kind,
                }
            })?,
            api_base_url: row.get("api_base_url"),
            owner: row.get("owner"),
            repository: row.get("repository"),
            remote_url_hash: row.get("remote_url_hash"),
            secret_ref: row.get("secret_ref"),
        },
        source_ref: row.get("source_ref"),
        target_ref: row.get("target_ref"),
        commit_oid: row.get("commit_oid"),
        expected_revision: row.get("expected_revision"),
        approval_id: row
            .get::<Option<String>, _>("approval_id")
            .map(ApprovalId::new),
        idempotency_key: row.get("idempotency_key"),
        request_hash: row.get("request_hash"),
        status: ForgeOperationStatus::parse(&status).ok_or_else(|| {
            AgentStoreError::InvalidPersistedValue {
                kind: "forge operation status",
                value: status,
            }
        })?,
        result: row
            .get::<Option<String>, _>("result_json")
            .map(|value| serde_json::from_str(&value))
            .transpose()?,
        error_code: row.get("error_code"),
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}
