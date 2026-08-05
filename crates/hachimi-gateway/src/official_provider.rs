use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};

use hachimi_channel_providers::{
    AccountRuntime, AccountRuntimeConfig, IlinkMediaKind, ProviderAdapter, ProviderError,
    ProviderEventFrame, TransportProof, WechatIlinkAdapter, WechatIlinkClient,
    WecomAiBotDeliveryResult, WecomAiBotMediaKind, WecomAiBotTransport, WecomAiBotTransportEvent,
    official_adapters,
};
use hachimi_enterprise::{
    EnterpriseApiClient, EnterpriseApiError, EnterpriseCredential, EnterpriseMediaInput,
    EnterpriseMediaKind, EnterpriseMessageTarget, EnterpriseStreamEvent, spawn_enterprise_stream,
};
use hachimi_protocol::{
    ChannelConversationAddress, ChannelMessagePart, ChannelProviderAccount, ChannelProviderHealth,
    ChannelProviderHealthState, ChannelProviderManifest, ChannelProviderRuntimeKind,
    DeliveryAttempt, IntegrationProviderId, RemoteMediaDescriptor, VerifiedChannelMessage,
};
use hachimi_storage::AgentStore;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

use crate::{
    ChannelDeliveryOutcome, ChannelProvider, ChannelProviderFuture, ChannelProviderRegistry,
    GatewayError,
};

#[derive(Clone)]
pub struct OfficialChannelProvider {
    adapter: Arc<dyn ProviderAdapter>,
    store: AgentStore,
    accounts: Arc<RwLock<BTreeMap<String, AccountRuntime>>>,
    pollers: Arc<Mutex<BTreeMap<String, tokio::task::JoinHandle<()>>>>,
    wecom_ai_transports: Arc<RwLock<BTreeMap<String, Arc<WecomAiBotTransport>>>>,
}

impl std::fmt::Debug for OfficialChannelProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OfficialChannelProvider")
            .field("provider_id", &self.adapter.provider_id())
            .finish_non_exhaustive()
    }
}

impl OfficialChannelProvider {
    #[must_use]
    pub fn new(adapter: Arc<dyn ProviderAdapter>, store: AgentStore) -> Self {
        Self {
            adapter,
            store,
            accounts: Arc::new(RwLock::new(BTreeMap::new())),
            pollers: Arc::new(Mutex::new(BTreeMap::new())),
            wecom_ai_transports: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }

    fn runtime(&self, account_id: &str) -> Result<AccountRuntime, GatewayError> {
        self.accounts
            .read()
            .map_err(|_| GatewayError::ProviderStatePoisoned)?
            .get(account_id)
            .cloned()
            .ok_or(GatewayError::ProviderUnavailable)
    }

    fn start_ilink_poller(&self, runtime: AccountRuntime) -> Result<(), GatewayError> {
        let config = runtime
            .config()
            .map_err(|_| GatewayError::ProviderUnavailable)?;
        let mut pollers = self
            .pollers
            .lock()
            .map_err(|_| GatewayError::ProviderStatePoisoned)?;
        if pollers
            .get(&config.account_id)
            .is_some_and(|poller| !poller.is_finished())
        {
            return Ok(());
        }
        if let Some(stale) = pollers.remove(&config.account_id) {
            stale.abort();
        }
        let credential = load_ilink_credential(&config)?;
        let client = WechatIlinkClient::authenticated(&credential.base_url, credential.bot_token)
            .map_err(|_| GatewayError::ProviderCredentialUnavailable)?;
        if credential.bot_id != config.tenant_key {
            return Err(GatewayError::ProviderCredentialUnavailable);
        }
        let account_id = config.account_id.clone();
        let store = self.store.clone();
        let task = tokio::spawn(async move {
            run_ilink_poller(store, runtime, client, credential.bot_id).await;
        });
        pollers.insert(account_id, task);
        Ok(())
    }

    fn start_enterprise_stream(&self, runtime: AccountRuntime) -> Result<(), GatewayError> {
        let config = runtime
            .config()
            .map_err(|_| GatewayError::ProviderUnavailable)?;
        let mut pollers = self
            .pollers
            .lock()
            .map_err(|_| GatewayError::ProviderStatePoisoned)?;
        if pollers
            .get(&config.account_id)
            .is_some_and(|poller| !poller.is_finished())
        {
            return Ok(());
        }
        if let Some(stale) = pollers.remove(&config.account_id) {
            stale.abort();
        }
        let credential = load_enterprise_credential(&config)?;
        let api = EnterpriseApiClient::new().map_err(|_| GatewayError::ProviderUnavailable)?;
        let (transport, mut events) = spawn_enterprise_stream(api, credential);
        let account_id = config.account_id.clone();
        let task = tokio::spawn(async move {
            let _transport = transport;
            while let Some(event) = events.recv().await {
                if let Ok(frame) = enterprise_event_frame(&runtime, event) {
                    if runtime.accept_frame(frame).is_err() {
                        let _ = runtime.record_transport_failure();
                    } else {
                        let _ = runtime.record_transport_success();
                    }
                }
            }
            let _ = runtime.record_transport_failure();
        });
        pollers.insert(account_id, task);
        Ok(())
    }

    fn start_wecom_ai_bot(&self, runtime: AccountRuntime) -> Result<(), GatewayError> {
        let config = runtime
            .config()
            .map_err(|_| GatewayError::ProviderUnavailable)?;
        let mut pollers = self
            .pollers
            .lock()
            .map_err(|_| GatewayError::ProviderStatePoisoned)?;
        if pollers
            .get(&config.account_id)
            .is_some_and(|poller| !poller.is_finished())
        {
            return Ok(());
        }
        if let Some(stale) = pollers.remove(&config.account_id) {
            stale.abort();
        }
        self.wecom_ai_transports
            .write()
            .map_err(|_| GatewayError::ProviderStatePoisoned)?
            .remove(&config.account_id);
        let credential = load_wecom_ai_credential(&config)?;
        let (transport, mut events) =
            WecomAiBotTransport::spawn(credential.bot_id, credential.secret)
                .map_err(|_| GatewayError::ProviderCredentialUnavailable)?;
        let transport = Arc::new(transport);
        let task_transport = Arc::clone(&transport);
        let store = self.store.clone();
        let account_id = config.account_id.clone();
        let task = tokio::spawn(async move {
            let _transport = task_transport;
            while let Some(event) = events.recv().await {
                match event {
                    WecomAiBotTransportEvent::Message {
                        payload,
                        connection_id,
                        received_at_ms,
                    } => {
                        let Ok(config) = runtime.config() else {
                            break;
                        };
                        if runtime
                            .accept_frame(ProviderEventFrame {
                                account_id: config.account_id,
                                tenant_key: config.tenant_key,
                                payload,
                                proof: TransportProof::Stream {
                                    connection_id,
                                    received_at_ms,
                                },
                            })
                            .is_ok()
                        {
                            let _ = runtime.record_transport_success();
                        }
                    }
                    WecomAiBotTransportEvent::Degraded => {
                        let _ = runtime.record_transport_failure();
                    }
                    WecomAiBotTransportEvent::AuthenticationExpired => {
                        mark_provider_account_attention(&store, &runtime).await;
                        break;
                    }
                }
            }
        });
        self.wecom_ai_transports
            .write()
            .map_err(|_| GatewayError::ProviderStatePoisoned)?
            .insert(account_id.clone(), transport);
        pollers.insert(account_id, task);
        Ok(())
    }
}

pub(crate) fn register_official_providers(
    registry: &ChannelProviderRegistry,
    store: &AgentStore,
) -> Result<(), GatewayError> {
    for adapter in official_adapters(None) {
        let adapter: Arc<dyn ProviderAdapter> = Arc::from(adapter);
        registry.register(Arc::new(OfficialChannelProvider::new(
            adapter,
            store.clone(),
        )))?;
    }
    Ok(())
}

impl ChannelProvider for OfficialChannelProvider {
    fn manifest(&self) -> ChannelProviderManifest {
        let id = self.adapter.provider_id().as_str();
        ChannelProviderManifest {
            id: id.into(),
            plugin_id: None,
            runtime_kind: ChannelProviderRuntimeKind::Builtin,
            entrypoint: None,
            content_hash: format!("builtin:{id}:channel-provider-v2"),
            required_scopes: vec!["channels.receive".into(), "channels.deliver".into()],
        }
    }

