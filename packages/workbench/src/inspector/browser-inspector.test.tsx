import type {
  BrowserWorkspace,
  BrowserWorkspaceMutation,
  EmbeddedBrowserPermissionRequiredEvent,
  WorkbenchSessionSnapshot,
} from "@hachimi/contracts";
import type { JSX } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { BrowserShortcutRequested, WorkbenchCommandPort } from "../workbench-command-port";
import { BrowserInspector } from "./browser-inspector";

vi.mock("@hachimi/ui", () => {
  const Icon = () => <span aria-hidden="true" />;
  return {
    ArrowLeft: Icon,
    ArrowRight: Icon,
    ExternalLink: Icon,
    Globe: Icon,
    Hand: Icon,
    Plus: Icon,
    Play: Icon,
    RefreshCw: Icon,
    Send: Icon,
    ShieldAlert: Icon,
    ShieldCheck: Icon,
    Square: Icon,
    X: Icon,
    Button: (props: {
      children?: JSX.Element;
      class?: string;
      disabled?: boolean;
      title?: string;
      "aria-label"?: string;
      "aria-selected"?: boolean;
      "data-testid"?: string;
      role?: JSX.HTMLAttributes<HTMLButtonElement>["role"];
      onClick?: () => void;
    }) => (
      <button
        class={props.class}
        disabled={props.disabled}
        title={props.title}
        aria-label={props["aria-label"]}
        aria-selected={props["aria-selected"]}
        data-testid={props["data-testid"]}
        role={props.role}
        onClick={() => props.onClick?.()}
      >
        {props.children}
      </button>
    ),
    TextField: (props: {
      label: string;
      testId?: string;
      value?: string;
      placeholder?: string;
      ref?: (element: HTMLInputElement) => void;
      onFocus?: JSX.EventHandler<HTMLInputElement, FocusEvent>;
      onBlur?: JSX.EventHandler<HTMLInputElement, FocusEvent>;
      onInput?: JSX.EventHandler<HTMLInputElement, InputEvent>;
      onKeyDown?: JSX.EventHandler<HTMLInputElement, KeyboardEvent>;
    }) => (
      <label>
        {props.label}
        <input
          ref={props.ref}
          data-testid={props.testId}
          value={props.value}
          placeholder={props.placeholder}
          onFocus={props.onFocus}
          onBlur={props.onBlur}
          onInput={props.onInput}
          onKeyDown={props.onKeyDown}
        />
      </label>
    ),
  };
});

function tab(id: string, url = "about:blank") {
  return {
    id,
    workspaceId: "workspace-1",
    url,
    title: url === "about:blank" ? "" : "Example",
    faviconToken: null,
    loading: false,
    canGoBack: false,
    canGoForward: false,
    runtimeLoaded: true,
    navigationError: null,
    revision: 1,
    inputEpoch: 1,
    createdAtMs: 1,
    updatedAtMs: 1,
  } as BrowserWorkspace["tabs"][number];
}

function automationLease(status: "active" | "suspended") {
  return {
    id: "lease-1",
    surface: "embedded",
    workspaceId: "workspace-1",
    tabId: "tab-1",
    ownerSessionId: "session-1",
    ownerRunId: "run-1",
    runGeneration: 1,
    capabilities: ["observe", "act"],
    status,
    revision: 1,
    expiresAtMs: Date.now() + 60_000,
    createdAtMs: 1,
    updatedAtMs: 1,
  } as NonNullable<BrowserWorkspace["automationLease"]>;
}

