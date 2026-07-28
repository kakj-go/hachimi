import type { McpCallSummaryRecord } from "@hachimi/contracts";
import { For, Show } from "solid-js";

export function McpCallHistory(props: {
  calls: McpCallSummaryRecord[];
  copy: (zh: string, en: string) => string;
}) {
  return (
    <section class="mcp-call-history" aria-labelledby="mcp-call-history-title">
      <header class="mcp-tools-heading">
        <div>
          <h2 id="mcp-call-history-title">
            {props.copy("最近调用", "Recent calls")} · {props.calls.length}
          </h2>
          <span>
            {props.copy(
              "仅保存 Tool、结果类型、耗时和关联 Run；不保存参数或返回正文。",
              "Only Tool identity, outcome, duration, and Run lineage are retained; arguments and response bodies are not stored.",
            )}
          </span>
        </div>
      </header>
      <Show
        when={props.calls.length > 0}
        fallback={
          <div class="extension-empty">
            {props.copy("暂无 Agent 调用记录。", "No Agent calls have been recorded yet.")}
          </div>
        }
      >
        <div class="mcp-call-list">
          <For each={props.calls}>
            {(call) => (
              <article class="mcp-call-row" data-outcome={call.outcome}>
                <span class="extension-status-dot" data-state={outcomeTone(call.outcome)} />
                <div class="mcp-call-copy">
                  <strong>{call.toolName}</strong>
                  <small>
                    {outcomeLabel(call.outcome, props.copy)} · {call.durationMs} ms
                  </small>
                </div>
                <time datetime={new Date(call.createdAtMs).toISOString()}>
                  {new Date(call.createdAtMs).toLocaleString()}
                </time>
              </article>
            )}
          </For>
        </div>
      </Show>
    </section>
  );
}

function outcomeTone(outcome: McpCallSummaryRecord["outcome"]) {
  if (outcome === "succeeded") return "ready";
  if (outcome === "cancelled") return "disabled";
  return "failed";
}

function outcomeLabel(
  outcome: McpCallSummaryRecord["outcome"],
  copy: (zh: string, en: string) => string,
) {
  switch (outcome) {
    case "succeeded":
      return copy("成功", "Succeeded");
    case "tool_error":
      return copy("工具返回错误", "Tool error");
    case "transport_error":
      return copy("连接错误", "Transport error");
    case "cancelled":
      return copy("已取消", "Cancelled");
  }
}
