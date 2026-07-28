import { invoke } from "@tauri-apps/api/core";
import type {
  AppSettings,
  ApprovalDecisionRequest,
  ApprovalRequestRecord,
  AttachmentRecord,
  AvatarCatalogSnapshot,
  AvatarImportCommitRequest,
  AvatarImportInspection,
  AvatarRuntimeAsset,
  BootstrapState,
  CheckoutRecord,
  ControlInitializeRequest,
  ControlInitializeResponse,
  EventSubscriptionId,
  EventSubscriptionRequest,
  EventSubscriptionSnapshot,
  DiffScope,
  DiffReadFileRequest,
  DiffReadFileResponse,
  FsFileChunk,
  FsListPage,
  FsListRequest,
  FsReadChunkRequest,
  FsWriteRequest,
  FsWriteResponse,
  FsSearchId,
  FsSearchSnapshot,
  FsSearchStartRequest,
  FsSearchUpdateRequest,
  FsWatchId,
  FsWatchRegistration,
  FsWatchRequest,
  FrontendLogEntry,
  GitRefRecord,
  ProjectGitInitialCommitRequest,
  ProjectGitInitialCommitResponse,
  ProjectGitSnapshot,
  SandboxRepairRequest,
  SandboxRuntimeSnapshot,
  GitMutationRequest,
  GitMutationResponse,
  GitWorkspaceRequest,
  GitWorkspaceSnapshot,
  InteractiveRegionsUpdate,
  LlmSettingsInput,
  LlmSettingsView,
  LlmTestResult,
  McpServerHealthRecord,
  McpServerId,
  McpServerDraft,
  McpServerView,
  McpConnectionTestResult,
  McpCallSummaryListRequest,
  McpCallSummaryRecord,
  McpAuthStatusRecord,
  McpOAuthLoginRequest,
  McpOAuthLoginResponse,
  McpInventorySnapshot,
  McpPromptGetRequest,
  McpPromptResult,
  McpResourceContent,
  McpResourceReadRequest,
  McpToolView,
  InteractionMotionBindingUpdateRequest,
  MotionAssetBindingsClearRequest,
  MotionBindingResetRequest,
  MotionCatalogSnapshot,
  MotionEnabledUpdateRequest,
  MotionImportCommitRequest,
  MotionImportInspection,
  MotionMetadataUpdateRequest,
  MotionRuntimeAsset,
  MutationContext,
  PetContextMenuRequest,
  PetTurnRequest,
  PlanAcceptanceRequest,
  ProjectId,
  ProjectRecord,
  ProcessListRequest,
  ProcessReadRequest,
  ProcessReadSnapshot,
  ProcessResizeRequest,
  ProcessSessionRecord,
  ProcessSpawnRequest,
  ProcessTerminateRequest,
  ProcessWriteRequest,
  ReviewFinding,
  ReviewFindingUpdateRequest,
  ReviewId,
  ReviewSnapshot,
  ReviewStartRequest,
  ReviewStartSnapshot,
  RunRecord,
  RunControlRequest,
  RunSteerRecord,
  RunDiffSnapshot,
  ScheduleCreateRequest,
  ScheduleDefinition,
  ScheduleGrantRecord,
  ScheduleId,
  SchedulePreview,
  ScheduleSnapshot,
  ScheduleSpec,
  ScheduleUpdateRequest,
  SessionForkRequest,
  SessionMetadataUpdateRequest,
  SessionPage,
  SessionRecord,
  SessionResumeRequest,
  SessionResumeSnapshot,
  SessionSearchRequest,
  SkillEntryCreateRequest,
  SkillEntryRenameRequest,
  SkillFileSnapshot,
  SkillFileWriteRequest,
  SkillId,
  SkillPreviewResource,
  SkillPreviewResourceRequest,
  SkillRecord,
  SkillSubscriptionId,
  SkillTreeNode,
  SpeechRecognitionRuntimeState,
  SpeechRecognitionSettingsInput,
  ThemeScheme,
  TaskInteractiveContinuation,
  TaskRunId,
  TaskRunRecord,
  VoiceCatalogSnapshot,
  VoiceImportCommitRequest,
  VoiceModelInspection,
  VoiceRuntimeState,
  VoiceSettingsInput,
  UserInputRequestRecord,
  UserInputResolution,
  WorkbenchRoute,
  WorkbenchPlanAcceptanceSnapshot,
  WorkbenchSessionSnapshot,
  WorkbenchTaskSnapshot,
  WorkbenchTaskStartRequest,
} from "./generated";

