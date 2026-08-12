import { describe, expect, it } from "vitest";
import type { MotionTransitionProfile } from "@hachimi/contracts";
import { Euler, Quaternion, Vector3 } from "three";
import { AnimationGraph } from "./animation-graph";
import { FullPoseInertializer, zeroVelocity } from "./inertializer";
import { buildMotionFeatureIndex, inferFootContact } from "./motion-feature-index";
import { TransitionPlanner } from "./transition-planner";
import type { MotionFeatureFrame } from "./types";
import {
  deserializeMotionFeatureIndex,
  serializeMotionFeatureIndex,
} from "../motion-asset-library";

function pose(angle: number, root = 0) {
  return {
    rotations: new Map([
      ["hips", new Quaternion().setFromEuler(new Euler(0, angle, 0))],
      ["left_upper_arm", new Quaternion().setFromEuler(new Euler(angle * 0.5, 0, 0))],
    ]),
    hipsPosition: new Vector3(root, 0, 0),
    expressions: new Map([["happy", Math.min(Math.abs(angle), 1)]]),
    lookAt: { yawDegrees: angle * 20, pitchDegrees: angle * 5 },
  };
}

const profile: MotionTransitionProfile = {
  id: "action.standard",
  family: "gesture",
  preferredDurationMs: 150,
  minimumDurationMs: 80,
  maximumDurationMs: 240,
  interruptPolicy: "safe_point",
  blendProfileId: "dead_blend.v1",
  syncGroup: null,
  entryWindows: [{ startMs: 0, endMs: 120 }],
  exitWindows: [],
  channelMask: ["full_body"],
};

