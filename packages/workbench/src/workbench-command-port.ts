import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  commands,
  type EventSubscriptionSnapshot,
  type BrowserDownloadSnapshot,
  type BrowserWorkspace,
  type BrowserWorkspaceChanged,
  type EmbeddedBrowserPermissionRequiredEvent,
  type FsChangeEvent,
  type FsSearchSnapshot,
  type SessionRunActivity,
  type SkillChangeEvent,
  type WorkbenchEnvironmentChanged,
} from "@hachimi/contracts";

export type BrowserShortcutRequested = {
  kind: "shortcut_requested";
  tab_id: string;
  shortcut: "focus_address" | "new_tab" | "close_tab" | "reload" | "back" | "forward";
};

export type WorkbenchCommandPort = Pick<
  typeof commands,
  | "initializeAgentControl"
  | "localHostCommand"
  | "listMcpServers"
  | "listMcpTools"
  | "listSkills"
  | "subscribeSkills"
  | "unsubscribeSkills"
  | "listWorkbenchProjects"
  | "getWorkbenchProjectToolContext"
  | "listRunRecoveries"
  | "resolveRunRecovery"
  | "listWorkbenchSessions"
  | "searchAgentSessions"
  | "listProjectGitRefs"
  | "inspectProjectGit"
  | "refreshProjectGit"
  | "createProjectEmptyInitialCommit"
  | "getSandboxStatus"
  | "refreshSandboxStatus"
  | "repairSandbox"
  | "getWorkbenchSession"
  | "getWorkbenchEnvironment"
  | "openBrowserWorkspace"
  | "mutateBrowserWorkspace"
  | "updateBrowserSurfaceLayout"
  | "getBrowserHistory"
  | "getEmbeddedBrowserSettings"
  | "chooseBrowserDownloadDirectory"
  | "updateEmbeddedBrowserSettings"
  | "clearEmbeddedBrowserData"
  | "getBrowserDownloads"
  | "manageBrowserDownload"
  | "listEmbeddedBrowserPermissionRequests"
  | "listEmbeddedBrowserSitePermissions"
  | "resolveEmbeddedBrowserPermission"
  | "revokeEmbeddedBrowserSitePermission"
  | "openSystemBrowser"
  | "handoffWorkbenchSession"
  | "resumeAgentSession"
  | "subscribeAgentEvents"
  | "unsubscribeAgentEvents"
  | "addWorkbenchProject"
  | "manageWorkbenchProject"
  | "importWorkbenchAttachment"
  | "readWorkbenchAttachment"
  | "startWorkbenchTask"
  | "forkAgentSession"
  | "steerAgentRun"
  | "resolveWorkbenchApproval"
  | "cancelWorkbenchRun"
  | "resolveUserInput"
  | "acceptWorkbenchPlan"
  | "reviseWorkbenchPlan"
  | "executeWorkbenchGit"
  | "listWorkspaceFiles"
  | "readWorkspaceFileChunk"
  | "writeWorkspaceFile"
  | "getWorkspaceGit"
  | "mutateWorkspaceGit"
  | "listGitRemotes"
  | "pushGitRemote"
  | "queryForgeChange"
  | "mutateForgeChange"
  | "updateForgeCredential"
  | "watchWorkspaceFiles"
  | "unwatchWorkspaceFiles"
  | "startWorkspaceFileSearch"
  | "updateWorkspaceFileSearch"
  | "cancelWorkspaceFileSearch"
  | "getWorkspaceDiff"
  | "readWorkspaceDiffFile"
  | "spawnProcess"
  | "writeProcessStdin"
  | "resizeProcess"
  | "terminateProcess"
  | "readProcess"
  | "listProcesses"
  | "createSchedule"
  | "getSchedule"
  | "listSchedules"
  | "previewSchedule"
  | "updateSchedule"
  | "setScheduleEnabled"
  | "removeSchedule"
  | "reauthorizeSchedule"
  | "revokeScheduleGrant"
  | "runScheduleNow"
  | "ingestScheduleEvent"
  | "listScheduleEventReceipts"
  | "getTaskRun"
  | "listTaskRuns"
  | "cancelTaskRun"
  | "retryTaskRun"
  | "continueTaskInteractively"
  | "startReview"
  | "getReview"
  | "listReviews"
  | "updateReviewFinding"
  | "updateAgentSessionMetadata"
