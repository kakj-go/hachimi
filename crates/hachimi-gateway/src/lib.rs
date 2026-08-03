mod enterprise_provider;
mod provider;
mod sidecar_provider;

pub use enterprise_provider::EnterpriseChannelProvider;
pub use provider::{
    ChannelDeliveryOutcome, ChannelProvider, ChannelProviderFuture, ChannelProviderRegistry,
};
pub use sidecar_provider::SandboxedStdioChannelProvider;

use std::{
    collections::BTreeSet,
    path::Path,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};

use hachimi_core::FeatureAvailability;
use hachimi_protocol::{
    ChannelDeliveryId, ChannelEnvelope, ChannelMessageId, ChannelProviderAccount,
    ChannelProviderHealth, ChannelProviderHealthState, ChannelProviderManifest,
    ChannelProviderRuntimeKind, ChannelRouteKey, DeliveryAttempt, DeliveryAttemptStatus,
    GatewayHealth, IngressReceipt, IngressStatus, RunId, SessionId,
};
use hachimi_storage::AgentStore;
use sha2::{Digest, Sha256};
use sqlx::Row;
use thiserror::Error;

const MAX_MESSAGE_CHARS: usize = 32_000;
const MAX_DELIVERY_ATTEMPTS: u32 = 8;

#[must_use]
pub const fn availability() -> FeatureAvailability {
    FeatureAvailability::Available
}

#[derive(Debug, Clone)]
pub struct LocalBuiltinProviders {
    pub registry: ChannelProviderRegistry,
    pub accounts: Vec<ChannelProviderAccount>,
    pub loopback: LoopbackWebhookChannel,
    pub mock_poll: MockPollChannel,
}

pub fn local_builtin_providers(
    store: AgentStore,
    loopback_token: &str,
) -> Result<LocalBuiltinProviders, GatewayError> {
    local_builtin_providers_with_enterprise(store, loopback_token, true)
}

pub fn local_builtin_providers_with_enterprise(
    store: AgentStore,
    loopback_token: &str,
    enterprise_integrations_enabled: bool,
) -> Result<LocalBuiltinProviders, GatewayError> {
    let loopback_route = ChannelRouteKey {
        channel: "loopback-webhook".into(),
        account: "local".into(),
        peer: "local-user".into(),
        thread: "main".into(),
    };
    let mock_route = ChannelRouteKey {
        channel: "mock-poll".into(),
        account: "local".into(),
        peer: "local-user".into(),
        thread: "main".into(),
    };
    let loopback =
        LoopbackWebhookChannel::new(loopback_token, BTreeSet::from([loopback_route.clone()]));
    let mock_poll = MockPollChannel::new(store.clone(), BTreeSet::from([mock_route.clone()]));
    let registry = ChannelProviderRegistry::default();
    registry.register(Arc::new(loopback.clone()))?;
    registry.register(Arc::new(mock_poll.clone()))?;
    if enterprise_integrations_enabled {
        for platform in [
            hachimi_protocol::EnterprisePlatform::Wecom,
            hachimi_protocol::EnterprisePlatform::DingTalk,
            hachimi_protocol::EnterprisePlatform::Feishu,
        ] {
            registry.register(Arc::new(EnterpriseChannelProvider::new(
                store.clone(),
                platform,
            )))?;
        }
    }
    Ok(LocalBuiltinProviders {
        registry,
        accounts: vec![
            ChannelProviderAccount {
                id: "loopback-local".into(),
                provider_id: "loopback-webhook".into(),
                display_name: "Local loopback webhook".into(),
                secret_ref: Some("keyring:channel:loopback-webhook:local".into()),
                enabled: true,
                route_allowlist: vec![loopback_route],
                config_revision: 1,
            },
            ChannelProviderAccount {
                id: "mock-local".into(),
                provider_id: "mock-poll".into(),
                display_name: "Local deterministic mock poll".into(),
                secret_ref: None,
                enabled: true,
                route_allowlist: vec![mock_route],
                config_revision: 1,
            },
        ],
        loopback,
        mock_poll,
    })
}

