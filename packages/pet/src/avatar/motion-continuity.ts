import type { MotionCatalogEntry, MotionTransitionProfile } from "@hachimi/contracts";

export { cloneSampledPose, limitPoseStep } from "@hachimi/avatar-motion-runtime";

export function motionEnvelope(
  entry: MotionCatalogEntry,
  timeMs: number,
  profile: MotionTransitionProfile | undefined,
): number {
  const duration = profile?.preferredDurationMs ?? 180;
  const fadeIn = smooth01(timeMs / Math.max(duration, 1));
  if (entry.loopMode !== "once") return fadeIn;
  const remaining = entry.durationMs - timeMs;
  return Math.min(fadeIn, smooth01(remaining / Math.max(duration, 1)));
}

export function isMouthExpression(expression: string): boolean {
  return ["aa", "ih", "ou", "ee", "oh"].includes(expression.toLowerCase());
}

function smooth01(value: number): number {
  const clamped = Math.min(Math.max(value, 0), 1);
  return clamped * clamped * (3 - 2 * clamped);
}
