import { describe, expect, it } from "vitest";

import { reduceAgentEventWatermark } from "./agent-event-watermark";

describe("agent event watermark", () => {
  it("deduplicates replayed and out-of-order events without regressing", () => {
    const update = reduceAgentEventWatermark(10, [
      { sequence: 12, event: "later" },
      { sequence: 9, event: "stale" },
      { sequence: 11, event: "next" },
      { sequence: 12, event: "duplicate" },
    ]);

    expect(update.events).toEqual([
      { sequence: 11, event: "next" },
      { sequence: 12, event: "later" },
    ]);
    expect(update.nextSequence).toBe(12);
  });

  it("accepts numeric gaps because transcript items share the session sequence", () => {
    const update = reduceAgentEventWatermark(20, [{ sequence: 24 }]);
    expect(update.events).toEqual([{ sequence: 24 }]);
    expect(update.nextSequence).toBe(24);
  });

  it("keeps the watermark unchanged when a pushed batch contains only replays", () => {
    const update = reduceAgentEventWatermark(30, [{ sequence: 29 }, { sequence: 30 }]);
    expect(update.events).toEqual([]);
    expect(update.nextSequence).toBe(30);
  });
});