function createHarness(leaseStatus?: "active" | "suspended") {
  let nextTab = 2;
  let shortcutHandler: ((event: BrowserShortcutRequested) => void) | undefined;
  let permissionHandler: ((event: EmbeddedBrowserPermissionRequiredEvent) => void) | undefined;
  let workspace: BrowserWorkspace = {
    id: "workspace-1",
    ownerSessionId: "session-1",
    activeTabId: "tab-1",
    runtimeState: "ready",
    tabs: [tab("tab-1", "https://example.com/")],
    automationLease: leaseStatus ? automationLease(leaseStatus) : null,
    revision: 1,
    updatedAtMs: 1,
  };
  const mutations: BrowserWorkspaceMutation[] = [];
  const mutateBrowserWorkspace = vi.fn(async (request: { mutation: BrowserWorkspaceMutation }) => {
    mutations.push(request.mutation);
    const mutation = request.mutation;
    if (mutation.kind === "new_tab") {
      const id = `tab-${nextTab++}`;
      workspace = {
        ...workspace,
        activeTabId: id,
        tabs: [...workspace.tabs, tab(id)],
        revision: workspace.revision + 1,
      };
    } else if (mutation.kind === "activate_tab") {
      workspace = { ...workspace, activeTabId: mutation.tab_id, revision: workspace.revision + 1 };
    } else if (mutation.kind === "close_tab") {
      const tabs = workspace.tabs.filter((entry) => entry.id !== mutation.tab_id);
      workspace = {
        ...workspace,
        tabs,
        activeTabId: tabs[0]?.id ?? "tab-replacement",
        revision: workspace.revision + 1,
      };
    } else if (mutation.kind === "take_over_automation" && workspace.automationLease) {
      workspace = {
        ...workspace,
        automationLease: { ...workspace.automationLease, status: "suspended" },
        revision: workspace.revision + 1,
      };
    } else if (mutation.kind === "resume_automation" && workspace.automationLease) {
      workspace = {
        ...workspace,
        automationLease: { ...workspace.automationLease, status: "active" },
        revision: workspace.revision + 1,
      };
    }
    return workspace;
  });
  const commandPort = {
    openBrowserWorkspace: vi.fn(async () => workspace),
    mutateBrowserWorkspace,
    updateBrowserSurfaceLayout: vi.fn(async () => undefined),
    getBrowserHistory: vi.fn(async () => []),
    listEmbeddedBrowserPermissionRequests: vi.fn(async () => []),
    resolveEmbeddedBrowserPermission: vi.fn(async (request) => ({
      id: request.requestId,
      workspaceId: "workspace-1",
      tabId: "tab-1",
      automationLeaseId: "lease-1",
      ownerSessionId: "session-1",
      ownerRunId: "run-1",
      runGeneration: 1,
      origin: "https://example.com",
      capabilities: ["observe", "act"],
      privateNetwork: false,
      status: request.decision === "deny" ? "denied" : "allowed",
      expectedTabRevision: 1,
      createdAtMs: 1,
      expiresAtMs: Date.now() + 60_000,
    })),
    openSystemBrowser: vi.fn(async () => undefined),
    onBrowserTabStateChange: vi.fn(async () => () => undefined),
    onBrowserWorkspaceChange: vi.fn(async () => () => undefined),
    onBrowserShortcutRequested: vi.fn(
      async (handler: (event: BrowserShortcutRequested) => void) => {
        shortcutHandler = handler;
        return () => undefined;
      },
    ),
    onBrowserPermissionRequired: vi.fn(
      async (handler: (event: EmbeddedBrowserPermissionRequiredEvent) => void) => {
        permissionHandler = handler;
        return () => undefined;
      },
    ),
    onBrowserRuntimeCrash: vi.fn(async () => () => undefined),
  } as unknown as WorkbenchCommandPort;
  const snapshot = { session: { id: "session-1" } } as WorkbenchSessionSnapshot;
  return {
    commandPort,
    mutateBrowserWorkspace,
    mutations,
    snapshot,
    emitShortcut: (event: BrowserShortcutRequested) => shortcutHandler?.(event),
    emitPermission: (event: EmbeddedBrowserPermissionRequiredEvent) => permissionHandler?.(event),
  };
}

beforeEach(() => {
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    callback(0);
    return 1;
  });
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
});

