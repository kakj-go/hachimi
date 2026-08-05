use std::collections::BTreeSet;

use hachimi_protocol::{
    ChannelAccessPolicy, ChannelAccessPolicyUpsert, ChannelAuthorization,
    ChannelAuthorizationTarget, ChannelAuthorizationUpsert, ChannelChatKind, ChannelDmPolicy,
    ChannelGrant, ChannelGroupHistoryPolicy, ChannelIdentityGroup, ChannelIdentityLinkCode,
    ChannelIdentityLinkCodeRequest, ChannelIdentityTransferCommitRequest,
    ChannelIdentityTransferMember, ChannelIdentityTransferPreview, ChannelIdentityTransferResult,
    ChannelMentionKind, ChannelMentionPolicy, ChannelMessagePart, ChannelPairingCode,
    ChannelPairingCodeRequest, ChannelTopicPolicy, EntryProfile, SessionContextBinding, SessionId,
    SessionRecord, VerifiedChannelMessage,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::{GatewayError, GatewayHost, from_i64, to_i64};

const PAIRING_TTL_MS: i64 = 10 * 60 * 1_000;
const PAIRING_COOLDOWN_MS: i64 = 10 * 60 * 1_000;
const PAIRING_FAILURE_LIMIT: i64 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingResolution {
    pub binding_key_hash: String,
    pub binding_key_json: String,
    pub identity_group_id: Option<String>,
    pub session_id: Option<SessionId>,
    pub authorization: Option<ChannelAuthorization>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingConsumeOutcome {
    pub authorization: ChannelAuthorization,
    pub consumed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelControlCommand {
    Connect { code: String },
    Link { code: String },
    New,
    Status,
}

pub fn parse_control_command(
    message: &VerifiedChannelMessage,
) -> Result<Option<ChannelControlCommand>, GatewayError> {
    let text = message.text();
    let trimmed = text.trim();
    let Some(name) = trimmed.split_whitespace().next() else {
        return Ok(None);
    };
    if !matches!(name, "/connect" | "/link" | "/new" | "/status") {
        return Ok(None);
    }
    if message
        .parts
        .iter()
        .any(|part| !matches!(part, ChannelMessagePart::Text { .. }))
    {
        return Err(GatewayError::InvalidMessage);
    }
    let arguments = trimmed.split_whitespace().collect::<Vec<_>>();
    match arguments.as_slice() {
        ["/connect", code] => Ok(Some(ChannelControlCommand::Connect {
            code: (*code).into(),
        })),
        ["/link", code] => Ok(Some(ChannelControlCommand::Link {
            code: (*code).into(),
        })),
        ["/new"] => Ok(Some(ChannelControlCommand::New)),
        ["/status"] => Ok(Some(ChannelControlCommand::Status)),
        _ => Err(GatewayError::InvalidMessage),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BindingKey<'a> {
    scope: &'static str,
    provider_id: Option<&'a str>,
    account_id: Option<&'a str>,
    tenant_key: Option<&'a str>,
    chat_id: Option<&'a str>,
    topic_id: Option<&'a str>,
    actor_id: Option<&'a str>,
    identity_group_id: Option<&'a str>,
}

impl GatewayHost {
    pub async fn remember_external_identity(
        &self,
        message: &VerifiedChannelMessage,
        timestamp_ms: i64,
    ) -> Result<String, GatewayError> {
        let id = Uuid::now_v7().to_string();
        sqlx::query("INSERT INTO channel_external_identities(id, account_id, provider_id, tenant_key, actor_id, display_name, identity_group_id, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, NULL, ?, ?) ON CONFLICT(account_id, tenant_key, actor_id) DO UPDATE SET display_name = COALESCE(excluded.display_name, channel_external_identities.display_name), updated_at_ms = excluded.updated_at_ms")
            .bind(&id)
            .bind(&message.address.account_id)
            .bind(&message.address.provider_id)
            .bind(&message.address.tenant_key)
            .bind(&message.actor.external_id)
            .bind(&message.actor.display_name)
            .bind(timestamp_ms)
            .bind(timestamp_ms)
            .execute(self.store.pool())
            .await?;
        sqlx::query_scalar("SELECT id FROM channel_external_identities WHERE account_id = ? AND tenant_key = ? AND actor_id = ? AND provider_id = ?")
            .bind(&message.address.account_id)
            .bind(&message.address.tenant_key)
            .bind(&message.actor.external_id)
            .bind(&message.address.provider_id)
            .fetch_one(self.store.pool())
            .await
            .map_err(GatewayError::from)
    }

    pub async fn authorize_message(
        &self,
        message: &VerifiedChannelMessage,
    ) -> Result<Option<ChannelAuthorization>, GatewayError> {
        let formal_account: Option<(String, String)> = sqlx::query_as(
            "SELECT state, tenant_key FROM integration_provider_accounts WHERE id = ? AND provider_id = ? AND messaging_enabled = 1",
        )
        .bind(&message.address.account_id)
        .bind(&message.address.provider_id)
        .fetch_optional(self.store.pool())
        .await?;
        let Some((state, tenant_key)) = formal_account else {
            let plugin_account: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channel_provider_accounts WHERE id = ? AND provider_id = ? AND enabled = 1")
                .bind(&message.address.account_id)
                .bind(&message.address.provider_id)
                .fetch_one(self.store.pool())
                .await?;
            return if plugin_account > 0 {
                Ok(None)
            } else {
                Err(GatewayError::RouteNotAllowed)
            };
        };
        if tenant_key != message.address.tenant_key
            || !matches!(state.as_str(), "healthy" | "degraded")
        {
            return Err(GatewayError::RouteNotAllowed);
        }
        if let Some(authorization) = self.find_authorization(message).await? {
            let ceiling = self.grant_ceiling(&message.address.account_id).await?;
            if !grant_is_subset(&authorization.grant, &ceiling) {
                return Err(GatewayError::AuthorizationConflict);
            }
            enforce_authorization(message, &authorization)?;
            return Ok(Some(authorization));
        }
        if message.address.chat_kind == ChannelChatKind::Dm {
            let row = sqlx::query("SELECT dm_policy, allowlist_actor_ids_json FROM channel_access_policies WHERE account_id = ?")
                .bind(&message.address.account_id)
                .fetch_optional(self.store.pool())
                .await?;
            let Some(row) = row else {
                return Err(GatewayError::RouteNotAllowed);
            };
            let policy = parse_dm_policy(row.get("dm_policy"))?;
            let allowlist: Vec<String> = serde_json::from_str(row.get("allowlist_actor_ids_json"))?;
            if policy == ChannelDmPolicy::Open
                || (policy == ChannelDmPolicy::Allowlist
                    && allowlist.iter().any(|id| id == &message.actor.external_id))
            {
                return Ok(None);
            }
        }
        Err(GatewayError::RouteNotAllowed)
    }

    pub async fn access_policy(
        &self,
        account_id: &str,
    ) -> Result<ChannelAccessPolicy, GatewayError> {
        let row = sqlx::query("SELECT dm_policy, allowlist_actor_ids_json, grant_ceiling_json, revision FROM channel_access_policies WHERE account_id = ?")
            .bind(account_id)
            .fetch_optional(self.store.pool())
            .await?
            .ok_or(GatewayError::ProviderUnavailable)?;
        Ok(ChannelAccessPolicy {
            account_id: account_id.into(),
            dm_policy: parse_dm_policy(row.get("dm_policy"))?,
            allowlist_actor_ids: serde_json::from_str(row.get("allowlist_actor_ids_json"))?,
            grant_ceiling: serde_json::from_str(row.get("grant_ceiling_json"))?,
            revision: from_i64(row.get("revision")),
        })
    }

    pub async fn upsert_access_policy(
        &self,
        input: ChannelAccessPolicyUpsert,
        timestamp_ms: i64,
    ) -> Result<ChannelAccessPolicy, GatewayError> {
        validate_policy_input(&input)?;
        let result = sqlx::query("UPDATE channel_access_policies SET dm_policy = ?, allowlist_actor_ids_json = ?, grant_ceiling_json = ?, revision = revision + 1, updated_at_ms = ? WHERE account_id = ? AND revision = ?")
            .bind(dm_policy_str(input.dm_policy))
            .bind(serde_json::to_string(&input.allowlist_actor_ids)?)
            .bind(serde_json::to_string(&input.grant_ceiling)?)
            .bind(timestamp_ms)
            .bind(&input.account_id)
            .bind(to_i64(input.expected_revision))
            .execute(self.store.pool())
            .await?;
        if result.rows_affected() != 1 {
            return Err(GatewayError::AuthorizationConflict);
        }
        self.access_policy(&input.account_id).await
    }

    async fn grant_ceiling(&self, account_id: &str) -> Result<ChannelGrant, GatewayError> {
        let value: String = sqlx::query_scalar(
            "SELECT grant_ceiling_json FROM channel_access_policies WHERE account_id = ?",
        )
        .bind(account_id)
        .fetch_optional(self.store.pool())
        .await?
        .ok_or(GatewayError::RouteNotAllowed)?;
        serde_json::from_str(&value).map_err(Into::into)
    }

    async fn find_authorization(
        &self,
        message: &VerifiedChannelMessage,
    ) -> Result<Option<ChannelAuthorization>, GatewayError> {
        let row = match message.address.chat_kind {
            ChannelChatKind::Dm => sqlx::query("SELECT * FROM channel_authorizations WHERE account_id = ? AND tenant_key = ? AND chat_kind = 'dm' AND chat_id = ? AND actor_id = ? AND enabled = 1 ORDER BY revision DESC LIMIT 1")
                .bind(&message.address.account_id)
                .bind(&message.address.tenant_key)
                .bind(&message.address.chat_id)
                .bind(&message.actor.external_id)
                .fetch_optional(self.store.pool())
                .await?,
            ChannelChatKind::Group => sqlx::query("SELECT * FROM channel_authorizations WHERE account_id = ? AND tenant_key = ? AND chat_kind = 'group' AND chat_id = ? AND actor_id IS NULL AND enabled = 1 ORDER BY CASE WHEN topic_id = ? THEN 0 WHEN topic_id IS NULL THEN 1 ELSE 2 END, revision DESC LIMIT 1")
                .bind(&message.address.account_id)
                .bind(&message.address.tenant_key)
                .bind(&message.address.chat_id)
                .bind(&message.address.topic_id)
                .fetch_optional(self.store.pool())
                .await?,
        };
        row.map(decode_authorization).transpose()
    }

    pub async fn list_authorizations(
        &self,
        account_id: &str,
    ) -> Result<Vec<ChannelAuthorization>, GatewayError> {
        let rows = sqlx::query(
            "SELECT * FROM channel_authorizations WHERE account_id = ? ORDER BY created_at_ms, id",
        )
        .bind(account_id)
        .fetch_all(self.store.pool())
        .await?;
        rows.into_iter().map(decode_authorization).collect()
    }

    pub async fn upsert_authorization(
        &self,
        input: ChannelAuthorizationUpsert,
        source: &str,
        timestamp_ms: i64,
    ) -> Result<ChannelAuthorization, GatewayError> {
        validate_authorization_input(&input)?;
        let ceiling = self.grant_ceiling(&input.account_id).await?;
        if !grant_is_subset(&input.grant, &ceiling) {
            return Err(GatewayError::AuthorizationConflict);
        }
        let account: Option<(String, String)> = sqlx::query_as(
            "SELECT provider_id, tenant_key FROM integration_provider_accounts WHERE id = ?",
        )
        .bind(&input.account_id)
        .fetch_optional(self.store.pool())
        .await?;
        let Some((provider_id, tenant_key)) = account else {
            return Err(GatewayError::ProviderUnavailable);
        };
        if input.address.account_id != input.account_id
            || input.address.provider_id != provider_id
            || input.address.tenant_key != tenant_key
        {
            return Err(GatewayError::AuthorizationConflict);
        }
        validate_provider_authorization_scope(&provider_id, &input)?;
        let duplicate_address: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM channel_authorizations WHERE account_id = ? AND tenant_key = ? AND chat_kind = ? AND chat_id = ? AND COALESCE(topic_id, '') = COALESCE(?, '') AND COALESCE(actor_id, '') = COALESCE(?, '') AND id != ?)")
            .bind(&input.account_id)
            .bind(&input.address.tenant_key)
            .bind(chat_kind_str(input.address.chat_kind))
            .bind(&input.address.chat_id)
            .bind(&input.address.topic_id)
            .bind(&input.actor_id)
            .bind(&input.id)
            .fetch_one(self.store.pool())
            .await?;
        if duplicate_address {
            return Err(GatewayError::AuthorizationConflict);
        }
        let existing: Option<i64> =
            sqlx::query_scalar("SELECT revision FROM channel_authorizations WHERE id = ?")
                .bind(&input.id)
                .fetch_optional(self.store.pool())
                .await?;
        match (existing, input.expected_revision) {
            (None, None) => {}
            (Some(current), Some(expected)) if from_i64(current) == expected => {}
            _ => return Err(GatewayError::AuthorizationConflict),
        }
        let revision = input.expected_revision.unwrap_or(0).saturating_add(1);
        sqlx::query("INSERT INTO channel_authorizations(id, account_id, provider_id, target, tenant_key, chat_kind, chat_id, topic_id, actor_id, group_history_policy, topic_policy, mention_policy, grant_json, source, enabled, revision, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET provider_id = excluded.provider_id, target = excluded.target, tenant_key = excluded.tenant_key, chat_kind = excluded.chat_kind, chat_id = excluded.chat_id, topic_id = excluded.topic_id, actor_id = excluded.actor_id, group_history_policy = excluded.group_history_policy, topic_policy = excluded.topic_policy, mention_policy = excluded.mention_policy, grant_json = excluded.grant_json, enabled = excluded.enabled, revision = excluded.revision, updated_at_ms = excluded.updated_at_ms")
            .bind(&input.id)
            .bind(&input.account_id)
            .bind(&input.address.provider_id)
            .bind(target_str(input.target))
            .bind(&input.address.tenant_key)
            .bind(chat_kind_str(input.address.chat_kind))
            .bind(&input.address.chat_id)
            .bind(&input.address.topic_id)
            .bind(&input.actor_id)
            .bind(input.group_history_policy.map(group_policy_str))
            .bind(topic_policy_str(input.topic_policy))
            .bind(mention_policy_str(input.mention_policy))
            .bind(serde_json::to_string(&input.grant)?)
            .bind(source)
            .bind(input.enabled)
            .bind(to_i64(revision))
            .bind(timestamp_ms)
            .bind(timestamp_ms)
            .execute(self.store.pool())
            .await?;
        self.list_authorizations(&input.account_id)
            .await?
            .into_iter()
            .find(|authorization| authorization.id == input.id)
            .ok_or(GatewayError::AuthorizationConflict)
    }

    pub async fn create_pairing_code(
        &self,
        request: ChannelPairingCodeRequest,
        timestamp_ms: i64,
    ) -> Result<ChannelPairingCode, GatewayError> {
        if request.account_id.trim().is_empty()
            || (request.target == ChannelAuthorizationTarget::GroupConversation
                && request.group_history_policy.is_none())
            || (request.target == ChannelAuthorizationTarget::DmIdentity
                && request.group_history_policy.is_some())
        {
            return Err(GatewayError::InvalidMessage);
        }
        let provider_id: Option<String> = sqlx::query_scalar("SELECT provider_id FROM integration_provider_accounts WHERE id = ? AND messaging_enabled = 1")
            .bind(&request.account_id)
            .fetch_optional(self.store.pool())
            .await?;
        let Some(provider_id) = provider_id else {
            return Err(GatewayError::ProviderUnavailable);
        };
        validate_provider_pairing_scope(&provider_id, &request)?;
        validate_grant(&request.grant)?;
        let ceiling = self.grant_ceiling(&request.account_id).await?;
        if !grant_is_subset(&request.grant, &ceiling) {
            return Err(GatewayError::AuthorizationConflict);
        }
        let id = Uuid::now_v7().to_string();
        let code = crockford_128(*Uuid::new_v4().as_bytes());
        let expires_at_ms = timestamp_ms.saturating_add(PAIRING_TTL_MS);
        sqlx::query("INSERT INTO channel_pairing_codes(id, account_id, code_hash, target, group_history_policy, topic_policy, mention_policy, grant_json, expires_at_ms, consumed_at_ms, consumed_authorization_id, created_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, ?)")
            .bind(&id)
            .bind(&request.account_id)
            .bind(digest_hex(code.as_bytes()))
            .bind(target_str(request.target))
            .bind(request.group_history_policy.map(group_policy_str))
            .bind(topic_policy_str(request.topic_policy))
            .bind(mention_policy_str(request.mention_policy))
            .bind(serde_json::to_string(&request.grant)?)
            .bind(expires_at_ms)
            .bind(timestamp_ms)
            .execute(self.store.pool())
            .await?;
        Ok(ChannelPairingCode {
            id,
            code,
            account_id: request.account_id,
            target: request.target,
            expires_at_ms,
        })
    }

    pub async fn create_identity_link_code(
        &self,
        request: ChannelIdentityLinkCodeRequest,
        timestamp_ms: i64,
    ) -> Result<ChannelIdentityLinkCode, GatewayError> {
        if request.account_id.trim().is_empty() || request.actor_id.trim().is_empty() {
            return Err(GatewayError::InvalidMessage);
        }
        let source_external_identity_id: String = sqlx::query_scalar("SELECT identity.id FROM channel_external_identities AS identity INNER JOIN channel_authorizations AS authorization ON authorization.account_id = identity.account_id AND authorization.tenant_key = identity.tenant_key AND authorization.actor_id = identity.actor_id WHERE identity.account_id = ? AND identity.actor_id = ? AND authorization.chat_kind = 'dm' AND authorization.enabled = 1 LIMIT 1")
            .bind(&request.account_id)
            .bind(&request.actor_id)
            .fetch_optional(self.store.pool())
            .await?
            .ok_or(GatewayError::RouteNotAllowed)?;
        let id = Uuid::now_v7().to_string();
        let code = crockford_128(*Uuid::new_v4().as_bytes());
        let expires_at_ms = timestamp_ms.saturating_add(PAIRING_TTL_MS);
        sqlx::query("INSERT INTO channel_identity_link_codes(id, source_external_identity_id, code_hash, expires_at_ms, consumed_at_ms, consumed_identity_group_id, created_at_ms) VALUES(?, ?, ?, ?, NULL, NULL, ?)")
            .bind(&id)
            .bind(source_external_identity_id)
            .bind(digest_hex(code.as_bytes()))
            .bind(expires_at_ms)
            .bind(timestamp_ms)
            .execute(self.store.pool())
            .await?;
        Ok(ChannelIdentityLinkCode {
            id,
            code,
            account_id: request.account_id,
            actor_id: request.actor_id,
            expires_at_ms,
        })
    }

    pub async fn consume_identity_link_code(
        &self,
        message: &VerifiedChannelMessage,
        code: &str,
        session_id: &SessionId,
        timestamp_ms: i64,
    ) -> Result<ChannelIdentityGroup, GatewayError> {
        if message.address.chat_kind != ChannelChatKind::Dm {
            return Err(GatewayError::RouteNotAllowed);
        }
        let target_identity_id = self
            .remember_external_identity(message, timestamp_ms)
            .await?;
        let mut transaction = self.store.pool().begin().await?;
        let row = sqlx::query("SELECT code.id AS code_id, code.expires_at_ms, source.id AS source_identity_id, member.identity_group_id AS source_group_id FROM channel_identity_link_codes AS code INNER JOIN channel_external_identities AS source ON source.id = code.source_external_identity_id LEFT JOIN channel_identity_group_members AS member ON member.external_identity_id = source.id WHERE code.code_hash = ? AND code.consumed_at_ms IS NULL AND code.expires_at_ms > ?")
            .bind(digest_hex(normalize_code(code).as_bytes()))
            .bind(timestamp_ms)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(GatewayError::PairingRejected)?;
        let source_identity_id = row.get::<String, _>("source_identity_id");
        if source_identity_id == target_identity_id {
            return Err(GatewayError::IdentityOwnershipConflict);
        }
        let source_group_id = row.get::<Option<String>, _>("source_group_id");
        let target_group_id: Option<String> = sqlx::query_scalar("SELECT identity_group_id FROM channel_identity_group_members WHERE external_identity_id = ?")
            .bind(&target_identity_id)
            .fetch_optional(&mut *transaction)
            .await?;
        if target_group_id.is_some() && target_group_id != source_group_id {
            sqlx::query("INSERT OR IGNORE INTO channel_identity_transfer_requests(id, link_code_id, source_external_identity_id, target_external_identity_id, source_group_id, target_group_id, revision, status, expires_at_ms, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, 1, 'pending', ?, ?, ?)")
                .bind(Uuid::now_v7().to_string())
                .bind(row.get::<&str, _>("code_id"))
                .bind(&source_identity_id)
                .bind(&target_identity_id)
                .bind(&source_group_id)
                .bind(&target_group_id)
                .bind(row.get::<i64, _>("expires_at_ms"))
                .bind(timestamp_ms)
                .bind(timestamp_ms)
                .execute(&mut *transaction)
                .await?;
            transaction.commit().await?;
            return Err(GatewayError::IdentityOwnershipConflict);
        }
        if let Some(group_id) = source_group_id
            .as_ref()
            .filter(|group| Some(*group) == target_group_id.as_ref())
        {
            let consumed = sqlx::query("UPDATE channel_identity_link_codes SET consumed_at_ms = ?, consumed_identity_group_id = ? WHERE id = ? AND consumed_at_ms IS NULL")
                .bind(timestamp_ms)
                .bind(group_id)
                .bind(row.get::<&str, _>("code_id"))
                .execute(&mut *transaction)
                .await?;
            if consumed.rows_affected() != 1 {
                return Err(GatewayError::PairingRejected);
            }
            let existing = decode_identity_group(&mut transaction, group_id).await?;
            transaction.commit().await?;
            return Ok(existing);
        }
        let mut members = vec![source_identity_id, target_identity_id.clone()];
        if let Some(source_group_id) = source_group_id.as_deref() {
            members = sqlx::query_scalar("SELECT external_identity_id FROM channel_identity_group_members WHERE identity_group_id = ?")
                .bind(source_group_id)
                .fetch_all(&mut *transaction)
                .await?;
            if !members.iter().any(|member| member == &target_identity_id) {
                members.push(target_identity_id);
            }
            sqlx::query("DELETE FROM channel_identity_groups WHERE id = ?")
                .bind(source_group_id)
                .execute(&mut *transaction)
                .await?;
        }
        let group_id = Uuid::now_v7().to_string();
        sqlx::query("INSERT INTO channel_identity_groups(id, session_id, revision, created_at_ms, updated_at_ms) VALUES(?, ?, 1, ?, ?)")
            .bind(&group_id)
            .bind(session_id.as_str())
            .bind(timestamp_ms)
            .bind(timestamp_ms)
            .execute(&mut *transaction)
            .await?;
        for member in &members {
            sqlx::query("INSERT INTO channel_identity_group_members(identity_group_id, external_identity_id, created_at_ms) VALUES(?, ?, ?)")
                .bind(&group_id)
                .bind(member)
                .bind(timestamp_ms)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("UPDATE channel_external_identities SET identity_group_id = ?, updated_at_ms = ? WHERE id = ?")
                .bind(&group_id)
                .bind(timestamp_ms)
                .bind(member)
                .execute(&mut *transaction)
                .await?;
        }
        let consumed = sqlx::query("UPDATE channel_identity_link_codes SET consumed_at_ms = ?, consumed_identity_group_id = ? WHERE id = ? AND consumed_at_ms IS NULL")
            .bind(timestamp_ms)
            .bind(&group_id)
            .bind(row.get::<&str, _>("code_id"))
            .execute(&mut *transaction)
            .await?;
        if consumed.rows_affected() != 1 {
            return Err(GatewayError::PairingRejected);
        }
        transaction.commit().await?;
        Ok(ChannelIdentityGroup {
            id: group_id,
            session_id: session_id.clone(),
            member_count: u32::try_from(members.len()).unwrap_or(u32::MAX),
            revision: 1,
        })
    }

    pub async fn list_identity_transfer_previews(
        &self,
        account_id: &str,
        timestamp_ms: i64,
    ) -> Result<Vec<ChannelIdentityTransferPreview>, GatewayError> {
        if account_id.trim().is_empty() || account_id.len() > 128 {
            return Err(GatewayError::InvalidMessage);
        }
        let rows = sqlx::query("SELECT request.id, request.source_group_id, request.target_group_id, request.revision, request.expires_at_ms, source_group.revision AS source_group_revision, target_group.revision AS target_group_revision, source.id AS source_external_identity_id, source.provider_id AS source_provider_id, source.account_id AS source_account_id, source.tenant_key AS source_tenant_key, source.actor_id AS source_actor_id, source.display_name AS source_display_name, source.identity_group_id AS source_identity_group_id, target.id AS target_external_identity_id, target.provider_id AS target_provider_id, target.account_id AS target_account_id, target.tenant_key AS target_tenant_key, target.actor_id AS target_actor_id, target.display_name AS target_display_name, target.identity_group_id AS target_identity_group_id FROM channel_identity_transfer_requests AS request INNER JOIN channel_external_identities AS source ON source.id = request.source_external_identity_id INNER JOIN channel_external_identities AS target ON target.id = request.target_external_identity_id LEFT JOIN channel_identity_groups AS source_group ON source_group.id = request.source_group_id LEFT JOIN channel_identity_groups AS target_group ON target_group.id = request.target_group_id WHERE request.status = 'pending' AND request.expires_at_ms > ? AND (source.account_id = ? OR target.account_id = ?) ORDER BY request.created_at_ms, request.id")
            .bind(timestamp_ms)
            .bind(account_id)
            .bind(account_id)
            .fetch_all(self.store.pool())
            .await?;
        rows.into_iter()
            .map(decode_identity_transfer_preview)
            .collect()
    }

    pub async fn transfer_identity(
        &self,
        request: ChannelIdentityTransferCommitRequest,
        timestamp_ms: i64,
    ) -> Result<ChannelIdentityTransferResult, GatewayError> {
        if request.id.trim().is_empty() || request.id.len() > 128 || request.expected_revision == 0
        {
            return Err(GatewayError::InvalidMessage);
        }
        let session = SessionRecord {
            id: SessionId::random(),
            context: SessionContextBinding::General,
            entry_profile: EntryProfile::Workbench,
            title: "跨平台共享会话".into(),
            archived: false,
            pinned: false,
            parent_session_id: None,
            source_run_id: None,
            created_at_ms: timestamp_ms,
            updated_at_ms: timestamp_ms,
        };
        self.store.create_session(&session).await?;
        let transfer = self
            .transfer_identity_to_session(&request, &session.id, timestamp_ms)
            .await;
        if transfer.is_err() {
            let _ = self
                .store
                .update_session_metadata(&session.id, None, Some(true), None, timestamp_ms)
                .await;
        }
        transfer
    }

    async fn transfer_identity_to_session(
        &self,
        request: &ChannelIdentityTransferCommitRequest,
        session_id: &SessionId,
        timestamp_ms: i64,
    ) -> Result<ChannelIdentityTransferResult, GatewayError> {
        let mut transaction = self.store.pool().begin().await?;
        let row = sqlx::query("SELECT request.link_code_id, request.source_external_identity_id, request.target_external_identity_id, request.source_group_id, request.target_group_id, request.revision, source_group.revision AS source_group_revision, target_group.revision AS target_group_revision FROM channel_identity_transfer_requests AS request INNER JOIN channel_identity_link_codes AS code ON code.id = request.link_code_id LEFT JOIN channel_identity_groups AS source_group ON source_group.id = request.source_group_id LEFT JOIN channel_identity_groups AS target_group ON target_group.id = request.target_group_id WHERE request.id = ? AND request.status = 'pending' AND request.expires_at_ms > ? AND code.consumed_at_ms IS NULL")
            .bind(&request.id)
            .bind(timestamp_ms)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(GatewayError::IdentityOwnershipConflict)?;
        let source_group_id = row.get::<Option<String>, _>("source_group_id");
        let target_group_id = row.get::<Option<String>, _>("target_group_id");
        let source_revision = row
            .get::<Option<i64>, _>("source_group_revision")
            .map(from_i64);
        let target_revision = row
            .get::<Option<i64>, _>("target_group_revision")
            .map(from_i64);
        if from_i64(row.get("revision")) != request.expected_revision
            || source_revision != request.expected_source_group_revision
            || target_revision != request.expected_target_group_revision
        {
            return Err(GatewayError::IdentityOwnershipConflict);
        }
        let source_identity_id = row.get::<String, _>("source_external_identity_id");
        let target_identity_id = row.get::<String, _>("target_external_identity_id");
        let mut members = BTreeSet::from([source_identity_id.clone(), target_identity_id.clone()]);
        for group_id in source_group_id.iter().chain(target_group_id.iter()) {
            members.extend(
                sqlx::query_scalar::<_, String>("SELECT external_identity_id FROM channel_identity_group_members WHERE identity_group_id = ?")
                    .bind(group_id)
                    .fetch_all(&mut *transaction)
                    .await?,
            );
        }
        for group_id in source_group_id.iter().chain(target_group_id.iter()) {
            sqlx::query("DELETE FROM channel_identity_group_members WHERE identity_group_id = ?")
                .bind(group_id)
                .execute(&mut *transaction)
                .await?;
        }
        for group_id in source_group_id.iter().chain(target_group_id.iter()) {
            sqlx::query("DELETE FROM channel_identity_groups WHERE id = ?")
                .bind(group_id)
                .execute(&mut *transaction)
                .await?;
        }
        let group_id = Uuid::now_v7().to_string();
        sqlx::query("INSERT INTO channel_identity_groups(id, session_id, revision, created_at_ms, updated_at_ms) VALUES(?, ?, 1, ?, ?)")
            .bind(&group_id)
            .bind(session_id.as_str())
            .bind(timestamp_ms)
            .bind(timestamp_ms)
            .execute(&mut *transaction)
            .await?;
        for member in &members {
            sqlx::query("INSERT INTO channel_identity_group_members(identity_group_id, external_identity_id, created_at_ms) VALUES(?, ?, ?)")
                .bind(&group_id)
                .bind(member)
                .bind(timestamp_ms)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("UPDATE channel_external_identities SET identity_group_id = ?, updated_at_ms = ? WHERE id = ?")
                .bind(&group_id)
                .bind(timestamp_ms)
                .bind(member)
                .execute(&mut *transaction)
                .await?;
        }
        let consumed = sqlx::query("UPDATE channel_identity_link_codes SET consumed_at_ms = ?, consumed_identity_group_id = ? WHERE id = ? AND consumed_at_ms IS NULL")
            .bind(timestamp_ms)
            .bind(&group_id)
            .bind(row.get::<&str, _>("link_code_id"))
            .execute(&mut *transaction)
            .await?;
        let completed = sqlx::query("UPDATE channel_identity_transfer_requests SET status = 'completed', revision = revision + 1, updated_at_ms = ? WHERE id = ? AND status = 'pending' AND revision = ?")
            .bind(timestamp_ms)
            .bind(&request.id)
            .bind(to_i64(request.expected_revision))
            .execute(&mut *transaction)
            .await?;
        if consumed.rows_affected() != 1 || completed.rows_affected() != 1 {
            return Err(GatewayError::IdentityOwnershipConflict);
        }
        sqlx::query("INSERT INTO audit_events(principal, session_id, run_id, run_generation, operation, target_summary, decision, result_code, created_at_ms) VALUES('workbench', ?, NULL, NULL, 'channel.identity_transfer', ?, 'confirmed', 'identity_transferred', ?)")
            .bind(session_id.as_str())
            .bind(format!("{} members", members.len()))
            .bind(timestamp_ms)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        let identity_group = ChannelIdentityGroup {
            id: group_id,
            session_id: session_id.clone(),
            member_count: u32::try_from(members.len()).unwrap_or(u32::MAX),
            revision: 1,
        };
        Ok(ChannelIdentityTransferResult {
            identity_group,
            previous_source_group_id: source_group_id,
            previous_target_group_id: target_group_id,
            session_id: session_id.clone(),
        })
    }

    pub async fn consume_pairing_code(
        &self,
        message: &VerifiedChannelMessage,
        code: &str,
        timestamp_ms: i64,
    ) -> Result<PairingConsumeOutcome, GatewayError> {
        if self
            .pairing_cooldown_active(
                &message.address.account_id,
                &message.actor.external_id,
                timestamp_ms,
            )
            .await?
        {
            return Err(GatewayError::PairingRejected);
        }
        let row = sqlx::query("SELECT id, target, group_history_policy, topic_policy, mention_policy, grant_json FROM channel_pairing_codes WHERE account_id = ? AND code_hash = ? AND consumed_at_ms IS NULL AND expires_at_ms > ?")
            .bind(&message.address.account_id)
            .bind(digest_hex(normalize_code(code).as_bytes()))
            .bind(timestamp_ms)
            .fetch_optional(self.store.pool())
            .await?;
        let Some(row) = row else {
            self.record_pairing_failure(
                &message.address.account_id,
                &message.actor.external_id,
                timestamp_ms,
            )
            .await?;
            return Err(GatewayError::PairingRejected);
        };
        let target = parse_target(row.get("target"))?;
        if (target == ChannelAuthorizationTarget::DmIdentity
            && message.address.chat_kind != ChannelChatKind::Dm)
            || (target == ChannelAuthorizationTarget::GroupConversation
                && message.address.chat_kind != ChannelChatKind::Group)
        {
            self.record_pairing_failure(
                &message.address.account_id,
                &message.actor.external_id,
                timestamp_ms,
            )
            .await?;
            return Err(GatewayError::PairingRejected);
        }
        let authorization_id = Uuid::now_v7().to_string();
        let topic_policy = parse_topic_policy(row.get("topic_policy"))?;
        let mut address = message.address.clone();
        if target == ChannelAuthorizationTarget::GroupConversation
            && topic_policy == ChannelTopicPolicy::InheritGroup
        {
            address.topic_id = None;
        }
        let input = ChannelAuthorizationUpsert {
            id: authorization_id.clone(),
            account_id: message.address.account_id.clone(),
            target,
            address,
            actor_id: (target == ChannelAuthorizationTarget::DmIdentity)
                .then(|| message.actor.external_id.clone()),
            group_history_policy: row
                .get::<Option<&str>, _>("group_history_policy")
                .map(parse_group_policy)
                .transpose()?,
            topic_policy,
            mention_policy: parse_mention_policy(row.get("mention_policy"))?,
            grant: serde_json::from_str(row.get("grant_json"))?,
            enabled: true,
            expected_revision: None,
        };
        validate_provider_authorization_scope(&message.address.provider_id, &input)?;
        validate_grant(&input.grant)?;
        let ceiling = self.grant_ceiling(&input.account_id).await?;
        if !grant_is_subset(&input.grant, &ceiling) {
            return Err(GatewayError::AuthorizationConflict);
        }
        let mut transaction = self.store.pool().begin().await?;
        insert_authorization(&mut transaction, &input, "pairing", timestamp_ms).await?;
        let consumed = sqlx::query("UPDATE channel_pairing_codes SET consumed_at_ms = ?, consumed_authorization_id = ? WHERE id = ? AND consumed_at_ms IS NULL AND expires_at_ms > ?")
            .bind(timestamp_ms)
            .bind(&authorization_id)
            .bind(row.get::<&str, _>("id"))
            .bind(timestamp_ms)
            .execute(&mut *transaction)
            .await?;
        if consumed.rows_affected() != 1 {
            return Err(GatewayError::PairingRejected);
        }
        sqlx::query("DELETE FROM channel_pairing_attempts WHERE account_id = ? AND actor_id = ?")
            .bind(&message.address.account_id)
            .bind(&message.actor.external_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        let authorization = self
            .list_authorizations(&message.address.account_id)
            .await?
            .into_iter()
            .find(|authorization| authorization.id == authorization_id)
            .ok_or(GatewayError::AuthorizationConflict)?;
        Ok(PairingConsumeOutcome {
            authorization,
            consumed: true,
        })
    }

    pub async fn resolve_binding(
        &self,
        message: &VerifiedChannelMessage,
    ) -> Result<BindingResolution, GatewayError> {
        let authorization = self.authorize_message(message).await?;
        let (key_json, identity_group_id) =
            self.binding_key(message, authorization.as_ref()).await?;
        let hash = digest_hex(key_json.as_bytes());
        let row = sqlx::query("SELECT binding.session_id, session.archived FROM channel_session_bindings AS binding INNER JOIN sessions AS session ON session.id = binding.session_id WHERE binding.binding_key_hash = ?")
            .bind(&hash)
            .fetch_optional(self.store.pool())
            .await?;
        let session_id = match row {
            Some(row) if !row.get::<bool, _>("archived") => {
                Some(SessionId::new(row.get::<String, _>("session_id")))
            }
            Some(_) => {
                sqlx::query("DELETE FROM channel_session_bindings WHERE binding_key_hash = ?")
                    .bind(&hash)
                    .execute(self.store.pool())
                    .await?;
                None
            }
            None => None,
        };
        Ok(BindingResolution {
            binding_key_hash: hash,
            binding_key_json: key_json,
            identity_group_id,
            session_id,
            authorization,
        })
    }

    pub async fn bind_message(
        &self,
        message: &VerifiedChannelMessage,
        session_id: &SessionId,
        timestamp_ms: i64,
    ) -> Result<(), GatewayError> {
        let resolution = self.resolve_binding(message).await?;
        let active: Option<bool> =
            sqlx::query_scalar("SELECT archived = 0 FROM sessions WHERE id = ?")
                .bind(session_id.as_str())
                .fetch_optional(self.store.pool())
                .await?;
        if active != Some(true) {
            return Err(GatewayError::IngressConflict);
        }
        let (key_json, identity_group_id) = self
            .binding_key(message, resolution.authorization.as_ref())
            .await?;
        let authorization_revision = resolution
            .authorization
            .as_ref()
            .map_or(1, |value| value.revision);
        sqlx::query("INSERT INTO channel_session_bindings(binding_key_hash, binding_key_json, account_id, authorization_id, authorization_revision, identity_group_id, session_id, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(binding_key_hash) DO UPDATE SET authorization_id = excluded.authorization_id, authorization_revision = excluded.authorization_revision, identity_group_id = excluded.identity_group_id, session_id = excluded.session_id, updated_at_ms = excluded.updated_at_ms")
            .bind(&resolution.binding_key_hash)
            .bind(key_json)
            .bind(&message.address.account_id)
            .bind(resolution.authorization.as_ref().map(|value| value.id.as_str()))
            .bind(to_i64(authorization_revision))
            .bind(identity_group_id)
            .bind(session_id.as_str())
            .bind(timestamp_ms)
            .bind(timestamp_ms)
            .execute(self.store.pool())
            .await?;
        Ok(())
    }

    pub async fn reset_binding(
        &self,
        message: &VerifiedChannelMessage,
    ) -> Result<Option<SessionId>, GatewayError> {
        let resolution = self.resolve_binding(message).await?;
        sqlx::query("DELETE FROM channel_session_bindings WHERE binding_key_hash = ?")
            .bind(&resolution.binding_key_hash)
            .execute(self.store.pool())
            .await?;
        Ok(resolution.session_id)
    }

    async fn binding_key(
        &self,
        message: &VerifiedChannelMessage,
        authorization: Option<&ChannelAuthorization>,
    ) -> Result<(String, Option<String>), GatewayError> {
        let address = &message.address;
        if address.chat_kind == ChannelChatKind::Dm {
            let identity_group_id: Option<String> = sqlx::query_scalar("SELECT member.identity_group_id FROM channel_external_identities AS identity INNER JOIN channel_identity_group_members AS member ON member.external_identity_id = identity.id WHERE identity.account_id = ? AND identity.tenant_key = ? AND identity.actor_id = ?")
                .bind(&address.account_id)
                .bind(&address.tenant_key)
                .bind(&message.actor.external_id)
                .fetch_optional(self.store.pool())
                .await?;
            let key = if let Some(group_id) = identity_group_id.as_deref() {
                BindingKey {
                    scope: "identity_group_dm",
                    provider_id: None,
                    account_id: None,
                    tenant_key: None,
                    chat_id: None,
                    topic_id: None,
                    actor_id: None,
                    identity_group_id: Some(group_id),
                }
            } else {
                BindingKey {
                    scope: "provider_dm",
                    provider_id: Some(&address.provider_id),
                    account_id: Some(&address.account_id),
                    tenant_key: Some(&address.tenant_key),
                    chat_id: None,
                    topic_id: None,
                    actor_id: Some(&message.actor.external_id),
                    identity_group_id: None,
                }
            };
            return Ok((serde_json::to_string(&key)?, identity_group_id));
        }
        let authorization = authorization.ok_or(GatewayError::RouteNotAllowed)?;
        let isolate_topic = authorization.topic_policy == ChannelTopicPolicy::IsolateTopic;
        let per_sender =
            authorization.group_history_policy == Some(ChannelGroupHistoryPolicy::PerSender);
        let key = BindingKey {
            scope: if per_sender {
                "group_per_sender"
            } else {
                "group_shared"
            },
            provider_id: Some(&address.provider_id),
            account_id: Some(&address.account_id),
            tenant_key: Some(&address.tenant_key),
            chat_id: Some(&address.chat_id),
            topic_id: isolate_topic
                .then_some(address.topic_id.as_deref())
                .flatten(),
            actor_id: per_sender.then_some(message.actor.external_id.as_str()),
            identity_group_id: None,
        };
        Ok((serde_json::to_string(&key)?, None))
    }

    async fn pairing_cooldown_active(
        &self,
        account_id: &str,
        actor_id: &str,
        timestamp_ms: i64,
    ) -> Result<bool, GatewayError> {
        let cooldown: Option<i64> = sqlx::query_scalar("SELECT cooldown_until_ms FROM channel_pairing_attempts WHERE account_id = ? AND actor_id = ?")
            .bind(account_id)
            .bind(actor_id)
            .fetch_optional(self.store.pool())
            .await?
            .flatten();
        Ok(cooldown.is_some_and(|until| until > timestamp_ms))
    }

    async fn record_pairing_failure(
        &self,
        account_id: &str,
        actor_id: &str,
        timestamp_ms: i64,
    ) -> Result<(), GatewayError> {
        sqlx::query("INSERT INTO channel_pairing_attempts(account_id, actor_id, failure_count, cooldown_until_ms, updated_at_ms) VALUES(?, ?, 1, NULL, ?) ON CONFLICT(account_id, actor_id) DO UPDATE SET failure_count = CASE WHEN channel_pairing_attempts.cooldown_until_ms IS NOT NULL AND channel_pairing_attempts.cooldown_until_ms <= excluded.updated_at_ms THEN 1 WHEN channel_pairing_attempts.cooldown_until_ms IS NULL THEN channel_pairing_attempts.failure_count + 1 ELSE channel_pairing_attempts.failure_count END, cooldown_until_ms = CASE WHEN channel_pairing_attempts.cooldown_until_ms IS NOT NULL AND channel_pairing_attempts.cooldown_until_ms <= excluded.updated_at_ms THEN NULL WHEN channel_pairing_attempts.cooldown_until_ms IS NULL AND channel_pairing_attempts.failure_count + 1 >= ? THEN excluded.updated_at_ms + ? ELSE channel_pairing_attempts.cooldown_until_ms END, updated_at_ms = excluded.updated_at_ms")
            .bind(account_id)
            .bind(actor_id)
            .bind(timestamp_ms)
            .bind(PAIRING_FAILURE_LIMIT)
            .bind(PAIRING_COOLDOWN_MS)
            .execute(self.store.pool())
            .await?;
        Ok(())
    }
}

async fn insert_authorization(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    input: &ChannelAuthorizationUpsert,
    source: &str,
    timestamp_ms: i64,
) -> Result<(), GatewayError> {
    validate_authorization_input(input)?;
    sqlx::query("INSERT INTO channel_authorizations(id, account_id, provider_id, target, tenant_key, chat_kind, chat_id, topic_id, actor_id, group_history_policy, topic_policy, mention_policy, grant_json, source, enabled, revision, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?)")
        .bind(&input.id)
        .bind(&input.account_id)
        .bind(&input.address.provider_id)
        .bind(target_str(input.target))
        .bind(&input.address.tenant_key)
        .bind(chat_kind_str(input.address.chat_kind))
        .bind(&input.address.chat_id)
        .bind(&input.address.topic_id)
        .bind(&input.actor_id)
        .bind(input.group_history_policy.map(group_policy_str))
        .bind(topic_policy_str(input.topic_policy))
        .bind(mention_policy_str(input.mention_policy))
        .bind(serde_json::to_string(&input.grant)?)
        .bind(source)
        .bind(input.enabled)
        .bind(timestamp_ms)
        .bind(timestamp_ms)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

fn validate_authorization_input(input: &ChannelAuthorizationUpsert) -> Result<(), GatewayError> {
    if input.id.trim().is_empty()
        || input.account_id != input.address.account_id
        || input.address.chat_id.trim().is_empty()
        || (input.target == ChannelAuthorizationTarget::DmIdentity
            && (input.address.chat_kind != ChannelChatKind::Dm
                || input.actor_id.as_deref().is_none_or(str::is_empty)
                || input.group_history_policy.is_some()))
        || (input.target == ChannelAuthorizationTarget::GroupConversation
            && (input.address.chat_kind != ChannelChatKind::Group
                || input.actor_id.is_some()
                || input.group_history_policy.is_none()))
    {
        return Err(GatewayError::InvalidMessage);
    }
    validate_grant(&input.grant)
}

fn validate_provider_authorization_scope(
    provider_id: &str,
    input: &ChannelAuthorizationUpsert,
) -> Result<(), GatewayError> {
    if input.target == ChannelAuthorizationTarget::GroupConversation
        && !provider_supports_groups(provider_id)
    {
        return Err(GatewayError::InvalidMessage);
    }
    if (input.address.topic_id.is_some() || input.topic_policy == ChannelTopicPolicy::IsolateTopic)
        && (provider_id != "feishu"
            || input.target != ChannelAuthorizationTarget::GroupConversation
            || input.topic_policy != ChannelTopicPolicy::IsolateTopic
            || input.address.topic_id.as_deref().is_none_or(str::is_empty))
    {
        return Err(GatewayError::InvalidMessage);
    }
    Ok(())
}

fn validate_provider_pairing_scope(
    provider_id: &str,
    request: &ChannelPairingCodeRequest,
) -> Result<(), GatewayError> {
    if request.target == ChannelAuthorizationTarget::GroupConversation
        && !provider_supports_groups(provider_id)
    {
        return Err(GatewayError::InvalidMessage);
    }
    if request.topic_policy == ChannelTopicPolicy::IsolateTopic
        && (provider_id != "feishu"
            || request.target != ChannelAuthorizationTarget::GroupConversation)
    {
        return Err(GatewayError::InvalidMessage);
    }
    Ok(())
}

fn provider_supports_groups(provider_id: &str) -> bool {
    matches!(provider_id, "dingtalk" | "feishu" | "wecom_ai_bot")
}

fn validate_policy_input(input: &ChannelAccessPolicyUpsert) -> Result<(), GatewayError> {
    if input.account_id.trim().is_empty()
        || input.account_id.len() > 128
        || !valid_values(&input.allowlist_actor_ids, 512, 256)
    {
        return Err(GatewayError::InvalidMessage);
    }
    validate_grant(&input.grant_ceiling)
}

fn validate_grant(grant: &ChannelGrant) -> Result<(), GatewayError> {
    if !valid_values(&grant.skill_ids, 128, 256)
        || !valid_values(&grant.mcp_server_ids, 128, 256)
        || !valid_connector_selections(&grant.connector_selections)
        || !valid_values(&grant.read_only_workspace_roots, 64, 1_024)
        || !valid_values(&grant.network_hosts, 128, 253)
        || grant.network_hosts.iter().any(|host| {
            host.contains("://")
                || host.contains('/')
                || host.contains('\\')
                || host.chars().any(char::is_whitespace)
        })
    {
        return Err(GatewayError::InvalidMessage);
    }
    Ok(())
}

fn valid_values(values: &[String], max_items: usize, max_chars: usize) -> bool {
    if values.len() > max_items {
        return false;
    }
    let mut unique = BTreeSet::new();
    values.iter().all(|value| {
        let trimmed = value.trim();
        !trimmed.is_empty()
            && trimmed.chars().count() <= max_chars
            && trimmed == value
            && unique.insert(value)
    })
}

fn grant_is_subset(grant: &ChannelGrant, ceiling: &ChannelGrant) -> bool {
    is_subset(&grant.skill_ids, &ceiling.skill_ids)
        && is_subset(&grant.mcp_server_ids, &ceiling.mcp_server_ids)
        && grant.connector_selections.iter().all(|selection| {
            ceiling.connector_selections.iter().any(|allowed| {
                allowed.account_id == selection.account_id
                    && allowed.contribution_revision == selection.contribution_revision
                    && is_subset(&selection.allowed_actions, &allowed.allowed_actions)
            })
        })
        && is_subset(
            &grant.read_only_workspace_roots,
            &ceiling.read_only_workspace_roots,
        )
        && is_subset(&grant.network_hosts, &ceiling.network_hosts)
}

fn valid_connector_selections(selections: &[hachimi_protocol::ScheduleConnectorSelection]) -> bool {
    selections.len() <= 64
        && selections.iter().all(|selection| {
            !selection.account_id.as_str().trim().is_empty()
                && selection.account_id.as_str().len() <= 256
                && selection.contribution_revision.account_id.as_ref()
                    == Some(&selection.account_id)
                && selection
                    .contribution_revision
                    .host_identity_hash
                    .as_deref()
                    .is_some_and(valid_sha256)
                && selection
                    .contribution_revision
                    .schema_hash
                    .as_deref()
                    .is_some_and(valid_sha256)
                && selection
                    .contribution_revision
                    .action_hash
                    .as_deref()
                    .is_some_and(valid_sha256)
                && valid_sha256(&selection.contribution_revision.content_hash)
                && valid_values(&selection.allowed_actions, 128, 256)
                && !selection.allowed_actions.is_empty()
        })
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_subset(values: &[String], ceiling: &[String]) -> bool {
    values.iter().all(|value| ceiling.contains(value))
}

fn enforce_authorization(
    message: &VerifiedChannelMessage,
    authorization: &ChannelAuthorization,
) -> Result<(), GatewayError> {
    if message.address.chat_kind == ChannelChatKind::Group
        && (authorization.mention_policy == ChannelMentionPolicy::Disabled
            || (authorization.mention_policy == ChannelMentionPolicy::Required
                && !message.mentions.iter().any(|mention| {
                    matches!(
                        mention.kind,
                        ChannelMentionKind::Bot | ChannelMentionKind::All
                    )
                })))
    {
        return Err(GatewayError::RouteNotAllowed);
    }
    Ok(())
}

async fn decode_identity_group(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    group_id: &str,
) -> Result<ChannelIdentityGroup, GatewayError> {
    let row = sqlx::query("SELECT identity_group.id, identity_group.session_id, identity_group.revision, COUNT(member.external_identity_id) AS member_count FROM channel_identity_groups AS identity_group LEFT JOIN channel_identity_group_members AS member ON member.identity_group_id = identity_group.id WHERE identity_group.id = ? GROUP BY identity_group.id")
        .bind(group_id)
        .fetch_one(&mut **transaction)
        .await?;
    Ok(ChannelIdentityGroup {
        id: row.get("id"),
        session_id: SessionId::new(row.get::<String, _>("session_id")),
        member_count: u32::try_from(row.get::<i64, _>("member_count")).unwrap_or(u32::MAX),
        revision: from_i64(row.get("revision")),
    })
}

fn decode_identity_transfer_preview(
    row: sqlx::sqlite::SqliteRow,
) -> Result<ChannelIdentityTransferPreview, GatewayError> {
    let member = |prefix: &str| -> ChannelIdentityTransferMember {
        ChannelIdentityTransferMember {
            external_identity_id: row.get(format!("{prefix}_external_identity_id").as_str()),
            provider_id: row.get(format!("{prefix}_provider_id").as_str()),
            account_id: row.get(format!("{prefix}_account_id").as_str()),
            tenant_key: row.get(format!("{prefix}_tenant_key").as_str()),
            actor_id: row.get(format!("{prefix}_actor_id").as_str()),
            display_name: row.get(format!("{prefix}_display_name").as_str()),
            identity_group_id: row.get(format!("{prefix}_identity_group_id").as_str()),
        }
    };
    Ok(ChannelIdentityTransferPreview {
        id: row.get("id"),
        source: member("source"),
        target: member("target"),
        source_group_id: row.get("source_group_id"),
        target_group_id: row.get("target_group_id"),
        source_group_revision: row
            .get::<Option<i64>, _>("source_group_revision")
            .map(from_i64),
        target_group_revision: row
            .get::<Option<i64>, _>("target_group_revision")
            .map(from_i64),
        revision: from_i64(row.get("revision")),
        expires_at_ms: row.get("expires_at_ms"),
    })
}

fn decode_authorization(
    row: sqlx::sqlite::SqliteRow,
) -> Result<ChannelAuthorization, GatewayError> {
    let chat_kind = parse_chat_kind(row.get("chat_kind"))?;
    Ok(ChannelAuthorization {
        id: row.get("id"),
        account_id: row.get("account_id"),
        target: parse_target(row.get("target"))?,
        address: hachimi_protocol::ChannelConversationAddress {
            provider_id: row.get("provider_id"),
            account_id: row.get("account_id"),
            tenant_key: row.get("tenant_key"),
            chat_kind,
            chat_id: row.get("chat_id"),
            topic_id: row.get("topic_id"),
        },
        actor_id: row.get("actor_id"),
        group_history_policy: row
            .get::<Option<&str>, _>("group_history_policy")
            .map(parse_group_policy)
            .transpose()?,
        topic_policy: parse_topic_policy(row.get("topic_policy"))?,
        mention_policy: parse_mention_policy(row.get("mention_policy"))?,
        grant: serde_json::from_str(row.get("grant_json"))?,
        enabled: row.get("enabled"),
        revision: from_i64(row.get("revision")),
    })
}

fn target_str(value: ChannelAuthorizationTarget) -> &'static str {
    match value {
        ChannelAuthorizationTarget::DmIdentity => "dm_identity",
        ChannelAuthorizationTarget::GroupConversation => "group_conversation",
    }
}

fn chat_kind_str(value: ChannelChatKind) -> &'static str {
    match value {
        ChannelChatKind::Dm => "dm",
        ChannelChatKind::Group => "group",
    }
}

fn group_policy_str(value: ChannelGroupHistoryPolicy) -> &'static str {
    match value {
        ChannelGroupHistoryPolicy::Shared => "shared",
        ChannelGroupHistoryPolicy::PerSender => "per_sender",
    }
}

fn topic_policy_str(value: ChannelTopicPolicy) -> &'static str {
    match value {
        ChannelTopicPolicy::InheritGroup => "inherit_group",
        ChannelTopicPolicy::IsolateTopic => "isolate_topic",
    }
}

fn mention_policy_str(value: ChannelMentionPolicy) -> &'static str {
    match value {
        ChannelMentionPolicy::Required => "required",
        ChannelMentionPolicy::AllMessages => "all_messages",
        ChannelMentionPolicy::Disabled => "disabled",
    }
}

fn parse_target(value: &str) -> Result<ChannelAuthorizationTarget, GatewayError> {
    match value {
        "dm_identity" => Ok(ChannelAuthorizationTarget::DmIdentity),
        "group_conversation" => Ok(ChannelAuthorizationTarget::GroupConversation),
        _ => Err(GatewayError::AuthorizationConflict),
    }
}

fn parse_chat_kind(value: &str) -> Result<ChannelChatKind, GatewayError> {
    match value {
        "dm" => Ok(ChannelChatKind::Dm),
        "group" => Ok(ChannelChatKind::Group),
        _ => Err(GatewayError::AuthorizationConflict),
    }
}

fn parse_dm_policy(value: &str) -> Result<ChannelDmPolicy, GatewayError> {
    match value {
        "pairing" => Ok(ChannelDmPolicy::Pairing),
        "allowlist" => Ok(ChannelDmPolicy::Allowlist),
        "open" => Ok(ChannelDmPolicy::Open),
        "disabled" => Ok(ChannelDmPolicy::Disabled),
        _ => Err(GatewayError::AuthorizationConflict),
    }
}

fn dm_policy_str(value: ChannelDmPolicy) -> &'static str {
    match value {
        ChannelDmPolicy::Pairing => "pairing",
        ChannelDmPolicy::Allowlist => "allowlist",
        ChannelDmPolicy::Open => "open",
        ChannelDmPolicy::Disabled => "disabled",
    }
}

