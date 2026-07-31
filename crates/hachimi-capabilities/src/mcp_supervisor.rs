// SPDX-License-Identifier: Apache-2.0
// Adapted from OpenAI Codex commit 4c43465133428898aa84f0bfc02c306ed65fb66a:
// codex-rs/rmcp-client/src/* and codex-rs/windows-sandbox-rs/src/{spawn_prep,process,policy}.rs.
//! Lifecycle supervisor for configured local stdio MCP servers.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use hachimi_protocol::{
    CapabilityGrantSet, CheckoutId, ComputerGrant, FileSystemAccess, FileSystemGrant,
    McpServerHealthRecord, McpServerHealthState, McpServerId, McpServerRecord, McpServerTransport,
    NetworkGrant, PermissionGrantScope, PermissionProfile, ProcessGrant, RunId, SessionId,
    ToolEffect,
};
use hachimi_sandbox::{
    SandboxBackend, SandboxLaunchSpec, SandboxNetworkPolicy, SandboxStatus, prepare_workspace_acl,
    validate_checkout_root,
};
use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::{
    McpClientError, McpClientHandle, McpHttpClient, McpHttpServerConfig, McpPrompt, McpResource,
    McpResourceTemplate, McpStdioClient, McpStdioServerConfig, McpToolDefinition,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpRuntimeSnapshot {
    pub health: McpServerHealthRecord,
    pub tools: Vec<McpToolDefinition>,
    pub resources: Vec<McpResource>,
    pub resource_templates: Vec<McpResourceTemplate>,
    pub prompts: Vec<McpPrompt>,
    pub inventory_errors: BTreeMap<String, String>,
}

#[derive(Debug)]
struct RuntimeEntry {
    generation: u64,
    health: McpServerHealthRecord,
    tools: Vec<McpToolDefinition>,
    resources: Vec<McpResource>,
    resource_templates: Vec<McpResourceTemplate>,
    prompts: Vec<McpPrompt>,
    inventory_errors: BTreeMap<String, String>,
    client: Option<Arc<McpClientHandle>>,
    cancellation: CancellationToken,
}

impl RuntimeEntry {
    fn snapshot(&self) -> McpRuntimeSnapshot {
        McpRuntimeSnapshot {
            health: self.health.clone(),
            tools: self.tools.clone(),
            resources: self.resources.clone(),
            resource_templates: self.resource_templates.clone(),
            prompts: self.prompts.clone(),
            inventory_errors: self.inventory_errors.clone(),
        }
    }
}

#[derive(Clone)]
pub struct McpStdioSandboxHost {
    backend: Arc<dyn SandboxBackend>,
    host_root: PathBuf,
}

impl std::fmt::Debug for McpStdioSandboxHost {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpStdioSandboxHost")
            .field("host_root", &self.host_root)
            .finish_non_exhaustive()
    }
}

impl McpStdioSandboxHost {
    #[must_use]
    pub fn new(backend: Arc<dyn SandboxBackend>, host_root: impl Into<PathBuf>) -> Self {
        Self {
            backend,
            host_root: host_root.into(),
        }
    }
}

#[derive(Debug, Default, Clone)]
enum McpStdioMode {
    #[default]
    RequireSandbox,
    Sandboxed(McpStdioSandboxHost),
    UnrestrictedForTests,
}

#[derive(Debug, Default)]
pub struct McpSupervisor {
    next_generation: AtomicU64,
    servers: Mutex<BTreeMap<McpServerId, RuntimeEntry>>,
    stdio_mode: McpStdioMode,
}

impl McpSupervisor {
    #[must_use]
    pub fn with_stdio_sandbox(host: McpStdioSandboxHost) -> Self {
        Self {
            stdio_mode: McpStdioMode::Sandboxed(host),
            ..Self::default()
        }
    }

    /// Direct process spawning exists only for protocol-level integration tests.
    #[doc(hidden)]
    #[must_use]
    pub fn allow_unrestricted_stdio_for_tests() -> Self {
        Self {
            stdio_mode: McpStdioMode::UnrestrictedForTests,
            ..Self::default()
        }
    }

    /// Creates an empty supervisor that preserves the configured stdio Host boundary.
    ///
    /// Connection tests and disabled-server inventory refreshes must not mutate the
    /// active runtime registry, but they must use the same sandbox policy as normal
    /// server startup. In particular, creating `Self::default()` at those call sites
    /// would silently discard a Desktop-provided restricted stdio Host.
    #[must_use]
    pub fn isolated(&self) -> Self {
        Self {
            stdio_mode: self.stdio_mode.clone(),
            ..Self::default()
        }
    }

