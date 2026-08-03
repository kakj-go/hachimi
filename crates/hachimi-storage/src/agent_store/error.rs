use hachimi_protocol::{ApprovalId, PlanId, RunId, RunStatus, SessionId};
use thiserror::Error;

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
    AttachmentNotFound(hachimi_protocol::AttachmentId),
    #[error("browser workspace does not exist: {0}")]
    BrowserWorkspaceNotFound(hachimi_protocol::BrowserWorkspaceId),
    #[error("browser tab does not exist: {0}")]
    BrowserTabNotFound(hachimi_protocol::BrowserTabId),
    #[error("browser automation lease does not exist: {0}")]
    BrowserAutomationLeaseNotFound(hachimi_protocol::BrowserAutomationLeaseId),
    #[error("browser automation lease revision precondition failed")]
    BrowserAutomationLeaseRevisionConflict,
    #[error("browser automation lease is unavailable for this workspace")]
    BrowserAutomationLeaseUnavailable,
    #[error("browser workspace revision precondition failed")]
    BrowserWorkspaceRevisionConflict,
    #[error("embedded browser settings revision precondition failed")]
    EmbeddedBrowserSettingsRevisionConflict,
    #[error("embedded browser permission request does not exist: {0}")]
    EmbeddedBrowserPermissionRequestNotFound(hachimi_protocol::ItemId),
    #[error("embedded browser permission request is stale or no longer pending")]
    EmbeddedBrowserPermissionRequestStale,
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
    McpServerNotFound(hachimi_protocol::McpServerId),
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
