mod builtin;
mod durable;
mod official_provider;
mod provider;
mod routing;
mod sidecar_provider;

pub use builtin::{LocalBuiltinProviders, LoopbackWebhookChannel, MockPollChannel};
pub use durable::{ReactiveDeliverySource, remote_media_metadata_hash};
pub use official_provider::OfficialChannelProvider;
pub use provider::{
    ChannelDeliveryOutcome, ChannelProvider, ChannelProviderFuture, ChannelProviderRegistry,
};
pub use routing::{
    BindingResolution, ChannelControlCommand, PairingConsumeOutcome, parse_control_command,
};
pub use sidecar_provider::SandboxedStdioChannelProvider;

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, RwLock},
};

use hachimi_core::FeatureAvailability;
use hachimi_protocol::{
    ChannelProviderAccount, ChannelProviderHealth, ChannelProviderHealthState,
    ChannelProviderManifest, ChannelProviderRuntimeKind, GatewayHealth, RuntimeComponentState,
};
use hachimi_storage::AgentStore;
use sqlx::Row;
use thiserror::Error;

pub const MAX_MESSAGE_CHARS: usize = 32_000;
pub const MAX_DELIVERY_ATTEMPTS: u32 = 8;
pub const CLAIM_TTL_MS: i64 = 60_000;

#[must_use]
pub const fn availability() -> FeatureAvailability {
    FeatureAvailability::Available
}

pub fn local_builtin_providers(
    store: AgentStore,
    loopback_token: &str,
) -> Result<LocalBuiltinProviders, GatewayError> {
    let builtins = builtin::local_builtin_providers(store.clone(), loopback_token)?;
    official_provider::register_official_providers(&builtins.registry, &store)?;
    Ok(builtins)
}

pub fn local_builtin_providers_with_enterprise(
    store: AgentStore,
    loopback_token: &str,
    integration_providers_enabled: bool,
) -> Result<LocalBuiltinProviders, GatewayError> {
    let builtins = builtin::local_builtin_providers(store.clone(), loopback_token)?;
    if integration_providers_enabled {
        official_provider::register_official_providers(&builtins.registry, &store)?;
    }
    Ok(builtins)
}

