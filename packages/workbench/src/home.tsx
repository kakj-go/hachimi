import {
  CONTROL_PROTOCOL_VERSION,
  commandFailure,
  type ApprovalPolicy,
  type ApprovalRequestRecord,
  type ApprovalStatus,
  type BehaviorMode,
  type ControlInitializeResponse,
  type McpToolProgressRecord,
  type ProjectRecord,
  type ProposedPlan,
  type RunRecord,
  type RunRecoveryDecisionAction,
  type RunRecoverySnapshot,
  type SessionRecord,
  type SkillRecord,
  type UserInputAnswer,
  type UserInputRequestRecord,
  type UserInputResolutionAction,
  type WorkbenchRoute,
  type WorkbenchSessionSnapshot,
  type WorkbenchTaskSnapshot,
} from "@hachimi/contracts";
import {
  liveItemText,
  reconcilePendingUserInputs,
  reduceLiveItemDeltas,
  type LiveItemDelta,
  type LiveItemDeltas,
} from "./agent-live-items";
import { useI18n, type AppLocale } from "@hachimi/i18n";
import { reduceAgentEventWatermark } from "./agent-event-watermark";
import { ProviderContextPayload } from "./provider-context-payload";
import { AgentTaskPanel } from "./agent-task-panel";
import {
  AgentMessage,
  ApprovalCard,
  Badge,
  Bot,
  Box,
  Button,
  Composer,
  ComposerInput,
  Dialog,
  GitBranch,
  Palette,
  PanelLeftClose,
  PlanCard,
  Play,
  PromptCard,
  Send,
  ShieldCheck,
  Square,
  TerminalSquare,
  TextField,
  Volume2,
} from "@hachimi/ui";
import {
  For,
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
  ApprovalPolicyPopover,
  ComposerContextControls,
  ComposerOptionsPopover,
  SkillReferenceList,
  type ComposerPopoverId,
} from "./composer-popovers";
import { WorkspaceBrowser } from "./workspace-browser";
import { TaskCenter } from "./task-center";
import { TerminalPanel } from "./terminal";
import { UserInputCard } from "./user-input-card";
import { ReviewPanel } from "./review-panel";
import { ProjectSidebar, type ProjectMenuAction, type SessionMenuAction } from "./project-sidebar";
import "./composer-attachments.css";
import "./composer-popovers.css";

