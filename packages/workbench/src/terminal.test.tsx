import type {
  ProcessOutputChunk,
  ProcessSessionRecord,
  WorkbenchSessionSnapshot,
} from "@hachimi/contracts";
import { I18nProvider } from "@hachimi/i18n";
import type { JSX } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { TerminalPanel } from "./terminal";
import type { WorkbenchCommandPort } from "./workbench-command-port";

const xtermHarness = vi.hoisted(() => ({
  instances: [] as Array<{
    rows: number;
    cols: number;
    writes: Uint8Array[];
    emitData: (data: string) => void;
  }>,
}));

vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    rows = 20;
    cols = 100;
    writes: Uint8Array[] = [];
    private dataHandlers = new Set<(data: string) => void>();
    constructor() {
      xtermHarness.instances.push(this);
    }
    loadAddon() {}
    open(element: HTMLElement) {
      element.append(document.createElement("textarea"));
    }
    onData(handler: (data: string) => void) {
      this.dataHandlers.add(handler);
      return { dispose: () => this.dataHandlers.delete(handler) };
    }
    emitData(data: string) {
      for (const handler of this.dataHandlers) handler(data);
    }
    write(bytes: Uint8Array) {
      this.writes.push(bytes);
    }
    focus() {}
    dispose() {}
  },
}));

vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    fit() {}
  },
}));

vi.mock("@hachimi/ui", () => {
  const Icon = () => <span aria-hidden="true" />;
  return {
    Plus: Icon,
    Square: Icon,
    TerminalSquare: Icon,
    X: Icon,
    Button: (props: {
      children?: JSX.Element;
      class?: string;
      role?: JSX.HTMLAttributes<HTMLButtonElement>["role"];
      disabled?: boolean;
      title?: string;
      "aria-label"?: string;
      "aria-pressed"?: boolean;
      onClick?: () => void;
    }) => (
      <button
        class={props.class}
        role={props.role}
        disabled={props.disabled}
        title={props.title}
        aria-label={props["aria-label"]}
        aria-pressed={props["aria-pressed"]}
        onClick={() => props.onClick?.()}
      >
        {props.children}
      </button>
    ),
  };
});

function snapshot(suffix = "default"): WorkbenchSessionSnapshot {
  return {
    session: {
      id: `session-terminal-${suffix}`,
      context: { kind: "project", project_id: "project-1", checkout_id: "checkout-1" },
      entryProfile: "workbench",
      title: "Terminal",
      archived: false,
      pinned: false,
      parentSessionId: null,
      sourceRunId: null,
      createdAtMs: 1,
      updatedAtMs: 1,
    },
    checkout: {
      id: "checkout-1",
      projectId: "project-1",
      kind: "local",
      path: "D:\\workspace\\hachimi",
      baseRevision: null,
      headRevision: null,
      status: "ready",
      pinned: false,
      createdAtMs: 1,
      updatedAtMs: 1,
    },
    runs: [
      {
        id: `run-terminal-${suffix}`,
        sessionId: `session-terminal-${suffix}`,
        origin: { kind: "interactive" },
        purpose: "task",
        status: "running",
        generation: 7,
        configuration: {},
        createdAtMs: 1,
        updatedAtMs: 1,
      } as unknown as WorkbenchSessionSnapshot["runs"][number],
    ],
    events: [],
    transcript: [],
    attachments: [],
    pendingApprovals: [],
    planDocuments: [],
    planConfirmations: [],
    executionPlans: [],
    artifacts: [],
    agentTasks: [],
    runSummaries: [],
    browserSessions: [],
    browserAutomationLeases: [],
    externalBrowserObservations: [],
    hostAccessRequests: [],
    computerControlSessions: [],
    sources: [],
  };
}

function process(
  suffix = "default",
  index = 1,
  status: ProcessSessionRecord["status"] = "running",
): ProcessSessionRecord {
  return {
    id: `process-terminal-${suffix}-${index}`,
    sessionId: `session-terminal-${suffix}`,
    runId: null,
    checkoutId: "checkout-1",
    runGeneration: null,
    ownerClientId: "window:workbench",
    commandSummary: "powershell.exe",
    interactive: true,
    status,
    exitCode: status === "exited" ? 0 : null,
    outputLimitBytes: 1024,
    createdAtMs: 1,
    updatedAtMs: 1,
    reconnectExpiresAtMs: status === "running" ? 10_000 : 2,
  };
}

function chunk(sequence: number, bytes: number[]): ProcessOutputChunk {
  return {
    sequence,
    stream: "stdout",
    deltaBase64: btoa(String.fromCharCode(...bytes)),
    capReached: false,
  };
}

