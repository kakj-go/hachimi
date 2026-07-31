use super::*;
use hachimi_control_plane::{
    BrowserAppRequest, BrowserAppResponse, ChannelAppRequest, ChannelAppResponse,
    ComputerAppRequest, ComputerAppResponse, ConnectorAppRequest, ConnectorAppResponse,
    GatewayAppRequest, GatewayAppResponse, PluginAppRequest, PluginAppResponse,
};
use hachimi_protocol::{
    BrowserHostSettings, BrowserProfileKind, CapabilityGrantSet, ComputerAppRule, RunId,
    SandboxCapabilityReport, ScheduleEventEnvelope, ScheduleEventResourceRef, ScheduleEventSource,
    ScheduleEventSourceKind, SessionId,
};
use sqlx::Row;

impl DesktopAppDomainHandler {
    pub(super) async fn dispatch_browser(
        &self,
        context: &AppServerContext,
        request: BrowserAppRequest,
    ) -> Result<BrowserAppResponse, AppServerDomainError> {
        self.require_runtime_feature(
            self.features.runtime_features.desktop_control,
            "desktop_control",
        )?;
        self.require_feature(self.features.browser_control, "browser_control_disabled")?;
        match request {
            BrowserAppRequest::BeginPairing => {
                Ok(BrowserAppResponse::Pairing(self.browser.begin_pairing()))
            }
            BrowserAppRequest::GetHostSettings => {
                let preferred = self.browser_preferred_profile().await?;
                Ok(BrowserAppResponse::HostSettings(BrowserHostSettings {
                    preferred_profile_kind: preferred,
                    latest_pairing: self.browser.latest_confirmed_pairing(),
                }))
            }
            BrowserAppRequest::ListPermissions => {
                let rows = sqlx::query("SELECT sessions.id AS browser_session_id, sessions.owner_session_id, sessions.owner_run_id, sessions.record_json, runs.generation, permissions.permission_json FROM browser_site_permissions AS permissions INNER JOIN browser_sessions AS sessions ON sessions.id = permissions.browser_session_id INNER JOIN runs ON runs.id = sessions.owner_run_id ORDER BY permissions.updated_at_ms DESC LIMIT 500")
                    .fetch_all(self.store.pool())
                    .await
                    .map_err(domain_error("browser_permission_list_failed"))?;
                let mut entries = Vec::with_capacity(rows.len());
                for row in rows {
                    let browser_session_id = hachimi_protocol::BrowserSessionId::new(
                        row.get::<String, _>("browser_session_id"),
                    );
                    let session: hachimi_protocol::BrowserSession =
                        serde_json::from_str(row.get("record_json"))
                            .map_err(domain_error("browser_session_decode_failed"))?;
                    let permission: hachimi_protocol::BrowserSitePermission =
                        serde_json::from_str(row.get("permission_json"))
                            .map_err(domain_error("browser_permission_decode_failed"))?;
                    if permission
                        .expires_at_ms
                        .is_some_and(|expires| expires <= now_ms())
                    {
                        continue;
                    }
                    let rule_rows = sqlx::query("SELECT rule_kind, allow_private_network, expires_at_ms FROM browser_network_rules WHERE browser_session_id = ? AND origin = ? ORDER BY rule_kind")
                        .bind(browser_session_id.as_str())
                        .bind(&permission.origin)
                        .fetch_all(self.store.pool())
                        .await
                        .map_err(domain_error("browser_network_rule_list_failed"))?;
                    let network_rules = rule_rows
                        .into_iter()
                        .map(|rule| hachimi_protocol::BrowserNetworkRule {
                            origin: permission.origin.clone(),
                            kind: if rule.get::<String, _>("rule_kind") == "document" {
                                hachimi_protocol::BrowserNetworkRuleKind::Document
                            } else {
                                hachimi_protocol::BrowserNetworkRuleKind::Resource
                            },
                            allow_private_network: rule.get("allow_private_network"),
                            expires_at_ms: rule.get("expires_at_ms"),
                        })
                        .collect();
                    entries.push(hachimi_protocol::BrowserPermissionLedgerEntry {
                        browser_session_id,
                        owner_session_id: hachimi_protocol::SessionId::new(
                            row.get::<String, _>("owner_session_id"),
                        ),
                        owner_run_id: hachimi_protocol::RunId::new(
                            row.get::<String, _>("owner_run_id"),
                        ),
                        run_generation: u64::try_from(row.get::<i64, _>("generation"))
                            .unwrap_or_default(),
                        browser_revision: session.revision,
                        permission,
                        network_rules,
                    });
                }
                Ok(BrowserAppResponse::Permissions(entries))
            }
            BrowserAppRequest::ListPermissionRequests => {
                self.capture_browser_network_denials().await?;
                let rows = sqlx::query("SELECT requests.id, requests.browser_session_id, requests.owner_session_id, requests.owner_run_id, requests.origin, requests.capabilities_json, requests.network_kind, requests.private_network, requests.status, requests.expected_browser_revision, requests.created_at_ms, requests.expires_at_ms, runs.generation FROM browser_permission_requests AS requests INNER JOIN runs ON runs.id = requests.owner_run_id ORDER BY requests.created_at_ms DESC LIMIT 500")
                    .fetch_all(self.store.pool())
                    .await
                    .map_err(domain_error("browser_permission_request_list_failed"))?;
                let requests = rows
                    .into_iter()
                    .map(decode_browser_permission_request)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(BrowserAppResponse::PermissionRequests(requests))
            }
            BrowserAppRequest::SetPreferredProfile(profile_kind) => {
                if profile_kind == BrowserProfileKind::ChromeExtension
                    && self.browser.latest_confirmed_pairing().is_none()
                {
                    return Err(AppServerDomainError::new(
                        "browser_pairing_required",
                        "Confirm an unexpired Chrome extension pairing before selecting this mode",
                    ));
                }
                sqlx::query("UPDATE browser_host_settings SET preferred_profile_kind = ?, updated_at_ms = ? WHERE singleton = 1")
                    .bind(match profile_kind {
                        BrowserProfileKind::Isolated => "isolated",
                        BrowserProfileKind::ChromeExtension => "chrome_extension",
                    })
                    .bind(now_ms())
                    .execute(self.store.pool())
                    .await
                    .map_err(domain_error("browser_host_settings_store_failed"))?;
                Ok(BrowserAppResponse::HostSettings(BrowserHostSettings {
                    preferred_profile_kind: profile_kind,
                    latest_pairing: self.browser.latest_confirmed_pairing(),
                }))
            }
            BrowserAppRequest::Start {
                session_id,
                run_id,
                profile_kind,
                initial_url,
                pairing_id,
            } => {
                let (_, sandbox, run_generation) = self.run_security(&session_id, &run_id).await?;
                let session = self
                    .browser
                    .start_session(
                        profile_kind,
                        session_id,
                        run_id,
                        run_generation,
                        initial_url.as_deref(),
                        &sandbox,
                        pairing_id.as_ref(),
                    )
                    .await
                    .map_err(domain_error("browser_start_failed"))?;
                self.persist_browser_session(&session).await?;
                self.store
                    .set_desktop_control_browser_session(
                        &session.owner_session_id,
                        Some(&session.id),
                        "observing",
                        now_ms(),
                    )
                    .await
                    .map_err(domain_error("desktop_control_browser_state_failed"))?;
                Ok(BrowserAppResponse::Session(session))
            }
            BrowserAppRequest::GrantSitePermission {
                context: mutation,
                session_id,
                run_id,
                browser_session_id,
                expected_revision,
                origin,
                capabilities,
                decision,
                network_kind,
                allow_private_network,
                expires_at_ms,
            } => {
                Self::validate_mutation(context, &mutation)?;
                if mutation.expected_run_id.as_ref() != Some(&run_id) {
                    return Err(AppServerDomainError::new(
                        "browser_permission_run_precondition_failed",
                        "Browser permission mutation is not bound to the active Run",
                    ));
                }
                let run = self
                    .store
                    .get_run(&run_id)
                    .await
                    .map_err(domain_error("browser_permission_run_lookup_failed"))?
                    .ok_or_else(|| {
                        AppServerDomainError::new("host_run_not_found", "Run does not exist")
                    })?;
                if mutation.expected_generation != Some(run.generation) {
                    return Err(AppServerDomainError::new(
                        "browser_permission_generation_precondition_failed",
                        "Browser permission mutation is stale",
                    ));
                }
                let (grants, sandbox, _) = self.run_security(&session_id, &run_id).await?;
                if sandbox.readiness != hachimi_protocol::SandboxReadiness::Ready
                    || !sandbox.os_enforced
                    || !sandbox.filesystem_enforced
                    || !sandbox.process_enforced
                    || !sandbox.network_enforced
                {
                    return Err(AppServerDomainError::new(
                        "browser_permission_sandbox_not_ready",
                        "Sandbox enforcement is not ready",
                    ));
                }
                if self.browser_owner_session(&browser_session_id).await? != session_id {
                    return Err(AppServerDomainError::new(
                        "browser_permission_session_precondition_failed",
                        "Browser Session ownership changed",
                    ));
                }
                let normalized_origin = hachimi_browser::normalized_origin(&origin)
                    .map_err(domain_error("browser_permission_origin_invalid"))?;
                let exact_origin_allowed = grants
                    .browser
                    .origins
                    .iter()
                    .any(|allowed| allowed == &normalized_origin)
                    || (grants.profile == hachimi_protocol::PermissionProfile::ExternalSandbox
                        && grants.browser.origins.is_empty());
                if !exact_origin_allowed
                    || capabilities.iter().any(|capability| match capability {
                        hachimi_protocol::BrowserCapability::Observe => !grants.browser.observe,
                        hachimi_protocol::BrowserCapability::Act => !grants.browser.act,
                        hachimi_protocol::BrowserCapability::Upload => !grants.browser.upload,
                        hachimi_protocol::BrowserCapability::Download => !grants.browser.download,
                        hachimi_protocol::BrowserCapability::CookieStorage => {
                            !grants.browser.cookie_storage
                        }
                        hachimi_protocol::BrowserCapability::Cdp => !grants.browser.cdp,
                    })
                {
                    return Err(AppServerDomainError::new(
                        "browser_permission_outside_run_grant",
                        "Requested Browser permission is outside the active Run grant",
                    ));
                }
                let permission = self
                    .browser
                    .grant_site_permission(
                        &browser_session_id,
                        &session_id,
                        &run_id,
                        expected_revision,
                        &origin,
                        capabilities,
                        decision,
                        network_kind,
                        allow_private_network,
                        &context.principal,
                        expires_at_ms,
                    )
                    .await
                    .map_err(domain_error("browser_permission_failed"))?;
                sqlx::query(
                    "INSERT INTO browser_site_permissions(browser_session_id, origin, permission_json, updated_at_ms) VALUES(?, ?, ?, ?) ON CONFLICT(browser_session_id, origin) DO UPDATE SET permission_json = excluded.permission_json, updated_at_ms = excluded.updated_at_ms",
                )
                .bind(browser_session_id.as_str())
                .bind(&permission.origin)
                .bind(serde_json::to_string(&permission).map_err(domain_error("browser_permission_encode_failed"))?)
                .bind(now_ms())
                .execute(self.store.pool())
                .await
                .map_err(domain_error("browser_permission_store_failed"))?;
                sqlx::query(
                    "INSERT INTO browser_network_rules(browser_session_id, origin, rule_kind, allow_private_network, expires_at_ms, revision, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?) ON CONFLICT(browser_session_id, origin, rule_kind) DO UPDATE SET allow_private_network = excluded.allow_private_network, expires_at_ms = excluded.expires_at_ms, revision = excluded.revision, updated_at_ms = excluded.updated_at_ms",
                )
                .bind(browser_session_id.as_str())
                .bind(&permission.origin)
                .bind(match network_kind {
                    hachimi_protocol::BrowserNetworkRuleKind::Document => "document",
                    hachimi_protocol::BrowserNetworkRuleKind::Resource => "resource",
                })
                .bind(allow_private_network)
                .bind(expires_at_ms)
                .bind(i64::try_from(expected_revision.saturating_add(1)).unwrap_or(i64::MAX))
                .bind(now_ms())
                .execute(self.store.pool())
                .await
                .map_err(domain_error("browser_network_rule_store_failed"))?;
                sqlx::query("UPDATE browser_permission_requests SET status = ?, updated_at_ms = ? WHERE browser_session_id = ? AND owner_run_id = ? AND origin = ? AND network_kind = ? AND status = 'pending'")
                    .bind(if decision == hachimi_protocol::BrowserPermissionDecision::Deny {
                        "denied"
                    } else {
                        "allowed"
                    })
                    .bind(now_ms())
                    .bind(browser_session_id.as_str())
                    .bind(run_id.as_str())
                    .bind(&permission.origin)
                    .bind(match network_kind {
                        hachimi_protocol::BrowserNetworkRuleKind::Document => "document",
                        hachimi_protocol::BrowserNetworkRuleKind::Resource => "resource",
                    })
                    .execute(self.store.pool())
                    .await
                    .map_err(domain_error("browser_permission_request_store_failed"))?;
                Ok(BrowserAppResponse::Permission(permission))
            }
            BrowserAppRequest::RevokeSitePermission {
                context: mutation,
                session_id,
                run_id,
                browser_session_id,
                expected_revision,
                origin,
            } => {
                Self::validate_mutation(context, &mutation)?;
                let run = self
                    .store
                    .get_run(&run_id)
                    .await
                    .map_err(domain_error("browser_permission_run_lookup_failed"))?
                    .ok_or_else(|| {
                        AppServerDomainError::new("host_run_not_found", "Run does not exist")
                    })?;
                if mutation.expected_run_id.as_ref() != Some(&run_id)
                    || mutation.expected_generation != Some(run.generation)
                    || self.browser_owner_session(&browser_session_id).await? != session_id
                {
                    return Err(AppServerDomainError::new(
                        "browser_permission_revoke_precondition_failed",
                        "Browser permission revoke precondition is stale",
                    ));
                }
                let normalized = hachimi_browser::normalized_origin(&origin)
                    .map_err(domain_error("browser_permission_origin_invalid"))?;
                let removed = self
                    .browser
                    .revoke_site_permission(
                        &browser_session_id,
                        &session_id,
                        &run_id,
                        expected_revision,
                        &normalized,
                    )
                    .await
                    .map_err(domain_error("browser_permission_revoke_failed"))?;
                let mut transaction = self
                    .store
                    .pool()
                    .begin()
                    .await
                    .map_err(domain_error("browser_permission_revoke_store_failed"))?;
                sqlx::query("DELETE FROM browser_site_permissions WHERE browser_session_id = ? AND origin = ?")
                    .bind(browser_session_id.as_str())
                    .bind(&normalized)
                    .execute(&mut *transaction)
                    .await
                    .map_err(domain_error("browser_permission_revoke_store_failed"))?;
                sqlx::query(
                    "DELETE FROM browser_network_rules WHERE browser_session_id = ? AND origin = ?",
                )
                .bind(browser_session_id.as_str())
                .bind(&normalized)
                .execute(&mut *transaction)
                .await
                .map_err(domain_error("browser_permission_revoke_store_failed"))?;
                transaction
                    .commit()
                    .await
                    .map_err(domain_error("browser_permission_revoke_store_failed"))?;
                Ok(BrowserAppResponse::PermissionRevoked(removed))
            }
            BrowserAppRequest::Observe {
                browser_session_id,
                run_id,
            } => {
                let session_id = self.browser_owner_session(&browser_session_id).await?;
                let (_, _, run_generation) = self.run_security(&session_id, &run_id).await?;
                let observation = self
                    .browser
                    .observe(&browser_session_id, &run_id, run_generation)
                    .await
                    .map_err(domain_error("browser_observe_failed"))?;
                self.store
                    .touch_desktop_control_observation(
                        &session_id,
                        "observing",
                        observation.created_at_ms,
                    )
                    .await
                    .map_err(domain_error("desktop_control_observation_state_failed"))?;
                Ok(BrowserAppResponse::Observation(observation))
            }
            BrowserAppRequest::Act { run_id, request } => {
                let owner_session = self
                    .browser_owner_session(&request.browser_session_id)
                    .await?;
                let action_id = self
                    .prepare_desktop_control_action(
                        &owner_session,
                        &run_id,
                        request.run_generation,
                        browser_action_kind(&request.action),
                        request.browser_session_id.as_str(),
                        &format!(
                            "{}:{}",
                            request.observation_id.as_str(),
                            request.expected_revision
                        ),
                        &request,
                    )
                    .await?;
                self.update_desktop_control_action(
                    &owner_session,
                    action_id.as_deref(),
                    "dispatched",
                    None,
                )
                .await?;
                let pending =
                    if let hachimi_protocol::BrowserAction::Navigate { url } = &request.action {
                        let origin = hachimi_browser::normalized_origin(url)
                            .map_err(domain_error("browser_permission_origin_invalid"))?;
                        self.browser
                            .session_snapshot(&request.browser_session_id, &run_id)
                            .ok()
                            .map(|session| (session, origin))
                    } else {
                        None
                    };
                match self.browser.authorize_action(&run_id, &request).await {
                    Ok(result) => {
                        self.update_desktop_control_action(
                            &owner_session,
                            action_id.as_deref(),
                            "completed",
                            Some(&result.result_code),
                        )
                        .await?;
                        self.store
                            .touch_desktop_control_observation(
                                &owner_session,
                                "observing",
                                now_ms(),
                            )
                            .await
                            .map_err(domain_error("desktop_control_action_state_failed"))?;
                        Ok(BrowserAppResponse::Action(result))
                    }
                    Err(hachimi_browser::BrowserHostError::PermissionMissing) => {
                        self.update_desktop_control_action(
                            &owner_session,
                            action_id.as_deref(),
                            "denied",
                            Some("browser_permission_required"),
                        )
                        .await?;
                        let Some((session, origin)) = pending else {
                            return Err(AppServerDomainError::new(
                                "browser_permission_required",
                                "Browser action requires an explicit site permission",
                            ));
                        };
                        let now = now_ms();
                        let expires_at_ms = now.saturating_add(10 * 60 * 1_000);
                        let existing = sqlx::query("SELECT id FROM browser_permission_requests WHERE browser_session_id = ? AND owner_run_id = ? AND origin = ? AND network_kind = 'document' AND status = 'pending' AND expires_at_ms > ? ORDER BY created_at_ms DESC LIMIT 1")
                            .bind(session.id.as_str())
                            .bind(run_id.as_str())
                            .bind(&origin)
                            .bind(now)
                            .fetch_optional(self.store.pool())
                            .await
                            .map_err(domain_error("browser_permission_request_lookup_failed"))?;
                        let request_id = existing
                            .map_or_else(hachimi_protocol::ItemId::random, |row| {
                                hachimi_protocol::ItemId::new(row.get::<String, _>("id"))
                            });
                        sqlx::query("INSERT OR IGNORE INTO browser_permission_requests(id, browser_session_id, owner_session_id, owner_run_id, origin, capabilities_json, network_kind, private_network, status, expected_browser_revision, created_at_ms, expires_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, 'document', 0, 'pending', ?, ?, ?, ?)")
                            .bind(request_id.as_str())
                            .bind(session.id.as_str())
                            .bind(session.owner_session_id.as_str())
                            .bind(run_id.as_str())
                            .bind(&origin)
                            .bind(serde_json::to_string(&vec![hachimi_protocol::BrowserCapability::Observe, hachimi_protocol::BrowserCapability::Act]).map_err(domain_error("browser_permission_request_encode_failed"))?)
                            .bind(i64::try_from(session.revision).unwrap_or(i64::MAX))
                            .bind(now)
                            .bind(expires_at_ms)
                            .bind(now)
                            .execute(self.store.pool())
                            .await
                            .map_err(domain_error("browser_permission_request_store_failed"))?;
                        let _ = self
                            .store
                            .append_event(
                                &session.owner_session_id,
                                Some(&run_id),
                                "browser.permission_required",
                                serde_json::json!({
                                    "requestId": request_id,
                                    "origin": origin,
                                    "networkKind": "document",
                                    "expiresAtMs": expires_at_ms
                                }),
                            )
                            .await;
                        Err(AppServerDomainError::new(
                            "browser_permission_required",
                            format!(
                                "Browser origin permission {request_id} requires user confirmation"
                            ),
                        ))
                    }
                    Err(error) => {
                        let (status, result_code) = browser_action_error_status(&error);
                        self.update_desktop_control_action(
                            &owner_session,
                            action_id.as_deref(),
                            status,
                            Some(result_code),
                        )
                        .await?;
                        Err(domain_error("browser_action_failed")(error))
                    }
                }
            }
            BrowserAppRequest::StageUpload {
                browser_session_id,
                run_id,
                source,
            } => {
                let session_id = self.browser_owner_session(&browser_session_id).await?;
                let _ = self.run_security(&session_id, &run_id).await?;
                self.browser
                    .stage_upload(&browser_session_id, &run_id, &source)
                    .await
                    .map(BrowserAppResponse::FileToken)
                    .map_err(domain_error("browser_upload_stage_failed"))
            }
            BrowserAppRequest::ImportDownload {
                browser_session_id,
                run_id,
                download_token,
                destination,
            } => {
                let session_id = self.browser_owner_session(&browser_session_id).await?;
                let _ = self.run_security(&session_id, &run_id).await?;
                self.browser
                    .import_download(&browser_session_id, &run_id, &download_token, &destination)
                    .await
                    .map(BrowserAppResponse::ImportedDownload)
                    .map_err(domain_error("browser_download_import_failed"))
            }
            BrowserAppRequest::TakeOver {
                browser_session_id,
                run_id,
            } => {
                let session = self
                    .browser
                    .take_over(&browser_session_id, &run_id)
                    .await
                    .map_err(domain_error("browser_takeover_failed"))?;
                self.persist_browser_session(&session).await?;
                self.invalidate_browser_permission_ledger(&browser_session_id)
                    .await?;
                self.store
                    .set_desktop_control_browser_session(
                        &session.owner_session_id,
                        None,
                        "taken_over",
                        now_ms(),
                    )
                    .await
                    .map_err(domain_error("desktop_control_browser_state_failed"))?;
                Ok(BrowserAppResponse::Session(session))
            }
            BrowserAppRequest::Stop {
                browser_session_id,
                run_id,
            } => {
                let session = self
                    .browser
                    .stop(&browser_session_id, &run_id)
                    .await
                    .map_err(domain_error("browser_stop_failed"))?;
                self.persist_browser_session(&session).await?;
                self.invalidate_browser_permission_ledger(&browser_session_id)
                    .await?;
                self.store
                    .set_desktop_control_browser_session(
                        &session.owner_session_id,
                        None,
                        "stopped",
                        now_ms(),
                    )
                    .await
                    .map_err(domain_error("desktop_control_browser_state_failed"))?;
                Ok(BrowserAppResponse::Session(session))
            }
        }
    }

