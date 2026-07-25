import { Quaternion, Vector3, type Object3D } from "three";
import type { AvatarAdaptationProfile } from "@hachimi/contracts";

export interface EndEffectorGoal {
  bone: "left_hand" | "right_hand" | "left_foot" | "right_foot";
  normalizedOffset: readonly [number, number, number];
  pole: "left_elbow" | "right_elbow" | "left_knee" | "right_knee";
  weight: number;
  contact: boolean;
}

export interface LegChain {
  upper: Object3D;
  lower: Object3D;
  foot: Object3D;
  toes?: Object3D;
  pole: Vector3;
}

export interface CapsuleBinding {
  id: string;
  bone: Object3D;
  radius: number;
  halfHeight: number;
  movable: boolean;
}

export type FootContactPhase = "air" | "heel" | "flat" | "toe";

export interface FootSolveDiagnostics {
  maxDrift: number;
}

export interface AvatarConstraintRig {
  bones: ReadonlyMap<string, Object3D>;
  restRotations: ReadonlyMap<string, Quaternion>;
  profile: AvatarAdaptationProfile;
  height: number;
  groundY?: number;
}

export interface AvatarConstraintInput {
  leftFootPhase: FootContactPhase;
  rightFootPhase: FootContactPhase;
  footStrength: number;
  endEffectors: readonly EndEffectorGoal[];
  centerOfMass: readonly [number, number, number];
}

export interface AvatarConstraintDiagnostics {
  leftFootPhase: FootContactPhase;
  rightFootPhase: FootContactPhase;
  maxFootDriftNormalized: number;
  groundPenetrationNormalized: number;
  maxJointCorrectionDegrees: number;
  collisionCount: number;
  centerOfMassOutsideSupport: boolean;
}

interface FootLock {
  target: Vector3;
  rotation: Quaternion;
  contact: boolean;
  phase: FootContactPhase;
}

/** Two-bone foot planting for the normalized VRM humanoid. */
export class ConstraintSolver {
  private readonly left: FootLock = {
    target: new Vector3(),
    rotation: new Quaternion(),
    contact: false,
    phase: "air",
  };
  private readonly right: FootLock = {
    target: new Vector3(),
    rotation: new Quaternion(),
    contact: false,
    phase: "air",
  };

  updateFootPlant(
    side: "left" | "right",
    chain: LegChain | undefined,
    contact: boolean,
    strength: number,
  ): void {
    if (!chain) return;
    const lock = side === "left" ? this.left : this.right;
    if (contact && !lock.contact) chain.foot.getWorldPosition(lock.target);
    lock.contact = contact;
    if (contact) solveTwoBoneIk(chain, lock.target, strength);
  }

  updateLowerBody(
    hips: Object3D | undefined,
    leftChain: LegChain | undefined,
    rightChain: LegChain | undefined,
    leftContact: boolean,
    rightContact: boolean,
    strength: number,
  ): FootSolveDiagnostics {
    return this.updateLowerBodyPhased(
      hips,
      leftChain,
      rightChain,
      leftContact ? "flat" : "air",
      rightContact ? "flat" : "air",
      strength,
    );
  }