async function settle() {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

beforeEach(() => {
  vi.useFakeTimers();
  xtermHarness.instances.length = 0;
  Object.defineProperty(globalThis, "ResizeObserver", {
    configurable: true,
    value: class {
      constructor(private callback: ResizeObserverCallback) {}
      observe(target: Element) {
        this.callback(
          [{ target, contentRect: { width: 800, height: 360 } } as ResizeObserverEntry],
          this as unknown as ResizeObserver,
        );
      }
      disconnect() {}
      unobserve() {}
    },
  });
});

afterEach(() => {
  vi.useRealTimers();
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

describe("TerminalPanel", () => {
  it("reconnects a PTY, streams raw output to xterm, and resizes it", async () => {
    const legacyRunTerminal = {
      ...process("legacy"),
      runId: "run-terminal-legacy",
      runGeneration: 7,
    };
    const port = {
      listProcesses: vi.fn(async () => [legacyRunTerminal, process("stream")]),
      readProcess: vi.fn(async () => ({
        process: process("stream", 1, "exited"),
        chunks: [chunk(1, [0xe4]), chunk(2, [0xbd, 0xa0]), chunk(3, [0xff])],
        nextSequence: 3,
        closed: true,
      })),
      resizeProcess: vi.fn(async () => undefined),
    } as unknown as WorkbenchCommandPort;
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="en-US">
          <TerminalPanel
            projectId="project-stream"
            snapshot={snapshot("stream")}
            commandPort={port}
          />
        </I18nProvider>
      ),
      root,
    );
    await settle();

    expect(root.querySelectorAll(".terminal-tab-select")).toHaveLength(1);
    const output = (xtermHarness.instances.at(-1)?.writes ?? []).flatMap((bytes) => [...bytes]);
    expect(new TextDecoder().decode(Uint8Array.from(output))).toBe("你�");
    expect(port.resizeProcess).toHaveBeenCalledWith(
      expect.objectContaining({ size: { rows: 20, cols: 100 } }),
    );
    dispose();
  });

  it("writes every xterm data event immediately and supports multiple shells", async () => {
    const onClose = vi.fn();
    const port = {
      listProcesses: vi.fn(async () => []),
      spawnProcess: vi
        .fn()
        .mockResolvedValueOnce(process("multi", 1))
        .mockResolvedValueOnce(process("multi", 2)),
      readProcess: vi.fn(async (request: { processSessionId: string }) => ({
        process: request.processSessionId.endsWith("2") ? process("multi", 2) : process("multi", 1),
        chunks: [],
        nextSequence: 0,
        closed: false,
      })),
      resizeProcess: vi.fn(async () => undefined),
      writeProcessStdin: vi.fn(async () => undefined),
      terminateProcess: vi.fn(async () => process("multi", 1, "terminated")),
    } as unknown as WorkbenchCommandPort;
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="en-US">
          <TerminalPanel
            projectId="project-multi"
            snapshot={snapshot("multi")}
            commandPort={port}
            onClose={onClose}
          />
        </I18nProvider>
      ),
      root,
    );
    await settle();

    expect(port.listProcesses).toHaveBeenCalledWith(
      expect.objectContaining({ sessionId: "session-terminal-multi", runId: null }),
    );
    expect(port.spawnProcess).toHaveBeenCalledWith(
      expect.objectContaining({
        sessionId: "session-terminal-multi",
        context: expect.objectContaining({ expectedRunId: null, expectedGeneration: null }),
        timeoutMs: null,
      }),
    );

    xtermHarness.instances[0]?.emitData("Write-Output 你好\r");
    await settle();
    const write = vi.mocked(port.writeProcessStdin).mock.calls[0]?.[0];
    const written = Uint8Array.from(atob(write!.deltaBase64!), (value) => value.charCodeAt(0));
    expect(new TextDecoder().decode(written)).toBe("Write-Output 你好\r");
    expect(root.textContent).not.toContain("Send");
    expect(root.textContent).not.toContain("Stop");

    root.querySelector<HTMLButtonElement>('[aria-label="Interrupt command"]')?.click();
    await settle();
    const interrupt = vi.mocked(port.writeProcessStdin).mock.calls[1]?.[0];
    const interruptBytes = Uint8Array.from(atob(interrupt!.deltaBase64!), (value) =>
      value.charCodeAt(0),
    );
    expect(new TextDecoder().decode(interruptBytes)).toBe("\x03");
    expect(interrupt?.processSessionId).toBe("process-terminal-multi-1");

    root.querySelector<HTMLButtonElement>('[aria-label="New terminal"]')?.click();
    await settle();
    expect(port.spawnProcess).toHaveBeenCalledTimes(2);
    expect(root.querySelectorAll(".terminal-tab-select")).toHaveLength(2);

    const closeButtons = root.querySelectorAll<HTMLButtonElement>('[aria-label="Close terminal"]');
    closeButtons[0]?.click();
    await settle();
    expect(port.terminateProcess).toHaveBeenCalledWith(
      expect.objectContaining({ processSessionId: "process-terminal-multi-1" }),
    );
    expect(onClose).not.toHaveBeenCalled();
    closeButtons[1]?.click();
    await settle();
    expect(root.querySelectorAll(".terminal-tab-select")).toHaveLength(0);
    expect(onClose).toHaveBeenCalledOnce();
    dispose();
  });

  it("shares one automatic launch across mounts for the same project", async () => {
    let resolveLaunch: ((record: ProcessSessionRecord) => void) | undefined;
    const launch = new Promise<ProcessSessionRecord>((resolve) => {
      resolveLaunch = resolve;
    });
    const port = {
      listProcesses: vi.fn(async () => []),
      spawnProcess: vi.fn(() => launch),
      readProcess: vi.fn(async () => ({
        process: process("shared"),
        chunks: [],
        nextSequence: 0,
        closed: false,
      })),
      resizeProcess: vi.fn(async () => undefined),
    } as unknown as WorkbenchCommandPort;
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="en-US">
          <TerminalPanel
            projectId="project-shared"
            snapshot={snapshot("shared")}
            commandPort={port}
          />
          <TerminalPanel
            projectId="project-shared"
            snapshot={snapshot("shared")}
            commandPort={port}
          />
        </I18nProvider>
      ),
      root,
    );
    await settle();
    expect(port.spawnProcess).toHaveBeenCalledOnce();
    resolveLaunch?.(process("shared"));
    await settle();
    expect(root.querySelectorAll(".terminal-tab-select")).toHaveLength(2);
    dispose();
  });
});
