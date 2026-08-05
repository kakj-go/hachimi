// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/app-server/src/request_processors/*
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// Modified for Hachimi: transport-neutral domain routing with Session/Run
// generation fencing delegated to the authoritative domain services.

//! Typed non-lifecycle requests routed by the App Server.
//!
//! The App Server owns authorization and request routing. Domain handlers own
//! filesystem, process, extension, review, and automation state. Keeping this
//! trait free of Tauri types lets the same handler run from a Workbench window,
//! the background scheduler, or a future local socket transport.

use std::{future::Future, pin::Pin};

use hachimi_protocol::{
    BrowserActionRequest, BrowserActionResult, BrowserCapability, BrowserFileToken,
    BrowserImportedDownload, BrowserObservation, BrowserPairingId, BrowserPermissionDecision,
    BrowserProfileKind, BrowserSession, BrowserSessionId, BrowserSitePermission,
    ChannelConversationAddress, ComputerActionRequest, ComputerActionResult, ComputerAppRule,
    ComputerFrame, ComputerWindowIdentity, ConnectorAccount, ConnectorAccountId,
    ConnectorAccountUpsert, ConnectorInvocationRequest, ConnectorInvocationResult, ControlMethod,
    DeliveryAttempt, DiffReadFileRequest, DiffReadFileResponse, DiffScope, FsFileChunk, FsListPage,
    FsListRequest, FsReadChunkRequest, FsSearchId, FsSearchSnapshot, FsSearchStartRequest,
    FsSearchUpdateRequest, FsWatchId, FsWatchRegistration, FsWatchRequest, GatewayHealth,
    IngressReceipt, InstalledPlugin, McpAuthStatusRecord, McpCallSummaryListRequest,
    McpCallSummaryRecord, McpInventorySnapshot, McpOAuthLoginRequest, McpOAuthLoginResponse,
    McpPromptGetRequest, McpPromptResult, McpResourceContent, McpResourceReadRequest, McpServerId,
    MutationContext, PluginId, ProcessListRequest, ProcessReadRequest, ProcessReadSnapshot,
    ProcessResizeRequest, ProcessSessionRecord, ProcessSpawnRequest, ProcessTerminateRequest,
    ProcessWriteRequest, ProjectId, ReviewFinding, ReviewFindingUpdateRequest, ReviewId,
    ReviewSnapshot, ReviewStartRequest, ReviewStartSnapshot, RunDiffSnapshot,
    ScheduleCreateRequest, ScheduleDefinition, ScheduleEventIngressRequest, ScheduleEventReceipt,
    ScheduleGrantRecord, ScheduleId, SchedulePreview, ScheduleSnapshot, ScheduleSpec,
    ScheduleUpdateRequest, SessionId, SkillEntryCreateRequest, SkillEntryRenameRequest,
    SkillFileSnapshot, SkillFileWriteRequest, SkillId, SkillPreviewResource,
    SkillPreviewResourceRequest, SkillRecord, SkillTreeNode, TaskInteractiveContinuation,
    TaskRunId, TaskRunRecord, VerifiedChannelMessage,
};

use crate::AppServerContext;

#[derive(Debug)]
pub enum AppServerDomainRequest {
    Fs(FsAppRequest),
    Process(ProcessAppRequest),
    Review(ReviewAppRequest),
    Mcp(McpAppRequest),
    Skills(SkillsAppRequest),
    Schedule(Box<ScheduleAppRequest>),
    Task(TaskAppRequest),
    Browser(BrowserAppRequest),
    Computer(ComputerAppRequest),
    Plugin(PluginAppRequest),
    Connector(ConnectorAppRequest),
    Channel(ChannelAppRequest),
    Gateway(GatewayAppRequest),
}

impl AppServerDomainRequest {
    #[must_use]
    pub const fn control_method(&self) -> ControlMethod {
        match self {
            Self::Mcp(_) => ControlMethod::ConnectorsManage,
            Self::Browser(BrowserAppRequest::Observe { .. }) => ControlMethod::BrowserObserve,
            Self::Browser(_) => ControlMethod::BrowserControl,
            Self::Computer(ComputerAppRequest::Observe { .. }) => ControlMethod::ComputerObserve,
            Self::Computer(_) => ControlMethod::ComputerControl,
            Self::Plugin(_) => ControlMethod::ConnectorsManage,
            Self::Connector(ConnectorAppRequest::Invoke(_)) => ControlMethod::ConnectorsInvoke,
            Self::Connector(_) => ControlMethod::ConnectorsManage,
            Self::Channel(_) => ControlMethod::ChannelsManage,
            Self::Gateway(_) => ControlMethod::GatewayManage,
            Self::Skills(_) => ControlMethod::SkillsManage,
            Self::Fs(_)
            | Self::Process(_)
            | Self::Review(_)
            | Self::Schedule(_)
            | Self::Task(_) => ControlMethod::WorkbenchWindow,
        }
    }
}