afterEach(() => {
  document.body.replaceChildren();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("BrowserInspector workspace", () => {
  it("opens a persistent workspace without requiring an active Run", async () => {
    const harness = createHarness();
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <BrowserInspector
          snapshot={harness.snapshot}
          commandPort={harness.commandPort}
          locale="zh-CN"
        />
      ),
      host,
    );

    await vi.waitFor(() =>
      expect(harness.commandPort.openBrowserWorkspace).toHaveBeenCalledWith("session-1", null),
    );
    expect(host.querySelector('[data-testid="browser-native-surface"]')).toBeTruthy();
    expect(host.querySelector('[data-testid="browser-tab-tab-1"]')).toBeTruthy();
    expect(harness.commandPort.updateBrowserSurfaceLayout).toHaveBeenCalled();
    dispose();
  });

  it("creates, switches, and closes persisted CEF tabs", async () => {
    const harness = createHarness();
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <BrowserInspector
          snapshot={harness.snapshot}
          commandPort={harness.commandPort}
          locale="zh-CN"
        />
      ),
      host,
    );

    await vi.waitFor(() =>
      expect(host.querySelector('[data-testid="browser-tab-tab-1"]')).toBeTruthy(),
    );
    host.querySelector<HTMLButtonElement>('[data-testid="browser-new-tab"]')?.click();
    await vi.waitFor(() =>
      expect(host.querySelector('[data-testid="browser-tab-tab-2"]')).toBeTruthy(),
    );
    host.querySelector<HTMLButtonElement>('[data-testid="browser-tab-tab-1"]')?.click();
    await vi.waitFor(() =>
      expect(harness.mutations.some((item) => item.kind === "activate_tab")).toBe(true),
    );
    host.querySelector<HTMLButtonElement>('[data-testid="browser-tab-close-tab-2"]')?.click();
    await vi.waitFor(() =>
      expect(host.querySelector('[data-testid="browser-tab-tab-2"]')).toBeNull(),
    );
    expect(harness.mutations.map((item) => item.kind)).toEqual([
      "new_tab",
      "activate_tab",
      "close_tab",
    ]);
    dispose();
  });

  it("restores a stable source tab and opens unbound sources in a new tab", async () => {
    const restored = createHarness();
    const restoredHost = document.createElement("div");
    document.body.append(restoredHost);
    const disposeRestored = render(
      () => (
        <BrowserInspector
          snapshot={restored.snapshot}
          commandPort={restored.commandPort}
          locale="zh-CN"
          browserTabId="tab-1"
          initialUrl="https://example.com/"
        />
      ),
      restoredHost,
    );
    await vi.waitFor(() => expect(restored.commandPort.openBrowserWorkspace).toHaveBeenCalled());
    expect(restored.mutations).toEqual([]);
    disposeRestored();

    const unbound = createHarness();
    const unboundHost = document.createElement("div");
    document.body.append(unboundHost);
    const disposeUnbound = render(
      () => (
        <BrowserInspector
          snapshot={unbound.snapshot}
          commandPort={unbound.commandPort}
          locale="zh-CN"
          initialUrl="https://docs.example.com/guide"
        />
      ),
      unboundHost,
    );
    await vi.waitFor(() =>
      expect(unbound.mutations).toContainEqual({
        kind: "new_tab",
        url: "https://docs.example.com/guide",
      }),
    );
    disposeUnbound();
  });

  it("lets the user suspend and explicitly resume the active Agent lease", async () => {
    const harness = createHarness("active");
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <BrowserInspector
          snapshot={harness.snapshot}
          commandPort={harness.commandPort}
          locale="zh-CN"
        />
      ),
      host,
    );

    await vi.waitFor(() =>
      expect(host.querySelector('[data-testid="browser-take-over"]')).toBeTruthy(),
    );
    host.querySelector<HTMLButtonElement>('[data-testid="browser-take-over"]')?.click();
    await vi.waitFor(() =>
      expect(host.querySelector('[data-testid="browser-resume-agent"]')).toBeTruthy(),
    );
    host.querySelector<HTMLButtonElement>('[data-testid="browser-resume-agent"]')?.click();
    await vi.waitFor(() =>
      expect(harness.mutations.map((item) => item.kind)).toEqual([
        "take_over_automation",
        "resume_automation",
      ]),
    );
    dispose();
  });

  it("handles browser shortcuts forwarded while the native CEF surface has focus", async () => {
    const harness = createHarness();
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <BrowserInspector
          snapshot={harness.snapshot}
          commandPort={harness.commandPort}
          locale="zh-CN"
        />
      ),
      host,
    );
    await vi.waitFor(() => expect(harness.commandPort.openBrowserWorkspace).toHaveBeenCalled());

    harness.emitShortcut({
      kind: "shortcut_requested",
      tab_id: "tab-1",
      shortcut: "focus_address",
    });
    expect(document.activeElement).toBe(host.querySelector('[data-testid="browser-address"]'));
    harness.emitShortcut({
      kind: "shortcut_requested",
      tab_id: "tab-1",
      shortcut: "new_tab",
    });
    await vi.waitFor(() =>
      expect(harness.mutations.some((mutation) => mutation.kind === "new_tab")).toBe(true),
    );
    dispose();
  });

  it("resolves Agent-only embedded site permission requests in the browser panel", async () => {
    const harness = createHarness();
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <BrowserInspector
          snapshot={harness.snapshot}
          commandPort={harness.commandPort}
          locale="zh-CN"
        />
      ),
      host,
    );
    await vi.waitFor(() => expect(harness.commandPort.openBrowserWorkspace).toHaveBeenCalled());
    harness.emitPermission({
      reasonCode: "agent_site_permission_required",
      request: {
        id: "permission-1",
        workspaceId: "workspace-1",
        tabId: "tab-1",
        automationLeaseId: "lease-1",
        ownerSessionId: "session-1",
        ownerRunId: "run-1",
        runGeneration: 1,
        origin: "https://example.com",
        capabilities: ["observe", "act"],
        privateNetwork: false,
        status: "pending",
        expectedTabRevision: 1,
        createdAtMs: 1,
        expiresAtMs: Date.now() + 60_000,
      },
    });
    await vi.waitFor(() => expect(host.textContent).toContain("Agent 请求访问此网站"));
    await vi.waitFor(() =>
      expect(
        host.querySelector<HTMLButtonElement>('[data-testid="browser-permission-allow-session"]')
          ?.disabled,
      ).toBe(false),
    );
    host
      .querySelector<HTMLButtonElement>('[data-testid="browser-permission-allow-session"]')!
      .click();
    await vi.waitFor(() =>
      expect(harness.commandPort.resolveEmbeddedBrowserPermission).toHaveBeenCalledWith({
        requestId: "permission-1",
        decision: "allow_session",
      }),
    );
    dispose();
  });

  it("keeps the system-browser action in the toolbar without a downloads entry", async () => {
    const harness = createHarness();
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <BrowserInspector
          snapshot={harness.snapshot}
          commandPort={harness.commandPort}
          locale="zh-CN"
        />
      ),
      host,
    );

    await vi.waitFor(() => expect(harness.commandPort.openBrowserWorkspace).toHaveBeenCalled());
    expect(host.querySelector('[aria-label="下载"]')).toBeNull();
    host.querySelector<HTMLButtonElement>('[aria-label="在系统浏览器打开"]')?.click();
    await vi.waitFor(() =>
      expect(harness.commandPort.openSystemBrowser).toHaveBeenCalledWith("https://example.com/"),
    );
    dispose();
  });
});