    pub(super) async fn dispatch_computer(
        &self,
        context: &AppServerContext,
        request: ComputerAppRequest,
    ) -> Result<ComputerAppResponse, AppServerDomainError> {
        self.require_runtime_feature(
            self.features.runtime_features.desktop_control,
            "desktop_control",
        )?;
        match request {
            ComputerAppRequest::ListWindows => {
                self.require_feature(self.features.computer_observe, "computer_observe_disabled")?;
                self.computer
                    .list_windows()
                    .await
                    .map(ComputerAppResponse::Windows)
                    .map_err(domain_error("computer_window_list_failed"))
            }
            ComputerAppRequest::ListGlobalAppRules => {
                self.require_feature(self.features.computer_observe, "computer_observe_disabled")?;
                let rows =
                    sqlx::query("SELECT rule_json FROM computer_global_app_rules ORDER BY app_id")
                        .fetch_all(self.store.pool())
                        .await
                        .map_err(domain_error("computer_global_rule_list_failed"))?;
                let rules = rows
                    .into_iter()
                    .map(|row| serde_json::from_str::<ComputerAppRule>(row.get("rule_json")))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(domain_error("computer_global_rule_decode_failed"))?;
                Ok(ComputerAppResponse::Rules(rules))
            }
            ComputerAppRequest::SetGlobalAppRule(mut rule) => {
                self.require_feature(self.features.computer_observe, "computer_observe_disabled")?;
                if rule.app_id.trim().is_empty() || rule.app_id.chars().count() > 512 {
                    return Err(AppServerDomainError::new(
                        "computer_app_id_invalid",
                        "Computer app ID must contain 1-512 characters",
                    ));
                }
                rule.always_allowed = true;
                rule.granted_by = context.principal.clone();
                rule.updated_at_ms = now_ms();
                sqlx::query(
                    "INSERT INTO computer_global_app_rules(app_id, rule_json, updated_at_ms) VALUES(?, ?, ?) ON CONFLICT(app_id) DO UPDATE SET rule_json = excluded.rule_json, updated_at_ms = excluded.updated_at_ms",
                )
                .bind(&rule.app_id)
                .bind(serde_json::to_string(&rule).map_err(domain_error("computer_global_rule_encode_failed"))?)
                .bind(rule.updated_at_ms)
                .execute(self.store.pool())
                .await
                .map_err(domain_error("computer_global_rule_store_failed"))?;
                Ok(ComputerAppResponse::Rule(rule))
            }
            ComputerAppRequest::RemoveGlobalAppRule(app_id) => {
                self.require_feature(self.features.computer_observe, "computer_observe_disabled")?;
                let removed = sqlx::query("DELETE FROM computer_global_app_rules WHERE app_id = ?")
                    .bind(app_id)
                    .execute(self.store.pool())
                    .await
                    .map_err(domain_error("computer_global_rule_remove_failed"))?
                    .rows_affected()
                    > 0;
                Ok(ComputerAppResponse::Removed(removed))
            }
            ComputerAppRequest::SetAppRule {
                session_id,
                mut rule,
            } => {
                self.require_feature(self.features.computer_observe, "computer_observe_disabled")?;
                if self
                    .store
                    .get_session(&session_id)
                    .await
                    .map_err(domain_error("computer_session_lookup_failed"))?
                    .is_none()
                {
                    return Err(AppServerDomainError::new(
                        "computer_session_not_found",
                        "Session does not exist",
                    ));
                }
                rule.granted_by = context.principal.clone();
                rule.updated_at_ms = now_ms();
                self.persist_computer_rule(&session_id, &rule).await?;
                self.computer.set_app_rule(&session_id, rule.clone());
                Ok(ComputerAppResponse::Rule(rule))
            }
            ComputerAppRequest::Observe {
                session_id,
                run_id,
                window_handle,
            } => {
                self.require_feature(self.features.computer_observe, "computer_observe_disabled")?;
                let (grants, sandbox, run_generation) =
                    self.run_security(&session_id, &run_id).await?;
                self.load_computer_global_rules(&session_id).await?;
                self.load_computer_rules(&session_id).await?;
                let frame = self
                    .computer
                    .observe(
                        session_id,
                        run_id,
                        run_generation,
                        &window_handle,
                        &grants,
                        &sandbox,
                    )
                    .await
                    .map_err(domain_error("computer_observe_failed"))?;
                self.store
                    .set_desktop_control_computer_observation(
                        &frame.session_id,
                        Some(&frame.target.app_id),
                        Some(&frame.target.fingerprint),
                        frame.input_epoch,
                        "observing",
                        Some(frame.created_at_ms),
                        now_ms(),
                    )
                    .await
                    .map_err(domain_error("desktop_control_computer_state_failed"))?;
                Ok(ComputerAppResponse::Frame(frame))
            }
            ComputerAppRequest::Act { run_id, request } => {
                self.require_feature(self.features.computer_control, "computer_control_disabled")?;
                let frame = self
                    .computer
                    .frame_snapshot(&request.frame_id)
                    .ok_or_else(|| {
                        AppServerDomainError::new(
                            "computer_frame_not_found",
                            "Computer frame does not exist or already expired",
                        )
                    })?;
                let action_id = self
                    .prepare_desktop_control_action(
                        &frame.session_id,
                        &run_id,
                        request.run_generation,
                        computer_action_kind(&request.action),
                        &frame.target.fingerprint,
                        &format!(
                            "{}:{}",
                            request.frame_id.as_str(),
                            request.expected_input_epoch
                        ),
                        &request,
                    )
                    .await?;
                let grants = self
                    .store
                    .latest_active_capability_grants(&run_id)
                    .await
                    .map_err(domain_error("computer_grant_lookup_failed"))?
                    .ok_or_else(|| {
                        AppServerDomainError::new(
                            "computer_grant_missing",
                            "active Run grant is unavailable",
                        )
                    })?;
                self.update_desktop_control_action(
                    &frame.session_id,
                    action_id.as_deref(),
                    "dispatched",
                    None,
                )
                .await?;
                match self.computer.act(&request, &grants).await {
                    Ok(result) => {
                        self.update_desktop_control_action(
                            &frame.session_id,
                            action_id.as_deref(),
                            "completed",
                            Some(&result.result_code),
                        )
                        .await?;
                        self.store
                            .set_desktop_control_computer_observation(
                                &frame.session_id,
                                Some(&frame.target.app_id),
                                Some(&frame.target.fingerprint),
                                result.next_input_epoch,
                                "observing",
                                None,
                                now_ms(),
                            )
                            .await
                            .map_err(domain_error("desktop_control_computer_state_failed"))?;
                        Ok(ComputerAppResponse::Action(result))
                    }
                    Err(error) => {
                        let (status, result_code) = computer_action_error_status(&error);
                        self.update_desktop_control_action(
                            &frame.session_id,
                            action_id.as_deref(),
                            status,
                            Some(result_code),
                        )
                        .await?;
                        Err(domain_error("computer_action_failed")(error))
                    }
                }
            }
            ComputerAppRequest::TakeOver(session_id) => {
                let next_epoch = self.computer.take_over(&session_id);
                self.store
                    .set_desktop_control_computer_observation(
                        &session_id,
                        None,
                        None,
                        next_epoch,
                        "taken_over",
                        None,
                        now_ms(),
                    )
                    .await
                    .map_err(domain_error("desktop_control_computer_state_failed"))?;
                Ok(ComputerAppResponse::TakenOver(next_epoch))
            }
        }
    }

