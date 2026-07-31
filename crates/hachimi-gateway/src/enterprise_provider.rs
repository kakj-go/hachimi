use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, RwLock},
};

use hachimi_enterprise::{
    EnterpriseApiClient, EnterpriseApiError, EnterpriseCredential, EnterpriseEventAuth,
    EnterpriseMessageTarget, EnterpriseRawEvent, EnterpriseStreamRuntime, spawn_enterprise_stream,
    verify_enterprise_event,
};
use hachimi_protocol::{
    ChannelEnvelope, ChannelMessageId, ChannelProviderAccount, ChannelProviderHealth,
    ChannelProviderHealthState, ChannelProviderManifest, ChannelProviderRuntimeKind,
    DeliveryAttempt, EnterpriseAttachmentMetadata, EnterpriseMention, EnterprisePlatform,
    IngressReceipt,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::Row;
use zeroize::Zeroize;

use crate::{ChannelDeliveryOutcome, ChannelProvider, ChannelProviderFuture, GatewayError, now_ms};

#[derive(Clone)]
pub struct EnterpriseChannelProvider {
    store: hachimi_storage::AgentStore,
    platform: EnterprisePlatform,
    api: EnterpriseApiClient,
    accounts: Arc<RwLock<BTreeMap<String, ChannelProviderAccount>>>,
    running: Arc<RwLock<bool>>,
    ingress: Arc<std::sync::Mutex<VecDeque<ChannelEnvelope>>>,
    streams: Arc<std::sync::Mutex<BTreeMap<String, AccountStreamRuntime>>>,
}

#[derive(Debug)]
struct AccountStreamRuntime {
    config_revision: u64,
    stream: EnterpriseStreamRuntime,
    forwarder: tokio::task::JoinHandle<()>,
}

impl std::fmt::Debug for EnterpriseChannelProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EnterpriseChannelProvider")
            .field("platform", &self.platform)
            .field(
                "configured_accounts",
                &self.accounts.read().map(|value| value.len()).unwrap_or(0),
            )
            .finish_non_exhaustive()
    }
}

impl EnterpriseChannelProvider {
    #[must_use]
    pub fn new(store: hachimi_storage::AgentStore, platform: EnterprisePlatform) -> Self {
        Self {
            store,
            platform,
            api: EnterpriseApiClient::default(),
            accounts: Arc::new(RwLock::new(BTreeMap::new())),
            running: Arc::new(RwLock::new(false)),
            ingress: Arc::new(std::sync::Mutex::new(VecDeque::new())),
            streams: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        }
    }