#[derive(Debug)]
pub enum AppServerDomainResponse {
    Fs(FsAppResponse),
    Process(ProcessAppResponse),
    Review(Box<ReviewAppResponse>),
    Mcp(McpAppResponse),
    Skills(SkillsAppResponse),
    Schedule(Box<ScheduleAppResponse>),
    Task(Box<TaskAppResponse>),
    Browser(BrowserAppResponse),
    Computer(ComputerAppResponse),
    Plugin(PluginAppResponse),
    Connector(ConnectorAppResponse),
    Channel(ChannelAppResponse),
    Gateway(GatewayAppResponse),
}

#[derive(Debug)]
pub enum BrowserAppRequest {
    ListPermissions,
    ListPermissionRequests,
    Start {
        session_id: SessionId,
        run_id: hachimi_protocol::RunId,
        profile_kind: BrowserProfileKind,
        initial_url: Option<String>,
        pairing_id: Option<BrowserPairingId>,
    },
    GrantSitePermission {
        context: hachimi_protocol::MutationContext,
        session_id: SessionId,
        run_id: hachimi_protocol::RunId,
        browser_session_id: BrowserSessionId,
        expected_revision: u64,
        origin: String,
        capabilities: Vec<BrowserCapability>,
        decision: BrowserPermissionDecision,
        network_kind: hachimi_protocol::BrowserNetworkRuleKind,
        allow_private_network: bool,
        expires_at_ms: Option<i64>,
    },
    RevokeSitePermission {
        context: hachimi_protocol::MutationContext,
        session_id: SessionId,
        run_id: hachimi_protocol::RunId,
        browser_session_id: BrowserSessionId,
        expected_revision: u64,
        origin: String,
    },
    Observe {
        browser_session_id: BrowserSessionId,
        run_id: hachimi_protocol::RunId,
    },
    Act {
        run_id: hachimi_protocol::RunId,
        request: BrowserActionRequest,
    },
    StageUpload {
        browser_session_id: BrowserSessionId,
        run_id: hachimi_protocol::RunId,
        source: std::path::PathBuf,
    },
    ImportDownload {
        browser_session_id: BrowserSessionId,
        run_id: hachimi_protocol::RunId,
        download_token: String,
        destination: std::path::PathBuf,
    },
    TakeOver {
        browser_session_id: BrowserSessionId,
        run_id: hachimi_protocol::RunId,
    },
    Stop {
        browser_session_id: BrowserSessionId,
        run_id: hachimi_protocol::RunId,
    },
}

#[derive(Debug)]
pub enum BrowserAppResponse {
    Session(BrowserSession),
    Permission(BrowserSitePermission),
    Permissions(Vec<hachimi_protocol::BrowserPermissionLedgerEntry>),
    PermissionRequests(Vec<hachimi_protocol::BrowserPermissionRequest>),
    PermissionRevoked(bool),
    Observation(BrowserObservation),
    Action(BrowserActionResult),
    FileToken(BrowserFileToken),
    ImportedDownload(BrowserImportedDownload),
}

#[derive(Debug)]
pub enum ComputerAppRequest {
    ListWindows,
    ListGlobalAppRules,
    SetGlobalAppRule(ComputerAppRule),
    RemoveGlobalAppRule(String),
    SetAppRule {
        session_id: SessionId,
        rule: ComputerAppRule,
    },
    Observe {
        session_id: SessionId,
        run_id: hachimi_protocol::RunId,
        window_handle: String,
    },
    Act {
        run_id: hachimi_protocol::RunId,
        request: ComputerActionRequest,
    },
    TakeOver(SessionId),
}

#[derive(Debug)]
pub enum ComputerAppResponse {
    Rule(ComputerAppRule),
    Rules(Vec<ComputerAppRule>),
    Windows(Vec<ComputerWindowIdentity>),
    Removed(bool),
    Frame(ComputerFrame),
    Action(ComputerActionResult),
    TakenOver(u64),
}

#[derive(Debug)]
pub enum PluginAppRequest {
    InstallLocal(std::path::PathBuf),
    List,
    ListContributions(Option<PluginId>),
    GetContributionSurface {
        plugin_id: PluginId,
        contribution_id: String,
    },
    Get(PluginId),
    HealthCheck(PluginId),
    PermissionDiff(PluginId),
    RevisionHead(PluginId),
    ListRevisions(PluginId),
    LifecycleJournal(Option<PluginId>),
    SetEnabled {
        plugin_id: PluginId,
        enabled: bool,
    },
    Rollback {
        plugin_id: PluginId,
        revision: Option<String>,
    },
    Uninstall(PluginId),
}

