import {
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
import {
  AgentMessage,
  ApprovalCard,
  Archive,
  Badge,
  Bot,
  Box,
  Button,
  CalendarClock,
  Check,
  ChevronDown,
  Composer,
  ComposerInput,
  Dialog,
  Dropdown,
  ExternalLink,
  FolderOpen,
  GitBranch,
  GitFork,
  MessageCircle,
  MoreHorizontal,
  Palette,
  PanelLeftClose,
  PlanCard,
  Play,
  Plus,
  PromptCard,
  Search,
  SearchField,
  SelectField,
  Send,
  Settings,
  ShieldCheck,
  Sidebar,
  Square,
  TerminalSquare,
  TextField,
  Trash2,
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
import { ReviewPanel } from "./review-panel";
import "./composer-attachments.css";
import "./composer-popovers.css";

type ProjectMenuAction =
  | "pin"
  | "open"
  | "create_permanent_worktree"
  | "rename"
  | "mark_read"
  | "archive_tasks"
  | "remove";

function ProjectSidebar(props: {
  openSettings: () => void;
  openMotionLab: () => void;
  motionLabEnabled: boolean;
  schedulerEnabled: boolean;
  onNewTask: (projectId?: string) => void;
  onOpenTasks: () => void;
  activeView: "agent" | "tasks";
  projects: ProjectRecord[];
  sessions: SessionRecord[];
  selectedProjectId: string | undefined;
  selectedSessionId: string | undefined;
  pinnedProjectIds: readonly string[];
  unreadSessionIds: ReadonlySet<string>;
  loading: boolean;
  addingProject: boolean;
  onAddProject: () => void;
  onSelectProject: (projectId: string) => void;
  onSelectSession: (session: SessionRecord) => void;
  onProjectAction: (project: ProjectRecord, action: ProjectMenuAction) => void;
}) {
  const i18n = useI18n();
  const [search, setSearch] = createSignal("");
  const [searchOpen, setSearchOpen] = createSignal(false);
  const [expandedProjectIds, setExpandedProjectIds] = createSignal<string[]>([]);
  createEffect(() => {
    const projectIds = props.projects.map((project) => project.id);
    setExpandedProjectIds((current) => [
      ...current.filter((id) => projectIds.includes(id)),
      ...projectIds.filter((id) => !current.includes(id)),
    ]);
  });
  const filteredProjects = createMemo(() => {
    const query = search().trim().toLocaleLowerCase();
    if (!query) return props.projects;
    return props.projects.filter(
      (project) =>
        project.displayName.toLocaleLowerCase().includes(query) ||
        props.sessions.some(
          (session) =>
            sessionProjectId(session) === project.id &&
            session.title.toLocaleLowerCase().includes(query),
        ),
    );
  });
  const visibleSessions = (projectId: string) => {
    const query = search().trim().toLocaleLowerCase();
    return props.sessions.filter(
      (session) =>
        !session.archived &&
        sessionProjectId(session) === projectId &&
        (!query || session.title.toLocaleLowerCase().includes(query)),
    );
  };
  const toggleProject = (projectId: string) => {
    props.onSelectProject(projectId);
    setExpandedProjectIds((current) =>
      current.includes(projectId)
        ? current.filter((id) => id !== projectId)
        : [...current, projectId],
    );
  };
  return (
    <Sidebar class="project-sidebar">
      <div class="project-sidebar-brand">
        <span class="hachimi-mini-mark">H</span>
        <strong>Hachimi</strong>
        <Button
          type="button"
          aria-label={i18n.t("settings.search")}
          aria-expanded={searchOpen()}
          onClick={() => setSearchOpen((value) => !value)}
        >
          <Search size={17} />
        </Button>
      </div>
      <Show when={searchOpen()}>
        <SearchField
          label={i18n.t("settings.search")}
          placeholder={i18n.t("settings.search")}
          value={search()}
          onInput={(event) => setSearch(event.currentTarget.value)}
        />
      </Show>
      <nav class="project-quick-nav" aria-label={i18n.t("workbench.home")}>
        <Button
          type="button"
          classList={{ active: props.activeView === "agent" }}
          data-testid="workbench-new-task"
          onClick={() => props.onNewTask()}
        >
          <Plus size={17} /> <span>{i18n.t("workbench.newTask")}</span>
        </Button>
        <Show when={props.schedulerEnabled}>
          <Button
            type="button"
            classList={{ active: props.activeView === "tasks" }}
            data-testid="workbench-task-tab"
            onClick={() => props.onOpenTasks()}
          >
            <CalendarClock size={17} />
            <span>{i18n.locale() === "zh-CN" ? "任务" : "Tasks"}</span>
          </Button>
        </Show>
        <Show when={props.motionLabEnabled}>
          <Button type="button" onClick={() => props.openMotionLab()}>
            <Play size={17} />{" "}
            <span>{i18n.locale() === "zh-CN" ? "动作库实验室" : "Motion Library Lab"}</span>
          </Button>
        </Show>
      </nav>
      <div class="project-sidebar-scroll">
        <section class="project-list-section">
          <div class="project-list-heading">
            <h2>{i18n.t("workbench.projects")}</h2>
            <Button
              type="button"
              data-testid="workbench-add-project"
              aria-label={i18n.t("workbench.addProject")}
              title={i18n.t("workbench.addProject")}
              disabled={props.addingProject}
              onClick={() => props.onAddProject()}
            >
              <Plus size={14} />
            </Button>
          </div>
          <Show
            when={filteredProjects().length > 0}
            fallback={
              <p class="project-empty">
                {props.loading
                  ? i18n.t("workbench.loadingProjects")
                  : i18n.t("workbench.noProjects")}
              </p>
            }
          >
            <For each={filteredProjects()}>
              {(project) => {
                const expanded = () => expandedProjectIds().includes(project.id);
                return (
                  <>
                    <div class="project-row-shell">
                      <Button
                        type="button"
                        class="project-row"
                        classList={{ selected: props.selectedProjectId === project.id }}
                        aria-expanded={expanded()}
                        title={project.displayName}
                        onClick={() => toggleProject(project.id)}
                      >
                        <span class="project-row-main">
                          <FolderOpen size={16} />
                          <span class="project-row-name">{project.displayName}</span>
                        </span>
                        <ChevronDown
                          size={15}
                          class="project-chevron"
                          classList={{ collapsed: !expanded() }}
                        />
                      </Button>
                      <div class="project-row-actions">
                        <Dropdown
                          label={
                            i18n.locale() === "zh-CN"
                              ? `${project.displayName} 项目操作`
                              : `${project.displayName} project actions`
                          }
                          triggerTestId={`project-more-${project.id}`}
                          actions={[
                            {
                              id: "pin",
                              label: props.pinnedProjectIds.includes(project.id)
                                ? i18n.locale() === "zh-CN"
                                  ? "取消置顶项目"
                                  : "Unpin project"
                                : i18n.locale() === "zh-CN"
                                  ? "置顶项目"
                                  : "Pin project",
                              icon: <GitBranch size={16} />,
                            },
                            {
                              id: "open",
                              label:
                                i18n.locale() === "zh-CN"
                                  ? "在资源管理器中打开"
                                  : "Open in file explorer",
                              icon: <ExternalLink size={16} />,
                            },
                            {
                              id: "create_permanent_worktree",
                              label:
                                i18n.locale() === "zh-CN"
                                  ? "创建永久工作树"
                                  : "Create permanent worktree",
                              icon: <GitFork size={16} />,
                              disabled: !project.gitRoot,
                            },
                            {
                              id: "rename",
                              label: i18n.locale() === "zh-CN" ? "重命名项目" : "Rename project",
                              icon: <MoreHorizontal size={16} />,
                            },
                            {
                              id: "mark_read",
                              label: i18n.locale() === "zh-CN" ? "全部标为已读" : "Mark all read",
                              icon: <Check size={16} />,
                            },
                            {
                              id: "archive_tasks",
                              label: i18n.locale() === "zh-CN" ? "归档任务" : "Archive tasks",
                              icon: <Archive size={16} />,
                              disabled: visibleSessions(project.id).length === 0,
                            },
                            {
                              id: "remove",
                              label: i18n.locale() === "zh-CN" ? "移除" : "Remove",
                              icon: <Trash2 size={16} />,
                              danger: true,
                              separatorBefore: true,
                            },
                          ]}
                          onSelect={(action) =>
                            props.onProjectAction(project, action as ProjectMenuAction)
                          }
                        >
                          <MoreHorizontal size={16} />
                        </Dropdown>
                        <Button
                          type="button"
                          class="project-new-task"
                          data-testid={`project-new-task-${project.id}`}
                          aria-label={
                            i18n.locale() === "zh-CN"
                              ? `在 ${project.displayName} 中新建任务`
                              : `New task in ${project.displayName}`
                          }
                          title={i18n.locale() === "zh-CN" ? "新建任务" : "New task"}
                          onClick={() => props.onNewTask(project.id)}
                        >
                          <Plus size={16} />
                        </Button>
                      </div>
                    </div>
                    <Show when={expanded()}>
                      <div class="project-sessions">
                        <For each={visibleSessions(project.id)}>
                          {(session) => (
                            <Button
                              type="button"
                              classList={{ selected: props.selectedSessionId === session.id }}
                              onClick={() => props.onSelectSession(session)}
                            >
                              <MessageCircle size={14} />
                              <span>{session.title}</span>
                              <Show when={props.unreadSessionIds.has(session.id)}>
                                <i class="session-unread-dot" aria-label="Unread" />
                              </Show>
                            </Button>
                          )}
                        </For>
                        <Show when={visibleSessions(project.id).length === 0}>
                          <p class="session-empty">{i18n.t("workbench.noSessions")}</p>
                        </Show>
                      </div>
                    </Show>
                  </>
                );
              }}
            </For>
          </Show>
        </section>
        <Show
          when={props.sessions.some(
            (session) => session.context.kind === "general" && !session.archived,
          )}
        >
          <section class="project-list-section">
            <div class="project-list-heading">
              <h2>{i18n.locale() === "zh-CN" ? "通用会话" : "General sessions"}</h2>
            </div>
            <div class="project-sessions general-sessions">
              <For
                each={props.sessions.filter(
                  (session) => session.context.kind === "general" && !session.archived,
                )}
              >
                {(session) => (
                  <Button
                    type="button"
                    classList={{ selected: props.selectedSessionId === session.id }}
                    onClick={() => props.onSelectSession(session)}
                  >
                    <MessageCircle size={14} />
                    <span>{session.title}</span>
                  </Button>
                )}
              </For>
            </div>
          </section>
        </Show>
      </div>
      <Button
        type="button"
        class="sidebar-account"
        data-testid="workbench-open-settings"
        aria-label={i18n.t("settings.title")}
        onClick={() => props.openSettings()}
      >
        <span class="account-avatar">M</span>
        <span>
          <strong>my_codex</strong>
          <small>{i18n.t("settings.title")}</small>
        </span>
        <Settings size={17} />
      </Button>
    </Sidebar>
  );
}

function UserInputCard(props: {
  request: UserInputRequestRecord;
  resolving: boolean;
  onResolve: (
    request: UserInputRequestRecord,
    answers: UserInputAnswer[],
    action: UserInputResolutionAction,
  ) => void;
}) {
  const i18n = useI18n();
  const [answers, setAnswers] = createSignal<Record<string, string>>(
    untrack(() =>
      Object.fromEntries(
        props.request.questions.map((question) => [
          question.id,
          question.defaultAnswer ?? question.options[0]?.value ?? "",
        ]),
      ),
    ),
  );
  const complete = () =>
    props.request.questions.every((question) => (answers()[question.id] ?? "").trim().length > 0);
  return (
    <article class="user-input-card agent-card approval" data-component="user-input-card">
      <header>
        <MessageCircle size={17} />
        <span>
          <strong>{i18n.locale() === "zh-CN" ? "需要你的输入" : "Your input is needed"}</strong>
          <small>
            {i18n.locale() === "zh-CN"
              ? "回答只会交给当前运行；密钥类回答不会写入历史"
              : "Answers go only to the active run; secret answers are never persisted"}
          </small>
        </span>
      </header>
      <For each={props.request.questions}>
        {(question) => (
          <div class="user-input-question">
            <Show when={question.options.length > 0}>
              <SelectField
                label={question.header}
                description={question.prompt}
                value={answers()[question.id] ?? ""}
                options={[
                  ...question.options.map((option) => ({
                    value: option.value,
                    label: option.label,
                  })),
                  {
                    value: "",
                    label: i18n.locale() === "zh-CN" ? "自由输入…" : "Free-form answer…",
                  },
                ]}
                onChange={(value) =>
                  setAnswers((current) => ({
                    ...current,
                    [question.id]: value,
                  }))
                }
              />
            </Show>
            <TextField
              label={question.options.length > 0 ? question.prompt : question.header}
              {...(question.options.length > 0 ? {} : { description: question.prompt })}
              type={question.secret ? "password" : "text"}
              value={answers()[question.id] ?? ""}
              placeholder={i18n.locale() === "zh-CN" ? "输入回答" : "Enter an answer"}
              onInput={(event) =>
                setAnswers((current) => ({
                  ...current,
                  [question.id]: event.currentTarget.value,
                }))
              }
            />
          </div>
        )}
      </For>
      <footer>
        <Button
          size="small"
          variant="ghost"
          data-testid="workbench-decline-user-input"
          disabled={props.resolving}
          onClick={() => props.onResolve(props.request, [], "decline")}
        >
          {i18n.locale() === "zh-CN" ? "拒绝提供" : "Decline"}
        </Button>
        <Button
          size="small"
          variant="ghost"
          data-testid="workbench-cancel-user-input"
          disabled={props.resolving}
          onClick={() => props.onResolve(props.request, [], "cancel")}
        >
          {i18n.locale() === "zh-CN" ? "取消请求" : "Cancel request"}
        </Button>
        <Button
          size="small"
          variant="primary"
          data-testid="workbench-submit-user-input"
          disabled={props.resolving || !complete()}
          onClick={() =>
            props.onResolve(
              props.request,
              props.request.questions.map((question) => ({
                questionId: question.id,
                value: answers()[question.id] ?? "",
              })),
              "submit",
            )
          }
        >
          {i18n.locale() === "zh-CN" ? "提交回答" : "Submit answers"}
        </Button>
      </footer>
    </article>
  );
}

function SessionTimeline(props: {
  snapshot: WorkbenchSessionSnapshot;
  liveItemDeltas: LiveItemDeltas;
  pendingUserInputs: UserInputRequestRecord[];
  resolvingApprovalId: string | undefined;
  resolvingUserInputId: string | undefined;
  acceptingPlanId: string | undefined;
  cancelling: boolean;
  onResolveApproval: (approval: ApprovalRequestRecord, decision: ApprovalStatus) => void;
  onAcceptPlan: (plan: ProposedPlan) => void;
  onResolveUserInput: (
    request: UserInputRequestRecord,
    answers: UserInputAnswer[],
    action: UserInputResolutionAction,
  ) => void;
  onCancel: (run: RunRecord) => void;
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
              <pre>
                {timelineItemText(
                  item.payload,
                  item.status === "in_progress"
                    ? liveItemText(props.liveItemDeltas[item.id])
                    : undefined,
                )}
              </pre>
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
  const [agentControl, setAgentControl] = createSignal<ControlInitializeResponse>();
  const [resolvingApprovalId, setResolvingApprovalId] = createSignal<string>();
  const [resolvingUserInputId, setResolvingUserInputId] = createSignal<string>();
  const [acceptingPlanId, setAcceptingPlanId] = createSignal<string>();
  const [cancellingRun, setCancellingRun] = createSignal(false);
  const [projectActionBusy, setProjectActionBusy] = createSignal(false);
  const [renameTarget, setRenameTarget] = createSignal<ProjectRecord>();
  const [renameDraft, setRenameDraft] = createSignal("");
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

  onMount(() => {
    let disposed = false;
    void commandPort
      .initializeAgentControl({
        clientVersion: "hachimi-desktop/0.2.0",
        protocolVersion: 18,
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
    if (!sessionId) {
      setSessionSnapshot(undefined);
      setLiveItemDeltas({});
      setPendingUserInputs([]);
      return;
    }
    let disposed = false;
    let subscriptionId: string | undefined;
    let lastSequence = 0;
    let stopEvents: (() => void) | undefined;
    const loadProjection = async () => {
      try {
        const [snapshot, resume] = await Promise.all([
          commandPort.getWorkbenchSession(sessionId),
          commandPort.resumeAgentSession({
            sessionId,
            metadataOnly: true,
            transcriptBeforeSequence: null,
            transcriptLimit: 0,
          }),
        ]);
        if (!disposed && untrack(selectedSessionId) === sessionId) {
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
    setDraftProjectId(undefined);
    setTaskSnapshot(undefined);
    setFailure(undefined);
    setReadSessions((current) => ({ ...current, [session.id]: session.updatedAtMs }));
  }

  function newTask(projectId?: string) {
    setActiveView("agent");
    const nextProjectId = projectId ?? selectedProjectId();
    if (nextProjectId) setSelectedProjectId(nextProjectId);
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
            context: {
              requestId: crypto.randomUUID(),
              clientId: "window:workbench",
              protocolVersion: 18,
              idempotencyKey: crypto.randomUUID(),
              expectedRunId: null,
              expectedGeneration: null,
            },
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
    if (!project) {
      setFailure(i18n.t("workbench.projectRequired"));
      return;
    }
    if (!prompt) {
      setFailure(i18n.t("workbench.promptRequired"));
      return;
    }
    if (projectGit.executionKind() === "managed_worktree" && !projectGit.baseRevision()) {
      setFailure(i18n.t("workbench.branchRequired"));
      return;
    }
    setSubmitting(true);
    setFailure(undefined);
    try {
      const projectId = project.id;
      const snapshot = await commandPort.startWorkbenchTask({
        idempotencyKey: crypto.randomUUID(),
        projectId,
        prompt,
        executionTarget:
          projectGit.executionKind() === "managed_worktree"
            ? {
                kind: "managed_worktree",
                project_id: projectId,
                base_revision: projectGit.baseRevision(),
              }
            : { kind: "local", project_id: projectId },
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
        loading={loading()}
        addingProject={addingProject()}
        onAddProject={() => void addProject()}
        onSelectProject={selectProject}
        onSelectSession={selectSession}
        onProjectAction={(project, action) => void handleProjectAction(project, action)}
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
              const session = sessions().find((candidate) => candidate.id === sessionId);
              setActiveView("agent");
              setSelectedProjectId(session ? sessionProjectId(session) : undefined);
              setSelectedSessionId(sessionId);
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
                  liveItemDeltas={liveItemDeltas()}
                  pendingUserInputs={pendingUserInputs()}
                  resolvingApprovalId={resolvingApprovalId()}
                  resolvingUserInputId={resolvingUserInputId()}
                  acceptingPlanId={acceptingPlanId()}
                  cancelling={cancellingRun()}
                  onResolveApproval={(approval, decision) =>
                    void resolveApproval(approval, decision)
                  }
                  onResolveUserInput={(request, answers, action) =>
                    void resolveUserInput(request, answers, action)
                  }
                  onAcceptPlan={(plan) => void acceptPlan(plan)}
                  onCancel={(run) => void cancelRun(run)}
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
                    <WorkspaceBrowser snapshot={snapshot()} commandPort={commandPort} />
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
                    !selectedProject() ||
                    (projectGit.executionKind() === "managed_worktree" &&
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