#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("gateway route is not authorized")]
    RouteNotAllowed,
    #[error("gateway rejected a bot-generated loop")]
    BotLoop,
    #[error("gateway message is invalid")]
    InvalidMessage,
    #[error("gateway payload changed for an existing event key")]
    PayloadConflict,
    #[error("gateway ingress was not found or changed state")]
    IngressConflict,
    #[error("gateway delivery was not found or changed state")]
    DeliveryConflict,
    #[error("gateway idempotency key was reused with different input")]
    IdempotencyConflict,
    #[error("gateway startup registration failed: {0}")]
    StartupRegistration(String),
    #[error("mock poll channel is disconnected")]
    ChannelDisconnected,
    #[error("channel provider manifest or configuration is invalid")]
    InvalidProvider,
    #[error("channel provider runtime state lock is unavailable")]
    ProviderStatePoisoned,
    #[error("channel provider is unavailable")]
    ProviderUnavailable,
    #[error("channel provider credential is unavailable")]
    ProviderCredentialUnavailable,
    #[error("channel sidecar failed closed: {0}")]
    Sidecar(&'static str),
    #[error("channel provider account configuration revision changed")]
    ProviderRevisionConflict,
    #[error("channel authorization changed or does not exist")]
    AuthorizationConflict,
    #[error("channel pairing code is invalid, expired, or already consumed")]
    PairingRejected,
    #[error("channel identity is already linked to another local subject")]
    IdentityOwnershipConflict,
    #[error("gateway database failed: {0}")]
    Database(#[from] sqlx::Error),
    #[error("gateway storage failed: {0}")]
    Storage(#[from] hachimi_storage::AgentStoreError),
    #[error("gateway serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct GatewayHost {
    pub(crate) store: AgentStore,
    pub(crate) providers: ChannelProviderRegistry,
    pub(crate) channels: Arc<RwLock<BTreeSet<String>>>,
    observed_provider_accounts: Arc<RwLock<BTreeMap<String, ChannelProviderAccount>>>,
    provider_ingress_enabled: bool,
}

impl GatewayHost {
    #[must_use]
    pub fn new(store: AgentStore, channels: impl IntoIterator<Item = String>) -> Self {
        Self {
            store,
            providers: ChannelProviderRegistry::default(),
            channels: Arc::new(RwLock::new(channels.into_iter().collect())),
            observed_provider_accounts: Arc::new(RwLock::new(BTreeMap::new())),
            provider_ingress_enabled: false,
        }
    }

    #[must_use]
    pub fn with_registry(store: AgentStore, providers: ChannelProviderRegistry) -> Self {
        let channels = providers.provider_ids().into_iter().collect();
        Self {
            store,
            providers,
            channels: Arc::new(RwLock::new(channels)),
            observed_provider_accounts: Arc::new(RwLock::new(BTreeMap::new())),
            provider_ingress_enabled: false,
        }
    }

    #[must_use]
    pub const fn with_provider_ingress_enabled(mut self) -> Self {
        self.provider_ingress_enabled = true;
        self
    }

    pub async fn register_provider(
        &self,
        provider: Arc<dyn ChannelProvider>,
        enabled: bool,
    ) -> Result<ChannelProviderManifest, GatewayError> {
        let manifest = provider.manifest();
        validate_manifest(&manifest)?;
        self.providers.register(provider)?;
        sqlx::query("INSERT INTO channel_provider_manifests(provider_id, plugin_id, manifest_json, content_hash, enabled, config_revision, health, diagnostic, updated_at_ms, contribution_enabled) VALUES(?, ?, ?, ?, ?, 1, 'disabled', NULL, ?, 1) ON CONFLICT(provider_id) DO UPDATE SET plugin_id = excluded.plugin_id, manifest_json = excluded.manifest_json, content_hash = excluded.content_hash, enabled = excluded.enabled, contribution_enabled = 1, config_revision = channel_provider_manifests.config_revision + 1, updated_at_ms = excluded.updated_at_ms")
            .bind(&manifest.id)
            .bind(manifest.plugin_id.as_ref().map(hachimi_protocol::PluginId::as_str))
            .bind(serde_json::to_string(&manifest)?)
            .bind(&manifest.content_hash)
            .bind(enabled)
            .bind(now_ms())
            .execute(self.store.pool())
            .await?;
        if enabled {
            self.channels
                .write()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?
                .insert(manifest.id.clone());
        }
        Ok(manifest)
    }

    pub async fn bootstrap_provider_accounts(
        &self,
        accounts: &[ChannelProviderAccount],
    ) -> Result<(), GatewayError> {
        for manifest in self.providers.manifests() {
            validate_manifest(&manifest)?;
            sqlx::query("INSERT INTO channel_provider_manifests(provider_id, plugin_id, manifest_json, content_hash, enabled, config_revision, health, diagnostic, updated_at_ms, contribution_enabled) VALUES(?, ?, ?, ?, 1, 1, 'disabled', NULL, ?, 1) ON CONFLICT(provider_id) DO UPDATE SET manifest_json = excluded.manifest_json, content_hash = excluded.content_hash, enabled = 1, contribution_enabled = 1, updated_at_ms = excluded.updated_at_ms")
                .bind(&manifest.id)
                .bind(manifest.plugin_id.as_ref().map(hachimi_protocol::PluginId::as_str))
                .bind(serde_json::to_string(&manifest)?)
                .bind(&manifest.content_hash)
                .bind(now_ms())
                .execute(self.store.pool())
                .await?;
        }
        for account in accounts {
            validate_account(account)?;
            let Some(provider) = self.providers.resolve(&account.provider_id) else {
                continue;
            };
            provider.configure(account).await?;
            sqlx::query("INSERT INTO channel_provider_accounts(id, provider_id, display_name, tenant_key, credential_ref, enabled, state, config_json, credential_revision, config_revision, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET provider_id = excluded.provider_id, display_name = excluded.display_name, tenant_key = excluded.tenant_key, credential_ref = excluded.credential_ref, enabled = excluded.enabled, state = excluded.state, config_json = excluded.config_json, credential_revision = excluded.credential_revision, config_revision = excluded.config_revision, updated_at_ms = excluded.updated_at_ms")
                .bind(&account.id)
                .bind(&account.provider_id)
                .bind(&account.display_name)
                .bind(&account.tenant_key)
                .bind(&account.credential_ref)
                .bind(account.enabled)
                .bind(account_state(account.state))
                .bind(serde_json::to_string(&account.config)?)
                .bind(to_i64(account.credential_revision))
                .bind(to_i64(account.config_revision))
                .bind(now_ms())
                .execute(self.store.pool())
                .await?;
            self.observed_provider_accounts
                .write()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?
                .insert(account.id.clone(), account.clone());
        }
        Ok(())
    }

    pub async fn list_provider_accounts(
        &self,
    ) -> Result<Vec<ChannelProviderAccount>, GatewayError> {
        let rows = sqlx::query("SELECT id, provider_id, display_name, tenant_key, credential_ref, enabled, state, config_json, credential_revision, config_revision FROM channel_provider_accounts ORDER BY provider_id, id")
            .fetch_all(self.store.pool())
            .await?;
        rows.into_iter().map(decode_account).collect()
    }

    pub async fn upsert_provider_account(
        &self,
        input: hachimi_protocol::ChannelProviderAccountUpsert,
    ) -> Result<ChannelProviderAccount, GatewayError> {
        let existing = self
            .list_provider_accounts()
            .await?
            .into_iter()
            .find(|account| account.id == input.id);
        match (&existing, input.expected_config_revision) {
            (None, None) => {}
            (Some(account), Some(revision)) if account.config_revision == revision => {}
            _ => return Err(GatewayError::ProviderRevisionConflict),
        }
        let credential_revision = existing.as_ref().map_or(1, |account| {
            account.credential_revision + u64::from(input.credential.is_some())
        });
        let account = ChannelProviderAccount {
            id: input.id,
            provider_id: input.provider_id,
            display_name: input.display_name,
            tenant_key: input.tenant_key,
            credential_ref: existing.and_then(|account| account.credential_ref),
            enabled: input.enabled,
            state: if input.enabled {
                hachimi_protocol::ChannelAccountState::Starting
            } else {
                hachimi_protocol::ChannelAccountState::Draft
            },
            config: input.config,
            credential_revision,
            config_revision: input
                .expected_config_revision
                .unwrap_or(0)
                .saturating_add(1),
        };
        self.bootstrap_provider_accounts(std::slice::from_ref(&account))
            .await?;
        Ok(account)
    }

    pub async fn reload_configuration(&self) -> Result<(), GatewayError> {
        let accounts = self.list_provider_accounts().await?;
        for account in &accounts {
            let Some(provider) = self.providers.resolve(&account.provider_id) else {
                continue;
            };
            provider.reload(account).await?;
        }
        *self
            .observed_provider_accounts
            .write()
            .map_err(|_| GatewayError::ProviderStatePoisoned)? = accounts
            .into_iter()
            .map(|account| (account.id.clone(), account))
            .collect();
        Ok(())
    }

    pub async fn reconcile_provider_accounts(&self) -> Result<bool, GatewayError> {
        let current = self
            .list_provider_accounts()
            .await?
            .into_iter()
            .map(|account| (account.id.clone(), account))
            .collect::<BTreeMap<_, _>>();
        let previous = self
            .observed_provider_accounts
            .read()
            .map_err(|_| GatewayError::ProviderStatePoisoned)?
            .clone();
        if current == previous {
            return Ok(false);
        }
        for (id, account) in &current {
            if previous.get(id) == Some(account) {
                continue;
            }
            if let Some(provider) = self.providers.resolve(&account.provider_id) {
                provider.reload(account).await?;
            }
        }
        for (id, account) in &previous {
            if !current.contains_key(id)
                && let Some(provider) = self.providers.resolve(&account.provider_id)
            {
                provider.remove_account(id).await?;
            }
        }
        *self
            .observed_provider_accounts
            .write()
            .map_err(|_| GatewayError::ProviderStatePoisoned)? = current;
        self.start_provider_ingress().await?;
        Ok(true)
    }

    pub async fn start_provider_ingress(&self) -> Result<(), GatewayError> {
        if !self.provider_ingress_enabled {
            return Ok(());
        }
        for provider_id in self.providers.provider_ids() {
            if let Some(provider) = self.providers.resolve(&provider_id) {
                provider.start_ingress().await?;
            }
        }
        Ok(())
    }

    pub async fn provider_manifests(&self) -> Result<Vec<ChannelProviderManifest>, GatewayError> {
        let rows = sqlx::query(
            "SELECT manifest_json FROM channel_provider_manifests ORDER BY provider_id",
        )
        .fetch_all(self.store.pool())
        .await?;
        rows.into_iter()
            .map(|row| serde_json::from_str(row.get("manifest_json")).map_err(Into::into))
            .collect()
    }

    pub async fn provider_health(&self) -> Result<Vec<ChannelProviderHealth>, GatewayError> {
        let mut health = Vec::new();
        for id in self.providers.provider_ids() {
            if let Some(provider) = self.providers.resolve(&id) {
                let account_health = provider.account_health().await?;
                if account_health.is_empty() {
                    health.push(provider.health().await?);
                } else {
                    health.extend(account_health);
                }
            }
        }
        Ok(health)
    }

    pub async fn persist_provider_health(&self, timestamp_ms: i64) -> Result<(), GatewayError> {
        for health in self.provider_health().await? {
            let Some(account_id) = health.account_id.as_deref() else {
                continue;
            };
            sqlx::query("INSERT INTO channel_provider_runtime_health(provider_id, account_id, state, diagnostic, last_event_at_ms, last_delivery_at_ms, last_handshake_at_ms, last_frame_at_ms, last_error_code, next_reconnect_at_ms, consecutive_failures, config_revision, observed_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(provider_id, account_id) DO UPDATE SET state = excluded.state, diagnostic = excluded.diagnostic, last_event_at_ms = excluded.last_event_at_ms, last_delivery_at_ms = excluded.last_delivery_at_ms, last_handshake_at_ms = excluded.last_handshake_at_ms, last_frame_at_ms = excluded.last_frame_at_ms, last_error_code = excluded.last_error_code, next_reconnect_at_ms = excluded.next_reconnect_at_ms, consecutive_failures = excluded.consecutive_failures, config_revision = excluded.config_revision, observed_at_ms = excluded.observed_at_ms")
                .bind(&health.provider_id)
                .bind(account_id)
                .bind(provider_health_state(health.state))
                .bind(&health.diagnostic)
                .bind(health.last_event_at_ms)
                .bind(health.last_delivery_at_ms)
                .bind(health.last_handshake_at_ms)
                .bind(health.last_frame_at_ms)
                .bind(&health.last_error_code)
                .bind(health.next_reconnect_at_ms)
                .bind(i64::from(health.consecutive_failures))
                .bind(to_i64(health.config_revision))
                .bind(timestamp_ms)
                .execute(self.store.pool())
                .await?;
        }
        Ok(())
    }

    pub async fn persisted_provider_health(
        &self,
    ) -> Result<Vec<ChannelProviderHealth>, GatewayError> {
        let rows = sqlx::query("SELECT provider_id, account_id, state, diagnostic, last_event_at_ms, last_delivery_at_ms, last_handshake_at_ms, last_frame_at_ms, last_error_code, next_reconnect_at_ms, consecutive_failures, config_revision FROM channel_provider_runtime_health ORDER BY provider_id, account_id")
            .fetch_all(self.store.pool())
            .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ChannelProviderHealth {
                    provider_id: row.get("provider_id"),
                    account_id: Some(row.get("account_id")),
                    state: parse_provider_health_state(row.get("state"))?,
                    diagnostic: row.get("diagnostic"),
                    last_event_at_ms: row.get("last_event_at_ms"),
                    last_delivery_at_ms: row.get("last_delivery_at_ms"),
                    last_handshake_at_ms: row.get("last_handshake_at_ms"),
                    last_frame_at_ms: row.get("last_frame_at_ms"),
                    last_error_code: row.get("last_error_code"),
                    next_reconnect_at_ms: row.get("next_reconnect_at_ms"),
                    consecutive_failures: to_u32(row.get("consecutive_failures")),
                    config_revision: from_i64(row.get("config_revision")),
                })
            })
            .collect()
    }

    pub async fn set_plugin_providers_enabled(
        &self,
        plugin_id: &hachimi_protocol::PluginId,
        enabled: bool,
    ) -> Result<(), GatewayError> {
        sqlx::query("UPDATE channel_provider_manifests SET enabled = ?, config_revision = config_revision + 1, updated_at_ms = ? WHERE plugin_id = ?")
            .bind(enabled)
            .bind(now_ms())
            .bind(plugin_id.as_str())
            .execute(self.store.pool())
            .await?;
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT provider_id FROM channel_provider_manifests WHERE plugin_id = ?",
        )
        .bind(plugin_id.as_str())
        .fetch_all(self.store.pool())
        .await?;
        let mut channels = self
            .channels
            .write()
            .map_err(|_| GatewayError::ProviderStatePoisoned)?;
        for id in ids {
            if enabled {
                channels.insert(id);
            } else {
                channels.remove(&id);
            }
        }
        Ok(())
    }

    pub async fn set_builtin_provider_contribution_enabled(
        &self,
        provider_id: &str,
        enabled: bool,
    ) -> Result<(), GatewayError> {
        sqlx::query("UPDATE channel_provider_manifests SET contribution_enabled = ?, config_revision = config_revision + 1, updated_at_ms = ? WHERE provider_id = ?")
            .bind(enabled)
            .bind(now_ms())
            .bind(provider_id)
            .execute(self.store.pool())
            .await?;
        let mut channels = self
            .channels
            .write()
            .map_err(|_| GatewayError::ProviderStatePoisoned)?;
        if enabled {
            channels.insert(provider_id.into());
        } else {
            channels.remove(provider_id);
        }
        Ok(())
    }

    pub async fn heartbeat(&self, process_id: u32, timestamp_ms: i64) -> Result<(), GatewayError> {
        sqlx::query("UPDATE gateway_runtime_state SET process_id = ?, last_heartbeat_ms = ?, updated_at_ms = ? WHERE singleton = 1")
            .bind(i64::from(process_id))
            .bind(timestamp_ms)
            .bind(timestamp_ms)
            .execute(self.store.pool())
            .await?;
        Ok(())
    }

    pub async fn record_runtime_start(
        &self,
        attempt: u32,
        timestamp_ms: i64,
    ) -> Result<(), GatewayError> {
        sqlx::query("UPDATE gateway_runtime_state SET process_id = NULL, last_started_at_ms = ?, restart_attempt = ?, last_error_code = NULL, revision = revision + 1, updated_at_ms = ? WHERE singleton = 1")
            .bind(timestamp_ms)
            .bind(i64::from(attempt))
            .bind(timestamp_ms)
            .execute(self.store.pool())
            .await?;
        Ok(())
    }

    pub async fn record_runtime_failure(
        &self,
        attempt: u32,
        error_code: &str,
        timestamp_ms: i64,
    ) -> Result<(), GatewayError> {
        sqlx::query("UPDATE gateway_runtime_state SET process_id = NULL, restart_attempt = ?, last_error_code = ?, revision = revision + 1, updated_at_ms = ? WHERE singleton = 1")
            .bind(i64::from(attempt))
            .bind(error_code)
            .bind(timestamp_ms)
            .execute(self.store.pool())
            .await?;
        Ok(())
    }

    pub async fn clear_runtime(&self, timestamp_ms: i64) -> Result<(), GatewayError> {
        sqlx::query("UPDATE gateway_runtime_state SET process_id = NULL, last_heartbeat_ms = NULL, updated_at_ms = ? WHERE singleton = 1")
            .bind(timestamp_ms)
            .execute(self.store.pool())
            .await?;
        Ok(())
    }

    pub async fn health(&self) -> Result<GatewayHealth, GatewayError> {
        let state = sqlx::query("SELECT revision, process_id, last_heartbeat_ms, last_started_at_ms, restart_attempt, last_error_code FROM gateway_runtime_state WHERE singleton = 1")
            .fetch_one(self.store.pool())
            .await?;
        let pending_ingress: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channel_ingress WHERE status IN ('accepted', 'claimed', 'run_created')")
            .fetch_one(self.store.pool())
            .await?;
        let pending_deliveries: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channel_outbox WHERE status IN ('pending', 'claimed', 'retry_scheduled')")
            .fetch_one(self.store.pool())
            .await?;
        let last_heartbeat_ms = state.get::<Option<i64>, _>("last_heartbeat_ms");
        let running = state.get::<Option<i64>, _>("process_id").is_some()
            && last_heartbeat_ms
                .is_some_and(|heartbeat| now_ms().saturating_sub(heartbeat) <= 15_000);
        let last_error_code = state.get::<Option<String>, _>("last_error_code");
        Ok(GatewayHealth {
            running,
            state: if running {
                RuntimeComponentState::Ready
            } else if last_error_code.is_some() {
                RuntimeComponentState::Retrying
            } else {
                RuntimeComponentState::Starting
            },
            last_heartbeat_ms,
            last_started_at_ms: state.get("last_started_at_ms"),
            restart_attempt: to_u32(state.get("restart_attempt")),
            last_error_code,
            channels: self
                .channels
                .read()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?
                .iter()
                .cloned()
                .collect(),
            pending_ingress: to_u32(pending_ingress),
            pending_deliveries: to_u32(pending_deliveries),
            revision: from_i64(state.get("revision")),
        })
    }
}

