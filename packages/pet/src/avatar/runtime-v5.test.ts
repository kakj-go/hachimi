import { describe, expect, it } from "vitest";
import { Euler, MathUtils, Quaternion, Vector2, Vector3 } from "three";
import { runAvatarFramePipeline } from "./avatar-frame-pipeline";
import { BehaviorScheduler } from "./behavior-scheduler";
import { InteractionFeedbackRuntime } from "./interaction-feedback";
import { PetMotionOrchestrator } from "./motion-orchestrator";
import { limitPoseStep, motionEnvelope } from "./motion-continuity";

describe("Pet Runtime V5 behavior", () => {
  it("runs the continuity-sensitive frame stages in the fixed order", () => {
    const calls: string[] = [];
    runAvatarFramePipeline({
      sampleAndCompose: () => calls.push("compose"),
      inertialize: () => calls.push("inertia"),
      applyInteractionFeedback: () => calls.push("interaction"),
      solveFootContactsAndIk: () => calls.push("ik"),
      applyFaceGazeAndLipSync: () => calls.push("face"),
      updateSpringBones: () => calls.push("spring"),
    });
    expect(calls).toEqual(["compose", "inertia", "interaction", "ik", "face", "spring"]);
  });

  it("coalesces continuous interaction and releases it instead of replaying clips", () => {
    const feedback = new InteractionFeedbackRuntime();
    feedback.begin("head_top", 1, 0.25, 0);
    feedback.update(0.8, -1);
    expect(feedback.frame(30)).toMatchObject({ active: true, pressure: 0.8, direction: -1 });
    feedback.end(50);
    expect(feedback.frame(100).release).toBeGreaterThan(0);
    expect(feedback.frame(300).release).toBe(0);
    feedback.setDrag(true, new Vector2(0.6, -0.2), 400);
    expect(feedback.frame(410).dragVelocity.x).toBeCloseTo(0.6);
  });

  it("forces a direct safe-point interaction by 120ms", () => {
    const scheduler = new BehaviorScheduler<string>();
    scheduler.schedule({
      id: "touch",
      category: "interaction",
      slot: "action",
      priority: 90,
      interruptPolicy: "safe_point",
      requestedAt: 1_000,
      maximumWaitMs: 120,
      payload: "touch",
    });
    expect(scheduler.takeReady(1_119)).toHaveLength(0);
    expect(scheduler.takeReady(1_120)).toHaveLength(1);
  });

  it("gives direct interaction priority over autonomous actions", () => {
    const orchestrator = new PetMotionOrchestrator();
    for (const [requestId, priority] of [
      ["ambient", 40],
      ["touch", 90],
    ] as const) {
      orchestrator.submit(
        {
          requestId,
          motionId: requestId,
          slot: "action",
          active: true,
          priority,
          interruptPolicy: "safe_point",
          mirror: false,
          channelWeights: [],
          locomotion: null,
        },
        priority,
      );
    }
    expect(orchestrator.winners()[0]?.requestId).toBe("touch");
  });

  it("enforces the V5 per-frame bone, root, and LookAt continuity limits", () => {
    const previous = {
      rotations: new Map([["hips", new Quaternion()]]),
      hipsPosition: new Vector3(),
      expressions: new Map<string, number>(),
      lookAt: { yawDegrees: 0, pitchDegrees: 0 },
    };
    const next = {
      rotations: new Map([
        ["hips", new Quaternion().setFromEuler(new Euler(0, MathUtils.degToRad(90), 0))],
      ]),
      hipsPosition: new Vector3(1, 0, 0),
      expressions: new Map<string, number>(),
      lookAt: { yawDegrees: 30, pitchDegrees: -30 },
    };
    const metrics = limitPoseStep(next, previous, 2);
    expect(previous.rotations.get("hips")!.angleTo(next.rotations.get("hips")!)).toBeCloseTo(
      MathUtils.degToRad(12),
    );
    expect(next.hipsPosition.length()).toBeCloseTo(0.01);
    expect(next.lookAt).toEqual({ yawDegrees: 4, pitchDegrees: -4 });
    expect(metrics.boneDegrees).toBeCloseTo(12);
    expect(metrics.rootHeightRatio).toBeCloseTo(0.005);
    expect(metrics.lookAtDegrees).toBe(4);
  });

  it("uses transition profiles instead of fixed V4 fade values", () => {
    const entry = { loopMode: "once", durationMs: 1_000 } as never;
    const transition = { preferredDurationMs: 100 } as never;
    expect(motionEnvelope(entry, 50, transition)).toBeCloseTo(0.5);
    expect(motionEnvelope(entry, 950, transition)).toBeCloseTo(0.5);
  });

  it("replaces non-finite pose channels before they can reach the avatar skeleton", () => {
    const pose = {
      rotations: new Map([["hips", new Quaternion(Number.NaN, 0, 0, 1)]]),
      hipsPosition: new Vector3(Number.NaN, 0, 0),
      expressions: new Map([["happy", Number.NaN]]),
      lookAt: { yawDegrees: Number.NaN, pitchDegrees: Number.POSITIVE_INFINITY },
    };
    limitPoseStep(pose, undefined, 2);
    expect(pose.rotations.get("hips")?.toArray().every(Number.isFinite)).toBe(true);
    expect(pose.hipsPosition.toArray().every(Number.isFinite)).toBe(true);
    expect(pose.expressions.get("happy")).toBe(0);
    expect(pose.lookAt).toEqual({ yawDegrees: 0, pitchDegrees: 0 });
  });
});
