import { Vector3, type Object3D } from "three";
import type { FootContactPhase } from "./constraint-solver";

export interface FootContactSample {
  phase: FootContactPhase;
  heightNormalized: number;
  verticalSpeedNormalized: number;
  planarSpeedNormalized: number;
}

export interface FootContactFrame {
  left: FootContactSample;
  right: FootContactSample;
  centerOfMass: [number, number, number];
}

export interface FootSoleOffsets {
  left: Vector3 | undefined;
  right: Vector3 | undefined;
}

interface FootHistory {
  position: Vector3 | undefined;
  phase: FootContactPhase;
}

/** Infers contact from the retargeted pose before IK, with hysteresis to prevent contact chatter. */
export class FootContactAnalyzer {
  private readonly left: FootHistory = { position: undefined, phase: "flat" };
  private readonly right: FootHistory = { position: undefined, phase: "flat" };

  reset(): void {
    this.left.position = undefined;
    this.right.position = undefined;
    this.left.phase = "flat";
    this.right.phase = "flat";
  }

  update(
    bones: ReadonlyMap<string, Object3D>,
    height: number,
    groundY: number,
    deltaSeconds: number,
    soleOffsets: FootSoleOffsets = { left: undefined, right: undefined },
  ): FootContactFrame {
    const safeHeight = Math.max(Math.abs(height), 0.001);
    const delta = Math.max(deltaSeconds, 1 / 240);
    const left = this.sampleFoot(
      bones.get("left_foot"),
      bones.get("left_toes"),
      this.left,
      safeHeight,
      groundY,
      delta,
      soleOffsets.left,
    );
    const right = this.sampleFoot(
      bones.get("right_foot"),
      bones.get("right_toes"),
      this.right,
      safeHeight,
      groundY,
      delta,
      soleOffsets.right,
    );
    return {
      left,
      right,
      centerOfMass: estimateCenterOfMass(bones, safeHeight),
    };
  }

  private sampleFoot(
    foot: Object3D | undefined,
    toes: Object3D | undefined,
    history: FootHistory,
    height: number,
    groundY: number,
    delta: number,
    soleOffset: Vector3 | undefined,
  ): FootContactSample {
    if (!foot) {
      return {
        phase: "air",
        heightNormalized: 1,
        verticalSpeedNormalized: 0,
        planarSpeedNormalized: 0,
      };
    }
    const position = soleOffset
      ? foot.localToWorld(soleOffset.clone())
      : foot.getWorldPosition(new Vector3());
    const previous = history.position;
    const velocity = previous
      ? position
          .clone()
          .sub(previous)
          .multiplyScalar(1 / delta / height)
      : new Vector3();
    history.position = position.clone();
    const footHeight = (position.y - groundY) / height;
    const toesHeight = toes
      ? (toes.getWorldPosition(new Vector3()).y - groundY) / height
      : footHeight;
    const releaseHeight = history.phase === "air" ? 0.014 : 0.026;
    const movingUp = velocity.y > 0.11;
    const movingFast = Math.hypot(velocity.x, velocity.z) > 0.42;
    let phase: FootContactPhase = "air";
    if (Math.min(footHeight, toesHeight) <= releaseHeight && !movingUp) {
      const tilt = toesHeight - footHeight;
      phase = tilt < -0.008 ? "toe" : tilt > 0.012 ? "heel" : "flat";
      if (movingFast && Math.max(footHeight, toesHeight) > 0.018) phase = "air";
    }
    history.phase = phase;
    return {
      phase,
      heightNormalized: footHeight,
      verticalSpeedNormalized: velocity.y,
      planarSpeedNormalized: Math.hypot(velocity.x, velocity.z),
    };
  }
}

export function estimateCenterOfMass(
  bones: ReadonlyMap<string, Object3D>,
  height: number,
): [number, number, number] {
  const hips = bones.get("hips");
  if (!hips) return [0, 0, 0];
  const origin = hips.getWorldPosition(new Vector3());
  const weighted = new Vector3();
  let total = 0;
  for (const [name, weight] of [
    ["hips", 0.42],
    ["spine", 0.18],
    ["chest", 0.2],
    ["head", 0.12],
    ["left_upper_leg", 0.04],
    ["right_upper_leg", 0.04],
  ] as const) {
    const bone = bones.get(name);
    if (!bone) continue;
    weighted.addScaledVector(bone.getWorldPosition(new Vector3()), weight);
    total += weight;
  }
  if (total <= 0) return [0, 0, 0];
  weighted
    .multiplyScalar(1 / total)
    .sub(origin)
    .multiplyScalar(1 / Math.max(height, 0.001));
  return [weighted.x, weighted.y, weighted.z];
}
