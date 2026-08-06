use hachimi_control_plane::{
    AppServerContext, AppServerDomainRequest, AppServerDomainResponse, AppServerError,
    AppServerRequest, AppServerResponse, BrowserAppRequest, BrowserAppResponse, ChannelAppRequest,
    ChannelAppResponse, ComputerAppRequest, ComputerAppResponse, ConnectorAppRequest,
    ConnectorAppResponse, GatewayAppRequest, GatewayAppResponse, PluginAppRequest,
    PluginAppResponse,
};
use hachimi_core::WindowKind;
use hachimi_protocol::{
    BrowserAction, BrowserSessionId, ClientContext, LocalHostCommandRequest,
    LocalHostCommandResponse, RunId, SessionId, SessionSourceOrigin,
    WorkbenchEnvironmentChangeReason,
};
use tauri::{Manager, WebviewWindow};

use super::{CommandError, DesktopState};

fn app_error(error: AppServerError) -> CommandError {
    match error {
        AppServerError::Domain { code, message } => CommandError::new(code, message),
        other => CommandError::operation("local_host_app_server_failed", other),
    }
}

async fn dispatch(
    window: &WebviewWindow,
    state: &DesktopState,
    request: AppServerDomainRequest,
) -> Result<AppServerDomainResponse, CommandError> {
    let kind = WindowKind::from_label(window.label()).ok_or_else(|| {
        CommandError::new(
            "unknown_window",
            format!("unknown window: {}", window.label()),
        )
    })?;
    let client = ClientContext::for_window(kind);
    let context = AppServerContext {
        principal: client.client_id.0.clone(),
        client,
    };
    match state
        .app_server
        .dispatch(&context, AppServerRequest::Domain(Box::new(request)))
        .await
        .map_err(app_error)?
    {
        AppServerResponse::Domain(response) => Ok(*response),
        _ => Err(CommandError::new(
            "local_host_protocol_mismatch",
            "App Server returned a non-domain response",
        )),
    }
}