#[derive(Debug)]
pub enum PluginAppResponse {
    Plugin(InstalledPlugin),
    OptionalPlugin(Option<InstalledPlugin>),
    Plugins(Vec<InstalledPlugin>),
    Contributions(Vec<hachimi_protocol::InstalledContribution>),
    ContributionSurface(hachimi_protocol::PluginContributionSurface),
    PermissionDiff(Option<hachimi_protocol::PluginPermissionDiff>),
    RevisionHead(Option<hachimi_protocol::PluginRevisionHead>),
    Revisions(Vec<hachimi_protocol::PluginRevisionRecord>),
    LifecycleJournal(Vec<hachimi_protocol::PluginLifecycleJournalRecord>),
    Removed(bool),
}

#[derive(Debug)]
pub enum ConnectorAppRequest {
    UpsertAccount(ConnectorAccountUpsert),
    ListAccounts,
    GetAccount(ConnectorAccountId),
    GetDriverDescriptor {
        plugin_id: hachimi_protocol::PluginId,
        connector_id: String,
    },
    RevokeAccount(ConnectorAccountId),
    Invoke(ConnectorInvocationRequest),
}

#[derive(Debug)]
pub enum ConnectorAppResponse {
    Account(ConnectorAccount),
    Accounts(Vec<ConnectorAccount>),
    OptionalAccount(Option<ConnectorAccount>),
    DriverDescriptor(hachimi_protocol::ConnectorDriverDescriptor),
    Invocation(ConnectorInvocationResult),
}

#[derive(Debug)]
pub enum ChannelAppRequest {
    /// Authenticated, already-ledgered ingress dispatched by the Gateway
    /// supervisor. The Gateway owns transport/claim state; the App Server
    /// owns Session/Run creation and execution.
    DispatchIngress {
        message: VerifiedChannelMessage,
    },
    LoopbackReceive {
        bearer_token: String,
        message: VerifiedChannelMessage,
    },
    MockPollPush(VerifiedChannelMessage),
    MockPollSetConnected(bool),
    MockPollDrain,
    EnqueueDelivery {
        address: ChannelConversationAddress,
        idempotency_key: String,
        text: String,
    },
}

#[derive(Debug)]
pub enum ChannelAppResponse {
    Ingress(IngressReceipt),
    Ingresses(Vec<IngressReceipt>),
    MockPollConnected(bool),
    Delivery(DeliveryAttempt),
}

#[derive(Debug)]
pub enum GatewayAppRequest {
    Health,
    ProviderManifests,
    ProviderHealth,
    ListProviderAccounts,
    UpsertProviderAccount(hachimi_protocol::ChannelProviderAccountUpsert),
    Reconcile,
}

#[derive(Debug)]
pub enum GatewayAppResponse {
    Health(GatewayHealth),
    ProviderManifests(Vec<hachimi_protocol::ChannelProviderManifest>),
    ProviderHealth(Vec<hachimi_protocol::ChannelProviderHealth>),
    ProviderAccounts(Vec<hachimi_protocol::ChannelProviderAccount>),
    ProviderAccount(hachimi_protocol::ChannelProviderAccount),
    Reconciled,
}

#[derive(Debug)]
pub enum FsAppRequest {
    List(FsListRequest),
    ReadChunk(FsReadChunkRequest),
    Watch(FsWatchRequest),
    Unwatch(FsWatchId),
    SearchStart(FsSearchStartRequest),
    SearchUpdate(FsSearchUpdateRequest),
    SearchCancel(FsSearchId),
    DiffGet(DiffScope),
    DiffReadFile(DiffReadFileRequest),
}

#[derive(Debug)]
pub enum FsAppResponse {
    List(FsListPage),
    FileChunk(FsFileChunk),
    Watch(FsWatchRegistration),
    Unwatched(bool),
    Search(FsSearchSnapshot),
    SearchCancelled(bool),
    Diff(RunDiffSnapshot),
    DiffFile(DiffReadFileResponse),
}

#[derive(Debug)]
pub enum ProcessAppRequest {
    Spawn(ProcessSpawnRequest),
    Write(ProcessWriteRequest),
    Resize(ProcessResizeRequest),
    Terminate(ProcessTerminateRequest),
    Read(ProcessReadRequest),
    List(ProcessListRequest),
}

#[derive(Debug)]
pub enum ProcessAppResponse {
    Process(ProcessSessionRecord),
    Processes(Vec<ProcessSessionRecord>),
    Read(ProcessReadSnapshot),
    Acknowledged,
}

#[derive(Debug)]
pub enum ReviewAppRequest {
    Start(ReviewStartRequest),
    Get(ReviewId),
    List(SessionId),
    UpdateFinding(ReviewFindingUpdateRequest),
}

