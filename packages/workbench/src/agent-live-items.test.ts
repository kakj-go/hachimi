import { describe, expect, it } from "vitest";

import type { RunEventEnvelope } from "@hachimi/contracts";
import { reconcilePendingUserInputs, reduceLiveItemDeltas } from "./agent-live-items";

const event = (payload: RunEventEnvelope["payload"], sequence: number): RunEventEnvelope => ({
  protocolVersion: 2,
  sequence,
  sessionId: "session-1" as RunEventEnvelope["sessionId"],
  runId: "run-1" as RunEventEnvelope["runId"],
  createdAtMs: sequence,
  payload,
});

describe("active item replay", () => {
  it("renders assistant, reasoning, and tool deltas independently", () => {
    const result = reduceLiveItemDeltas({}, [
      event(
        { type: "item_started", data: { item_id: "assistant-1" as never, kind: "assistant" } },
        1,
      ),
      event({ type: "item_delta", data: { item_id: "assistant-1" as never, delta: "hello" } }, 2),
      event(
        { type: "item_started", data: { item_id: "reasoning-1" as never, kind: "reasoning" } },
        3,
      ),
      event({ type: "item_delta", data: { item_id: "reasoning-1" as never, delta: "why" } }, 4),
      event(
        { type: "item_started", data: { item_id: "tool-1" as never, kind: "tool_execution" } },
        5,
      ),
      event({ type: "item_delta", data: { item_id: "tool-1" as never, delta: "running" } }, 6),
    ]);

    expect(result).toEqual({
      "assistant-1": { text: "hello", kind: "assistant" },
      "reasoning-1": { text: "why", kind: "reasoning" },
      "tool-1": { text: "running", kind: "tool_execution" },
    });
  });

  it("drops stale deltas after completed payload and bounds display memory", () => {
    const result = reduceLiveItemDeltas({}, [
      event({ type: "item_started", data: { item_id: "item-1" as never, kind: "assistant" } }, 1),
      event({ type: "item_delta", data: { item_id: "item-1" as never, delta: "old" } }, 2),
      event(
        {
          type: "item_completed",
          data: {
            item_id: "item-1" as never,
            status: "completed",
            payload: { type: "assistant", text: "final" } as never,
          },
        },
        3,
      ),
      event({ type: "item_delta", data: { item_id: "item-1" as never, delta: "late" } }, 4),
    ]);
    // A late delta cannot revive the completed transcript projection.
    expect(result["item-1"]).toBeUndefined();

    const bounded = reduceLiveItemDeltas({}, [
      event({ type: "item_started", data: { item_id: "item-2" as never, kind: "assistant" } }, 5),
      event(
        { type: "item_delta", data: { item_id: "item-2" as never, delta: "x".repeat(300_000) } },
        6,
      ),
    ]);
    expect(bounded["item-2"]?.text).toHaveLength(262_144);
  });

  it("keeps the stable pending UserInput identity across projection refreshes", () => {
    const current = { id: "input-1", questions: [] } as never;
    const replacement = { id: "input-1", questions: [] } as never;
    const next = reconcilePendingUserInputs([current], [replacement]);

    expect(next[0]).toBe(current);
    expect(reconcilePendingUserInputs([current], [])).toEqual([]);
  });
});
