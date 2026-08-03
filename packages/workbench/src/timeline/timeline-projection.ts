import type {
  RunSummaryRecord,
  TranscriptItem,
  TranscriptItemKind,
  WorkbenchSessionSnapshot,
} from "@hachimi/contracts";

export type TimelineProjectionEntry =
  | { kind: "item"; item: TranscriptItem }
  | { kind: "tool_batch"; id: string; items: TranscriptItem[] }
  | { kind: "run_summary"; summary: RunSummaryRecord };

const TOOL_ACTIVITY_KINDS = new Set<TranscriptItemKind>([
  "tool_execution",
  "command_execution",
  "file_change",
  "mcp_call",
  "dynamic_tool_call",
]);

export function isToolActivity(kind: TranscriptItemKind): boolean {
  return TOOL_ACTIVITY_KINDS.has(kind);
}

export function isDiffToolActivity(item: TranscriptItem): boolean {
  return (
    item.kind === "tool_execution" &&
    item.payload.type === "tool_execution" &&
    item.payload.data.name === "apply_patch"
  );
}

/**
 * Projects the durable transcript into Codex-style display groups. Tool calls
 * stay individually persisted and addressable, but contiguous calls from the
 * same run render as one expandable batch. Terminal run summaries are placed
 * at the end of their own run instead of being detached at the timeline tail.
 */
export function projectSessionTimeline(
  snapshot: Pick<WorkbenchSessionSnapshot, "runs" | "transcript" | "runSummaries">,
): TimelineProjectionEntry[] {
  const entries: TimelineProjectionEntry[] = [];
  const summaries = new Map(snapshot.runSummaries.map((summary) => [summary.runId, summary]));
  const emittedSummaries = new Set<string>();
  const transcript = snapshot.transcript
    .filter((item) => item.kind !== "collab_tool_call")
    .toSorted((left, right) => left.sequence - right.sequence || left.id.localeCompare(right.id));

  const emitSummary = (runId: string | null) => {
    if (!runId || emittedSummaries.has(runId)) return;
    const summary = summaries.get(runId);
    if (!summary) return;
    entries.push({ kind: "run_summary", summary });
    emittedSummaries.add(runId);
  };

  for (let index = 0; index < transcript.length; index += 1) {
    const item = transcript[index];
    if (!item) continue;
    if (isToolActivity(item.kind)) {
      const items = [item];
      while (
        index + 1 < transcript.length &&
        transcript[index + 1]?.runId === item.runId &&
        isToolActivity(transcript[index + 1]!.kind)
      ) {
        index += 1;
        items.push(transcript[index]!);
      }
      entries.push({ kind: "tool_batch", id: `tool-batch-${item.id}`, items });
    } else {
      entries.push({ kind: "item", item });
    }

    const next = transcript[index + 1];
    if (item.runId && next?.runId !== item.runId) emitSummary(item.runId);
  }

  for (const run of snapshot.runs) emitSummary(run.id);
  return entries;
}

export function toolBatchLabel(items: readonly TranscriptItem[], zh: boolean): string {
  const count = items.length;
  if (items.every((item) => item.kind === "command_execution")) {
    return zh
      ? count === 1
        ? "运行了命令"
        : `运行了 ${count} 个命令`
      : count === 1
        ? "Ran a command"
        : `Ran ${count} commands`;
  }
  if (items.every((item) => item.kind === "file_change" || isDiffToolActivity(item))) {
    return zh
      ? count === 1
        ? "编辑了文件"
        : `编辑了 ${count} 个文件`
      : count === 1
        ? "Edited a file"
        : `Edited ${count} files`;
  }
  return zh
    ? count === 1
      ? "使用了工具"
      : `使用了 ${count} 个工具`
    : count === 1
      ? "Used a tool"
      : `Used ${count} tools`;
}