fn validate_manifest(manifest: &ChannelProviderManifest) -> Result<(), GatewayError> {
    if manifest.id.trim().is_empty()
        || manifest.id.len() > 128
        || manifest.content_hash.trim().is_empty()
        || (manifest.runtime_kind == ChannelProviderRuntimeKind::SandboxedStdioJsonRpc
            && (manifest.plugin_id.is_none() || manifest.entrypoint.is_none()))
    {
        return Err(GatewayError::InvalidProvider);
    }
    Ok(())
}

fn validate_account(account: &ChannelProviderAccount) -> Result<(), GatewayError> {
    if account.id.trim().is_empty()
        || account.provider_id.trim().is_empty()
        || account.tenant_key.trim().is_empty()
        || account.config_revision == 0
        || account.credential_revision == 0
    {
        return Err(GatewayError::InvalidProvider);
    }
    Ok(())
}

fn decode_account(row: sqlx::sqlite::SqliteRow) -> Result<ChannelProviderAccount, GatewayError> {
    Ok(ChannelProviderAccount {
        id: row.get("id"),
        provider_id: row.get("provider_id"),
        display_name: row.get("display_name"),
        tenant_key: row.get("tenant_key"),
        credential_ref: row.get("credential_ref"),
        enabled: row.get("enabled"),
        state: parse_account_state(row.get("state"))?,
        config: serde_json::from_str(row.get("config_json"))?,
        credential_revision: from_i64(row.get("credential_revision")),
        config_revision: from_i64(row.get("config_revision")),
    })
}

