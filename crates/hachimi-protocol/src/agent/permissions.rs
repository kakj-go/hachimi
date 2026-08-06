use serde::{Deserialize, Serialize};
use specta::Type;

use super::{
    AuthoritySnapshotId, BrowserGrant, ComputerGrant, ConnectorAccountId, FileSystemGrant,
    McpServerId, NetworkGrant, PermissionProfile, ProcessGrant, RunId, SessionId,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpPermissionRule {
    pub server_id: McpServerId,
    pub tool_name: String,
    pub schema_hash: String,
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorPermissionRule {
    pub account_id: ConnectorAccountId,
    pub actions: Vec<String>,
    pub read_only_actions: Vec<String>,
    pub contribution_revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScopedPermissionRules {
    pub file_system: Vec<FileSystemGrant>,
    pub network: NetworkGrant,
    pub process: ProcessGrant,
    pub browser: BrowserGrant,
    pub computer: ComputerGrant,
    pub mcp: Vec<McpPermissionRule>,
    pub connectors: Vec<ConnectorPermissionRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentPermissionPolicy {
    pub level: PermissionProfile,
    pub rules: ScopedPermissionRules,
    #[specta(type = specta_typescript::Number)]
    pub revision: u64,
}

impl Default for AgentPermissionPolicy {
    fn default() -> Self {
        Self {
            level: PermissionProfile::ReadOnly,
            rules: ScopedPermissionRules::default(),
            revision: 0,
        }
    }
}

impl AgentPermissionPolicy {
    #[must_use]
    pub fn allows_mcp(
        &self,
        server_id: &McpServerId,
        tool_name: &str,
        schema_hash: &str,
        requires_write: bool,
    ) -> bool {
        self.level == PermissionProfile::FullAccess
            || (self.level != PermissionProfile::ReadOnly || !requires_write)
                && self.rules.mcp.iter().any(|rule| {
                    &rule.server_id == server_id
                        && rule.tool_name == tool_name
                        && rule.schema_hash == schema_hash
                        && (!requires_write || !rule.read_only)
                })
    }

    #[must_use]
    pub fn allows_connector(
        &self,
        account_id: &ConnectorAccountId,
        action: &str,
        requires_write: bool,
    ) -> bool {
        self.level == PermissionProfile::FullAccess
            || (self.level != PermissionProfile::ReadOnly || !requires_write)
                && self.rules.connectors.iter().any(|rule| {
                    &rule.account_id == account_id
                        && rule.actions.iter().any(|allowed| allowed == action)
                        && (!requires_write
                            || !rule
                                .read_only_actions
                                .iter()
                                .any(|read_only| read_only == action))
                })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityMode {
    Interactive,
    Unattended,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunAuthoritySnapshot {
    pub id: AuthoritySnapshotId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub policy: AgentPermissionPolicy,
    pub mode: AuthorityMode,
    pub source: String,
    pub workspace_root: String,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
}
