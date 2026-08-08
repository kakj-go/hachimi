import { describe, expect, it } from "vitest";

import { isManualCompactionCommand } from "./manual-compaction";

describe("manual compaction command", () => {
  it("recognizes only the exact trimmed command", () => {
    expect(isManualCompactionCommand("/compact")).toBe(true);
    expect(isManualCompactionCommand("  /compact\n")).toBe(true);
    expect(isManualCompactionCommand("/compact foo")).toBe(false);
    expect(isManualCompactionCommand("/Compact")).toBe(false);
  });
});