#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("gateway message is unauthenticated")]
    Unauthenticated,
    #[error("gateway route is not allowed")]
    RouteNotAllowed,
    #[error("gateway rejected a bot-generated loop")]
    BotLoop,
    #[error("gateway message is invalid")]
    InvalidMessage,
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
    #[error("gateway database failed: {0}")]
    Database(#[from] sqlx::Error),
    #[error("gateway storage failed: {0}")]
    Storage(#[from] hachimi_storage::AgentStoreError),
    #[error("gateway serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct GatewayHost {
    store: AgentStore,
    providers: ChannelProviderRegistry,
    channels: Arc<RwLock<BTreeSet<String>>>,
    provider_ingress_enabled: bool,
}

impl GatewayHost {
    #[must_use]
    pub fn new(store: AgentStore, channels: impl IntoIterator<Item = String>) -> Self {
        Self {
            store,
            providers: ChannelProviderRegistry::default(),
            channels: Arc::new(RwLock::new(channels.into_iter().collect())),
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
            provider_ingress_enabled: false,
        }
    }

    #[must_use]
    pub fn with_provider_ingress_enabled(mut self) -> Self {
        self.provider_ingress_enabled = true;
        self
    }

    pub async fn bootstrap_provider_accounts(
        &self,
        accounts: &[ChannelProviderAccount],
    ) -> Result<(), GatewayError> {
        let manifests = self.providers.manifests();
        let mut transaction = self.store.pool().begin().await?;
        for manifest in manifests {
            let enabled = accounts
                .iter()
                .any(|account| account.provider_id == manifest.id && account.enabled);
            sqlx::query("INSERT INTO channel_provider_manifests(provider_id, plugin_id, manifest_json, content_hash, enabled, contribution_enabled, config_revision, health, diagnostic, updated_at_ms) VALUES(?, ?, ?, ?, ?, 1, 1, ?, NULL, ?) ON CONFLICT(provider_id) DO UPDATE SET plugin_id = excluded.plugin_id, manifest_json = excluded.manifest_json, content_hash = excluded.content_hash, enabled = CASE WHEN channel_provider_manifests.content_hash = excluded.content_hash THEN channel_provider_manifests.enabled ELSE 0 END, config_revision = CASE WHEN channel_provider_manifests.content_hash = excluded.content_hash THEN channel_provider_manifests.config_revision ELSE channel_provider_manifests.config_revision + 1 END, health = CASE WHEN channel_provider_manifests.content_hash = excluded.content_hash THEN channel_provider_manifests.health ELSE 'needs_attention' END, diagnostic = CASE WHEN channel_provider_manifests.content_hash = excluded.content_hash THEN channel_provider_manifests.diagnostic ELSE 'channel_provider_revision_drift' END, updated_at_ms = excluded.updated_at_ms")
                .bind(&manifest.id)
                .bind(manifest.plugin_id.as_ref().map(hachimi_protocol::PluginId::as_str))
                .bind(serde_json::to_string(&manifest)?)
                .bind(&manifest.content_hash)
                .bind(enabled)
                .bind(if enabled { "starting" } else { "disabled" })
                .bind(now_ms())
                .execute(&mut *transaction)
                .await?;
        }
        for account in accounts {
            if self.providers.resolve(&account.provider_id).is_none()
                || account.id.trim().is_empty()
                || account.route_allowlist.is_empty()
                || account
                    .route_allowlist
                    .iter()
                    .any(|route| route.channel != account.provider_id)
            {
                return Err(GatewayError::InvalidProvider);
            }
            sqlx::query("INSERT INTO channel_provider_accounts(id, provider_id, display_name, secret_ref, enabled, route_allowlist_json, config_revision, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name, secret_ref = COALESCE(channel_provider_accounts.secret_ref, excluded.secret_ref), updated_at_ms = excluded.updated_at_ms")
                .bind(&account.id)
                .bind(&account.provider_id)
                .bind(&account.display_name)
                .bind(&account.secret_ref)
                .bind(account.enabled)
                .bind(serde_json::to_string(&account.route_allowlist)?)
                .bind(i64::try_from(account.config_revision).unwrap_or(i64::MAX))
                .bind(now_ms())
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        self.reload_configuration().await
    }

    pub async fn reload_configuration(&self) -> Result<(), GatewayError> {
        let rows = sqlx::query("SELECT account.id, account.provider_id, account.display_name, account.secret_ref, account.enabled, account.route_allowlist_json, account.config_revision FROM channel_provider_accounts AS account INNER JOIN channel_provider_manifests AS manifest ON manifest.provider_id = account.provider_id WHERE account.enabled = 1 AND manifest.enabled = 1 AND manifest.contribution_enabled = 1 ORDER BY account.provider_id, account.id")
            .fetch_all(self.store.pool())
            .await?;
        let mut enabled = BTreeSet::new();
        for row in rows {
            let account = ChannelProviderAccount {
                id: row.get("id"),
                provider_id: row.get("provider_id"),
                display_name: row.get("display_name"),
                secret_ref: row.get("secret_ref"),
                enabled: row.get("enabled"),
                route_allowlist: serde_json::from_str(row.get("route_allowlist_json"))?,
                config_revision: u64::try_from(row.get::<i64, _>("config_revision"))
                    .unwrap_or_default(),
            };
            let provider = self
                .providers
                .resolve(&account.provider_id)
                .ok_or(GatewayError::ProviderUnavailable)?;
            let result = async {
                provider.reload(&account).await?;
                provider.start().await?;
                if self.provider_ingress_enabled {
                    provider.start_ingress().await?;
                }
                provider.health().await
            }
            .await;
            match result {
                Ok(health) if health.state == ChannelProviderHealthState::Healthy => {
                    enabled.insert(account.provider_id.clone());
                    self.persist_provider_health(&health).await?;
                }
                Ok(health) => self.persist_provider_health(&health).await?,
                Err(error) => {
                    sqlx::query("UPDATE channel_provider_manifests SET health = 'failed', diagnostic = ?, updated_at_ms = ? WHERE provider_id = ?")
                        .bind(error.to_string())
                        .bind(now_ms())
                        .bind(&account.provider_id)
                        .execute(self.store.pool())
                        .await?;
                }
            }
        }
        for provider_id in self.providers.provider_ids() {
            if !enabled.contains(&provider_id)
                && let Some(provider) = self.providers.resolve(&provider_id)
            {
                let _ = provider.stop().await;
            }
        }
        *self
            .channels
            .write()
            .map_err(|_| GatewayError::ProviderStatePoisoned)? = enabled;
        Ok(())
    }

    pub async fn list_provider_accounts(
        &self,
    ) -> Result<Vec<ChannelProviderAccount>, GatewayError> {
        let rows = sqlx::query("SELECT id, provider_id, display_name, secret_ref, enabled, route_allowlist_json, config_revision FROM channel_provider_accounts ORDER BY provider_id, id")
            .fetch_all(self.store.pool())
            .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ChannelProviderAccount {
                    id: row.get("id"),
                    provider_id: row.get("provider_id"),
                    display_name: row.get("display_name"),
                    secret_ref: row.get("secret_ref"),
                    enabled: row.get("enabled"),
                    route_allowlist: serde_json::from_str(row.get("route_allowlist_json"))?,
                    config_revision: u64::try_from(row.get::<i64, _>("config_revision"))
                        .unwrap_or_default(),
                })
            })
            .collect()
    }

    pub async fn upsert_provider_account(
        &self,
        mut account: ChannelProviderAccount,
        expected_config_revision: Option<u64>,
        preserve_existing_secret: bool,
    ) -> Result<ChannelProviderAccount, GatewayError> {
        if self.providers.resolve(&account.provider_id).is_none()
            || account.id.trim().is_empty()
            || account.display_name.trim().is_empty()
            || account.route_allowlist.is_empty()
            || account
                .route_allowlist
                .iter()
                .any(|route| route.channel != account.provider_id)
        {
            return Err(GatewayError::InvalidProvider);
        }
        let current = sqlx::query(
            "SELECT config_revision, secret_ref FROM channel_provider_accounts WHERE id = ?",
        )
        .bind(&account.id)
        .fetch_optional(self.store.pool())
        .await?;
        let current_revision = current
            .as_ref()
            .map(|row| u64::try_from(row.get::<i64, _>("config_revision")).unwrap_or_default());
        if current_revision != expected_config_revision {
            return Err(GatewayError::ProviderRevisionConflict);
        }
        if preserve_existing_secret && account.secret_ref.is_none() {
            account.secret_ref = current.and_then(|row| row.get("secret_ref"));
        }
        account.config_revision = current_revision.unwrap_or_default().saturating_add(1);
        sqlx::query("INSERT INTO channel_provider_accounts(id, provider_id, display_name, secret_ref, enabled, route_allowlist_json, config_revision, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET provider_id = excluded.provider_id, display_name = excluded.display_name, secret_ref = excluded.secret_ref, enabled = excluded.enabled, route_allowlist_json = excluded.route_allowlist_json, config_revision = excluded.config_revision, updated_at_ms = excluded.updated_at_ms")
            .bind(&account.id)
            .bind(&account.provider_id)
            .bind(account.display_name.trim())
            .bind(&account.secret_ref)
            .bind(account.enabled)
            .bind(serde_json::to_string(&account.route_allowlist)?)
            .bind(i64::try_from(account.config_revision).unwrap_or(i64::MAX))
            .bind(now_ms())
            .execute(self.store.pool())
            .await?;
        sqlx::query("UPDATE channel_provider_manifests SET enabled = contribution_enabled = 1 AND EXISTS(SELECT 1 FROM channel_provider_accounts WHERE provider_id = ? AND enabled = 1), config_revision = config_revision + 1, updated_at_ms = ? WHERE provider_id = ?")
            .bind(&account.provider_id)
            .bind(now_ms())
            .bind(&account.provider_id)
            .execute(self.store.pool())
            .await?;
        self.reload_configuration().await?;
        Ok(account)
    }

    pub async fn register_provider(
        &self,
        provider: Arc<dyn ChannelProvider>,
        contribution_enabled: bool,
    ) -> Result<(), GatewayError> {
        let manifest = provider.manifest();
        self.providers.register(provider)?;
        sqlx::query("INSERT INTO channel_provider_manifests(provider_id, plugin_id, manifest_json, content_hash, enabled, contribution_enabled, config_revision, health, diagnostic, updated_at_ms) VALUES(?, ?, ?, ?, 0, ?, 1, 'disabled', NULL, ?) ON CONFLICT(provider_id) DO UPDATE SET plugin_id = excluded.plugin_id, manifest_json = excluded.manifest_json, content_hash = excluded.content_hash, contribution_enabled = excluded.contribution_enabled, enabled = CASE WHEN channel_provider_manifests.content_hash = excluded.content_hash AND excluded.contribution_enabled = 1 THEN EXISTS(SELECT 1 FROM channel_provider_accounts WHERE provider_id = excluded.provider_id AND enabled = 1) ELSE 0 END, config_revision = CASE WHEN channel_provider_manifests.content_hash = excluded.content_hash THEN channel_provider_manifests.config_revision ELSE channel_provider_manifests.config_revision + 1 END, health = CASE WHEN channel_provider_manifests.content_hash = excluded.content_hash THEN CASE WHEN excluded.contribution_enabled = 1 THEN channel_provider_manifests.health ELSE 'disabled' END ELSE 'needs_attention' END, diagnostic = CASE WHEN channel_provider_manifests.content_hash = excluded.content_hash THEN NULL ELSE 'channel_provider_revision_drift' END, updated_at_ms = excluded.updated_at_ms")
            .bind(&manifest.id)
            .bind(manifest.plugin_id.as_ref().map(hachimi_protocol::PluginId::as_str))
            .bind(serde_json::to_string(&manifest)?)
            .bind(&manifest.content_hash)
            .bind(contribution_enabled)
            .bind(now_ms())
            .execute(self.store.pool())
            .await?;
        self.reload_configuration().await
    }

    pub async fn set_plugin_providers_enabled(
        &self,
        plugin_id: &hachimi_protocol::PluginId,
        enabled: bool,
    ) -> Result<(), GatewayError> {
        sqlx::query("UPDATE channel_provider_manifests SET contribution_enabled = ?, enabled = CASE WHEN ? = 1 THEN EXISTS(SELECT 1 FROM channel_provider_accounts WHERE provider_id = channel_provider_manifests.provider_id AND enabled = 1) ELSE 0 END, health = CASE WHEN ? = 1 THEN health ELSE 'disabled' END, diagnostic = CASE WHEN ? = 1 THEN diagnostic ELSE NULL END, config_revision = config_revision + 1, updated_at_ms = ? WHERE plugin_id = ?")
            .bind(enabled)
            .bind(enabled)
            .bind(enabled)
            .bind(enabled)
            .bind(now_ms())
            .bind(plugin_id.as_str())
            .execute(self.store.pool())
            .await?;
        self.reload_configuration().await
    }

    pub async fn set_builtin_provider_contribution_enabled(
        &self,
        provider_id: &str,
        enabled: bool,
    ) -> Result<(), GatewayError> {
        let manifest = self
            .providers
            .resolve(provider_id)
            .map(|provider| provider.manifest())
            .filter(|manifest| {
                manifest.runtime_kind == ChannelProviderRuntimeKind::Builtin
                    && manifest.plugin_id.is_none()
                    && matches!(manifest.id.as_str(), "wecom" | "dingtalk" | "feishu")
            })
            .ok_or(GatewayError::InvalidProvider)?;
        sqlx::query("UPDATE channel_provider_manifests SET contribution_enabled = ?, enabled = CASE WHEN ? = 1 THEN EXISTS(SELECT 1 FROM channel_provider_accounts WHERE provider_id = channel_provider_manifests.provider_id AND enabled = 1) ELSE 0 END, health = CASE WHEN ? = 1 THEN health ELSE 'disabled' END, diagnostic = CASE WHEN ? = 1 THEN diagnostic ELSE NULL END, config_revision = config_revision + 1, updated_at_ms = ? WHERE provider_id = ? AND plugin_id IS NULL")
            .bind(enabled)
            .bind(enabled)
            .bind(enabled)
            .bind(enabled)
            .bind(now_ms())
            .bind(&manifest.id)
            .execute(self.store.pool())
            .await?;
        self.reload_configuration().await
    }

    pub async fn provider_health(&self) -> Result<Vec<ChannelProviderHealth>, GatewayError> {
        let mut values = Vec::new();
        for provider_id in self.providers.provider_ids() {
            if let Some(provider) = self.providers.resolve(&provider_id) {
                values.push(provider.health().await?);
            }
        }
        Ok(values)
    }

    pub async fn provider_manifests(&self) -> Result<Vec<ChannelProviderManifest>, GatewayError> {
        let rows = sqlx::query(
            "SELECT manifest_json FROM channel_provider_manifests ORDER BY provider_id",
        )
        .fetch_all(self.store.pool())
        .await?;
        rows.into_iter()
            .map(|row| serde_json::from_str(row.get("manifest_json")).map_err(GatewayError::from))
            .collect()
    }

    pub async fn ingest_provider(
        &self,
        provider_id: &str,
        credential: Option<&str>,
        envelope: ChannelEnvelope,
    ) -> Result<IngressReceipt, GatewayError> {
        let provider = self
            .providers
            .resolve(provider_id)
            .ok_or(GatewayError::ProviderUnavailable)?;
        let envelope = provider.receive(credential, envelope).await?;
        let rows = sqlx::query("SELECT route_allowlist_json FROM channel_provider_accounts WHERE provider_id = ? AND enabled = 1 ORDER BY id")
            .bind(provider_id)
            .fetch_all(self.store.pool())
            .await?;
        let mut allowed_routes = BTreeSet::new();
        for row in rows {
            allowed_routes.extend(serde_json::from_str::<Vec<ChannelRouteKey>>(
                row.get("route_allowlist_json"),
            )?);
        }
        if allowed_routes.is_empty() {
            return Err(GatewayError::RouteNotAllowed);
        }
        let receipt = self.ingest(&envelope, &allowed_routes).await?;
        provider.ack_ingress(&envelope, &receipt).await?;
        Ok(receipt)
    }

    pub async fn deliver_with_provider(
        &self,
        delivery: &DeliveryAttempt,
    ) -> Result<ChannelDeliveryOutcome, GatewayError> {
        let provider = self
            .providers
            .resolve(&delivery.route.channel)
            .ok_or(GatewayError::ProviderUnavailable)?;
        let outcome = provider.deliver(delivery).await?;
        if outcome.delivered {
            provider.ack(delivery).await?;
        }
        Ok(outcome)
    }

    pub async fn process_next_provider_delivery(
        &self,
        now_ms: i64,
    ) -> Result<Option<DeliveryAttempt>, GatewayError> {
        for provider_id in self.providers.provider_ids() {
            let Some(provider) = self.providers.resolve(&provider_id) else {
                continue;
            };
            if !provider.push_delivery() || !self.channel_enabled(&provider_id) {
                continue;
            }
            let Some(delivery) = self
                .claim_next_delivery_for_channel(&provider_id, now_ms)
                .await?
            else {
                continue;
            };
            let outcome = provider.deliver(&delivery).await?;
            if outcome.delivered {
                provider.ack(&delivery).await?;
            }
            let completed = self
                .finish_delivery(
                    &delivery.id,
                    outcome.delivered,
                    outcome.retryable,
                    (!outcome.delivered).then_some(outcome.result_code.as_str()),
                    now_ms,
                )
                .await?;
            return Ok(Some(completed));
        }
        Ok(None)
    }

    pub async fn process_next_provider_ingress(
        &self,
    ) -> Result<Option<IngressReceipt>, GatewayError> {
        for provider_id in self.providers.provider_ids() {
            if !self.channel_enabled(&provider_id) {
                continue;
            }
            let Some(provider) = self.providers.resolve(&provider_id) else {
                continue;
            };
            let Some(envelope) = provider.claim_ingress().await? else {
                continue;
            };
            return self
                .ingest_provider(&provider_id, None, envelope)
                .await
                .map(Some);
        }
        Ok(None)
    }

    async fn persist_provider_health(
        &self,
        health: &ChannelProviderHealth,
    ) -> Result<(), GatewayError> {
        sqlx::query("UPDATE channel_provider_manifests SET health = ?, diagnostic = ?, config_revision = MAX(config_revision, ?), updated_at_ms = ? WHERE provider_id = ?")
            .bind(provider_health_state(health.state))
            .bind(&health.diagnostic)
            .bind(i64::try_from(health.config_revision).unwrap_or(i64::MAX))
            .bind(now_ms())
            .bind(&health.provider_id)
            .execute(self.store.pool())
            .await?;
        Ok(())
    }

    fn channel_enabled(&self, channel: &str) -> bool {
        self.channels
            .read()
            .map(|channels| channels.contains(channel))
            .unwrap_or(false)
    }

    pub async fn ingest(
        &self,
        envelope: &ChannelEnvelope,
        allowed_routes: &BTreeSet<ChannelRouteKey>,
    ) -> Result<IngressReceipt, GatewayError> {
        validate_envelope(envelope)?;
        if !envelope.authenticated {
            return Err(GatewayError::Unauthenticated);
        }
        if envelope.bot_generated {
            return Err(GatewayError::BotLoop);
        }
        if !self.channel_enabled(&envelope.route.channel)
            || !allowed_routes.contains(&envelope.route)
        {
            return Err(GatewayError::RouteNotAllowed);
        }
        let now = envelope.received_at_ms;
        let result = sqlx::query(
            "INSERT OR IGNORE INTO channel_ingress(message_id, route_key, envelope_json, status, session_id, run_id, result_code, received_at_ms, updated_at_ms) VALUES(?, ?, ?, 'accepted', NULL, NULL, 'accepted', ?, ?)",
        )
        .bind(envelope.message_id.as_str())
        .bind(route_key(&envelope.route)?)
        .bind(serde_json::to_string(envelope)?)
        .bind(now)
        .bind(now)
        .execute(self.store.pool())
        .await?;
        if result.rows_affected() == 0 {
            self.append_channel_audit(&envelope.route, "channel.ingress", "duplicate", now)
                .await?;
            return Ok(IngressReceipt {
                message_id: envelope.message_id.clone(),
                status: IngressStatus::Duplicate,
                session_id: None,
                run_id: None,
                result_code: "duplicate".into(),
            });
        }
        self.append_channel_audit(&envelope.route, "channel.ingress", "accepted", now)
            .await?;
        Ok(IngressReceipt {
            message_id: envelope.message_id.clone(),
            status: IngressStatus::Accepted,
            session_id: None,
            run_id: None,
            result_code: "accepted".into(),
        })
    }

    pub async fn claim_next_ingress(
        &self,
        now_ms: i64,
    ) -> Result<Option<ChannelEnvelope>, GatewayError> {
        let mut transaction = self.store.pool().begin().await?;
        let row = sqlx::query(
            "SELECT message_id, envelope_json FROM channel_ingress WHERE status = 'accepted' ORDER BY received_at_ms, message_id LIMIT 1",
        )
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else {
            transaction.commit().await?;
            return Ok(None);
        };
        let message_id = row.get::<String, _>("message_id");
        let updated = sqlx::query(
            "UPDATE channel_ingress SET status = 'claimed', result_code = 'claimed', updated_at_ms = ? WHERE message_id = ? AND status = 'accepted'",
        )
        .bind(now_ms)
        .bind(&message_id)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(GatewayError::IngressConflict);
        }
        let envelope = serde_json::from_str(row.get("envelope_json"))?;
        transaction.commit().await?;
        Ok(Some(envelope))
    }

    pub async fn session_for_route(
        &self,
        route: &ChannelRouteKey,
    ) -> Result<Option<SessionId>, GatewayError> {
        let row = sqlx::query("SELECT session_id FROM channel_session_routes WHERE route_key = ?")
            .bind(route_key(route)?)
            .fetch_optional(self.store.pool())
            .await?;
        Ok(row.map(|row| SessionId::new(row.get::<String, _>("session_id"))))
    }

    pub async fn bind_route(
        &self,
        route: &ChannelRouteKey,
        session_id: &SessionId,
        now_ms: i64,
    ) -> Result<(), GatewayError> {
        sqlx::query(
            "INSERT INTO channel_session_routes(route_key, session_id, updated_at_ms) VALUES(?, ?, ?) ON CONFLICT(route_key) DO UPDATE SET session_id = excluded.session_id, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(route_key(route)?)
        .bind(session_id.as_str())
        .bind(now_ms)
        .execute(self.store.pool())
        .await?;
        Ok(())
    }

    pub async fn finish_ingress(
        &self,
        message_id: &ChannelMessageId,
        session_id: &SessionId,
        run_id: &RunId,
        needs_attention: bool,
        now_ms: i64,
    ) -> Result<IngressReceipt, GatewayError> {
        let status = if needs_attention {
            "needs_attention"
        } else {
            "completed"
        };
        let result = sqlx::query(
            "UPDATE channel_ingress SET status = ?, session_id = ?, run_id = ?, result_code = ?, updated_at_ms = ? WHERE message_id = ? AND status = 'claimed'",
        )
        .bind(status)
        .bind(session_id.as_str())
        .bind(run_id.as_str())
        .bind(status)
        .bind(now_ms)
        .bind(message_id.as_str())
        .execute(self.store.pool())
        .await?;
        if result.rows_affected() != 1 {
            return Err(GatewayError::IngressConflict);
        }
        Ok(IngressReceipt {
            message_id: message_id.clone(),
            status: if needs_attention {
                IngressStatus::NeedsAttention
            } else {
                IngressStatus::Completed
            },
            session_id: Some(session_id.clone()),
            run_id: Some(run_id.clone()),
            result_code: status.into(),
        })
    }

    pub async fn fail_ingress(
        &self,
        message_id: &ChannelMessageId,
        result_code: &str,
        now_ms: i64,
    ) -> Result<IngressReceipt, GatewayError> {
        if result_code.trim().is_empty() || result_code.chars().count() > 128 {
            return Err(GatewayError::InvalidMessage);
        }
        let result = sqlx::query(
            "UPDATE channel_ingress SET status = 'needs_attention', result_code = ?, updated_at_ms = ? WHERE message_id = ? AND status = 'claimed'",
        )
        .bind(result_code)
        .bind(now_ms)
        .bind(message_id.as_str())
        .execute(self.store.pool())
        .await?;
        if result.rows_affected() != 1 {
            return Err(GatewayError::IngressConflict);
        }
        Ok(IngressReceipt {
            message_id: message_id.clone(),
            status: IngressStatus::NeedsAttention,
            session_id: None,
            run_id: None,
            result_code: result_code.into(),
        })
    }

    pub async fn enqueue_delivery(
        &self,
        route: ChannelRouteKey,
        idempotency_key: &str,
        text: &str,
        now_ms: i64,
    ) -> Result<DeliveryAttempt, GatewayError> {
        if idempotency_key.trim().is_empty()
            || idempotency_key.len() > 128
            || text.trim().is_empty()
            || text.chars().count() > MAX_MESSAGE_CHARS
        {
            return Err(GatewayError::InvalidMessage);
        }
        let candidate = DeliveryAttempt {
            id: ChannelDeliveryId::random(),
            route: route.clone(),
            idempotency_key: idempotency_key.into(),
            text: text.into(),
            status: DeliveryAttemptStatus::Pending,
            attempt: 0,
            next_attempt_at_ms: Some(now_ms),
            error_code: None,
        };
        sqlx::query(
            "INSERT OR IGNORE INTO channel_deliveries(id, route_key, idempotency_key, text, status, attempt, next_attempt_at_ms, error_code, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, 'pending', 0, ?, NULL, ?, ?)",
        )
        .bind(candidate.id.as_str())
        .bind(route_key(&route)?)
        .bind(idempotency_key)
        .bind(text)
        .bind(now_ms)
        .bind(now_ms)
        .bind(now_ms)
        .execute(self.store.pool())
        .await?;
        self.delivery_by_key(idempotency_key)
            .await?
            .ok_or(GatewayError::DeliveryConflict)
    }

    pub async fn claim_next_delivery(
        &self,
        now_ms: i64,
    ) -> Result<Option<DeliveryAttempt>, GatewayError> {
        let mut transaction = self.store.pool().begin().await?;
        let row = sqlx::query(
            "SELECT id, route_key, idempotency_key, text, status, attempt, next_attempt_at_ms, error_code FROM channel_deliveries WHERE status IN ('pending', 'retry_scheduled') AND COALESCE(next_attempt_at_ms, 0) <= ? ORDER BY COALESCE(next_attempt_at_ms, 0), created_at_ms, id LIMIT 1",
        )
        .bind(now_ms)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else {
            transaction.commit().await?;
            return Ok(None);
        };
        let id = row.get::<String, _>("id");
        let updated = sqlx::query(
            "UPDATE channel_deliveries SET status = 'claimed', attempt = attempt + 1, next_attempt_at_ms = NULL, updated_at_ms = ? WHERE id = ? AND status IN ('pending', 'retry_scheduled')",
        )
        .bind(now_ms)
        .bind(&id)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(GatewayError::DeliveryConflict);
        }
        transaction.commit().await?;
        self.delivery(&ChannelDeliveryId::new(id)).await
    }

    pub async fn claim_next_delivery_for_channel(
        &self,
        channel: &str,
        now_ms: i64,
    ) -> Result<Option<DeliveryAttempt>, GatewayError> {
        if !self.channel_enabled(channel) {
            return Err(GatewayError::RouteNotAllowed);
        }
        let mut transaction = self.store.pool().begin().await?;
        let row = sqlx::query(
            "SELECT id FROM channel_deliveries WHERE status IN ('pending', 'retry_scheduled') AND COALESCE(next_attempt_at_ms, 0) <= ? AND json_extract(route_key, '$.channel') = ? ORDER BY COALESCE(next_attempt_at_ms, 0), created_at_ms, id LIMIT 1",
        )
        .bind(now_ms)
        .bind(channel)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else {
            transaction.commit().await?;
            return Ok(None);
        };
        let id = row.get::<String, _>("id");
        let updated = sqlx::query(
            "UPDATE channel_deliveries SET status = 'claimed', attempt = attempt + 1, next_attempt_at_ms = NULL, updated_at_ms = ? WHERE id = ? AND status IN ('pending', 'retry_scheduled')",
        )
        .bind(now_ms)
        .bind(&id)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(GatewayError::DeliveryConflict);
        }
        transaction.commit().await?;
        self.delivery(&ChannelDeliveryId::new(id)).await
    }

    pub async fn finish_delivery(
        &self,
        delivery_id: &ChannelDeliveryId,
        succeeded: bool,
        retryable: bool,
        error_code: Option<&str>,
        now_ms: i64,
    ) -> Result<DeliveryAttempt, GatewayError> {
        let current = self
            .delivery(delivery_id)
            .await?
            .ok_or(GatewayError::DeliveryConflict)?;
        if current.status != DeliveryAttemptStatus::Claimed {
            return Err(GatewayError::DeliveryConflict);
        }
        let (status, next_attempt_at_ms) = if succeeded {
            ("delivered", None)
        } else if retryable && current.attempt < MAX_DELIVERY_ATTEMPTS {
            let exponent = current.attempt.saturating_sub(1).min(10);
            let delay_ms = 1_000_i64.saturating_mul(1_i64 << exponent);
            ("retry_scheduled", Some(now_ms.saturating_add(delay_ms)))
        } else {
            ("failed", None)
        };
        sqlx::query(
            "UPDATE channel_deliveries SET status = ?, next_attempt_at_ms = ?, error_code = ?, updated_at_ms = ? WHERE id = ? AND status = 'claimed'",
        )
        .bind(status)
        .bind(next_attempt_at_ms)
        .bind(error_code)
        .bind(now_ms)
        .bind(delivery_id.as_str())
        .execute(self.store.pool())
        .await?;
        self.append_channel_audit(
            &current.route,
            "channel.delivery",
            if succeeded { "delivered" } else { status },
            now_ms,
        )
        .await?;
        self.delivery(delivery_id)
            .await?
            .ok_or(GatewayError::DeliveryConflict)
    }

    async fn append_channel_audit(
        &self,
        route: &ChannelRouteKey,
        operation: &str,
        result_code: &str,
        created_at_ms: i64,
    ) -> Result<(), GatewayError> {
        let route_bytes = serde_json::to_vec(route)?;
        let route_hash = Sha256::digest(route_bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        self.store
            .append_audit_metadata(hachimi_storage::AuditMetadataRecord {
                principal: format!("channel:{}", route.channel),
                session_id: None,
                run_id: None,
                run_generation: None,
                operation: operation.into(),
                target_summary: format!("channel:{}:route_sha256:{route_hash}", route.channel),
                decision: if result_code == "accepted" || result_code == "delivered" {
                    "allowed".into()
                } else {
                    "observed".into()
                },
                result_code: result_code.into(),
                created_at_ms,
            })
            .await?;
        Ok(())
    }

    pub async fn set_startup_registration(
        &self,
        executable: &Path,
        enabled: bool,
        now_ms: i64,
    ) -> Result<GatewayHealth, GatewayError> {
        update_user_startup(executable, enabled)?;
        sqlx::query(
            "UPDATE gateway_runtime_state SET startup_registered = ?, revision = revision + 1, updated_at_ms = ? WHERE singleton = 1",
        )
        .bind(enabled)
        .bind(now_ms)
        .execute(self.store.pool())
        .await?;
        self.health().await
    }

    pub async fn reconcile_startup(&self, now_ms: i64) -> Result<(), GatewayError> {
        sqlx::query(
            "UPDATE channel_ingress SET status = 'accepted', result_code = 'reconciled', updated_at_ms = ? WHERE status = 'claimed'",
        )
        .bind(now_ms)
        .execute(self.store.pool())
        .await?;
        sqlx::query(
            "UPDATE channel_deliveries SET status = 'retry_scheduled', next_attempt_at_ms = ?, error_code = 'gateway_restarted', updated_at_ms = ? WHERE status = 'claimed'",
        )
        .bind(now_ms)
        .bind(now_ms)
        .execute(self.store.pool())
        .await?;
        Ok(())
    }

    pub async fn heartbeat(&self, process_id: u32, now_ms: i64) -> Result<(), GatewayError> {
        sqlx::query(
            "UPDATE gateway_runtime_state SET process_id = ?, last_heartbeat_ms = ?, revision = revision + 1, updated_at_ms = ? WHERE singleton = 1",
        )
        .bind(i64::from(process_id))
        .bind(now_ms)
        .bind(now_ms)
        .execute(self.store.pool())
        .await?;
        Ok(())
    }

    pub async fn health(&self) -> Result<GatewayHealth, GatewayError> {
        let state = sqlx::query(
            "SELECT startup_registered, revision, last_heartbeat_ms FROM gateway_runtime_state WHERE singleton = 1",
        )
        .fetch_one(self.store.pool())
        .await?;
        let pending_ingress = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM channel_ingress WHERE status IN ('accepted', 'claimed')",
        )
        .fetch_one(self.store.pool())
        .await?;
        let pending_deliveries = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM channel_deliveries WHERE status IN ('pending', 'claimed', 'retry_scheduled')",
        )
        .fetch_one(self.store.pool())
        .await?;
        Ok(GatewayHealth {
            running: state
                .get::<Option<i64>, _>("last_heartbeat_ms")
                .is_some_and(|heartbeat| now_ms().saturating_sub(heartbeat) <= 15_000),
            startup_registered: state.get::<bool, _>("startup_registered"),
            channels: self
                .channels
                .read()
                .map(|channels| channels.iter().cloned().collect())
                .unwrap_or_default(),
            pending_ingress: u32::try_from(pending_ingress).unwrap_or(u32::MAX),
            pending_deliveries: u32::try_from(pending_deliveries).unwrap_or(u32::MAX),
            revision: u64::try_from(state.get::<i64, _>("revision")).unwrap_or_default(),
        })
    }

    async fn delivery_by_key(
        &self,
        idempotency_key: &str,
    ) -> Result<Option<DeliveryAttempt>, GatewayError> {
        let row = sqlx::query(
            "SELECT id, route_key, idempotency_key, text, status, attempt, next_attempt_at_ms, error_code FROM channel_deliveries WHERE idempotency_key = ?",
        )
        .bind(idempotency_key)
        .fetch_optional(self.store.pool())
        .await?;
        row.map(decode_delivery).transpose()
    }

    async fn delivery(
        &self,
        id: &ChannelDeliveryId,
    ) -> Result<Option<DeliveryAttempt>, GatewayError> {
        let row = sqlx::query(
            "SELECT id, route_key, idempotency_key, text, status, attempt, next_attempt_at_ms, error_code FROM channel_deliveries WHERE id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(self.store.pool())
        .await?;
        row.map(decode_delivery).transpose()
    }
}

