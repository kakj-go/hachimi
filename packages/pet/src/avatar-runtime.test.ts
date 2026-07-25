import { describe, expect, it } from "vitest";
import {
  Bone,
  Box3,
  BufferGeometry,
  Euler,
  Float32BufferAttribute,
  Group,
  Object3D,
  Quaternion,
  Skeleton,
  SkinnedMesh,
  Uint16BufferAttribute,
  Vector3,
  type Intersection,
} from "three";
import {
  applyCanonicalBoneEuler,
  capturePresentationRootBaseline,
  classifyHit,
  restorePresentationRoot,
  stabilizePresentationRoot,
} from "./avatar-runtime";

describe("avatar presentation and normalized bone application", () => {
  it("restores a captured presentation transform exactly", () => {
    const root = new Group();
    root.position.set(1, 2, 3);
    root.rotation.set(0.1, 0.2, 0.3);
    const baseline = capturePresentationRootBaseline(root);
    root.position.set(9, 9, 9);
    restorePresentationRoot(root, baseline);
    expect(root.position.distanceTo(new Vector3(1, 2, 3))).toBeLessThan(1e-8);
    expect(root.quaternion.angleTo(baseline.quaternion)).toBeLessThan(1e-6);
  });

  it("fails closed on non-finite or stage-breaking root transforms", () => {
    const root = new Group();
    const baseline = capturePresentationRootBaseline(root);
    root.position.x = 10;
    expect(stabilizePresentationRoot(root, baseline, 1.6)).toBe(false);
    expect(root.position.length()).toBe(0);
  });

  it("applies canonical rotation through an arbitrary parent basis", () => {
    const model = new Group();
    model.rotation.set(0, Math.PI / 3, 0);
    const parent = new Object3D();
    const bone = new Object3D();
    model.add(parent);
    parent.add(bone);
    model.updateWorldMatrix(true, true);
    applyCanonicalBoneEuler(model, bone, new Euler(0.2, 0, 0));
    expect(bone.getWorldQuaternion(new Quaternion()).angleTo(new Quaternion())).toBeGreaterThan(
      0.1,
    );
  });

  it("classifies a real skinned triangle by its dominant humanoid bone", () => {
    const geometry = new BufferGeometry();
    geometry.setAttribute(
      "position",
      new Float32BufferAttribute([-0.1, 1, 0, 0.1, 1, 0, 0, 1.2, 0], 3),
    );
    geometry.setAttribute(
      "skinIndex",
      new Uint16BufferAttribute([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], 4),
    );
    geometry.setAttribute(
      "skinWeight",
      new Float32BufferAttribute([1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0], 4),
    );
    const head = new Bone();
    const mesh = new SkinnedMesh(geometry);
    mesh.add(head);
    mesh.bind(new Skeleton([head]));
    const hit = {
      distance: 1,
      object: mesh,
      point: new Vector3(0, 1.1, 0),
      face: { a: 0, b: 1, c: 2, normal: new Vector3(0, 0, 1), materialIndex: 0 },
    } as Intersection<Object3D>;
    expect(
      classifyHit(hit, {
        semanticRegions: new Map([[head, "face"]]),
        bounds: new Box3(new Vector3(-0.5, 0, -0.5), new Vector3(0.5, 1.7, 0.5)),
      }),
    ).toBe("face");
  });
});
