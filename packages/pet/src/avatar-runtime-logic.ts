import type { InteractionRegion } from "@hachimi/contracts";
import type { MotionCatalogEntry } from "@hachimi/contracts";

export function classifyRelativeHit(normalizedX: number, normalizedY: number): InteractionRegion {
  const side = normalizedX < 0 ? "left" : "right";
  const absoluteX = Math.abs(normalizedX);
  if (normalizedY >= 0.84) return "head_top";
  if (normalizedY >= 0.7) return "face";
  if (normalizedY >= 0.48 && absoluteX >= 0.62) return `${side}_hand`;
  if (normalizedY >= 0.42 && absoluteX >= 0.38) return `${side}_arm`;
  if (normalizedY >= 0.5) return "chest";
  if (normalizedY >= 0.35) return "belly";
  if (normalizedY >= 0.25) return "hips";
  if (normalizedY >= 0.08) return `${side}_leg`;
  return "foot";
}

export function isPlaybackOlder(candidateId: string, latestId: string | undefined): boolean {
  if (!latestId) return false;
  const candidate = Number(candidateId);
  const latest = Number(latestId);
  return Number.isFinite(candidate) && Number.isFinite(latest) && candidate < latest;
}

export function speechReleaseEnvelope(releaseFrom: number, elapsedMs: number): number {
  const amplitude = Math.min(Math.max(Number.isFinite(releaseFrom) ? releaseFrom : 0, 0), 1);
  const time = Math.min(Math.max(Number.isFinite(elapsedMs) ? elapsedMs / 110 : 1, 0), 1);
  const smooth = time * time * (3 - 2 * time);
  return amplitude * (1 - smooth);
}

export function idleMotionWeight(entry: MotionCatalogEntry, energy: number): number {
  const normalized = Math.min(Math.max(Number.isFinite(energy) ? energy : 0.5, 0), 1);
  const name = entry.name.toLowerCase();
  if (/energetic|powerful|flamboyant/.test(name)) return 0.25 + normalized * 1.75;
  if (/shy|ladylike|gentleman|cool/.test(name)) return 1.65 - normalized * 0.85;
  return 0.85 + (1 - Math.abs(normalized - 0.5) * 2) * 0.4;
}

export function selectWeightedIdle(
  entries: readonly MotionCatalogEntry[],
  energy: number,
  random: number,
): MotionCatalogEntry | undefined {
  if (entries.length === 0) return undefined;
  const weights = entries.map((entry) => idleMotionWeight(entry, energy));
  const total = weights.reduce((sum, value) => sum + value, 0);
  let cursor = Math.min(Math.max(Number.isFinite(random) ? random : 0, 0), 0.999_999) * total;
  for (let index = 0; index < entries.length; index += 1) {
    cursor -= weights[index] ?? 0;
    if (cursor < 0) return entries[index];
  }
  return entries.at(-1);
}

export function selectShyIdle(
  entries: readonly MotionCatalogEntry[],
): MotionCatalogEntry | undefined {
  return entries.find((entry) => /shy.?waiting/i.test(entry.name)) ?? entries[0];
}

export function selectAmbientIdle(
  entries: readonly MotionCatalogEntry[],
  lastMotionId: string | undefined,
  energy: number,
  random: number,
): MotionCatalogEntry | undefined {
  const alternatives = entries.filter(
    (entry) => !/shy.?waiting/i.test(entry.name) && entry.id !== lastMotionId,
  );
  const pool =
    alternatives.length > 0
      ? alternatives
      : entries.filter((entry) => !/shy.?waiting/i.test(entry.name));
  return selectWeightedIdle(pool, energy, random);
}

export function canStartQueuedInteraction(
  now: number,
  executeAt: number,
  idleReturnReadyAt: number,
  foregroundActive: boolean,
): boolean {
  return !foregroundActive && now >= executeAt && now >= idleReturnReadyAt;
}
