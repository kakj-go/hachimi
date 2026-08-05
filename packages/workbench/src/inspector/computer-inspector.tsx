import {
  commandFailure,
  type ComputerControlSession,
  type WorkbenchSessionSnapshot,
} from "@hachimi/contracts";
import { Badge, Button, Hand, Monitor, Play, Square } from "@hachimi/ui";
import { Show, createEffect, createMemo, createSignal } from "solid-js";

import type { WorkbenchCommandPort } from "../workbench-command-port";

export function ComputerInspector(props: {
  snapshot: WorkbenchSessionSnapshot;
  commandPort: WorkbenchCommandPort;
  locale: "zh-CN" | "en-US";
  controlSessionId?: string;
}) {
  const zh = () => props.locale === "zh-CN";
  const initial = () =>
    props.snapshot.computerControlSessions.find(
      (candidate) => !props.controlSessionId || candidate.id === props.controlSessionId,
    );
  const [control, setControl] = createSignal<ComputerControlSession | undefined>(initial());
  const [busy, setBusy] = createSignal(false);
  const [failure, setFailure] = createSignal<string>();
  const [frameSource, setFrameSource] = createSignal<string>();
  const frame = createMemo(() => control()?.latestFrame);

  createEffect(() => {
    const next = initial();
    setControl(next);
    setFrameSource(undefined);
    const current = next?.latestFrame;
    if (!current) return;
    void props.commandPort
      .getComputerControlFrame(next.ownerSessionId, current.id)
      .then((preview) => {
        if (preview.frameId === current.id) {
          setFrameSource(`data:${preview.mediaType};base64,${preview.dataBase64}`);
        }
      })
      .catch(() => undefined);
  });

  async function change(action: "take_over" | "resume" | "stop") {
    const current = control();
    if (!current) return;
    setBusy(true);
    setFailure(undefined);
    try {
      setFrameSource(undefined);
      if (action === "take_over") {
        setControl(await props.commandPort.takeOverComputerControl(current.ownerSessionId));
      } else if (action === "resume") {
        setControl(await props.commandPort.resumeComputerControl(current.ownerSessionId));
      } else {
        setControl(await props.commandPort.stopComputerControl(current.ownerSessionId));
      }
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div class="computer-inspector">
      <header>
        <Monitor size={18} />
        <div>
          <strong>{control()?.app?.displayName ?? "Computer Use"}</strong>
          <span>{control()?.window?.title ?? control()?.app?.appId}</span>
        </div>
        <Badge tone={control()?.status === "active" ? "success" : "neutral"}>
          {control()?.status ?? (zh() ? "未运行" : "Inactive")}
        </Badge>
      </header>
      <div class="computer-inspector-frame">
        <Show
          when={frameSource()}
          fallback={
            <Show
              when={frame()}
              fallback={
                <span>
                  {zh() ? "等待新的内存截图..." : "Waiting for a fresh in-memory frame..."}
                </span>
              }
            >
              {(current) => (
                <div>
                  <Monitor size={28} />
                  <strong>{`${current().width} x ${current().height}`}</strong>
                  <span>{zh() ? "内存截图已过期" : "The in-memory frame has expired"}</span>
                </div>
              )}
            </Show>
          }
        >
          {(source) => <img data-testid="computer-frame-preview" src={source()} alt="" />}
        </Show>
      </div>
      <Show when={failure()}>
        {(message) => <p class="browser-inspector-error">{message()}</p>}
      </Show>
      <Show when={control()}>
        {(current) => (
          <div class="browser-automation-control" data-status={current().status}>
            <span>
              {current().status === "active"
                ? zh()
                  ? "Agent 正在控制应用"
                  : "Agent is controlling the application"
                : zh()
                  ? "Agent 控制已暂停"
                  : "Agent control is paused"}
            </span>
            <Show when={current().status === "active"}>
              <Button disabled={busy()} onClick={() => void change("take_over")}>
                <Hand size={14} />
                {zh() ? "接管" : "Take over"}
              </Button>
            </Show>
            <Show when={current().status === "suspended"}>
              <Button disabled={busy()} onClick={() => void change("resume")}>
                <Play size={14} />
                {zh() ? "恢复 Agent" : "Resume Agent"}
              </Button>
            </Show>
            <Button variant="danger" disabled={busy()} onClick={() => void change("stop")}>
              <Square size={13} />
              {zh() ? "停止" : "Stop"}
            </Button>
          </div>
        )}
      </Show>
    </div>
  );
}
