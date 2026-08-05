use hachimi_enterprise::{EnterpriseApiClient, EnterpriseCredential};
use hachimi_protocol::{
    ChannelAccessPolicy, ChannelAccessPolicyUpsert, ChannelAccountState, ChannelAuthorization,
    ChannelAuthorizationUpsert, ChannelIdentityLinkCode, ChannelIdentityLinkCodeRequest,
    ChannelIdentityTransferCommitRequest, ChannelIdentityTransferPreview,
    ChannelIdentityTransferResult, ChannelPairingCode, ChannelPairingCodeRequest,
    ChannelProviderAccount, CredentialFieldDefinition, CredentialFieldKind, IlinkQrLoginRequest,
    IlinkQrSession, IntegrationAccountCapabilitiesUpdate, IntegrationAccountProbeResult,
    IntegrationAccountProbeSnapshot, IntegrationAccountUpsert, IntegrationAuthMethod,
    IntegrationCapability, IntegrationCredentialInput, IntegrationProbeDimension,
    IntegrationProviderAccount, IntegrationProviderDefinition, IntegrationProviderId,
    IntegrationTransport,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use tauri::{Manager, State, WebviewWindow};
use uuid::Uuid;
use zeroize::Zeroize;

use crate::{CommandError, DesktopState, require_window};

mod runtime_health;
mod upsert_target;
mod validation;

use self::runtime_health::{persisted_provider_health, wait_for_gateway_ready};
use self::upsert_target::{
    integration_account_store_error, resolve_integration_upsert_target, revision_conflict,
};
use self::validation::{credential_shape_valid, validate_capabilities, validate_upsert};

const KEYRING_SERVICE: &str = "com.hachimi.integration";
const EMPTY_CHANNEL_GRANT_JSON: &str = r#"{"skillIds":[],"mcpServerIds":[],"connectorSelections":[],"readOnlyWorkspaceRoots":[],"networkHosts":[]}"#;

fn probe_dimension(ok: bool, success_code: &str, failure_code: &str) -> IntegrationProbeDimension {
    IntegrationProbeDimension {
        ok,
        result_code: if ok { success_code } else { failure_code }.into(),
        diagnostic: (!ok).then(|| failure_code.into()),
    }
}

fn ingress_probe_dimension(
    messaging_enabled: bool,
    transport_ok: bool,
    reverse_proxy_ok: bool,
) -> IntegrationProbeDimension {
    if !messaging_enabled {
        return probe_dimension(true, "ingress_disabled", "ingress_disabled");
    }
    if !reverse_proxy_ok {
        return probe_dimension(false, "ingress_healthy", "wecom_callback_proxy_unreachable");
    }
    probe_dimension(
        transport_ok,
        "ingress_healthy",
        "ingress_transport_unavailable",
    )
}

fn egress_probe_dimension(
    messaging_enabled: bool,
    transport_ok: bool,
    credential_ok: bool,
    provider_send_ready: bool,
) -> IntegrationProbeDimension {
    if !messaging_enabled {
        return probe_dimension(true, "egress_disabled", "egress_disabled");
    }
    if !provider_send_ready {
        return probe_dimension(false, "egress_ready", "dingtalk_robot_code_missing");
    }
    if !credential_ok {
        return probe_dimension(false, "egress_ready", "egress_authentication_failed");
    }
    probe_dimension(transport_ok, "egress_ready", "egress_transport_unavailable")
}

fn api_probe_dimension(api_access_enabled: bool, credential_ok: bool) -> IntegrationProbeDimension {
    if !api_access_enabled {
        return probe_dimension(true, "api_disabled", "api_disabled");
    }
    probe_dimension(
        credential_ok,
        "api_authenticated",
        "api_authentication_failed",
    )
}

pub(super) async fn reconcile_integration_startup(
    store: &hachimi_storage::AgentStore,
    gateway: &hachimi_gateway::GatewayHost,
) -> Result<(), CommandError> {
    reconcile_lifecycle_journals(store).await?;
    let rows = sqlx::query("SELECT id, provider_id, display_name, tenant_key, credential_ref, messaging_enabled, state, config_json, credential_revision, config_revision FROM integration_provider_accounts WHERE state IN ('starting', 'healthy', 'degraded') ORDER BY provider_id, id")
        .fetch_all(store.pool())
        .await
        .map_err(|error| CommandError::operation("integration_reconcile_load_failed", error))?;
    for row in rows {
        let id: String = row.get("id");
        let provider_id = parse_provider_id(row.get("provider_id"))?;
        let expected_ref = credential_reference(provider_id, &id);
        let credential_present = row.get::<Option<&str>, _>("credential_ref")
            == Some(&expected_ref)
            && integration_keyring_entry(provider_id, &id)
                .and_then(|entry| entry.get_password())
                .is_ok();
        if !credential_present {
            sqlx::query("UPDATE integration_provider_accounts SET state = 'needs_attention', diagnostic = 'credential_missing', updated_at_ms = ? WHERE id = ?")
                .bind(now_ms())
                .bind(&id)
                .execute(store.pool())
                .await
                .map_err(|error| CommandError::operation("integration_reconcile_store_failed", error))?;
            sqlx::query("UPDATE integration_lifecycle_journal SET phase = 'credential_check', status = 'failed', error_code = 'credential_missing', updated_at_ms = ? WHERE account_id = ? AND operation = 'upsert' AND status = 'in_progress'")
                .bind(now_ms())
                .bind(&id)
                .execute(store.pool())
                .await
                .map_err(|error| CommandError::operation("integration_journal_reconcile_failed", error))?;
            continue;
        }
        let account = ChannelProviderAccount {
            id: id.clone(),
            provider_id: provider_id.as_str().into(),
            display_name: row.get("display_name"),
            tenant_key: row.get("tenant_key"),
            credential_ref: Some(expected_ref),
            enabled: row.get("messaging_enabled"),
            state: parse_account_state(row.get("state"))?,
            config: serde_json::from_str(row.get("config_json"))
                .map_err(|error| CommandError::operation("integration_config_invalid", error))?,
            credential_revision: from_i64(row.get("credential_revision")),
            config_revision: from_i64(row.get("config_revision")),
        };
        if let Err(error) = gateway
            .bootstrap_provider_accounts(std::slice::from_ref(&account))
            .await
        {
            sqlx::query("UPDATE integration_provider_accounts SET state = 'needs_attention', diagnostic = 'runtime_restore_failed', updated_at_ms = ? WHERE id = ?")
                .bind(now_ms())
                .bind(&id)
                .execute(store.pool())
                .await
                .map_err(|store_error| CommandError::operation("integration_reconcile_store_failed", store_error))?;
            tracing::warn!(account_id = %id, %error, "Integration runtime restore failed");
            sqlx::query("UPDATE integration_lifecycle_journal SET phase = 'runtime', status = 'failed', error_code = 'runtime_restore_failed', updated_at_ms = ? WHERE account_id = ? AND operation = 'upsert' AND status = 'in_progress'")
                .bind(now_ms())
                .bind(&id)
                .execute(store.pool())
                .await
                .map_err(|store_error| CommandError::operation("integration_journal_reconcile_failed", store_error))?;
        } else if account.state == ChannelAccountState::Starting {
            sqlx::query("UPDATE integration_provider_accounts SET state = 'healthy', diagnostic = NULL, updated_at_ms = ? WHERE id = ? AND state = 'starting'")
                .bind(now_ms())
                .bind(&id)
                .execute(store.pool())
                .await
                .map_err(|store_error| CommandError::operation("integration_reconcile_store_failed", store_error))?;
        }
    }
    queue_orphaned_media_secret_cleanup(store).await?;
    reconcile_secret_cleanup(store).await?;
    sqlx::query("UPDATE integration_lifecycle_journal SET status = 'committed', phase = 'reconciled', error_code = NULL, updated_at_ms = ? WHERE operation = 'upsert' AND status = 'in_progress' AND EXISTS(SELECT 1 FROM integration_provider_accounts AS account WHERE account.id = integration_lifecycle_journal.account_id AND account.state IN ('healthy', 'degraded'))")
        .bind(now_ms())
        .execute(store.pool())
        .await
        .map_err(|error| CommandError::operation("integration_journal_reconcile_failed", error))?;
    sqlx::query("UPDATE integration_lifecycle_journal SET status = 'committed', phase = 'reconciled', error_code = NULL, updated_at_ms = ? WHERE status = 'deferred_cleanup' AND NOT EXISTS(SELECT 1 FROM integration_secret_cleanup_queue AS cleanup WHERE cleanup.account_id = integration_lifecycle_journal.account_id)")
        .bind(now_ms())
        .execute(store.pool())
        .await
        .map_err(|error| CommandError::operation("integration_journal_reconcile_failed", error))?;
    Ok(())
}

async fn reconcile_lifecycle_journals(
    store: &hachimi_storage::AgentStore,
) -> Result<(), CommandError> {
    let rows = sqlx::query("SELECT id, account_id, operation, phase, credential_ref, updated_at_ms FROM integration_lifecycle_journal WHERE status = 'in_progress' ORDER BY created_at_ms, id")
        .fetch_all(store.pool())
        .await
        .map_err(|error| CommandError::operation("integration_journal_reconcile_failed", error))?;
    for row in rows {
        let journal_id: String = row.get("id");
        let account_id: String = row.get("account_id");
        let operation: &str = row.get("operation");
        let phase: &str = row.get("phase");
        let credential_ref: Option<&str> = row.get("credential_ref");
        let journal_updated_at_ms: i64 = row.get("updated_at_ms");
        let account_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM integration_provider_accounts WHERE id = ?)",
        )
        .bind(&account_id)
        .fetch_one(store.pool())
        .await
        .map_err(|error| CommandError::operation("integration_journal_reconcile_failed", error))?;
        match operation {
            "upsert" if !account_exists => {
                if let Some(secret_ref) = credential_ref {
                    queue_secret_cleanup_for_store(store, &account_id, secret_ref, now_ms())
                        .await?;
                }
                sqlx::query("UPDATE integration_lifecycle_journal SET phase = 'credential_cleanup', status = 'deferred_cleanup', error_code = 'upsert_account_missing', updated_at_ms = ? WHERE id = ?")
                    .bind(now_ms())
                    .bind(&journal_id)
                    .execute(store.pool())
                    .await
                    .map_err(|error| CommandError::operation("integration_journal_reconcile_failed", error))?;
            }
            "upsert" if matches!(phase, "credential" | "account") => {
                let (account_updated_at_ms, account_state): (i64, String) = sqlx::query_as(
                    "SELECT updated_at_ms, state FROM integration_provider_accounts WHERE id = ?",
                )
                .bind(&account_id)
                .fetch_one(store.pool())
                .await
                .map_err(|error| {
                    CommandError::operation("integration_journal_reconcile_failed", error)
                })?;
                if phase == "credential"
                    || account_state != "starting"
                    || account_updated_at_ms < journal_updated_at_ms
                {
                    sqlx::query("UPDATE integration_provider_accounts SET state = 'needs_attention', diagnostic = 'integration_update_interrupted', updated_at_ms = ? WHERE id = ?")
                        .bind(now_ms())
                        .bind(&account_id)
                        .execute(store.pool())
                        .await
                        .map_err(|error| CommandError::operation("integration_journal_reconcile_failed", error))?;
                    sqlx::query("UPDATE integration_lifecycle_journal SET phase = 'account_check', status = 'failed', error_code = 'integration_update_interrupted', updated_at_ms = ? WHERE id = ?")
                        .bind(now_ms())
                        .bind(&journal_id)
                        .execute(store.pool())
                        .await
                        .map_err(|error| CommandError::operation("integration_journal_reconcile_failed", error))?;
                }
            }
            "remove" if account_exists => {
                stage_account_removal(store.pool(), &account_id, credential_ref).await?;
                sqlx::query("UPDATE integration_lifecycle_journal SET phase = 'credential_cleanup', status = 'deferred_cleanup', error_code = 'remove_resumed', updated_at_ms = ? WHERE id = ?")
                    .bind(now_ms())
                    .bind(&journal_id)
                    .execute(store.pool())
                    .await
                    .map_err(|error| CommandError::operation("integration_journal_reconcile_failed", error))?;
            }
            "remove" => {
                if let Some(secret_ref) = credential_ref {
                    queue_secret_cleanup_for_store(store, &account_id, secret_ref, now_ms())
                        .await?;
                }
                sqlx::query("UPDATE integration_lifecycle_journal SET phase = 'credential_cleanup', status = 'deferred_cleanup', error_code = ?, updated_at_ms = ? WHERE id = ?")
                    .bind(format!("remove_resumed_after_{phase}"))
                    .bind(now_ms())
                    .bind(&journal_id)
                    .execute(store.pool())
                    .await
                    .map_err(|error| CommandError::operation("integration_journal_reconcile_failed", error))?;
            }
            _ => {}
        }
    }
    Ok(())
}