    /// Applies one persisted configuration. Updating a running definition always creates a new
    /// generation; initialization results from the previous process are discarded.
    pub async fn apply(&self, record: &McpServerRecord) -> McpRuntimeSnapshot {
        self.apply_with_headers(record, BTreeMap::new()).await
    }

    pub async fn apply_with_headers(
        &self,
        record: &McpServerRecord,
        resolved_headers: BTreeMap<String, String>,
    ) -> McpRuntimeSnapshot {
        if !record.enabled {
            return self.transition_to_inactive(record.id.clone(), true).await;
        }

        let generation = self.next_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let cancellation = CancellationToken::new();
        let starting = RuntimeEntry {
            generation,
            health: health(record.id.clone(), McpServerHealthState::Starting, None),
            tools: Vec::new(),
            resources: Vec::new(),
            resource_templates: Vec::new(),
            prompts: Vec::new(),
            inventory_errors: BTreeMap::new(),
            client: None,
            cancellation: cancellation.clone(),
        };
        let previous = self
            .servers
            .lock()
            .await
            .insert(record.id.clone(), starting);
        shutdown_entry(previous).await;

        let initialized = connect_client(
            record,
            resolved_headers,
            &self.stdio_mode,
            cancellation.child_token(),
        )
        .await;
        let (client, tools, failure_code) = match initialized {
            Ok(client) => {
                let client = Arc::new(client);
                match client.list_tools(cancellation.child_token()).await {
                    Ok(tools) => (Some(client), tools, None),
                    Err(error) => {
                        let code = error.stable_code();
                        let _ = client.shutdown().await;
                        (None, Vec::new(), Some(code))
                    }
                }
            }
            Err(error) => (None, Vec::new(), Some(error.stable_code())),
        };
        let inventory = match client.as_ref() {
            Some(client) => discover_inventory(client, cancellation.child_token()).await,
            None => McpInventoryDiscovery::default(),
        };

        let mut servers = self.servers.lock().await;
        let Some(entry) = servers.get_mut(&record.id) else {
            drop(servers);
            if let Some(client) = client {
                let _ = client.shutdown().await;
            }
            return inactive_snapshot(record.id.clone(), false);
        };
        if entry.generation != generation {
            let current = entry.snapshot();
            drop(servers);
            if let Some(client) = client {
                let _ = client.shutdown().await;
            }
            return current;
        }

        if let Some(code) = failure_code {
            entry.health = health(record.id.clone(), McpServerHealthState::Failed, Some(code));
            entry.tools.clear();
            entry.resources.clear();
            entry.resource_templates.clear();
            entry.prompts.clear();
            entry.inventory_errors.clear();
            entry.client = None;
        } else if let Some(client) = client {
            let server_info = client.server_info();
            entry.health = McpServerHealthRecord {
                server_id: record.id.clone(),
                state: McpServerHealthState::Ready,
                server_name: Some(server_info.name.clone()),
                server_version: Some(server_info.version.clone()),
                protocol_version: Some(server_info.protocol_version.clone()),
                tool_count: u32::try_from(tools.len()).unwrap_or(u32::MAX),
                error_code: None,
                checked_at_ms: now_ms(),
            };
            entry.tools = tools;
            entry.resources = inventory.resources;
            entry.resource_templates = inventory.resource_templates;
            entry.prompts = inventory.prompts;
            entry.inventory_errors = inventory.errors;
            entry.client = Some(client);
        }
        entry.snapshot()
    }

    pub async fn stop(&self, server_id: &McpServerId) -> McpRuntimeSnapshot {
        self.transition_to_inactive(server_id.clone(), false).await
    }

    pub async fn remove(&self, server_id: &McpServerId) -> bool {
        let previous = self.servers.lock().await.remove(server_id);
        let existed = previous.is_some();
        shutdown_entry(previous).await;
        existed
    }

    #[must_use]
    pub async fn get(&self, server_id: &McpServerId) -> Option<McpRuntimeSnapshot> {
        self.servers
            .lock()
            .await
            .get(server_id)
            .map(RuntimeEntry::snapshot)
    }

    #[must_use]
    pub async fn list(&self) -> Vec<McpRuntimeSnapshot> {
        self.servers
            .lock()
            .await
            .values()
            .map(RuntimeEntry::snapshot)
            .collect()
    }

