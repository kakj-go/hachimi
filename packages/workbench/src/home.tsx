import {
  CONTROL_PROTOCOL_VERSION,
  commandFailure,
  type PermissionProfile,
  type ApprovalRequestRecord,
  type ApprovalStatus,
  type BehaviorMode,
  type ControlInitializeResponse,
  type HostAccessDecision,
  type HostAccessRequestRecord,
  type ProjectRecord,
  type ProposedPlan,
  type RunRecord,
  type RunRecoveryDecisionAction,
  type RunRecoverySnapshot,
  type SessionPermissionConfig,
  type SessionRecord,
  type SkillRecord,
  type UserInputAnswer,
  type UserInputRequestRecord,
  type UserInputResolutionAction,
  type WorkbenchRoute,
  type WorkbenchSessionSnapshot,
  type WorkbenchSessionListItem,
  type WorkbenchTaskSnapshot,
} from "@hachimi/contracts";
import {
  reconcilePendingUserInputs,
  reduceLiveItemDeltas,
  type LiveItemDelta,
} from "./agent-live-items";
import { useI18n } from "@hachimi/i18n";
import { reduceAgentEventWatermark } from "./agent-event-watermark";
import {
  Button,
  ChevronDown,
  Composer,
  ComposerInput,
  Dialog,
  Send,
  Square,
  TerminalSquare,
  TextField,
} from "@hachimi/ui";
import {
  Show,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  onMount,
  untrack,
} from "solid-js";
import { desktopWorkbenchCommandPort, type WorkbenchCommandPort } from "./workbench-command-port";
import { directUserMutationContext, runMutationContext } from "./mutation-context";
import { createProjectGitController } from "./project-git-controller";
import { SandboxReadinessBanner } from "./sandbox-readiness-banner";
import { ComposerAttachmentList, type ComposerAttachmentPreview } from "./composer-attachments";
import {
  PermissionProfilePopover,
  ComposerContextControls,
  ComposerOptionsPopover,
  PlanModeChip,
  SkillReferenceList,
  type ComposerPopoverId,
} from "./composer-popovers";
import { TaskCenter } from "./task-center";
import { TerminalPanel } from "./terminal";
import { ProjectSidebar, type ProjectMenuAction, type SessionMenuAction } from "./project-sidebar";
import { WorkbenchGate } from "./gates/workbench-gate";
import { WorkbenchResizeHandle } from "./layout/workbench-resize-handle";
import { WorkbenchToolbar } from "./layout/workbench-toolbar";
import {
  readWorkbenchLayout,
  persistWorkbenchLayout,
  inspectorNeedsProjectTools,
  type InspectorResource,
} from "./state/workbench-layout";
import {
  EMPTY_INSPECTOR_TABS,
  closeInspectorTab,
  openInspectorTab,
  selectInspectorTab,
  showInspectorLauncher,
  showInspectorTabs,
} from "./state/inspector-tabs";
import { createProjectToolContext } from "./state/project-tool-context";
import "./composer-attachments.css";
import "./composer-popovers.css";
import "./workbench-v2.css";
import "./inspector/browser-inspector.css";
import "./inspector/workspace-tabs.css";
import { DraftAttachmentInspector } from "./inspector/draft-attachment-inspector";
import { ConnectedEnvironmentSummary } from "./inspector/environment-summary";
import { SessionInspector } from "./inspector/session-inspector";
import { SessionTimeline as CodexSessionTimeline } from "./timeline/session-timeline";
import { createSessionScrollController } from "./timeline/use-session-scroll";
import {
  SELECTED_PROJECT_STORAGE_KEY,
  SELECTED_SESSION_STORAGE_KEY,
  PINNED_PROJECTS_STORAGE_KEY,
  REMOVED_PROJECTS_STORAGE_KEY,
  READ_SESSIONS_STORAGE_KEY,
  readSessionSelection,
  persistSessionSelection,
  readLocalJson,
  persistLocalJson,
  revokeAttachmentPreview,
  sessionProjectId,
  isTerminalRunStatus,
} from "./state/home-utilities";

