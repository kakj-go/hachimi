use serde::{Deserialize, Serialize};
use specta::Type;

use super::{
    AgentTaskRecord, ApprovalId, ApprovalRequestRecord, ApprovalStatus, ArtifactId, ArtifactRecord,
    AttachmentId, BehaviorMode, BrowserAutomationLease, BrowserAutomationLeaseId,
    BrowserAutomationSurfaceKind, BrowserSession, BrowserSessionId, BrowserTabId, CheckoutId,
    CheckoutKind, CheckoutRecord, ComputerControlSession, ComputerControlSessionId,
    ExecutionTarget, ExternalBrowserLeaseObservation, GitRemoteRecord, HostAccessRequestRecord,
    MutationContext, PlanId, PlanStepId, ProjectId, ProjectRecord, ProposedPlan, RunEventEnvelope,
    RunId, RunRecord, RunStatus, SessionRecord, SessionSourceId, SkillId, TranscriptItem,
};
use crate::FileDiffStatus;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitRefRecord {
    pub name: String,
    pub revision: String,
    pub remote: bool,
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkbenchGitAction {
    Commit { message: Option<String> },
    SwitchBranch { branch: String, remote: bool },
    CreateBranch { branch: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkbenchGitRequest {
    pub context: MutationContext,
    pub idempotency_key: String,
    pub session_id: super::SessionId,
    pub checkout_id: CheckoutId,
    pub expected_head: Option<String>,
    pub status_fingerprint: String,
    pub include_unstaged: bool,
    pub action: WorkbenchGitAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum WorkbenchGitPhaseStatus {
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkbenchGitPhaseResult {
    pub status: WorkbenchGitPhaseStatus,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkbenchGitResponse {
    pub stage: WorkbenchGitPhaseResult,
    pub commit: WorkbenchGitPhaseResult,
    pub head: Option<String>,
    pub status_fingerprint: String,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SessionSourceKind {
    Upload,
    Web,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SessionSourceOrigin {
    Upload,
    Browser,
    Mcp,
    Connector,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionSourceRecord {
    pub id: SessionSourceId,
    pub session_id: super::SessionId,
    pub run_id: Option<RunId>,
    pub kind: SessionSourceKind,
    pub origin: SessionSourceOrigin,
    pub attachment_id: Option<AttachmentId>,
    pub url: Option<String>,
    pub title: Option<String>,
    pub browser_tab_id: Option<BrowserTabId>,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = specta_typescript::Number)]
    pub last_used_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentChangeSummary {
    pub changed_files: u32,
    pub additions: u32,
    pub deletions: u32,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentGitSummary {
    pub branch: Option<String>,
    pub head_sha: Option<String>,
    pub detached: bool,
    pub status_fingerprint: String,
    pub uncommitted_files: u32,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub default_comparison_ref: Option<String>,
    pub refs: Vec<GitRefRecord>,
    pub remotes: Vec<GitRemoteRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EnvironmentActivity {
    Browser {
        lease_id: BrowserAutomationLeaseId,
        surface: BrowserAutomationSurfaceKind,
        browser_tab_id: Option<BrowserTabId>,
        browser_session_id: Option<BrowserSessionId>,
        run_id: RunId,
        domain: String,
    },
    Computer {
        control_session_id: ComputerControlSessionId,
        run_id: Option<RunId>,
        app_id: String,
        app_name: String,
    },
    Plan {
        plan_id: PlanId,
        step_id: PlanStepId,
        description: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentHandoffState {
    pub local_checkout_id: Option<CheckoutId>,
    pub managed_checkout_id: Option<CheckoutId>,
    pub can_handoff: bool,
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkbenchEnvironmentSnapshot {
    pub session_id: super::SessionId,
    pub checkout: CheckoutRecord,
    #[specta(type = specta_typescript::Number)]
    pub binding_revision: u64,
    pub baseline_revision: Option<String>,
    pub changes: EnvironmentChangeSummary,
    pub git: EnvironmentGitSummary,
    pub handoff: EnvironmentHandoffState,
    pub activity: Option<EnvironmentActivity>,
    pub sources: Vec<SessionSourceRecord>,
    #[specta(type = specta_typescript::Number)]
    pub revision: u64,
    #[specta(type = specta_typescript::Number)]
    pub generated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum WorkbenchEnvironmentChangeReason {
    Files,
    Git,
    Binding,
    Plan,
    Browser,
    Sources,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkbenchEnvironmentChanged {
    pub session_id: super::SessionId,
    #[specta(type = specta_typescript::Number)]
    pub revision: u64,
    pub reasons: Vec<WorkbenchEnvironmentChangeReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkbenchHandoffRequest {
    pub idempotency_key: String,
    pub session_id: super::SessionId,
    pub source_checkout_id: CheckoutId,
    pub target_kind: CheckoutKind,
    pub expected_head: Option<String>,
    pub status_fingerprint: String,
    #[specta(type = specta_typescript::Number)]
    pub expected_binding_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkbenchHandoffResponse {
    pub session: SessionRecord,
    pub checkout: CheckoutRecord,
    pub environment: WorkbenchEnvironmentSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkbenchTaskStartRequest {
    pub idempotency_key: String,
    #[serde(default)]
    pub entry_profile: super::EntryProfile,
    pub session_id: Option<super::SessionId>,
    pub project_id: Option<ProjectId>,
    pub prompt: String,
    pub execution_target: Option<ExecutionTarget>,
    pub behavior_mode: BehaviorMode,
    pub approval_policy: super::ApprovalPolicy,
    pub attachment_ids: Vec<AttachmentId>,
    /// Explicit Skill identities selected by the user. This is authoritative
    /// when names collide; `$name` remains only an unambiguous text shortcut.
    #[serde(default)]
    pub skill_ids: Vec<SkillId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkbenchTaskSnapshot {
    pub project: Option<ProjectRecord>,
    pub checkout: Option<CheckoutRecord>,
    pub session: SessionRecord,
    pub run: RunRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkbenchAttachmentPreview {
    pub attachment: super::AttachmentRecord,
    pub utf8_text: Option<String>,
    pub data_url: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionRunActivity {
    pub id: RunId,
    pub status: RunStatus,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkbenchSessionListItem {
    pub session: SessionRecord,
    pub latest_run: Option<SessionRunActivity>,
    pub latest_terminal_run: Option<SessionRunActivity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunSummaryFile {
    pub path: String,
    pub previous_path: Option<String>,
    pub status: FileDiffStatus,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunSummaryRecord {
    pub run_id: RunId,
    pub status: RunStatus,
    pub changed_files: u32,
    pub additions: u32,
    pub deletions: u32,
    pub files: Vec<RunSummaryFile>,
    pub diff_artifact_id: Option<ArtifactId>,
    pub diff_unavailable: bool,
    #[specta(type = specta_typescript::Number)]
    pub completed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkbenchSessionSnapshot {
    pub session: SessionRecord,
    pub checkout: Option<CheckoutRecord>,
    pub runs: Vec<RunRecord>,
    pub events: Vec<RunEventEnvelope>,
    pub transcript: Vec<TranscriptItem>,
    pub attachments: Vec<super::AttachmentRecord>,
    pub pending_approvals: Vec<ApprovalRequestRecord>,
    pub proposed_plans: Vec<ProposedPlan>,
    pub artifacts: Vec<ArtifactRecord>,
    pub agent_tasks: Vec<AgentTaskRecord>,
    pub run_summaries: Vec<RunSummaryRecord>,
    pub browser_sessions: Vec<BrowserSession>,
    pub browser_automation_leases: Vec<BrowserAutomationLease>,
    pub external_browser_observations: Vec<ExternalBrowserLeaseObservation>,
    pub host_access_requests: Vec<HostAccessRequestRecord>,
    pub computer_control_sessions: Vec<ComputerControlSession>,
    pub sources: Vec<SessionSourceRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalDecisionRequest {
    pub approval_id: ApprovalId,
    pub decision: ApprovalStatus,
    pub expected_run_id: RunId,
    #[specta(type = specta_typescript::Number)]
    pub expected_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlanAcceptanceRequest {
    pub idempotency_key: String,
    pub plan_id: PlanId,
    pub expected_revision: u32,
    /// Locale-aware text shown as the user's confirmation in durable history.
    pub user_message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlanRevisionRequest {
    pub idempotency_key: String,
    pub plan_id: PlanId,
    pub expected_revision: u32,
    pub instructions: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkbenchPlanAcceptanceSnapshot {
    pub plan: ProposedPlan,
    pub task: WorkbenchTaskSnapshot,
}
