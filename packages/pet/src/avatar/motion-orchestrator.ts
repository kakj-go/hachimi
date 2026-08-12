import type { MotionIntentRequest, MotionSlot } from "@hachimi/contracts";

const SLOT_ORDER: readonly MotionSlot[] = ["base", "locomotion", "speech", "action"];

/** Product-owned intent arbitration. Runtime sampling never decides product priority. */
export class PetMotionOrchestrator {
  private readonly intents = new Map<string, MotionIntentRequest & { updatedAt: number }>();

  submit(intent: MotionIntentRequest, nowMs: number): boolean {
    if (!intent.requestId.trim()) return false;
    if (!intent.active) return this.intents.delete(intent.requestId);
    this.intents.set(intent.requestId, { ...intent, updatedAt: nowMs });
    return true;
  }

  winners(): readonly MotionIntentRequest[] {
    return SLOT_ORDER.flatMap((slot) => {
      const winner = [...this.intents.values()]
        .filter((intent) => intent.slot === slot)
        .sort(
          (left, right) =>
            right.priority - left.priority ||
            right.updatedAt - left.updatedAt ||
            left.requestId.localeCompare(right.requestId),
        )[0];
      return winner ? [winner] : [];
    });
  }

  clear(): void {
    this.intents.clear();
  }
}
