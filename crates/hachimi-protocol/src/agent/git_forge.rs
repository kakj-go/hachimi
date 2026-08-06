//! Generic Git remote and Forge PR/MR contracts.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::{ApprovalId, CheckoutId, ForgeOperationId, MutationContext, RunId, SessionId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum ForgeKind {
    GitHub,
    GitLab,
    Gitee,
    GiteaForgejo,
    #[default]
    Unknown,
}

impl ForgeKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GitHub => "github",
            Self::GitLab => "gitlab",
            Self::Gitee => "gitee",
            Self::GiteaForgejo => "gitea_forgejo",
            Self::Unknown => "unknown",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "github" => Self::GitHub,
            "gitlab" => Self::GitLab,
            "gitee" => Self::Gitee,
            "gitea_forgejo" => Self::GiteaForgejo,
            "unknown" => Self::Unknown,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitRemoteRecord {
    pub name: String,
    /// Redacted display URL. Embedded credentials are never returned.
    pub display_url: String,
    pub remote_url_hash: String,
    pub forge_kind: ForgeKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitRemoteListRequest {
    pub session_id: SessionId,
    pub checkout_id: CheckoutId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitPushRequest {
    pub context: MutationContext,
    pub session_id: SessionId,
    pub checkout_id: CheckoutId,
    pub remote_name: String,
    pub expected_remote_url_hash: String,
    pub source_ref: String,
    pub target_ref: String,
    pub expected_commit_oid: String,
    pub approval_id: Option<ApprovalId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitPushResponse {
    pub remote_name: String,
    pub remote_url_hash: String,
    pub source_ref: String,
    pub target_ref: String,
    pub commit_oid: String,
    pub confirmed: bool,
    pub result_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ForgeRepositoryIdentity {
    pub forge_kind: ForgeKind,
    pub api_base_url: String,
    pub owner: String,
    pub repository: String,
    pub remote_url_hash: String,
    /// Opaque Credential Manager account/reference, never secret material.
    pub secret_ref: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ForgeChangeState {
    Open,
    Closed,
    Merged,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ForgeChangeRecord {
    pub forge_kind: ForgeKind,
    #[specta(type = specta_typescript::Number)]
    pub number: u64,
    pub title: String,
    pub body: String,
    pub source_ref: String,
    pub target_ref: String,
    pub source_commit_oid: Option<String>,
    pub state: ForgeChangeState,
    pub web_url: Option<String>,
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ForgeChangeMutation {
    Create {
        title: String,
        body: String,
        source_ref: String,
        target_ref: String,
    },
    Update {
        #[specta(type = specta_typescript::Number)]
        number: u64,
        title: String,
        body: String,
        source_ref: String,
        target_ref: String,
    },
    Close {
        #[specta(type = specta_typescript::Number)]
        number: u64,
    },
    Merge {
        #[specta(type = specta_typescript::Number)]
        number: u64,
        merge_title: Option<String>,
        merge_message: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ForgeChangeQueryRequest {
    pub repository: ForgeRepositoryIdentity,
    #[specta(type = specta_typescript::Number)]
    pub number: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ForgeChangeMutationRequest {
    pub context: MutationContext,
    pub session_id: SessionId,
    pub checkout_id: CheckoutId,
    pub repository: ForgeRepositoryIdentity,
    pub mutation: ForgeChangeMutation,
    pub expected_revision: Option<String>,
    pub expected_commit_oid: String,
    /// Mandatory and freshly resolved for merge; optional for lower-risk mutations.
    pub approval_id: Option<ApprovalId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ForgeCredentialUpdateRequest {
    pub secret_ref: String,
    /// `None` clears the Credential Manager entry.
    pub secret: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ForgeCredentialState {
    pub secret_ref: String,
    pub configured: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ForgeOperationStatus {
    Claimed,
    Dispatched,
    Confirmed,
    Failed,
    Indeterminate,
}

impl ForgeOperationStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claimed => "claimed",
            Self::Dispatched => "dispatched",
            Self::Confirmed => "confirmed",
            Self::Failed => "failed",
            Self::Indeterminate => "indeterminate",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "claimed" => Self::Claimed,
            "dispatched" => Self::Dispatched,
            "confirmed" => Self::Confirmed,
            "failed" => Self::Failed,
            "indeterminate" => Self::Indeterminate,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ForgeOperationRecord {
    pub id: ForgeOperationId,
    pub session_id: SessionId,
    pub run_id: Option<RunId>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub run_generation: Option<u64>,
    pub operation_kind: String,
    pub repository: ForgeRepositoryIdentity,
    pub source_ref: Option<String>,
    pub target_ref: Option<String>,
    pub commit_oid: String,
    pub expected_revision: Option<String>,
    pub approval_id: Option<ApprovalId>,
    pub idempotency_key: String,
    pub request_hash: String,
    pub status: ForgeOperationStatus,
    pub result: Option<ForgeChangeRecord>,
    pub error_code: Option<String>,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}