    pub(super) async fn dispatch_plugin(
        &self,
        request: PluginAppRequest,
    ) -> Result<PluginAppResponse, AppServerDomainError> {
        self.require_runtime_feature(
            self.features.runtime_features.plugin_runtime,
            "plugin_runtime",
        )?;
        match request {
            PluginAppRequest::InstallLocal(path) => {
                let plugin = self
                    .plugins
                    .install_local(path)
                    .await
                    .map_err(domain_error("plugin_install_failed"))?;
                self.refresh_plugin_sidecar_drivers().await?;
                if let Err(error) = self.reconcile_plugin_products(&plugin, false).await {
                    if let Ok(restored) = self.plugins.rollback(&plugin.manifest.id, None).await {
                        let _ = self
                            .reconcile_plugin_products(
                                &restored,
                                restored.status == hachimi_protocol::PluginStatus::Enabled,
                            )
                            .await;
                    } else {
                        let _ = self.remove_plugin_products(&plugin.manifest.id).await;
                        let _ = self.plugins.uninstall(&plugin.manifest.id).await;
                    }
                    return Err(error);
                }
                self.refresh_plugin_skill_roots().await?;
                Ok(PluginAppResponse::Plugin(plugin))
            }
            PluginAppRequest::List => self
                .plugins
                .list()
                .await
                .map(PluginAppResponse::Plugins)
                .map_err(domain_error("plugin_list_failed")),
            PluginAppRequest::ListContributions(plugin_id) => self
                .plugins
                .list_contributions(plugin_id.as_ref())
                .await
                .map(PluginAppResponse::Contributions)
                .map_err(domain_error("plugin_contribution_list_failed")),
            PluginAppRequest::GetContributionSurface {
                plugin_id,
                contribution_id,
            } => self
                .plugin_contribution_surface(&plugin_id, &contribution_id)
                .await
                .map(PluginAppResponse::ContributionSurface),
            PluginAppRequest::Get(id) => self
                .plugins
                .get(&id)
                .await
                .map(PluginAppResponse::OptionalPlugin)
                .map_err(domain_error("plugin_get_failed")),
            PluginAppRequest::HealthCheck(id) => self
                .plugins
                .health_check(&id)
                .await
                .map(PluginAppResponse::Plugin)
                .map_err(domain_error("plugin_health_failed")),
            PluginAppRequest::PermissionDiff(id) => self
                .plugins
                .permission_diff(&id)
                .await
                .map(PluginAppResponse::PermissionDiff)
                .map_err(domain_error("plugin_permission_diff_failed")),
            PluginAppRequest::RevisionHead(id) => self
                .plugins
                .revision_head(&id)
                .await
                .map(PluginAppResponse::RevisionHead)
                .map_err(domain_error("plugin_revision_head_failed")),
            PluginAppRequest::ListRevisions(id) => self
                .plugins
                .list_revisions(&id)
                .await
                .map(PluginAppResponse::Revisions)
                .map_err(domain_error("plugin_revision_list_failed")),
            PluginAppRequest::LifecycleJournal(id) => self
                .plugins
                .lifecycle_journal(id.as_ref())
                .await
                .map(PluginAppResponse::LifecycleJournal)
                .map_err(domain_error("plugin_lifecycle_journal_failed")),
            PluginAppRequest::SetEnabled { plugin_id, enabled } => {
                let current = self
                    .plugins
                    .get(&plugin_id)
                    .await
                    .map_err(domain_error("plugin_get_failed"))?
                    .ok_or_else(|| {
                        AppServerDomainError::new("plugin_not_found", "Plugin is not installed")
                    })?;
                let was_enabled = current.status == hachimi_protocol::PluginStatus::Enabled;
                if enabled {
                    self.refresh_plugin_sidecar_drivers().await?;
                    self.reconcile_plugin_products(&current, false).await?;
                }
                let plugin = self
                    .plugins
                    .set_enabled(&plugin_id, enabled)
                    .await
                    .map_err(domain_error("plugin_enable_failed"))?;
                if let Err(error) = self.reconcile_plugin_products(&plugin, enabled).await {
                    if let Ok(restored) = self.plugins.set_enabled(&plugin_id, was_enabled).await {
                        let _ = self.reconcile_plugin_products(&restored, was_enabled).await;
                    }
                    return Err(error);
                }
                self.refresh_plugin_skill_roots().await?;
                Ok(PluginAppResponse::Plugin(plugin))
            }
            PluginAppRequest::Rollback {
                plugin_id,
                revision,
            } => {
                self.remove_plugin_products(&plugin_id).await?;
                let plugin = self
                    .plugins
                    .rollback(&plugin_id, revision.as_deref())
                    .await
                    .map_err(domain_error("plugin_rollback_failed"))?;
                self.refresh_plugin_sidecar_drivers().await?;
                self.reconcile_plugin_products(
                    &plugin,
                    plugin.status == hachimi_protocol::PluginStatus::Enabled,
                )
                .await?;
                self.refresh_plugin_skill_roots().await?;
                Ok(PluginAppResponse::Plugin(plugin))
            }
            PluginAppRequest::Uninstall(id) => {
                self.remove_plugin_products(&id).await?;
                let removed = self
                    .plugins
                    .uninstall(&id)
                    .await
                    .map_err(domain_error("plugin_uninstall_failed"))?;
                self.refresh_plugin_skill_roots().await?;
                Ok(PluginAppResponse::Removed(removed))
            }
        }
    }