> & {
  onAgentEvents(handler: (batch: EventSubscriptionSnapshot) => void): Promise<UnlistenFn>;
  onSkillsChange(handler: (events: SkillChangeEvent[]) => void): Promise<UnlistenFn>;
  onWorkspaceChange(handler: (event: FsChangeEvent) => void): Promise<UnlistenFn>;
  onWorkspaceSearch(handler: (snapshot: FsSearchSnapshot) => void): Promise<UnlistenFn>;
  onSessionActivity(handler: (activity: SessionRunActivity) => void): Promise<UnlistenFn>;
  onEnvironmentChange(handler: (event: WorkbenchEnvironmentChanged) => void): Promise<UnlistenFn>;
  onBrowserWorkspaceChange(handler: (event: BrowserWorkspaceChanged) => void): Promise<UnlistenFn>;
  onBrowserTabStateChange(handler: (workspace: BrowserWorkspace) => void): Promise<UnlistenFn>;
  onBrowserDownloadUpdate(
    handler: (download: BrowserDownloadSnapshot) => void,
  ): Promise<UnlistenFn>;
  onBrowserPermissionRequired(
    handler: (event: EmbeddedBrowserPermissionRequiredEvent) => void,
  ): Promise<UnlistenFn>;
  onBrowserShortcutRequested(
    handler: (event: BrowserShortcutRequested) => void,
  ): Promise<UnlistenFn>;
  onBrowserRuntimeCrash(
    handler: (event: { generation: number; message: string }) => void,
  ): Promise<UnlistenFn>;
};

export const desktopWorkbenchCommandPort: WorkbenchCommandPort = {
  ...commands,
  onAgentEvents: async (handler) =>
    listen<EventSubscriptionSnapshot>("agent:events", (event) => handler(event.payload)),
  onSkillsChange: async (handler) =>
    listen<SkillChangeEvent[]>("skills:changed", (event) => handler(event.payload)),
  onWorkspaceChange: async (handler) =>
    listen<FsChangeEvent>("workbench-fs-change", (event) => handler(event.payload)),
  onWorkspaceSearch: async (handler) =>
    listen<FsSearchSnapshot>("workbench-fs-search", (event) => handler(event.payload)),
  onSessionActivity: async (handler) =>
    listen<SessionRunActivity>("workbench:session-activity-changed", (event) =>
      handler(event.payload),
    ),
  onEnvironmentChange: async (handler) =>
    listen<WorkbenchEnvironmentChanged>("workbench:environment-changed", (event) =>
      handler(event.payload),
    ),
  onBrowserWorkspaceChange: async (handler) =>
    listen<BrowserWorkspaceChanged>("browser:workspace-changed", (event) => handler(event.payload)),
  onBrowserTabStateChange: async (handler) =>
    listen<BrowserWorkspace>("browser:tab-state-changed", (event) => handler(event.payload)),
  onBrowserDownloadUpdate: async (handler) =>
    listen<BrowserDownloadSnapshot>("browser:download-updated", (event) => handler(event.payload)),
  onBrowserPermissionRequired: async (handler) =>
    listen<EmbeddedBrowserPermissionRequiredEvent>("browser:permission-required", (event) =>
      handler(event.payload),
    ),
  onBrowserShortcutRequested: async (handler) =>
    listen<BrowserShortcutRequested>("browser:shortcut-requested", (event) =>
      handler(event.payload),
    ),
  onBrowserRuntimeCrash: async (handler) =>
    listen<{ generation: number; message: string }>("browser:runtime-crashed", (event) =>
      handler(event.payload),
    ),
};
