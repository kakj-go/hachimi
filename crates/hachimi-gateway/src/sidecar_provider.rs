use std::{
    ffi::OsString,
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Duration,
};

use hachimi_protocol::{
    CapabilityGrantSet, ChannelProviderAccount, ChannelProviderHealth, ChannelProviderHealthState,
    ChannelProviderManifest, CheckoutId, ComputerGrant, DeliveryAttempt, FileSystemAccess,
    FileSystemGrant, NetworkGrant, PermissionGrantScope, PermissionProfile, ProcessGrant, RunId,
    SessionId, ToolEffect, VerifiedChannelMessage,
};
use hachimi_sandbox::{
    SandboxBackend, SandboxLaunchSpec, SandboxNetworkPolicy, grant_restricted_code_access,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroize;

use crate::{ChannelDeliveryOutcome, ChannelProvider, ChannelProviderFuture, GatewayError};

const SIDECAR_TIMEOUT: Duration = Duration::from_secs(60);
const SIDECAR_OUTPUT_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Clone)]
pub struct SandboxedStdioChannelProvider {
    backend: Arc<dyn SandboxBackend>,
    manifest: ChannelProviderManifest,
    bundle_root: PathBuf,
    executable: PathBuf,
    args: Vec<OsString>,
    account: Arc<RwLock<Option<ChannelProviderAccount>>>,
    state: Arc<RwLock<ChannelProviderHealthState>>,
}

impl std::fmt::Debug for SandboxedStdioChannelProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SandboxedStdioChannelProvider")
            .field("provider_id", &self.manifest.id)
            .field("bundle_root", &self.bundle_root)
            .field("executable", &self.executable)
            .finish_non_exhaustive()
    }
}

impl SandboxedStdioChannelProvider {
    pub fn new(
        backend: Arc<dyn SandboxBackend>,
        manifest: ChannelProviderManifest,
        bundle_root: PathBuf,
        executable: PathBuf,
        args: Vec<String>,
    ) -> Result<Self, GatewayError> {
        let bundle_root = bundle_root
            .canonicalize()
            .map_err(|_| GatewayError::Sidecar("bundle_root_missing"))?;
        let executable = executable
            .canonicalize()
            .map_err(|_| GatewayError::Sidecar("entrypoint_missing"))?;
        if manifest.id.trim().is_empty()
            || manifest.plugin_id.is_none()
            || manifest.runtime_kind
                != hachimi_protocol::ChannelProviderRuntimeKind::SandboxedStdioJsonRpc
            || !executable.starts_with(&bundle_root)
            || !executable.is_file()
        {
            return Err(GatewayError::InvalidProvider);
        }
        Ok(Self {
            backend,
            manifest,
            bundle_root,
            executable,
            args: args.into_iter().map(OsString::from).collect(),
            account: Arc::new(RwLock::new(None)),
            state: Arc::new(RwLock::new(ChannelProviderHealthState::Disabled)),
        })
    }