describe("Runtime V5 transition planning", () => {
  it("builds a skeleton/content/version feature key and selects the closest safe pose", () => {
    const index = buildMotionFeatureIndex({
      motionId: "target",
      contentHash: "sha",
      skeletonSignature: "hips,head",
      durationMs: 300,
      loop: false,
      entryWindows: profile.entryWindows,
      sample: (timeMs) => pose(timeMs / 300, timeMs / 3_000),
    });
    expect(index.cacheKey).toBe("hips,head:sha:v1");
    const source = index.frames[4]!;
    const plan = new TransitionPlanner().plan(source, index, profile, "safe_point");
    expect(plan.targetTimeMs).toBeCloseTo(source.timeMs, 4);
    expect(plan.targetTimeMs).toBeLessThanOrEqual(120);
  });

  it("round-trips every persisted feature channel without changing cache identity", () => {
    const index = buildMotionFeatureIndex({
      motionId: "persisted",
      contentHash: "content",
      skeletonSignature: "hips,head",
      durationMs: 180,
      loop: true,
      sample: (timeMs) => pose(timeMs / 180, timeMs / 1_800),
    });
    const restored = deserializeMotionFeatureIndex(
      serializeMotionFeatureIndex(index),
      index.cacheKey,
      index.motionId,
    );
    expect(restored?.cacheKey).toBe(index.cacheKey);
    expect(restored?.frames).toHaveLength(index.frames.length);
    expect(
      restored?.frames[2]?.pose.rotations
        .get("hips")
        ?.angleTo(index.frames[2]!.pose.rotations.get("hips")!),
    ).toBeLessThan(1e-7);
    expect(restored?.frames[2]?.velocity.hips.toArray()).toEqual(
      index.frames[2]!.velocity.hips.toArray(),
    );
    expect(restored?.frames[2]?.pose.lookAt).toEqual(index.frames[2]!.pose.lookAt);
  });

  it("dead-blends rotation, root, expression, and LookAt without exposing the target pose", () => {
    const inertializer = new FullPoseInertializer();
    const current = pose(0.8, 0.1);
    const target = pose(0, 0);
    inertializer.capture(current, target, zeroVelocity(), zeroVelocity());
    const first = inertializer.apply(target, 1 / 60);
    const settled = inertializer.apply(target, 1);
    expect(first.rotations.get("hips")!.angleTo(current.rotations.get("hips")!)).toBeLessThan(0.15);
    expect(first.hipsPosition!.x).toBeGreaterThan(0.08);
    expect(first.expressions.get("happy")).toBeGreaterThan(0.6);
    expect(first.lookAt!.yawDegrees).toBeGreaterThan(10);
    expect(settled.rotations.get("hips")!.angleTo(target.rotations.get("hips")!)).toBeLessThan(
      0.01,
    );
    expect(settled.hipsPosition!.x).toBeLessThan(0.001);
  });

  it("keeps planner p95 below one millisecond for a representative 60Hz index", () => {
    const index = buildMotionFeatureIndex({
      motionId: "target",
      contentHash: "sha",
      skeletonSignature: "standard",
      durationMs: 1_000,
      loop: true,
      sample: (timeMs) => pose(Math.sin(timeMs / 200) * 0.3),
    });
    const planner = new TransitionPlanner();
    const source = index.frames[20] as MotionFeatureFrame;
    const timings: number[] = [];
    for (let iteration = 0; iteration < 10_000; iteration += 1) {
      const started = performance.now();
      planner.plan(source, index, profile, "safe_point");
      timings.push(performance.now() - started);
    }
    timings.sort((left, right) => left - right);
    expect(timings[Math.floor(timings.length * 0.95)]).toBeLessThan(1);
  });

  it("arbitrates one deterministic winner per animation slot", () => {
    const entry = {
      id: "wave",
      slot: "action",
      transitionProfileId: profile.id,
      channelMask: ["full_body"],
      loopMode: "loop",
      durationMs: 1_000,
      mirrorable: true,
    } as never;
    const graph = new AnimationGraph(
      { entries: [entry], transitionProfiles: [profile as never] },
      () => pose(0),
    );
    for (const [requestId, priority] of [
      ["low", 40],
      ["touch", 90],
    ] as const) {
      graph.submit(
        {
          requestId,
          motionId: "wave",
          slot: "action",
          active: true,
          priority,
          interruptPolicy: "safe_point",
          mirror: false,
          channelWeights: [],
          locomotion: null,
        },
        0,
      );
    }
    expect(graph.update(120).map((layer) => layer.id)).toEqual(["touch"]);
  });

  it("uses profile-specific inertial half-lives in graph output", () => {
    const customProfile = {
      ...profile,
      inertialHalfLives: {
        rootMs: 140,
        bodyMs: 110,
        armsMs: 70,
        lookAtMs: 55,
        expressionMs: 45,
      },
    };
    const entry = {
      id: "custom",
      slot: "action",
      transitionProfileId: customProfile.id,
      channelMask: ["full_body"],
      loopMode: "loop",
      durationMs: 1_000,
      mirrorable: true,
    } as never;
    const graph = new AnimationGraph(
      { entries: [entry], transitionProfiles: [customProfile as never] },
      () => pose(0),
    );
    graph.submit(intent("custom", "immediate"), 0);
    expect(graph.update(1)[0]?.inertialHalfLives).toEqual({
      root: 0.14,
      body: 0.11,
      arms: 0.07,
      lookAt: 0.055,
      expression: 0.045,
    });
  });

  it("waits for a safe exit but forces entry at the 120ms interaction deadline", () => {
    const entries = ["source", "target"].map(
      (id) =>
        ({
          id,
          slot: "action",
          transitionProfileId: profile.id,
          channelMask: ["full_body"],
          loopMode: "loop",
          durationMs: 1_000,
          mirrorable: true,
        }) as never,
    );
    const graph = new AnimationGraph(
      { entries, transitionProfiles: [profile as never] },
      () => pose(0),
    );
    graph.setFeatureIndex(
      buildMotionFeatureIndex({
        motionId: "source",
        contentHash: "source",
        skeletonSignature: "standard",
        durationMs: 1_000,
        loop: true,
        exitWindows: [{ startMs: 340, endMs: 360 }],
        sample: () => pose(0),
      }),
    );
    graph.submit(intent("source", "immediate"), 0);
    expect(graph.update(300)[0]?.motionId).toBe("source");
    graph.submitWithOptions(intent("target", "safe_point"), 300, undefined, {
      maximumWaitMs: 120,
    });
    expect(graph.update(330)[0]?.motionId).toBe("source");
    expect(graph.update(360)[0]?.motionId).toBe("target");

    const noExitGraph = new AnimationGraph(
      { entries, transitionProfiles: [profile as never] },
      () => pose(0),
    );
    noExitGraph.setFeatureIndex(
      buildMotionFeatureIndex({
        motionId: "source",
        contentHash: "none",
        skeletonSignature: "standard",
        durationMs: 1_000,
        loop: true,
        exitWindows: [{ startMs: 900, endMs: 940 }],
        sample: () => pose(0),
      }),
    );
    noExitGraph.submit(intent("source", "immediate"), 0);
    noExitGraph.update(300);
    noExitGraph.submitWithOptions(intent("target", "safe_point"), 300, undefined, {
      maximumWaitMs: 120,
    });
    expect(noExitGraph.update(419)[0]?.motionId).toBe("source");
    expect(noExitGraph.update(420)[0]?.motionId).toBe("target");
  });

  it("keeps a winning finish intent until its one-shot clip completes", () => {
    const entry = {
      id: "wave",
      slot: "action",
      transitionProfileId: profile.id,
      channelMask: ["full_body"],
      loopMode: "once",
      durationMs: 1_000,
      mirrorable: true,
    } as never;
    const graph = new AnimationGraph(
      { entries: [entry], transitionProfiles: [profile as never] },
      () => pose(0),
    );
    graph.submit(
      {
        requestId: "finishing",
        motionId: "wave",
        slot: "action",
        active: true,
        priority: 40,
        interruptPolicy: "finish",
        mirror: false,
        channelWeights: [],
        locomotion: null,
      },
      0,
    );
    expect(graph.update(100)[0]?.id).toBe("finishing");
    graph.submit(
      {
        requestId: "touch",
        motionId: "wave",
        slot: "action",
        active: true,
        priority: 90,
        interruptPolicy: "immediate",
        mirror: false,
        channelWeights: [],
        locomotion: null,
      },
      120,
    );
    expect(graph.update(200)[0]?.id).toBe("finishing");
    expect(graph.update(1_001)[0]?.id).toBe("touch");
  });

  it("limits immediate target search to 120ms and uses the minimum blend duration", () => {
    const index = buildMotionFeatureIndex({
      motionId: "target",
      contentHash: "sha",
      skeletonSignature: "standard",
      durationMs: 500,
      loop: false,
      entryWindows: [{ startMs: 300, endMs: 400 }],
      sample: (timeMs) => pose(timeMs / 500),
    });
    const plan = new TransitionPlanner().plan(index.frames.at(-1), index, profile, "immediate");
    expect(plan.targetTimeMs).toBeLessThanOrEqual(120);
    expect(plan.durationMs).toBe(profile.minimumDurationMs);
    expect(plan.forced).toBe(true);
  });

  it("classifies stable and asymmetric foot motion without inventing missing contacts", () => {
    const velocity = zeroVelocity();
    expect(inferFootContact(velocity)).toBe("unknown");
    const stable = {
      ...velocity,
      angular: new Map([
        ["left_foot", new Vector3(0.1, 0, 0)],
        ["right_foot", new Vector3(0.2, 0, 0)],
      ]),
    };
    expect(inferFootContact(stable)).toBe("both");
    stable.angular.set("right_foot", new Vector3(2, 0, 0));
    expect(inferFootContact(stable)).toBe("left");
  });
});

function intent(motionId: string, interruptPolicy: "immediate" | "safe_point") {
  return {
    requestId: motionId,
    motionId,
    slot: "action" as const,
    active: true,
    priority: motionId === "target" ? 90 : 40,
    interruptPolicy,
    mirror: false,
    channelWeights: [],
    locomotion: null,
  };
}