async fn request_to_domain(
    window: &WebviewWindow,
    request: LocalHostCommandRequest,
) -> Result<Option<AppServerDomainRequest>, CommandError> {
    Ok(Some(match request {
        LocalHostCommandRequest::PluginChooseAndInstallBundle => {
            let Some(file) = rfd::AsyncFileDialog::new()
                .add_filter("Hachimi Plugin Bundle", &["zip"])
                .pick_file()
                .await
            else {
                return Ok(None);
            };
            AppServerDomainRequest::Plugin(PluginAppRequest::InstallLocal(
                file.path().to_path_buf(),
            ))
        }
        LocalHostCommandRequest::PluginChooseAndInstall => {
            let Some(folder) = rfd::AsyncFileDialog::new().pick_folder().await else {
                return Ok(None);
            };
            AppServerDomainRequest::Plugin(PluginAppRequest::InstallLocal(
                folder.path().to_path_buf(),
            ))
        }
        LocalHostCommandRequest::PluginInstallBuiltinSampleCrm => {
            let resource_dir =
                window.app_handle().path().resource_dir().map_err(|error| {
                    CommandError::operation("plugin_resource_dir_failed", error)
                })?;
            AppServerDomainRequest::Plugin(PluginAppRequest::InstallLocal(
                resource_dir.join("plugins").join("sample-crm"),
            ))
        }
        LocalHostCommandRequest::PluginInstallBuiltinEnterprise { provider_id } => {
            let resource_dir =
                window.app_handle().path().resource_dir().map_err(|error| {
                    CommandError::operation("plugin_resource_dir_failed", error)
                })?;
            let bundle_name = provider_id.as_str();
            AppServerDomainRequest::Plugin(PluginAppRequest::InstallLocal(
                resource_dir.join("plugins").join(bundle_name),
            ))
        }
        LocalHostCommandRequest::BrowserListPermissions => {
            AppServerDomainRequest::Browser(BrowserAppRequest::ListPermissions)
        }
        LocalHostCommandRequest::BrowserListPermissionRequests => {
            AppServerDomainRequest::Browser(BrowserAppRequest::ListPermissionRequests)
        }
        LocalHostCommandRequest::BrowserStart {
            session_id,
            run_id,
            profile_kind,
            initial_url,
            pairing_id,
        } => AppServerDomainRequest::Browser(BrowserAppRequest::Start {
            session_id,
            run_id,
            profile_kind,
            initial_url,
            pairing_id,
        }),
        LocalHostCommandRequest::BrowserGrantSitePermission {
            context,
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
        } => AppServerDomainRequest::Browser(BrowserAppRequest::GrantSitePermission {
            context,
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
        }),
        LocalHostCommandRequest::BrowserRevokeSitePermission {
            context,
            session_id,
            run_id,
            browser_session_id,
            expected_revision,
            origin,
        } => AppServerDomainRequest::Browser(BrowserAppRequest::RevokeSitePermission {
            context,
            session_id,
            run_id,
            browser_session_id,
            expected_revision,
            origin,
        }),
        LocalHostCommandRequest::BrowserObserve {
            browser_session_id,
            run_id,
        } => AppServerDomainRequest::Browser(BrowserAppRequest::Observe {
            browser_session_id,
            run_id,
        }),
        LocalHostCommandRequest::BrowserAct { run_id, request } => {
            AppServerDomainRequest::Browser(BrowserAppRequest::Act { run_id, request })
        }
        LocalHostCommandRequest::BrowserChooseUpload {
            browser_session_id,
            run_id,
        } => {
            let Some(file) = rfd::AsyncFileDialog::new().pick_file().await else {
                return Ok(None);
            };
            AppServerDomainRequest::Browser(BrowserAppRequest::StageUpload {
                browser_session_id,
                run_id,
                source: file.path().to_path_buf(),
            })
        }
        LocalHostCommandRequest::BrowserImportDownload {
            browser_session_id,
            run_id,
            download_token,
            suggested_file_name,
        } => {
            let Some(file) = rfd::AsyncFileDialog::new()
                .set_file_name(&suggested_file_name)
                .save_file()
                .await
            else {
                return Ok(None);
            };
            AppServerDomainRequest::Browser(BrowserAppRequest::ImportDownload {
                browser_session_id,
                run_id,
                download_token,
                destination: file.path().to_path_buf(),
            })
        }
        LocalHostCommandRequest::BrowserTakeOver {
            browser_session_id,
            run_id,
        } => AppServerDomainRequest::Browser(BrowserAppRequest::TakeOver {
            browser_session_id,
            run_id,
        }),
        LocalHostCommandRequest::BrowserStop {
            browser_session_id,
            run_id,
        } => AppServerDomainRequest::Browser(BrowserAppRequest::Stop {
            browser_session_id,
            run_id,
        }),
        LocalHostCommandRequest::ComputerSetAppRule { session_id, rule } => {
            AppServerDomainRequest::Computer(ComputerAppRequest::SetAppRule { session_id, rule })
        }
        LocalHostCommandRequest::ComputerListWindows => {
            AppServerDomainRequest::Computer(ComputerAppRequest::ListWindows)
        }
        LocalHostCommandRequest::ComputerListGlobalAppRules => {
            AppServerDomainRequest::Computer(ComputerAppRequest::ListGlobalAppRules)
        }
        LocalHostCommandRequest::ComputerSetGlobalAppRule { rule } => {
            AppServerDomainRequest::Computer(ComputerAppRequest::SetGlobalAppRule(rule))
        }
        LocalHostCommandRequest::ComputerRemoveGlobalAppRule { app_id } => {
            AppServerDomainRequest::Computer(ComputerAppRequest::RemoveGlobalAppRule(app_id))
        }
        LocalHostCommandRequest::ComputerObserve {
            session_id,
            run_id,
            window_handle,
        } => AppServerDomainRequest::Computer(ComputerAppRequest::Observe {
            session_id,
            run_id,
            window_handle,
        }),
        LocalHostCommandRequest::ComputerAct { run_id, request } => {
            AppServerDomainRequest::Computer(ComputerAppRequest::Act { run_id, request })
        }
        LocalHostCommandRequest::ComputerTakeOver { session_id } => {
            AppServerDomainRequest::Computer(ComputerAppRequest::TakeOver(session_id))
        }
        LocalHostCommandRequest::PluginList => {
            AppServerDomainRequest::Plugin(PluginAppRequest::List)
        }
        LocalHostCommandRequest::PluginListContributions { plugin_id } => {
            AppServerDomainRequest::Plugin(PluginAppRequest::ListContributions(plugin_id))
        }
        LocalHostCommandRequest::PluginGetContributionSurface {
            plugin_id,
            contribution_id,
        } => AppServerDomainRequest::Plugin(PluginAppRequest::GetContributionSurface {
            plugin_id,
            contribution_id,
        }),
        LocalHostCommandRequest::PluginGet { plugin_id } => {
            AppServerDomainRequest::Plugin(PluginAppRequest::Get(plugin_id))
        }
        LocalHostCommandRequest::PluginHealthCheck { plugin_id } => {
            AppServerDomainRequest::Plugin(PluginAppRequest::HealthCheck(plugin_id))
        }
        LocalHostCommandRequest::PluginPermissionDiff { plugin_id } => {
            AppServerDomainRequest::Plugin(PluginAppRequest::PermissionDiff(plugin_id))
        }
        LocalHostCommandRequest::PluginRevisionHead { plugin_id } => {
            AppServerDomainRequest::Plugin(PluginAppRequest::RevisionHead(plugin_id))
        }
        LocalHostCommandRequest::PluginListRevisions { plugin_id } => {
            AppServerDomainRequest::Plugin(PluginAppRequest::ListRevisions(plugin_id))
        }
        LocalHostCommandRequest::PluginLifecycleJournal { plugin_id } => {
            AppServerDomainRequest::Plugin(PluginAppRequest::LifecycleJournal(plugin_id))
        }
        LocalHostCommandRequest::PluginSetEnabled { plugin_id, enabled } => {
            AppServerDomainRequest::Plugin(PluginAppRequest::SetEnabled { plugin_id, enabled })
        }
        LocalHostCommandRequest::PluginRollback {
            plugin_id,
            revision,
        } => AppServerDomainRequest::Plugin(PluginAppRequest::Rollback {
            plugin_id,
            revision,
        }),
        LocalHostCommandRequest::PluginUninstall { plugin_id } => {
            AppServerDomainRequest::Plugin(PluginAppRequest::Uninstall(plugin_id))
        }
        LocalHostCommandRequest::ConnectorUpsertAccount { account } => {
            AppServerDomainRequest::Connector(ConnectorAppRequest::UpsertAccount(account))
        }
        LocalHostCommandRequest::ConnectorListAccounts => {
            AppServerDomainRequest::Connector(ConnectorAppRequest::ListAccounts)
        }
        LocalHostCommandRequest::ConnectorGetAccount { account_id } => {
            AppServerDomainRequest::Connector(ConnectorAppRequest::GetAccount(account_id))
        }
        LocalHostCommandRequest::ConnectorGetDriverDescriptor {
            plugin_id,
            connector_id,
        } => AppServerDomainRequest::Connector(ConnectorAppRequest::GetDriverDescriptor {
            plugin_id,
            connector_id,
        }),
        LocalHostCommandRequest::ConnectorRevokeAccount { account_id } => {
            AppServerDomainRequest::Connector(ConnectorAppRequest::RevokeAccount(account_id))
        }
        LocalHostCommandRequest::ConnectorInvoke { request } => {
            AppServerDomainRequest::Connector(ConnectorAppRequest::Invoke(request))
        }
        LocalHostCommandRequest::ChannelLoopbackReceive {
            bearer_token,
            message,
        } => AppServerDomainRequest::Channel(ChannelAppRequest::LoopbackReceive {
            bearer_token,
            message,
        }),
        LocalHostCommandRequest::ChannelMockPollPush { message } => {
            AppServerDomainRequest::Channel(ChannelAppRequest::MockPollPush(message))
        }
        LocalHostCommandRequest::ChannelMockPollSetConnected { connected } => {
            AppServerDomainRequest::Channel(ChannelAppRequest::MockPollSetConnected(connected))
        }
        LocalHostCommandRequest::ChannelMockPollDrain => {
            AppServerDomainRequest::Channel(ChannelAppRequest::MockPollDrain)
        }
        LocalHostCommandRequest::ChannelEnqueueDelivery {
            address,
            idempotency_key,
            text,
        } => AppServerDomainRequest::Channel(ChannelAppRequest::EnqueueDelivery {
            address,
            idempotency_key,
            text,
        }),
        LocalHostCommandRequest::GatewayHealth => {
            AppServerDomainRequest::Gateway(GatewayAppRequest::Health)
        }
        LocalHostCommandRequest::GatewayProviderManifests => {
            AppServerDomainRequest::Gateway(GatewayAppRequest::ProviderManifests)
        }
        LocalHostCommandRequest::GatewayProviderHealth => {
            AppServerDomainRequest::Gateway(GatewayAppRequest::ProviderHealth)
        }
        LocalHostCommandRequest::GatewayListProviderAccounts => {
            AppServerDomainRequest::Gateway(GatewayAppRequest::ListProviderAccounts)
        }
        LocalHostCommandRequest::GatewayUpsertProviderAccount { account } => {
            AppServerDomainRequest::Gateway(GatewayAppRequest::UpsertProviderAccount(account))
        }
        LocalHostCommandRequest::GatewayReconcile => {
            AppServerDomainRequest::Gateway(GatewayAppRequest::Reconcile)
        }
    }))
}

