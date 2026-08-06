import type { SkillRecord } from "@hachimi/contracts";
import { describe, expect, it } from "vitest";

import { skillDisplayName } from "./skill-display";

const skill = (patch: Partial<SkillRecord>): SkillRecord => ({
  id: "skill-1",
  scope: "user",
  namespace: null,
  name: "documents",
  qualifiedName: "documents",
  description: "Documents",
  dependencies: [],
  editable: true,
  enabled: true,
  contentHash: "hash",
  treeRevision: "revision",
  diagnostics: [],
  updatedAtMs: 1,
  ...patch,
});

describe("skillDisplayName", () => {
  it("uses concise product names for built-in office Skills", () => {
    expect(skillDisplayName(skill({ scope: "built_in" }), true)).toBe("Word");
    expect(
      skillDisplayName(
        skill({ scope: "built_in", name: "Office Spreadsheets", qualifiedName: "spreadsheets" }),
        true,
      ),
    ).toBe("Excel");
  });

  it("uses a configured alias for custom Skills", () => {
    expect(
      skillDisplayName(
        skill({ interface: { displayName: "发布助手", ...emptySkillInterface() } }),
        true,
      ),
    ).toBe("发布助手");
  });
});

function emptySkillInterface() {
  return {
    shortDescription: null,
    iconSmall: null,
    iconLarge: null,
    brandColor: null,
    defaultPrompt: null,
  };
}