    /// Returns a ready client and the exact tool-definition generation discovered with it.
    /// Callers must not reuse this pair after applying a newer server configuration.
    pub async fn client_and_tools(
        &self,
        server_id: &McpServerId,
    ) -> Option<(Arc<McpClientHandle>, Vec<McpToolDefinition>)> {
        let servers = self.servers.lock().await;
        let entry = servers.get(server_id)?;
        if entry.health.state != McpServerHealthState::Ready {
            return None;
        }
        Some((Arc::clone(entry.client.as_ref()?), entry.tools.clone()))
    }

    pub async fn refresh_health(&self, server_id: &McpServerId) -> Option<McpRuntimeSnapshot> {
        let (generation, client, cancellation) = {
            let servers = self.servers.lock().await;
            let entry = servers.get(server_id)?;
            let client = entry.client.as_ref().map(Arc::clone)?;
            (entry.generation, client, entry.cancellation.child_token())
        };
        let ping = client.ping(cancellation).await;
        let mut servers = self.servers.lock().await;
        let entry = servers.get_mut(server_id)?;
        if entry.generation != generation {
            return Some(entry.snapshot());
        }
        entry.health.checked_at_ms = now_ms();
        if let Err(error) = ping {
            entry.health.state = McpServerHealthState::Failed;
            entry.health.error_code = Some(error.stable_code().into());
            entry.health.tool_count = 0;
            entry.health.server_name = None;
            entry.health.server_version = None;
            entry.health.protocol_version = None;
            entry.tools.clear();
            entry.resources.clear();
            entry.resource_templates.clear();
            entry.prompts.clear();
            entry.inventory_errors.clear();
            entry.client = None;
            drop(servers);
            let _ = client.shutdown().await;
            return self.get(server_id).await;
        }
        Some(entry.snapshot())
    }

    /// Refreshes the non-tool inventory for one ready generation. Individual inventory surface
    /// failures are retained as stable errors and do not unregister otherwise healthy tools.
    pub async fn refresh_inventory(&self, server_id: &McpServerId) -> Option<McpRuntimeSnapshot> {
        let (generation, client, cancellation) = {
            let servers = self.servers.lock().await;
            let entry = servers.get(server_id)?;
            if entry.health.state != McpServerHealthState::Ready {
                return Some(entry.snapshot());
            }
            (
                entry.generation,
                Arc::clone(entry.client.as_ref()?),
                entry.cancellation.child_token(),
            )
        };
        let inventory = discover_inventory(&client, cancellation).await;
        let mut servers = self.servers.lock().await;
        let entry = servers.get_mut(server_id)?;
        if entry.generation != generation {
            return Some(entry.snapshot());
        }
        entry.resources = inventory.resources;
        entry.resource_templates = inventory.resource_templates;
        entry.prompts = inventory.prompts;
        entry.inventory_errors = inventory.errors;
        Some(entry.snapshot())
    }

    async fn transition_to_inactive(
        &self,
        server_id: McpServerId,
        disabled: bool,
    ) -> McpRuntimeSnapshot {
        let generation = self.next_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let state = if disabled {
            McpServerHealthState::Disabled
        } else {
            McpServerHealthState::Stopped
        };
        let entry = RuntimeEntry {
            generation,
            health: health(server_id.clone(), state, None),
            tools: Vec::new(),
            resources: Vec::new(),
            resource_templates: Vec::new(),
            prompts: Vec::new(),
            inventory_errors: BTreeMap::new(),
            client: None,
            cancellation: CancellationToken::new(),
        };
        let snapshot = entry.snapshot();
        let previous = self.servers.lock().await.insert(server_id, entry);
        shutdown_entry(previous).await;
        snapshot
    }
}

