import { describe, expect, it } from "vitest";
import { Bone, Euler, Quaternion, Vector3 } from "three";
import { FootContactAnalyzer } from "./foot-contact-analyzer";
import {
  channelsForFullBody,
  channelsForSpeechBody,
  composeMotionLayers,
  measureMotionPoseSeam,
} from "./motion-composer";
import { StageLocomotionController } from "./stage-locomotion";
import { selectMotionForIntent } from "./motion-semantic-selector";
import { ConstraintSolver, type LegChain } from "./constraint-solver";
import { MOTION_COMPILED_CACHE_LIMIT, pruneLeastRecentlyUsed } from "./motion-asset-library";

function pose(rotations: ReadonlyMap<string, Quaternion>) {
  return { rotations, expressions: new Map<string, number>() };
}

function randomLeg(seed: number): { root: Bone; chain: LegChain } {
  const random = mulberry32(seed);
  const root = new Bone();
  const upper = new Bone();
  const lower = new Bone();
  const foot = new Bone();
  upper.position.set((random() - 0.5) * 0.3, -(0.28 + random() * 0.45), 0);
  lower.position.set(0, -(0.25 + random() * 0.42), (random() - 0.5) * 0.06);
  foot.position.set(0, -(0.06 + random() * 0.12), 0.05 + random() * 0.18);
  root.add(upper);
  upper.add(lower);
  lower.add(foot);
  root.updateWorldMatrix(true, true);
  return { root, chain: { upper, lower, foot, pole: new Vector3(0, 0, 1) } };
}

describe("Avatar Motion Runtime V5 composition", () => {
  it("selects finger-complete semantic motions before rigid alternatives", () => {
    const rigid = {
      id: "rigid",
      family: "speech",
      name: "Talking",
      tags: ["talking"],
      description: "talking",
      hasFingerMotion: false,
      fingerBoneCount: 0,
      sourceProject: "Clawatar",
    } as never;
    const natural = {
      id: "natural",
      family: "speech",
      name: "Open palm talking",
      tags: ["talking"],
      description: "talking",
      hasFingerMotion: true,
      fingerBoneCount: 30,
      sourceProject: "OpenMaiWaifu",
    } as never;
    expect(
      selectMotionForIntent([rigid, natural], {
        family: "speech",
        tags: ["talking"],
        preferFingerMotion: true,
        preferredSource: "OpenMaiWaifu",
      })?.id,
    ).toBe("natural");
  });

  it("returns exactly to normalized rest when the last one-shot layer is removed", () => {
    const rest = new Map([
      ["hips", new Quaternion().setFromEuler(new Euler(0.05, 0, 0))],
      ["left_upper_arm", new Quaternion().setFromEuler(new Euler(0, 0, 0.12))],
    ]);
    const result = composeMotionLayers(rest, []);
    for (const [bone, rotation] of rest) {
      expect(result.rotations.get(bone)?.angleTo(rotation)).toBeLessThan(1e-8);
    }
    expect(result.contributingLayerIds).toHaveLength(0);
  });

  it("keeps the per-avatar compiled LRU at 24 and preserves the active motion", () => {
    const slots = new Map(
      Array.from({ length: 40 }, (_, index) => [`motion-${index}`, { lastUsed: index }]),
    );
    pruneLeastRecentlyUsed(slots, MOTION_COMPILED_CACHE_LIMIT, "motion-0", (slot) => slot.lastUsed);
    expect(slots.size).toBe(24);
    expect(slots.has("motion-0")).toBe(true);
    expect(slots.has("motion-1")).toBe(false);
  });

  it("keeps speech arms independent from lower body and mouth", () => {
    const rest = new Map([
      ["hips", new Quaternion()],
      ["left_upper_arm", new Quaternion()],
      ["jaw", new Quaternion()],
    ]);
    const idleHips = new Quaternion().setFromEuler(new Euler(0.2, 0, 0));
    const speechArm = new Quaternion().setFromEuler(new Euler(0, 0, 0.8));
    const speechJaw = new Quaternion().setFromEuler(new Euler(0.4, 0, 0));
    const result = composeMotionLayers(rest, [
      {
        id: "idle",
        pose: pose(new Map([["hips", idleHips]])),
        priority: 0,
        weight: 1,
        channels: channelsForFullBody(),
      },
      {
        id: "speech",
        pose: pose(
          new Map([
            ["hips", new Quaternion().setFromEuler(new Euler(1, 0, 0))],
            ["left_upper_arm", speechArm],
            ["jaw", speechJaw],
          ]),
        ),
        priority: 40,
        weight: 1,
        channels: channelsForSpeechBody(),
      },
    ]);
    expect(result.rotations.get("hips")?.angleTo(idleHips)).toBeLessThan(1e-6);
    expect(result.rotations.get("left_upper_arm")?.angleTo(speechArm)).toBeLessThan(1e-6);
    expect(result.rotations.get("jaw")?.angleTo(new Quaternion())).toBeLessThan(1e-6);
  });

  it("measures real loop rotation and root seams", () => {
    const seam = measureMotionPoseSeam(
      {
        ...pose(new Map([["hips", new Quaternion()]])),
        hipsPosition: new Vector3(),
      },
      {
        ...pose(new Map([["hips", new Quaternion().setFromEuler(new Euler(0, Math.PI / 6, 0))]])),
        hipsPosition: new Vector3(0.1, 0, 0),
      },
    );
    expect(seam.maxRotationDegrees).toBeCloseTo(30, 4);
    expect(seam.rootDistance).toBeCloseTo(0.1, 6);
  });
});

