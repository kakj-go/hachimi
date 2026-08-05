use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex, RwLock},
};

use hachimi_protocol::{
    ChannelAccountState, ChannelProviderAccount, ChannelProviderHealth, ChannelProviderHealthState,
    ChannelProviderManifest, ChannelProviderRuntimeKind, DeliveryAttempt, VerifiedChannelMessage,
};
use serde_json::json;

use crate::{
    ChannelDeliveryOutcome, ChannelProvider, ChannelProviderFuture, ChannelProviderRegistry,
    GatewayError,
};

#[derive(Debug, Clone)]
pub struct LocalBuiltinProviders {
    pub registry: ChannelProviderRegistry,
    pub accounts: Vec<ChannelProviderAccount>,
    pub loopback: LoopbackWebhookChannel,
    pub mock_poll: MockPollChannel,
}

pub(crate) fn local_builtin_providers(
    _store: hachimi_storage::AgentStore,
    loopback_token: &str,
) -> Result<LocalBuiltinProviders, GatewayError> {
    let loopback = LoopbackWebhookChannel::new(loopback_token);
    let mock_poll = MockPollChannel::default();
    let registry = ChannelProviderRegistry::default();
    registry.register(Arc::new(loopback.clone()))?;
    registry.register(Arc::new(mock_poll.clone()))?;
    Ok(LocalBuiltinProviders {
        registry,
        accounts: vec![
            local_account("loopback-local", "loopback-webhook", true),
            local_account("mock-local", "mock-poll", false),
        ],
        loopback,
        mock_poll,
    })
}

fn local_account(id: &str, provider_id: &str, credential: bool) -> ChannelProviderAccount {
    ChannelProviderAccount {
        id: id.into(),
        provider_id: provider_id.into(),
        display_name: id.into(),
        tenant_key: "local".into(),
        credential_ref: credential.then(|| format!("keyring:channel:{provider_id}:{id}:primary")),
        enabled: true,
        state: ChannelAccountState::Starting,
        config: json!({}),
        credential_revision: 1,
        config_revision: 1,
    }
}

#[derive(Debug, Clone)]
pub struct LoopbackWebhookChannel {
    token_hash: String,
    accounts: Arc<RwLock<BTreeMap<String, ChannelProviderAccount>>>,
    running: Arc<RwLock<bool>>,
}

impl LoopbackWebhookChannel {
    #[must_use]
    pub fn new(token: &str) -> Self {
        Self {
            token_hash: digest_hex(token.as_bytes()),
            accounts: Arc::new(RwLock::new(BTreeMap::new())),
            running: Arc::new(RwLock::new(false)),
        }
    }

    pub async fn receive(
        &self,
        gateway: &crate::GatewayHost,
        bearer_token: &str,
        message: VerifiedChannelMessage,
    ) -> Result<hachimi_protocol::IngressReceipt, GatewayError> {
        gateway
            .ingest_provider("loopback-webhook", Some(bearer_token), message)
            .await
    }
}

impl ChannelProvider for LoopbackWebhookChannel {
    fn manifest(&self) -> ChannelProviderManifest {
        builtin_manifest("loopback-webhook")
    }

    fn configure<'a>(
        &'a self,
        account: &'a ChannelProviderAccount,
    ) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move {
            validate_provider_account(account, "loopback-webhook")?;
            self.accounts
                .write()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?
                .insert(account.id.clone(), account.clone());
            Ok(())
        })
    }

    fn start<'a>(&'a self) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move {
            *self
                .running
                .write()
                .map_err(|_| GatewayError::ProviderStatePoisoned)? = true;
            Ok(())
        })
    }

    fn stop<'a>(&'a self) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move {
            *self
                .running
                .write()
                .map_err(|_| GatewayError::ProviderStatePoisoned)? = false;
            Ok(())
        })
    }

    fn health<'a>(&'a self) -> ChannelProviderFuture<'a, ChannelProviderHealth> {
        Box::pin(async move {
            let running = *self
                .running
                .read()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?;
            Ok(health("loopback-webhook", running))
        })
    }

    fn accept_verified<'a>(
        &'a self,
        credential: Option<&'a str>,
        message: VerifiedChannelMessage,
    ) -> ChannelProviderFuture<'a, VerifiedChannelMessage> {
        Box::pin(async move {
            if credential
                .map(|value| digest_hex(value.as_bytes()))
                .as_deref()
                != Some(self.token_hash.as_str())
                || message.address.provider_id != "loopback-webhook"
                || message.event_key.provider_id != "loopback-webhook"
            {
                return Err(GatewayError::ProviderCredentialUnavailable);
            }
            Ok(message)
        })
    }

    fn deliver<'a>(
        &'a self,
        _attempt: &'a DeliveryAttempt,
    ) -> ChannelProviderFuture<'a, ChannelDeliveryOutcome> {
        Box::pin(async {
            Ok(ChannelDeliveryOutcome {
                delivered: true,
                retryable: false,
                indeterminate: false,
                result_code: "loopback_delivered".into(),
                provider_receipt: None,
            })
        })
    }

    fn ack_delivery<'a>(&'a self, _delivery: &'a DeliveryAttempt) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Debug, Clone, Default)]
