import type {
  AvatarAdaptationProfile,
  BehaviorChannel,
  InteractionRegion,
} from "@hachimi/contracts";
import type { FootContactState, FootSoleOffsets } from "@hachimi/avatar-motion-runtime";
import { Quaternion, Vector3, type Object3D } from "three";

export interface PresentationRootBaseline {
  position: Vector3;
  quaternion: Quaternion;
  scale: Vector3;
}

export interface AvatarPointerHit {
  region: InteractionRegion;
  direction: -1 | 1;
  headTopContact: boolean;
}

export function capturePresentationRootBaseline(root: Object3D): PresentationRootBaseline {
  return {
    position: root.position.clone(),
    quaternion: root.quaternion.clone(),
    scale: root.scale.clone(),
  };
}

export function restorePresentationRoot(
  root: Object3D,
  baseline: PresentationRootBaseline,
): void {
  root.position.copy(baseline.position);
  root.quaternion.copy(baseline.quaternion);
  root.scale.copy(baseline.scale);
}

function scaleRatioWithinBounds(value: number, base: number): boolean {
  const ratio = Math.abs(value) / Math.max(Math.abs(base), 0.000_001);
  return ratio >= 0.75 && ratio <= 1.25;
}

export function stabilizePresentationRoot(
  root: Object3D,
  baseline: PresentationRootBaseline,
  avatarHeight: number,
): boolean {
  const maximumOffset = Math.max(Math.abs(avatarHeight), 0.001) * 0.35;
  const offsetX = root.position.x - baseline.position.x;
  const offsetY = root.position.y - baseline.position.y;
  const offsetZ = root.position.z - baseline.position.z;
  const offsetSquared = offsetX * offsetX + offsetY * offsetY + offsetZ * offsetZ;
  const finitePosition =
    Number.isFinite(root.position.x) &&
    Number.isFinite(root.position.y) &&
    Number.isFinite(root.position.z);
  const finiteQuaternion =
    Number.isFinite(root.quaternion.x) &&
    Number.isFinite(root.quaternion.y) &&
    Number.isFinite(root.quaternion.z) &&
    Number.isFinite(root.quaternion.w);
  const finiteScale =
    Number.isFinite(root.scale.x) && Number.isFinite(root.scale.y) && Number.isFinite(root.scale.z);
  const scaleWithinBounds =
    scaleRatioWithinBounds(root.scale.x, baseline.scale.x) &&
    scaleRatioWithinBounds(root.scale.y, baseline.scale.y) &&
    scaleRatioWithinBounds(root.scale.z, baseline.scale.z);
  const valid =
    finitePosition &&
    finiteQuaternion &&
    finiteScale &&
    offsetSquared <= maximumOffset * maximumOffset &&
    root.quaternion.lengthSq() >= 0.5 &&
    root.quaternion.lengthSq() <= 1.5 &&
    scaleWithinBounds;
  if (!valid) restorePresentationRoot(root, baseline);
  return valid;
}

export function finiteNumber(value: number | null | undefined, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

export function profileSoleOffsets(profile: AvatarAdaptationProfile): FootSoleOffsets {
  const offset = (id: string): Vector3 | undefined => {
    const contact = profile.contacts?.find((value) => value.id === id);
    if (!contact) return undefined;
    return new Vector3(
      finiteNumber(contact.localPosition[0], 0),
      finiteNumber(contact.localPosition[1], 0),
      finiteNumber(contact.localPosition[2], 0),
    );
  };
  return { left: offset("left_sole"), right: offset("right_sole") };
}

export function isLeftRegion(region: InteractionRegion, direction: -1 | 1): boolean {
  return region.startsWith("left_") || (!region.startsWith("right_") && direction < 0);
}

export function speechChannelWeights(): Array<{ channel: BehaviorChannel; weight: number }> {
  return [
    { channel: "upper_body", weight: 650 },
    { channel: "left_arm", weight: 1_000 },
    { channel: "right_arm", weight: 1_000 },
    { channel: "fingers", weight: 1_000 },
    { channel: "head", weight: 280 },
  ];
}

export function locomotionChannelWeights(): Array<{ channel: BehaviorChannel; weight: number }> {
  return [
    { channel: "root", weight: 1_000 },
    { channel: "locomotion", weight: 1_000 },
    { channel: "lower_body", weight: 1_000 },
    { channel: "upper_body", weight: 1_000 },
    { channel: "left_arm", weight: 1_000 },
    { channel: "right_arm", weight: 1_000 },
    { channel: "fingers", weight: 1_000 },
    { channel: "head", weight: 1_000 },
  ];
}

export function contactState(left: string, right: string): FootContactState {
  const leftGrounded = left !== "air";
  const rightGrounded = right !== "air";
  if (leftGrounded && rightGrounded) return "both";
  if (leftGrounded) return "left";
  if (rightGrounded) return "right";
  return "air";
}
