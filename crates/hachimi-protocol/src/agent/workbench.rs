use serde::{Deserialize, Serialize};
use specta::Type;

use super::{
    AgentTaskRecord, ApprovalId, ApprovalRequestRecord, ApprovalStatus, ArtifactRecord,
    AttachmentId, BehaviorMode, CheckoutRecord, ExecutionTarget, PlanId, ProjectId, ProjectRecord,
    ProposedPlan, RunEventEnvelope, RunId, RunRecord, SessionRecord, SkillId, TranscriptItem,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitRefRecord {
    pub name: String,
    pub revision: String,
    pub remote: bool,
    pub current: bool,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkbenchSessionSnapshot {
    pub session: SessionRecord,
    pub runs: Vec<RunRecord>,
    pub events: Vec<RunEventEnvelope>,
    pub transcript: Vec<TranscriptItem>,
    pub pending_approvals: Vec<ApprovalRequestRecord>,
    pub proposed_plans: Vec<ProposedPlan>,
    pub artifacts: Vec<ArtifactRecord>,
    pub agent_tasks: Vec<AgentTaskRecord>,
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkbenchPlanAcceptanceSnapshot {
    pub plan: ProposedPlan,
    pub task: WorkbenchTaskSnapshot,
}
