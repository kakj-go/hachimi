import type {
  FsChangeEvent,
  FsListPage,
  RunDiffSnapshot,
  WorkbenchSessionSnapshot,
} from "@hachimi/contracts";
import { I18nProvider } from "@hachimi/i18n";
import type { JSX } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { WorkbenchCommandPort } from "./workbench-command-port";
import { WorkspaceBrowser } from "./workspace-browser";

vi.mock("@hachimi/ui", () => {
  const Icon = () => <span aria-hidden="true" />;
  const Button = (props: JSX.ButtonHTMLAttributes<HTMLButtonElement>) => (
    <button {...props}>{props.children}</button>
  );
  return {
    AlertTriangle: Icon,
    Button,
    Check: Icon,
    ComposerInput: (props: JSX.TextareaHTMLAttributes<HTMLTextAreaElement> & { label: string }) => (
      <textarea {...props} aria-label={props.label} />
    ),
    ChevronDown: Icon,
    File: Icon,
    FolderOpen: Icon,
    GitBranch: Icon,
    GitFork: Icon,
    RefreshCw: Icon,
    Search: Icon,
    SearchField: (props: Record<string, unknown>) => (
      <label>
        <input
          value={props.value as string}
          placeholder={props.placeholder as string}
          onInput={(event) => (props.onInput as ((event: InputEvent) => void) | undefined)?.(event)}
        />
      </label>
    ),
    X: Icon,
  };
});

type ChangeHandler = (event: FsChangeEvent) => void;

function snapshot(): WorkbenchSessionSnapshot {
  return {
    session: {
      id: "session-1",
      context: { kind: "project", project_id: "project-1", checkout_id: "checkout-1" },
    },
    runs: [{ id: "run-1", generation: 4, status: "succeeded" }],
    events: [],
    transcript: [],
    pendingApprovals: [],
    proposedPlans: [],
    artifacts: [],
  } as unknown as WorkbenchSessionSnapshot;
}

function filePage(version = 1): FsListPage {
  return {
    path: "",
    entries: [
      {
        path: `src/version-${version}.rs`,
        name: `version-${version}.rs`,
        kind: "file",
        byteSize: 12,
        modifiedAtMs: 1,
        hidden: false,
        hasChildren: false,
        gitStatus: version > 1 ? "modified" : null,
      },
    ],
    nextCursor: null,
    etag: `etag-${version}`,
  };
}

function diffSnapshot(additions = 1): RunDiffSnapshot {
  return {
    scope: { kind: "run", run_id: "run-1" },
    files: [
      {
        path: "src/lib.rs",
        previousPath: null,
        status: "modified",
        additions,
        deletions: 0,
        binary: false,
        tooLarge: false,
        hunks: [
          {
            header: "@@ -1 +1 @@",
            lines: [{ kind: "addition", oldLine: null, newLine: 1, text: `version ${additions}` }],
          },
        ],
      },
    ],
    artifactId: null,
    truncated: false,
    generatedAtMs: additions,
  };
}

function createPort() {
  let changeHandler: ChangeHandler | undefined;
  let listVersion = 0;
  const port = {
    listWorkspaceFiles: vi.fn(async () => filePage(++listVersion)),
    readWorkspaceFileChunk: vi.fn(),
    writeWorkspaceFile: vi.fn(async () => ({
      path: "src/version-1.rs",
      byteSize: 12,
      etag: `sha256:${"b".repeat(64)}`,
    })),
    watchWorkspaceFiles: vi.fn(async () => ({
      id: "watch-1",
      sessionId: "session-1",
      checkoutId: "checkout-1",
      path: "",
      generation: 7,
    })),
    unwatchWorkspaceFiles: vi.fn(async () => true),
    startWorkspaceFileSearch: vi.fn(),
    updateWorkspaceFileSearch: vi.fn(),
    cancelWorkspaceFileSearch: vi.fn(async () => true),
    getWorkspaceDiff: vi.fn(async () => diffSnapshot()),
    getWorkspaceGit: vi.fn(async () => ({
      branch: "main",
      headSha: "a".repeat(40),
      detached: false,
      status: [],
      recentCommits: [],
    })),
    mutateWorkspaceGit: vi.fn(),
    readWorkspaceDiffFile: vi.fn(),
    onWorkspaceChange: vi.fn(async (handler: ChangeHandler) => {
      changeHandler = handler;
      return () => {
        changeHandler = undefined;
      };
    }),
  } as unknown as WorkbenchCommandPort;
  return {
    port,
    handler: () => changeHandler,
  };
}