export interface CommandFailure {
  code: string;
  message: string;
}

export const commands = {
  getBootstrapState: () => invoke<BootstrapState>("get_bootstrap_state"),
  frontendReady: () => invoke<void>("frontend_ready"),
  writeFrontendLog: (entry: FrontendLogEntry) => invoke<void>("write_frontend_log", { entry }),
  getSettings: () => invoke<AppSettings>("get_settings"),
  updateSettings: (settings: AppSettings) => invoke<AppSettings>("update_settings", { settings }),
  resetLocalData: () => invoke<void>("reset_local_data"),
  importThemeProfile: (scheme: ThemeScheme) =>
    invoke<AppSettings | null>("import_theme_profile", { scheme }),
  copyThemeProfile: (profileId: string) => invoke<void>("copy_theme_profile", { profileId }),
  resetThemeProfile: (profileId: string) =>
    invoke<AppSettings>("reset_theme_profile", { profileId }),
  deleteThemeProfile: (profileId: string) =>
    invoke<AppSettings>("delete_theme_profile", { profileId }),
  setInteractiveRegions: (update: InteractiveRegionsUpdate) =>
    invoke<void>("set_interactive_regions", { update }),
  setAlwaysOnTop: (enabled: boolean) => invoke<AppSettings>("set_always_on_top", { enabled }),
  startPetDragging: () => invoke<void>("start_pet_dragging"),
  hidePetWindow: () => invoke<void>("hide_pet_window"),
  showPetContextMenu: (request: PetContextMenuRequest) =>
    invoke<void>("show_pet_context_menu", { request }),
  openWorkbench: (route: WorkbenchRoute) => invoke<void>("open_workbench", { route }),
  hideWorkbench: () => invoke<void>("hide_workbench"),
  minimizeWorkbench: () => invoke<void>("minimize_workbench"),
  toggleMaximizeWorkbench: () => invoke<void>("toggle_maximize_workbench"),
  startWorkbenchDragging: () => invoke<void>("start_workbench_dragging"),
  startWorkbenchResize: (direction: string) =>
    invoke<void>("start_workbench_resize", { direction }),
  listMcpServers: () => invoke<McpServerView[]>("list_mcp_servers"),
  getMcpEchoServerUrl: () => invoke<string>("get_mcp_echo_server_url"),
  getMcpServer: (serverId: McpServerId) => invoke<McpServerView>("get_mcp_server", { serverId }),
  testMcpServer: (request: McpServerDraft) =>
    invoke<McpConnectionTestResult>("test_mcp_server", { request }),
  upsertMcpServer: (request: McpServerDraft) =>
    invoke<McpServerView>("upsert_mcp_server", { request }),
  setMcpServerEnabled: (serverId: McpServerId, enabled: boolean) =>
    invoke<McpServerView>("set_mcp_server_enabled", { serverId, enabled }),
  refreshMcpServer: (serverId: McpServerId) =>
    invoke<McpServerHealthRecord>("refresh_mcp_server", { serverId }),
  removeMcpServer: (serverId: McpServerId) => invoke<boolean>("remove_mcp_server", { serverId }),
  listMcpTools: (serverId: McpServerId) => invoke<McpToolView[]>("list_mcp_tools", { serverId }),
  discoverMcpTools: (serverId: McpServerId) =>
    invoke<McpConnectionTestResult>("discover_mcp_tools", { serverId }),
  setMcpToolEnabled: (serverId: McpServerId, toolName: string, enabled: boolean) =>
    invoke<McpToolView>("set_mcp_tool_enabled", { serverId, toolName, enabled }),
  getMcpInventory: (serverId: McpServerId) =>
    invoke<McpInventorySnapshot>("get_mcp_inventory", { serverId }),
  refreshMcpInventory: (serverId: McpServerId) =>
    invoke<McpInventorySnapshot>("refresh_mcp_inventory", { serverId }),
  readMcpResource: (request: McpResourceReadRequest) =>
    invoke<McpResourceContent[]>("read_mcp_resource", { request }),
  getMcpPrompt: (request: McpPromptGetRequest) =>
    invoke<McpPromptResult>("get_mcp_prompt", { request }),
  listMcpCallSummaries: (request: McpCallSummaryListRequest) =>
    invoke<McpCallSummaryRecord[]>("list_mcp_call_summaries", { request }),
  getMcpAuthStatus: (serverId: McpServerId) =>
    invoke<McpAuthStatusRecord>("get_mcp_auth_status", { serverId }),
  startMcpOAuthLogin: (request: McpOAuthLoginRequest) =>
    invoke<McpOAuthLoginResponse>("start_mcp_oauth_login", { request }),
  logoutMcpOAuth: (serverId: McpServerId) =>
    invoke<McpAuthStatusRecord>("logout_mcp_oauth", { serverId }),
  listSkills: (projectId?: string) =>
    invoke<SkillRecord[]>("list_skills", { projectId: projectId ?? null }),
  createSkill: (name: string) => invoke<SkillRecord>("create_skill", { name }),
  importSkillArchive: () => invoke<SkillRecord | null>("import_skill_archive"),
  importSkillDroppedFiles: (dropToken: string, skillId: SkillId, parentPath: string) =>
    invoke<SkillTreeNode>("import_skill_dropped_files", { dropToken, skillId, parentPath }),
  renameSkill: (skillId: SkillId, name: string) =>
    invoke<SkillRecord>("rename_skill", { skillId, name }),
  removeSkill: (skillId: SkillId) => invoke<boolean>("remove_skill", { skillId }),
  setSkillEnabled: (skillId: SkillId, enabled: boolean) =>
    invoke<SkillRecord>("set_skill_enabled", { skillId, enabled }),
  getSkillTree: (skillId: SkillId) => invoke<SkillTreeNode>("get_skill_tree", { skillId }),
  readSkillFile: (skillId: SkillId, relativePath: string) =>
    invoke<SkillFileSnapshot>("read_skill_file", { skillId, relativePath }),
  readSkillPreviewResource: (request: SkillPreviewResourceRequest) =>
    invoke<SkillPreviewResource>("read_skill_preview_resource", { request }),
  writeSkillFile: (request: SkillFileWriteRequest) =>
    invoke<SkillFileSnapshot>("write_skill_file", { request }),
  createSkillEntry: (request: SkillEntryCreateRequest) =>
    invoke<SkillTreeNode>("create_skill_entry", { request }),
  renameSkillEntry: (request: SkillEntryRenameRequest) =>
    invoke<SkillTreeNode>("rename_skill_entry", { request }),
  removeSkillEntry: (skillId: SkillId, relativePath: string) =>
    invoke<SkillTreeNode>("remove_skill_entry", { skillId, relativePath }),
  validateSkill: (skillId: SkillId) => invoke<SkillRecord>("validate_skill", { skillId }),
  subscribeSkills: () => invoke<SkillSubscriptionId>("subscribe_skills"),
  unsubscribeSkills: (subscriptionId: SkillSubscriptionId) =>
    invoke<boolean>("unsubscribe_skills", { subscriptionId }),
  listWorkbenchProjects: () => invoke<ProjectRecord[]>("list_workbench_projects"),
  addWorkbenchProject: () => invoke<ProjectRecord | null>("add_workbench_project"),
  manageWorkbenchProject: (
    projectId: ProjectId,
    action: "open" | "rename" | "create_permanent_worktree",
    value: string | null = null,
  ) => invoke<ProjectRecord>("manage_workbench_project", { projectId, action, value }),
  importWorkbenchAttachment: () => invoke<AttachmentRecord | null>("import_workbench_attachment"),
  listWorkbenchSessions: (projectId: ProjectId | null = null) =>
    invoke<SessionRecord[]>("list_workbench_sessions", { projectId }),
  getWorkbenchSession: (sessionId: string) =>
    invoke<WorkbenchSessionSnapshot>("get_workbench_session", { sessionId }),
  resolveWorkbenchApproval: (request: ApprovalDecisionRequest) =>
    invoke<ApprovalRequestRecord>("resolve_workbench_approval", { request }),
  acceptWorkbenchPlan: (request: PlanAcceptanceRequest) =>
    invoke<WorkbenchPlanAcceptanceSnapshot>("accept_workbench_plan", { request }),
  listProjectGitRefs: (projectId: ProjectId) =>
    invoke<GitRefRecord[]>("list_project_git_refs", { projectId }),
  inspectProjectGit: (projectId: ProjectId) =>
    invoke<ProjectGitSnapshot>("inspect_project_git", { projectId }),
  refreshProjectGit: (projectId: ProjectId) =>
    invoke<ProjectGitSnapshot>("refresh_project_git", { projectId }),
  createProjectEmptyInitialCommit: (request: ProjectGitInitialCommitRequest) =>
    invoke<ProjectGitInitialCommitResponse>("create_project_empty_initial_commit", { request }),
  getSandboxStatus: () => invoke<SandboxRuntimeSnapshot>("get_sandbox_status"),
  refreshSandboxStatus: () => invoke<SandboxRuntimeSnapshot>("refresh_sandbox_status"),
  repairSandbox: (request: SandboxRepairRequest) =>
    invoke<SandboxRuntimeSnapshot>("repair_sandbox", { request }),
  pinWorkbenchCheckout: (checkoutId: string, pinned: boolean) =>
    invoke<CheckoutRecord>("pin_workbench_checkout", { checkoutId, pinned }),
  cleanupWorkbenchCheckout: (checkoutId: string) =>
    invoke<CheckoutRecord>("cleanup_workbench_checkout", { checkoutId }),
  startWorkbenchTask: (request: WorkbenchTaskStartRequest) =>
    invoke<WorkbenchTaskSnapshot>("start_workbench_task", { request }),
  cancelWorkbenchRun: (runId: string, expectedGeneration: number) =>
    invoke<RunRecord>("cancel_workbench_run", { runId, expectedGeneration }),
  listWorkspaceFiles: (request: FsListRequest) =>
    invoke<FsListPage>("list_workspace_files", { request }),
  readWorkspaceFileChunk: (request: FsReadChunkRequest) =>
    invoke<FsFileChunk>("read_workspace_file_chunk", { request }),
  writeWorkspaceFile: (request: FsWriteRequest) =>
    invoke<FsWriteResponse>("write_workspace_file", { request }),
  getWorkspaceGit: (request: GitWorkspaceRequest) =>
    invoke<GitWorkspaceSnapshot>("get_workspace_git", { request }),
  mutateWorkspaceGit: (request: GitMutationRequest) =>
    invoke<GitMutationResponse>("mutate_workspace_git", { request }),
  watchWorkspaceFiles: (request: FsWatchRequest) =>
    invoke<FsWatchRegistration>("watch_workspace_files", { request }),
  unwatchWorkspaceFiles: (watchId: FsWatchId) =>
    invoke<boolean>("unwatch_workspace_files", { watchId }),
  startWorkspaceFileSearch: (request: FsSearchStartRequest) =>
    invoke<FsSearchSnapshot>("start_workspace_file_search", { request }),
  updateWorkspaceFileSearch: (request: FsSearchUpdateRequest) =>
    invoke<FsSearchSnapshot>("update_workspace_file_search", { request }),
  cancelWorkspaceFileSearch: (searchId: FsSearchId) =>
    invoke<boolean>("cancel_workspace_file_search", { searchId }),
  getWorkspaceDiff: (scope: DiffScope) => invoke<RunDiffSnapshot>("get_workspace_diff", { scope }),
  readWorkspaceDiffFile: (request: DiffReadFileRequest) =>
    invoke<DiffReadFileResponse>("read_workspace_diff_file", { request }),
  spawnProcess: (request: ProcessSpawnRequest) =>
    invoke<ProcessSessionRecord>("spawn_process", { request }),
  writeProcessStdin: (request: ProcessWriteRequest) =>
    invoke<void>("write_process_stdin", { request }),
  resizeProcess: (request: ProcessResizeRequest) => invoke<void>("resize_process", { request }),
  terminateProcess: (request: ProcessTerminateRequest) =>
    invoke<ProcessSessionRecord>("terminate_process", { request }),
  readProcess: (request: ProcessReadRequest) =>
    invoke<ProcessReadSnapshot>("read_process", { request }),
  listProcesses: (
    request: ProcessListRequest = { sessionId: null, runId: null, includeTerminal: false },
  ) => invoke<ProcessSessionRecord[]>("list_processes", { request }),
  createSchedule: (request: ScheduleCreateRequest) =>
    invoke<ScheduleSnapshot>("create_schedule", { request }),
  getSchedule: (scheduleId: ScheduleId) =>
    invoke<ScheduleSnapshot | null>("get_schedule", { scheduleId }),
  listSchedules: () => invoke<ScheduleDefinition[]>("list_schedules"),
  previewSchedule: (schedule: ScheduleSpec, count = 5) =>
    invoke<SchedulePreview>("preview_schedule", { schedule, count }),
  updateSchedule: (request: ScheduleUpdateRequest) =>
    invoke<ScheduleDefinition>("update_schedule", { request }),
  setScheduleEnabled: (
    context: MutationContext,
    scheduleId: ScheduleId,
    enabled: boolean,
    expectedConfigRevision: number,
  ) =>
    invoke<ScheduleDefinition>("set_schedule_enabled", {
      context,
      scheduleId,
      enabled,
      expectedConfigRevision,
    }),
  removeSchedule: (context: MutationContext, scheduleId: ScheduleId) =>
    invoke<boolean>("remove_schedule", { context, scheduleId }),
  reauthorizeSchedule: (context: MutationContext, scheduleId: ScheduleId) =>
    invoke<ScheduleGrantRecord>("reauthorize_schedule", { context, scheduleId }),
  revokeScheduleGrant: (context: MutationContext, scheduleId: ScheduleId) =>
    invoke<ScheduleGrantRecord | null>("revoke_schedule_grant", { context, scheduleId }),
  runScheduleNow: (context: MutationContext, scheduleId: ScheduleId) =>
    invoke<TaskRunRecord>("run_schedule_now", { context, scheduleId }),
  getTaskRun: (taskRunId: TaskRunId) => invoke<TaskRunRecord | null>("get_task_run", { taskRunId }),
  listTaskRuns: (scheduleId: ScheduleId | null = null, limit = 100) =>
    invoke<TaskRunRecord[]>("list_task_runs", { scheduleId, limit }),
  cancelTaskRun: (context: MutationContext, taskRunId: TaskRunId) =>
    invoke<TaskRunRecord>("cancel_task_run", { context, taskRunId }),
  retryTaskRun: (context: MutationContext, taskRunId: TaskRunId) =>
    invoke<TaskRunRecord>("retry_task_run", { context, taskRunId }),
  continueTaskInteractively: (context: MutationContext, taskRunId: TaskRunId) =>
    invoke<TaskInteractiveContinuation>("continue_task_interactively", { context, taskRunId }),
  startReview: (request: ReviewStartRequest) =>
    invoke<ReviewStartSnapshot>("start_review", { request }),
  getReview: (reviewId: ReviewId) => invoke<ReviewSnapshot>("get_review", { reviewId }),
  listReviews: (sessionId: string) => invoke<ReviewSnapshot[]>("list_reviews", { sessionId }),
  updateReviewFinding: (request: ReviewFindingUpdateRequest) =>
    invoke<ReviewFinding>("update_review_finding", { request }),
  initializeAgentControl: (request: ControlInitializeRequest) =>
    invoke<ControlInitializeResponse>("initialize_agent_control", { request }),
  searchAgentSessions: (request: SessionSearchRequest) =>
    invoke<SessionPage>("search_agent_sessions", { request }),
  resumeAgentSession: (request: SessionResumeRequest) =>
    invoke<SessionResumeSnapshot>("resume_agent_session", { request }),
  forkAgentSession: (request: SessionForkRequest) =>
    invoke<SessionRecord>("fork_agent_session", { request }),
  updateAgentSessionMetadata: (request: SessionMetadataUpdateRequest) =>
    invoke<SessionRecord>("update_agent_session_metadata", { request }),
  steerAgentRun: (request: RunControlRequest) =>
    invoke<RunSteerRecord>("steer_agent_run", { request }),
  interruptAgentRun: (request: RunControlRequest) =>
    invoke<RunRecord>("interrupt_agent_run", { request }),
  subscribeAgentEvents: (request: EventSubscriptionRequest) =>
    invoke<EventSubscriptionSnapshot>("subscribe_agent_events", { request }),
  unsubscribeAgentEvents: (subscriptionId: EventSubscriptionId) =>
    invoke<boolean>("unsubscribe_agent_events", { subscriptionId }),
  listPendingUserInput: (sessionId: string | null = null) =>
    invoke<UserInputRequestRecord[]>("list_pending_user_input", { sessionId }),
  resolveUserInput: (resolution: UserInputResolution) =>
    invoke<UserInputRequestRecord>("resolve_user_input", { resolution }),
  cancelUserInput: (request: RunControlRequest) =>
    invoke<RunRecord>("cancel_user_input", { request }),
  getLlmSettings: () => invoke<LlmSettingsView>("get_llm_settings"),
  saveLlmSettings: (input: LlmSettingsInput) =>
    invoke<LlmSettingsView>("save_llm_settings", { input }),
  saveAndTestLlmSettings: (input: LlmSettingsInput) =>
    invoke<LlmTestResult>("save_and_test_llm_settings", { input }),
  listAvatarModels: () => invoke<AvatarCatalogSnapshot>("list_avatar_models"),
  inspectAvatarModel: () => invoke<AvatarImportInspection | null>("inspect_avatar_model"),
  commitAvatarModelImport: (request: AvatarImportCommitRequest) =>
    invoke<AvatarCatalogSnapshot>("commit_avatar_model_import", { request }),
  cancelAvatarModelImport: (token: string) => invoke<void>("cancel_avatar_model_import", { token }),
  selectAvatarModel: (id: string) =>
    invoke<AvatarCatalogSnapshot>("select_avatar_model", { request: { id } }),
  deleteAvatarModel: (id: string) =>
    invoke<AvatarCatalogSnapshot>("delete_avatar_model", { request: { id } }),
  getCurrentAvatarAsset: () => invoke<AvatarRuntimeAsset | null>("get_current_avatar_asset"),
  getAvatarRuntimeAsset: (id: string) =>
    invoke<AvatarRuntimeAsset | null>("get_avatar_runtime_asset", { request: { id } }),
  listMotionCatalog: () => invoke<MotionCatalogSnapshot>("list_motion_catalog"),
  inspectMotionFile: () => invoke<MotionImportInspection | null>("inspect_motion_file"),
  commitMotionImport: (request: MotionImportCommitRequest) =>
    invoke<MotionCatalogSnapshot>("commit_motion_import", { request }),
  cancelMotionImport: (token: string) => invoke<void>("cancel_motion_import", { token }),
  updateMotionMetadata: (request: MotionMetadataUpdateRequest) =>
    invoke<MotionCatalogSnapshot>("update_motion_metadata", { request }),
  deleteUserMotion: (id: string) =>
    invoke<MotionCatalogSnapshot>("delete_user_motion", { request: { id } }),
  setInteractionMotionBinding: (request: InteractionMotionBindingUpdateRequest) =>
    invoke<MotionCatalogSnapshot>("set_interaction_motion_binding", { request }),
  clearMotionInteractionBindings: (request: MotionAssetBindingsClearRequest) =>
    invoke<MotionCatalogSnapshot>("clear_motion_interaction_bindings", { request }),
  setMotionEnabled: (request: MotionEnabledUpdateRequest) =>
    invoke<MotionCatalogSnapshot>("set_motion_enabled", { request }),
  resetMotionBindings: () => invoke<MotionCatalogSnapshot>("reset_motion_bindings"),
  resetMotionBinding: (request: MotionBindingResetRequest) =>
    invoke<MotionCatalogSnapshot>("reset_motion_binding", { request }),
  getMotionRuntimeAsset: (id: string) =>
    invoke<MotionRuntimeAsset | null>("get_motion_runtime_asset", { request: { id } }),
  startPetTurn: (request: PetTurnRequest) => invoke<void>("start_pet_turn", { request }),
  cancelPetTurn: () => invoke<void>("cancel_pet_turn"),
  getVoiceRuntimeState: () => invoke<VoiceRuntimeState>("get_voice_runtime_state"),
  getSpeechRecognitionState: () =>
    invoke<SpeechRecognitionRuntimeState>("get_speech_recognition_state"),
  updateSpeechRecognitionSettings: (input: SpeechRecognitionSettingsInput) =>
    invoke<SpeechRecognitionRuntimeState>("update_speech_recognition_settings", { input }),
  listVoiceModels: () => invoke<VoiceCatalogSnapshot>("list_voice_models"),
  inspectVoiceModel: () => invoke<VoiceModelInspection | null>("inspect_voice_model"),
  commitVoiceModelImport: (request: VoiceImportCommitRequest) =>
    invoke<VoiceCatalogSnapshot>("commit_voice_model_import", { request }),
  cancelVoiceModelImport: (token: string) => invoke<void>("cancel_voice_model_import", { token }),
  selectVoiceModel: (id: string) =>
    invoke<VoiceCatalogSnapshot>("select_voice_model", { request: { id } }),
  deleteVoiceModel: (id: string) =>
    invoke<VoiceCatalogSnapshot>("delete_voice_model", { request: { id } }),
  updateVoiceSettings: (input: VoiceSettingsInput) =>
    invoke<VoiceRuntimeState>("update_voice_settings", { input }),
  setMuted: (muted: boolean) => invoke<VoiceRuntimeState>("set_muted", { muted }),
  previewDefaultVoice: () => invoke<VoiceRuntimeState>("preview_default_voice"),
  stopSpeech: () => invoke<VoiceRuntimeState>("stop_speech"),
  recognizePetSpeech: () => invoke<string>("recognize_pet_speech"),
  exitApp: () => invoke<void>("exit_app"),
};