export function HomePage(props: {
  navigate: (route: WorkbenchRoute) => void;
  motionLabEnabled: boolean;
  runRecoveryEnabled: boolean;
  multiAgentEnabled: boolean;
  gitRemoteMutationsEnabled: boolean;
  workspaceToolsEnabled: boolean;
  schedulerEnabled: boolean;
  commandPort?: WorkbenchCommandPort;
}) {
  const i18n = useI18n();
  const commandPort = untrack(() => props.commandPort ?? desktopWorkbenchCommandPort);
  const [activeView, setActiveView] = createSignal<"agent" | "tasks">("agent");
  const [draft, setDraft] = createSignal("");
  const [projects, setProjects] = createSignal<ProjectRecord[]>([]);
  const [sessions, setSessions] = createSignal<SessionRecord[]>([]);
  const [sessionItems, setSessionItems] = createSignal<WorkbenchSessionListItem[]>([]);
  const [pinnedProjectIds, setPinnedProjectIds] = createSignal<string[]>(
    readLocalJson<string[]>(PINNED_PROJECTS_STORAGE_KEY, []),
  );
  const [removedProjectIds, setRemovedProjectIds] = createSignal<string[]>(
    readLocalJson<string[]>(REMOVED_PROJECTS_STORAGE_KEY, []),
  );
  const [readSessions, setReadSessions] = createSignal<Record<string, string>>(
    readLocalJson<Record<string, string>>(READ_SESSIONS_STORAGE_KEY, {}),
  );
  const initialLayout = readWorkbenchLayout();
  const [summaryPinned, setSummaryPinned] = createSignal(initialLayout.summaryPinned);
  const [bottomPanelOpen, setBottomPanelOpen] = createSignal(initialLayout.bottomPanelOpen);
  const [inspectorVisible, setInspectorVisible] = createSignal(initialLayout.sidebarVisible);
  const [projectSidebarWidth, setProjectSidebarWidth] = createSignal(
    initialLayout.projectSidebarWidth,
  );
  const [inspectorWidth, setInspectorWidth] = createSignal(initialLayout.inspectorWidth);
  const [bottomPanelHeight, setBottomPanelHeight] = createSignal(initialLayout.bottomPanelHeight);
  const safeProjectSidebarWidth = () =>
    Math.min(480, Math.max(220, Math.round(projectSidebarWidth())));
  const inspectorWidthMaximum = () =>
    Math.max(300, Math.min(820, window.innerWidth - safeProjectSidebarWidth() - 420));
  const safeInspectorWidth = () =>
    Math.min(inspectorWidthMaximum(), Math.max(300, Math.round(inspectorWidth())));
  const safeBottomPanelHeight = () =>
    Math.min(
      Math.max(140, Math.min(520, window.innerHeight - 260)),
      Math.max(140, Math.round(bottomPanelHeight())),
    );
  const [inspectorTabs, setInspectorTabs] = createSignal(EMPTY_INSPECTOR_TABS);
  const inspectorResource = () => inspectorTabs().resource;
  const openInspector = (resource: InspectorResource) => {
    setInspectorTabs((current) =>
      openInspectorTab(
        current,
        resource,
        () => globalThis.crypto?.randomUUID?.() ?? `inspector-${Date.now()}-${Math.random()}`,
      ),
    );
    setInspectorVisible(true);
    if (!sessionSnapshot() && inspectorNeedsProjectTools(resource))
      void ensureProjectToolSnapshot();
  };
  const selectInspector = (tabId: string) => {
    setInspectorTabs((current) => selectInspectorTab(current, tabId));
  };
  const closeInspector = (tabId: string) => {
    setInspectorTabs((current) => {
      const next = closeInspectorTab(current, tabId);
      if (next.tabs.length === 0) setInspectorVisible(false);
      return next;
    });
  };
  const openInspectorLauncher = () => {
    setInspectorTabs((current) => showInspectorLauncher(current));
    setInspectorVisible(true);
  };
  const [dismissedPlanId, setDismissedPlanId] = createSignal<string>();
  const [selectedProjectId, setSelectedProjectId] = createSignal<string | undefined>(
    readSessionSelection(SELECTED_PROJECT_STORAGE_KEY),
  );
  const [selectedSessionId, setSelectedSessionId] = createSignal<string | undefined>(
    readSessionSelection(SELECTED_SESSION_STORAGE_KEY),
  );
  const [sessionProjectionRevision, setSessionProjectionRevision] = createSignal(0);
  const [behaviorMode, setBehaviorMode] = createSignal<BehaviorMode>("default");
  const [permissionProfile, setPermissionProfile] = createSignal<PermissionProfile>("writable");
  const [sessionPermissionConfig, setSessionPermissionConfig] =
    createSignal<SessionPermissionConfig>();
  const [activePopover, setActivePopover] = createSignal<ComposerPopoverId>();
  const [skills, setSkills] = createSignal<SkillRecord[]>([]);
  const [skillsLoading, setSkillsLoading] = createSignal(true);
  const [skillsError, setSkillsError] = createSignal<string>();
  const [selectedSkillIds, setSelectedSkillIds] = createSignal<string[]>([]);
  const [attachments, setAttachments] = createSignal<ComposerAttachmentPreview[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [addingProject, setAddingProject] = createSignal(false);
  const [submitting, setSubmitting] = createSignal(false);
  const [failure, setFailure] = createSignal<string>();
  const [taskSnapshot, setTaskSnapshot] = createSignal<WorkbenchTaskSnapshot>();
  const [sessionSnapshot, setSessionSnapshot] = createSignal<WorkbenchSessionSnapshot>();
  const [liveItemDeltas, setLiveItemDeltas] = createSignal<Record<string, LiveItemDelta>>({});
  const [sessionViewport, setSessionViewport] = createSignal<HTMLElement>();
  const [sessionTimelineContent, setSessionTimelineContent] = createSignal<HTMLElement>();
  const sessionScroll = createSessionScrollController({
    viewport: sessionViewport,
    content: sessionTimelineContent,
    sessionKey: selectedSessionId,
    revision: () => {
      const snapshot = sessionSnapshot();
      const liveLength = Object.values(liveItemDeltas()).reduce(
        (total, delta) => total + delta.text.length,
        0,
      );
      return `${snapshot?.transcript.at(-1)?.sequence ?? 0}:${
        snapshot?.runSummaries.at(-1)?.completedAtMs ?? 0
      }:${liveLength}:${sessionProjectionRevision()}`;
    },
  });
  const [pendingUserInputs, setPendingUserInputs] = createSignal<UserInputRequestRecord[]>([]);
  const [runRecoveries, setRunRecoveries] = createSignal<RunRecoverySnapshot[]>([]);
  const [agentControl, setAgentControl] = createSignal<ControlInitializeResponse>();
  const [resolvingApprovalId, setResolvingApprovalId] = createSignal<string>();
  const [resolvingHostAccessId, setResolvingHostAccessId] = createSignal<string>();
  const [resolvingUserInputId, setResolvingUserInputId] = createSignal<string>();
  const [acceptingPlanId, setAcceptingPlanId] = createSignal<string>();
  const [revisingPlanId, setRevisingPlanId] = createSignal<string>();
  const [cancellingRun, setCancellingRun] = createSignal(false);
  const [resolvingRecoveryId, setResolvingRecoveryId] = createSignal<string>();
  const [projectActionBusy, setProjectActionBusy] = createSignal(false);
  const [sessionActionBusy, setSessionActionBusy] = createSignal(false);
  const [renameTarget, setRenameTarget] = createSignal<ProjectRecord>();
  const [renameDraft, setRenameDraft] = createSignal("");
  const [renameSessionTarget, setRenameSessionTarget] = createSignal<SessionRecord>();
  const [renameSessionDraft, setRenameSessionDraft] = createSignal("");
  const [removeTarget, setRemoveTarget] = createSignal<ProjectRecord>();
  const projectTools = createProjectToolContext(commandPort, setFailure);
  const toolSnapshot = createMemo(() => projectTools.snapshot(selectedProjectId()));
  const inspectorSnapshot = createMemo(
    () =>
      sessionSnapshot() ??
      (inspectorNeedsProjectTools(inspectorResource()) ? toolSnapshot() : undefined),
  );
  const visibleProjects = createMemo(() => {
    const removed = new Set(removedProjectIds());
    const pinned = new Set(pinnedProjectIds());
    return projects()
      .filter((project) => !removed.has(project.id))
      .toSorted(
        (left, right) =>
          Number(pinned.has(right.id)) - Number(pinned.has(left.id)) ||
          right.updatedAtMs - left.updatedAtMs,
      );
  });
  const selectedProject = createMemo(() =>
    visibleProjects().find((project) => project.id === selectedProjectId()),
  );
  const projectGit = createProjectGitController({
    commandPort,
    selectedProject,
    onFailure: setFailure,
    onProjectReconciled: (projectId, gitRoot) =>
      setProjects((current) => {
        const target = current.find((project) => project.id === projectId);
        if (!target || target.gitRoot === gitRoot) return current;
        return current.map((project) =>
          project.id === projectId ? { ...project, gitRoot } : project,
        );
      }),
  });
  const unreadSessionIds = createMemo(
    () =>
      new Set(
        sessionItems()
          .filter(
            (item) =>
              !item.session.archived &&
              item.session.id !== selectedSessionId() &&
              item.latestTerminalRun &&
              readSessions()[item.session.id] !== item.latestTerminalRun.id,
          )
          .map((item) => item.session.id),
      ),
  );
  const runningSessionIds = createMemo(
    () =>
      new Set(
        sessionItems()
          .filter((item) => item.latestRun && !isTerminalRunStatus(item.latestRun.status))
          .map((item) => item.session.id),
      ),
  );
  const failedSessionIds = createMemo(
    () =>
      new Set(
        sessionItems()
          .filter((item) => item.latestTerminalRun && item.latestTerminalRun.status !== "succeeded")
          .map((item) => item.session.id),
      ),
  );
  const selectedSkills = createMemo(() =>
    selectedSkillIds()
      .map((skillId) => skills().find((skill) => skill.id === skillId))
      .filter((skill): skill is SkillRecord => Boolean(skill)),
  );
  const latestRun = createMemo(() => {
    const runs = sessionSnapshot()?.runs;
    return runs?.[runs.length - 1];
  });
  const activeRun = createMemo(() =>
    sessionSnapshot()
      ?.runs.toReversed()
      .find((run) => !isTerminalRunStatus(run.status)),
  );
  const activeApproval = createMemo(() => sessionSnapshot()?.pendingApprovals[0]);
  const activeHostAccess = createMemo(() =>
    sessionSnapshot()?.hostAccessRequests.find((request) => request.status === "pending"),
  );
  const activePlan = createMemo(() =>
    sessionSnapshot()
      ?.proposedPlans.toReversed()
      .find((plan) => plan.status === "proposed" && plan.id !== dismissedPlanId()),
  );
  const hasComposerGate = createMemo(() =>
    Boolean(pendingUserInputs()[0] || activeHostAccess() || activeApproval() || activePlan()),
  );
  const activeGateKind = createMemo<"approval" | "host_access" | "plan" | "user_input" | undefined>(
    () => {
      if (pendingUserInputs()[0]) return "user_input";
      if (activeHostAccess()) return "host_access";
      if (activeApproval()) return "approval";
      if (activePlan()) return "plan";
      return undefined;
    },
  );
  const selectedRunRecoveries = createMemo(() =>
    runRecoveries().filter((value) => value.recovery.sessionId === selectedSessionId()),
  );
  let composerInput: HTMLTextAreaElement | undefined;
  let stopSkillChanges: (() => void) | undefined;
  let stopSessionActivity: (() => void) | undefined;
  let skillSubscriptionId: string | undefined;

  createEffect(() => persistSessionSelection(SELECTED_PROJECT_STORAGE_KEY, selectedProjectId()));
  createEffect(() => projectTools.clearUnless(selectedProjectId()));
  createEffect(() => {
    const projectId = selectedProjectId();
    if (bottomPanelOpen() && projectId) void projectTools.ensure(projectId);
  });
  createEffect(() => persistSessionSelection(SELECTED_SESSION_STORAGE_KEY, selectedSessionId()));
  createEffect(() => {
    const sessionId = selectedSessionId();
    if (!sessionId) {
      setSessionPermissionConfig(undefined);
      return;
    }
    let disposed = false;
    void commandPort
      .getSessionPermissionConfig({ sessionId, entryProfile: "workbench" })
      .then((config) => {
        if (!disposed && untrack(selectedSessionId) === sessionId) {
          setSessionPermissionConfig(config);
          setPermissionProfile(config.policy.level);
        }
      })
      .catch((error) => {
        if (!disposed) setFailure(commandFailure(error).message);
      });
    onCleanup(() => {
      disposed = true;
    });
  });
  createEffect(() => persistLocalJson(PINNED_PROJECTS_STORAGE_KEY, pinnedProjectIds()));
  createEffect(() => persistLocalJson(REMOVED_PROJECTS_STORAGE_KEY, removedProjectIds()));
  createEffect(() => persistLocalJson(READ_SESSIONS_STORAGE_KEY, readSessions()));
  createEffect(() =>
    persistWorkbenchLayout({
      summaryPinned: summaryPinned(),
      bottomPanelOpen: bottomPanelOpen(),
      sidebarVisible: inspectorVisible(),
      projectSidebarWidth: safeProjectSidebarWidth(),
      inspectorWidth: safeInspectorWidth(),
      bottomPanelHeight: safeBottomPanelHeight(),
    }),
  );

  async function refreshWorkbench(preferredProjectId?: string) {
    setLoading(true);
    setFailure(undefined);
    try {
      const [nextProjects, nextSessions] = await Promise.all([
        commandPort.listWorkbenchProjects(),
        commandPort.listWorkbenchSessions(),
      ]);
      setProjects(nextProjects);
      setSessionItems(nextSessions);
      const nextSessionRecords = nextSessions.map((item) => item.session);
      setSessions(nextSessionRecords);
      const removed = new Set(removedProjectIds());
      const availableProjects = nextProjects.filter((project) => !removed.has(project.id));
      const currentProjectId = selectedProjectId();
      const nextProjectId =
        preferredProjectId && availableProjects.some((project) => project.id === preferredProjectId)
          ? preferredProjectId
          : availableProjects.some((project) => project.id === currentProjectId)
            ? currentProjectId
            : availableProjects[0]?.id;
      setSelectedProjectId(nextProjectId);
      if (!nextSessionRecords.some((session) => session.id === selectedSessionId())) {
        setSelectedSessionId(undefined);
      }
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setLoading(false);
    }
  }

  async function refreshSessionActivity() {
    const nextItems = await commandPort.listWorkbenchSessions();
    setSessionItems(nextItems);
    setSessions(nextItems.map((item) => item.session));
    const selected = selectedSessionId();
    const terminal = nextItems.find((item) => item.session.id === selected)?.latestTerminalRun;
    if (selected && terminal) {
      setReadSessions((current) => ({ ...current, [selected]: terminal.id }));
    }
    return nextItems;
  }

  async function refreshSkills() {
    setSkillsLoading(true);
    setSkillsError(undefined);
    try {
      const nextSkills = await commandPort.listSkills(selectedProjectId());
      setSkills(nextSkills);
      const availableIds = new Set(nextSkills.map((skill) => skill.id));
      setSelectedSkillIds((current) => current.filter((skillId) => availableIds.has(skillId)));
    } catch (error) {
      setSkillsError(commandFailure(error).message);
    } finally {
      setSkillsLoading(false);
    }
  }

  async function clearSessionExtraAuthorizations() {
    const sessionId = selectedSessionId();
    if (!sessionId) return;
    try {
      setSessionPermissionConfig(await commandPort.clearSessionExtraAuthorizations(sessionId));
    } catch (error) {
      setFailure(commandFailure(error).message);
    }
  }

  async function refreshRunRecoveries() {
    if (!props.runRecoveryEnabled) {
      setRunRecoveries([]);
      return;
    }
    try {
      setRunRecoveries(await commandPort.listRunRecoveries());
    } catch (error) {
      setFailure(commandFailure(error).message);
    }
  }

  onMount(() => {
    let disposed = false;
    void commandPort
      .initializeAgentControl({
        clientVersion: "hachimi-desktop/0.3.0-alpha.8",
        protocolVersion: CONTROL_PROTOCOL_VERSION,
        supportedFeatures: [
          "session_lifecycle_v2",
          "typed_items",
          "user_input",
          "event_resume",
          "workbench",
          "workspace_tools",
        ],
        experimentalFeatures: [],
      })
      .then(setAgentControl)
      .catch((error) => setFailure(commandFailure(error).message));
    void refreshWorkbench();
    void refreshSkills();
    void refreshRunRecoveries();
    void commandPort
      // eslint-disable-next-line solid/reactivity -- native session events run after mount.
      .onSessionActivity(() => void refreshSessionActivity())
      .then((stop) => {
        if (disposed) stop();
        else stopSessionActivity = stop;
      })
      .catch((error) => setFailure(commandFailure(error).message));
    void (async () => {
      // eslint-disable-next-line solid/reactivity -- the native Skill event invokes this after mount.
      const stop = await commandPort.onSkillsChange(() => void refreshSkills());
      if (disposed) {
        stop();
        return;
      }
      stopSkillChanges = stop;
      const subscriptionId = await commandPort.subscribeSkills();
      if (disposed) {
        await commandPort.unsubscribeSkills(subscriptionId).catch(() => false);
        return;
      }
      skillSubscriptionId = subscriptionId;
    })().catch((error) => setSkillsError(commandFailure(error).message));
    const handleNewTask = () => newTask();
    const handleAddProject = () => void addProject();
    window.addEventListener("hachimi:new-task", handleNewTask);
    window.addEventListener("hachimi:add-project", handleAddProject);
    onCleanup(() => {
      disposed = true;
      stopSkillChanges?.();
      stopSessionActivity?.();
      if (skillSubscriptionId)
        void commandPort.unsubscribeSkills(skillSubscriptionId).catch(() => false);
      window.removeEventListener("hachimi:new-task", handleNewTask);
      window.removeEventListener("hachimi:add-project", handleAddProject);
    });
  });
  onCleanup(() => {
    for (const attachment of untrack(attachments)) revokeAttachmentPreview(attachment);
  });
  createEffect(() => {
    const sessionId = selectedSessionId();
    const agentViewActive = activeView() === "agent";
    sessionProjectionRevision();
    if (!sessionId) {
      setSessionSnapshot(undefined);
      setLiveItemDeltas({});
      setPendingUserInputs([]);
      return;
    }
    if (!agentViewActive) return;
    let disposed = false;
    let subscriptionId: string | undefined;
    let lastSequence = 0;
    let projectionRequestId = 0;
    let stopEvents: (() => void) | undefined;
    let stopEnvironment: (() => void) | undefined;
    const loadProjection = async () => {
      const requestId = ++projectionRequestId;
      try {
        const resume = await commandPort.resumeAgentSession({
          sessionId,
          metadataOnly: true,
          transcriptBeforeSequence: null,
          transcriptLimit: 0,
        });
        const snapshot = await commandPort.getWorkbenchSession(sessionId);
        if (
          requestId === projectionRequestId &&
          !disposed &&
          untrack(selectedSessionId) === sessionId
        ) {
          setSessionSnapshot(snapshot);
          const inProgress = snapshot.transcript.filter((item) => item.status === "in_progress");
          setLiveItemDeltas((current) => {
            const seeded = { ...current };
            for (const item of inProgress) {
              seeded[item.id] ??= { text: "", kind: item.kind };
            }
            return Object.fromEntries(
              Object.entries(seeded).filter(([itemId]) =>
                inProgress.some((item) => item.id === itemId),
              ),
            ) as Record<string, LiveItemDelta>;
          });
          setPendingUserInputs((current) =>
            reconcilePendingUserInputs(current, resume.pendingUserInputs),
          );
          setSessions((current) => [
            snapshot.session,
            ...current.filter((session) => session.id !== snapshot.session.id),
          ]);
        }
        return resume;
      } catch (error) {
        if (!disposed) setFailure(commandFailure(error).message);
        return undefined;
      }
    };
    const connect = async () => {
      stopEnvironment = await commandPort.onEnvironmentChange((event) => {
        if (!disposed && event.sessionId === sessionId && event.reasons.includes("browser")) {
          void loadProjection();
        }
      });
      if (disposed) {
        stopEnvironment();
        return;
      }
      // eslint-disable-next-line solid/reactivity -- this external event callback intentionally updates the current Session signals.
      stopEvents = await commandPort.onAgentEvents((batch) => {
        if (
          disposed ||
          !subscriptionId ||
          batch.subscription.id !== subscriptionId ||
          batch.subscription.sessionId !== sessionId
        )
          return;
        const unseen = reduceAgentEventWatermark(lastSequence, batch.catchUp);
        if (unseen.events.length === 0) return;
        lastSequence = unseen.nextSequence;
        setLiveItemDeltas((current) => reduceLiveItemDeltas(current, unseen.events));
        if (unseen.events.some((event) => event.payload.type !== "item_delta")) {
          void loadProjection();
          void refreshRunRecoveries();
          void refreshSessionActivity();
        }
      });
      if (disposed) {
        stopEvents();
        return;
      }
      const resume = await loadProjection();
      if (!resume || disposed) return;
      const activeReplay = resume.activeEventReplay ?? [];
      setLiveItemDeltas((current) => reduceLiveItemDeltas(current, activeReplay));
      lastSequence = Math.max(
        resume.snapshotSequence,
        ...activeReplay.map((event) => event.sequence),
      );
      try {
        const subscription = await commandPort.subscribeAgentEvents({
          sessionId,
          afterSequence: lastSequence,
        });
        if (disposed) {
          await commandPort.unsubscribeAgentEvents(subscription.subscription.id).catch(() => false);
          return;
        }
        subscriptionId = subscription.subscription.id;
        const unseen = reduceAgentEventWatermark(lastSequence, subscription.catchUp);
        if (unseen.events.length > 0) {
          lastSequence = unseen.nextSequence;
          await loadProjection();
        }
      } catch (error) {
        if (!disposed) setFailure(commandFailure(error).message);
      }
    };
    setSessionSnapshot(undefined);
    setLiveItemDeltas({});
    setPendingUserInputs([]);
    void connect();
    onCleanup(() => {
      disposed = true;
      stopEvents?.();
      stopEnvironment?.();
      if (subscriptionId)
        void commandPort.unsubscribeAgentEvents(subscriptionId).catch(() => false);
    });
  });

  async function addProject() {
    setAddingProject(true);
    setFailure(undefined);
    try {
      const project = await commandPort.addWorkbenchProject();
      if (project) {
        setRemovedProjectIds((current) => current.filter((id) => id !== project.id));
        await refreshWorkbench(project.id);
      }
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setAddingProject(false);
    }
  }

  function chooseAttachments() {
    setActivePopover(undefined);
    setFailure(undefined);
    void importAttachment();
  }

  async function importAttachment() {
    try {
      const imported = await commandPort.importWorkbenchAttachment();
      if (!imported) return;
      setAttachments((current) =>
        current.some((attachment) => attachment.attachmentId === imported.id)
          ? current
          : [
              ...current,
              {
                id: crypto.randomUUID(),
                sourceKey: "import:" + imported.id,
                kind: "file",
                name: imported.originalName,
                mimeType: imported.mimeType,
                byteSize: imported.byteSize,
                fileCount: 1,
                attachmentId: imported.id,
              },
            ],
      );
    } catch (error) {
      setFailure(commandFailure(error).message);
    }
  }

  function removeAttachment(attachmentId: string) {
    setAttachments((current) => {
      const removed = current.find((attachment) => attachment.id === attachmentId);
      if (removed) revokeAttachmentPreview(removed);
      return current.filter((attachment) => attachment.id !== attachmentId);
    });
  }

  function clearAttachments() {
    setAttachments((current) => {
      for (const attachment of current) revokeAttachmentPreview(attachment);
      return [];
    });
  }

  function selectProject(projectId: string) {
    setActiveView("agent");
    setSelectedProjectId(projectId);
    setSelectedSessionId(undefined);
    setTaskSnapshot(undefined);
    setSessionSnapshot(undefined);
    setPendingUserInputs([]);
  }

  async function ensureProjectToolSnapshot() {
    const projectId = selectedProjectId();
    return projectId ? projectTools.ensure(projectId) : undefined;
  }

  function openBottomTerminal() {
    if (!selectedProjectId()) return;
    setBottomPanelOpen(true);
    void ensureProjectToolSnapshot();
  }

  function selectSession(session: SessionRecord) {
    setActiveView("agent");
    setSelectedProjectId(sessionProjectId(session));
    setSelectedSessionId(session.id);
    setSessionProjectionRevision((revision) => revision + 1);
    setTaskSnapshot(undefined);
    setFailure(undefined);
    const terminal = sessionItems().find(
      (item) => item.session.id === session.id,
    )?.latestTerminalRun;
    if (terminal) setReadSessions((current) => ({ ...current, [session.id]: terminal.id }));
  }

  function newTask(projectId?: string) {
    setActiveView("agent");
    const nextProjectId = projectId;
    setSelectedProjectId(nextProjectId);
    setDraft("");
    clearAttachments();
    setSelectedSkillIds([]);
    setSelectedSessionId(undefined);
    setTaskSnapshot(undefined);
    setSessionSnapshot(undefined);
    setPendingUserInputs([]);
    setFailure(undefined);
    projectGit.resetForDraft();
    queueMicrotask(() => composerInput?.focus());
  }

  async function handleProjectAction(project: ProjectRecord, action: ProjectMenuAction) {
    setFailure(undefined);
    if (action === "pin") {
      setPinnedProjectIds((current) =>
        current.includes(project.id)
          ? current.filter((id) => id !== project.id)
          : [project.id, ...current],
      );
      return;
    }
    if (action === "rename") {
      setRenameTarget(project);
      setRenameDraft(project.displayName);
      return;
    }
    if (action === "mark_read") {
      setReadSessions((current) => ({
        ...current,
        ...Object.fromEntries(
          sessions()
            .filter((session) => sessionProjectId(session) === project.id)
            .flatMap((session) => {
              const terminal = sessionItems().find(
                (item) => item.session.id === session.id,
              )?.latestTerminalRun;
              return terminal ? [[session.id, terminal.id]] : [];
            }),
        ),
      }));
      return;
    }
    if (action === "remove") {
      setRemoveTarget(project);
      return;
    }
    if (action === "archive_tasks") {
      await archiveProjectTasks(project);
      return;
    }
    setProjectActionBusy(true);
    try {
      if (action === "open") {
        await commandPort.manageWorkbenchProject(project.id, "open");
      } else {
        const refs = await commandPort.listProjectGitRefs(project.id);
        const revision = refs.find((reference) => reference.current)?.name ?? refs[0]?.name;
        if (!revision) throw new Error(i18n.t("workbench.branchRequired"));
        await commandPort.manageWorkbenchProject(project.id, "create_permanent_worktree", revision);
      }
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setProjectActionBusy(false);
    }
  }

  async function renameProject() {
    const project = renameTarget();
    const displayName = renameDraft().trim();
    if (!project || !displayName || displayName === project.displayName) {
      setRenameTarget(undefined);
      return;
    }
    setProjectActionBusy(true);
    setFailure(undefined);
    try {
      const renamed = await commandPort.manageWorkbenchProject(project.id, "rename", displayName);
      setProjects((current) =>
        current.map((candidate) => (candidate.id === renamed.id ? renamed : candidate)),
      );
      setRenameTarget(undefined);
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setProjectActionBusy(false);
    }
  }

  async function handleSessionAction(session: SessionRecord, action: SessionMenuAction) {
    setFailure(undefined);
    if (action === "rename") {
      setRenameSessionTarget(session);
      setRenameSessionDraft(session.title);
      return;
    }
    setSessionActionBusy(true);
    try {
      if (action === "fork") {
        const source =
          selectedSessionId() === session.id && sessionSnapshot()
            ? sessionSnapshot()!
            : await commandPort.getWorkbenchSession(session.id);
        const sourceRun = source.runs.toReversed().find((run) => isTerminalRunStatus(run.status));
        if (!sourceRun) {
          throw new Error(
            i18n.locale() === "zh-CN"
              ? "该会话还没有可用于 Fork 的终态运行。"
              : "This session has no terminal run available to fork.",
          );
        }
        const forked = await commandPort.forkAgentSession({
          context: directUserMutationContext(),
          sourceSessionId: session.id,
          sourceRunId: sourceRun.id,
          title: `${session.title} (fork)`,
        });
        setSessions((current) => [forked, ...current.filter((item) => item.id !== forked.id)]);
        selectSession(forked);
        setSessionSnapshot(await commandPort.getWorkbenchSession(forked.id));
        return;
      }
      const updated = await commandPort.updateAgentSessionMetadata({
        context: directUserMutationContext(),
        sessionId: session.id,
        title: null,
        archived: action === "archive" ? true : action === "unarchive" ? false : null,
        pinned: action === "pin" ? !session.pinned : null,
      });
      setSessions((current) =>
        current.map((candidate) => (candidate.id === updated.id ? updated : candidate)),
      );
      if (selectedSessionId() === updated.id) {
        if (updated.archived) {
          setSelectedSessionId(undefined);
          setSessionSnapshot(undefined);
        } else {
          setSessionSnapshot((current) => (current ? { ...current, session: updated } : current));
        }
      }
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setSessionActionBusy(false);
    }
  }

  async function renameSession() {
    const session = renameSessionTarget();
    const title = renameSessionDraft().trim();
    if (!session || !title || title === session.title) {
      setRenameSessionTarget(undefined);
      return;
    }
    setSessionActionBusy(true);
    setFailure(undefined);
    try {
      const updated = await commandPort.updateAgentSessionMetadata({
        context: directUserMutationContext(),
        sessionId: session.id,
        title,
        archived: null,
        pinned: null,
      });
      setSessions((current) =>
        current.map((candidate) => (candidate.id === updated.id ? updated : candidate)),
      );
      setSessionSnapshot((current) =>
        current?.session.id === updated.id ? { ...current, session: updated } : current,
      );
      setRenameSessionTarget(undefined);
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setSessionActionBusy(false);
    }
  }

  async function archiveProjectTasks(project: ProjectRecord) {
    const targets = sessions().filter(
      (session) => sessionProjectId(session) === project.id && !session.archived,
    );
    if (targets.length === 0) return;
    setProjectActionBusy(true);
    setFailure(undefined);
    try {
      const archived = await Promise.all(
        targets.map((session) =>
          commandPort.updateAgentSessionMetadata({
            context: directUserMutationContext(),
            sessionId: session.id,
            title: null,
            archived: true,
            pinned: null,
          }),
        ),
      );
      const archivedById = new Map(archived.map((session) => [session.id, session]));
      setSessions((current) => current.map((session) => archivedById.get(session.id) ?? session));
      if (selectedSessionId() && archivedById.has(selectedSessionId()!)) newTask(project.id);
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setProjectActionBusy(false);
    }
  }

  function removeProjectFromSidebar() {
    const project = removeTarget();
    if (!project) return;
    setRemovedProjectIds((current) =>
      current.includes(project.id) ? current : [...current, project.id],
    );
    setPinnedProjectIds((current) => current.filter((id) => id !== project.id));
    if (selectedProjectId() === project.id) {
      const nextProject = visibleProjects().find((candidate) => candidate.id !== project.id);
      setSelectedProjectId(nextProject?.id);
      setSelectedSessionId(undefined);
      setSessionSnapshot(undefined);
      setTaskSnapshot(undefined);
    }
    setRemoveTarget(undefined);
  }

  async function startTask() {
    const project = selectedProject();
    const prompt = draft().trim();
    if (!prompt) {
      setFailure(i18n.t("workbench.promptRequired"));
      return;
    }
    const selectedSession = sessionSnapshot();
    const steeringRun = activeRun();
    if (
      !selectedSession &&
      project &&
      projectGit.executionKind() === "managed_worktree" &&
      !projectGit.baseRevision()
    ) {
      setFailure(i18n.t("workbench.branchRequired"));
      return;
    }
    setSubmitting(true);
    setFailure(undefined);
    try {
      if (steeringRun) {
        if (attachments().length > 0 || selectedSkillIds().length > 0) {
          throw new Error(
            i18n.locale() === "zh-CN"
              ? "运行中的会话只接受文本引导；请在新一轮运行中添加附件或 Skills。"
              : "Active runs accept text steering only; add attachments or Skills in a fresh run.",
          );
        }
        await commandPort.steerAgentRun({
          context: runMutationContext(steeringRun),
          runId: steeringRun.id,
          input: prompt,
        });
        setDraft("");
        if (selectedSessionId()) {
          setSessionSnapshot(await commandPort.getWorkbenchSession(selectedSessionId()!));
        }
        return;
      }
      const snapshot = await commandPort.startWorkbenchTask({
        idempotencyKey: crypto.randomUUID(),
        entryProfile: "workbench",
        sessionId: selectedSession?.session.id ?? null,
        projectId: project?.id ?? null,
        prompt,
        executionTarget: selectedSession
          ? null
          : project && projectGit.executionKind() === "managed_worktree"
            ? {
                kind: "managed_worktree",
                project_id: project.id,
                base_revision: projectGit.baseRevision(),
              }
            : project
              ? { kind: "local", project_id: project.id }
              : null,
        behaviorMode: behaviorMode(),
        permissionProfile: behaviorMode() === "plan" ? "read_only" : permissionProfile(),
        attachmentIds: attachments().flatMap((attachment) =>
          attachment.attachmentId ? [attachment.attachmentId] : [],
        ),
        skillIds: selectedSkillIds(),
      });
      setTaskSnapshot(snapshot);
      setSelectedSessionId(snapshot.session.id);
      setSessions((current) => [
        snapshot.session,
        ...current.filter((session) => session.id !== snapshot.session.id),
      ]);
      void refreshSessionActivity();
      setDraft("");
      clearAttachments();
      setSelectedSkillIds([]);
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setSubmitting(false);
    }
  }

  function updatePopover(id: ComposerPopoverId, open: boolean) {
    setActivePopover((current) => (open ? id : current === id ? undefined : current));
    if (open && id === "options" && !skillsLoading()) void refreshSkills();
  }

  function toggleSkill(skillId: string) {
    setSelectedSkillIds((current) =>
      current.includes(skillId)
        ? current.filter((currentSkillId) => currentSkillId !== skillId)
        : [...current, skillId],
    );
  }

  async function resolveApproval(approval: ApprovalRequestRecord, decision: ApprovalStatus) {
    setResolvingApprovalId(approval.id);
    setFailure(undefined);
    try {
      await commandPort.resolveWorkbenchApproval({
        approvalId: approval.id,
        decision,
        expectedRunId: approval.runId,
        expectedGeneration: approval.runGeneration,
      });
      const sessionId = selectedSessionId();
      if (sessionId) setSessionSnapshot(await commandPort.getWorkbenchSession(sessionId));
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setResolvingApprovalId(undefined);
    }
  }

  async function resolveHostAccess(request: HostAccessRequestRecord, decision: HostAccessDecision) {
    setResolvingHostAccessId(request.id);
    setFailure(undefined);
    try {
      const resolved = await commandPort.resolveHostAccessRequest({
        requestId: request.id,
        decision,
      });
      setSessionSnapshot((current) =>
        current
          ? {
              ...current,
              hostAccessRequests: current.hostAccessRequests.map((candidate) =>
                candidate.id === resolved.id ? resolved : candidate,
              ),
            }
          : current,
      );
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setResolvingHostAccessId(undefined);
    }
  }

  async function resolveRecovery(snapshot: RunRecoverySnapshot, action: RunRecoveryDecisionAction) {
    setResolvingRecoveryId(snapshot.recovery.id);
    setFailure(undefined);
    try {
      await commandPort.resolveRunRecovery({
        context: runMutationContext({
          id: snapshot.recovery.runId,
          generation: snapshot.recovery.interruptedGeneration,
        }),
        recoveryId: snapshot.recovery.id,
        expectedRunId: snapshot.recovery.runId,
        expectedInterruptedGeneration: snapshot.recovery.interruptedGeneration,
        action,
      });
      await Promise.all([refreshRunRecoveries(), refreshWorkbench()]);
      setSessionProjectionRevision((value) => value + 1);
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setResolvingRecoveryId(undefined);
    }
  }

  async function cancelRun(run: RunRecord) {
    setCancellingRun(true);
    setFailure(undefined);
    try {
      await commandPort.cancelWorkbenchRun(run.id, run.generation);
      const sessionId = selectedSessionId();
      if (sessionId) setSessionSnapshot(await commandPort.getWorkbenchSession(sessionId));
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setCancellingRun(false);
    }
  }

  async function resolveUserInput(
    request: UserInputRequestRecord,
    answers: UserInputAnswer[],
    action: UserInputResolutionAction,
  ) {
    setResolvingUserInputId(request.id);
    setFailure(undefined);
    try {
      await commandPort.resolveUserInput({
        requestId: request.id,
        expectedRunId: request.runId,
        expectedGeneration: request.runGeneration,
        action,
        answers,
        resolvedBy: "workbench",
        resolvedAtMs: Date.now(),
      });
      setPendingUserInputs((current) => current.filter((item) => item.id !== request.id));
      const sessionId = selectedSessionId();
      if (sessionId) setSessionSnapshot(await commandPort.getWorkbenchSession(sessionId));
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setResolvingUserInputId(undefined);
    }
  }

  async function acceptPlan(plan: ProposedPlan) {
    setAcceptingPlanId(plan.id);
    setFailure(undefined);
    try {
      const accepted = await commandPort.acceptWorkbenchPlan({
        idempotencyKey: crypto.randomUUID(),
        planId: plan.id,
        expectedRevision: plan.revision,
        userMessage: i18n.locale() === "zh-CN" ? "是的，执行此计划" : "Yes, implement this plan",
      });
      setTaskSnapshot(accepted.task);
      setSelectedSessionId(accepted.task.session.id);
      setSessionSnapshot(await commandPort.getWorkbenchSession(accepted.task.session.id));
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setAcceptingPlanId(undefined);
    }
  }

  async function revisePlan(plan: ProposedPlan, instructions: string) {
    setRevisingPlanId(plan.id);
    setFailure(undefined);
    try {
      const task = await commandPort.reviseWorkbenchPlan({
        idempotencyKey: crypto.randomUUID(),
        planId: plan.id,
        expectedRevision: plan.revision,
        instructions,
      });
      setTaskSnapshot(task);
      setDismissedPlanId(plan.id);
      setSelectedSessionId(task.session.id);
      setSessionSnapshot(await commandPort.getWorkbenchSession(task.session.id));
      void refreshSessionActivity();
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setRevisingPlanId(undefined);
    }
  }

  return (
    <div
      class="home-layout"
      style={{ "--workbench-project-sidebar-width": `${safeProjectSidebarWidth()}px` }}
    >
      <ProjectSidebar
        openSettings={() => props.navigate("settings/general")}
        openMotionLab={() => props.navigate("developer/motion-lab")}
        motionLabEnabled={props.motionLabEnabled}
        schedulerEnabled={props.schedulerEnabled}
        onNewTask={newTask}
        onOpenTasks={() => setActiveView("tasks")}
        activeView={activeView()}
        projects={visibleProjects()}
        sessions={sessions()}
        selectedProjectId={selectedProjectId()}
        selectedSessionId={selectedSessionId()}
        pinnedProjectIds={pinnedProjectIds()}
        unreadSessionIds={unreadSessionIds()}
        runningSessionIds={runningSessionIds()}
        failedSessionIds={failedSessionIds()}
        loading={loading()}
        addingProject={addingProject()}
        onAddProject={() => void addProject()}
        onSelectProject={selectProject}
        onSelectSession={selectSession}
        onProjectAction={(project, action) => void handleProjectAction(project, action)}
        onSessionAction={(session, action) => void handleSessionAction(session, action)}
      />
      <WorkbenchResizeHandle
        class="workbench-project-resize-handle"
        orientation="vertical"
        value={safeProjectSidebarWidth()}
        minimum={220}
        maximum={Math.max(
          220,
          Math.min(
            480,
            window.innerWidth -
              (inspectorVisible() && window.innerWidth > 1050 ? safeInspectorWidth() : 0) -
              420,
          ),
        )}
        defaultValue={288}
        label={i18n.locale() === "zh-CN" ? "调整项目栏宽度" : "Resize project sidebar"}
        onChange={setProjectSidebarWidth}
      />
      <main
        class="home-main"
        classList={{ "tasks-active": activeView() === "tasks" }}
        style={{
          "--workbench-workspace-width": `${activeView() === "agent" && inspectorVisible() ? safeInspectorWidth() : 0}px`,
          "--workbench-bottom-panel-height": `${safeBottomPanelHeight()}px`,
        }}
      >
        <Show when={activeView() === "agent"}>
          <WorkbenchToolbar
            locale={i18n.locale()}
            hasProject={Boolean(selectedProject())}
            hasSession={Boolean(sessionSnapshot())}
            sessionTitle={sessionSnapshot()?.session.title}
            summaryPinned={summaryPinned()}
            bottomPanelOpen={bottomPanelOpen()}
            sidebarVisible={inspectorVisible()}
            onOpenLocation={() => {
              const project = selectedProject();
              if (project) void handleProjectAction(project, "open");
            }}
            onToggleSummary={() => {
              setSummaryPinned((value) => !value);
            }}
            onToggleBottomPanel={() => {
              if (bottomPanelOpen()) {
                setBottomPanelOpen(false);
                return;
              }
              openBottomTerminal();
            }}
            onToggleSidebar={() => {
              if (inspectorVisible()) {
                setInspectorVisible(false);
                return;
              }
              setInspectorTabs((current) => showInspectorTabs(current));
              setInspectorVisible(true);
            }}
          />
        </Show>
        <Show when={activeView() === "tasks"}>
          <TaskCenter
            commandPort={commandPort}
            skills={skills()}
            onOpenSession={(sessionId) => {
              void commandPort
                .getWorkbenchSession(sessionId)
                .then((snapshot) => {
                  setSelectedProjectId(sessionProjectId(snapshot.session));
                  setSelectedSessionId(sessionId);
                  setSessionSnapshot(snapshot);
                  setActiveView("agent");
                })
                .catch((error) => setFailure(commandFailure(error).message));
            }}
          />
        </Show>
        <div class="home-agent-surface" classList={{ hidden: activeView() === "tasks" }}>
          <Show
            when={sessionSnapshot()}
            fallback={
              <div class="session-workspace-layout draft-workspace-layout">
                <div class="welcome-block">
                  <div class="welcome-mark">
                    <span>H</span>
                  </div>
                  <h1>
                    {selectedProject()
                      ? i18n
                          .t("workbench.buildPromptForProject")
                          .replace("{project}", selectedProject()!.displayName)
                      : i18n.t("workbench.buildPrompt")}
                  </h1>
                </div>
                <Show when={inspectorVisible()}>
                  <Show
                    when={inspectorSnapshot()}
                    fallback={
                      <DraftAttachmentInspector
                        resource={inspectorResource()}
                        hasProject={Boolean(selectedProject())}
                        loading={projectTools.loading(selectedProjectId())}
                        commandPort={commandPort}
                        locale={i18n.locale()}
                        tabs={inspectorTabs().tabs}
                        activeTabId={inspectorTabs().activeTabId}
                        onOpenInspector={openInspector}
                        onSelectTab={selectInspector}
                        onCloseTab={closeInspector}
                        onOpenLauncher={openInspectorLauncher}
                        onOpenTerminal={openBottomTerminal}
                      />
                    }
                  >
                    {(snapshot) => (
                      <SessionInspector
                        snapshot={snapshot()}
                        resource={inspectorResource()}
                        commandPort={commandPort}
                        locale={i18n.locale()}
                        tabs={inspectorTabs().tabs}
                        activeTabId={inspectorTabs().activeTabId}
                        onOpenInspector={openInspector}
                        onSelectTab={selectInspector}
                        onCloseTab={closeInspector}
                        onOpenLauncher={openInspectorLauncher}
                        onOpenTerminal={openBottomTerminal}
                      />
                    )}
                  </Show>
                </Show>
              </div>
            }
          >
            {(snapshot) => (
              <div class="session-workspace-layout">
                <div class="session-primary-column">
                  <Show when={summaryPinned()}>
                    <div class="workbench-pinned-summary">
                      <ConnectedEnvironmentSummary
                        snapshot={snapshot()}
                        commandPort={commandPort}
                        locale={i18n.locale()}
                        remotePushEnabled={props.gitRemoteMutationsEnabled}
                        onOpenInspector={openInspector}
                        onHandoff={(response) => {
                          setSessionSnapshot((current) =>
                            current
                              ? {
                                  ...current,
                                  session: response.session,
                                  checkout: response.checkout,
                                }
                              : current,
                          );
                          setSessions((current) => [
                            response.session,
                            ...current.filter((item) => item.id !== response.session.id),
                          ]);
                          setSessionProjectionRevision((revision) => revision + 1);
                        }}
                      />
                    </div>
                  </Show>
                  <div ref={setSessionViewport} class="session-scroll-viewport">
                    <CodexSessionTimeline
                      snapshot={snapshot()}
                      pendingGate={activeGateKind()}
                      recoveries={selectedRunRecoveries()}
                      liveItemDeltas={liveItemDeltas()}
                      resolvingRecoveryId={resolvingRecoveryId()}
                      onContentMount={setSessionTimelineContent}
                      onResolveRecovery={(recovery, action) =>
                        void resolveRecovery(recovery, action)
                      }
                      onOpenItem={(item) => {
                        if (item.kind === "plan" && item.payload.type === "plan") {
                          openInspector({ kind: "plan", planId: item.payload.data.plan_id });
                        } else if (
                          item.kind === "file_change" &&
                          item.payload.type === "file_change"
                        ) {
                          openInspector({
                            kind: "files",
                            path: item.payload.data.path,
                          });
                        } else {
                          openInspector({ kind: "review" });
                        }
                      }}
                      onOpenAttachment={(attachment) =>
                        openInspector({
                          kind: "attachment",
                          attachmentId: attachment.id,
                          name: attachment.originalName,
                        })
                      }
                      onOpenPath={(path) => openInspector({ kind: "files", path })}
                      onOpenDiff={(runId, path) =>
                        openInspector({
                          kind: "review",
                          diffRunId: runId,
                          diffScope: "run",
                          ...(path ? { path } : {}),
                        })
                      }
                    />
                  </div>
                </div>
                <Show when={inspectorVisible() && inspectorResource()}>
                  <Show
                    when={inspectorSnapshot()}
                    fallback={
                      <DraftAttachmentInspector
                        resource={inspectorResource()}
                        hasProject={Boolean(selectedProject())}
                        loading={projectTools.loading(selectedProjectId())}
                        commandPort={commandPort}
                        locale={i18n.locale()}
                        tabs={inspectorTabs().tabs}
                        activeTabId={inspectorTabs().activeTabId}
                        onOpenInspector={openInspector}
                        onSelectTab={selectInspector}
                        onCloseTab={closeInspector}
                        onOpenLauncher={openInspectorLauncher}
                        onOpenTerminal={openBottomTerminal}
                      />
                    }
                  >
                    {(inspector) => (
                      <SessionInspector
                        snapshot={inspector()}
                        resource={inspectorResource()}
                        commandPort={commandPort}
                        locale={i18n.locale()}
                        tabs={inspectorTabs().tabs}
                        activeTabId={inspectorTabs().activeTabId}
                        onOpenInspector={openInspector}
                        onSelectTab={selectInspector}
                        onCloseTab={closeInspector}
                        onOpenLauncher={openInspectorLauncher}
                        onOpenTerminal={openBottomTerminal}
                      />
                    )}
                  </Show>
                </Show>
              </div>
            )}
          </Show>
          <Show when={inspectorVisible()}>
            <WorkbenchResizeHandle
              class="workbench-inspector-resize-handle"
              orientation="vertical"
              value={safeInspectorWidth()}
              minimum={300}
              maximum={Math.max(
                300,
                Math.min(820, window.innerWidth - safeProjectSidebarWidth() - 420),
              )}
              defaultValue={380}
              direction={-1}
              label={i18n.locale() === "zh-CN" ? "调整右侧工作区宽度" : "Resize right workspace"}
              onChange={setInspectorWidth}
            />
          </Show>
          <div class="composer-wrap">
            <Show when={sessionSnapshot() && !sessionScroll.atBottom()}>
              <div class="timeline-jump-bottom-wrap">
                <Button
                  class="timeline-jump-bottom"
                  aria-label={i18n.locale() === "zh-CN" ? "回到底部" : "Jump to latest"}
                  title={i18n.locale() === "zh-CN" ? "回到底部" : "Jump to latest"}
                  onClick={sessionScroll.scrollToBottom}
                >
                  <ChevronDown size={17} />
                </Button>
              </div>
            </Show>
            <SandboxReadinessBanner
              commandPort={commandPort}
              initialReport={agentControl()?.sandbox}
              onFailure={setFailure}
            />
            <Show when={hasComposerGate()}>
              <WorkbenchGate
                locale={i18n.locale()}
                userInput={pendingUserInputs()[0]}
                hostAccess={activeHostAccess()}
                approval={activeApproval()}
                plan={activePlan()}
                resolvingUserInput={Boolean(resolvingUserInputId())}
                resolvingHostAccess={Boolean(resolvingHostAccessId())}
                resolvingApproval={Boolean(resolvingApprovalId())}
                acceptingPlan={Boolean(acceptingPlanId())}
                revisingPlan={Boolean(revisingPlanId())}
                onResolveUserInput={(request, answers, action) =>
                  void resolveUserInput(request, answers, action)
                }
                onResolveApproval={(approval, decision) => void resolveApproval(approval, decision)}
                onResolveHostAccess={(request, decision) =>
                  void resolveHostAccess(request, decision)
                }
                onAcceptPlan={(plan) => void acceptPlan(plan)}
                onRevisePlan={(plan, instructions) => void revisePlan(plan, instructions)}
                onDismissPlan={(plan) => setDismissedPlanId(plan.id)}
              />
            </Show>
            <div classList={{ hidden: hasComposerGate() }}>
              <ComposerContextControls
                activePopover={activePopover()}
                onOpenChange={updatePopover}
                projects={visibleProjects()}
                selectedProject={selectedProject()}
                executionKind={projectGit.executionKind()}
                gitRefs={projectGit.refs()}
                baseRevision={projectGit.baseRevision()}
                gitSnapshot={projectGit.snapshot()}
                gitLoading={projectGit.loading()}
                onSelectProject={selectProject}
                onSelectExecution={projectGit.setExecutionKind}
                onSelectBranch={projectGit.setBaseRevision}
                onRefreshGit={projectGit.refresh}
                onCreateInitialCommit={projectGit.openInitialCommit}
              />
              <Composer class="composer">
                <Show when={failure()}>
                  {(message) => <p class="composer-error">{message()}</p>}
                </Show>
                <ComposerAttachmentList
                  attachments={attachments()}
                  onRemove={removeAttachment}
                  onOpen={(attachment) => {
                    if (attachment.attachmentId) {
                      openInspector({
                        kind: "attachment",
                        attachmentId: attachment.attachmentId,
                        name: attachment.name,
                      });
                    }
                  }}
                />
                <div class="composer-editor">
                  <SkillReferenceList
                    skills={selectedSkills()}
                    onRemove={(skillId) => toggleSkill(skillId)}
                  />
                  <ComposerInput
                    ref={(element) => (composerInput = element)}
                    data-testid="workbench-composer-input"
                    label={i18n.t("workbench.draft")}
                    placeholder={i18n.t("workbench.draft")}
                    value={draft()}
                    onInput={(event) => setDraft(event.currentTarget.value)}
                  />
                </div>
                <div class="composer-footer">
                  <div class="composer-options">
                    <ComposerOptionsPopover
                      activePopover={activePopover()}
                      onOpenChange={updatePopover}
                      behaviorMode={behaviorMode()}
                      skills={skills()}
                      skillsLoading={skillsLoading()}
                      skillsError={skillsError()}
                      selectedSkillIds={selectedSkillIds()}
                      onChooseAttachments={chooseAttachments}
                      onTogglePlanMode={() => {
                        setBehaviorMode((current) => (current === "plan" ? "default" : "plan"));
                        setActivePopover(undefined);
                      }}
                      onToggleSkill={toggleSkill}
                    />
                    <PermissionProfilePopover
                      activePopover={activePopover()}
                      onOpenChange={updatePopover}
                      value={permissionProfile()}
                      onChange={setPermissionProfile}
                      extraAuthorizations={sessionPermissionConfig()?.extraAuthorizations ?? []}
                      onClearExtraAuthorizations={() => void clearSessionExtraAuthorizations()}
                    />
                    <Show when={behaviorMode() === "plan"}>
                      <PlanModeChip onDisable={() => setBehaviorMode("default")} />
                    </Show>
                  </div>
                  <Button
                    class="composer-send"
                    type="button"
                    data-testid="workbench-start-task"
                    disabled={
                      activeRun()
                        ? cancellingRun()
                        : submitting() ||
                          !props.workspaceToolsEnabled ||
                          (!selectedSessionId() &&
                            Boolean(selectedProject()) &&
                            projectGit.executionKind() === "managed_worktree" &&
                            !projectGit.baseRevision()) ||
                          !draft().trim()
                    }
                    title={
                      activeRun()
                        ? i18n.t("workbench.cancelRun")
                        : props.workspaceToolsEnabled
                          ? i18n.t("workbench.startTask")
                          : i18n.t("workbench.workspaceToolsDisabled")
                    }
                    onClick={() => {
                      const run = activeRun();
                      if (run) void cancelRun(run);
                      else void startTask();
                    }}
                  >
                    <Show when={activeRun()} fallback={<Send size={16} />}>
                      <Square size={15} />
                    </Show>
                  </Button>
                </div>
              </Composer>
              <Show when={!latestRun() && !taskSnapshot()}>
                <p class="composer-capability-note">
                  {props.workspaceToolsEnabled
                    ? i18n.t("workbench.taskReady")
                    : i18n.t("workbench.workspaceToolsDisabled")}
                </p>
              </Show>
            </div>
          </div>
          <Show when={bottomPanelOpen()}>
            <WorkbenchResizeHandle
              class="workbench-bottom-resize-handle"
              orientation="horizontal"
              value={safeBottomPanelHeight()}
              minimum={140}
              maximum={Math.max(140, Math.min(520, window.innerHeight - 260))}
              defaultValue={250}
              direction={-1}
              label={i18n.locale() === "zh-CN" ? "调整终端高度" : "Resize terminal"}
              onChange={setBottomPanelHeight}
            />
          </Show>
          <Show when={bottomPanelOpen()}>
            <div class="workbench-bottom-panel">
              <Show
                when={
                  toolSnapshot()?.session.context.kind === "project" ? toolSnapshot() : undefined
                }
                fallback={
                  <div class="draft-bottom-panel-empty">
                    <TerminalSquare size={18} />
                    <span>
                      {selectedProject()
                        ? projectTools.loading(selectedProjectId())
                          ? i18n.locale() === "zh-CN"
                            ? "正在准备项目终端…"
                            : "Preparing project terminal…"
                          : i18n.locale() === "zh-CN"
                            ? "项目终端暂时不可用"
                            : "Project terminal is temporarily unavailable"
                        : i18n.locale() === "zh-CN"
                          ? "请先选择项目"
                          : "Select a project first"}
                    </span>
                    <Button variant="ghost" onClick={() => setBottomPanelOpen(false)}>
                      ×
                    </Button>
                  </div>
                }
              >
                {(snapshot) => (
                  <TerminalPanel
                    projectId={selectedProjectId()!}
                    snapshot={snapshot()}
                    commandPort={commandPort}
                    onClose={() => setBottomPanelOpen(false)}
                  />
                )}
              </Show>
            </div>
          </Show>
        </div>
      </main>
      {projectGit.initialCommitDialog()}
      <Dialog
        open={Boolean(renameTarget())}
        title={i18n.locale() === "zh-CN" ? "重命名项目" : "Rename project"}
        description={
          i18n.locale() === "zh-CN"
            ? "项目路径不会改变，只更新工作台中显示的名称。"
            : "The project path stays unchanged; only its Workbench name is updated."
        }
        onOpenChange={(open) => {
          if (!open) setRenameTarget(undefined);
        }}
      >
        <div class="dialog-form project-dialog-form">
          <TextField
            label={i18n.locale() === "zh-CN" ? "项目名称" : "Project name"}
            value={renameDraft()}
            maxLength={120}
            autofocus
            onInput={(event) => setRenameDraft(event.currentTarget.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") void renameProject();
            }}
          />
          <div class="dialog-actions">
            <Button variant="ghost" onClick={() => setRenameTarget(undefined)}>
              {i18n.locale() === "zh-CN" ? "取消" : "Cancel"}
            </Button>
            <Button
              variant="primary"
              disabled={projectActionBusy() || !renameDraft().trim()}
              onClick={() => void renameProject()}
            >
              {i18n.locale() === "zh-CN" ? "保存" : "Save"}
            </Button>
          </div>
        </div>
      </Dialog>
      <Dialog
        open={Boolean(renameSessionTarget())}
        title={i18n.locale() === "zh-CN" ? "重命名会话" : "Rename session"}
        description={
          i18n.locale() === "zh-CN"
            ? "只更新会话标题，不改变历史、上下文或授权。"
            : "Only the title changes; history, context, and authority remain unchanged."
        }
        onOpenChange={(open) => {
          if (!open) setRenameSessionTarget(undefined);
        }}
      >
        <div class="dialog-form project-dialog-form">
          <TextField
            label={i18n.locale() === "zh-CN" ? "会话标题" : "Session title"}
            value={renameSessionDraft()}
            maxLength={200}
            autofocus
            onInput={(event) => setRenameSessionDraft(event.currentTarget.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") void renameSession();
            }}
          />
          <div class="dialog-actions">
            <Button variant="ghost" onClick={() => setRenameSessionTarget(undefined)}>
              {i18n.locale() === "zh-CN" ? "取消" : "Cancel"}
            </Button>
            <Button
              variant="primary"
              disabled={sessionActionBusy() || !renameSessionDraft().trim()}
              onClick={() => void renameSession()}
            >
              {i18n.locale() === "zh-CN" ? "保存" : "Save"}
            </Button>
          </div>
        </div>
      </Dialog>
      <Dialog
        open={Boolean(removeTarget())}
        title={i18n.locale() === "zh-CN" ? "移除项目" : "Remove project"}
        description={
          i18n.locale() === "zh-CN"
            ? `从工作台侧栏移除 ${removeTarget()?.displayName ?? ""}。项目文件和任务历史不会被删除，之后可通过“添加项目”重新加入。`
            : `Remove ${removeTarget()?.displayName ?? ""} from the Workbench sidebar. Files and task history are preserved and the folder can be added again later.`
        }
        tone="danger"
        onOpenChange={(open) => {
          if (!open) setRemoveTarget(undefined);
        }}
      >
        <div class="dialog-actions">
          <Button variant="ghost" onClick={() => setRemoveTarget(undefined)}>
            {i18n.locale() === "zh-CN" ? "取消" : "Cancel"}
          </Button>
          <Button tone="danger" onClick={removeProjectFromSidebar}>
            {i18n.locale() === "zh-CN" ? "移除" : "Remove"}
          </Button>
        </div>
      </Dialog>
    </div>
  );
}
