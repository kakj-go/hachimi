import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it } from "vitest";

import { WorkbenchResizeHandle } from "./workbench-resize-handle";

function renderHandle(direction: 1 | -1 = 1) {
  const root = document.createElement("div");
  document.body.append(root);
  const [value, setValue] = createSignal(300);
  const dispose = render(
    () => (
      <WorkbenchResizeHandle
        orientation="vertical"
        value={value()}
        minimum={220}
        maximum={480}
        defaultValue={288}
        direction={direction}
        label="Resize pane"
        onChange={setValue}
      />
    ),
    root,
  );
  return {
    dispose,
    element: root.querySelector<HTMLElement>('[role="separator"]')!,
    value,
  };
}

afterEach(() => document.body.replaceChildren());

describe("WorkbenchResizeHandle", () => {
  it("supports directional keyboard resizing and clamps at pane bounds", () => {
    const handle = renderHandle(-1);
    handle.element.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }));
    expect(handle.value()).toBe(310);
    for (let index = 0; index < 30; index += 1) {
      handle.element.dispatchEvent(
        new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }),
      );
    }
    expect(handle.value()).toBe(480);
    handle.dispose();
  });

  it("resets to the default value on double click", () => {
    const handle = renderHandle();
    handle.element.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    expect(handle.value()).toBe(288);
    handle.dispose();
  });
});
