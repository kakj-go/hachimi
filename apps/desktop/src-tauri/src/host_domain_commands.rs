use hachimi_protocol::{
    ChannelProviderAccount, ChannelProviderHealth, ChannelProviderManifest, ConnectorAccount,
    ConnectorDriverDescriptor, GatewayHealth, InstalledContribution, InstalledPlugin,
    LocalHostCommandRequest, LocalHostCommandResponse, PluginContributionSurface,
    PluginLifecycleJournalRecord, PluginPermissionDiff, PluginRevisionRecord,
};

use super::*;

async fn execute(
    window: &WebviewWindow,
    state: &DesktopState,
    request: LocalHostCommandRequest,
) -> Result<LocalHostCommandResponse, CommandError> {
    crate::local_host_commands::execute_local_host_command(window, state, request).await
}

fn mismatch(expected: &str) -> CommandError {
    CommandError::new(
        "host_domain_protocol_mismatch",
        format!("Host domain returned a response other than {expected}"),
    )
}

#[tauri::command]
pub(super) async fn list_installed_plugins(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Vec<InstalledPlugin>, CommandError> {
    match execute(&window, &state, LocalHostCommandRequest::PluginList).await? {
        LocalHostCommandResponse::Plugins(plugins) => Ok(plugins),
        _ => Err(mismatch("plugins")),
    }
}

#[tauri::command]
pub(super) async fn list_installed_plugin_contributions(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    plugin_id: Option<hachimi_protocol::PluginId>,
) -> Result<Vec<InstalledContribution>, CommandError> {
    match execute(
        &window,
        &state,
        LocalHostCommandRequest::PluginListContributions { plugin_id },
    )
    .await?
    {
        LocalHostCommandResponse::PluginContributions(contributions) => Ok(contributions),
        _ => Err(mismatch("plugin contributions")),
    }
}

#[tauri::command]
pub(super) async fn get_plugin_contribution_surface(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    plugin_id: hachimi_protocol::PluginId,
    contribution_id: String,
) -> Result<PluginContributionSurface, CommandError> {
    match execute(
        &window,
        &state,
        LocalHostCommandRequest::PluginGetContributionSurface {
            plugin_id,
            contribution_id,
        },
    )
    .await?
    {
        LocalHostCommandResponse::PluginContributionSurface(surface) => Ok(surface),
        _ => Err(mismatch("plugin contribution surface")),
    }
}

async fn mutate_plugin(
    window: &WebviewWindow,
    state: &DesktopState,
    request: LocalHostCommandRequest,
) -> Result<Option<InstalledPlugin>, CommandError> {
    match execute(window, state, request).await? {
        LocalHostCommandResponse::Plugin(plugin) => Ok(plugin),
        _ => Err(mismatch("plugin")),
    }
}

#[tauri::command]
pub(super) async fn check_plugin_health(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    plugin_id: hachimi_protocol::PluginId,
) -> Result<Option<InstalledPlugin>, CommandError> {
    mutate_plugin(
        &window,
        &state,
        LocalHostCommandRequest::PluginHealthCheck { plugin_id },
    )
    .await
}

#[tauri::command]
pub(super) async fn get_plugin_permission_diff(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    plugin_id: hachimi_protocol::PluginId,
) -> Result<Option<PluginPermissionDiff>, CommandError> {
    match execute(
        &window,
        &state,
        LocalHostCommandRequest::PluginPermissionDiff { plugin_id },
    )
    .await?
    {
        LocalHostCommandResponse::PluginPermissionDiff(diff) => Ok(diff),
        _ => Err(mismatch("plugin permission diff")),
    }
}

#[tauri::command]
pub(super) async fn list_plugin_revisions(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    plugin_id: hachimi_protocol::PluginId,
) -> Result<Vec<PluginRevisionRecord>, CommandError> {
    match execute(
        &window,
        &state,
        LocalHostCommandRequest::PluginListRevisions { plugin_id },
    )
    .await?
    {
        LocalHostCommandResponse::PluginRevisions(revisions) => Ok(revisions),
        _ => Err(mismatch("plugin revisions")),
    }
}

#[tauri::command]
pub(super) async fn list_plugin_lifecycle_journal(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    plugin_id: Option<hachimi_protocol::PluginId>,
) -> Result<Vec<PluginLifecycleJournalRecord>, CommandError> {
    match execute(
        &window,
        &state,
        LocalHostCommandRequest::PluginLifecycleJournal { plugin_id },
    )
    .await?
    {
        LocalHostCommandResponse::PluginLifecycleJournal(journal) => Ok(journal),
        _ => Err(mismatch("plugin lifecycle journal")),
    }
}

#[tauri::command]
pub(super) async fn list_connector_accounts(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Vec<ConnectorAccount>, CommandError> {
    match execute(
        &window,
        &state,
        LocalHostCommandRequest::ConnectorListAccounts,
    )
    .await?
    {
        LocalHostCommandResponse::ConnectorAccounts(accounts) => Ok(accounts),
        _ => Err(mismatch("connector accounts")),
    }
}

#[tauri::command]
pub(super) async fn get_connector_driver_descriptor(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    plugin_id: hachimi_protocol::PluginId,
    connector_id: String,
) -> Result<ConnectorDriverDescriptor, CommandError> {
    match execute(
        &window,
        &state,
        LocalHostCommandRequest::ConnectorGetDriverDescriptor {
            plugin_id,
            connector_id,
        },
    )
    .await?
    {
        LocalHostCommandResponse::ConnectorDriverDescriptor(descriptor) => Ok(descriptor),
        _ => Err(mismatch("connector driver descriptor")),
    }
}

#[tauri::command]
pub(super) async fn get_gateway_health(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<GatewayHealth, CommandError> {
    match execute(&window, &state, LocalHostCommandRequest::GatewayHealth).await? {
        LocalHostCommandResponse::GatewayHealth(health) => Ok(health),
        _ => Err(mismatch("gateway health")),
    }
}

#[tauri::command]
pub(super) async fn list_channel_provider_manifests(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Vec<ChannelProviderManifest>, CommandError> {
    match execute(
        &window,
        &state,
        LocalHostCommandRequest::GatewayProviderManifests,
    )
    .await?
    {
        LocalHostCommandResponse::ChannelProviderManifests(manifests) => Ok(manifests),
        _ => Err(mismatch("channel provider manifests")),
    }
}

#[tauri::command]
pub(super) async fn list_channel_provider_health(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Vec<ChannelProviderHealth>, CommandError> {
    match execute(
        &window,
        &state,
        LocalHostCommandRequest::GatewayProviderHealth,
    )
    .await?
    {
        LocalHostCommandResponse::ChannelProviderHealth(health) => Ok(health),
        _ => Err(mismatch("channel provider health")),
    }
}

#[tauri::command]
pub(super) async fn list_channel_provider_accounts(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Vec<ChannelProviderAccount>, CommandError> {
    match execute(
        &window,
        &state,
        LocalHostCommandRequest::GatewayListProviderAccounts,
    )
    .await?
    {
        LocalHostCommandResponse::ChannelProviderAccounts(accounts) => Ok(accounts),
        _ => Err(mismatch("channel provider accounts")),
    }
}
