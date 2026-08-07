//! Stable, transport-neutral Harness Agent contracts.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::{ClientId, LlmSettings, ProviderProtocolKind, RequestId};

mod agent_tasks;
mod enterprise;
mod gateway;
mod git_forge;
mod hosts;
mod ids;
mod lifecycle;
mod local_hosts;
mod mcp;
mod permissions;
mod plugins;
mod process;
mod project_git;
mod recovery;
mod review;
mod runtime;
mod sandbox_runtime;
mod schedule;
mod skills;
mod workbench;
mod workspace;

pub use agent_tasks::*;
pub use enterprise::*;
pub use gateway::*;
pub use git_forge::*;
pub use hosts::*;
pub use ids::*;
pub use lifecycle::*;
pub use local_hosts::*;
pub use mcp::*;
pub use permissions::*;
pub use plugins::*;
pub use process::*;
pub use project_git::*;
pub use recovery::*;
pub use review::*;
pub use runtime::*;
pub use sandbox_runtime::*;
pub use schedule::*;
pub use skills::*;
pub use workbench::*;
pub use workspace::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecutionTarget {
    Local {
        project_id: ProjectId,
    },
    ManagedWorktree {
        project_id: ProjectId,
        base_revision: String,
    },
}

impl ExecutionTarget {
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        match self {
            Self::Local { project_id } | Self::ManagedWorktree { project_id, .. } => project_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum RunDriverKind {
    #[default]
    ToolLoop,
    Realtime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum RunPurpose {
    #[default]
    Task,
    Review,
    Automation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum EntryProfile {
    #[default]
    Workbench,
    PetConversation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadKind {
    #[default]
    General,
    Coding,
    Office,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadResolutionSource {
    UserOverride,
    ExplicitSkill,
    BuiltInSkill,
    StructuredClassification,
    GeneralFallback,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkloadResolution {
    pub workload: WorkloadKind,
    pub source: WorkloadResolutionSource,
    pub activated_skill_ids: Vec<SkillId>,
    pub reason: String,
    pub classifier_revision: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum BehaviorMode {
    #[default]
    Default,
    Plan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    #[default]
    OnlyWhenNeeded,
    AlwaysAskSideEffects,
    NeverPrompt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionProfile {
    ReadOnly,
    #[default]
    Writable,
    FullAccess,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionPermissionConfig {
    pub policy: AgentPermissionPolicy,
    #[serde(default)]
    pub extra_authorizations: Vec<SessionExtraAuthorizationSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionExtraAuthorizationSummary {
    pub id: ApprovalId,
    pub action: String,
    pub resource: String,
    pub target_host: String,
    #[specta(type = specta_typescript::Number)]
    pub granted_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionPermissionConfigRequest {
    pub session_id: Option<SessionId>,
    pub entry_profile: EntryProfile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionPermissionConfigUpdate {
    pub session_id: Option<SessionId>,
    pub entry_profile: EntryProfile,
    #[specta(type = specta_typescript::Number)]
    pub expected_revision: u64,
    pub config: SessionPermissionConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunBudget {
    pub max_model_requests: u32,
    pub max_tool_calls: u32,
    pub max_parallel_read_tools: u16,
    #[specta(type = specta_typescript::Number)]
    pub model_timeout_ms: u64,
    #[specta(type = specta_typescript::Number)]
    pub tool_timeout_ms: u64,
}

impl Default for RunBudget {
    fn default() -> Self {
        Self {
            max_model_requests: 32,
            max_tool_calls: 128,
            max_parallel_read_tools: 8,
            model_timeout_ms: 120_000,
            tool_timeout_ms: 120_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunConfiguration {
    pub model_snapshot: LlmSettings,
    pub driver: RunDriverKind,
    pub entry_profile: EntryProfile,
    pub workload_override: Option<WorkloadKind>,
    pub behavior_mode: BehaviorMode,
    pub execution_target: Option<ExecutionTarget>,
    pub approval_policy: ApprovalPolicy,
    pub permission_profile: PermissionProfile,
    pub budget: RunBudget,
    #[serde(default)]
    pub accepted_plan_id: Option<PlanId>,
    #[serde(default)]
    pub accepted_plan_revision: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CheckoutKind {
    Local,
    ManagedWorktree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum CheckoutStatus {
    #[default]
    Preparing,
    Ready,
    Dirty,
    CleanupBlocked,
    Removed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    #[default]
    Queued,
    Preparing,
    Running,
    WaitingApproval,
    WaitingUserInput,
    Recovering,
    WaitingRecoveryDecision,
    Cancelling,
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
    Interrupted,
    Lost,
}

impl RunStatus {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded
                | Self::Failed
                | Self::TimedOut
                | Self::Cancelled
                | Self::Interrupted
                | Self::Lost
        )
    }

    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        use RunStatus::{
            Cancelled, Cancelling, Failed, Interrupted, Lost, Preparing, Queued, Recovering,
            Running, Succeeded, TimedOut, WaitingApproval, WaitingRecoveryDecision,
            WaitingUserInput,
        };
        matches!(
            (self, next),
            (Queued, Preparing | Cancelled | Interrupted | Lost)
                | (Preparing, Running | Failed | Cancelled | Interrupted | Lost)
                | (
                    Running,
                    WaitingApproval
                        | WaitingUserInput
                        | Cancelling
                        | Succeeded
                        | Failed
                        | TimedOut
                        | Interrupted
                        | Lost
                )
                | (
                    WaitingApproval,
                    Running | Cancelling | Failed | Interrupted | Lost
                )
                | (
                    WaitingUserInput,
                    Running | Cancelling | Failed | Interrupted | Lost
                )
                | (
                    Interrupted | Lost,
                    Recovering | WaitingRecoveryDecision | Cancelled
                )
                | (
                    Recovering,
                    Queued
                        | Preparing
                        | Running
                        | WaitingRecoveryDecision
                        | Failed
                        | Cancelled
                        | Lost
                )
                | (
                    WaitingRecoveryDecision,
                    Queued | Recovering | Preparing | Running | Failed | Cancelled | Lost
                )
                | (Cancelling, Cancelled | Failed | Interrupted | Lost)
        )
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Preparing => "preparing",
            Self::Running => "running",
            Self::WaitingApproval => "waiting_approval",
            Self::WaitingUserInput => "waiting_user_input",
            Self::Recovering => "recovering",
            Self::WaitingRecoveryDecision => "waiting_recovery_decision",
            Self::Cancelling => "cancelling",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
            Self::Lost => "lost",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "queued" => Self::Queued,
            "preparing" => Self::Preparing,
            "running" => Self::Running,
            "waiting_approval" => Self::WaitingApproval,
            "waiting_user_input" => Self::WaitingUserInput,
            "recovering" => Self::Recovering,
            "waiting_recovery_decision" => Self::WaitingRecoveryDecision,
            "cancelling" => Self::Cancelling,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "timed_out" => Self::TimedOut,
            "cancelled" => Self::Cancelled,
            "interrupted" => Self::Interrupted,
            "lost" => Self::Lost,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum UserInputStatus {
    #[default]
    Pending,
    Resolved,
    Cancelled,
    Expired,
    Interrupted,
}

impl UserInputStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Resolved => "resolved",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
            Self::Interrupted => "interrupted",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "resolved" => Self::Resolved,
            "cancelled" => Self::Cancelled,
            "expired" => Self::Expired,
            "interrupted" => Self::Interrupted,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UserInputOption {
    pub label: String,
    pub value: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UserInputQuestion {
    pub id: String,
    pub header: String,
    pub prompt: String,
    pub options: Vec<UserInputOption>,
    pub secret: bool,
    #[specta(type = Option<specta_typescript::Number>)]
    pub auto_resolution_ms: Option<u64>,
    pub default_answer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UserInputAnswer {
    pub question_id: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UserInputDisplayAnswer {
    pub question_id: String,
    pub value: Option<String>,
    pub secret_provided: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UserInputRequestRecord {
    pub id: UserInputRequestId,
    pub session_id: SessionId,
    pub run_id: RunId,
    #[specta(type = specta_typescript::Number)]
    pub run_generation: u64,
    pub item_id: ItemId,
    pub questions: Vec<UserInputQuestion>,
    #[serde(default)]
    pub display_answers: Vec<UserInputDisplayAnswer>,
    pub status: UserInputStatus,
    #[specta(type = Option<specta_typescript::Number>)]
    pub expires_at_ms: Option<i64>,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = Option<specta_typescript::Number>)]
    pub resolved_at_ms: Option<i64>,
    pub resolved_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UserInputResolution {
    pub request_id: UserInputRequestId,
    pub expected_run_id: RunId,
    #[specta(type = specta_typescript::Number)]
    pub expected_generation: u64,
    #[serde(default)]
    pub action: UserInputResolutionAction,
    pub answers: Vec<UserInputAnswer>,
    pub resolved_by: String,
    #[specta(type = specta_typescript::Number)]
    pub resolved_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum UserInputResolutionAction {
    #[default]
    Submit,
    Decline,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Pending,
    InProgress,
    #[default]
    Completed,
    Failed,
    Interrupted,
}

impl ItemStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "interrupted" => Self::Interrupted,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    pub id: PlanStepId,
    pub description: String,
    pub status: PlanStepStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ItemRelations {
    pub tool_call_id: Option<ToolCallId>,
    pub agent_task_id: Option<AgentTaskId>,
    pub approval_id: Option<ApprovalId>,
    pub user_input_request_id: Option<UserInputRequestId>,
    pub process_session_id: Option<ProcessSessionId>,
    pub plan_step_id: Option<PlanStepId>,
    pub artifact_ids: Vec<ArtifactId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptItemKind {
    User,
    Assistant,
    Reasoning,
    ToolExecution,
    Plan,
    Approval,
    UserInputRequest,
    CommandExecution,
    FileChange,
    McpCall,
    DynamicToolCall,
    CollabToolCall,
    ContextCompaction,
    Review,
    SystemContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentMessagePhase {
    Commentary,
    FinalAnswer,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ItemPayload {
    User {
        text: String,
        attachment_ids: Vec<AttachmentId>,
    },
    Assistant {
        text: String,
        #[serde(default)]
        phase: AgentMessagePhase,
    },
    Reasoning {
        summary: String,
        source: ReasoningSummarySource,
        provider_endpoint_id: Option<ProviderEndpointId>,
        provider_account_id: Option<ProviderAccountId>,
        protocol: ProviderProtocolKind,
        capability_revision: String,
    },
    Plan {
        plan_id: PlanId,
        revision: u32,
        text: String,
        steps: Vec<PlanStep>,
    },
    ToolExecution {
        tool_call_id: ToolCallId,
        name: String,
        #[specta(type = specta_typescript::Unknown)]
        arguments: Value,
        #[specta(type = specta_typescript::Number)]
        step_revision: u64,
        tool_plan_hash: String,
        registry_revision: String,
        result: Option<ToolExecutionResult>,
    },
    Approval {
        approval_id: ApprovalId,
        status: ApprovalStatus,
        summary: String,
    },
    UserInputRequest {
        request_id: UserInputRequestId,
        questions: Vec<UserInputQuestion>,
        #[serde(default)]
        display_answers: Vec<UserInputDisplayAnswer>,
    },
    CommandExecution {
        process_session_id: ProcessSessionId,
        command_summary: String,
        command: String,
        cwd: Option<String>,
        status: String,
        aggregated_output: String,
        exit_code: Option<i32>,
        #[specta(type = Option<specta_typescript::Number>)]
        duration_ms: Option<u64>,
        output_artifact_id: Option<ArtifactId>,
    },
    FileChange {
        path: String,
        change_kind: String,
        artifact_id: Option<ArtifactId>,
    },
    McpCall {
        server_id: McpServerId,
        tool_name: String,
        status: String,
        #[specta(type = specta_typescript::Unknown)]
        arguments: Value,
        #[specta(type = Option<specta_typescript::Unknown>)]
        result: Option<Value>,
        error: Option<String>,
    },
    DynamicToolCall {
        namespace: String,
        name: String,
        status: String,
        #[specta(type = specta_typescript::Unknown)]
        arguments: Value,
        #[specta(type = Option<specta_typescript::Unknown>)]
        result: Option<Value>,
        error: Option<String>,
    },
    CollabToolCall {
        tool_name: String,
        agent_task_id: Option<AgentTaskId>,
        parent_run_id: RunId,
        child_run_id: Option<RunId>,
        title: String,
        status: String,
        summary: Option<String>,
        usage: TokenUsage,
    },
    ContextCompaction {
        checkpoint_id: Option<CompactionCheckpointId>,
        trigger: CompactionTrigger,
        phase: CompactionPhase,
        implementation: CompactionImplementation,
        reason: CompactionReason,
        token_snapshot: Option<CompactionTokenSnapshot>,
        trimmed_history_groups: u32,
        warnings: Vec<String>,
        error_code: Option<String>,
        summary_source: CompactionSummarySource,
        provider_endpoint_id: Option<ProviderEndpointId>,
        provider_account_id: Option<ProviderAccountId>,
        capability_revision: Option<String>,
        fallback_reason: Option<String>,
    },
    Review {
        review_id: ReviewId,
        summary: String,
        overall_correctness: String,
        overall_confidence_score: f32,
        finding_count: u32,
        used_plain_text_fallback: bool,
    },
    SystemContext {
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolExecutionResult {
    pub status: String,
    pub model_content: String,
    #[specta(type = specta_typescript::Unknown)]
    pub structured_content: Value,
    pub stable_result_code: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptItem {
    pub id: ItemId,
    pub session_id: SessionId,
    pub run_id: Option<RunId>,
    #[specta(type = specta_typescript::Number)]
    pub sequence: u64,
    pub kind: TranscriptItemKind,
    pub status: ItemStatus,
    pub payload: ItemPayload,
    pub relations: ItemRelations,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ItemDeltaPayload {
    Text { text: String },
    CommandOutput { stream: String, text: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum CompactionReason {
    #[default]
    Automatic,
    Manual,
    Reactive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum CompactionTrigger {
    #[default]
    Auto,
    Manual,
    ProviderOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum CompactionPhase {
    #[default]
    PreRun,
    MidRun,
    Standalone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum CompactionImplementation {
    #[default]
    Local,
    Remote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum CompactionSummarySource {
    #[default]
    Local,
    ProviderRemote,
    LocalFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningSummarySource {
    #[default]
    ProviderPublic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum TokenCountSource {
    Provider,
    Tokenizer,
    #[default]
    ConservativeEstimate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "camelCase")]
pub struct CompactionTokenSnapshot {
    pub billed_usage: TokenUsage,
    #[specta(type = specta_typescript::Number)]
    pub active_context_tokens_before: u64,
    #[specta(type = specta_typescript::Number)]
    pub active_context_tokens_after: u64,
    #[specta(type = specta_typescript::Number)]
    pub remaining_context_tokens: u64,
    pub source: TokenCountSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "camelCase")]
pub struct CompactionLifecycle {
    pub trigger: CompactionTrigger,
    pub phase: CompactionPhase,
    pub implementation: CompactionImplementation,
    pub token_snapshot: Option<CompactionTokenSnapshot>,
    pub trimmed_history_groups: u32,
    pub summary_source: CompactionSummarySource,
    pub provider_endpoint_id: Option<ProviderEndpointId>,
    pub provider_account_id: Option<ProviderAccountId>,
    pub capability_revision: Option<String>,
    pub fallback_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CompactionSummary {
    pub semantic_markdown: String,
    pub latest_user_goal: Option<String>,
    pub preserved_identifiers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CompactionQuality {
    pub accepted: bool,
    #[specta(type = specta_typescript::Number)]
    pub source_items: u64,
    #[specta(type = specta_typescript::Number)]
    pub source_chars: u64,
    #[specta(type = specta_typescript::Number)]
    pub summary_chars: u64,
    #[specta(type = specta_typescript::Number)]
    pub recent_tail_items: u64,
    #[specta(type = specta_typescript::Number)]
    pub preserved_identifier_count: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CompactionCheckpoint {
    pub id: CompactionCheckpointId,
    pub session_id: SessionId,
    pub run_id: Option<RunId>,
    pub previous_checkpoint_id: Option<CompactionCheckpointId>,
    #[specta(type = specta_typescript::Number)]
    pub covered_through_sequence: u64,
    pub reason: CompactionReason,
    #[serde(default)]
    pub lifecycle: CompactionLifecycle,
    pub summary: CompactionSummary,
    pub quality: CompactionQuality,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunUsageSnapshot {
    pub run_id: RunId,
    pub billed_usage: TokenUsage,
    #[specta(type = specta_typescript::Number)]
    pub active_context_tokens: u64,
    #[specta(type = specta_typescript::Number)]
    pub remaining_context_tokens: u64,
    pub source: TokenCountSource,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectExecutionStatus {
    Claimed,
    Dispatched,
    Succeeded,
    Failed,
    Cancelled,
    Indeterminate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SideEffectExecutionRecord {
    pub id: SideEffectExecutionId,
    pub session_id: SessionId,
    pub run_id: RunId,
    #[specta(type = specta_typescript::Number)]
    pub run_generation: u64,
    pub tool_call_id: ToolCallId,
    pub idempotency_key: String,
    pub parameter_hash: String,
    pub approval_id: Option<ApprovalId>,
    pub host_request_id: Option<String>,
    pub status: SideEffectExecutionStatus,
    pub result_code: Option<String>,
    pub result_reference: Option<ArtifactId>,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum RunEventPayload {
    ItemStarted {
        item: Box<TranscriptItem>,
    },
    ItemDelta {
        item_id: ItemId,
        delta: ItemDeltaPayload,
    },
    ItemCompleted {
        item: Box<TranscriptItem>,
    },
    PlanUpdated {
        plan_id: PlanId,
        explanation: Option<String>,
        steps: Vec<PlanStep>,
    },
    DiffUpdated {
        artifact_id: Option<ArtifactId>,
        changed_files: u32,
        additions: u32,
        deletions: u32,
    },
    UserInputRequested {
        request_id: UserInputRequestId,
    },
    Generic {
        event: String,
        #[specta(type = specta_typescript::Unknown)]
        data: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunEventEnvelope {
    pub protocol_version: u32,
    #[specta(type = specta_typescript::Number)]
    pub sequence: u64,
    pub session_id: SessionId,
    pub run_id: Option<RunId>,
    pub payload: RunEventPayload,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
}

impl RunEventEnvelope {
    /// Returns the stable event name without reintroducing an untyped payload.
    #[must_use]
    pub fn event_name(&self) -> &str {
        match &self.payload {
            RunEventPayload::ItemStarted { .. } => "item.started",
            RunEventPayload::ItemDelta { .. } => "item.delta",
            RunEventPayload::ItemCompleted { .. } => "item.completed",
            RunEventPayload::PlanUpdated { .. } => "plan.updated",
            RunEventPayload::DiffUpdated { .. } => "run.diff.updated",
            RunEventPayload::UserInputRequested { .. } => "user_input.requested",
            RunEventPayload::Generic { event, .. } => event,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProposedPlanStatus {
    #[default]
    Proposed,
    Accepted,
    Superseded,
}

impl ProposedPlanStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Accepted => "accepted",
            Self::Superseded => "superseded",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "proposed" => Self::Proposed,
            "accepted" => Self::Accepted,
            "superseded" => Self::Superseded,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProposedPlan {
    pub id: PlanId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub revision: u32,
    pub goal: String,
    pub assumptions: Vec<String>,
    pub steps: Vec<PlanStep>,
    pub affected_resources: Vec<String>,
    pub verification: Vec<String>,
    pub risks: Vec<String>,
    pub open_questions: Vec<String>,
    pub content_markdown: String,
    pub status: ProposedPlanStatus,
    pub accepted_run_id: Option<RunId>,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = Option<specta_typescript::Number>)]
    pub accepted_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRecord {
    pub id: ProjectId,
    pub display_name: String,
    pub root_path: String,
    pub git_root: Option<String>,
    pub trusted: bool,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentRecord {
    pub id: AttachmentId,
    pub content_hash: String,
    pub original_name: String,
    pub mime_type: String,
    #[specta(type = specta_typescript::Number)]
    pub byte_size: u64,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CheckoutRecord {
    pub id: CheckoutId,
    pub project_id: ProjectId,
    pub kind: CheckoutKind,
    pub path: String,
    pub base_revision: Option<String>,
    pub head_revision: Option<String>,
    pub status: CheckoutStatus,
    pub pinned: bool,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    pub id: SessionId,
    pub context: SessionContextBinding,
    pub entry_profile: EntryProfile,
    pub title: String,
    pub archived: bool,
    pub pinned: bool,
    pub parent_session_id: Option<SessionId>,
    pub source_run_id: Option<RunId>,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub id: RunId,
    pub session_id: SessionId,
    pub status: RunStatus,
    pub purpose: RunPurpose,
    #[serde(default)]
    pub origin: RunOrigin,
    #[specta(type = specta_typescript::Number)]
    pub generation: u64,
    pub configuration: RunConfiguration,
    pub requested_capabilities: ProviderCapabilities,
    pub negotiated_capabilities: ProviderCapabilities,
    pub provider_capability_probe: Option<crate::ProviderCapabilityProbe>,
    pub capability_degradations: Vec<CapabilityDegradation>,
    pub failure_code: Option<String>,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpServerTransport {
    Stdio {
        command: String,
        args: Vec<String>,
        cwd: Option<String>,
    },
    StreamableHttp {
        url: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpHeaderView {
    pub name: String,
    pub value: Option<String>,
    pub secret: bool,
    pub configured: bool,
    #[serde(default, skip_serializing)]
    #[specta(skip)]
    pub credential_reference: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpHeaderInput {
    pub name: String,
    pub value: Option<String>,
    pub secret: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpServerRecord {
    pub id: McpServerId,
    pub display_name: String,
    pub enabled: bool,
    pub transport: McpServerTransport,
    pub headers: Vec<McpHeaderView>,
    /// Tool names explicitly classified by the user as read-only. A server's own annotations
    /// never populate this list and therefore cannot grant itself lower-risk access.
    pub read_only_tools: Vec<String>,
    #[specta(type = specta_typescript::Number)]
    pub startup_timeout_ms: u64,
    #[specta(type = specta_typescript::Number)]
    pub request_timeout_ms: u64,
    #[specta(type = specta_typescript::Number)]
    pub max_message_bytes: u64,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename = "McpServerDraft", rename_all = "camelCase")]
pub struct McpServerUpsertRequest {
    pub id: McpServerId,
    pub display_name: String,
    pub enabled: bool,
    pub transport: McpServerTransport,
    pub headers: Vec<McpHeaderInput>,
    pub read_only_tools: Vec<String>,
    #[specta(type = specta_typescript::Number)]
    pub startup_timeout_ms: u64,
    #[specta(type = specta_typescript::Number)]
    pub request_timeout_ms: u64,
    #[specta(type = specta_typescript::Number)]
    pub max_message_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum McpServerHealthState {
    Disabled,
    #[default]
    Stopped,
    Starting,
    Ready,
    Failed,
}

impl McpServerHealthState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "disabled" => Self::Disabled,
            "stopped" => Self::Stopped,
            "starting" => Self::Starting,
            "ready" => Self::Ready,
            "failed" => Self::Failed,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpServerHealthRecord {
    pub server_id: McpServerId,
    pub state: McpServerHealthState,
    pub server_name: Option<String>,
    pub server_version: Option<String>,
    pub protocol_version: Option<String>,
    pub tool_count: u32,
    /// Stable, non-sensitive error category. Transport payloads and stderr are never persisted.
    pub error_code: Option<String>,
    pub failure_count: u32,
    #[specta(type = Option<specta_typescript::Number>)]
    pub next_retry_at_ms: Option<i64>,
    #[specta(type = specta_typescript::Number)]
    pub checked_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpServerView {
    pub configuration: McpServerRecord,
    pub health: McpServerHealthRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpToolView {
    pub server_id: McpServerId,
    pub name: String,
    pub exposed_name: String,
    pub description: Option<String>,
    #[specta(type = specta_typescript::Unknown)]
    pub input_schema: Value,
    pub required_parameters: Vec<String>,
    pub enabled: bool,
    pub stale: bool,
    pub validation_error: Option<String>,
    pub schema_hash: String,
    /// Hash of the configured MCP Host identity (transport, endpoint/command,
    /// and sandbox profile), excluding credentials and header values.
    pub host_identity_hash: String,
    #[specta(type = specta_typescript::Number)]
    pub discovered_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpToolOverride {
    pub server_id: McpServerId,
    pub tool_name: String,
    pub enabled: bool,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpConnectionTestResult {
    pub success: bool,
    pub server_name: Option<String>,
    pub server_version: Option<String>,
    pub protocol_version: Option<String>,
    pub tools: Vec<McpToolView>,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    DiffEvidence,
    CommandEvidence,
    EnterpriseAttachment,
}

impl ArtifactKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DiffEvidence => "diff_evidence",
            Self::CommandEvidence => "command_evidence",
            Self::EnterpriseAttachment => "enterprise_attachment",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "diff_evidence" => Self::DiffEvidence,
            "command_evidence" => Self::CommandEvidence,
            "enterprise_attachment" => Self::EnterpriseAttachment,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRecord {
    pub id: ArtifactId,
    pub run_id: Option<RunId>,
    pub kind: ArtifactKind,
    pub display_name: String,
    pub content_hash: Option<String>,
    #[specta(type = specta_typescript::Unknown)]
    pub metadata: Value,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    #[default]
    Pending,
    Approved,
    Denied,
    Expired,
    Cancelled,
}

impl ApprovalStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::Expired => "expired",
            Self::Cancelled => "cancelled",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "approved" => Self::Approved,
            "denied" => Self::Denied,
            "expired" => Self::Expired,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalGrantScope {
    #[default]
    Once,
    Session,
    TimedLease,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequestRecord {
    pub id: ApprovalId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub tool_call_id: ToolCallId,
    #[specta(type = specta_typescript::Number)]
    pub run_generation: u64,
    pub status: ApprovalStatus,
    pub action: String,
    pub resource: String,
    pub parameter_hash: String,
    pub risk_summary: String,
    pub target_host: String,
    pub required_scopes: Vec<String>,
    pub grant_scope: ApprovalGrantScope,
    pub uses_remaining: u32,
    pub requester_principal: String,
    pub resolved_by: Option<String>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub expires_at_ms: Option<i64>,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = Option<specta_typescript::Number>)]
    pub resolved_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalResolution {
    pub approval_id: ApprovalId,
    pub decision: ApprovalStatus,
    pub parameter_hash: String,
    #[specta(type = specta_typescript::Number)]
    pub run_generation: u64,
    pub resolved_by: String,
    #[specta(type = specta_typescript::Number)]
    pub resolved_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ToolEffect {
    ReadOnly,
    WorkspaceWrite,
    Process,
    ExternalSideEffect,
    BrowserObserve,
    BrowserAct,
    ComputerObserve,
    ComputerAct,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    #[specta(type = specta_typescript::Unknown)]
    pub input_schema: Value,
    pub effect: ToolEffect,
    pub parallel_safe: bool,
    pub required_scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DynamicToolRegistration {
    pub namespace: String,
    pub name: String,
    pub description: String,
    #[specta(type = specta_typescript::Unknown)]
    pub input_schema: Value,
    pub deferred: bool,
    pub requires_strict_schema: bool,
    pub output_media: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DynamicToolValidationError {
    pub code: String,
    pub field: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ModelRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelToolCall {
    pub id: ToolCallId,
    pub name: String,
    #[specta(type = specta_typescript::Unknown)]
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelInputImage {
    pub media_type: String,
    pub data_base64: String,
    pub source_label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelMessage {
    pub role: ModelRole,
    pub content: String,
    pub name: Option<String>,
    pub tool_call_id: Option<ToolCallId>,
    pub tool_calls: Vec<ModelToolCall>,
    #[serde(default)]
    pub input_images: Vec<ModelInputImage>,
}

impl ModelMessage {
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ModelRole::User,
            content: content.into(),
            name: None,
            tool_call_id: None,
            tool_calls: Vec::new(),
            input_images: Vec::new(),
        }
    }

    #[must_use]
    pub fn user_with_images(
        content: impl Into<String>,
        input_images: Vec<ModelInputImage>,
    ) -> Self {
        Self {
            role: ModelRole::User,
            content: content.into(),
            name: None,
            tool_call_id: None,
            tool_calls: Vec::new(),
            input_images,
        }
    }

    #[must_use]
    pub fn assistant(content: impl Into<String>, tool_calls: Vec<ModelToolCall>) -> Self {
        Self {
            role: ModelRole::Assistant,
            content: content.into(),
            name: None,
            tool_call_id: None,
            tool_calls,
            input_images: Vec::new(),
        }
    }

    #[must_use]
    pub fn tool(call: &ModelToolCall, content: impl Into<String>) -> Self {
        Self {
            role: ModelRole::Tool,
            content: content.into(),
            name: Some(call.name.clone()),
            tool_call_id: Some(call.id.clone()),
            tool_calls: Vec::new(),
            input_images: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelRequest {
    pub messages: Vec<ModelMessage>,
    pub tools: Vec<ToolDescriptor>,
    pub parallel_tool_calls: bool,
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelCompactionRequest {
    pub messages: Vec<ModelMessage>,
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelCompactionResult {
    pub replacement_messages: Vec<ModelMessage>,
    pub usage: TokenUsage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    #[specta(type = specta_typescript::Number)]
    pub input_tokens: u64,
    #[specta(type = specta_typescript::Number)]
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ModelFinishReason {
    Stop,
    ToolCalls,
    Length,
    ContentFilter,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelEvent {
    AgentMessageStarted {
        message_id: String,
        phase: AgentMessagePhase,
    },
    AgentMessageDelta {
        message_id: String,
        delta: String,
    },
    AgentMessageCompleted {
        message_id: String,
    },
    /// Compatibility input for providers and fixtures that do not expose
    /// message Item boundaries. The Agent runtime normalizes it to one
    /// request-scoped message with an inferred phase.
    TextDelta {
        delta: String,
    },
    /// Provider-authored reasoning summary delta. This variant is emitted only
    /// when the negotiated provider capability explicitly supports summaries.
    ReasoningDelta {
        delta: String,
    },
    ToolCallDelta {
        index: u32,
        id: Option<ToolCallId>,
        name_delta: String,
        arguments_delta: String,
    },
    ToolCallCompleted {
        call: ModelToolCall,
    },
    Usage {
        usage: TokenUsage,
    },
    Completed {
        finish_reason: ModelFinishReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ProviderCapabilities {
    pub tool_calls: bool,
    pub parallel_tool_calls: bool,
    pub namespaced_tools: bool,
    pub deferred_tools: bool,
    pub strict_json_schema: bool,
    pub output_schema: bool,
    pub text_input: bool,
    pub image_input: bool,
    pub audio_input: bool,
    pub streaming_usage: bool,
    pub reasoning_summary: bool,
    pub realtime: bool,
    pub http_transport: bool,
    pub websocket_transport: bool,
    pub remote_compaction: bool,
    pub embeddings: bool,
    #[specta(type = Option<specta_typescript::Number>)]
    pub context_window: Option<u64>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub max_output_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDegradation {
    pub capability: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MutationContext {
    pub request_id: RequestId,
    pub client_id: ClientId,
    pub protocol_version: u32,
    pub idempotency_key: String,
    pub expected_run_id: Option<RunId>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub expected_generation: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PermissionGrantScope {
    Session,
    Run,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FileSystemAccess {
    Read,
    Write,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FileSystemGrant {
    pub access: FileSystemAccess,
    pub roots: Vec<String>,
    pub globs: Vec<String>,
    /// Exact paths relative to one of the grant roots.
    #[serde(default)]
    pub files: Vec<String>,
    pub special_roots: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "camelCase")]
pub struct NetworkGrant {
    pub enabled: bool,
    #[serde(default)]
    pub unrestricted_hosts: bool,
    pub hosts: Vec<String>,
    pub protocols: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProcessGrant {
    pub spawn: bool,
    pub interactive: bool,
    #[serde(default)]
    pub unrestricted_commands: bool,
    pub allowed_commands: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "camelCase")]
pub struct ComputerGrant {
    pub observe: bool,
    pub act: bool,
    #[serde(default)]
    pub unrestricted_targets: bool,
    pub allowed_applications: Vec<String>,
    pub max_actions: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct BrowserGrant {
    pub observe: bool,
    pub act: bool,
    pub upload: bool,
    pub download: bool,
    pub cookie_storage: bool,
    pub cdp: bool,
    #[serde(default)]
    pub unrestricted_origins: bool,
    pub origins: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(default, rename_all = "camelCase")]
pub struct CapabilityGrantSet {
    pub profile: PermissionProfile,
    pub scope: PermissionGrantScope,
    pub session_id: SessionId,
    pub run_id: Option<RunId>,
    pub source: String,
    pub file_system: Vec<FileSystemGrant>,
    pub network: NetworkGrant,
    pub process: ProcessGrant,
    pub browser: BrowserGrant,
    pub computer: ComputerGrant,
    pub review_each_command: bool,
    #[specta(type = Option<specta_typescript::Number>)]
    pub expires_at_ms: Option<i64>,
}

impl Default for CapabilityGrantSet {
    fn default() -> Self {
        Self {
            profile: PermissionProfile::ReadOnly,
            scope: PermissionGrantScope::Run,
            session_id: SessionId::new("unbound"),
            run_id: None,
            source: "default".into(),
            file_system: Vec::new(),
            network: NetworkGrant::default(),
            process: ProcessGrant::default(),
            browser: BrowserGrant::default(),
            computer: ComputerGrant::default(),
            review_each_command: true,
            expires_at_ms: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum SandboxReadiness {
    #[default]
    Unavailable,
    SetupRequired,
    Degraded,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SandboxCapabilityReport {
    pub backend: String,
    pub readiness: SandboxReadiness,
    pub os_enforced: bool,
    pub filesystem_enforced: bool,
    pub process_enforced: bool,
    pub network_enforced: bool,
    pub version: Option<String>,
    pub stable_error_code: Option<String>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProcessStatus {
    #[default]
    Starting,
    Running,
    Exited,
    Terminated,
    Failed,
    Expired,
}

impl ProcessStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Exited => "exited",
            Self::Terminated => "terminated",
            Self::Failed => "failed",
            Self::Expired => "expired",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "starting" => Self::Starting,
            "running" => Self::Running,
            "exited" => Self::Exited,
            "terminated" => Self::Terminated,
            "failed" => Self::Failed,
            "expired" => Self::Expired,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProcessSessionRecord {
    pub id: ProcessSessionId,
    pub session_id: SessionId,
    pub run_id: Option<RunId>,
    pub checkout_id: CheckoutId,
    #[specta(type = Option<specta_typescript::Number>)]
    pub run_generation: Option<u64>,
    pub owner_client_id: ClientId,
    pub command_summary: String,
    pub interactive: bool,
    pub status: ProcessStatus,
    pub exit_code: Option<i32>,
    #[specta(type = specta_typescript::Number)]
    pub output_limit_bytes: u64,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
    #[specta(type = Option<specta_typescript::Number>)]
    pub reconnect_expires_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDelivery {
    Inline,
    Detached,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ReviewTarget {
    UncommittedChanges,
    BaseBranch(String),
    Commit(String),
    Custom(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSeverity {
    Info,
    Warning,
    Error,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ReviewFindingStatus {
    Open,
    Acknowledged,
    Resolved,
    Dismissed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRecord {
    pub id: ReviewId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub target: ReviewTarget,
    pub delivery: ReviewDelivery,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFinding {
    pub id: ReviewFindingId,
    pub review_id: ReviewId,
    pub severity: ReviewSeverity,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub message: String,
    pub evidence: String,
    pub status: ReviewFindingStatus,
}

#[cfg(test)]
#[path = "agent/tests.rs"]
mod tests;