async fn queue_orphaned_media_secret_cleanup(
    store: &hachimi_storage::AgentStore,
) -> Result<(), CommandError> {
    let timestamp_ms = now_ms();
    sqlx::query("INSERT OR IGNORE INTO integration_secret_cleanup_queue(secret_ref, account_id, attempt, next_attempt_at_ms, error_code, created_at_ms, updated_at_ms) SELECT secret.secret_ref, secret.account_id, 0, ?, 'orphaned_media_secret', ?, ? FROM channel_media_secrets AS secret LEFT JOIN channel_attachment_metadata AS metadata ON metadata.platform = secret.platform AND metadata.account_id = secret.account_id AND metadata.event_id = secret.event_id AND metadata.remote_id = secret.remote_id WHERE metadata.remote_id IS NULL OR metadata.download_status = 'completed'")
        .bind(timestamp_ms)
        .bind(timestamp_ms)
        .bind(timestamp_ms)
        .execute(store.pool())
        .await
        .map_err(|error| CommandError::operation("integration_media_cleanup_queue_failed", error))?;
    Ok(())
}

async fn reconcile_secret_cleanup(store: &hachimi_storage::AgentStore) -> Result<(), CommandError> {
    let rows = sqlx::query("SELECT secret_ref, account_id, attempt FROM integration_secret_cleanup_queue WHERE next_attempt_at_ms <= ? ORDER BY next_attempt_at_ms LIMIT 32")
        .bind(now_ms())
        .fetch_all(store.pool())
        .await
        .map_err(|error| CommandError::operation("integration_cleanup_load_failed", error))?;
    for row in rows {
        let secret_ref: String = row.get("secret_ref");
        let account_id: String = row.get("account_id");
        let username = cleanup_keyring_username(&secret_ref, &account_id)?;
        let deleted = username.is_some_and(|username| {
            keyring::Entry::new(KEYRING_SERVICE, username)
                .and_then(|entry| entry.delete_credential())
                .is_ok_and(|()| true)
                || keyring::Entry::new(KEYRING_SERVICE, username)
                    .and_then(|entry| entry.get_password())
                    .is_err_and(|error| matches!(error, keyring::Error::NoEntry))
        });
        if deleted {
            sqlx::query("DELETE FROM channel_media_secrets WHERE secret_ref = ?")
                .bind(&secret_ref)
                .execute(store.pool())
                .await
                .map_err(|error| {
                    CommandError::operation("integration_cleanup_store_failed", error)
                })?;
            sqlx::query("DELETE FROM integration_secret_cleanup_queue WHERE secret_ref = ?")
                .bind(&secret_ref)
                .execute(store.pool())
                .await
                .map_err(|error| {
                    CommandError::operation("integration_cleanup_store_failed", error)
                })?;
        } else {
            let attempt = row.get::<i64, _>("attempt").saturating_add(1);
            sqlx::query("UPDATE integration_secret_cleanup_queue SET attempt = ?, next_attempt_at_ms = ?, error_code = 'delete_failed', updated_at_ms = ? WHERE secret_ref = ?")
                .bind(attempt)
                .bind(now_ms().saturating_add((attempt.min(8) + 1) * 60_000))
                .bind(now_ms())
                .bind(&secret_ref)
                .execute(store.pool())
                .await
                .map_err(|error| CommandError::operation("integration_cleanup_store_failed", error))?;
        }
    }
    Ok(())
}

