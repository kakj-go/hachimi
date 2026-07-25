import type { MotionCatalogEntry, MotionRuntimeAsset } from "@hachimi/contracts";
import type { VRM, VRMHumanBoneName } from "@pixiv/three-vrm";
import { createVRMAnimationClip, type VRMAnimation } from "@pixiv/three-vrm-animation";
import {
  Euler,
  MathUtils,
  Quaternion,
  QuaternionKeyframeTrack,
  Vector3,
  VectorKeyframeTrack,
  type AnimationClip,
  type Interpolant,
  type KeyframeTrack,
} from "three";
import type { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";

export interface SampledMotionPose {
  rotations: ReadonlyMap<string, Quaternion>;
  hipsPosition?: Vector3;
  expressions: ReadonlyMap<string, number>;
  lookAt?: { yawDegrees: number; pitchDegrees: number };
}

interface CompiledTrack {
  target: string;
  property: "quaternion" | "position" | "weight";
  kind: "bone" | "expression" | "look_at";
  interpolant: Interpolant;
  value: Float32Array;
  initialValue?: Float32Array;
}

interface CompiledSlot {
  motion: CompiledMotion;
  lastUsed: number;
}

const MAX_COMPILED_PER_AVATAR = 24;
const MAX_SOURCE_ANIMATIONS = 48;

export const VRM_HUMANOID_BONES: readonly VRMHumanBoneName[] = [
  "hips",
  "spine",
  "chest",
  "upperChest",
  "neck",
  "head",
  "leftEye",
  "rightEye",
  "jaw",
  "leftShoulder",
  "leftUpperArm",
  "leftLowerArm",
  "leftHand",
  "rightShoulder",
  "rightUpperArm",
  "rightLowerArm",
  "rightHand",
  "leftUpperLeg",
  "leftLowerLeg",
  "leftFoot",
  "leftToes",
  "rightUpperLeg",
  "rightLowerLeg",
  "rightFoot",
  "rightToes",
  "leftThumbMetacarpal",
  "leftThumbProximal",
  "leftThumbDistal",
  "leftIndexProximal",
  "leftIndexIntermediate",
  "leftIndexDistal",
  "leftMiddleProximal",
  "leftMiddleIntermediate",
  "leftMiddleDistal",
  "leftRingProximal",
  "leftRingIntermediate",
  "leftRingDistal",
  "leftLittleProximal",
  "leftLittleIntermediate",
  "leftLittleDistal",
  "rightThumbMetacarpal",
  "rightThumbProximal",
  "rightThumbDistal",
  "rightIndexProximal",
  "rightIndexIntermediate",
  "rightIndexDistal",
  "rightMiddleProximal",
  "rightMiddleIntermediate",
  "rightMiddleDistal",
  "rightRingProximal",
  "rightRingIntermediate",
  "rightRingDistal",
  "rightLittleProximal",
  "rightLittleIntermediate",
  "rightLittleDistal",
];

const RUNTIME_BONE_NAMES: Partial<Record<VRMHumanBoneName, string>> = {
  upperChest: "upper_chest",
  leftEye: "left_eye",
  rightEye: "right_eye",
  leftShoulder: "left_shoulder",
  leftUpperArm: "left_upper_arm",
  leftLowerArm: "left_lower_arm",
  leftHand: "left_hand",
  rightShoulder: "right_shoulder",
  rightUpperArm: "right_upper_arm",
  rightLowerArm: "right_lower_arm",
  rightHand: "right_hand",
  leftUpperLeg: "left_upper_leg",
  leftLowerLeg: "left_lower_leg",
  leftFoot: "left_foot",
  leftToes: "left_toes",
  rightUpperLeg: "right_upper_leg",
  rightLowerLeg: "right_lower_leg",
  rightFoot: "right_foot",
  rightToes: "right_toes",
  leftThumbMetacarpal: "left_thumb_proximal",
  leftThumbProximal: "left_thumb_intermediate",
  leftThumbDistal: "left_thumb_distal",
  leftIndexProximal: "left_index_proximal",
  leftIndexIntermediate: "left_index_intermediate",
  leftIndexDistal: "left_index_distal",
  leftMiddleProximal: "left_middle_proximal",
  leftMiddleIntermediate: "left_middle_intermediate",
  leftMiddleDistal: "left_middle_distal",
  leftRingProximal: "left_ring_proximal",
  leftRingIntermediate: "left_ring_intermediate",
  leftRingDistal: "left_ring_distal",
  leftLittleProximal: "left_little_proximal",
  leftLittleIntermediate: "left_little_intermediate",
  leftLittleDistal: "left_little_distal",
  rightThumbMetacarpal: "right_thumb_proximal",
  rightThumbProximal: "right_thumb_intermediate",
  rightThumbDistal: "right_thumb_distal",
  rightIndexProximal: "right_index_proximal",
  rightIndexIntermediate: "right_index_intermediate",
  rightIndexDistal: "right_index_distal",
  rightMiddleProximal: "right_middle_proximal",
  rightMiddleIntermediate: "right_middle_intermediate",
  rightMiddleDistal: "right_middle_distal",
  rightRingProximal: "right_ring_proximal",
  rightRingIntermediate: "right_ring_intermediate",
  rightRingDistal: "right_ring_distal",
  rightLittleProximal: "right_little_proximal",
  rightLittleIntermediate: "right_little_intermediate",
  rightLittleDistal: "right_little_distal",
};

export function runtimeBoneName(bone: VRMHumanBoneName): string {
  return RUNTIME_BONE_NAMES[bone] ?? bone;
}

class CompiledMotion {
  constructor(
    readonly entry: MotionCatalogEntry,
    private readonly tracks: readonly CompiledTrack[],
  ) {}

  sample(timeMs: number, mirror: boolean): SampledMotionPose {
    const durationMs = Math.max(this.entry.durationMs, 1);
    const positionMs =
      this.entry.playbackMode === "loop"
        ? ((timeMs % durationMs) + durationMs) % durationMs
        : Math.min(Math.max(timeMs, 0), durationMs - 0.001);
    const timeSeconds = positionMs / 1000;
    const rotations = new Map<string, Quaternion>();
    const expressions = new Map<string, number>();
    let hipsPosition: Vector3 | undefined;
    let lookAt: SampledMotionPose["lookAt"];
    for (const track of this.tracks) {
      track.interpolant.evaluate(timeSeconds);
      if (track.kind === "expression") {
        expressions.set(track.target, Math.min(Math.max(track.value[0] ?? 0, 0), 1));
      } else if (track.kind === "look_at") {
        const rotation = new Quaternion(
          track.value[0] ?? 0,
          track.value[1] ?? 0,
          track.value[2] ?? 0,
          track.value[3] ?? 1,
        ).normalize();
        const euler = new Euler().setFromQuaternion(rotation, "YXZ");
        lookAt = {
          yawDegrees: MathUtils.radToDeg(mirror ? -euler.y : euler.y),
          pitchDegrees: MathUtils.radToDeg(euler.x),
        };
      } else if (track.property === "quaternion") {
        const bone = mirror ? mirrorBone(track.target) : track.target;
        const rotation = new Quaternion(
          track.value[0] ?? 0,
          track.value[1] ?? 0,
          track.value[2] ?? 0,
          track.value[3] ?? 1,
        ).normalize();
        if (mirror) rotation.set(rotation.x, -rotation.y, -rotation.z, rotation.w);
        rotations.set(bone, rotation);
      } else if (track.target === "hips" && this.entry.rootMode !== "discard") {
        const initial = track.initialValue;
        const initialX = initial?.[0] ?? 0;
        const initialY = initial?.[1] ?? 0;
        const initialZ = initial?.[2] ?? 0;
        const relativeX = (track.value[0] ?? 0) - initialX;
        const relativeY = (track.value[1] ?? 0) - initialY;
        const relativeZ = (track.value[2] ?? 0) - initialZ;
        hipsPosition = new Vector3(
          mirror ? -relativeX : relativeX,
          relativeY,
          this.entry.rootMode === "stage" ? relativeZ : 0,
        );
      }
    }
    return {
      rotations,
      expressions,
      ...(hipsPosition ? { hipsPosition } : {}),
      ...(lookAt ? { lookAt } : {}),
    };
  }
}

export class MotionAssetLibrary {
  private readonly entries = new Map<string, MotionCatalogEntry>();
  private readonly sources = new Map<string, Promise<VRMAnimation>>();
  private readonly sourceUse = new Map<string, number>();
  private readonly compiled = new WeakMap<VRM, Map<string, CompiledSlot>>();
  private readonly pending = new WeakMap<VRM, Map<string, Promise<CompiledMotion>>>();
  private useCounter = 0;

  constructor(
    private readonly loader: GLTFLoader,
    private readonly resolveAsset: (id: string) => Promise<MotionRuntimeAsset | null>,
  ) {}

  setCatalog(entries: readonly MotionCatalogEntry[]): void {
    this.entries.clear();
    for (const entry of entries) this.entries.set(entry.id, entry);
  }

  entry(id: string): MotionCatalogEntry | undefined {
    return this.entries.get(id);
  }

  async preload(ids: readonly string[]): Promise<void> {
    await Promise.all(ids.filter((id) => this.entries.has(id)).map((id) => this.loadSource(id)));
  }

  async prepare(vrm: VRM, id: string): Promise<void> {
    await this.compiledMotion(vrm, id);
  }

  sample(vrm: VRM, id: string, timeMs: number, mirror = false): SampledMotionPose | undefined {
    const slot = this.compiled.get(vrm)?.get(id);
    if (!slot) return undefined;
    slot.lastUsed = ++this.useCounter;
    return slot.motion.sample(timeMs, mirror && slot.motion.entry.mirrorable);
  }

  clear(vrm: VRM): void {
    this.compiled.delete(vrm);
    this.pending.delete(vrm);
  }

  compiledCount(vrm: VRM): number {
    return this.compiled.get(vrm)?.size ?? 0;
  }

  sourceCount(): number {
    return this.sources.size;
  }

  private async compiledMotion(vrm: VRM, id: string): Promise<CompiledMotion> {
    let byId = this.compiled.get(vrm);
    if (!byId) {
      byId = new Map();
      this.compiled.set(vrm, byId);
    }
    const existing = byId.get(id);
    if (existing) {
      existing.lastUsed = ++this.useCounter;
      return existing.motion;
    }
    let pendingById = this.pending.get(vrm);
    if (!pendingById) {
      pendingById = new Map();
      this.pending.set(vrm, pendingById);
    }
    const existingPending = pendingById.get(id);
    if (existingPending) return existingPending;
    const pending = Promise.all([this.loadSource(id), this.resolveEntry(id)])
      .then(([animation, entry]) => {
        const clip = createVRMAnimationClip(animation, vrm);
        const motion = compileClip(clip, vrm, entry);
        byId.set(id, { motion, lastUsed: ++this.useCounter });
        evictCompiled(byId, id);
        return motion;
      })
      .finally(() => pendingById?.delete(id));
    pendingById.set(id, pending);
    return pending;
  }

  private async resolveEntry(id: string): Promise<MotionCatalogEntry> {
    const known = this.entries.get(id);
    if (known) return known;
    const asset = await this.resolveAsset(id);
    if (!asset) throw new Error(`Unknown motion asset: ${id}`);
    this.entries.set(id, asset.entry);
    return asset.entry;
  }

  private loadSource(id: string): Promise<VRMAnimation> {
    const existing = this.sources.get(id);
    if (existing) {
      this.sourceUse.set(id, ++this.useCounter);
      return existing;
    }
    const pending = this.resolveAsset(id)
      .then(async (asset) => {
        if (!asset) throw new Error(`Unknown motion asset: ${id}`);
        this.entries.set(id, asset.entry);
        const response = await fetch(asset.assetUrl, { cache: "force-cache" });
        if (!response.ok) throw new Error(`Unable to read VRMA ${id} (${response.status})`);
        const gltf = await this.loader.parseAsync(await response.arrayBuffer(), "");
        const animations = gltf.userData["vrmAnimations"] as VRMAnimation[] | undefined;
        if (animations?.length !== 1) throw new Error(`VRMA ${id} must contain one animation`);
        this.sourceUse.set(id, ++this.useCounter);
        evictSources(this.sources, this.sourceUse, id);
        return animations[0]!;
      })
      .catch((error: unknown) => {
        this.sources.delete(id);
        this.sourceUse.delete(id);
        throw error;
      });
    this.sources.set(id, pending);
    return pending;
  }
}

function compileClip(clip: AnimationClip, vrm: VRM, entry: MotionCatalogEntry): CompiledMotion {
  const targets = new Map<string, string>();
  for (const bone of VRM_HUMANOID_BONES) {
    const node = vrm.humanoid.getNormalizedBoneNode(bone);
    if (!node) continue;
    const runtimeName = runtimeBoneName(bone);
    targets.set(node.name, runtimeName);
    targets.set(node.uuid, runtimeName);
  }
  const expressionTargets = new Map<string, string>();
  for (const expression of vrm.expressionManager?.expressions ?? []) {
    expressionTargets.set(expression.name, expression.expressionName);
    expressionTargets.set(expression.uuid, expression.expressionName);
  }
  const tracks: CompiledTrack[] = [];
  for (const track of clip.tracks) {
    const target = trackTarget(track.name);
    const bone = targets.get(target);
    const expression = expressionTargets.get(target);
    const property = track.name.slice(track.name.lastIndexOf(".") + 1);
    const lookAt = target.includes("VRMLookAtQuaternionProxy") && property === "quaternion";
    if (!bone && !expression && !lookAt) {
      continue;
    }
    if (
      !(
        track instanceof QuaternionKeyframeTrack ||
        track instanceof VectorKeyframeTrack ||
        (expression && property === "weight")
      )
    ) {
      continue;
    }
    const value = new Float32Array(track.getValueSize());
    const interpolant = createInterpolant(track, value);
    let initialValue: Float32Array | undefined;
    if (bone === "hips" && property === "position") {
      initialValue = new Float32Array(track.getValueSize());
      createInterpolant(track, initialValue).evaluate(0);
    }
    tracks.push({
      target: bone ?? expression ?? "look_at",
      property: property as CompiledTrack["property"],
      kind: bone ? "bone" : expression ? "expression" : "look_at",
      interpolant,
      value,
      ...(initialValue ? { initialValue } : {}),
    });
  }
  if (tracks.length === 0) throw new Error(`VRMA ${entry.id} has no usable humanoid tracks`);
  return new CompiledMotion(entry, tracks);
}

function createInterpolant(track: KeyframeTrack, value: Float32Array): Interpolant {
  return (
    track as KeyframeTrack & {
      createInterpolant: (result: Float32Array) => Interpolant;
    }
  ).createInterpolant(value);
}

function trackTarget(trackName: string): string {
  const bracket = /\.bones\[([^\]]+)]/.exec(trackName)?.[1];
  if (bracket) return bracket;
  const separator = trackName.lastIndexOf(".");
  return separator < 0 ? trackName : trackName.slice(0, separator);
}

