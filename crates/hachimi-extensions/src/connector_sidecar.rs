use std::{ffi::OsString, path::PathBuf, sync::Arc, time::Duration};

use hachimi_protocol::{
    CapabilityGrantSet, CheckoutId, ComputerGrant, ConnectorDriverDescriptor, ConnectorHealth,
    ConnectorInvocationRequest, ConnectorRevision, ConnectorRuntimeKind, FileSystemAccess,
    FileSystemGrant, NetworkGrant, PermissionGrantScope, PermissionProfile, PluginId, ProcessGrant,
    RunId, SessionId, ToolEffect,
};
use hachimi_sandbox::{
    SandboxBackend, SandboxLaunchSpec, SandboxNetworkPolicy, grant_restricted_code_access,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroize;

use crate::{
    ConnectorDriver, ConnectorDriverContext, ConnectorDriverFuture, ExtensionHostError, PluginHost,
    connector_descriptor,
};

const SIDECAR_TIMEOUT: Duration = Duration::from_secs(60);
const SIDECAR_OUTPUT_LIMIT: usize = 4 * 1024 * 1024;

impl PluginHost {
    pub async fn register_sidecar_drivers(
        &self,
        backend: Arc<dyn SandboxBackend>,
    ) -> Result<u64, ExtensionHostError> {
        *self.hook_backend.write() = Some(Arc::clone(&backend));
        self.store
            .attach_plugin_hook_runtime(Arc::new(self.clone()))?;
        let mut registered = 0_u64;
        for plugin in self.list().await? {
            for contribution in plugin.manifest.contributions.iter().filter(|contribution| {
                contribution.kind == hachimi_protocol::PluginContributionKind::Connector
            }) {
                let descriptor = connector_descriptor(&plugin, &contribution.id)?;
                if descriptor.runtime_kind != ConnectorRuntimeKind::SandboxedStdioJsonRpc {
                    continue;
                }
                let entrypoint = descriptor
                    .entrypoint
                    .clone()
                    .ok_or(ExtensionHostError::Sidecar("entrypoint_missing"))?;
                let driver = SandboxedStdioConnectorDriver::new(
                    Arc::clone(&backend),
                    PathBuf::from(&plugin.root_path),
                    entrypoint,
                    descriptor.args,
                    descriptor.actions,
                )?;
                self.drivers
                    .register(&descriptor.host_identity, Arc::new(driver));
                registered = registered.saturating_add(1);
            }
        }
        Ok(registered)
    }
}

#[derive(Clone)]
pub struct SandboxedStdioConnectorDriver {
    backend: Arc<dyn SandboxBackend>,
    bundle_root: PathBuf,
    executable: PathBuf,
    args: Vec<OsString>,
    actions: Vec<String>,
}

impl std::fmt::Debug for SandboxedStdioConnectorDriver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SandboxedStdioConnectorDriver")
            .field("bundle_root", &self.bundle_root)
            .field("executable", &self.executable)
            .field("actions", &self.actions)
            .finish_non_exhaustive()
    }
}

impl SandboxedStdioConnectorDriver {
    pub fn new(
        backend: Arc<dyn SandboxBackend>,
        bundle_root: PathBuf,
        executable: PathBuf,
        args: Vec<String>,
        actions: Vec<String>,
    ) -> Result<Self, ExtensionHostError> {
        let bundle_root = bundle_root
            .canonicalize()
            .map_err(|_| ExtensionHostError::Sidecar("bundle_root_missing"))?;
        let executable = executable
            .canonicalize()
            .map_err(|_| ExtensionHostError::Sidecar("entrypoint_missing"))?;
        if !executable.starts_with(&bundle_root) || !executable.is_file() {
            return Err(ExtensionHostError::Sidecar("entrypoint_escape"));
        }
        Ok(Self {
            backend,
            bundle_root,
            executable,
            args: args.into_iter().map(OsString::from).collect(),
            actions,
        })
    }

