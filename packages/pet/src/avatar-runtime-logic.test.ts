import { describe, expect, it } from "vitest";
import {
  classifyRelativeHit,
  isPlaybackOlder,
  speechReleaseEnvelope,
  idleMotionWeight,
  canStartQueuedInteraction,
  selectAmbientIdle,
  selectShyIdle,
  selectWeightedIdle,
} from "./avatar-runtime-logic";

describe("avatar runtime decisions", () => {
  it("uses exactly one catalog motion per interaction region", () => {
    const binding = {
      region: "right_hand" as const,
      motionId: "wave",
      cooldownMs: 1_500,
      mirrorBySide: true,
    };
    expect(binding.motionId).toBe("wave");
  });

  it("classifies bounding-box fallback regions", () => {
    expect(classifyRelativeHit(0, 0.9)).toBe("head_top");
    expect(classifyRelativeHit(0.8, 0.55)).toBe("right_hand");
    expect(classifyRelativeHit(0, 0.55)).toBe("chest");
    expect(classifyRelativeHit(-0.2, 0.1)).toBe("left_leg");
  });

  it("rejects late numeric playback generations without assuming UUID ordering", () => {
    expect(isPlaybackOlder("8", "9")).toBe(true);
    expect(isPlaybackOlder("10", "9")).toBe(false);
    expect(isPlaybackOlder("playback-a", "playback-b")).toBe(false);
  });

  it("releases a stopped mouth naturally below the acceptance threshold", () => {
    expect(speechReleaseEnvelope(0.8, 0)).toBeCloseTo(0.8);
    expect(speechReleaseEnvelope(0.8, 55)).toBeCloseTo(0.4);
    expect(speechReleaseEnvelope(0.8, 120)).toBeLessThan(0.05);
  });

  it("weights calm and energetic OpenMai idles by runtime energy", () => {
    const calm = { name: "shy waiting" } as never;
    const active = { name: "energetic waiting" } as never;
    expect(idleMotionWeight(calm, 0.1)).toBeGreaterThan(idleMotionWeight(active, 0.1));
    expect(idleMotionWeight(active, 0.95)).toBeGreaterThan(idleMotionWeight(calm, 0.95));
    expect(selectWeightedIdle([calm, active], 0.95, 0.99)).toBe(active);
  });

  it("keeps shy waiting as the permanent base idle", () => {
    const standard = { id: "standard", name: "standard waiting" } as never;
    const shy = { id: "shy", name: "shy waiting" } as never;
    expect(selectShyIdle([standard, shy])).toBe(shy);
  });

  it("selects a non-shy ambient motion without immediately repeating", () => {
    const shy = { id: "shy", name: "shy waiting" } as never;
    const calm = { id: "calm", name: "ladylike waiting" } as never;
    const active = { id: "active", name: "energetic waiting" } as never;
    expect(selectAmbientIdle([shy, calm, active], "calm", 0.8, 0)).toBe(active);
  });

  it("holds queued interaction until the foreground finishes and shy idle settles", () => {
    expect(canStartQueuedInteraction(1_000, 900, 950, true)).toBe(false);
    expect(canStartQueuedInteraction(1_000, 900, 1_100, false)).toBe(false);
    expect(canStartQueuedInteraction(1_100, 900, 1_100, false)).toBe(true);
  });
});