async function settle() {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

function mount(port: WorkbenchCommandPort, mode: "files" | "review" = "files") {
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(
    () => (
      <I18nProvider initialLocale="en-US">
        <WorkspaceBrowser mode={mode} snapshot={snapshot()} commandPort={port} />
      </I18nProvider>
    ),
    host,
  );
  return { host, dispose };
}

afterEach(() => {
  vi.useRealTimers();
  document.body.replaceChildren();
});

describe("WorkspaceBrowser command adapter", () => {
  it("rejects stale Watch generations and reloads current invalidations", async () => {
    const adapter = createPort();
    const { host, dispose } = mount(adapter.port);
    await settle();
    const list = vi.mocked(adapter.port.listWorkspaceFiles);
    expect(list).toHaveBeenCalledTimes(1);
    expect(host.textContent).toContain("version-1.rs");
    expect(host.textContent).not.toContain("Search");
    expect(host.textContent).not.toContain("Git");

    adapter.handler()?.({
      watchId: "watch-1",
      generation: 6,
      kind: "invalidated",
      paths: [],
      overflowed: true,
    });
    await settle();
    expect(list).toHaveBeenCalledTimes(1);

    adapter.handler()?.({
      watchId: "watch-1",
      generation: 7,
      kind: "invalidated",
      paths: [],
      overflowed: true,
    });
    await settle();
    expect(list).toHaveBeenCalledTimes(2);
    expect(host.textContent).toContain("Changes invalidated; reloaded");
    expect(host.textContent).toContain("version-2.rs");

    dispose();
    await settle();
    expect(adapter.port.unwatchWorkspaceFiles).toHaveBeenCalledWith("watch-1");
  });

  it("recomputes Diff only for the current Watch generation", async () => {
    const adapter = createPort();
    vi.mocked(adapter.port.getWorkspaceDiff)
      .mockResolvedValueOnce(diffSnapshot(1))
      .mockResolvedValueOnce(diffSnapshot(2));
    const { host, dispose } = mount(adapter.port, "review");
    await settle();
    expect(host.textContent).toContain("+1 −0");

    adapter.handler()?.({
      watchId: "watch-old",
      generation: 7,
      kind: "modified",
      paths: ["src/lib.rs"],
      overflowed: false,
    });
    await settle();
    expect(adapter.port.getWorkspaceDiff).toHaveBeenCalledTimes(1);

    adapter.handler()?.({
      watchId: "watch-1",
      generation: 7,
      kind: "modified",
      paths: ["src/lib.rs"],
      overflowed: false,
    });
    await settle();
    expect(adapter.port.getWorkspaceDiff).toHaveBeenCalledTimes(2);
    expect(host.textContent).toContain("+2 −0");
    dispose();
  });

  it("reports an empty file chunk response without throwing a bridge TypeError", async () => {
    const adapter = createPort();
    vi.mocked(adapter.port.readWorkspaceFileChunk).mockResolvedValue(null as never);
    const { host, dispose } = mount(adapter.port);
    await settle();
    [...host.querySelectorAll("button")]
      .find((button) => button.textContent?.includes("version-1.rs"))
      ?.click();
    await settle();
    expect(host.textContent).toContain("Workspace file read returned an empty response.");
    dispose();
  });

  it("loads an oversized Run Diff through the bounded diff.read_file adapter", async () => {
    const adapter = createPort();
    const large = diffSnapshot();
    large.files[0]!.tooLarge = true;
    large.files[0]!.hunks = [];
    large.artifactId = "artifact-1";
    large.truncated = true;
    vi.mocked(adapter.port.getWorkspaceDiff).mockResolvedValue(large);
    vi.mocked(adapter.port.readWorkspaceDiffFile).mockResolvedValue({
      scope: large.scope,
      path: "src/lib.rs",
      offset: 0,
      nextOffset: 21,
      byteSize: 21,
      eof: true,
      dataBase64: "",
      utf8Text: "@@ -1 +1 @@\n+bounded\n",
      etag: "sha256:diff",
    });
    const { host, dispose } = mount(adapter.port, "review");
    await settle();
    [...host.querySelectorAll("button")]
      .find((button) => button.textContent?.includes("Load full Diff"))
      ?.click();
    await settle();
    expect(host.textContent).toContain("+bounded");
    expect(adapter.port.readWorkspaceDiffFile).toHaveBeenCalledWith(
      expect.objectContaining({ path: "src/lib.rs", offset: 0, limit: 256 * 1024 }),
    );
    dispose();
  });

  it("saves a fully loaded UTF-8 file with the authoritative ETag", async () => {
    const adapter = createPort();
    vi.mocked(adapter.port.readWorkspaceFileChunk).mockResolvedValue({
      path: "src/version-1.rs",
      offset: 0,
      nextOffset: 10,
      byteSize: 10,
      eof: true,
      binary: false,
      dataBase64: "",
      utf8Text: "version 1\n",
      etag: `sha256:${"a".repeat(64)}`,
    });
    const { host, dispose } = mount(adapter.port);
    await settle();
    [...host.querySelectorAll("button")]
      .find((button) => button.textContent?.includes("version-1.rs"))
      ?.click();
    await settle();

    const editor = host.querySelector<HTMLTextAreaElement>(
      '[data-testid="workspace-editor-fallback"]',
    )!;
    expect(editor.value).toBe("version 1\n");
    editor.value = "version 2\n";
    editor.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await settle();
    host.querySelector<HTMLButtonElement>('[data-testid="workspace-save-file"]')?.click();
    await settle();

    expect(adapter.port.writeWorkspaceFile).toHaveBeenCalledWith(
      expect.objectContaining({
        sessionId: "session-1",
        checkoutId: "checkout-1",
        path: "src/version-1.rs",
        content: "version 2\n",
        ifMatch: `sha256:${"a".repeat(64)}`,
        context: expect.objectContaining({ expectedRunId: "run-1", expectedGeneration: 4 }),
      }),
    );
    expect(host.textContent).not.toContain("Unsaved");
    dispose();
  });

  it("requires an explicit choice after an ETag conflict", async () => {
    const adapter = createPort();
    vi.mocked(adapter.port.readWorkspaceFileChunk).mockResolvedValue({
      path: "src/version-1.rs",
      offset: 0,
      nextOffset: 10,
      byteSize: 10,
      eof: true,
      binary: false,
      dataBase64: "",
      utf8Text: "version 1\n",
      etag: `sha256:${"a".repeat(64)}`,
    });
    vi.mocked(adapter.port.writeWorkspaceFile).mockRejectedValue({
      code: "workspace_conflict",
      message: "file changed after it was read",
    });
    const { host, dispose } = mount(adapter.port);
    await settle();
    [...host.querySelectorAll("button")]
      .find((button) => button.textContent?.includes("version-1.rs"))
      ?.click();
    await settle();
    const editor = host.querySelector<HTMLTextAreaElement>(
      '[data-testid="workspace-editor-fallback"]',
    )!;
    editor.value = "local draft\n";
    editor.dispatchEvent(new InputEvent("input", { bubbles: true }));
    host.querySelector<HTMLButtonElement>('[data-testid="workspace-save-file"]')?.click();
    await settle();
    expect(host.querySelector('[role="alert"]')?.textContent).toContain(
      "changed after it was read",
    );
    expect(
      host.querySelector<HTMLButtonElement>('[data-testid="workspace-save-file"]')?.disabled,
    ).toBe(true);
    dispose();
  });
});
