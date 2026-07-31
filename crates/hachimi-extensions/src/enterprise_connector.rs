use std::sync::Arc;

use hachimi_enterprise::{
    EnterpriseApiClient, EnterpriseApiError, EnterpriseCredential, EnterpriseMessageTarget,
};
use hachimi_protocol::{
    ConnectorDriverDescriptor, ConnectorHealth, ConnectorInvocationRequest, ConnectorRevision,
    ConnectorRuntimeKind, EnterprisePlatform, PluginId,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::Row;
use zeroize::Zeroize;

use crate::{
    ConnectorDriver, ConnectorDriverContext, ConnectorDriverFuture, ExtensionHostError, now_ms,
};

const ENTERPRISE_ACTIONS: &[&str] = &[
    "account_identity",
    "department_list",
    "member_list",
    "message_send",
    "event_subscribe",
];

#[derive(Debug, Clone)]
pub struct EnterpriseConnectorDriver {
    platform: EnterprisePlatform,
    api: EnterpriseApiClient,
}

impl EnterpriseConnectorDriver {
    #[must_use]
    pub fn new(platform: EnterprisePlatform) -> Self {
        Self {
            platform,
            api: EnterpriseApiClient::default(),
        }
    }

    async fn execute(
        &self,
        mut context: ConnectorDriverContext,
        request: &ConnectorInvocationRequest,
    ) -> Result<Value, ExtensionHostError> {
        let mut raw_credential = context
            .credential
            .take()
            .ok_or(ExtensionHostError::ConnectorNotHealthy)?;
        let credential = EnterpriseCredential::parse(&raw_credential)
            .map_err(|_| ExtensionHostError::InvalidInvocation)?;
        raw_credential.zeroize();
        if credential.platform() != self.platform {
            return Err(ExtensionHostError::ConnectorDrift);
        }
        let integration_id = upsert_integration_account(&context, &credential).await?;
        let operation: Result<Value, EnterpriseApiError> = async {
            match request.action.as_str() {
                "account_identity" => {
                    self.api
                        .account_identity(context.account.id.as_str(), &credential)
                        .await
                }
                "department_list" => {
                    let page = self
                        .api
                        .departments(
                            context.account.id.as_str(),
                            &credential,
                            optional_string(&request.arguments, "parentId")?,
                            optional_string(&request.arguments, "pageToken")?,
                            optional_u32(&request.arguments, "pageSize")?,
                        )
                        .await?;
                    Ok(json!({
                        "items": page.items,
                        "nextPageToken": page.next_page_token,
                        "hasMore": page.has_more,
                    }))
                }
                "member_list" => {
                    let department_id = required_string(&request.arguments, "departmentId")?;
                    let page = self
                        .api
                        .members(
                            context.account.id.as_str(),
                            &credential,
                            department_id,
                            optional_string(&request.arguments, "pageToken")?,
                            optional_u32(&request.arguments, "pageSize")?,
                        )
                        .await?;
                    Ok(json!({
                        "items": page.items,
                        "nextPageToken": page.next_page_token,
                        "hasMore": page.has_more,
                    }))
                }
                "message_send" => {
                    self.send_message(&context, request, &credential, &integration_id)
                        .await
                }
                "event_subscribe" => Ok(json!({
                    "eventSourceId": format!(
                        "enterprise:{}:{}",
                        self.platform.as_str(),
                        context.account.id.as_str()
                    ),
                    "ingressMode": credential.ingress_mode(),
                    "state": "starting",
                })),
                _ => Err(EnterpriseApiError::InvalidRequest),
            }
        }
        .await;
        match operation {
            Ok(value) => {
                persist_success(&context, &integration_id).await?;
                Ok(value)
            }
            Err(error) => {
                persist_api_error(&context, &integration_id, &error).await?;
                Err(map_api_error(error))
            }
        }
    }

    async fn send_message(
        &self,
        context: &ConnectorDriverContext,
        request: &ConnectorInvocationRequest,
        credential: &EnterpriseCredential,
        integration_id: &str,
    ) -> Result<Value, EnterpriseApiError> {
        let target = EnterpriseMessageTarget {
            peer: required_string(&request.arguments, "peer")?.to_owned(),
            thread: optional_string(&request.arguments, "thread")?.map(str::to_owned),
            group: request
                .arguments
                .get("group")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        };
        let text = required_string(&request.arguments, "text")?;
        let input_hash = hash_value(&json!({
            "platform": self.platform,
            "target": {
                "peer": target.peer,
                "thread": target.thread,
                "group": target.group,
            },
            "text": text,
        }))
        .map_err(|_| EnterpriseApiError::InvalidRequest)?;
        match claim_operation(
            context,
            integration_id,
            &request.idempotency_key,
            "message_send",
            &input_hash,
        )
        .await
        .map_err(|_| EnterpriseApiError::Transport)?
        {
            OperationClaim::Completed(value) => return Ok(value),
            OperationClaim::Indeterminate => return Err(EnterpriseApiError::Indeterminate),
            OperationClaim::Execute => {}
        }
        match self
            .api
            .send_text(
                context.account.id.as_str(),
                credential,
                &target,
                text,
                &request.idempotency_key,
            )
            .await
        {
            Ok(value) => {
                complete_operation(context, integration_id, &request.idempotency_key, &value)
                    .await
                    .map_err(|_| EnterpriseApiError::Indeterminate)?;
                Ok(value)
            }
            Err(EnterpriseApiError::Indeterminate) => {
                mark_operation(
                    context,
                    integration_id,
                    &request.idempotency_key,
                    "indeterminate",
                    "enterprise_outcome_indeterminate",
                )
                .await
                .map_err(|_| EnterpriseApiError::Indeterminate)?;
                Err(EnterpriseApiError::Indeterminate)
            }
            Err(error) => {
                mark_operation(
                    context,
                    integration_id,
                    &request.idempotency_key,
                    "failed",
                    error.code(),
                )
                .await
                .map_err(|_| EnterpriseApiError::Transport)?;
                Err(error)
            }
        }
    }
}

impl ConnectorDriver for EnterpriseConnectorDriver {
    fn descriptor(
        &self,
        plugin_id: &PluginId,
        connector_id: &str,
        revision: ConnectorRevision,
    ) -> ConnectorDriverDescriptor {
        ConnectorDriverDescriptor {
            plugin_id: plugin_id.clone(),
            connector_id: connector_id.to_owned(),
            runtime_kind: ConnectorRuntimeKind::Builtin,
            revision,
            actions: ENTERPRISE_ACTIONS
                .iter()
                .map(|action| (*action).to_owned())
                .collect(),
        }
    }

    fn health<'a>(
        &'a self,
        context: &'a ConnectorDriverContext,
    ) -> ConnectorDriverFuture<'a, ConnectorHealth> {
        Box::pin(async move {
            let Some(raw) = context.credential.as_deref() else {
                return Ok(ConnectorHealth::Revoked);
            };
            let credential = EnterpriseCredential::parse(raw)
                .map_err(|_| ExtensionHostError::InvalidInvocation)?;
            Ok(if credential.platform() == self.platform {
                ConnectorHealth::Healthy
            } else {
                ConnectorHealth::HostIdentityDrift
            })
        })
    }

    fn invoke<'a>(
        &'a self,
        context: ConnectorDriverContext,
        request: &'a ConnectorInvocationRequest,
    ) -> ConnectorDriverFuture<'a, Value> {
        Box::pin(async move { self.execute(context, request).await })
    }

    fn revoke<'a>(&'a self, context: ConnectorDriverContext) -> ConnectorDriverFuture<'a, ()> {
        Box::pin(async move {
            self.api.revoke(context.account.id.as_str());
            sqlx::query("UPDATE enterprise_integration_accounts SET state = 'revoked', diagnostic = 'enterprise_credential_revoked', updated_at_ms = ? WHERE connector_account_id = ?")
                .bind(now_ms())
                .bind(context.account.id.as_str())
                .execute(context.store.pool())
                .await?;
            Ok(())
        })
    }
}