    async fn call(
        &self,
        method: &'static str,
        mut context: ConnectorDriverContext,
        request: Option<&ConnectorInvocationRequest>,
    ) -> Result<Value, ExtensionHostError> {
        let id = uuid::Uuid::now_v7().to_string();
        let wire = SidecarRequest {
            jsonrpc: "2.0",
            id: &id,
            method,
            params: SidecarParams {
                account_id: context.account.id.as_str(),
                plugin_id: context.account.plugin_id.as_str(),
                connector_id: &context.account.connector_id,
                credential: context.credential.as_deref(),
                request,
            },
        };
        let mut input = serde_json::to_vec(&wire)?;
        input.push(b'\n');
        if let Some(credential) = &mut context.credential {
            credential.zeroize();
        }

        let temporary = tempfile::Builder::new()
            .prefix("hachimi-connector-sidecar-")
            .tempdir()
            .map_err(|_| ExtensionHostError::Sidecar("temp_create_failed"))?;
        grant_restricted_code_access(&self.bundle_root, false)
            .map_err(|_| ExtensionHostError::Sidecar("bundle_acl_failed"))?;
        grant_restricted_code_access(temporary.path(), true)
            .map_err(|_| ExtensionHostError::Sidecar("temp_acl_failed"))?;
        let session_id = SessionId::random();
        let run_id = RunId::random();
        let temporary_root = temporary.path().to_string_lossy().into_owned();
        let bundle_root = self.bundle_root.to_string_lossy().into_owned();
        let executable = self.executable.to_string_lossy().into_owned();
        let grants = CapabilityGrantSet {
            profile: PermissionProfile::WorkspaceWrite,
            scope: PermissionGrantScope::Run,
            session_id: session_id.clone(),
            run_id: Some(run_id.clone()),
            source: "plugin_connector_sidecar_runtime".into(),
            file_system: vec![
                FileSystemGrant {
                    access: FileSystemAccess::Read,
                    roots: vec![bundle_root, temporary_root.clone()],
                    globs: Vec::new(),
                    special_roots: Vec::new(),
                },
                FileSystemGrant {
                    access: FileSystemAccess::Write,
                    roots: vec![temporary_root],
                    globs: Vec::new(),
                    special_roots: Vec::new(),
                },
            ],
            network: NetworkGrant::default(),
            process: ProcessGrant {
                spawn: true,
                interactive: false,
                allowed_commands: vec![executable],
            },
            browser: Default::default(),
            computer: ComputerGrant::default(),
            review_each_command: false,
            expires_at_ms: None,
        };
        let child = self
            .backend
            .spawn_restricted(
                SandboxLaunchSpec {
                    session_id,
                    run_id,
                    run_generation: 1,
                    checkout_id: CheckoutId::random(),
                    checkout_root: temporary.path().to_path_buf(),
                    grants,
                    required_effect: ToolEffect::Process,
                    executable: self.executable.clone(),
                    args: self.args.clone(),
                    cwd: temporary.path().to_path_buf(),
                    environment: restricted_environment(temporary.path()),
                    stdin: Some(input),
                    interactive_stdin: false,
                    timeout: SIDECAR_TIMEOUT,
                    output_limit: SIDECAR_OUTPUT_LIMIT,
                    network_policy: SandboxNetworkPolicy::DenyAll,
                },
                CancellationToken::new(),
            )
            .await
            .map_err(|_| ExtensionHostError::Sidecar("sandbox_spawn_failed"))?;
        let output = child
            .wait()
            .await
            .map_err(|_| ExtensionHostError::Sidecar("process_failed"))?;
        if output.exit_code != Some(0) || output.truncated {
            return Err(ExtensionHostError::Sidecar("invalid_exit"));
        }
        let response: SidecarResponse = serde_json::from_slice(&output.stdout)
            .map_err(|_| ExtensionHostError::Sidecar("response_invalid"))?;
        if response.jsonrpc != "2.0" || response.id != id || response.error.is_some() {
            return Err(ExtensionHostError::Sidecar("response_rejected"));
        }
        response
            .result
            .ok_or(ExtensionHostError::Sidecar("result_missing"))
    }
}

impl ConnectorDriver for SandboxedStdioConnectorDriver {
    fn descriptor(
        &self,
        plugin_id: &PluginId,
        connector_id: &str,
        revision: ConnectorRevision,
    ) -> ConnectorDriverDescriptor {
        ConnectorDriverDescriptor {
            plugin_id: plugin_id.clone(),
            connector_id: connector_id.into(),
            runtime_kind: ConnectorRuntimeKind::SandboxedStdioJsonRpc,
            revision,
            actions: self.actions.clone(),
        }
    }

    fn health<'a>(
        &'a self,
        context: &'a ConnectorDriverContext,
    ) -> ConnectorDriverFuture<'a, ConnectorHealth> {
        let context = context.clone();
        Box::pin(async move {
            let result = self.call("health", context, None).await?;
            Ok(
                if result.get("status").and_then(Value::as_str) == Some("healthy") {
                    ConnectorHealth::Healthy
                } else {
                    ConnectorHealth::Failed
                },
            )
        })
    }

    fn invoke<'a>(
        &'a self,
        context: ConnectorDriverContext,
        request: &'a ConnectorInvocationRequest,
    ) -> ConnectorDriverFuture<'a, Value> {
        Box::pin(async move { self.call("invoke", context, Some(request)).await })
    }

    fn webhook<'a>(
        &'a self,
        context: ConnectorDriverContext,
        request: &'a ConnectorInvocationRequest,
    ) -> ConnectorDriverFuture<'a, Value> {
        Box::pin(async move { self.call("webhook", context, Some(request)).await })
    }

    fn poll<'a>(
        &'a self,
        context: ConnectorDriverContext,
        request: &'a ConnectorInvocationRequest,
    ) -> ConnectorDriverFuture<'a, Value> {
        Box::pin(async move { self.call("poll", context, Some(request)).await })
    }

    fn revoke<'a>(&'a self, context: ConnectorDriverContext) -> ConnectorDriverFuture<'a, ()> {
        Box::pin(async move {
            self.call("revoke", context, None).await?;
            Ok(())
        })
    }
}

#[derive(Serialize)]
struct SidecarRequest<'a> {
    jsonrpc: &'static str,
    id: &'a str,
    method: &'static str,
    params: SidecarParams<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SidecarParams<'a> {
    account_id: &'a str,
    plugin_id: &'a str,
    connector_id: &'a str,
    credential: Option<&'a str>,
    request: Option<&'a ConnectorInvocationRequest>,
}

#[derive(Deserialize)]
struct SidecarResponse {
    jsonrpc: String,
    id: String,
    result: Option<Value>,
    error: Option<Value>,
}

fn restricted_environment(temporary: &std::path::Path) -> Vec<(OsString, OsString)> {
    let mut environment = vec![
        (OsString::from("TEMP"), temporary.as_os_str().to_owned()),
        (OsString::from("TMP"), temporary.as_os_str().to_owned()),
    ];
    for name in ["SYSTEMROOT", "WINDIR", "PATH", "PATHEXT"] {
        if let Some(value) = std::env::var_os(name) {
            environment.push((OsString::from(name), value));
        }
    }
    environment
}
