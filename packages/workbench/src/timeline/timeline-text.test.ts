import { describe, expect, it } from "vitest";

import { timelineActivityLabel, timelineItemText } from "./timeline-text";

describe("timeline tool presentation", () => {
  const payload = {
    type: "tool_execution",
    data: {
      name: "browser_observe",
      result: {
        status: "succeeded",
        stableResultCode: "browser_observe_completed",
        modelContent: JSON.stringify({
          title: "Workbench fixture",
          origin: "http://127.0.0.1:1234",
          browserSessionId: "secret-runtime-id",
        }),
      },
    },
  };

  it("uses the concrete tool name for a compact activity row", () => {
    expect(timelineActivityLabel("tool_execution", payload, "zh-CN")).toBe("browser_observe");
  });

  it("summarizes structured output without rendering raw JSON", () => {
    const text = timelineItemText(payload);
    expect(text).toContain("succeeded");
    expect(text).toContain("Workbench fixture");
    expect(text).toContain("http://127.0.0.1:1234");
    expect(text).not.toContain("browserSessionId");
    expect(text).not.toContain("{");
  });

  it("keeps a stable tool error code visible", () => {
    const text = timelineItemText({
      ...payload,
      data: {
        ...payload.data,
        result: {
          status: "failed",
          stableResultCode: "failed",
          modelContent: JSON.stringify({ errorCode: "forge_remote_drift" }),
        },
      },
    });
    expect(text).toContain("forge_remote_drift");
    expect(text).not.toContain("{");
  });

  it("keeps dynamic tool failures visible without a generic payload dump", () => {
    const text = timelineItemText({
      type: "dynamic_tool_call",
      data: {
        namespace: "git",
        name: "push",
        status: "failed",
        result: { error: "forge_remote_drift: remote changed" },
        error: "forge_remote_drift: remote changed",
      },
    });
    expect(text).toBe("failed · forge_remote_drift: remote changed");
  });
});
