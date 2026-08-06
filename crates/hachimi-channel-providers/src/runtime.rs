use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

use hachimi_protocol::{
    ChannelAccountState, ChannelOutboundPayload, ChannelProviderHealthState,
    IntegrationProbeDimension, IntegrationProviderId, VerifiedChannelMessage,
};
use serde_json::Value;

use crate::{ProviderAdapter, ProviderError, ProviderEventFrame};

#[derive(Debug, Clone, PartialEq)]
pub struct AccountRuntimeConfig {
    pub account_id: String,
    pub provider_id: IntegrationProviderId,
    pub tenant_key: String,
    pub credential_ref: String,
    pub config: Value,
    pub config_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeProbe {
    pub credential: IntegrationProbeDimension,
    pub ingress: IntegrationProbeDimension,
    pub egress: IntegrationProbeDimension,
    pub api: IntegrationProbeDimension,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRuntimeSnapshot {
    pub state: ChannelAccountState,
    pub health: ChannelProviderHealthState,
    pub config_revision: u64,
    pub consecutive_failures: u32,
    pub last_event_at_ms: Option<i64>,
    pub last_delivery_at_ms: Option<i64>,
    pub last_handshake_at_ms: Option<i64>,
    pub last_frame_at_ms: Option<i64>,
    pub last_error_code: Option<String>,
    pub next_reconnect_at_ms: Option<i64>,
}

#[derive(Clone)]
pub struct AccountRuntime {
    adapter: Arc<dyn ProviderAdapter>,
    config: Arc<RwLock<AccountRuntimeConfig>>,
    snapshot: Arc<RwLock<AccountRuntimeSnapshot>>,
    ingress: Arc<Mutex<VecDeque<VerifiedChannelMessage>>>,
}

impl std::fmt::Debug for AccountRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AccountRuntime")
            .finish_non_exhaustive()
    }
}

impl AccountRuntime {
    pub fn new(
        adapter: Arc<dyn ProviderAdapter>,
        config: AccountRuntimeConfig,
    ) -> Result<Self, ProviderError> {
        if adapter.provider_id() != config.provider_id || config.account_id.trim().is_empty() {
            return Err(ProviderError::InvalidEvent);
        }
        let revision = config.config_revision;
        Ok(Self {
            adapter,
            config: Arc::new(RwLock::new(config)),
            snapshot: Arc::new(RwLock::new(AccountRuntimeSnapshot {
                state: ChannelAccountState::Draft,
                health: ChannelProviderHealthState::Disabled,
                config_revision: revision,
                consecutive_failures: 0,
                last_event_at_ms: None,
                last_delivery_at_ms: None,
                last_handshake_at_ms: None,
                last_frame_at_ms: None,
                last_error_code: None,
                next_reconnect_at_ms: None,
            })),
            ingress: Arc::new(Mutex::new(VecDeque::new())),
        })
    }

    pub fn start(&self) -> Result<(), ProviderError> {
        let mut snapshot = self
            .snapshot
            .write()
            .map_err(|_| ProviderError::RuntimeNotReady)?;
        snapshot.state = ChannelAccountState::Healthy;
        snapshot.health = ChannelProviderHealthState::Healthy;
        snapshot.consecutive_failures = 0;
        snapshot.last_handshake_at_ms = Some(now_ms());
        snapshot.last_error_code = None;
        snapshot.next_reconnect_at_ms = None;
        Ok(())
    }

    pub fn stop(&self) -> Result<(), ProviderError> {
        let mut snapshot = self
            .snapshot
            .write()
            .map_err(|_| ProviderError::RuntimeNotReady)?;
        snapshot.state = ChannelAccountState::Draft;
        snapshot.health = ChannelProviderHealthState::Disabled;
        snapshot.next_reconnect_at_ms = None;
        Ok(())
    }