    fn provider_id(&self) -> &'static str {
        self.platform.channel_provider_id()
    }

    fn configured_account(&self, account_id: &str) -> Result<ChannelProviderAccount, GatewayError> {
        self.accounts
            .read()
            .map_err(|_| GatewayError::ProviderStatePoisoned)?
            .get(account_id)
            .cloned()
            .ok_or(GatewayError::ProviderUnavailable)
    }

    async fn load_credential(
        &self,
        account: &ChannelProviderAccount,
        supplied: Option<&str>,
    ) -> Result<EnterpriseCredential, GatewayError> {
        let mut raw = if let Some(supplied) = supplied {
            supplied.to_owned()
        } else {
            channel_secret(account)?
        };
        let credential = EnterpriseCredential::parse(&raw)
            .map_err(|_| GatewayError::ProviderCredentialUnavailable)?;
        raw.zeroize();
        if credential.platform() != self.platform {
            return Err(GatewayError::ProviderCredentialUnavailable);
        }
        Ok(credential)
    }

    async fn receive_verified(
        &self,
        credential: &EnterpriseCredential,
        envelope: ChannelEnvelope,
    ) -> Result<ChannelEnvelope, GatewayError> {
        let raw = raw_event(self.platform, credential, &envelope)?;
        let verified = verify_enterprise_event(credential, raw, now_ms())
            .map_err(|_| GatewayError::Unauthenticated)?;
        let account = self.configured_account(&envelope.route.account)?;
        let integration_id = self
            .upsert_integration_account(&account, credential)
            .await?;
        let payload_hash = verified.payload_hash.clone();
        let inserted = sqlx::query("INSERT OR IGNORE INTO enterprise_event_receipts(platform, account_id, event_id, event_type, tenant_identity_hash, payload_hash, status, result_code, received_at_ms, acknowledged_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, 'accepted', 'accepted', ?, NULL, ?)")
            .bind(self.platform.as_str())
            .bind(&integration_id)
            .bind(&verified.event_id)
            .bind(&verified.event_type)
            .bind(digest_hex(verified.tenant_id.as_bytes()))
            .bind(&payload_hash)
            .bind(verified.received_at_ms)
            .bind(verified.received_at_ms)
            .execute(self.store.pool())
            .await?;
        if inserted.rows_affected() == 0 {
            let existing: String = sqlx::query_scalar("SELECT payload_hash FROM enterprise_event_receipts WHERE platform = ? AND account_id = ? AND event_id = ?")
                .bind(self.platform.as_str())
                .bind(&integration_id)
                .bind(&verified.event_id)
                .fetch_one(self.store.pool())
                .await?;
            if existing != payload_hash {
                return Err(GatewayError::InvalidMessage);
            }
        }
        for (index, mention) in verified.mentions.iter().enumerate() {
            sqlx::query("INSERT OR IGNORE INTO enterprise_event_mentions(platform, account_id, event_id, mention_index, mention_kind, target_id, display_text) VALUES(?, ?, ?, ?, ?, ?, ?)")
                .bind(self.platform.as_str())
                .bind(&integration_id)
                .bind(&verified.event_id)
                .bind(i64::try_from(index).unwrap_or(i64::MAX))
                .bind(match mention.kind {
                    hachimi_protocol::EnterpriseMentionKind::User => "user",
                    hachimi_protocol::EnterpriseMentionKind::Bot => "bot",
                    hachimi_protocol::EnterpriseMentionKind::All => "all",
                })
                .bind(&mention.target_id)
                .bind(&mention.display_text)
                .execute(self.store.pool())
                .await?;
        }
        for attachment in &verified.attachments {
            let metadata_hash =
                digest_hex(&serde_json::to_vec(attachment).map_err(GatewayError::Serialization)?);
            sqlx::query("INSERT OR IGNORE INTO enterprise_attachment_metadata(platform, account_id, event_id, remote_id, file_name, mime_type, declared_size_bytes, metadata_hash, artifact_id, created_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?, NULL, ?)")
                .bind(self.platform.as_str())
                .bind(&integration_id)
                .bind(&verified.event_id)
                .bind(&attachment.remote_id)
                .bind(&attachment.file_name)
                .bind(&attachment.mime_type)
                .bind(attachment.declared_size_bytes.and_then(|value| i64::try_from(value).ok()))
                .bind(metadata_hash)
                .bind(verified.received_at_ms)
                .execute(self.store.pool())
                .await?;
        }
        Ok(ChannelEnvelope {
            message_id: ChannelMessageId::new(verified.event_id),
            route: hachimi_protocol::ChannelRouteKey {
                channel: self.provider_id().into(),
                account: envelope.route.account,
                peer: verified.peer,
                thread: verified.thread,
            },
            sender: verified.sender,
            text: verified.text,
            metadata: json!({
                "platform": self.platform,
                "eventType": verified.event_type,
                "payloadHash": payload_hash,
                "mentions": verified.mentions,
                "attachments": verified.attachments,
                "attachmentBodiesDownloaded": false,
            }),
            authenticated: true,
            bot_generated: envelope.bot_generated,
            received_at_ms: verified.received_at_ms,
        })
    }

    async fn upsert_integration_account(
        &self,
        account: &ChannelProviderAccount,
        credential: &EnterpriseCredential,
    ) -> Result<String, GatewayError> {
        let account_id = account.id.as_str();
        let id = format!("channel:{account_id}");
        let now = now_ms();
        sqlx::query("INSERT INTO enterprise_integration_accounts(id, platform, connector_account_id, channel_account_id, tenant_identity_hash, ingress_mode, event_source_id, state, diagnostic, credential_revision, source_account_updated_at_ms, created_at_ms, updated_at_ms) VALUES(?, ?, NULL, ?, ?, ?, ?, 'healthy', NULL, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET platform = excluded.platform, channel_account_id = excluded.channel_account_id, tenant_identity_hash = excluded.tenant_identity_hash, ingress_mode = excluded.ingress_mode, event_source_id = excluded.event_source_id, state = CASE WHEN enterprise_integration_accounts.tenant_identity_hash = excluded.tenant_identity_hash THEN 'healthy' ELSE 'needs_attention' END, diagnostic = CASE WHEN enterprise_integration_accounts.tenant_identity_hash = excluded.tenant_identity_hash THEN NULL ELSE 'enterprise_tenant_identity_changed' END, credential_revision = excluded.credential_revision, source_account_updated_at_ms = excluded.source_account_updated_at_ms, updated_at_ms = excluded.updated_at_ms")
            .bind(&id)
            .bind(self.platform.as_str())
            .bind(account_id)
            .bind(digest_hex(credential.tenant_id().as_bytes()))
            .bind(ingress_mode(credential.ingress_mode()))
            .bind(format!("enterprise:{}:{account_id}", self.platform.as_str()))
            .bind(i64::try_from(account.config_revision).unwrap_or(i64::MAX))
            .bind(i64::try_from(account.config_revision).unwrap_or(i64::MAX))
            .bind(now)
            .bind(now)
            .execute(self.store.pool())
            .await?;
        Ok(id)
    }

    async fn deliver_text(
        &self,
        attempt: &DeliveryAttempt,
    ) -> Result<ChannelDeliveryOutcome, GatewayError> {
        let account = self.configured_account(&attempt.route.account)?;
        let credential = self.load_credential(&account, None).await?;
        let integration_id = self
            .upsert_integration_account(&account, &credential)
            .await?;
        let input_hash = hash_value(&json!({
            "platform": self.platform,
            "route": attempt.route,
            "text": attempt.text,
        }))?;
        match self
            .claim_operation(&integration_id, &attempt.idempotency_key, &input_hash)
            .await?
        {
            EnterpriseOperationClaim::Completed => {
                return Ok(ChannelDeliveryOutcome {
                    delivered: true,
                    retryable: false,
                    result_code: "enterprise_already_delivered".into(),
                });
            }
            EnterpriseOperationClaim::Indeterminate => {
                return Ok(ChannelDeliveryOutcome {
                    delivered: false,
                    retryable: false,
                    result_code: "enterprise_outcome_indeterminate".into(),
                });
            }
            EnterpriseOperationClaim::Execute => {}
        }
        let target = EnterpriseMessageTarget {
            peer: attempt.route.peer.clone(),
            thread: Some(attempt.route.thread.clone()),
            group: attempt.route.thread != attempt.route.peer,
        };
        match self
            .api
            .send_text(
                &attempt.route.account,
                &credential,
                &target,
                &attempt.text,
                &attempt.idempotency_key,
            )
            .await
        {
            Ok(result) => {
                sqlx::query("INSERT INTO enterprise_operation_ledger(account_id, idempotency_key, operation, input_hash, status, provider_request_id, provider_result_id, result_json, error_code, created_at_ms, updated_at_ms) VALUES(?, ?, 'message_send', ?, 'completed', NULL, ?, ?, NULL, ?, ?) ON CONFLICT(account_id, idempotency_key) DO UPDATE SET status = 'completed', provider_result_id = excluded.provider_result_id, result_json = excluded.result_json, error_code = NULL, updated_at_ms = excluded.updated_at_ms")
                    .bind(&integration_id)
                    .bind(&attempt.idempotency_key)
                    .bind(&input_hash)
                    .bind(provider_result_id(&result))
                    .bind(serde_json::to_string(&result)?)
                    .bind(now_ms())
                    .bind(now_ms())
                    .execute(self.store.pool())
                    .await?;
                Ok(ChannelDeliveryOutcome {
                    delivered: true,
                    retryable: false,
                    result_code: "enterprise_delivered".into(),
                })
            }
            Err(error) => {
                let indeterminate = matches!(error, EnterpriseApiError::Indeterminate);
                let retryable = error.retryable() && !indeterminate;
                sqlx::query("INSERT INTO enterprise_operation_ledger(account_id, idempotency_key, operation, input_hash, status, provider_request_id, provider_result_id, result_json, error_code, created_at_ms, updated_at_ms) VALUES(?, ?, 'message_send', ?, ?, NULL, NULL, NULL, ?, ?, ?) ON CONFLICT(account_id, idempotency_key) DO UPDATE SET status = excluded.status, error_code = excluded.error_code, updated_at_ms = excluded.updated_at_ms")
                    .bind(&integration_id)
                    .bind(&attempt.idempotency_key)
                    .bind(&input_hash)
                    .bind(if indeterminate { "indeterminate" } else { "failed" })
                    .bind(error.code())
                    .bind(now_ms())
                    .bind(now_ms())
                    .execute(self.store.pool())
                    .await?;
                Ok(ChannelDeliveryOutcome {
                    delivered: false,
                    retryable,
                    result_code: error.code().into(),
                })
            }
        }
    }

    async fn start_streams(&self) -> Result<(), GatewayError> {
        if !matches!(
            self.platform,
            EnterprisePlatform::DingTalk | EnterprisePlatform::Feishu
        ) {
            return Ok(());
        }
        let accounts = self
            .accounts
            .read()
            .map_err(|_| GatewayError::ProviderStatePoisoned)?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for account in accounts {
            let unchanged = self
                .streams
                .lock()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?
                .get(&account.id)
                .is_some_and(|runtime| runtime.config_revision == account.config_revision);
            if unchanged {
                continue;
            }
            if let Some(previous) = self
                .streams
                .lock()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?
                .remove(&account.id)
            {
                previous.forwarder.abort();
                tokio::spawn(previous.stream.stop());
            }
            let credential = self.load_credential(&account, None).await?;
            let (stream, mut receiver) = spawn_enterprise_stream(self.api.clone(), credential);
            let queue = Arc::clone(&self.ingress);
            let platform = self.platform;
            let account_id = account.id.clone();
            let forwarder = tokio::spawn(async move {
                while let Some(event) = receiver.recv().await {
                    let envelope = ChannelEnvelope {
                        message_id: ChannelMessageId::new(event.event_id.clone()),
                        route: hachimi_protocol::ChannelRouteKey {
                            channel: platform.channel_provider_id().into(),
                            account: account_id.clone(),
                            peer: event.peer,
                            thread: event.thread,
                        },
                        sender: event.sender,
                        text: event.text,
                        metadata: json!({
                            "eventId": event.event_id,
                            "eventType": event.event_type,
                            "timestampMs": event.timestamp_ms,
                            "connectionId": event.connection_id,
                            "transportAuthenticated": true,
                            "verificationToken": event.verification_token,
                            "mentions": event.mentions,
                            "attachments": event.attachments,
                            "payload": event.payload,
                        }),
                        authenticated: false,
                        bot_generated: false,
                        received_at_ms: event.timestamp_ms,
                    };
                    let Ok(mut queue) = queue.lock() else { break };
                    if queue.len() >= 1_024 {
                        queue.pop_front();
                    }
                    queue.push_back(envelope);
                }
            });
            self.streams
                .lock()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?
                .insert(
                    account.id,
                    AccountStreamRuntime {
                        config_revision: account.config_revision,
                        stream,
                        forwarder,
                    },
                );
        }
        Ok(())
    }

    async fn claim_operation(
        &self,
        integration_id: &str,
        idempotency_key: &str,
        input_hash: &str,
    ) -> Result<EnterpriseOperationClaim, GatewayError> {
        let now = now_ms();
        let inserted = sqlx::query("INSERT OR IGNORE INTO enterprise_operation_ledger(account_id, idempotency_key, operation, input_hash, status, provider_request_id, provider_result_id, result_json, error_code, created_at_ms, updated_at_ms) VALUES(?, ?, 'message_send', ?, 'claimed', NULL, NULL, NULL, NULL, ?, ?)")
            .bind(integration_id)
            .bind(idempotency_key)
            .bind(input_hash)
            .bind(now)
            .bind(now)
            .execute(self.store.pool())
            .await?;
        if inserted.rows_affected() == 1 {
            return Ok(EnterpriseOperationClaim::Execute);
        }
        let row = sqlx::query("SELECT input_hash, status FROM enterprise_operation_ledger WHERE account_id = ? AND idempotency_key = ?")
            .bind(integration_id)
            .bind(idempotency_key)
            .fetch_one(self.store.pool())
            .await?;
        if row.get::<String, _>("input_hash") != input_hash {
            return Err(GatewayError::IdempotencyConflict);
        }
        match row.get::<String, _>("status").as_str() {
            "completed" => Ok(EnterpriseOperationClaim::Completed),
            "failed" => {
                let updated = sqlx::query("UPDATE enterprise_operation_ledger SET status = 'claimed', error_code = NULL, updated_at_ms = ? WHERE account_id = ? AND idempotency_key = ? AND status = 'failed'")
                    .bind(now)
                    .bind(integration_id)
                    .bind(idempotency_key)
                    .execute(self.store.pool())
                    .await?;
                if updated.rows_affected() == 1 {
                    Ok(EnterpriseOperationClaim::Execute)
                } else {
                    Ok(EnterpriseOperationClaim::Indeterminate)
                }
            }
            "claimed" | "indeterminate" => Ok(EnterpriseOperationClaim::Indeterminate),
            _ => Err(GatewayError::InvalidMessage),
        }
    }
}