  updateLowerBodyPhased(
    hips: Object3D | undefined,
    leftChain: LegChain | undefined,
    rightChain: LegChain | undefined,
    leftPhase: FootContactPhase,
    rightPhase: FootContactPhase,
    strength: number,
  ): FootSolveDiagnostics {
    const leftContact = leftPhase !== "air";
    const rightContact = rightPhase !== "air";
    this.captureContact(this.left, leftChain, leftPhase);
    this.captureContact(this.right, rightChain, rightPhase);
    if (hips) {
      const correction = new Vector3();
      let contacts = 0;
      if (leftContact && leftChain) {
        correction.add(
          this.left.target.clone().sub(leftChain.foot.getWorldPosition(new Vector3())),
        );
        contacts += 1;
      }
      if (rightContact && rightChain) {
        correction.add(
          this.right.target.clone().sub(rightChain.foot.getWorldPosition(new Vector3())),
        );
        contacts += 1;
      }
      if (contacts > 0) {
        correction.multiplyScalar((Math.min(Math.max(strength, 0), 1) * 0.45) / contacts);
        // Hip compensation is intentionally conservative; the IK solves the remainder so root
        // motion cannot drift outside the Pet stage.
        const maximum =
          Math.max(
            leftChain?.upper
              .getWorldPosition(new Vector3())
              .distanceTo(leftChain.lower.getWorldPosition(new Vector3())) ?? 0,
            rightChain?.upper
              .getWorldPosition(new Vector3())
              .distanceTo(rightChain.lower.getWorldPosition(new Vector3())) ?? 0,
            0.01,
          ) * 0.08;
        correction.clampLength(0, maximum);
        const supportCenter = new Vector3();
        if (leftContact) supportCenter.add(this.left.target);
        if (rightContact) supportCenter.add(this.right.target);
        supportCenter.multiplyScalar(1 / contacts);
        const hipWorld = hips.getWorldPosition(new Vector3());
        const balance = supportCenter.sub(hipWorld);
        balance.y = 0;
        balance.clampLength(0, maximum * 0.35);
        correction.addScaledVector(balance, Math.min(Math.max(strength, 0), 1) * 0.16);
        translateWorld(hips, correction);
        hips.updateWorldMatrix(true, true);
      }
    }
    if (leftContact && leftChain) {
      solveTwoBoneIk(leftChain, this.left.target, strength);
      stabilizeFoot(leftChain, this.left, strength);
    }
    if (rightContact && rightChain) {
      solveTwoBoneIk(rightChain, this.right.target, strength);
      stabilizeFoot(rightChain, this.right, strength);
    }
    return {
      maxDrift: Math.max(
        leftContact && leftChain
          ? leftChain.foot.getWorldPosition(new Vector3()).distanceTo(this.left.target)
          : 0,
        rightContact && rightChain
          ? rightChain.foot.getWorldPosition(new Vector3()).distanceTo(this.right.target)
          : 0,
      ),
    };
  }

  reset(): void {
    this.left.contact = false;
    this.right.contact = false;
    this.left.phase = "air";
    this.right.phase = "air";
  }

  measureFootDrift(leftChain: LegChain | undefined, rightChain: LegChain | undefined): number {
    return Math.max(
      this.left.contact && leftChain
        ? leftChain.foot.getWorldPosition(new Vector3()).distanceTo(this.left.target)
        : 0,
      this.right.contact && rightChain
        ? rightChain.foot.getWorldPosition(new Vector3()).distanceTo(this.right.target)
        : 0,
    );
  }

  private captureContact(
    lock: FootLock,
    chain: LegChain | undefined,
    phase: FootContactPhase,
  ): void {
    const contact = phase !== "air";
    if (chain && contact && !lock.contact) {
      chain.foot.getWorldPosition(lock.target);
      chain.foot.getWorldQuaternion(lock.rotation);
    }
    lock.contact = Boolean(chain && contact);
    lock.phase = chain ? phase : "air";
  }
}

function stabilizeFoot(chain: LegChain, lock: FootLock, strength: number): void {
  const phaseWeight = lock.phase === "flat" ? 1 : lock.phase === "heel" ? 0.72 : 0.58;
  const current = chain.foot.getWorldQuaternion(new Quaternion());
  const target = current.slerp(lock.rotation, Math.min(Math.max(strength * phaseWeight, 0), 1));
  if (chain.foot.parent) {
    const parentInverse = chain.foot.parent.getWorldQuaternion(new Quaternion()).invert();
    chain.foot.quaternion.copy(parentInverse.multiply(target)).normalize();
  } else {
    chain.foot.quaternion.copy(target).normalize();
  }
  if (lock.phase === "toe" && chain.toes) {
    chain.toes.quaternion.slerp(
      new Quaternion().setFromAxisAngle(new Vector3(1, 0, 0), -0.18),
      0.25,
    );
  }
  chain.foot.updateWorldMatrix(true, true);
}

/** Clamps a rest-relative joint into a swing cone plus a local-Y twist interval. */
export function applySwingTwistLimit(
  bone: Object3D,
  rest: Quaternion,
  swingDegrees: number,
  twistMinDegrees: number,
  twistMaxDegrees: number,
): number {
  const before = bone.quaternion.clone();
  const delta = rest.clone().invert().multiply(bone.quaternion).normalize();
  if (delta.w < 0) delta.set(-delta.x, -delta.y, -delta.z, -delta.w);
  const twist = new Quaternion(0, delta.y, 0, delta.w).normalize();
  const swing = delta.clone().multiply(twist.clone().invert()).normalize();
  const swingAngle = 2 * Math.acos(Math.min(Math.max(swing.w, -1), 1));
  const maxSwing = Math.max(swingDegrees, 0) * (Math.PI / 180);
  if (swingAngle > maxSwing && swingAngle > 1e-6) {
    swing.slerp(new Quaternion(), 1 - maxSwing / swingAngle);
  }
  let twistAngle = 2 * Math.atan2(twist.y, twist.w);
  if (twistAngle > Math.PI) twistAngle -= Math.PI * 2;
  if (twistAngle < -Math.PI) twistAngle += Math.PI * 2;
  const minimum = twistMinDegrees * (Math.PI / 180);
  const maximum = twistMaxDegrees * (Math.PI / 180);
  const limitedTwist = new Quaternion().setFromAxisAngle(
    new Vector3(0, 1, 0),
    Math.min(Math.max(twistAngle, minimum), maximum),
  );
  bone.quaternion.copy(rest).multiply(swing).multiply(limitedTwist).normalize();
  return (before.angleTo(bone.quaternion) * 180) / Math.PI;
}