    fn configure<'a>(
        &'a self,
        account: &'a ChannelProviderAccount,
    ) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move {
            let provider_id = parse_provider_id(&account.provider_id)?;
            if provider_id != self.adapter.provider_id() || account.tenant_key.trim().is_empty() {
                return Err(GatewayError::InvalidProvider);
            }
            let config = AccountRuntimeConfig {
                account_id: account.id.clone(),
                provider_id,
                tenant_key: account.tenant_key.clone(),
                credential_ref: account.credential_ref.clone().unwrap_or_default(),
                config: account.config.clone(),
                config_revision: account.config_revision,
            };
            let mut accounts = self
                .accounts
                .write()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?;
            if let Some(runtime) = accounts.get(&account.id) {
                let config_changed = runtime
                    .config()
                    .map_err(|_| GatewayError::ProviderUnavailable)?
                    != config;
                runtime
                    .reload(config)
                    .map_err(|_| GatewayError::ProviderRevisionConflict)?;
                if config_changed
                    && matches!(
                        self.adapter.provider_id(),
                        IntegrationProviderId::WechatIlink
                            | IntegrationProviderId::DingTalk
                            | IntegrationProviderId::Feishu
                            | IntegrationProviderId::WecomAiBot
                    )
                    && let Some(poller) = self
                        .pollers
                        .lock()
                        .map_err(|_| GatewayError::ProviderStatePoisoned)?
                        .remove(&account.id)
                {
                    poller.abort();
                    self.wecom_ai_transports
                        .write()
                        .map_err(|_| GatewayError::ProviderStatePoisoned)?
                        .remove(&account.id);
                }
                if account.enabled && account_state_can_run(account.state) {
                    runtime
                        .start()
                        .map_err(|_| GatewayError::ProviderUnavailable)?;
                } else if account.state == hachimi_protocol::ChannelAccountState::AwaitingAuth {
                    runtime
                        .mark_authentication_expired()
                        .map_err(|_| GatewayError::ProviderUnavailable)?;
                } else if account.state == hachimi_protocol::ChannelAccountState::NeedsAttention {
                    runtime
                        .mark_needs_attention()
                        .map_err(|_| GatewayError::ProviderUnavailable)?;
                } else {
                    runtime
                        .stop()
                        .map_err(|_| GatewayError::ProviderUnavailable)?;
                }
            } else {
                let runtime = AccountRuntime::new(self.adapter.clone(), config)
                    .map_err(|_| GatewayError::InvalidProvider)?;
                if account.enabled && account_state_can_run(account.state) {
                    runtime
                        .start()
                        .map_err(|_| GatewayError::ProviderUnavailable)?;
                } else if account.state == hachimi_protocol::ChannelAccountState::AwaitingAuth {
                    runtime
                        .mark_authentication_expired()
                        .map_err(|_| GatewayError::ProviderUnavailable)?;
                } else if account.state == hachimi_protocol::ChannelAccountState::NeedsAttention {
                    runtime
                        .mark_needs_attention()
                        .map_err(|_| GatewayError::ProviderUnavailable)?;
                }
                accounts.insert(account.id.clone(), runtime);
            }
            Ok(())
        })
    }

    fn start<'a>(&'a self) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move {
            for runtime in self
                .accounts
                .read()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?
                .values()
            {
                runtime
                    .start()
                    .map_err(|_| GatewayError::ProviderUnavailable)?;
            }
            Ok(())
        })
    }

    fn start_ingress<'a>(&'a self) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move {
            if !matches!(
                self.adapter.provider_id(),
                IntegrationProviderId::WechatIlink
                    | IntegrationProviderId::DingTalk
                    | IntegrationProviderId::Feishu
                    | IntegrationProviderId::WecomAiBot
            ) {
                return Ok(());
            }
            let runtimes = self
                .accounts
                .read()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?
                .values()
                .cloned()
                .collect::<Vec<_>>();
            for runtime in runtimes {
                if runtime
                    .snapshot()
                    .is_ok_and(|snapshot| snapshot.health == ChannelProviderHealthState::Healthy)
                {
                    let result = match self.adapter.provider_id() {
                        IntegrationProviderId::WechatIlink => {
                            self.start_ilink_poller(runtime.clone())
                        }
                        IntegrationProviderId::DingTalk | IntegrationProviderId::Feishu => {
                            self.start_enterprise_stream(runtime.clone())
                        }
                        IntegrationProviderId::WecomAiBot => {
                            self.start_wecom_ai_bot(runtime.clone())
                        }
                        _ => Ok(()),
                    };
                    if result.is_err() {
                        mark_provider_account_attention(&self.store, &runtime).await;
                    }
                }
            }
            Ok(())
        })
    }

    fn stop<'a>(&'a self) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move {
            for (_, poller) in self
                .pollers
                .lock()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?
                .split_off("")
            {
                poller.abort();
            }
            self.wecom_ai_transports
                .write()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?
                .clear();
            for runtime in self
                .accounts
                .read()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?
                .values()
            {
                runtime
                    .stop()
                    .map_err(|_| GatewayError::ProviderUnavailable)?;
            }
            Ok(())
        })
    }

    fn health<'a>(&'a self) -> ChannelProviderFuture<'a, ChannelProviderHealth> {
        Box::pin(async move {
            let accounts = self
                .accounts
                .read()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?;
            let snapshots = accounts
                .values()
                .filter_map(|runtime| runtime.snapshot().ok())
                .collect::<Vec<_>>();
            let state = if snapshots.is_empty() {
                ChannelProviderHealthState::Disabled
            } else if snapshots
                .iter()
                .any(|snapshot| snapshot.health == ChannelProviderHealthState::NeedsAttention)
            {
                ChannelProviderHealthState::NeedsAttention
            } else if snapshots
                .iter()
                .all(|snapshot| snapshot.health == ChannelProviderHealthState::Healthy)
            {
                ChannelProviderHealthState::Healthy
            } else {
                ChannelProviderHealthState::Degraded
            };
            Ok(ChannelProviderHealth {
                provider_id: self.adapter.provider_id().as_str().into(),
                account_id: None,
                state,
                diagnostic: None,
                last_event_at_ms: snapshots
                    .iter()
                    .filter_map(|snapshot| snapshot.last_event_at_ms)
                    .max(),
                last_delivery_at_ms: snapshots
                    .iter()
                    .filter_map(|snapshot| snapshot.last_delivery_at_ms)
                    .max(),
                last_handshake_at_ms: snapshots
                    .iter()
                    .filter_map(|snapshot| snapshot.last_handshake_at_ms)
                    .max(),
                last_frame_at_ms: snapshots
                    .iter()
                    .filter_map(|snapshot| snapshot.last_frame_at_ms)
                    .max(),
                last_error_code: snapshots
                    .iter()
                    .find_map(|snapshot| snapshot.last_error_code.clone()),
                next_reconnect_at_ms: snapshots
                    .iter()
                    .filter_map(|snapshot| snapshot.next_reconnect_at_ms)
                    .min(),
                consecutive_failures: snapshots
                    .iter()
                    .map(|snapshot| snapshot.consecutive_failures)
                    .max()
                    .unwrap_or(0),
                config_revision: snapshots
                    .iter()
                    .map(|snapshot| snapshot.config_revision)
                    .max()
                    .unwrap_or(0),
            })
        })
    }

    fn account_health<'a>(&'a self) -> ChannelProviderFuture<'a, Vec<ChannelProviderHealth>> {
        Box::pin(async move {
            let accounts = self
                .accounts
                .read()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?;
            accounts
                .values()
                .map(|runtime| {
                    let config = runtime
                        .config()
                        .map_err(|_| GatewayError::ProviderUnavailable)?;
                    let snapshot = runtime
                        .snapshot()
                        .map_err(|_| GatewayError::ProviderUnavailable)?;
                    Ok(ChannelProviderHealth {
                        provider_id: config.provider_id.as_str().into(),
                        account_id: Some(config.account_id),
                        state: snapshot.health,
                        diagnostic: None,
                        last_event_at_ms: snapshot.last_event_at_ms,
                        last_delivery_at_ms: snapshot.last_delivery_at_ms,
                        last_handshake_at_ms: snapshot.last_handshake_at_ms,
                        last_frame_at_ms: snapshot.last_frame_at_ms,
                        last_error_code: snapshot.last_error_code,
                        next_reconnect_at_ms: snapshot.next_reconnect_at_ms,
                        consecutive_failures: snapshot.consecutive_failures,
                        config_revision: snapshot.config_revision,
                    })
                })
                .collect()
        })
    }

    fn accept_verified<'a>(
        &'a self,
        _credential: Option<&'a str>,
        mut message: VerifiedChannelMessage,
    ) -> ChannelProviderFuture<'a, VerifiedChannelMessage> {
        Box::pin(async move {
            let runtime = self.runtime(&message.address.account_id)?;
            let snapshot = runtime
                .snapshot()
                .map_err(|_| GatewayError::ProviderUnavailable)?;
            if snapshot.health != ChannelProviderHealthState::Healthy
                || message.address.provider_id != self.adapter.provider_id().as_str()
            {
                return Err(GatewayError::ProviderUnavailable);
            }
            persist_media_secrets(&self.store, &mut message).await?;
            Ok(message)
        })
    }

    fn claim_ingress<'a>(&'a self) -> ChannelProviderFuture<'a, Option<VerifiedChannelMessage>> {
        Box::pin(async move {
            for runtime in self
                .accounts
                .read()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?
                .values()
            {
                if let Some(message) = runtime
                    .next_event()
                    .map_err(|_| GatewayError::ProviderUnavailable)?
                {
                    return Ok(Some(message));
                }
            }
            Ok(None)
        })
    }

    fn deliver<'a>(
        &'a self,
        attempt: &'a DeliveryAttempt,
    ) -> ChannelProviderFuture<'a, ChannelDeliveryOutcome> {
        Box::pin(async move {
            let runtime = self.runtime(&attempt.address.account_id)?;
            if self.adapter.provider_id() == IntegrationProviderId::WechatIlink {
                return deliver_ilink(&self.store, &runtime, attempt).await;
            }
            if matches!(
                self.adapter.provider_id(),
                IntegrationProviderId::DingTalk
                    | IntegrationProviderId::Feishu
                    | IntegrationProviderId::WecomApp
            ) {
                return deliver_enterprise(&self.store, &runtime, attempt).await;
            }
            if self.adapter.provider_id() == IntegrationProviderId::WecomAiBot {
                return deliver_wecom_ai_bot(self, &runtime, attempt).await;
            }
            runtime
                .deliver(&attempt.payload, crate::now_ms())
                .map_err(|_| GatewayError::ProviderUnavailable)?;
            Ok(ChannelDeliveryOutcome {
                delivered: true,
                retryable: false,
                indeterminate: false,
                result_code: "provider_delivery_accepted".into(),
                provider_receipt: None,
            })
        })
    }

    fn ack_delivery<'a>(&'a self, _delivery: &'a DeliveryAttempt) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn remove_account<'a>(&'a self, account_id: &'a str) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move {
            if let Some(poller) = self
                .pollers
                .lock()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?
                .remove(account_id)
            {
                poller.abort();
            }
            self.wecom_ai_transports
                .write()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?
                .remove(account_id);
            if let Some(runtime) = self
                .accounts
                .write()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?
                .remove(account_id)
            {
                runtime
                    .stop()
                    .map_err(|_| GatewayError::ProviderUnavailable)?;
            }
            Ok(())
        })
    }
}

