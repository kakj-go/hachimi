use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use hachimi_protocol::{
    CapabilityGrantSet, CheckoutId, ComputerGrant, ContributionRuntimeState, FileSystemAccess,
    FileSystemGrant, InstalledContribution, InstalledPlugin, NetworkGrant, PermissionGrantScope,
    PermissionProfile, PluginContribution, PluginContributionKind, PluginHookDescriptor,
    PluginHookInvocation, PluginHookOutcome, PluginHookRuntimeKind, PluginId, PluginStatus,
    ProcessGrant, RunId, SessionId, ToolEffect,
};
use hachimi_sandbox::{
    SandboxBackend, SandboxLaunchSpec, SandboxNetworkPolicy, grant_restricted_code_access,
};
use hachimi_storage::{PluginHookRuntime, PluginHookRuntimeFuture, PluginHookSubscription};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::{ExtensionHostError, PluginHost, hash_bundle, now_ms, safe_relative_path};

const HOOK_TIMEOUT: Duration = Duration::from_secs(10);
const HOOK_OUTPUT_LIMIT: usize = 64 * 1024;
const MAX_HOOK_ARGS: usize = 64;

impl PluginHost {
    pub(super) async fn reconcile_hook_subscriptions(
        &self,
        plugin: &InstalledPlugin,
        contribution: &PluginContribution,
        runtime: &InstalledContribution,
        enabled: bool,
    ) -> Result<(), ExtensionHostError> {
        let mut transaction = self.store.pool().begin().await?;
        sqlx::query(
            "DELETE FROM plugin_hook_subscriptions WHERE plugin_id = ? AND contribution_id = ?",
        )
        .bind(plugin.manifest.id.as_str())
        .bind(&contribution.id)
        .execute(&mut *transaction)
        .await?;
        if contribution.kind == PluginContributionKind::Hook
            && runtime.state == ContributionRuntimeState::Active
        {
            let (_, descriptor) = hook_descriptor(plugin, contribution)?;
            for event in descriptor.events {
                sqlx::query("INSERT INTO plugin_hook_subscriptions(plugin_id, contribution_id, event, runtime_revision, enabled, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?)")
                    .bind(plugin.manifest.id.as_str())
                    .bind(&contribution.id)
                    .bind(event.as_str())
                    .bind(&runtime.runtime_revision)
                    .bind(enabled)
                    .bind(now_ms())
                    .execute(&mut *transaction)
                    .await?;
            }
        }
        transaction.commit().await?;
        Ok(())
    }

