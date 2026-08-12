import { MathUtils, Quaternion, Vector3 } from "three";
import type { SampledMotionPose } from "./motion-asset-library";

export interface PoseStepMetrics {
  boneDegrees: number;
  rootHeightRatio: number;
  lookAtDegrees: number;
}

export function cloneSampledPose(pose: SampledMotionPose): SampledMotionPose {
  return {
    rotations: new Map([...pose.rotations].map(([name, rotation]) => [name, rotation.clone()])),
    expressions: new Map(pose.expressions),
    ...(pose.hipsPosition ? { hipsPosition: pose.hipsPosition.clone() } : {}),
    ...(pose.lookAt ? { lookAt: { ...pose.lookAt } } : {}),
  };
}

/** Applies Runtime V5's final visible-frame continuity bounds in place. */
export function limitPoseStep(
  pose: SampledMotionPose,
  previous: SampledMotionPose | undefined,
  avatarHeight: number,
): PoseStepMetrics {
  sanitizePose(pose, previous);
  if (!previous) return { boneDegrees: 0, rootHeightRatio: 0, lookAtDegrees: 0 };
  const maximumBoneStep = MathUtils.degToRad(12);
  let boneStep = 0;
  for (const [name, rotation] of pose.rotations) {
    const before = previous.rotations.get(name);
    if (!before) continue;
    const angle = before.angleTo(rotation);
    if (angle > maximumBoneStep) {
      const target = rotation.clone();
      rotation.copy(before).slerp(target, maximumBoneStep / angle);
    }
    boneStep = Math.max(boneStep, before.angleTo(rotation));
  }
  let rootStep = 0;
  if (pose.hipsPosition && previous.hipsPosition) {
    const offset = pose.hipsPosition.clone().sub(previous.hipsPosition);
    const maximumRootStep = Math.max(avatarHeight, 0.001) * 0.005;
    if (offset.length() > maximumRootStep) {
      pose.hipsPosition.copy(previous.hipsPosition).add(offset.setLength(maximumRootStep));
    }
    rootStep = pose.hipsPosition.distanceTo(previous.hipsPosition);
  }
  let lookAtStep = 0;
  if (pose.lookAt && previous.lookAt) {
    pose.lookAt.yawDegrees = MathUtils.clamp(
      pose.lookAt.yawDegrees,
      previous.lookAt.yawDegrees - 4,
      previous.lookAt.yawDegrees + 4,
    );
    pose.lookAt.pitchDegrees = MathUtils.clamp(
      pose.lookAt.pitchDegrees,
      previous.lookAt.pitchDegrees - 4,
      previous.lookAt.pitchDegrees + 4,
    );
    lookAtStep = Math.max(
      Math.abs(pose.lookAt.yawDegrees - previous.lookAt.yawDegrees),
      Math.abs(pose.lookAt.pitchDegrees - previous.lookAt.pitchDegrees),
    );
  }
  return {
    boneDegrees: MathUtils.radToDeg(boneStep),
    rootHeightRatio: rootStep / Math.max(avatarHeight, 0.001),
    lookAtDegrees: lookAtStep,
  };
}

function sanitizePose(pose: SampledMotionPose, previous: SampledMotionPose | undefined): void {
  for (const [name, rotation] of pose.rotations) {
    if (finiteQuaternion(rotation)) {
      rotation.normalize();
      continue;
    }
    const fallback = previous?.rotations.get(name);
    rotation.copy(fallback && finiteQuaternion(fallback) ? fallback : rotation.identity());
  }
  if (pose.hipsPosition && !finiteVector(pose.hipsPosition)) {
    const fallback = previous?.hipsPosition;
    pose.hipsPosition.copy(fallback && finiteVector(fallback) ? fallback : new Vector3());
  }
  const expressions = pose.expressions as Map<string, number>;
  for (const [name, value] of expressions) {
    if (Number.isFinite(value)) continue;
    const fallback = previous?.expressions.get(name);
    expressions.set(name, Number.isFinite(fallback) ? fallback! : 0);
  }
  if (pose.lookAt) {
    const previousLookAt = previous?.lookAt;
    if (!Number.isFinite(pose.lookAt.yawDegrees)) {
      pose.lookAt.yawDegrees = Number.isFinite(previousLookAt?.yawDegrees)
        ? previousLookAt!.yawDegrees
        : 0;
    }
    if (!Number.isFinite(pose.lookAt.pitchDegrees)) {
      pose.lookAt.pitchDegrees = Number.isFinite(previousLookAt?.pitchDegrees)
        ? previousLookAt!.pitchDegrees
        : 0;
    }
  }
}

function finiteQuaternion(value: Quaternion): boolean {
  return (
    Number.isFinite(value.x) &&
    Number.isFinite(value.y) &&
    Number.isFinite(value.z) &&
    Number.isFinite(value.w) &&
    value.lengthSq() > 1e-12
  );
}

function finiteVector(value: Vector3): boolean {
  return Number.isFinite(value.x) && Number.isFinite(value.y) && Number.isFinite(value.z);
}
