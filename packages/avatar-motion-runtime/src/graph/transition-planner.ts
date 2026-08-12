import type { MotionInterruptPolicy, MotionTransitionProfile } from "@hachimi/contracts";
import { MathUtils, Vector3 } from "three";
import type { MotionFeatureFrame, MotionFeatureIndex, TransitionPlan } from "./types";

const COST_WEIGHTS = { pose: 0.45, velocity: 0.25, footContact: 0.2, root: 0.1 } as const;

export class TransitionPlanner {
  plan(
    source: MotionFeatureFrame | undefined,
    target: MotionFeatureIndex,
    profile: MotionTransitionProfile,
    policy: MotionInterruptPolicy,
    maximumWaitMs = 120,
  ): TransitionPlan {
    const searchWindowMs = maximumWaitMs > 0 ? maximumWaitMs : 120;
    const candidates = target.frames.filter(
      (frame) => frame.safeEntry && frame.timeMs <= searchWindowMs,
    );
    const pool =
      candidates.length > 0
        ? candidates
        : target.frames.filter((frame) => frame.timeMs <= searchWindowMs);
    const scored = (pool.length > 0 ? pool : [target.frames[0]!]).map((frame) => {
      const costs = transitionCosts(source, frame);
      return {
        frame,
        costs,
        cost:
          costs.pose * COST_WEIGHTS.pose +
          costs.velocity * COST_WEIGHTS.velocity +
          costs.footContact * COST_WEIGHTS.footContact +
          costs.root * COST_WEIGHTS.root,
      };
    });
    scored.sort(
      (left, right) =>
        left.cost - right.cost ||
        (profile.syncGroup && source
          ? loopPhaseDistance(left.frame.loopPhase, source.loopPhase) -
            loopPhaseDistance(right.frame.loopPhase, source.loopPhase)
          : 0) ||
        left.frame.timeMs - right.frame.timeMs,
    );
    const best = scored[0]!;
    const forced = candidates.length === 0;
    const preferred =
      forced || policy === "immediate" ? profile.minimumDurationMs : profile.preferredDurationMs;
    return {
      targetTimeMs: best.frame.timeMs,
      durationMs: MathUtils.clamp(preferred, profile.minimumDurationMs, profile.maximumDurationMs),
      forced,
      cost: best.cost,
      costs: best.costs,
    };
  }
}

function loopPhaseDistance(left: number, right: number): number {
  const difference = Math.abs(left - right) % 1;
  return Math.min(difference, 1 - difference);
}

export function transitionCosts(
  source: MotionFeatureFrame | undefined,
  target: MotionFeatureFrame,
): TransitionPlan["costs"] {
  if (!source) return { pose: 0, velocity: 0, footContact: 0, root: 0 };
  let pose = 0;
  let velocity = 0;
  let count = 0;
  for (const [name, targetRotation] of target.pose.rotations) {
    const sourceRotation = source.pose.rotations.get(name);
    if (!sourceRotation) continue;
    pose += sourceRotation.angleTo(targetRotation) / Math.PI;
    velocity += Math.min(
      (source.velocity.angular.get(name) ?? new Vector3()).distanceTo(
        target.velocity.angular.get(name) ?? new Vector3(),
      ) / 20,
      1,
    );
    count += 1;
  }
  const contactsKnown = source.footContact !== "unknown" && target.footContact !== "unknown";
  return {
    pose: count > 0 ? pose / count : 1,
    velocity: count > 0 ? velocity / count : 1,
    footContact: contactsKnown && source.footContact !== target.footContact ? 1 : 0,
    root: Math.min(
      (source.pose.hipsPosition ?? new Vector3()).distanceTo(
        target.pose.hipsPosition ?? new Vector3(),
      ),
      1,
    ),
  };
}

export { COST_WEIGHTS as TRANSITION_COST_WEIGHTS };
