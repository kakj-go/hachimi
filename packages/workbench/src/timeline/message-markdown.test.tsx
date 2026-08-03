import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import { TimelineMessageText, renderMarkdown } from "./message-markdown";

afterEach(() => {
  document.body.replaceChildren();
  vi.unstubAllGlobals();
});

describe("renderMarkdown", () => {
  it("renders GFM and sanitizes raw HTML, scripts, and remote images", () => {
    const html = renderMarkdown(
      "## Result\n\n| Name | State |\n| --- | --- |\n| test | ok |\n\n<script>alert(1)</script>\n\n![remote](https://example.com/a.png)",
    );

    expect(html).toContain("<h2>Result</h2>");
    expect(html).toContain("<table>");
    expect(html).not.toContain("<script");
    expect(html).not.toContain("<img");
  });

  it("keeps workspace links interactive and strips unsafe link protocols", () => {
    const html = renderMarkdown(
      "[local](src/main.rs) [outside](../secret.txt) [unsafe](javascript:alert(1)) [web](https://example.com)",
      "D:/workspace/project",
    );

    expect(html).toContain('data-local-path="src/main.rs"');
    expect(html).not.toContain("../secret.txt");
    expect(html).not.toContain("javascript:");
    expect(html).toContain('rel="noopener noreferrer"');
  });
});

describe("TimelineMessageText", () => {
  it("commits only the latest streamed Markdown frame and routes local links", () => {
    const frames: FrameRequestCallback[] = [];
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      frames.push(callback);
      return frames.length;
    });
    vi.stubGlobal("cancelAnimationFrame", () => undefined);
    const [text, setText] = createSignal("Starting");
    const openPath = vi.fn();
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <TimelineMessageText
          text={text()}
          workspaceRoot="D:/workspace/project"
          onOpenPath={openPath}
        />
      ),
      host,
    );

    setText("**intermediate");
    setText("## Complete\n\n[main](src/main.rs)");
    for (const frame of frames.splice(0)) frame(0);

    expect(host.querySelector("h2")?.textContent).toBe("Complete");
    host.querySelector<HTMLAnchorElement>("a[data-local-path]")!.click();
    expect(openPath).toHaveBeenCalledWith("src/main.rs");
    dispose();
  });
});
