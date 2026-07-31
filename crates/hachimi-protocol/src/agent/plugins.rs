//! Local Plugin Bundle and Connector Host contracts.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use super::{ConnectorAccountId, PluginId};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "snake_case")]
pub enum PluginHookEvent {
    #[serde(rename = "run.before")]
    RunBefore,
    #[serde(rename = "run.after")]
    RunAfter,
    #[serde(rename = "tool.before")]
    ToolBefore,
    #[serde(rename = "tool.after")]
    ToolAfter,
    #[serde(rename = "schedule.before")]
    ScheduleBefore,
    #[serde(rename = "schedule.after")]
    ScheduleAfter,
}

impl PluginHookEvent {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RunBefore => "run.before",
            Self::RunAfter => "run.after",
            Self::ToolBefore => "tool.before",
            Self::ToolAfter => "tool.after",
            Self::ScheduleBefore => "schedule.before",
            Self::ScheduleAfter => "schedule.after",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PluginHookRuntimeKind {
    SandboxedStdioJsonRpc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginHookDescriptor {
    pub protocol_version: u32,
    pub runtime: PluginHookRuntimeKind,
    pub entrypoint: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub events: Vec<PluginHookEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginHookInvocation {
    pub event: PluginHookEvent,
    pub session_id: Option<super::SessionId>,
    pub run_id: Option<super::RunId>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub run_generation: Option<u64>,
    pub subject_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginHookMetadataEntry {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginHookOutcome {
    pub result_code: String,
    #[serde(default)]
    pub metadata: Vec<PluginHookMetadataEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PluginUiBridgeMethod {
    GetContext,
    ResolveAssetUrl,
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginContributionSurface {
    pub plugin_id: PluginId,
    pub contribution_id: String,
    pub kind: PluginContributionKind,
    pub runtime_revision: String,
    pub runtime_state: ContributionRuntimeState,
    pub diagnostic: Option<String>,
    pub last_result_code: Option<String>,
    pub entry_url: Option<String>,
    pub asset_base_url: Option<String>,
    pub allowed_bridge_methods: Vec<PluginUiBridgeMethod>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginUiContext {
    pub plugin_id: PluginId,
    pub contribution_id: String,
    pub runtime_revision: String,
    pub locale: String,
    pub theme: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum PluginUiBridgeRequest {
    GetContext {
        request_id: String,
    },
    ResolveAssetUrl {
        request_id: String,
        asset_contribution_id: String,
        relative_path: String,
    },
    Close {
        request_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PluginUiBridgeResponse {
    Context {
        request_id: String,
        value: PluginUiContext,
    },
    AssetUrl {
        request_id: String,
        value: String,
    },
    Closed {
        request_id: String,
    },
    Error {
        request_id: String,
        code: String,
    },
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "snake_case")]
pub enum PluginContributionKind {
    Skill,
    Hook,
    EventSource,
    Mcp,
    Connector,
    BrowserExtension,
    ScheduledTaskTemplate,
    Asset,
    CustomUi,
    Channel,
}

impl PluginContributionKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Skill => "skill",
            Self::Hook => "hook",
            Self::EventSource => "event_source",
            Self::Mcp => "mcp",
            Self::Connector => "connector",
            Self::BrowserExtension => "browser_extension",
            Self::ScheduledTaskTemplate => "scheduled_task_template",
            Self::Asset => "asset",
            Self::CustomUi => "custom_ui",
            Self::Channel => "channel",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginContribution {
    pub kind: PluginContributionKind,
    pub id: String,
    pub relative_path: String,
    #[serde(default)]
    pub required_scopes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ContributionRuntimeState {
    Staged,
    Registered,
    Starting,
    Active,
    Degraded,
    Failed,
    Disabled,
    Stopping,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PluginRevisionStatus {
    Staged,
    Validated,
    Activating,
    Healthy,
    Failed,
    Superseded,
    Removed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PluginLifecycleOperation {
    Install,
    Update,
    Enable,
    Disable,
    Rollback,
    Uninstall,
    Reconcile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PluginLifecyclePhase {
    Stage,
    Validate,
    PermissionReview,
    Activate,
    HealthCheck,
    Commit,
    Rollback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PluginLifecycleJournalStatus {
    InProgress,
    Committed,
    RolledBack,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginRevisionRecord {
    pub plugin_id: PluginId,
    pub revision: String,
    pub manifest: PluginManifest,
    pub content_hash: String,
    pub root_path: String,
    pub plugin_status: PluginStatus,
    pub status: PluginRevisionStatus,
    pub health_code: Option<String>,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginRevisionHead {
    pub plugin_id: PluginId,
    pub current_revision: String,
    pub known_good_revision: Option<String>,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginLifecycleJournalRecord {
    pub id: String,
    pub plugin_id: PluginId,
    pub operation: PluginLifecycleOperation,
    pub phase: PluginLifecyclePhase,
    pub status: PluginLifecycleJournalStatus,
    pub source_revision: Option<String>,
    pub candidate_revision: Option<String>,
    pub error_code: Option<String>,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InstalledContribution {
    pub plugin_id: PluginId,
    pub contribution_id: String,
    pub kind: PluginContributionKind,
    pub content_hash: String,
    pub runtime_revision: String,
    pub state: ContributionRuntimeState,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginPermissionDiff {
    pub plugin_id: PluginId,
    pub previous_scopes: Vec<String>,
    pub requested_scopes: Vec<String>,
    pub added_scopes: Vec<String>,
    pub requires_confirmation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    pub manifest_version: u32,
    pub id: PluginId,
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(default)]
    pub contributions: Vec<PluginContribution>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PluginStatus {
    Disabled,
    Enabled,
    NeedsAttention,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPlugin {
    pub manifest: PluginManifest,
    pub content_hash: String,
    pub root_path: String,
    pub status: PluginStatus,
    pub diagnostics: Vec<String>,
    #[specta(type = specta_typescript::Number)]
    pub installed_at_ms: i64,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorHealth {
    Healthy,
    Revoked,
    SchemaDrift,
    HostIdentityDrift,
    ActionDrift,
    RateLimited,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorRevision {
    pub host_identity_hash: String,
    pub schema_hash: String,
    pub action_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorRuntimeKind {
    Builtin,
    SandboxedStdioJsonRpc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorDriverDescriptor {
    pub plugin_id: PluginId,
    pub connector_id: String,
    pub runtime_kind: ConnectorRuntimeKind,
    pub revision: ConnectorRevision,
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorAccountUpsert {
    pub id: ConnectorAccountId,
    pub plugin_id: PluginId,
    pub connector_id: String,
    pub display_name: String,
    pub secret: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorAccount {
    pub id: ConnectorAccountId,
    pub plugin_id: PluginId,
    pub connector_id: String,
    pub display_name: String,
    pub secret_ref: Option<String>,
    pub revision: ConnectorRevision,
    pub health: ConnectorHealth,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorInvocationRequest {
    pub account_id: ConnectorAccountId,
    pub action: String,
    #[specta(type = specta_typescript::Unknown)]
    pub arguments: Value,
    pub idempotency_key: String,
    pub expected_revision: ConnectorRevision,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorInvocationResult {
    pub account_id: ConnectorAccountId,
    pub action: String,
    #[specta(type = specta_typescript::Unknown)]
    pub result: Value,
    #[specta(type = specta_typescript::Unknown)]
    pub metadata: Value,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ContributionRevision {
    pub plugin_id: PluginId,
    pub contribution_id: String,
    #[serde(default)]
    pub account_id: Option<ConnectorAccountId>,
    pub content_hash: String,
    pub host_identity_hash: Option<String>,
    pub schema_hash: Option<String>,
    #[serde(default)]
    pub action_hash: Option<String>,
}