fn cleanup_keyring_username<'a>(
    secret_ref: &'a str,
    account_id: &str,
) -> Result<Option<&'a str>, CommandError> {
    let Some(username) = secret_ref.strip_prefix("keyring:integration:") else {
        return Ok(None);
    };
    for provider in [
        IntegrationProviderId::DingTalk,
        IntegrationProviderId::Feishu,
        IntegrationProviderId::WecomAiBot,
        IntegrationProviderId::WecomApp,
        IntegrationProviderId::WechatIlink,
    ] {
        let account_prefix = format!("{}:{}:", provider.as_str(), account_id);
        let Some(kind) = username.strip_prefix(&account_prefix) else {
            continue;
        };
        let valid = kind == "primary"
            || kind.strip_prefix("conversation:").is_some_and(|digest| {
                digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            || kind.strip_prefix("media:").is_some_and(|digest| {
                digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            });
        return Ok(valid.then_some(username));
    }
    Ok(None)
}

#[tauri::command]
pub(super) async fn list_integration_providers(
    window: WebviewWindow,
) -> Result<Vec<IntegrationProviderDefinition>, CommandError> {
    require_window(&window, "workbench")?;
    Ok(provider_definitions())
}

#[tauri::command]
pub(super) async fn list_enterprise_integrations(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Vec<IntegrationProviderAccount>, CommandError> {
    require_window(&window, "workbench")?;
    load_accounts(&state).await
}

#[tauri::command]
pub(super) async fn begin_ilink_qr_login(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: IlinkQrLoginRequest,
) -> Result<IlinkQrSession, CommandError> {
    require_window(&window, "workbench")?;
    if request.account_id.trim().is_empty()
        || request.account_id.len() > 128
        || request.display_name.trim().is_empty()
        || request.display_name.chars().count() > 200
    {
        return Err(CommandError::new(
            "ilink_qr_request_invalid",
            "The iLink account request is invalid.",
        ));
    }
    let existing_provider: Option<String> =
        sqlx::query_scalar("SELECT provider_id FROM integration_provider_accounts WHERE id = ?")
            .bind(&request.account_id)
            .fetch_optional(state.agent_store.pool())
            .await
            .map_err(|error| CommandError::operation("ilink_account_load_failed", error))?;
    if existing_provider
        .as_deref()
        .is_some_and(|provider| provider != "wechat_ilink")
    {
        return Err(CommandError::new(
            "integration_account_provider_conflict",
            "The account ID belongs to another provider.",
        ));
    }
    let qr = hachimi_channel_providers::WechatIlinkClient::default()
        .fetch_qr_code()
        .await
        .map_err(|error| CommandError::operation("ilink_qr_fetch_failed", error))?;
    let timestamp_ms = now_ms();
    let expires_at_ms = timestamp_ms.saturating_add(120_000);
    let pending_tenant = format!("pending:{}", request.account_id);
    sqlx::query("INSERT INTO integration_provider_accounts(id, provider_id, display_name, tenant_key, tenant_identity_hash, transport, state, diagnostic, connector_account_id, credential_ref, credential_fingerprint, api_access_enabled, messaging_enabled, config_json, credential_revision, config_revision, last_event_at_ms, last_delivery_at_ms, next_reconnect_at_ms, consecutive_failures, created_at_ms, updated_at_ms) VALUES(?, 'wechat_ilink', ?, ?, ?, 'qr_long_poll', 'awaiting_auth', NULL, NULL, NULL, NULL, 0, 1, ?, 1, 1, NULL, NULL, NULL, 0, ?, ?) ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name, state = 'awaiting_auth', diagnostic = NULL, messaging_enabled = 1, config_json = excluded.config_json, config_revision = integration_provider_accounts.config_revision + 1, updated_at_ms = excluded.updated_at_ms")
        .bind(&request.account_id)
        .bind(request.display_name.trim())
        .bind(&pending_tenant)
        .bind(digest_hex(pending_tenant.as_bytes()))
        .bind(json!({"baseUrl": hachimi_channel_providers::ILINK_ORIGIN}).to_string())
        .bind(timestamp_ms)
        .bind(timestamp_ms)
        .execute(state.agent_store.pool())
        .await
        .map_err(|error| CommandError::operation("ilink_account_store_failed", error))?;
    sqlx::query("INSERT INTO channel_access_policies(account_id, dm_policy, allowlist_actor_ids_json, grant_ceiling_json, revision, updated_at_ms) VALUES(?, 'pairing', '[]', ?, 1, ?) ON CONFLICT(account_id) DO NOTHING")
        .bind(&request.account_id)
        .bind(EMPTY_CHANNEL_GRANT_JSON)
        .bind(timestamp_ms)
        .execute(state.agent_store.pool())
        .await
        .map_err(|error| CommandError::operation("ilink_policy_store_failed", error))?;
    let qr_content = normalize_qr_content(&qr.image_content);
    sqlx::query("INSERT INTO integration_ilink_qr_sessions(account_id, qrcode, qr_content, state, expires_at_ms, created_at_ms, updated_at_ms) VALUES(?, ?, ?, 'waiting', ?, ?, ?) ON CONFLICT(account_id) DO UPDATE SET qrcode = excluded.qrcode, qr_content = excluded.qr_content, state = 'waiting', expires_at_ms = excluded.expires_at_ms, updated_at_ms = excluded.updated_at_ms")
        .bind(&request.account_id)
        .bind(qr.qrcode)
        .bind(&qr_content)
        .bind(expires_at_ms)
        .bind(timestamp_ms)
        .bind(timestamp_ms)
        .execute(state.agent_store.pool())
        .await
        .map_err(|error| CommandError::operation("ilink_qr_store_failed", error))?;
    Ok(IlinkQrSession {
        account_id: request.account_id,
        qr_content,
        state: "waiting".into(),
        expires_at_ms,
    })
}

#[tauri::command]
pub(super) async fn poll_ilink_qr_login(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    account_id: String,
) -> Result<IlinkQrSession, CommandError> {
    require_window(&window, "workbench")?;
    let row = sqlx::query("SELECT qrcode, qr_content, state, expires_at_ms FROM integration_ilink_qr_sessions WHERE account_id = ?")
        .bind(&account_id)
        .fetch_optional(state.agent_store.pool())
        .await
        .map_err(|error| CommandError::operation("ilink_qr_load_failed", error))?
        .ok_or_else(|| CommandError::new("ilink_qr_session_not_found", "The iLink QR session does not exist."))?;
    let expires_at_ms = row.get::<i64, _>("expires_at_ms");
    let qr_content = row
        .get::<Option<String>, _>("qr_content")
        .unwrap_or_default();
    let state_value = row.get::<String, _>("state");
    if state_value == "confirmed" || state_value == "cancelled" {
        return Ok(IlinkQrSession {
            account_id,
            qr_content,
            state: state_value,
            expires_at_ms,
        });
    }
    if expires_at_ms <= now_ms() || state_value == "expired" {
        set_ilink_qr_state(&state, &account_id, "expired").await?;
        return Ok(IlinkQrSession {
            account_id,
            qr_content,
            state: "expired".into(),
            expires_at_ms,
        });
    }
    let qrcode = row.get::<Option<String>, _>("qrcode").ok_or_else(|| {
        CommandError::new(
            "ilink_qr_session_invalid",
            "The iLink QR session is incomplete.",
        )
    })?;
    let status = hachimi_channel_providers::WechatIlinkClient::default()
        .poll_qr_status(&qrcode)
        .await
        .map_err(|error| CommandError::operation("ilink_qr_poll_failed", error))?;
    match status {
        hachimi_channel_providers::IlinkQrStatus::Waiting => {
            set_ilink_qr_state(&state, &account_id, "waiting").await?;
            Ok(IlinkQrSession {
                account_id,
                qr_content,
                state: "waiting".into(),
                expires_at_ms,
            })
        }
        hachimi_channel_providers::IlinkQrStatus::Scanned => {
            set_ilink_qr_state(&state, &account_id, "scanned").await?;
            Ok(IlinkQrSession {
                account_id,
                qr_content,
                state: "scanned".into(),
                expires_at_ms,
            })
        }
        hachimi_channel_providers::IlinkQrStatus::Expired => {
            set_ilink_qr_state(&state, &account_id, "expired").await?;
            Ok(IlinkQrSession {
                account_id,
                qr_content,
                state: "expired".into(),
                expires_at_ms,
            })
        }
        hachimi_channel_providers::IlinkQrStatus::Confirmed(confirmed) => {
            let provider_id = IntegrationProviderId::WechatIlink;
            let credential_ref = credential_reference(provider_id, &account_id);
            let mut credential = json!({
                "providerId": provider_id,
                "botToken": confirmed.bot_token,
                "botId": confirmed.bot_id,
                "baseUrl": confirmed.base_url,
            })
            .to_string();
            let fingerprint = digest_hex(credential.as_bytes());
            let keyring_result = integration_keyring_entry(provider_id, &account_id)
                .and_then(|entry| entry.set_password(&credential));
            credential.zeroize();
            keyring_result.map_err(|error| {
                CommandError::operation("integration_secret_store_failed", error)
            })?;
            let timestamp_ms = now_ms();
            let tenant_hash = digest_hex(confirmed.bot_id.as_bytes());
            let config =
                json!({"baseUrl": confirmed.base_url, "ilinkUserId": confirmed.ilink_user_id});
            sqlx::query("UPDATE integration_provider_accounts SET tenant_key = ?, tenant_identity_hash = ?, state = 'starting', diagnostic = NULL, credential_ref = ?, credential_fingerprint = ?, config_json = ?, credential_revision = credential_revision + 1, config_revision = config_revision + 1, updated_at_ms = ? WHERE id = ? AND provider_id = 'wechat_ilink'")
                .bind(&confirmed.bot_id)
                .bind(tenant_hash)
                .bind(&credential_ref)
                .bind(fingerprint)
                .bind(config.to_string())
                .bind(timestamp_ms)
                .bind(&account_id)
                .execute(state.agent_store.pool())
                .await
                .map_err(|error| CommandError::operation("ilink_account_store_failed", error))?;
            let (display_name, credential_revision, config_revision): (String, i64, i64) = sqlx::query_as("SELECT display_name, credential_revision, config_revision FROM integration_provider_accounts WHERE id = ?")
                .bind(&account_id)
                .fetch_one(state.agent_store.pool())
                .await
                .map_err(|error| CommandError::operation("ilink_account_load_failed", error))?;
            let channel_account = ChannelProviderAccount {
                id: account_id.clone(),
                provider_id: provider_id.as_str().into(),
                display_name,
                tenant_key: confirmed.bot_id,
                credential_ref: Some(credential_ref),
                enabled: true,
                state: ChannelAccountState::Starting,
                config,
                credential_revision: from_i64(credential_revision),
                config_revision: from_i64(config_revision),
            };
            state
                .gateway
                .bootstrap_provider_accounts(std::slice::from_ref(&channel_account))
                .await
                .map_err(|error| {
                    CommandError::operation("integration_runtime_configure_failed", error)
                })?;
            sqlx::query("UPDATE integration_provider_accounts SET state = 'healthy', updated_at_ms = ? WHERE id = ?")
                .bind(timestamp_ms)
                .bind(&account_id)
                .execute(state.agent_store.pool())
                .await
                .map_err(|error| CommandError::operation("ilink_account_store_failed", error))?;
            sqlx::query("UPDATE integration_ilink_qr_sessions SET qrcode = NULL, qr_content = NULL, state = 'confirmed', updated_at_ms = ? WHERE account_id = ?")
                .bind(timestamp_ms)
                .bind(&account_id)
                .execute(state.agent_store.pool())
                .await
                .map_err(|error| CommandError::operation("ilink_qr_store_failed", error))?;
            Ok(IlinkQrSession {
                account_id,
                qr_content: String::new(),
                state: "confirmed".into(),
                expires_at_ms,
            })
        }
    }
}

#[tauri::command]
pub(super) async fn cancel_ilink_qr_login(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    account_id: String,
) -> Result<bool, CommandError> {
    require_window(&window, "workbench")?;
    let changed = sqlx::query("UPDATE integration_ilink_qr_sessions SET qrcode = NULL, qr_content = NULL, state = 'cancelled', updated_at_ms = ? WHERE account_id = ? AND state IN ('waiting', 'scanned', 'expired')")
        .bind(now_ms())
        .bind(&account_id)
        .execute(state.agent_store.pool())
        .await
        .map_err(|error| CommandError::operation("ilink_qr_store_failed", error))?
        .rows_affected() == 1;
    if changed {
        sqlx::query("DELETE FROM integration_provider_accounts WHERE id = ? AND provider_id = 'wechat_ilink' AND credential_ref IS NULL")
            .bind(&account_id)
            .execute(state.agent_store.pool())
            .await
            .map_err(|error| CommandError::operation("ilink_account_remove_failed", error))?;
    }
    Ok(changed)
}

#[tauri::command]
pub(super) async fn upsert_enterprise_integration(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: IntegrationAccountUpsert,
) -> Result<IntegrationProviderAccount, CommandError> {
    require_window(&window, "workbench")?;
    validate_upsert(&input)?;
    if input.messaging_enabled {
        wait_for_gateway_ready(&state).await?;
    }
    let provider_id = input.credential.provider_id();
    let tenant = tenant_id(&input.credential);
    let tenant_hash = digest_hex(tenant.as_bytes());
    let target = resolve_integration_upsert_target(
        state.agent_store.pool(),
        &input.id,
        provider_id,
        tenant,
        &tenant_hash,
        input.expected_config_revision,
    )
    .await?;
    let credential_ref = credential_reference(provider_id, &target.account_id);
    let journal_id = begin_journal(
        &state,
        &target.account_id,
        "upsert",
        "credential",
        Some(&credential_ref),
    )
    .await?;
    let mut credential = credential_json(&input.credential);
    let credential_fingerprint = digest_hex(credential.as_bytes());
    let keyring_result = integration_keyring_entry(provider_id, &target.account_id)
        .and_then(|entry| entry.set_password(&credential));
    credential.zeroize();
    if let Err(error) = keyring_result {
        fail_journal(
            &state,
            &journal_id,
            "credential",
            "integration_secret_store_failed",
        )
        .await;
        return Err(CommandError::operation(
            "integration_secret_store_failed",
            error,
        ));
    }
    update_journal(&state, &journal_id, "account", "in_progress", None).await?;
    let timestamp_ms = now_ms();
    let credential_revision = target
        .previous_revisions
        .map_or(1, |value| value.0.saturating_add(1));
    let config_revision = target
        .previous_revisions
        .map_or(1, |value| value.1.saturating_add(1));
    let config = public_config(&input.credential);
    sqlx::query("INSERT INTO integration_provider_accounts(id, provider_id, display_name, tenant_key, tenant_identity_hash, transport, state, diagnostic, connector_account_id, credential_ref, credential_fingerprint, api_access_enabled, messaging_enabled, config_json, credential_revision, config_revision, last_event_at_ms, last_delivery_at_ms, next_reconnect_at_ms, consecutive_failures, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, 'starting', NULL, NULL, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, NULL, 0, ?, ?) ON CONFLICT(id) DO UPDATE SET provider_id = excluded.provider_id, display_name = excluded.display_name, tenant_key = excluded.tenant_key, tenant_identity_hash = excluded.tenant_identity_hash, transport = excluded.transport, state = 'starting', diagnostic = NULL, credential_ref = excluded.credential_ref, credential_fingerprint = excluded.credential_fingerprint, api_access_enabled = excluded.api_access_enabled, messaging_enabled = excluded.messaging_enabled, config_json = excluded.config_json, credential_revision = excluded.credential_revision, config_revision = excluded.config_revision, updated_at_ms = excluded.updated_at_ms")
        .bind(&target.account_id)
        .bind(provider_id.as_str())
        .bind(input.display_name.trim())
        .bind(tenant)
        .bind(tenant_hash)
        .bind(transport_str(transport(provider_id)))
        .bind(&credential_ref)
        .bind(credential_fingerprint)
        .bind(input.api_access_enabled)
        .bind(input.messaging_enabled)
        .bind(config.to_string())
        .bind(to_i64(credential_revision))
        .bind(to_i64(config_revision))
        .bind(timestamp_ms)
        .bind(timestamp_ms)
        .execute(state.agent_store.pool())
        .await
        .map_err(integration_account_store_error)?;
    sqlx::query("INSERT INTO channel_access_policies(account_id, dm_policy, allowlist_actor_ids_json, grant_ceiling_json, revision, updated_at_ms) VALUES(?, 'pairing', '[]', ?, 1, ?) ON CONFLICT(account_id) DO NOTHING")
        .bind(&target.account_id)
        .bind(EMPTY_CHANNEL_GRANT_JSON)
        .bind(timestamp_ms)
        .execute(state.agent_store.pool())
        .await
        .map_err(|error| CommandError::operation("integration_policy_store_failed", error))?;

    update_journal(&state, &journal_id, "runtime", "in_progress", None).await?;
    let channel_account = ChannelProviderAccount {
        id: target.account_id.clone(),
        provider_id: provider_id.as_str().into(),
        display_name: input.display_name.clone(),
        tenant_key: tenant.into(),
        credential_ref: Some(credential_ref.clone()),
        enabled: input.messaging_enabled,
        state: ChannelAccountState::Starting,
        config,
        credential_revision,
        config_revision,
    };
    state
        .gateway
        .bootstrap_provider_accounts(std::slice::from_ref(&channel_account))
        .await
        .map_err(|error| CommandError::operation("integration_runtime_configure_failed", error))?;
    if input.api_access_enabled {
        sync_enterprise_connector(
            &window,
            &state,
            provider_id,
            &target.account_id,
            &input.display_name,
        )
        .await?;
    }
    sqlx::query("UPDATE integration_provider_accounts SET state = 'healthy', updated_at_ms = ? WHERE id = ?")
        .bind(now_ms())
        .bind(&target.account_id)
        .execute(state.agent_store.pool())
        .await
        .map_err(|error| CommandError::operation("integration_account_commit_failed", error))?;
    update_journal(&state, &journal_id, "commit", "committed", None).await?;
    load_account(&state, &target.account_id).await
}

#[tauri::command]
pub(super) async fn set_enterprise_integration_capabilities(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    update: IntegrationAccountCapabilitiesUpdate,
) -> Result<IntegrationProviderAccount, CommandError> {
    require_window(&window, "workbench")?;
    let account = load_account(&state, &update.id).await?;
    validate_capabilities(
        account.provider_id,
        update.api_access_enabled,
        update.messaging_enabled,
    )?;
    if update.messaging_enabled {
        wait_for_gateway_ready(&state).await?;
    }
    let changed = sqlx::query("UPDATE integration_provider_accounts SET api_access_enabled = ?, messaging_enabled = ?, state = CASE WHEN ? = 1 THEN 'starting' ELSE 'healthy' END, config_revision = config_revision + 1, updated_at_ms = ? WHERE id = ? AND config_revision = ?")
        .bind(update.api_access_enabled)
        .bind(update.messaging_enabled)
        .bind(update.messaging_enabled)
        .bind(now_ms())
        .bind(&update.id)
        .bind(to_i64(update.expected_config_revision))
        .execute(state.agent_store.pool())
        .await
        .map_err(|error| CommandError::operation("integration_capabilities_store_failed", error))?;
    if changed.rows_affected() != 1 {
        return Err(revision_conflict());
    }
    let config_revision = update.expected_config_revision.saturating_add(1);
    state
        .gateway
        .bootstrap_provider_accounts(&[ChannelProviderAccount {
            id: account.id.clone(),
            provider_id: account.provider_id.as_str().into(),
            display_name: account.display_name.clone(),
            tenant_key: tenant_key_for_account(&state, &account.id).await?,
            credential_ref: Some(credential_reference(account.provider_id, &account.id)),
            enabled: update.messaging_enabled,
            state: if update.messaging_enabled {
                ChannelAccountState::Starting
            } else {
                ChannelAccountState::Draft
            },
            config: config_for_account(&state, &account.id).await?,
            credential_revision: account.credential_revision,
            config_revision,
        }])
        .await
        .map_err(|error| CommandError::operation("integration_runtime_configure_failed", error))?;
    if update.api_access_enabled {
        sync_enterprise_connector(
            &window,
            &state,
            account.provider_id,
            &account.id,
            &account.display_name,
        )
        .await?;
    } else if let Some(connector_account_id) = &account.connector_account_id {
        let mut transaction = state.agent_store.pool().begin().await.map_err(|error| {
            CommandError::operation("enterprise_connector_unlink_failed", error)
        })?;
        sqlx::query(
            "UPDATE integration_provider_accounts SET connector_account_id = NULL WHERE id = ?",
        )
        .bind(&account.id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| CommandError::operation("enterprise_connector_unlink_failed", error))?;
        sqlx::query("DELETE FROM connector_accounts WHERE id = ?")
            .bind(connector_account_id.as_str())
            .execute(&mut *transaction)
            .await
            .map_err(|error| {
                CommandError::operation("enterprise_connector_unlink_failed", error)
            })?;
        transaction.commit().await.map_err(|error| {
            CommandError::operation("enterprise_connector_unlink_failed", error)
        })?;
    }
    sqlx::query("UPDATE integration_provider_accounts SET state = 'healthy' WHERE id = ?")
        .bind(&update.id)
        .execute(state.agent_store.pool())
        .await
        .map_err(|error| CommandError::operation("integration_account_commit_failed", error))?;
    load_account(&state, &update.id).await
}

#[tauri::command]
pub(super) async fn probe_enterprise_integration(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    id: String,
) -> Result<IntegrationAccountProbeResult, CommandError> {
    require_window(&window, "workbench")?;
    let account = load_account(&state, &id).await?;
    let credential_raw = integration_keyring_entry(account.provider_id, &id)
        .and_then(|entry| entry.get_password())
        .ok();
    let credential_input = credential_raw
        .as_deref()
        .and_then(|raw| serde_json::from_str::<IntegrationCredentialInput>(raw).ok());
    let shape_ok = credential_input
        .as_ref()
        .is_some_and(|credential| credential_shape_valid(credential, account.messaging_enabled));
    if account.messaging_enabled {
        wait_for_gateway_ready(&state).await?;
    }
    let health = persisted_provider_health(&state).await?;
    let provider_health = health.iter().find(|value| {
        value.provider_id == account.provider_id.as_str()
            && value.account_id.as_deref() == account.channel_account_id.as_deref()
    });
    let transport_ok = !account.messaging_enabled
        || provider_health.is_some_and(|value| {
            matches!(
                value.state,
                hachimi_protocol::ChannelProviderHealthState::Healthy
                    | hachimi_protocol::ChannelProviderHealthState::Degraded
            )
        });
    let enterprise_auth = if account.provider_id.supports_enterprise_api() && shape_ok {
        match credential_raw
            .as_deref()
            .and_then(|raw| EnterpriseCredential::parse(raw).ok())
        {
            Some(credential) => tokio::time::timeout(
                std::time::Duration::from_secs(12),
                EnterpriseApiClient::default().account_identity(&id, &credential),
            )
            .await
            .is_ok_and(|result| result.is_ok()),
            None => false,
        }
    } else {
        shape_ok
    };
    let reverse_proxy_ok =
        if account.provider_id == IntegrationProviderId::WecomApp && account.messaging_enabled {
            let base_url = config_for_account(&state, &id)
                .await
                .ok()
                .and_then(|config| {
                    config
                        .get("externalHttpsUrl")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                });
            match base_url {
                Some(base_url) => tokio::time::timeout(
                    std::time::Duration::from_secs(12),
                    EnterpriseApiClient::probe_wecom_callback_endpoint(&base_url, &id),
                )
                .await
                .is_ok_and(|result| result.is_ok()),
                None => false,
            }
        } else {
            true
        };
    let robot_code_ready = account.provider_id != IntegrationProviderId::DingTalk
        || credential_input.as_ref().is_some_and(|value| {
            matches!(value, IntegrationCredentialInput::DingTalk { robot_code: Some(code), .. } if !code.trim().is_empty())
        });
    let credential = probe_dimension(
        enterprise_auth,
        "credential_authenticated",
        if shape_ok {
            "credential_authentication_failed"
        } else {
            "credential_missing_or_invalid"
        },
    );
    let ingress =
        ingress_probe_dimension(account.messaging_enabled, transport_ok, reverse_proxy_ok);
    let egress = egress_probe_dimension(
        account.messaging_enabled,
        transport_ok,
        enterprise_auth,
        robot_code_ready,
    );
    let api = api_probe_dimension(account.api_access_enabled, enterprise_auth);
    let snapshot = IntegrationAccountProbeSnapshot {
        credential: credential.clone(),
        ingress: ingress.clone(),
        egress: egress.clone(),
        api: api.clone(),
        probed_at_ms: now_ms(),
    };
    let mut result = IntegrationAccountProbeResult {
        account,
        credential,
        ingress,
        egress,
        api,
    };
    let healthy = result.credential.ok && result.ingress.ok && result.egress.ok && result.api.ok;
    store_probe_snapshot(state.agent_store.pool(), &id, &snapshot).await?;
    let account_state = if healthy {
        "healthy"
    } else if !result.credential.ok {
        "needs_attention"
    } else {
        "degraded"
    };
    let diagnostic = match account_state {
        "needs_attention" => Some("integration_credentials_invalid"),
        "degraded" => Some("integration_transport_unavailable"),
        _ => None,
    };
    sqlx::query("UPDATE integration_provider_accounts SET state = ?, diagnostic = ?, updated_at_ms = ? WHERE id = ?")
        .bind(account_state)
        .bind(diagnostic)
        .bind(now_ms())
        .bind(&id)
        .execute(state.agent_store.pool())
        .await
        .map_err(|error| CommandError::operation("integration_probe_store_failed", error))?;
    result.account = load_account(&state, &id).await?;
    Ok(result)
}

#[tauri::command]
pub(super) async fn remove_enterprise_integration(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    id: String,
) -> Result<bool, CommandError> {
    require_window(&window, "workbench")?;
    let account = load_account(&state, &id).await?;
    let credential_ref = credential_reference(account.provider_id, &id);
    let journal_id = begin_journal(&state, &id, "remove", "account", Some(&credential_ref)).await?;
    sqlx::query("UPDATE integration_provider_accounts SET state = 'removing', updated_at_ms = ? WHERE id = ?")
        .bind(now_ms())
        .bind(&id)
        .execute(state.agent_store.pool())
        .await
        .map_err(|error| CommandError::operation("integration_remove_failed", error))?;
    stage_account_removal(state.agent_store.pool(), &id, Some(&credential_ref)).await?;
    update_journal(
        &state,
        &journal_id,
        "credential_cleanup",
        "deferred_cleanup",
        None,
    )
    .await?;
    reconcile_secret_cleanup(&state.agent_store).await?;
    let pending_cleanup: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM integration_secret_cleanup_queue WHERE account_id = ?)",
    )
    .bind(&id)
    .fetch_one(state.agent_store.pool())
    .await
    .map_err(|error| CommandError::operation("integration_cleanup_load_failed", error))?;
    if pending_cleanup {
        update_journal(
            &state,
            &journal_id,
            "credential_cleanup",
            "deferred_cleanup",
            Some("integration_secret_delete_deferred"),
        )
        .await?;
    } else {
        update_journal(&state, &journal_id, "commit", "committed", None).await?;
    }
    Ok(true)
}

