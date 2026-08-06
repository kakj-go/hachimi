import type { WorkbenchEnvironmentSnapshot } from "@hachimi/contracts";
import type { JSX } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { WorkbenchEnvironmentController } from "../state/workbench-environment-controller";
import { WorkbenchGitControls } from "./workbench-git-controls";

vi.mock("@hachimi/ui", () => {
  const Icon = () => <span aria-hidden="true" />;
  return {
    Badge: (props: { children?: JSX.Element }) => <span>{props.children}</span>,
    Button: (props: JSX.ButtonHTMLAttributes<HTMLButtonElement>) => (
      <button {...props}>{props.children}</button>
    ),
    Checkbox: (props: {
      label: string;
      checked: boolean;
      onChange: JSX.EventHandler<HTMLInputElement, Event>;
    }) => (
      <label>
        <input
          type="checkbox"
          checked={props.checked}
          onChange={(event) => props.onChange(event)}
        />
        {props.label}
      </label>
    ),
    FloatingPopover: (props: { trigger: JSX.Element; children?: JSX.Element }) => (
      <div>
        <button>{props.trigger}</button>
        <div>{props.children}</div>
      </div>
    ),
    Check: Icon,
    GitBranch: Icon,
    GitFork: Icon,
    Upload: Icon,
    SearchField: (props: {
      label: string;
      value: string;
      onInput: JSX.EventHandler<HTMLInputElement, InputEvent>;
    }) => (
      <label>
        {props.label}
        <input value={props.value} onInput={(event) => props.onInput(event)} />
      </label>
    ),
    TextField: (props: {
      label: string;
      value: string;
      onInput: JSX.EventHandler<HTMLInputElement, InputEvent>;
      onKeyDown?: JSX.EventHandler<HTMLInputElement, KeyboardEvent>;
    }) => (
      <label>
        {props.label}
        <input
          value={props.value}
          onInput={(event) => props.onInput(event)}
          onKeyDown={(event) => props.onKeyDown?.(event)}
        />
      </label>
    ),
  };
});

function environment(
  overrides: Partial<WorkbenchEnvironmentSnapshot["git"]> = {},
): WorkbenchEnvironmentSnapshot {
  return {
    sessionId: "session-1",
    checkout: {
      id: "checkout-1",
      projectId: "project-1",
      kind: "local",
      path: "C:/repo",
    },
    workspace: null,
    bindingRevision: 1,
    baselineRevision: "a".repeat(40),
    changes: { changedFiles: 1, additions: 2, deletions: 1, truncated: false },
    git: {
      branch: "main",
      headSha: "a".repeat(40),
      detached: false,
      statusFingerprint: "fingerprint",
      uncommittedFiles: 1,
      upstream: "origin/main",
      ahead: 1,
      behind: 0,
      defaultComparisonRef: "origin/main",
      refs: [
        { name: "main", revision: "a".repeat(40), remote: false, current: true },
        { name: "feature", revision: "b".repeat(40), remote: false, current: false },
      ],
      remotes: [
        {
          name: "origin",
          displayUrl: "https://example.test/repo",
          remoteUrlHash: "c".repeat(64),
          forgeKind: "unknown",
        },
      ],
      ...overrides,
    },
    handoff: {
      localCheckoutId: "checkout-1",
      managedCheckoutId: null,
      canHandoff: true,
      blockedReason: null,
    },
    activity: null,
    sources: [],
    revision: 1,
    generatedAtMs: 1,
  } as unknown as WorkbenchEnvironmentSnapshot;
}

function controller() {
  const executeGit = vi.fn(async () => ({
    stage: { status: "succeeded" as const, message: null },
    commit: { status: "succeeded" as const, message: null },
    head: "b".repeat(40),
    statusFingerprint: "clean",
    branch: "main",
  }));
  const pushGit = vi.fn(async () => ({
    remoteName: "origin",
    remoteUrlHash: "c".repeat(64),
    sourceRef: "HEAD",
    targetRef: "refs/heads/main",
    commitOid: "b".repeat(40),
    confirmed: true,
    resultCode: "ok",
  }));
  return {
    value: { executeGit, pushGit } as unknown as WorkbenchEnvironmentController,
    executeGit,
    pushGit,
  };
}

async function settle() {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

afterEach(() => document.body.replaceChildren());

describe("WorkbenchGitControls", () => {
  it("commits through the environment controller with unstaged changes included", async () => {
    const adapter = controller();
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <WorkbenchGitControls
          environment={environment()}
          controller={adapter.value}
          locale="en-US"
          remotePushEnabled
          onOpenDiff={vi.fn()}
        />
      ),
      host,
    );
    const inputs = host.querySelectorAll<HTMLInputElement>('input:not([type="checkbox"])');
    const message = inputs[2]!;
    message.value = "Update runtime";
    message.dispatchEvent(new InputEvent("input", { bubbles: true }));
    host.querySelector<HTMLButtonElement>('[data-testid="workbench-git-commit"]')?.click();
    await settle();

    expect(adapter.executeGit).toHaveBeenCalledWith(
      { kind: "commit", message: "Update runtime" },
      true,
    );
    dispose();
  });

  it("opens the default branch Diff without executing a Git mutation", () => {
    const adapter = controller();
    const openDiff = vi.fn();
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <WorkbenchGitControls
          environment={environment()}
          controller={adapter.value}
          locale="en-US"
          remotePushEnabled
          onOpenDiff={openDiff}
        />
      ),
      host,
    );
    host.querySelector<HTMLButtonElement>('[data-testid="workbench-git-compare"]')?.click();

    expect(openDiff).toHaveBeenCalledWith("origin/main", ["main", "feature"]);
    expect(adapter.executeGit).not.toHaveBeenCalled();
    dispose();
  });

  it("hides remote actions for detached HEAD", () => {
    const adapter = controller();
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <WorkbenchGitControls
          environment={environment({ branch: null, detached: true })}
          controller={adapter.value}
          locale="en-US"
          remotePushEnabled
          onOpenDiff={vi.fn()}
        />
      ),
      host,
    );

    expect(host.querySelector('[data-testid="workbench-git-push"]')).toBeNull();
    expect(host.querySelector('[data-testid="workbench-git-commit-and-push"]')).toBeNull();
    dispose();
  });
});
