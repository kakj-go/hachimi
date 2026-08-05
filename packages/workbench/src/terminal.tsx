import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import {
  commandFailure,
  type ProcessReadRequest,
  type ProcessSessionRecord,
  type WorkbenchSessionSnapshot,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import { Button, Plus, Square, TerminalSquare, X } from "@hachimi/ui";
import { For, Show, createEffect, createSignal, on, onCleanup, onMount, untrack } from "solid-js";

import { directUserMutationContext } from "./mutation-context";
import type { WorkbenchCommandPort } from "./workbench-command-port";
import "./terminal.css";

type TerminalTab = {
  record: ProcessSessionRecord;
  title: string;
};

const automaticStarts = new Map<string, Promise<ProcessSessionRecord>>();

function isLive(record: ProcessSessionRecord) {
  return record.status === "running" || record.status === "starting";
}

function decodeBase64(value: string): Uint8Array {
  const binary = atob(value);
  return Uint8Array.from(binary, (character) => character.charCodeAt(0));
}

function encodeBytes(value: string): string {
  const bytes = new TextEncoder().encode(value);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

function terminalTitle(snapshot: WorkbenchSessionSnapshot, index: number) {
  const path = snapshot.checkout?.path;
  if (!path) return `PowerShell ${index + 1}`;
  return path.endsWith("\\") || path.endsWith("/") ? path : `${path}\\`;
}

export function TerminalPanel(props: {
  projectId: string;
  snapshot: WorkbenchSessionSnapshot;
  commandPort: WorkbenchCommandPort;
  onClose?: () => void;
}) {
  const i18n = useI18n();
  const [tabs, setTabs] = createSignal<TerminalTab[]>([]);
  const [activeId, setActiveId] = createSignal<string>();
  const [failure, setFailure] = createSignal<string>();
  const [starting, setStarting] = createSignal(false);
  const [interrupting, setInterrupting] = createSignal(false);
  const hiddenProcessIds = new Set<string>();
  let initialized = false;

  const checkoutId = () =>
    props.snapshot.session.context.kind === "project"
      ? props.snapshot.session.context.checkout_id
      : undefined;

  function mergeProcess(record: ProcessSessionRecord) {
    if (hiddenProcessIds.has(record.id)) return;
    if (!isLive(record)) automaticStarts.delete(props.projectId);
    setTabs((current) => {
      const index = current.findIndex((tab) => tab.record.id === record.id);
      if (index >= 0) {
        if (current[index]?.record.status === record.status) return current;
        return current.map((tab, position) => (position === index ? { ...tab, record } : tab));
      }
      return [...current, { record, title: terminalTitle(props.snapshot, current.length) }];
    });
    setActiveId((current) => current ?? record.id);
  }

  async function spawn(sharedAutomaticStart = false) {
    const currentCheckout = checkoutId();
    if (!currentCheckout) {
      setFailure(
        i18n.locale() === "zh-CN"
          ? "请先选择项目后再打开终端。"
          : "Select a project before opening a terminal.",
      );
      return;
    }
    setStarting(true);
    setFailure(undefined);
    const key = props.projectId;
    try {
      let launch = sharedAutomaticStart ? automaticStarts.get(key) : undefined;
      if (!launch) {
        launch = props.commandPort.spawnProcess({
          context: directUserMutationContext(),
          sessionId: props.snapshot.session.id,
          checkoutId: currentCheckout,
          command: ["powershell.exe"],
          tty: true,
          streamStdin: true,
          streamOutput: true,
          outputBytesCap: 2 * 1024 * 1024,
          timeoutMs: null,
          environment: {},
          size: { rows: 24, cols: 100 },
        });
        if (sharedAutomaticStart) automaticStarts.set(key, launch);
      }
      const record = await launch;
      mergeProcess(record);
      setActiveId(record.id);
    } catch (error) {
      if (sharedAutomaticStart) automaticStarts.delete(key);
      setFailure(commandFailure(error).message);
    } finally {
      setStarting(false);
    }
  }

  async function refreshProcesses() {
    try {
      const records = await props.commandPort.listProcesses({
        sessionId: props.snapshot.session.id,
        runId: null,
        includeTerminal: true,
      });
      const reconnectable = records.filter(
        (record) =>
          record.runId === null &&
          record.runGeneration === null &&
          record.interactive &&
          isLive(record) &&
          !hiddenProcessIds.has(record.id),
      );
      for (const record of reconnectable) mergeProcess(record);
      if (!initialized) {
        initialized = true;
        if (reconnectable.length === 0) await spawn(true);
      }
    } catch (error) {
      setFailure(commandFailure(error).message);
    }
  }

  async function closeTab(record: ProcessSessionRecord) {
    hiddenProcessIds.add(record.id);
    const current = untrack(tabs);
    const index = current.findIndex((tab) => tab.record.id === record.id);
    const remaining = current.filter((tab) => tab.record.id !== record.id);
    setTabs(remaining);
    if (untrack(activeId) === record.id) {
      setActiveId(remaining[Math.min(index, remaining.length - 1)]?.record.id);
    }
    if (remaining.length === 0) props.onClose?.();
    if (!isLive(record)) return;
    try {
      await props.commandPort.terminateProcess({
        context: directUserMutationContext(),
        processSessionId: record.id,
      });
    } catch (error) {
      setFailure(commandFailure(error).message);
    }
  }

  async function interruptActive() {
    const record = untrack(tabs).find((tab) => tab.record.id === untrack(activeId))?.record;
    if (!record || !isLive(record) || untrack(interrupting)) return;
    setInterrupting(true);
    setFailure(undefined);
    try {
      await props.commandPort.writeProcessStdin({
        context: directUserMutationContext(),
        processSessionId: record.id,
        writeId: crypto.randomUUID(),
        deltaBase64: encodeBytes("\x03"),
        closeStdin: false,
      });
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setInterrupting(false);
    }
  }

  createEffect(
    on(
      () => [props.projectId, props.snapshot.session.id] as const,
      () => {
        let disposed = false;
        initialized = false;
        setTabs([]);
        setActiveId(undefined);
        hiddenProcessIds.clear();
        const refresh = () => {
          if (!disposed) void refreshProcesses();
        };
        refresh();
        const timer = window.setInterval(refresh, 1_500);
        onCleanup(() => {
          disposed = true;
          window.clearInterval(timer);
        });
      },
    ),
  );

  return (
    <section class="terminal-panel" aria-label={i18n.locale() === "zh-CN" ? "终端" : "Terminal"}>
      <div
        class="terminal-tabs"
        role="toolbar"
        aria-label={i18n.locale() === "zh-CN" ? "终端会话" : "Terminal sessions"}
      >
        <For each={tabs()}>
          {(tab) => (
            <div
              class="terminal-tab"
              classList={{ active: activeId() === tab.record.id }}
              data-process-status={tab.record.status}
            >
              <Button
                class="terminal-tab-select"
                aria-pressed={activeId() === tab.record.id}
                title={tab.title}
                onClick={() => setActiveId(tab.record.id)}
              >
                <TerminalSquare size={14} />
                <span>{tab.title}</span>
              </Button>
              <Button
                class="terminal-tab-close"
                title={i18n.locale() === "zh-CN" ? "关闭终端" : "Close terminal"}
                aria-label={i18n.locale() === "zh-CN" ? "关闭终端" : "Close terminal"}
                onClick={() => void closeTab(tab.record)}
              >
                <X size={13} />
              </Button>
            </div>
          )}
        </For>
        <Button
          class="terminal-new-tab"
          title={i18n.locale() === "zh-CN" ? "新建终端" : "New terminal"}
          aria-label={i18n.locale() === "zh-CN" ? "新建终端" : "New terminal"}
          disabled={starting()}
          onClick={() => void spawn()}
        >
          <Plus size={16} />
        </Button>
        <Show when={tabs().find((tab) => tab.record.id === activeId() && isLive(tab.record))}>
          <Button
            class="terminal-interrupt"
            title={
              i18n.locale() === "zh-CN" ? "中断当前命令 (Ctrl+C)" : "Interrupt command (Ctrl+C)"
            }
            aria-label={i18n.locale() === "zh-CN" ? "中断当前命令" : "Interrupt command"}
            disabled={interrupting()}
            onClick={() => void interruptActive()}
          >
            <Square size={13} />
          </Button>
        </Show>
        <Show when={props.onClose}>
          <Button
            class="terminal-panel-close"
            title={i18n.locale() === "zh-CN" ? "隐藏终端面板" : "Hide terminal panel"}
            aria-label={i18n.locale() === "zh-CN" ? "隐藏终端面板" : "Hide terminal panel"}
            onClick={() => props.onClose?.()}
          >
            <X size={15} />
          </Button>
        </Show>
      </div>
      <div class="terminal-sessions">
        <For each={tabs()}>
          {(tab) => (
            <TerminalSession
              record={tab.record}
              active={activeId() === tab.record.id}
              commandPort={props.commandPort}
              onRecord={mergeProcess}
              onFailure={setFailure}
            />
          )}
        </For>
        <Show when={tabs().length === 0 && !starting()}>
          <Button class="terminal-empty" onClick={() => void spawn()}>
            <TerminalSquare size={28} />
            <span>{i18n.locale() === "zh-CN" ? "新建终端" : "New terminal"}</span>
          </Button>
        </Show>
      </div>
      <Show when={failure()}>
        {(message) => (
          <div class="terminal-failure" role="alert">
            <span>{message()}</span>
            <Button
              title={i18n.locale() === "zh-CN" ? "关闭错误" : "Dismiss error"}
              aria-label={i18n.locale() === "zh-CN" ? "关闭错误" : "Dismiss error"}
              onClick={() => setFailure(undefined)}
            >
              <X size={12} />
            </Button>
          </div>
        )}
      </Show>
    </section>
  );
}

function TerminalSession(props: {
  record: ProcessSessionRecord;
  active: boolean;
  commandPort: WorkbenchCommandPort;
  onRecord: (record: ProcessSessionRecord) => void;
  onFailure: (message: string) => void;
}) {
  let host: HTMLDivElement | undefined;
  let fit: FitAddon | undefined;
  let terminal: Terminal | undefined;

  onMount(() => {
    const record = untrack(() => props.record);
    const commandPort = untrack(() => props.commandPort);
    const onRecord = untrack(() => props.onRecord);
    const onFailure = untrack(() => props.onFailure);
    const initiallyActive = untrack(() => props.active);
    if (!host) return;
    let disposed = false;
    let pollTimer: number | undefined;
    let previousSize = "";
    let writeChain = Promise.resolve();

    terminal = new Terminal({
      allowProposedApi: false,
      convertEol: false,
      cursorBlink: true,
      cursorStyle: "bar",
      disableStdin: !isLive(record),
      fontFamily: "Cascadia Mono, Consolas, monospace",
      fontSize: 13,
      lineHeight: 1.35,
      scrollback: 5_000,
      theme: {
        background: "#111315",
        foreground: "#e8e8e8",
        cursor: "#e8e8e8",
        selectionBackground: "#3b424a",
      },
    });
    fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.open(host);

    const send = terminal.onData((data) => {
      if (!isLive(record)) return;
      writeChain = writeChain
        .then(() =>
          commandPort.writeProcessStdin({
            context: directUserMutationContext(),
            processSessionId: record.id,
            writeId: crypto.randomUUID(),
            deltaBase64: encodeBytes(data),
            closeStdin: false,
          }),
        )
        .catch((error) => onFailure(commandFailure(error).message));
    });

    const resize = () => {
      if (disposed || !terminal || !fit || !isLive(record)) return;
      try {
        fit.fit();
      } catch {
        return;
      }
      const size = { rows: terminal.rows, cols: terminal.cols };
      const key = `${size.rows}:${size.cols}`;
      if (size.rows < 1 || size.cols < 1 || key === previousSize) return;
      previousSize = key;
      void commandPort
        .resizeProcess({
          context: directUserMutationContext(),
          processSessionId: record.id,
          size,
        })
        .catch((error) => onFailure(commandFailure(error).message));
    };
    const observer = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(resize);
    observer?.observe(host);
    queueMicrotask(() => {
      resize();
      if (initiallyActive) terminal?.focus();
    });

    let afterSequence = 0;
    const poll = async () => {
      if (disposed) return;
      const request: ProcessReadRequest = {
        processSessionId: record.id,
        afterSequence,
        maxBytes: 256 * 1024,
        waitMs: 100,
      };
      try {
        const snapshot = await commandPort.readProcess(request);
        if (disposed) return;
        afterSequence = snapshot.nextSequence;
        for (const chunk of snapshot.chunks.toSorted(
          (left, right) => left.sequence - right.sequence,
        )) {
          terminal?.write(decodeBase64(chunk.deltaBase64));
        }
        onRecord(snapshot.process);
        if (!snapshot.closed) pollTimer = window.setTimeout(() => void poll(), 80);
      } catch (error) {
        if (!disposed) {
          onFailure(commandFailure(error).message);
          pollTimer = window.setTimeout(() => void poll(), 500);
        }
      }
    };
    void poll();

    onCleanup(() => {
      disposed = true;
      if (pollTimer) window.clearTimeout(pollTimer);
      observer?.disconnect();
      send.dispose();
      terminal?.dispose();
      terminal = undefined;
      fit = undefined;
    });
  });

  createEffect(() => {
    if (!props.active) return;
    queueMicrotask(() => {
      try {
        fit?.fit();
      } catch {
        // The hidden tab can be measured only after it becomes active.
      }
      terminal?.focus();
    });
  });

  return (
    <div
      ref={host}
      class="terminal-session"
      classList={{ active: props.active }}
      data-process-id={props.record.id}
    />
  );
}
