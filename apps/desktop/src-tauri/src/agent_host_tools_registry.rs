use super::*;

pub(crate) struct LocalHostToolContext {
    pub(crate) browser: Arc<hachimi_browser::BrowserHost>,
    pub(crate) embedded_browser: Arc<crate::embedded_browser_agent::EmbeddedAgentBrowser>,
    pub(crate) computer: Arc<hachimi_computer::ComputerHost>,
    pub(crate) plugins: hachimi_extensions::PluginHost,
    pub(crate) store: hachimi_storage::AgentStore,
    pub(crate) session_id: SessionId,
    pub(crate) run_id: hachimi_protocol::RunId,
    pub(crate) grants: CapabilityGrantSet,
    pub(crate) authority_mode: hachimi_protocol::AuthorityMode,
    pub(crate) sandbox: hachimi_protocol::SandboxCapabilityReport,
    pub(crate) host_revision_snapshot: Option<HostRevisionSnapshot>,
    pub(crate) browser_enabled: bool,
    pub(crate) computer_observe_enabled: bool,
    pub(crate) computer_control_enabled: bool,
    pub(crate) enterprise_integrations_enabled: bool,
    pub(crate) browser_environment_change_sink: EnvironmentChangeSink,
    pub(crate) source_environment_change_sink: EnvironmentChangeSink,
}

pub(crate) fn local_host_tool_executors(
    context: LocalHostToolContext,
) -> Vec<Arc<dyn ToolExecutor>> {
    let LocalHostToolContext {
        browser,
        embedded_browser,
        computer,
        plugins,
        store,
        session_id,
        run_id,
        grants,
        authority_mode,
        sandbox,
        host_revision_snapshot,
        browser_enabled,
        computer_observe_enabled,
        computer_control_enabled,
        enterprise_integrations_enabled,
        browser_environment_change_sink,
        source_environment_change_sink,
    } = context;
    let mut tools: Vec<Arc<dyn ToolExecutor>> = Vec::new();
    if enterprise_integrations_enabled {
        tools.extend([
            Arc::new(ConnectorListTool {
                host: plugins.clone(),
                host_revision_snapshot: host_revision_snapshot.clone(),
            }),
            Arc::new(ConnectorInvokeTool {
                host: plugins.clone(),
                store: store.clone(),
                session_id: session_id.clone(),
                run_id: run_id.clone(),
                host_revision_snapshot: host_revision_snapshot.clone(),
                environment_change_sink: source_environment_change_sink,
            }),
            Arc::new(EnterpriseAttachmentDownloadTool {
                host: plugins,
                store: store.clone(),
                session_id: session_id.clone(),
                run_id: run_id.clone(),
                host_revision_snapshot: host_revision_snapshot.clone(),
            }),
        ] as [Arc<dyn ToolExecutor>; 3]);
    }
    if browser_enabled {
        tools.extend([
            Arc::new(BrowserStartTool {
                host: Arc::clone(&browser),
                embedded: Arc::clone(&embedded_browser),
                store: store.clone(),
                session_id: session_id.clone(),
                run_id: run_id.clone(),
                grants: grants.clone(),
                authority_mode,
                sandbox: sandbox.clone(),
                environment_change_sink: Arc::clone(&browser_environment_change_sink),
            }),
            Arc::new(BrowserObserveTool {
                host: Arc::clone(&browser),
                embedded: Arc::clone(&embedded_browser),
                store: store.clone(),
                session_id: session_id.clone(),
                run_id: run_id.clone(),
                grants: grants.clone(),
                authority_mode,
                environment_change_sink: Arc::clone(&browser_environment_change_sink),
            }),
            Arc::new(BrowserActTool {
                host: Arc::clone(&browser),
                embedded: Arc::clone(&embedded_browser),
                store: store.clone(),
                session_id: session_id.clone(),
                run_id: run_id.clone(),
                grants: grants.clone(),
                authority_mode,
                environment_change_sink: Arc::clone(&browser_environment_change_sink),
            }),
            Arc::new(BrowserStopTool {
                host: browser,
                embedded: embedded_browser,
                store: store.clone(),
                session_id: session_id.clone(),
                run_id: run_id.clone(),
                environment_change_sink: browser_environment_change_sink,
            }),
        ] as [Arc<dyn ToolExecutor>; 4]);
    }
    if computer_observe_enabled {
        tools.extend([
            Arc::new(ComputerListWindowsTool {
                host: Arc::clone(&computer),
            }),
            Arc::new(ComputerObserveTool {
                host: Arc::clone(&computer),
                store: store.clone(),
                session_id: session_id.clone(),
                run_id: run_id.clone(),
                grants: grants.clone(),
                authority_mode,
                sandbox,
            }),
        ] as [Arc<dyn ToolExecutor>; 2]);
    }
    if computer_control_enabled {
        tools.extend([
            Arc::new(ComputerActTool {
                host: Arc::clone(&computer),
                store: store.clone(),
                session_id: session_id.clone(),
                run_id: run_id.clone(),
                grants,
            }),
            Arc::new(ComputerStopTool {
                host: computer,
                store: store.clone(),
                session_id: session_id.clone(),
            }),
        ] as [Arc<dyn ToolExecutor>; 2]);
    }
    tools
}