#[derive(Debug, Clone)]
pub struct LoopbackWebhookChannel {
    token_hash: String,
    allowed_routes: BTreeSet<ChannelRouteKey>,
    account: Arc<RwLock<Option<ChannelProviderAccount>>>,
    state: Arc<RwLock<ChannelProviderHealthState>>,
}

impl LoopbackWebhookChannel {
    #[must_use]
    pub fn new(token: &str, allowed_routes: BTreeSet<ChannelRouteKey>) -> Self {
        Self {
            token_hash: digest(token.as_bytes()),
            allowed_routes,
            account: Arc::new(RwLock::new(None)),
            state: Arc::new(RwLock::new(ChannelProviderHealthState::Disabled)),
        }
    }

    pub async fn receive(
        &self,
        gateway: &GatewayHost,
        token: &str,
        mut envelope: ChannelEnvelope,
    ) -> Result<IngressReceipt, GatewayError> {
        if digest(token.as_bytes()) != self.token_hash {
            return Err(GatewayError::Unauthenticated);
        }
        envelope.authenticated = true;
        gateway.ingest(&envelope, &self.allowed_routes).await
    }
}

impl ChannelProvider for LoopbackWebhookChannel {
    fn manifest(&self) -> ChannelProviderManifest {
        ChannelProviderManifest {
            id: "loopback-webhook".into(),
            plugin_id: None,
            runtime_kind: hachimi_protocol::ChannelProviderRuntimeKind::Builtin,
            entrypoint: None,
            content_hash: digest(b"hachimi.channel.loopback-webhook.v1"),
            required_scopes: vec!["channel.receive".into(), "channel.deliver".into()],
        }
    }

