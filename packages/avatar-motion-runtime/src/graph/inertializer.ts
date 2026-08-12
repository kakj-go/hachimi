import { Quaternion, Vector2, Vector3 } from "three";
import type { SampledMotionPose } from "../motion-asset-library";
import { estimateMotionPoseVelocity } from "./motion-feature-index";
import type { MotionPoseVelocity } from "./types";

interface RotationState {
  offset: Vector3;
  velocity: Vector3;
}

interface ScalarState {
  offset: number;
  velocity: number;
}

export interface InertialHalfLives {
  root: number;
  body: number;
  arms: number;
  lookAt: number;
  expression: number;
}

export const DEFAULT_INERTIAL_HALF_LIVES: Readonly<InertialHalfLives> = {
  root: 0.1,
  body: 0.08,
  arms: 0.065,
  lookAt: 0.06,
  expression: 0.05,
};

/** Dead-blends the rendered pose and its velocity onto a newly sampled target pose. */
export class FullPoseInertializer {
  private readonly rotations = new Map<string, RotationState>();
  private readonly expressions = new Map<string, ScalarState>();
  private hipsOffset = new Vector3();
  private hipsVelocity = new Vector3();
  private lookAtOffset = new Vector2();
  private lookAtVelocity = new Vector2();
  private elapsed = Number.POSITIVE_INFINITY;

  capture(
    current: SampledMotionPose,
    target: SampledMotionPose,
    currentVelocity: MotionPoseVelocity = zeroVelocity(),
    targetVelocity: MotionPoseVelocity = zeroVelocity(),
  ): void {
    this.reset();
    for (const [name, targetRotation] of target.rotations) {
      const currentRotation = current.rotations.get(name);
      if (!currentRotation) continue;
      const difference = targetRotation.clone().invert().multiply(currentRotation).normalize();
      if (difference.w < 0) difference.set(-difference.x, -difference.y, -difference.z, -difference.w);
      this.rotations.set(name, {
        offset: quaternionLog(difference),
        velocity: (currentVelocity.angular.get(name) ?? new Vector3())
          .clone()
          .sub(targetVelocity.angular.get(name) ?? new Vector3()),
      });
    }
    this.hipsOffset
      .copy(current.hipsPosition ?? new Vector3())
      .sub(target.hipsPosition ?? new Vector3());
    this.hipsVelocity.copy(currentVelocity.hips).sub(targetVelocity.hips);
    for (const name of new Set([...current.expressions.keys(), ...target.expressions.keys()])) {
      this.expressions.set(name, {
        offset: (current.expressions.get(name) ?? 0) - (target.expressions.get(name) ?? 0),
        velocity:
          (currentVelocity.expressions.get(name) ?? 0) -
          (targetVelocity.expressions.get(name) ?? 0),
      });
    }
    this.lookAtOffset.set(
      (current.lookAt?.yawDegrees ?? 0) - (target.lookAt?.yawDegrees ?? 0),
      (current.lookAt?.pitchDegrees ?? 0) - (target.lookAt?.pitchDegrees ?? 0),
    );
    this.lookAtVelocity.copy(currentVelocity.lookAt).sub(targetVelocity.lookAt);
    this.elapsed = 0;
  }

  apply(
    target: SampledMotionPose,
    deltaSeconds: number,
    halfLives: InertialHalfLives = DEFAULT_INERTIAL_HALF_LIVES,
  ): SampledMotionPose {
    this.elapsed += Math.max(deltaSeconds, 0);
    const rotations = new Map<string, Quaternion>();
    for (const [name, targetRotation] of target.rotations) {
      const state = this.rotations.get(name);
      if (!state) {
        rotations.set(name, targetRotation.clone());
        continue;
      }
      const decay = decayAt(this.elapsed, armBone(name) ? halfLives.arms : halfLives.body);
      const residual = state.offset
        .clone()
        .addScaledVector(state.velocity, this.elapsed)
        .multiplyScalar(decay);
      rotations.set(name, targetRotation.clone().multiply(quaternionExp(residual)).normalize());
    }
    const rootDecay = decayAt(this.elapsed, halfLives.root);
    const hipsPosition = (target.hipsPosition ?? new Vector3())
      .clone()
      .add(this.hipsOffset.clone().addScaledVector(this.hipsVelocity, this.elapsed).multiplyScalar(rootDecay));
    const expressionDecay = decayAt(this.elapsed, halfLives.expression);
    const expressions = new Map<string, number>();
    for (const name of new Set([...target.expressions.keys(), ...this.expressions.keys()])) {
      const state = this.expressions.get(name);
      const targetValue = target.expressions.get(name) ?? 0;
      const residual = state ? (state.offset + state.velocity * this.elapsed) * expressionDecay : 0;
      expressions.set(name, Math.min(Math.max(targetValue + residual, 0), 1));
    }
    const lookDecay = decayAt(this.elapsed, halfLives.lookAt);
    const lookResidual = this.lookAtOffset
      .clone()
      .addScaledVector(this.lookAtVelocity, this.elapsed)
      .multiplyScalar(lookDecay);
    const lookAt = {
      yawDegrees: (target.lookAt?.yawDegrees ?? 0) + lookResidual.x,
      pitchDegrees: (target.lookAt?.pitchDegrees ?? 0) + lookResidual.y,
    };
    if (Math.max(rootDecay, expressionDecay, lookDecay, decayAt(this.elapsed, halfLives.body)) < 0.001) {
      this.reset();
    }
    return { rotations, hipsPosition, expressions, lookAt };
  }

  reset(): void {
    this.rotations.clear();
    this.expressions.clear();
    this.hipsOffset.set(0, 0, 0);
    this.hipsVelocity.set(0, 0, 0);
    this.lookAtOffset.set(0, 0);
    this.lookAtVelocity.set(0, 0);
    this.elapsed = Number.POSITIVE_INFINITY;
  }
}

export function velocityBetweenPoses(
  previous: SampledMotionPose | undefined,
  current: SampledMotionPose,
  deltaSeconds: number,
): MotionPoseVelocity {
  return previous ? estimateMotionPoseVelocity(previous, current, deltaSeconds) : zeroVelocity();
}

export function zeroVelocity(): MotionPoseVelocity {
  return {
    angular: new Map(),
    hips: new Vector3(),
    expressions: new Map(),
    lookAt: new Vector2(),
  };
}

function decayAt(elapsed: number, halfLife: number): number {
  return Math.exp((-Math.LN2 * elapsed) / Math.max(halfLife, 0.001));
}

function quaternionLog(quaternion: Quaternion): Vector3 {
  const sine = Math.hypot(quaternion.x, quaternion.y, quaternion.z);
  if (sine < 1e-8) return new Vector3();
  const angle = 2 * Math.atan2(sine, Math.min(Math.max(quaternion.w, -1), 1));
  return new Vector3(quaternion.x, quaternion.y, quaternion.z).multiplyScalar(angle / sine);
}

function quaternionExp(rotation: Vector3): Quaternion {
  const angle = rotation.length();
  if (angle < 1e-8) return new Quaternion();
  return new Quaternion().setFromAxisAngle(rotation.clone().multiplyScalar(1 / angle), angle);
}

function armBone(name: string): boolean {
  return /_(shoulder|upper_arm|lower_arm|hand|thumb|index|middle|ring|little)/.test(name);
}