describe("dynamic contact and locomotion", () => {
  it("uses height and velocity hysteresis for foot contacts", () => {
    const hips = new Bone();
    const left = new Bone();
    const right = new Bone();
    hips.add(left, right);
    left.position.set(-0.1, 0.005, 0);
    right.position.set(0.1, 0.005, 0);
    hips.updateWorldMatrix(true, true);
    const analyzer = new FootContactAnalyzer();
    const grounded = analyzer.update(
      new Map([
        ["hips", hips],
        ["left_foot", left],
        ["right_foot", right],
      ]),
      1.7,
      0,
      1 / 60,
    );
    expect(grounded.left.phase).toBe("flat");
    left.position.y = 0.2;
    hips.updateWorldMatrix(true, true);
    const lifted = analyzer.update(
      new Map([
        ["hips", hips],
        ["left_foot", left],
        ["right_foot", right],
      ]),
      1.7,
      0,
      1 / 60,
    );
    expect(lifted.left.phase).toBe("air");
    expect(lifted.right.phase).toBe("flat");
  });

  it("starts, walks, brakes, and stays inside the Pet stage", () => {
    const locomotion = new StageLocomotionController();
    locomotion.walkTo(10);
    const phases = new Set<string>();
    let frame = locomotion.update(1 / 60);
    for (let index = 0; index < 300; index += 1) {
      frame = locomotion.update(1 / 60);
      phases.add(frame.phase);
    }
    expect(phases.has("walk")).toBe(true);
    expect(phases.has("stop")).toBe(true);
    expect(frame.phase).toBe("idle");
    expect(frame.positionX).toBeCloseTo(0.3, 4);
  });

  it("holds the turn phase until velocity follows the new facing", () => {
    const locomotion = new StageLocomotionController();
    locomotion.walkTo(0.3);
    let frame = locomotion.update(1 / 60);
    while (frame.phase !== "walk") frame = locomotion.update(1 / 60);
    locomotion.walkTo(-0.3);
    const phases: string[] = [];
    for (let index = 0; index < 120; index += 1) {
      frame = locomotion.update(1 / 60);
      phases.push(frame.phase);
      if (frame.phase === "walk" && frame.facing === -1) break;
    }
    expect(phases[0]).toBe("turn");
    expect(phases).toContain("turn");
    expect(frame).toMatchObject({ phase: "walk", facing: -1 });
  });

  it("solves 500 legal body proportions without NaN and keeps p95 below 2ms", () => {
    const solveTimes: number[] = [];
    for (let index = 1; index <= 500; index += 1) {
      const { root, chain } = randomLeg(index);
      const target = chain.foot
        .getWorldPosition(new Vector3())
        .add(new Vector3(Math.sin(index) * 0.04, Math.cos(index * 0.7) * 0.025, 0));
      const solver = new ConstraintSolver();
      const started = performance.now();
      solver.updateLowerBody(root, chain, undefined, true, false, 0.82);
      solveTimes.push(performance.now() - started);
      root.updateWorldMatrix(true, true);
      expect(chain.foot.quaternion.toArray().every(Number.isFinite)).toBe(true);
      expect(target.toArray().every(Number.isFinite)).toBe(true);
    }
    solveTimes.sort((left, right) => left - right);
    expect(solveTimes[Math.floor(solveTimes.length * 0.95)]).toBeLessThan(2);
  });
});

function mulberry32(seed: number): () => number {
  return () => {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let value = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    value = (value + Math.imul(value ^ (value >>> 7), 61 | value)) ^ value;
    return ((value ^ (value >>> 14)) >>> 0) / 4_294_967_296;
  };
}
