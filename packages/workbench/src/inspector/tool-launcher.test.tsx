import type { JSX } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import { InspectorToolLauncher } from "./tool-launcher";

vi.mock("@hachimi/ui", () => {
  const Icon = () => <span aria-hidden="true" />;
  return {
    File: Icon,
    GitPullRequest: Icon,
    Globe: Icon,
    TerminalSquare: Icon,
    Button: (props: {
      children?: JSX.Element;
      class?: string;
      disabled?: boolean;
      title?: string;
      onClick?: () => void;
    }) => (
      <button
        class={props.class}
        disabled={props.disabled}
        title={props.title}
        onClick={() => props.onClick?.()}
      >
        {props.children}
      </button>
    ),
  };
});

afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

describe("InspectorToolLauncher", () => {
  it("opens project tools inside the right workspace", () => {
    const root = document.createElement("div");
    document.body.append(root);
    const onOpenInspector = vi.fn();
    const onOpenTerminal = vi.fn();
    const dispose = render(
      () => (
        <InspectorToolLauncher
          locale="zh-CN"
          hasProject
          onOpenInspector={onOpenInspector}
          onOpenTerminal={onOpenTerminal}
        />
      ),
      root,
    );
    const labels = [...root.querySelectorAll("button")].map((button) => button.textContent);
    expect(labels).toEqual(["审阅Ctrl+Shift+G", "终端", "浏览器Ctrl+T", "文件Ctrl+P"]);
    [...root.querySelectorAll("button")]
      .find((button) => button.textContent === "审阅Ctrl+Shift+G")
      ?.click();
    expect(onOpenInspector).toHaveBeenCalledWith({ kind: "review", diffScope: "checkout" });
    [...root.querySelectorAll("button")].find((button) => button.textContent === "终端")?.click();
    expect(onOpenTerminal).toHaveBeenCalledOnce();
    dispose();
  });

  it("shows the right workspace while disabling project-bound tools without a project", () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(
      () => (
        <InspectorToolLauncher
          locale="zh-CN"
          hasProject={false}
          onOpenInspector={vi.fn()}
          onOpenTerminal={vi.fn()}
        />
      ),
      root,
    );
    const terminal = [...root.querySelectorAll("button")].find(
      (button) => button.textContent === "终端",
    );
    expect(terminal?.disabled).toBe(true);
    expect(root.querySelector('[data-testid="workbench-resource-menu"]')).not.toBeNull();
    dispose();
  });
});