    fn push_delivery(&self) -> bool {
        false
    }

    fn configure<'a>(
        &'a self,
        account: &'a ChannelProviderAccount,
    ) -> ChannelProviderFuture<'a, ()> {
        let valid = account.provider_id == "loopback-webhook"
            && account.enabled
            && !account.route_allowlist.is_empty()
            && account
                .route_allowlist
                .iter()
                .all(|route| route.channel == "loopback-webhook");
        Box::pin(async move {
            if !valid {
                return Err(GatewayError::InvalidProvider);
            }
            *self
                .account
                .write()
                .map_err(|_| GatewayError::ProviderStatePoisoned)? = Some(account.clone());
            Ok(())
        })
    }

    fn start<'a>(&'a self) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move {
            *self
                .state
                .write()
                .map_err(|_| GatewayError::ProviderStatePoisoned)? =
                ChannelProviderHealthState::Healthy;
            Ok(())
        })
    }

    fn stop<'a>(&'a self) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move {
            *self
                .state
                .write()
                .map_err(|_| GatewayError::ProviderStatePoisoned)? =
                ChannelProviderHealthState::Disabled;
            Ok(())
        })
    }

    fn health<'a>(&'a self) -> ChannelProviderFuture<'a, ChannelProviderHealth> {
        Box::pin(async move {
            Ok(ChannelProviderHealth {
                provider_id: "loopback-webhook".into(),
                state: *self
                    .state
                    .read()
                    .map_err(|_| GatewayError::ProviderStatePoisoned)?,
                diagnostic: None,
                config_revision: self
                    .account
                    .read()
                    .map_err(|_| GatewayError::ProviderStatePoisoned)?
                    .as_ref()
                    .map_or(0, |account| account.config_revision),
            })
        })
    }

    fn receive<'a>(
        &'a self,
        credential: Option<&'a str>,
        mut envelope: ChannelEnvelope,
    ) -> ChannelProviderFuture<'a, ChannelEnvelope> {
        Box::pin(async move {
            let credential = credential.ok_or(GatewayError::Unauthenticated)?;
            if digest(credential.as_bytes()) != self.token_hash {
                return Err(GatewayError::Unauthenticated);
            }
            if !self.allowed_routes.contains(&envelope.route) {
                return Err(GatewayError::RouteNotAllowed);
            }
            envelope.authenticated = true;
            Ok(envelope)
        })
    }

    fn deliver<'a>(
        &'a self,
        _attempt: &'a DeliveryAttempt,
    ) -> ChannelProviderFuture<'a, ChannelDeliveryOutcome> {
        Box::pin(async {
            Ok(ChannelDeliveryOutcome {
                delivered: false,
                retryable: true,
                result_code: "loopback_pull_delivery_pending".into(),
            })
        })
    }

    fn ack<'a>(&'a self, _delivery: &'a DeliveryAttempt) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Debug, Clone)]