function SessionTimeline(props: {
  snapshot: WorkbenchSessionSnapshot;
  multiAgentEnabled: boolean;
  recoveries: RunRecoverySnapshot[];
  liveItemDeltas: LiveItemDeltas;
  pendingUserInputs: UserInputRequestRecord[];
  resolvingApprovalId: string | undefined;
  resolvingUserInputId: string | undefined;
  acceptingPlanId: string | undefined;
  cancelling: boolean;
  resolvingRecoveryId: string | undefined;
  onResolveApproval: (approval: ApprovalRequestRecord, decision: ApprovalStatus) => void;
  onAcceptPlan: (plan: ProposedPlan) => void;
  onResolveUserInput: (
    request: UserInputRequestRecord,
    answers: UserInputAnswer[],
    action: UserInputResolutionAction,
  ) => void;
  onCancel: (run: RunRecord) => void;
  onResolveRecovery: (recovery: RunRecoverySnapshot, action: RunRecoveryDecisionAction) => void;
}) {
  const i18n = useI18n();
  const latestRun = () => props.snapshot.runs[props.snapshot.runs.length - 1];
  const mcpProgress = () => latestMcpProgress(props.snapshot, latestRun()?.id);
  const canCancel = () => {
    const status = latestRun()?.status;
    return (
      status === "queued" ||
      status === "preparing" ||
      status === "running" ||
      status === "waiting_approval" ||
      status === "waiting_user_input" ||
      status === "cancelling"
    );
  };
  return (
    <section class="session-timeline" aria-label={i18n.t("workbench.timeline")}>
      <header class="session-timeline-header">
        <div>
          <small>{i18n.t("workbench.task")}</small>
          <h1>{props.snapshot.session.title}</h1>
        </div>
        <Show when={latestRun()}>
          {(run) => (
            <div class="run-status-actions">
              <Badge tone={run().status === "failed" ? "danger" : "info"}>{run().status}</Badge>
              <Show when={canCancel()}>
                <Button
                  size="small"
                  disabled={props.cancelling}
                  onClick={() => props.onCancel(run())}
                >
                  <Square size={13} /> {i18n.t("workbench.cancelRun")}
                </Button>
              </Show>
            </div>
          )}
        </Show>
      </header>
      <Show when={props.recoveries.length > 0}>
        <div class="recovery-stack" data-testid="run-recovery-stack">
          <For each={props.recoveries}>
            {(snapshot) => {
              const recovery = () => snapshot.recovery;
              const checkpoint = () => snapshot.checkpoint;
              const resolving = () => props.resolvingRecoveryId === recovery().id;
              const canResumeSafe = () =>
                !recovery().sideEffectExecutionId &&
                (checkpoint()?.recoveryPolicy === "read_only_replayable" ||
                  checkpoint()?.recoveryPolicy === "idempotent_with_receipt");
              const canRetry = () =>
                Boolean(recovery().sideEffectExecutionId) &&
                checkpoint()?.recoveryPolicy === "idempotent_with_receipt";
              return (
                <article class="recovery-card" data-testid={`run-recovery-${recovery().id}`}>
                  <div class="recovery-card-heading">
                    <span>
                      <ShieldCheck size={17} />
                      <strong>{i18n.t("workbench.recoveryRequired")}</strong>
                    </span>
                    <Badge tone={recovery().state === "awaiting_user" ? "warning" : "info"}>
                      {recovery().state}
                    </Badge>
                  </div>
                  <p>{i18n.t("workbench.recoveryDescription")}</p>
                  <small>
                    {recovery().reasonCode} · generation {recovery().interruptedGeneration} →{" "}
                    {recovery().resumeGeneration}
                    <Show when={checkpoint()}>
                      {(value) => ` · ${value().phase} · ${value().recoveryPolicy}`}
                    </Show>
                  </small>
                  <Show when={recovery().sideEffectExecutionId}>
                    <small class="recovery-risk">{i18n.t("workbench.recoveryIndeterminate")}</small>
                  </Show>
                  <footer>
                    <Button
                      size="small"
                      disabled={resolving() || recovery().state === "resuming"}
                      onClick={() => props.onResolveRecovery(snapshot, "abandon_run")}
                    >
                      {i18n.t("workbench.recoveryAbandon")}
                    </Button>
                    <Show when={recovery().sideEffectExecutionId}>
                      <Button
                        size="small"
                        disabled={resolving() || recovery().state === "resuming"}
                        onClick={() =>
                          props.onResolveRecovery(snapshot, "confirm_effect_succeeded")
                        }
                      >
                        {i18n.t("workbench.recoveryConfirmSucceeded")}
                      </Button>
                    </Show>
                    <Show when={canRetry()}>
                      <Button
                        size="small"
                        variant="primary"
                        disabled={resolving() || recovery().state === "resuming"}
                        onClick={() => props.onResolveRecovery(snapshot, "retry_idempotent_effect")}
                      >
                        {i18n.t("workbench.recoveryRetry")}
                      </Button>
                    </Show>
                    <Show when={canResumeSafe()}>
                      <Button
                        size="small"
                        variant="primary"
                        disabled={resolving() || recovery().state === "resuming"}
                        onClick={() => props.onResolveRecovery(snapshot, "resume_safe_remainder")}
                      >
                        {i18n.t("workbench.recoveryResumeSafe")}
                      </Button>
                    </Show>
                  </footer>
                </article>
              );
            }}
          </For>
        </div>
      </Show>
      <Show when={props.snapshot.pendingApprovals.length > 0}>
        <div class="approval-stack">
          <For each={props.snapshot.pendingApprovals}>
            {(approval) => (
              <ApprovalCard
                title={i18n.t("workbench.approvalRequired")}
                description={approval.resource}
                icon={<ShieldCheck size={17} />}
                actions={
                  <>
                    <Button
                      size="small"
                      disabled={props.resolvingApprovalId === approval.id}
                      onClick={() => props.onResolveApproval(approval, "denied")}
                    >
                      {i18n.t("workbench.deny")}
                    </Button>
                    <Button
                      size="small"
                      variant="primary"
                      data-testid="workbench-approve-once"
                      disabled={props.resolvingApprovalId === approval.id}
                      onClick={() => props.onResolveApproval(approval, "approved")}
                    >
                      {i18n.t("workbench.approveOnce")}
                    </Button>
                  </>
                }
              >
                <small>{approval.action}</small>
                <small>{approval.riskSummary}</small>
              </ApprovalCard>
            )}
          </For>
        </div>
      </Show>
      <Show when={props.pendingUserInputs.length > 0}>
        <div class="user-input-stack">
          <For each={props.pendingUserInputs}>
            {(request) => (
              <UserInputCard
                request={request}
                resolving={props.resolvingUserInputId === request.id}
                onResolve={props.onResolveUserInput}
              />
            )}
          </For>
        </div>
      </Show>
      <Show when={props.snapshot.proposedPlans.length > 0}>
        <div class="plan-stack">
          <For each={props.snapshot.proposedPlans}>
            {(plan) => (
              <PlanCard
                title={i18n
                  .t("workbench.proposedPlanRevision")
                  .replace("{revision}", String(plan.revision))}
                icon={<ShieldCheck size={17} />}
                actions={
                  <Show when={plan.status === "proposed"}>
                    <span>{i18n.t("workbench.planAcceptCreatesRun")}</span>
                    <Button
                      size="small"
                      variant="primary"
                      data-testid="workbench-execute-plan"
                      disabled={props.acceptingPlanId === plan.id}
                      onClick={() => props.onAcceptPlan(plan)}
                    >
                      <Play size={13} /> {i18n.t("workbench.executePlan")}
                    </Button>
                  </Show>
                }
              >
                <Badge tone={plan.status === "accepted" ? "success" : "info"}>{plan.status}</Badge>
                <pre>{clipTimelineText(plan.contentMarkdown)}</pre>
              </PlanCard>
            )}
          </For>
        </div>
      </Show>
      <Show when={props.snapshot.artifacts.length > 0}>
        <div class="evidence-stack">
          <For each={props.snapshot.artifacts.slice(-12)}>
            {(artifact) => (
              <article class="evidence-card code-panel" data-component="evidence-card">
                <header>
                  <span>
                    {artifact.kind === "diff_evidence" ? (
                      <GitBranch size={16} />
                    ) : (
                      <TerminalSquare size={16} />
                    )}
                    <strong>{artifact.displayName}</strong>
                  </span>
                  <Badge tone="neutral">
                    {artifact.kind === "diff_evidence"
                      ? i18n.t("workbench.diffEvidence")
                      : i18n.t("workbench.commandEvidence")}
                  </Badge>
                </header>
                <pre>{timelineItemText(artifact.metadata)}</pre>
              </article>
            )}
          </For>
        </div>
      </Show>
      <Show when={props.multiAgentEnabled}>
        <AgentTaskPanel tasks={props.snapshot.agentTasks} locale={i18n.locale()} />
      </Show>
      <Show when={canCancel() && mcpProgress().length > 0}>
        <div class="mcp-progress-stack" aria-label="MCP Tool progress">
          <For each={mcpProgress()}>
            {(progress) => (
              <article class="mcp-progress-card">
                <div>
                  <strong>{progress.toolCallId}</strong>
                  <small>
                    {i18n.locale() === "zh-CN"
                      ? "MCP 服务进度（不可信展示数据）"
                      : "MCP server progress (untrusted display data)"}
                  </small>
                </div>
                <progress
                  value={progress.progress}
                  max={Math.max(progress.total ?? progress.progress, progress.progress, 1)}
                />
                <span>
                  {progress.message ?? (i18n.locale() === "zh-CN" ? "正在执行…" : "Running…")}
                </span>
              </article>
            )}
          </For>
        </div>
      </Show>
      <div class="timeline-items agent-thread">
        <For each={props.snapshot.transcript}>
          {(item) => (
            <AgentMessage
              class={["timeline-item", "timeline-" + item.kind].join(" ")}
              component={item.kind === "tool_execution" ? "tool-call" : "agent-message"}
              role={item.kind === "user" ? "user" : "assistant"}
              author={timelineKindLabel(item.kind, i18n.locale())}
              meta={<time>{new Date(item.createdAtMs).toLocaleTimeString()}</time>}
            >
              <ProviderContextPayload
                payload={item.payload}
                locale={i18n.locale()}
                focusable={item.kind === "tool_execution"}
                text={timelineItemText(
                  item.payload,
                  item.status === "in_progress"
                    ? liveItemText(props.liveItemDeltas[item.id])
                    : undefined,
                )}
              />
            </AgentMessage>
          )}
        </For>
        <Show when={props.snapshot.transcript.length === 0}>
          <p class="timeline-empty">{i18n.t("workbench.timelineEmpty")}</p>
        </Show>
      </div>
    </section>
  );
}