/** Shared final constraint pipeline used by both Pet and Motion Library Lab. */
export class AvatarConstraintPipeline {
  private readonly feet = new ConstraintSolver();

  solve(rig: AvatarConstraintRig, input: AvatarConstraintInput): AvatarConstraintDiagnostics {
    const leftLeg = chain(rig, "left", "leg");
    const rightLeg = chain(rig, "right", "leg");
    this.feet.updateLowerBodyPhased(
      rig.bones.get("hips"),
      leftLeg,
      rightLeg,
      input.leftFootPhase,
      input.rightFootPhase,
      clamp01(input.footStrength),
    );
    for (const goal of input.endEffectors) {
      const side = goal.bone.startsWith("left") ? "left" : "right";
      const arm = chain(rig, side, "arm");
      if (!arm) continue;
      const shoulder = arm.upper.getWorldPosition(new Vector3());
      const target = shoulder.add(
        new Vector3(...goal.normalizedOffset).multiplyScalar(Math.max(rig.height, 0.001)),
      );
      solveTwoBoneIk(arm, target, Math.min(clamp01(goal.weight), 0.86));
    }
    let maxJointCorrectionDegrees = 0;
    for (const limit of rig.profile.jointLimits ?? []) {
      const bone = rig.bones.get(limit.bone);
      const rest = rig.restRotations.get(limit.bone);
      if (!bone || !rest) continue;
      maxJointCorrectionDegrees = Math.max(
        maxJointCorrectionDegrees,
        applySwingTwistLimit(
          bone,
          rest,
          limit.swingDegrees ?? 0,
          limit.twistMinDegrees ?? 0,
          limit.twistMaxDegrees ?? 0,
        ),
      );
    }
    const capsules: CapsuleBinding[] = [];
    for (const capsule of rig.profile.collisionCapsules ?? []) {
      const bone = rig.bones.get(capsule.bone);
      if (!bone) continue;
      capsules.push({
        id: capsule.bone,
        bone,
        radius: finite(capsule.radius, rig.height * 0.03),
        halfHeight: finite(capsule.halfHeight, rig.height * 0.05),
        movable: /arm|hand|leg|foot/.test(capsule.bone),
      });
    }
    const collisionCount = resolveCapsuleCollisions(capsules, 0.65);
    const height = Math.max(rig.height, 0.001);
    const finalFootDrift = this.feet.measureFootDrift(leftLeg, rightLeg);
    const groundY = rig.groundY ?? 0;
    const soleYs = ["left_sole", "right_sole"]
      .map((id) => contactWorldPosition(rig, id)?.y)
      .filter((value): value is number => value !== undefined);
    const groundPenetrationNormalized =
      soleYs.length === 0 ? 0 : Math.max(0, groundY - Math.min(...soleYs)) / height;
    return {
      leftFootPhase: input.leftFootPhase,
      rightFootPhase: input.rightFootPhase,
      maxFootDriftNormalized: finalFootDrift / height,
      groundPenetrationNormalized,
      maxJointCorrectionDegrees,
      collisionCount,
      centerOfMassOutsideSupport: centerOfMassOutsideSupport(rig, input),
    };
  }

  reset(): void {
    this.feet.reset();
  }
}

function chain(
  rig: AvatarConstraintRig,
  side: "left" | "right",
  kind: "leg" | "arm",
): LegChain | undefined {
  const upper = rig.bones.get(`${side}_upper_${kind}`);
  const lower = rig.bones.get(`${side}_lower_${kind}`);
  const foot = rig.bones.get(kind === "leg" ? `${side}_foot` : `${side}_hand`);
  const toes = kind === "leg" ? rig.bones.get(`${side}_toes`) : undefined;
  if (!upper || !lower || !foot) return undefined;
  const source =
    kind === "leg"
      ? side === "left"
        ? rig.profile.leftKneePole
        : rig.profile.rightKneePole
      : side === "left"
        ? rig.profile.leftElbowPole
        : rig.profile.rightElbowPole;
  const fallback = kind === "leg" ? [0, 0, 1] : [side === "left" ? -1 : 1, 0, 0.2];
  return {
    upper,
    lower,
    foot,
    ...(toes ? { toes } : {}),
    pole: new Vector3(
      finite(source?.[0], fallback[0]!),
      finite(source?.[1], fallback[1]!),
      finite(source?.[2], fallback[2]!),
    ),
  };
}