    async fn call(
        &self,
        method: &'static str,
        account: &ChannelProviderAccount,
        transport_credential: Option<&str>,
        message: Option<&VerifiedChannelMessage>,
        delivery: Option<&DeliveryAttempt>,
    ) -> Result<Value, GatewayError> {
        let mut credential = match transport_credential {
            Some(value) => Some(value.to_owned()),
            None => channel_secret(account)?,
        };
        let id = uuid::Uuid::now_v7().to_string();
        let wire = SidecarRequest {
            jsonrpc: "2.0",
            id: &id,
            method,
            params: SidecarParams {
                account: SidecarAccount {
                    id: &account.id,
                    provider_id: &account.provider_id,
                    display_name: &account.display_name,
                    config_revision: account.config_revision,
                    tenant_key: &account.tenant_key,
                    config: &account.config,
                },
                credential: credential.as_deref(),
                message,
                delivery,
            },
        };
        let mut input = serde_json::to_vec(&wire)?;
        input.push(b'\n');
        if let Some(value) = &mut credential {
            value.zeroize();
        }

        let temporary = tempfile::Builder::new()
            .prefix("hachimi-channel-sidecar-")
            .tempdir()
            .map_err(|_| GatewayError::Sidecar("temp_create_failed"))?;
        grant_restricted_code_access(&self.bundle_root, false)
            .map_err(|_| GatewayError::Sidecar("bundle_acl_failed"))?;
        grant_restricted_code_access(temporary.path(), true)
            .map_err(|_| GatewayError::Sidecar("temp_acl_failed"))?;
        let session_id = SessionId::random();
        let run_id = RunId::random();
        let temporary_root = temporary.path().to_string_lossy().into_owned();
        let bundle_root = self.bundle_root.to_string_lossy().into_owned();
        let executable = self.executable.to_string_lossy().into_owned();
        let grants = CapabilityGrantSet {
            profile: PermissionProfile::Writable,
            scope: PermissionGrantScope::Run,
            session_id: session_id.clone(),
            run_id: Some(run_id.clone()),
            source: "plugin_channel_sidecar_runtime".into(),
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
            .map_err(|_| GatewayError::Sidecar("sandbox_spawn_failed"))?;
        let output = child
            .wait()
            .await
            .map_err(|_| GatewayError::Sidecar("process_failed"))?;
        if output.exit_code != Some(0) || output.truncated {
            return Err(GatewayError::Sidecar("invalid_exit"));
        }
        let response: SidecarResponse = serde_json::from_slice(&output.stdout)
            .map_err(|_| GatewayError::Sidecar("response_invalid"))?;
        if response.jsonrpc != "2.0" || response.id != id || response.error.is_some() {
            return Err(GatewayError::Sidecar("response_rejected"));
        }
        response
            .result
            .ok_or(GatewayError::Sidecar("result_missing"))
    }

    fn configured_account(&self) -> Result<ChannelProviderAccount, GatewayError> {
        self.account
            .read()
            .map_err(|_| GatewayError::ProviderStatePoisoned)?
            .clone()
            .ok_or(GatewayError::ProviderUnavailable)
    }

    fn set_state(&self, state: ChannelProviderHealthState) -> Result<(), GatewayError> {
        *self
            .state
            .write()
            .map_err(|_| GatewayError::ProviderStatePoisoned)? = state;
        Ok(())
    }
}

impl ChannelProvider for SandboxedStdioChannelProvider {
    fn manifest(&self) -> ChannelProviderManifest {
        self.manifest.clone()
    }

