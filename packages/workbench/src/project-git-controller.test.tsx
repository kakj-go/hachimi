import type { ProjectGitSnapshot, ProjectRecord } from "@hachimi/contracts";
import { createProjectGitController } from "./project-git-controller";
import type { WorkbenchCommandPort } from "./workbench-command-port";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@hachimi/ui", () => ({
  Button: (props: Record<string, unknown>) => (
    <button
      type={(props.type as "button" | "submit" | undefined) ?? "button"}
      data-testid={props["data-testid"] as string | undefined}
      disabled={props.disabled as boolean | undefined}
      onClick={(event) =>
        (props.onClick as ((event: MouseEvent) => void) | undefined)?.(event as MouseEvent)
      }
    >
      {props.children as never}
    </button>
  ),
  Dialog: (props: Record<string, unknown>) => (
    <div role="dialog" hidden={!props.open}>
      {props.children as never}
    </div>
  ),
  TextField: (props: Record<string, unknown>) => (
    <label>
      {props.label as never}
      <input
        value={props.value as string}
        onInput={(event) =>
          (props.onInput as ((event: InputEvent) => void) | undefined)?.(event as InputEvent)
        }
      />
    </label>
  ),
}));

const project: ProjectRecord = {
  id: "project-1",
  displayName: "Fixture",
  rootPath: "C:\\fixture",
  gitRoot: null,
  trusted: true,
  createdAtMs: 1,
  updatedAtMs: 1,
};

const unborn: ProjectGitSnapshot = {
  projectId: project.id,
  gitRoot: project.rootPath,
  state: { kind: "unborn", branch: "main" },
  observedAtMs: 2,
};

const ready: ProjectGitSnapshot = {
  projectId: project.id,
  gitRoot: project.rootPath,
  state: { kind: "ready", branch: "main", head: "0123456789abcdef" },
  observedAtMs: 3,
};

async function settle() {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

describe("createProjectGitController", () => {
  it("does not restart inspection when reconciled Project metadata changes identity", async () => {
    const port = {
      inspectProjectGit: vi.fn(async () => ready),
      listProjectGitRefs: vi.fn(async () => [
        {
          name: "main",
          revision: ready.state.kind === "ready" ? ready.state.head : "",
          remote: false,
          current: true,
        },
      ]),
    } as unknown as WorkbenchCommandPort;
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => {
      const [selectedProject, setSelectedProject] = createSignal<ProjectRecord | undefined>(
        project,
      );
      createProjectGitController({
        commandPort: port,
        selectedProject,
        onProjectReconciled: (_projectId, gitRoot) =>
          setSelectedProject((current) => (current ? { ...current, gitRoot } : current)),
        onFailure: vi.fn(),
      });
      return <div />;
    }, root);

    await settle();
    await settle();
    expect(port.inspectProjectGit).toHaveBeenCalledTimes(1);
    dispose();
  });

  it("reconciles an unborn repository and immediately exposes its base branch after refresh", async () => {
    const port = {
      inspectProjectGit: vi.fn(async () => unborn),
      refreshProjectGit: vi.fn(async () => ready),
      listProjectGitRefs: vi.fn(async () => [
        {
          name: "main",
          revision: ready.state.kind === "ready" ? ready.state.head : "",
          remote: false,
          current: true,
        },
      ]),
    } as unknown as WorkbenchCommandPort;
    const reconciled = vi.fn();
    const failure = vi.fn();
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => {
      const [selectedProject] = createSignal<ProjectRecord | undefined>(project);
      const controller = createProjectGitController({
        commandPort: port,
        selectedProject,
        onProjectReconciled: reconciled,
        onFailure: failure,
      });
      return (
        <div>
          <span data-testid="kind">{controller.snapshot()?.state.kind ?? "loading"}</span>
          <span data-testid="base">{controller.baseRevision()}</span>
          <button type="button" onClick={controller.refresh}>
            refresh
          </button>
        </div>
      );
    }, root);

    await settle();
    expect(root.querySelector('[data-testid="kind"]')?.textContent).toBe("unborn");
    expect(root.querySelector('[data-testid="base"]')?.textContent).toBe("");
    expect(reconciled).toHaveBeenCalledWith(project.id, project.rootPath);

    root.querySelector("button")?.click();
    await settle();
    expect(port.refreshProjectGit).toHaveBeenCalledWith(project.id);
    expect(port.listProjectGitRefs).toHaveBeenCalledWith(project.id);
    expect(root.querySelector('[data-testid="kind"]')?.textContent).toBe("ready");
    expect(root.querySelector('[data-testid="base"]')?.textContent).toBe("main");
    expect(failure).not.toHaveBeenCalled();
    dispose();
  });

  it("keeps the verified base branch when starting another draft for the same project", async () => {
    const port = {
      inspectProjectGit: vi.fn(async () => ready),
      listProjectGitRefs: vi.fn(async () => [
        {
          name: "main",
          revision: ready.state.kind === "ready" ? ready.state.head : "",
          remote: false,
          current: true,
        },
      ]),
    } as unknown as WorkbenchCommandPort;
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => {
      const [selectedProject] = createSignal<ProjectRecord | undefined>(project);
      const controller = createProjectGitController({
        commandPort: port,
        selectedProject,
        onProjectReconciled: vi.fn(),
        onFailure: vi.fn(),
      });
      return (
        <div>
          <span data-testid="kind">{controller.executionKind()}</span>
          <span data-testid="base">{controller.baseRevision()}</span>
          <button
            type="button"
            onClick={() => {
              controller.setExecutionKind("managed_worktree");
              controller.resetForDraft();
            }}
          >
            reset
          </button>
        </div>
      );
    }, root);

    await settle();
    expect(root.querySelector('[data-testid="base"]')?.textContent).toBe("main");

    root.querySelector("button")?.click();
    expect(root.querySelector('[data-testid="kind"]')?.textContent).toBe("local");
    expect(root.querySelector('[data-testid="base"]')?.textContent).toBe("main");
    expect(port.inspectProjectGit).toHaveBeenCalledTimes(1);
    dispose();
  });
});
