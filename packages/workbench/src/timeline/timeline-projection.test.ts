import type { TranscriptItem, WorkbenchSessionSnapshot } from "@hachimi/contracts";
import { describe, expect, it } from "vitest";

import { isDiffToolActivity, projectSessionTimeline, toolBatchLabel } from "./timeline-projection";

function item(
  id: string,
  sequence: number,
  kind: TranscriptItem["kind"],
  runId = "run-1",
): TranscriptItem {
  const payload =
    kind === "command_execution"
      ? {
          type: "command_execution" as const,
          data: {
            process_session_id: id,
            command_summary: "pnpm test",
            command: "pnpm test",
            cwd: null,
            status: "succeeded",
            aggregated_output: "ok",
            exit_code: 0,
            duration_ms: 10,
            output_artifact_id: null,
          },
        }
      : kind === "assistant"
        ? { type: "assistant" as const, data: { text: "done" } }
        : {
            type: "file_change" as const,
            data: { path: "src/main.rs", change_kind: "modified", artifact_id: null },
          };
  return {
    id,
    sessionId: "session-1",
    runId,
    sequence,
    kind,
    status: "completed",
    payload,
    relations: { artifactIds: [] },
    createdAtMs: sequence,
  } as TranscriptItem;
}

function applyPatchItem(): TranscriptItem {
  return {
    ...item("patch", 1, "file_change"),
    kind: "tool_execution",
    payload: {
      type: "tool_execution",
      data: {
        tool_call_id: "tool-call-patch",
        name: "apply_patch",
        arguments: { patch: "*** Begin Patch" },
        step_revision: 1,
        tool_plan_hash: "plan-hash",
        registry_revision: "registry-revision",
        result: null,
      },
    },
  } as TranscriptItem;
}

describe("projectSessionTimeline", () => {
  it("groups contiguous tool activity and places each summary after its run", () => {
    const snapshot = {
      transcript: [
        item("assistant", 1, "assistant"),
        item("command", 2, "command_execution"),
        item("file", 3, "file_change"),
      ],
      runs: [{ id: "run-1" }],
      runSummaries: [
        {
          runId: "run-1",
          status: "succeeded",
          changedFiles: 1,
          additions: 2,
          deletions: 1,
          files: [],
          diffArtifactId: null,
          diffUnavailable: false,
          completedAtMs: 4,
        },
      ],
    } as unknown as Pick<WorkbenchSessionSnapshot, "runs" | "transcript" | "runSummaries">;

    const projected = projectSessionTimeline(snapshot);
    expect(projected.map((entry) => entry.kind)).toEqual(["item", "tool_batch", "run_summary"]);
    expect(projected[1]?.kind === "tool_batch" && projected[1].items).toHaveLength(2);
  });

  it("uses compact localized batch labels", () => {
    const commands = [item("one", 1, "command_execution"), item("two", 2, "command_execution")];
    expect(toolBatchLabel(commands, true)).toBe("运行了 2 个命令");
    expect(toolBatchLabel(commands, false)).toBe("Ran 2 commands");
  });

  it("treats apply_patch as a diff-backed file edit", () => {
    const patch = applyPatchItem();
    expect(isDiffToolActivity(patch)).toBe(true);
    expect(toolBatchLabel([patch], true)).toBe("编辑了文件");
    expect(toolBatchLabel([patch], false)).toBe("Edited a file");
  });

  it("removes collaboration items from the primary timeline", () => {
    const snapshot = {
      transcript: [
        item("assistant-before", 1, "assistant"),
        item("collaboration", 2, "collab_tool_call"),
        item("assistant-after", 3, "assistant"),
      ],
      runs: [{ id: "run-1" }],
      runSummaries: [],
    } as unknown as Pick<WorkbenchSessionSnapshot, "runs" | "transcript" | "runSummaries">;

    const projected = projectSessionTimeline(snapshot);
    expect(projected).toHaveLength(2);
    expect(
      projected.some((entry) => entry.kind === "item" && entry.item.kind === "collab_tool_call"),
    ).toBe(false);
  });
});
