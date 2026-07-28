//! Registry and narrow host clients for external capabilities.

mod mcp;
mod mcp_client;
mod mcp_echo;
mod mcp_elicitation;
mod mcp_http;
mod mcp_inventory;
mod mcp_media;
mod mcp_oauth;
mod mcp_progress;
mod mcp_supervisor;

use std::collections::BTreeMap;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

pub use hachimi_protocol::{
    McpPrompt, McpPromptArgument, McpPromptMessage, McpPromptResult, McpPromptRole, McpResource,
    McpResourceContent, McpResourceTemplate,
};
pub use mcp::{
    MCP_PROTOCOL_VERSION, McpCallResult, McpClientError, McpServerInfo, McpStdioClient,
    McpStdioServerConfig, McpToolDefinition, mcp_exposed_tool_name,
};
pub use mcp_client::McpClientHandle;
pub use mcp_echo::McpEchoServer;
pub use mcp_elicitation::{
    McpRunCorrelation, McpServerRequest, McpServerRequestFuture, McpServerRequestHandler,
    McpServerRequestId, McpServerRequestResponse,
};
pub use mcp_http::{McpHttpClient, McpHttpServerConfig};
pub use mcp_inventory::{McpResourcePage, McpResourceTemplatePage};
pub use mcp_media::{McpMediaError, McpMediaHost};
pub use mcp_oauth::{
    McpOAuthCredential, McpOAuthDiscovery, McpOAuthError, McpOAuthLoginHandle, discover_mcp_oauth,
    refresh_mcp_oauth_credential, start_mcp_oauth_login,
};
pub use mcp_progress::{McpProgressFuture, McpProgressHandler, McpProgressNotification};
pub use mcp_supervisor::{McpRuntimeSnapshot, McpStdioSandboxHost, McpSupervisor};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDescriptor {
    pub host_id: String,
    pub host_kind: String,
    pub commands: Vec<String>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CapabilityRegistryError {
    #[error("capability host descriptor is invalid")]
    InvalidDescriptor,
    #[error("capability host is already registered: {0}")]
    DuplicateHost(String),
}

#[derive(Debug, Default)]
pub struct CapabilityRegistry {
    hosts: RwLock<BTreeMap<String, CapabilityDescriptor>>,
}

impl CapabilityRegistry {
    #[must_use]
    pub fn list(&self) -> Vec<CapabilityDescriptor> {
        self.hosts.read().values().cloned().collect()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hosts.read().is_empty()
    }

    pub fn register(
        &self,
        descriptor: CapabilityDescriptor,
    ) -> Result<(), CapabilityRegistryError> {
        if descriptor.host_id.trim().is_empty()
            || descriptor.host_id.len() > 128
            || descriptor.host_kind.trim().is_empty()
            || descriptor.host_kind.len() > 64
            || descriptor.commands.len() > 1_024
            || descriptor
                .commands
                .iter()
                .any(|command| command.trim().is_empty() || command.len() > 256)
        {
            return Err(CapabilityRegistryError::InvalidDescriptor);
        }
        let mut hosts = self.hosts.write();
        if hosts.contains_key(&descriptor.host_id) {
            return Err(CapabilityRegistryError::DuplicateHost(descriptor.host_id));
        }
        hosts.insert(descriptor.host_id.clone(), descriptor);
        Ok(())
    }

    pub fn unregister(&self, host_id: &str) -> Option<CapabilityDescriptor> {
        self.hosts.write().remove(host_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_starts_without_high_permission_hosts() {
        assert!(CapabilityRegistry::default().is_empty());
    }

    #[test]
    fn registry_rejects_duplicates_and_lists_deterministically() {
        let registry = CapabilityRegistry::default();
        let descriptor = CapabilityDescriptor {
            host_id: "mcp:docs".into(),
            host_kind: "mcp_stdio".into(),
            commands: vec!["echo".into()],
        };
        registry.register(descriptor.clone()).expect("register");
        assert!(matches!(
            registry.register(descriptor.clone()),
            Err(CapabilityRegistryError::DuplicateHost(_))
        ));
        assert_eq!(registry.list(), vec![descriptor.clone()]);
        assert_eq!(registry.unregister("mcp:docs"), Some(descriptor));
        assert!(registry.is_empty());
    }
}
