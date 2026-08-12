import { describe, expect, it } from "vitest";
import { motionFrameHealthy, transitionPeaksAccepted } from "./motion-lab-runtime";

describe("Motion Library Lab diagnostics", () => {
  it("rejects invalid solve frames", () => {
    const frame = {
      timeMs: 20,
      phase: 0.2,
      activeBones: 4,
      fingerBones: 30,
      maxAngleDegrees: 22,
      collisionCount: 0,
      maxFootDriftNormalized: 0.001,
      groundPenetrationNormalized: 0,
      maxJointCorrectionDegrees: 0,
      solveTimeMs: 0.4,
      compiledCacheSize: 3,
      activeBoneNames: ["hips", "left_index_distal"],
      rootPosition: [0, 0, 0] as const,
      rootDistance: 0,
      loopSeamDegrees: 0.3,
      loopSeamRootDistance: 0,
      leftFootPhase: "flat",
      rightFootPhase: "flat",
      contactTimeline: "FF FF",
    };
    expect(motionFrameHealthy(frame)).toBe(true);
    expect(
      motionFrameHealthy({
        ...frame,
        phase: Number.NaN,
      }),
    ).toBe(false);
    expect(
      motionFrameHealthy({
        ...frame,
        rootPosition: [Number.NaN, 0, 0],
      }),
    ).toBe(false);
  });

  it("applies the V5 transition admission thresholds and rejects non-finite peaks", () => {
    expect(
      transitionPeaksAccepted({
        peakBoneStepDegrees: 12,
        peakRootStepNormalized: 0.005,
        peakLookAtStepDegrees: 4,
      }),
    ).toBe(true);
    expect(
      transitionPeaksAccepted({
        peakBoneStepDegrees: 12.01,
        peakRootStepNormalized: 0.005,
        peakLookAtStepDegrees: 4,
      }),
    ).toBe(false);
    expect(
      transitionPeaksAccepted({
        peakBoneStepDegrees: Number.NaN,
        peakRootStepNormalized: 0,
        peakLookAtStepDegrees: 0,
      }),
    ).toBe(false);
  });
});
