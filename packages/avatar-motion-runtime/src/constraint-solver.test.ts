import { describe, expect, it } from "vitest";
import { Bone, Euler, Quaternion, Vector3 } from "three";
import { ConstraintSolver, applySwingTwistLimit, solveTwoBoneIk } from "./constraint-solver";

function leg() {
  const hips = new Bone();
  const upper = new Bone();
  const lower = new Bone();
  const foot = new Bone();
  upper.position.set(0, -0.5, 0);
  lower.position.set(0, -0.5, 0);
  foot.position.set(0, -0.15, 0.1);
  hips.add(upper);
  upper.add(lower);
  lower.add(foot);
  hips.updateWorldMatrix(true, true);
  return { hips, upper, lower, foot, pole: new Vector3(0, 0, 1) };
}

describe("shared ConstraintSolver", () => {
  it("solves a reachable foot target without flipping the knee", () => {
    const chain = leg();
    const target = new Vector3(0.12, -1.05, 0.08);
    solveTwoBoneIk(chain, target, 1);
    chain.hips.updateWorldMatrix(true, true);
    expect(chain.foot.getWorldPosition(new Vector3()).distanceTo(target)).toBeLessThan(0.08);
    expect(chain.lower.getWorldPosition(new Vector3()).z).toBeGreaterThanOrEqual(-0.01);
  });

  it("locks contact feet and applies bounded hip compensation", () => {
    const left = leg();
    const right = leg();
    right.hips = left.hips;
    right.upper.position.x = -0.2;
    left.upper.position.x = 0.2;
    left.hips.add(right.upper);
    left.hips.updateWorldMatrix(true, true);
    const solver = new ConstraintSolver();
    solver.updateLowerBody(left.hips, left, right, true, true, 0.8);
    const before = left.hips.position.clone();
    left.foot.position.y += 0.03;
    solver.updateLowerBody(left.hips, left, right, true, true, 0.8);
    expect(left.hips.position.distanceTo(before)).toBeLessThan(0.1);
  });

  it("clamps rest-relative swing and twist", () => {
    const bone = new Bone();
    const rest = new Quaternion();
    bone.quaternion.setFromEuler(new Euler(1.2, 1.1, 0.8));
    applySwingTwistLimit(bone, rest, 30, -20, 20);
    expect(rest.angleTo(bone.quaternion)).toBeLessThan((55 * Math.PI) / 180);
  });
});
