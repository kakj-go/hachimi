import type { MotionInterruptPolicy, MotionSlot } from "@hachimi/contracts";

export const PET_MOTION_PRIORITIES = {
  drag: 100,
  interaction: 90,
  locomotion: 80,
  speech: 70,
  autonomous: 40,
  idle: 10,
} as const;

export interface ScheduledBehavior<T> {
  id: string;
  category: string;
  slot: MotionSlot;
  priority: number;
  interruptPolicy: MotionInterruptPolicy;
  requestedAt: number;
  deadlineAt: number;
  payload: T;
}

/** Coalesces repeated input and guarantees that direct interaction cannot wait past its deadline. */
export class BehaviorScheduler<T> {
  private readonly pending = new Map<string, ScheduledBehavior<T>>();

  schedule(
    behavior: Omit<ScheduledBehavior<T>, "deadlineAt"> & { maximumWaitMs?: number },
  ): void {
    const maximumWaitMs = behavior.maximumWaitMs ?? (behavior.priority >= 70 ? 120 : 240);
    this.pending.set(behavior.category, {
      ...behavior,
      deadlineAt: behavior.requestedAt + maximumWaitMs,
    });
  }

  takeReady(nowMs: number, safeSlots: ReadonlySet<MotionSlot> = new Set()): ScheduledBehavior<T>[] {
    const ready = [...this.pending.values()]
      .filter(
        (behavior) =>
          behavior.interruptPolicy === "immediate" ||
          safeSlots.has(behavior.slot) ||
          nowMs >= behavior.deadlineAt,
      )
      .sort(
        (left, right) =>
          right.priority - left.priority ||
          left.requestedAt - right.requestedAt ||
          left.id.localeCompare(right.id),
      );
    for (const behavior of ready) this.pending.delete(behavior.category);
    return ready;
  }

  clear(): void {
    this.pending.clear();
  }

  get size(): number {
    return this.pending.size;
  }
}
