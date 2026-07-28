//! Transport-neutral Session context, Run origin and Scheduler contracts.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::{
    ArtifactId, AvatarId, EntryProfile, ExecutionTarget, McpServerId, MutationContext,
    PermissionProfile, ProjectId, RunId, ScheduleGrantId, ScheduleId, SessionId, SkillId,
    TaskRunId, WorkloadKind,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionContextBinding {
    General,
    Project {
        project_id: ProjectId,
        checkout_id: super::CheckoutId,
    },
    Avatar {
        avatar_id: Option<AvatarId>,
    },
}

impl SessionContextBinding {
    #[must_use]
    pub const fn project_id(&self) -> Option<&ProjectId> {
        match self {
            Self::Project { project_id, .. } => Some(project_id),
            Self::General | Self::Avatar { .. } => None,
        }
    }

    #[must_use]
    pub const fn checkout_id(&self) -> Option<&super::CheckoutId> {
        match self {
            Self::Project { checkout_id, .. } => Some(checkout_id),
            Self::General | Self::Avatar { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunOrigin {
    #[default]
    Interactive,
    Handoff {
        source_session_id: SessionId,
        source_run_id: RunId,
    },
    Scheduled {
        schedule_id: ScheduleId,
        task_run_id: TaskRunId,
        #[specta(type = specta_typescript::Number)]
        scheduled_for_ms: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScheduleSpec {
    At {
        #[specta(type = specta_typescript::Number)]
        timestamp_ms: i64,
    },
    Every {
        #[specta(type = specta_typescript::Number)]
        interval_ms: u64,
        #[specta(type = specta_typescript::Number)]
        anchor_ms: i64,
    },
    Cron {
        expression: String,
        timezone: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScheduleContextTemplate {
    General,
    Project {
        project_id: ProjectId,
        execution_target: ExecutionTarget,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum MisfirePolicy {
    #[default]
    Skip,
    CatchUpOnce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryPolicy {
    #[default]
    TaskTabOnly,
    TaskTabAndSystemNotification,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleHealth {
    #[default]
    Healthy,
    NeedsAuthorization,
    NeedsAttention,
    Invalid,
}

impl ScheduleHealth {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::NeedsAuthorization => "needs_authorization",
            Self::NeedsAttention => "needs_attention",
            Self::Invalid => "invalid",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "healthy" => Self::Healthy,
            "needs_authorization" => Self::NeedsAuthorization,
            "needs_attention" => Self::NeedsAttention,
            "invalid" => Self::Invalid,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpToolSelection {
    pub server_id: McpServerId,
    pub tool_name: String,
    pub schema_hash: String,
    pub host_identity_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleSkillSelection {
    pub skill_id: SkillId,
    pub content_hash: String,
    pub tree_revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SchedulePermissionConfig {
    pub permission_profile: PermissionProfile,
    pub allow_file_read: bool,
    pub allow_file_write: bool,
    pub allow_exec: bool,
    pub external_targets: Vec<String>,
}

impl Default for SchedulePermissionConfig {
    fn default() -> Self {
        Self {
            permission_profile: PermissionProfile::ReadOnly,
            allow_file_read: false,
            allow_file_write: false,
            allow_exec: false,
            external_targets: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleDefinition {
    pub id: ScheduleId,
    pub name: String,
    pub enabled: bool,
    pub prompt: String,
    pub schedule: ScheduleSpec,
    pub entry_profile: EntryProfile,
    pub workload_override: Option<WorkloadKind>,
    pub context_template: ScheduleContextTemplate,
    pub tool_allowlist: Vec<String>,
    pub skill_allowlist: Vec<SkillId>,
    pub mcp_tool_allowlist: Vec<McpToolSelection>,
    pub permission_config: SchedulePermissionConfig,
    #[specta(type = specta_typescript::Number)]
    pub permission_revision: u64,
    #[specta(type = specta_typescript::Number)]
    pub timeout_ms: u64,
    pub misfire_policy: MisfirePolicy,
    pub delivery_policy: DeliveryPolicy,
    #[specta(type = specta_typescript::Number)]
    pub config_revision: u64,
    pub created_by: String,
    #[specta(type = Option<specta_typescript::Number>)]
    pub next_run_at_ms: Option<i64>,
    pub health: ScheduleHealth,
    pub health_reason: Option<String>,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleGrantStatus {
    #[default]
    Active,
    Revoked,
    Superseded,
}

impl ScheduleGrantStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Revoked => "revoked",
            Self::Superseded => "superseded",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "active" => Self::Active,
            "revoked" => Self::Revoked,
            "superseded" => Self::Superseded,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleAuthorizationScope {
    pub entry_profile: EntryProfile,
    pub workload_override: Option<WorkloadKind>,
    pub context_template: ScheduleContextTemplate,
    pub tool_allowlist: Vec<String>,
    pub skill_allowlist: Vec<SkillId>,
    #[serde(default)]
    pub skill_revisions: Vec<ScheduleSkillSelection>,
    pub mcp_tool_allowlist: Vec<McpToolSelection>,
    pub permission_config: SchedulePermissionConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleGrantRecord {
    pub id: ScheduleGrantId,
    pub schedule_id: ScheduleId,
    #[specta(type = specta_typescript::Number)]
    pub permission_revision: u64,
    pub scope_hash: String,
    pub scope: ScheduleAuthorizationScope,
    pub status: ScheduleGrantStatus,
    pub granted_by: String,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = Option<specta_typescript::Number>)]
    pub revoked_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskRunStatus {
    #[default]
    Queued,
    Preparing,
    Running,
    NeedsAttention,
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
    Lost,
    Skipped,
}

impl TaskRunStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Preparing => "preparing",
            Self::Running => "running",
            Self::NeedsAttention => "needs_attention",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
            Self::Lost => "lost",
            Self::Skipped => "skipped",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "queued" => Self::Queued,
            "preparing" => Self::Preparing,
            "running" => Self::Running,
            "needs_attention" => Self::NeedsAttention,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "timed_out" => Self::TimedOut,
            "cancelled" => Self::Cancelled,
            "lost" => Self::Lost,
            "skipped" => Self::Skipped,
            _ => return None,
        })
    }

    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::NeedsAttention
                | Self::Succeeded
                | Self::Failed
                | Self::TimedOut
                | Self::Cancelled
                | Self::Lost
                | Self::Skipped
        )
    }

    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        use TaskRunStatus::{
            Cancelled, Failed, Lost, NeedsAttention, Preparing, Queued, Running, Skipped,
            Succeeded, TimedOut,
        };
        matches!(
            (self, next),
            (
                Queued,
                Preparing | Cancelled | NeedsAttention | Skipped | Lost
            ) | (
                Preparing,
                Running | Cancelled | Failed | TimedOut | NeedsAttention | Lost
            ) | (
                Running,
                Cancelled | Succeeded | Failed | TimedOut | NeedsAttention | Lost
            )
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskRunTrigger {
    #[default]
    Scheduled,
    Manual,
    Retry,
    CatchUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    #[default]
    Pending,
    NotRequested,
    Delivered,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskRunRecord {
    pub id: TaskRunId,
    pub schedule_id: Option<ScheduleId>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub schedule_revision: Option<u64>,
    pub trigger: TaskRunTrigger,
    #[specta(type = Option<specta_typescript::Number>)]
    pub scheduled_for_ms: Option<i64>,
    pub invocation_key: String,
    pub requester_session_id: Option<SessionId>,
    pub execution_session_id: Option<SessionId>,
    pub run_id: Option<RunId>,
    pub permission_snapshot_hash: Option<String>,
    pub status: TaskRunStatus,
    pub progress_percent: Option<u8>,
    pub result_summary: Option<String>,
    pub error_code: Option<String>,
    pub error_summary: Option<String>,
    pub artifact_ids: Vec<ArtifactId>,
    pub delivery_status: DeliveryStatus,
    pub delivery_error_code: Option<String>,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = Option<specta_typescript::Number>)]
    pub started_at_ms: Option<i64>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub finished_at_ms: Option<i64>,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleCreateRequest {
    pub context: MutationContext,
    pub definition: ScheduleDefinition,
    pub authorize: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleUpdateRequest {
    pub context: MutationContext,
    pub definition: ScheduleDefinition,
    #[specta(type = specta_typescript::Number)]
    pub expected_config_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SchedulePreview {
    pub valid: bool,
    pub error_code: Option<String>,
    #[specta(type = Vec<specta_typescript::Number>)]
    pub next_occurrences_ms: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleSnapshot {
    pub definition: ScheduleDefinition,
    pub active_grant: Option<ScheduleGrantRecord>,
    pub recent_runs: Vec<TaskRunRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskInteractiveContinuation {
    pub task_run: TaskRunRecord,
    pub session: super::SessionRecord,
    pub run: super::RunRecord,
}