#[tauri::command]
pub(super) async fn list_channel_authorizations(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    account_id: String,
) -> Result<Vec<ChannelAuthorization>, CommandError> {
    require_window(&window, "workbench")?;
    state
        .gateway
        .list_authorizations(&account_id)
        .await
        .map_err(|error| CommandError::operation("channel_authorization_list_failed", error))
}

#[tauri::command]
pub(super) async fn upsert_channel_authorization(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    mut input: ChannelAuthorizationUpsert,
) -> Result<ChannelAuthorization, CommandError> {
    require_window(&window, "workbench")?;
    let account = load_account(&state, &input.account_id).await?;
    input.address.account_id = account.id.clone();
    input.address.provider_id = account.provider_id.as_str().into();
    input.address.tenant_key = tenant_key_for_account(&state, &account.id).await?;
    state
        .gateway
        .upsert_authorization(input, "manual", now_ms())
        .await
        .map_err(|error| CommandError::operation("channel_authorization_upsert_failed", error))
}

#[tauri::command]
pub(super) async fn create_channel_pairing_code(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ChannelPairingCodeRequest,
) -> Result<ChannelPairingCode, CommandError> {
    require_window(&window, "workbench")?;
    state
        .gateway
        .create_pairing_code(request, now_ms())
        .await
        .map_err(|error| CommandError::operation("channel_pairing_code_create_failed", error))
}

