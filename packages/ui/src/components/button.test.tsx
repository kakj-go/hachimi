import { render } from "solid-js/web";
import { describe, expect, it } from "vitest";
import { Button } from "./button";

describe("Button shared state contract", () => {
  it("exposes invalid, loading, density, and semantic state attributes", () => {
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <Button variant="primary" size="large" tone="accent" density="compact" loading invalid>
          Send
        </Button>
      ),
      host,
    );
    const button = host.querySelector("button")!;
    expect(button).toHaveAttribute("data-component", "button");
    expect(button).toHaveAttribute("data-variant", "primary");
    expect(button).toHaveAttribute("data-size", "large");
    expect(button).toHaveAttribute("data-tone", "accent");
    expect(button).toHaveAttribute("data-density", "compact");
    expect(button).toHaveAttribute("data-state", "loading");
    expect(button).toHaveAttribute("aria-invalid", "true");
    expect(button).toBeDisabled();
    dispose();
    host.remove();
  });
});
