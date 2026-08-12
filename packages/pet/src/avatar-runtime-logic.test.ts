import { describe, expect, it } from "vitest";
import {
  AMBIENT_MOTION_MAX_DELAY_MS,
  AMBIENT_MOTION_MIN_DELAY_MS,
  ambientMotionDelayMs,
  classifyRelativeHit,
  isPlaybackOlder,
  speechReleaseEnvelope,
  idleMotionWeight,
  rememberAmbientMotion,
  selectAmbientIdle,
  selectWaitingIdle,
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

  it("uses the generic waiting motion as the permanent base idle", () => {
    const standard = { id: "standard", name: "standard waiting" } as never;
    const waiting = { id: "waiting", name: "waiting" } as never;
    const cool = { id: "cool", name: "cool waiting" } as never;
    expect(selectWaitingIdle([standard, waiting, cool])).toBe(waiting);
    expect(selectWaitingIdle([standard, cool])).toBeUndefined();
  });

  it("selects only one-shot ambient actions and avoids recent motions", () => {
    const waiting = { id: "waiting", name: "waiting", loopMode: "loop" } as never;
    const happy = { id: "happy", name: "happy", loopMode: "once" } as never;
    const laughing = { id: "laughing", name: "laughing", loopMode: "once" } as never;
    const stretching = { id: "stretching", name: "stretching", loopMode: "once" } as never;
    expect(
      selectAmbientIdle([waiting, happy, laughing, stretching], ["happy", "laughing"], 0.8, 0),
    ).toBe(stretching);
    expect(selectAmbientIdle([waiting], [], 0.5, 0)).toBeUndefined();
  });

  it("randomizes ambient delays between 12 and 25 seconds", () => {
    expect(ambientMotionDelayMs(0)).toBe(AMBIENT_MOTION_MIN_DELAY_MS);
    expect(ambientMotionDelayMs(0.5)).toBe(18_500);
    expect(ambientMotionDelayMs(1)).toBe(AMBIENT_MOTION_MAX_DELAY_MS);
  });

  it("remembers up to three motions without exhausting the candidate pool", () => {
    expect(rememberAmbientMotion([], "only", 1)).toEqual([]);
    expect(rememberAmbientMotion(["a", "b"], "c", 3)).toEqual(["b", "c"]);
    expect(rememberAmbientMotion(["a", "b", "c"], "d", 5)).toEqual(["b", "c", "d"]);
  });
});