#[tauri::command]
pub(super) async fn create_channel_identity_link_code(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ChannelIdentityLinkCodeRequest,
) -> Result<ChannelIdentityLinkCode, CommandError> {
    require_window(&window, "workbench")?;
    state
        .gateway
        .create_identity_link_code(request, now_ms())
        .await
        .map_err(|error| CommandError::operation("channel_identity_link_code_create_failed", error))
}

#[tauri::command]
pub(super) async fn list_channel_identity_transfer_previews(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    account_id: String,
) -> Result<Vec<ChannelIdentityTransferPreview>, CommandError> {
    require_window(&window, "workbench")?;
    state
        .gateway
        .list_identity_transfer_previews(&account_id, now_ms())
        .await
        .map_err(|error| CommandError::operation("channel_identity_transfer_list_failed", error))
}

#[tauri::command]
pub(super) async fn transfer_channel_identity(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ChannelIdentityTransferCommitRequest,
) -> Result<ChannelIdentityTransferResult, CommandError> {
    require_window(&window, "workbench")?;
    state
        .gateway
        .transfer_identity(request, now_ms())
        .await
        .map_err(|error| CommandError::operation("channel_identity_transfer_failed", error))
}

#[tauri::command]
pub(super) async fn get_channel_access_policy(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    account_id: String,
) -> Result<ChannelAccessPolicy, CommandError> {
    require_window(&window, "workbench")?;
    state
        .gateway
        .access_policy(&account_id)
        .await
        .map_err(|error| CommandError::operation("channel_policy_load_failed", error))
}

#[tauri::command]
pub(super) async fn update_channel_access_policy(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: ChannelAccessPolicyUpsert,
) -> Result<ChannelAccessPolicy, CommandError> {
    require_window(&window, "workbench")?;
    state
        .gateway
        .upsert_access_policy(input, now_ms())
        .await
        .map_err(|error| CommandError::operation("channel_policy_update_failed", error))
}

