//! Durable Multi-Agent task lineage and budget contracts.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::{AgentTaskId, AgentTaskMessageId, ArtifactId, RunBudget, RunId, SessionId, TokenUsage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentTaskStatus {
    #[default]
    Queued,
    Running,
    Waiting,
    NeedsAttention,
    Succeeded,
    Failed,
    Cancelled,
}

impl AgentTaskStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::NeedsAttention => "needs_attention",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "waiting" => Self::Waiting,
            "needs_attention" => Self::NeedsAttention,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }

    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskRecord {
    pub id: AgentTaskId,
    pub root_task_id: AgentTaskId,
    pub root_run_id: RunId,
    pub parent_task_id: Option<AgentTaskId>,
    pub parent_session_id: SessionId,
    pub parent_run_id: RunId,
    pub child_session_id: SessionId,
    pub child_run_id: RunId,
    pub title: String,
    pub depth: u8,
    pub status: AgentTaskStatus,
    pub reserved_budget: RunBudget,
    pub usage: TokenUsage,
    pub artifact_ids: Vec<ArtifactId>,
    pub result_summary: Option<String>,
    pub error_code: Option<String>,
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
pub struct AgentTaskMessageRecord {
    pub id: AgentTaskMessageId,
    pub task_id: AgentTaskId,
    pub sender_run_id: RunId,
    pub recipient_run_id: RunId,
    pub content: String,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = Option<specta_typescript::Number>)]
    pub delivered_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskCollection {
    pub tasks: Vec<AgentTaskRecord>,
    pub messages: Vec<AgentTaskMessageRecord>,
}