    async fn invoke_hook(
        &self,
        subscription: &PluginHookSubscription,
        invocation: PluginHookInvocation,
        cancellation: CancellationToken,
    ) -> Result<PluginHookOutcome, &'static str> {
        let backend = self
            .hook_backend
            .read()
            .clone()
            .ok_or("plugin_hook_runtime_unavailable")?;
        let plugin_id = PluginId::new(subscription.plugin_id.clone());
        let plugin = self
            .get(&plugin_id)
            .await
            .map_err(|_| "plugin_hook_plugin_unavailable")?
            .ok_or("plugin_hook_plugin_unavailable")?;
        if plugin.status != PluginStatus::Enabled
            || hash_bundle(Path::new(&plugin.root_path))
                .map(|(hash, _)| hash != plugin.content_hash)
                .unwrap_or(true)
        {
            return Err("plugin_hook_content_drift");
        }
        let contribution = plugin
            .manifest
            .contributions
            .iter()
            .find(|contribution| {
                contribution.kind == PluginContributionKind::Hook
                    && contribution.id == subscription.contribution_id
            })
            .ok_or("plugin_hook_contribution_unavailable")?;
        let runtime = self
            .list_contributions(Some(&plugin_id))
            .await
            .map_err(|_| "plugin_hook_runtime_state_unavailable")?
            .into_iter()
            .find(|runtime| runtime.contribution_id == contribution.id)
            .ok_or("plugin_hook_runtime_state_unavailable")?;
        if runtime.state != ContributionRuntimeState::Active
            || runtime.runtime_revision != subscription.runtime_revision
        {
            return Err("plugin_hook_revision_drift");
        }
        let (bundle_root, descriptor) =
            hook_descriptor(&plugin, contribution).map_err(|_| "plugin_hook_descriptor_invalid")?;
        if !descriptor.events.contains(&invocation.event) {
            return Err("plugin_hook_event_not_declared");
        }
        let runtime_root = self.prepare_hook_acl_roots(&bundle_root)?;
        let executable = bundle_root
            .join(
                safe_relative_path(&descriptor.entrypoint)
                    .map_err(|_| "plugin_hook_entrypoint_escape")?,
            )
            .canonicalize()
            .map_err(|_| "plugin_hook_entrypoint_missing")?;
        if !executable.starts_with(&bundle_root) || !executable.is_file() {
            return Err("plugin_hook_entrypoint_escape");
        }
        execute_hook_sidecar(
            backend,
            bundle_root,
            runtime_root,
            executable,
            descriptor.args,
            invocation,
            cancellation,
        )
        .await
    }

    fn prepare_hook_acl_roots(&self, bundle_root: &Path) -> Result<PathBuf, &'static str> {
        let runtime_root = self.install_root.join(".hook-runtime");
        std::fs::create_dir_all(&runtime_root).map_err(|_| "plugin_hook_temp_create_failed")?;
        let runtime_root = runtime_root
            .canonicalize()
            .map_err(|_| "plugin_hook_temp_create_failed")?;
        let mut prepared = self.hook_acl_roots.lock();
        for (root, write, error_code) in [
            (bundle_root, false, "plugin_hook_bundle_acl_failed"),
            (runtime_root.as_path(), true, "plugin_hook_temp_acl_failed"),
        ] {
            if prepared.contains(root) {
                continue;
            }
            grant_restricted_code_access(root, write).map_err(|_| error_code)?;
            prepared.insert(root.to_path_buf());
        }
        drop(prepared);
        Ok(runtime_root)
    }
}

impl PluginHookRuntime for PluginHost {
    fn invoke<'a>(
        &'a self,
        subscription: &'a PluginHookSubscription,
        invocation: PluginHookInvocation,
        cancellation: CancellationToken,
    ) -> PluginHookRuntimeFuture<'a> {
        Box::pin(async move {
            self.invoke_hook(subscription, invocation, cancellation)
                .await
                .map_err(str::to_owned)
        })
    }
}

pub(super) fn hook_descriptor(
    plugin: &InstalledPlugin,
    contribution: &PluginContribution,
) -> Result<(PathBuf, PluginHookDescriptor), ExtensionHostError> {
    let root = PathBuf::from(&plugin.root_path)
        .canonicalize()
        .map_err(|_| ExtensionHostError::ContributionDrift)?;
    let target = root
        .join(safe_relative_path(&contribution.relative_path)?)
        .canonicalize()
        .map_err(|_| ExtensionHostError::ContributionDrift)?;
    if !target.starts_with(&root) || !target.is_file() {
        return Err(ExtensionHostError::ContributionEscape);
    }
    let descriptor: PluginHookDescriptor = serde_json::from_slice(&std::fs::read(target)?)?;
    let mut events = descriptor.events.clone();
    events.sort_unstable();
    events.dedup();
    let valid = descriptor.protocol_version == 1
        && descriptor.runtime == PluginHookRuntimeKind::SandboxedStdioJsonRpc
        && !descriptor.entrypoint.trim().is_empty()
        && !descriptor.events.is_empty()
        && events.len() == descriptor.events.len()
        && descriptor.args.len() <= MAX_HOOK_ARGS
        && descriptor
            .args
            .iter()
            .all(|argument| argument.len() <= 4_096 && !argument.contains('\0'));
    if !valid {
        return Err(ExtensionHostError::InvalidManifest(
            "plugin hook v1 descriptor is invalid".into(),
        ));
    }
    Ok((root, descriptor))
}