fn domain_to_response(
    response: AppServerDomainResponse,
) -> Result<LocalHostCommandResponse, CommandError> {
    Ok(match response {
        AppServerDomainResponse::Browser(BrowserAppResponse::Session(value)) => {
            LocalHostCommandResponse::BrowserSession(value)
        }
        AppServerDomainResponse::Browser(BrowserAppResponse::Permission(value)) => {
            LocalHostCommandResponse::BrowserPermission(value)
        }
        AppServerDomainResponse::Browser(BrowserAppResponse::Permissions(value)) => {
            LocalHostCommandResponse::BrowserPermissions(value)
        }
        AppServerDomainResponse::Browser(BrowserAppResponse::PermissionRequests(value)) => {
            LocalHostCommandResponse::BrowserPermissionRequests(value)
        }
        AppServerDomainResponse::Browser(BrowserAppResponse::PermissionRevoked(value)) => {
            LocalHostCommandResponse::Removed(value)
        }
        AppServerDomainResponse::Browser(BrowserAppResponse::Observation(value)) => {
            LocalHostCommandResponse::BrowserObservation(value)
        }
        AppServerDomainResponse::Browser(BrowserAppResponse::Action(value)) => {
            LocalHostCommandResponse::BrowserAction(value)
        }
        AppServerDomainResponse::Browser(BrowserAppResponse::FileToken(value)) => {
            LocalHostCommandResponse::BrowserFileToken(value)
        }
        AppServerDomainResponse::Browser(BrowserAppResponse::ImportedDownload(value)) => {
            LocalHostCommandResponse::BrowserImportedDownload(value)
        }
        AppServerDomainResponse::Computer(ComputerAppResponse::Rule(value)) => {
            LocalHostCommandResponse::ComputerRule(value)
        }
        AppServerDomainResponse::Computer(ComputerAppResponse::Rules(value)) => {
            LocalHostCommandResponse::ComputerRules(value)
        }
        AppServerDomainResponse::Computer(ComputerAppResponse::Windows(value)) => {
            LocalHostCommandResponse::ComputerWindows(value)
        }
        AppServerDomainResponse::Computer(ComputerAppResponse::Removed(value)) => {
            LocalHostCommandResponse::Removed(value)
        }
        AppServerDomainResponse::Computer(ComputerAppResponse::Frame(value)) => {
            LocalHostCommandResponse::ComputerFrame(*value)
        }
        AppServerDomainResponse::Computer(ComputerAppResponse::Action(value)) => {
            LocalHostCommandResponse::ComputerAction(value)
        }
        AppServerDomainResponse::Computer(ComputerAppResponse::TakenOver(value)) => {
            LocalHostCommandResponse::ComputerTakenOver(value)
        }
        AppServerDomainResponse::Plugin(PluginAppResponse::Plugin(value)) => {
            LocalHostCommandResponse::Plugin(Some(value))
        }
        AppServerDomainResponse::Plugin(PluginAppResponse::OptionalPlugin(value)) => {
            LocalHostCommandResponse::Plugin(value)
        }
        AppServerDomainResponse::Plugin(PluginAppResponse::Plugins(value)) => {
            LocalHostCommandResponse::Plugins(value)
        }
        AppServerDomainResponse::Plugin(PluginAppResponse::Contributions(value)) => {
            LocalHostCommandResponse::PluginContributions(value)
        }
        AppServerDomainResponse::Plugin(PluginAppResponse::ContributionSurface(value)) => {
            LocalHostCommandResponse::PluginContributionSurface(value)
        }
        AppServerDomainResponse::Plugin(PluginAppResponse::PermissionDiff(value)) => {
            LocalHostCommandResponse::PluginPermissionDiff(value)
        }
        AppServerDomainResponse::Plugin(PluginAppResponse::RevisionHead(value)) => {
            LocalHostCommandResponse::PluginRevisionHead(value)
        }
        AppServerDomainResponse::Plugin(PluginAppResponse::Revisions(value)) => {
            LocalHostCommandResponse::PluginRevisions(value)
        }
        AppServerDomainResponse::Plugin(PluginAppResponse::LifecycleJournal(value)) => {
            LocalHostCommandResponse::PluginLifecycleJournal(value)
        }
        AppServerDomainResponse::Plugin(PluginAppResponse::Removed(value)) => {
            LocalHostCommandResponse::Removed(value)
        }
        AppServerDomainResponse::Connector(ConnectorAppResponse::Account(value)) => {
            LocalHostCommandResponse::ConnectorAccount(Some(value))
        }
        AppServerDomainResponse::Connector(ConnectorAppResponse::Accounts(value)) => {
            LocalHostCommandResponse::ConnectorAccounts(value)
        }
        AppServerDomainResponse::Connector(ConnectorAppResponse::OptionalAccount(value)) => {
            LocalHostCommandResponse::ConnectorAccount(value)
        }
        AppServerDomainResponse::Connector(ConnectorAppResponse::DriverDescriptor(value)) => {
            LocalHostCommandResponse::ConnectorDriverDescriptor(value)
        }
        AppServerDomainResponse::Connector(ConnectorAppResponse::Invocation(value)) => {
            LocalHostCommandResponse::ConnectorInvocation(value)
        }
        AppServerDomainResponse::Channel(ChannelAppResponse::Ingress(value)) => {
            LocalHostCommandResponse::Ingress(value)
        }
        AppServerDomainResponse::Channel(ChannelAppResponse::Ingresses(value)) => {
            LocalHostCommandResponse::Ingresses(value)
        }
        AppServerDomainResponse::Channel(ChannelAppResponse::MockPollConnected(value)) => {
            LocalHostCommandResponse::MockPollConnected(value)
        }
        AppServerDomainResponse::Channel(ChannelAppResponse::Delivery(value)) => {
            LocalHostCommandResponse::Delivery(value)
        }
        AppServerDomainResponse::Gateway(GatewayAppResponse::Health(value)) => {
            LocalHostCommandResponse::GatewayHealth(value)
        }
        AppServerDomainResponse::Gateway(GatewayAppResponse::ProviderManifests(value)) => {
            LocalHostCommandResponse::ChannelProviderManifests(value)
        }
        AppServerDomainResponse::Gateway(GatewayAppResponse::ProviderHealth(value)) => {
            LocalHostCommandResponse::ChannelProviderHealth(value)
        }
        AppServerDomainResponse::Gateway(GatewayAppResponse::ProviderAccounts(value)) => {
            LocalHostCommandResponse::ChannelProviderAccounts(value)
        }
        AppServerDomainResponse::Gateway(GatewayAppResponse::ProviderAccount(value)) => {
            LocalHostCommandResponse::ChannelProviderAccount(value)
        }
        AppServerDomainResponse::Gateway(GatewayAppResponse::Reconciled) => {
            LocalHostCommandResponse::GatewayReconciled
        }
        _ => {
            return Err(CommandError::new(
                "local_host_protocol_mismatch",
                "App Server returned a response for a different local Host domain",
            ));
        }
    })
}

