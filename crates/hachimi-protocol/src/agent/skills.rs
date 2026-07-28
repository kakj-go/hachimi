use serde::{Deserialize, Serialize};
use specta::Type;

use super::{SkillActivationId, SkillId, WorkloadKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SkillActivationSource {
    ExplicitSelection,
    Mention,
    ModelRead,
    BuiltInDiscovery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillClassification {
    pub skill_id: SkillId,
    pub content_revision: String,
    pub workload: WorkloadKind,
    pub confidence_basis_points: u16,
    pub reason: String,
    pub classifier_revision: String,
    #[specta(type = specta_typescript::Number)]
    pub classified_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillActivation {
    pub id: SkillActivationId,
    pub skill_id: SkillId,
    pub content_revision: String,
    pub source: SkillActivationSource,
    #[specta(type = specta_typescript::Number)]
    pub activated_at_step_revision: u64,
    pub classified_workload: WorkloadKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SkillDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillDiagnostic {
    pub code: String,
    pub message: String,
    pub relative_path: Option<String>,
    pub severity: SkillDiagnosticSeverity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SkillEntryKind {
    File,
    Directory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SkillEditorKind {
    Markdown,
    Text,
    Unsupported,
}

/// Host-owned discovery scope for a Skill. Ordering is resolved by the Skill
/// catalog and is not an authorization grant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SkillScope {
    BuiltIn,
    #[default]
    User,
    Repo,
    System,
    Admin,
}

/// Optional tool dependency declared by `agents/openai.yaml` next to a Skill.
/// Dependency metadata is diagnostic only and never registers or authorizes a tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillToolDependency {
    #[serde(rename = "type")]
    pub kind: String,
    pub value: String,
    pub description: Option<String>,
    pub transport: Option<String>,
    pub command: Option<String>,
    pub url: Option<String>,
}

/// Optional presentation metadata loaded from `agents/openai.yaml`.
/// Paths remain Skill-relative references and never grant host filesystem access.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillInterface {
    pub display_name: Option<String>,
    pub short_description: Option<String>,
    pub icon_small: Option<String>,
    pub icon_large: Option<String>,
    pub brand_color: Option<String>,
    pub default_prompt: Option<String>,
}

/// Invocation policy declared by a Skill package. This only narrows discovery;
/// it cannot register a tool or create an authorization grant.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillPolicy {
    pub allow_implicit_invocation: Option<bool>,
    /// Classification hint only. It is trusted for Built-in Skills; User/Repo
    /// Skills still require the bounded structured classifier.
    pub workload: Option<WorkloadKind>,
}

impl SkillPolicy {
    #[must_use]
    pub fn allows_implicit_invocation(&self) -> bool {
        self.allow_implicit_invocation.unwrap_or(true)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillTreeNode {
    pub name: String,
    pub relative_path: String,
    pub kind: SkillEntryKind,
    pub editor_kind: SkillEditorKind,
    #[specta(type = specta_typescript::Number)]
    pub size_bytes: u64,
    pub revision: Option<String>,
    pub children: Vec<SkillTreeNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillRecord {
    pub id: SkillId,
    pub scope: SkillScope,
    pub namespace: Option<String>,
    pub name: String,
    pub qualified_name: String,
    pub description: String,
    #[serde(default)]
    pub interface: Option<SkillInterface>,
    #[serde(default)]
    pub policy: SkillPolicy,
    pub dependencies: Vec<SkillToolDependency>,
    pub editable: bool,
    pub enabled: bool,
    pub content_hash: String,
    pub tree_revision: String,
    pub diagnostics: Vec<SkillDiagnostic>,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillFileSnapshot {
    pub skill_id: SkillId,
    pub relative_path: String,
    pub editor_kind: SkillEditorKind,
    pub content: Option<String>,
    #[specta(type = specta_typescript::Number)]
    pub size_bytes: u64,
    pub revision: String,
    pub diagnostics: Vec<SkillDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillFileWriteRequest {
    pub skill_id: SkillId,
    pub relative_path: String,
    pub content: String,
    pub expected_revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillPreviewResourceRequest {
    pub skill_id: SkillId,
    pub source_path: String,
    pub destination: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillPreviewResource {
    pub skill_id: SkillId,
    pub source_path: String,
    pub relative_path: String,
    pub editor_kind: SkillEditorKind,
    pub text: Option<String>,
    pub media_type: Option<String>,
    pub data_base64: Option<String>,
    #[specta(type = specta_typescript::Number)]
    pub size_bytes: u64,
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillEntryCreateRequest {
    pub skill_id: SkillId,
    pub parent_path: String,
    pub name: String,
    pub kind: SkillEntryKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillEntryRenameRequest {
    pub skill_id: SkillId,
    pub relative_path: String,
    pub new_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SkillChangeKind {
    Created,
    Updated,
    Renamed,
    Removed,
    Reindexed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillChangeEvent {
    pub skill_id: SkillId,
    pub relative_paths: Vec<String>,
    pub kind: SkillChangeKind,
    pub tree_revision: String,
}
