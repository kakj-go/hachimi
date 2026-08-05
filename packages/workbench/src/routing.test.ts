import { describe, expect, it } from "vitest";
import { normalizeWorkbenchRoute, pushWorkbenchHistory, WORKBENCH_ROUTES } from "./routing";

describe("workbench routing", () => {
  it("accepts the independent motion settings page and redirects removed settings links", () => {
    expect(normalizeWorkbenchRoute("settings/llm")).toBe("settings/llm");
    expect(normalizeWorkbenchRoute("settings/avatar")).toBe("settings/avatar");
    expect(normalizeWorkbenchRoute("settings/appearance")).toBe("settings/appearance");
    expect(normalizeWorkbenchRoute("settings/motion")).toBe("settings/motion");
    expect(normalizeWorkbenchRoute("settings/skills")).toBe("settings/skills");
    expect(normalizeWorkbenchRoute("settings/mcp")).toBe("settings/mcp");
    expect(normalizeWorkbenchRoute("developer/motion-lab")).toBe("developer/motion-lab");
    for (const route of WORKBENCH_ROUTES) expect(normalizeWorkbenchRoute(route)).toBe(route);
    expect(normalizeWorkbenchRoute("settings/integrations")).toBe("settings/integrations");
    for (const removed of [
      "settings/connected-apps",
      "settings/channels",
      "settings/gateway",
      "settings/plugins",
    ]) {
      expect(normalizeWorkbenchRoute(removed)).toBe("settings/general");
    }
    expect(normalizeWorkbenchRoute("settings/worktrees")).toBe("settings/general");
    expect(normalizeWorkbenchRoute("workspace/admin")).toBe("home");
    expect(normalizeWorkbenchRoute(null)).toBe("home");
  });

  it("drops forward entries after navigating from history", () => {
    expect(
      pushWorkbenchHistory(["home", "settings/llm", "settings/voice"], 1, "settings/general"),
    ).toEqual({ history: ["home", "settings/llm", "settings/general"], index: 2 });
  });
});