pub(crate) fn account_state(state: hachimi_protocol::ChannelAccountState) -> &'static str {
    use hachimi_protocol::ChannelAccountState;
    match state {
        ChannelAccountState::Draft => "draft",
        ChannelAccountState::AwaitingAuth => "awaiting_auth",
        ChannelAccountState::Starting => "starting",
        ChannelAccountState::Healthy => "healthy",
        ChannelAccountState::Degraded => "degraded",
        ChannelAccountState::NeedsAttention => "needs_attention",
        ChannelAccountState::Revoked => "revoked",
        ChannelAccountState::Removing => "removing",
    }
}

fn parse_account_state(value: &str) -> Result<hachimi_protocol::ChannelAccountState, GatewayError> {
    use hachimi_protocol::ChannelAccountState;
    match value {
        "draft" => Ok(ChannelAccountState::Draft),
        "awaiting_auth" => Ok(ChannelAccountState::AwaitingAuth),
        "starting" => Ok(ChannelAccountState::Starting),
        "healthy" => Ok(ChannelAccountState::Healthy),
        "degraded" => Ok(ChannelAccountState::Degraded),
        "needs_attention" => Ok(ChannelAccountState::NeedsAttention),
        "revoked" => Ok(ChannelAccountState::Revoked),
        "removing" => Ok(ChannelAccountState::Removing),
        _ => Err(GatewayError::InvalidProvider),
    }
}