async fn persist_media_secrets(
    store: &AgentStore,
    message: &mut VerifiedChannelMessage,
) -> Result<(), GatewayError> {
    let secrets = take_media_secrets(message)?;
    if secrets.is_empty() {
        return Ok(());
    }
    if !matches!(
        message.address.provider_id.as_str(),
        "wecom_ai_bot" | "wechat_ilink"
    ) {
        return Err(GatewayError::InvalidMessage);
    }
    for secret in &secrets {
        let remote_id = secret
            .get("remote_id")
            .and_then(serde_json::Value::as_str)
            .filter(|value| {
                message.parts.iter().any(|part| match part {
                    ChannelMessagePart::Text { .. } => false,
                    ChannelMessagePart::Image { media }
                    | ChannelMessagePart::File { media }
                    | ChannelMessagePart::Audio { media }
                    | ChannelMessagePart::Video { media } => media.remote_id == *value,
                })
            })
            .ok_or(GatewayError::InvalidMessage)?;
        let serialized = Zeroizing::new(serde_json::to_string(secret)?);
        let fingerprint = digest_hex(serialized.as_bytes());
        let identity = digest_hex(
            format!(
                "{}:{}:{}:{}",
                message.address.provider_id,
                message.address.account_id,
                message.event_key.external_message_id,
                remote_id
            )
            .as_bytes(),
        );
        let username = format!(
            "{}:{}:media:{}",
            message.address.provider_id, message.address.account_id, identity
        );
        let secret_ref = format!("keyring:integration:{username}");
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT secret_fingerprint FROM channel_media_secrets WHERE platform = ? AND account_id = ? AND event_id = ? AND remote_id = ?",
        )
        .bind(&message.address.provider_id)
        .bind(&message.address.account_id)
        .bind(&message.event_key.external_message_id)
        .bind(remote_id)
        .fetch_optional(store.pool())
        .await?;
        if existing
            .as_deref()
            .is_some_and(|value| value != fingerprint)
        {
            return Err(GatewayError::PayloadConflict);
        }
        let entry = keyring::Entry::new("com.hachimi.integration", &username)
            .map_err(|_| GatewayError::ProviderCredentialUnavailable)?;
        entry
            .set_password(&serialized)
            .map_err(|_| GatewayError::ProviderCredentialUnavailable)?;
        let result = sqlx::query("INSERT INTO channel_media_secrets(platform, account_id, event_id, remote_id, secret_ref, secret_fingerprint, created_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?) ON CONFLICT(platform, account_id, event_id, remote_id) DO UPDATE SET secret_ref = excluded.secret_ref, secret_fingerprint = excluded.secret_fingerprint")
            .bind(&message.address.provider_id)
            .bind(&message.address.account_id)
            .bind(&message.event_key.external_message_id)
            .bind(remote_id)
            .bind(&secret_ref)
            .bind(&fingerprint)
            .bind(message.received_at_ms)
            .execute(store.pool())
            .await;
        if let Err(error) = result {
            let _ = entry.delete_credential();
            return Err(error.into());
        }
    }
    Ok(())
}

