import { I18nProvider } from "@hachimi/i18n";
import { For, Show } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import { TaskCenter } from "./task-center";
import { createPermissionPolicy } from "./permission-policy-editor";
import type { WorkbenchCommandPort } from "./workbench-command-port";

vi.mock("@hachimi/ui", () => {
  const Icon = () => <span aria-hidden="true" />;
  return {
    AlertTriangle: Icon,
    Badge: (props: { children: unknown }) => <span>{props.children as never}</span>,
    Button: (props: Record<string, unknown>) => (
      <button
        type={(props.type as "button" | "submit" | undefined) ?? "button"}
        data-testid={props["data-testid"] as string | undefined}
        disabled={props.disabled as boolean | undefined}
        onClick={(event) =>
          (props.onClick as ((event: MouseEvent) => void) | undefined)?.(event as MouseEvent)
        }
      >
        {props.children as never}
      </button>
    ),
    CalendarClock: Icon,
    Check: Icon,
    ChevronDown: Icon,
    Checkbox: (props: Record<string, unknown>) => (
      <label>
        <input
          type="checkbox"
          checked={props.checked as boolean | undefined}
          disabled={props.disabled as boolean | undefined}
          onChange={(event) => (props.onChange as ((event: Event) => void) | undefined)?.(event)}
        />
        {props.label as never}
      </label>
    ),
    Clock3: Icon,
    Dialog: (props: Record<string, unknown>) => (
      <Show when={props.open}>
        <div role="dialog" aria-label={props.title as string}>
          <h2>{props.title as never}</h2>
          {props.description as never}
          {props.children as never}
        </div>
      </Show>
    ),
    ExternalLink: Icon,
    FolderOpen: Icon,
    FormField: (props: Record<string, unknown>) => (
      <label>
        {props.label as never}
        {props.children as never}
      </label>
    ),
    GitBranch: Icon,
    History: Icon,
    IconButton: (props: Record<string, unknown>) => (
      <button
        type="button"
        aria-label={props.label as string}
        data-testid={props["data-testid"] as string | undefined}
        disabled={props.disabled as boolean | undefined}
        onClick={(event) =>
          (props.onClick as ((event: MouseEvent) => void) | undefined)?.(event as MouseEvent)
        }
      >
        {props.children as never}
      </button>
    ),
    PageHeading: (props: Record<string, unknown>) => (
      <header class={props.class as string | undefined}>
        <span>{props.eyebrow as never}</span>
        <h1>{props.title as never}</h1>
        <p>{props.description as never}</p>
        <div>{props.actions as never}</div>
      </header>
    ),
    PermissionPolicyEditor: (props: Record<string, unknown>) => (
      <label>
        {props.zh ? "权限档位" : "Permission level"}
        <select
          aria-label={props.zh ? "权限档位" : "Permission level"}
          data-testid={props.testId as string | undefined}
          disabled={props.disabled as boolean | undefined}
          value={(props.value as { level: string }).level}
          onChange={(event) =>
            (props.onChange as ((next: Record<string, unknown>) => void) | undefined)?.({
              ...(props.value as Record<string, unknown>),
              level: event.currentTarget.value,
            })
          }
        >
          <option value="read_only">Read only</option>
          <option value="writable">Writable</option>
          <option value="full_access">Full access</option>
        </select>
      </label>
    ),
    Play: Icon,
    Pencil: Icon,
    Plus: Icon,
    RefreshCw: Icon,
    Settings: Icon,
    ShieldCheck: Icon,
    SelectField: (props: Record<string, unknown>) => (
      <label>
        {props.label as never}
        <select
          data-testid={props.testId as string | undefined}
          value={props.value as string}
          onChange={(event) =>
            (props.onChange as ((value: string) => void) | undefined)?.(event.currentTarget.value)
          }
        >
          <For each={props.options as Array<{ value: string; label: string }>}>
            {(option) => <option value={option.value}>{option.label}</option>}
          </For>
        </select>
      </label>
    ),
    SegmentedControl: (props: Record<string, unknown>) => (
      <label>
        {props.label as never}
        <select
          aria-label={props.label as string}
          value={props.value as string}
          onChange={(event) =>
            (props.onChange as ((value: string) => void) | undefined)?.(event.currentTarget.value)
          }
        >
          <For each={props.options as Array<{ value: string; label: string }>}>
            {(option) => <option value={option.value}>{option.label}</option>}
          </For>
        </select>
      </label>
    ),
    Square: Icon,
    Switch: (props: Record<string, unknown>) => (
      <input
        type="checkbox"
        aria-label={props.label as string}
        data-testid={props.testId as string | undefined}
        checked={props.checked as boolean | undefined}
        disabled={props.disabled as boolean | undefined}
        onChange={(event) =>
          (props.onChange as ((checked: boolean) => void) | undefined)?.(
            event.currentTarget.checked,
          )
        }
      />
    ),
    Tabs: (props: Record<string, unknown>) => (
      <div>
        {
          (props.tabs as Array<{ value: string; label: string; content: unknown }>).find(
            (tab) => tab.value === props.value,
          )?.content as never
        }
      </div>
    ),
    TextArea: (props: Record<string, unknown>) => (
      <label>
        {props.label as never}
        <textarea
          data-testid={props["data-testid"] as string | undefined}
          value={props.value as string}
          placeholder={props.placeholder as string}
          onInput={(event) => (props.onInput as ((event: InputEvent) => void) | undefined)?.(event)}
        />
      </label>
    ),
    TextField: (props: Record<string, unknown>) => (
      <label>
        {props.label as never}
        <input
          data-testid={props.testId as string | undefined}
          type={(props.type as string | undefined) ?? "text"}
          value={props.value as string}
          onInput={(event) => (props.onInput as ((event: InputEvent) => void) | undefined)?.(event)}
        />
      </label>
    ),
    Trash2: Icon,
    Tooltip: (props: { children: unknown }) => <>{props.children as never}</>,
  };
});

afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

describe("TaskCenter", () => {
  it("creates an Event schedule with an exact typed matcher and no future timestamp", async () => {
    const createSchedule = vi.fn(async (request) => ({
      definition: request.definition,
      recentRuns: [],
    }));
    const port = {
      listSchedules: vi.fn(async () => []),
      listTaskRuns: vi.fn(async () => []),
      searchAgentSessions: vi.fn(async () => ({ items: [], nextCursor: null })),
      listProjectGitRefs: vi.fn(async () => []),
      listMcpServers: vi.fn(async () => []),
      listMcpTools: vi.fn(async () => []),
      createSchedule,
    } as unknown as WorkbenchCommandPort;
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="en-US">
          <TaskCenter commandPort={port} skills={[]} onOpenSession={() => undefined} />
        </I18nProvider>
      ),
      root,
    );

    await Promise.resolve();
    (root.querySelector('[data-testid="task-create-toggle"]') as HTMLButtonElement).click();
    const frequency = root.querySelector('[data-testid="task-frequency"]') as HTMLSelectElement;
    frequency.value = "event";
    frequency.dispatchEvent(new Event("change", { bubbles: true }));
    const setInput = (testId: string, value: string) => {
      const input = root.querySelector(`[data-testid="${testId}"]`) as HTMLInputElement;
      input.value = value;
      input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    };
    setInput("task-event-source-principal", "plugin:calendar");
    setInput("task-event-source-id", "primary-calendar");
    setInput("task-event-type", "meeting.changed");
    const prompt = [...root.querySelectorAll("textarea")].at(-1)!;
    prompt.value = "Summarize the referenced meeting.";
    prompt.dispatchEvent(new InputEvent("input", { bubbles: true }));
    (root.querySelector("form") as HTMLFormElement).dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );

    await vi.waitFor(() => expect(createSchedule).toHaveBeenCalledTimes(1));
    const definition = createSchedule.mock.calls[0]![0].definition;
    expect(definition.schedule).toEqual({
      kind: "event",
      matcher: {
        source: {
          kind: "workspace",
          principal: "plugin:calendar",
          id: "primary-calendar",
        },
        eventType: "meeting.changed",
        subjectPrefix: null,
        labels: {},
        resource: null,
      },
    });
    expect(definition.nextRunAtMs).toBeNull();
    dispose();
  });

  it("creates an authorized General Office prompt schedule", async () => {
    const createSchedule = vi.fn(async (request) => ({
      definition: request.definition,
      recentRuns: [],
    }));
    const port = {
      listSchedules: vi.fn(async () => []),
      listTaskRuns: vi.fn(async () => []),
      searchAgentSessions: vi.fn(async () => ({ items: [], nextCursor: null })),
      listProjectGitRefs: vi.fn(async () => []),
      listMcpServers: vi.fn(async () => []),
      listMcpTools: vi.fn(async () => []),
      createSchedule,
    } as unknown as WorkbenchCommandPort;
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="zh-CN">
          <TaskCenter commandPort={port} skills={[]} onOpenSession={() => undefined} />
        </I18nProvider>
      ),
      root,
    );

    await Promise.resolve();
    (root.querySelector('[data-testid="task-create-toggle"]') as HTMLButtonElement).click();
    const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
    textarea.value = "汇总今天的会议记录";
    textarea.dispatchEvent(new InputEvent("input", { bubbles: true }));
    (root.querySelector("form") as HTMLFormElement).dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );
    await vi.waitFor(() => expect(createSchedule).toHaveBeenCalledTimes(1));
    const request = createSchedule.mock.calls[0]![0];
    expect(request.definition.contextTemplate).toEqual({
      kind: "workspace",
      workspace: { kind: "managed" },
      conversation_mode: "per_run_session",
    });
    expect(request.definition.entryProfile).toBe("workbench");
    expect(request.definition.workloadOverride).toBeNull();
    expect(request.definition.permissionPolicy.level).toBe("read_only");
    expect(request.definition.schedule.kind).toBe("cron");
    dispose();
  });

  it("edits a prompt without replacing or reauthorizing the existing capability scope", async () => {
    const schedule = {
      id: "schedule-1",
      name: "Daily summary",
      enabled: true,
      prompt: "Old prompt",
      schedule: { kind: "cron", expression: "0 0 9 * * *", timezone: "UTC" },
      entryProfile: "workbench",
      workloadOverride: null,
      contextTemplate: {
        kind: "workspace",
        workspace: { kind: "managed" },
        conversation_mode: "per_run_session",
      },
      skillAllowlist: [],
      mcpToolAllowlist: [],
      permissionPolicy: createPermissionPolicy("read_only"),
      permissionRevision: 1,
      timeoutMs: 120_000,
      misfirePolicy: "skip",
      deliveryPolicy: "task_tab_only",
      configRevision: 1,
      createdBy: "user",
      nextRunAtMs: Date.now() + 60_000,
      health: "healthy",
      healthReason: null,
      createdAtMs: Date.now(),
      updatedAtMs: Date.now(),
    } as const;
    const updateSchedule = vi.fn(async (request) => ({
      ...request.definition,
      configRevision: 2,
    }));
    const port = {
      listSchedules: vi.fn(async () => [schedule]),
      listTaskRuns: vi.fn(async () => []),
      searchAgentSessions: vi.fn(async () => ({ items: [], nextCursor: null })),
      listProjectGitRefs: vi.fn(async () => []),
      listMcpServers: vi.fn(async () => []),
      listMcpTools: vi.fn(async () => []),
      updateSchedule,
    } as unknown as WorkbenchCommandPort;
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="zh-CN">
          <TaskCenter commandPort={port} skills={[]} onOpenSession={() => undefined} />
        </I18nProvider>
      ),
      root,
    );

    const editButton = await vi.waitFor(() => {
      const button = root.querySelector('[data-testid="task-edit"]');
      expect(button).toBeTruthy();
      return button as HTMLButtonElement;
    });
    editButton.click();
    const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
    textarea.value = "New prompt";
    textarea.dispatchEvent(new InputEvent("input", { bubbles: true }));
    (root.querySelector("form") as HTMLFormElement).dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );

    await vi.waitFor(() => expect(updateSchedule).toHaveBeenCalledTimes(1));
    expect(updateSchedule.mock.calls[0]![0].definition.id).toBe(schedule.id);
    expect(updateSchedule.mock.calls[0]![0].definition.prompt).toBe("New prompt");
    dispose();
  });

  it("updates system notification delivery without reauthorizing the capability scope", async () => {
    const schedule = scheduleFixture(Date.now());
    const updateSchedule = vi.fn(async (request) => ({
      ...request.definition,
      configRevision: request.definition.configRevision + 1,
    }));
    const port = {
      listSchedules: vi.fn(async () => [schedule]),
      listTaskRuns: vi.fn(async () => []),
      searchAgentSessions: vi.fn(async () => ({ items: [], nextCursor: null })),
      listProjectGitRefs: vi.fn(async () => []),
      listMcpServers: vi.fn(async () => []),
      listMcpTools: vi.fn(async () => []),
      updateSchedule,
    } as unknown as WorkbenchCommandPort;
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="zh-CN">
          <TaskCenter commandPort={port} skills={[]} onOpenSession={() => undefined} />
        </I18nProvider>
      ),
      root,
    );

    const editButton = await vi.waitFor(() => {
      const button = root.querySelector('[data-testid="task-edit"]');
      expect(button).toBeTruthy();
      return button as HTMLButtonElement;
    });
    editButton.click();
    const delivery = root.querySelector(
      '[data-testid="task-delivery-policy"]',
    ) as HTMLSelectElement;
    delivery.value = "task_tab_and_system_notification";
    delivery.dispatchEvent(new Event("change", { bubbles: true }));
    (root.querySelector("form") as HTMLFormElement).dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );

    await vi.waitFor(() => expect(updateSchedule).toHaveBeenCalledTimes(1));
    expect(updateSchedule.mock.calls[0]![0].definition.deliveryPolicy).toBe(
      "task_tab_and_system_notification",
    );
    dispose();
  });

  it("pins a selected MCP side effect to server, tool and schema in the Schedule definition", async () => {
    const createSchedule = vi.fn(async (request) => ({
      definition: request.definition,
      recentRuns: [],
    }));
    const server = {
      configuration: {
        id: "office-mcp",
        displayName: "Office fixture",
        enabled: true,
        transport: { kind: "streamable_http", url: "http://127.0.0.1:1234/mcp" },
        headers: [],
        readOnlyTools: ["create_document"],
        startupTimeoutMs: 1_000,
        requestTimeoutMs: 1_000,
        maxMessageBytes: 65_536,
        createdAtMs: 1,
        updatedAtMs: 1,
      },
      health: {
        serverId: "office-mcp",
        state: "ready",
        serverName: "office-fixture",
        serverVersion: "1.0.0",
        protocolVersion: "2025-06-18",
        toolCount: 2,
        errorCode: null,
        checkedAtMs: 1,
      },
    } as const;
    const tools = [
      {
        serverId: "office-mcp",
        name: "create_document",
        exposedName: "mcp_office_create_document",
        description: "Create a document",
        inputSchema: { type: "object" },
        requiredParameters: [],
        enabled: true,
        stale: false,
        validationError: null,
        schemaHash: "create-schema",
        hostIdentityHash: "office-host-v1",
        discoveredAtMs: 1,
      },
      {
        serverId: "office-mcp",
        name: "send_document",
        exposedName: "mcp_office_send_document",
        description: "Send a document",
        inputSchema: { type: "object" },
        requiredParameters: [],
        enabled: true,
        stale: false,
        validationError: null,
        schemaHash: "send-schema",
        hostIdentityHash: "office-host-v1",
        discoveredAtMs: 1,
      },
    ] as const;
    const port = {
      listSchedules: vi.fn(async () => []),
      listTaskRuns: vi.fn(async () => []),
      searchAgentSessions: vi.fn(async () => ({ items: [], nextCursor: null })),
      listProjectGitRefs: vi.fn(async () => []),
      listMcpServers: vi.fn(async () => [server]),
      listMcpTools: vi.fn(async () => [...tools]),
      createSchedule,
    } as unknown as WorkbenchCommandPort;
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="zh-CN">
          <TaskCenter commandPort={port} skills={[]} onOpenSession={() => undefined} />
        </I18nProvider>
      ),
      root,
    );

    await vi.waitFor(() => expect(port.listMcpTools).toHaveBeenCalledWith("office-mcp"));
    (root.querySelector('[data-testid="task-create-toggle"]') as HTMLButtonElement).click();
    const permissionLevel = root.querySelector<HTMLSelectElement>('select[aria-label="权限档位"]')!;
    permissionLevel.value = "writable";
    permissionLevel.dispatchEvent(new Event("change", { bubbles: true }));
    const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
    textarea.value = "Create and send the report";
    textarea.dispatchEvent(new InputEvent("input", { bubbles: true }));
    const sendLabel = [...root.querySelectorAll("label")].find((label) =>
      label.textContent?.includes("send_document"),
    );
    expect(sendLabel).toBeTruthy();
    (sendLabel!.querySelector('input[type="checkbox"]') as HTMLInputElement).click();
    (root.querySelector("form") as HTMLFormElement).dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );

    await vi.waitFor(() => expect(createSchedule).toHaveBeenCalledTimes(1));
    const definition = createSchedule.mock.calls[0]![0].definition;
    expect(definition.mcpToolAllowlist).toEqual([
      {
        serverId: "office-mcp",
        toolName: "send_document",
        schemaHash: "send-schema",
        hostIdentityHash: "office-host-v1",
      },
    ]);
    expect(definition.permissionPolicy.level).toBe("writable");
    expect(definition.permissionPolicy.rules.mcp).toEqual([
      {
        serverId: "office-mcp",
        toolName: "send_document",
        schemaHash: "send-schema",
        readOnly: false,
      },
    ]);
    dispose();

    createSchedule.mockClear();
    vi.mocked(port.listMcpTools).mockClear();
    const fullAccessRoot = document.createElement("div");
    document.body.append(fullAccessRoot);
    const disposeFullAccess = render(
      () => (
        <I18nProvider initialLocale="zh-CN">
          <TaskCenter commandPort={port} skills={[]} onOpenSession={() => undefined} />
        </I18nProvider>
      ),
      fullAccessRoot,
    );
    await vi.waitFor(() => expect(port.listMcpTools).toHaveBeenCalledWith("office-mcp"));
    (
      fullAccessRoot.querySelector('[data-testid="task-create-toggle"]') as HTMLButtonElement
    ).click();
    const fullAccessLevel = fullAccessRoot.querySelector<HTMLSelectElement>(
      'select[aria-label="权限档位"]',
    )!;
    fullAccessLevel.value = "writable";
    fullAccessLevel.dispatchEvent(new Event("change", { bubbles: true }));
    const fullAccessSendLabel = [...fullAccessRoot.querySelectorAll("label")].find((label) =>
      label.textContent?.includes("send_document"),
    )!;
    (fullAccessSendLabel.querySelector('input[type="checkbox"]') as HTMLInputElement).click();
    fullAccessLevel.value = "full_access";
    fullAccessLevel.dispatchEvent(new Event("change", { bubbles: true }));
    expect(fullAccessRoot.querySelector('[data-testid="task-mcp-tools"]')).toBeNull();
    const fullAccessPrompt = fullAccessRoot.querySelector("textarea") as HTMLTextAreaElement;
    fullAccessPrompt.value = "Use any configured integration";
    fullAccessPrompt.dispatchEvent(new InputEvent("input", { bubbles: true }));
    (fullAccessRoot.querySelector("form") as HTMLFormElement).dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );
    await vi.waitFor(() => expect(createSchedule).toHaveBeenCalledTimes(1));
    const fullAccessDefinition = createSchedule.mock.calls[0]![0].definition;
    expect(fullAccessDefinition.permissionPolicy.level).toBe("full_access");
    expect(fullAccessDefinition.mcpToolAllowlist).toEqual([]);
    expect(fullAccessDefinition.permissionPolicy.rules.mcp).toEqual([]);
    expect(fullAccessDefinition.hostRevisionSnapshot.connectors).toEqual([]);
    disposeFullAccess();
  });

  it("serializes an advanced Cron expression with the local IANA timezone", async () => {
    const createSchedule = vi.fn(async (request) => ({
      definition: request.definition,
      recentRuns: [],
    }));
    const port = {
      listSchedules: vi.fn(async () => []),
      listTaskRuns: vi.fn(async () => []),
      searchAgentSessions: vi.fn(async () => ({ items: [], nextCursor: null })),
      listProjectGitRefs: vi.fn(async () => []),
      listMcpServers: vi.fn(async () => []),
      listMcpTools: vi.fn(async () => []),
      createSchedule,
    } as unknown as WorkbenchCommandPort;
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="zh-CN">
          <TaskCenter commandPort={port} skills={[]} onOpenSession={() => undefined} />
        </I18nProvider>
      ),
      root,
    );

    await Promise.resolve();
    (root.querySelector('[data-testid="task-create-toggle"]') as HTMLButtonElement).click();
    const frequency = [...root.querySelectorAll("label")]
      .find((label) => label.textContent?.includes("频率"))
      ?.querySelector("select") as HTMLSelectElement;
    frequency.value = "cron";
    frequency.dispatchEvent(new Event("change", { bubbles: true }));
    await Promise.resolve();
    const cron = [...root.querySelectorAll("label")]
      .find((label) => label.textContent?.trim().startsWith("Cron"))
      ?.querySelector("input") as HTMLInputElement;
    cron.value = "0 15 9 * * 1-5";
    cron.dispatchEvent(new InputEvent("input", { bubbles: true }));
    const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
    textarea.value = "Generate the weekday report";
    textarea.dispatchEvent(new InputEvent("input", { bubbles: true }));
    (root.querySelector("form") as HTMLFormElement).dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );

    await vi.waitFor(() => expect(createSchedule).toHaveBeenCalledTimes(1));
    const schedule = createSchedule.mock.calls[0]![0].definition.schedule;
    expect(schedule).toEqual({
      kind: "cron",
      expression: "0 15 9 * * 1-5",
      timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC",
    });
    dispose();
  });

  it("routes NeedsAttention to a fresh interactive continuation and retries only retryable rows", async () => {
    const now = Date.now();
    const schedule = scheduleFixture(now);
    const needsAttention = taskFixture("task-attention", "needs_attention", now);
    const cancelled = taskFixture("task-cancelled", "cancelled", now - 1);
    const continueTaskInteractively = vi.fn(async (context: unknown, taskRunId: string) => {
      void context;
      void taskRunId;
      return {
        taskRun: needsAttention,
        session: { id: "continuation-session" },
        run: { id: "continuation-run", generation: 1 },
      };
    });
    const retryTaskRun = vi.fn(async (context: unknown, taskRunId: string) => {
      void context;
      void taskRunId;
      return taskFixture("task-retry", "queued", now + 1);
    });
    const onOpenSession = vi.fn();
    const port = {
      listSchedules: vi.fn(async () => [schedule]),
      listTaskRuns: vi.fn(async () => [needsAttention, cancelled]),
      searchAgentSessions: vi.fn(async () => ({ items: [], nextCursor: null })),
      listProjectGitRefs: vi.fn(async () => []),
      listMcpServers: vi.fn(async () => []),
      listMcpTools: vi.fn(async () => []),
      continueTaskInteractively,
      retryTaskRun,
    } as unknown as WorkbenchCommandPort;
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="zh-CN">
          <TaskCenter commandPort={port} skills={[]} onOpenSession={onOpenSession} />
        </I18nProvider>
      ),
      root,
    );

    const history = await vi.waitFor(() => {
      const button = root.querySelector('[data-testid="task-history"]');
      expect(button).toBeTruthy();
      return button as HTMLButtonElement;
    });
    history.click();

    const continuation = await vi.waitFor(() => {
      const button = root.querySelector('[data-testid="task-continue"]');
      expect(button).toBeTruthy();
      return button as HTMLButtonElement;
    });
    continuation.click();
    await vi.waitFor(() => expect(continueTaskInteractively).toHaveBeenCalledTimes(1));
    expect(continueTaskInteractively.mock.calls[0]![1]).toBe("task-attention");
    await vi.waitFor(() => expect(onOpenSession).toHaveBeenCalledWith("continuation-session"));

    history.click();
    const retry = root.querySelector('[data-testid="task-retry"]') as HTMLButtonElement;
    retry.click();
    await vi.waitFor(() => expect(retryTaskRun).toHaveBeenCalledTimes(1));
    expect(retryTaskRun.mock.calls[0]![1]).toBe("task-cancelled");
    dispose();
  });

  it("keeps card actions integrated and opens a completed run in its session", async () => {
    const now = Date.now();
    const schedule = scheduleFixture(now);
    const completed = taskFixture("task-completed", "succeeded", now, "session-completed");
    const setScheduleEnabled = vi.fn(
      async (context: unknown, scheduleId: string, enabled: boolean) => {
        void context;
        void scheduleId;
        void enabled;
        return { ...schedule, enabled: false };
      },
    );
    const removeSchedule = vi.fn(async (context: unknown, scheduleId: string) => {
      void context;
      void scheduleId;
      return true;
    });
    const onOpenSession = vi.fn();
    const port = {
      listSchedules: vi.fn(async () => [schedule]),
      listTaskRuns: vi.fn(async () => [completed]),
      searchAgentSessions: vi.fn(async () => ({ items: [], nextCursor: null })),
      listProjectGitRefs: vi.fn(async () => []),
      listMcpServers: vi.fn(async () => []),
      listMcpTools: vi.fn(async () => []),
      setScheduleEnabled,
      removeSchedule,
    } as unknown as WorkbenchCommandPort;
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="zh-CN">
          <TaskCenter commandPort={port} skills={[]} onOpenSession={onOpenSession} />
        </I18nProvider>
      ),
      root,
    );

    const toggle = await vi.waitFor(() => {
      const input = root.querySelector('[data-testid="task-toggle-enabled"]');
      expect(input).toBeTruthy();
      return input as HTMLInputElement;
    });
    toggle.click();
    await vi.waitFor(() => expect(setScheduleEnabled).toHaveBeenCalledTimes(1));
    expect(setScheduleEnabled.mock.calls[0]![2]).toBe(false);

    (root.querySelector('[data-testid="task-history"]') as HTMLButtonElement).click();
    await vi.waitFor(() => {
      expect(root.querySelector('[data-testid="task-run-trigger"]')?.textContent).toBe("手动执行");
      expect(root.querySelector('[data-testid="task-run-duration"]')?.textContent).toContain(
        "2 秒",
      );
    });
    (root.querySelector('[data-testid="task-open-session"]') as HTMLButtonElement).click();
    expect(onOpenSession).toHaveBeenCalledWith("session-completed");

    const deleteButton = await vi.waitFor(() => {
      const button = root.querySelector('[data-testid="task-delete"]') as HTMLButtonElement;
      expect(button.disabled).toBe(false);
      return button;
    });
    deleteButton.click();
    const confirmDelete = await vi.waitFor(() => {
      const button = root.querySelector('[data-testid="task-delete-confirm"]');
      expect(button).toBeTruthy();
      return button as HTMLButtonElement;
    });
    confirmDelete.click();
    await vi.waitFor(() => expect(removeSchedule).toHaveBeenCalledTimes(1));
    dispose();
  });
});