enum EnterpriseOperationClaim {
    Execute,
    Completed,
    Indeterminate,
}

impl ChannelProvider for EnterpriseChannelProvider {
    fn manifest(&self) -> ChannelProviderManifest {
        ChannelProviderManifest {
            id: self.provider_id().into(),
            plugin_id: None,
            runtime_kind: ChannelProviderRuntimeKind::Builtin,
            entrypoint: None,
            content_hash: digest_hex(
                format!("enterprise-channel-v1:{}", self.provider_id()).as_bytes(),
            ),
            required_scopes: vec![
                "channel.receive".into(),
                "channel.send".into(),
                "contacts.read".into(),
            ],
        }
    }

    fn configure<'a>(
        &'a self,
        account: &'a ChannelProviderAccount,
    ) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move {
            if account.provider_id != self.provider_id()
                || account.id.is_empty()
                || account.secret_ref.is_none()
                || account.route_allowlist.is_empty()
                || account
                    .route_allowlist
                    .iter()
                    .any(|route| route.channel != self.provider_id() || route.account != account.id)
            {
                return Err(GatewayError::InvalidProvider);
            }
            let credential = self.load_credential(account, None).await?;
            self.upsert_integration_account(account, &credential)
                .await?;
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

    fn start_ingress<'a>(&'a self) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move { self.start_streams().await })
    }

    fn stop<'a>(&'a self) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move {
            *self
                .running
                .write()
                .map_err(|_| GatewayError::ProviderStatePoisoned)? = false;
            let streams = {
                let mut streams = self
                    .streams
                    .lock()
                    .map_err(|_| GatewayError::ProviderStatePoisoned)?;
                std::mem::take(&mut *streams)
                    .into_values()
                    .collect::<Vec<_>>()
            };
            for runtime in streams {
                runtime.forwarder.abort();
                runtime.stream.stop().await;
            }
            Ok(())
        })
    }

    fn health<'a>(&'a self) -> ChannelProviderFuture<'a, ChannelProviderHealth> {
        Box::pin(async move {
            let running = *self
                .running
                .read()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?;
            let accounts = self
                .accounts
                .read()
                .map_err(|_| GatewayError::ProviderStatePoisoned)?;
            let revision = accounts
                .values()
                .map(|account| account.config_revision)
                .max()
                .unwrap_or_default();
            Ok(ChannelProviderHealth {
                provider_id: self.provider_id().into(),
                state: if running && !accounts.is_empty() {
                    ChannelProviderHealthState::Healthy
                } else {
                    ChannelProviderHealthState::Disabled
                },
                diagnostic: None,
                config_revision: revision,
            })
        })
    }

    fn receive<'a>(
        &'a self,
        credential: Option<&'a str>,
        envelope: ChannelEnvelope,
    ) -> ChannelProviderFuture<'a, ChannelEnvelope> {
        Box::pin(async move {
            let account = self.configured_account(&envelope.route.account)?;
            let credential = self.load_credential(&account, credential).await?;
            self.receive_verified(&credential, envelope).await
        })
    }

    fn claim_ingress<'a>(&'a self) -> ChannelProviderFuture<'a, Option<ChannelEnvelope>> {
        Box::pin(async move {
            self.ingress
                .lock()
                .map_err(|_| GatewayError::ProviderStatePoisoned)
                .map(|mut queue| queue.pop_front())
        })
    }

    fn deliver<'a>(
        &'a self,
        attempt: &'a DeliveryAttempt,
    ) -> ChannelProviderFuture<'a, ChannelDeliveryOutcome> {
        Box::pin(async move { self.deliver_text(attempt).await })
    }

    fn ack<'a>(&'a self, _delivery: &'a DeliveryAttempt) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn ack_ingress<'a>(
        &'a self,
        envelope: &'a ChannelEnvelope,
        receipt: &'a IngressReceipt,
    ) -> ChannelProviderFuture<'a, ()> {
        Box::pin(async move {
            let integration_id = format!("channel:{}", envelope.route.account);
            sqlx::query("UPDATE enterprise_event_receipts SET status = 'acknowledged', result_code = ?, acknowledged_at_ms = ?, updated_at_ms = ? WHERE platform = ? AND account_id = ? AND event_id = ? AND status IN ('accepted', 'duplicate')")
                .bind(&receipt.result_code)
                .bind(now_ms())
                .bind(now_ms())
                .bind(self.platform.as_str())
                .bind(integration_id)
                .bind(envelope.message_id.as_str())
                .execute(self.store.pool())
                .await?;
            Ok(())
        })
    }
}

