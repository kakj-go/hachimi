import type { AgentTaskRecord } from "@hachimi/contracts";
import type { AppLocale } from "@hachimi/i18n";
import { For, Show } from "solid-js";

type AgentTaskPanelProps = {
  tasks: readonly AgentTaskRecord[];
  locale: AppLocale;
};

export function AgentTaskPanel(props: AgentTaskPanelProps) {
  return (
    <Show when={props.tasks.length > 0}>
      <section class="agent-task-panel" aria-label="Multi-Agent Tasks">
        <header>
          <strong>{props.locale === "zh-CN" ? "协作子任务" : "Agent subtasks"}</strong>
          <span>{props.tasks.length}</span>
        </header>
        <div class="agent-task-grid">
          <For each={props.tasks}>
            {(task) => (
              <article class="agent-task-card" data-status={task.status}>
                <header>
                  <strong>{task.title}</strong>
                  <span>{task.status}</span>
                </header>
                <small>
                  {props.locale === "zh-CN" ? "层级" : "Depth"} {task.depth} · Run {task.childRunId}
                </small>
                <small>
                  {props.locale === "zh-CN" ? "预算" : "Budget"}:{" "}
                  {task.reservedBudget.maxModelRequests}/{task.reservedBudget.maxToolCalls} ·{" "}
                  {props.locale === "zh-CN" ? "用量" : "Usage"}:{" "}
                  {task.usage.inputTokens + task.usage.outputTokens}
                </small>
                <Show when={task.resultSummary}>
                  <p>{task.resultSummary}</p>
                </Show>
              </article>
            )}
          </For>
        </div>
      </section>
    </Show>
  );
}