function scheduleFixture(now: number) {
  return {
    id: "schedule-advanced",
    name: "Advanced lifecycle",
    enabled: true,
    prompt: "Run the advanced lifecycle fixture",
    schedule: { kind: "cron", expression: "0 0 9 * * *", timezone: "UTC" },
    entryProfile: "workbench",
    workloadOverride: null,
    contextTemplate: {
      kind: "workspace",
      workspace: { kind: "managed" },
      conversation_mode: "per_run_session",
    },
    skillAllowlist: [],
    mcpToolAllowlist: [],
    permissionPolicy: createPermissionPolicy("read_only"),
    permissionRevision: 1,
    timeoutMs: 120_000,
    misfirePolicy: "skip",
    deliveryPolicy: "task_tab_only",
    configRevision: 1,
    createdBy: "user:test",
    nextRunAtMs: now + 60_000,
    health: "healthy",
    healthReason: null,
    createdAtMs: now,
    updatedAtMs: now,
  } as const;
}

function taskFixture(
  id: string,
  status: "needs_attention" | "cancelled" | "queued" | "succeeded",
  now: number,
  executionSessionId: string | null = null,
) {
  return {
    id,
    scheduleId: "schedule-advanced",
    scheduleRevision: 1,
    trigger: "manual",
    scheduledForMs: now,
    invocationKey: `fixture:${id}`,
    requesterSessionId: null,
    executionSessionId,
    runId: null,
    permissionSnapshotHash: "fixture-scope",
    status,
    progressPercent: null,
    resultSummary: null,
    errorCode: status === "needs_attention" ? "schedule_schema_changed" : "task_cancelled",
    errorSummary: status === "needs_attention" ? "Pinned extension changed" : "Cancelled",
    artifactIds: [],
    deliveryStatus: "not_requested",
    deliveryErrorCode: null,
    createdAtMs: now,
    startedAtMs: status === "queued" ? null : now - 2_000,
    finishedAtMs: now,
    updatedAtMs: now,
  } as const;
}