    pub(super) async fn refresh_plugin_skill_roots(&self) -> Result<(), AppServerDomainError> {
        let mut roots = self.skills.catalog_roots();
        roots.retain(|root| {
            !root
                .namespace
                .as_deref()
                .is_some_and(|namespace| namespace.starts_with("plugin-"))
        });
        for (plugin_id, path) in self
            .plugins
            .enabled_skill_roots()
            .await
            .map_err(domain_error("plugin_skill_roots_failed"))?
        {
            roots.push(
                hachimi_skills::SkillCatalogRoot::new(path, hachimi_protocol::SkillScope::System)
                    .with_namespace(plugin_skill_namespace(plugin_id.as_str())),
            );
        }
        self.skills
            .set_catalog_roots(roots)
            .map_err(domain_error("plugin_skill_roots_failed"))
    }

    pub(super) async fn refresh_plugin_sidecar_drivers(&self) -> Result<(), AppServerDomainError> {
        if self.sandbox_runtime.snapshot().report.backend == "desktop-e2e-deterministic" {
            return Ok(());
        }
        let backend: Arc<dyn hachimi_sandbox::SandboxBackend> = self.sandbox_runtime.clone();
        self.plugins
            .register_sidecar_drivers(backend)
            .await
            .map(|_| ())
            .map_err(domain_error("plugin_sidecar_register_failed"))
    }

