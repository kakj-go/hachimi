import userEvent from "@testing-library/user-event";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it } from "vitest";

import { FloatingPopover } from "./popover";

afterEach(() => {
  document.body.replaceChildren();
});

describe("FloatingPopover", () => {
  it("dismisses on outside interaction and Escape", async () => {
    const host = document.createElement("div");
    document.body.append(host);

    function Harness() {
      const [open, setOpen] = createSignal(false);
      return (
        <>
          <FloatingPopover
            open={open()}
            onOpenChange={setOpen}
            label="Task options"
            triggerTestId="popover-trigger"
            trigger="Open"
          >
            <button type="button">Menu action</button>
          </FloatingPopover>
          <button type="button" data-testid="outside">
            Outside
          </button>
        </>
      );
    }

    const dispose = render(() => <Harness />, host);
    await userEvent.click(host.querySelector('[data-testid="popover-trigger"]')!);
    expect(
      document.body.querySelector('[data-component="floating-popover-content"]'),
    ).not.toBeNull();

    await userEvent.click(host.querySelector('[data-testid="outside"]')!);
    expect(document.body.querySelector('[data-component="floating-popover-content"]')).toBeNull();

    await userEvent.click(host.querySelector('[data-testid="popover-trigger"]')!);
    await userEvent.keyboard("{Escape}");
    expect(document.body.querySelector('[data-component="floating-popover-content"]')).toBeNull();

    dispose();
  });
});