function mirrorBone(bone: string): string {
  if (bone.startsWith("left_")) return `right_${bone.slice(5)}`;
  if (bone.startsWith("right_")) return `left_${bone.slice(6)}`;
  return bone;
}

function evictCompiled(slots: Map<string, CompiledSlot>, keep: string): void {
  pruneLeastRecentlyUsed(slots, MAX_COMPILED_PER_AVATAR, keep, (slot) => slot.lastUsed);
}

function evictSources(
  sources: Map<string, Promise<VRMAnimation>>,
  uses: Map<string, number>,
  keep: string,
): void {
  while (sources.size > MAX_SOURCE_ANIMATIONS) {
    const victim = [...sources.keys()]
      .filter((id) => id !== keep)
      .sort((left, right) => (uses.get(left) ?? 0) - (uses.get(right) ?? 0))[0];
    if (!victim) return;
    sources.delete(victim);
    uses.delete(victim);
  }
}

export const MOTION_COMPILED_CACHE_LIMIT = MAX_COMPILED_PER_AVATAR;

export function pruneLeastRecentlyUsed<T>(
  slots: Map<string, T>,
  limit: number,
  keep: string,
  lastUsed: (value: T) => number,
): void {
  while (slots.size > Math.max(Math.floor(limit), 0)) {
    const victim = [...slots]
      .filter(([id]) => id !== keep)
      .sort((left, right) => lastUsed(left[1]) - lastUsed(right[1]))[0]?.[0];
    if (!victim) return;
    slots.delete(victim);
  }
}
