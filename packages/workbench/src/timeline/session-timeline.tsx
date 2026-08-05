/* eslint-disable solid/components-return-once, solid/reactivity -- Projection entries are immutable records owned by the parent For. */
import type {
  RunRecord,
  RunRecoveryDecisionAction,
  RunRecoverySnapshot,
  TranscriptItem,
  WorkbenchSessionSnapshot,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import {
  AgentMessage,
  Archive,
  Badge,
  Button,
  ChevronDown,
  CircleHelp,
  File,
  Lightbulb,
  Maximize2,
  ShieldCheck,
  TerminalSquare,
} from "@hachimi/ui";
import { For, Show, createMemo, onCleanup } from "solid-js";

import type { LiveItemDeltas } from "../agent-live-items";
import { liveItemText } from "../agent-live-items";
import { ProviderContextPayload } from "../provider-context-payload";
import { latestMcpProgress } from "./mcp-progress";
import { TimelineMessageText } from "./message-markdown";
import { TimelineRunCompletion } from "./run-completion-summary";
import { SessionAttachments } from "./session-attachments";
import {
  projectSessionTimeline,
  isDiffToolActivity,
  toolBatchLabel,
  type TimelineProjectionEntry,
} from "./timeline-projection";
import { timelineActivityLabel, timelineItemText, timelineKindLabel } from "./timeline-text";

export function SessionTimeline(props: {
  snapshot: WorkbenchSessionSnapshot;
  pendingGate: "approval" | "host_access" | "plan" | "user_input" | undefined;
  recoveries: RunRecoverySnapshot[];
  liveItemDeltas: LiveItemDeltas;
  resolvingRecoveryId: string | undefined;
  onContentMount: (element: HTMLElement | undefined) => void;
  onResolveRecovery: (recovery: RunRecoverySnapshot, action: RunRecoveryDecisionAction) => void;
  onOpenItem: (item: TranscriptItem) => void;
  onOpenAttachment: (attachment: WorkbenchSessionSnapshot["attachments"][number]) => void;
  onOpenPath: (path: string) => void;
  onOpenDiff: (runId: string, path?: string) => void;
}) {
  const i18n = useI18n();
  const latestRun = () => props.snapshot.runs[props.snapshot.runs.length - 1];
  const mcpProgress = () => latestMcpProgress(props.snapshot, latestRun()?.id);
  const timeline = createMemo(() => projectSessionTimeline(props.snapshot));
  const runIsActive = (run: RunRecord | undefined) =>
    Boolean(run && !run.status.match(/succeeded|failed|timed_out|cancelled|interrupted|lost/));
  onCleanup(() => props.onContentMount(undefined));

  return (
    <section
      ref={props.onContentMount}
      class="session-timeline"
      data-testid="workbench-session-timeline"
      data-run-id={latestRun()?.id}
      data-run-status={latestRun()?.status}
      data-agent-task-count={props.snapshot.agentTasks.length}
      data-agent-task-statuses={props.snapshot.agentTasks.map((task) => task.status).join(",")}
      aria-label={i18n.t("workbench.timeline")}
    >
      <RecoveryStack
        recoveries={props.recoveries}
        resolvingRecoveryId={props.resolvingRecoveryId}
        onResolve={props.onResolveRecovery}
      />
      <Show when={runIsActive(latestRun()) && mcpProgress().length > 0}>
        <div class="mcp-progress-stack" aria-label="MCP Tool progress">
          <For each={mcpProgress()}>
            {(progress) => (
              <article class="mcp-progress-card">
                <div>
                  <strong>{progress.toolCallId}</strong>
                  <small>
                    {i18n.locale() === "zh-CN"
                      ? "MCP 服务进度（不可信展示数据）"
                      : "MCP server progress (untrusted display data)"}
                  </small>
                </div>
                <progress
                  value={progress.progress}
                  max={Math.max(progress.total ?? progress.progress, progress.progress, 1)}
                />
                <span>
                  {progress.message ?? (i18n.locale() === "zh-CN" ? "正在执行…" : "Running…")}
                </span>
              </article>
            )}
          </For>
        </div>
      </Show>
      <div class="timeline-items agent-thread">
        <For each={timeline()}>
          {(entry) => (
            <TimelineEntry
              entry={entry}
              snapshot={props.snapshot}
              liveItemDeltas={props.liveItemDeltas}
              onOpenItem={props.onOpenItem}
              onOpenAttachment={props.onOpenAttachment}
              onOpenPath={props.onOpenPath}
              onOpenDiff={props.onOpenDiff}
            />
          )}
        </For>
        <Show when={props.pendingGate} keyed>
          {(kind) => <PendingGateStatus kind={kind} />}
        </Show>
        <Show when={timeline().length === 0 && !props.pendingGate}>
          <p class="timeline-empty">{i18n.t("workbench.timelineEmpty")}</p>
        </Show>
      </div>
    </section>
  );
}

export function PendingGateStatus(props: {
  kind: "approval" | "host_access" | "plan" | "user_input";
}) {
  const i18n = useI18n();
  const zh = () => i18n.locale() === "zh-CN";
  const copy = () => {
    if (props.kind === "approval") {
      return zh()
        ? ["等待批准", "Agent 已暂停，批准或拒绝后继续"]
        : ["Waiting for approval", "The agent is paused until you allow or deny the request"];
    }
    if (props.kind === "host_access") {
      return zh()
        ? ["等待访问授权", "Agent 已暂停，请选择目标访问范围"]
        : ["Waiting for access", "The agent is paused until you choose an access scope"];
    }
    if (props.kind === "plan") {
      return zh()
        ? ["等待确认计划", "请选择实施、修改或跳过"]
        : ["Waiting for plan review", "Choose whether to implement, revise, or skip the plan"];
    }
    return zh()
      ? ["等待回答", "回答问题后 Agent 将继续"]
      : ["Waiting for your answer", "The agent will continue after you answer"];
  };
  const icon = () => {
    if (props.kind === "approval" || props.kind === "host_access") {
      return <ShieldCheck size={16} />;
    }
    if (props.kind === "plan") return <Lightbulb size={16} />;
    return <CircleHelp size={16} />;
  };
  return (
    <div class="timeline-gate-status" data-gate-kind={props.kind} role="status">
      {icon()}
      <span>
        <strong>{copy()[0]}</strong>
        <small>{copy()[1]}</small>
      </span>
      <i aria-hidden="true" />
    </div>
  );
}

function TimelineEntry(props: {
  entry: TimelineProjectionEntry;
  snapshot: WorkbenchSessionSnapshot;
  liveItemDeltas: LiveItemDeltas;
  onOpenItem: (item: TranscriptItem) => void;
  onOpenAttachment: (attachment: WorkbenchSessionSnapshot["attachments"][number]) => void;
  onOpenPath: (path: string) => void;
  onOpenDiff: (runId: string, path?: string) => void;
}) {
  const i18n = useI18n();
  if (props.entry.kind === "tool_batch") {
    return (
      <ToolActivityBatch
        items={props.entry.items}
        liveItemDeltas={props.liveItemDeltas}
        onOpenDiff={props.onOpenDiff}
      />
    );
  }
  if (props.entry.kind === "run_summary") {
    return (
      <TimelineRunCompletion
        summary={props.entry.summary}
        locale={i18n.locale()}
        onOpenDiff={props.onOpenDiff}
      />
    );
  }

  const item = props.entry.item;
  const text = () =>
    timelineItemText(
      item.payload,
      item.status === "in_progress" ? liveItemText(props.liveItemDeltas[item.id]) : undefined,
    );
  if (item.kind === "user" || item.kind === "assistant") {
    return (
      <AgentMessage
        class={["timeline-item", `timeline-${item.kind}`].join(" ")}
        role={item.kind}
        author={timelineKindLabel(item.kind, i18n.locale())}
      >
        <TimelineMessageText
          text={text()}
          workspaceRoot={props.snapshot.checkout?.path}
          onOpenPath={props.onOpenPath}
        />
        <SessionAttachments
          payload={item.payload}
          attachments={props.snapshot.attachments}
          onOpen={props.onOpenAttachment}
        />
      </AgentMessage>
    );
  }
  if (item.kind === "reasoning") {
    return (
      <article class="timeline-narration" data-status={item.status}>
        <TimelineMessageText
          text={text()}
          workspaceRoot={props.snapshot.checkout?.path}
          onOpenPath={props.onOpenPath}
        />
      </article>
    );
  }
  if (item.kind === "context_compaction") return <CompactionLine item={item} />;
  if (item.kind === "user_input_request") return <UserInputHistory item={item} />;
  if (item.kind === "plan" && item.payload.type === "plan") {
    const planData = item.payload.data;
    const plan = props.snapshot.proposedPlans.find(
      (candidate) => candidate.id === planData.plan_id,
    );
    return (
      <article class="timeline-plan-card" data-status={plan?.status ?? item.status}>
        <header>
          <span>
            <Lightbulb size={17} />
            <strong>{i18n.locale() === "zh-CN" ? "计划" : "Plan"}</strong>
          </span>
          <Button
            size="small"
            variant="ghost"
            aria-label={i18n.locale() === "zh-CN" ? "查看完整计划" : "Open full plan"}
            onClick={() => props.onOpenItem(item)}
          >
            <Maximize2 size={14} />
          </Button>
        </header>
        <TimelineMessageText
          text={planData.text}
          workspaceRoot={props.snapshot.checkout?.path}
          onOpenPath={props.onOpenPath}
        />
      </article>
    );
  }

  return (
    <article class={`timeline-system-line timeline-${item.kind}`} data-status={item.status}>
      <strong>{timelineActivityLabel(item.kind, item.payload, i18n.locale())}</strong>
      <span>{text()}</span>
    </article>
  );
}

function ToolActivityBatch(props: {
  items: TranscriptItem[];
  liveItemDeltas: LiveItemDeltas;
  onOpenDiff: (runId: string, path?: string) => void;
}) {
  const i18n = useI18n();
  const active = () => props.items.some((item) => item.status === "in_progress");
  return (
    <details
      class="tool-activity-batch"
      open={active()}
      data-status={active() ? "in_progress" : "completed"}
    >
      <summary>
        <TerminalSquare size={16} />
        <strong>{toolBatchLabel(props.items, i18n.locale() === "zh-CN")}</strong>
        <ChevronDown size={15} />
      </summary>
      <div class="tool-activity-list">
        <For each={props.items}>
          {(item) => {
            const text = () =>
              timelineItemText(
                item.payload,
                item.status === "in_progress"
                  ? liveItemText(props.liveItemDeltas[item.id])
                  : undefined,
              );
            if (item.kind === "file_change" && item.payload.type === "file_change") {
              const fileData = item.payload.data;
              return (
                <Button
                  class="tool-file-change"
                  disabled={!item.runId}
                  onClick={() => {
                    if (item.runId) props.onOpenDiff(item.runId, fileData.path);
                  }}
                >
                  <File size={15} />
                  <span>
                    <strong>{fileData.path}</strong>
                    <small>{fileData.change_kind}</small>
                  </span>
                  <Badge tone={item.status === "failed" ? "danger" : "neutral"}>
                    {item.status}
                  </Badge>
                </Button>
              );
            }
            if (isDiffToolActivity(item)) {
              return (
                <Button
                  class="tool-file-change"
                  disabled={!item.runId}
                  onClick={() => {
                    if (item.runId) props.onOpenDiff(item.runId);
                  }}
                >
                  <File size={15} />
                  <span>
                    <strong>
                      {i18n.locale() === "zh-CN" ? "应用了文件补丁" : "Applied a file patch"}
                    </strong>
                    <small>apply_patch</small>
                  </span>
                  <Badge tone={item.status === "failed" ? "danger" : "neutral"}>
                    {item.status}
                  </Badge>
                </Button>
              );
            }
            return (
              <details class="tool-activity-item">
                <summary>
                  <span>{timelineActivityLabel(item.kind, item.payload, i18n.locale())}</span>
                  <small>{item.status}</small>
                  <ChevronDown size={13} />
                </summary>
                <ProviderContextPayload
                  payload={item.payload}
                  locale={i18n.locale()}
                  focusable={false}
                  text={text()}
                />
              </details>
            );
          }}
        </For>
      </div>
    </details>
  );
}

function UserInputHistory(props: { item: TranscriptItem }) {
  const i18n = useI18n();
  if (props.item.payload.type !== "user_input_request") return null;
  const data = props.item.payload.data;
  const completed = () => props.item.status === "completed";
  const answerFor = (questionId: string) =>
    data.display_answers?.find((answer) => answer.questionId === questionId);
  const answerLabel = (question: (typeof data.questions)[number]) => {
    const answer = answerFor(question.id);
    if (!answer) return i18n.locale() === "zh-CN" ? "未回答" : "Not answered";
    if (answer.secretProvided)
      return i18n.locale() === "zh-CN" ? "已提供敏感回答" : "Secret answer provided";
    return (
      question.options.find((option) => option.value === answer.value)?.label ?? answer.value ?? ""
    );
  };
  return (
    <details class="user-input-history" data-status={props.item.status}>
      <summary>
        <CircleHelp size={16} />
        <strong>
          {i18n.locale() === "zh-CN"
            ? completed()
              ? `已询问 ${data.questions.length} 个问题`
              : "正在询问问题"
            : completed()
              ? `Asked ${data.questions.length} questions`
              : "Asking a question"}
        </strong>
        <ChevronDown size={14} />
      </summary>
      <div class="user-input-history-list">
        <For each={data.questions}>
          {(question) => (
            <div>
              <strong>{question.header}</strong>
              <span>{question.prompt}</span>
              <small>
                {i18n.locale() === "zh-CN" ? "你的选择：" : "Your answer: "}
                {answerLabel(question)}
              </small>
            </div>
          )}
        </For>
      </div>
    </details>
  );
}

function CompactionLine(props: { item: TranscriptItem }) {
  const i18n = useI18n();
  const failed = () => props.item.status === "failed" || props.item.status === "interrupted";
  return (
    <div class="context-compaction-line" data-status={props.item.status}>
      <Archive size={16} />
      <span>
        {i18n.locale() === "zh-CN"
          ? props.item.status === "in_progress"
            ? "正在自动压缩上下文"
            : failed()
              ? "上下文自动压缩未完成"
              : "上下文已自动压缩"
          : props.item.status === "in_progress"
            ? "Compacting context"
            : failed()
              ? "Context compaction did not complete"
              : "Context compacted automatically"}
      </span>
    </div>
  );
}

function RecoveryStack(props: {
  recoveries: RunRecoverySnapshot[];
  resolvingRecoveryId: string | undefined;
  onResolve: (recovery: RunRecoverySnapshot, action: RunRecoveryDecisionAction) => void;
}) {
  const i18n = useI18n();
  return (
    <Show when={props.recoveries.length > 0}>
      <div class="recovery-stack" data-testid="run-recovery-stack">
        <For each={props.recoveries}>
          {(snapshot) => {
            const recovery = () => snapshot.recovery;
            const checkpoint = () => snapshot.checkpoint;
            const resolving = () => props.resolvingRecoveryId === recovery().id;
            const canResumeSafe = () =>
              !recovery().sideEffectExecutionId &&
              (checkpoint()?.recoveryPolicy === "read_only_replayable" ||
                checkpoint()?.recoveryPolicy === "idempotent_with_receipt");
            const canRetry = () =>
              Boolean(recovery().sideEffectExecutionId) &&
              checkpoint()?.recoveryPolicy === "idempotent_with_receipt";
            return (
              <article class="recovery-card" data-testid={`run-recovery-${recovery().id}`}>
                <div class="recovery-card-heading">
                  <span>
                    <ShieldCheck size={17} />
                    <strong>{i18n.t("workbench.recoveryRequired")}</strong>
                  </span>
                  <Badge tone={recovery().state === "awaiting_user" ? "warning" : "info"}>
                    {recovery().state}
                  </Badge>
                </div>
                <p>{i18n.t("workbench.recoveryDescription")}</p>
                <small>
                  {recovery().reasonCode} · generation {recovery().interruptedGeneration} →{" "}
                  {recovery().resumeGeneration}
                </small>
                <footer>
                  <Button
                    size="small"
                    disabled={resolving()}
                    onClick={() => props.onResolve(snapshot, "abandon_run")}
                  >
                    {i18n.t("workbench.recoveryAbandon")}
                  </Button>
                  <Show when={recovery().sideEffectExecutionId}>
                    <Button
                      size="small"
                      disabled={resolving()}
                      onClick={() => props.onResolve(snapshot, "confirm_effect_succeeded")}
                    >
                      {i18n.t("workbench.recoveryConfirmSucceeded")}
                    </Button>
                  </Show>
                  <Show when={canRetry()}>
                    <Button
                      size="small"
                      variant="primary"
                      disabled={resolving()}
                      onClick={() => props.onResolve(snapshot, "retry_idempotent_effect")}
                    >
                      {i18n.t("workbench.recoveryRetry")}
                    </Button>
                  </Show>
                  <Show when={canResumeSafe()}>
                    <Button
                      size="small"
                      variant="primary"
                      disabled={resolving()}
                      onClick={() => props.onResolve(snapshot, "resume_safe_remainder")}
                    >
                      {i18n.t("workbench.recoveryResumeSafe")}
                    </Button>
                  </Show>
                </footer>
              </article>
            );
          }}
        </For>
      </div>
    </Show>
  );
}
