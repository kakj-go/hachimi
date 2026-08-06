import type { ScheduleDefinition, ScheduleEventReceipt, TaskRunRecord } from "@hachimi/contracts";
import {
  Badge,
  Button,
  CalendarClock,
  Clock3,
  Dialog,
  ExternalLink,
  RefreshCw,
  Square,
  Tabs,
} from "@hachimi/ui";
import { For, Show, createEffect, createSignal } from "solid-js";

import {
  eventReceiptLabel,
  eventReceiptTone,
  formatTaskDuration,
  formatTaskTime,
  taskRunStatusLabel,
  taskRunTriggerLabel,
} from "./task-center-format";

export function TaskHistoryDialog(props: {
  schedule: ScheduleDefinition | undefined;
  runs: TaskRunRecord[];
  events: ScheduleEventReceipt[];
  zh: boolean;
  busyId: string | undefined;
  onClose: () => void;
  onOpenSession: (sessionId: string) => void;
  onCancel: (run: TaskRunRecord) => void;
  onRetry: (run: TaskRunRecord) => void;
  onContinue: (run: TaskRunRecord) => void;
}) {
  const [tab, setTab] = createSignal("runs");
  let selectedScheduleId: string | undefined;

  createEffect(() => {
    const scheduleId = props.schedule?.id;
    if (scheduleId && scheduleId !== selectedScheduleId) setTab("runs");
    selectedScheduleId = scheduleId;
  });

  const runContent = () => (
    <div class="task-history-list" data-testid="task-run-history">
      <Show
        when={props.runs.length > 0}
        fallback={<div class="task-history-empty">{props.zh ? "尚无运行记录" : "No runs yet"}</div>}
      >
        <For each={props.runs}>
          {(run) => (
            <div
              class="task-history-row"
              data-testid="task-run-row"
              data-run-id={run.id}
              data-run-status={run.status}
            >
              <Button
                type="button"
                class="task-history-main"
                variant="ghost"
                data-testid="task-open-session"
                disabled={!run.executionSessionId}
                onClick={() =>
                  run.executionSessionId && props.onOpenSession(run.executionSessionId)
                }
              >
                <span class={`task-status-dot ${run.status}`} aria-hidden="true" />
                <span class="task-history-copy">
                  <span>
                    <strong data-testid="task-run-status">
                      {taskRunStatusLabel(run.status, props.zh)}
                    </strong>
                    <span data-testid="task-run-trigger">
                      <Badge tone="neutral">{taskRunTriggerLabel(run.trigger, props.zh)}</Badge>
                    </span>
                  </span>
                  <span class="task-history-meta">
                    <small>
                      <CalendarClock size={13} aria-hidden="true" />
                      {formatTaskTime(run.startedAtMs ?? run.createdAtMs, props.zh)}
                    </small>
                    <small data-testid="task-run-duration">
                      <Clock3 size={13} aria-hidden="true" />
                      {props.zh ? "用时" : "Duration"} {formatTaskDuration(run, props.zh)}
                    </small>
                  </span>
                  <Show when={run.resultSummary}>{(summary) => <p>{summary()}</p>}</Show>
                  <Show when={run.errorSummary}>
                    {(summary) => <p class="task-history-error">{summary()}</p>}
                  </Show>
                  <Show when={!run.executionSessionId}>
                    <small>{props.zh ? "会话正在准备中" : "Session is being prepared"}</small>
                  </Show>
                </span>
                <Show when={run.executionSessionId}>
                  <ExternalLink size={15} aria-hidden="true" />
                </Show>
              </Button>
              <div class="task-history-actions">
                <Show when={["queued", "preparing", "running"].includes(run.status)}>
                  <Button
                    size="small"
                    variant="ghost"
                    disabled={props.busyId === run.id}
                    data-testid="task-cancel"
                    onClick={() => props.onCancel(run)}
                  >
                    <Square size={13} />
                    {props.zh ? "取消" : "Cancel"}
                  </Button>
                </Show>
                <Show when={run.status === "needs_attention"}>
                  <Button
                    size="small"
                    variant="ghost"
                    disabled={props.busyId === run.id}
                    data-testid="task-continue"
                    onClick={() => props.onContinue(run)}
                  >
                    {props.zh ? "转为交互" : "Continue"}
                  </Button>
                </Show>
                <Show when={["failed", "timed_out", "lost", "cancelled"].includes(run.status)}>
                  <Button
                    size="small"
                    variant="ghost"
                    disabled={props.busyId === run.id}
                    data-testid="task-retry"
                    onClick={() => props.onRetry(run)}
                  >
                    <RefreshCw size={13} />
                    {props.zh ? "重试" : "Retry"}
                  </Button>
                </Show>
              </div>
            </div>
          )}
        </For>
      </Show>
    </div>
  );

  const eventContent = () => (
    <div class="task-event-list" data-testid="task-event-history">
      <Show
        when={props.events.length > 0}
        fallback={
          <div class="task-history-empty">{props.zh ? "尚无触发事件" : "No events yet"}</div>
        }
      >
        <For each={props.events}>
          {(receipt) => (
            <div class="task-event-row" data-receipt-status={receipt.status}>
              <Badge tone={eventReceiptTone(receipt.status)}>
                {eventReceiptLabel(receipt.status, props.zh)}
              </Badge>
              <span>
                <strong>{receipt.event.eventType}</strong>
                <small>
                  {receipt.event.source.kind} · {receipt.event.source.id}
                </small>
                <small>
                  {receipt.event.subject ?? receipt.event.eventId} ·{" "}
                  {formatTaskTime(receipt.event.receivedAtMs, props.zh)}
                </small>
              </span>
              <small>
                {props.zh ? "运行" : "Runs"} {receipt.taskRuns.length}
              </small>
            </div>
          )}
        </For>
      </Show>
    </div>
  );

  return (
    <Dialog
      open={Boolean(props.schedule)}
      size="wide"
      title={props.schedule?.name ?? (props.zh ? "运行历史" : "Run history")}
      closeLabel={props.zh ? "关闭" : "Close"}
      onOpenChange={(open) => {
        if (!open) props.onClose();
      }}
    >
      <div class="task-history-dialog">
        <Show when={props.schedule?.schedule.kind === "event"} fallback={runContent()}>
          <Tabs
            value={tab()}
            onChange={setTab}
            tabs={[
              { value: "runs", label: props.zh ? "运行" : "Runs", content: runContent() },
              { value: "events", label: props.zh ? "触发事件" : "Events", content: eventContent() },
            ]}
          />
        </Show>
      </div>
    </Dialog>
  );
}