fn parse_group_policy(value: &str) -> Result<ChannelGroupHistoryPolicy, GatewayError> {
    match value {
        "shared" => Ok(ChannelGroupHistoryPolicy::Shared),
        "per_sender" => Ok(ChannelGroupHistoryPolicy::PerSender),
        _ => Err(GatewayError::AuthorizationConflict),
    }
}

fn parse_topic_policy(value: &str) -> Result<ChannelTopicPolicy, GatewayError> {
    match value {
        "inherit_group" => Ok(ChannelTopicPolicy::InheritGroup),
        "isolate_topic" => Ok(ChannelTopicPolicy::IsolateTopic),
        _ => Err(GatewayError::AuthorizationConflict),
    }
}

fn parse_mention_policy(value: &str) -> Result<ChannelMentionPolicy, GatewayError> {
    match value {
        "required" => Ok(ChannelMentionPolicy::Required),
        "all_messages" => Ok(ChannelMentionPolicy::AllMessages),
        "disabled" => Ok(ChannelMentionPolicy::Disabled),
        _ => Err(GatewayError::AuthorizationConflict),
    }
}

fn normalize_code(value: &str) -> String {
    value
        .chars()
        .filter(|character| !matches!(character, '-' | ' '))
        .flat_map(char::to_uppercase)
        .collect()
}

