import { beforeEach, describe, expect, it } from "vitest";

import {
  DEFAULT_WORKBENCH_LAYOUT,
  WORKBENCH_LAYOUT_STORAGE_KEY,
  persistWorkbenchLayout,
  readWorkbenchLayout,
} from "./workbench-layout";

describe("workbench layout", () => {
  beforeEach(() => window.localStorage.clear());

  it("starts with summary, terminal and inspector hidden", () => {
    expect(readWorkbenchLayout()).toEqual({
      summaryPinned: false,
      bottomPanelOpen: false,
      sidebarVisible: false,
      projectSidebarWidth: 288,
      inspectorWidth: 380,
      bottomPanelHeight: 250,
    });
  });

  it("restores persisted layout state", () => {
    persistWorkbenchLayout({
      summaryPinned: true,
      bottomPanelOpen: true,
      sidebarVisible: true,
      projectSidebarWidth: 320,
      inspectorWidth: 440,
      bottomPanelHeight: 300,
    });
    expect(window.localStorage.getItem(WORKBENCH_LAYOUT_STORAGE_KEY)).toBeTruthy();
    expect(readWorkbenchLayout()).toEqual({
      summaryPinned: true,
      bottomPanelOpen: true,
      sidebarVisible: true,
      projectSidebarWidth: 320,
      inspectorWidth: 440,
      bottomPanelHeight: 300,
    });
    expect(DEFAULT_WORKBENCH_LAYOUT.sidebarVisible).toBe(false);
  });

  it("repairs invalid and out-of-range persisted pane dimensions", () => {
    window.localStorage.setItem(
      WORKBENCH_LAYOUT_STORAGE_KEY,
      JSON.stringify({
        summaryPinned: "yes",
        bottomPanelOpen: true,
        sidebarVisible: null,
        projectSidebarWidth: 1_450,
        inspectorWidth: -20,
        bottomPanelHeight: "500",
      }),
    );

    expect(readWorkbenchLayout()).toEqual({
      summaryPinned: false,
      bottomPanelOpen: true,
      sidebarVisible: false,
      projectSidebarWidth: 480,
      inspectorWidth: 300,
      bottomPanelHeight: 250,
    });
  });
});