pub struct MockPollChannel {
    store: AgentStore,
    allowed_routes: BTreeSet<ChannelRouteKey>,
    connected: Arc<Mutex<bool>>,
    account: Arc<RwLock<Option<ChannelProviderAccount>>>,
    state: Arc<RwLock<ChannelProviderHealthState>>,
}

impl MockPollChannel {
    #[must_use]
    pub fn new(store: AgentStore, allowed_routes: BTreeSet<ChannelRouteKey>) -> Self {
        Self {
            store,
            allowed_routes,
            connected: Arc::new(Mutex::new(true)),
            account: Arc::new(RwLock::new(None)),
            state: Arc::new(RwLock::new(ChannelProviderHealthState::Disabled)),
        }
    }

    pub fn set_connected(&self, connected: bool) {
        *self.connected.lock().expect("mock poll connected lock") = connected;
    }

    #[must_use]
    pub fn connected(&self) -> bool {
        *self.connected.lock().expect("mock poll connected lock")
    }

    pub async fn push(&self, mut envelope: ChannelEnvelope) -> Result<(), GatewayError> {
        if !self.connected() {
            return Err(GatewayError::ChannelDisconnected);
        }
        validate_envelope(&envelope)?;
        if envelope.route.channel != "mock-poll" || !self.allowed_routes.contains(&envelope.route) {
            return Err(GatewayError::RouteNotAllowed);
        }
        envelope.authenticated = true;
        let now = now_ms();
        sqlx::query(
            "INSERT INTO mock_poll_inbox(message_id, envelope_json, status, received_at_ms, updated_at_ms) VALUES(?, ?, 'queued', ?, ?) ON CONFLICT(message_id) DO UPDATE SET envelope_json = excluded.envelope_json, status = 'queued', updated_at_ms = excluded.updated_at_ms",
        )
        .bind(envelope.message_id.as_str())
        .bind(serde_json::to_string(&envelope)?)
        .bind(envelope.received_at_ms)
        .bind(now)
        .execute(self.store.pool())
        .await?;
        Ok(())
    }

