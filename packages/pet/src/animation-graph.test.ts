import { describe, expect, it } from "vitest";
import { Quaternion } from "three";
import { PoseTransitionInertializer, estimatePoseAngularVelocity } from "./animation-graph";

describe("transition-only animation graph", () => {
  it("decays a captured offset onto fresh targets instead of filtering every frame", () => {
    const inertializer = new PoseTransitionInertializer();
    const current = new Map([
      ["head", new Quaternion().setFromAxisAngle({ x: 1, y: 0, z: 0 }, 0.6)],
    ]);
    const target = new Map([["head", new Quaternion()]]);
    inertializer.capture(current, target);
    const first = inertializer.apply(target, 0.016).get("head")!;
    const later = inertializer.apply(target, 0.3).get("head")!;
    expect(first.angleTo(new Quaternion())).toBeGreaterThan(later.angleTo(new Quaternion()));
  });

  it("carries measured angular velocity through a phase transition", () => {
    const previous = new Map([["head", new Quaternion()]]);
    const current = new Map([
      ["head", new Quaternion().setFromAxisAngle({ x: 0, y: 1, z: 0 }, 0.2)],
    ]);
    const velocity = estimatePoseAngularVelocity(previous, current, 0.1);
    expect(velocity.get("head")?.y).toBeCloseTo(2, 5);

    const inertializer = new PoseTransitionInertializer();
    const target = new Map([["head", new Quaternion()]]);
    inertializer.capture(current, target, velocity);
    const carried = inertializer.apply(target, 0.016).get("head")!;
    expect(carried.angleTo(target.get("head")!)).toBeGreaterThan(0.15);
  });
});