export function commandFailure(error: unknown): CommandFailure {
  if (typeof error === "object" && error !== null && "message" in error) {
    const value = error as { code?: unknown; message: unknown };
    return {
      code: typeof value.code === "string" ? value.code : "command_failed",
      message: String(value.message),
    };
  }
  return { code: "command_failed", message: String(error) };
}

let frontendLoggingInstalled = false;

function formatLogArgument(value: unknown): string {
  if (value instanceof Error) return value.stack || `${value.name}: ${value.message}`;
  if (typeof value === "string") return value;
  if (value === null || value === undefined) return String(value);
  if (typeof value === "number" || typeof value === "boolean" || typeof value === "bigint") {
    return String(value);
  }
  return Object.prototype.toString.call(value);
}

export function installFrontendLogging(): void {
  if (frontendLoggingInstalled || typeof window === "undefined") return;
  frontendLoggingInstalled = true;
  const emit = (level: FrontendLogEntry["level"], values: unknown[]) => {
    const message = values.map(formatLogArgument).join(" ").slice(0, 4_096);
    if (message) void commands.writeFrontendLog({ level, message }).catch(() => undefined);
  };
  const originalInfo = console.info.bind(console);
  const originalWarn = console.warn.bind(console);
  const originalError = console.error.bind(console);
  console.info = (...values: unknown[]) => {
    originalInfo(...values);
    emit("info", values);
  };
  console.warn = (...values: unknown[]) => {
    originalWarn(...values);
    emit("warn", values);
  };
  console.error = (...values: unknown[]) => {
    originalError(...values);
    emit("error", values);
  };
  window.addEventListener("error", (event) => {
    emit("error", [event.error instanceof Error ? event.error : event.message]);
  });
  window.addEventListener("unhandledrejection", (event) => {
    emit("error", [event.reason]);
  });
  emit("info", ["frontend logging initialized"]);
}
