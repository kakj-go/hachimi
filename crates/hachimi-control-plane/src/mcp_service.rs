//! Persistence/runtime bridge for MCP configuration and per-tool exposure policy.

use std::{collections::BTreeMap, fmt, sync::Arc};

use hachimi_capabilities::{
    CapabilityDescriptor, CapabilityRegistry, CapabilityRegistryError, McpClientError,
    McpClientHandle, McpOAuthCredential, McpOAuthError, McpOAuthLoginHandle, McpRuntimeSnapshot,
    McpSupervisor, McpToolDefinition, discover_mcp_oauth, mcp_exposed_tool_name,
    refresh_mcp_oauth_credential, start_mcp_oauth_login,
};
use hachimi_protocol::{
    McpAuthStatus, McpAuthStatusRecord, McpCallSummaryListRequest, McpCallSummaryRecord,
    McpConnectionTestResult, McpInventorySnapshot, McpOAuthLoginRequest, McpOAuthLoginResponse,
    McpPromptGetRequest, McpPromptResult, McpResourceContent, McpResourceReadRequest,
    McpServerHealthRecord, McpServerHealthState, McpServerId, McpServerRecord, McpServerTransport,
    McpServerView, McpToolView,
};
use hachimi_storage::{AgentStore, AgentStoreError};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub trait McpSecretResolver: Send + Sync {
    fn resolve(&self, credential_reference: &str) -> Result<Option<String>, String>;
    fn persist(&self, credential_reference: &str, value: &str) -> Result<(), String>;
    fn delete(&self, credential_reference: &str) -> Result<(), String>;
}

#[derive(Debug, Default)]
pub struct EmptyMcpSecretResolver;

impl McpSecretResolver for EmptyMcpSecretResolver {
    fn resolve(&self, _credential_reference: &str) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn persist(&self, _credential_reference: &str, _value: &str) -> Result<(), String> {
        Err("MCP secret persistence is unavailable".into())
    }