    pub fn reload(&self, config: AccountRuntimeConfig) -> Result<(), ProviderError> {
        let mut current = self
            .config
            .write()
            .map_err(|_| ProviderError::RuntimeNotReady)?;
        if current.account_id != config.account_id
            || current.provider_id != config.provider_id
            || config.config_revision < current.config_revision
        {
            return Err(ProviderError::InvalidEvent);
        }
        if config.config_revision == current.config_revision {
            return if *current == config {
                Ok(())
            } else {
                Err(ProviderError::InvalidEvent)
            };
        }
        let revision = config.config_revision;
        *current = config;
        self.snapshot
            .write()
            .map_err(|_| ProviderError::RuntimeNotReady)?
            .config_revision = revision;
        Ok(())
    }

    pub fn accept_frame(&self, frame: ProviderEventFrame) -> Result<(), ProviderError> {
        let message = self.adapter.normalize(frame)?;
        self.push_verified(message)
    }

    pub fn push_verified(&self, message: VerifiedChannelMessage) -> Result<(), ProviderError> {
        let config = self
            .config
            .read()
            .map_err(|_| ProviderError::RuntimeNotReady)?;
        if message.address.provider_id != config.provider_id.as_str()
            || message.address.account_id != config.account_id
            || message.address.tenant_key != config.tenant_key
        {
            return Err(ProviderError::InvalidEvent);
        }
        drop(config);
        let snapshot = self
            .snapshot
            .read()
            .map_err(|_| ProviderError::RuntimeNotReady)?;
        if snapshot.health != ChannelProviderHealthState::Healthy {
            return Err(ProviderError::RuntimeNotReady);
        }
        drop(snapshot);
        let mut snapshot = self
            .snapshot
            .write()
            .map_err(|_| ProviderError::RuntimeNotReady)?;
        snapshot.last_event_at_ms = Some(message.received_at_ms);
        snapshot.last_frame_at_ms = Some(message.received_at_ms);
        snapshot.last_error_code = None;
        snapshot.next_reconnect_at_ms = None;
        drop(snapshot);
        self.ingress
            .lock()
            .map_err(|_| ProviderError::RuntimeNotReady)?
            .push_back(message);
        Ok(())
    }

    pub fn next_event(&self) -> Result<Option<VerifiedChannelMessage>, ProviderError> {
        Ok(self
            .ingress
            .lock()
            .map_err(|_| ProviderError::RuntimeNotReady)?
            .pop_front())
    }

    pub fn config(&self) -> Result<AccountRuntimeConfig, ProviderError> {
        self.config
            .read()
            .map(|config| config.clone())
            .map_err(|_| ProviderError::RuntimeNotReady)
    }

    pub fn record_transport_failure(&self) -> Result<(), ProviderError> {
        let mut snapshot = self
            .snapshot
            .write()
            .map_err(|_| ProviderError::RuntimeNotReady)?;
        snapshot.state = ChannelAccountState::Degraded;
        snapshot.health = ChannelProviderHealthState::Degraded;
        snapshot.consecutive_failures = snapshot.consecutive_failures.saturating_add(1);
        snapshot.last_error_code = Some("provider_transport_unavailable".into());
        snapshot.next_reconnect_at_ms = Some(now_ms().saturating_add(
            if snapshot.consecutive_failures >= 3 {
                30_000
            } else {
                2_000
            },
        ));
        Ok(())
    }

    pub fn record_transport_success(&self) -> Result<(), ProviderError> {
        let mut snapshot = self
            .snapshot
            .write()
            .map_err(|_| ProviderError::RuntimeNotReady)?;
        snapshot.state = ChannelAccountState::Healthy;
        snapshot.health = ChannelProviderHealthState::Healthy;
        snapshot.consecutive_failures = 0;
        snapshot.last_handshake_at_ms = Some(now_ms());
        snapshot.last_error_code = None;
        snapshot.next_reconnect_at_ms = None;
        Ok(())
    }

