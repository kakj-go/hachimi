import type { ProjectRecord, SkillRecord } from "@hachimi/contracts";
import { I18nProvider } from "@hachimi/i18n";
import { Show, createMemo, createSignal, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  ApprovalPolicyPopover,
  ComposerContextControls,
  ComposerOptionsPopover,
  PlanModeChip,
  SkillReferenceList,
} from "./composer-popovers";

vi.mock("@hachimi/ui", () => {
  const Icon = (props: { class?: string }) => <span class={props.class} aria-hidden="true" />;
  const Button = (props: JSX.ButtonHTMLAttributes<HTMLButtonElement>) => (
    <button {...props}>{props.children}</button>
  );
  const FloatingPopover = (props: {
    open: boolean;
    trigger: JSX.Element;
    children: JSX.Element;
    triggerTestId?: string;
    triggerClass?: string;
  }) => (
    <div>
      <button type="button" class={props.triggerClass} data-testid={props.triggerTestId}>
        {props.trigger}
      </button>
      <Show when={props.open}>{props.children}</Show>
    </div>
  );
  const Tooltip = (props: { label: string; children: JSX.Element }) => (
    <span data-tooltip-label={props.label}>{props.children}</span>
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
    Tooltip,
    X: Icon,
  };
});

describe("composer plan mode chip", () => {
  it("exposes a stable icon slot and disables plan mode from the whole chip", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const disable = vi.fn();
    const dispose = render(
      () => (
        <I18nProvider initialLocale="zh-CN">
          <PlanModeChip onDisable={disable} />
        </I18nProvider>
      ),
      host,
    );
    await Promise.resolve();

    const chip = host.querySelector<HTMLButtonElement>('[data-testid="workbench-plan-mode-chip"]')!;
    expect(chip.textContent).toContain("计划");
    expect(chip.getAttribute("aria-label")).toBe("关闭计划模式");
    expect(chip.querySelector(".composer-plan-mode-default-icon")).not.toBeNull();
    expect(chip.querySelector(".composer-plan-mode-remove-icon")).not.toBeNull();
    expect(host.querySelector("[data-tooltip-label='关闭计划模式']")).not.toBeNull();

    chip.click();
    expect(disable).toHaveBeenCalledOnce();
    dispose();
  });
});

describe("composer approval policies", () => {
  it("distinguishes policy risk and updates the trigger treatment", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const change = vi.fn();

    function Harness() {
      const [policy, setPolicy] = createSignal<
        "always_ask_side_effects" | "only_when_needed" | "never_prompt"
      >("only_when_needed");
      return (
        <I18nProvider initialLocale="zh-CN">
          <ApprovalPolicyPopover
            activePopover="approval"
            onOpenChange={() => undefined}
            value={policy()}
            onChange={(value) => {
              change(value);
              setPolicy(value);
            }}
          />
        </I18nProvider>
      );
    }

    const dispose = render(() => <Harness />, host);
    await Promise.resolve();

    const trigger = host.querySelector<HTMLButtonElement>(
      '[data-testid="workbench-approval-policy"]',
    )!;
    const ask = host.querySelector<HTMLButtonElement>(
      '[data-testid="workbench-approval-policy-always_ask_side_effects"]',
    )!;
    const recommended = host.querySelector<HTMLButtonElement>(
      '[data-testid="workbench-approval-policy-only_when_needed"]',
    )!;
    const fullAccess = host.querySelector<HTMLButtonElement>(
      '[data-testid="workbench-approval-policy-never_prompt"]',
    )!;

    expect(ask.dataset.tone).toBe("neutral");
    expect(recommended.dataset.tone).toBe("recommended");
    expect(recommended.getAttribute("aria-pressed")).toBe("true");
    expect(fullAccess.dataset.tone).toBe("danger");
    expect(trigger.classList.contains("policy-recommended")).toBe(true);

    ask.click();
    await Promise.resolve();
    expect(change).toHaveBeenLastCalledWith("always_ask_side_effects");
    expect(ask.getAttribute("aria-pressed")).toBe("true");
    expect(trigger.classList.contains("policy-neutral")).toBe(true);

    fullAccess.click();
    await Promise.resolve();
    expect(change).toHaveBeenLastCalledWith("never_prompt");
    expect(fullAccess.getAttribute("aria-pressed")).toBe("true");
    expect(trigger.classList.contains("policy-danger")).toBe(true);
    expect(host.textContent).toContain("高风险操作可能直接执行");

    dispose();
  });
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

  it("shows only the base branch control for a Managed Worktree", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const refresh = vi.fn();
    const dispose = render(
      () => (
        <I18nProvider initialLocale="zh-CN">
          <ComposerContextControls
            activePopover="branch"
            onOpenChange={() => undefined}
            projects={[project]}
            selectedProject={project}
            executionKind="managed_worktree"
            gitRefs={[
              {
                name: "main",
                revision: "1234567890abcdef",
                current: true,
                remote: false,
              },
            ]}
            baseRevision="main"
            gitSnapshot={{
              projectId: project.id,
              gitRoot: project.rootPath,
              state: { kind: "ready", branch: "main", head: "1234567890abcdef" },
              observedAtMs: 1,
            }}
            gitLoading={false}
            onSelectProject={() => undefined}
            onSelectExecution={() => undefined}
            onSelectBranch={() => undefined}
            onRefreshGit={refresh}
            onCreateInitialCommit={() => undefined}
          />
        </I18nProvider>
      ),
      host,
    );
    await Promise.resolve();
    expect(host.querySelector('[data-testid="workbench-project-git-state"]')).toBeNull();
    expect(host.querySelector('[data-testid="workbench-base-branch"]')).not.toBeNull();
    expect(host.textContent).toContain("刷新分支");
    host.querySelector<HTMLButtonElement>(".composer-branch-refresh")!.click();
    expect(refresh).toHaveBeenCalledOnce();
    dispose();
  });
});
