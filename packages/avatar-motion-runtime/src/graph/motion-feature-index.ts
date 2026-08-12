import { MathUtils, Quaternion, Vector2, Vector3 } from "three";
import { measureMotionPoseSeam } from "../motion-composer";
import type { SampledMotionPose } from "../motion-asset-library";
import {
  MOTION_FEATURE_SAMPLE_HZ,
  MOTION_FEATURE_VERSION,
  type FootContactState,
  type MotionFeatureFrame,
  type MotionFeatureIndex,
  type MotionPoseVelocity,
} from "./types";

export interface MotionFeatureBuildOptions {
  motionId: string;
  contentHash: string;
  skeletonSignature: string;
  durationMs: number;
  loop: boolean;
  entryWindows?: readonly { startMs: number; endMs: number }[];
  exitWindows?: readonly { startMs: number; endMs: number }[];
  contactAt?: (timeMs: number, pose: SampledMotionPose) => FootContactState;
  sample: (timeMs: number) => SampledMotionPose;
}

export function buildMotionFeatureIndex(options: MotionFeatureBuildOptions): MotionFeatureIndex {
  const durationMs = Math.max(options.durationMs, 1);
  const frameMs = 1_000 / MOTION_FEATURE_SAMPLE_HZ;
  const frameCount = Math.max(2, Math.ceil(durationMs / frameMs) + 1);
  const poses = Array.from({ length: frameCount }, (_, index) =>
    clonePose(options.sample(Math.min(index * frameMs, durationMs - 0.001))),
  );
  const frames = poses.map((pose, index): MotionFeatureFrame => {
    const timeMs = Math.min(index * frameMs, durationMs - 0.001);
    const previous = poses[Math.max(index - 1, 0)]!;
    const next = poses[Math.min(index + 1, poses.length - 1)]!;
    const elapsedSeconds = Math.max(
      (Math.min(index + 1, poses.length - 1) - Math.max(index - 1, 0)) * frameMs * 0.001,
      1 / MOTION_FEATURE_SAMPLE_HZ,
    );
    const velocity = estimateMotionPoseVelocity(previous, next, elapsedSeconds);
    return {
      timeMs,
      loopPhase: timeMs / durationMs,
      pose,
      velocity,
      footContact: options.contactAt?.(timeMs, pose) ?? inferFootContact(velocity),
      safeEntry: inWindows(timeMs, options.entryWindows, timeMs <= 120),
      safeExit: inWindows(timeMs, options.exitWindows, true),
    };
  });
  const seam = measureMotionPoseSeam(poses[0]!, poses.at(-1)!);
  return {
    cacheKey: `${options.skeletonSignature}:${options.contentHash}:v${MOTION_FEATURE_VERSION}`,
    motionId: options.motionId,
    durationMs,
    sampleHz: MOTION_FEATURE_SAMPLE_HZ,
    frames,
    loopSeamDegrees: options.loop ? seam.maxRotationDegrees : 0,
    loopSeamRootDistance: options.loop ? seam.rootDistance : 0,
  };
}

export function inferFootContact(velocity: MotionPoseVelocity): FootContactState {
  const left = limbAngularSpeed(velocity, "left");
  const right = limbAngularSpeed(velocity, "right");
  if (left === undefined || right === undefined) return "unknown";
  if (left <= 0.45 && right <= 0.45) return "both";
  if (left <= 1.2 && left + 0.35 < right) return "left";
  if (right <= 1.2 && right + 0.35 < left) return "right";
  if (left > 3 && right > 3) return "air";
  return "unknown";
}

function limbAngularSpeed(
  velocity: MotionPoseVelocity,
  side: "left" | "right",
): number | undefined {
  const values = [`${side}_foot`, `${side}_lower_leg`, `${side}_upper_leg`]
    .map((name) => velocity.angular.get(name)?.length())
    .filter((value): value is number => value !== undefined);
  if (values.length === 0) return undefined;
  return values.reduce((sum, value) => sum + value, 0) / values.length;
}

export function estimateMotionPoseVelocity(
  previous: SampledMotionPose,
  current: SampledMotionPose,
  deltaSeconds: number,
): MotionPoseVelocity {
  const delta = Math.max(deltaSeconds, 1 / 240);
  const angular = new Map<string, Vector3>();
  for (const [name, rotation] of current.rotations) {
    const before = previous.rotations.get(name);
    if (!before) continue;
    const difference = before.clone().invert().multiply(rotation).normalize();
    if (difference.w < 0)
      difference.set(-difference.x, -difference.y, -difference.z, -difference.w);
    angular.set(
      name,
      quaternionLog(difference)
        .multiplyScalar(1 / delta)
        .clampLength(0, 20),
    );
  }
  const expressions = new Map<string, number>();
  for (const [name, value] of current.expressions) {
    expressions.set(name, (value - (previous.expressions.get(name) ?? 0)) / delta);
  }
  return {
    angular,
    hips: (current.hipsPosition ?? new Vector3())
      .clone()
      .sub(previous.hipsPosition ?? new Vector3())
      .multiplyScalar(1 / delta),
    expressions,
    lookAt: new Vector2(
      ((current.lookAt?.yawDegrees ?? 0) - (previous.lookAt?.yawDegrees ?? 0)) / delta,
      ((current.lookAt?.pitchDegrees ?? 0) - (previous.lookAt?.pitchDegrees ?? 0)) / delta,
    ),
  };
}

export function nearestFeatureFrame(index: MotionFeatureIndex, timeMs: number): MotionFeatureFrame {
  const frame = Math.round((Math.max(timeMs, 0) / 1_000) * index.sampleHz);
  return index.frames[Math.min(frame, index.frames.length - 1)]!;
}

function inWindows(
  timeMs: number,
  windows: readonly { startMs: number; endMs: number }[] | undefined,
  fallback: boolean,
): boolean {
  if (!windows || windows.length === 0) return fallback;
  return windows.some(({ startMs, endMs }) => timeMs >= startMs && timeMs <= endMs);
}

function quaternionLog(quaternion: Quaternion): Vector3 {
  const sine = Math.hypot(quaternion.x, quaternion.y, quaternion.z);
  if (sine < 1e-8) return new Vector3();
  const angle = 2 * Math.atan2(sine, MathUtils.clamp(quaternion.w, -1, 1));
  return new Vector3(quaternion.x, quaternion.y, quaternion.z).multiplyScalar(angle / sine);
}

function clonePose(pose: SampledMotionPose): SampledMotionPose {
  return {
    rotations: new Map([...pose.rotations].map(([name, value]) => [name, value.clone()])),
    expressions: new Map(pose.expressions),
    ...(pose.hipsPosition ? { hipsPosition: pose.hipsPosition.clone() } : {}),
    ...(pose.lookAt ? { lookAt: { ...pose.lookAt } } : {}),
  };
}
