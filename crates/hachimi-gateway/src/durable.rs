use std::collections::BTreeSet;

use hachimi_protocol::{
    ChannelChatKind, ChannelConversationAddress, ChannelDeliveryId, ChannelEventKey, ChannelGrant,
    ChannelMessagePart, ChannelOutboundPayload, DeliveryAttempt, DeliveryAttemptStatus,
    IngressReceipt, IngressStatus, RemoteMediaDescriptor, RunId, SessionId, VerifiedChannelMessage,
};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::{
    CLAIM_TTL_MS, ChannelControlCommand, ChannelDeliveryOutcome, GatewayError, GatewayHost,
    MAX_DELIVERY_ATTEMPTS, MAX_MESSAGE_CHARS, parse_control_command,
};

const MAX_MEDIA_PART_BYTES: u64 = 25 * 1024 * 1024;
const MAX_MEDIA_MESSAGE_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Default)]
struct OutboxProvenance {
    authorization_id: Option<String>,
    authorization_revision: Option<u64>,
    account_config_revision: Option<u64>,
    reactive_external_message_id: Option<String>,
    run_id: Option<String>,
    final_item_id: Option<String>,
    part_index: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct ReactiveDeliverySource<'a> {
    pub event_key: &'a ChannelEventKey,
    pub run_id: Option<&'a RunId>,
    pub final_item_id: &'a str,
}

impl GatewayHost {
    pub async fn ingest_provider(
        &self,
        provider_id: &str,
        credential: Option<&str>,
        message: VerifiedChannelMessage,
    ) -> Result<IngressReceipt, GatewayError> {
        let provider = self
            .providers
            .resolve(provider_id)
            .ok_or(GatewayError::ProviderUnavailable)?;
        let message = provider.accept_verified(credential, message).await?;
        if message.event_key.provider_id != provider_id
            || message.address.provider_id != provider_id
        {
            return Err(GatewayError::InvalidMessage);
        }
        let receipt = self.ingest_verified(&message).await?;
        // Platform ACK follows durable acceptance and never waits for Agent execution.
        provider.ack_ingress(&message, &receipt).await?;
        Ok(receipt)
    }

    pub async fn ingest_verified(
        &self,
        message: &VerifiedChannelMessage,
    ) -> Result<IngressReceipt, GatewayError> {
        validate_message(message)?;
        if !self
            .channels
            .read()
            .map_err(|_| GatewayError::ProviderStatePoisoned)?
            .contains(&message.address.provider_id)
        {
            return Err(GatewayError::ProviderUnavailable);
        }
        let serialized = serde_json::to_vec(message)?;
        let payload_hash = digest_hex(&serialized);
        let key = &message.event_key;
        // Deduplicate before command side effects. A platform may retry the
        // same callback after the ACK boundary, and /connect must not consume
        // its one-time code twice.
        if let Some(row) = sqlx::query("SELECT payload_hash, status, session_id, run_id, result_code FROM channel_ingress WHERE provider_id = ? AND account_id = ? AND external_message_id = ?")
            .bind(&key.provider_id)
            .bind(&key.account_id)
            .bind(&key.external_message_id)
            .fetch_optional(self.store.pool())
            .await?
        {
            if row.get::<&str, _>("payload_hash") != payload_hash {
                let target_summary = serde_json::json!({
                    "providerId": key.provider_id,
                    "accountId": key.account_id,
                    "externalMessageIdHash": digest_hex(key.external_message_id.as_bytes()),
                    "storedPayloadHash": row.get::<&str, _>("payload_hash"),
                    "receivedPayloadHash": payload_hash,
                });
                sqlx::query("INSERT INTO audit_events(principal, session_id, run_id, run_generation, operation, target_summary, decision, result_code, created_at_ms) VALUES('channel_gateway', NULL, NULL, NULL, 'channel.ingress_payload_conflict', ?, 'blocked', 'payload_hash_changed', ?)")
                    .bind(target_summary.to_string())
                    .bind(message.received_at_ms)
                    .execute(self.store.pool())
                    .await?;
                return Err(GatewayError::PayloadConflict);
            }
            return Ok(IngressReceipt {
                event_key: key.clone(),
                status: IngressStatus::Duplicate,
                session_id: row.get::<Option<String>, _>("session_id").map(SessionId::new),
                run_id: row.get::<Option<String>, _>("run_id").map(RunId::new),
                result_code: row.get("result_code"),
            });
        }
        let account_tenant: Option<String> = sqlx::query_scalar(
            "SELECT tenant_key FROM integration_provider_accounts WHERE id = ? AND provider_id = ? AND messaging_enabled = 1 AND state IN ('starting', 'healthy', 'degraded')",
        )
        .bind(&message.address.account_id)
        .bind(&message.address.provider_id)
        .fetch_optional(self.store.pool())
        .await?;
        if is_managed_integration_provider(&message.address.provider_id)
            && account_tenant.as_deref() != Some(message.address.tenant_key.as_str())
        {
            return Err(GatewayError::RouteNotAllowed);
        }
        let is_formal_account = account_tenant.is_some();
        let control = parse_control_command(message)?;
        let authorization = match control {
            Some(ChannelControlCommand::Connect { ref code }) => Some(
                self.consume_pairing_code(message, code, message.received_at_ms)
                    .await?
                    .authorization,
            ),
            _ => self.authorize_message(message).await?,
        };
        if is_formal_account {
            self.remember_external_identity(message, message.received_at_ms)
                .await?;
        }
        let grant_snapshot = authorization
            .as_ref()
            .map(|value| &value.grant)
            .cloned()
            .unwrap_or_default();
        let mut transaction = self.store.pool().begin().await?;
        let inserted = sqlx::query("INSERT OR IGNORE INTO channel_ingress(provider_id, account_id, external_message_id, address_json, actor_id, payload_hash, normalized_payload_json, status, claim_token, claim_expires_at_ms, session_id, run_id, authorization_id, authorization_revision, grant_snapshot_json, result_code, provider_receipt, received_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, 'accepted', NULL, NULL, NULL, NULL, ?, ?, ?, 'accepted', NULL, ?, ?)")
            .bind(&key.provider_id)
            .bind(&key.account_id)
            .bind(&key.external_message_id)
            .bind(serde_json::to_string(&message.address)?)
            .bind(&message.actor.external_id)
            .bind(&payload_hash)
            .bind(String::from_utf8(serialized).map_err(|_| GatewayError::InvalidMessage)?)
            .bind(authorization.as_ref().map(|value| value.id.as_str()))
            .bind(authorization.as_ref().map(|value| crate::to_i64(value.revision)))
            .bind(serde_json::to_string(&grant_snapshot)?)
            .bind(message.received_at_ms)
            .bind(message.received_at_ms)
            .execute(&mut *transaction)
            .await?;
        if inserted.rows_affected() == 0 {
            return Err(GatewayError::IngressConflict);
        }
        if is_formal_account {
            for media in message.parts.iter().filter_map(media_descriptor) {
                sqlx::query("INSERT INTO channel_attachment_metadata(platform, account_id, event_id, remote_id, resource_key, file_name, mime_type, declared_size_bytes, expected_content_hash, metadata_hash, artifact_id, created_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?)")
                    .bind(&key.provider_id)
                    .bind(&key.account_id)
                    .bind(&key.external_message_id)
                    .bind(&media.remote_id)
                    .bind(&media.resource_key)
                    .bind(&media.file_name)
                    .bind(&media.mime_type)
                    .bind(media.declared_size_bytes.map(crate::to_i64))
                    .bind(&media.content_hash)
                    .bind(remote_media_metadata_hash(media)?)
                    .bind(message.received_at_ms)
                    .execute(&mut *transaction)
                    .await?;
            }
        }
        transaction.commit().await?;
        Ok(IngressReceipt {
            event_key: key.clone(),
            status: IngressStatus::Accepted,
            session_id: None,
            run_id: None,
            result_code: "accepted".into(),
        })
    }

