import type { TaskRunRecord } from "@hachimi/contracts";
import { describe, expect, it } from "vitest";

import { formatTaskDuration, taskRunTriggerLabel } from "./task-center-format";

describe("task center formatting", () => {
  it("localizes every run trigger instead of exposing protocol values", () => {
    expect(taskRunTriggerLabel("manual", true)).toBe("手动执行");
    expect(taskRunTriggerLabel("manual", false)).toBe("Manual");
    expect(taskRunTriggerLabel("scheduled", true)).toBe("计划触发");
    expect(taskRunTriggerLabel("retry", false)).toBe("Retry");
    expect(taskRunTriggerLabel("catch_up", true)).toBe("补偿执行");
    expect(taskRunTriggerLabel("event", false)).toBe("Event");
  });

  it("formats completed run duration in the selected language", () => {
    const run = {
      status: "succeeded",
      startedAtMs: 1_000,
      finishedAtMs: 63_000,
      updatedAtMs: 63_000,
    } as TaskRunRecord;

    expect(formatTaskDuration(run, true)).toBe("1 分 2 秒");
    expect(formatTaskDuration(run, false)).toBe("1 min 2 sec");
  });
});