pub(super) async fn execute_local_host_command(
    window: &WebviewWindow,
    state: &DesktopState,
    request: LocalHostCommandRequest,
) -> Result<LocalHostCommandResponse, CommandError> {
    let browser_source = browser_source_candidate(&request);
    let runtime = state.control_plane.feature_flags().runtime_features;
    if is_plugin_command(&request) && !runtime.plugin_runtime {
        return Err(CommandError::new("feature_disabled", "plugin_runtime"));
    }
    if matches!(
        &request,
        LocalHostCommandRequest::PluginInstallBuiltinEnterprise { .. }
    ) && !runtime.enterprise_integrations
    {
        return Err(CommandError::new(
            "feature_disabled",
            "enterprise_integrations",
        ));
    }
    let Some(request) = request_to_domain(window, request).await? else {
        return Ok(LocalHostCommandResponse::Cancelled);
    };
    let response = domain_to_response(dispatch(window, state, request).await?)?;
    if let Some(candidate) = browser_source
        && browser_source_succeeded(&response)
    {
        let browser_session_id = candidate.browser_session_id.or_else(|| match &response {
            LocalHostCommandResponse::BrowserSession(session) => Some(session.id.clone()),
            _ => None,
        });
        let browser_session = match &response {
            LocalHostCommandResponse::BrowserSession(session) => Some(session.clone()),
            _ => match browser_session_id.as_ref() {
                Some(browser_session_id) => state
                    .agent_store
                    .get_browser_session(browser_session_id)
                    .await
                    .map_err(|error| {
                        CommandError::operation("browser_source_owner_failed", error)
                    })?,
                None => None,
            },
        };
        let response_url = match &response {
            LocalHostCommandResponse::BrowserObservation(observation) => {
                canonical_browser_url(&observation.url)
            }
            LocalHostCommandResponse::BrowserAction(result) => result
                .output
                .as_ref()
                .and_then(|output| output.get("url"))
                .and_then(serde_json::Value::as_str)
                .and_then(canonical_browser_url),
            LocalHostCommandResponse::BrowserSession(session) => session
                .current_url
                .as_deref()
                .and_then(canonical_browser_url),
            _ => None,
        };
        let title = match &response {
            LocalHostCommandResponse::BrowserObservation(observation) => {
                let title = observation.title.trim();
                (!title.is_empty()).then(|| title.to_owned())
            }
            LocalHostCommandResponse::BrowserAction(result) => result
                .output
                .as_ref()
                .and_then(|output| output.get("title"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .map(str::to_owned),
            _ => None,
        };
        let Some(url) = response_url
            .or_else(|| {
                browser_session
                    .as_ref()
                    .and_then(|session| session.current_url.as_deref())
                    .and_then(canonical_browser_url)
            })
            .or_else(|| {
                candidate
                    .fallback_url
                    .as_deref()
                    .and_then(canonical_browser_url)
            })
        else {
            return Ok(response);
        };
        let owner = match candidate.session_id {
            Some(session_id) => Some((session_id, candidate.run_id)),
            None => browser_session.map(|session| (session.owner_session_id, session.owner_run_id)),
        };
        if let Some((session_id, run_id)) = owner {
            state
                .agent_store
                .upsert_session_web_source(
                    &session_id,
                    Some(&run_id),
                    SessionSourceOrigin::Browser,
                    &url,
                    title.as_deref(),
                    None,
                )
                .await
                .map_err(|error| CommandError::operation("browser_source_store_failed", error))?;
            if let Ok(environment) = state.workbench.environment_snapshot(&session_id).await {
                crate::environment_commands::emit_workbench_environment(
                    window.app_handle(),
                    &environment,
                    vec![
                        WorkbenchEnvironmentChangeReason::Browser,
                        WorkbenchEnvironmentChangeReason::Sources,
                    ],
                );
            }
        }
    }
    Ok(response)
}

struct BrowserSourceCandidate {
    session_id: Option<SessionId>,
    run_id: RunId,
    browser_session_id: Option<BrowserSessionId>,
    fallback_url: Option<String>,
}

fn browser_source_candidate(request: &LocalHostCommandRequest) -> Option<BrowserSourceCandidate> {
    match request {
        LocalHostCommandRequest::BrowserStart {
            session_id,
            run_id,
            initial_url,
            ..
        } => Some(BrowserSourceCandidate {
            session_id: Some(session_id.clone()),
            run_id: run_id.clone(),
            browser_session_id: None,
            fallback_url: initial_url.clone(),
        }),
        LocalHostCommandRequest::BrowserObserve {
            browser_session_id,
            run_id,
        } => Some(BrowserSourceCandidate {
            session_id: None,
            run_id: run_id.clone(),
            browser_session_id: Some(browser_session_id.clone()),
            fallback_url: None,
        }),
        LocalHostCommandRequest::BrowserAct { run_id, request } => {
            let fallback_url = match &request.action {
                BrowserAction::Navigate { url } => Some(url),
                BrowserAction::TabNew { url: Some(url) } => Some(url),
                _ => None,
            }
            .cloned();
            Some(BrowserSourceCandidate {
                session_id: None,
                run_id: run_id.clone(),
                browser_session_id: Some(request.browser_session_id.clone()),
                fallback_url,
            })
        }
        _ => None,
    }
}

fn browser_source_succeeded(response: &LocalHostCommandResponse) -> bool {
    match response {
        LocalHostCommandResponse::BrowserSession(_) => true,
        LocalHostCommandResponse::BrowserObservation(_) => true,
        LocalHostCommandResponse::BrowserAction(result) => result.accepted,
        _ => false,
    }
}

fn canonical_browser_url(value: &str) -> Option<String> {
    let mut url = url::Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
    {
        return None;
    }
    url.set_fragment(None);
    Some(url.into())
}

fn is_plugin_command(request: &LocalHostCommandRequest) -> bool {
    matches!(
        request,
        LocalHostCommandRequest::PluginChooseAndInstallBundle
            | LocalHostCommandRequest::PluginChooseAndInstall
            | LocalHostCommandRequest::PluginInstallBuiltinSampleCrm
            | LocalHostCommandRequest::PluginInstallBuiltinEnterprise { .. }
            | LocalHostCommandRequest::PluginList
            | LocalHostCommandRequest::PluginListContributions { .. }
            | LocalHostCommandRequest::PluginGetContributionSurface { .. }
            | LocalHostCommandRequest::PluginGet { .. }
            | LocalHostCommandRequest::PluginHealthCheck { .. }
            | LocalHostCommandRequest::PluginPermissionDiff { .. }
            | LocalHostCommandRequest::PluginRevisionHead { .. }
            | LocalHostCommandRequest::PluginListRevisions { .. }
            | LocalHostCommandRequest::PluginLifecycleJournal { .. }
            | LocalHostCommandRequest::PluginSetEnabled { .. }
            | LocalHostCommandRequest::PluginRollback { .. }
            | LocalHostCommandRequest::PluginUninstall { .. }
    )
}
