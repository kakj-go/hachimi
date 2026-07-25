import type { MotionCatalogEntry } from "@hachimi/contracts";
import { describe, expect, it } from "vitest";
import {
  INTERACTION_REGIONS,
  MOTION_CATEGORIES,
  interactionRegionLabel,
  motionCategoryLabel,
  motionDescription,
  motionName,
} from "./motion-localization";

const entry = {
  name: "standard waiting",
  nameZh: "标准待机",
  description: "Natural idle motion.",
  descriptionZh: "自然的待机动作。",
} as MotionCatalogEntry;

describe("motion settings localization", () => {
  it("uses curated catalog text in both locales", () => {
    expect(motionName(entry, "zh-CN")).toBe("标准待机");
    expect(motionName(entry, "en-US")).toBe("standard waiting");
    expect(motionDescription(entry, "zh-CN")).toBe("自然的待机动作。");
    expect(motionDescription(entry, "en-US")).toBe("Natural idle motion.");
  });

  it("provides labels for every category and interaction region", () => {
    for (const category of MOTION_CATEGORIES) {
      expect(motionCategoryLabel(category, "zh-CN")).not.toBe(category);
      expect(motionCategoryLabel(category, "en-US")).not.toBe("");
    }
    expect(INTERACTION_REGIONS).toHaveLength(13);
    for (const region of INTERACTION_REGIONS) {
      expect(interactionRegionLabel(region, "zh-CN")).not.toBe(region);
      expect(interactionRegionLabel(region, "en-US")).not.toBe("");
    }
  });
});
