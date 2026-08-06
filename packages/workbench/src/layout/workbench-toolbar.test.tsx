import type { JSX } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import { WorkbenchToolbar } from "./workbench-toolbar";

vi.mock("@hachimi/ui", () => {
  const Icon = () => <span aria-hidden="true" />;
  return {
    FolderOpen: Icon,
    MoreHorizontal: Icon,
    PanelLeftClose: Icon,
    SlidersHorizontal: Icon,
    TerminalSquare: Icon,
    Button: (props: {
      children?: JSX.Element;
      class?: string;
      disabled?: boolean;
      title?: string;
      "aria-label"?: string;
      "data-testid"?: string;
      onClick?: () => void;
    }) => (
      <button
        class={props.class}
        disabled={props.disabled}
        title={props.title}
        aria-label={props["aria-label"]}
        data-testid={props["data-testid"]}
        onClick={() => props.onClick?.()}
      >
        {props.children}
      </button>
    ),
  };
});

function renderToolbar(hasProject: boolean) {
  const root = document.createElement("div");
  document.body.append(root);
  const onToggleSidebar = vi.fn();
  const onToggleBottomPanel = vi.fn();
  const dispose = render(
    () => (
      <WorkbenchToolbar
        locale="zh-CN"
        hasProject={hasProject}
        hasSession={false}
        sessionTitle={undefined}
        summaryPinned={false}
        bottomPanelOpen={false}
        sidebarVisible={false}
        onOpenLocation={vi.fn()}
        onToggleSummary={vi.fn()}
        onToggleBottomPanel={onToggleBottomPanel}
        onToggleSidebar={onToggleSidebar}
      />
    ),
    root,
  );
  return { dispose, onToggleBottomPanel, onToggleSidebar };
}

afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

describe("WorkbenchToolbar project tools", () => {
  it("opens the terminal and right workspace from the top toolbar", () => {
    const view = renderToolbar(true);
    document
      .querySelector<HTMLButtonElement>('[data-testid="workbench-toggle-bottom-panel"]')
      ?.click();
    expect(view.onToggleBottomPanel).toHaveBeenCalledOnce();
    document
      .querySelector<HTMLButtonElement>('[data-testid="workbench-toggle-inspector"]')
      ?.click();
    expect(view.onToggleSidebar).toHaveBeenCalledOnce();
    expect(document.querySelector('[data-testid="workbench-resource-menu"]')).toBeNull();
    view.dispose();
  });

  it("keeps the workspace toggle available without a project", () => {
    const view = renderToolbar(false);
    const sidebar = document.querySelector<HTMLButtonElement>(
      '[data-testid="workbench-toggle-inspector"]',
    );
    expect(sidebar?.disabled).toBe(false);
    sidebar?.click();
    expect(view.onToggleSidebar).toHaveBeenCalledOnce();
    view.dispose();
  });
});
