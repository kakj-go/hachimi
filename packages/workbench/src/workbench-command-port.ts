import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  commands,
  type EventSubscriptionSnapshot,
  type FsChangeEvent,
  type FsSearchSnapshot,
  type SkillChangeEvent,
} from "@hachimi/contracts";

export type WorkbenchCommandPort = Pick<
  typeof commands,
  | "initializeAgentControl"
  | "listMcpServers"
  | "listMcpTools"
  | "listSkills"
  | "subscribeSkills"
  | "unsubscribeSkills"
  | "listWorkbenchProjects"
  | "listWorkbenchSessions"
  | "listProjectGitRefs"
  | "inspectProjectGit"
  | "refreshProjectGit"
  | "createProjectEmptyInitialCommit"
  | "getSandboxStatus"
  | "refreshSandboxStatus"
  | "repairSandbox"
  | "getWorkbenchSession"
  | "resumeAgentSession"
  | "subscribeAgentEvents"
  | "unsubscribeAgentEvents"
  | "addWorkbenchProject"
  | "manageWorkbenchProject"
  | "importWorkbenchAttachment"
  | "startWorkbenchTask"
  | "resolveWorkbenchApproval"
  | "cancelWorkbenchRun"
  | "resolveUserInput"
  | "acceptWorkbenchPlan"
  | "listWorkspaceFiles"
  | "readWorkspaceFileChunk"
  | "writeWorkspaceFile"
  | "getWorkspaceGit"
  | "mutateWorkspaceGit"
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
};
