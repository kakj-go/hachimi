import { describe, expect, it } from "vitest";
import { WORKBENCH_MENU_ROUTES } from "./menu-routes";

describe("Pet Workbench menu", () => {
  it("maps every Workbench entry to a fixed safe route", () => {
    expect(WORKBENCH_MENU_ROUTES).toEqual({
      workbench: "home",
      llm: "settings/llm",
      avatar: "settings/avatar",
      voice: "settings/voice",
      interaction: "settings/motion",
    });
  });
});
