import {
  commandFailure,
  type BrowserCapability,
  type ConnectorAccount,
  type ConnectorDriverDescriptor,
  type DeliveryPolicy,
  type GitRefRecord,
  type McpServerView,
  type McpToolSelection,
  type McpToolView,
  type ProjectGitSnapshot,
  type ProjectRecord,
  type ScheduleDefinition,
  type ScheduleBrowserGrant,
  type ScheduleConnectorSelection,
  type ScheduleEventReceipt,
  type ScheduleEventSourceKind,
  type ScheduleSpec,
  type SessionRecord,
  type SkillRecord,
  type TaskRunRecord,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import {
  AlertTriangle,
  Button,
  ChevronDown,
  Checkbox,
  Dialog,
  PageHeading,
  Plus,
  ShieldCheck,
  SelectField,
  TextArea,
  TextField,
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

import type { WorkbenchCommandPort } from "./workbench-command-port";
import { directUserMutationContext } from "./mutation-context";
import { TaskCard } from "./task-card";
import { TaskHistoryDialog } from "./task-history-dialog";
import { TaskEventForm } from "./task-event-form";
import { RuntimeHealthBanner } from "./runtime-health";
import "./task-center.css";

type ScheduleFrequency = "once" | "daily" | "weekly" | "cron" | "event";
type ScheduleContext = "general" | "project" | "session_continuation";

type TaskMcpTool = {
  server: McpServerView;
  tool: McpToolView;
  readOnly: boolean;
};

type TaskConnector = {
  account: ConnectorAccount;
  descriptor: ConnectorDriverDescriptor;
  contentHash: string;
};

const BROWSER_CAPABILITIES: BrowserCapability[] = [
  "observe",
  "act",
  "download",
  "cookie_storage",
  "cdp",
];

const CORE_READ_TOOLS = [
  "workspace_read_file",
  "workspace_list_directory",
  "workspace_search_text",
  "workspace_git_status",
  "workspace_git_diff",
];

export function TaskCenter(props: {
  commandPort: WorkbenchCommandPort;
  projects: ProjectRecord[];
  skills: SkillRecord[];
  onOpenSession: (sessionId: string) => void;
}) {
  const i18n = useI18n();
  // eslint-disable-next-line solid/reactivity -- commandPort is an immutable injected service.
  const commandPort = props.commandPort;
  const zh = () => i18n.locale() === "zh-CN";
  const [schedules, setSchedules] = createSignal<ScheduleDefinition[]>([]);
  const [taskRuns, setTaskRuns] = createSignal<TaskRunRecord[]>([]);
  const [eventReceipts, setEventReceipts] = createSignal<ScheduleEventReceipt[]>([]);
  const [selectedScheduleId, setSelectedScheduleId] = createSignal<string>();
  const [removeScheduleId, setRemoveScheduleId] = createSignal<string>();
  const [showCreate, setShowCreate] = createSignal(false);
  const [editingScheduleId, setEditingScheduleId] = createSignal<string>();
  const [loading, setLoading] = createSignal(true);
  const [submitting, setSubmitting] = createSignal(false);
  const [busyId, setBusyId] = createSignal<string>();
  const [failure, setFailure] = createSignal<string>();
  const [nameError, setNameError] = createSignal<string>();
  const [promptError, setPromptError] = createSignal<string>();
  const [advancedOpen, setAdvancedOpen] = createSignal(false);
  const [name, setName] = createSignal(zh() ? "每日任务" : "Daily task");
  const [prompt, setPrompt] = createSignal("");
  const [frequency, setFrequency] = createSignal<ScheduleFrequency>("daily");
  const [runAt, setRunAt] = createSignal(defaultDateTime());
  const [cron, setCron] = createSignal("0 9 * * *");
  const [eventSourceKind, setEventSourceKind] = createSignal<ScheduleEventSourceKind>("workspace");
  const [eventSourcePrincipal, setEventSourcePrincipal] = createSignal("");
  const [eventSourceId, setEventSourceId] = createSignal("");
  const [eventType, setEventType] = createSignal("");
  const [eventSubjectPrefix, setEventSubjectPrefix] = createSignal("");
  const [eventLabels, setEventLabels] = createSignal("");
  const [eventResourceKind, setEventResourceKind] = createSignal("");
  const [eventResourceId, setEventResourceId] = createSignal("");
  const [eventResourceRevision, setEventResourceRevision] = createSignal("");
  const [deliveryPolicy, setDeliveryPolicy] = createSignal<DeliveryPolicy>("task_tab_only");
  const [contextKind, setContextKind] = createSignal<ScheduleContext>("general");
  const [sessions, setSessions] = createSignal<SessionRecord[]>([]);
  const [sessionId, setSessionId] = createSignal<string>();
  const [profile, setProfile] = createSignal<"" | "office" | "coding">("");
  const [projectId, setProjectId] = createSignal<string>();
  const [executionKind, setExecutionKind] = createSignal<"local" | "managed_worktree">("local");
  const [gitRefs, setGitRefs] = createSignal<GitRefRecord[]>([]);
  const [projectGit, setProjectGit] = createSignal<ProjectGitSnapshot>();
  const [baseRevision, setBaseRevision] = createSignal("");
  const [allowWrite, setAllowWrite] = createSignal(false);
  const [allowExec, setAllowExec] = createSignal(false);
  const [selectedSkillIds, setSelectedSkillIds] = createSignal<string[]>([]);
  const [mcpTools, setMcpTools] = createSignal<TaskMcpTool[]>([]);
  const [selectedMcpTools, setSelectedMcpTools] = createSignal<McpToolSelection[]>([]);
  const [connectors, setConnectors] = createSignal<TaskConnector[]>([]);
  const [selectedConnectorActions, setSelectedConnectorActions] = createSignal<
    Record<string, string[]>
  >({});
  const [browserUnattended, setBrowserUnattended] = createSignal(false);
  const [browserDocumentOrigins, setBrowserDocumentOrigins] = createSignal("");
  const [browserResourceOrigins, setBrowserResourceOrigins] = createSignal("");
  const [browserCapabilities, setBrowserCapabilities] = createSignal<BrowserCapability[]>([
    "observe",
    "act",
  ]);
  const [browserPrivateNetwork, setBrowserPrivateNetwork] = createSignal(false);
  const [maxOccurrences, setMaxOccurrences] = createSignal("");
  const [endAt, setEndAt] = createSignal("");
  const [stopAfterSuccess, setStopAfterSuccess] = createSignal(false);
  const selectedSchedule = createMemo(() =>
    schedules().find((schedule) => schedule.id === selectedScheduleId()),
  );
  const selectedRuns = createMemo(() =>
    taskRuns().filter((task) => task.scheduleId === selectedScheduleId()),
  );
  const selectedEvents = createMemo(() =>
    eventReceipts().filter((receipt) =>
      receipt.matchedScheduleIds.includes(selectedScheduleId() ?? ""),
    ),
  );
  const projectGitDescription = createMemo(() => {
    const state = projectGit()?.state;
    if (state?.kind !== "unborn") return "";
    return zh()
      ? `${state.branch} 尚无提交，请先在工作台创建空首提。`
      : `${state.branch} has no commits; create the empty initial commit in Workbench first.`;
  });

  async function refresh() {
    try {
      const [nextSchedules, nextRuns, nextEvents, sessionPage] = await Promise.all([
        props.commandPort.listSchedules(),
        props.commandPort.listTaskRuns(null, 200),
        Promise.resolve(props.commandPort.listScheduleEventReceipts?.(100) ?? []),
        props.commandPort.searchAgentSessions({
          projectId: null,
          query: null,
          archived: false,
          before: null,
          limit: 200,
        }),
      ]);
      setSchedules(nextSchedules);
      setTaskRuns(nextRuns);
      setEventReceipts(nextEvents);
      const eligibleSessions = sessionPage.items.filter(
        (session) => session.entryProfile === "workbench",
      );
      setSessions(eligibleSessions);
      setSessionId((current) =>
        current && eligibleSessions.some((session) => session.id === current)
          ? current
          : eligibleSessions[0]?.id,
      );
      setSelectedScheduleId((current) =>
        current && nextSchedules.some((schedule) => schedule.id === current) ? current : undefined,
      );
      setRemoveScheduleId((current) =>
        current && nextSchedules.some((schedule) => schedule.id === current) ? current : undefined,
      );
      setFailure(undefined);
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setLoading(false);
    }
  }

  async function refreshMcpTools() {
    const servers = (await commandPort.listMcpServers()).filter(
      (server) => server.configuration.enabled && server.health.state === "ready",
    );
    const tools = await Promise.all(
      servers.map(async (server) =>
        (await commandPort.listMcpTools(server.configuration.id))
          .filter(
            (tool) =>
              tool.enabled && !tool.stale && !tool.validationError && tool.schemaHash.length > 0,
          )
          .map((tool) => ({
            server,
            tool,
            readOnly: server.configuration.readOnlyTools.includes(tool.name),
          })),
      ),
    );
    setMcpTools(tools.flat());
  }

  async function refreshConnectors() {
    const [accounts, plugins] = await Promise.all([
      commandPort.listConnectorAccounts(),
      commandPort.listPlugins(),
    ]);
    const activeAccounts = accounts.filter((account) => account.health === "healthy");
    const catalog = await Promise.all(
      activeAccounts.map(async (account) => {
        const descriptor = await commandPort.getConnectorDriverDescriptor(
          account.pluginId,
          account.connectorId,
        );
        const plugin = plugins.find((value) => value.manifest.id === account.pluginId);
        if (!plugin) return undefined;
        return {
          account,
          descriptor,
          contentHash: plugin.contentHash,
        } satisfies TaskConnector;
      }),
    );
    setConnectors(catalog.filter((entry): entry is TaskConnector => Boolean(entry)));
  }

  onMount(() => {
    void refresh();
    void refreshMcpTools().catch((error) => setFailure(commandFailure(error).message));
    void refreshConnectors().catch((error) => setFailure(commandFailure(error).message));
    // eslint-disable-next-line solid/reactivity -- polling intentionally reads the latest signals.
    const timer = window.setInterval(() => void refresh(), 3_000);
    onCleanup(() => window.clearInterval(timer));
  });

  async function loadProjectGitState(nextProjectId: string) {
    try {
      const snapshot = await props.commandPort.inspectProjectGit(nextProjectId);
      if (untrack(projectId) !== nextProjectId) return;
      setProjectGit(snapshot);
      if (snapshot.state.kind !== "ready") {
        setExecutionKind("local");
        setGitRefs([]);
        setBaseRevision("");
        return;
      }
      const refs = await props.commandPort.listProjectGitRefs(nextProjectId);
      if (untrack(projectId) !== nextProjectId) return;
      setGitRefs(refs);
      const preferred = refs.find((reference) => reference.current) ?? refs[0];
      setBaseRevision((current) =>
        refs.some((reference) => reference.name === current) ? current : (preferred?.name ?? ""),
      );
    } catch (error) {
      setFailure(commandFailure(error).message);
    }
  }

  createEffect(() => {
    const nextProjectId = projectId() ?? props.projects[0]?.id;
    if (!projectId() && nextProjectId) setProjectId(nextProjectId);
    if (contextKind() !== "project" || !nextProjectId) {
      setGitRefs([]);
      setProjectGit(undefined);
      return;
    }
    void loadProjectGitState(nextProjectId);
  });

  function resetForm() {
    setEditingScheduleId(undefined);
    setName(zh() ? "每日任务" : "Daily task");
    setPrompt("");
    setFrequency("daily");
    setRunAt(defaultDateTime());
    setCron("0 9 * * *");
    setEventSourceKind("workspace");
    setEventSourcePrincipal("");
    setEventSourceId("");
    setEventType("");
    setEventSubjectPrefix("");
    setEventLabels("");
    setEventResourceKind("");
    setEventResourceId("");
    setEventResourceRevision("");
    setDeliveryPolicy("task_tab_only");
    setContextKind("general");
    setProfile("");
    setExecutionKind("local");
    setAllowWrite(false);
    setAllowExec(false);
    setSelectedSkillIds([]);
    setSelectedMcpTools([]);
    setSelectedConnectorActions({});
    setBrowserUnattended(false);
    setBrowserDocumentOrigins("");
    setBrowserResourceOrigins("");
    setBrowserCapabilities(["observe", "act"]);
    setBrowserPrivateNetwork(false);
    setMaxOccurrences("");
    setEndAt("");
    setStopAfterSuccess(false);
    setNameError(undefined);
    setPromptError(undefined);
    setAdvancedOpen(false);
  }

  async function submitSchedule() {
    if (!name().trim() || !prompt().trim()) {
      setNameError(!name().trim() ? (zh() ? "请输入任务名称。" : "Enter a task name.") : undefined);
      setPromptError(
        !prompt().trim() ? (zh() ? "请输入任务提示词。" : "Enter a task prompt.") : undefined,
      );
      return;
    }
    setNameError(undefined);
    setPromptError(undefined);
    setFailure(undefined);
    if (contextKind() === "project" && !projectId()) {
      setFailure(zh() ? "请选择项目。" : "Select a project.");
      setAdvancedOpen(true);
      return;
    }
    if (contextKind() === "session_continuation" && !sessionId()) {
      setFailure(zh() ? "请选择要续接的对话。" : "Select a Session to continue.");
      setAdvancedOpen(true);
      return;
    }
    if (executionKind() === "managed_worktree" && !baseRevision()) {
      setFailure(zh() ? "请选择 Worktree 基础分支。" : "Select a Worktree base branch.");
      setAdvancedOpen(true);
      return;
    }
    if (frequency() === "event") {
      if (!eventSourcePrincipal().trim() || !eventSourceId().trim() || !eventType().trim()) {
        setFailure(
          zh()
            ? "Event 任务需要来源 Principal、来源 ID 和事件类型。"
            : "Event tasks require a source principal, source ID, and event type.",
        );
        return;
      }
      if (Boolean(eventResourceKind().trim()) !== Boolean(eventResourceId().trim())) {
        setFailure(
          zh()
            ? "资源引用必须同时填写类型和 ID。"
            : "A resource reference requires both kind and ID.",
        );
        return;
      }
      try {
        parseEventLabels(eventLabels());
      } catch (error) {
        setFailure(error instanceof Error ? error.message : String(error));
        return;
      }
    }
    const parsedMax = Number.parseInt(maxOccurrences(), 10);
    if (maxOccurrences().trim() && (!Number.isFinite(parsedMax) || parsedMax < 1)) {
      setFailure(
        zh() ? "最大执行次数必须是正整数。" : "Maximum occurrences must be a positive integer.",
      );
      setAdvancedOpen(true);
      return;
    }
    if (endAt() && new Date(endAt()).getTime() <= Date.now()) {
      setFailure(zh() ? "截止时间必须晚于当前时间。" : "End time must be in the future.");
      setAdvancedOpen(true);
      return;
    }
    if (browserUnattended() && parseOrigins(browserDocumentOrigins()).length === 0) {
      setFailure(
        zh()
          ? "无人值守 Browser 至少需要一个精确的文档 Origin。"
          : "Unattended Browser requires at least one exact document origin.",
      );
      setAdvancedOpen(true);
      return;
    }
    setSubmitting(true);
    try {
      const draft = buildDefinition();
      const editing = schedules().find((schedule) => schedule.id === editingScheduleId());
      if (editing) {
        await updateExistingSchedule(editing, draft);
      } else {
        await props.commandPort.createSchedule({
          context: directUserMutationContext(),
          definition: draft,
          authorize: true,
        });
      }
      await refresh();
      setShowCreate(false);
      resetForm();
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setSubmitting(false);
    }
  }

  async function updateExistingSchedule(
    current: ScheduleDefinition,
    draft: ScheduleDefinition,
  ): Promise<{ definition: ScheduleDefinition }> {
    const definition: ScheduleDefinition = {
      ...current,
      name: draft.name,
      prompt: draft.prompt,
      schedule: draft.schedule,
      entryProfile: draft.entryProfile,
      workloadOverride: draft.workloadOverride,
      contextTemplate: draft.contextTemplate,
      toolAllowlist: draft.toolAllowlist,
      skillAllowlist: draft.skillAllowlist,
      mcpToolAllowlist: draft.mcpToolAllowlist,
      contributionRevisions: draft.contributionRevisions ?? [],
      hostGrant: draft.hostGrant ?? {
        connectors: [],
        browser: null,
        computerUnattended: false,
      },
      permissionConfig: draft.permissionConfig,
      deliveryPolicy: draft.deliveryPolicy,
      ...(draft.stopConditions ? { stopConditions: draft.stopConditions } : {}),
    };
    const updated = await props.commandPort.updateSchedule({
      context: directUserMutationContext(),
      definition,
      expectedConfigRevision: current.configRevision,
    });
    if (authorizationScopeChanged(current, updated)) {
      await props.commandPort.reauthorizeSchedule(directUserMutationContext(), updated.id);
    }
    return { definition: updated };
  }

  function beginEdit(schedule: ScheduleDefinition) {
    setFailure(undefined);
    setEditingScheduleId(schedule.id);
    setName(schedule.name);
    setPrompt(schedule.prompt);
    setDeliveryPolicy(schedule.deliveryPolicy);
    applyScheduleToForm(schedule.schedule);
    if (schedule.contextTemplate.kind === "project") {
      setContextKind("project");
      setProjectId(schedule.contextTemplate.project_id);
      setProfile(
        schedule.workloadOverride === "coding" || schedule.workloadOverride === "office"
          ? schedule.workloadOverride
          : "",
      );
      const target = schedule.contextTemplate.execution_target;
      setExecutionKind(target.kind);
      setBaseRevision(target.kind === "managed_worktree" ? target.base_revision : "");
    } else if (schedule.contextTemplate.kind === "session_continuation") {
      setContextKind("session_continuation");
      setSessionId(schedule.contextTemplate.session_id);
      setProfile(
        schedule.workloadOverride === "coding" || schedule.workloadOverride === "office"
          ? schedule.workloadOverride
          : "",
      );
      setExecutionKind("local");
    } else {
      setContextKind("general");
      setProfile(
        schedule.workloadOverride === "coding" || schedule.workloadOverride === "office"
          ? schedule.workloadOverride
          : "",
      );
      setExecutionKind("local");
    }
    setAllowWrite(schedule.permissionConfig.allowFileWrite);
    setAllowExec(schedule.permissionConfig.allowExec);
    setSelectedSkillIds([...schedule.skillAllowlist]);
    setSelectedMcpTools([...schedule.mcpToolAllowlist]);
    setSelectedConnectorActions(
      Object.fromEntries(
        (schedule.hostGrant?.connectors ?? []).map((selection) => [
          selection.accountId,
          [...selection.allowedActions],
        ]),
      ),
    );
    const browser = schedule.hostGrant?.browser;
    setBrowserUnattended(Boolean(browser?.enabled));
    setBrowserDocumentOrigins(browser?.documentOrigins.join("\n") ?? "");
    setBrowserResourceOrigins(browser?.resourceOrigins.join("\n") ?? "");
    setBrowserCapabilities(
      browser?.capabilities.length ? [...browser.capabilities] : ["observe", "act"],
    );
    setBrowserPrivateNetwork(Boolean(browser?.allowPrivateNetwork));
    setMaxOccurrences(schedule.stopConditions?.maxOccurrences?.toString() ?? "");
    setEndAt(
      schedule.stopConditions?.endAtMs ? toLocalDateTime(schedule.stopConditions.endAtMs) : "",
    );
    setStopAfterSuccess(schedule.stopConditions?.stopAfterSuccess ?? false);
    setShowCreate(true);
  }

  function applyScheduleToForm(schedule: ScheduleSpec) {
    if (schedule.kind === "event") {
      setFrequency("event");
      setEventSourceKind(schedule.matcher.source.kind);
      setEventSourcePrincipal(schedule.matcher.source.principal);
      setEventSourceId(schedule.matcher.source.id);
      setEventType(schedule.matcher.eventType);
      setEventSubjectPrefix(schedule.matcher.subjectPrefix ?? "");
      setEventLabels(formatEventLabels(schedule.matcher.labels));
      setEventResourceKind(schedule.matcher.resource?.kind ?? "");
      setEventResourceId(schedule.matcher.resource?.id ?? "");
      setEventResourceRevision(schedule.matcher.resource?.revision ?? "");
      return;
    }
    if (schedule.kind === "at") {
      setFrequency("once");
      setRunAt(toLocalDateTime(schedule.timestamp_ms));
      return;
    }
    if (schedule.kind === "every") {
      setFrequency(schedule.interval_ms >= 7 * 86_400_000 ? "weekly" : "daily");
      setRunAt(toLocalDateTime(schedule.anchor_ms));
      return;
    }
    setCron(schedule.expression);
    const weekly = /^0\s+(\d+)\s+(\d+)\s+\*\s+\*\s+(\d)$/.exec(schedule.expression);
    const daily = /^0\s+(\d+)\s+(\d+)\s+\*\s+\*\s+\*$/.exec(schedule.expression);
    if (weekly || daily) {
      setFrequency(weekly ? "weekly" : "daily");
      const match = weekly ?? daily!;
      const next = new Date();
      next.setHours(Number(match[2]), Number(match[1]), 0, 0);
      if (weekly) next.setDate(next.getDate() + ((Number(match[3]) - next.getDay() + 7) % 7));
      setRunAt(toLocalDateTime(next.getTime()));
    } else {
      setFrequency("cron");
    }
  }

  function buildDefinition(): ScheduleDefinition {
    const now = Date.now();
    const selectedProjectId = projectId();
    const projectContext = contextKind() === "project" && selectedProjectId;
    const continuationSessionId =
      contextKind() === "session_continuation" ? sessionId() : undefined;
    const projectSideEffects = Boolean(projectContext && (allowWrite() || allowExec()));
    const mcpToolAllowlist = [...selectedMcpTools()].sort((left, right) =>
      `${left.serverId}\0${left.toolName}\0${left.schemaHash}\0${left.hostIdentityHash}`.localeCompare(
        `${right.serverId}\0${right.toolName}\0${right.schemaHash}\0${right.hostIdentityHash}`,
      ),
    );
    const externalMcpTools = mcpToolAllowlist.filter((selection) => {
      const catalog = mcpTools().find(
        (entry) =>
          entry.server.configuration.id === selection.serverId &&
          entry.tool.name === selection.toolName &&
          entry.tool.schemaHash === selection.schemaHash &&
          entry.tool.hostIdentityHash === selection.hostIdentityHash,
      );
      return !catalog?.readOnly;
    });
    const connectorSelections = connectors()
      .map((entry): ScheduleConnectorSelection | undefined => {
        const allowedActions = selectedConnectorActions()[entry.account.id] ?? [];
        if (allowedActions.length === 0) return undefined;
        return {
          accountId: entry.account.id,
          contributionRevision: {
            pluginId: entry.account.pluginId,
            contributionId: entry.account.connectorId,
            accountId: entry.account.id,
            contentHash: entry.contentHash,
            hostIdentityHash: entry.descriptor.revision.hostIdentityHash,
            schemaHash: entry.descriptor.revision.schemaHash,
            actionHash: entry.descriptor.revision.actionHash,
          },
          allowedActions: [...allowedActions].sort(),
        };
      })
      .filter((selection): selection is ScheduleConnectorSelection => Boolean(selection));
    const browserGrant: ScheduleBrowserGrant | null = browserUnattended()
      ? {
          enabled: true,
          documentOrigins: parseOrigins(browserDocumentOrigins()),
          resourceOrigins: parseOrigins(browserResourceOrigins()),
          capabilities: [...browserCapabilities()].sort(),
          allowPrivateNetwork: browserPrivateNetwork(),
        }
      : null;
    const toolAllowlist = projectContext
      ? [
          ...CORE_READ_TOOLS,
          ...(allowWrite() ? ["workspace_write_file", "workspace_replace_text"] : []),
          ...(allowExec() ? ["workspace_exec"] : []),
        ]
      : [];
    return {
      id: crypto.randomUUID(),
      name: name().trim(),
      enabled: true,
      prompt: prompt().trim(),
      schedule: scheduleSpec(),
      entryProfile: "workbench",
      workloadOverride: profile() || null,
      contextTemplate: projectContext
        ? {
            kind: "project",
            project_id: selectedProjectId,
            execution_target:
              executionKind() === "managed_worktree"
                ? {
                    kind: "managed_worktree",
                    project_id: selectedProjectId,
                    base_revision: baseRevision(),
                  }
                : { kind: "local", project_id: selectedProjectId },
          }
        : continuationSessionId
          ? { kind: "session_continuation", session_id: continuationSessionId }
          : { kind: "general" },
      toolAllowlist,
      skillAllowlist: selectedSkillIds(),
      mcpToolAllowlist,
      permissionConfig: {
        permissionProfile:
          externalMcpTools.length > 0 || connectorSelections.length > 0 || browserGrant
            ? "external_sandbox"
            : projectSideEffects
              ? "workspace_write"
              : "read_only",
        allowFileRead: Boolean(projectContext),
        allowFileWrite: Boolean(projectContext && allowWrite()),
        allowExec: Boolean(projectContext && allowExec()),
        externalTargets: externalMcpTools.map(
          (selection) => `mcp:${selection.serverId}:${selection.toolName}`,
        ),
      },
      contributionRevisions: connectorSelections.map((selection) => selection.contributionRevision),
      hostGrant: {
        connectors: connectorSelections,
        browser: browserGrant,
        computerUnattended: false,
      },
      permissionRevision: 0,
      timeoutMs: 30 * 60 * 1_000,
      misfirePolicy: "skip",
      deliveryPolicy: deliveryPolicy(),
      stopConditions: {
        maxOccurrences: maxOccurrences().trim()
          ? Math.max(1, Number.parseInt(maxOccurrences(), 10))
          : null,
        endAtMs: endAt() ? new Date(endAt()).getTime() : null,
        stopAfterSuccess: stopAfterSuccess(),
      },
      configRevision: 0,
      createdBy: "",
      nextRunAtMs: null,
      health: "healthy",
      healthReason: null,
      createdAtMs: now,
      updatedAtMs: now,
    };
  }

  function scheduleSpec(): ScheduleSpec {
    const timestamp = new Date(runAt()).getTime();
    if (frequency() === "event") {
      const resourceKind = eventResourceKind().trim();
      return {
        kind: "event",
        matcher: {
          source: {
            kind: eventSourceKind(),
            principal: eventSourcePrincipal().trim(),
            id: eventSourceId().trim(),
          },
          eventType: eventType().trim(),
          subjectPrefix: eventSubjectPrefix().trim() || null,
          labels: parseEventLabels(eventLabels()),
          resource: resourceKind
            ? {
                kind: resourceKind,
                id: eventResourceId().trim(),
                revision: eventResourceRevision().trim() || null,
              }
            : null,
        },
      };
    }
    if (frequency() === "once") return { kind: "at", timestamp_ms: timestamp };
    if (frequency() === "cron") {
      return {
        kind: "cron",
        expression: cron().trim(),
        timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC",
      };
    }
    const date = new Date(runAt());
    const minute = date.getMinutes();
    const hour = date.getHours();
    const expression =
      frequency() === "weekly"
        ? `0 ${minute} ${hour} * * ${date.getDay()}`
        : `0 ${minute} ${hour} * * *`;
    return {
      kind: "cron",
      expression,
      timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC",
    };
  }

  async function mutate(id: string, operation: Promise<unknown>) {
    setBusyId(id);
    try {
      await operation;
      await refresh();
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusyId(undefined);
    }
  }

  return (
    <section class="task-center" data-testid="workbench-task-center">
      <PageHeading
        class="task-center-header"
        title={zh() ? "任务" : "Tasks"}
        actions={
          <Button
            variant="primary"
            onClick={() => {
              setFailure(undefined);
              resetForm();
              setShowCreate(true);
            }}
            data-testid="task-create-toggle"
          >
            <Plus size={16} /> {zh() ? "新建任务" : "New task"}
          </Button>
        }
      />

      <RuntimeHealthBanner component="scheduler" zh={zh()} />

      <Show when={failure()}>
        {(message) => (
          <div class="task-center-error" role="alert">
            <AlertTriangle size={16} /> {message()}
          </div>
        )}
      </Show>

      <Dialog
        open={showCreate()}
        size="wide"
        title={
          editingScheduleId() ? (zh() ? "编辑任务" : "Edit task") : zh() ? "新建任务" : "New task"
        }
        closeLabel={zh() ? "关闭" : "Close"}
        loading={submitting()}
        onOpenChange={(open) => {
          if (!open) {
            setShowCreate(false);
            resetForm();
          }
        }}
      >
        <form
          class="task-editor-form"
          onSubmit={(event) => {
            event.preventDefault();
            void submitSchedule();
          }}
        >
          <Show when={failure()}>
            {(message) => (
              <div class="task-editor-error" role="alert">
                <AlertTriangle size={15} />
                {message()}
              </div>
            )}
          </Show>
          <div class="task-form-grid">
            <TextField
              label={zh() ? "名称" : "Name"}
              testId="task-name"
              value={name()}
              {...(nameError() ? { error: nameError()! } : {})}
              autofocus
              onInput={(event) => {
                setName(event.currentTarget.value);
                if (event.currentTarget.value.trim()) setNameError(undefined);
              }}
            />
            <SelectField
              label={zh() ? "执行范围" : "Context"}
              testId="task-context"
              value={contextKind()}
              options={[
                { value: "general", label: zh() ? "通用办公" : "General office" },
                { value: "project", label: zh() ? "项目" : "Project" },
                {
                  value: "session_continuation",
                  label: zh() ? "现有对话续接" : "Existing Session continuation",
                },
              ]}
              onChange={(value) => setContextKind(value as ScheduleContext)}
            />
            <SelectField
              label={zh() ? "频率" : "Frequency"}
              testId="task-frequency"
              value={frequency()}
              options={[
                { value: "once", label: zh() ? "一次" : "Once" },
                { value: "daily", label: zh() ? "每天" : "Daily" },
                { value: "weekly", label: zh() ? "每周" : "Weekly" },
                { value: "cron", label: "Cron" },
                { value: "event", label: "Event" },
              ]}
              onChange={(value) => setFrequency(value as ScheduleFrequency)}
            />
            <Show when={frequency() !== "event"}>
              <div>
                <Show
                  when={frequency() === "cron"}
                  fallback={
                    <TextField
                      label={zh() ? "执行时间" : "Run at"}
                      testId="task-run-at"
                      type="datetime-local"
                      value={runAt()}
                      onInput={(event) => setRunAt(event.currentTarget.value)}
                    />
                  }
                >
                  <TextField
                    label="Cron"
                    testId="task-cron"
                    value={cron()}
                    onInput={(event) => setCron(event.currentTarget.value)}
                  />
                </Show>
              </div>
            </Show>
          </div>
          <Show when={frequency() === "event"}>
            <TaskEventForm
              zh={zh()}
              sourceKind={eventSourceKind()}
              sourcePrincipal={eventSourcePrincipal()}
              sourceId={eventSourceId()}
              eventType={eventType()}
              subjectPrefix={eventSubjectPrefix()}
              labels={eventLabels()}
              resourceKind={eventResourceKind()}
              resourceId={eventResourceId()}
              resourceRevision={eventResourceRevision()}
              onSourceKind={setEventSourceKind}
              onSourcePrincipal={setEventSourcePrincipal}
              onSourceId={setEventSourceId}
              onEventType={setEventType}
              onSubjectPrefix={setEventSubjectPrefix}
              onLabels={setEventLabels}
              onResourceKind={setEventResourceKind}
              onResourceId={setEventResourceId}
              onResourceRevision={setEventResourceRevision}
            />
          </Show>
          <Show when={contextKind() === "project"}>
            <div class="task-form-grid task-project-options">
              <SelectField
                label={zh() ? "项目" : "Project"}
                testId="task-project"
                value={projectId() ?? ""}
                options={props.projects.map((project) => ({
                  value: project.id,
                  label: project.displayName,
                }))}
                onChange={setProjectId}
              />
              <SelectField
                label={zh() ? "工作负载" : "Workload"}
                testId="task-profile"
                value={profile()}
                options={[
                  { value: "", label: zh() ? "自动判断" : "Automatic" },
                  { value: "coding", label: "Coding" },
                  { value: "office", label: "Office" },
                ]}
                onChange={(value) => setProfile(value as "" | "office" | "coding")}
              />
              <SelectField
                label={zh() ? "工作区" : "Workspace"}
                testId="task-execution-target"
                value={executionKind()}
                options={[
                  { value: "local", label: "Local" },
                  {
                    value: "managed_worktree",
                    label: "Managed Worktree",
                    disabled: projectGit()?.state.kind !== "ready",
                  },
                ]}
                description={projectGitDescription()}
                onChange={(value) => setExecutionKind(value as "local" | "managed_worktree")}
              />
              <Show when={executionKind() === "managed_worktree"}>
                <SelectField
                  label={zh() ? "基础分支" : "Base branch"}
                  testId="task-base-revision"
                  value={baseRevision()}
                  options={gitRefs().map((reference) => ({
                    value: reference.name,
                    label: reference.name,
                  }))}
                  onChange={setBaseRevision}
                />
              </Show>
            </div>
          </Show>
          <Show when={contextKind() === "session_continuation"}>
            <div class="task-form-grid task-project-options">
              <SelectField
                label={zh() ? "对话" : "Session"}
                testId="task-session-continuation"
                value={sessionId() ?? ""}
                options={sessions().map((session) => ({
                  value: session.id,
                  label: `${session.title} · ${session.id.slice(0, 8)}`,
                }))}
                description={
                  zh()
                    ? "每次触发都在同一 lane 创建 fresh Run，不恢复旧审批、临时权限或 Host lease。"
                    : "Each occurrence creates a fresh Run in the same lane without restoring approvals, temporary grants, or Host leases."
                }
                onChange={setSessionId}
              />
              <SelectField
                label={zh() ? "工作负载" : "Workload"}
                value={profile()}
                options={[
                  { value: "", label: zh() ? "自动判断" : "Automatic" },
                  { value: "coding", label: "Coding" },
                  { value: "office", label: "Office" },
                ]}
                onChange={(value) => setProfile(value as "" | "office" | "coding")}
              />
            </div>
          </Show>
          <TextArea
            class="task-prompt-field"
            data-testid="task-prompt"
            label={zh() ? "提示词" : "Prompt"}
            value={prompt()}
            invalid={Boolean(promptError())}
            {...(promptError() ? { description: promptError()! } : {})}
            onInput={(event) => {
              setPrompt(event.currentTarget.value);
              if (event.currentTarget.value.trim()) setPromptError(undefined);
            }}
            placeholder={
              zh()
                ? "例如：汇总今天的会议记录并列出待办"
                : "Example: summarize today's meeting notes and list action items"
            }
          />
          <details
            class="task-advanced-section"
            open={advancedOpen()}
            onToggle={(event) => setAdvancedOpen(event.currentTarget.open)}
          >
            <summary>
              <span>
                <strong>{zh() ? "高级设置" : "Advanced settings"}</strong>
                <small>
                  {zh()
                    ? `${selectedSkillIds().length + selectedMcpTools().length} 项扩展 · ${allowWrite() || allowExec() ? "扩展权限" : "只读"}`
                    : `${selectedSkillIds().length + selectedMcpTools().length} extensions · ${allowWrite() || allowExec() ? "elevated" : "read only"}`}
                </small>
              </span>
              <ChevronDown size={16} aria-hidden="true" />
            </summary>
            <div class="task-permission-section">
              <SelectField
                label={zh() ? "完成通知" : "Completion notification"}
                testId="task-delivery-policy"
                value={deliveryPolicy()}
                options={[
                  {
                    value: "task_tab_only",
                    label: zh() ? "仅任务中心" : "Task Center only",
                  },
                  {
                    value: "task_tab_and_system_notification",
                    label: zh() ? "任务中心 + 系统通知" : "Task Center + system notification",
                  },
                ]}
                description={
                  zh()
                    ? "系统通知只显示任务名称和终态，不包含提示词、结果正文或路径。"
                    : "System notifications contain only the task name and terminal status, never prompts, result text, or paths."
                }
                onChange={(value) => setDeliveryPolicy(value as DeliveryPolicy)}
              />
              <div class="task-form-grid task-stop-options">
                <TextField
                  label={zh() ? "最大执行次数" : "Maximum occurrences"}
                  type="number"
                  value={maxOccurrences()}
                  placeholder={zh() ? "不限" : "Unlimited"}
                  onInput={(event) => setMaxOccurrences(event.currentTarget.value)}
                />
                <TextField
                  label={zh() ? "截止时间" : "End at"}
                  type="datetime-local"
                  value={endAt()}
                  onInput={(event) => setEndAt(event.currentTarget.value)}
                />
              </div>
              <Checkbox
                class="task-check"
                label={zh() ? "首次成功后停止" : "Stop after first success"}
                checked={stopAfterSuccess()}
                onChange={(event) => setStopAfterSuccess(event.currentTarget.checked)}
              />
              <Show when={contextKind() === "project"}>
                <Checkbox
                  class="task-check"
                  label={zh() ? "允许项目写入" : "Allow project writes"}
                  checked={allowWrite()}
                  onChange={(event) => setAllowWrite(event.currentTarget.checked)}
                />
                <Checkbox
                  class="task-check"
                  label={zh() ? "允许受限命令" : "Allow restricted commands"}
                  checked={allowExec()}
                  onChange={(event) => setAllowExec(event.currentTarget.checked)}
                />
              </Show>
              <div class="task-skill-grid">
                <For each={props.skills.filter((skill) => skill.enabled)}>
                  {(skill) => (
                    <Checkbox
                      class="task-check"
                      label={skill.qualifiedName}
                      checked={selectedSkillIds().includes(skill.id)}
                      onChange={(event) =>
                        setSelectedSkillIds((current) =>
                          event.currentTarget.checked
                            ? [...current, skill.id]
                            : current.filter((id) => id !== skill.id),
                        )
                      }
                    />
                  )}
                </For>
              </div>
              <Show when={mcpTools().length > 0}>
                <div class="task-extension-heading">
                  <strong>{zh() ? "MCP 工具" : "MCP tools"}</strong>
                  <small>
                    {zh()
                      ? "后台调用固定到 Server、Tool 和 Schema 版本；非只读工具会进入持久授权范围。"
                      : "Background calls are pinned to the Server, Tool, and Schema revision. Non-read-only tools enter the persisted authorization scope."}
                  </small>
                </div>
                <div class="task-skill-grid" data-testid="task-mcp-tools">
                  <For each={mcpTools()}>
                    {(entry) => {
                      const selected = () =>
                        selectedMcpTools().some(
                          (selection) =>
                            selection.serverId === entry.server.configuration.id &&
                            selection.toolName === entry.tool.name &&
                            selection.schemaHash === entry.tool.schemaHash &&
                            selection.hostIdentityHash === entry.tool.hostIdentityHash,
                        );
                      return (
                        <Checkbox
                          class="task-check"
                          label={`${entry.server.configuration.displayName} / ${entry.tool.name}${
                            entry.readOnly ? (zh() ? "（只读）" : " (read only)") : ""
                          }`}
                          checked={selected()}
                          onChange={(event) => {
                            const selection: McpToolSelection = {
                              serverId: entry.server.configuration.id,
                              toolName: entry.tool.name,
                              schemaHash: entry.tool.schemaHash,
                              hostIdentityHash: entry.tool.hostIdentityHash,
                            };
                            setSelectedMcpTools((current) =>
                              event.currentTarget.checked
                                ? [
                                    ...current.filter((item) => !sameMcpTool(item, selection)),
                                    selection,
                                  ]
                                : current.filter((item) => !sameMcpTool(item, selection)),
                            );
                          }}
                        />
                      );
                    }}
                  </For>
                </div>
              </Show>
              <Show when={connectors().length > 0}>
                <div class="task-extension-heading">
                  <strong>{zh() ? "Connector" : "Connectors"}</strong>
                  <small>
                    {zh()
                      ? "账户、贡献点内容和 Host/Schema/Action revision 会固定到本次授权。"
                      : "Account, contribution content, and Host/Schema/Action revisions are pinned to this authorization."}
                  </small>
                </div>
                <div class="task-connector-list" data-testid="task-connectors">
                  <For each={connectors()}>
                    {(entry) => (
                      <div class="task-connector-card">
                        <strong>{entry.account.displayName}</strong>
                        <small>
                          {entry.account.pluginId} / {entry.account.connectorId}
                        </small>
                        <div class="task-skill-grid">
                          <For each={entry.descriptor.actions}>
                            {(action) => (
                              <Checkbox
                                class="task-check"
                                label={action}
                                checked={(
                                  selectedConnectorActions()[entry.account.id] ?? []
                                ).includes(action)}
                                onChange={(event) =>
                                  setSelectedConnectorActions((current) => {
                                    const selected = current[entry.account.id] ?? [];
                                    const next = event.currentTarget.checked
                                      ? [...new Set([...selected, action])]
                                      : selected.filter((value) => value !== action);
                                    return { ...current, [entry.account.id]: next };
                                  })
                                }
                              />
                            )}
                          </For>
                        </div>
                      </div>
                    )}
                  </For>
                </div>
              </Show>
              <div class="task-extension-heading">
                <strong>
                  {zh() ? "内置 Browser（无人值守）" : "Embedded Browser (unattended)"}
                </strong>
                <small>
                  {zh()
                    ? "默认关闭；定时任务只使用内置 Browser。Origin 和能力会固定到 ScheduleGrant，Computer 无人值守始终不支持。"
                    : "Off by default; scheduled tasks use only the embedded Browser. Origins and capabilities are pinned to the ScheduleGrant; unattended Computer remains unsupported."}
                </small>
              </div>
              <Checkbox
                class="task-check"
                label={zh() ? "启用无人值守 Browser" : "Enable unattended Browser"}
                checked={browserUnattended()}
                onChange={(event) => setBrowserUnattended(event.currentTarget.checked)}
              />
              <Show when={browserUnattended()}>
                <div class="task-browser-grant" data-testid="task-browser-grant">
                  <TextArea
                    label={zh() ? "文档 Origin（每行一个）" : "Document origins (one per line)"}
                    value={browserDocumentOrigins()}
                    onInput={(event) => setBrowserDocumentOrigins(event.currentTarget.value)}
                    placeholder="https://example.com"
                  />
                  <TextArea
                    label={zh() ? "资源 Origin（每行一个）" : "Resource origins (one per line)"}
                    value={browserResourceOrigins()}
                    onInput={(event) => setBrowserResourceOrigins(event.currentTarget.value)}
                    placeholder="https://static.example.com"
                  />
                  <div class="task-skill-grid">
                    <For each={BROWSER_CAPABILITIES}>
                      {(capability) => (
                        <Checkbox
                          class="task-check"
                          label={capability}
                          checked={browserCapabilities().includes(capability)}
                          onChange={(event) =>
                            setBrowserCapabilities((current) =>
                              event.currentTarget.checked
                                ? [...new Set([...current, capability])]
                                : current.filter((value) => value !== capability),
                            )
                          }
                        />
                      )}
                    </For>
                  </div>
                  <Checkbox
                    class="task-check"
                    label={
                      zh() ? "允许已列出的私网 Origin" : "Allow listed private-network origins"
                    }
                    checked={browserPrivateNetwork()}
                    onChange={(event) => setBrowserPrivateNetwork(event.currentTarget.checked)}
                  />
                </div>
              </Show>
            </div>
          </details>
          <footer class="task-form-actions">
            <Button
              variant="ghost"
              type="button"
              onClick={() => {
                setShowCreate(false);
                resetForm();
              }}
            >
              {zh() ? "取消" : "Cancel"}
            </Button>
            <Button variant="primary" type="submit" disabled={submitting()} data-testid="task-save">
              <ShieldCheck size={16} />
              {editingScheduleId()
                ? zh()
                  ? "保存更改"
                  : "Save changes"
                : zh()
                  ? "创建并授权"
                  : "Create and authorize"}
            </Button>
          </footer>
        </form>
      </Dialog>

      <Show
        when={schedules().length > 0}
        fallback={
          <div class="task-empty">
            <strong>
              {loading()
                ? zh()
                  ? "正在加载任务"
                  : "Loading tasks"
                : zh()
                  ? "还没有任务"
                  : "No tasks yet"}
            </strong>
            <Show when={!loading()}>
              <Button
                variant="primary"
                onClick={() => {
                  setFailure(undefined);
                  resetForm();
                  setShowCreate(true);
                }}
              >
                <Plus size={16} />
                {zh() ? "新建任务" : "New task"}
              </Button>
            </Show>
          </div>
        }
      >
        <div class="task-card-grid">
          <For each={schedules()}>
            {(schedule) => (
              <TaskCard
                schedule={schedule}
                recentRun={taskRuns().find((task) => task.scheduleId === schedule.id)}
                projects={props.projects}
                zh={zh()}
                busy={busyId() === schedule.id}
                onRun={() =>
                  void mutate(
                    schedule.id,
                    props.commandPort.runScheduleNow(directUserMutationContext(), schedule.id),
                  )
                }
                onHistory={() => setSelectedScheduleId(schedule.id)}
                onToggle={(enabled) =>
                  void mutate(
                    schedule.id,
                    props.commandPort.setScheduleEnabled(
                      directUserMutationContext(),
                      schedule.id,
                      enabled,
                      schedule.configRevision,
                    ),
                  )
                }
                onEdit={() => beginEdit(schedule)}
                onDelete={() => setRemoveScheduleId(schedule.id)}
                onReauthorize={() =>
                  void mutate(
                    schedule.id,
                    props.commandPort.reauthorizeSchedule(directUserMutationContext(), schedule.id),
                  )
                }
              />
            )}
          </For>
        </div>
      </Show>

      <TaskHistoryDialog
        schedule={selectedSchedule()}
        runs={selectedRuns()}
        events={selectedEvents()}
        zh={zh()}
        busyId={busyId()}
        onClose={() => setSelectedScheduleId(undefined)}
        onOpenSession={(sessionId) => {
          setSelectedScheduleId(undefined);
          props.onOpenSession(sessionId);
        }}
        onCancel={(run) =>
          void mutate(run.id, props.commandPort.cancelTaskRun(directUserMutationContext(), run.id))
        }
        onRetry={(run) =>
          void mutate(run.id, props.commandPort.retryTaskRun(directUserMutationContext(), run.id))
        }
        onContinue={(run) => {
          const openSession = props.onOpenSession;
          void mutate(
            run.id,
            props.commandPort
              .continueTaskInteractively(directUserMutationContext(), run.id)
              .then((continuation) => {
                setSelectedScheduleId(undefined);
                openSession(continuation.session.id);
              }),
          );
        }}
      />

      <Dialog
        open={Boolean(removeScheduleId())}
        tone="danger"
        title={zh() ? "删除任务" : "Delete task"}
        description={
          zh()
            ? `删除“${schedules().find((schedule) => schedule.id === removeScheduleId())?.name ?? ""}”？运行历史和对应会话会保留。`
            : `Delete “${schedules().find((schedule) => schedule.id === removeScheduleId())?.name ?? ""}”? Run history and sessions will be preserved.`
        }
        closeLabel={zh() ? "关闭" : "Close"}
        onOpenChange={(open) => {
          if (!open) setRemoveScheduleId(undefined);
        }}
      >
        <div class="task-delete-actions">
          <Button variant="ghost" onClick={() => setRemoveScheduleId(undefined)}>
            {zh() ? "取消" : "Cancel"}
          </Button>
          <Button
            variant="danger"
            data-testid="task-delete-confirm"
            disabled={Boolean(removeScheduleId() && busyId() === removeScheduleId())}
            onClick={() => {
              const id = removeScheduleId();
              if (!id) return;
              void mutate(
                id,
                props.commandPort
                  .removeSchedule(directUserMutationContext(), id)
                  .then(() => setRemoveScheduleId(undefined)),
              );
            }}
          >
            {zh() ? "删除任务" : "Delete task"}
          </Button>
        </div>
      </Dialog>
    </section>
  );
}

function defaultDateTime(): string {
  const date = new Date(Date.now() + 60 * 60 * 1_000);
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60_000);
  return local.toISOString().slice(0, 16);
}

function toLocalDateTime(timestampMs: number): string {
  const date = new Date(timestampMs);
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60_000);
  return local.toISOString().slice(0, 16);
}