async fn execute_hook_sidecar(
    backend: Arc<dyn SandboxBackend>,
    bundle_root: PathBuf,
    runtime_root: PathBuf,
    executable: PathBuf,
    args: Vec<String>,
    invocation: PluginHookInvocation,
    cancellation: CancellationToken,
) -> Result<PluginHookOutcome, &'static str> {
    let id = uuid::Uuid::now_v7().to_string();
    let mut input = serde_json::to_vec(&HookRequest {
        jsonrpc: "2.0",
        id: &id,
        method: "hook.invoke",
        params: &invocation,
    })
    .map_err(|_| "plugin_hook_request_invalid")?;
    input.push(b'\n');
    let temporary = tempfile::Builder::new()
        .prefix("hachimi-hook-sidecar-")
        .tempdir_in(runtime_root)
        .map_err(|_| "plugin_hook_temp_create_failed")?;
    let session_id = invocation
        .session_id
        .clone()
        .unwrap_or_else(SessionId::random);
    let run_id = invocation.run_id.clone().unwrap_or_else(RunId::random);
    let grant_session_id = session_id.clone();
    let grant_run_id = run_id.clone();
    let temporary_root = temporary.path().to_string_lossy().into_owned();
    let bundle_path = bundle_root.to_string_lossy().into_owned();
    let executable_path = executable.to_string_lossy().into_owned();
    let child = backend
        .spawn_restricted(
            SandboxLaunchSpec {
                session_id,
                run_id,
                run_generation: invocation.run_generation.unwrap_or(1),
                checkout_id: CheckoutId::random(),
                checkout_root: temporary.path().to_path_buf(),
                grants: CapabilityGrantSet {
                    profile: PermissionProfile::WorkspaceWrite,
                    scope: PermissionGrantScope::Run,
                    session_id: grant_session_id,
                    run_id: Some(grant_run_id),
                    source: "plugin_hook_sidecar_runtime".into(),
                    file_system: vec![
                        FileSystemGrant {
                            access: FileSystemAccess::Read,
                            roots: vec![bundle_path, temporary_root.clone()],
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
                        allowed_commands: vec![executable_path],
                    },
                    browser: Default::default(),
                    computer: ComputerGrant::default(),
                    review_each_command: false,
                    expires_at_ms: None,
                },
                required_effect: ToolEffect::Process,
                executable,
                args: args.into_iter().map(OsString::from).collect(),
                cwd: temporary.path().to_path_buf(),
                environment: restricted_environment(temporary.path()),
                stdin: Some(input),
                interactive_stdin: false,
                timeout: HOOK_TIMEOUT,
                output_limit: HOOK_OUTPUT_LIMIT,
                network_policy: SandboxNetworkPolicy::DenyAll,
            },
            cancellation,
        )
        .await
        .map_err(|_| "plugin_hook_sandbox_spawn_failed")?;
    let output = child
        .wait()
        .await
        .map_err(|_| "plugin_hook_process_failed")?;
    if output.exit_code != Some(0) || output.truncated {
        return Err("plugin_hook_invalid_exit");
    }
    let response: HookResponse =
        serde_json::from_slice(&output.stdout).map_err(|_| "plugin_hook_response_invalid")?;
    if response.jsonrpc != "2.0" || response.id != id || response.error.is_some() {
        return Err("plugin_hook_response_rejected");
    }
    response.result.ok_or("plugin_hook_result_missing")
}

#[derive(Serialize)]
struct HookRequest<'a> {
    jsonrpc: &'static str,
    id: &'a str,
    method: &'static str,
    params: &'a PluginHookInvocation,
}

#[derive(Deserialize)]
struct HookResponse {
    jsonrpc: String,
    id: String,
    result: Option<PluginHookOutcome>,
    error: Option<serde_json::Value>,
}

fn restricted_environment(temporary: &Path) -> Vec<(OsString, OsString)> {
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
