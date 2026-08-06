use serde::{Deserialize, Serialize};
use specta::Type;

use super::{ScheduleId, SessionId, WorkspaceId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AgentWorkspaceKind {
    Managed,
    SelectedDirectory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentWorkspaceOwner {
    Session { session_id: SessionId },
    Schedule { schedule_id: ScheduleId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentWorkspaceStatus {
    #[default]
    Ready,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentWorkspace {
    pub id: WorkspaceId,
    pub kind: AgentWorkspaceKind,
    pub owner: AgentWorkspaceOwner,
    pub root_path: String,
    pub status: AgentWorkspaceStatus,
    pub status_reason: Option<String>,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScheduleWorkspaceSpec {
    #[default]
    Managed,
    SelectedDirectory {
        root_path: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleConversationMode {
    SharedSession,
    #[default]
    PerRunSession,
}