fn provider_health_state(state: ChannelProviderHealthState) -> &'static str {
    match state {
        ChannelProviderHealthState::Disabled => "disabled",
        ChannelProviderHealthState::Starting => "starting",
        ChannelProviderHealthState::Healthy => "healthy",
        ChannelProviderHealthState::Degraded => "degraded",
        ChannelProviderHealthState::NeedsAttention => "needs_attention",
        ChannelProviderHealthState::Revoked => "revoked",
        ChannelProviderHealthState::Failed => "failed",
    }
}

fn parse_provider_health_state(value: &str) -> Result<ChannelProviderHealthState, GatewayError> {
    match value {
        "disabled" => Ok(ChannelProviderHealthState::Disabled),
        "starting" => Ok(ChannelProviderHealthState::Starting),
        "healthy" => Ok(ChannelProviderHealthState::Healthy),
        "degraded" => Ok(ChannelProviderHealthState::Degraded),
        "needs_attention" => Ok(ChannelProviderHealthState::NeedsAttention),
        "revoked" => Ok(ChannelProviderHealthState::Revoked),
        "failed" => Ok(ChannelProviderHealthState::Failed),
        _ => Err(GatewayError::InvalidProvider),
    }
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(i64::MAX)
}

pub(crate) fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

pub(crate) fn from_i64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

pub(crate) fn to_u32(value: i64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