    pub async fn drain(&self, gateway: &GatewayHost) -> Result<Vec<IngressReceipt>, GatewayError> {
        if !self.connected() {
            return Err(GatewayError::ChannelDisconnected);
        }
        let rows = sqlx::query(
            "SELECT message_id, envelope_json FROM mock_poll_inbox WHERE status = 'queued' ORDER BY received_at_ms, message_id",
        )
        .fetch_all(self.store.pool())
        .await?;
        let mut receipts = Vec::with_capacity(rows.len());
        for row in rows {
            let message_id = row.get::<String, _>("message_id");
            let envelope: ChannelEnvelope = serde_json::from_str(row.get("envelope_json"))?;
            let receipt = gateway.ingest(&envelope, &self.allowed_routes).await?;
            sqlx::query(
                "UPDATE mock_poll_inbox SET status = 'drained', updated_at_ms = ? WHERE message_id = ? AND status = 'queued'",
            )
            .bind(now_ms())
            .bind(message_id)
            .execute(self.store.pool())
            .await?;
            receipts.push(receipt);
        }
        Ok(receipts)
    }
}

impl ChannelProvider for MockPollChannel {
    fn manifest(&self) -> ChannelProviderManifest {
        ChannelProviderManifest {
            id: "mock-poll".into(),
            plugin_id: None,
            runtime_kind: hachimi_protocol::ChannelProviderRuntimeKind::Builtin,
            entrypoint: None,
            content_hash: digest(b"hachimi.channel.mock-poll.v1"),
            required_scopes: vec!["channel.receive".into(), "channel.deliver".into()],
        }
    }

    fn configure<'a>(
        &'a self,
        account: &'a ChannelProviderAccount,
    ) -> ChannelProviderFuture<'a, ()> {
        let valid = account.provider_id == "mock-poll"
            && account.enabled
            && !account.route_allowlist.is_empty()
            && account
                .route_allowlist
                .iter()
                .all(|route| route.channel == "mock-poll");
        Box::pin(async move {
            if !valid {
                return Err(GatewayError::InvalidProvider);
            }
            *self
                .account
                .write()
                .map_err(|_| GatewayError::ProviderStatePoisoned)? = Some(account.clone());
            Ok(())
        })
    }

    fn start<'a>(&'a self) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move {
            *self
                .state
                .write()
                .map_err(|_| GatewayError::ProviderStatePoisoned)? =
                ChannelProviderHealthState::Healthy;
            Ok(())
        })
    }

    fn stop<'a>(&'a self) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move {
            *self
                .state
                .write()
                .map_err(|_| GatewayError::ProviderStatePoisoned)? =
                ChannelProviderHealthState::Disabled;
            Ok(())
        })
    }

    fn health<'a>(&'a self) -> ChannelProviderFuture<'a, ChannelProviderHealth> {
        Box::pin(async move {
            Ok(ChannelProviderHealth {
                provider_id: "mock-poll".into(),
                state: *self
                    .state
                    .read()
                    .map_err(|_| GatewayError::ProviderStatePoisoned)?,
                diagnostic: (!self.connected()).then_some("mock_poll_disconnected".into()),
                config_revision: self
                    .account
                    .read()
                    .map_err(|_| GatewayError::ProviderStatePoisoned)?
                    .as_ref()
                    .map_or(0, |account| account.config_revision),
            })
        })
    }

    fn receive<'a>(
        &'a self,
        _credential: Option<&'a str>,
        mut envelope: ChannelEnvelope,
    ) -> ChannelProviderFuture<'a, ChannelEnvelope> {
        Box::pin(async move {
            if !self.connected() {
                return Err(GatewayError::ChannelDisconnected);
            }
            if !self.allowed_routes.contains(&envelope.route) {
                return Err(GatewayError::RouteNotAllowed);
            }
            envelope.authenticated = true;
            Ok(envelope)
        })
    }

    fn deliver<'a>(
        &'a self,
        _attempt: &'a DeliveryAttempt,
    ) -> ChannelProviderFuture<'a, ChannelDeliveryOutcome> {
        Box::pin(async move {
            Ok(ChannelDeliveryOutcome {
                delivered: self.connected(),
                retryable: !self.connected(),
                result_code: if self.connected() {
                    "mock_poll_delivered"
                } else {
                    "mock_poll_disconnected"
                }
                .into(),
            })
        })
    }

    fn ack<'a>(&'a self, _delivery: &'a DeliveryAttempt) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

fn validate_envelope(envelope: &ChannelEnvelope) -> Result<(), GatewayError> {
    let route = &envelope.route;
    let valid_route = [
        &route.channel,
        &route.account,
        &route.peer,
        &route.thread,
        &envelope.sender,
    ]
    .into_iter()
    .all(|value| !value.trim().is_empty() && value.chars().count() <= 256);
    if !valid_route
        || envelope.text.trim().is_empty()
        || envelope.text.chars().count() > MAX_MESSAGE_CHARS
        || !envelope.metadata.is_object()
    {
        return Err(GatewayError::InvalidMessage);
    }
    Ok(())
}

fn route_key(route: &ChannelRouteKey) -> Result<String, serde_json::Error> {
    serde_json::to_string(route)
}

fn decode_delivery(row: sqlx::sqlite::SqliteRow) -> Result<DeliveryAttempt, GatewayError> {
    Ok(DeliveryAttempt {
        id: ChannelDeliveryId::new(row.get::<String, _>("id")),
        route: serde_json::from_str(row.get("route_key"))?,
        idempotency_key: row.get("idempotency_key"),
        text: row.get("text"),
        status: match row.get::<String, _>("status").as_str() {
            "pending" => DeliveryAttemptStatus::Pending,
            "claimed" => DeliveryAttemptStatus::Claimed,
            "delivered" => DeliveryAttemptStatus::Delivered,
            "retry_scheduled" => DeliveryAttemptStatus::RetryScheduled,
            "failed" => DeliveryAttemptStatus::Failed,
            _ => return Err(GatewayError::DeliveryConflict),
        },
        attempt: u32::try_from(row.get::<i64, _>("attempt")).unwrap_or(u32::MAX),
        next_attempt_at_ms: row.get("next_attempt_at_ms"),
        error_code: row.get("error_code"),
    })
}