    pub async fn claim_next_ingress(
        &self,
        timestamp_ms: i64,
    ) -> Result<Option<VerifiedChannelMessage>, GatewayError> {
        let mut transaction = self.store.pool().begin().await?;
        let row = sqlx::query("SELECT provider_id, account_id, external_message_id, normalized_payload_json FROM channel_ingress WHERE (status = 'accepted' AND claim_token IS NULL) OR (status = 'run_created' AND COALESCE(claim_expires_at_ms, 0) <= ?) ORDER BY received_at_ms, provider_id, account_id, external_message_id LIMIT 1")
            .bind(timestamp_ms)
            .fetch_optional(&mut *transaction)
            .await?;
        let Some(row) = row else {
            transaction.commit().await?;
            return Ok(None);
        };
        let key = row_key(&row);
        let claim_token = Uuid::now_v7().to_string();
        let updated = sqlx::query("UPDATE channel_ingress SET status = 'claimed', claim_token = ?, claim_expires_at_ms = ?, result_code = 'claimed', updated_at_ms = ? WHERE provider_id = ? AND account_id = ? AND external_message_id = ? AND ((status = 'accepted' AND claim_token IS NULL) OR (status = 'run_created' AND COALESCE(claim_expires_at_ms, 0) <= ?))")
            .bind(&claim_token)
            .bind(timestamp_ms.saturating_add(CLAIM_TTL_MS))
            .bind(timestamp_ms)
            .bind(&key.provider_id)
            .bind(&key.account_id)
            .bind(&key.external_message_id)
            .bind(timestamp_ms)
            .execute(&mut *transaction)
            .await?;
        if updated.rows_affected() != 1 {
            return Err(GatewayError::IngressConflict);
        }
        let message = serde_json::from_str(
            row.get::<Option<&str>, _>("normalized_payload_json")
                .ok_or(GatewayError::IngressConflict)?,
        )?;
        transaction.commit().await?;
        Ok(Some(message))
    }

    pub async fn record_ingress_run(
        &self,
        key: &ChannelEventKey,
        session_id: &SessionId,
        run_id: &RunId,
        timestamp_ms: i64,
    ) -> Result<IngressReceipt, GatewayError> {
        let updated = sqlx::query("UPDATE channel_ingress SET status = 'run_created', session_id = ?, run_id = ?, result_code = 'run_created', updated_at_ms = ? WHERE provider_id = ? AND account_id = ? AND external_message_id = ? AND status = 'claimed' AND (run_id IS NULL OR (run_id = ? AND session_id = ?))")
            .bind(session_id.as_str())
            .bind(run_id.as_str())
            .bind(timestamp_ms)
            .bind(&key.provider_id)
            .bind(&key.account_id)
            .bind(&key.external_message_id)
            .bind(run_id.as_str())
            .bind(session_id.as_str())
            .execute(self.store.pool())
            .await?;
        if updated.rows_affected() != 1 {
            return Err(GatewayError::IngressConflict);
        }
        Ok(IngressReceipt {
            event_key: key.clone(),
            status: IngressStatus::RunCreated,
            session_id: Some(session_id.clone()),
            run_id: Some(run_id.clone()),
            result_code: "run_created".into(),
        })
    }

