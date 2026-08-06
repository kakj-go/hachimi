use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{Arc, RwLock},
};

use hachimi_protocol::{
    ChannelProviderAccount, ChannelProviderHealth, ChannelProviderManifest, DeliveryAttempt,
    IngressReceipt, VerifiedChannelMessage,
};

use crate::GatewayError;

pub type ChannelProviderFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, GatewayError>> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelDeliveryOutcome {
    pub delivered: bool,
    pub retryable: bool,
    pub indeterminate: bool,
    pub result_code: String,
    pub provider_receipt: Option<String>,
}

pub trait ChannelProvider: Send + Sync {
    fn manifest(&self) -> ChannelProviderManifest;

    fn push_delivery(&self) -> bool {
        true
    }

    fn configure<'a>(
        &'a self,
        account: &'a ChannelProviderAccount,
    ) -> ChannelProviderFuture<'a, ()>;

    fn start<'a>(&'a self) -> ChannelProviderFuture<'a, ()>;

    fn start_ingress<'a>(&'a self) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn stop<'a>(&'a self) -> ChannelProviderFuture<'a, ()>;

    fn health<'a>(&'a self) -> ChannelProviderFuture<'a, ChannelProviderHealth>;

    fn account_health<'a>(&'a self) -> ChannelProviderFuture<'a, Vec<ChannelProviderHealth>> {
        Box::pin(async move { Ok(vec![self.health().await?]) })
    }

    /// Transport adapters return this type only after authentication and replay checks.
    fn accept_verified<'a>(
        &'a self,
        credential: Option<&'a str>,
        message: VerifiedChannelMessage,
    ) -> ChannelProviderFuture<'a, VerifiedChannelMessage>;

    fn claim_ingress<'a>(&'a self) -> ChannelProviderFuture<'a, Option<VerifiedChannelMessage>> {
        Box::pin(async { Ok(None) })
    }

    fn deliver<'a>(
        &'a self,
        attempt: &'a DeliveryAttempt,
    ) -> ChannelProviderFuture<'a, ChannelDeliveryOutcome>;

    fn ack_delivery<'a>(&'a self, delivery: &'a DeliveryAttempt) -> ChannelProviderFuture<'a, ()>;

    fn ack_ingress<'a>(
        &'a self,
        _message: &'a VerifiedChannelMessage,
        _receipt: &'a IngressReceipt,
    ) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn reload<'a>(&'a self, account: &'a ChannelProviderAccount) -> ChannelProviderFuture<'a, ()> {
        self.configure(account)
    }

    fn remove_account<'a>(&'a self, _account_id: &'a str) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Clone, Default)]
pub struct ChannelProviderRegistry {
    providers: Arc<RwLock<BTreeMap<String, Arc<dyn ChannelProvider>>>>,
}

impl std::fmt::Debug for ChannelProviderRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChannelProviderRegistry")
            .field("provider_ids", &self.provider_ids())
            .finish()
    }
}

impl ChannelProviderRegistry {
    pub fn register(&self, provider: Arc<dyn ChannelProvider>) -> Result<(), GatewayError> {
        let manifest = provider.manifest();
        if manifest.id.trim().is_empty() || manifest.id.len() > 128 {
            return Err(GatewayError::InvalidProvider);
        }
        self.providers
            .write()
            .map_err(|_| GatewayError::ProviderStatePoisoned)?
            .insert(manifest.id, provider);
        Ok(())
    }

    pub fn resolve(&self, provider_id: &str) -> Option<Arc<dyn ChannelProvider>> {
        self.providers.read().ok()?.get(provider_id).cloned()
    }

    #[must_use]
    pub fn provider_ids(&self) -> Vec<String> {
        self.providers
            .read()
            .map(|providers| providers.keys().cloned().collect())
            .unwrap_or_default()
    }

    #[must_use]
    pub fn manifests(&self) -> Vec<ChannelProviderManifest> {
        self.providers
            .read()
            .map(|providers| {
                providers
                    .values()
                    .map(|provider| provider.manifest())
                    .collect()
            })
            .unwrap_or_default()
    }
}