function latestMcpProgress(
  snapshot: WorkbenchSessionSnapshot,
  runId: string | undefined,
): ValidMcpProgress[] {
  if (!runId) return [];
  const latest = new Map<string, ValidMcpProgress>();
  for (const event of snapshot.events) {
    if (
      event.payload.type !== "generic" ||
      event.payload.data.event !== "mcp.tool.progress" ||
      event.runId !== runId
    )
      continue;
    const progress = parseMcpProgress(event.payload.data.data);
    if (progress) latest.set(progress.toolCallId, progress);
  }
  return [...latest.values()];
}

type ValidMcpProgress = Omit<McpToolProgressRecord, "progress"> & { progress: number };

function parseMcpProgress(value: unknown): ValidMcpProgress | undefined {
  if (!value || typeof value !== "object") return undefined;
  const record = value as Record<string, unknown>;
  if (
    typeof record.serverId !== "string" ||
    typeof record.sessionId !== "string" ||
    typeof record.runId !== "string" ||
    typeof record.runGeneration !== "number" ||
    typeof record.toolCallId !== "string" ||
    typeof record.progress !== "number" ||
    !Number.isFinite(record.progress) ||
    record.progress < 0
  )
    return undefined;
  if (
    record.total !== null &&
    (typeof record.total !== "number" || !Number.isFinite(record.total) || record.total <= 0)
  )
    return undefined;
  if (record.message !== null && typeof record.message !== "string") return undefined;
  return record as ValidMcpProgress;
}