    pub async fn ingress_run(
        &self,
        key: &hachimi_protocol::ChannelEventKey,
    ) -> Result<Option<(SessionId, RunId)>, GatewayError> {
        let row = sqlx::query("SELECT session_id, run_id FROM channel_ingress WHERE provider_id = ? AND account_id = ? AND external_message_id = ? AND run_id IS NOT NULL AND session_id IS NOT NULL")
            .bind(&key.provider_id)
            .bind(&key.account_id)
            .bind(&key.external_message_id)
            .fetch_optional(self.store.pool())
            .await?;
        Ok(row.map(|row| {
            (
                SessionId::new(row.get::<String, _>("session_id")),
                RunId::new(row.get::<String, _>("run_id")),
            )
        }))
    }

    pub async fn ingress_grant_snapshot(
        &self,
        key: &ChannelEventKey,
    ) -> Result<ChannelGrant, GatewayError> {
        let value: String = sqlx::query_scalar("SELECT grant_snapshot_json FROM channel_ingress WHERE provider_id = ? AND account_id = ? AND external_message_id = ?")
            .bind(&key.provider_id)
            .bind(&key.account_id)
            .bind(&key.external_message_id)
            .fetch_optional(self.store.pool())
            .await?
            .ok_or(GatewayError::IngressConflict)?;
        serde_json::from_str(&value).map_err(Into::into)
    }

    pub async fn finish_control_ingress(
        &self,
        key: &hachimi_protocol::ChannelEventKey,
        session_id: Option<&SessionId>,
        result_code: &str,
        timestamp_ms: i64,
    ) -> Result<IngressReceipt, GatewayError> {
        let updated = sqlx::query("UPDATE channel_ingress SET status = 'completed', normalized_payload_json = NULL, claim_token = NULL, claim_expires_at_ms = NULL, session_id = COALESCE(?, session_id), result_code = ?, updated_at_ms = ? WHERE provider_id = ? AND account_id = ? AND external_message_id = ? AND status = 'claimed'")
            .bind(session_id.map(SessionId::as_str))
            .bind(result_code)
            .bind(timestamp_ms)
            .bind(&key.provider_id)
            .bind(&key.account_id)
            .bind(&key.external_message_id)
            .execute(self.store.pool())
            .await?;
        if updated.rows_affected() != 1 {
            return Err(GatewayError::IngressConflict);
        }
        Ok(IngressReceipt {
            event_key: key.clone(),
            status: IngressStatus::Completed,
            session_id: session_id.cloned(),
            run_id: None,
            result_code: result_code.into(),
        })
    }

