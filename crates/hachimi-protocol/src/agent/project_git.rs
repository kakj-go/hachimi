use serde::{Deserialize, Serialize};
use specta::Type;

use super::{MutationContext, ProjectId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProjectGitState {
    NotRepository,
    Unborn {
        branch: String,
    },
    Ready {
        branch: Option<String>,
        head: String,
    },
    Detached {
        head: String,
    },
    Unavailable {
        error_code: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectGitSnapshot {
    pub project_id: ProjectId,
    pub git_root: Option<String>,
    pub state: ProjectGitState,
    #[specta(type = specta_typescript::Number)]
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectGitInitialCommitRequest {
    pub context: MutationContext,
    pub project_id: ProjectId,
    pub author_name: String,
    pub author_email: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectGitInitialCommitResponse {
    pub snapshot: ProjectGitSnapshot,
    pub commit_sha: String,
}