fn take_media_secrets(message: &mut VerifiedChannelMessage) -> Result<Vec<Value>, GatewayError> {
    let Some(context) = message.provider_context.as_object_mut() else {
        return Ok(Vec::new());
    };
    let Some(secrets) = context.remove("_media_secrets") else {
        return Ok(Vec::new());
    };
    secrets
        .as_array()
        .cloned()
        .ok_or(GatewayError::InvalidMessage)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IlinkPrimaryCredential {
    provider_id: IntegrationProviderId,
    bot_token: String,
    bot_id: String,
    base_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WecomAiPrimaryCredential {
    provider_id: IntegrationProviderId,
    bot_id: String,
    secret: String,
}

fn load_wecom_ai_credential(
    config: &AccountRuntimeConfig,
) -> Result<WecomAiPrimaryCredential, GatewayError> {
    let expected = format!(
        "keyring:integration:wecom_ai_bot:{}:primary",
        config.account_id
    );
    if config.provider_id != IntegrationProviderId::WecomAiBot || config.credential_ref != expected
    {
        return Err(GatewayError::ProviderCredentialUnavailable);
    }
    let raw = Zeroizing::new(
        keyring::Entry::new(
            "com.hachimi.integration",
            &format!("wecom_ai_bot:{}:primary", config.account_id),
        )
        .and_then(|entry| entry.get_password())
        .map_err(|_| GatewayError::ProviderCredentialUnavailable)?,
    );
    let credential: WecomAiPrimaryCredential =
        serde_json::from_str(&raw).map_err(|_| GatewayError::ProviderCredentialUnavailable)?;
    if credential.provider_id != IntegrationProviderId::WecomAiBot
        || credential.bot_id != config.tenant_key
        || credential.secret.trim().is_empty()
    {
        return Err(GatewayError::ProviderCredentialUnavailable);
    }
    Ok(credential)
}

async fn deliver_wecom_ai_bot(
    provider: &OfficialChannelProvider,
    runtime: &AccountRuntime,
    attempt: &DeliveryAttempt,
) -> Result<ChannelDeliveryOutcome, GatewayError> {
    if attempt.payload.parts.is_empty() {
        return Ok(delivery_failure("wecom_ai_delivery_invalid", false));
    }
    let transport = provider
        .wecom_ai_transports
        .read()
        .map_err(|_| GatewayError::ProviderStatePoisoned)?
        .get(&attempt.address.account_id)
        .cloned();
    let Some(transport) = transport else {
        return Ok(delivery_failure("wecom_ai_disconnected", true));
    };
    let group = attempt.address.chat_kind == hachimi_protocol::ChannelChatKind::Group;
    let mut delivered_parts = 0_usize;
    let mut last_receipt = None;
    for (index, part) in attempt.payload.parts.iter().enumerate() {
        let key = format!("{}:{index}", attempt.idempotency_key);
        let result = match part {
            ChannelMessagePart::Text { text } => {
                transport
                    .send_text(attempt.address.chat_id.clone(), group, text.clone(), key)
                    .await
            }
            ChannelMessagePart::Image { media }
            | ChannelMessagePart::File { media }
            | ChannelMessagePart::Audio { media }
            | ChannelMessagePart::Video { media } => {
                let managed =
                    managed_outbound_media(&provider.store, &attempt.address, media).await?;
                let kind = match part {
                    ChannelMessagePart::Image { .. } => WecomAiBotMediaKind::Image,
                    ChannelMessagePart::File { .. } => WecomAiBotMediaKind::File,
                    ChannelMessagePart::Audio { .. } => WecomAiBotMediaKind::Voice,
                    ChannelMessagePart::Video { .. } => WecomAiBotMediaKind::Video,
                    ChannelMessagePart::Text { .. } => unreachable!(),
                };
                transport
                    .send_media(
                        attempt.address.chat_id.clone(),
                        group,
                        kind,
                        managed.file_name,
                        managed.bytes,
                        key,
                    )
                    .await
            }
        };
        match result {
            WecomAiBotDeliveryResult::Delivered { provider_receipt } => {
                delivered_parts = delivered_parts.saturating_add(1);
                last_receipt = Some(provider_receipt);
            }
            WecomAiBotDeliveryResult::Retryable => {
                return Ok(if delivered_parts == 0 {
                    delivery_failure("wecom_ai_delivery_retryable", true)
                } else {
                    delivery_indeterminate("wecom_ai_partial_delivery")
                });
            }
            WecomAiBotDeliveryResult::Permanent => {
                return Ok(if delivered_parts == 0 {
                    delivery_failure("wecom_ai_delivery_rejected", false)
                } else {
                    delivery_indeterminate("wecom_ai_partial_delivery")
                });
            }
            WecomAiBotDeliveryResult::Indeterminate => {
                return Ok(delivery_indeterminate(if delivered_parts == 0 {
                    "wecom_ai_delivery_indeterminate"
                } else {
                    "wecom_ai_partial_delivery"
                }));
            }
        }
    }
    runtime
        .deliver(&attempt.payload, crate::now_ms())
        .map_err(|_| GatewayError::ProviderUnavailable)?;
    Ok(ChannelDeliveryOutcome {
        delivered: true,
        retryable: false,
        indeterminate: false,
        result_code: "wecom_ai_delivered".into(),
        provider_receipt: last_receipt,
    })
}

fn load_enterprise_credential(
    config: &AccountRuntimeConfig,
) -> Result<EnterpriseCredential, GatewayError> {
    if !matches!(
        config.provider_id,
        IntegrationProviderId::DingTalk
            | IntegrationProviderId::Feishu
            | IntegrationProviderId::WecomApp
    ) {
        return Err(GatewayError::InvalidProvider);
    }
    let expected = format!(
        "keyring:integration:{}:{}:primary",
        config.provider_id.as_str(),
        config.account_id
    );
    if config.credential_ref != expected {
        return Err(GatewayError::ProviderCredentialUnavailable);
    }
    let raw = Zeroizing::new(
        keyring::Entry::new(
            "com.hachimi.integration",
            &format!(
                "{}:{}:primary",
                config.provider_id.as_str(),
                config.account_id
            ),
        )
        .and_then(|entry| entry.get_password())
        .map_err(|_| GatewayError::ProviderCredentialUnavailable)?,
    );
    let credential = EnterpriseCredential::parse(&raw)
        .map_err(|_| GatewayError::ProviderCredentialUnavailable)?;
    if credential.platform() != config.provider_id || credential.tenant_id() != config.tenant_key {
        return Err(GatewayError::ProviderCredentialUnavailable);
    }
    Ok(credential)
}

fn enterprise_event_frame(
    runtime: &AccountRuntime,
    event: EnterpriseStreamEvent,
) -> Result<ProviderEventFrame, GatewayError> {
    let config = runtime
        .config()
        .map_err(|_| GatewayError::ProviderUnavailable)?;
    if event.platform != config.provider_id {
        return Err(GatewayError::InvalidProvider);
    }
    let mut payload = event.payload;
    let payload_object = payload
        .as_object_mut()
        .ok_or(GatewayError::InvalidMessage)?;
    payload_object.insert(
        "attachments".into(),
        serde_json::to_value(event.attachments)?,
    );
    if event.platform == IntegrationProviderId::Feishu
        && let Some(message) = payload.pointer_mut("/event/message")
        && let Some(message) = message.as_object_mut()
    {
        message.insert("text".into(), serde_json::Value::String(event.text));
    }
    Ok(ProviderEventFrame {
        account_id: config.account_id,
        tenant_key: config.tenant_key,
        payload,
        proof: TransportProof::Stream {
            connection_id: event.connection_id,
            received_at_ms: if event.timestamp_ms > 0 {
                event.timestamp_ms
            } else {
                crate::now_ms()
            },
        },
    })
}

async fn deliver_enterprise(
    store: &AgentStore,
    runtime: &AccountRuntime,
    attempt: &DeliveryAttempt,
) -> Result<ChannelDeliveryOutcome, GatewayError> {
    if attempt.payload.parts.is_empty() {
        return Ok(delivery_failure("enterprise_delivery_invalid", false));
    }
    let config = runtime
        .config()
        .map_err(|_| GatewayError::ProviderUnavailable)?;
    let credential = load_enterprise_credential(&config)?;
    let api = EnterpriseApiClient::new().map_err(|_| GatewayError::ProviderUnavailable)?;
    let target = EnterpriseMessageTarget {
        peer: attempt.address.chat_id.clone(),
        thread: None,
        group: attempt.address.chat_kind == hachimi_protocol::ChannelChatKind::Group,
    };
    let mut last_receipt = None;
    let mut delivered_parts = 0_usize;
    for (index, part) in attempt.payload.parts.iter().enumerate() {
        let idempotency_key = format!("{}:{index}", attempt.idempotency_key);
        let result = match part {
            ChannelMessagePart::Text { text } => {
                api.send_text(
                    &attempt.address.account_id,
                    &credential,
                    &target,
                    text,
                    &idempotency_key,
                )
                .await
            }
            ChannelMessagePart::Image { media } => {
                let media = managed_outbound_media(store, &attempt.address, media).await?;
                api.send_media(EnterpriseMediaInput {
                    account_id: &attempt.address.account_id,
                    credential: &credential,
                    target: &target,
                    kind: EnterpriseMediaKind::Image,
                    file_name: &media.file_name,
                    mime_type: &media.mime_type,
                    bytes: &media.bytes,
                    idempotency_key: &idempotency_key,
                })
                .await
            }
            ChannelMessagePart::File { media } => {
                let media = managed_outbound_media(store, &attempt.address, media).await?;
                api.send_media(EnterpriseMediaInput {
                    account_id: &attempt.address.account_id,
                    credential: &credential,
                    target: &target,
                    kind: EnterpriseMediaKind::File,
                    file_name: &media.file_name,
                    mime_type: &media.mime_type,
                    bytes: &media.bytes,
                    idempotency_key: &idempotency_key,
                })
                .await
            }
            ChannelMessagePart::Audio { .. } | ChannelMessagePart::Video { .. } => {
                Err(EnterpriseApiError::InvalidRequest)
            }
        };
        match result {
            Ok(response) => {
                delivered_parts = delivered_parts.saturating_add(1);
                last_receipt = provider_receipt(&response).or(last_receipt);
            }
            Err(error) => {
                if matches!(
                    error,
                    EnterpriseApiError::Authentication | EnterpriseApiError::InvalidCredential
                ) {
                    mark_provider_account_attention(store, runtime).await;
                }
                return Ok(if delivered_parts > 0 {
                    delivery_indeterminate("enterprise_partial_delivery")
                } else {
                    ChannelDeliveryOutcome {
                        delivered: false,
                        retryable: error.retryable(),
                        indeterminate: error == EnterpriseApiError::Indeterminate,
                        result_code: error.code().into(),
                        provider_receipt: None,
                    }
                });
            }
        }
    }
    runtime
        .deliver(&attempt.payload, crate::now_ms())
        .map_err(|_| GatewayError::ProviderUnavailable)?;
    Ok(ChannelDeliveryOutcome {
        delivered: true,
        retryable: false,
        indeterminate: false,
        result_code: "enterprise_delivered".into(),
        provider_receipt: last_receipt,
    })
}

fn provider_receipt(response: &serde_json::Value) -> Option<String> {
    [
        "/message_id",
        "/messageId",
        "/data/message_id",
        "/data/processQueryKey",
        "/msgid",
    ]
    .into_iter()
    .find_map(|pointer| {
        let value = response.pointer(pointer)?;
        value
            .as_str()
            .map(str::to_owned)
            .or_else(|| (value.is_number() || value.is_boolean()).then(|| value.to_string()))
    })
}

fn load_ilink_credential(
    config: &AccountRuntimeConfig,
) -> Result<IlinkPrimaryCredential, GatewayError> {
    let expected = format!(
        "keyring:integration:wechat_ilink:{}:primary",
        config.account_id
    );
    if config.credential_ref != expected {
        return Err(GatewayError::ProviderCredentialUnavailable);
    }
    let raw = Zeroizing::new(
        keyring::Entry::new(
            "com.hachimi.integration",
            &format!("wechat_ilink:{}:primary", config.account_id),
        )
        .and_then(|entry| entry.get_password())
        .map_err(|_| GatewayError::ProviderCredentialUnavailable)?,
    );
    let credential: IlinkPrimaryCredential =
        serde_json::from_str(&raw).map_err(|_| GatewayError::ProviderCredentialUnavailable)?;
    if credential.provider_id != IntegrationProviderId::WechatIlink
        || credential.bot_token.trim().is_empty()
        || credential.bot_id.trim().is_empty()
    {
        return Err(GatewayError::ProviderCredentialUnavailable);
    }
    Ok(credential)
}

async fn run_ilink_poller(
    store: AgentStore,
    runtime: AccountRuntime,
    client: WechatIlinkClient,
    bot_id: String,
) {
    let mut cursor = String::new();
    let mut consecutive_failures = 0_u32;
    loop {
        match client.get_updates(&cursor).await {
            Ok(batch) => {
                cursor = batch.cursor;
                consecutive_failures = 0;
                let _ = runtime.record_transport_success();
                for payload in batch.messages {
                    let frame = ProviderEventFrame {
                        account_id: runtime
                            .config()
                            .map(|config| config.account_id)
                            .unwrap_or_default(),
                        tenant_key: bot_id.clone(),
                        payload,
                        proof: TransportProof::IlinkPoll {
                            bot_id: bot_id.clone(),
                            received_at_ms: crate::now_ms(),
                        },
                    };
                    let message = match WechatIlinkAdapter.normalize(frame) {
                        Ok(message) => message,
                        Err(ProviderError::BotLoop | ProviderError::InvalidEvent) => continue,
                        Err(_) => {
                            let _ = runtime.record_transport_failure();
                            continue;
                        }
                    };
                    let context_token = message
                        .provider_context
                        .get("context_token")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned);
                    let Some(context_token) = context_token else {
                        continue;
                    };
                    if persist_ilink_context_token(
                        &store,
                        &message.address,
                        &context_token,
                        message.received_at_ms,
                    )
                    .await
                    .is_err()
                    {
                        let _ = runtime.record_transport_failure();
                        continue;
                    }
                    let _ = runtime.push_verified(message);
                }
            }
            Err(ProviderError::AuthenticationExpired) => {
                mark_ilink_authentication_expired(&store, &runtime).await;
                break;
            }
            Err(_) => {
                consecutive_failures = consecutive_failures.saturating_add(1);
                let _ = runtime.record_transport_failure();
                let delay = if consecutive_failures >= 3 { 30 } else { 2 };
                tokio::time::sleep(Duration::from_secs(delay)).await;
            }
        }
    }
}

async fn persist_ilink_context_token(
    store: &AgentStore,
    address: &ChannelConversationAddress,
    token: &str,
    timestamp_ms: i64,
) -> Result<(), GatewayError> {
    let hash = conversation_hash(address)?;
    let username = format!("wechat_ilink:{}:conversation:{hash}", address.account_id);
    let secret_ref = format!("keyring:integration:{username}");
    let entry = keyring::Entry::new("com.hachimi.integration", &username)
        .map_err(|_| GatewayError::ProviderCredentialUnavailable)?;
    entry
        .set_password(token)
        .map_err(|_| GatewayError::ProviderCredentialUnavailable)?;
    let result = sqlx::query("INSERT INTO channel_route_secrets(account_id, conversation_hash, secret_ref, token_fingerprint, expires_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, NULL, ?) ON CONFLICT(account_id, conversation_hash) DO UPDATE SET secret_ref = excluded.secret_ref, token_fingerprint = excluded.token_fingerprint, expires_at_ms = NULL, updated_at_ms = excluded.updated_at_ms")
        .bind(&address.account_id)
        .bind(&hash)
        .bind(&secret_ref)
        .bind(digest_hex(token.as_bytes()))
        .bind(timestamp_ms)
        .execute(store.pool())
        .await;
    if let Err(error) = result {
        let _ = entry.delete_credential();
        return Err(error.into());
    }
    Ok(())
}

async fn resolve_ilink_context_token(
    store: &AgentStore,
    address: &ChannelConversationAddress,
) -> Result<Zeroizing<String>, GatewayError> {
    let hash = conversation_hash(address)?;
    let secret_ref: Option<String> = sqlx::query_scalar(
        "SELECT secret_ref FROM channel_route_secrets WHERE account_id = ? AND conversation_hash = ?",
    )
    .bind(&address.account_id)
    .bind(&hash)
    .fetch_optional(store.pool())
    .await?;
    let expected = format!(
        "keyring:integration:wechat_ilink:{}:conversation:{hash}",
        address.account_id
    );
    if secret_ref.as_deref() != Some(expected.as_str()) {
        return Err(GatewayError::ProviderCredentialUnavailable);
    }
    let token = keyring::Entry::new(
        "com.hachimi.integration",
        &format!("wechat_ilink:{}:conversation:{hash}", address.account_id),
    )
    .and_then(|entry| entry.get_password())
    .map_err(|_| GatewayError::ProviderCredentialUnavailable)?;
    Ok(Zeroizing::new(token))
}

async fn deliver_ilink(
    store: &AgentStore,
    runtime: &AccountRuntime,
    attempt: &DeliveryAttempt,
) -> Result<ChannelDeliveryOutcome, GatewayError> {
    if attempt.payload.parts.is_empty() {
        return Ok(delivery_failure("ilink_delivery_invalid", false));
    }
    let config = runtime
        .config()
        .map_err(|_| GatewayError::ProviderUnavailable)?;
    let credential = load_ilink_credential(&config)?;
    let client = WechatIlinkClient::authenticated(&credential.base_url, credential.bot_token)
        .map_err(|_| GatewayError::ProviderCredentialUnavailable)?;
    let context_token = match resolve_ilink_context_token(store, &attempt.address).await {
        Ok(token) => token,
        Err(GatewayError::ProviderCredentialUnavailable) => {
            return Ok(delivery_failure("ilink_context_token_missing", false));
        }
        Err(error) => return Err(error),
    };
    let mut last_receipt = None;
    let mut delivered_parts = 0_usize;
    for (part_index, part) in attempt.payload.parts.iter().enumerate() {
        let result = match part {
            ChannelMessagePart::Text { text } => {
                let mut result = Ok(None);
                for (chunk_index, chunk) in
                    split_text(text, hachimi_channel_providers::ILINK_TEXT_LIMIT)
                        .iter()
                        .enumerate()
                {
                    let key = format!("{}:{part_index}:{chunk_index}", attempt.idempotency_key);
                    result = client
                        .send_text(&attempt.address.chat_id, chunk, &context_token, &key)
                        .await
                        .map(Some);
                    if result.is_err() {
                        break;
                    }
                }
                result
            }
            ChannelMessagePart::Image { media } => {
                let media = managed_outbound_media(store, &attempt.address, media).await?;
                client
                    .send_media(
                        &attempt.address.chat_id,
                        &context_token,
                        IlinkMediaKind::Image,
                        &media.file_name,
                        &media.bytes,
                        &format!("{}:{part_index}", attempt.idempotency_key),
                    )
                    .await
                    .map(Some)
            }
            ChannelMessagePart::File { media } => {
                let media = managed_outbound_media(store, &attempt.address, media).await?;
                client
                    .send_media(
                        &attempt.address.chat_id,
                        &context_token,
                        IlinkMediaKind::File,
                        &media.file_name,
                        &media.bytes,
                        &format!("{}:{part_index}", attempt.idempotency_key),
                    )
                    .await
                    .map(Some)
            }
            ChannelMessagePart::Video { media } => {
                let media = managed_outbound_media(store, &attempt.address, media).await?;
                client
                    .send_media(
                        &attempt.address.chat_id,
                        &context_token,
                        IlinkMediaKind::Video,
                        &media.file_name,
                        &media.bytes,
                        &format!("{}:{part_index}", attempt.idempotency_key),
                    )
                    .await
                    .map(Some)
            }
            ChannelMessagePart::Audio { .. } => Err(ProviderError::InvalidEvent),
        };
        match result {
            Ok(receipt) => {
                delivered_parts = delivered_parts.saturating_add(1);
                if let Some(receipt) = receipt {
                    last_receipt = Some(receipt.external_message_id.unwrap_or(receipt.client_id));
                }
            }
            Err(ProviderError::AuthenticationExpired) => {
                mark_ilink_authentication_expired(store, runtime).await;
                return Ok(if delivered_parts == 0 {
                    delivery_failure("ilink_reauthentication_required", false)
                } else {
                    delivery_indeterminate("ilink_partial_delivery")
                });
            }
            Err(ProviderError::InvalidEvent | ProviderError::MediaLimit) => {
                return Ok(if delivered_parts == 0 {
                    delivery_failure("ilink_delivery_invalid", false)
                } else {
                    delivery_indeterminate("ilink_partial_delivery")
                });
            }
            Err(_) => {
                return Ok(if delivered_parts == 0 {
                    delivery_failure("ilink_delivery_retryable", true)
                } else {
                    delivery_indeterminate("ilink_partial_delivery")
                });
            }
        }
    }
    runtime
        .deliver(&attempt.payload, crate::now_ms())
        .map_err(|_| GatewayError::ProviderUnavailable)?;
    Ok(ChannelDeliveryOutcome {
        delivered: true,
        retryable: false,
        indeterminate: false,
        result_code: "ilink_delivered".into(),
        provider_receipt: last_receipt,
    })
}

struct ManagedOutboundMedia {
    file_name: String,
    mime_type: String,
    bytes: Vec<u8>,
}

async fn managed_outbound_media(
    store: &AgentStore,
    address: &ChannelConversationAddress,
    descriptor: &RemoteMediaDescriptor,
) -> Result<ManagedOutboundMedia, GatewayError> {
    if descriptor.provider_id.as_str() != address.provider_id
        || descriptor.download_required
        || descriptor.remote_id.trim().is_empty()
        || descriptor.remote_id.len() > 256
    {
        return Err(GatewayError::InvalidMessage);
    }
    let row: Option<(String, String, String, i64, String)> = sqlx::query_as(
        "SELECT content_hash, original_name, mime_type, byte_size, managed_path FROM attachments WHERE id = ?",
    )
    .bind(&descriptor.remote_id)
    .fetch_optional(store.pool())
    .await?;
    let Some((content_hash, file_name, mime_type, byte_size, managed_path)) = row else {
        return Err(GatewayError::InvalidMessage);
    };
    if byte_size <= 0 || byte_size > 25 * 1024 * 1024 {
        return Err(GatewayError::InvalidMessage);
    }
    if descriptor
        .content_hash
        .as_deref()
        .is_some_and(|expected| expected != content_hash)
        || descriptor
            .mime_type
            .as_deref()
            .is_some_and(|expected| expected != mime_type)
    {
        return Err(GatewayError::PayloadConflict);
    }
    let root = store
        .managed_artifact_root()
        .canonicalize()
        .map_err(|_| GatewayError::ProviderUnavailable)?;
    let path = PathBuf::from(managed_path)
        .canonicalize()
        .map_err(|_| GatewayError::ProviderUnavailable)?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err(GatewayError::InvalidMessage);
    }
    let bytes = std::fs::read(path).map_err(|_| GatewayError::ProviderUnavailable)?;
    if i64::try_from(bytes.len()).unwrap_or(i64::MAX) != byte_size
        || digest_hex(&bytes) != content_hash
    {
        return Err(GatewayError::PayloadConflict);
    }
    Ok(ManagedOutboundMedia {
        file_name,
        mime_type,
        bytes,
    })
}

