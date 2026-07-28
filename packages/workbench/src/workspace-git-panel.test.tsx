import type { GitWorkspaceSnapshot, WorkbenchSessionSnapshot } from "@hachimi/contracts";
import { I18nProvider } from "@hachimi/i18n";
import type { JSX } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { WorkbenchCommandPort } from "./workbench-command-port";
import { WorkspaceGitPanel } from "./workspace-git-panel";

vi.mock("@hachimi/ui", () => {
  const Icon = () => <span aria-hidden="true" />;
  return {
    Badge: (props: { children?: JSX.Element }) => <span>{props.children}</span>,
    Button: (props: JSX.ButtonHTMLAttributes<HTMLButtonElement>) => (
      <button {...props}>{props.children}</button>
    ),
    Check: Icon,
    GitBranch: Icon,
    RefreshCw: Icon,
    TextField: (props: {
      label: string;
      value: string;
      placeholder?: string;
      onInput?: JSX.EventHandler<HTMLInputElement, InputEvent>;
    }) => (
      <label>
        {props.label}
        <input
          value={props.value}
          placeholder={props.placeholder}
          onInput={(event) => props.onInput?.(event)}
        />
      </label>
    ),
  };
});

function session(): WorkbenchSessionSnapshot {
  return {
    session: {
      id: "session-1",
      context: { kind: "project", project_id: "project-1", checkout_id: "checkout-1" },
    },
    runs: [{ id: "run-1", generation: 5, status: "succeeded" }],
    events: [],
    transcript: [],
    pendingApprovals: [],
    proposedPlans: [],
    artifacts: [],
  } as unknown as WorkbenchSessionSnapshot;
}

function workingTree(indexStatus = " ", worktreeStatus = "M"): GitWorkspaceSnapshot {
  return {
    branch: "main",
    headSha: "a".repeat(40),
    detached: false,
    status: [
      {
        indexStatus,
        worktreeStatus,
        path: "src/lib.rs",
        previousPath: null,
      },
    ],
    recentCommits: [
      {
        sha: "a".repeat(40),
        abbreviatedSha: "aaaaaaaa",
        subject: "initial",
        authorName: "Hachimi Test",
        committedAtMs: 1,
      },
    ],
  };
}

function createPort() {
  const port = {
    getWorkspaceGit: vi.fn(async () => workingTree()),
    mutateWorkspaceGit: vi
      .fn()
      .mockResolvedValueOnce({ snapshot: workingTree("M", " "), commitSha: null })
      .mockResolvedValueOnce({
        snapshot: { ...workingTree(), status: [], recentCommits: [] },
        commitSha: "b".repeat(40),
      }),
  } as unknown as WorkbenchCommandPort;
  return port;
}

async function settle() {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

afterEach(() => document.body.replaceChildren());

describe("WorkspaceGitPanel", () => {
  it("uses fixed typed stage and local commit mutations", async () => {
    const port = createPort();
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="en-US">
          <WorkspaceGitPanel snapshot={session()} commandPort={port} revision={0} />
        </I18nProvider>
      ),
      host,
    );
    await settle();
    expect(host.textContent).toContain("main");
    expect(host.textContent).toContain("initial");
    [...host.querySelectorAll("button")]
      .find((button) => button.textContent?.trim() === "Stage")
      ?.click();
    await settle();
    expect(port.mutateWorkspaceGit).toHaveBeenNthCalledWith(
      1,
      expect.objectContaining({
        sessionId: "session-1",
        checkoutId: "checkout-1",
        mutation: { kind: "stage", paths: ["src/lib.rs"] },
        context: expect.objectContaining({ expectedRunId: "run-1", expectedGeneration: 5 }),
      }),
    );

    const input = host.querySelector<HTMLInputElement>(
      'input[placeholder="Describe the local changes"]',
    )!;
    input.value = "local commit";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    host.querySelector<HTMLButtonElement>('[data-testid="workspace-git-commit"]')?.click();
    await settle();
    expect(port.mutateWorkspaceGit).toHaveBeenNthCalledWith(
      2,
      expect.objectContaining({ mutation: { kind: "commit", message: "local commit" } }),
    );
    expect(host.textContent).toContain("Created local commit bbbbbbbb");
    dispose();
  });
});