function authorizationScopeChanged(
  current: ScheduleDefinition,
  updated: ScheduleDefinition,
): boolean {
  const snapshot = (schedule: ScheduleDefinition) =>
    JSON.stringify({
      entryProfile: schedule.entryProfile,
      workloadOverride: schedule.workloadOverride,
      contextTemplate: schedule.contextTemplate,
      toolAllowlist: [...schedule.toolAllowlist].sort(),
      skillAllowlist: [...schedule.skillAllowlist].sort(),
      mcpToolAllowlist: [...schedule.mcpToolAllowlist].sort((left, right) =>
        `${left.serverId}\0${left.toolName}\0${left.schemaHash}\0${left.hostIdentityHash}`.localeCompare(
          `${right.serverId}\0${right.toolName}\0${right.schemaHash}\0${right.hostIdentityHash}`,
        ),
      ),
      contributionRevisions: [...(schedule.contributionRevisions ?? [])].sort((left, right) =>
        `${left.pluginId}\0${left.contributionId}\0${left.accountId ?? ""}`.localeCompare(
          `${right.pluginId}\0${right.contributionId}\0${right.accountId ?? ""}`,
        ),
      ),
      hostGrant: schedule.hostGrant ?? {
        connectors: [],
        browser: null,
        computerUnattended: false,
      },
      permissionConfig: {
        ...schedule.permissionConfig,
        externalTargets: [...schedule.permissionConfig.externalTargets].sort(),
      },
    });
  return snapshot(current) !== snapshot(updated);
}