#[cfg(windows)]
fn update_user_startup(executable: &Path, enabled: bool) -> Result<(), GatewayError> {
    if !executable.is_absolute() || !executable.is_file() {
        return Err(GatewayError::StartupRegistration(
            "gateway executable must be an existing absolute path".into(),
        ));
    }
    if !enabled {
        let existing = hachimi_process_policy::std_command(
            "reg.exe",
            hachimi_process_policy::ProcessPolicy::HiddenCaptured,
        )
        .args([
            "QUERY",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
            "/v",
            "HachimiGateway",
        ])
        .output()
        .map_err(|error| GatewayError::StartupRegistration(error.to_string()))?;
        if !existing.status.success() {
            return Ok(());
        }
    }
    let mut command = hachimi_process_policy::std_command(
        "reg.exe",
        hachimi_process_policy::ProcessPolicy::HiddenCaptured,
    );
    command.args([
        if enabled { "ADD" } else { "DELETE" },
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
        "/v",
        "HachimiGateway",
        "/f",
    ]);
    if enabled {
        command.args(["/t", "REG_SZ", "/d"]);
        command.arg(format!("\"{}\" --gateway", executable.display()));
    }
    let output = command
        .output()
        .map_err(|error| GatewayError::StartupRegistration(error.to_string()))?;
    if !output.status.success() {
        return Err(GatewayError::StartupRegistration(
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(512)
                .collect(),
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn update_user_startup(_executable: &Path, _enabled: bool) -> Result<(), GatewayError> {
    Err(GatewayError::StartupRegistration(
        "per-user startup registration is Windows-only".into(),
    ))
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn provider_health_state(state: ChannelProviderHealthState) -> &'static str {
    match state {
        ChannelProviderHealthState::Disabled => "disabled",
        ChannelProviderHealthState::Starting => "starting",
        ChannelProviderHealthState::Healthy => "healthy",
        ChannelProviderHealthState::NeedsAttention => "needs_attention",
        ChannelProviderHealthState::Failed => "failed",
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or_default()
}

#[allow(dead_code)]
fn retry_delay(attempt: u32) -> Duration {
    Duration::from_secs(1_u64 << attempt.saturating_sub(1).min(10))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn enterprise_feature_switch_removes_only_enterprise_providers() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let builtins =
            local_builtin_providers_with_enterprise(store, "token", false).expect("builtins");
        assert_eq!(
            builtins.registry.provider_ids(),
            vec!["loopback-webhook".to_owned(), "mock-poll".to_owned()]
        );
    }
    use serde_json::json;
    #[cfg(windows)]
    use std::path::PathBuf;

    #[cfg(windows)]
    struct UserStartupGuard(PathBuf);

    #[cfg(windows)]
    impl Drop for UserStartupGuard {
        fn drop(&mut self) {
            let _ = update_user_startup(&self.0, false);
        }
    }

    fn route(thread: &str) -> ChannelRouteKey {
        ChannelRouteKey {
            channel: "loopback-webhook".into(),
            account: "local".into(),
            peer: "user-1".into(),
            thread: thread.into(),
        }
    }

    fn envelope(id: &str, route: ChannelRouteKey) -> ChannelEnvelope {
        ChannelEnvelope {
            message_id: ChannelMessageId::new(id),
            route,
            sender: "user-1".into(),
            text: "hello".into(),
            metadata: json!({}),
            authenticated: false,
            bot_generated: false,
            received_at_ms: 1,
        }
    }

    #[derive(Debug)]
    struct TestPluginProvider {
        content_hash: String,
    }

    impl TestPluginProvider {
        fn new(content_hash: &str) -> Self {
            Self {
                content_hash: content_hash.into(),
            }
        }
    }

    impl ChannelProvider for TestPluginProvider {
        fn manifest(&self) -> ChannelProviderManifest {
            ChannelProviderManifest {
                id: "plugin.provider-test.channel".into(),
                plugin_id: Some(hachimi_protocol::PluginId::from("provider-test")),
                runtime_kind: hachimi_protocol::ChannelProviderRuntimeKind::SandboxedStdioJsonRpc,
                entrypoint: Some("channel.json".into()),
                content_hash: self.content_hash.clone(),
                required_scopes: vec!["channel.receive".into(), "channel.deliver".into()],
            }
        }

        fn configure<'a>(
            &'a self,
            _account: &'a ChannelProviderAccount,
        ) -> ChannelProviderFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }

        fn start<'a>(&'a self) -> ChannelProviderFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }

        fn stop<'a>(&'a self) -> ChannelProviderFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }

        fn health<'a>(&'a self) -> ChannelProviderFuture<'a, ChannelProviderHealth> {
            Box::pin(async {
                Ok(ChannelProviderHealth {
                    provider_id: "plugin.provider-test.channel".into(),
                    state: ChannelProviderHealthState::Healthy,
                    diagnostic: None,
                    config_revision: 1,
                })
            })
        }

        fn receive<'a>(
            &'a self,
            _credential: Option<&'a str>,
            envelope: ChannelEnvelope,
        ) -> ChannelProviderFuture<'a, ChannelEnvelope> {
            Box::pin(async move { Ok(envelope) })
        }

        fn deliver<'a>(
            &'a self,
            _attempt: &'a DeliveryAttempt,
        ) -> ChannelProviderFuture<'a, ChannelDeliveryOutcome> {
            Box::pin(async {
                Ok(ChannelDeliveryOutcome {
                    delivered: true,
                    retryable: false,
                    result_code: "test_delivered".into(),
                })
            })
        }

        fn ack<'a>(&'a self, _delivery: &'a DeliveryAttempt) -> ChannelProviderFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    #[ignore = "mutates and restores the current user's Windows Run registry key"]
    async fn windows_per_user_startup_registration_roundtrips() {
        let executable = std::env::var_os("HACHIMI_STANDARD_USER_HACHIMI_EXE")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_exe().expect("current executable"));
        assert!(executable.is_absolute() && executable.is_file());
        let _guard = UserStartupGuard(executable.clone());
        let store = AgentStore::connect_in_memory().await.expect("store");
        let gateway = GatewayHost::new(store, vec!["loopback-webhook".into()]);

        let enabled = gateway
            .set_startup_registration(&executable, true, 1)
            .await
            .expect("register per-user startup");
        assert!(enabled.startup_registered);
        let query = hachimi_process_policy::std_command(
            "reg.exe",
            hachimi_process_policy::ProcessPolicy::HiddenCaptured,
        )
        .args([
            "QUERY",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
            "/v",
            "HachimiGateway",
        ])
        .output()
        .expect("query per-user startup");
        assert!(query.status.success());
        let value = String::from_utf8_lossy(&query.stdout);
        assert!(value.contains(&executable.to_string_lossy().into_owned()));
        assert!(value.contains("--gateway"));

        let disabled = gateway
            .set_startup_registration(&executable, false, 2)
            .await
            .expect("remove per-user startup");
        assert!(!disabled.startup_registered);
        let absent = hachimi_process_policy::std_command(
            "reg.exe",
            hachimi_process_policy::ProcessPolicy::HiddenCaptured,
        )
        .args([
            "QUERY",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
            "/v",
            "HachimiGateway",
        ])
        .output()
        .expect("query removed startup");
        assert!(!absent.status.success());
    }

    #[tokio::test]
    async fn loopback_auth_dedup_and_restart_reconciliation_are_durable() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let gateway = GatewayHost::new(
            store.clone(),
            vec!["loopback-webhook".into(), "mock-poll".into()],
        );
        let allowed = BTreeSet::from([route("thread-1")]);
        let channel = LoopbackWebhookChannel::new("secret-token", allowed);
        let message = envelope("message-1", route("thread-1"));
        assert!(matches!(
            channel.receive(&gateway, "wrong", message.clone()).await,
            Err(GatewayError::Unauthenticated)
        ));
        assert_eq!(
            channel
                .receive(&gateway, "secret-token", message.clone())
                .await
                .expect("accepted")
                .status,
            IngressStatus::Accepted
        );
        assert_eq!(
            channel
                .receive(&gateway, "secret-token", message)
                .await
                .expect("duplicate")
                .status,
            IngressStatus::Duplicate
        );
        assert!(
            gateway
                .claim_next_ingress(2)
                .await
                .expect("claim")
                .is_some()
        );
        gateway.reconcile_startup(3).await.expect("reconcile");
        assert!(
            gateway
                .claim_next_ingress(4)
                .await
                .expect("reclaim")
                .is_some()
        );

        let audit = sqlx::query(
            "SELECT target_summary, result_code FROM audit_events WHERE operation = 'channel.ingress' ORDER BY id LIMIT 1",
        )
        .fetch_one(store.pool())
        .await
        .expect("channel ingress audit");
        let summary = audit.get::<String, _>("target_summary");
        assert!(summary.starts_with("channel:loopback-webhook:route_sha256:"));
        assert!(!summary.contains("user-1"));
        assert!(!summary.contains("thread-1"));
        assert!(!summary.contains("hello"));
        assert_eq!(audit.get::<String, _>("result_code"), "accepted");
    }

    #[tokio::test]
    async fn delivery_is_idempotent_and_retries_with_a_bound() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let gateway = GatewayHost::new(store, vec!["loopback-webhook".into()]);
        let first = gateway
            .enqueue_delivery(route("thread-1"), "reply-1", "done", 10)
            .await
            .expect("enqueue");
        let replay = gateway
            .enqueue_delivery(route("thread-1"), "reply-1", "done", 10)
            .await
            .expect("replay");
        assert_eq!(first.id, replay.id);
        let claimed = gateway
            .claim_next_delivery(10)
            .await
            .expect("claim")
            .expect("delivery");
        assert_eq!(claimed.attempt, 1);
        let retry = gateway
            .finish_delivery(&claimed.id, false, true, Some("offline"), 10)
            .await
            .expect("retry");
        assert_eq!(retry.status, DeliveryAttemptStatus::RetryScheduled);
        assert_eq!(retry.next_attempt_at_ms, Some(1_010));
    }

    #[tokio::test]
    async fn plugin_provider_enable_disable_and_revision_drift_fail_closed() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        sqlx::query("INSERT INTO plugin_installations(plugin_id, manifest_json, content_hash, root_path, status, diagnostics_json, installed_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, 'enabled', '[]', ?, ?)")
            .bind("provider-test")
            .bind("{}")
            .bind("plugin-content-hash")
            .bind("C:\\plugins\\provider-test")
            .bind(1_i64)
            .bind(1_i64)
            .execute(store.pool())
            .await
            .expect("install provider plugin fixture");
        let gateway = GatewayHost::new(store.clone(), Vec::<String>::new());
        gateway
            .register_provider(Arc::new(TestPluginProvider::new("revision-one")), true)
            .await
            .expect("register provider");
        let provider_route = ChannelRouteKey {
            channel: "plugin.provider-test.channel".into(),
            account: "local".into(),
            peer: "user".into(),
            thread: "main".into(),
        };
        gateway
            .upsert_provider_account(
                ChannelProviderAccount {
                    id: "provider-account".into(),
                    provider_id: "plugin.provider-test.channel".into(),
                    display_name: "Provider account".into(),
                    secret_ref: None,
                    enabled: true,
                    route_allowlist: vec![provider_route],
                    config_revision: 0,
                },
                None,
                false,
            )
            .await
            .expect("enable provider account");

        gateway
            .register_provider(Arc::new(TestPluginProvider::new("revision-two")), true)
            .await
            .expect("register changed provider");
        let drifted = sqlx::query("SELECT enabled, contribution_enabled, health, diagnostic FROM channel_provider_manifests WHERE provider_id = ?")
            .bind("plugin.provider-test.channel")
            .fetch_one(store.pool())
            .await
            .expect("drifted provider row");
        assert!(!drifted.get::<bool, _>("enabled"));
        assert!(drifted.get::<bool, _>("contribution_enabled"));
        assert_eq!(drifted.get::<String, _>("health"), "needs_attention");
        assert_eq!(
            drifted.get::<Option<String>, _>("diagnostic").as_deref(),
            Some("channel_provider_revision_drift")
        );

        let plugin_id = hachimi_protocol::PluginId::from("provider-test");
        gateway
            .set_plugin_providers_enabled(&plugin_id, false)
            .await
            .expect("disable contribution");
        let disabled = sqlx::query("SELECT enabled, contribution_enabled, health, diagnostic FROM channel_provider_manifests WHERE provider_id = ?")
            .bind("plugin.provider-test.channel")
            .fetch_one(store.pool())
            .await
            .expect("disabled provider row");
        assert!(!disabled.get::<bool, _>("enabled"));
        assert!(!disabled.get::<bool, _>("contribution_enabled"));
        assert_eq!(disabled.get::<String, _>("health"), "disabled");
        assert_eq!(disabled.get::<Option<String>, _>("diagnostic"), None);

        gateway
            .register_provider(Arc::new(TestPluginProvider::new("revision-two")), true)
            .await
            .expect("explicitly re-enable reviewed revision");
        let reviewed = sqlx::query("SELECT enabled, contribution_enabled, health, diagnostic FROM channel_provider_manifests WHERE provider_id = ?")
            .bind("plugin.provider-test.channel")
            .fetch_one(store.pool())
            .await
            .expect("reviewed provider row");
        assert!(reviewed.get::<bool, _>("enabled"));
        assert!(reviewed.get::<bool, _>("contribution_enabled"));
        assert_eq!(reviewed.get::<String, _>("health"), "healthy");
        assert_eq!(reviewed.get::<Option<String>, _>("diagnostic"), None);
    }

    #[tokio::test]
    async fn builtin_enterprise_provider_contribution_enablement_is_durable_and_fail_closed() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let builtins =
            local_builtin_providers(store.clone(), "loopback-token").expect("built-in providers");
        let gateway = GatewayHost::with_registry(store.clone(), builtins.registry);
        gateway
            .bootstrap_provider_accounts(&builtins.accounts)
            .await
            .expect("bootstrap providers");

        gateway
            .set_builtin_provider_contribution_enabled("wecom", false)
            .await
            .expect("disable contribution");
        let disabled = sqlx::query(
            "SELECT enabled, contribution_enabled, health FROM channel_provider_manifests WHERE provider_id = 'wecom'",
        )
        .fetch_one(store.pool())
        .await
        .expect("disabled provider row");
        assert!(!disabled.get::<bool, _>("enabled"));
        assert!(!disabled.get::<bool, _>("contribution_enabled"));
        assert_eq!(disabled.get::<String, _>("health"), "disabled");

        gateway
            .set_builtin_provider_contribution_enabled("wecom", true)
            .await
            .expect("enable contribution");
        let enabled = sqlx::query(
            "SELECT enabled, contribution_enabled FROM channel_provider_manifests WHERE provider_id = 'wecom'",
        )
        .fetch_one(store.pool())
        .await
        .expect("enabled provider row");
        assert!(!enabled.get::<bool, _>("enabled"));
        assert!(enabled.get::<bool, _>("contribution_enabled"));
        assert!(matches!(
            gateway
                .set_builtin_provider_contribution_enabled("loopback-webhook", true)
                .await,
            Err(GatewayError::InvalidProvider)
        ));
    }

    #[tokio::test]
    async fn route_keys_keep_threads_isolated() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let session = hachimi_protocol::SessionRecord {
            id: SessionId::from("session-1"),
            context: hachimi_protocol::SessionContextBinding::General,
            entry_profile: hachimi_protocol::EntryProfile::Workbench,
            title: "Channel".into(),
            archived: false,
            pinned: false,
            parent_session_id: None,
            source_run_id: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        store.create_session(&session).await.expect("session");
        let gateway = GatewayHost::new(store, vec!["loopback-webhook".into()]);
        gateway
            .bind_route(&route("thread-1"), &session.id, 1)
            .await
            .expect("bind");
        assert_eq!(
            gateway
                .session_for_route(&route("thread-1"))
                .await
                .expect("route"),
            Some(session.id)
        );
        assert_eq!(
            gateway
                .session_for_route(&route("thread-2"))
                .await
                .expect("route"),
            None
        );
    }

    #[tokio::test]
    async fn mock_poll_queue_survives_restart_and_replays_duplicates_safely() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let gateway = GatewayHost::new(
            store.clone(),
            vec!["loopback-webhook".into(), "mock-poll".into()],
        );
        let mock_route = ChannelRouteKey {
            channel: "mock-poll".into(),
            account: "local".into(),
            peer: "user-1".into(),
            thread: "thread-1".into(),
        };
        let allowed = BTreeSet::from([mock_route.clone()]);
        let mut message = envelope("mock-message-1", mock_route);
        message.received_at_ms = 10;

        let first_process = MockPollChannel::new(store.clone(), allowed.clone());
        first_process.push(message.clone()).await.expect("queue");
        drop(first_process);

        let restarted = MockPollChannel::new(store, allowed);
        let receipts = restarted
            .drain(&gateway)
            .await
            .expect("drain after restart");
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].status, IngressStatus::Accepted);

        restarted.push(message).await.expect("duplicate queue");
        let duplicate = restarted.drain(&gateway).await.expect("duplicate drain");
        assert_eq!(duplicate.len(), 1);
        assert_eq!(duplicate[0].status, IngressStatus::Duplicate);
    }
}