pub struct MockPollChannel {
    account: Arc<RwLock<Option<ChannelProviderAccount>>>,
    connected: Arc<RwLock<bool>>,
    ingress: Arc<Mutex<VecDeque<VerifiedChannelMessage>>>,
    deliveries: Arc<Mutex<Vec<DeliveryAttempt>>>,
}

impl MockPollChannel {
    pub fn set_connected(&self, connected: bool) -> Result<(), GatewayError> {
        *self
            .connected
            .write()
            .map_err(|_| GatewayError::ProviderStatePoisoned)? = connected;
        Ok(())
    }

    pub async fn push(&self, message: VerifiedChannelMessage) -> Result<(), GatewayError> {
        if !*self
            .connected
            .read()
            .map_err(|_| GatewayError::ProviderStatePoisoned)?
        {
            return Err(GatewayError::ChannelDisconnected);
        }
        self.ingress
            .lock()
            .map_err(|_| GatewayError::ProviderStatePoisoned)?
            .push_back(message);
        Ok(())
    }

    pub async fn drain_deliveries(&self) -> Result<Vec<DeliveryAttempt>, GatewayError> {
        Ok(std::mem::take(
            &mut *self
                .deliveries
                .lock()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?,
        ))
    }

    pub async fn drain(
        &self,
        gateway: &crate::GatewayHost,
    ) -> Result<Vec<hachimi_protocol::IngressReceipt>, GatewayError> {
        let mut receipts = Vec::new();
        while let Some(message) = <Self as ChannelProvider>::claim_ingress(self).await? {
            receipts.push(gateway.ingest_provider("mock-poll", None, message).await?);
        }
        Ok(receipts)
    }
}

impl ChannelProvider for MockPollChannel {
    fn manifest(&self) -> ChannelProviderManifest {
        builtin_manifest("mock-poll")
    }

    fn configure<'a>(
        &'a self,
        account: &'a ChannelProviderAccount,
    ) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move {
            validate_provider_account(account, "mock-poll")?;
            *self
                .account
                .write()
                .map_err(|_| GatewayError::ProviderStatePoisoned)? = Some(account.clone());
            Ok(())
        })
    }

    fn start<'a>(&'a self) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move { self.set_connected(true) })
    }

    fn stop<'a>(&'a self) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move { self.set_connected(false) })
    }

    fn health<'a>(&'a self) -> ChannelProviderFuture<'a, ChannelProviderHealth> {
        Box::pin(async move {
            let connected = *self
                .connected
                .read()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?;
            Ok(health("mock-poll", connected))
        })
    }

    fn accept_verified<'a>(
        &'a self,
        _credential: Option<&'a str>,
        message: VerifiedChannelMessage,
    ) -> ChannelProviderFuture<'a, VerifiedChannelMessage> {
        Box::pin(async move {
            if message.address.provider_id != "mock-poll" {
                return Err(GatewayError::InvalidMessage);
            }
            Ok(message)
        })
    }

    fn claim_ingress<'a>(&'a self) -> ChannelProviderFuture<'a, Option<VerifiedChannelMessage>> {
        Box::pin(async move {
            Ok(self
                .ingress
                .lock()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?
                .pop_front())
        })
    }

    fn deliver<'a>(
        &'a self,
        attempt: &'a DeliveryAttempt,
    ) -> ChannelProviderFuture<'a, ChannelDeliveryOutcome> {
        Box::pin(async move {
            if !*self
                .connected
                .read()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?
            {
                return Ok(ChannelDeliveryOutcome {
                    delivered: false,
                    retryable: true,
                    indeterminate: false,
                    result_code: "mock_disconnected".into(),
                    provider_receipt: None,
                });
            }
            self.deliveries
                .lock()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?
                .push(attempt.clone());
            Ok(ChannelDeliveryOutcome {
                delivered: true,
                retryable: false,
                indeterminate: false,
                result_code: "mock_delivered".into(),
                provider_receipt: Some(format!("mock:{}", attempt.id)),
            })
        })
    }

    fn ack_delivery<'a>(&'a self, _delivery: &'a DeliveryAttempt) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

fn builtin_manifest(id: &str) -> ChannelProviderManifest {
    ChannelProviderManifest {
        id: id.into(),
        plugin_id: None,
        runtime_kind: ChannelProviderRuntimeKind::Builtin,
        entrypoint: None,
        content_hash: format!("builtin:{id}:v2"),
        required_scopes: vec!["channels.receive".into(), "channels.deliver".into()],
    }
}

fn validate_provider_account(
    account: &ChannelProviderAccount,
    provider_id: &str,
) -> Result<(), GatewayError> {
    if account.provider_id != provider_id || account.tenant_key.trim().is_empty() {
        return Err(GatewayError::InvalidProvider);
    }
    Ok(())
}

fn health(provider_id: &str, running: bool) -> ChannelProviderHealth {
    ChannelProviderHealth {
        provider_id: provider_id.into(),
        account_id: None,
        state: if running {
            ChannelProviderHealthState::Healthy
        } else {
            ChannelProviderHealthState::Disabled
        },
        diagnostic: None,
        last_event_at_ms: None,
        last_delivery_at_ms: None,
        last_handshake_at_ms: None,
        last_frame_at_ms: None,
        last_error_code: None,
        next_reconnect_at_ms: None,
        consecutive_failures: 0,
        config_revision: 1,
    }
}

fn digest_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