fn provider_definitions() -> Vec<IntegrationProviderDefinition> {
    vec![
        definition(
            IntegrationProviderId::DingTalk,
            "钉钉",
            "DingTalk",
            "integration-icons/dingtalk.svg",
            IntegrationAuthMethod::ClientSecret,
            vec![
                text("clientId", "Client/App ID"),
                secret("clientSecret", "Client/App Secret"),
                required_for(
                    "agentId",
                    "Agent ID",
                    CredentialFieldKind::Text,
                    IntegrationCapability::ApiAccess,
                ),
                optional_for(
                    "robotCode",
                    "Robot Code",
                    CredentialFieldKind::Text,
                    IntegrationCapability::ProactiveDelivery,
                ),
            ],
        ),
        definition(
            IntegrationProviderId::Feishu,
            "飞书",
            "Feishu",
            "integration-icons/feishu.svg",
            IntegrationAuthMethod::ClientSecret,
            vec![text("appId", "App ID"), secret("appSecret", "App Secret")],
        ),
        definition(
            IntegrationProviderId::WecomAiBot,
            "企微 AI Bot",
            "WeCom AI Bot",
            "integration-icons/wecom.ico",
            IntegrationAuthMethod::BotSecret,
            vec![text("botId", "Bot ID"), secret("secret", "Secret")],
        ),
        definition(
            IntegrationProviderId::WecomApp,
            "企微自建应用",
            "WeCom custom app",
            "integration-icons/wecom.ico",
            IntegrationAuthMethod::CallbackSecret,
            vec![
                text("corpId", "Corp ID"),
                secret("corpSecret", "Corp Secret"),
                text("agentId", "Agent ID"),
                required_for(
                    "callbackToken",
                    "Callback Token",
                    CredentialFieldKind::Secret,
                    IntegrationCapability::Messaging,
                ),
                required_for(
                    "encodingAesKey",
                    "Encoding AES Key",
                    CredentialFieldKind::Secret,
                    IntegrationCapability::Messaging,
                ),
                required_for(
                    "externalHttpsUrl",
                    "External HTTPS URL",
                    CredentialFieldKind::HttpsUrl,
                    IntegrationCapability::Messaging,
                ),
            ],
        ),
        definition(
            IntegrationProviderId::WechatIlink,
            "微信 iLink / ClawBot",
            "WeChat iLink / ClawBot",
            "integration-icons/wechat-ilink.svg",
            IntegrationAuthMethod::QrCode,
            Vec::new(),
        ),
    ]
}

fn definition(
    id: IntegrationProviderId,
    name_zh: &str,
    name_en: &str,
    icon_asset: &str,
    auth_method: IntegrationAuthMethod,
    credential_fields: Vec<CredentialFieldDefinition>,
) -> IntegrationProviderDefinition {
    let mut capabilities = vec![IntegrationCapability::Messaging, IntegrationCapability::Dm];
    capabilities.push(IntegrationCapability::MediaReceive);
    capabilities.push(IntegrationCapability::MediaSend);
    if !matches!(id, IntegrationProviderId::WechatIlink) {
        capabilities.push(IntegrationCapability::ProactiveDelivery);
    }
    if !matches!(
        id,
        IntegrationProviderId::WechatIlink | IntegrationProviderId::WecomApp
    ) {
        capabilities.push(IntegrationCapability::Group);
    }
    if id == IntegrationProviderId::Feishu {
        capabilities.push(IntegrationCapability::Topic);
    }
    if id.supports_enterprise_api() {
        capabilities.push(IntegrationCapability::ApiAccess);
    }
    if id == IntegrationProviderId::WechatIlink {
        capabilities.push(IntegrationCapability::QrLogin);
    }
    IntegrationProviderDefinition {
        id,
        name_zh: name_zh.into(),
        name_en: name_en.into(),
        icon_asset: icon_asset.into(),
        transport: transport(id),
        auth_method,
        capabilities,
        credential_fields,
        source_status: if id == IntegrationProviderId::WechatIlink {
            "public_qualification_unverified"
        } else {
            "official_wire_contract"
        }
        .into(),
    }
}

fn text(id: &str, label: &str) -> CredentialFieldDefinition {
    field(id, label, CredentialFieldKind::Text, true)
}
fn secret(id: &str, label: &str) -> CredentialFieldDefinition {
    field(id, label, CredentialFieldKind::Secret, true)
}
fn field(
    id: &str,
    label: &str,
    kind: CredentialFieldKind,
    required: bool,
) -> CredentialFieldDefinition {
    CredentialFieldDefinition {
        id: id.into(),
        label: label.into(),
        kind,
        required,
        capability: None,
    }
}

fn required_for(
    id: &str,
    label: &str,
    kind: CredentialFieldKind,
    capability: IntegrationCapability,
) -> CredentialFieldDefinition {
    CredentialFieldDefinition {
        id: id.into(),
        label: label.into(),
        kind,
        required: true,
        capability: Some(capability),
    }
}

fn optional_for(
    id: &str,
    label: &str,
    kind: CredentialFieldKind,
    capability: IntegrationCapability,
) -> CredentialFieldDefinition {
    CredentialFieldDefinition {
        required: false,
        ..required_for(id, label, kind, capability)
    }
}

fn credential_json(input: &IntegrationCredentialInput) -> String {
    serde_json::to_string(input).expect("credential contract serializes")
}

fn public_config(input: &IntegrationCredentialInput) -> Value {
    match input {
        IntegrationCredentialInput::DingTalk {
            agent_id,
            robot_code,
            ..
        } => json!({"agentId":agent_id,"robotCodeConfigured":robot_code.is_some()}),
        IntegrationCredentialInput::Feishu { .. } => json!({}),
        IntegrationCredentialInput::WecomAiBot { .. } => {
            json!({"endpoint":hachimi_channel_providers::OPENWS_ENDPOINT,"subscribeAckTimeoutSecs":hachimi_channel_providers::SUBSCRIBE_ACK_TIMEOUT_SECS,"heartbeatIntervalSecs":hachimi_channel_providers::HEARTBEAT_INTERVAL_SECS})
        }
        IntegrationCredentialInput::WecomApp {
            external_https_url, ..
        } => json!({"externalHttpsUrl":external_https_url}),
        IntegrationCredentialInput::WechatIlink { base_url, .. } => json!({"baseUrl":base_url}),
    }
}

fn tenant_id(input: &IntegrationCredentialInput) -> &str {
    match input {
        IntegrationCredentialInput::DingTalk { client_id, .. } => client_id,
        IntegrationCredentialInput::Feishu { app_id, .. } => app_id,
        IntegrationCredentialInput::WecomAiBot { bot_id, .. } => bot_id,
        IntegrationCredentialInput::WecomApp { corp_id, .. } => corp_id,
        IntegrationCredentialInput::WechatIlink { bot_id, .. } => bot_id,
    }
}

fn transport(provider_id: IntegrationProviderId) -> IntegrationTransport {
    match provider_id {
        IntegrationProviderId::DingTalk => IntegrationTransport::Stream,
        IntegrationProviderId::Feishu => IntegrationTransport::LongConnection,
        IntegrationProviderId::WecomAiBot => IntegrationTransport::WebSocket,
        IntegrationProviderId::WecomApp => IntegrationTransport::EncryptedCallback,
        IntegrationProviderId::WechatIlink => IntegrationTransport::QrLongPoll,
    }
}

async fn load_accounts(
    state: &DesktopState,
) -> Result<Vec<IntegrationProviderAccount>, CommandError> {
    let rows = sqlx::query(
        "SELECT * FROM integration_provider_accounts ORDER BY provider_id, display_name, id",
    )
    .fetch_all(state.agent_store.pool())
    .await
    .map_err(|error| CommandError::operation("integration_account_list_failed", error))?;
    let mut accounts = Vec::with_capacity(rows.len());
    for row in rows {
        accounts.push(decode_account(state, row).await?);
    }
    Ok(accounts)
}

async fn load_account(
    state: &DesktopState,
    id: &str,
) -> Result<IntegrationProviderAccount, CommandError> {
    let row = sqlx::query("SELECT * FROM integration_provider_accounts WHERE id = ?")
        .bind(id)
        .fetch_optional(state.agent_store.pool())
        .await
        .map_err(|error| CommandError::operation("integration_account_load_failed", error))?
        .ok_or_else(|| {
            CommandError::new(
                "integration_account_not_found",
                "Integration account does not exist.",
            )
        })?;
    decode_account(state, row).await
}