fn delivery_failure(code: &str, retryable: bool) -> ChannelDeliveryOutcome {
    ChannelDeliveryOutcome {
        delivered: false,
        retryable,
        indeterminate: false,
        result_code: code.into(),
        provider_receipt: None,
    }
}

fn delivery_indeterminate(code: &str) -> ChannelDeliveryOutcome {
    ChannelDeliveryOutcome {
        delivered: false,
        retryable: false,
        indeterminate: true,
        result_code: code.into(),
        provider_receipt: None,
    }
}

fn split_text(value: &str, limit: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_len = 0;
    for character in value.chars() {
        if current_len == limit {
            chunks.push(std::mem::take(&mut current));
            current_len = 0;
        }
        current.push(character);
        current_len += 1;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

async fn mark_ilink_authentication_expired(store: &AgentStore, runtime: &AccountRuntime) {
    let _ = runtime.mark_authentication_expired();
    let Ok(config) = runtime.config() else {
        return;
    };
    let timestamp_ms = crate::now_ms();
    let _ = sqlx::query("UPDATE integration_provider_accounts SET state = 'awaiting_auth', diagnostic = 'ilink_reauthentication_required', updated_at_ms = ? WHERE id = ? AND provider_id = 'wechat_ilink'")
        .bind(timestamp_ms)
        .bind(&config.account_id)
        .execute(store.pool())
        .await;
    let _ = sqlx::query("UPDATE channel_provider_accounts SET state = 'awaiting_auth', updated_at_ms = ? WHERE id = ? AND provider_id = 'wechat_ilink'")
        .bind(timestamp_ms)
        .bind(config.account_id)
        .execute(store.pool())
        .await;
}

async fn mark_provider_account_attention(store: &AgentStore, runtime: &AccountRuntime) {
    let _ = runtime.mark_needs_attention();
    let Ok(config) = runtime.config() else {
        return;
    };
    let timestamp_ms = crate::now_ms();
    let _ = sqlx::query("UPDATE integration_provider_accounts SET state = 'needs_attention', diagnostic = 'provider_credentials_or_transport_require_attention', updated_at_ms = ? WHERE id = ?")
        .bind(timestamp_ms)
        .bind(&config.account_id)
        .execute(store.pool())
        .await;
    let _ = sqlx::query("UPDATE channel_provider_accounts SET state = 'needs_attention', updated_at_ms = ? WHERE id = ?")
        .bind(timestamp_ms)
        .bind(config.account_id)
        .execute(store.pool())
        .await;
}

fn conversation_hash(address: &ChannelConversationAddress) -> Result<String, GatewayError> {
    Ok(digest_hex(&serde_json::to_vec(address)?))
}

fn digest_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn parse_provider_id(value: &str) -> Result<IntegrationProviderId, GatewayError> {
    match value {
        "dingtalk" => Ok(IntegrationProviderId::DingTalk),
        "feishu" => Ok(IntegrationProviderId::Feishu),
        "wecom_ai_bot" => Ok(IntegrationProviderId::WecomAiBot),
        "wecom_app" => Ok(IntegrationProviderId::WecomApp),
        "wechat_ilink" => Ok(IntegrationProviderId::WechatIlink),
        _ => Err(GatewayError::InvalidProvider),
    }
}

fn account_state_can_run(state: hachimi_protocol::ChannelAccountState) -> bool {
    matches!(
        state,
        hachimi_protocol::ChannelAccountState::Starting
            | hachimi_protocol::ChannelAccountState::Healthy
            | hachimi_protocol::ChannelAccountState::Degraded
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use hachimi_protocol::{
        AttachmentId, AttachmentRecord, ChannelActor, ChannelChatKind, ChannelEventKey,
        ChannelMention,
    };
    use serde_json::json;

    #[test]
    fn ilink_text_chunks_are_unicode_safe_and_bounded() {
        let chunks = split_text("你a好b界", 2);
        assert_eq!(chunks, ["你a", "好b", "界"]);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 2));
    }

    fn test_address() -> ChannelConversationAddress {
        ChannelConversationAddress {
            provider_id: "dingtalk".into(),
            account_id: "account-1".into(),
            tenant_key: "tenant-1".into(),
            chat_kind: ChannelChatKind::Dm,
            chat_id: "user-1".into(),
            topic_id: None,
        }
    }

    fn test_descriptor(id: &str, hash: &str) -> RemoteMediaDescriptor {
        RemoteMediaDescriptor {
            provider_id: IntegrationProviderId::DingTalk,
            remote_id: id.into(),
            resource_key: Some("attachment".into()),
            file_name: Some("image.png".into()),
            mime_type: Some("image/png".into()),
            declared_size_bytes: Some(11),
            content_hash: Some(hash.into()),
            download_required: false,
        }
    }

    #[tokio::test]
    async fn managed_outbound_media_rejects_path_hash_and_size_drift() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let bytes = b"image-bytes";
        let hash = digest_hex(bytes);
        let path = store.managed_artifact_root().join(&hash);
        std::fs::write(&path, bytes).expect("managed media");
        let record = AttachmentRecord {
            id: AttachmentId::new("attachment-1"),
            content_hash: hash.clone(),
            original_name: "image.png".into(),
            mime_type: "image/png".into(),
            byte_size: bytes.len() as u64,
            created_at_ms: 1,
        };
        store
            .upsert_attachment(&record, &path)
            .await
            .expect("attachment");
        let descriptor = test_descriptor(record.id.as_str(), &hash);
        assert!(
            managed_outbound_media(&store, &test_address(), &descriptor)
                .await
                .is_ok()
        );

        std::fs::write(&path, b"hash-drift").expect("drift");
        assert!(matches!(
            managed_outbound_media(&store, &test_address(), &descriptor).await,
            Err(GatewayError::PayloadConflict)
        ));
        std::fs::write(&path, bytes).expect("restore");
        sqlx::query("UPDATE attachments SET byte_size = byte_size + 1 WHERE id = ?")
            .bind(record.id.as_str())
            .execute(store.pool())
            .await
            .expect("size drift");
        assert!(matches!(
            managed_outbound_media(&store, &test_address(), &descriptor).await,
            Err(GatewayError::PayloadConflict)
        ));

        let outside = tempfile::NamedTempFile::new().expect("outside");
        std::fs::write(outside.path(), bytes).expect("outside bytes");
        sqlx::query("UPDATE attachments SET byte_size = ?, managed_path = ? WHERE id = ?")
            .bind(bytes.len() as i64)
            .bind(outside.path().to_string_lossy().as_ref())
            .bind(record.id.as_str())
            .execute(store.pool())
            .await
            .expect("outside path");
        assert!(matches!(
            managed_outbound_media(&store, &test_address(), &descriptor).await,
            Err(GatewayError::InvalidMessage)
        ));
    }

    #[test]
    fn media_download_secrets_are_removed_before_ingress_serialization() {
        let mut message = VerifiedChannelMessage {
            event_key: ChannelEventKey {
                provider_id: "wechat_ilink".into(),
                account_id: "account-1".into(),
                external_message_id: "message-1".into(),
            },
            address: ChannelConversationAddress {
                provider_id: "wechat_ilink".into(),
                ..test_address()
            },
            actor: ChannelActor {
                external_id: "user-1".into(),
                display_name: None,
                is_bot: false,
            },
            parts: vec![ChannelMessagePart::Image {
                media: RemoteMediaDescriptor {
                    provider_id: IntegrationProviderId::WechatIlink,
                    remote_id: "remote-1".into(),
                    resource_key: Some("ilink_cdn".into()),
                    file_name: None,
                    mime_type: None,
                    declared_size_bytes: None,
                    content_hash: None,
                    download_required: true,
                },
            }],
            mentions: Vec::<ChannelMention>::new(),
            quote: None,
            received_at_ms: 1,
            provider_context: json!({
                "_media_secrets": [{
                    "remote_id": "remote-1",
                    "aes_key": "must-not-reach-sqlite",
                    "download_url": "https://example.invalid/private"
                }]
            }),
        };
        let secrets = take_media_secrets(&mut message).expect("secrets");
        assert_eq!(secrets.len(), 1);
        let serialized = serde_json::to_string(&message).expect("message");
        assert!(!serialized.contains("must-not-reach-sqlite"));
        assert!(!serialized.contains("example.invalid"));
    }
}