    pub fn mark_authentication_expired(&self) -> Result<(), ProviderError> {
        let mut snapshot = self
            .snapshot
            .write()
            .map_err(|_| ProviderError::RuntimeNotReady)?;
        snapshot.state = ChannelAccountState::AwaitingAuth;
        snapshot.health = ChannelProviderHealthState::NeedsAttention;
        snapshot.consecutive_failures = snapshot.consecutive_failures.saturating_add(1);
        snapshot.last_error_code = Some("provider_authentication_expired".into());
        snapshot.next_reconnect_at_ms = None;
        Ok(())
    }

    pub fn mark_needs_attention(&self) -> Result<(), ProviderError> {
        let mut snapshot = self
            .snapshot
            .write()
            .map_err(|_| ProviderError::RuntimeNotReady)?;
        snapshot.state = ChannelAccountState::NeedsAttention;
        snapshot.health = ChannelProviderHealthState::NeedsAttention;
        snapshot.consecutive_failures = snapshot.consecutive_failures.saturating_add(1);
        snapshot.last_error_code =
            Some("provider_credentials_or_transport_require_attention".into());
        snapshot.next_reconnect_at_ms = None;
        Ok(())
    }

    pub fn deliver(
        &self,
        payload: &ChannelOutboundPayload,
        now_ms: i64,
    ) -> Result<(), ProviderError> {
        if payload.parts.is_empty() || payload.parts.len() > crate::MAX_MESSAGE_PARTS {
            return Err(ProviderError::MediaLimit);
        }
        let mut snapshot = self
            .snapshot
            .write()
            .map_err(|_| ProviderError::RuntimeNotReady)?;
        if snapshot.health != ChannelProviderHealthState::Healthy {
            return Err(ProviderError::RuntimeNotReady);
        }
        snapshot.last_delivery_at_ms = Some(now_ms);
        Ok(())
    }

    pub fn probe(&self) -> Result<RuntimeProbe, ProviderError> {
        let config = self
            .config
            .read()
            .map_err(|_| ProviderError::RuntimeNotReady)?;
        let snapshot = self
            .snapshot
            .read()
            .map_err(|_| ProviderError::RuntimeNotReady)?;
        let healthy = snapshot.health == ChannelProviderHealthState::Healthy;
        let dimension = |ok: bool, code: &str| IntegrationProbeDimension {
            ok,
            result_code: code.into(),
            diagnostic: None,
        };
        Ok(RuntimeProbe {
            credential: dimension(
                !config.credential_ref.is_empty(),
                "credential_reference_present",
            ),
            ingress: dimension(healthy, "ingress_runtime_state"),
            egress: dimension(healthy, "egress_runtime_state"),
            api: dimension(
                !config.provider_id.supports_enterprise_api() || healthy,
                "api_runtime_state",
            ),
        })
    }

    pub fn snapshot(&self) -> Result<AccountRuntimeSnapshot, ProviderError> {
        self.snapshot
            .read()
            .map(|snapshot| snapshot.clone())
            .map_err(|_| ProviderError::RuntimeNotReady)
    }
}

fn now_ms() -> i64 {
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
    use super::*;

    fn config(revision: u64) -> AccountRuntimeConfig {
        AccountRuntimeConfig {
            account_id: "account-1".into(),
            provider_id: IntegrationProviderId::DingTalk,
            tenant_key: "tenant-1".into(),
            credential_ref: "keyring:integration:dingtalk:account-1:primary".into(),
            config: serde_json::json!({"robotCode": "robot-1"}),
            config_revision: revision,
        }
    }

    #[test]
    fn equal_revision_reload_is_idempotent_but_detects_drift() {
        let runtime =
            AccountRuntime::new(Arc::new(crate::DingTalkAdapter), config(2)).expect("runtime");
        runtime
            .reload(config(2))
            .expect("same revision and content");
        let mut drifted = config(2);
        drifted.tenant_key = "different-tenant".into();
        assert_eq!(runtime.reload(drifted), Err(ProviderError::InvalidEvent));
        assert_eq!(runtime.reload(config(1)), Err(ProviderError::InvalidEvent));
        runtime.reload(config(3)).expect("newer revision");
    }
}