async fn connect_client(
    record: &McpServerRecord,
    headers: BTreeMap<String, String>,
    stdio_mode: &McpStdioMode,
    cancellation: CancellationToken,
) -> Result<McpClientHandle, McpClientError> {
    match &record.transport {
        McpServerTransport::Stdio { command, args, cwd } => {
            let config = McpStdioServerConfig {
                server_id: record.id.as_str().into(),
                command: PathBuf::from(command),
                args: args.iter().map(OsString::from).collect(),
                cwd: cwd.as_ref().map(PathBuf::from),
                startup_timeout: Duration::from_millis(record.startup_timeout_ms),
                request_timeout: Duration::from_millis(record.request_timeout_ms),
                max_message_bytes: usize::try_from(record.max_message_bytes).unwrap_or(usize::MAX),
            };
            let client = match stdio_mode {
                McpStdioMode::RequireSandbox => Err(McpClientError::HostSandbox(
                    "mcp_host_sandbox_not_configured",
                )),
                McpStdioMode::UnrestrictedForTests => {
                    McpStdioClient::connect_unrestricted_for_tests(config, cancellation).await
                }
                McpStdioMode::Sandboxed(host) => host.connect(config, cancellation).await,
            }?;
            Ok(McpClientHandle::Stdio(Box::new(client)))
        }
        McpServerTransport::StreamableHttp { url } => {
            let url = url::Url::parse(url).map_err(|_| {
                McpClientError::InvalidConfiguration("remote MCP URL is invalid".into())
            })?;
            McpHttpClient::connect(
                McpHttpServerConfig {
                    server_id: record.id.as_str().into(),
                    url,
                    headers,
                    startup_timeout: Duration::from_millis(record.startup_timeout_ms),
                    request_timeout: Duration::from_millis(record.request_timeout_ms),
                    max_message_bytes: usize::try_from(record.max_message_bytes)
                        .unwrap_or(usize::MAX),
                },
                cancellation,
            )
            .await
            .map(|client| McpClientHandle::StreamableHttp(Box::new(client)))
        }
    }
}

impl McpStdioSandboxHost {
    async fn connect(
        &self,
        mut config: McpStdioServerConfig,
        cancellation: CancellationToken,
    ) -> Result<McpStdioClient, McpClientError> {
        if SandboxStatus::from_report(&self.backend.capability_report()) != SandboxStatus::Enforced
        {
            return Err(McpClientError::HostSandbox("mcp_host_sandbox_not_enforced"));
        }
        if !config.command.is_absolute() {
            return Err(McpClientError::HostSandbox(
                "mcp_host_executable_must_be_absolute",
            ));
        }
        if config.cwd.as_ref().is_some_and(|cwd| !cwd.is_absolute()) {
            return Err(McpClientError::HostSandbox("mcp_host_cwd_must_be_absolute"));
        }
        let executable = std::fs::canonicalize(&config.command).map_err(|_| {
            McpClientError::HostSandbox("mcp_host_executable_must_be_absolute_and_readable")
        })?;
        if !executable.is_file() {
            return Err(McpClientError::HostSandbox(
                "mcp_host_executable_is_not_a_file",
            ));
        }
        let server_root = self
            .host_root
            .join(safe_server_component(&config.server_id));
        let temp_root = server_root.join("temp");
        std::fs::create_dir_all(&temp_root)
            .map_err(|_| McpClientError::HostSandbox("mcp_host_root_unavailable"))?;
        let cwd = match config.cwd.as_ref() {
            Some(cwd) => std::fs::canonicalize(cwd)
                .map_err(|_| McpClientError::HostSandbox("mcp_host_cwd_unavailable"))?,
            None => std::fs::canonicalize(&server_root)
                .map_err(|_| McpClientError::HostSandbox("mcp_host_root_unavailable"))?,
        };
        if !cwd.is_dir() {
            return Err(McpClientError::HostSandbox(
                "mcp_host_cwd_is_not_a_directory",
            ));
        }
        validate_checkout_root(&cwd)
            .map_err(|_| McpClientError::HostSandbox("mcp_host_cwd_rejected"))?;
        prepare_workspace_acl(&cwd, &temp_root, &executable)
            .map_err(|_| McpClientError::HostSandbox("mcp_host_acl_preparation_failed"))?;

        config.command = executable.clone();
        config.cwd = Some(cwd.clone());
        let session_id = SessionId::random();
        let run_id = RunId::random();
        let checkout_id = CheckoutId::random();
        let root = cwd.to_string_lossy().into_owned();
        let temp = temp_root.to_string_lossy().into_owned();
        let grants = CapabilityGrantSet {
            profile: PermissionProfile::WorkspaceWrite,
            scope: PermissionGrantScope::Run,
            session_id: session_id.clone(),
            run_id: Some(run_id.clone()),
            source: "mcp_stdio_host_runtime".into(),
            file_system: vec![
                FileSystemGrant {
                    access: FileSystemAccess::Read,
                    roots: vec![root.clone(), temp.clone()],
                    globs: Vec::new(),
                    special_roots: Vec::new(),
                },
                FileSystemGrant {
                    access: FileSystemAccess::Write,
                    roots: vec![root, temp],
                    globs: Vec::new(),
                    special_roots: Vec::new(),
                },
            ],
            network: NetworkGrant::default(),
            process: ProcessGrant {
                spawn: true,
                interactive: true,
                allowed_commands: vec![executable.to_string_lossy().into_owned()],
            },
            browser: Default::default(),
            computer: ComputerGrant::default(),
            review_each_command: false,
            expires_at_ms: None,
        };
        let environment = restricted_mcp_environment(&temp_root);
        let launch = SandboxLaunchSpec {
            session_id,
            run_id,
            run_generation: 1,
            checkout_id,
            checkout_root: cwd.clone(),
            grants,
            required_effect: ToolEffect::Process,
            executable,
            args: config.args.clone(),
            cwd,
            environment,
            stdin: None,
            interactive_stdin: true,
            timeout: Duration::from_secs(24 * 60 * 60),
            output_limit: config.max_message_bytes,
            network_policy: SandboxNetworkPolicy::DenyAll,
        };
        McpStdioClient::connect_sandboxed(config, Arc::clone(&self.backend), launch, cancellation)
            .await
    }
}