pub(crate) fn builtin_enterprise_drivers() -> Vec<(&'static str, Arc<dyn ConnectorDriver>)> {
    vec![
        (
            "hachimi.enterprise.wecom.v1",
            Arc::new(EnterpriseConnectorDriver::new(EnterprisePlatform::Wecom)),
        ),
        (
            "hachimi.enterprise.dingtalk.v1",
            Arc::new(EnterpriseConnectorDriver::new(EnterprisePlatform::DingTalk)),
        ),
        (
            "hachimi.enterprise.feishu.v1",
            Arc::new(EnterpriseConnectorDriver::new(EnterprisePlatform::Feishu)),
        ),
    ]
}

async fn upsert_integration_account(
    context: &ConnectorDriverContext,
    credential: &EnterpriseCredential,
) -> Result<String, ExtensionHostError> {
    let id = format!("connector:{}", context.account.id.as_str());
    let tenant_identity_hash = digest_hex(credential.tenant_id().as_bytes());
    let event_source_id = format!(
        "enterprise:{}:{}",
        credential.platform().as_str(),
        context.account.id.as_str()
    );
    let now = now_ms();
    sqlx::query("INSERT INTO enterprise_integration_accounts(id, platform, connector_account_id, channel_account_id, tenant_identity_hash, ingress_mode, event_source_id, state, diagnostic, credential_revision, source_account_updated_at_ms, created_at_ms, updated_at_ms) VALUES(?, ?, ?, NULL, ?, ?, ?, 'healthy', NULL, 1, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET platform = excluded.platform, connector_account_id = excluded.connector_account_id, tenant_identity_hash = excluded.tenant_identity_hash, ingress_mode = excluded.ingress_mode, event_source_id = excluded.event_source_id, state = CASE WHEN enterprise_integration_accounts.tenant_identity_hash = excluded.tenant_identity_hash THEN enterprise_integration_accounts.state ELSE 'needs_attention' END, diagnostic = CASE WHEN enterprise_integration_accounts.tenant_identity_hash = excluded.tenant_identity_hash THEN enterprise_integration_accounts.diagnostic ELSE 'enterprise_tenant_identity_changed' END, credential_revision = CASE WHEN enterprise_integration_accounts.source_account_updated_at_ms = excluded.source_account_updated_at_ms THEN enterprise_integration_accounts.credential_revision ELSE enterprise_integration_accounts.credential_revision + 1 END, source_account_updated_at_ms = excluded.source_account_updated_at_ms, updated_at_ms = excluded.updated_at_ms")
        .bind(&id)
        .bind(credential.platform().as_str())
        .bind(context.account.id.as_str())
        .bind(tenant_identity_hash)
        .bind(ingress_mode(credential.ingress_mode()))
        .bind(event_source_id)
        .bind(context.account.updated_at_ms)
        .bind(now)
        .bind(now)
        .execute(context.store.pool())
        .await?;
    Ok(id)
}