#[derive(Debug)]
pub enum ReviewAppResponse {
    Started(ReviewStartSnapshot),
    Review(ReviewSnapshot),
    Reviews(Vec<ReviewSnapshot>),
    Finding(ReviewFinding),
}

#[derive(Debug)]
pub enum McpAppRequest {
    Inventory(McpServerId),
    RefreshInventory(McpServerId),
    ReadResource(McpResourceReadRequest),
    GetPrompt(McpPromptGetRequest),
    ListCalls(McpCallSummaryListRequest),
    AuthStatus(McpServerId),
    StartOauth(McpOAuthLoginRequest),
    Logout(McpServerId),
}

#[derive(Debug)]
pub enum McpAppResponse {
    Inventory(McpInventorySnapshot),
    Resource(Vec<McpResourceContent>),
    Prompt(McpPromptResult),
    Calls(Vec<McpCallSummaryRecord>),
    Auth(McpAuthStatusRecord),
    Oauth(McpOAuthLoginResponse),
}

#[derive(Debug)]
pub enum SkillsAppRequest {
    List(Option<ProjectId>),
    Create {
        name: String,
    },
    Rename {
        skill_id: SkillId,
        name: String,
    },
    Remove(SkillId),
    SetEnabled {
        skill_id: SkillId,
        enabled: bool,
    },
    Tree(SkillId),
    ReadFile {
        skill_id: SkillId,
        relative_path: String,
    },
    ReadPreviewResource(SkillPreviewResourceRequest),
    WriteFile(SkillFileWriteRequest),
    CreateEntry(SkillEntryCreateRequest),
    RenameEntry(SkillEntryRenameRequest),
    RemoveEntry {
        skill_id: SkillId,
        relative_path: String,
    },
    Validate(SkillId),
}

#[derive(Debug)]
pub enum SkillsAppResponse {
    Skills(Vec<SkillRecord>),
    Skill(SkillRecord),
    Removed(bool),
    Tree(SkillTreeNode),
    File(SkillFileSnapshot),
    PreviewResource(SkillPreviewResource),
}

#[derive(Debug)]
pub enum ScheduleAppRequest {
    Create(ScheduleCreateRequest),
    Get(ScheduleId),
    List,
    Preview {
        schedule: ScheduleSpec,
        count: usize,
    },
    Update(ScheduleUpdateRequest),
    SetEnabled {
        context: MutationContext,
        schedule_id: ScheduleId,
        enabled: bool,
        expected_config_revision: u64,
    },
    Remove {
        context: MutationContext,
        schedule_id: ScheduleId,
    },
    Reauthorize {
        context: MutationContext,
        schedule_id: ScheduleId,
    },
    RevokeGrant {
        context: MutationContext,
        schedule_id: ScheduleId,
    },
    RunNow {
        context: MutationContext,
        schedule_id: ScheduleId,
    },
    IngestEvent(ScheduleEventIngressRequest),
    ListEvents {
        limit: u32,
    },
}

#[derive(Debug)]
pub enum ScheduleAppResponse {
    Snapshot(Option<ScheduleSnapshot>),
    Created(ScheduleSnapshot),
    Schedules(Vec<ScheduleDefinition>),
    Preview(SchedulePreview),
    Schedule(ScheduleDefinition),
    Removed(bool),
    Grant(Option<ScheduleGrantRecord>),
    Task(TaskRunRecord),
    EventReceipt(ScheduleEventReceipt),
    EventReceipts(Vec<ScheduleEventReceipt>),
}

#[derive(Debug)]
pub enum TaskAppRequest {
    Get(TaskRunId),
    List {
        schedule_id: Option<ScheduleId>,
        limit: u32,
    },
    Cancel {
        context: MutationContext,
        task_run_id: TaskRunId,
    },
    Retry {
        context: MutationContext,
        task_run_id: TaskRunId,
    },
    ContinueInteractively {
        context: MutationContext,
        task_run_id: TaskRunId,
    },
}

#[derive(Debug)]
pub enum TaskAppResponse {
    Task(Option<TaskRunRecord>),
    Tasks(Vec<TaskRunRecord>),
    Updated(TaskRunRecord),
    Continuation(Box<TaskInteractiveContinuation>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppServerDomainError {
    pub code: String,
    pub message: String,
}

impl AppServerDomainError {
    #[must_use]
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

pub type DomainFuture<'a> = Pin<
    Box<dyn Future<Output = Result<AppServerDomainResponse, AppServerDomainError>> + Send + 'a>,
>;

pub trait AppServerDomainHandler: Send + Sync {
    fn dispatch<'a>(
        &'a self,
        context: &'a AppServerContext,
        request: AppServerDomainRequest,
    ) -> DomainFuture<'a>;
}