fn crockford_128(bytes: [u8; 16]) -> String {
    const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut value = u128::from_be_bytes(bytes);
    let mut output = [b'0'; 26];
    for character in output.iter_mut().rev() {
        *character = ALPHABET[(value & 31) as usize];
        value >>= 5;
    }
    String::from_utf8(output.to_vec()).expect("Crockford alphabet is ASCII")
}

fn digest_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hachimi_protocol::{
        ChannelActor, ChannelAuthorizationTarget, ChannelConversationAddress, ChannelEventKey,
        ChannelMentionPolicy, ChannelTopicPolicy, EntryProfile,
    };

    #[test]
    fn pairing_codes_are_128_bit_crockford_values() {
        let code = crockford_128([0xff; 16]);
        assert_eq!(code.len(), 26);
        assert!(
            code.bytes()
                .all(|byte| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&byte))
        );
    }

    fn group_message(actor_id: &str, topic_id: Option<&str>) -> VerifiedChannelMessage {
        VerifiedChannelMessage {
            event_key: ChannelEventKey {
                provider_id: "feishu".into(),
                account_id: "account-1".into(),
                external_message_id: format!("message-{actor_id}"),
            },
            address: ChannelConversationAddress {
                provider_id: "feishu".into(),
                account_id: "account-1".into(),
                tenant_key: "tenant-1".into(),
                chat_kind: ChannelChatKind::Group,
                chat_id: "chat-1".into(),
                topic_id: topic_id.map(str::to_owned),
            },
            actor: ChannelActor {
                external_id: actor_id.into(),
                display_name: None,
                is_bot: false,
            },
            parts: vec![ChannelMessagePart::Text {
                text: "hello".into(),
            }],
            mentions: Vec::new(),
            quote: None,
            received_at_ms: 1,
            provider_context: serde_json::Value::Null,
        }
    }

    fn group_authorization(
        history: ChannelGroupHistoryPolicy,
        topic: ChannelTopicPolicy,
    ) -> ChannelAuthorization {
        ChannelAuthorization {
            id: "authorization-1".into(),
            account_id: "account-1".into(),
            target: ChannelAuthorizationTarget::GroupConversation,
            address: group_message("user-1", None).address,
            actor_id: None,
            group_history_policy: Some(history),
            topic_policy: topic,
            mention_policy: ChannelMentionPolicy::AllMessages,
            grant: ChannelGrant::default(),
            enabled: true,
            revision: 1,
        }
    }

    #[tokio::test]
    async fn group_binding_keys_respect_shared_sender_and_topic_scopes() {
        let store = hachimi_storage::AgentStore::connect_in_memory()
            .await
            .expect("store");
        let gateway = GatewayHost::new(store, ["feishu".into()]);
        let shared = group_authorization(
            ChannelGroupHistoryPolicy::Shared,
            ChannelTopicPolicy::InheritGroup,
        );
        let first = gateway
            .binding_key(&group_message("user-1", Some("topic-1")), Some(&shared))
            .await
            .expect("first")
            .0;
        let second = gateway
            .binding_key(&group_message("user-2", Some("topic-2")), Some(&shared))
            .await
            .expect("second")
            .0;
        assert_eq!(first, second);

        let private = group_authorization(
            ChannelGroupHistoryPolicy::PerSender,
            ChannelTopicPolicy::InheritGroup,
        );
        let first = gateway
            .binding_key(&group_message("user-1", None), Some(&private))
            .await
            .expect("first")
            .0;
        let second = gateway
            .binding_key(&group_message("user-2", None), Some(&private))
            .await
            .expect("second")
            .0;
        assert_ne!(first, second);

        let isolated = group_authorization(
            ChannelGroupHistoryPolicy::Shared,
            ChannelTopicPolicy::IsolateTopic,
        );
        let first = gateway
            .binding_key(&group_message("user-1", Some("topic-1")), Some(&isolated))
            .await
            .expect("first")
            .0;
        let second = gateway
            .binding_key(&group_message("user-1", Some("topic-2")), Some(&isolated))
            .await
            .expect("second")
            .0;
        assert_ne!(first, second);
    }

    async fn seed_identity_account(
        store: &hachimi_storage::AgentStore,
        values: (&str, &str, &str, &str),
    ) {
        sqlx::query("INSERT INTO integration_provider_accounts(id, provider_id, display_name, tenant_key, tenant_identity_hash, transport, state, diagnostic, connector_account_id, credential_ref, credential_fingerprint, api_access_enabled, messaging_enabled, config_json, credential_revision, config_revision, last_event_at_ms, last_delivery_at_ms, next_reconnect_at_ms, consecutive_failures, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, 'stream', 'healthy', NULL, NULL, NULL, NULL, 0, 1, '{}', 1, 1, NULL, NULL, NULL, 0, 1, 1)")
            .bind(values.0)
            .bind(values.1)
            .bind(values.2)
            .bind(values.3)
            .bind(format!("hash-{}", values.0))
            .execute(store.pool())
            .await
            .expect("account");
    }

    fn identity_session(id: &str) -> SessionRecord {
        SessionRecord {
            id: SessionId::new(id),
            context: SessionContextBinding::General,
            entry_profile: EntryProfile::Workbench,
            title: id.into(),
            archived: false,
            pinned: false,
            parent_session_id: None,
            source_run_id: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    #[tokio::test]
    async fn explicit_identity_transfer_creates_blank_session_and_preserves_old_transcripts() {
        let store = hachimi_storage::AgentStore::connect_in_memory()
            .await
            .expect("store");
        seed_identity_account(&store, ("account-a", "dingtalk", "A", "tenant-a")).await;
        seed_identity_account(&store, ("account-b", "feishu", "B", "tenant-b")).await;
        let old_a = identity_session("old-session-a");
        let old_b = identity_session("old-session-b");
        store.create_session(&old_a).await.expect("old a");
        store.create_session(&old_b).await.expect("old b");
        for (identity, account, provider, tenant, actor, group) in [
            (
                "identity-a",
                "account-a",
                "dingtalk",
                "tenant-a",
                "user-a",
                "group-a",
            ),
            (
                "identity-b",
                "account-b",
                "feishu",
                "tenant-b",
                "user-b",
                "group-b",
            ),
        ] {
            sqlx::query("INSERT INTO channel_external_identities(id, account_id, provider_id, tenant_key, actor_id, display_name, identity_group_id, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, 1, 1)")
                .bind(identity)
                .bind(account)
                .bind(provider)
                .bind(tenant)
                .bind(actor)
                .bind(actor)
                .bind(group)
                .execute(store.pool())
                .await
                .expect("identity");
        }
        for (group, session, identity) in [
            ("group-a", old_a.id.as_str(), "identity-a"),
            ("group-b", old_b.id.as_str(), "identity-b"),
        ] {
            sqlx::query("INSERT INTO channel_identity_groups(id, session_id, revision, created_at_ms, updated_at_ms) VALUES(?, ?, 2, 1, 1)")
                .bind(group)
                .bind(session)
                .execute(store.pool())
                .await
                .expect("group");
            sqlx::query("INSERT INTO channel_identity_group_members(identity_group_id, external_identity_id, created_at_ms) VALUES(?, ?, 1)")
                .bind(group)
                .bind(identity)
                .execute(store.pool())
                .await
                .expect("member");
        }
        sqlx::query("INSERT INTO channel_identity_link_codes(id, source_external_identity_id, code_hash, expires_at_ms, consumed_at_ms, consumed_identity_group_id, created_at_ms) VALUES('link-1', 'identity-a', 'hash', 10000, NULL, NULL, 1)")
            .execute(store.pool())
            .await
            .expect("link");
        sqlx::query("INSERT INTO channel_identity_transfer_requests(id, link_code_id, source_external_identity_id, target_external_identity_id, source_group_id, target_group_id, revision, status, expires_at_ms, created_at_ms, updated_at_ms) VALUES('transfer-1', 'link-1', 'identity-a', 'identity-b', 'group-a', 'group-b', 1, 'pending', 10000, 1, 1)")
            .execute(store.pool())
            .await
            .expect("transfer");
        let gateway = GatewayHost::new(store.clone(), Vec::<String>::new());
        let previews = gateway
            .list_identity_transfer_previews("account-b", 2)
            .await
            .expect("previews");
        assert_eq!(previews.len(), 1);
        let result = gateway
            .transfer_identity(
                ChannelIdentityTransferCommitRequest {
                    id: previews[0].id.clone(),
                    expected_revision: previews[0].revision,
                    expected_source_group_revision: previews[0].source_group_revision,
                    expected_target_group_revision: previews[0].target_group_revision,
                },
                3,
            )
            .await
            .expect("transfer");
        assert_ne!(result.session_id, old_a.id);
        assert_ne!(result.session_id, old_b.id);
        assert_eq!(result.identity_group.member_count, 2);
        assert!(store.get_session(&old_a.id).await.expect("old a").is_some());
        assert!(store.get_session(&old_b.id).await.expect("old b").is_some());
        assert!(
            store
                .list_transcript(&result.session_id)
                .await
                .expect("new transcript")
                .is_empty()
        );
        let audit: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_events WHERE operation = 'channel.identity_transfer' AND session_id = ?")
            .bind(result.session_id.as_str())
            .fetch_one(store.pool())
            .await
            .expect("audit");
        assert_eq!(audit, 1);
    }
}