async fn persist_success(
    context: &ConnectorDriverContext,
    integration_id: &str,
) -> Result<(), ExtensionHostError> {
    let now = now_ms();
    sqlx::query("UPDATE enterprise_integration_accounts SET state = 'healthy', diagnostic = NULL, updated_at_ms = ? WHERE id = ?")
        .bind(now)
        .bind(integration_id)
        .execute(context.store.pool())
        .await?;
    sqlx::query("INSERT INTO enterprise_token_state(account_id, token_fingerprint, expires_at_ms, refresh_after_ms, last_result_code, updated_at_ms) VALUES(?, NULL, NULL, NULL, 'ok', ?) ON CONFLICT(account_id) DO UPDATE SET last_result_code = 'ok', updated_at_ms = excluded.updated_at_ms")
        .bind(integration_id)
        .bind(now)
        .execute(context.store.pool())
        .await?;
    sqlx::query("DELETE FROM enterprise_rate_limit_state WHERE account_id = ?")
        .bind(integration_id)
        .execute(context.store.pool())
        .await?;
    Ok(())
}

async fn persist_api_error(
    context: &ConnectorDriverContext,
    integration_id: &str,
    error: &EnterpriseApiError,
) -> Result<(), ExtensionHostError> {
    let now = now_ms();
    let state = match error {
        EnterpriseApiError::RateLimited { .. } => "rate_limited",
        EnterpriseApiError::Authentication | EnterpriseApiError::InvalidCredential => "revoked",
        EnterpriseApiError::Indeterminate => "needs_attention",
        EnterpriseApiError::InvalidRequest => "needs_attention",
        _ => "failed",
    };
    sqlx::query("UPDATE enterprise_integration_accounts SET state = ?, diagnostic = ?, updated_at_ms = ? WHERE id = ?")
        .bind(state)
        .bind(error.code())
        .bind(now)
        .bind(integration_id)
        .execute(context.store.pool())
        .await?;
    let retry_after_ms = match error {
        EnterpriseApiError::RateLimited { retry_after_ms } => *retry_after_ms,
        _ if error.retryable() => Some(now.saturating_add(1_000)),
        _ => None,
    };
    sqlx::query("INSERT INTO enterprise_rate_limit_state(account_id, attempt, retry_after_ms, last_error_code, updated_at_ms) VALUES(?, 1, ?, ?, ?) ON CONFLICT(account_id) DO UPDATE SET attempt = enterprise_rate_limit_state.attempt + 1, retry_after_ms = excluded.retry_after_ms, last_error_code = excluded.last_error_code, updated_at_ms = excluded.updated_at_ms")
        .bind(integration_id)
        .bind(retry_after_ms)
        .bind(error.code())
        .bind(now)
        .execute(context.store.pool())
        .await?;
    Ok(())
}