fn raw_event(
    platform: EnterprisePlatform,
    credential: &EnterpriseCredential,
    envelope: &ChannelEnvelope,
) -> Result<EnterpriseRawEvent, GatewayError> {
    let metadata = envelope
        .metadata
        .as_object()
        .ok_or(GatewayError::InvalidMessage)?;
    let tenant_id = metadata
        .get("tenantId")
        .and_then(Value::as_str)
        .unwrap_or_else(|| credential.tenant_id())
        .to_owned();
    let auth = match platform {
        EnterprisePlatform::Wecom => EnterpriseEventAuth::WecomCallback {
            timestamp: metadata_string(metadata, "timestamp")?,
            nonce: metadata_string(metadata, "nonce")?,
            signature: metadata_string(metadata, "signature")?,
            encrypted: metadata_string(metadata, "encrypted")?,
        },
        EnterprisePlatform::DingTalk => EnterpriseEventAuth::Stream {
            timestamp_ms: metadata_i64(metadata, "timestampMs")?,
            connection_id: metadata_string(metadata, "connectionId")?,
            transport_authenticated: metadata
                .get("transportAuthenticated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
        EnterprisePlatform::Feishu => EnterpriseEventAuth::LongConnection {
            timestamp_ms: metadata_i64(metadata, "timestampMs")?,
            connection_id: metadata_string(metadata, "connectionId")?,
            transport_authenticated: metadata
                .get("transportAuthenticated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            verification_token: metadata
                .get("verificationToken")
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
    };
    let attachments = metadata
        .get("attachments")
        .cloned()
        .map(serde_json::from_value::<Vec<EnterpriseAttachmentMetadata>>)
        .transpose()?
        .unwrap_or_default();
    let mentions = metadata
        .get("mentions")
        .cloned()
        .map(serde_json::from_value::<Vec<EnterpriseMention>>)
        .transpose()?
        .unwrap_or_default();
    Ok(EnterpriseRawEvent {
        platform,
        account_id: envelope.route.account.clone(),
        tenant_id,
        event_id: metadata
            .get("eventId")
            .and_then(Value::as_str)
            .map(str::to_owned),
        event_type: metadata
            .get("eventType")
            .and_then(Value::as_str)
            .map(str::to_owned),
        peer: Some(envelope.route.peer.clone()),
        thread: Some(envelope.route.thread.clone()),
        sender: Some(envelope.sender.clone()),
        text: Some(envelope.text.clone()),
        mentions,
        attachments,
        payload: metadata.get("payload").cloned().unwrap_or(Value::Null),
        auth,
    })
}

fn metadata_string(
    metadata: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<String, GatewayError> {
    metadata
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 1_048_576)
        .map(str::to_owned)
        .ok_or(GatewayError::InvalidMessage)
}

fn metadata_i64(metadata: &serde_json::Map<String, Value>, key: &str) -> Result<i64, GatewayError> {
    metadata
        .get(key)
        .and_then(Value::as_i64)
        .ok_or(GatewayError::InvalidMessage)
}

fn channel_secret(account: &ChannelProviderAccount) -> Result<String, GatewayError> {
    let expected = format!("keyring:channel:{}:{}", account.provider_id, account.id);
    if account.secret_ref.as_deref() != Some(&expected) {
        return Err(GatewayError::ProviderCredentialUnavailable);
    }
    keyring::Entry::new(
        "com.hachimi.channel",
        &format!("{}:{}", account.provider_id, account.id),
    )
    .map_err(|_| GatewayError::ProviderCredentialUnavailable)?
    .get_password()
    .map_err(|_| GatewayError::ProviderCredentialUnavailable)
}

fn provider_result_id(value: &Value) -> Option<&str> {
    value
        .pointer("/data/message_id")
        .or_else(|| value.get("msgid"))
        .or_else(|| value.get("processQueryKey"))
        .and_then(Value::as_str)
}

fn ingress_mode(mode: hachimi_protocol::EnterpriseIngressMode) -> &'static str {
    match mode {
        hachimi_protocol::EnterpriseIngressMode::EncryptedCallback => "encrypted_callback",
        hachimi_protocol::EnterpriseIngressMode::Stream => "stream",
        hachimi_protocol::EnterpriseIngressMode::LongConnection => "long_connection",
    }
}

fn digest_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hash_value(value: &Value) -> Result<String, GatewayError> {
    Ok(digest_hex(&serde_json::to_vec(value)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn enterprise_manifests_are_builtin_and_platform_scoped() {
        let store = hachimi_storage::AgentStore::connect_in_memory()
            .await
            .expect("store");
        let provider = EnterpriseChannelProvider::new(store, EnterprisePlatform::Feishu);
        let manifest = provider.manifest();
        assert_eq!(manifest.id, "feishu");
        assert_eq!(manifest.runtime_kind, ChannelProviderRuntimeKind::Builtin);
        assert!(manifest.plugin_id.is_none());
    }
}
