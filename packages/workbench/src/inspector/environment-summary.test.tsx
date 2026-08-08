import type { WorkbenchEnvironmentSnapshot } from "@hachimi/contracts";
import type { JSX } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { WorkbenchEnvironmentController } from "../state/workbench-environment-controller";
import { EnvironmentSummary } from "./environment-summary";

vi.mock("@hachimi/ui", () => {
  const Icon = () => <span aria-hidden="true" />;
  return {
    Box: Icon,
    Check: Icon,
    File: Icon,
    Globe: Icon,
    HardDrive: Icon,
    Laptop: Icon,
    Link2: Icon,
    Lightbulb: Icon,
    Plus: Icon,
    Button: (props: JSX.ButtonHTMLAttributes<HTMLButtonElement>) => (
      <button {...props}>{props.children}</button>
    ),
    FloatingPopover: (props: {
      trigger: JSX.Element;
      triggerTestId?: string;
      children?: JSX.Element;
    }) => (
      <div>
        <button data-testid={props.triggerTestId}>{props.trigger}</button>
        <div>{props.children}</div>
      </div>
    ),
  };
});

vi.mock("../git/workbench-git-controls", () => ({
  WorkbenchGitControls: (props: { onOpenDiff: (branch?: string, branches?: string[]) => void }) => (
    <button
      data-testid="workbench-git-compare"
      onClick={() => props.onOpenDiff("origin/main", ["main", "feature"])}
    >
      Compare branch
    </button>
  ),
}));

function environment(
  activity: WorkbenchEnvironmentSnapshot["activities"][number] = {
    kind: "browser",
    lease_id: "browser-lease-1",
    surface: "embedded",
    browser_tab_id: "browser-tab-1",
    browser_session_id: null,
    run_id: "run-1",
    domain: "docs.example.com",
  },
): WorkbenchEnvironmentSnapshot {
  return {
    sessionId: "session-1",
    checkout: {
      id: "checkout-1",
      projectId: "project-1",
      kind: "local",
      path: "C:/repo",
      baseRevision: null,
      headRevision: "a".repeat(40),
      status: "ready",
      pinned: false,
      createdAtMs: 1,
      updatedAtMs: 1,
    },
    workspace: null,
    bindingRevision: 1,
    baselineRevision: "a".repeat(40),
    changes: { changedFiles: 3, additions: 7, deletions: 2, truncated: false },
    git: {
      branch: "main",
      headSha: "a".repeat(40),
      detached: false,
      statusFingerprint: "status",
      uncommittedFiles: 2,
      upstream: "origin/main",
      ahead: 0,
      behind: 0,
      defaultComparisonRef: "origin/main",
      refs: [],
      remotes: [],
    },
    handoff: {
      localCheckoutId: "checkout-1",
      managedCheckoutId: "checkout-2",
      canHandoff: true,
      blockedReason: null,
    },
    activities: [activity],
    sources: [
      {
        id: "source-upload",
        sessionId: "session-1",
        runId: "run-1",
        kind: "upload",
        origin: "upload",
        attachmentId: "attachment-1",
        url: null,
        title: "design.png",
        browserTabId: null,
        createdAtMs: 1,
        lastUsedAtMs: 2,
      },
      {
        id: "source-web",
        sessionId: "session-1",
        runId: "run-1",
        kind: "web",
        origin: "browser",
        attachmentId: null,
        url: "https://docs.example.com/guide",
        title: "Guide",
        browserTabId: "browser-tab-1",
        createdAtMs: 2,
        lastUsedAtMs: 3,
      },
    ],
    revision: 3,
    generatedAtMs: 4,
  };
}

afterEach(() => document.body.replaceChildren());

describe("EnvironmentSummary", () => {
  it("routes environment, handoff, activity, comparison and source actions", async () => {
    const openInspector = vi.fn();
    const handoff = vi.fn(async () => undefined);
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <EnvironmentSummary
          environment={environment()}
          controller={{} as WorkbenchEnvironmentController}
          locale="en-US"
          remotePushEnabled
          handoffBusy={false}
          handoffFailure={undefined}
          onHandoff={handoff}
          onOpenInspector={openInspector}
        />
      ),
      host,
    );

    host.querySelector<HTMLButtonElement>('[data-testid="workbench-summary-diff"]')?.click();
    host.querySelector<HTMLButtonElement>('[data-testid="workbench-git-compare"]')?.click();
    host
      .querySelector<HTMLButtonElement>('[data-testid="workbench-summary-browser-activity"]')
      ?.click();
    host.querySelector<HTMLButtonElement>('[title="design.png"]')?.click();
    host.querySelector<HTMLButtonElement>('[title="Guide"]')?.click();
    host.querySelector<HTMLButtonElement>('[data-testid="workbench-summary-sources-all"]')?.click();
    const worktree = Array.from(host.querySelectorAll<HTMLButtonElement>("button")).find((button) =>
      button.textContent?.includes("Managed checkout for this session"),
    );
    worktree?.click();
    await Promise.resolve();

    expect(openInspector.mock.calls).toEqual([
      [{ kind: "review", diffScope: "session" }],
      [
        {
          kind: "review",
          diffScope: "branch",
          diffBaseBranch: "origin/main",
          diffBranches: ["main", "feature"],
        },
      ],
      [
        {
          kind: "browser",
          leaseId: "browser-lease-1",
          surface: "embedded",
          browserTabId: "browser-tab-1",
        },
      ],
      [{ kind: "attachment", attachmentId: "attachment-1", name: "design.png" }],
      [
        {
          kind: "browser",
          browserTabId: "browser-tab-1",
          initialUrl: "https://docs.example.com/guide",
        },
      ],
      [{ kind: "sources" }],
    ]);
    expect(handoff).toHaveBeenCalledWith("managed_worktree");
    expect(host.textContent).toContain("Visiting docs.example.com");
    expect(host.textContent).not.toMatch(/pull request|cloud execution|background process/i);
    dispose();
  });

  it("keeps the plan title stable while exposing the active step as context", () => {
    const openInspector = vi.fn();
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <EnvironmentSummary
          environment={environment({
            kind: "plan",
            plan_id: "plan-1",
            revision: 1,
            title: "Branch verification",
            confirmation_status: "accepted",
            execution_run_id: "run-1",
            execution_status: "running",
            current_step: {
              id: "step-2",
              description: "Verify the branch Diff",
              status: "in_progress",
            },
          })}
          controller={{} as WorkbenchEnvironmentController}
          locale="en-US"
          remotePushEnabled={false}
          handoffBusy={false}
          handoffFailure={undefined}
          onHandoff={() => Promise.resolve()}
          onOpenInspector={openInspector}
        />
      ),
      host,
    );

    host
      .querySelector<HTMLButtonElement>('[data-testid="workbench-summary-plan-activity"]')
      ?.click();
    expect(host.textContent).toContain("Branch verification");
    expect(
      host.querySelector('[data-testid="workbench-summary-plan-activity"]')?.getAttribute("title"),
    ).toBe("Verify the branch Diff");
    expect(openInspector).toHaveBeenCalledWith({ kind: "plan", planId: "plan-1" });
    dispose();
  });
});
