import { describe, expect, it } from "vitest";

import {
  EMPTY_INSPECTOR_TABS,
  closeInspectorTab,
  openInspectorTab,
  selectInspectorTab,
  showInspectorLauncher,
  showInspectorTabs,
} from "./inspector-tabs";

describe("inspector tabs", () => {
  let nextId = 0;
  const createId = () => `inspector-${++nextId}`;

  it("keeps resource tabs while the launcher or panel visibility changes", () => {
    const files = openInspectorTab(
      EMPTY_INSPECTOR_TABS,
      { kind: "files", path: "src/main.rs" },
      createId,
    );
    const browser = openInspectorTab(files, { kind: "browser" }, createId);
    const launcher = showInspectorLauncher(browser);

    expect(launcher.tabs).toHaveLength(2);
    expect(launcher.resource).toEqual({ kind: "tools" });
    expect(closeInspectorTab(launcher, browser.tabs[1]!.id).resource).toEqual({ kind: "tools" });
    expect(showInspectorTabs(launcher).resource).toEqual({ kind: "browser" });
  });

  it("reuses tool tabs while refreshing their selected resource", () => {
    const first = openInspectorTab(
      EMPTY_INSPECTOR_TABS,
      { kind: "files", path: "src/main.rs" },
      createId,
    );
    const updated = openInspectorTab(first, { kind: "files", path: "src/lib.rs" }, createId);

    expect(updated.tabs).toHaveLength(1);
    expect(updated.resource).toEqual({ kind: "files", path: "src/lib.rs" });
  });

  it("selects an adjacent tab and becomes empty after the final close", () => {
    const files = openInspectorTab(EMPTY_INSPECTOR_TABS, { kind: "files" }, createId);
    const browser = openInspectorTab(files, { kind: "browser" }, createId);
    const selectedFiles = selectInspectorTab(browser, files.tabs[0]!.id);
    const afterFiles = closeInspectorTab(selectedFiles, files.tabs[0]!.id);

    expect(afterFiles.resource).toEqual({ kind: "browser" });
    expect(closeInspectorTab(afterFiles, browser.tabs[1]!.id)).toEqual(EMPTY_INSPECTOR_TABS);
  });
});