    pub(super) async fn dispatch_connector(
        &self,
        request: ConnectorAppRequest,
    ) -> Result<ConnectorAppResponse, AppServerDomainError> {
        self.require_runtime_feature(
            self.features.runtime_features.plugin_runtime,
            "plugin_runtime",
        )?;
        match request {
            ConnectorAppRequest::UpsertAccount(account) => {
                self.require_enterprise_connector_feature(&account.plugin_id)?;
                self.plugins
                    .upsert_connector_account(account)
                    .await
                    .map(ConnectorAppResponse::Account)
                    .map_err(domain_error("connector_account_upsert_failed"))
            }
            ConnectorAppRequest::ListAccounts => {
                let mut accounts = self
                    .plugins
                    .list_connector_accounts()
                    .await
                    .map_err(domain_error("connector_account_list_failed"))?;
                if !self.features.runtime_features.enterprise_integrations {
                    accounts.retain(|account| !is_enterprise_plugin(&account.plugin_id));
                }
                Ok(ConnectorAppResponse::Accounts(accounts))
            }
            ConnectorAppRequest::GetAccount(id) => {
                let account = self
                    .plugins
                    .connector_account(&id)
                    .await
                    .map_err(domain_error("connector_account_get_failed"))?;
                if let Some(account) = account.as_ref() {
                    self.require_enterprise_connector_feature(&account.plugin_id)?;
                }
                Ok(ConnectorAppResponse::OptionalAccount(account))
            }
            ConnectorAppRequest::GetDriverDescriptor {
                plugin_id,
                connector_id,
            } => {
                self.require_enterprise_connector_feature(&plugin_id)?;
                self.plugins
                    .connector_driver_descriptor(&plugin_id, &connector_id)
                    .await
                    .map(ConnectorAppResponse::DriverDescriptor)
                    .map_err(domain_error("connector_driver_descriptor_failed"))
            }
            ConnectorAppRequest::RevokeAccount(id) => {
                let account = self
                    .plugins
                    .connector_account(&id)
                    .await
                    .map_err(domain_error("connector_account_get_failed"))?
                    .ok_or_else(|| {
                        AppServerDomainError::new(
                            "connector_account_not_found",
                            "Connector account does not exist",
                        )
                    })?;
                self.require_enterprise_connector_feature(&account.plugin_id)?;
                self.plugins
                    .revoke_connector_account(&id)
                    .await
                    .map(ConnectorAppResponse::Account)
                    .map_err(domain_error("connector_account_revoke_failed"))
            }
            ConnectorAppRequest::Invoke(request) => {
                let account = self
                    .plugins
                    .connector_account(&request.account_id)
                    .await
                    .map_err(domain_error("connector_account_get_failed"))?
                    .ok_or_else(|| {
                        AppServerDomainError::new(
                            "connector_account_not_found",
                            "Connector account does not exist",
                        )
                    })?;
                self.require_enterprise_connector_feature(&account.plugin_id)?;
                self.plugins
                    .invoke_connector(&request)
                    .await
                    .map(ConnectorAppResponse::Invocation)
                    .map_err(domain_error("connector_invoke_failed"))
            }
        }
    }

