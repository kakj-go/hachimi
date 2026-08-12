import type { MotionCatalogEntry, MotionFamily } from "@hachimi/contracts";

export interface MotionSemanticIntent {
  family: MotionFamily;
  tags?: readonly string[];
  preferredNames?: readonly string[];
  requireFingerMotion?: boolean;
  preferFingerMotion?: boolean;
  preferredSource?: string;
  random?: number;
}

/** Resolves a semantic intent to a stable catalog ID without hard-coding any asset ID. */
export function selectMotionForIntent(
  entries: readonly MotionCatalogEntry[],
  intent: MotionSemanticIntent,
): MotionCatalogEntry | undefined {
  const requiredTags = new Set((intent.tags ?? []).map(normalize));
  const preferredNames = (intent.preferredNames ?? []).map(normalize);
  const candidates = entries
    .filter(
      (entry) =>
        entry.family === intent.family &&
        (!intent.requireFingerMotion || entry.hasFingerMotion),
    )
    .map((entry) => {
      const search = normalize(`${entry.name} ${entry.tags.join(" ")} ${entry.description}`);
      let score = 1;
      for (const tag of requiredTags) if (search.includes(tag)) score += 4;
      preferredNames.forEach((name, index) => {
        if (search.includes(name)) score += Math.max(8 - index, 1);
      });
      if (intent.preferFingerMotion && entry.hasFingerMotion) score += 5;
      if (
        intent.preferredSource &&
        normalize(entry.sourceProject).includes(normalize(intent.preferredSource))
      ) {
        score += 3;
      }
      return { entry, score };
    })
    .sort(
      (left, right) =>
        right.score - left.score ||
        right.entry.fingerBoneCount - left.entry.fingerBoneCount ||
        left.entry.id.localeCompare(right.entry.id),
    );
  if (candidates.length === 0) return undefined;
  const bestScore = candidates[0]!.score;
  const best = candidates.filter((candidate) => candidate.score === bestScore);
  const random = Math.min(Math.max(intent.random ?? 0, 0), 0.999_999);
  return best[Math.floor(random * best.length)]?.entry;
}

function normalize(value: string): string {
  return value
    .toLowerCase()
    .replaceAll(/[^a-z0-9\u4e00-\u9fff]+/g, " ")
    .trim();
}