    fn configure<'a>(
        &'a self,
        account: &'a ChannelProviderAccount,
    ) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move {
            if account.provider_id != self.manifest.id || account.tenant_key.trim().is_empty() {
                return Err(GatewayError::InvalidProvider);
            }
            self.call("configure", account, None, None, None).await?;
            *self
                .account
                .write()
                .map_err(|_| GatewayError::ProviderStatePoisoned)? = Some(account.clone());
            self.set_state(if account.enabled {
                ChannelProviderHealthState::Starting
            } else {
                ChannelProviderHealthState::Disabled
            })
        })
    }

    fn start<'a>(&'a self) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move {
            let account = self.configured_account()?;
            self.call("start", &account, None, None, None).await?;
            self.set_state(ChannelProviderHealthState::Starting)
        })
    }

    fn stop<'a>(&'a self) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move {
            if let Ok(account) = self.configured_account() {
                self.call("stop", &account, None, None, None).await?;
            }
            self.set_state(ChannelProviderHealthState::Disabled)
        })
    }

    fn health<'a>(&'a self) -> ChannelProviderFuture<'a, ChannelProviderHealth> {
        Box::pin(async move {
            let account = match self.configured_account() {
                Ok(account) => account,
                Err(_) => {
                    return Ok(ChannelProviderHealth {
                        provider_id: self.manifest.id.clone(),
                        account_id: None,
                        state: ChannelProviderHealthState::Disabled,
                        diagnostic: None,
                        last_event_at_ms: None,
                        last_delivery_at_ms: None,
                        last_handshake_at_ms: None,
                        last_frame_at_ms: None,
                        last_error_code: None,
                        next_reconnect_at_ms: None,
                        consecutive_failures: 0,
                        config_revision: 0,
                    });
                }
            };
            match self.call("health", &account, None, None, None).await {
                Ok(result) if result.get("state").and_then(Value::as_str) == Some("healthy") => {
                    self.set_state(ChannelProviderHealthState::Healthy)?;
                    Ok(ChannelProviderHealth {
                        provider_id: self.manifest.id.clone(),
                        account_id: Some(account.id.clone()),
                        state: ChannelProviderHealthState::Healthy,
                        diagnostic: None,
                        last_event_at_ms: None,
                        last_delivery_at_ms: None,
                        last_handshake_at_ms: Some(crate::now_ms()),
                        last_frame_at_ms: None,
                        last_error_code: None,
                        next_reconnect_at_ms: None,
                        consecutive_failures: 0,
                        config_revision: account.config_revision,
                    })
                }
                Ok(_) => {
                    self.set_state(ChannelProviderHealthState::NeedsAttention)?;
                    Ok(ChannelProviderHealth {
                        provider_id: self.manifest.id.clone(),
                        account_id: Some(account.id.clone()),
                        state: ChannelProviderHealthState::NeedsAttention,
                        diagnostic: Some("channel_sidecar_health_rejected".into()),
                        last_event_at_ms: None,
                        last_delivery_at_ms: None,
                        last_handshake_at_ms: None,
                        last_frame_at_ms: None,
                        last_error_code: Some("channel_sidecar_health_rejected".into()),
                        next_reconnect_at_ms: None,
                        consecutive_failures: 1,
                        config_revision: account.config_revision,
                    })
                }
                Err(error) => {
                    self.set_state(ChannelProviderHealthState::Failed)?;
                    Ok(ChannelProviderHealth {
                        provider_id: self.manifest.id.clone(),
                        account_id: Some(account.id.clone()),
                        state: ChannelProviderHealthState::Failed,
                        diagnostic: Some(error.to_string()),
                        last_event_at_ms: None,
                        last_delivery_at_ms: None,
                        last_handshake_at_ms: None,
                        last_frame_at_ms: None,
                        last_error_code: Some("channel_sidecar_unavailable".into()),
                        next_reconnect_at_ms: None,
                        consecutive_failures: 1,
                        config_revision: account.config_revision,
                    })
                }
            }
        })
    }

    fn accept_verified<'a>(
        &'a self,
        credential: Option<&'a str>,
        message: VerifiedChannelMessage,
    ) -> ChannelProviderFuture<'a, VerifiedChannelMessage> {
        Box::pin(async move {
            let account = self.configured_account()?;
            let result = self
                .call(
                    "accept_verified",
                    &account,
                    credential,
                    Some(&message),
                    None,
                )
                .await?;
            serde_json::from_value(result).map_err(GatewayError::from)
        })
    }

    fn deliver<'a>(
        &'a self,
        attempt: &'a DeliveryAttempt,
    ) -> ChannelProviderFuture<'a, ChannelDeliveryOutcome> {
        Box::pin(async move {
            let account = self.configured_account()?;
            let result = self
                .call("deliver", &account, None, None, Some(attempt))
                .await?;
            Ok(ChannelDeliveryOutcome {
                delivered: result
                    .get("delivered")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                retryable: result
                    .get("retryable")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                indeterminate: result
                    .get("indeterminate")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                result_code: result
                    .get("resultCode")
                    .and_then(Value::as_str)
                    .unwrap_or("channel_sidecar_delivery_rejected")
                    .to_owned(),
                provider_receipt: result
                    .get("providerReceipt")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        })
    }

    fn ack_delivery<'a>(&'a self, delivery: &'a DeliveryAttempt) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move {
            let account = self.configured_account()?;
            self.call("ack", &account, None, None, Some(delivery))
                .await?;
            Ok(())
        })
    }

    fn reload<'a>(&'a self, account: &'a ChannelProviderAccount) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move {
            if account.provider_id != self.manifest.id || account.tenant_key.trim().is_empty() {
                return Err(GatewayError::InvalidProvider);
            }
            self.call("reload", account, None, None, None).await?;
            *self
                .account
                .write()
                .map_err(|_| GatewayError::ProviderStatePoisoned)? = Some(account.clone());
            self.set_state(if account.enabled {
                ChannelProviderHealthState::Starting
            } else {
                ChannelProviderHealthState::Disabled
            })
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
    account: SidecarAccount<'a>,
    credential: Option<&'a str>,
    message: Option<&'a VerifiedChannelMessage>,
    delivery: Option<&'a DeliveryAttempt>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SidecarAccount<'a> {
    id: &'a str,
    provider_id: &'a str,
    display_name: &'a str,
    config_revision: u64,
    tenant_key: &'a str,
    config: &'a Value,
}

#[derive(Deserialize)]
struct SidecarResponse {
    jsonrpc: String,
    id: String,
    result: Option<Value>,
    error: Option<Value>,
}

fn channel_secret(account: &ChannelProviderAccount) -> Result<Option<String>, GatewayError> {
    let Some(reference) = account.credential_ref.as_deref() else {
        return Ok(None);
    };
    let expected = format!(
        "keyring:channel:{}:{}:primary",
        account.provider_id, account.id
    );
    if reference != expected {
        return Err(GatewayError::ProviderCredentialUnavailable);
    }
    let entry = keyring::Entry::new(
        "com.hachimi.channel",
        &format!("{}:{}", account.provider_id, account.id),
    )
    .map_err(|_| GatewayError::ProviderCredentialUnavailable)?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Err(GatewayError::ProviderCredentialUnavailable),
        Err(_) => Err(GatewayError::ProviderCredentialUnavailable),
    }
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