function contactWorldPosition(rig: AvatarConstraintRig, id: string): Vector3 | undefined {
  const contact = rig.profile.contacts?.find((value) => value.id === id);
  const bone = contact ? rig.bones.get(contact.bone) : undefined;
  if (!contact || !bone) return undefined;
  return bone.localToWorld(
    new Vector3(
      finite(contact.localPosition[0], 0),
      finite(contact.localPosition[1], 0),
      finite(contact.localPosition[2], 0),
    ),
  );
}

function centerOfMassOutsideSupport(
  rig: AvatarConstraintRig,
  input: AvatarConstraintInput,
): boolean {
  const contacts = [
    input.leftFootPhase === "air" ? undefined : contactWorldPosition(rig, "left_sole"),
    input.rightFootPhase === "air" ? undefined : contactWorldPosition(rig, "right_sole"),
  ].filter((value): value is Vector3 => Boolean(value));
  const hips = rig.bones.get("hips");
  if (contacts.length === 0 || !hips) return false;
  const center = hips
    .getWorldPosition(new Vector3())
    .add(new Vector3(...input.centerOfMass).multiplyScalar(Math.max(rig.height, 0.001)));
  const margin = Math.max(rig.height, 0.001) * 0.035;
  return (
    center.x < Math.min(...contacts.map((value) => value.x)) - margin ||
    center.x > Math.max(...contacts.map((value) => value.x)) + margin ||
    center.z < Math.min(...contacts.map((value) => value.z)) - margin ||
    center.z > Math.max(...contacts.map((value) => value.z)) + margin
  );
}

