import {
  commandFailure,
  type DeliveryPolicy,
  type GitRefRecord,
  type McpServerView,
  type McpToolSelection,
  type McpToolView,
  type MutationContext,
  type ProjectGitSnapshot,
  type ProjectRecord,
  type ScheduleDefinition,
  type ScheduleSpec,
  type SkillRecord,
  type TaskRunRecord,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import {
  AlertTriangle,
  Badge,
  Button,
  CalendarClock,
  Check,
  Checkbox,
  GitBranch,
  PageHeading,
  Play,
  Plus,
  RefreshCw,
  Settings,
  ShieldCheck,
  SelectField,
  Square,
  TextArea,
  TextField,
  Trash2,
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
import "./task-center.css";

type ScheduleFrequency = "once" | "daily" | "weekly" | "cron";
type ScheduleContext = "general" | "project";

type TaskMcpTool = {
  server: McpServerView;
  tool: McpToolView;
  readOnly: boolean;
};

const CORE_READ_TOOLS = [
  "workspace_read_file",
  "workspace_list_directory",
  "workspace_search_text",
  "workspace_git_status",
  "workspace_git_diff",
];

function mutationContext(): MutationContext {
  return {
    requestId: crypto.randomUUID(),
    clientId: "window:workbench",
    protocolVersion: 18,
    idempotencyKey: crypto.randomUUID(),
    expectedRunId: null,
    expectedGeneration: null,
  };
}

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
  const [selectedScheduleId, setSelectedScheduleId] = createSignal<string>();
  const [showCreate, setShowCreate] = createSignal(false);
  const [editingScheduleId, setEditingScheduleId] = createSignal<string>();
  const [loading, setLoading] = createSignal(true);
  const [submitting, setSubmitting] = createSignal(false);
  const [busyId, setBusyId] = createSignal<string>();
  const [failure, setFailure] = createSignal<string>();
  const [name, setName] = createSignal(zh() ? "每日任务" : "Daily task");
  const [prompt, setPrompt] = createSignal("");
  const [frequency, setFrequency] = createSignal<ScheduleFrequency>("daily");
  const [runAt, setRunAt] = createSignal(defaultDateTime());
  const [cron, setCron] = createSignal("0 9 * * *");
  const [deliveryPolicy, setDeliveryPolicy] = createSignal<DeliveryPolicy>("task_tab_only");
  const [contextKind, setContextKind] = createSignal<ScheduleContext>("general");
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
  const selectedSchedule = createMemo(() =>
    schedules().find((schedule) => schedule.id === selectedScheduleId()),
  );
  const selectedRuns = createMemo(() =>
    taskRuns().filter((task) => task.scheduleId === selectedScheduleId()),
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
      const [nextSchedules, nextRuns] = await Promise.all([
        props.commandPort.listSchedules(),
        props.commandPort.listTaskRuns(null, 200),
      ]);
      setSchedules(nextSchedules);
      setTaskRuns(nextRuns);
      setSelectedScheduleId((current) =>
        current && nextSchedules.some((schedule) => schedule.id === current)
          ? current
          : nextSchedules[0]?.id,
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

  onMount(() => {
    void refresh();
    void refreshMcpTools().catch((error) => setFailure(commandFailure(error).message));
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
    setDeliveryPolicy("task_tab_only");
    setContextKind("general");
    setProfile("");
    setExecutionKind("local");
    setAllowWrite(false);
    setAllowExec(false);
    setSelectedSkillIds([]);
    setSelectedMcpTools([]);
  }

  async function submitSchedule() {
    if (!name().trim() || !prompt().trim()) {
      setFailure(zh() ? "请输入任务名称和提示词。" : "Enter a task name and prompt.");
      return;
    }
    if (contextKind() === "project" && !projectId()) {
      setFailure(zh() ? "请选择项目。" : "Select a project.");
      return;
    }
    if (executionKind() === "managed_worktree" && !baseRevision()) {
      setFailure(zh() ? "请选择 Worktree 基础分支。" : "Select a Worktree base branch.");
      return;
    }
    setSubmitting(true);
    try {
      const draft = buildDefinition();
      const editing = schedules().find((schedule) => schedule.id === editingScheduleId());
      const snapshot = editing
        ? await updateExistingSchedule(editing, draft)
        : await props.commandPort.createSchedule({
            context: mutationContext(),
            definition: draft,
            authorize: true,
          });
      await refresh();
      setSelectedScheduleId(snapshot.definition.id);
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
      mcpToolAllowlist: current.mcpToolAllowlist,
      permissionConfig: draft.permissionConfig,
      deliveryPolicy: draft.deliveryPolicy,
    };
    const updated = await props.commandPort.updateSchedule({
      context: mutationContext(),
      definition,
      expectedConfigRevision: current.configRevision,
    });
    if (authorizationScopeChanged(current, updated)) {
      await props.commandPort.reauthorizeSchedule(mutationContext(), updated.id);
    }
    return { definition: updated };
  }

  function beginEdit(schedule: ScheduleDefinition) {
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
    setShowCreate(true);
  }

  function applyScheduleToForm(schedule: ScheduleSpec) {
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
        : { kind: "general" },
      toolAllowlist,
      skillAllowlist: selectedSkillIds(),
      mcpToolAllowlist,
      permissionConfig: {
        permissionProfile:
          externalMcpTools.length > 0
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
      permissionRevision: 0,
      timeoutMs: 30 * 60 * 1_000,
      misfirePolicy: "skip",
      deliveryPolicy: deliveryPolicy(),
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
        eyebrow={
          <>
            <CalendarClock size={15} /> {zh() ? "自动化" : "Automation"}
          </>
        }
        title={zh() ? "任务" : "Tasks"}
        description={
          zh()
            ? "按计划创建全新的 Agent Run；后台任务只使用你明确授权的 Skill、工具和项目权限。"
            : "Create fresh Agent Runs on a schedule using only explicitly authorized skills, tools, and project permissions."
        }
        actions={
          <Button
            variant="primary"
            onClick={() => {
              if (showCreate()) {
                setShowCreate(false);
                resetForm();
              } else {
                resetForm();
                setShowCreate(true);
              }
            }}
            data-testid="task-create-toggle"
          >
            <Plus size={16} /> {zh() ? "新建任务" : "New task"}
          </Button>
        }
      />

      <Show when={failure()}>
        {(message) => (
          <div class="task-center-error" role="alert">
            <AlertTriangle size={16} /> {message()}
          </div>
        )}
      </Show>

      <Show when={showCreate()}>
        <form
          class="task-create-card"
          onSubmit={(event) => {
            event.preventDefault();
            void submitSchedule();
          }}
        >
          <div class="task-form-grid">
            <TextField
              label={zh() ? "名称" : "Name"}
              testId="task-name"
              value={name()}
              onInput={(event) => setName(event.currentTarget.value)}
            />
            <SelectField
              label={zh() ? "执行范围" : "Context"}
              testId="task-context"
              value={contextKind()}
              options={[
                { value: "general", label: zh() ? "通用办公" : "General office" },
                { value: "project", label: zh() ? "项目" : "Project" },
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
              ]}
              onChange={(value) => setFrequency(value as ScheduleFrequency)}
            />
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
          </div>
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
          <TextArea
            class="task-prompt-field"
            data-testid="task-prompt"
            label={zh() ? "提示词" : "Prompt"}
            value={prompt()}
            onInput={(event) => setPrompt(event.currentTarget.value)}
            placeholder={
              zh()
                ? "例如：汇总今天的会议记录并列出待办"
                : "Example: summarize today's meeting notes and list action items"
            }
          />
          <div class="task-permission-section">
            <div>
              <strong>{zh() ? "权限与扩展" : "Permissions and extensions"}</strong>
              <small>
                {zh()
                  ? "修改以下范围会要求重新授权。后台任务不会弹出临时审批。"
                  : "Changing this scope requires reauthorization. Background tasks never wait on an interactive approval."}
              </small>
            </div>
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
          </div>
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
      </Show>

      <div class="task-center-layout">
        <div class="task-schedule-list">
          <Show
            when={schedules().length > 0}
            fallback={
              <div class="task-empty">
                <CalendarClock size={28} />
                <strong>
                  {loading()
                    ? zh()
                      ? "加载中…"
                      : "Loading…"
                    : zh()
                      ? "还没有任务"
                      : "No tasks yet"}
                </strong>
              </div>
            }
          >
            <For each={schedules()}>
              {(schedule) => {
                const recent = () => taskRuns().find((task) => task.scheduleId === schedule.id);
                return (
                  <Button
                    type="button"
                    class="task-schedule-row"
                    data-testid="task-schedule-row"
                    aria-label={schedule.name}
                    title={schedule.name}
                    classList={{ selected: selectedScheduleId() === schedule.id }}
                    onClick={() => setSelectedScheduleId(schedule.id)}
                  >
                    <span class="task-schedule-icon">
                      <CalendarClock size={17} />
                    </span>
                    <span>
                      <strong>{schedule.name}</strong>
                      <small>{scheduleLabel(schedule, zh())}</small>
                    </span>
                    <Badge tone={healthTone(schedule.health)}>{schedule.health}</Badge>
                    <small>{recent()?.status ?? (zh() ? "尚未运行" : "Not run")}</small>
                  </Button>
                );
              }}
            </For>
          </Show>
        </div>

        <Show when={selectedSchedule()}>
          {(schedule) => (
            <article class="task-detail-card">
              <header>
                <div>
                  <h2>{schedule().name}</h2>
                  <p>{schedule().prompt}</p>
                </div>
                <Badge tone={healthTone(schedule().health)}>{schedule().health}</Badge>
              </header>
              <div class="task-detail-meta">
                <span>
                  <CalendarClock size={14} /> {scheduleLabel(schedule(), zh())}
                </span>
                <span>
                  <GitBranch size={14} /> {contextLabel(schedule(), props.projects, zh())}
                </span>
                <span>
                  <ShieldCheck size={14} /> revision {schedule().permissionRevision}
                </span>
              </div>
              <div class="task-detail-actions">
                <Button
                  variant="default"
                  data-testid="task-run-now"
                  disabled={busyId() === schedule().id}
                  onClick={() => {
                    const current = schedule();
                    void mutate(
                      current.id,
                      props.commandPort.runScheduleNow(mutationContext(), current.id),
                    );
                  }}
                >
                  <Play size={15} /> {zh() ? "立即运行" : "Run now"}
                </Button>
                <Button
                  variant="ghost"
                  data-testid="task-toggle-enabled"
                  disabled={busyId() === schedule().id}
                  onClick={() => {
                    const current = schedule();
                    void mutate(
                      current.id,
                      props.commandPort.setScheduleEnabled(
                        mutationContext(),
                        current.id,
                        !current.enabled,
                        current.configRevision,
                      ),
                    );
                  }}
                >
                  {schedule().enabled ? <Square size={14} /> : <Check size={14} />}
                  {schedule().enabled ? (zh() ? "停用" : "Disable") : zh() ? "启用" : "Enable"}
                </Button>
                <Button
                  variant="ghost"
                  data-testid="task-edit"
                  disabled={busyId() === schedule().id}
                  onClick={() => beginEdit(schedule())}
                >
                  <Settings size={14} /> {zh() ? "编辑" : "Edit"}
                </Button>
                <Show when={schedule().health !== "healthy"}>
                  <Button
                    variant="ghost"
                    onClick={() => {
                      const current = schedule();
                      void mutate(
                        current.id,
                        props.commandPort.reauthorizeSchedule(mutationContext(), current.id),
                      );
                    }}
                  >
                    <RefreshCw size={14} /> {zh() ? "重新授权" : "Reauthorize"}
                  </Button>
                </Show>
                <Button
                  variant="danger"
                  onClick={() => {
                    if (
                      !window.confirm(
                        zh()
                          ? "删除任务定义？历史运行会保留。"
                          : "Delete the task definition? Run history is retained.",
                      )
                    )
                      return;
                    const current = schedule();
                    void mutate(
                      current.id,
                      props.commandPort.removeSchedule(mutationContext(), current.id),
                    );
                  }}
                >
                  <Trash2 size={14} /> {zh() ? "删除" : "Delete"}
                </Button>
              </div>
              <section class="task-run-history">
                <h3>{zh() ? "运行历史" : "Run history"}</h3>
                <Show
                  when={selectedRuns().length > 0}
                  fallback={<p class="task-history-empty">{zh() ? "尚未运行" : "No runs yet"}</p>}
                >
                  <For each={selectedRuns()}>
                    {(task) => (
                      <div class="task-run-row" data-testid="task-run-row">
                        <span class={`task-status-dot ${task.status}`} />
                        <span>
                          <strong data-testid="task-run-status">{task.status}</strong>
                          <small>{formatTime(task.createdAtMs)}</small>
                          <Show when={task.errorSummary}>
                            <small class="task-run-error">{task.errorSummary}</small>
                          </Show>
                        </span>
                        <div class="task-run-actions">
                          <Show when={task.executionSessionId}>
                            <Button
                              type="button"
                              onClick={() => props.onOpenSession(task.executionSessionId!)}
                            >
                              {zh() ? "查看" : "Open"}
                            </Button>
                          </Show>
                          <Show when={["queued", "preparing", "running"].includes(task.status)}>
                            <Button
                              type="button"
                              data-testid="task-cancel"
                              onClick={() => {
                                const taskId = task.id;
                                void mutate(
                                  taskId,
                                  props.commandPort.cancelTaskRun(mutationContext(), taskId),
                                );
                              }}
                            >
                              {zh() ? "取消" : "Cancel"}
                            </Button>
                          </Show>
                          <Show when={task.status === "needs_attention"}>
                            <Button
                              type="button"
                              onClick={() => {
                                const taskId = task.id;
                                const openSession = props.onOpenSession;
                                void mutate(
                                  taskId,
                                  props.commandPort
                                    .continueTaskInteractively(mutationContext(), taskId)
                                    .then((continuation) => openSession(continuation.session.id)),
                                );
                              }}
                            >
                              {zh() ? "转为交互" : "Continue"}
                            </Button>
                          </Show>
                          <Show
                            when={["failed", "timed_out", "lost", "cancelled"].includes(
                              task.status,
                            )}
                          >
                            <Button
                              type="button"
                              onClick={() => {
                                const taskId = task.id;
                                void mutate(
                                  taskId,
                                  props.commandPort.retryTaskRun(mutationContext(), taskId),
                                );
                              }}
                            >
                              {zh() ? "重试" : "Retry"}
                            </Button>
                          </Show>
                        </div>
                      </div>
                    )}
                  </For>
                </Show>
              </section>
            </article>
          )}
        </Show>
      </div>
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
      permissionConfig: {
        ...schedule.permissionConfig,
        externalTargets: [...schedule.permissionConfig.externalTargets].sort(),
      },
    });
  return snapshot(current) !== snapshot(updated);
}

function sameMcpTool(left: McpToolSelection, right: McpToolSelection): boolean {
  return (
    left.serverId === right.serverId &&
    left.toolName === right.toolName &&
    left.schemaHash === right.schemaHash &&
    left.hostIdentityHash === right.hostIdentityHash
  );
}

function formatTime(value: number | null): string {
  return value
    ? new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(value)
    : "—";
}

function scheduleLabel(schedule: ScheduleDefinition, zh: boolean): string {
  const next = formatTime(schedule.nextRunAtMs);
  if (!schedule.enabled) return zh ? "已停用" : "Disabled";
  return `${zh ? "下次" : "Next"}: ${next}`;
}

function contextLabel(
  schedule: ScheduleDefinition,
  projects: ProjectRecord[],
  zh: boolean,
): string {
  if (schedule.contextTemplate.kind === "general") return zh ? "通用" : "General";
  const projectId = schedule.contextTemplate.project_id;
  return projects.find((project) => project.id === projectId)?.displayName ?? projectId;
}

function healthTone(
  health: ScheduleDefinition["health"],
): "neutral" | "success" | "warning" | "danger" {
  if (health === "healthy") return "success";
  if (health === "invalid") return "danger";
  return "warning";
}
