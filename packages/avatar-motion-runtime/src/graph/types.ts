import type {
  BehaviorChannel,
  MotionCatalogEntry,
  MotionIntentRequest,
  MotionSlot,
  MotionTransitionProfile,
} from "@hachimi/contracts";
import type { Quaternion, Vector2, Vector3 } from "three";
import type { MotionChannelWeights } from "../motion-composer";
import type { SampledMotionPose } from "../motion-asset-library";

export const MOTION_FEATURE_VERSION = 1;
export const MOTION_FEATURE_SAMPLE_HZ = 60;

export type FootContactState = "left" | "right" | "both" | "air" | "unknown";

export interface MotionPoseVelocity {
  angular: ReadonlyMap<string, Vector3>;
  hips: Vector3;
  expressions: ReadonlyMap<string, number>;
  lookAt: Vector2;
}

export interface MotionFeatureFrame {
  timeMs: number;
  loopPhase: number;
  pose: SampledMotionPose;
  velocity: MotionPoseVelocity;
  footContact: FootContactState;
  safeEntry: boolean;
  safeExit: boolean;
}

export interface MotionFeatureIndex {
  cacheKey: string;
  motionId: string;
  durationMs: number;
  sampleHz: number;
  frames: readonly MotionFeatureFrame[];
  loopSeamDegrees: number;
  loopSeamRootDistance: number;
}

export interface TransitionPlan {
  targetTimeMs: number;
  durationMs: number;
  forced: boolean;
  cost: number;
  costs: {
    pose: number;
    velocity: number;
    footContact: number;
    root: number;
  };
}

export interface AnimationGraphLayer {
  id: string;
  motionId: string;
  slot: MotionSlot;
  priority: number;
  pose: SampledMotionPose;
  weight: number;
  channels: MotionChannelWeights;
  inertialHalfLives: {
    root: number;
    body: number;
    arms: number;
    lookAt: number;
    expression: number;
  };
}

export interface AnimationGraphNode {
  intent: MotionIntentRequest;
  entry: MotionCatalogEntry;
  profile: MotionTransitionProfile;
  startedAt: number;
  transitionStartedAt: number;
  activateAt: number;
  playbackTimeMs: number;
  lastUpdatedAt: number;
  targetStartTimeMs: number;
  transitionDurationMs: number;
  forced: boolean;
}

export interface AnimationGraphSubmitOptions {
  maximumWaitMs?: number;
  transitionElapsedMs?: number;
}

export interface MotionGraphCatalog {
  entries: readonly MotionCatalogEntry[];
  transitionProfiles: readonly MotionTransitionProfile[];
}

export function channelWeightsFromIntent(
  channels: readonly { channel: BehaviorChannel; weight: number }[],
  fallback: readonly BehaviorChannel[],
): MotionChannelWeights {
  const values: MotionChannelWeights = {};
  for (const { channel, weight } of channels) values[channel] = clamp01(weight / 1_000);
  if (channels.length === 0) for (const channel of fallback) values[channel] = 1;
  return values;
}

function clamp01(value: number): number {
  return Math.min(Math.max(Number.isFinite(value) ? value : 0, 0), 1);
}

export type PoseSampler = (motionId: string, timeMs: number, mirror: boolean) => SampledMotionPose;
