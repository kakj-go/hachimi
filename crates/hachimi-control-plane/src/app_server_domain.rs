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
    ControlMethod, DiffReadFileRequest, DiffReadFileResponse, DiffScope, FsFileChunk, FsListPage,
    FsListRequest, FsReadChunkRequest, FsSearchId, FsSearchSnapshot, FsSearchStartRequest,
    FsSearchUpdateRequest, FsWatchId, FsWatchRegistration, FsWatchRequest, McpAuthStatusRecord,
    McpCallSummaryListRequest, McpCallSummaryRecord, McpInventorySnapshot, McpOAuthLoginRequest,
    McpOAuthLoginResponse, McpPromptGetRequest, McpPromptResult, McpResourceContent,
    McpResourceReadRequest, McpServerId, MutationContext, ProcessListRequest, ProcessReadRequest,
    ProcessReadSnapshot, ProcessResizeRequest, ProcessSessionRecord, ProcessSpawnRequest,
    ProcessTerminateRequest, ProcessWriteRequest, ProjectId, ReviewFinding,
    ReviewFindingUpdateRequest, ReviewId, ReviewSnapshot, ReviewStartRequest, ReviewStartSnapshot,
    RunDiffSnapshot, ScheduleCreateRequest, ScheduleDefinition, ScheduleGrantRecord, ScheduleId,
    SchedulePreview, ScheduleSnapshot, ScheduleSpec, ScheduleUpdateRequest, SessionId,
    SkillEntryCreateRequest, SkillEntryRenameRequest, SkillFileSnapshot, SkillFileWriteRequest,
    SkillId, SkillPreviewResource, SkillPreviewResourceRequest, SkillRecord, SkillTreeNode,
    TaskInteractiveContinuation, TaskRunId, TaskRunRecord,
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
}

impl AppServerDomainRequest {
    #[must_use]
    pub const fn control_method(&self) -> ControlMethod {
        match self {
            Self::Mcp(_) => ControlMethod::ConnectorsManage,
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
    Review(ReviewAppResponse),
    Mcp(McpAppResponse),
    Skills(SkillsAppResponse),
    Schedule(ScheduleAppResponse),
    Task(Box<TaskAppResponse>),
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
