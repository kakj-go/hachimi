import {
  type AgentPermissionPolicy,
  commandFailure,
  type ConnectorAccount,
  type ConnectorDriverDescriptor,
  type ConnectorRevisionSelection,
  type DeliveryPolicy,
  type McpServerView,
  type McpToolSelection,
  type McpToolView,
  type ScheduleDefinition,
  type ScheduleEventReceipt,
  type ScheduleEventSourceKind,
  type ScheduleSpec,
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
  FolderOpen,
  PageHeading,
  Plus,
  ShieldCheck,
  SelectField,
  TextArea,
  TextField,
} from "@hachimi/ui";
import { For, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";

import type { WorkbenchCommandPort } from "./workbench-command-port";
import { directUserMutationContext } from "./mutation-context";
import { PermissionPolicyEditor, createPermissionPolicy } from "./permission-policy-editor";
import { PermissionScopeConfirmation } from "./permission-scope-confirmation";
import { permissionScopeRisk } from "./permission-scope-risk";
import { TaskCard } from "./task-card";
import { TaskHistoryDialog } from "./task-history-dialog";
import { TaskEventForm } from "./task-event-form";
import { RuntimeHealthBanner } from "./runtime-health";
import "./task-center.css";

type ScheduleFrequency = "once" | "daily" | "weekly" | "cron" | "event";
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

export function TaskCenter(props: {
  commandPort: WorkbenchCommandPort;
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
  const [confirmingPermissions, setConfirmingPermissions] = createSignal(false);
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
  const [profile, setProfile] = createSignal<"" | "office" | "coding">("");
  const [workspaceKind, setWorkspaceKind] = createSignal<"managed" | "selected_directory">(
    "managed",
  );
  const [workspacePath, setWorkspacePath] = createSignal("");
  const [conversationMode, setConversationMode] = createSignal<
    "shared_session" | "per_run_session"
  >("per_run_session");
  const [permissionPolicy, setPermissionPolicy] = createSignal<AgentPermissionPolicy>(
    createPermissionPolicy("read_only"),
  );
  const [selectedSkillIds, setSelectedSkillIds] = createSignal<string[]>([]);
  const [mcpTools, setMcpTools] = createSignal<TaskMcpTool[]>([]);
  const [selectedMcpTools, setSelectedMcpTools] = createSignal<McpToolSelection[]>([]);
  const [connectors, setConnectors] = createSignal<TaskConnector[]>([]);
  const [selectedConnectorActions, setSelectedConnectorActions] = createSignal<
    Record<string, string[]>
  >({});
  const [selectedReadOnlyConnectorActions, setSelectedReadOnlyConnectorActions] = createSignal<
    Record<string, string[]>
  >({});
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
  async function refresh() {
    try {
      const [nextSchedules, nextRuns, nextEvents] = await Promise.all([
        props.commandPort.listSchedules(),
        props.commandPort.listTaskRuns(null, 200),
        Promise.resolve(props.commandPort.listScheduleEventReceipts?.(100) ?? []),
      ]);
      setSchedules(nextSchedules);
      setTaskRuns(nextRuns);
      setEventReceipts(nextEvents);
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
    setWorkspaceKind("managed");
    setWorkspacePath("");
    setConversationMode("per_run_session");
    setProfile("");
    setPermissionPolicy(createPermissionPolicy("read_only"));
    setSelectedSkillIds([]);
    setSelectedMcpTools([]);
    setSelectedConnectorActions({});
    setSelectedReadOnlyConnectorActions({});
    setMaxOccurrences("");
    setEndAt("");
    setStopAfterSuccess(false);
    setNameError(undefined);
    setPromptError(undefined);
    setAdvancedOpen(false);
  }

  async function submitSchedule(confirmedPermissions = false) {
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
    if (workspaceKind() === "selected_directory" && !workspacePath().trim()) {
      setFailure(zh() ? "请选择任务目录。" : "Choose a task directory.");
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
    const browserPolicy = permissionPolicy().rules.browser;
    if (
      permissionPolicy().level !== "full_access" &&
      (browserPolicy.observe ||
        browserPolicy.act ||
        browserPolicy.upload ||
        browserPolicy.download) &&
      !browserPolicy.unrestrictedOrigins &&
      (browserPolicy.origins ?? []).length === 0
    ) {
      setFailure(
        zh()
          ? "无人值守 Browser 至少需要一个精确的文档 Origin。"
          : "Unattended Browser requires at least one exact document origin.",
      );
      setAdvancedOpen(true);
      return;
    }
    if (permissionScopeRisk(permissionPolicy()).hasUnrestrictedScope && !confirmedPermissions) {
      setConfirmingPermissions(true);
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
      skillAllowlist: draft.skillAllowlist,
      skillRevisions: [],
      mcpToolAllowlist: draft.mcpToolAllowlist,
      contributionRevisions: draft.contributionRevisions ?? [],
      hostRevisionSnapshot: draft.hostRevisionSnapshot ?? {
        connectors: [],
      },
      permissionPolicy: draft.permissionPolicy,
      deliveryPolicy: draft.deliveryPolicy,
      ...(draft.stopConditions ? { stopConditions: draft.stopConditions } : {}),
    };
    const updated = await props.commandPort.updateSchedule({
      context: directUserMutationContext(),
      definition,
      expectedConfigRevision: current.configRevision,
    });
    return { definition: updated };
  }

  function beginEdit(schedule: ScheduleDefinition) {
    setFailure(undefined);
    setEditingScheduleId(schedule.id);
    setName(schedule.name);
    setPrompt(schedule.prompt);
    setDeliveryPolicy(schedule.deliveryPolicy);
    applyScheduleToForm(schedule.schedule);
    if (schedule.contextTemplate.kind !== "workspace") return;
    setWorkspaceKind(schedule.contextTemplate.workspace.kind);
    setWorkspacePath(
      schedule.contextTemplate.workspace.kind === "selected_directory"
        ? schedule.contextTemplate.workspace.root_path
        : "",
    );
    setConversationMode(schedule.contextTemplate.conversation_mode);
    setProfile(
      schedule.workloadOverride === "coding" || schedule.workloadOverride === "office"
        ? schedule.workloadOverride
        : "",
    );
    setPermissionPolicy(schedule.permissionPolicy);
    setSelectedSkillIds([...schedule.skillAllowlist]);
    setSelectedMcpTools([...schedule.mcpToolAllowlist]);
    setSelectedConnectorActions(
      Object.fromEntries(
        (schedule.hostRevisionSnapshot?.connectors ?? []).map((selection) => [
          selection.accountId,
          [...selection.allowedActions],
        ]),
      ),
    );
    setSelectedReadOnlyConnectorActions(
      Object.fromEntries(
        schedule.permissionPolicy.rules.connectors.map((rule) => [
          rule.accountId,
          [...rule.readOnlyActions],
        ]),
      ),
    );
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
    const policy = permissionPolicy();
    const fullAccess = policy.level === "full_access";
    const mcpToolAllowlist = (fullAccess ? [] : [...selectedMcpTools()]).sort((left, right) =>
      `${left.serverId}\0${left.toolName}\0${left.schemaHash}\0${left.hostIdentityHash}`.localeCompare(
        `${right.serverId}\0${right.toolName}\0${right.schemaHash}\0${right.hostIdentityHash}`,
      ),
    );
    const connectorSelections = (fullAccess ? [] : connectors())
      .map((entry): ConnectorRevisionSelection | undefined => {
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
      .filter((selection): selection is ConnectorRevisionSelection => Boolean(selection));
    const policyWithExtensions: AgentPermissionPolicy = {
      ...policy,
      rules: {
        ...policy.rules,
        mcp: mcpToolAllowlist.map((selection) => ({
          serverId: selection.serverId,
          toolName: selection.toolName,
          schemaHash: selection.schemaHash,
          readOnly:
            mcpTools().find(
              (entry) =>
                entry.server.configuration.id === selection.serverId &&
                entry.tool.name === selection.toolName &&
                entry.tool.schemaHash === selection.schemaHash &&
                entry.tool.hostIdentityHash === selection.hostIdentityHash,
            )?.readOnly ?? false,
        })),
        connectors: connectorSelections.map((selection) => ({
          accountId: selection.accountId,
          actions: [...selection.allowedActions].sort(),
          readOnlyActions: [...(selectedReadOnlyConnectorActions()[selection.accountId] ?? [])]
            .filter((action) => selection.allowedActions.includes(action))
            .sort(),
          contributionRevision: selection.contributionRevision.actionHash ?? "",
        })),
      },
    };
    return {
      id: crypto.randomUUID(),
      name: name().trim(),
      enabled: true,
      prompt: prompt().trim(),
      schedule: scheduleSpec(),
      entryProfile: "workbench",
      workloadOverride: profile() || null,
      contextTemplate: {
        kind: "workspace",
        workspace:
          workspaceKind() === "managed"
            ? { kind: "managed" }
            : { kind: "selected_directory", root_path: workspacePath().trim() },
        conversation_mode: conversationMode(),
      },
      skillAllowlist: selectedSkillIds(),
      skillRevisions: [],
      mcpToolAllowlist,
      permissionPolicy: policyWithExtensions,
      contributionRevisions: connectorSelections.map((selection) => selection.contributionRevision),
      hostRevisionSnapshot: {
        connectors: connectorSelections,
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
              label={zh() ? "任务目录" : "Task workspace"}
              testId="task-workspace-kind"
              value={workspaceKind()}
              options={[
                { value: "managed", label: zh() ? "应用内置目录" : "Managed workspace" },
                {
                  value: "selected_directory",
                  label: zh() ? "选择普通目录" : "Selected directory",
                },
              ]}
              onChange={(value) => setWorkspaceKind(value as "managed" | "selected_directory")}
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
          <div class="task-form-grid task-project-options">
            <SelectField
              label={zh() ? "会话模式" : "Conversation mode"}
              testId="task-conversation-mode"
              value={conversationMode()}
              options={[
                {
                  value: "per_run_session",
                  label: zh() ? "每次新建会话" : "New session per run",
                },
                {
                  value: "shared_session",
                  label: zh() ? "复用同一会话" : "Shared session",
                },
              ]}
              onChange={(value) =>
                setConversationMode(value as "shared_session" | "per_run_session")
              }
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
          </div>
          <Show when={workspaceKind() === "selected_directory"}>
            <div class="task-directory-picker">
              <TextField
                label={zh() ? "目录" : "Directory"}
                testId="task-workspace-path"
                value={workspacePath()}
                disabled
              />
              <Button
                type="button"
                variant="default"
                onClick={() => {
                  void commandPort
                    .chooseScheduleWorkspaceDirectory()
                    .then((path) => path && setWorkspacePath(path))
                    .catch((error) => setFailure(commandFailure(error).message));
                }}
              >
                <FolderOpen size={16} /> {zh() ? "选择" : "Choose"}
              </Button>
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
                    ? `${selectedSkillIds().length + selectedMcpTools().length} 项扩展 · ${permissionPolicy().level}`
                    : `${selectedSkillIds().length + selectedMcpTools().length} extensions · ${permissionPolicy().level}`}
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
              <PermissionPolicyEditor
                value={permissionPolicy()}
                testId="task-permission"
                zh={zh()}
                onChange={(policy) => {
                  setPermissionPolicy(policy);
                  if (policy.level === "read_only") {
                    setSelectedReadOnlyConnectorActions({ ...selectedConnectorActions() });
                  }
                }}
              />
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
              <Show when={mcpTools().length > 0 && permissionPolicy().level !== "full_access"}>
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
                          disabled={!entry.readOnly && permissionPolicy().level === "read_only"}
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
              <Show when={connectors().length > 0 && permissionPolicy().level !== "full_access"}>
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
                            {(action) => {
                              const selected = () =>
                                (selectedConnectorActions()[entry.account.id] ?? []).includes(
                                  action,
                                );
                              const readOnly = () =>
                                (
                                  selectedReadOnlyConnectorActions()[entry.account.id] ?? []
                                ).includes(action);
                              return (
                                <div class="task-connector-action">
                                  <Checkbox
                                    class="task-check"
                                    label={action}
                                    checked={selected()}
                                    onChange={(event) => {
                                      const checked = event.currentTarget.checked;
                                      setSelectedConnectorActions((current) => {
                                        const values = current[entry.account.id] ?? [];
                                        const next = checked
                                          ? [...new Set([...values, action])]
                                          : values.filter((value) => value !== action);
                                        return { ...current, [entry.account.id]: next };
                                      });
                                      setSelectedReadOnlyConnectorActions((current) => {
                                        const values = current[entry.account.id] ?? [];
                                        const shouldReadOnly =
                                          checked && permissionPolicy().level === "read_only";
                                        const next = shouldReadOnly
                                          ? [...new Set([...values, action])]
                                          : checked
                                            ? values
                                            : values.filter((value) => value !== action);
                                        return { ...current, [entry.account.id]: next };
                                      });
                                    }}
                                  />
                                  <Show
                                    when={selected() && permissionPolicy().level === "writable"}
                                  >
                                    <Checkbox
                                      class="task-check"
                                      label={zh() ? "只读" : "Read only"}
                                      checked={readOnly()}
                                      onChange={(event) =>
                                        setSelectedReadOnlyConnectorActions((current) => {
                                          const values = current[entry.account.id] ?? [];
                                          const next = event.currentTarget.checked
                                            ? [...new Set([...values, action])]
                                            : values.filter((value) => value !== action);
                                          return { ...current, [entry.account.id]: next };
                                        })
                                      }
                                    />
                                  </Show>
                                </div>
                              );
                            }}
                          </For>
                        </div>
                      </div>
                    )}
                  </For>
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

      <PermissionScopeConfirmation
        open={confirmingPermissions()}
        policy={permissionPolicy()}
        zh={zh()}
        onCancel={() => setConfirmingPermissions(false)}
        onConfirm={() => {
          setConfirmingPermissions(false);
          void submitSchedule(true);
        }}
      />

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