function timelineKindLabel(kind: string, locale: AppLocale): string {
  const labels: Record<string, [string, string]> = {
    user: ["用户", "User"],
    assistant: ["Hachimi", "Hachimi"],
    reasoning: ["思考", "Reasoning"],
    tool_call: ["工具调用", "Tool call"],
    tool_execution: ["工具执行", "Tool execution"],
    tool_result: ["工具结果", "Tool result"],
    plan: ["计划", "Plan"],
    approval: ["审批", "Approval"],
    user_input_request: ["用户输入", "User input"],
    system_context: ["系统", "System"],
  };
  const label = labels[kind];
  return label ? label[locale === "zh-CN" ? 0 : 1] : kind;
}

function timelineItemText(content: unknown, liveDelta?: string): string {
  if (liveDelta !== undefined) return clipTimelineText(liveDelta);
  if (typeof content === "string") return content;
  if (content && typeof content === "object") {
    const record = content as Record<string, unknown>;
    for (const key of ["text", "modelContent", "message"] as const) {
      if (typeof record[key] === "string") return clipTimelineText(record[key]);
    }
  }
  try {
    return clipTimelineText(JSON.stringify(content, null, 2));
  } catch {
    return String(content);
  }
}

function clipTimelineText(value: string): string {
  return value.length > 6_000 ? `${value.slice(0, 3_000)}\n…\n${value.slice(-3_000)}` : value;
}

const SELECTED_PROJECT_STORAGE_KEY = "hachimi.workbench.selectedProjectId";
const SELECTED_SESSION_STORAGE_KEY = "hachimi.workbench.selectedSessionId";
const PINNED_PROJECTS_STORAGE_KEY = "hachimi.workbench.pinnedProjectIds";
const REMOVED_PROJECTS_STORAGE_KEY = "hachimi.workbench.removedProjectIds";
const READ_SESSIONS_STORAGE_KEY = "hachimi.workbench.readSessions";

