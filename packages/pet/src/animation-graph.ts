import { Quaternion, Vector3 } from "three";

interface InertialBoneState {
  offset: Quaternion;
  velocity: Vector3;
}

/**
 * Captures pose and angular-velocity error once at a program transition. Subsequent frames decay
 * that stored error onto the freshly sampled target, so contacts solved later cannot be undone.
 */
export class PoseTransitionInertializer {
  private readonly bones = new Map<string, InertialBoneState>();
  private elapsedSeconds = Number.POSITIVE_INFINITY;

  capture(
    current: ReadonlyMap<string, Quaternion>,
    target: ReadonlyMap<string, Quaternion>,
    angularVelocity: ReadonlyMap<string, Vector3> = new Map(),
  ): void {
    this.bones.clear();
    for (const [name, targetRotation] of target) {
      const currentRotation = current.get(name);
      if (!currentRotation) continue;
      const shortestCurrent = currentRotation.clone();
      if (targetRotation.dot(shortestCurrent) < 0) {
        shortestCurrent.set(
          -shortestCurrent.x,
          -shortestCurrent.y,
          -shortestCurrent.z,
          -shortestCurrent.w,
        );
      }
      this.bones.set(name, {
        offset: targetRotation.clone().invert().multiply(shortestCurrent).normalize(),
        velocity: angularVelocity.get(name)?.clone() ?? new Vector3(),
      });
    }
    this.elapsedSeconds = 0;
  }

  apply(
    targets: ReadonlyMap<string, Quaternion>,
    deltaSeconds: number,
    halfLifeSeconds = 0.075,
  ): Map<string, Quaternion> {
    this.elapsedSeconds += Math.max(deltaSeconds, 0);
    const decay = Math.exp((-Math.LN2 * this.elapsedSeconds) / Math.max(halfLifeSeconds, 0.001));
    const result = new Map<string, Quaternion>();
    for (const [name, target] of targets) {
      const state = this.bones.get(name);
      if (!state || decay < 0.001) {
        result.set(name, target.clone());
        continue;
      }
      const velocityOffset = state.velocity.clone().multiplyScalar(this.elapsedSeconds * decay);
      const offset = new Quaternion().slerp(state.offset, decay);
      if (velocityOffset.lengthSq() > 1e-10) offset.multiply(quaternionExp(velocityOffset));
      result.set(name, target.clone().multiply(offset).normalize());
    }
    if (decay < 0.001) this.bones.clear();
    return result;
  }

  reset(): void {
    this.bones.clear();
    this.elapsedSeconds = Number.POSITIVE_INFINITY;
  }
}

export function quaternionLog(quaternion: Quaternion): Vector3 {
  const normalized = quaternion.clone().normalize();
  const sine = Math.hypot(normalized.x, normalized.y, normalized.z);
  if (sine < 1e-8) return new Vector3();
  const angle = 2 * Math.atan2(sine, clamp(normalized.w, -1, 1));
  return new Vector3(normalized.x, normalized.y, normalized.z).multiplyScalar(angle / sine);
}

export function quaternionExp(rotationVector: Vector3): Quaternion {
  const angle = rotationVector.length();
  if (angle < 1e-8) return new Quaternion();
  return new Quaternion().setFromAxisAngle(rotationVector.clone().multiplyScalar(1 / angle), angle);
}

/** Estimates local-space angular velocity from two rendered poses for transition inertia. */
export function estimatePoseAngularVelocity(
  previous: ReadonlyMap<string, Quaternion> | undefined,
  current: ReadonlyMap<string, Quaternion>,
  deltaSeconds: number,
  maximumRadiansPerSecond = 20,
): Map<string, Vector3> {
  const result = new Map<string, Vector3>();
  if (!previous || !Number.isFinite(deltaSeconds) || deltaSeconds <= 1e-5) return result;
  for (const [name, currentRotation] of current) {
    const previousRotation = previous.get(name);
    if (!previousRotation) continue;
    const delta = previousRotation.clone().invert().multiply(currentRotation).normalize();
    if (delta.w < 0) delta.set(-delta.x, -delta.y, -delta.z, -delta.w);
    const velocity = quaternionLog(delta).multiplyScalar(1 / deltaSeconds);
    if (!velocity.toArray().every(Number.isFinite)) continue;
    velocity.clampLength(0, Math.max(maximumRadiansPerSecond, 0));
    result.set(name, velocity);
  }
  return result;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(Math.max(value, minimum), maximum);
}