    pub(super) async fn dispatch_channel(
        &self,
        context: &AppServerContext,
        request: ChannelAppRequest,
    ) -> Result<ChannelAppResponse, AppServerDomainError> {
        self.require_feature(self.features.local_gateway, "local_gateway_disabled")?;
        match request {
            ChannelAppRequest::DispatchIngress { envelope } => {
                if is_enterprise_provider_id(&envelope.route.channel) {
                    self.require_runtime_feature(
                        self.features.runtime_features.enterprise_integrations,
                        "enterprise_integrations",
                    )?;
                }
                if !context.principal.starts_with("channel:") {
                    return Err(AppServerDomainError::new(
                        "channel_principal_invalid",
                        "authenticated channel principal is required",
                    ));
                }
                if matches!(
                    envelope.route.channel.as_str(),
                    "wecom" | "ding_talk" | "feishu"
                ) {
                    let event_type = envelope
                        .metadata
                        .get("eventType")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("message.received")
                        .to_owned();
                    let revision = envelope
                        .metadata
                        .get("payloadHash")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned);
                    self.scheduler
                        .ingest_event(ScheduleEventEnvelope {
                            event_id: envelope.message_id.as_str().to_owned(),
                            source: ScheduleEventSource {
                                kind: ScheduleEventSourceKind::Channel,
                                principal: context.principal.clone(),
                                id: format!(
                                    "enterprise:{}:{}",
                                    envelope.route.channel, envelope.route.account
                                ),
                            },
                            event_type,
                            subject: Some(envelope.route.peer.clone()),
                            labels: BTreeMap::from([
                                ("platform".into(), envelope.route.channel.clone()),
                                ("account".into(), envelope.route.account.clone()),
                            ]),
                            resource: Some(ScheduleEventResourceRef {
                                kind: "enterprise_message".into(),
                                id: envelope.message_id.as_str().to_owned(),
                                revision,
                            }),
                            occurred_at_ms: envelope.received_at_ms,
                        })
                        .await
                        .map_err(domain_error("enterprise_event_ingress_failed"))?;
                }
                self.run_launcher
                    .dispatch_channel_ingress(context.principal.clone(), envelope)
                    .await
                    .map(ChannelAppResponse::Ingress)
            }
            ChannelAppRequest::LoopbackReceive {
                bearer_token,
                envelope,
            } => self
                .loopback_channel
                .receive(&self.gateway, &bearer_token, envelope)
                .await
                .map(ChannelAppResponse::Ingress)
                .map_err(domain_error("loopback_receive_failed")),
            ChannelAppRequest::MockPollPush(envelope) => {
                self.mock_poll_channel
                    .push(envelope)
                    .await
                    .map_err(domain_error("mock_poll_push_failed"))?;
                Ok(ChannelAppResponse::Ingresses(Vec::new()))
            }
            ChannelAppRequest::MockPollSetConnected(connected) => {
                self.mock_poll_channel.set_connected(connected);
                Ok(ChannelAppResponse::MockPollConnected(connected))
            }
            ChannelAppRequest::MockPollDrain => self
                .mock_poll_channel
                .drain(&self.gateway)
                .await
                .map(ChannelAppResponse::Ingresses)
                .map_err(domain_error("mock_poll_drain_failed")),
            ChannelAppRequest::EnqueueDelivery {
                route,
                idempotency_key,
                text,
            } => {
                if is_enterprise_provider_id(&route.channel) {
                    self.require_runtime_feature(
                        self.features.runtime_features.enterprise_integrations,
                        "enterprise_integrations",
                    )?;
                }
                self.gateway
                    .enqueue_delivery(route, &idempotency_key, &text, now_ms())
                    .await
                    .map(ChannelAppResponse::Delivery)
                    .map_err(domain_error("channel_delivery_enqueue_failed"))
            }
        }
    }

    pub(super) async fn dispatch_gateway(
        &self,
        request: GatewayAppRequest,
    ) -> Result<GatewayAppResponse, AppServerDomainError> {
        self.require_feature(self.features.local_gateway, "local_gateway_disabled")?;
        match request {
            GatewayAppRequest::Health => self
                .gateway
                .health()
                .await
                .map(GatewayAppResponse::Health)
                .map_err(domain_error("gateway_health_failed")),
            GatewayAppRequest::ProviderManifests => self
                .gateway
                .provider_manifests()
                .await
                .map(GatewayAppResponse::ProviderManifests)
                .map_err(domain_error("gateway_provider_manifest_list_failed")),
            GatewayAppRequest::ProviderHealth => self
                .gateway
                .provider_health()
                .await
                .map(GatewayAppResponse::ProviderHealth)
                .map_err(domain_error("gateway_provider_health_failed")),
            GatewayAppRequest::ListProviderAccounts => self
                .gateway
                .list_provider_accounts()
                .await
                .map(GatewayAppResponse::ProviderAccounts)
                .map_err(domain_error("gateway_provider_account_list_failed")),
            GatewayAppRequest::UpsertProviderAccount(input) => {
                if is_enterprise_provider_id(&input.provider_id) {
                    self.require_runtime_feature(
                        self.features.runtime_features.enterprise_integrations,
                        "enterprise_integrations",
                    )?;
                }
                let preserve_existing_secret = input.credential.is_none();
                let secret_ref = if let Some(credential) = input.credential.as_deref() {
                    let entry = keyring::Entry::new(
                        "com.hachimi.channel",
                        &format!("{}:{}", input.provider_id, input.id),
                    )
                    .map_err(domain_error("gateway_credential_store_failed"))?;
                    if credential.is_empty() {
                        let _ = entry.delete_credential();
                        None
                    } else {
                        entry
                            .set_password(credential)
                            .map_err(domain_error("gateway_credential_store_failed"))?;
                        Some(format!(
                            "keyring:channel:{}:{}",
                            input.provider_id, input.id
                        ))
                    }
                } else {
                    None
                };
                self.gateway
                    .upsert_provider_account(
                        hachimi_protocol::ChannelProviderAccount {
                            id: input.id,
                            provider_id: input.provider_id,
                            display_name: input.display_name,
                            secret_ref,
                            enabled: input.enabled,
                            route_allowlist: input.route_allowlist,
                            config_revision: input.expected_config_revision.unwrap_or_default(),
                        },
                        input.expected_config_revision,
                        preserve_existing_secret,
                    )
                    .await
                    .map(GatewayAppResponse::ProviderAccount)
                    .map_err(domain_error("gateway_provider_account_upsert_failed"))
            }
            GatewayAppRequest::SetStartupEnabled(enabled) => {
                let executable = std::env::current_exe()
                    .map_err(domain_error("gateway_executable_lookup_failed"))?;
                let mut health = self
                    .gateway
                    .set_startup_registration(&executable, enabled, now_ms())
                    .await
                    .map_err(domain_error("gateway_startup_update_failed"))?;
                if enabled {
                    crate::gateway_process::ensure_running(&executable)
                        .map_err(domain_error("gateway_process_start_failed"))?;
                    for _ in 0..20 {
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        health = self
                            .gateway
                            .health()
                            .await
                            .map_err(domain_error("gateway_health_failed"))?;
                        if health.running {
                            break;
                        }
                    }
                }
                Ok(GatewayAppResponse::Health(health))
            }
            GatewayAppRequest::Reconcile => {
                self.gateway
                    .reconcile_startup(now_ms())
                    .await
                    .map_err(domain_error("gateway_reconcile_failed"))?;
                Ok(GatewayAppResponse::Reconciled)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn prepare_desktop_control_action<T: serde::Serialize>(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        expected_generation: u64,
        action_kind: &str,
        target_fingerprint: &str,
        observation_revision: &str,
        payload: &T,
    ) -> Result<Option<String>, AppServerDomainError> {
        if !self
            .store
            .desktop_control_session_exists(session_id)
            .await
            .map_err(domain_error("desktop_control_session_lookup_failed"))?
        {
            return Ok(None);
        }
        let run = self
            .store
            .get_run(run_id)
            .await
            .map_err(domain_error("desktop_control_run_lookup_failed"))?
            .filter(|run| &run.session_id == session_id)
            .ok_or_else(|| {
                AppServerDomainError::new(
                    "desktop_control_run_precondition_failed",
                    "DesktopControl Run ownership changed",
                )
            })?;
        if run.generation != expected_generation {
            return Err(AppServerDomainError::new(
                "stale_run_generation",
                "DesktopControl action belongs to an expired Run generation",
            ));
        }
        let action_id = mutation_fingerprint(
            action_kind,
            &(run_id.as_str(), observation_revision, payload),
        )?;
        let target_fingerprint_hash =
            mutation_fingerprint("desktop-control-target", &target_fingerprint)?;
        let input = hachimi_storage::DesktopControlActionLedgerInput {
            session_id: session_id.clone(),
            run_id: run_id.clone(),
            generation: run.generation,
            action_id: action_id.clone(),
            action_kind: action_kind.to_owned(),
            target_fingerprint_hash,
            observation_revision: observation_revision.to_owned(),
            now_ms: now_ms(),
        };
        let inserted = self
            .store
            .prepare_desktop_control_action(&input)
            .await
            .map_err(domain_error("desktop_control_action_prepare_failed"))?;
        if !inserted {
            return Err(AppServerDomainError::new(
                "desktop_control_action_duplicate",
                "DesktopControl action was already claimed and will not be replayed",
            ));
        }
        self.store
            .update_desktop_control_action(
                session_id,
                &action_id,
                "approved",
                Some("run_grant_and_product_scope"),
                now_ms(),
            )
            .await
            .map_err(domain_error("desktop_control_action_approve_failed"))?;
        Ok(Some(action_id))
    }

    async fn update_desktop_control_action(
        &self,
        session_id: &SessionId,
        action_id: Option<&str>,
        status: &str,
        result_code: Option<&str>,
    ) -> Result<(), AppServerDomainError> {
        let Some(action_id) = action_id else {
            return Ok(());
        };
        self.store
            .update_desktop_control_action(session_id, action_id, status, result_code, now_ms())
            .await
            .map_err(domain_error("desktop_control_action_update_failed"))
    }

    fn require_feature(
        &self,
        enabled: bool,
        code: &'static str,
    ) -> Result<(), AppServerDomainError> {
        enabled.then_some(()).ok_or_else(|| {
            AppServerDomainError::new(code, "feature is disabled by its local kill switch")
        })
    }

    fn require_runtime_feature(
        &self,
        enabled: bool,
        feature_key: &'static str,
    ) -> Result<(), AppServerDomainError> {
        enabled
            .then_some(())
            .ok_or_else(|| AppServerDomainError::new("feature_disabled", feature_key))
    }

    fn require_enterprise_connector_feature(
        &self,
        plugin_id: &hachimi_protocol::PluginId,
    ) -> Result<(), AppServerDomainError> {
        if is_enterprise_plugin(plugin_id) {
            self.require_runtime_feature(
                self.features.runtime_features.enterprise_integrations,
                "enterprise_integrations",
            )?;
        }
        Ok(())
    }

    async fn run_security(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
    ) -> Result<(CapabilityGrantSet, SandboxCapabilityReport, u64), AppServerDomainError> {
        let run = self
            .store
            .get_run(run_id)
            .await
            .map_err(domain_error("host_run_lookup_failed"))?
            .ok_or_else(|| AppServerDomainError::new("host_run_not_found", "Run does not exist"))?;
        if &run.session_id != session_id || run.status.is_terminal() {
            return Err(AppServerDomainError::new(
                "host_run_precondition_failed",
                "Run ownership changed or the Run is terminal",
            ));
        }
        let grants = self
            .store
            .latest_active_capability_grants(run_id)
            .await
            .map_err(domain_error("host_grant_lookup_failed"))?
            .ok_or_else(|| {
                AppServerDomainError::new("host_grant_missing", "active Run grant is unavailable")
            })?;
        let sandbox = self
            .store
            .latest_sandbox_report(run_id)
            .await
            .map_err(domain_error("host_sandbox_lookup_failed"))?
            .ok_or_else(|| {
                AppServerDomainError::new(
                    "host_sandbox_missing",
                    "Run Sandbox snapshot is unavailable",
                )
            })?;
        if grants.session_id != *session_id || grants.run_id.as_ref() != Some(run_id) {
            return Err(AppServerDomainError::new(
                "host_security_lineage_failed",
                "Run security snapshot lineage does not match",
            ));
        }
        Ok((grants, sandbox, run.generation))
    }

    async fn persist_browser_session(
        &self,
        session: &hachimi_protocol::BrowserSession,
    ) -> Result<(), AppServerDomainError> {
        sqlx::query(
            "INSERT INTO browser_sessions(id, owner_session_id, owner_run_id, record_json, updated_at_ms, owner_run_generation) VALUES(?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET record_json = excluded.record_json, owner_run_generation = excluded.owner_run_generation, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(session.id.as_str())
        .bind(session.owner_session_id.as_str())
        .bind(session.owner_run_id.as_str())
        .bind(serde_json::to_string(session).map_err(domain_error("browser_session_encode_failed"))?)
        .bind(now_ms())
        .bind(i64::try_from(session.run_generation).unwrap_or(i64::MAX))
        .execute(self.store.pool())
        .await
        .map_err(domain_error("browser_session_store_failed"))?;
        Ok(())
    }

    async fn invalidate_browser_permission_ledger(
        &self,
        browser_session_id: &hachimi_protocol::BrowserSessionId,
    ) -> Result<(), AppServerDomainError> {
        let mut transaction = self
            .store
            .pool()
            .begin()
            .await
            .map_err(domain_error("browser_permission_invalidate_failed"))?;
        sqlx::query("DELETE FROM browser_site_permissions WHERE browser_session_id = ?")
            .bind(browser_session_id.as_str())
            .execute(&mut *transaction)
            .await
            .map_err(domain_error("browser_permission_invalidate_failed"))?;
        sqlx::query("DELETE FROM browser_network_rules WHERE browser_session_id = ?")
            .bind(browser_session_id.as_str())
            .execute(&mut *transaction)
            .await
            .map_err(domain_error("browser_permission_invalidate_failed"))?;
        sqlx::query("UPDATE browser_permission_requests SET status = 'expired', updated_at_ms = ? WHERE browser_session_id = ? AND status = 'pending'")
            .bind(now_ms())
            .bind(browser_session_id.as_str())
            .execute(&mut *transaction)
            .await
            .map_err(domain_error("browser_permission_invalidate_failed"))?;
        transaction
            .commit()
            .await
            .map_err(domain_error("browser_permission_invalidate_failed"))
    }

    async fn browser_owner_session(
        &self,
        browser_session_id: &hachimi_protocol::BrowserSessionId,
    ) -> Result<SessionId, AppServerDomainError> {
        let row = sqlx::query("SELECT owner_session_id FROM browser_sessions WHERE id = ?")
            .bind(browser_session_id.as_str())
            .fetch_optional(self.store.pool())
            .await
            .map_err(domain_error("browser_session_lookup_failed"))?
            .ok_or_else(|| {
                AppServerDomainError::new(
                    "browser_session_not_found",
                    "Browser session does not exist",
                )
            })?;
        Ok(SessionId::new(row.get::<String, _>("owner_session_id")))
    }

    async fn capture_browser_network_denials(&self) -> Result<(), AppServerDomainError> {
        for candidate in self.browser.drain_network_permission_candidates().await {
            let run = self
                .store
                .get_run(&candidate.session.owner_run_id)
                .await
                .map_err(domain_error("browser_network_denial_run_lookup_failed"))?;
            let Some(run) = run else { continue };
            if run.status.is_terminal()
                || run.session_id != candidate.session.owner_session_id
                || candidate.observed_at_ms <= 0
            {
                continue;
            }
            let now = now_ms();
            let expires_at_ms = now.saturating_add(10 * 60 * 1_000);
            let network_kind = match candidate.network_kind {
                hachimi_protocol::BrowserNetworkRuleKind::Document => "document",
                hachimi_protocol::BrowserNetworkRuleKind::Resource => "resource",
            };
            let existing = sqlx::query("SELECT id FROM browser_permission_requests WHERE browser_session_id = ? AND owner_run_id = ? AND origin = ? AND network_kind = ? AND status = 'pending' AND expires_at_ms > ? ORDER BY created_at_ms DESC LIMIT 1")
                .bind(candidate.session.id.as_str())
                .bind(candidate.session.owner_run_id.as_str())
                .bind(&candidate.origin)
                .bind(network_kind)
                .bind(now)
                .fetch_optional(self.store.pool())
                .await
                .map_err(domain_error("browser_network_denial_lookup_failed"))?;
            if existing.is_some() {
                continue;
            }
            let request_id = hachimi_protocol::ItemId::random();
            sqlx::query("INSERT INTO browser_permission_requests(id, browser_session_id, owner_session_id, owner_run_id, origin, capabilities_json, network_kind, private_network, status, expected_browser_revision, created_at_ms, expires_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?, ?, ?)")
                .bind(request_id.as_str())
                .bind(candidate.session.id.as_str())
                .bind(candidate.session.owner_session_id.as_str())
                .bind(candidate.session.owner_run_id.as_str())
                .bind(&candidate.origin)
                .bind(serde_json::to_string(&vec![hachimi_protocol::BrowserCapability::Observe]).map_err(domain_error("browser_network_denial_encode_failed"))?)
                .bind(network_kind)
                .bind(candidate.private_network)
                .bind(i64::try_from(candidate.session.revision).unwrap_or(i64::MAX))
                .bind(now)
                .bind(expires_at_ms)
                .bind(now)
                .execute(self.store.pool())
                .await
                .map_err(domain_error("browser_network_denial_store_failed"))?;
            let _ = self
                .store
                .append_event(
                    &candidate.session.owner_session_id,
                    Some(&candidate.session.owner_run_id),
                    "browser.permission_required",
                    serde_json::json!({
                        "requestId": request_id,
                        "origin": candidate.origin,
                        "networkKind": network_kind,
                        "privateNetwork": candidate.private_network,
                        "expiresAtMs": expires_at_ms
                    }),
                )
                .await;
        }
        Ok(())
    }

    async fn browser_preferred_profile(&self) -> Result<BrowserProfileKind, AppServerDomainError> {
        let row = sqlx::query(
            "SELECT preferred_profile_kind FROM browser_host_settings WHERE singleton = 1",
        )
        .fetch_one(self.store.pool())
        .await
        .map_err(domain_error("browser_host_settings_load_failed"))?;
        match row.get::<String, _>("preferred_profile_kind").as_str() {
            "isolated" => Ok(BrowserProfileKind::Isolated),
            "chrome_extension" => Ok(BrowserProfileKind::ChromeExtension),
            _ => Err(AppServerDomainError::new(
                "browser_host_settings_invalid",
                "stored Browser profile preference is invalid",
            )),
        }
    }

    async fn persist_computer_rule(
        &self,
        session_id: &SessionId,
        rule: &ComputerAppRule,
    ) -> Result<(), AppServerDomainError> {
        sqlx::query(
            "INSERT INTO computer_app_rules(session_id, app_id, rule_json, updated_at_ms) VALUES(?, ?, ?, ?) ON CONFLICT(session_id, app_id) DO UPDATE SET rule_json = excluded.rule_json, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(session_id.as_str())
        .bind(&rule.app_id)
        .bind(serde_json::to_string(rule).map_err(domain_error("computer_rule_encode_failed"))?)
        .bind(rule.updated_at_ms)
        .execute(self.store.pool())
        .await
        .map_err(domain_error("computer_rule_store_failed"))?;
        Ok(())
    }

    async fn load_computer_rules(
        &self,
        session_id: &SessionId,
    ) -> Result<(), AppServerDomainError> {
        let rows = sqlx::query("SELECT rule_json FROM computer_app_rules WHERE session_id = ?")
            .bind(session_id.as_str())
            .fetch_all(self.store.pool())
            .await
            .map_err(domain_error("computer_rule_load_failed"))?;
        for row in rows {
            let rule = serde_json::from_str::<ComputerAppRule>(row.get("rule_json"))
                .map_err(domain_error("computer_rule_decode_failed"))?;
            self.computer.set_app_rule(session_id, rule);
        }
        Ok(())
    }

    async fn load_computer_global_rules(
        &self,
        session_id: &SessionId,
    ) -> Result<(), AppServerDomainError> {
        let rows = sqlx::query("SELECT rule_json FROM computer_global_app_rules ORDER BY app_id")
            .fetch_all(self.store.pool())
            .await
            .map_err(domain_error("computer_global_rule_load_failed"))?;
        for row in rows {
            let rule = serde_json::from_str::<ComputerAppRule>(row.get("rule_json"))
                .map_err(domain_error("computer_global_rule_decode_failed"))?;
            self.computer.set_app_rule(session_id, rule);
        }
        Ok(())
    }
}

fn browser_action_kind(action: &hachimi_protocol::BrowserAction) -> &'static str {
    use hachimi_protocol::BrowserAction;
    match action {
        BrowserAction::Navigate { .. } => "browser.navigate",
        BrowserAction::Back => "browser.back",
        BrowserAction::Forward => "browser.forward",
        BrowserAction::Reload { .. } => "browser.reload",
        BrowserAction::Stop => "browser.stop_loading",
        BrowserAction::Click { .. } => "browser.click",
        BrowserAction::Hover { .. } => "browser.hover",
        BrowserAction::DoubleClick { .. } => "browser.double_click",
        BrowserAction::Scroll { .. } => "browser.scroll",
        BrowserAction::DragDrop { .. } => "browser.drag_drop",
        BrowserAction::Clear { .. } => "browser.clear",
        BrowserAction::Fill { .. } => "browser.fill",
        BrowserAction::SelectOption { .. } => "browser.select_option",
        BrowserAction::PressKeys { .. } => "browser.press_keys",
        BrowserAction::WaitFor { .. } => "browser.wait_for",
        BrowserAction::TabList => "browser.tab_list",
        BrowserAction::TabNew { .. } => "browser.tab_new",
        BrowserAction::TabSwitch { .. } => "browser.tab_switch",
        BrowserAction::TabClose { .. } => "browser.tab_close",
        BrowserAction::TypeText { .. } => "browser.type_text",
        BrowserAction::Upload { .. } => "browser.upload",
        BrowserAction::Download { .. } => "browser.download",
        BrowserAction::ReadStorage => "browser.read_storage",
        BrowserAction::WriteStorage { .. } => "browser.write_storage",
        BrowserAction::Cdp { .. } => "browser.cdp",
    }
}

fn computer_action_kind(action: &hachimi_protocol::ComputerAction) -> &'static str {
    use hachimi_protocol::ComputerAction;
    match action {
        ComputerAction::MouseMove { .. } => "computer.mouse_move",
        ComputerAction::MouseClick { .. } => "computer.mouse_click",
        ComputerAction::MouseDown { .. } => "computer.mouse_down",
        ComputerAction::MouseUp { .. } => "computer.mouse_up",
        ComputerAction::MouseDoubleClick { .. } => "computer.mouse_double_click",
        ComputerAction::MouseDrag { .. } => "computer.mouse_drag",
        ComputerAction::Scroll { .. } => "computer.scroll",
        ComputerAction::KeyPress { .. } => "computer.key_press",
        ComputerAction::KeyDown { .. } => "computer.key_down",
        ComputerAction::KeyUp { .. } => "computer.key_up",
        ComputerAction::KeyChord { .. } => "computer.key_chord",
        ComputerAction::TypeText { .. } => "computer.type_text",
        ComputerAction::WindowFocus => "computer.window_focus",
        ComputerAction::WindowMove { .. } => "computer.window_move",
        ComputerAction::WindowResize { .. } => "computer.window_resize",
        ComputerAction::WindowMinimize => "computer.window_minimize",
        ComputerAction::WindowMaximize => "computer.window_maximize",
        ComputerAction::WindowRestore => "computer.window_restore",
        ComputerAction::WindowClose => "computer.window_close",
        ComputerAction::LaunchApp { .. } => "computer.launch_app",
    }
}

fn browser_action_error_status(
    error: &hachimi_browser::BrowserHostError,
) -> (&'static str, &'static str) {
    use hachimi_browser::BrowserHostError;
    match error {
        BrowserHostError::Broker(_)
        | BrowserHostError::ExtensionCommandTimeout
        | BrowserHostError::DownloadFailed => ("indeterminate", "browser_result_unknown"),
        _ => ("denied", "browser_action_rejected"),
    }
}

fn computer_action_error_status(
    error: &hachimi_computer::ComputerHostError,
) -> (&'static str, &'static str) {
    match error {
        hachimi_computer::ComputerHostError::Broker(_) => {
            ("indeterminate", "computer_result_unknown")
        }
        _ => ("denied", "computer_action_rejected"),
    }
}

fn is_enterprise_plugin(plugin_id: &hachimi_protocol::PluginId) -> bool {
    is_enterprise_provider_id(plugin_id.as_str())
}

fn is_enterprise_provider_id(value: &str) -> bool {
    matches!(value, "wecom" | "dingtalk" | "feishu")
}

fn decode_browser_permission_request(
    row: sqlx::sqlite::SqliteRow,
) -> Result<hachimi_protocol::BrowserPermissionRequest, AppServerDomainError> {
    let expires_at_ms = row.get::<i64, _>("expires_at_ms");
    let stored_status = row.get::<String, _>("status");
    let status = if expires_at_ms <= now_ms() && stored_status == "pending" {
        hachimi_protocol::BrowserPermissionRequestStatus::Expired
    } else {
        match stored_status.as_str() {
            "pending" => hachimi_protocol::BrowserPermissionRequestStatus::Pending,
            "allowed" => hachimi_protocol::BrowserPermissionRequestStatus::Allowed,
            "denied" => hachimi_protocol::BrowserPermissionRequestStatus::Denied,
            "expired" => hachimi_protocol::BrowserPermissionRequestStatus::Expired,
            _ => {
                return Err(AppServerDomainError::new(
                    "browser_permission_request_status_invalid",
                    "Browser permission request has an invalid status",
                ));
            }
        }
    };
    Ok(hachimi_protocol::BrowserPermissionRequest {
        id: hachimi_protocol::ItemId::new(row.get::<String, _>("id")),
        browser_session_id: hachimi_protocol::BrowserSessionId::new(
            row.get::<String, _>("browser_session_id"),
        ),
        owner_session_id: hachimi_protocol::SessionId::new(
            row.get::<String, _>("owner_session_id"),
        ),
        owner_run_id: hachimi_protocol::RunId::new(row.get::<String, _>("owner_run_id")),
        run_generation: u64::try_from(row.get::<i64, _>("generation")).unwrap_or_default(),
        origin: row.get("origin"),
        capabilities: serde_json::from_str(row.get("capabilities_json"))
            .map_err(domain_error("browser_permission_request_decode_failed"))?,
        network_kind: if row.get::<String, _>("network_kind") == "document" {
            hachimi_protocol::BrowserNetworkRuleKind::Document
        } else {
            hachimi_protocol::BrowserNetworkRuleKind::Resource
        },
        private_network: row.get("private_network"),
        status,
        expected_browser_revision: u64::try_from(row.get::<i64, _>("expected_browser_revision"))
            .unwrap_or_default(),
        created_at_ms: row.get("created_at_ms"),
        expires_at_ms,
    })
}
