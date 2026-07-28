import type { ProjectRecord, SkillRecord } from "@hachimi/contracts";
import { I18nProvider } from "@hachimi/i18n";
import { Show, createMemo, createSignal, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  ComposerContextControls,
  ComposerOptionsPopover,
  SkillReferenceList,
} from "./composer-popovers";

vi.mock("@hachimi/ui", () => {
  const Icon = () => <span aria-hidden="true" />;
  const Button = (props: JSX.ButtonHTMLAttributes<HTMLButtonElement>) => (
    <button {...props}>{props.children}</button>
  );
  const FloatingPopover = (props: {
    open: boolean;
    trigger: JSX.Element;
    children: JSX.Element;
    triggerTestId?: string;
  }) => (
    <div>
      <button type="button" data-testid={props.triggerTestId}>
        {props.trigger}
      </button>
      <Show when={props.open}>{props.children}</Show>
    </div>
  );
  return {
    AlertTriangle: Icon,
    Button,
    Check: Icon,
    ChevronDown: Icon,
    FileText: Icon,
    FloatingPopover,
    FolderOpen: Icon,
    GitBranch: Icon,
    GitFork: Icon,
    Hand: Icon,
    Laptop: Icon,
    Lightbulb: Icon,
    Paperclip: Icon,
    Plus: Icon,
    RefreshCw: Icon,
    ShieldAlert: Icon,
    ShieldCheck: Icon,
    ShieldOff: Icon,
    Sparkles: Icon,
  };
});

const skill: SkillRecord = {
  id: "skill-documents",
  scope: "user",
  namespace: null,
  name: "documents",
  qualifiedName: "documents",
  description: "Create and edit document artifacts",
  dependencies: [],
  editable: true,
  enabled: true,
  contentHash: "content-hash",
  treeRevision: "tree-revision",
  diagnostics: [],
  updatedAtMs: 1,
};

const project: ProjectRecord = {
  id: "project-1",
  displayName: "Demo",
  rootPath: "C:\\demo",
  gitRoot: null,
  trusted: false,
  createdAtMs: 1,
  updatedAtMs: 1,
};

afterEach(() => {
  document.body.replaceChildren();
});

describe("composer Skill references", () => {
  it("adds and removes a structured Skill reference", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const toggle = vi.fn();

    function Harness() {
      const [selectedIds, setSelectedIds] = createSignal<string[]>([]);
      const selectedSkills = createMemo(() => (selectedIds().includes(skill.id) ? [skill] : []));
      const toggleSkill = (skillId: string) => {
        toggle(skillId);
        setSelectedIds((current) => (current.includes(skillId) ? [] : [skillId]));
      };
      return (
        <I18nProvider initialLocale="zh-CN">
          <ComposerOptionsPopover
            activePopover="options"
            onOpenChange={() => undefined}
            behaviorMode="default"
            skills={[skill]}
            skillsLoading={false}
            skillsError={undefined}
            selectedSkillIds={selectedIds()}
            onChooseAttachments={() => undefined}
            onTogglePlanMode={() => undefined}
            onToggleSkill={toggleSkill}
          />
          <SkillReferenceList skills={selectedSkills()} onRemove={toggleSkill} />
        </I18nProvider>
      );
    }

    const dispose = render(() => <Harness />, host);
    await Promise.resolve();
    document.body
      .querySelector<HTMLButtonElement>('[data-testid="workbench-skill-documents"]')!
      .click();
    await Promise.resolve();
    expect(toggle).toHaveBeenCalledWith(skill.id);
    expect(host.textContent).toContain("documents");

    host.querySelector<HTMLButtonElement>(".composer-skill-reference")!.click();
    await Promise.resolve();
    expect(host.querySelector(".composer-skill-reference")).toBeNull();

    dispose();
  });
});

describe("composer project Git state", () => {
  it("shows an unborn branch and keeps Managed Worktree disabled", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const createInitial = vi.fn();
    const dispose = render(
      () => (
        <I18nProvider initialLocale="zh-CN">
          <ComposerContextControls
            activePopover="execution"
            onOpenChange={() => undefined}
            projects={[project]}
            selectedProject={project}
            executionKind="local"
            gitRefs={[]}
            baseRevision=""
            gitSnapshot={{
              projectId: project.id,
              gitRoot: project.rootPath,
              state: { kind: "unborn", branch: "main" },
              observedAtMs: 1,
            }}
            gitLoading={false}
            onSelectProject={() => undefined}
            onSelectExecution={() => undefined}
            onSelectBranch={() => undefined}
            onRefreshGit={() => undefined}
            onCreateInitialCommit={createInitial}
          />
        </I18nProvider>
      ),
      host,
    );
    await Promise.resolve();
    expect(host.textContent).toContain("main · 尚无提交");
    expect(
      host.querySelector<HTMLButtonElement>('[data-testid="workbench-execution-worktree"]')!
        .disabled,
    ).toBe(true);
    host.querySelector<HTMLButtonElement>('[data-testid="project-git-create-initial"]')!.click();
    expect(createInitial).toHaveBeenCalledOnce();
    dispose();
  });
});
