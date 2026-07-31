import {
  commandFailure,
  type ProcessReadRequest,
  type ProcessReadSnapshot,
  type ProcessSessionRecord,
  type RunRecord,
  type WorkbenchSessionSnapshot,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import { Button, Square, TerminalSquare, TextField } from "@hachimi/ui";
import { Show, createEffect, createSignal, onCleanup, onMount, untrack } from "solid-js";

import type { WorkbenchCommandPort } from "./workbench-command-port";
import { runMutationContext } from "./mutation-context";
import "./terminal.css";

function encodeBytes(value: string): string {
  const bytes = new TextEncoder().encode(value);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

function decodeBytes(value: string, decoder: TextDecoder, stream: boolean): string {
  const binary = atob(value);
  const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
  return decoder.decode(bytes, { stream });
}

function activeRun(snapshot: WorkbenchSessionSnapshot): RunRecord | undefined {
  return snapshot.runs[snapshot.runs.length - 1];
}

export function TerminalPanel(props: {
  snapshot: WorkbenchSessionSnapshot;
  commandPort: WorkbenchCommandPort;
}) {
  const i18n = useI18n();
  const [open, setOpen] = createSignal(false);
  const [process, setProcess] = createSignal<ProcessSessionRecord>();
  const [output, setOutput] = createSignal("");
  const [input, setInput] = createSignal("");
  const [failure, setFailure] = createSignal<string>();
  const [outputCapped, setOutputCapped] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [outputElement, setOutputElement] = createSignal<HTMLPreElement>();
  const [lastSize, setLastSize] = createSignal<{ rows: number; cols: number }>();

  const run = () => activeRun(props.snapshot);
  const checkoutId = () =>
    props.snapshot.session.context.kind === "project"
      ? props.snapshot.session.context.checkout_id
      : undefined;
  const canStart = () =>
    Boolean(
      checkoutId() &&
      ["running", "waiting_approval", "waiting_user_input"].includes(run()?.status ?? ""),
    );

  onMount(() => {
    let disposed = false;
    void props.commandPort
      .listProcesses({
        sessionId: props.snapshot.session.id,
        runId: run()?.id ?? null,
        includeTerminal: true,
      })
      .then((records) => {
        if (disposed) return;
        const reconnectable = records
          .filter((record) => record.interactive)
          .findLast((record) => record.status === "running" || record.status === "starting");
        if (reconnectable) {
          setProcess(reconnectable);
          setOpen(true);
        }
      })
      .catch((error) => {
        if (!disposed) setFailure(commandFailure(error).message);
      });
    onCleanup(() => {
      disposed = true;
    });
  });

  createEffect(() => {
    const current = process();
    if (!current) return;
    const commandPort = props.commandPort;
    let disposed = false;
    let afterSequence = 0;
    const decoders = {
      stdout: new TextDecoder(),
      stderr: new TextDecoder(),
    };
    const poll = async () => {
      if (disposed) return;
      const request: ProcessReadRequest = {
        processSessionId: current.id,
        afterSequence,
        maxBytes: 256 * 1024,
        waitMs: 100,
      };
      try {
        const snapshot: ProcessReadSnapshot = await commandPort.readProcess(request);
        if (disposed) return;
        afterSequence = snapshot.nextSequence;
        const text = snapshot.chunks
          .toSorted((left, right) => left.sequence - right.sequence)
          .map((chunk) => {
            if (chunk.capReached) setOutputCapped(true);
            return decodeBytes(chunk.deltaBase64, decoders[chunk.stream], true);
          })
          .join("");
        const completedText = snapshot.closed
          ? `${text}${decoders.stdout.decode()}${decoders.stderr.decode()}`
          : text;
        if (completedText) {
          setOutput((previous) => `${previous}${completedText}`.slice(-512 * 1024));
          queueMicrotask(() => {
            const element = untrack(outputElement);
            if (element) element.scrollTop = element.scrollHeight;
          });
        }
        if (snapshot.closed) setProcess(snapshot.process);
      } catch (error) {
        if (!disposed) setFailure(commandFailure(error).message);
      }
    };
    void poll();
    const timer = window.setInterval(() => void poll(), 300);
    onCleanup(() => {
      disposed = true;
      window.clearInterval(timer);
    });
  });

  createEffect(() => {
    const element = outputElement();
    const current = process();
    const currentRun = run();
    if (!element || !current || !currentRun || current.status !== "running") return;
    if (typeof ResizeObserver === "undefined") return;
    let previous = "";
    const observer = new ResizeObserver((entries) => {
      const rectangle = entries[0]?.contentRect;
      if (!rectangle) return;
      const size = {
        rows: Math.max(8, Math.min(120, Math.floor(rectangle.height / 18))),
        cols: Math.max(20, Math.min(240, Math.floor(rectangle.width / 8))),
      };
      const key = `${size.rows}:${size.cols}`;
      if (key === previous) return;
      previous = key;
      void props.commandPort
        .resizeProcess({
          context: runMutationContext(currentRun),
          processSessionId: current.id,
          size,
        })
        .then(() => setLastSize(size))
        .catch((error) => setFailure(commandFailure(error).message));
    });
    observer.observe(element);
    onCleanup(() => observer.disconnect());
  });

  async function start() {
    const currentRun = run();
    const currentCheckout = checkoutId();
    if (!currentRun || !currentCheckout) {
      setFailure(
        i18n.locale() === "zh-CN"
          ? "Terminal 需要一个运行中的项目任务。"
          : "Terminal requires a running project task.",
      );
      return;
    }
    setBusy(true);
    setFailure(undefined);
    setOutput("");
    setOutputCapped(false);
    try {
      const record = await props.commandPort.spawnProcess({
        context: runMutationContext(currentRun),
        sessionId: props.snapshot.session.id,
        checkoutId: currentCheckout,
        command: ["powershell.exe", "-NoLogo", "-NoProfile", "-NonInteractive"],
        tty: true,
        streamStdin: true,
        streamOutput: true,
        outputBytesCap: 2 * 1024 * 1024,
        timeoutMs: 30 * 60 * 1000,
        environment: {},
        size: { rows: 24, cols: 100 },
      });
      setProcess(record);
      setLastSize({ rows: 24, cols: 100 });
      setOpen(true);
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
    }
  }

  async function send() {
    const current = process();
    const currentRun = run();
    const value = input();
    if (!current || !currentRun || !value) return;
    setFailure(undefined);
    try {
      await props.commandPort.writeProcessStdin({
        context: runMutationContext(currentRun),
        processSessionId: current.id,
        writeId: crypto.randomUUID(),
        deltaBase64: encodeBytes(`${value}\r\n`),
        closeStdin: false,
      });
      setInput("");
    } catch (error) {
      setFailure(commandFailure(error).message);
    }
  }

  async function terminate() {
    const current = process();
    const currentRun = run();
    if (!current || !currentRun) return;
    setFailure(undefined);
    try {
      setProcess(
        await props.commandPort.terminateProcess({
          context: runMutationContext(currentRun),
          processSessionId: current.id,
        }),
      );
    } catch (error) {
      setFailure(commandFailure(error).message);
    }
  }

  return (
    <section
      class="terminal-panel"
      aria-label={i18n.locale() === "zh-CN" ? "终端" : "Terminal"}
      data-terminal-rows={lastSize()?.rows}
      data-terminal-cols={lastSize()?.cols}
    >
      <div class="terminal-panel-heading">
        <div>
          <TerminalSquare size={16} />
          <strong>{i18n.locale() === "zh-CN" ? "终端" : "Terminal"}</strong>
          <Show when={process()}>
            {(record) => <span class="terminal-status">{record().status}</span>}
          </Show>
        </div>
        <div class="terminal-actions">
          <Show when={!process() || process()?.status !== "running"}>
            <Button size="small" disabled={busy() || !canStart()} onClick={() => void start()}>
              {busy()
                ? i18n.locale() === "zh-CN"
                  ? "启动中…"
                  : "Starting…"
                : i18n.locale() === "zh-CN"
                  ? "打开"
                  : "Open"}
            </Button>
          </Show>
          <Show when={process()?.status === "running"}>
            <Button size="small" variant="ghost" onClick={() => void terminate()}>
              <Square size={13} /> {i18n.locale() === "zh-CN" ? "终止" : "Stop"}
            </Button>
          </Show>
          <Button size="small" variant="ghost" onClick={() => setOpen((value) => !value)}>
            {open()
              ? i18n.locale() === "zh-CN"
                ? "收起"
                : "Hide"
              : i18n.locale() === "zh-CN"
                ? "展开"
                : "Show"}
          </Button>
        </div>
      </div>
      <Show when={open()}>
        <pre ref={setOutputElement} class="terminal-output" tabIndex={0}>
          {output() || (i18n.locale() === "zh-CN" ? "等待输出…" : "Waiting for output…")}
        </pre>
        <Show when={outputCapped()}>
          <p class="terminal-output-cap" role="status">
            {i18n.locale() === "zh-CN"
              ? "输出已达到上限，后续内容已截断。"
              : "Output reached its cap; later bytes were truncated."}
          </p>
        </Show>
        <form
          class="terminal-input-row"
          onSubmit={(event) => {
            event.preventDefault();
            void send();
          }}
        >
          <TextField
            label={i18n.locale() === "zh-CN" ? "命令" : "Command"}
            value={input()}
            disabled={process()?.status !== "running"}
            placeholder={
              i18n.locale() === "zh-CN" ? "输入命令并回车" : "Type a command and press Enter"
            }
            onInput={(event) => setInput(event.currentTarget.value)}
          />
          <Button size="small" type="submit" disabled={process()?.status !== "running" || !input()}>
            {i18n.locale() === "zh-CN" ? "发送" : "Send"}
          </Button>
        </form>
      </Show>
      <Show when={failure()}>
        <p class="terminal-failure" role="alert">
          {failure()}
        </p>
      </Show>
    </section>
  );
}
