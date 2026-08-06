//! Durable active-Run recovery contracts.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::{
    MutationContext, RunId, RunRecoveryId, RunStatus, RunStepCheckpointId, SessionId,
    SideEffectExecutionId, ToolCallId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RunStepPhase {
    Sampling,
    ToolPrepared,
    ToolClaimed,
    ToolDispatched,
    ToolCompleted,
    ProjectionCommitted,
}

impl RunStepPhase {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sampling => "sampling",
            Self::ToolPrepared => "tool_prepared",
            Self::ToolClaimed => "tool_claimed",
            Self::ToolDispatched => "tool_dispatched",
            Self::ToolCompleted => "tool_completed",
            Self::ProjectionCommitted => "projection_committed",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "sampling" => Self::Sampling,
            "tool_prepared" => Self::ToolPrepared,
            "tool_claimed" => Self::ToolClaimed,
            "tool_dispatched" => Self::ToolDispatched,
            "tool_completed" => Self::ToolCompleted,
            "projection_committed" => Self::ProjectionCommitted,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolRecoveryPolicy {
    ReadOnlyReplayable,
    IdempotentWithReceipt,
    #[default]
    NonReplayable,
}

impl ToolRecoveryPolicy {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnlyReplayable => "read_only_replayable",
            Self::IdempotentWithReceipt => "idempotent_with_receipt",
            Self::NonReplayable => "non_replayable",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "read_only_replayable" => Self::ReadOnlyReplayable,
            "idempotent_with_receipt" => Self::IdempotentWithReceipt,
            "non_replayable" => Self::NonReplayable,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RunRecoveryState {
    EligibleAuto,
    AwaitingUser,
    Resuming,
    Resumed,
    Abandoned,
    Failed,
}

impl RunRecoveryState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EligibleAuto => "eligible_auto",
            Self::AwaitingUser => "awaiting_user",
            Self::Resuming => "resuming",
            Self::Resumed => "resumed",
            Self::Abandoned => "abandoned",
            Self::Failed => "failed",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "eligible_auto" => Self::EligibleAuto,
            "awaiting_user" => Self::AwaitingUser,
            "resuming" => Self::Resuming,
            "resumed" => Self::Resumed,
            "abandoned" => Self::Abandoned,
            "failed" => Self::Failed,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RunRecoveryDecisionAction {
    ResumeSafeRemainder,
    ConfirmEffectSucceeded,
    RetryIdempotentEffect,
    AbandonRun,
}

impl RunRecoveryDecisionAction {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ResumeSafeRemainder => "resume_safe_remainder",
            Self::ConfirmEffectSucceeded => "confirm_effect_succeeded",
            Self::RetryIdempotentEffect => "retry_idempotent_effect",
            Self::AbandonRun => "abandon_run",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "resume_safe_remainder" => Self::ResumeSafeRemainder,
            "confirm_effect_succeeded" => Self::ConfirmEffectSucceeded,
            "retry_idempotent_effect" => Self::RetryIdempotentEffect,
            "abandon_run" => Self::AbandonRun,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunStepCheckpoint {
    pub id: RunStepCheckpointId,
    pub session_id: SessionId,
    pub run_id: RunId,
    #[specta(type = specta_typescript::Number)]
    pub run_generation: u64,
    #[specta(type = specta_typescript::Number)]
    pub step_index: u64,
    pub phase: RunStepPhase,
    pub tool_call_id: Option<ToolCallId>,
    pub tool_name: Option<String>,
    pub side_effect_execution_id: Option<SideEffectExecutionId>,
    pub recovery_policy: ToolRecoveryPolicy,
    pub parameter_hash: Option<String>,
    pub world_revision: String,
    pub provider_revision: String,
    #[serde(default)]
    pub revision_snapshot: RecoveryRevisionSnapshot,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryRevisionSnapshot {
    pub agents_revision: String,
    pub skills_revision: String,
    pub mcp_revision: String,
    pub plugin_revision: String,
    pub host_revision: String,
    pub provider_revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunRecoveryRecord {
    pub id: RunRecoveryId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub previous_status: RunStatus,
    #[specta(type = specta_typescript::Number)]
    pub interrupted_generation: u64,
    #[specta(type = specta_typescript::Number)]
    pub resume_generation: u64,
    pub state: RunRecoveryState,
    pub reason_code: String,
    pub checkpoint_id: Option<RunStepCheckpointId>,
    pub side_effect_execution_id: Option<SideEffectExecutionId>,
    pub decision_action: Option<RunRecoveryDecisionAction>,
    pub decision_idempotency_key: Option<String>,
    pub resolved_by: Option<String>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub resolved_at_ms: Option<i64>,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunRecoveryDecisionRequest {
    pub context: MutationContext,
    pub recovery_id: RunRecoveryId,
    pub expected_run_id: RunId,
    #[specta(type = specta_typescript::Number)]
    pub expected_interrupted_generation: u64,
    pub action: RunRecoveryDecisionAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunRecoverySnapshot {
    pub recovery: RunRecoveryRecord,
    pub checkpoint: Option<RunStepCheckpoint>,
}
