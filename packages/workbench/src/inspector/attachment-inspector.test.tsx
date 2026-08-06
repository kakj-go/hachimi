import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { describe, expect, it, vi } from "vitest";

import type { WorkbenchCommandPort } from "../workbench-command-port";
import { AttachmentInspector } from "./attachment-inspector";

vi.mock("@hachimi/ui", () => ({
  Badge: (props: { children?: JSX.Element }) => <span>{props.children}</span>,
}));

describe("AttachmentInspector", () => {
  it("renders a persisted text attachment in the shared inspector", async () => {
    const commandPort = {
      readWorkbenchAttachment: vi.fn().mockResolvedValue({
        attachment: {
          id: "attachment-1",
          contentHash: "hash",
          originalName: "notes.txt",
          mimeType: "text/plain",
          byteSize: 5,
          createdAtMs: 1,
        },
        utf8Text: "hello",
        dataUrl: null,
        truncated: false,
      }),
    } as unknown as WorkbenchCommandPort;

    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <AttachmentInspector attachmentId="attachment-1" commandPort={commandPort} locale="en-US" />
      ),
      host,
    );
    await Promise.resolve();
    await Promise.resolve();

    expect(host.textContent).toContain("hello");
    expect(commandPort.readWorkbenchAttachment).toHaveBeenCalledWith("attachment-1");
    dispose();
    host.remove();
  });
});