function readSessionSelection(key: string): string | undefined {
  try {
    return window.sessionStorage.getItem(key) ?? undefined;
  } catch {
    return undefined;
  }
}

function persistSessionSelection(key: string, value: string | undefined) {
  try {
    if (value) window.sessionStorage.setItem(key, value);
    else window.sessionStorage.removeItem(key);
  } catch {
    // WebView storage can be unavailable during teardown; persisted Agent
    // state remains authoritative and the user can select the Session again.
  }
}

function readLocalJson<T>(key: string, fallback: T): T {
  try {
    const value = window.localStorage.getItem(key);
    return value ? (JSON.parse(value) as T) : fallback;
  } catch {
    return fallback;
  }
}

function persistLocalJson(key: string, value: unknown) {
  try {
    window.localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Local UI preferences are best effort; persisted Agent data remains authoritative.
  }
}

export function HomePage(props: {
  navigate: (route: WorkbenchRoute) => void;
  motionLabEnabled: boolean;
  desktopControlEnabled: boolean;
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
  const [pinnedProjectIds, setPinnedProjectIds] = createSignal<string[]>(
    readLocalJson<string[]>(PINNED_PROJECTS_STORAGE_KEY, []),
  );
  const [removedProjectIds, setRemovedProjectIds] = createSignal<string[]>(
    readLocalJson<string[]>(REMOVED_PROJECTS_STORAGE_KEY, []),
  );
  const [readSessions, setReadSessions] = createSignal<Record<string, number>>(
    readLocalJson<Record<string, number>>(READ_SESSIONS_STORAGE_KEY, {}),
  );
  const [selectedProjectId, setSelectedProjectId] = createSignal<string | undefined>(
    readSessionSelection(SELECTED_PROJECT_STORAGE_KEY),
  );
  const [selectedSessionId, setSelectedSessionId] = createSignal<string | undefined>(
    readSessionSelection(SELECTED_SESSION_STORAGE_KEY),
  );
  const [sessionProjectionRevision, setSessionProjectionRevision] = createSignal(0);
  const [draftProjectId, setDraftProjectId] = createSignal<string>();
  const [behaviorMode, setBehaviorMode] = createSignal<BehaviorMode>("default");
  const [approvalPolicy, setApprovalPolicy] = createSignal<ApprovalPolicy>("only_when_needed");
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
  const [pendingUserInputs, setPendingUserInputs] = createSignal<UserInputRequestRecord[]>([]);
  const [runRecoveries, setRunRecoveries] = createSignal<RunRecoverySnapshot[]>([]);
  const [agentControl, setAgentControl] = createSignal<ControlInitializeResponse>();
  const [resolvingApprovalId, setResolvingApprovalId] = createSignal<string>();
  const [resolvingUserInputId, setResolvingUserInputId] = createSignal<string>();
  const [acceptingPlanId, setAcceptingPlanId] = createSignal<string>();
  const [cancellingRun, setCancellingRun] = createSignal(false);
  const [resolvingRecoveryId, setResolvingRecoveryId] = createSignal<string>();
  const [projectActionBusy, setProjectActionBusy] = createSignal(false);
  const [sessionActionBusy, setSessionActionBusy] = createSignal(false);
  const [renameTarget, setRenameTarget] = createSignal<ProjectRecord>();
  const [renameDraft, setRenameDraft] = createSignal("");
  const [renameSessionTarget, setRenameSessionTarget] = createSignal<SessionRecord>();
  const [renameSessionDraft, setRenameSessionDraft] = createSignal("");
  const [removeTarget, setRemoveTarget] = createSignal<ProjectRecord>();
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
        sessions()
          .filter(
            (session) =>
              !session.archived &&
              session.id !== selectedSessionId() &&
              session.updatedAtMs > (readSessions()[session.id] ?? 0),
          )
          .map((session) => session.id),
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
  const selectedRunRecoveries = createMemo(() =>
    runRecoveries().filter((value) => value.recovery.sessionId === selectedSessionId()),
  );
  let attachmentFileInput: HTMLInputElement | undefined;
  let attachmentFolderInput: HTMLInputElement | undefined;
  let composerInput: HTMLTextAreaElement | undefined;
  let stopSkillChanges: (() => void) | undefined;
  let skillSubscriptionId: string | undefined;

  createEffect(() => persistSessionSelection(SELECTED_PROJECT_STORAGE_KEY, selectedProjectId()));
  createEffect(() => persistSessionSelection(SELECTED_SESSION_STORAGE_KEY, selectedSessionId()));
  createEffect(() => persistLocalJson(PINNED_PROJECTS_STORAGE_KEY, pinnedProjectIds()));
  createEffect(() => persistLocalJson(REMOVED_PROJECTS_STORAGE_KEY, removedProjectIds()));
  createEffect(() => persistLocalJson(READ_SESSIONS_STORAGE_KEY, readSessions()));

  async function refreshWorkbench(preferredProjectId?: string) {
    setLoading(true);
    setFailure(undefined);
    try {
      const [nextProjects, nextSessions] = await Promise.all([
        commandPort.listWorkbenchProjects(),
        commandPort.listWorkbenchSessions(),
      ]);
      setProjects(nextProjects);
      setSessions(nextSessions);
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
      if (!nextSessions.some((session) => session.id === selectedSessionId())) {
        setSelectedSessionId(undefined);
      }
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setLoading(false);
    }
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
        clientVersion: "hachimi-desktop/0.2.1",
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

  function chooseAttachments(kind: "files" | "folder") {
    setActivePopover(undefined);
    setFailure(undefined);
    (kind === "files" ? attachmentFileInput : attachmentFolderInput)?.click();
  }

  function addSelectedFiles(fileList: FileList | null) {
    const files = Array.from(fileList ?? []);
    if (files.length === 0) return;
    setAttachments((current) => {
      const knownSources = new Set(current.map((attachment) => attachment.sourceKey));
      const added = files
        .filter((file) => !knownSources.has(fileSourceKey(file)))
        .map(createFileAttachmentPreview);
      return added.length > 0 ? [...current, ...added] : current;
    });
  }

  function addSelectedFolder(fileList: FileList | null) {
    const files = Array.from(fileList ?? []);
    if (files.length === 0) return;
    const folderName =
      files[0]?.webkitRelativePath.split("/")[0]?.trim() || i18n.t("workbench.folder");
    const sourceKey = `folder:${folderName}:${files.map(fileSourceKey).sort().join("|")}`;
    setAttachments((current) => {
      if (current.some((attachment) => attachment.sourceKey === sourceKey)) return current;
      return [
        ...current,
        {
          id: crypto.randomUUID(),
          sourceKey,
          kind: "folder",
          name: folderName,
          mimeType: "inode/directory",
          byteSize: files.reduce((total, file) => total + file.size, 0),
          fileCount: files.length,
        },
      ];
    });
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
    setDraftProjectId(projectId);
    setSelectedSessionId(undefined);
    setTaskSnapshot(undefined);
    setSessionSnapshot(undefined);
    setPendingUserInputs([]);
  }

  function selectSession(session: SessionRecord) {
    setActiveView("agent");
    setSelectedProjectId(sessionProjectId(session));
    setSelectedSessionId(session.id);
    setSessionProjectionRevision((revision) => revision + 1);
    setDraftProjectId(undefined);
    setTaskSnapshot(undefined);
    setFailure(undefined);
    setReadSessions((current) => ({ ...current, [session.id]: session.updatedAtMs }));
  }

  function newTask(projectId?: string) {
    setActiveView("agent");
    const nextProjectId = projectId;
    setSelectedProjectId(nextProjectId);
    setDraftProjectId(nextProjectId);
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
            .map((session) => [session.id, session.updatedAtMs]),
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
        approvalPolicy: approvalPolicy(),
        attachmentIds: attachments().flatMap((attachment) =>
          attachment.attachmentId ? [attachment.attachmentId] : [],
        ),
        skillIds: selectedSkillIds(),
      });
      setTaskSnapshot(snapshot);
      setSelectedSessionId(snapshot.session.id);
      setDraftProjectId(undefined);
      setSessions((current) => [
        snapshot.session,
        ...current.filter((session) => session.id !== snapshot.session.id),
      ]);
      setReadSessions((current) => ({
        ...current,
        [snapshot.session.id]: snapshot.session.updatedAtMs,
      }));
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

  const cards = createMemo(() => [
    {
      icon: <Bot size={20} />,
      title: i18n.t("workbench.guide.llm"),
      description: i18n.t("workbench.guide.llmDescription"),
      route: "settings/llm" as const,
    },
    {
      icon: <Box size={20} />,
      title: i18n.t("workbench.guide.avatar"),
      description: i18n.t("workbench.guide.avatarDescription"),
      route: "settings/avatar" as const,
    },
    {
      icon: <Volume2 size={20} />,
      title: i18n.t("workbench.guide.voice"),
      description: i18n.t("workbench.guide.voiceDescription"),
      route: "settings/voice" as const,
    },
    {
      icon: <Palette size={20} />,
      title: i18n.t("workbench.guide.general"),
      description: i18n.t("workbench.guide.generalDescription"),
      route: "settings/general" as const,
    },
  ]);
  return (
    <div class="home-layout">
      <ProjectSidebar
        openSettings={() => props.navigate("settings/general")}
        openMotionLab={() => props.navigate("developer/motion-lab")}
        openDesktopControl={() => props.navigate("desktop-control")}
        motionLabEnabled={props.motionLabEnabled}
        desktopControlEnabled={props.desktopControlEnabled}
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
        loading={loading()}
        addingProject={addingProject()}
        onAddProject={() => void addProject()}
        onSelectProject={selectProject}
        onSelectSession={selectSession}
        onProjectAction={(project, action) => void handleProjectAction(project, action)}
        onSessionAction={(session, action) => void handleSessionAction(session, action)}
      />
      <main class="home-main">
        <div class="home-layout-actions">
          <Button
            type="button"
            disabled
            aria-label="Layout"
            title={i18n.t("workbench.menuDisabled")}
          >
            <PanelLeftClose size={17} />
          </Button>
        </div>
        <Show when={activeView() === "tasks"}>
          <TaskCenter
            commandPort={commandPort}
            projects={visibleProjects()}
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
                <div class="guide-cards">
                  <For each={cards()}>
                    {(card) => (
                      <PromptCard onClick={() => props.navigate(card.route)}>
                        <span>{card.icon}</span>
                        <strong>{card.title}</strong>
                        <small>{card.description}</small>
                      </PromptCard>
                    )}
                  </For>
                </div>
              </div>
            }
          >
            {(snapshot) => (
              <div class="session-workspace-layout">
                <SessionTimeline
                  snapshot={snapshot()}
                  multiAgentEnabled={props.multiAgentEnabled}
                  recoveries={selectedRunRecoveries()}
                  liveItemDeltas={liveItemDeltas()}
                  pendingUserInputs={pendingUserInputs()}
                  resolvingApprovalId={resolvingApprovalId()}
                  resolvingUserInputId={resolvingUserInputId()}
                  acceptingPlanId={acceptingPlanId()}
                  cancelling={cancellingRun()}
                  resolvingRecoveryId={resolvingRecoveryId()}
                  onResolveApproval={(approval, decision) =>
                    void resolveApproval(approval, decision)
                  }
                  onResolveUserInput={(request, answers, action) =>
                    void resolveUserInput(request, answers, action)
                  }
                  onAcceptPlan={(plan) => void acceptPlan(plan)}
                  onCancel={(run) => void cancelRun(run)}
                  onResolveRecovery={(recovery, action) => void resolveRecovery(recovery, action)}
                />
                <Show when={snapshot().session.context.kind === "project"}>
                  <div class="session-workspace-tools">
                    <ReviewPanel
                      snapshot={snapshot()}
                      commandPort={commandPort}
                      onOpenSession={(session) => {
                        setSessions((current) => [
                          session,
                          ...current.filter((candidate) => candidate.id !== session.id),
                        ]);
                        selectSession(session);
                      }}
                    />
                    <WorkspaceBrowser
                      snapshot={snapshot()}
                      commandPort={commandPort}
                      gitRemoteMutationsEnabled={props.gitRemoteMutationsEnabled}
                    />
                    <TerminalPanel snapshot={snapshot()} commandPort={commandPort} />
                  </div>
                </Show>
              </div>
            )}
          </Show>
          <div class="composer-wrap">
            <SandboxReadinessBanner
              commandPort={commandPort}
              initialReport={agentControl()?.sandbox}
              onFailure={setFailure}
            />
            <Show when={behaviorMode() === "plan"}>
              <div class="plan-mode-banner composer-notice" role="status">
                <ShieldCheck size={15} />
                <span>{i18n.t("workbench.planBanner")}</span>
              </div>
            </Show>
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
            <Show when={draftProjectId() && selectedProject()}>
              <div
                class="composer-draft-state"
                data-testid="workbench-project-task-draft"
                role="status"
              >
                {i18n.locale() === "zh-CN" ? "新任务" : "New task"} ·{" "}
                {selectedProject()!.displayName}
              </div>
            </Show>
            <Composer class="composer">
              <input
                ref={(element) => (attachmentFileInput = element)}
                class="composer-attachment-input"
                data-component="file-input"
                data-testid="workbench-attachment-file-input"
                type="file"
                multiple
                onChange={(event) => {
                  addSelectedFiles(event.currentTarget.files);
                  event.currentTarget.value = "";
                }}
              />
              <input
                ref={(element) => {
                  attachmentFolderInput = element;
                  element.setAttribute("webkitdirectory", "");
                }}
                class="composer-attachment-input"
                data-component="file-input"
                data-testid="workbench-attachment-folder-input"
                type="file"
                multiple
                onChange={(event) => {
                  addSelectedFolder(event.currentTarget.files);
                  event.currentTarget.value = "";
                }}
              />
              <Show when={failure()}>{(message) => <p class="composer-error">{message()}</p>}</Show>
              <ComposerAttachmentList attachments={attachments()} onRemove={removeAttachment} />
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
                  <ApprovalPolicyPopover
                    activePopover={activePopover()}
                    onOpenChange={updatePopover}
                    value={approvalPolicy()}
                    onChange={setApprovalPolicy}
                  />
                </div>
                <Button
                  class="composer-send"
                  type="button"
                  data-testid="workbench-start-task"
                  disabled={
                    submitting() ||
                    !props.workspaceToolsEnabled ||
                    (!selectedSessionId() &&
                      Boolean(selectedProject()) &&
                      projectGit.executionKind() === "managed_worktree" &&
                      !projectGit.baseRevision()) ||
                    !draft().trim()
                  }
                  title={
                    props.workspaceToolsEnabled
                      ? i18n.t("workbench.startTask")
                      : i18n.t("workbench.workspaceToolsDisabled")
                  }
                  onClick={() => void startTask()}
                >
                  <Send size={16} />
                </Button>
              </div>
              <p class="composer-capability-note">
                {latestRun() || taskSnapshot()
                  ? i18n
                      .t("workbench.taskQueued")
                      .replace("{status}", latestRun()?.status ?? taskSnapshot()!.run.status)
                  : props.workspaceToolsEnabled
                    ? i18n.t("workbench.taskReady")
                    : i18n.t("workbench.workspaceToolsDisabled")}
              </p>
            </Composer>
          </div>
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

function fileSourceKey(file: File): string {
  return [file.webkitRelativePath, file.name, file.size, file.lastModified].join(":");
}

function createFileAttachmentPreview(file: File): ComposerAttachmentPreview {
  const previewUrl =
    file.type.startsWith("image/") && typeof URL.createObjectURL === "function"
      ? URL.createObjectURL(file)
      : undefined;
  return {
    id: crypto.randomUUID(),
    sourceKey: fileSourceKey(file),
    kind: "file",
    name: file.name,
    mimeType: file.type || "application/octet-stream",
    byteSize: file.size,
    fileCount: 1,
    ...(previewUrl ? { previewUrl } : {}),
  };
}

function revokeAttachmentPreview(attachment: ComposerAttachmentPreview) {
  if (attachment.previewUrl && typeof URL.revokeObjectURL === "function") {
    URL.revokeObjectURL(attachment.previewUrl);
  }
}

function sessionProjectId(session: SessionRecord): string | undefined {
  return session.context.kind === "project" ? session.context.project_id : undefined;
}

function isTerminalRunStatus(status: RunRecord["status"]): boolean {
  return ["succeeded", "failed", "timed_out", "cancelled", "interrupted", "lost"].includes(status);
}