    pub async fn finish_ingress(
        &self,
        key: &ChannelEventKey,
        session_id: &SessionId,
        run_id: &RunId,
        needs_attention: bool,
        timestamp_ms: i64,
    ) -> Result<IngressReceipt, GatewayError> {
        let status = if needs_attention {
            "needs_attention"
        } else {
            "completed"
        };
        let updated = sqlx::query("UPDATE channel_ingress SET status = ?, normalized_payload_json = NULL, claim_token = NULL, claim_expires_at_ms = NULL, session_id = ?, run_id = ?, result_code = ?, updated_at_ms = ? WHERE provider_id = ? AND account_id = ? AND external_message_id = ? AND status = 'run_created'")
            .bind(status)
            .bind(session_id.as_str())
            .bind(run_id.as_str())
            .bind(status)
            .bind(timestamp_ms)
            .bind(&key.provider_id)
            .bind(&key.account_id)
            .bind(&key.external_message_id)
            .execute(self.store.pool())
            .await?;
        if updated.rows_affected() != 1 {
            return Err(GatewayError::IngressConflict);
        }
        Ok(IngressReceipt {
            event_key: key.clone(),
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
        key: &ChannelEventKey,
        result_code: &str,
        timestamp_ms: i64,
    ) -> Result<IngressReceipt, GatewayError> {
        if result_code.trim().is_empty() || result_code.len() > 128 {
            return Err(GatewayError::InvalidMessage);
        }
        let updated = sqlx::query("UPDATE channel_ingress SET status = 'needs_attention', normalized_payload_json = NULL, claim_token = NULL, claim_expires_at_ms = NULL, result_code = ?, updated_at_ms = ? WHERE provider_id = ? AND account_id = ? AND external_message_id = ? AND status IN ('claimed', 'run_created')")
            .bind(result_code)
            .bind(timestamp_ms)
            .bind(&key.provider_id)
            .bind(&key.account_id)
            .bind(&key.external_message_id)
            .execute(self.store.pool())
            .await?;
        if updated.rows_affected() != 1 {
            return Err(GatewayError::IngressConflict);
        }
        Ok(IngressReceipt {
            event_key: key.clone(),
            status: IngressStatus::NeedsAttention,
            session_id: None,
            run_id: None,
            result_code: result_code.into(),
        })
    }

    pub async fn enqueue_delivery(
        &self,
        address: ChannelConversationAddress,
        idempotency_key: &str,
        payload: ChannelOutboundPayload,
        timestamp_ms: i64,
    ) -> Result<DeliveryAttempt, GatewayError> {
        let context = self.proactive_delivery_context(&address).await?;
        self.insert_delivery(address, idempotency_key, payload, context, timestamp_ms)
            .await
    }

    pub async fn enqueue_reactive_delivery(
        &self,
        source: ReactiveDeliverySource<'_>,
        address: ChannelConversationAddress,
        part_index: u32,
        payload: ChannelOutboundPayload,
        timestamp_ms: i64,
    ) -> Result<DeliveryAttempt, GatewayError> {
        let ReactiveDeliverySource {
            event_key,
            run_id,
            final_item_id,
        } = source;
        if final_item_id.trim().is_empty() || final_item_id.len() > 256 {
            return Err(GatewayError::InvalidMessage);
        }
        let row = sqlx::query("SELECT address_json, authorization_id, authorization_revision FROM channel_ingress WHERE provider_id = ? AND account_id = ? AND external_message_id = ?")
            .bind(&event_key.provider_id)
            .bind(&event_key.account_id)
            .bind(&event_key.external_message_id)
            .fetch_optional(self.store.pool())
            .await?
            .ok_or(GatewayError::IngressConflict)?;
        let ingress_address: ChannelConversationAddress =
            serde_json::from_str(row.get("address_json"))?;
        if ingress_address != address {
            return Err(GatewayError::DeliveryConflict);
        }
        let account_config_revision = self.account_config_revision(&address).await?;
        let idempotency_key = reactive_delivery_key(event_key, run_id, final_item_id, part_index);
        let authorization_id: Option<String> = row.get("authorization_id");
        let authorization_revision = row
            .get::<Option<i64>, _>("authorization_revision")
            .map(crate::from_i64);
        self.insert_delivery(
            address,
            &idempotency_key,
            payload,
            OutboxProvenance {
                authorization_id,
                authorization_revision,
                account_config_revision,
                reactive_external_message_id: Some(event_key.external_message_id.clone()),
                run_id: run_id.map(|value| value.as_str().to_owned()),
                final_item_id: Some(final_item_id.to_owned()),
                part_index: Some(part_index),
            },
            timestamp_ms,
        )
        .await
    }

    async fn insert_delivery(
        &self,
        address: ChannelConversationAddress,
        idempotency_key: &str,
        payload: ChannelOutboundPayload,
        provenance: OutboxProvenance,
        timestamp_ms: i64,
    ) -> Result<DeliveryAttempt, GatewayError> {
        validate_outbound(idempotency_key, &payload)?;
        let candidate = DeliveryAttempt {
            id: ChannelDeliveryId::random(),
            address: address.clone(),
            idempotency_key: idempotency_key.into(),
            payload: payload.clone(),
            status: DeliveryAttemptStatus::Pending,
            attempt: 0,
            claim_token: None,
            next_attempt_at_ms: Some(timestamp_ms),
            error_code: None,
            provider_receipt: None,
        };
        let address_json = serde_json::to_string(&address)?;
        let payload_json = serde_json::to_string(&payload)?;
        sqlx::query("INSERT OR IGNORE INTO channel_outbox(id, provider_id, account_id, address_json, payload_json, reply_context_json, idempotency_key, status, attempt, claim_token, claim_expires_at_ms, next_attempt_at_ms, error_code, provider_receipt, authorization_id, authorization_revision, account_config_revision, reactive_external_message_id, run_id, final_item_id, part_index, dispatched_at_ms, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, NULL, ?, 'pending', 0, NULL, NULL, ?, NULL, NULL, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?)")
            .bind(candidate.id.as_str())
            .bind(&address.provider_id)
            .bind(&address.account_id)
            .bind(&address_json)
            .bind(&payload_json)
            .bind(idempotency_key)
            .bind(timestamp_ms)
            .bind(provenance.authorization_id.as_deref())
            .bind(provenance.authorization_revision.map(crate::to_i64))
            .bind(provenance.account_config_revision.map(crate::to_i64))
            .bind(provenance.reactive_external_message_id.as_deref())
            .bind(provenance.run_id.as_deref())
            .bind(provenance.final_item_id.as_deref())
            .bind(provenance.part_index.map(i64::from))
            .bind(timestamp_ms)
            .bind(timestamp_ms)
            .execute(self.store.pool())
            .await?;
        let persisted = sqlx::query(
            "SELECT address_json, payload_json FROM channel_outbox WHERE idempotency_key = ?",
        )
        .bind(idempotency_key)
        .fetch_one(self.store.pool())
        .await?;
        if persisted.get::<&str, _>("address_json") != address_json
            || persisted.get::<&str, _>("payload_json") != payload_json
        {
            return Err(GatewayError::IdempotencyConflict);
        }
        self.delivery_by_key(idempotency_key)
            .await?
            .ok_or(GatewayError::DeliveryConflict)
    }

    pub async fn enqueue_text_delivery(
        &self,
        address: ChannelConversationAddress,
        idempotency_key: &str,
        text: &str,
        reply_to: Option<String>,
        timestamp_ms: i64,
    ) -> Result<DeliveryAttempt, GatewayError> {
        self.enqueue_delivery(
            address,
            idempotency_key,
            ChannelOutboundPayload {
                parts: vec![ChannelMessagePart::Text { text: text.into() }],
                reply_to_external_message_id: reply_to,
            },
            timestamp_ms,
        )
        .await
    }

    pub async fn enqueue_reactive_text_delivery(
        &self,
        source: ReactiveDeliverySource<'_>,
        address: ChannelConversationAddress,
        text: &str,
        reply_to: Option<String>,
        timestamp_ms: i64,
    ) -> Result<DeliveryAttempt, GatewayError> {
        self.enqueue_reactive_delivery(
            source,
            address,
            0,
            ChannelOutboundPayload {
                parts: vec![ChannelMessagePart::Text { text: text.into() }],
                reply_to_external_message_id: reply_to,
            },
            timestamp_ms,
        )
        .await
    }

    async fn proactive_delivery_context(
        &self,
        address: &ChannelConversationAddress,
    ) -> Result<OutboxProvenance, GatewayError> {
        let account_config_revision = self.account_config_revision(address).await?;
        if !is_managed_integration_provider(&address.provider_id) {
            return Ok(OutboxProvenance::default());
        }
        if address.provider_id == "wechat_ilink" {
            return Err(GatewayError::RouteNotAllowed);
        }
        if (address.chat_kind == ChannelChatKind::Group
            && !provider_supports_groups(&address.provider_id))
            || (address.topic_id.is_some() && address.provider_id != "feishu")
        {
            return Err(GatewayError::InvalidMessage);
        }
        let rows = sqlx::query("SELECT id, revision, chat_kind, chat_id, topic_id, actor_id, topic_policy, enabled FROM channel_authorizations WHERE account_id = ? AND provider_id = ? AND tenant_key = ? AND chat_id = ? AND enabled = 1 ORDER BY revision DESC")
            .bind(&address.account_id)
            .bind(&address.provider_id)
            .bind(&address.tenant_key)
            .bind(&address.chat_id)
            .fetch_all(self.store.pool())
            .await?;
        let authorization = rows
            .iter()
            .find(|row| authorization_row_matches_address(row, address))
            .ok_or(GatewayError::RouteNotAllowed)?;
        let authorization_id: String = authorization.get("id");
        let authorization_revision = crate::from_i64(authorization.get("revision"));
        Ok(OutboxProvenance {
            authorization_id: Some(authorization_id),
            authorization_revision: Some(authorization_revision),
            account_config_revision,
            ..OutboxProvenance::default()
        })
    }

    async fn account_config_revision(
        &self,
        address: &ChannelConversationAddress,
    ) -> Result<Option<u64>, GatewayError> {
        if !is_managed_integration_provider(&address.provider_id) {
            return Ok(None);
        }
        let row = sqlx::query("SELECT provider_id, state, messaging_enabled, config_revision FROM integration_provider_accounts WHERE id = ?")
            .bind(&address.account_id)
            .fetch_optional(self.store.pool())
            .await?
            .ok_or(GatewayError::ProviderUnavailable)?;
        let state: &str = row.get("state");
        if row.get::<&str, _>("provider_id") != address.provider_id
            || !row.get::<bool, _>("messaging_enabled")
            || !matches!(state, "healthy" | "degraded")
        {
            return Err(GatewayError::ProviderUnavailable);
        }
        Ok(Some(crate::from_i64(row.get("config_revision"))))
    }

    async fn delivery_authorization_is_current(&self, id: &str) -> Result<bool, GatewayError> {
        let row = sqlx::query("SELECT provider_id, account_id, address_json, authorization_id, authorization_revision, account_config_revision, reactive_external_message_id FROM channel_outbox WHERE id = ?")
            .bind(id)
            .fetch_optional(self.store.pool())
            .await?
            .ok_or(GatewayError::DeliveryConflict)?;
        let provider_id: &str = row.get("provider_id");
        if !is_managed_integration_provider(provider_id) {
            return Ok(true);
        }
        let account_id: &str = row.get("account_id");
        let account = sqlx::query("SELECT provider_id, state, messaging_enabled, config_revision FROM integration_provider_accounts WHERE id = ?")
            .bind(account_id)
            .fetch_optional(self.store.pool())
            .await?;
        let Some(account) = account else {
            return Ok(false);
        };
        let state: &str = account.get("state");
        let expected_config_revision: Option<i64> = row.get("account_config_revision");
        if account.get::<&str, _>("provider_id") != provider_id
            || !account.get::<bool, _>("messaging_enabled")
            || !matches!(state, "healthy" | "degraded")
            || expected_config_revision != Some(account.get("config_revision"))
        {
            return Ok(false);
        }
        let address: ChannelConversationAddress = serde_json::from_str(row.get("address_json"))?;
        let authorization_id: Option<&str> = row.get("authorization_id");
        let authorization_revision: Option<i64> = row.get("authorization_revision");
        if let Some(authorization_id) = authorization_id {
            let authorization = sqlx::query("SELECT revision, chat_kind, chat_id, topic_id, actor_id, topic_policy, enabled FROM channel_authorizations WHERE id = ? AND account_id = ? AND provider_id = ? AND tenant_key = ?")
                .bind(authorization_id)
                .bind(account_id)
                .bind(provider_id)
                .bind(&address.tenant_key)
                .fetch_optional(self.store.pool())
                .await?;
            return Ok(authorization.is_some_and(|authorization| {
                authorization.get::<bool, _>("enabled")
                    && authorization_revision == Some(authorization.get("revision"))
                    && authorization_row_matches_address(&authorization, &address)
            }));
        }
        if authorization_revision.is_some() {
            return Ok(false);
        }
        let Some(external_message_id) = row.get::<Option<&str>, _>("reactive_external_message_id")
        else {
            return Ok(false);
        };
        let ingress_address: Option<String> = sqlx::query_scalar("SELECT address_json FROM channel_ingress WHERE provider_id = ? AND account_id = ? AND external_message_id = ? AND authorization_id IS NULL")
            .bind(provider_id)
            .bind(account_id)
            .bind(external_message_id)
            .fetch_optional(self.store.pool())
            .await?;
        Ok(ingress_address.is_some_and(|value| value == row.get::<&str, _>("address_json")))
    }

    pub async fn claim_next_delivery(
        &self,
        timestamp_ms: i64,
    ) -> Result<Option<DeliveryAttempt>, GatewayError> {
        self.claim_delivery(None, timestamp_ms).await
    }

    pub async fn claim_next_delivery_for_channel(
        &self,
        provider_id: &str,
        timestamp_ms: i64,
    ) -> Result<Option<DeliveryAttempt>, GatewayError> {
        self.claim_delivery(Some(provider_id), timestamp_ms).await
    }

    async fn claim_delivery(
        &self,
        provider_id: Option<&str>,
        timestamp_ms: i64,
    ) -> Result<Option<DeliveryAttempt>, GatewayError> {
        for _ in 0..32 {
            let mut transaction = self.store.pool().begin().await?;
            let row = if let Some(provider_id) = provider_id {
                sqlx::query("SELECT * FROM channel_outbox WHERE provider_id = ? AND status IN ('pending', 'retry_scheduled') AND COALESCE(next_attempt_at_ms, 0) <= ? ORDER BY COALESCE(next_attempt_at_ms, 0), created_at_ms, id LIMIT 1")
                    .bind(provider_id)
                    .bind(timestamp_ms)
                    .fetch_optional(&mut *transaction)
                    .await?
            } else {
                sqlx::query("SELECT * FROM channel_outbox WHERE status IN ('pending', 'retry_scheduled') AND COALESCE(next_attempt_at_ms, 0) <= ? ORDER BY COALESCE(next_attempt_at_ms, 0), created_at_ms, id LIMIT 1")
                    .bind(timestamp_ms)
                    .fetch_optional(&mut *transaction)
                    .await?
            };
            let Some(row) = row else {
                transaction.commit().await?;
                return Ok(None);
            };
            let id = row.get::<String, _>("id");
            let token = Uuid::now_v7().to_string();
            let updated = sqlx::query("UPDATE channel_outbox SET status = 'claimed', attempt = attempt + 1, claim_token = ?, claim_expires_at_ms = ?, next_attempt_at_ms = NULL, updated_at_ms = ? WHERE id = ? AND status IN ('pending', 'retry_scheduled')")
                .bind(&token)
                .bind(timestamp_ms.saturating_add(CLAIM_TTL_MS))
                .bind(timestamp_ms)
                .bind(&id)
                .execute(&mut *transaction)
                .await?;
            if updated.rows_affected() != 1 {
                return Err(GatewayError::DeliveryConflict);
            }
            transaction.commit().await?;
            if self.delivery_authorization_is_current(&id).await? {
                return self.delivery_by_id(&id).await;
            }
            sqlx::query("UPDATE channel_outbox SET status = 'permanent_failure', claim_token = NULL, claim_expires_at_ms = NULL, error_code = 'delivery_authorization_stale', updated_at_ms = ? WHERE id = ? AND status = 'claimed' AND claim_token = ?")
                .bind(timestamp_ms)
                .bind(&id)
                .bind(&token)
                .execute(self.store.pool())
                .await?;
        }
        Ok(None)
    }

    pub async fn mark_delivery_dispatched(
        &self,
        id: &ChannelDeliveryId,
        timestamp_ms: i64,
    ) -> Result<DeliveryAttempt, GatewayError> {
        let current = self
            .delivery_by_id(id.as_str())
            .await?
            .ok_or(GatewayError::DeliveryConflict)?;
        if current.status != DeliveryAttemptStatus::Claimed {
            return Err(GatewayError::DeliveryConflict);
        }
        let updated = sqlx::query("UPDATE channel_outbox SET dispatched_at_ms = COALESCE(dispatched_at_ms, ?), updated_at_ms = ? WHERE id = ? AND status = 'claimed' AND claim_token = ?")
            .bind(timestamp_ms)
            .bind(timestamp_ms)
            .bind(id.as_str())
            .bind(current.claim_token.as_deref())
            .execute(self.store.pool())
            .await?;
        if updated.rows_affected() != 1 {
            return Err(GatewayError::DeliveryConflict);
        }
        self.delivery_by_id(id.as_str())
            .await?
            .ok_or(GatewayError::DeliveryConflict)
    }

    pub async fn finish_delivery(
        &self,
        id: &ChannelDeliveryId,
        outcome: ChannelDeliveryOutcome,
        timestamp_ms: i64,
    ) -> Result<DeliveryAttempt, GatewayError> {
        let current = self
            .delivery_by_id(id.as_str())
            .await?
            .ok_or(GatewayError::DeliveryConflict)?;
        if current.status != DeliveryAttemptStatus::Claimed {
            return Err(GatewayError::DeliveryConflict);
        }
        let (status, next_attempt) = if outcome.delivered {
            ("delivered", None)
        } else if outcome.indeterminate {
            ("indeterminate", None)
        } else if outcome.retryable && current.attempt < MAX_DELIVERY_ATTEMPTS {
            (
                "retry_scheduled",
                Some(timestamp_ms.saturating_add(retry_delay_ms(current.attempt))),
            )
        } else {
            ("permanent_failure", None)
        };
        let updated = sqlx::query("UPDATE channel_outbox SET status = ?, claim_token = NULL, claim_expires_at_ms = NULL, next_attempt_at_ms = ?, error_code = ?, provider_receipt = ?, updated_at_ms = ? WHERE id = ? AND status = 'claimed' AND claim_token = ?")
            .bind(status)
            .bind(next_attempt)
            .bind((!outcome.delivered).then_some(outcome.result_code.as_str()))
            .bind(outcome.provider_receipt.as_deref())
            .bind(timestamp_ms)
            .bind(id.as_str())
            .bind(current.claim_token.as_deref())
            .execute(self.store.pool())
            .await?;
        if updated.rows_affected() != 1 {
            return Err(GatewayError::DeliveryConflict);
        }
        self.delivery_by_id(id.as_str())
            .await?
            .ok_or(GatewayError::DeliveryConflict)
    }

    pub async fn process_next_provider_ingress(
        &self,
    ) -> Result<Option<IngressReceipt>, GatewayError> {
        if !self.provider_ingress_enabled {
            return Ok(None);
        }
        for provider_id in self.providers.provider_ids() {
            let Some(provider) = self.providers.resolve(&provider_id) else {
                continue;
            };
            let Some(message) = provider.claim_ingress().await? else {
                continue;
            };
            return self
                .ingest_provider(&provider_id, None, message)
                .await
                .map(Some);
        }
        Ok(None)
    }

    pub async fn process_next_provider_delivery(
        &self,
        timestamp_ms: i64,
    ) -> Result<Option<DeliveryAttempt>, GatewayError> {
        for provider_id in self.providers.provider_ids() {
            let Some(provider) = self.providers.resolve(&provider_id) else {
                continue;
            };
            if !provider.push_delivery() {
                continue;
            }
            let Some(delivery) = self
                .claim_next_delivery_for_channel(&provider_id, timestamp_ms)
                .await?
            else {
                continue;
            };
            let delivery = self
                .mark_delivery_dispatched(&delivery.id, timestamp_ms)
                .await?;
            let outcome = match provider.deliver(&delivery).await {
                Ok(outcome) => outcome,
                Err(error) => {
                    tracing::warn!(
                        delivery_id = %delivery.id,
                        %error,
                        "Provider delivery failed after dispatch began"
                    );
                    return self
                        .finish_delivery(
                            &delivery.id,
                            ChannelDeliveryOutcome {
                                delivered: false,
                                retryable: false,
                                indeterminate: true,
                                result_code: "provider_dispatch_error".into(),
                                provider_receipt: None,
                            },
                            timestamp_ms,
                        )
                        .await
                        .map(Some);
                }
            };
            if outcome.delivered
                && let Err(error) = provider.ack_delivery(&delivery).await
            {
                tracing::warn!(
                    delivery_id = %delivery.id,
                    %error,
                    "Provider delivery ACK failed after a confirmed delivery"
                );
            }
            return self
                .finish_provider_delivery(&delivery, outcome, timestamp_ms)
                .await
                .map(Some);
        }
        Ok(None)
    }

    async fn finish_provider_delivery(
        &self,
        delivery: &DeliveryAttempt,
        outcome: ChannelDeliveryOutcome,
        timestamp_ms: i64,
    ) -> Result<DeliveryAttempt, GatewayError> {
        self.finish_delivery(&delivery.id, outcome, timestamp_ms)
            .await
    }

    pub async fn reconcile_startup(&self, timestamp_ms: i64) -> Result<(), GatewayError> {
        sqlx::query("UPDATE channel_ingress SET status = CASE WHEN run_id IS NULL THEN 'accepted' ELSE 'run_created' END, claim_token = NULL, claim_expires_at_ms = NULL, result_code = 'reconciled', updated_at_ms = ? WHERE status = 'claimed' AND COALESCE(claim_expires_at_ms, 0) <= ?")
            .bind(timestamp_ms)
            .bind(timestamp_ms)
            .execute(self.store.pool())
            .await?;
        sqlx::query("UPDATE channel_outbox SET status = 'indeterminate', claim_token = NULL, claim_expires_at_ms = NULL, next_attempt_at_ms = NULL, error_code = 'dispatch_outcome_unknown', updated_at_ms = ? WHERE status = 'claimed' AND dispatched_at_ms IS NOT NULL AND COALESCE(claim_expires_at_ms, 0) <= ?")
            .bind(timestamp_ms)
            .bind(timestamp_ms)
            .execute(self.store.pool())
            .await?;
        sqlx::query("UPDATE channel_outbox SET status = 'retry_scheduled', claim_token = NULL, claim_expires_at_ms = NULL, next_attempt_at_ms = ?, error_code = 'claim_recovered', updated_at_ms = ? WHERE status = 'claimed' AND dispatched_at_ms IS NULL AND COALESCE(claim_expires_at_ms, 0) <= ?")
            .bind(timestamp_ms)
            .bind(timestamp_ms)
            .bind(timestamp_ms)
            .execute(self.store.pool())
            .await?;
        sqlx::query("DELETE FROM channel_session_bindings WHERE session_id IN (SELECT id FROM sessions WHERE archived = 1)")
            .execute(self.store.pool())
            .await?;
        self.reload_configuration().await?;
        Ok(())
    }

    async fn delivery_by_key(&self, key: &str) -> Result<Option<DeliveryAttempt>, GatewayError> {
        let row = sqlx::query("SELECT * FROM channel_outbox WHERE idempotency_key = ?")
            .bind(key)
            .fetch_optional(self.store.pool())
            .await?;
        row.map(decode_delivery).transpose()
    }

    async fn delivery_by_id(&self, id: &str) -> Result<Option<DeliveryAttempt>, GatewayError> {
        let row = sqlx::query("SELECT * FROM channel_outbox WHERE id = ?")
            .bind(id)
            .fetch_optional(self.store.pool())
            .await?;
        row.map(decode_delivery).transpose()
    }
}

fn validate_message(message: &VerifiedChannelMessage) -> Result<(), GatewayError> {
    let address = &message.address;
    if message.event_key.provider_id != address.provider_id
        || message.event_key.account_id != address.account_id
        || message.event_key.external_message_id.trim().is_empty()
        || address.provider_id.trim().is_empty()
        || address.account_id.trim().is_empty()
        || address.tenant_key.trim().is_empty()
        || address.chat_id.trim().is_empty()
        || message.actor.external_id.trim().is_empty()
        || message.actor.is_bot
        || message.parts.is_empty()
        || message.parts.len() > 8
        || (address.chat_kind == ChannelChatKind::Group
            && is_managed_integration_provider(&address.provider_id)
            && !provider_supports_groups(&address.provider_id))
        || (address.topic_id.is_some()
            && is_managed_integration_provider(&address.provider_id)
            && address.provider_id != "feishu")
    {
        return Err(GatewayError::InvalidMessage);
    }
    let mut declared_total = 0_u64;
    let mut remote_ids = BTreeSet::new();
    for media in message.parts.iter().filter_map(media_descriptor) {
        if media.provider_id.as_str() != address.provider_id
            || media.remote_id.trim().is_empty()
            || media.remote_id.len() > 1_024
            || media
                .resource_key
                .as_ref()
                .is_some_and(|value| value.len() > 256)
            || media
                .file_name
                .as_ref()
                .is_some_and(|value| value.len() > 255)
            || media
                .mime_type
                .as_ref()
                .is_some_and(|value| value.len() > 256)
            || !media.download_required
            || media
                .declared_size_bytes
                .is_some_and(|size| size > MAX_MEDIA_PART_BYTES)
            || media.content_hash.as_ref().is_some_and(|hash| {
                hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            || !remote_ids.insert(media.remote_id.as_str())
        {
            return Err(GatewayError::InvalidMessage);
        }
        declared_total = declared_total.saturating_add(media.declared_size_bytes.unwrap_or(0));
    }
    if declared_total > MAX_MEDIA_MESSAGE_BYTES {
        return Err(GatewayError::InvalidMessage);
    }
    Ok(())
}

fn is_managed_integration_provider(provider_id: &str) -> bool {
    matches!(
        provider_id,
        "dingtalk" | "feishu" | "wecom_ai_bot" | "wecom_app" | "wechat_ilink"
    )
}

fn provider_supports_groups(provider_id: &str) -> bool {
    matches!(provider_id, "dingtalk" | "feishu" | "wecom_ai_bot")
}

fn authorization_row_matches_address(
    row: &sqlx::sqlite::SqliteRow,
    address: &ChannelConversationAddress,
) -> bool {
    if row.get::<&str, _>("chat_id") != address.chat_id {
        return false;
    }
    let topic_id: Option<&str> = row.get("topic_id");
    let actor_id: Option<&str> = row.get("actor_id");
    match address.chat_kind {
        ChannelChatKind::Dm => {
            row.get::<&str, _>("chat_kind") == "dm"
                && topic_id.is_none()
                && actor_id == Some(address.chat_id.as_str())
        }
        ChannelChatKind::Group => {
            if row.get::<&str, _>("chat_kind") != "group" || actor_id.is_some() {
                return false;
            }
            match row.get::<&str, _>("topic_policy") {
                "inherit_group" => true,
                "isolate_topic" => {
                    address.topic_id.is_some() && topic_id == address.topic_id.as_deref()
                }
                _ => false,
            }
        }
    }
}

fn media_descriptor(part: &ChannelMessagePart) -> Option<&RemoteMediaDescriptor> {
    match part {
        ChannelMessagePart::Text { .. } => None,
        ChannelMessagePart::Image { media }
        | ChannelMessagePart::File { media }
        | ChannelMessagePart::Audio { media }
        | ChannelMessagePart::Video { media } => Some(media),
    }
}

pub fn remote_media_metadata_hash(media: &RemoteMediaDescriptor) -> Result<String, GatewayError> {
    Ok(digest_hex(&serde_json::to_vec(media)?))
}

fn validate_outbound(key: &str, payload: &ChannelOutboundPayload) -> Result<(), GatewayError> {
    if key.trim().is_empty()
        || key.len() > 256
        || payload.parts.is_empty()
        || payload.parts.len() > 8
        || payload.parts.iter().any(|part| {
            matches!(part, ChannelMessagePart::Text { text } if text.trim().is_empty() || text.chars().count() > MAX_MESSAGE_CHARS)
        })
    {
        return Err(GatewayError::InvalidMessage);
    }
    Ok(())
}

fn decode_delivery(row: sqlx::sqlite::SqliteRow) -> Result<DeliveryAttempt, GatewayError> {
    Ok(DeliveryAttempt {
        id: ChannelDeliveryId::new(row.get::<String, _>("id")),
        address: serde_json::from_str(row.get("address_json"))?,
        idempotency_key: row.get("idempotency_key"),
        payload: serde_json::from_str(row.get("payload_json"))?,
        status: match row.get::<&str, _>("status") {
            "pending" => DeliveryAttemptStatus::Pending,
            "claimed" => DeliveryAttemptStatus::Claimed,
            "delivered" => DeliveryAttemptStatus::Delivered,
            "retry_scheduled" => DeliveryAttemptStatus::RetryScheduled,
            "permanent_failure" => DeliveryAttemptStatus::PermanentFailure,
            "indeterminate" => DeliveryAttemptStatus::Indeterminate,
            _ => return Err(GatewayError::DeliveryConflict),
        },
        attempt: u32::try_from(row.get::<i64, _>("attempt")).unwrap_or(u32::MAX),
        claim_token: row.get("claim_token"),
        next_attempt_at_ms: row.get("next_attempt_at_ms"),
        error_code: row.get("error_code"),
        provider_receipt: row.get("provider_receipt"),
    })
}

fn row_key(row: &sqlx::sqlite::SqliteRow) -> ChannelEventKey {
    ChannelEventKey {
        provider_id: row.get("provider_id"),
        account_id: row.get("account_id"),
        external_message_id: row.get("external_message_id"),
    }
}

fn retry_delay_ms(attempt: u32) -> i64 {
    let exponent = attempt.saturating_sub(1).min(10);
    1_000_i64.saturating_mul(1_i64 << exponent)
}

fn digest_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn reactive_delivery_key(
    event_key: &ChannelEventKey,
    run_id: Option<&RunId>,
    final_item_id: &str,
    part_index: u32,
) -> String {
    let mut hasher = Sha256::new();
    for value in [
        event_key.provider_id.as_str(),
        event_key.account_id.as_str(),
        event_key.external_message_id.as_str(),
        run_id.map(RunId::as_str).unwrap_or(""),
        final_item_id,
        &part_index.to_string(),
    ] {
        hasher.update(value.len().to_le_bytes());
        hasher.update(value.as_bytes());
    }
    format!("channel-outbox:{}", digest_hex(&hasher.finalize()))
}
