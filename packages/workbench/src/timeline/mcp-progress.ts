import type { McpToolProgressRecord, WorkbenchSessionSnapshot } from "@hachimi/contracts";

type ValidMcpProgress = Omit<McpToolProgressRecord, "progress"> & { progress: number };

export function latestMcpProgress(
  snapshot: WorkbenchSessionSnapshot,
  runId: string | undefined,
): ValidMcpProgress[] {
  if (!runId) return [];
  const latest = new Map<string, ValidMcpProgress>();
  for (const event of snapshot.events) {
    if (
      event.payload.type !== "generic" ||
      event.payload.data.event !== "mcp.tool.progress" ||
      event.runId !== runId
    ) {
      continue;
    }
    const progress = parseMcpProgress(event.payload.data.data);
    if (progress) latest.set(progress.toolCallId, progress);
  }
  return [...latest.values()];
}

function parseMcpProgress(value: unknown): ValidMcpProgress | undefined {
  if (!value || typeof value !== "object") return undefined;
  const record = value as Record<string, unknown>;
  if (
    typeof record.serverId !== "string" ||
    typeof record.sessionId !== "string" ||
    typeof record.runId !== "string" ||
    typeof record.runGeneration !== "number" ||
    typeof record.toolCallId !== "string" ||
    typeof record.progress !== "number" ||
    !Number.isFinite(record.progress) ||
    record.progress < 0
  ) {
    return undefined;
  }
  if (
    record.total !== null &&
    (typeof record.total !== "number" || !Number.isFinite(record.total) || record.total <= 0)
  ) {
    return undefined;
  }
  if (record.message !== null && typeof record.message !== "string") return undefined;
  return record as ValidMcpProgress;
}
