import type { BehaviorChannel } from "@hachimi/contracts";
import { MathUtils, Vector3, type Quaternion } from "three";
import type { SampledMotionPose } from "./motion-asset-library";

export type MotionChannelWeights = Partial<Record<BehaviorChannel, number>>;

export interface MotionCompositionLayer {
  id: string;
  pose: SampledMotionPose;
  priority: number;
  weight: number;
  channels: MotionChannelWeights;
}

export interface ComposedMotionPose extends SampledMotionPose {
  contributingLayerIds: readonly string[];
}

export interface MotionPoseSeam {
  maxRotationDegrees: number;
  rootDistance: number;
}

const MOUTH_BONES = new Set(["jaw"]);
const HEAD_BONES = new Set(["neck", "head", "left_eye", "right_eye"]);
const LOWER_BODY_BONES = new Set([
  "hips",
  "left_upper_leg",
  "left_lower_leg",
  "left_foot",
  "left_toes",
  "right_upper_leg",
  "right_lower_leg",
  "right_foot",
  "right_toes",
]);

export function channelForBone(bone: string): BehaviorChannel {
  if (MOUTH_BONES.has(bone)) return "mouth";
  if (HEAD_BONES.has(bone)) return "head";
  if (isFingerBone(bone)) return "fingers";
  if (bone.startsWith("left_") && /(shoulder|arm|hand)/.test(bone)) return "left_arm";
  if (bone.startsWith("right_") && /(shoulder|arm|hand)/.test(bone)) return "right_arm";
  if (LOWER_BODY_BONES.has(bone)) return "lower_body";
  return "upper_body";
}

export function channelWeight(weights: MotionChannelWeights, channel: BehaviorChannel): number {
  const exact = weights[channel];
  if (exact !== undefined) return clamp01(exact);
  if (channel === "lower_body") {
    return clamp01(weights.lower_body ?? weights.locomotion ?? weights.full_body ?? 0);
  }
  if (
    channel === "upper_body" ||
    channel === "left_arm" ||
    channel === "right_arm" ||
    channel === "fingers" ||
    channel === "head"
  ) {
    return clamp01(weights[channel] ?? weights.upper_body ?? weights.full_body ?? 0);
  }
  return clamp01(weights.full_body ?? 0);
}

/**
 * Composes independent semantic channels in priority order. A high-priority arm layer can replace
 * an arm without disturbing the lower body, fingers, face, gaze, mouth, or root channel.
 */
export function composeMotionLayers(
  restRotations: ReadonlyMap<string, Quaternion>,
  layers: readonly MotionCompositionLayer[],
): ComposedMotionPose {
  const ordered = [...layers].sort(
    (left, right) => left.priority - right.priority || left.id.localeCompare(right.id),
  );
  const rotations = new Map(
    [...restRotations].map(([name, rotation]) => [name, rotation.clone()] as const),
  );
  const expressions = new Map<string, number>();
  const hipsPosition = new Vector3();
  let hasRoot = false;
  let lookAt: ComposedMotionPose["lookAt"];
  const contributingLayerIds: string[] = [];

  for (const layer of ordered) {
    const layerWeight = clamp01(layer.weight);
    if (layerWeight <= 0) continue;
    let contributed = false;
    for (const [bone, target] of layer.pose.rotations) {
      const current = rotations.get(bone);
      if (!current) continue;
      const weight = layerWeight * channelWeight(layer.channels, channelForBone(bone));
      if (weight <= 0) continue;
      current.slerp(target, weight).normalize();
      contributed = true;
    }
    if (layer.pose.hipsPosition) {
      const weight = layerWeight * channelWeight(layer.channels, "root");
      if (weight > 0) {
        hipsPosition.lerp(layer.pose.hipsPosition, weight);
        hasRoot = true;
        contributed = true;
      }
    }
    for (const [expression, target] of layer.pose.expressions) {
      const weight = layerWeight * channelWeight(layer.channels, "face");
      if (weight <= 0) continue;
      expressions.set(
        expression,
        MathUtils.lerp(expressions.get(expression) ?? 0, clamp01(target), weight),
      );
      contributed = true;
    }
    if (layer.pose.lookAt) {
      const weight = layerWeight * channelWeight(layer.channels, "gaze");
      if (weight > 0) {
        lookAt = {
          yawDegrees: MathUtils.lerp(lookAt?.yawDegrees ?? 0, layer.pose.lookAt.yawDegrees, weight),
          pitchDegrees: MathUtils.lerp(
            lookAt?.pitchDegrees ?? 0,
            layer.pose.lookAt.pitchDegrees,
            weight,
          ),
        };
        contributed = true;
      }
    }
    if (contributed) contributingLayerIds.push(layer.id);
  }

  return {
    rotations,
    expressions,
    ...(hasRoot ? { hipsPosition } : {}),
    ...(lookAt ? { lookAt } : {}),
    contributingLayerIds,
  };
}

export function channelsForFullBody(includeFace = true): MotionChannelWeights {
  return {
    root: 1,
    locomotion: 1,
    full_body: 1,
    lower_body: 1,
    upper_body: 1,
    left_arm: 1,
    right_arm: 1,
    fingers: 1,
    head: 1,
    gaze: includeFace ? 1 : 0,
    face: includeFace ? 1 : 0,
    mouth: includeFace ? 1 : 0,
  };
}

export function channelsForSpeechBody(weight = 1): MotionChannelWeights {
  return {
    root: 0,
    locomotion: 0,
    lower_body: 0,
    upper_body: weight * 0.65,
    left_arm: weight,
    right_arm: weight,
    fingers: weight,
    head: weight * 0.28,
    gaze: 0,
    face: 0,
    mouth: 0,
  };
}

export function measureMotionPoseSeam(
  start: SampledMotionPose,
  end: SampledMotionPose,
): MotionPoseSeam {
  let maxRotationDegrees = 0;
  for (const [bone, startRotation] of start.rotations) {
    const endRotation = end.rotations.get(bone);
    if (!endRotation) continue;
    maxRotationDegrees = Math.max(
      maxRotationDegrees,
      MathUtils.radToDeg(startRotation.angleTo(endRotation)),
    );
  }
  return {
    maxRotationDegrees,
    rootDistance: start.hipsPosition?.distanceTo(end.hipsPosition ?? new Vector3()) ?? 0,
  };
}

function isFingerBone(bone: string): boolean {
  return /_(thumb|index|middle|ring|little)_/.test(bone);
}

function clamp01(value: number): number {
  return Math.min(Math.max(Number.isFinite(value) ? value : 0, 0), 1);
}