fn safe_server_component(server_id: &str) -> String {
    server_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .take(64)
        .collect()
}

fn restricted_mcp_environment(temp_root: &std::path::Path) -> Vec<(OsString, OsString)> {
    let mut environment = super::mcp::sanitized_environment()
        .into_iter()
        .filter(|(name, _)| {
            !matches!(
                name.to_str(),
                Some("TEMP" | "TMP" | "USERPROFILE" | "LOCALAPPDATA" | "APPDATA")
            )
        })
        .collect::<Vec<_>>();
    environment.push(("TEMP".into(), temp_root.as_os_str().to_owned()));
    environment.push(("TMP".into(), temp_root.as_os_str().to_owned()));
    environment.push(("USERPROFILE".into(), temp_root.as_os_str().to_owned()));
    environment.push(("LOCALAPPDATA".into(), temp_root.as_os_str().to_owned()));
    environment.push(("APPDATA".into(), temp_root.as_os_str().to_owned()));
    environment
}

async fn shutdown_entry(entry: Option<RuntimeEntry>) {
    if let Some(entry) = entry {
        entry.cancellation.cancel();
        if let Some(client) = entry.client {
            let _ = client.shutdown().await;
        }
    }
}

fn health(
    server_id: McpServerId,
    state: McpServerHealthState,
    error_code: Option<&str>,
) -> McpServerHealthRecord {
    McpServerHealthRecord {
        server_id,
        state,
        server_name: None,
        server_version: None,
        protocol_version: None,
        tool_count: 0,
        error_code: error_code.map(str::to_owned),
        checked_at_ms: now_ms(),
    }
}

fn inactive_snapshot(server_id: McpServerId, disabled: bool) -> McpRuntimeSnapshot {
    McpRuntimeSnapshot {
        health: health(
            server_id,
            if disabled {
                McpServerHealthState::Disabled
            } else {
                McpServerHealthState::Stopped
            },
            None,
        ),
        tools: Vec::new(),
        resources: Vec::new(),
        resource_templates: Vec::new(),
        prompts: Vec::new(),
        inventory_errors: BTreeMap::new(),
    }
}

#[derive(Default)]
struct McpInventoryDiscovery {
    resources: Vec<McpResource>,
    resource_templates: Vec<McpResourceTemplate>,
    prompts: Vec<McpPrompt>,
    errors: BTreeMap<String, String>,
}

async fn discover_inventory(
    client: &McpClientHandle,
    cancellation: CancellationToken,
) -> McpInventoryDiscovery {
    let mut discovery = McpInventoryDiscovery::default();
    match client.list_resources(cancellation.child_token()).await {
        Ok(resources) => discovery.resources = resources,
        Err(error) => {
            discovery
                .errors
                .insert("resources".into(), error.stable_code().into());
        }
    }
    match client
        .list_resource_templates(cancellation.child_token())
        .await
    {
        Ok(templates) => discovery.resource_templates = templates,
        Err(error) => {
            discovery
                .errors
                .insert("resource_templates".into(), error.stable_code().into());
        }
    }
    match client.list_prompts(cancellation).await {
        Ok(prompts) => discovery.prompts = prompts,
        Err(error) => {
            discovery
                .errors
                .insert("prompts".into(), error.stable_code().into());
        }
    }
    discovery
}

fn now_ms() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}