function parseOrigins(value: string): string[] {
  return [
    ...new Set(
      value
        .split(/[\r\n,]+/)
        .map((origin) => origin.trim())
        .filter(Boolean),
    ),
  ].sort();
}

function sameMcpTool(left: McpToolSelection, right: McpToolSelection): boolean {
  return (
    left.serverId === right.serverId &&
    left.toolName === right.toolName &&
    left.schemaHash === right.schemaHash &&
    left.hostIdentityHash === right.hostIdentityHash
  );
}

function parseEventLabels(value: string): Record<string, string> {
  const labels: Record<string, string> = {};
  const lines = value
    .split(/[\r\n]+/)
    .map((line) => line.trim())
    .filter(Boolean);
  if (lines.length > 16) throw new Error("Event labels are limited to 16 exact-match entries.");
  for (const line of lines) {
    const separator = line.indexOf("=");
    if (separator <= 0) throw new Error(`Invalid Event label: ${line}`);
    const key = line.slice(0, separator).trim();
    const labelValue = line.slice(separator + 1).trim();
    if (!key || key.length > 128 || labelValue.length > 256) {
      throw new Error(`Invalid Event label: ${line}`);
    }
    labels[key] = labelValue;
  }
  return Object.fromEntries(
    Object.entries(labels).sort(([left], [right]) => left.localeCompare(right)),
  );
}

function formatEventLabels(labels: Record<string, string>): string {
  return Object.entries(labels)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([key, value]) => `${key}=${value}`)
    .join("\n");
}