function finite(value: number | null | undefined, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function clamp01(value: number): number {
  return Math.min(Math.max(Number.isFinite(value) ? value : 0, 0), 1);
}

export function solveTwoBoneIk(chain: LegChain, target: Vector3, strength = 1): void {
  const hip = chain.upper.getWorldPosition(new Vector3());
  const knee = chain.lower.getWorldPosition(new Vector3());
  const ankle = chain.foot.getWorldPosition(new Vector3());
  const upperLength = Math.max(hip.distanceTo(knee), 1e-5);
  const lowerLength = Math.max(knee.distanceTo(ankle), 1e-5);
  const toTarget = target.clone().sub(hip);
  const distance = Math.min(
    Math.max(toTarget.length(), Math.abs(upperLength - lowerLength) + 1e-4),
    upperLength + lowerLength - 1e-4,
  );
  if (!Number.isFinite(distance) || distance <= 0) return;
  const forward = toTarget.normalize();
  const pole = chain.pole
    .clone()
    .sub(forward.clone().multiplyScalar(chain.pole.dot(forward)))
    .normalize();
  const along =
    (upperLength * upperLength - lowerLength * lowerLength + distance * distance) / (2 * distance);
  const height = Math.sqrt(Math.max(upperLength * upperLength - along * along, 0));
  const desiredKnee = hip.clone().addScaledVector(forward, along).addScaledVector(pole, height);
  rotateBoneToward(chain.upper, knee.clone().sub(hip), desiredKnee.clone().sub(hip), strength);
  chain.upper.updateWorldMatrix(true, true);
  const updatedKnee = chain.lower.getWorldPosition(new Vector3());
  const updatedAnkle = chain.foot.getWorldPosition(new Vector3());
  rotateBoneToward(
    chain.lower,
    updatedAnkle.sub(updatedKnee),
    target.clone().sub(updatedKnee),
    strength,
  );
  chain.lower.updateWorldMatrix(true, true);
}

/** Final conservative self-collision pass for limb capsules against torso/head capsules. */
export function resolveCapsuleCollisions(
  capsules: readonly CapsuleBinding[],
  strength = 0.65,
): number {
  let resolved = 0;
  for (let leftIndex = 0; leftIndex < capsules.length; leftIndex += 1) {
    for (let rightIndex = leftIndex + 1; rightIndex < capsules.length; rightIndex += 1) {
      const left = capsules[leftIndex]!;
      const right = capsules[rightIndex]!;
      if (left.movable === right.movable) continue;
      const limb = left.movable ? left : right;
      const body = left.movable ? right : left;
      if (isAncestor(limb.bone, body.bone) || isAncestor(body.bone, limb.bone)) continue;
      const limbSegment = worldCapsuleSegment(limb);
      const bodySegment = worldCapsuleSegment(body);
      const closest = closestSegmentPoints(
        limbSegment.start,
        limbSegment.end,
        bodySegment.start,
        bodySegment.end,
      );
      const separation = closest.left.clone().sub(closest.right);
      const distance = separation.length();
      const required = Math.max(limb.radius + body.radius, 0);
      if (!Number.isFinite(distance) || distance >= required) continue;
      const normal =
        distance > 1e-5
          ? separation.multiplyScalar(1 / distance)
          : limbSegment.axis.clone().cross(bodySegment.axis).normalize();
      if (normal.lengthSq() < 1e-8) normal.set(limb.id.startsWith("left") ? -1 : 1, 0, 0);
      const penetration = required - distance;
      const desiredAxis = limbSegment.axis
        .clone()
        .addScaledVector(normal, (penetration / Math.max(limb.halfHeight, 0.01)) * strength)
        .normalize();
      rotateBoneToward(limb.bone, limbSegment.axis, desiredAxis, Math.min(strength, 0.75));
      limb.bone.updateWorldMatrix(true, true);
      resolved += 1;
    }
  }
  return resolved;
}

function worldCapsuleSegment(capsule: CapsuleBinding): {
  start: Vector3;
  end: Vector3;
  axis: Vector3;
} {
  const center = capsule.bone.getWorldPosition(new Vector3());
  const axis = new Vector3(0, 1, 0)
    .applyQuaternion(capsule.bone.getWorldQuaternion(new Quaternion()))
    .normalize();
  return {
    start: center.clone().addScaledVector(axis, -capsule.halfHeight),
    end: center.clone().addScaledVector(axis, capsule.halfHeight),
    axis,
  };
}

function closestSegmentPoints(
  firstStart: Vector3,
  firstEnd: Vector3,
  secondStart: Vector3,
  secondEnd: Vector3,
): { left: Vector3; right: Vector3 } {
  const first = firstEnd.clone().sub(firstStart);
  const second = secondEnd.clone().sub(secondStart);
  const offset = firstStart.clone().sub(secondStart);
  const a = first.dot(first);
  const e = second.dot(second);
  const b = first.dot(second);
  const c = first.dot(offset);
  const f = second.dot(offset);
  const denominator = a * e - b * b;
  let firstT = denominator > 1e-8 ? Math.min(Math.max((b * f - c * e) / denominator, 0), 1) : 0;
  let secondT = e > 1e-8 ? (b * firstT + f) / e : 0;
  if (secondT < 0) {
    secondT = 0;
    firstT = a > 1e-8 ? Math.min(Math.max(-c / a, 0), 1) : 0;
  } else if (secondT > 1) {
    secondT = 1;
    firstT = a > 1e-8 ? Math.min(Math.max((b - c) / a, 0), 1) : 0;
  }
  return {
    left: firstStart.clone().addScaledVector(first, firstT),
    right: secondStart.clone().addScaledVector(second, secondT),
  };
}

function isAncestor(candidate: Object3D, object: Object3D): boolean {
  let parent = object.parent;
  while (parent) {
    if (parent === candidate) return true;
    parent = parent.parent;
  }
  return false;
}

function rotateBoneToward(
  bone: Object3D,
  currentDirection: Vector3,
  desiredDirection: Vector3,
  strength: number,
): void {
  if (currentDirection.lengthSq() < 1e-8 || desiredDirection.lengthSq() < 1e-8) return;
  const worldDelta = new Quaternion().setFromUnitVectors(
    currentDirection.normalize(),
    desiredDirection.normalize(),
  );
  worldDelta.slerp(new Quaternion(), 1 - Math.min(Math.max(strength, 0), 1));
  const world = bone.getWorldQuaternion(new Quaternion());
  const targetWorld = worldDelta.multiply(world);
  if (bone.parent) {
    const parentInverse = bone.parent.getWorldQuaternion(new Quaternion()).invert();
    bone.quaternion.copy(parentInverse.multiply(targetWorld)).normalize();
  } else {
    bone.quaternion.copy(targetWorld).normalize();
  }
}

function translateWorld(object: Object3D, worldOffset: Vector3): void {
  if (!object.parent) {
    object.position.add(worldOffset);
    return;
  }
  const origin = object.parent.worldToLocal(new Vector3());
  const translated = object.parent.worldToLocal(worldOffset.clone());
  object.position.add(translated.sub(origin));
}
