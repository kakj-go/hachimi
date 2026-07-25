import { describe, expect, it, vi } from "vitest";
import { createSerializedAutosave } from "./appearance-save";

describe("serialized appearance autosave", () => {
  it("coalesces debounced values and serializes later writes", async () => {
    vi.useFakeTimers();
    const saved: number[] = [];
    const autosave = createSerializedAutosave({
      initial: 0,
      save: async (value: number) => {
        saved.push(value);
        return value;
      },
      onConfirmed: () => undefined,
      onRollback: () => undefined,
    });
    autosave.schedule(1);
    autosave.schedule(2);
    await vi.advanceTimersByTimeAsync(250);
    await autosave.flush();
    expect(saved).toEqual([2]);
    autosave.dispose();
    vi.useRealTimers();
  });

  it("reports pending, saving, and confirmed states", async () => {
    const statuses: string[] = [];
    const autosave = createSerializedAutosave({
      initial: 0,
      save: async (value: number) => value,
      onConfirmed: () => undefined,
      onRollback: () => undefined,
      onStatusChange: (status) => statuses.push(status),
    });
    autosave.schedule(1, true);
    await autosave.flush();
    expect(statuses).toEqual(["pending", "saving", "saved"]);
    autosave.dispose();
  });

  it("rolls back to the last confirmed value after failure", async () => {
    let rolledBack = -1;
    const statuses: string[] = [];
    const autosave = createSerializedAutosave({
      initial: 4,
      save: async () => Promise.reject(new Error("disk full")),
      onConfirmed: () => undefined,
      onRollback: (value) => {
        rolledBack = value;
      },
      onStatusChange: (status) => statuses.push(status),
    });
    autosave.schedule(5, true);
    await autosave.flush();
    expect(rolledBack).toBe(4);
    expect(statuses).toEqual(["pending", "saving", "error"]);
  });
});
