import type { WorkbenchSessionSnapshot } from "@hachimi/contracts";
import type { JSX } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { WorkbenchCommandPort } from "../workbench-command-port";
import { ComputerInspector } from "./computer-inspector";

vi.mock("@hachimi/ui", () => {
  const Icon = () => <span aria-hidden="true" />;
  return {
    Badge: (props: { children?: JSX.Element }) => <span>{props.children}</span>,
    Button: (props: JSX.ButtonHTMLAttributes<HTMLButtonElement>) => (
      <button {...props}>{props.children}</button>
    ),
    Hand: Icon,
    Monitor: Icon,
    Play: Icon,
    Square: Icon,
  };
});

afterEach(() => {
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

describe("ComputerInspector", () => {
  it("renders the bounded in-memory frame and clears it when the user takes over", async () => {
    const control = {
      id: "computer-control-1",
      ownerSessionId: "session-1",
      ownerRunId: "run-1",
      runGeneration: 1,
      app: {
        appId: "notepad.exe",
        displayName: "Notepad",
        executablePath: null,
        publisher: null,
        identityHash: "app-hash",
      },
      window: { title: "Notes" },
      latestFrame: {
        id: "frame-1",
        sessionId: "session-1",
        width: 800,
        height: 600,
      },
      status: "active",
      revision: 1,
      updatedAtMs: 1,
    };
    const takeOverComputerControl = vi.fn(async () => ({
      ...control,
      latestFrame: null,
      status: "suspended" as const,
      revision: 2,
    }));
    const commandPort = {
      getComputerControlFrame: vi.fn(async () => ({
        frameId: "frame-1",
        mediaType: "image/png",
        dataBase64: "iVBORw0KGgo=",
        sha256: "frame-hash",
        expiresAtMs: Date.now() + 30_000,
      })),
      takeOverComputerControl,
      resumeComputerControl: vi.fn(),
      stopComputerControl: vi.fn(),
    } as unknown as WorkbenchCommandPort;
    const snapshot = {
      computerControlSessions: [control],
    } as unknown as WorkbenchSessionSnapshot;
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => <ComputerInspector snapshot={snapshot} commandPort={commandPort} locale="en-US" />,
      host,
    );

    await vi.waitFor(() =>
      expect(
        host.querySelector('[data-testid="computer-frame-preview"]')?.getAttribute("src"),
      ).toContain("iVBOR"),
    );
    expect(host.textContent).toContain("Notepad");
    const takeOver = [...host.querySelectorAll<HTMLButtonElement>("button")].find((button) =>
      button.textContent?.includes("Take over"),
    );
    takeOver?.click();
    await vi.waitFor(() => expect(takeOverComputerControl).toHaveBeenCalledWith("session-1"));
    expect(host.querySelector('[data-testid="computer-frame-preview"]')).toBeNull();
    dispose();
  });
});