async fn decode_account(
    state: &DesktopState,
    row: sqlx::sqlite::SqliteRow,
) -> Result<IntegrationProviderAccount, CommandError> {
    let id: String = row.get("id");
    let provider_id = parse_provider_id(row.get("provider_id"))?;
    let authorizations = state
        .gateway
        .list_authorizations(&id)
        .await
        .map_err(|error| CommandError::operation("channel_authorization_list_failed", error))?;
    let runtime_health = state
        .gateway
        .persisted_provider_health()
        .await
        .map_err(|error| CommandError::operation("integration_runtime_health_unavailable", error))?
        .into_iter()
        .find(|health| health.account_id.as_deref() == Some(id.as_str()));
    let probe = load_probe_snapshot(state.agent_store.pool(), &id).await?;
    Ok(IntegrationProviderAccount {
        id: id.clone(),
        display_name: row.get("display_name"),
        provider_id,
        connector_account_id: row
            .get::<Option<String>, _>("connector_account_id")
            .map(hachimi_protocol::ConnectorAccountId::new),
        channel_account_id: Some(id),
        tenant_identity_hash: row.get("tenant_identity_hash"),
        transport: parse_transport(row.get("transport"))?,
        state: parse_account_state(row.get("state"))?,
        diagnostic: row.get("diagnostic"),
        api_access_enabled: row.get("api_access_enabled"),
        messaging_enabled: row.get("messaging_enabled"),
        authorizations,
        last_event_at_ms: row.get("last_event_at_ms"),
        last_delivery_at_ms: row.get("last_delivery_at_ms"),
        last_handshake_at_ms: runtime_health
            .as_ref()
            .and_then(|health| health.last_handshake_at_ms),
        last_frame_at_ms: runtime_health
            .as_ref()
            .and_then(|health| health.last_frame_at_ms),
        last_error_code: runtime_health
            .as_ref()
            .and_then(|health| health.last_error_code.clone()),
        next_reconnect_at_ms: runtime_health
            .as_ref()
            .and_then(|health| health.next_reconnect_at_ms),
        consecutive_failures: runtime_health.map_or(0, |health| health.consecutive_failures),
        probe,
        credential_revision: from_i64(row.get("credential_revision")),
        config_revision: from_i64(row.get("config_revision")),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

async fn load_probe_snapshot(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<Option<IntegrationAccountProbeSnapshot>, CommandError> {
    sqlx::query("SELECT credential_json, ingress_json, egress_json, api_json, probed_at_ms FROM integration_probe_snapshots WHERE account_id = ?")
        .bind(account_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| CommandError::operation("integration_probe_load_failed", error))?
        .map(|probe| {
            Ok::<_, CommandError>(IntegrationAccountProbeSnapshot {
                credential: serde_json::from_str(probe.get("credential_json"))
                    .map_err(|error| CommandError::operation("integration_probe_invalid", error))?,
                ingress: serde_json::from_str(probe.get("ingress_json"))
                    .map_err(|error| CommandError::operation("integration_probe_invalid", error))?,
                egress: serde_json::from_str(probe.get("egress_json"))
                    .map_err(|error| CommandError::operation("integration_probe_invalid", error))?,
                api: serde_json::from_str(probe.get("api_json"))
                    .map_err(|error| CommandError::operation("integration_probe_invalid", error))?,
                probed_at_ms: probe.get("probed_at_ms"),
            })
        })
        .transpose()
}

async fn store_probe_snapshot(
    pool: &SqlitePool,
    account_id: &str,
    snapshot: &IntegrationAccountProbeSnapshot,
) -> Result<(), CommandError> {
    sqlx::query("INSERT INTO integration_probe_snapshots(account_id, credential_json, ingress_json, egress_json, api_json, probed_at_ms) VALUES(?, ?, ?, ?, ?, ?) ON CONFLICT(account_id) DO UPDATE SET credential_json = excluded.credential_json, ingress_json = excluded.ingress_json, egress_json = excluded.egress_json, api_json = excluded.api_json, probed_at_ms = excluded.probed_at_ms")
        .bind(account_id)
        .bind(serde_json::to_string(&snapshot.credential).map_err(|error| CommandError::operation("integration_probe_store_failed", error))?)
        .bind(serde_json::to_string(&snapshot.ingress).map_err(|error| CommandError::operation("integration_probe_store_failed", error))?)
        .bind(serde_json::to_string(&snapshot.egress).map_err(|error| CommandError::operation("integration_probe_store_failed", error))?)
        .bind(serde_json::to_string(&snapshot.api).map_err(|error| CommandError::operation("integration_probe_store_failed", error))?)
        .bind(snapshot.probed_at_ms)
        .execute(pool)
        .await
        .map_err(|error| CommandError::operation("integration_probe_store_failed", error))?;
    Ok(())
}

async fn tenant_key_for_account(state: &DesktopState, id: &str) -> Result<String, CommandError> {
    sqlx::query_scalar("SELECT tenant_key FROM integration_provider_accounts WHERE id = ?")
        .bind(id)
        .fetch_one(state.agent_store.pool())
        .await
        .map_err(|error| CommandError::operation("integration_account_load_failed", error))
}

async fn config_for_account(state: &DesktopState, id: &str) -> Result<Value, CommandError> {
    let raw: String =
        sqlx::query_scalar("SELECT config_json FROM integration_provider_accounts WHERE id = ?")
            .bind(id)
            .fetch_one(state.agent_store.pool())
            .await
            .map_err(|error| CommandError::operation("integration_account_load_failed", error))?;
    serde_json::from_str(&raw)
        .map_err(|error| CommandError::operation("integration_config_invalid", error))
}

async fn sync_enterprise_connector(
    window: &WebviewWindow,
    state: &DesktopState,
    provider_id: IntegrationProviderId,
    account_id: &str,
    display_name: &str,
) -> Result<(), CommandError> {
    if !provider_id.supports_enterprise_api() {
        return Ok(());
    }
    let resource_dir = window
        .app_handle()
        .path()
        .resource_dir()
        .map_err(|error| CommandError::operation("plugin_resource_dir_failed", error))?;
    let plugin_id = hachimi_protocol::PluginId::new(provider_id.as_str());
    let bundle = resource_dir.join("plugins").join(provider_id.as_str());
    if state
        .plugin_host
        .get(&plugin_id)
        .await
        .map_err(|error| CommandError::operation("enterprise_connector_plugin_load_failed", error))?
        .is_none()
    {
        state
            .plugin_host
            .install_local(&bundle)
            .await
            .map_err(|error| {
                CommandError::operation("enterprise_connector_plugin_install_failed", error)
            })?;
    }
    state
        .plugin_host
        .set_enabled(&plugin_id, true)
        .await
        .map_err(|error| {
            CommandError::operation("enterprise_connector_plugin_enable_failed", error)
        })?;
    let connector_account_id =
        hachimi_protocol::ConnectorAccountId::new(format!("integration:{account_id}"));
    let account = state
        .plugin_host
        .upsert_integration_connector_account(
            hachimi_protocol::ConnectorAccountUpsert {
                id: connector_account_id.clone(),
                plugin_id,
                connector_id: provider_id.as_str().into(),
                display_name: display_name.into(),
                secret: None,
            },
            provider_id,
            account_id,
        )
        .await
        .map_err(|error| {
            CommandError::operation("enterprise_connector_account_store_failed", error)
        })?;
    sqlx::query("UPDATE integration_provider_accounts SET connector_account_id = ? WHERE id = ?")
        .bind(account.id.as_str())
        .bind(account_id)
        .execute(state.agent_store.pool())
        .await
        .map_err(|error| CommandError::operation("enterprise_connector_link_failed", error))?;
    Ok(())
}

async fn begin_journal(
    state: &DesktopState,
    account_id: &str,
    operation: &str,
    phase: &str,
    credential_ref: Option<&str>,
) -> Result<String, CommandError> {
    let id = Uuid::now_v7().to_string();
    let timestamp_ms = now_ms();
    sqlx::query("INSERT INTO integration_lifecycle_journal(id, account_id, operation, phase, status, credential_ref, error_code, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, 'in_progress', ?, NULL, ?, ?)")
        .bind(&id).bind(account_id).bind(operation).bind(phase).bind(credential_ref).bind(timestamp_ms).bind(timestamp_ms)
        .execute(state.agent_store.pool()).await.map_err(|error| CommandError::operation("integration_journal_store_failed", error))?;
    Ok(id)
}

async fn update_journal(
    state: &DesktopState,
    id: &str,
    phase: &str,
    status: &str,
    error_code: Option<&str>,
) -> Result<(), CommandError> {
    sqlx::query("UPDATE integration_lifecycle_journal SET phase = ?, status = ?, error_code = ?, updated_at_ms = ? WHERE id = ?")
        .bind(phase).bind(status).bind(error_code).bind(now_ms()).bind(id)
        .execute(state.agent_store.pool()).await.map_err(|error| CommandError::operation("integration_journal_store_failed", error))?;
    Ok(())
}

async fn fail_journal(state: &DesktopState, id: &str, phase: &str, code: &str) {
    let _ = update_journal(state, id, phase, "failed", Some(code)).await;
}

async fn queue_secret_cleanup_for_store(
    store: &hachimi_storage::AgentStore,
    account_id: &str,
    secret_ref: &str,
    timestamp_ms: i64,
) -> Result<(), CommandError> {
    sqlx::query("INSERT INTO integration_secret_cleanup_queue(secret_ref, account_id, attempt, next_attempt_at_ms, error_code, created_at_ms, updated_at_ms) VALUES(?, ?, 0, ?, 'delete_failed', ?, ?) ON CONFLICT(secret_ref) DO UPDATE SET next_attempt_at_ms = excluded.next_attempt_at_ms, updated_at_ms = excluded.updated_at_ms")
        .bind(secret_ref).bind(account_id).bind(timestamp_ms).bind(timestamp_ms).bind(timestamp_ms)
        .execute(store.pool()).await.map_err(|error| CommandError::operation("integration_cleanup_queue_failed", error))?;
    Ok(())
}

async fn stage_account_removal(
    pool: &SqlitePool,
    account_id: &str,
    primary_credential_ref: Option<&str>,
) -> Result<(), CommandError> {
    let timestamp_ms = now_ms();
    let mut transaction = pool
        .begin()
        .await
        .map_err(|error| CommandError::operation("integration_remove_failed", error))?;
    let connector_account_id: Option<String> = sqlx::query_scalar(
        "SELECT connector_account_id FROM integration_provider_accounts WHERE id = ?",
    )
    .bind(account_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| CommandError::operation("integration_remove_failed", error))?
    .flatten();
    if let Some(secret_ref) = primary_credential_ref {
        sqlx::query("INSERT OR IGNORE INTO integration_secret_cleanup_queue(secret_ref, account_id, attempt, next_attempt_at_ms, error_code, created_at_ms, updated_at_ms) VALUES(?, ?, 0, ?, 'account_removal', ?, ?)")
            .bind(secret_ref)
            .bind(account_id)
            .bind(timestamp_ms)
            .bind(timestamp_ms)
            .bind(timestamp_ms)
            .execute(&mut *transaction)
            .await
            .map_err(|error| CommandError::operation("integration_cleanup_queue_failed", error))?;
    }
    sqlx::query("INSERT OR IGNORE INTO integration_secret_cleanup_queue(secret_ref, account_id, attempt, next_attempt_at_ms, error_code, created_at_ms, updated_at_ms) SELECT secret_ref, account_id, 0, ?, 'account_removal', ?, ? FROM channel_route_secrets WHERE account_id = ?")
        .bind(timestamp_ms)
        .bind(timestamp_ms)
        .bind(timestamp_ms)
        .bind(account_id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| CommandError::operation("integration_cleanup_queue_failed", error))?;
    sqlx::query("INSERT OR IGNORE INTO integration_secret_cleanup_queue(secret_ref, account_id, attempt, next_attempt_at_ms, error_code, created_at_ms, updated_at_ms) SELECT secret_ref, account_id, 0, ?, 'account_removal', ?, ? FROM channel_media_secrets WHERE account_id = ?")
        .bind(timestamp_ms)
        .bind(timestamp_ms)
        .bind(timestamp_ms)
        .bind(account_id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| CommandError::operation("integration_cleanup_queue_failed", error))?;
    sqlx::query("DELETE FROM channel_provider_accounts WHERE id = ?")
        .bind(account_id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| CommandError::operation("integration_remove_failed", error))?;
    sqlx::query("DELETE FROM integration_provider_accounts WHERE id = ?")
        .bind(account_id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| CommandError::operation("integration_remove_failed", error))?;
    if let Some(connector_account_id) = connector_account_id {
        sqlx::query("DELETE FROM connector_accounts WHERE id = ?")
            .bind(connector_account_id)
            .execute(&mut *transaction)
            .await
            .map_err(|error| CommandError::operation("integration_remove_failed", error))?;
    }
    transaction
        .commit()
        .await
        .map_err(|error| CommandError::operation("integration_remove_failed", error))?;
    Ok(())
}

fn integration_keyring_entry(
    provider_id: IntegrationProviderId,
    account_id: &str,
) -> Result<keyring::Entry, keyring::Error> {
    keyring::Entry::new(
        KEYRING_SERVICE,
        &format!("{}:{account_id}:primary", provider_id.as_str()),
    )
}

async fn set_ilink_qr_state(
    state: &DesktopState,
    account_id: &str,
    value: &str,
) -> Result<(), CommandError> {
    sqlx::query("UPDATE integration_ilink_qr_sessions SET state = ?, updated_at_ms = ? WHERE account_id = ?")
        .bind(value)
        .bind(now_ms())
        .bind(account_id)
        .execute(state.agent_store.pool())
        .await
        .map_err(|error| CommandError::operation("ilink_qr_store_failed", error))?;
    Ok(())
}

fn normalize_qr_content(value: &str) -> String {
    if value.starts_with("data:image/") {
        value.into()
    } else {
        format!("data:image/png;base64,{value}")
    }
}

fn credential_reference(provider_id: IntegrationProviderId, account_id: &str) -> String {
    format!(
        "keyring:integration:{}:{account_id}:primary",
        provider_id.as_str()
    )
}

fn parse_provider_id(value: &str) -> Result<IntegrationProviderId, CommandError> {
    match value {
        "dingtalk" => Ok(IntegrationProviderId::DingTalk),
        "feishu" => Ok(IntegrationProviderId::Feishu),
        "wecom_ai_bot" => Ok(IntegrationProviderId::WecomAiBot),
        "wecom_app" => Ok(IntegrationProviderId::WecomApp),
        "wechat_ilink" => Ok(IntegrationProviderId::WechatIlink),
        _ => Err(persisted_invalid("provider", value)),
    }
}
fn parse_transport(value: &str) -> Result<IntegrationTransport, CommandError> {
    match value {
        "encrypted_callback" => Ok(IntegrationTransport::EncryptedCallback),
        "stream" => Ok(IntegrationTransport::Stream),
        "long_connection" => Ok(IntegrationTransport::LongConnection),
        "web_socket" => Ok(IntegrationTransport::WebSocket),
        "qr_long_poll" => Ok(IntegrationTransport::QrLongPoll),
        _ => Err(persisted_invalid("transport", value)),
    }
}
fn parse_account_state(value: &str) -> Result<ChannelAccountState, CommandError> {
    match value {
        "draft" => Ok(ChannelAccountState::Draft),
        "awaiting_auth" => Ok(ChannelAccountState::AwaitingAuth),
        "starting" => Ok(ChannelAccountState::Starting),
        "healthy" => Ok(ChannelAccountState::Healthy),
        "degraded" => Ok(ChannelAccountState::Degraded),
        "needs_attention" => Ok(ChannelAccountState::NeedsAttention),
        "revoked" => Ok(ChannelAccountState::Revoked),
        "removing" => Ok(ChannelAccountState::Removing),
        _ => Err(persisted_invalid("account state", value)),
    }
}
fn transport_str(value: IntegrationTransport) -> &'static str {
    match value {
        IntegrationTransport::EncryptedCallback => "encrypted_callback",
        IntegrationTransport::Stream => "stream",
        IntegrationTransport::LongConnection => "long_connection",
        IntegrationTransport::WebSocket => "web_socket",
        IntegrationTransport::QrLongPoll => "qr_long_poll",
    }
}
fn persisted_invalid(kind: &str, value: &str) -> CommandError {
    CommandError::new(
        "integration_persisted_value_invalid",
        format!("Invalid {kind}: {value}"),
    )
}
fn digest_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}
fn from_i64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dimension(ok: bool, code: &str) -> IntegrationProbeDimension {
        IntegrationProbeDimension {
            ok,
            result_code: code.into(),
            diagnostic: (!ok).then(|| code.into()),
        }
    }

    #[test]
    fn probe_dimensions_keep_ingress_independent_from_api_authentication() {
        let ingress = ingress_probe_dimension(true, true, true);
        let egress = egress_probe_dimension(true, true, false, true);
        let api = api_probe_dimension(true, false);
        assert_eq!(ingress.result_code, "ingress_healthy");
        assert!(ingress.ok);
        assert_eq!(egress.result_code, "egress_authentication_failed");
        assert!(!egress.ok);
        assert_eq!(api.result_code, "api_authentication_failed");
        assert!(!api.ok);
    }

    #[tokio::test]
    async fn probe_snapshot_survives_store_reconnect() {
        let root = tempfile::tempdir().expect("temporary database root");
        let database = root.path().join("agent.db");
        let store = hachimi_storage::AgentStore::connect(&database)
            .await
            .expect("store");
        sqlx::query("INSERT INTO integration_provider_accounts(id, provider_id, display_name, tenant_key, tenant_identity_hash, transport, state, api_access_enabled, messaging_enabled, config_json, credential_revision, config_revision, consecutive_failures, created_at_ms, updated_at_ms) VALUES('account-1', 'wecom_app', 'Account', 'tenant', 'tenant-hash', 'encrypted_callback', 'needs_attention', 1, 1, '{}', 1, 1, 0, 10, 10)")
            .execute(store.pool())
            .await
            .expect("account");
        let snapshot = IntegrationAccountProbeSnapshot {
            credential: test_dimension(false, "credential_authentication_failed"),
            ingress: test_dimension(true, "ingress_healthy"),
            egress: test_dimension(false, "egress_authentication_failed"),
            api: test_dimension(false, "api_authentication_failed"),
            probed_at_ms: 42,
        };
        store_probe_snapshot(store.pool(), "account-1", &snapshot)
            .await
            .expect("snapshot");
        drop(store);

        let reopened = hachimi_storage::AgentStore::connect(&database)
            .await
            .expect("reopened store");
        let loaded = load_probe_snapshot(reopened.pool(), "account-1")
            .await
            .expect("loaded snapshot");
        assert_eq!(loaded, Some(snapshot));
    }

    #[test]
    fn cleanup_reference_accepts_scoped_conversation_tokens() {
        let digest = "a".repeat(64);
        let secret_ref =
            format!("keyring:integration:wechat_ilink:account-1:conversation:{digest}");
        let expected = format!("wechat_ilink:account-1:conversation:{digest}");
        assert_eq!(
            cleanup_keyring_username(&secret_ref, "account-1").expect("valid reference"),
            Some(expected.as_str())
        );
        assert_eq!(
            cleanup_keyring_username(&secret_ref, "another-account").expect("scoped reference"),
            None
        );
    }

    #[tokio::test]
    async fn account_removal_queues_all_keyring_references_before_cascade() {
        let store = hachimi_storage::AgentStore::connect_in_memory()
            .await
            .expect("store");
        sqlx::query("INSERT INTO integration_provider_accounts(id, provider_id, display_name, tenant_key, tenant_identity_hash, transport, state, credential_ref, messaging_enabled, created_at_ms, updated_at_ms) VALUES('account-1', 'wechat_ilink', 'iLink', 'tenant', 'tenant-hash', 'qr_long_poll', 'healthy', 'keyring:integration:wechat_ilink:account-1:primary', 1, 1, 1)")
            .execute(store.pool())
            .await
            .expect("account");
        let conversation_ref = format!(
            "keyring:integration:wechat_ilink:account-1:conversation:{}",
            "b".repeat(64)
        );
        let media_ref = format!(
            "keyring:integration:wechat_ilink:account-1:media:{}",
            "c".repeat(64)
        );
        sqlx::query("INSERT INTO channel_route_secrets(account_id, conversation_hash, secret_ref, updated_at_ms) VALUES('account-1', 'conversation', ?, 1)")
            .bind(&conversation_ref)
            .execute(store.pool())
            .await
            .expect("route secret");
        sqlx::query("INSERT INTO channel_media_secrets(platform, account_id, event_id, remote_id, secret_ref, secret_fingerprint, created_at_ms) VALUES('wechat_ilink', 'account-1', 'event', 'media', ?, 'fingerprint', 1)")
            .bind(&media_ref)
            .execute(store.pool())
            .await
            .expect("media secret");
        stage_account_removal(
            store.pool(),
            "account-1",
            Some("keyring:integration:wechat_ilink:account-1:primary"),
        )
        .await
        .expect("stage removal");
        let queued: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM integration_secret_cleanup_queue WHERE account_id = 'account-1'",
        )
        .fetch_one(store.pool())
        .await
        .expect("cleanup count");
        let account_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM integration_provider_accounts WHERE id = 'account-1')",
        )
        .fetch_one(store.pool())
        .await
        .expect("account state");
        assert_eq!(queued, 3);
        assert!(!account_exists);
    }
}
