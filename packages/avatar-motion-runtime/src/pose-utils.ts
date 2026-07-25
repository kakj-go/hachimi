import { Quaternion, type Euler, type Object3D } from "three";

/** Applies an additive semantic rotation in the avatar root coordinate frame. */
export function applyCanonicalBoneEuler(
  avatarRoot: Object3D,
  bone: Object3D,
  euler: Euler,
  scratch: readonly Quaternion[] = [],
): void {
  avatarRoot.updateWorldMatrix(true, false);
  bone.parent?.updateWorldMatrix(true, false);
  bone.updateWorldMatrix(false, false);
  const rootWorld = scratch[0] ?? new Quaternion();
  const rootInverse = scratch[1] ?? new Quaternion();
  const parentWorld = scratch[2] ?? new Quaternion();
  const parentInverse = scratch[3] ?? new Quaternion();
  const boneWorld = scratch[4] ?? new Quaternion();
  const semanticDelta = scratch[5] ?? new Quaternion();
  const worldDelta = scratch[6] ?? new Quaternion();
  avatarRoot.getWorldQuaternion(rootWorld);
  rootInverse.copy(rootWorld).invert();
  bone.parent?.getWorldQuaternion(parentWorld);
  parentInverse.copy(parentWorld).invert();
  bone.getWorldQuaternion(boneWorld);
  semanticDelta.setFromEuler(euler);
  worldDelta.copy(rootWorld).multiply(semanticDelta).multiply(rootInverse);
  bone.quaternion.copy(parentInverse.multiply(worldDelta).multiply(boneWorld)).normalize();
}
