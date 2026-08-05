//! Frontend-safe command contract for local Browser, Computer and extension Hosts.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LocalHostCommandRequest {
    BrowserListPermissions,
    BrowserListPermissionRequests,
    BrowserStart {
        session_id: SessionId,
        run_id: RunId,
        profile_kind: BrowserProfileKind,
        initial_url: Option<String>,
        pairing_id: Option<BrowserPairingId>,
    },
    BrowserGrantSitePermission {
        context: MutationContext,
        session_id: SessionId,
        run_id: RunId,
        browser_session_id: BrowserSessionId,
        #[specta(type = specta_typescript::Number)]
        expected_revision: u64,
        origin: String,
        capabilities: Vec<BrowserCapability>,
        decision: BrowserPermissionDecision,
        network_kind: BrowserNetworkRuleKind,
        allow_private_network: bool,
        #[specta(type = Option<specta_typescript::Number>)]
        expires_at_ms: Option<i64>,
    },
    BrowserRevokeSitePermission {
        context: MutationContext,
        session_id: SessionId,
        run_id: RunId,
        browser_session_id: BrowserSessionId,
        #[specta(type = specta_typescript::Number)]
        expected_revision: u64,
        origin: String,
    },
    BrowserObserve {
        browser_session_id: BrowserSessionId,
        run_id: RunId,
    },
    BrowserAct {
        run_id: RunId,
        request: BrowserActionRequest,
    },
    BrowserChooseUpload {
        browser_session_id: BrowserSessionId,
        run_id: RunId,
    },
    BrowserImportDownload {
        browser_session_id: BrowserSessionId,
        run_id: RunId,
        download_token: String,
        suggested_file_name: String,
    },
    BrowserTakeOver {
        browser_session_id: BrowserSessionId,
        run_id: RunId,
    },
    BrowserStop {
        browser_session_id: BrowserSessionId,
        run_id: RunId,
    },
    ComputerSetAppRule {
        session_id: SessionId,
        rule: ComputerAppRule,
    },
    ComputerListWindows,
    ComputerListGlobalAppRules,
    ComputerSetGlobalAppRule {
        rule: ComputerAppRule,
    },
    ComputerRemoveGlobalAppRule {
        app_id: String,
    },
    ComputerObserve {
        session_id: SessionId,
        run_id: RunId,
        window_handle: String,
    },
    ComputerAct {
        run_id: RunId,
        request: ComputerActionRequest,
    },
    ComputerTakeOver {
        session_id: SessionId,
    },
    PluginChooseAndInstallBundle,
    PluginChooseAndInstall,
    PluginInstallBuiltinSampleCrm,
    PluginInstallBuiltinEnterprise {
        provider_id: IntegrationProviderId,
    },
    PluginList,
    PluginListContributions {
        plugin_id: Option<PluginId>,
    },
    PluginGetContributionSurface {
        plugin_id: PluginId,
        contribution_id: String,
    },
    PluginGet {
        plugin_id: PluginId,
    },
    PluginHealthCheck {
        plugin_id: PluginId,
    },
    PluginPermissionDiff {
        plugin_id: PluginId,
    },
    PluginRevisionHead {
        plugin_id: PluginId,
    },
    PluginListRevisions {
        plugin_id: PluginId,
    },
    PluginLifecycleJournal {
        plugin_id: Option<PluginId>,
    },
    PluginSetEnabled {
        plugin_id: PluginId,
        enabled: bool,
    },
    PluginRollback {
        plugin_id: PluginId,
        revision: Option<String>,
    },
    PluginUninstall {
        plugin_id: PluginId,
    },
    ConnectorUpsertAccount {
        account: ConnectorAccountUpsert,
    },
    ConnectorListAccounts,
    ConnectorGetAccount {
        account_id: ConnectorAccountId,
    },
    ConnectorGetDriverDescriptor {
        plugin_id: PluginId,
        connector_id: String,
    },
    ConnectorRevokeAccount {
        account_id: ConnectorAccountId,
    },
    ConnectorInvoke {
        request: ConnectorInvocationRequest,
    },
    ChannelLoopbackReceive {
        bearer_token: String,
        message: VerifiedChannelMessage,
    },
    ChannelMockPollPush {
        message: VerifiedChannelMessage,
    },
    ChannelMockPollSetConnected {
        connected: bool,
    },
    ChannelMockPollDrain,
    ChannelEnqueueDelivery {
        address: ChannelConversationAddress,
        idempotency_key: String,
        text: String,
    },
    GatewayHealth,
    GatewayProviderManifests,
    GatewayProviderHealth,
    GatewayListProviderAccounts,
    GatewayUpsertProviderAccount {
        account: ChannelProviderAccountUpsert,
    },
    GatewayReconcile,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum LocalHostCommandResponse {
    Cancelled,
    BrowserSession(BrowserSession),
    BrowserPermission(BrowserSitePermission),
    BrowserPermissions(Vec<BrowserPermissionLedgerEntry>),
    BrowserPermissionRequests(Vec<BrowserPermissionRequest>),
    BrowserObservation(BrowserObservation),
    BrowserAction(BrowserActionResult),
    BrowserFileToken(BrowserFileToken),
    BrowserImportedDownload(BrowserImportedDownload),
    ComputerRule(ComputerAppRule),
    ComputerRules(Vec<ComputerAppRule>),
    ComputerWindows(Vec<ComputerWindowIdentity>),
    ComputerFrame(ComputerFrame),
    ComputerAction(ComputerActionResult),
    ComputerTakenOver(#[specta(type = specta_typescript::Number)] u64),
    Plugin(Option<InstalledPlugin>),
    Plugins(Vec<InstalledPlugin>),
    PluginContributions(Vec<InstalledContribution>),
    PluginContributionSurface(PluginContributionSurface),
    PluginPermissionDiff(Option<super::PluginPermissionDiff>),
    PluginRevisionHead(Option<super::PluginRevisionHead>),
    PluginRevisions(Vec<super::PluginRevisionRecord>),
    PluginLifecycleJournal(Vec<super::PluginLifecycleJournalRecord>),
    Removed(bool),
    ConnectorAccount(Option<ConnectorAccount>),
    ConnectorAccounts(Vec<ConnectorAccount>),
    ConnectorDriverDescriptor(ConnectorDriverDescriptor),
    ConnectorInvocation(ConnectorInvocationResult),
    Ingress(IngressReceipt),
    Ingresses(Vec<IngressReceipt>),
    MockPollConnected(bool),
    Delivery(DeliveryAttempt),
    GatewayHealth(GatewayHealth),
    ChannelProviderManifests(Vec<ChannelProviderManifest>),
    ChannelProviderHealth(Vec<ChannelProviderHealth>),
    ChannelProviderAccounts(Vec<ChannelProviderAccount>),
    ChannelProviderAccount(ChannelProviderAccount),
    GatewayReconciled,
}