    fn delete(&self, _credential_reference: &str) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum McpControlServiceError {
    #[error(transparent)]
    Store(#[from] AgentStoreError),
    #[error(transparent)]
    Registry(#[from] CapabilityRegistryError),
    #[error("MCP credential is unavailable")]
    CredentialUnavailable,
    #[error("MCP runtime is not ready")]
    RuntimeUnavailable,
    #[error("MCP OAuth conflicts with an explicit Authorization header")]
    AuthenticationConflict,
    #[error(transparent)]
    OAuth(#[from] McpOAuthError),
    #[error(transparent)]
    Client(#[from] McpClientError),
}

#[derive(Debug, Clone)]
pub struct McpReadyRuntime {
    pub configuration: McpServerRecord,
    pub client: Arc<McpClientHandle>,
    pub tools: Vec<McpToolDefinition>,
}

#[derive(Debug, Clone, Default)]
pub struct McpReconciliationReport {
    pub views: Vec<McpServerView>,
    pub failures: u32,
}

#[derive(Clone)]
pub struct McpControlService {
    store: AgentStore,
    supervisor: Arc<McpSupervisor>,
    capabilities: Arc<CapabilityRegistry>,
    secrets: Arc<dyn McpSecretResolver>,
    oauth_refresh: Arc<tokio::sync::Mutex<()>>,
}

impl fmt::Debug for McpControlService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpControlService")
            .finish_non_exhaustive()
    }
}

impl McpControlService {
    #[must_use]
    pub fn new(
        store: AgentStore,
        supervisor: Arc<McpSupervisor>,
        capabilities: Arc<CapabilityRegistry>,
    ) -> Self {
        Self::with_secret_resolver(
            store,
            supervisor,
            capabilities,
            Arc::new(EmptyMcpSecretResolver),
        )
    }

    #[must_use]
    pub fn with_secret_resolver(
        store: AgentStore,
        supervisor: Arc<McpSupervisor>,
        capabilities: Arc<CapabilityRegistry>,
        secrets: Arc<dyn McpSecretResolver>,
    ) -> Self {
        Self {
            store,
            supervisor,
            capabilities,
            secrets,
            oauth_refresh: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    #[must_use]
    pub fn with_default_runtime(store: AgentStore, capabilities: Arc<CapabilityRegistry>) -> Self {
        Self::new(store, Arc::new(McpSupervisor::default()), capabilities)
    }

    #[must_use]
    pub fn with_default_runtime_and_secret_resolver(
        store: AgentStore,
        capabilities: Arc<CapabilityRegistry>,
        secrets: Arc<dyn McpSecretResolver>,
    ) -> Self {
        Self::with_secret_resolver(
            store,
            Arc::new(McpSupervisor::default()),
            capabilities,
            secrets,
        )
    }

    pub async fn upsert(
        &self,
        server: &McpServerRecord,
    ) -> Result<McpServerView, McpControlServiceError> {
        let configuration = self.store.upsert_mcp_server(server).await?;
        let health = self.apply_runtime(&configuration).await?;
        Ok(McpServerView {
            configuration,
            health,
        })
    }

    pub async fn set_enabled(
        &self,
        server_id: &McpServerId,
        enabled: bool,
        updated_at_ms: i64,
    ) -> Result<McpServerView, McpControlServiceError> {
        let mut server = self.server(server_id).await?;
        server.enabled = enabled;
        server.updated_at_ms = updated_at_ms;
        self.upsert(&server).await
    }

    pub async fn start(
        &self,
        server_id: &McpServerId,
    ) -> Result<McpServerView, McpControlServiceError> {
        let server = self.server(server_id).await?;
        let health = self.apply_runtime(&server).await?;
        Ok(McpServerView {
            configuration: server,
            health,
        })
    }

    pub async fn stop(
        &self,
        server_id: &McpServerId,
    ) -> Result<McpServerHealthRecord, McpControlServiceError> {
        let server = self.server(server_id).await?;
        self.capabilities.unregister(&host_id(server_id));
        let snapshot = if server.enabled {
            self.supervisor.stop(server_id).await
        } else {
            self.supervisor.apply(&server).await
        };
        Ok(self.store.set_mcp_server_health(&snapshot.health).await?)
    }

    pub async fn refresh_health(
        &self,
        server_id: &McpServerId,
    ) -> Result<McpServerHealthRecord, McpControlServiceError> {
        let snapshot = self.supervisor.refresh_health(server_id).await;
        let health = if let Some(snapshot) = snapshot {
            if snapshot.health.state != McpServerHealthState::Ready {
                self.capabilities.unregister(&host_id(server_id));
            }
            snapshot.health
        } else {
            let mut health = self
                .store
                .get_mcp_server_health(server_id)
                .await?
                .ok_or_else(|| AgentStoreError::McpServerNotFound(server_id.clone()))?;
            health.state = McpServerHealthState::Stopped;
            health.server_name = None;
            health.server_version = None;
            health.protocol_version = None;
            health.tool_count = 0;
            health.error_code = Some("runtime_not_loaded".into());
            health
        };
        Ok(self.store.set_mcp_server_health(&health).await?)
    }

    pub async fn remove(&self, server_id: &McpServerId) -> Result<bool, McpControlServiceError> {
        let auth_reference = self.store.get_mcp_auth_reference(server_id).await?;
        self.capabilities.unregister(&host_id(server_id));
        self.supervisor.remove(server_id).await;
        let removed = self.store.remove_mcp_server(server_id).await?;
        if removed && let Some(reference) = auth_reference {
            self.delete_or_defer_secret(&reference).await;
        }
        Ok(removed)
    }

    pub async fn auth_status(
        &self,
        server_id: &McpServerId,
    ) -> Result<McpAuthStatusRecord, McpControlServiceError> {
        let server = self.server(server_id).await?;
        let McpServerTransport::StreamableHttp { url } = &server.transport else {
            return Ok(McpAuthStatusRecord {
                server_id: server_id.clone(),
                status: McpAuthStatus::Unsupported,
                scopes_supported: Vec::new(),
            });
        };
        if has_authorization_header(&server) {
            return Ok(McpAuthStatusRecord {
                server_id: server_id.clone(),
                status: McpAuthStatus::BearerToken,
                scopes_supported: Vec::new(),
            });
        }
        if let Some(reference) = self.store.get_mcp_auth_reference(server_id).await?
            && let Some(secret) = self
                .secrets
                .resolve(&reference)
                .map_err(|_| McpControlServiceError::CredentialUnavailable)?
            && McpOAuthCredential::from_secret_json(&secret).is_ok_and(|credential| {
                credential.server_url() == url
                    && credential.can_authorize(u64::try_from(now_ms()).unwrap_or(u64::MAX))
            })
        {
            let scopes_supported = discover_mcp_oauth(url)
                .await
                .unwrap_or(None)
                .map(|discovery| discovery.scopes_supported().to_vec())
                .unwrap_or_default();
            return Ok(McpAuthStatusRecord {
                server_id: server_id.clone(),
                status: McpAuthStatus::OAuth,
                scopes_supported,
            });
        }
        // Match Codex auth-status behavior: discovery failure means the
        // provider cannot currently prove OAuth support; saved credentials
        // above remain authoritative and errors do not leak provider bodies.
        let discovery = discover_mcp_oauth(url).await.unwrap_or(None);
        Ok(McpAuthStatusRecord {
            server_id: server_id.clone(),
            status: if discovery.is_some() {
                McpAuthStatus::NotLoggedIn
            } else {
                McpAuthStatus::Unsupported
            },
            scopes_supported: discovery
                .map(|metadata| metadata.scopes_supported().to_vec())
                .unwrap_or_default(),
        })
    }

    pub async fn start_oauth_login(
        &self,
        request: &McpOAuthLoginRequest,
    ) -> Result<(McpOAuthLoginResponse, McpOAuthLoginHandle), McpControlServiceError> {
        let server = self.server(&request.server_id).await?;
        let McpServerTransport::StreamableHttp { url } = &server.transport else {
            return Err(McpOAuthError::Unsupported.into());
        };
        if has_authorization_header(&server) {
            return Err(McpControlServiceError::AuthenticationConflict);
        }
        let handle = start_mcp_oauth_login(url, &request.scopes, request.timeout_secs).await?;
        let response = McpOAuthLoginResponse {
            authorization_url: handle.authorization_url().to_owned(),
        };
        Ok((response, handle))
    }

    /// Completes the browser login with Keyring-first/SQLite-second ordering.
    /// A database failure rolls back the newly written secret; a superseded
    /// secret is removed only after the new opaque reference commits.
    pub async fn finish_oauth_login(
        &self,
        server_id: &McpServerId,
        handle: McpOAuthLoginHandle,
    ) -> Result<(), McpControlServiceError> {
        let credential = handle.wait().await?;
        let server = self.server(server_id).await?;
        let McpServerTransport::StreamableHttp { url } = &server.transport else {
            return Err(McpOAuthError::Unsupported.into());
        };
        if credential.server_url() != url {
            return Err(McpOAuthError::InvalidConfiguration.into());
        }
        let reference = format!("oauth:{}:{}", server_id.as_str(), uuid::Uuid::now_v7());
        let secret = credential.to_secret_json()?;
        self.secrets
            .persist(&reference, &secret)
            .map_err(|_| McpControlServiceError::CredentialUnavailable)?;
        let previous = match self
            .store
            .replace_mcp_auth_reference(server_id, Some(&reference))
            .await
        {
            Ok(previous) => previous,
            Err(error) => {
                let _ = self.secrets.delete(&reference);
                return Err(error.into());
            }
        };
        if let Some(previous) = previous {
            self.delete_or_defer_secret(&previous).await;
        }
        if server.enabled {
            self.apply_runtime(&server).await?;
        }
        Ok(())
    }

    pub async fn logout_oauth(
        &self,
        server_id: &McpServerId,
    ) -> Result<McpAuthStatusRecord, McpControlServiceError> {
        let server = self.server(server_id).await?;
        let previous = self
            .store
            .replace_mcp_auth_reference(server_id, None)
            .await?;
        self.capabilities.unregister(&host_id(server_id));
        self.supervisor.stop(server_id).await;
        if let Some(reference) = previous {
            self.delete_or_defer_secret(&reference).await;
        }
        if server.enabled {
            let _ = self.apply_runtime(&server).await;
        }
        self.auth_status(server_id).await
    }

    pub async fn reconcile_startup(
        &self,
    ) -> Result<McpReconciliationReport, McpControlServiceError> {
        let mut report = McpReconciliationReport::default();
        for server in self.store.list_mcp_servers().await? {
            let (health, failed) = self.apply_isolated(&server).await;
            report.failures = report.failures.saturating_add(u32::from(failed));
            report.views.push(McpServerView {
                configuration: server,
                health,
            });
        }
        Ok(report)
    }

    pub async fn retry_due(
        &self,
        timestamp_ms: i64,
    ) -> Result<McpReconciliationReport, McpControlServiceError> {
        self.retry_failed(timestamp_ms, true).await
    }

    pub async fn retry_failed_now(
        &self,
    ) -> Result<McpReconciliationReport, McpControlServiceError> {
        self.retry_failed(now_ms(), false).await
    }

    async fn retry_failed(
        &self,
        timestamp_ms: i64,
        due_only: bool,
    ) -> Result<McpReconciliationReport, McpControlServiceError> {
        let health = self
            .store
            .list_mcp_server_health()
            .await?
            .into_iter()
            .map(|value| (value.server_id.clone(), value))
            .collect::<BTreeMap<_, _>>();
        let mut report = McpReconciliationReport::default();
        for server in self.store.list_mcp_servers().await? {
            let Some(current) = health.get(&server.id) else {
                continue;
            };
            if !server.enabled
                || current.state != McpServerHealthState::Failed
                || (due_only
                    && current
                        .next_retry_at_ms
                        .is_some_and(|next| next > timestamp_ms))
            {
                continue;
            }
            let (next, failed) = self.apply_isolated(&server).await;
            report.failures = report.failures.saturating_add(u32::from(failed));
            report.views.push(McpServerView {
                configuration: server,
                health: next,
            });
        }
        Ok(report)
    }

    pub async fn list(&self) -> Result<Vec<McpServerView>, McpControlServiceError> {
        let health = self
            .store
            .list_mcp_server_health()
            .await?
            .into_iter()
            .map(|health| (health.server_id.clone(), health))
            .collect::<BTreeMap<_, _>>();
        self.store
            .list_mcp_servers()
            .await?
            .into_iter()
            .map(|configuration| {
                let state = health
                    .get(&configuration.id)
                    .cloned()
                    .ok_or_else(|| AgentStoreError::McpServerNotFound(configuration.id.clone()))?;
                Ok(McpServerView {
                    configuration,
                    health: state,
                })
            })
            .collect()
    }

    pub async fn get(
        &self,
        server_id: &McpServerId,
    ) -> Result<McpServerView, McpControlServiceError> {
        let configuration = self
            .store
            .get_mcp_server(server_id)
            .await?
            .ok_or_else(|| AgentStoreError::McpServerNotFound(server_id.clone()))?;
        let health = self
            .store
            .get_mcp_server_health(server_id)
            .await?
            .ok_or_else(|| AgentStoreError::McpServerNotFound(server_id.clone()))?;
        Ok(McpServerView {
            configuration,
            health,
        })
    }

    pub async fn list_tools(
        &self,
        server_id: &McpServerId,
    ) -> Result<Vec<McpToolView>, McpControlServiceError> {
        let health = self
            .store
            .get_mcp_server_health(server_id)
            .await?
            .ok_or_else(|| AgentStoreError::McpServerNotFound(server_id.clone()))?;
        Ok(self
            .store
            .list_mcp_tools(server_id, health.state != McpServerHealthState::Ready)
            .await?)
    }

    pub async fn inventory(
        &self,
        server_id: &McpServerId,
    ) -> Result<McpInventorySnapshot, McpControlServiceError> {
        self.server(server_id).await?;
        Ok(self
            .store
            .get_mcp_inventory(server_id)
            .await?
            .unwrap_or_else(|| empty_inventory(server_id.clone(), true)))
    }

    pub async fn list_call_summaries(
        &self,
        request: &McpCallSummaryListRequest,
    ) -> Result<Vec<McpCallSummaryRecord>, McpControlServiceError> {
        if let Some(server_id) = &request.server_id {
            self.server(server_id).await?;
        }
        Ok(self.store.list_mcp_call_summaries(request).await?)
    }

    /// Refreshes a saved server's Resources/Templates/Prompts without enabling its Agent tools.
    pub async fn refresh_inventory(
        &self,
        server_id: &McpServerId,
    ) -> Result<McpInventorySnapshot, McpControlServiceError> {
        let server = self.server(server_id).await?;
        let snapshot = if server.enabled {
            self.supervisor.refresh_inventory(server_id).await
        } else {
            let headers = self.resolve_headers(&server).await?;
            let mut enabled = server.clone();
            enabled.enabled = true;
            let temporary = self.supervisor.isolated();
            let snapshot = temporary.apply_with_headers(&enabled, headers).await;
            temporary.stop(server_id).await;
            Some(snapshot)
        };
        let refreshed_at_ms = now_ms();
        match snapshot {
            Some(snapshot) if snapshot.health.state == McpServerHealthState::Ready => Ok(self
                .store
                .update_mcp_inventory(
                    server_id,
                    &snapshot.resources,
                    &snapshot.resource_templates,
                    &snapshot.prompts,
                    &snapshot.inventory_errors,
                    refreshed_at_ms,
                )
                .await?),
            Some(snapshot) => {
                let errors = BTreeMap::from([(
                    "connection".into(),
                    snapshot
                        .health
                        .error_code
                        .unwrap_or_else(|| "runtime_not_ready".into()),
                )]);
                Ok(self
                    .store
                    .update_mcp_inventory(server_id, &[], &[], &[], &errors, refreshed_at_ms)
                    .await?)
            }
            None => Ok(self
                .store
                .update_mcp_inventory(
                    server_id,
                    &[],
                    &[],
                    &[],
                    &BTreeMap::from([("connection".into(), "runtime_not_loaded".into())]),
                    refreshed_at_ms,
                )
                .await?),
        }
    }

    pub async fn read_resource(
        &self,
        request: &McpResourceReadRequest,
    ) -> Result<Vec<McpResourceContent>, McpControlServiceError> {
        let (client, _) = self
            .supervisor
            .client_and_tools(&request.server_id)
            .await
            .ok_or(McpControlServiceError::RuntimeUnavailable)?;
        Ok(client
            .read_resource(&request.uri, tokio_util::sync::CancellationToken::new())
            .await?)
    }

    pub async fn get_prompt(
        &self,
        request: McpPromptGetRequest,
    ) -> Result<McpPromptResult, McpControlServiceError> {
        let (client, _) = self
            .supervisor
            .client_and_tools(&request.server_id)
            .await
            .ok_or(McpControlServiceError::RuntimeUnavailable)?;
        Ok(client
            .get_prompt(
                &request.name,
                request.arguments,
                tokio_util::sync::CancellationToken::new(),
            )
            .await?)
    }

    pub async fn set_tool_enabled(
        &self,
        server_id: &McpServerId,
        tool_name: &str,
        enabled: bool,
        updated_at_ms: i64,
    ) -> Result<McpToolView, McpControlServiceError> {
        let override_record = self
            .store
            .set_mcp_tool_enabled(server_id, tool_name, enabled, updated_at_ms)
            .await?;
        self.reregister(server_id).await?;
        self.list_tools(server_id)
            .await?
            .into_iter()
            .find(|tool| tool.name == override_record.tool_name)
            .ok_or_else(|| AgentStoreError::InvalidPersistedValue {
                kind: "MCP tool override",
                value: override_record.tool_name,
            })
            .map_err(Into::into)
    }

    pub async fn tool_enabled(
        &self,
        server_id: &McpServerId,
        tool_name: &str,
    ) -> Result<bool, McpControlServiceError> {
        Ok(self.store.mcp_tool_enabled(server_id, tool_name).await?)
    }

    pub async fn ready_runtimes(&self) -> Result<Vec<McpReadyRuntime>, McpControlServiceError> {
        let mut runtimes = Vec::new();
        for configuration in self.store.list_mcp_servers().await? {
            if !configuration.enabled {
                continue;
            }
            if let Some((client, tools)) = self.supervisor.client_and_tools(&configuration.id).await
            {
                let tools = self.filter_enabled(&configuration.id, tools).await?;
                runtimes.push(McpReadyRuntime {
                    configuration,
                    client,
                    tools,
                });
            }
        }
        Ok(runtimes)
    }

    pub async fn test_connection(
        &self,
        server: &McpServerRecord,
        resolved_headers: BTreeMap<String, String>,
    ) -> McpConnectionTestResult {
        let mut enabled = server.clone();
        enabled.enabled = true;
        let supervisor = self.supervisor.isolated();
        let snapshot = supervisor
            .apply_with_headers(&enabled, resolved_headers)
            .await;
        let success = snapshot.health.state == McpServerHealthState::Ready;
        let tools = definitions_to_views(&enabled, &snapshot.tools);
        let result = McpConnectionTestResult {
            success,
            server_name: snapshot.health.server_name.clone(),
            server_version: snapshot.health.server_version.clone(),
            protocol_version: snapshot.health.protocol_version.clone(),
            tools,
            error_code: snapshot.health.error_code.clone(),
        };
        supervisor.stop(&enabled.id).await;
        result
    }

    /// Discovers a saved server's tools without changing its enabled state or
    /// registering capabilities for the Agent. A successful discovery updates
    /// the durable tool snapshot so per-tool overrides remain effective.
    pub async fn discover_tools(
        &self,
        server_id: &McpServerId,
    ) -> Result<McpConnectionTestResult, McpControlServiceError> {
        let server = self.server(server_id).await?;
        let resolved_headers = self.resolve_headers(&server).await?;
        let mut result = self.test_connection(&server, resolved_headers).await;
        if result.success {
            self.store
                .replace_mcp_discovered_tools(server_id, &result.tools)
                .await?;
            // Re-read through the store so global tool overrides are applied.
            result.tools = self.store.list_mcp_tools(server_id, false).await?;
        } else {
            // A transient refresh failure must not erase the last verified
            // schema or its explicit per-tool overrides. Keep it visible as
            // stale while preserving the failed health result/error code.
            result.tools = self.store.list_mcp_tools(server_id, true).await?;
        }
        Ok(result)
    }

    async fn apply_runtime(
        &self,
        server: &McpServerRecord,
    ) -> Result<McpServerHealthRecord, McpControlServiceError> {
        self.capabilities.unregister(&host_id(&server.id));
        let headers = self.resolve_headers(server).await?;
        let mut snapshot = self.supervisor.apply_with_headers(server, headers).await;
        if snapshot.health.state == McpServerHealthState::Ready {
            self.store
                .update_mcp_inventory(
                    &server.id,
                    &snapshot.resources,
                    &snapshot.resource_templates,
                    &snapshot.prompts,
                    &snapshot.inventory_errors,
                    now_ms(),
                )
                .await?;
            let views = definitions_to_views(server, &snapshot.tools);
            self.store
                .replace_mcp_discovered_tools(&server.id, &views)
                .await?;
            snapshot.tools = self.filter_enabled(&server.id, snapshot.tools).await?;
            snapshot.health.tool_count = u32::try_from(snapshot.tools.len()).unwrap_or(u32::MAX);
            if let Err(error) = self.register_snapshot(server, &snapshot) {
                self.supervisor.stop(&server.id).await;
                snapshot.health.state = McpServerHealthState::Failed;
                snapshot.health.server_name = None;
                snapshot.health.server_version = None;
                snapshot.health.protocol_version = None;
                snapshot.health.tool_count = 0;
                snapshot.health.error_code = Some("capability_registration_failed".into());
                snapshot.tools.clear();
                self.store.set_mcp_server_health(&snapshot.health).await?;
                return Err(error.into());
            }
        }
        let previous = self.store.get_mcp_server_health(&server.id).await?;
        if snapshot.health.state == McpServerHealthState::Failed {
            let failures = previous
                .as_ref()
                .map_or(1, |health| health.failure_count.saturating_add(1));
            snapshot.health.failure_count = failures;
            snapshot.health.next_retry_at_ms = Some(now_ms().saturating_add(mcp_retry_delay_ms(
                failures,
                snapshot.health.error_code.as_deref(),
            )));
        } else {
            snapshot.health.failure_count = 0;
            snapshot.health.next_retry_at_ms = None;
        }
        Ok(self.store.set_mcp_server_health(&snapshot.health).await?)
    }

    async fn apply_isolated(&self, server: &McpServerRecord) -> (McpServerHealthRecord, bool) {
        match self.apply_runtime(server).await {
            Ok(health) => {
                let failed = health.state == McpServerHealthState::Failed;
                (health, failed)
            }
            Err(error) => {
                let code = mcp_service_error_code(&error);
                eprintln!(
                    "Hachimi MCP server recovery failed (server={}, code={}): {}",
                    server.id, code, error
                );
                let previous = self
                    .store
                    .get_mcp_server_health(&server.id)
                    .await
                    .ok()
                    .flatten();
                let failures = previous
                    .as_ref()
                    .map_or(1, |health| health.failure_count.saturating_add(1));
                let health = McpServerHealthRecord {
                    server_id: server.id.clone(),
                    state: if server.enabled {
                        McpServerHealthState::Failed
                    } else {
                        McpServerHealthState::Disabled
                    },
                    server_name: None,
                    server_version: None,
                    protocol_version: None,
                    tool_count: 0,
                    error_code: server.enabled.then(|| code.into()),
                    failure_count: if server.enabled { failures } else { 0 },
                    next_retry_at_ms: server
                        .enabled
                        .then(|| now_ms().saturating_add(mcp_retry_delay_ms(failures, Some(code)))),
                    checked_at_ms: now_ms(),
                };
                let _ = self.store.set_mcp_server_health(&health).await;
                (health, server.enabled)
            }
        }
    }

    async fn filter_enabled(
        &self,
        server_id: &McpServerId,
        tools: Vec<McpToolDefinition>,
    ) -> Result<Vec<McpToolDefinition>, McpControlServiceError> {
        let mut enabled = Vec::new();
        for tool in tools {
            if self.store.mcp_tool_enabled(server_id, &tool.name).await? {
                enabled.push(tool);
            }
        }
        Ok(enabled)
    }

    async fn resolve_headers(
        &self,
        server: &McpServerRecord,
    ) -> Result<BTreeMap<String, String>, McpControlServiceError> {
        let mut headers = BTreeMap::new();
        for header in &server.headers {
            let value = if header.secret {
                let reference = header
                    .credential_reference
                    .as_deref()
                    .ok_or(McpControlServiceError::CredentialUnavailable)?;
                self.secrets
                    .resolve(reference)
                    .map_err(|_| McpControlServiceError::CredentialUnavailable)?
                    .ok_or(McpControlServiceError::CredentialUnavailable)?
            } else {
                header.value.clone().unwrap_or_default()
            };
            headers.insert(header.name.clone(), value);
        }
        let Some(reference) = self.store.get_mcp_auth_reference(&server.id).await? else {
            return Ok(headers);
        };
        if headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("authorization"))
        {
            return Err(McpControlServiceError::AuthenticationConflict);
        }
        let McpServerTransport::StreamableHttp { url } = &server.transport else {
            return Err(McpOAuthError::Unsupported.into());
        };
        // Serialize refresh transactions so a late refresh cannot overwrite a
        // newer credential or expose an access token before Keyring commit.
        let _refresh = self.oauth_refresh.lock().await;
        let secret = self
            .secrets
            .resolve(&reference)
            .map_err(|_| McpControlServiceError::CredentialUnavailable)?
            .ok_or(McpControlServiceError::CredentialUnavailable)?;
        let mut credential = McpOAuthCredential::from_secret_json(&secret)?;
        if credential.server_url() != url {
            return Err(McpOAuthError::InvalidConfiguration.into());
        }
        if credential.needs_refresh(u64::try_from(now_ms()).unwrap_or(u64::MAX)) {
            credential = refresh_mcp_oauth_credential(credential).await?;
            let refreshed = credential.to_secret_json()?;
            self.secrets
                .persist(&reference, &refreshed)
                .map_err(|_| McpControlServiceError::CredentialUnavailable)?;
        }
        // The refreshed access token is made visible only after the Keyring
        // write above succeeds.
        headers.insert("Authorization".into(), credential.authorization_header()?);
        Ok(headers)
    }

    async fn delete_or_defer_secret(&self, reference: &str) {
        if self.secrets.delete(reference).is_err() {
            let _ = self
                .store
                .defer_mcp_keyring_cleanup(reference, now_ms())
                .await;
        }
    }

    async fn reregister(&self, server_id: &McpServerId) -> Result<(), McpControlServiceError> {
        self.capabilities.unregister(&host_id(server_id));
        let Some(mut snapshot) = self.supervisor.get(server_id).await else {
            return Ok(());
        };
        if snapshot.health.state != McpServerHealthState::Ready {
            return Ok(());
        }
        let server = self.server(server_id).await?;
        snapshot.tools = self.filter_enabled(server_id, snapshot.tools).await?;
        self.register_snapshot(&server, &snapshot)?;
        Ok(())
    }

    fn register_snapshot(
        &self,
        server: &McpServerRecord,
        snapshot: &McpRuntimeSnapshot,
    ) -> Result<(), CapabilityRegistryError> {
        self.capabilities.register(CapabilityDescriptor {
            host_id: host_id(&server.id),
            host_kind: match server.transport {
                McpServerTransport::Stdio { .. } => "mcp_stdio",
                McpServerTransport::StreamableHttp { .. } => "mcp_streamable_http",
            }
            .into(),
            commands: snapshot
                .tools
                .iter()
                .map(|tool| tool.name.clone())
                .collect(),
        })
    }

    async fn server(
        &self,
        server_id: &McpServerId,
    ) -> Result<McpServerRecord, McpControlServiceError> {
        self.store
            .get_mcp_server(server_id)
            .await?
            .ok_or_else(|| AgentStoreError::McpServerNotFound(server_id.clone()))
            .map_err(Into::into)
    }
}

fn empty_inventory(server_id: McpServerId, stale: bool) -> McpInventorySnapshot {
    McpInventorySnapshot {
        server_id,
        resources: Vec::new(),
        resource_templates: Vec::new(),
        prompts: Vec::new(),
        errors: BTreeMap::new(),
        stale,
        refreshed_at_ms: 0,
    }
}

fn has_authorization_header(server: &McpServerRecord) -> bool {
    server.headers.iter().any(|header| {
        header.configured
            && header.name.eq_ignore_ascii_case("authorization")
            && (header.value.as_ref().is_some_and(|value| !value.is_empty())
                || header.credential_reference.is_some())
    })
}

fn definitions_to_views(server: &McpServerRecord, tools: &[McpToolDefinition]) -> Vec<McpToolView> {
    let discovered_at_ms = now_ms();
    let host_identity_hash = mcp_host_identity_hash(server);
    tools
        .iter()
        .map(|tool| {
            let schema = serde_json::to_vec(&tool.input_schema).unwrap_or_default();
            let required_parameters = tool
                .input_schema
                .get("required")
                .and_then(serde_json::Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            McpToolView {
                server_id: server.id.clone(),
                name: tool.name.clone(),
                exposed_name: mcp_exposed_tool_name(server.id.as_str(), &tool.name),
                description: tool.description.clone(),
                input_schema: tool.input_schema.clone(),
                required_parameters,
                enabled: true,
                stale: false,
                validation_error: None,
                schema_hash: hex_digest(&Sha256::digest(schema)),
                host_identity_hash: host_identity_hash.clone(),
                discovered_at_ms,
            }
        })
        .collect()
}

/// Stable MCP Host identity used by persisted Schedule authorization. Secret
/// header values and OAuth credentials are deliberately excluded.
#[must_use]
pub fn mcp_host_identity_hash(server: &McpServerRecord) -> String {
    let identity = serde_json::to_vec(&serde_json::json!({
        "serverId": server.id,
        "transport": server.transport,
        "hostPolicy": "restricted-mcp-host-v1",
    }))
    .unwrap_or_default();
    hex_digest(&Sha256::digest(identity))
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn host_id(server_id: &McpServerId) -> String {
    format!("mcp:{}", server_id.as_str())
}

fn mcp_service_error_code(error: &McpControlServiceError) -> &'static str {
    match error {
        McpControlServiceError::Store(_) => "mcp_state_unavailable",
        McpControlServiceError::Registry(_) => "capability_registration_failed",
        McpControlServiceError::CredentialUnavailable => "mcp_credential_unavailable",
        McpControlServiceError::RuntimeUnavailable => "mcp_runtime_unavailable",
        McpControlServiceError::AuthenticationConflict => "mcp_authentication_conflict",
        McpControlServiceError::OAuth(_) => "mcp_oauth_failed",
        McpControlServiceError::Client(_) => "mcp_client_failed",
    }
}

fn mcp_retry_delay_ms(failures: u32, error_code: Option<&str>) -> i64 {
    if matches!(
        error_code,
        Some("mcp_credential_unavailable" | "mcp_authentication_conflict")
    ) {
        return 60_000;
    }
    match failures {
        0 | 1 => 1_000,
        2 => 5_000,
        3 => 15_000,
        _ => 60_000,
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use hachimi_protocol::McpHeaderView;

    use super::*;

    #[derive(Debug, Default)]
    struct MemorySecrets(Mutex<BTreeMap<String, String>>);

    impl McpSecretResolver for MemorySecrets {
        fn resolve(&self, reference: &str) -> Result<Option<String>, String> {
            Ok(self.0.lock().expect("secrets").get(reference).cloned())
        }

        fn persist(&self, reference: &str, value: &str) -> Result<(), String> {
            self.0
                .lock()
                .expect("secrets")
                .insert(reference.into(), value.into());
            Ok(())
        }

        fn delete(&self, reference: &str) -> Result<(), String> {
            self.0.lock().expect("secrets").remove(reference);
            Ok(())
        }
    }

    fn server(enabled: bool) -> McpServerRecord {
        McpServerRecord {
            id: McpServerId::from("missing-fixture"),
            display_name: "Missing fixture".into(),
            enabled,
            transport: McpServerTransport::Stdio {
                command: "definitely-not-a-real-hachimi-mcp-server".into(),
                args: Vec::new(),
                cwd: None,
            },
            headers: Vec::new(),
            read_only_tools: Vec::new(),
            startup_timeout_ms: 100,
            request_timeout_ms: 100,
            max_message_bytes: 1024 * 1024,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    #[test]
    fn mcp_host_identity_excludes_credentials_but_changes_with_transport() {
        let mut first = server(false);
        let baseline = mcp_host_identity_hash(&first);
        first.display_name = "renamed only".into();
        assert_eq!(baseline, mcp_host_identity_hash(&first));
        first.headers.push(McpHeaderView {
            name: "Authorization".into(),
            value: Some("secret-value".into()),
            secret: true,
            configured: true,
            credential_reference: Some("keyring:one".into()),
        });
        assert_eq!(baseline, mcp_host_identity_hash(&first));
        first.transport = McpServerTransport::Stdio {
            command: "different-host".into(),
            args: Vec::new(),
            cwd: None,
        };
        assert_ne!(baseline, mcp_host_identity_hash(&first));
    }

    #[tokio::test]
    async fn persisted_failure_and_disable_never_register_capabilities() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let supervisor = Arc::new(McpSupervisor::allow_unrestricted_stdio_for_tests());
        let capabilities = Arc::new(CapabilityRegistry::default());
        let service = McpControlService::new(store, supervisor, Arc::clone(&capabilities));

        let failed = service
            .upsert(&server(true))
            .await
            .expect("persist failure");
        assert_eq!(failed.health.state, McpServerHealthState::Failed);
        assert_eq!(failed.health.error_code.as_deref(), Some("spawn_failed"));
        assert!(capabilities.is_empty());

        let disabled = service
            .set_enabled(&failed.configuration.id, false, 2)
            .await
            .expect("disable");
        assert_eq!(disabled.health.state, McpServerHealthState::Disabled);
        assert!(capabilities.is_empty());
        assert_eq!(service.list().await.expect("list"), vec![disabled]);
    }

    #[tokio::test]
    async fn discovery_caches_tools_without_enabling_or_registering_the_server() {
        let echo = hachimi_capabilities::McpEchoServer::start().expect("echo server");
        let store = AgentStore::connect_in_memory().await.expect("store");
        let capabilities = Arc::new(CapabilityRegistry::default());
        let service = McpControlService::with_default_runtime(store, Arc::clone(&capabilities));
        let configuration = McpServerRecord {
            id: McpServerId::from("echo-discovery"),
            display_name: "Echo discovery".into(),
            enabled: false,
            transport: McpServerTransport::StreamableHttp {
                url: echo.url().to_owned(),
            },
            headers: Vec::new(),
            read_only_tools: Vec::new(),
            startup_timeout_ms: 10_000,
            request_timeout_ms: 10_000,
            max_message_bytes: 1024 * 1024,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        let saved = service.upsert(&configuration).await.expect("save disabled");
        assert_eq!(saved.health.state, McpServerHealthState::Disabled);

        let discovered = service
            .discover_tools(&configuration.id)
            .await
            .expect("discover");
        assert!(
            discovered.success,
            "discovery failed with {:?}",
            discovered.error_code
        );
        assert_eq!(discovered.tools.len(), 1);
        assert_eq!(discovered.tools[0].name, "echo");
        assert!(!discovered.tools[0].stale);
        assert!(capabilities.is_empty());

        service
            .set_tool_enabled(&configuration.id, "echo", false, 2)
            .await
            .expect("disable tool");
        let rediscovered = service
            .discover_tools(&configuration.id)
            .await
            .expect("rediscover");
        assert!(rediscovered.success);
        assert!(!rediscovered.tools[0].enabled);
        assert!(capabilities.is_empty());

        drop(echo);
        let unavailable = service
            .discover_tools(&configuration.id)
            .await
            .expect("retain stale snapshot");
        assert!(!unavailable.success);
        assert_eq!(unavailable.tools.len(), 1);
        assert!(unavailable.tools[0].stale);
        assert!(!unavailable.tools[0].enabled);
    }

    #[tokio::test]
    async fn auth_status_distinguishes_stdio_and_manual_bearer_without_discovery() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let service = McpControlService::with_default_runtime(
            store.clone(),
            Arc::new(CapabilityRegistry::default()),
        );
        let stdio = server(false);
        store.upsert_mcp_server(&stdio).await.expect("stdio");
        assert_eq!(
            service
                .auth_status(&stdio.id)
                .await
                .expect("stdio status")
                .status,
            McpAuthStatus::Unsupported
        );

        let http = McpServerRecord {
            id: McpServerId::from("manual-bearer"),
            display_name: "Manual bearer".into(),
            enabled: false,
            transport: McpServerTransport::StreamableHttp {
                url: "https://example.test/mcp".into(),
            },
            headers: vec![hachimi_protocol::McpHeaderView {
                name: "Authorization".into(),
                value: None,
                secret: true,
                configured: true,
                credential_reference: Some("manual:keyring".into()),
            }],
            read_only_tools: Vec::new(),
            startup_timeout_ms: 100,
            request_timeout_ms: 100,
            max_message_bytes: 1024 * 1024,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        store.upsert_mcp_server(&http).await.expect("http");
        assert_eq!(
            service
                .auth_status(&http.id)
                .await
                .expect("bearer status")
                .status,
            McpAuthStatus::BearerToken
        );
    }

    #[tokio::test]
    async fn refresh_is_persisted_before_new_bearer_is_exposed() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let endpoint = format!("http://{}/token", listener.local_addr().expect("address"));
        let token_server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .expect("timeout");
            let mut request = [0_u8; 4096];
            let read = stream.read(&mut request).expect("request");
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(request.contains("grant_type=refresh_token"));
            assert!(request.contains("refresh_token=stable-refresh"));
            let body = serde_json::json!({
                "access_token": "fresh-access",
                "token_type": "Bearer",
                "expires_in": 3600
            })
            .to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .expect("response");
        });

        let store = AgentStore::connect_in_memory().await.expect("store");
        let secrets = Arc::new(MemorySecrets::default());
        let service = McpControlService::with_default_runtime_and_secret_resolver(
            store.clone(),
            Arc::new(CapabilityRegistry::default()),
            secrets.clone(),
        );
        let http = McpServerRecord {
            id: McpServerId::from("oauth-refresh"),
            display_name: "OAuth refresh".into(),
            enabled: false,
            transport: McpServerTransport::StreamableHttp {
                url: "http://127.0.0.1:1/mcp".into(),
            },
            headers: Vec::new(),
            read_only_tools: Vec::new(),
            startup_timeout_ms: 100,
            request_timeout_ms: 100,
            max_message_bytes: 1024 * 1024,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        store.upsert_mcp_server(&http).await.expect("server");
        let reference = "oauth:keyring:test";
        let secret = serde_json::json!({
            "serverUrl": "http://127.0.0.1:1/mcp",
            "tokenEndpoint": endpoint,
            "clientId": "client",
            "clientSecret": null,
            "accessToken": "expired-access",
            "refreshToken": "stable-refresh",
            "tokenType": "Bearer",
            "scopes": ["mcp.read"],
            "expiresAtMs": 0
        })
        .to_string();
        secrets.persist(reference, &secret).expect("secret");
        store
            .replace_mcp_auth_reference(&http.id, Some(reference))
            .await
            .expect("reference");
        let headers = service.resolve_headers(&http).await.expect("headers");
        assert_eq!(
            headers.get("Authorization").map(String::as_str),
            Some("Bearer fresh-access")
        );
        let persisted = secrets
            .resolve(reference)
            .expect("resolve")
            .expect("persisted");
        assert!(persisted.contains("fresh-access"));
        assert!(!persisted.contains("expired-access"));
        token_server.join().expect("token server");
    }
}