enum OperationClaim {
    Execute,
    Completed(Value),
    Indeterminate,
}

async fn claim_operation(
    context: &ConnectorDriverContext,
    integration_id: &str,
    idempotency_key: &str,
    operation: &str,
    input_hash: &str,
) -> Result<OperationClaim, ExtensionHostError> {
    let now = now_ms();
    let inserted = sqlx::query("INSERT OR IGNORE INTO enterprise_operation_ledger(account_id, idempotency_key, operation, input_hash, status, provider_request_id, provider_result_id, result_json, error_code, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, 'claimed', NULL, NULL, NULL, NULL, ?, ?)")
        .bind(integration_id)
        .bind(idempotency_key)
        .bind(operation)
        .bind(input_hash)
        .bind(now)
        .bind(now)
        .execute(context.store.pool())
        .await?;
    if inserted.rows_affected() == 1 {
        return Ok(OperationClaim::Execute);
    }
    let row = sqlx::query("SELECT input_hash, status, result_json FROM enterprise_operation_ledger WHERE account_id = ? AND idempotency_key = ?")
        .bind(integration_id)
        .bind(idempotency_key)
        .fetch_one(context.store.pool())
        .await?;
    if row.get::<String, _>("input_hash") != input_hash {
        return Err(ExtensionHostError::IdempotencyConflict);
    }
    match row.get::<String, _>("status").as_str() {
        "completed" => {
            let result_json = row.get::<String, _>("result_json");
            Ok(OperationClaim::Completed(serde_json::from_str(
                &result_json,
            )?))
        }
        "failed" => {
            sqlx::query("UPDATE enterprise_operation_ledger SET status = 'claimed', error_code = NULL, updated_at_ms = ? WHERE account_id = ? AND idempotency_key = ? AND status = 'failed'")
                .bind(now)
                .bind(integration_id)
                .bind(idempotency_key)
                .execute(context.store.pool())
                .await?;
            Ok(OperationClaim::Execute)
        }
        "claimed" | "indeterminate" => Ok(OperationClaim::Indeterminate),
        _ => Err(ExtensionHostError::EnterpriseTransport),
    }
}

