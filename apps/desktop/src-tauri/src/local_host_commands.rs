use hachimi_control_plane::{
    AppServerContext, AppServerDomainRequest, AppServerDomainResponse, AppServerError,
    AppServerRequest, AppServerResponse, BrowserAppRequest, BrowserAppResponse, ChannelAppRequest,
    ChannelAppResponse, ComputerAppRequest, ComputerAppResponse, ConnectorAppRequest,
    ConnectorAppResponse, GatewayAppRequest, GatewayAppResponse, PluginAppRequest,
    PluginAppResponse,
};
use hachimi_core::WindowKind;
use hachimi_protocol::{ClientContext, LocalHostCommandRequest, LocalHostCommandResponse};
use tauri::{Manager, State, WebviewWindow};

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
        LocalHostCommandRequest::PluginInstallBuiltinEnterprise { platform } => {
            let resource_dir =
                window.app_handle().path().resource_dir().map_err(|error| {
                    CommandError::operation("plugin_resource_dir_failed", error)
                })?;
            let bundle_name = match platform {
                hachimi_protocol::EnterprisePlatform::Wecom => "wecom",
                hachimi_protocol::EnterprisePlatform::DingTalk => "dingtalk",
                hachimi_protocol::EnterprisePlatform::Feishu => "feishu",
            };
            AppServerDomainRequest::Plugin(PluginAppRequest::InstallLocal(
                resource_dir.join("plugins").join(bundle_name),
            ))
        }
        LocalHostCommandRequest::BrowserBeginPairing => {
            AppServerDomainRequest::Browser(BrowserAppRequest::BeginPairing)
        }
        LocalHostCommandRequest::BrowserGetHostSettings => {
            AppServerDomainRequest::Browser(BrowserAppRequest::GetHostSettings)
        }
        LocalHostCommandRequest::BrowserListPermissions => {
            AppServerDomainRequest::Browser(BrowserAppRequest::ListPermissions)
        }
        LocalHostCommandRequest::BrowserListPermissionRequests => {
            AppServerDomainRequest::Browser(BrowserAppRequest::ListPermissionRequests)
        }
        LocalHostCommandRequest::BrowserSetPreferredProfile { profile_kind } => {
            AppServerDomainRequest::Browser(BrowserAppRequest::SetPreferredProfile(profile_kind))
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
            envelope,
        } => AppServerDomainRequest::Channel(ChannelAppRequest::LoopbackReceive {
            bearer_token,
            envelope,
        }),
        LocalHostCommandRequest::ChannelMockPollPush { envelope } => {
            AppServerDomainRequest::Channel(ChannelAppRequest::MockPollPush(envelope))
        }
        LocalHostCommandRequest::ChannelMockPollSetConnected { connected } => {
            AppServerDomainRequest::Channel(ChannelAppRequest::MockPollSetConnected(connected))
        }
        LocalHostCommandRequest::ChannelMockPollDrain => {
            AppServerDomainRequest::Channel(ChannelAppRequest::MockPollDrain)
        }
        LocalHostCommandRequest::ChannelEnqueueDelivery {
            route,
            idempotency_key,
            text,
        } => AppServerDomainRequest::Channel(ChannelAppRequest::EnqueueDelivery {
            route,
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
        LocalHostCommandRequest::GatewaySetStartupEnabled { enabled } => {
            AppServerDomainRequest::Gateway(GatewayAppRequest::SetStartupEnabled(enabled))
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
        AppServerDomainResponse::Browser(BrowserAppResponse::Pairing(value)) => {
            LocalHostCommandResponse::BrowserPairing(value)
        }
        AppServerDomainResponse::Browser(BrowserAppResponse::HostSettings(value)) => {
            LocalHostCommandResponse::BrowserHostSettings(value)
        }
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
            LocalHostCommandResponse::ComputerFrame(value)
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

#[tauri::command]
pub(super) async fn local_host_command(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: LocalHostCommandRequest,
) -> Result<LocalHostCommandResponse, CommandError> {
    let runtime = state.control_plane.feature_flags().runtime_features;
    if is_plugin_command(&request) && !runtime.plugin_runtime {
        return Err(CommandError::new("feature_disabled", "plugin_runtime"));
    }
    if is_desktop_control_command(&request) && !runtime.desktop_control {
        return Err(CommandError::new("feature_disabled", "desktop_control"));
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
    let Some(request) = request_to_domain(&window, request).await? else {
        return Ok(LocalHostCommandResponse::Cancelled);
    };
    domain_to_response(dispatch(&window, &state, request).await?)
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

fn is_desktop_control_command(request: &LocalHostCommandRequest) -> bool {
    matches!(
        request,
        LocalHostCommandRequest::BrowserBeginPairing
            | LocalHostCommandRequest::BrowserGetHostSettings
            | LocalHostCommandRequest::BrowserListPermissions
            | LocalHostCommandRequest::BrowserListPermissionRequests
            | LocalHostCommandRequest::BrowserSetPreferredProfile { .. }
            | LocalHostCommandRequest::BrowserStart { .. }
            | LocalHostCommandRequest::BrowserGrantSitePermission { .. }
            | LocalHostCommandRequest::BrowserRevokeSitePermission { .. }
            | LocalHostCommandRequest::BrowserObserve { .. }
            | LocalHostCommandRequest::BrowserAct { .. }
            | LocalHostCommandRequest::BrowserChooseUpload { .. }
            | LocalHostCommandRequest::BrowserImportDownload { .. }
            | LocalHostCommandRequest::BrowserTakeOver { .. }
            | LocalHostCommandRequest::BrowserStop { .. }
            | LocalHostCommandRequest::ComputerSetAppRule { .. }
            | LocalHostCommandRequest::ComputerListWindows
            | LocalHostCommandRequest::ComputerListGlobalAppRules
            | LocalHostCommandRequest::ComputerSetGlobalAppRule { .. }
            | LocalHostCommandRequest::ComputerRemoveGlobalAppRule { .. }
            | LocalHostCommandRequest::ComputerObserve { .. }
            | LocalHostCommandRequest::ComputerAct { .. }
            | LocalHostCommandRequest::ComputerTakeOver { .. }
    )
}
