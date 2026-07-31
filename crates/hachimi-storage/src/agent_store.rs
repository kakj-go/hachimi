//! Dedicated SQLite persistence for Harness Agent state.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use hachimi_protocol::{
    ApprovalId, ApprovalRequestRecord, ApprovalResolution, ApprovalStatus, ArtifactId,
    ArtifactKind, ArtifactRecord, AttachmentId, AttachmentRecord, CONTROL_PROTOCOL_VERSION,
    CapabilityDegradation, CheckoutRecord, CompactionCheckpoint, CompactionCheckpointId,
    CompactionLifecycle, CompactionReason, ItemPayload, ItemStatus, McpHeaderView,
    McpServerHealthRecord, McpServerHealthState, McpServerId, McpServerRecord, McpServerTransport,
    PlanId, ProjectRecord, ProposedPlan, ProposedPlanStatus, RunEventEnvelope, RunEventPayload,
    RunId, RunRecord, RunStatus, SessionId, SessionRecord, ToolCallId, TranscriptItem,
    TranscriptItemKind,
};
use serde_json::{Value, json};
use sqlx::{
    Row, Sqlite, SqlitePool, Transaction,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use thiserror::Error;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

mod active_events;
mod agent_tasks;
mod audit;
mod desktop_control;
#[cfg(test)]
mod desktop_control_tests;
mod extensions;
mod git_forge;
#[cfg(test)]
mod git_forge_tests;
mod lifecycle;
mod mcp_cache;
mod migration_safety;
mod permissions;
mod plugin_hooks;
mod process;
mod project;
mod provider_registry;
mod recovery;
mod review;
mod row_decode;
mod run_bundle;
mod schedule;
mod schedule_event;
pub(crate) mod side_effects;
mod transaction_helpers;
mod usage;
mod user_input;
pub(crate) mod workspace_diff;

pub use agent_tasks::AgentTaskExecutionClaim;
pub use audit::AuditMetadataRecord;
pub use desktop_control::DesktopControlActionLedgerInput;
pub use extensions::{SkillFileIndexRecord, StoredSkillRecord};
pub use plugin_hooks::{
    PluginHookEventRecord, PluginHookRuntime, PluginHookRuntimeFuture, PluginHookSubscription,
};
pub use recovery::RecoveryToolFence;
use row_decode::*;
pub use run_bundle::CreatedAgentRun;
pub use schedule::{IdempotentMutationClaim, ScheduleInvocationClaim};
pub use schedule_event::{ScheduleEventIngestClaim, ScheduleEventLaunchClaim};
use transaction_helpers::*;
pub use workspace_diff::{ManagedRunDiffFile, RunFileBaselineRecord};

#[derive(Debug, Error)]
pub enum AgentStoreError {
    #[error("agent storage I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("agent database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("agent database migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("agent data serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid persisted {kind}: {value}")]
    InvalidPersistedValue { kind: &'static str, value: String },
    #[error("run does not exist: {0}")]
    RunNotFound(RunId),
    #[error("session does not exist: {0}")]
    SessionNotFound(SessionId),
    #[error("checkout does not exist: {0}")]
    CheckoutNotFound(hachimi_protocol::CheckoutId),
    #[error("attachment does not exist: {0}")]
    AttachmentNotFound(AttachmentId),
    #[error("proposed plan does not exist: {0}")]
    ProposedPlanNotFound(PlanId),
    #[error("proposed plan is no longer available for acceptance: {0}")]
    ProposedPlanNotAcceptable(PlanId),
    #[error("accepted run does not match the proposed plan session or revision")]
    ProposedPlanRunMismatch,
    #[error("checkout {checkout_id} already has a write lease held by run {holder_run_id}")]
    CheckoutWriteLeaseHeld {
        checkout_id: hachimi_protocol::CheckoutId,
        holder_run_id: RunId,
    },
    #[error("approval does not exist: {0}")]
    ApprovalNotFound(ApprovalId),
    #[error("approval is no longer pending: {0}")]
    ApprovalNotPending(ApprovalId),
    #[error("approval resolution does not match the approved parameters or run generation")]
    StaleApprovalResolution,
    #[error("approval decision must be approved, denied, or cancelled")]
    InvalidApprovalDecision,
    #[error("user input request does not exist: {0}")]
    UserInputNotFound(hachimi_protocol::UserInputRequestId),
    #[error("user input request is no longer pending: {0}")]
    UserInputNotPending(hachimi_protocol::UserInputRequestId),
    #[error("user input resolution does not match the active run generation")]
    StaleUserInputResolution,
    #[error("user input answer is invalid: {0}")]
    InvalidUserInputAnswer(&'static str),
    #[error("active run precondition failed")]
    RunPreconditionFailed,
    #[error("schedule does not exist: {0}")]
    ScheduleNotFound(hachimi_protocol::ScheduleId),
    #[error("schedule config revision precondition failed")]
    ScheduleRevisionConflict,
    #[error("schedule event id was reused with a different fingerprint")]
    ScheduleEventConflict,
    #[error("idempotency key was already used for another resource")]
    IdempotencyConflict,
    #[error("schedule grant does not exist: {0}")]
    ScheduleGrantNotFound(hachimi_protocol::ScheduleGrantId),
    #[error("task run does not exist: {0}")]
    TaskRunNotFound(hachimi_protocol::TaskRunId),
    #[error("review does not exist: {0}")]
    ReviewNotFound(hachimi_protocol::ReviewId),
    #[error("review finding does not exist: {0}")]
    ReviewFindingNotFound(hachimi_protocol::ReviewFindingId),
    #[error("illegal task run state transition")]
    InvalidTaskRunTransition,
    #[error("side-effect idempotency key was reused with different parameters")]
    SideEffectIdempotencyConflict,
    #[error("side-effect execution does not exist: {0}")]
    SideEffectNotFound(hachimi_protocol::SideEffectExecutionId),
    #[error("illegal side-effect transition from {from:?} to {to:?}")]
    InvalidSideEffectTransition {
        from: hachimi_protocol::SideEffectExecutionStatus,
        to: hachimi_protocol::SideEffectExecutionStatus,
    },
    #[error("side-effect approval is missing, stale, or already consumed")]
    SideEffectApprovalInvalid,
    #[error("compaction checkpoint quality was not accepted")]
    CompactionQualityRejected,
    #[error("compaction checkpoint does not advance the persisted session coverage")]
    CompactionSequenceNotAdvanced,
    #[error("compaction checkpoint predecessor does not match the latest persisted checkpoint")]
    CompactionPredecessorMismatch,
    #[error("compaction checkpoint covers transcript sequence that does not exist")]
    CompactionSequenceOutOfRange,
    #[error("MCP server does not exist: {0}")]
    McpServerNotFound(McpServerId),
    #[error("plugin hook runtime failed closed: {0}")]
    PluginHook(String),
    #[error("invalid MCP server configuration: {0}")]
    InvalidMcpServerConfiguration(&'static str),
    #[error("Skill does not exist: {0}")]
    SkillNotFound(hachimi_protocol::SkillId),
    #[error("illegal run transition from {from:?} to {to:?}")]
    InvalidRunTransition { from: RunStatus, to: RunStatus },
    #[error("run recovery does not exist: {0}")]
    RunRecoveryNotFound(hachimi_protocol::RunRecoveryId),
    #[error("run recovery decision is stale or not allowed")]
    InvalidRunRecoveryDecision,
    #[error("provider compatibility profile does not exist: {0}")]
    ProviderProfileNotFound(String),
    #[error("provider endpoint does not exist: {0}")]
    ProviderEndpointNotFound(hachimi_protocol::ProviderEndpointId),
    #[error("provider endpoint config revision precondition failed")]
    ProviderRevisionConflict,
    #[error("provider account does not exist: {0}")]
    ProviderAccountNotFound(hachimi_protocol::ProviderAccountId),
    #[error("agent task does not exist: {0}")]
    AgentTaskNotFound(hachimi_protocol::AgentTaskId),
    #[error("agent task lineage, budget, depth, or concurrency limit failed")]
    AgentTaskLimitExceeded,
    #[error("illegal agent task state transition")]
    InvalidAgentTaskTransition,
    #[error("invalid or stale Forge operation")]
    InvalidForgeOperation,
    #[error("database path has no usable parent")]
    InvalidPath,
    #[error("database migration lock remained busy for 30 seconds")]
    DatabaseMigrationBusy,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    pub interrupted_runs: u64,
    pub lost_tasks: u64,
    pub expired_processes: u64,
    pub expired_approvals: u64,
    pub interrupted_user_inputs: u64,
    pub stopped_mcp_servers: u64,
    pub indeterminate_side_effects: u64,
    pub cancelled_side_effect_claims: u64,
    pub auto_resume_run_ids: Vec<RunId>,
    pub awaiting_decision_run_ids: Vec<RunId>,
}

/// Storage-private attachment metadata used by the trusted attachment host.
/// The managed path is deliberately not part of the public protocol DTO.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedAttachmentRecord {
    pub attachment: AttachmentRecord,
    pub managed_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct AgentStore {
    pool: SqlitePool,
    managed_artifacts: Arc<ManagedArtifactRoot>,
    active_events: Arc<active_events::ActiveRunEvents>,
    plugin_hooks: Arc<plugin_hooks::PluginHookRuntimeSlot>,
}

#[derive(Debug)]
struct ManagedArtifactRoot {
    path: PathBuf,
    transient: bool,
}

impl Drop for ManagedArtifactRoot {
    fn drop(&mut self) {
        let safe = self.transient
            && self
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("memory-"))
            && self
                .path
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                == Some("hachimi-agent-artifacts");
        if safe {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

impl AgentStore {
    pub async fn connect(path: impl AsRef<Path>) -> Result<Self, AgentStoreError> {
        let path = path.as_ref();
        let parent = path.parent().ok_or(AgentStoreError::InvalidPath)?;
        std::fs::create_dir_all(parent)?;
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));
        Self::connect_options(
            options,
            parent.join("agent-artifacts"),
            false,
            Some(path.to_path_buf()),
        )
        .await
    }

    pub async fn connect_in_memory() -> Result<Self, AgentStoreError> {
        let options = SqliteConnectOptions::new()
            .filename(":memory:")
            .foreign_keys(true);
        let root = std::env::temp_dir()
            .join("hachimi-agent-artifacts")
            .join(format!("memory-{}", uuid::Uuid::now_v7()));
        Self::connect_options(options, root, true, None).await
    }

    async fn connect_options(
        options: SqliteConnectOptions,
        managed_artifact_root: PathBuf,
        transient: bool,
        database_path: Option<PathBuf>,
    ) -> Result<Self, AgentStoreError> {
        // A single connection makes the in-memory and file-backed behavior identical and gives
        // session event sequence allocation deterministic serialization.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        migration_safety::run_migrations(&pool, database_path.as_deref(), &MIGRATOR).await?;
        std::fs::create_dir_all(&managed_artifact_root)?;
        Ok(Self {
            pool,
            managed_artifacts: Arc::new(ManagedArtifactRoot {
                path: managed_artifact_root,
                transient,
            }),
            active_events: Arc::new(active_events::ActiveRunEvents::new()),
            plugin_hooks: Arc::new(plugin_hooks::PluginHookRuntimeSlot::default()),
        })
    }

    #[must_use]
    pub const fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Returns the trusted, process-private root used by Host implementations.
    /// Paths below this root must never be surfaced to model or WebView callers.
    #[must_use]
    pub fn managed_artifact_root(&self) -> &Path {
        &self.managed_artifacts.path
    }

    pub async fn create_proposed_plan(
        &self,
        mut plan: ProposedPlan,
    ) -> Result<ProposedPlan, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        if let Some(existing) = get_proposed_plan_by_run_tx(&mut transaction, &plan.run_id).await? {
            transaction.commit().await?;
            return Ok(existing);
        }
        let previous_revision = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(MAX(revision), 0) FROM proposed_plans WHERE session_id = ?",
        )
        .bind(plan.session_id.as_str())
        .fetch_one(&mut *transaction)
        .await?;
        plan.revision = u32::try_from(previous_revision.saturating_add(1)).unwrap_or(u32::MAX);
        plan.status = ProposedPlanStatus::Proposed;
        plan.accepted_run_id = None;
        plan.accepted_at_ms = None;
        sqlx::query(
            "UPDATE proposed_plans SET status = 'superseded' WHERE session_id = ? AND status = 'proposed'",
        )
        .bind(plan.session_id.as_str())
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO proposed_plans (id, session_id, run_id, revision, goal, assumptions_json, steps_json, affected_resources_json, verification_json, risks_json, open_questions_json, content_markdown, status, accepted_run_id, created_at_ms, accepted_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(plan.id.as_str())
        .bind(plan.session_id.as_str())
        .bind(plan.run_id.as_str())
        .bind(i64::from(plan.revision))
        .bind(&plan.goal)
        .bind(serde_json::to_string(&plan.assumptions)?)
        .bind(serde_json::to_string(&plan.steps)?)
        .bind(serde_json::to_string(&plan.affected_resources)?)
        .bind(serde_json::to_string(&plan.verification)?)
        .bind(serde_json::to_string(&plan.risks)?)
        .bind(serde_json::to_string(&plan.open_questions)?)
        .bind(&plan.content_markdown)
        .bind(plan.status.as_str())
        .bind(Option::<&str>::None)
        .bind(plan.created_at_ms)
        .bind(Option::<i64>::None)
        .execute(&mut *transaction)
        .await?;
        append_event_tx(
            &mut transaction,
            &plan.session_id,
            Some(&plan.run_id),
            "plan.proposed",
            json!({ "planId": plan.id, "revision": plan.revision }),
            plan.created_at_ms,
        )
        .await?;
        transaction.commit().await?;
        Ok(plan)
    }

    pub async fn get_proposed_plan(
        &self,
        plan_id: &PlanId,
    ) -> Result<Option<ProposedPlan>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM proposed_plans WHERE id = ?")
            .bind(plan_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(proposed_plan_from_row).transpose()
    }

    pub async fn list_proposed_plans(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<ProposedPlan>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM proposed_plans WHERE session_id = ? ORDER BY revision ASC, id ASC",
        )
        .bind(session_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(proposed_plan_from_row).collect()
    }

    pub async fn accept_proposed_plan_idempotent(
        &self,
        principal: &str,
        idempotency_key: &str,
        plan_id: &PlanId,
        run: &RunRecord,
    ) -> Result<(ProposedPlan, RunRecord), AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        if let Some(existing_id) = sqlx::query_scalar::<_, String>(
            "SELECT resource_id FROM idempotency_records WHERE principal = ? AND method = 'plan.accept' AND idempotency_key = ?",
        )
        .bind(principal)
        .bind(idempotency_key)
        .fetch_optional(&mut *transaction)
        .await?
        {
            let existing_run = get_run_tx(&mut transaction, &RunId::new(existing_id))
                .await?
                .ok_or_else(|| AgentStoreError::RunNotFound(run.id.clone()))?;
            let accepted_plan_id = existing_run
                .configuration
                .accepted_plan_id
                .as_ref()
                .ok_or(AgentStoreError::ProposedPlanRunMismatch)?;
            let plan = get_proposed_plan_tx(&mut transaction, accepted_plan_id)
                .await?
                .ok_or_else(|| AgentStoreError::ProposedPlanNotFound(accepted_plan_id.clone()))?;
            transaction.commit().await?;
            return Ok((plan, existing_run));
        }

        let mut plan = get_proposed_plan_tx(&mut transaction, plan_id)
            .await?
            .ok_or_else(|| AgentStoreError::ProposedPlanNotFound(plan_id.clone()))?;
        if plan.status != ProposedPlanStatus::Proposed {
            return Err(AgentStoreError::ProposedPlanNotAcceptable(plan.id));
        }
        if run.session_id != plan.session_id
            || run.status != RunStatus::Queued
            || run.configuration.accepted_plan_id.as_ref() != Some(&plan.id)
            || run.configuration.accepted_plan_revision != Some(plan.revision)
        {
            return Err(AgentStoreError::ProposedPlanRunMismatch);
        }
        let configuration_json = serde_json::to_string(&run.configuration)?;
        sqlx::query(
            "INSERT INTO runs (id, session_id, status, purpose, origin_json, generation, configuration_json, requested_capabilities_json, negotiated_capabilities_json, provider_capability_probe_json, capability_degradations_json, failure_code, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(run.id.as_str())
        .bind(run.session_id.as_str())
        .bind(run.status.as_str())
        .bind(enum_to_db(&run.purpose)?)
        .bind(serde_json::to_string(&run.origin)?)
        .bind(i64::try_from(run.generation).unwrap_or(i64::MAX))
        .bind(configuration_json)
        .bind(serde_json::to_string(&run.requested_capabilities)?)
        .bind(serde_json::to_string(&run.negotiated_capabilities)?)
        .bind(serde_json::to_string(&run.provider_capability_probe)?)
        .bind(serde_json::to_string(&run.capability_degradations)?)
        .bind(&run.failure_code)
        .bind(run.created_at_ms)
        .bind(run.updated_at_ms)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO run_attachments (run_id, attachment_id) SELECT ?, attachment_id FROM run_attachments WHERE run_id = ?",
        )
        .bind(run.id.as_str())
        .bind(plan.run_id.as_str())
        .execute(&mut *transaction)
        .await?;
        let accepted_at_ms = run.created_at_ms;
        sqlx::query(
            "UPDATE proposed_plans SET status = 'accepted', accepted_run_id = ?, accepted_at_ms = ? WHERE id = ? AND status = 'proposed'",
        )
        .bind(run.id.as_str())
        .bind(accepted_at_ms)
        .bind(plan.id.as_str())
        .execute(&mut *transaction)
        .await?;
        plan.status = ProposedPlanStatus::Accepted;
        plan.accepted_run_id = Some(run.id.clone());
        plan.accepted_at_ms = Some(accepted_at_ms);
        append_event_tx(
            &mut transaction,
            &run.session_id,
            Some(&run.id),
            "run.queued",
            json!({ "status": run.status, "acceptedPlanId": plan.id, "planRevision": plan.revision }),
            run.created_at_ms,
        )
        .await?;
        append_event_tx(
            &mut transaction,
            &run.session_id,
            Some(&run.id),
            "plan.accepted",
            json!({ "planId": plan.id, "revision": plan.revision, "executionRunId": run.id }),
            accepted_at_ms,
        )
        .await?;
        sqlx::query(
            "INSERT INTO idempotency_records (principal, method, idempotency_key, resource_id, response_json, created_at_ms) VALUES (?, 'plan.accept', ?, ?, '{}', ?)",
        )
        .bind(principal)
        .bind(idempotency_key)
        .bind(run.id.as_str())
        .bind(run.created_at_ms)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok((plan, run.clone()))
    }

    pub async fn create_checkout(
        &self,
        checkout: &CheckoutRecord,
    ) -> Result<CheckoutRecord, AgentStoreError> {
        sqlx::query(
            "INSERT INTO workspace_checkouts (id, project_id, kind, path, base_revision, head_revision, status, pinned, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(checkout.id.as_str())
        .bind(checkout.project_id.as_str())
        .bind(enum_to_db(&checkout.kind)?)
        .bind(&checkout.path)
        .bind(&checkout.base_revision)
        .bind(&checkout.head_revision)
        .bind(enum_to_db(&checkout.status)?)
        .bind(checkout.pinned)
        .bind(checkout.created_at_ms)
        .bind(checkout.updated_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(checkout.clone())
    }

    pub async fn list_checkouts(
        &self,
        project_id: &hachimi_protocol::ProjectId,
    ) -> Result<Vec<CheckoutRecord>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM workspace_checkouts WHERE project_id = ? AND status != 'removed' ORDER BY created_at_ms DESC",
        )
        .bind(project_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(checkout_from_row).collect()
    }

    pub async fn get_checkout(
        &self,
        checkout_id: &hachimi_protocol::CheckoutId,
    ) -> Result<Option<CheckoutRecord>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM workspace_checkouts WHERE id = ?")
            .bind(checkout_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(checkout_from_row).transpose()
    }

    pub async fn update_checkout_lifecycle(
        &self,
        checkout_id: &hachimi_protocol::CheckoutId,
        status: hachimi_protocol::CheckoutStatus,
        pinned: bool,
    ) -> Result<CheckoutRecord, AgentStoreError> {
        let updated_at_ms = now_ms();
        let result = sqlx::query(
            "UPDATE workspace_checkouts SET status = ?, pinned = ?, updated_at_ms = ? WHERE id = ?",
        )
        .bind(enum_to_db(&status)?)
        .bind(pinned)
        .bind(updated_at_ms)
        .bind(checkout_id.as_str())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            return Err(AgentStoreError::CheckoutNotFound(checkout_id.clone()));
        }
        self.get_checkout(checkout_id)
            .await?
            .ok_or_else(|| AgentStoreError::CheckoutNotFound(checkout_id.clone()))
    }

    pub async fn checkout_has_active_runs(
        &self,
        checkout_id: &hachimi_protocol::CheckoutId,
    ) -> Result<bool, AgentStoreError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM runs INNER JOIN sessions ON sessions.id = runs.session_id WHERE sessions.context_kind = 'project' AND json_extract(sessions.context_json, '$.checkout_id') = ? AND runs.status IN ('queued', 'preparing', 'running', 'waiting_approval', 'waiting_user_input', 'cancelling')",
        )
        .bind(checkout_id.as_str())
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    pub async fn checkout_has_write_lease(
        &self,
        checkout_id: &hachimi_protocol::CheckoutId,
    ) -> Result<bool, AgentStoreError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM checkout_write_leases WHERE checkout_id = ?",
        )
        .bind(checkout_id.as_str())
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    pub async fn acquire_checkout_write_lease(
        &self,
        checkout_id: &hachimi_protocol::CheckoutId,
        run_id: &RunId,
        run_generation: u64,
    ) -> Result<(), AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        if let Some(holder) = sqlx::query_scalar::<_, String>(
            "SELECT run_id FROM checkout_write_leases WHERE checkout_id = ?",
        )
        .bind(checkout_id.as_str())
        .fetch_optional(&mut *transaction)
        .await?
        {
            if holder == run_id.as_str() {
                transaction.commit().await?;
                return Ok(());
            }
            return Err(AgentStoreError::CheckoutWriteLeaseHeld {
                checkout_id: checkout_id.clone(),
                holder_run_id: RunId::new(holder),
            });
        }
        sqlx::query(
            "INSERT INTO checkout_write_leases (checkout_id, run_id, run_generation, acquired_at_ms) VALUES (?, ?, ?, ?)",
        )
        .bind(checkout_id.as_str())
        .bind(run_id.as_str())
        .bind(i64::try_from(run_generation).unwrap_or(i64::MAX))
        .bind(now_ms())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn release_checkout_write_lease(
        &self,
        checkout_id: &hachimi_protocol::CheckoutId,
        run_id: &RunId,
        run_generation: u64,
    ) -> Result<bool, AgentStoreError> {
        let result = sqlx::query(
            "DELETE FROM checkout_write_leases WHERE checkout_id = ? AND run_id = ? AND run_generation = ?",
        )
        .bind(checkout_id.as_str())
        .bind(run_id.as_str())
        .bind(i64::try_from(run_generation).unwrap_or(i64::MAX))
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn create_session(
        &self,
        session: &SessionRecord,
    ) -> Result<SessionRecord, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO sessions (id, context_kind, context_json, entry_profile, title, archived, pinned, parent_session_id, source_run_id, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(session.id.as_str())
        .bind(session_context_kind(&session.context))
        .bind(serde_json::to_string(&session.context)?)
        .bind(enum_to_db(&session.entry_profile)?)
        .bind(&session.title)
        .bind(session.archived)
        .bind(session.pinned)
        .bind(session.parent_session_id.as_ref().map(SessionId::as_str))
        .bind(session.source_run_id.as_ref().map(RunId::as_str))
        .bind(session.created_at_ms)
        .bind(session.updated_at_ms)
        .execute(&mut *transaction)
        .await?;
        append_event_tx(
            &mut transaction,
            &session.id,
            None,
            "session.created",
            json!({ "sessionId": session.id }),
            session.created_at_ms,
        )
        .await?;
        transaction.commit().await?;
        Ok(session.clone())
    }

    pub async fn list_sessions(
        &self,
        project_id: Option<&hachimi_protocol::ProjectId>,
    ) -> Result<Vec<SessionRecord>, AgentStoreError> {
        let rows = if let Some(project_id) = project_id {
            sqlx::query(
                "SELECT * FROM sessions WHERE context_kind = 'project' AND json_extract(context_json, '$.project_id') = ? ORDER BY updated_at_ms DESC, id ASC",
            )
            .bind(project_id.as_str())
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query("SELECT * FROM sessions ORDER BY updated_at_ms DESC, id ASC")
                .fetch_all(&self.pool)
                .await?
        };
        rows.iter().map(session_from_row).collect()
    }

    pub async fn get_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<SessionRecord>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM sessions WHERE id = ?")
            .bind(session_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(session_from_row).transpose()
    }

    pub async fn create_run_idempotent(
        &self,
        principal: &str,
        idempotency_key: &str,
        run: &RunRecord,
    ) -> Result<RunRecord, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        if let Some(existing_id) = sqlx::query_scalar::<_, String>(
            "SELECT resource_id FROM idempotency_records WHERE principal = ? AND method = 'run.start' AND idempotency_key = ?",
        )
        .bind(principal)
        .bind(idempotency_key)
        .fetch_optional(&mut *transaction)
        .await?
        {
            let existing = get_run_tx(&mut transaction, &RunId::new(existing_id)).await?;
            transaction.commit().await?;
            return existing.ok_or_else(|| AgentStoreError::RunNotFound(run.id.clone()));
        }

        let configuration_json = serde_json::to_string(&run.configuration)?;
        sqlx::query(
            "INSERT INTO runs (id, session_id, status, purpose, origin_json, generation, configuration_json, requested_capabilities_json, negotiated_capabilities_json, provider_capability_probe_json, capability_degradations_json, failure_code, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(run.id.as_str())
        .bind(run.session_id.as_str())
        .bind(run.status.as_str())
        .bind(enum_to_db(&run.purpose)?)
        .bind(serde_json::to_string(&run.origin)?)
        .bind(i64::try_from(run.generation).unwrap_or(i64::MAX))
        .bind(configuration_json)
        .bind(serde_json::to_string(&run.requested_capabilities)?)
        .bind(serde_json::to_string(&run.negotiated_capabilities)?)
        .bind(serde_json::to_string(&run.provider_capability_probe)?)
        .bind(serde_json::to_string(&run.capability_degradations)?)
        .bind(&run.failure_code)
        .bind(run.created_at_ms)
        .bind(run.updated_at_ms)
        .execute(&mut *transaction)
        .await?;
        append_event_tx(
            &mut transaction,
            &run.session_id,
            Some(&run.id),
            "run.queued",
            json!({ "status": run.status }),
            run.created_at_ms,
        )
        .await?;
        sqlx::query(
            "INSERT INTO idempotency_records (principal, method, idempotency_key, resource_id, response_json, created_at_ms) VALUES (?, 'run.start', ?, ?, '{}', ?)",
        )
        .bind(principal)
        .bind(idempotency_key)
        .bind(run.id.as_str())
        .bind(run.created_at_ms)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(run.clone())
    }

    pub async fn get_run(&self, run_id: &RunId) -> Result<Option<RunRecord>, AgentStoreError> {
        let mut connection = self.pool.acquire().await?;
        get_run_connection(&mut connection, run_id).await
    }

    pub async fn list_runs(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<RunRecord>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM runs WHERE session_id = ? ORDER BY created_at_ms ASC, id ASC",
        )
        .bind(session_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(run_from_row).collect()
    }

    pub async fn transition_run(
        &self,
        run_id: &RunId,
        next: RunStatus,
        failure_code: Option<&str>,
    ) -> Result<RunRecord, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let current = get_run_tx(&mut transaction, run_id)
            .await?
            .ok_or_else(|| AgentStoreError::RunNotFound(run_id.clone()))?;
        if current.status == next {
            transaction.commit().await?;
            return Ok(current);
        }
        if !current.status.can_transition_to(next) {
            return Err(AgentStoreError::InvalidRunTransition {
                from: current.status,
                to: next,
            });
        }
        let updated_at_ms = now_ms();
        sqlx::query("UPDATE runs SET status = ?, failure_code = ?, updated_at_ms = ? WHERE id = ?")
            .bind(next.as_str())
            .bind(failure_code)
            .bind(updated_at_ms)
            .bind(run_id.as_str())
            .execute(&mut *transaction)
            .await?;
        append_event_tx(
            &mut transaction,
            &current.session_id,
            Some(run_id),
            "run.status_changed",
            json!({ "from": current.status, "to": next, "failureCode": failure_code }),
            updated_at_ms,
        )
        .await?;
        let updated = get_run_tx(&mut transaction, run_id)
            .await?
            .ok_or_else(|| AgentStoreError::RunNotFound(run_id.clone()))?;
        transaction.commit().await?;
        Ok(updated)
    }

    pub async fn append_event(
        &self,
        session_id: &SessionId,
        run_id: Option<&RunId>,
        event: &str,
        payload: Value,
    ) -> Result<RunEventEnvelope, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let envelope = append_event_tx(
            &mut transaction,
            session_id,
            run_id,
            event,
            payload,
            now_ms(),
        )
        .await?;
        transaction.commit().await?;
        Ok(envelope)
    }

    pub async fn append_typed_event(
        &self,
        session_id: &SessionId,
        run_id: Option<&RunId>,
        event: &str,
        typed_payload: RunEventPayload,
        payload: Value,
    ) -> Result<RunEventEnvelope, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let envelope = append_event_typed_tx(
            &mut transaction,
            session_id,
            run_id,
            event,
            Some(typed_payload),
            payload,
            now_ms(),
        )
        .await?;
        transaction.commit().await?;
        Ok(envelope)
    }

    pub async fn list_events(
        &self,
        session_id: &SessionId,
        after_sequence: u64,
    ) -> Result<Vec<RunEventEnvelope>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT sequence, run_id, payload_json, created_at_ms FROM run_events WHERE session_id = ? AND sequence > ? ORDER BY sequence ASC",
        )
        .bind(session_id.as_str())
        .bind(i64::try_from(after_sequence).unwrap_or(i64::MAX))
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(RunEventEnvelope {
                    protocol_version: CONTROL_PROTOCOL_VERSION,
                    sequence: u64::try_from(row.get::<i64, _>("sequence")).unwrap_or_default(),
                    session_id: session_id.clone(),
                    run_id: row.get::<Option<String>, _>("run_id").map(RunId::new),
                    payload: serde_json::from_str(row.get("payload_json"))?,
                    created_at_ms: row.get("created_at_ms"),
                })
            })
            .collect()
    }

    pub async fn append_transcript_item(
        &self,
        mut item: TranscriptItem,
    ) -> Result<TranscriptItem, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        item.sequence =
            next_sequence_tx(&mut transaction, &item.session_id, item.created_at_ms).await?;
        sqlx::query(
            "INSERT INTO transcript_items (id, session_id, run_id, sequence, kind, status, payload_json, relations_json, created_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(item.id.as_str())
        .bind(item.session_id.as_str())
        .bind(item.run_id.as_ref().map(RunId::as_str))
        .bind(i64::try_from(item.sequence).unwrap_or(i64::MAX))
        .bind(transcript_kind_db(item.kind))
        .bind(item.status.as_str())
        .bind(serde_json::to_string(&item.payload)?)
        .bind(serde_json::to_string(&item.relations)?)
        .bind(item.created_at_ms)
        .execute(&mut *transaction)
        .await?;
        if item.status == ItemStatus::InProgress {
            append_event_typed_tx(
                &mut transaction,
                &item.session_id,
                item.run_id.as_ref(),
                "item.started",
                Some(RunEventPayload::ItemStarted {
                    item_id: item.id.clone(),
                    kind: item.kind,
                }),
                json!({ "itemId": item.id, "kind": item.kind }),
                item.created_at_ms,
            )
            .await?;
        }
        transaction.commit().await?;
        Ok(item)
    }

    pub async fn complete_transcript_item(
        &self,
        item_id: &hachimi_protocol::ItemId,
        status: ItemStatus,
        payload: ItemPayload,
    ) -> Result<TranscriptItem, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query("SELECT * FROM transcript_items WHERE id = ?")
            .bind(item_id.as_str())
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| AgentStoreError::InvalidPersistedValue {
                kind: "transcript item",
                value: item_id.to_string(),
            })?;
        let session_id = SessionId::new(row.get::<String, _>("session_id"));
        let run_id = row.get::<Option<String>, _>("run_id").map(RunId::new);
        let current = transcript_item_from_row(&row, &session_id)?;
        if current.status != ItemStatus::InProgress {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "transcript item status",
                value: format!("{:?}", current.status),
            });
        }
        sqlx::query(
            "UPDATE transcript_items SET status = ?, payload_json = ? WHERE id = ? AND status = 'in_progress'",
        )
        .bind(status.as_str())
        .bind(serde_json::to_string(&payload)?)
        .bind(item_id.as_str())
        .execute(&mut *transaction)
        .await?;
        append_event_typed_tx(
            &mut transaction,
            &session_id,
            run_id.as_ref(),
            "item.completed",
            Some(RunEventPayload::ItemCompleted {
                item_id: item_id.clone(),
                status,
                payload: Box::new(payload),
            }),
            json!({ "itemId": item_id, "status": status }),
            now_ms(),
        )
        .await?;
        let row = sqlx::query("SELECT * FROM transcript_items WHERE id = ?")
            .bind(item_id.as_str())
            .fetch_one(&mut *transaction)
            .await?;
        let item = transcript_item_from_row(&row, &session_id)?;
        transaction.commit().await?;
        self.active_events.complete_item(&session_id, item_id);
        Ok(item)
    }

    pub async fn list_transcript(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<TranscriptItem>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT id, run_id, sequence, kind, status, payload_json, relations_json, created_at_ms FROM transcript_items WHERE session_id = ? ORDER BY sequence ASC",
        )
        .bind(session_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| transcript_item_from_row(row, session_id))
            .collect()
    }

    pub async fn create_compaction_checkpoint(
        &self,
        checkpoint: &CompactionCheckpoint,
    ) -> Result<CompactionCheckpoint, AgentStoreError> {
        if !checkpoint.quality.accepted {
            return Err(AgentStoreError::CompactionQualityRejected);
        }
        let mut transaction = self.pool.begin().await?;
        let latest =
            latest_compaction_checkpoint_tx(&mut transaction, &checkpoint.session_id).await?;
        match latest.as_ref() {
            Some(previous) => {
                if checkpoint.previous_checkpoint_id.as_ref() != Some(&previous.id) {
                    return Err(AgentStoreError::CompactionPredecessorMismatch);
                }
                if checkpoint.covered_through_sequence <= previous.covered_through_sequence {
                    return Err(AgentStoreError::CompactionSequenceNotAdvanced);
                }
            }
            None if checkpoint.previous_checkpoint_id.is_some() => {
                return Err(AgentStoreError::CompactionPredecessorMismatch);
            }
            None => {}
        }
        let transcript_max = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT MAX(sequence) FROM transcript_items WHERE session_id = ?",
        )
        .bind(checkpoint.session_id.as_str())
        .fetch_one(&mut *transaction)
        .await?
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or_default();
        if checkpoint.covered_through_sequence == 0
            || checkpoint.covered_through_sequence > transcript_max
        {
            return Err(AgentStoreError::CompactionSequenceOutOfRange);
        }
        sqlx::query(
            "INSERT INTO compaction_checkpoints (id, session_id, covered_through_sequence, summary_json, quality_json, created_at_ms, run_id, previous_checkpoint_id, reason, trigger, phase, implementation, token_snapshot_json, trimmed_history_groups, summary_source, provider_endpoint_id, provider_account_id, capability_revision, fallback_reason) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(checkpoint.id.as_str())
        .bind(checkpoint.session_id.as_str())
        .bind(i64::try_from(checkpoint.covered_through_sequence).unwrap_or(i64::MAX))
        .bind(serde_json::to_string(&checkpoint.summary)?)
        .bind(serde_json::to_string(&checkpoint.quality)?)
        .bind(checkpoint.created_at_ms)
        .bind(checkpoint.run_id.as_ref().map(RunId::as_str))
        .bind(
            checkpoint
                .previous_checkpoint_id
                .as_ref()
                .map(CompactionCheckpointId::as_str),
        )
        .bind(enum_to_db(&checkpoint.reason)?)
        .bind(enum_to_db(&checkpoint.lifecycle.trigger)?)
        .bind(enum_to_db(&checkpoint.lifecycle.phase)?)
        .bind(enum_to_db(&checkpoint.lifecycle.implementation)?)
        .bind(
            checkpoint
                .lifecycle
                .token_snapshot
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?,
        )
        .bind(i64::from(checkpoint.lifecycle.trimmed_history_groups))
        .bind(enum_to_db(&checkpoint.lifecycle.summary_source)?)
        .bind(
            checkpoint
                .lifecycle
                .provider_endpoint_id
                .as_ref()
                .map(hachimi_protocol::ProviderEndpointId::as_str),
        )
        .bind(
            checkpoint
                .lifecycle
                .provider_account_id
                .as_ref()
                .map(hachimi_protocol::ProviderAccountId::as_str),
        )
        .bind(&checkpoint.lifecycle.capability_revision)
        .bind(&checkpoint.lifecycle.fallback_reason)
        .execute(&mut *transaction)
        .await?;
        append_event_tx(
            &mut transaction,
            &checkpoint.session_id,
            checkpoint.run_id.as_ref(),
            "context.compaction_checkpoint_created",
            json!({
                "checkpointId": checkpoint.id,
                "coveredThroughSequence": checkpoint.covered_through_sequence,
                "reason": checkpoint.reason,
                "sourceItems": checkpoint.quality.source_items,
                "sourceChars": checkpoint.quality.source_chars,
                "summaryChars": checkpoint.quality.summary_chars,
                "recentTailItems": checkpoint.quality.recent_tail_items,
                "preservedIdentifierCount": checkpoint.quality.preserved_identifier_count,
            }),
            checkpoint.created_at_ms,
        )
        .await?;
        transaction.commit().await?;
        Ok(checkpoint.clone())
    }

    pub async fn latest_compaction_checkpoint(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<CompactionCheckpoint>, AgentStoreError> {
        let row = sqlx::query(
            "SELECT * FROM compaction_checkpoints WHERE session_id = ? ORDER BY covered_through_sequence DESC LIMIT 1",
        )
        .bind(session_id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(compaction_checkpoint_from_row).transpose()
    }

    pub async fn list_compaction_checkpoints(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<CompactionCheckpoint>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM compaction_checkpoints WHERE session_id = ? ORDER BY covered_through_sequence ASC",
        )
        .bind(session_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(compaction_checkpoint_from_row).collect()
    }

    pub async fn upsert_mcp_server(
        &self,
        server: &McpServerRecord,
    ) -> Result<McpServerRecord, AgentStoreError> {
        validate_mcp_server(server)?;
        let (transport_kind, command, args, cwd, url) = match &server.transport {
            McpServerTransport::Stdio { command, args, cwd } => (
                "stdio",
                command.as_str(),
                args.as_slice(),
                cwd.as_ref(),
                None,
            ),
            McpServerTransport::StreamableHttp { url } => {
                ("streamable_http", "", &[][..], None, Some(url.as_str()))
            }
        };
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO mcp_servers (id, display_name, enabled, command, args_json, cwd, read_only_tools_json, startup_timeout_ms, request_timeout_ms, max_message_bytes, created_at_ms, updated_at_ms, transport_kind, url) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name, enabled = excluded.enabled, command = excluded.command, args_json = excluded.args_json, cwd = excluded.cwd, read_only_tools_json = excluded.read_only_tools_json, startup_timeout_ms = excluded.startup_timeout_ms, request_timeout_ms = excluded.request_timeout_ms, max_message_bytes = excluded.max_message_bytes, transport_kind = excluded.transport_kind, url = excluded.url, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(server.id.as_str())
        .bind(&server.display_name)
        .bind(server.enabled)
        .bind(command)
        .bind(serde_json::to_string(args)?)
        .bind(cwd)
        .bind(serde_json::to_string(&server.read_only_tools)?)
        .bind(i64::try_from(server.startup_timeout_ms).unwrap_or(i64::MAX))
        .bind(i64::try_from(server.request_timeout_ms).unwrap_or(i64::MAX))
        .bind(i64::try_from(server.max_message_bytes).unwrap_or(i64::MAX))
        .bind(server.created_at_ms)
        .bind(server.updated_at_ms)
        .bind(transport_kind)
        .bind(url)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("DELETE FROM mcp_headers WHERE server_id = ?")
            .bind(server.id.as_str())
            .execute(&mut *transaction)
            .await?;
        for header in &server.headers {
            sqlx::query(
                "INSERT INTO mcp_headers (server_id, name, value, secret, credential_reference, configured) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(server.id.as_str())
            .bind(&header.name)
            .bind(&header.value)
            .bind(header.secret)
            .bind(&header.credential_reference)
            .bind(header.configured)
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query(
            "INSERT INTO mcp_server_health (server_id, state, server_name, server_version, protocol_version, tool_count, error_code, checked_at_ms) VALUES (?, ?, NULL, NULL, NULL, 0, NULL, ?) ON CONFLICT(server_id) DO UPDATE SET state = excluded.state, server_name = NULL, server_version = NULL, protocol_version = NULL, tool_count = 0, error_code = NULL, checked_at_ms = excluded.checked_at_ms",
        )
        .bind(server.id.as_str())
        .bind(if server.enabled { "stopped" } else { "disabled" })
        .bind(server.updated_at_ms)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.get_mcp_server(&server.id)
            .await?
            .ok_or_else(|| AgentStoreError::McpServerNotFound(server.id.clone()))
    }

    pub async fn get_mcp_server(
        &self,
        server_id: &McpServerId,
    ) -> Result<Option<McpServerRecord>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM mcp_servers WHERE id = ?")
            .bind(server_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let headers = self.load_mcp_headers(server_id).await?;
        Ok(Some(mcp_server_from_row(&row, headers)?))
    }

    pub async fn list_mcp_servers(&self) -> Result<Vec<McpServerRecord>, AgentStoreError> {
        let rows = sqlx::query("SELECT * FROM mcp_servers ORDER BY display_name ASC, id ASC")
            .fetch_all(&self.pool)
            .await?;
        let mut headers = self.load_all_mcp_headers().await?;
        rows.iter()
            .map(|row| {
                let id = McpServerId::new(row.get::<String, _>("id"));
                mcp_server_from_row(row, headers.remove(&id).unwrap_or_default())
            })
            .collect()
    }

    async fn load_mcp_headers(
        &self,
        server_id: &McpServerId,
    ) -> Result<Vec<McpHeaderView>, AgentStoreError> {
        let rows = sqlx::query("SELECT * FROM mcp_headers WHERE server_id = ? ORDER BY name")
            .bind(server_id.as_str())
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(mcp_header_from_row).collect()
    }

    async fn load_all_mcp_headers(
        &self,
    ) -> Result<BTreeMap<McpServerId, Vec<McpHeaderView>>, AgentStoreError> {
        let rows = sqlx::query("SELECT * FROM mcp_headers ORDER BY server_id, name")
            .fetch_all(&self.pool)
            .await?;
        let mut headers = BTreeMap::new();
        for row in &rows {
            let server_id = McpServerId::new(row.get::<String, _>("server_id"));
            headers
                .entry(server_id)
                .or_insert_with(Vec::new)
                .push(mcp_header_from_row(row)?);
        }
        Ok(headers)
    }

    pub async fn remove_mcp_server(
        &self,
        server_id: &McpServerId,
    ) -> Result<bool, AgentStoreError> {
        let result = sqlx::query("DELETE FROM mcp_servers WHERE id = ?")
            .bind(server_id.as_str())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn set_mcp_server_health(
        &self,
        health: &McpServerHealthRecord,
    ) -> Result<McpServerHealthRecord, AgentStoreError> {
        let result = sqlx::query(
            "UPDATE mcp_server_health SET state = ?, server_name = ?, server_version = ?, protocol_version = ?, tool_count = ?, error_code = ?, checked_at_ms = ? WHERE server_id = ?",
        )
        .bind(health.state.as_str())
        .bind(&health.server_name)
        .bind(&health.server_version)
        .bind(&health.protocol_version)
        .bind(i64::from(health.tool_count))
        .bind(&health.error_code)
        .bind(health.checked_at_ms)
        .bind(health.server_id.as_str())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            return Err(AgentStoreError::McpServerNotFound(health.server_id.clone()));
        }
        Ok(health.clone())
    }

    pub async fn get_mcp_server_health(
        &self,
        server_id: &McpServerId,
    ) -> Result<Option<McpServerHealthRecord>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM mcp_server_health WHERE server_id = ?")
            .bind(server_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(mcp_server_health_from_row).transpose()
    }

    pub async fn list_mcp_server_health(
        &self,
    ) -> Result<Vec<McpServerHealthRecord>, AgentStoreError> {
        let rows = sqlx::query("SELECT * FROM mcp_server_health ORDER BY server_id ASC")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(mcp_server_health_from_row).collect()
    }

    pub async fn create_artifact(
        &self,
        artifact: &ArtifactRecord,
    ) -> Result<ArtifactRecord, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO artifacts (id, run_id, kind, display_name, content_hash, managed_path, metadata_json, created_at_ms) VALUES (?, ?, ?, ?, ?, NULL, ?, ?)",
        )
        .bind(artifact.id.as_str())
        .bind(artifact.run_id.as_ref().map(RunId::as_str))
        .bind(artifact.kind.as_str())
        .bind(&artifact.display_name)
        .bind(&artifact.content_hash)
        .bind(serde_json::to_string(&artifact.metadata)?)
        .bind(artifact.created_at_ms)
        .execute(&mut *transaction)
        .await?;
        if let Some(run_id) = &artifact.run_id {
            let run = get_run_tx(&mut transaction, run_id)
                .await?
                .ok_or_else(|| AgentStoreError::RunNotFound(run_id.clone()))?;
            append_event_tx(
                &mut transaction,
                &run.session_id,
                Some(run_id),
                "artifact.created",
                json!({
                    "artifactId": artifact.id,
                    "kind": artifact.kind,
                    "displayName": artifact.display_name,
                }),
                artifact.created_at_ms,
            )
            .await?;
        }
        transaction.commit().await?;
        Ok(artifact.clone())
    }

    pub async fn list_session_artifacts(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<ArtifactRecord>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT artifacts.* FROM artifacts INNER JOIN runs ON runs.id = artifacts.run_id WHERE runs.session_id = ? AND artifacts.kind != 'file_baseline' ORDER BY artifacts.created_at_ms ASC, artifacts.id ASC",
        )
        .bind(session_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(artifact_from_row).collect()
    }

    pub async fn create_approval(
        &self,
        approval: &ApprovalRequestRecord,
    ) -> Result<ApprovalRequestRecord, AgentStoreError> {
        if approval.status != ApprovalStatus::Pending {
            return Err(AgentStoreError::ApprovalNotPending(approval.id.clone()));
        }
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO approval_requests (id, session_id, run_id, tool_call_id, run_generation, status, action, resource, parameter_hash, risk_summary, target_host, required_scopes_json, grant_scope, uses_remaining, requester_principal, resolved_by, expires_at_ms, created_at_ms, resolved_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(approval.id.as_str())
        .bind(approval.session_id.as_str())
        .bind(approval.run_id.as_str())
        .bind(approval.tool_call_id.as_str())
        .bind(i64::try_from(approval.run_generation).unwrap_or(i64::MAX))
        .bind(approval.status.as_str())
        .bind(&approval.action)
        .bind(&approval.resource)
        .bind(&approval.parameter_hash)
        .bind(&approval.risk_summary)
        .bind(&approval.target_host)
        .bind(serde_json::to_string(&approval.required_scopes)?)
        .bind(enum_to_db(&approval.grant_scope)?)
        .bind(i64::from(approval.uses_remaining))
        .bind(&approval.requester_principal)
        .bind(&approval.resolved_by)
        .bind(approval.expires_at_ms)
        .bind(approval.created_at_ms)
        .bind(approval.resolved_at_ms)
        .execute(&mut *transaction)
        .await?;
        append_event_tx(
            &mut transaction,
            &approval.session_id,
            Some(&approval.run_id),
            "approval.requested",
            json!({
                "approvalId": approval.id,
                "toolCallId": approval.tool_call_id,
                "action": approval.action,
                "resource": approval.resource,
                "riskSummary": approval.risk_summary,
                "expiresAtMs": approval.expires_at_ms
            }),
            approval.created_at_ms,
        )
        .await?;
        transaction.commit().await?;
        Ok(approval.clone())
    }

    pub async fn get_approval(
        &self,
        approval_id: &ApprovalId,
    ) -> Result<Option<ApprovalRequestRecord>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM approval_requests WHERE id = ?")
            .bind(approval_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(approval_from_row).transpose()
    }

    pub async fn list_pending_approvals(
        &self,
    ) -> Result<Vec<ApprovalRequestRecord>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM approval_requests WHERE status = 'pending' ORDER BY created_at_ms ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(approval_from_row).collect()
    }

    pub async fn resolve_approval(
        &self,
        resolution: &ApprovalResolution,
    ) -> Result<ApprovalRequestRecord, AgentStoreError> {
        if !matches!(
            resolution.decision,
            ApprovalStatus::Approved | ApprovalStatus::Denied | ApprovalStatus::Cancelled
        ) {
            return Err(AgentStoreError::InvalidApprovalDecision);
        }
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query("SELECT * FROM approval_requests WHERE id = ?")
            .bind(resolution.approval_id.as_str())
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| AgentStoreError::ApprovalNotFound(resolution.approval_id.clone()))?;
        let mut approval = approval_from_row(&row)?;
        if approval.status != ApprovalStatus::Pending {
            return Err(AgentStoreError::ApprovalNotPending(approval.id));
        }
        if approval.parameter_hash != resolution.parameter_hash
            || approval.run_generation != resolution.run_generation
        {
            return Err(AgentStoreError::StaleApprovalResolution);
        }
        let run = get_run_tx(&mut transaction, &approval.run_id)
            .await?
            .ok_or_else(|| AgentStoreError::RunNotFound(approval.run_id.clone()))?;
        if run.generation != approval.run_generation || run.status != RunStatus::WaitingApproval {
            return Err(AgentStoreError::StaleApprovalResolution);
        }
        let expired = approval
            .expires_at_ms
            .is_some_and(|expires_at| resolution.resolved_at_ms >= expires_at);
        let status = if expired {
            ApprovalStatus::Expired
        } else {
            resolution.decision
        };
        let updated = sqlx::query(
            "UPDATE approval_requests SET status = ?, resolved_by = ?, resolved_at_ms = ? WHERE id = ? AND status = 'pending'",
        )
        .bind(status.as_str())
        .bind(&resolution.resolved_by)
        .bind(resolution.resolved_at_ms)
        .bind(resolution.approval_id.as_str())
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(AgentStoreError::ApprovalNotPending(approval.id));
        }
        append_event_tx(
            &mut transaction,
            &approval.session_id,
            Some(&approval.run_id),
            if status == ApprovalStatus::Expired {
                "approval.expired"
            } else {
                "approval.resolved"
            },
            json!({ "approvalId": approval.id, "status": status }),
            resolution.resolved_at_ms,
        )
        .await?;
        approval.status = status;
        approval.resolved_by = Some(resolution.resolved_by.clone());
        approval.resolved_at_ms = Some(resolution.resolved_at_ms);
        transaction.commit().await?;
        Ok(approval)
    }

    pub async fn cancel_run_approvals(
        &self,
        run_id: &RunId,
        resolved_at_ms: i64,
    ) -> Result<u64, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let pending = sqlx::query(
            "SELECT id, session_id FROM approval_requests WHERE run_id = ? AND status = 'pending'",
        )
        .bind(run_id.as_str())
        .fetch_all(&mut *transaction)
        .await?;
        for row in &pending {
            let approval_id = ApprovalId::new(row.get::<String, _>("id"));
            let session_id = SessionId::new(row.get::<String, _>("session_id"));
            sqlx::query(
                "UPDATE approval_requests SET status = 'cancelled', resolved_at_ms = ? WHERE id = ? AND status = 'pending'",
            )
            .bind(resolved_at_ms)
            .bind(approval_id.as_str())
            .execute(&mut *transaction)
            .await?;
            append_event_tx(
                &mut transaction,
                &session_id,
                Some(run_id),
                "approval.resolved",
                json!({ "approvalId": approval_id, "status": ApprovalStatus::Cancelled }),
                resolved_at_ms,
            )
            .await?;
        }
        transaction.commit().await?;
        Ok(u64::try_from(pending.len()).unwrap_or(u64::MAX))
    }

    pub async fn recover_interrupted(&self) -> Result<RecoveryReport, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let active_runs = sqlx::query(
            "SELECT id, session_id, generation, status AS effective_status FROM runs WHERE status IN ('preparing', 'running', 'waiting_approval', 'waiting_user_input', 'cancelling')",
        )
        .fetch_all(&mut *transaction)
        .await?;
        let recovered_at_ms = now_ms();
        sqlx::query("DELETE FROM checkout_write_leases")
            .execute(&mut *transaction)
            .await?;
        for row in &active_runs {
            let run_id = RunId::new(row.get::<String, _>("id"));
            let session_id = SessionId::new(row.get::<String, _>("session_id"));
            let previous = RunStatus::parse(row.get("effective_status")).ok_or_else(|| {
                AgentStoreError::InvalidPersistedValue {
                    kind: "run status",
                    value: row.get("effective_status"),
                }
            })?;
            sqlx::query("UPDATE runs SET status = 'recovering', failure_code = 'executor_lost', updated_at_ms = ? WHERE id = ?")
                .bind(recovered_at_ms)
                .bind(run_id.as_str())
                .execute(&mut *transaction)
                .await?;
            append_event_tx(
                &mut transaction,
                &session_id,
                Some(&run_id),
                "run.status_changed",
                json!({ "from": previous, "to": RunStatus::Recovering, "failureCode": "executor_lost" }),
                recovered_at_ms,
            )
            .await?;
        }

        let lost_tasks = sqlx::query(
            "UPDATE task_runs SET status = 'lost', error_summary = 'executor lost during restart', updated_at_ms = ? WHERE status = 'running'",
        )
        .bind(recovered_at_ms)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        let expired_processes = sqlx::query(
            "UPDATE process_sessions SET status = 'expired', reconnect_expires_at_ms = NULL, updated_at_ms = ? WHERE status IN ('starting', 'running')",
        )
        .bind(recovered_at_ms)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        let expired_approvals = sqlx::query(
            "UPDATE approval_requests SET status = 'expired', resolved_at_ms = ? WHERE status = 'pending'",
        )
        .bind(recovered_at_ms)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        let interrupted_user_inputs = sqlx::query(
            "UPDATE user_input_requests SET status = 'interrupted', resolved_at_ms = ?, resolved_by = 'system:restart' WHERE status = 'pending'",
        )
        .bind(recovered_at_ms)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        let in_progress_items = sqlx::query(
            "SELECT id, session_id, run_id, payload_json FROM transcript_items WHERE status = 'in_progress' ORDER BY sequence ASC",
        )
        .fetch_all(&mut *transaction)
        .await?;
        for row in &in_progress_items {
            let item_id = hachimi_protocol::ItemId::new(row.get::<String, _>("id"));
            let session_id = SessionId::new(row.get::<String, _>("session_id"));
            let run_id = row.get::<Option<String>, _>("run_id").map(RunId::new);
            let payload: ItemPayload = serde_json::from_str(row.get("payload_json"))?;
            sqlx::query("UPDATE transcript_items SET status = 'interrupted' WHERE id = ?")
                .bind(item_id.as_str())
                .execute(&mut *transaction)
                .await?;
            append_event_typed_tx(
                &mut transaction,
                &session_id,
                run_id.as_ref(),
                "item.completed",
                Some(RunEventPayload::ItemCompleted {
                    item_id,
                    status: ItemStatus::Interrupted,
                    payload: Box::new(payload),
                }),
                json!({ "status": ItemStatus::Interrupted }),
                recovered_at_ms,
            )
            .await?;
        }
        sqlx::query("UPDATE run_steers SET status = 'interrupted' WHERE status = 'pending'")
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "UPDATE capability_grants SET invalidated_at_ms = ? WHERE invalidated_at_ms IS NULL",
        )
        .bind(recovered_at_ms)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("DELETE FROM browser_site_permissions")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM browser_network_rules")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE browser_permission_requests SET status = 'expired', updated_at_ms = ? WHERE status = 'pending'")
            .bind(recovered_at_ms)
            .execute(&mut *transaction)
            .await?;
        let stopped_mcp_servers = sqlx::query(
            "UPDATE mcp_server_health SET state = 'stopped', server_name = NULL, server_version = NULL, protocol_version = NULL, tool_count = 0, error_code = 'host_restarted', checked_at_ms = ? WHERE state IN ('starting', 'ready')",
        )
        .bind(recovered_at_ms)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        let indeterminate_side_effects = sqlx::query(
            "UPDATE side_effect_executions SET status = 'indeterminate', result_code = 'host_result_unknown_after_restart', updated_at_ms = ? WHERE status = 'dispatched'",
        )
        .bind(recovered_at_ms)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        let cancelled_side_effect_claims = sqlx::query(
            "UPDATE side_effect_executions SET status = 'cancelled', result_code = 'cancelled_before_dispatch_on_restart', updated_at_ms = ? WHERE status = 'claimed'",
        )
        .bind(recovered_at_ms)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        let mut auto_resume_run_ids = Vec::new();
        let mut awaiting_decision_run_ids = Vec::new();
        for row in &active_runs {
            let run_id = RunId::new(row.get::<String, _>("id"));
            let session_id = SessionId::new(row.get::<String, _>("session_id"));
            let generation = u64::try_from(row.get::<i64, _>("generation")).map_err(|_| {
                AgentStoreError::InvalidPersistedValue {
                    kind: "run generation",
                    value: row.get::<i64, _>("generation").to_string(),
                }
            })?;
            let previous = RunStatus::parse(row.get("effective_status")).ok_or_else(|| {
                AgentStoreError::InvalidPersistedValue {
                    kind: "run status",
                    value: row.get("effective_status"),
                }
            })?;
            let latest_checkpoint = sqlx::query(
                "SELECT id, side_effect_execution_id, recovery_policy FROM run_step_checkpoints WHERE run_id = ? AND run_generation = ? ORDER BY step_index DESC, created_at_ms DESC, rowid DESC LIMIT 1",
            )
            .bind(run_id.as_str())
            .bind(i64::try_from(generation).unwrap_or(i64::MAX))
            .fetch_optional(&mut *transaction)
            .await?;
            let latest_checkpoint_id = latest_checkpoint
                .as_ref()
                .map(|row| row.get::<String, _>("id"));
            let checkpoint_side_effect_id = latest_checkpoint
                .as_ref()
                .and_then(|row| row.get::<Option<String>, _>("side_effect_execution_id"));
            let indeterminate_side_effect_id = sqlx::query_scalar::<_, String>(
                "SELECT id FROM side_effect_executions WHERE run_id = ? AND run_generation = ? AND status = 'indeterminate' ORDER BY updated_at_ms DESC LIMIT 1",
            )
            .bind(run_id.as_str())
            .bind(i64::try_from(generation).unwrap_or(i64::MAX))
            .fetch_optional(&mut *transaction)
            .await?;
            let checkpoint_policy = latest_checkpoint
                .as_ref()
                .map(|row| row.get::<String, _>("recovery_policy"));
            let recovery_side_effect_id = indeterminate_side_effect_id
                .clone()
                .or(checkpoint_side_effect_id);
            let recovery_side_effect = if let Some(id) = recovery_side_effect_id.as_deref() {
                sqlx::query("SELECT status, result_code FROM side_effect_executions WHERE id = ?")
                    .bind(id)
                    .fetch_optional(&mut *transaction)
                    .await?
            } else {
                None
            };
            let side_effect_proven_safe = recovery_side_effect.as_ref().is_none_or(|effect| {
                let status = effect.get::<String, _>("status");
                matches!(status.as_str(), "succeeded" | "failed")
                    || status == "cancelled"
                        && effect.get::<Option<String>, _>("result_code").as_deref()
                            == Some("cancelled_before_dispatch_on_restart")
            });
            let eligible = indeterminate_side_effect_id.is_none()
                && side_effect_proven_safe
                && !matches!(
                    previous,
                    RunStatus::WaitingApproval
                        | RunStatus::WaitingUserInput
                        | RunStatus::Cancelling
                )
                && checkpoint_policy.as_deref().is_some_and(|policy| {
                    matches!(policy, "read_only_replayable" | "idempotent_with_receipt")
                });
            let recovery_id = hachimi_protocol::RunRecoveryId::random();
            let state = if eligible {
                "eligible_auto"
            } else {
                "awaiting_user"
            };
            let reason_code = if eligible {
                "safe_checkpoint_auto_resume_eligible"
            } else if indeterminate_side_effect_id.is_some() {
                "side_effect_result_indeterminate"
            } else if latest_checkpoint_id.is_none() {
                "durable_checkpoint_missing"
            } else if previous == RunStatus::WaitingApproval {
                "approval_expired_on_restart"
            } else if previous == RunStatus::WaitingUserInput {
                "user_input_interrupted_on_restart"
            } else if previous == RunStatus::Cancelling {
                "cancellation_interrupted_on_restart"
            } else {
                "checkpoint_not_replayable"
            };
            sqlx::query(
                "INSERT INTO run_recoveries (id, session_id, run_id, previous_status, interrupted_generation, resume_generation, state, reason_code, checkpoint_id, side_effect_execution_id, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(recovery_id.as_str())
            .bind(session_id.as_str())
            .bind(run_id.as_str())
            .bind(previous.as_str())
            .bind(i64::try_from(generation).unwrap_or(i64::MAX))
            .bind(i64::try_from(generation.saturating_add(1)).unwrap_or(i64::MAX))
            .bind(state)
            .bind(reason_code)
            .bind(latest_checkpoint_id)
            .bind(recovery_side_effect_id)
            .bind(recovered_at_ms)
            .bind(recovered_at_ms)
            .execute(&mut *transaction)
            .await?;
            if !eligible {
                sqlx::query("UPDATE runs SET status = 'waiting_recovery_decision', updated_at_ms = ? WHERE id = ? AND generation = ? AND status = 'recovering'")
                    .bind(recovered_at_ms)
                    .bind(run_id.as_str())
                    .bind(i64::try_from(generation).unwrap_or(i64::MAX))
                    .execute(&mut *transaction)
                    .await?;
                append_event_tx(
                    &mut transaction,
                    &session_id,
                    Some(&run_id),
                    "run.status_changed",
                    json!({ "from": RunStatus::Recovering, "to": RunStatus::WaitingRecoveryDecision, "failureCode": reason_code }),
                    recovered_at_ms,
                )
                .await?;
            }
            if eligible {
                auto_resume_run_ids.push(run_id);
            } else {
                awaiting_decision_run_ids.push(run_id);
            }
        }
        transaction.commit().await?;
        for row in &in_progress_items {
            let item_id = hachimi_protocol::ItemId::new(row.get::<String, _>("id"));
            let session_id = SessionId::new(row.get::<String, _>("session_id"));
            self.active_events.complete_item(&session_id, &item_id);
        }
        Ok(RecoveryReport {
            interrupted_runs: u64::try_from(active_runs.len()).unwrap_or(u64::MAX),
            lost_tasks,
            expired_processes,
            expired_approvals,
            interrupted_user_inputs,
            stopped_mcp_servers,
            indeterminate_side_effects,
            cancelled_side_effect_claims,
            auto_resume_run_ids,
            awaiting_decision_run_ids,
        })
    }

    pub async fn recover_interrupted_with_run_recovery(
        &self,
        run_recovery_enabled: bool,
    ) -> Result<RecoveryReport, AgentStoreError> {
        let mut report = self.recover_interrupted().await?;
        if run_recovery_enabled || report.interrupted_runs == 0 {
            return Ok(report);
        }
        let mut transaction = self.pool.begin().await?;
        let recoverable = sqlx::query(
            "SELECT id, session_id, status AS effective_status FROM runs WHERE status IN ('recovering', 'waiting_recovery_decision')",
        )
        .fetch_all(&mut *transaction)
        .await?;
        let updated_at_ms = now_ms();
        for row in recoverable {
            let run_id = RunId::new(row.get::<String, _>("id"));
            let session_id = SessionId::new(row.get::<String, _>("session_id"));
            let previous = RunStatus::parse(row.get("effective_status")).ok_or_else(|| {
                AgentStoreError::InvalidPersistedValue {
                    kind: "run status",
                    value: row.get("effective_status"),
                }
            })?;
            sqlx::query(
                "UPDATE runs SET status = 'interrupted', failure_code = 'run_recovery_feature_disabled', updated_at_ms = ? WHERE id = ?",
            )
            .bind(updated_at_ms)
            .bind(run_id.as_str())
            .execute(&mut *transaction)
            .await?;
            append_event_tx(
                &mut transaction,
                &session_id,
                Some(&run_id),
                "run.status_changed",
                json!({
                    "from": previous,
                    "to": RunStatus::Interrupted,
                    "failureCode": "run_recovery_feature_disabled"
                }),
                updated_at_ms,
            )
            .await?;
        }
        sqlx::query(
            "UPDATE run_recoveries SET state = 'failed', reason_code = 'run_recovery_feature_disabled', updated_at_ms = ? WHERE state IN ('eligible_auto', 'awaiting_user')",
        )
        .bind(updated_at_ms)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        report.auto_resume_run_ids.clear();
        report.awaiting_decision_run_ids.clear();
        Ok(report)
    }
}

#[cfg(test)]
#[path = "agent_store/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "agent_store/agent_task_tests.rs"]
mod agent_task_tests;
