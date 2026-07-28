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

vi.mock("@hachimi/ui", () => {
  const Icon = () => <span aria-hidden="true" />;
  return {
    Square: Icon,
    TerminalSquare: Icon,
    Button: (props: {
      children?: JSX.Element;
      type?: "button" | "submit";
      disabled?: boolean;
      onClick?: () => void;
    }) => (
      <button
        type={props.type ?? "button"}
        disabled={props.disabled}
        onClick={() => props.onClick?.()}
      >
        {props.children}
      </button>
    ),
    TextField: (props: {
      label: string;
      value?: string;
      disabled?: boolean;
      placeholder?: string;
      onInput?: JSX.EventHandler<HTMLInputElement, InputEvent>;
    }) => (
      <label>
        {props.label}
        <input
          value={props.value ?? ""}
          disabled={props.disabled}
          placeholder={props.placeholder}
          onInput={(event) => props.onInput?.(event)}
        />
      </label>
    ),
  };
});

function snapshot(): WorkbenchSessionSnapshot {
  return {
    session: {
      id: "session-terminal",
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
    runs: [
      {
        id: "run-terminal",
        sessionId: "session-terminal",
        origin: { kind: "interactive" },
        purpose: "task",
        status: "running",
        generation: 7,
        configuration: {},
        createdAtMs: 1,
        updatedAtMs: 1,
      },
    ],
    events: [],
    transcript: [],
    pendingApprovals: [],
    proposedPlans: [],
    artifacts: [],
  } as unknown as WorkbenchSessionSnapshot;
}

function process(status: ProcessSessionRecord["status"] = "running"): ProcessSessionRecord {
  return {
    id: "process-terminal",
    sessionId: "session-terminal",
    runId: "run-terminal",
    checkoutId: "checkout-1",
    runGeneration: 7,
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

function chunk(sequence: number, bytes: number[], capReached = false): ProcessOutputChunk {
  return {
    sequence,
    stream: "stdout",
    deltaBase64: btoa(String.fromCharCode(...bytes)),
    capReached,
  };
}

async function settle() {
  await Promise.resolve();
  await Promise.resolve();
}

beforeEach(() => {
  vi.useFakeTimers();
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
  it("reconnects, streams split UTF-8, replaces invalid bytes, reports caps, and resizes", async () => {
    const readProcess = vi
      .fn()
      .mockResolvedValueOnce({
        process: process(),
        chunks: [chunk(1, [0xe4])],
        nextSequence: 1,
        closed: false,
      })
      .mockResolvedValueOnce({
        process: process("exited"),
        chunks: [chunk(2, [0xbd, 0xa0]), chunk(3, [0xff], true)],
        nextSequence: 3,
        closed: true,
      });
    const port = {
      listProcesses: vi.fn(async () => [process()]),
      readProcess,
      resizeProcess: vi.fn(async () => undefined),
    } as unknown as WorkbenchCommandPort;
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="en-US">
          <TerminalPanel snapshot={snapshot()} commandPort={port} />
        </I18nProvider>
      ),
      root,
    );
    await settle();
    await vi.advanceTimersByTimeAsync(300);
    await settle();

    expect(root.textContent).toContain("你�");
    expect(root.textContent).toContain("Output reached its cap");
    expect(port.resizeProcess).toHaveBeenCalledWith(
      expect.objectContaining({
        processSessionId: "process-terminal",
        size: { rows: 20, cols: 100 },
      }),
    );
    dispose();
  });

  it("encodes stdin once and terminates the active process", async () => {
    const port = {
      listProcesses: vi.fn(async () => []),
      spawnProcess: vi.fn(async () => process()),
      readProcess: vi.fn(async () => ({
        process: process(),
        chunks: [],
        nextSequence: 0,
        closed: false,
      })),
      resizeProcess: vi.fn(async () => undefined),
      writeProcessStdin: vi.fn(async () => undefined),
      terminateProcess: vi.fn(async () => process("terminated")),
    } as unknown as WorkbenchCommandPort;
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="en-US">
          <TerminalPanel snapshot={snapshot()} commandPort={port} />
        </I18nProvider>
      ),
      root,
    );
    await settle();
    const open = [...root.querySelectorAll("button")].find(
      (button) => button.textContent === "Open",
    );
    open?.click();
    await settle();
    const input = root.querySelector('input[placeholder="Type a command and press Enter"]');
    if (!(input instanceof HTMLInputElement)) throw new Error("terminal input missing");
    input.value = "Write-Output 你好";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    const form = input.closest("form");
    form?.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    await settle();
    const write = vi.mocked(port.writeProcessStdin).mock.calls[0]?.[0];
    expect(write).toBeDefined();
    expect(
      new TextDecoder().decode(Uint8Array.from(atob(write!.deltaBase64!), (c) => c.charCodeAt(0))),
    ).toBe("Write-Output 你好\r\n");

    const stop = [...root.querySelectorAll("button")].find((button) =>
      button.textContent?.includes("Stop"),
    );
    stop?.click();
    await settle();
    expect(port.terminateProcess).toHaveBeenCalledTimes(1);
    dispose();
  });

  it("does not reconnect an expired process after the TTL", async () => {
    const port = {
      listProcesses: vi.fn(async () => [process("expired")]),
    } as unknown as WorkbenchCommandPort;
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="en-US">
          <TerminalPanel snapshot={snapshot()} commandPort={port} />
        </I18nProvider>
      ),
      root,
    );
    await settle();
    expect(root.textContent).toContain("Open");
    expect(root.textContent).not.toContain("expired");
    dispose();
  });
});