async fn complete_operation(
    context: &ConnectorDriverContext,
    integration_id: &str,
    idempotency_key: &str,
    result: &Value,
) -> Result<(), ExtensionHostError> {
    let provider_result_id = result
        .pointer("/data/message_id")
        .or_else(|| result.get("msgid"))
        .or_else(|| result.get("processQueryKey"))
        .and_then(Value::as_str);
    sqlx::query("UPDATE enterprise_operation_ledger SET status = 'completed', provider_result_id = ?, result_json = ?, error_code = NULL, updated_at_ms = ? WHERE account_id = ? AND idempotency_key = ? AND status = 'claimed'")
        .bind(provider_result_id)
        .bind(serde_json::to_string(result)?)
        .bind(now_ms())
        .bind(integration_id)
        .bind(idempotency_key)
        .execute(context.store.pool())
        .await?;
    Ok(())
}

async fn mark_operation(
    context: &ConnectorDriverContext,
    integration_id: &str,
    idempotency_key: &str,
    status: &str,
    error_code: &str,
) -> Result<(), ExtensionHostError> {
    sqlx::query("UPDATE enterprise_operation_ledger SET status = ?, error_code = ?, updated_at_ms = ? WHERE account_id = ? AND idempotency_key = ? AND status = 'claimed'")
        .bind(status)
        .bind(error_code)
        .bind(now_ms())
        .bind(integration_id)
        .bind(idempotency_key)
        .execute(context.store.pool())
        .await?;
    Ok(())
}

fn required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, EnterpriseApiError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty() && value.len() <= 32_000)
        .ok_or(EnterpriseApiError::InvalidRequest)
}

fn optional_string<'a>(value: &'a Value, key: &str) -> Result<Option<&'a str>, EnterpriseApiError> {
    let value = value.get(key).and_then(Value::as_str);
    if value.is_some_and(|value| value.len() > 512) {
        return Err(EnterpriseApiError::InvalidRequest);
    }
    Ok(value.filter(|value| !value.is_empty()))
}

fn optional_u32(value: &Value, key: &str) -> Result<Option<u32>, EnterpriseApiError> {
    value
        .get(key)
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or(EnterpriseApiError::InvalidRequest)
        })
        .transpose()
}

fn map_api_error(error: EnterpriseApiError) -> ExtensionHostError {
    match error {
        EnterpriseApiError::InvalidCredential | EnterpriseApiError::Authentication => {
            ExtensionHostError::ConnectorNotHealthy
        }
        EnterpriseApiError::InvalidRequest | EnterpriseApiError::MalformedResponse => {
            ExtensionHostError::InvalidInvocation
        }
        EnterpriseApiError::RateLimited { .. } => ExtensionHostError::RateLimited,
        EnterpriseApiError::Provider { code, .. } => ExtensionHostError::EnterpriseProvider(code),
        EnterpriseApiError::Transport => ExtensionHostError::EnterpriseTransport,
        EnterpriseApiError::Indeterminate => ExtensionHostError::EnterpriseIndeterminate,
    }
}

fn ingress_mode(mode: hachimi_protocol::EnterpriseIngressMode) -> &'static str {
    match mode {
        hachimi_protocol::EnterpriseIngressMode::EncryptedCallback => "encrypted_callback",
        hachimi_protocol::EnterpriseIngressMode::Stream => "stream",
        hachimi_protocol::EnterpriseIngressMode::LongConnection => "long_connection",
    }
}

fn hash_value(value: &Value) -> Result<String, serde_json::Error> {
    serde_json::to_vec(value).map(|bytes| digest_hex(&bytes))
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

    #[test]
    fn builtin_driver_identities_are_fixed_and_complete() {
        let identities = builtin_enterprise_drivers()
            .into_iter()
            .map(|(identity, _)| identity)
            .collect::<Vec<_>>();
        assert_eq!(
            identities,
            vec![
                "hachimi.enterprise.wecom.v1",
                "hachimi.enterprise.dingtalk.v1",
                "hachimi.enterprise.feishu.v1",
            ]
        );
    }
}
