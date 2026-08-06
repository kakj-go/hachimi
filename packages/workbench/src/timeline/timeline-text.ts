import type { AppLocale } from "@hachimi/i18n";

export function timelineKindLabel(kind: string, locale: AppLocale): string {
  const labels: Record<string, [string, string]> = {
    user: ["用户", "User"],
    assistant: ["Hachimi", "Hachimi"],
    reasoning: ["思考", "Reasoning"],
    tool_call: ["工具调用", "Tool call"],
    tool_execution: ["工具执行", "Tool execution"],
    tool_result: ["工具结果", "Tool result"],
    plan: ["计划", "Plan"],
    approval: ["审批", "Approval"],
    user_input_request: ["用户输入", "User input"],
    command_execution: ["命令", "Command"],
    file_change: ["文件更改", "File changes"],
    mcp_call: ["MCP 调用", "MCP call"],
    dynamic_tool_call: ["动态工具", "Dynamic tool"],
    collab_tool_call: ["子代理", "Sub-agent"],
    context_compaction: ["上下文压缩", "Context compaction"],
    review: ["代码审查", "Review"],
    system_context: ["系统", "System"],
  };
  const label = labels[kind];
  return label ? label[locale === "zh-CN" ? 0 : 1] : kind;
}

export function timelineActivityLabel(kind: string, content: unknown, locale: AppLocale): string {
  if (!content || typeof content !== "object") return timelineKindLabel(kind, locale);
  const record = content as Record<string, unknown>;
  const data =
    record.data && typeof record.data === "object"
      ? (record.data as Record<string, unknown>)
      : record;
  if (record.type === "tool_execution" && typeof data.name === "string") return data.name;
  if (record.type === "mcp_call") {
    const tool = data.tool_name ?? data.toolName;
    if (typeof tool === "string") return `MCP · ${tool}`;
  }
  if (record.type === "dynamic_tool_call") {
    const namespace = typeof data.namespace === "string" ? data.namespace : "tool";
    const name = typeof data.name === "string" ? data.name : "call";
    return `${namespace}.${name}`;
  }
  if (record.type === "collab_tool_call" && typeof data.title === "string") return data.title;
  return timelineKindLabel(kind, locale);
}

export function timelineItemText(content: unknown, liveDelta?: string): string {
  if (liveDelta !== undefined) return clipTimelineText(liveDelta);
  if (typeof content === "string") return content;
  if (!content || typeof content !== "object") return "Activity completed";
  const record = content as Record<string, unknown>;
  const data =
    record.data && typeof record.data === "object"
      ? (record.data as Record<string, unknown>)
      : record;
  for (const key of ["text", "summary", "message", "command_summary", "commandSummary"] as const) {
    if (typeof data[key] === "string") return clipTimelineText(data[key]);
  }
  if (record.type === "tool_execution") {
    const result = data.result as Record<string, unknown> | undefined;
    const status = String(result?.status ?? "running");
    const resultCode =
      typeof result?.stableResultCode === "string" && result.stableResultCode
        ? result.stableResultCode
        : undefined;
    const summary = summarizeToolResult(result?.modelContent, result?.structuredContent);
    return clipTimelineText(
      [status, resultCode && resultCode !== status ? resultCode : undefined, summary]
        .filter(Boolean)
        .join(" · "),
    );
  }
  if (record.type === "command_execution") {
    const output = data.aggregated_output ?? data.aggregatedOutput;
    return clipTimelineText(
      typeof output === "string" && output
        ? `${String(data.command_summary ?? data.commandSummary ?? data.command ?? "Command")}\n${output}`
        : String(data.command_summary ?? data.commandSummary ?? data.command ?? "Command"),
    );
  }
  if (record.type === "user_input_request") return userInputText(data);
  if (record.type === "file_change") return String(data.path ?? "Files changed");
  if (record.type === "mcp_call") {
    const detail =
      typeof data.error === "string" && data.error
        ? data.error
        : summarizeStructuredValue(data.result);
    return [String(data.status ?? "running"), detail].filter(Boolean).join(" · ");
  }
  if (record.type === "dynamic_tool_call") {
    const detail =
      typeof data.error === "string" && data.error
        ? data.error
        : summarizeStructuredValue(data.result);
    return [String(data.status ?? "running"), detail].filter(Boolean).join(" · ");
  }
  if (record.type === "collab_tool_call") {
    const title = String(data.title ?? data.tool_name ?? data.toolName ?? "Sub-agent");
    const summary = data.summary ? "\n" + String(data.summary) : "";
    return title + " · " + String(data.status ?? "running") + summary;
  }
  if (record.type === "approval") return String(data.summary ?? "Approval");
  if (record.type === "context_compaction") return "Context compacted";
  return "Activity completed";
}

function summarizeToolResult(
  modelContent: unknown,
  structuredContent: unknown,
): string | undefined {
  if (typeof modelContent === "string" && modelContent.trim()) {
    const content = modelContent.trim();
    try {
      return summarizeStructuredValue(JSON.parse(content));
    } catch {
      return content;
    }
  }
  return summarizeStructuredValue(structuredContent);
}

function summarizeStructuredValue(value: unknown): string | undefined {
  if (value === null || value === undefined) return undefined;
  if (typeof value === "string" || typeof value === "number" || typeof value === "boolean")
    return String(value);
  if (Array.isArray(value)) {
    if (value.length === 0) return "0 items";
    const samples = value
      .slice(0, 3)
      .map((entry) => summarizeStructuredValue(entry))
      .filter((entry): entry is string => Boolean(entry));
    return [`${value.length} items`, ...samples].join(" · ");
  }
  if (typeof value !== "object") return undefined;
  const record = value as Record<string, unknown>;
  const fields = [
    "message",
    "summary",
    "title",
    "displayName",
    "name",
    "action",
    "status",
    "health",
    "resultCode",
    "result_code",
    "stableResultCode",
    "stable_result_code",
    "errorCode",
    "error_code",
    "fileName",
    "origin",
    "error",
    "code",
  ];
  const parts = fields
    .filter((key) => record[key] !== undefined)
    .slice(0, 4)
    .map((key) => summarizeStructuredValue(record[key]))
    .filter((entry): entry is string => Boolean(entry));
  if (parts.length > 0) return parts.join(" · ");
  const output = record.output;
  return output === value ? undefined : summarizeStructuredValue(output);
}

function userInputText(data: Record<string, unknown>): string {
  const questions = Array.isArray(data.questions)
    ? (data.questions as Record<string, unknown>[])
    : [];
  const answerValue = data.display_answers ?? data.displayAnswers;
  const answers = Array.isArray(answerValue) ? (answerValue as Record<string, unknown>[]) : [];
  return questions
    .map((question) => {
      const answer = answers.find(
        (candidate) => (candidate.question_id ?? candidate.questionId) === question.id,
      );
      const shown =
        (answer?.secret_provided ?? answer?.secretProvided) ? "已提供敏感回答" : answer?.value;
      return `${String(question.header ?? "Question")}: ${String(shown ?? question.prompt ?? "")}`;
    })
    .join("\n");
}

export function clipTimelineText(value: string): string {
  return value.length > 6_000 ? `${value.slice(0, 3_000)}\n…\n${value.slice(-3_000)}` : value;
}
