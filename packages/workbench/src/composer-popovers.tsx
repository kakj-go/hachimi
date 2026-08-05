import type {
  ApprovalPolicy,
  GitRefRecord,
  ProjectGitSnapshot,
  ProjectRecord,
  SkillRecord,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import {
  AlertTriangle,
  Button,
  Check,
  ChevronDown,
  FileText,
  FloatingPopover,
  FolderOpen,
  GitBranch,
  GitFork,
  Hand,
  Laptop,
  Lightbulb,
  Paperclip,
  Plus,
  RefreshCw,
  ShieldCheck,
  ShieldOff,
  Sparkles,
  Tooltip,
  X,
} from "@hachimi/ui";
import { For, Show, type JSX } from "solid-js";
import { skillDisplayName } from "./skill-display";

export type ComposerPopoverId = "project" | "execution" | "branch" | "options" | "approval";

interface PopoverStateProps {
  activePopover: ComposerPopoverId | undefined;
  onOpenChange: (id: ComposerPopoverId, open: boolean) => void;
}

type MenuRowTone = "neutral" | "recommended" | "danger";

function MenuRow(props: {
  icon: JSX.Element;
  label: string;
  description?: string;
  selected?: boolean;
  disabled?: boolean;
  tone?: MenuRowTone;
  testId?: string;
  onSelect: () => void;
}) {
  return (
    <Button
      type="button"
      class="composer-popover-row"
      classList={{ selected: props.selected }}
      data-tone={props.tone ?? "neutral"}
      data-testid={props.testId}
      disabled={props.disabled ?? false}
      aria-pressed={props.selected ?? false}
      onClick={() => props.onSelect()}
    >
      <span class="composer-popover-row-icon" aria-hidden="true">
        {props.icon}
      </span>
      <span class="composer-popover-row-copy">
        <strong>{props.label}</strong>
        <Show when={props.description}>
          <small>{props.description}</small>
        </Show>
      </span>
      <span class="composer-popover-row-check" aria-hidden="true">
        <Show when={props.selected}>
          <Check size={17} />
        </Show>
      </span>
    </Button>
  );
}

function ContextTrigger(props: { icon: JSX.Element; label: string }) {
  return (
    <>
      {props.icon}
      <span>{props.label}</span>
      <ChevronDown class="composer-trigger-chevron" size={13} aria-hidden="true" />
    </>
  );
}

export function ComposerContextControls(
  props: PopoverStateProps & {
    projects: ProjectRecord[];
    selectedProject: ProjectRecord | undefined;
    executionKind: "local" | "managed_worktree";
    gitRefs: GitRefRecord[];
    baseRevision: string;
    gitSnapshot: ProjectGitSnapshot | undefined;
    gitLoading: boolean;
    onSelectProject: (projectId: string) => void;
    onSelectExecution: (kind: "local" | "managed_worktree") => void;
    onSelectBranch: (revision: string) => void;
    onRefreshGit: () => void;
    onCreateInitialCommit: () => void;
  },
) {
  const i18n = useI18n();
  const copy = (zh: string, en: string) => (i18n.locale() === "zh-CN" ? zh : en);
  const worktreeAvailable = () => props.gitSnapshot?.state.kind === "ready";
  const gitLabel = () => {
    const state = props.gitSnapshot?.state;
    if (!state) return copy("正在检查 Git…", "Inspecting Git…");
    if (state.kind === "not_repository") return copy("非 Git 项目", "Not a Git project");
    if (state.kind === "unborn")
      return copy(`${state.branch} · 尚无提交`, `${state.branch} · No commits`);
    if (state.kind === "detached") return `detached · ${state.head.slice(0, 8)}`;
    if (state.kind === "unavailable") return copy("Git 状态不可用", "Git unavailable");
    return state.branch ?? state.head.slice(0, 8);
  };

  return (
    <div class="composer-context composer-context-row">
      <FloatingPopover
        open={props.activePopover === "project"}
        onOpenChange={(open) => props.onOpenChange("project", open)}
        label={i18n.t("workbench.project")}
        disabled={props.projects.length === 0}
        triggerClass="composer-control-trigger"
        triggerTestId="workbench-project-trigger"
        contentClass="composer-popover composer-project-popover"
        contentTestId="workbench-project-popover"
        trigger={
          <ContextTrigger
            icon={<FolderOpen size={16} />}
            label={props.selectedProject?.displayName ?? i18n.t("workbench.project")}
          />
        }
      >
        <div class="composer-popover-heading">{i18n.t("workbench.projects")}</div>
        <div class="composer-popover-list" role="listbox" aria-label={i18n.t("workbench.projects")}>
          <For each={props.projects}>
            {(project) => (
              <MenuRow
                icon={<FolderOpen size={18} />}
                label={project.displayName}
                description={project.rootPath}
                selected={project.id === props.selectedProject?.id}
                onSelect={() => {
                  props.onSelectProject(project.id);
                  props.onOpenChange("project", false);
                }}
              />
            )}
          </For>
        </div>
      </FloatingPopover>

      <FloatingPopover
        open={props.activePopover === "execution"}
        onOpenChange={(open) => props.onOpenChange("execution", open)}
        label={i18n.t("workbench.executionTarget")}
        triggerClass="composer-control-trigger"
        triggerTestId="workbench-execution-target"
        contentClass="composer-popover composer-execution-popover"
        contentTestId="workbench-execution-popover"
        trigger={
          <ContextTrigger
            icon={props.executionKind === "local" ? <Laptop size={16} /> : <GitFork size={16} />}
            label={
              props.executionKind === "local"
                ? i18n.t("workbench.executionLocal")
                : i18n.t("workbench.executionWorktree")
            }
          />
        }
      >
        <div class="composer-popover-heading">{i18n.t("workbench.executionPopoverTitle")}</div>
        <div class="composer-popover-list" role="listbox">
          <MenuRow
            icon={<Laptop size={18} />}
            label={i18n.t("workbench.executionLocal")}
            description={i18n.t("workbench.executionLocalDescription")}
            selected={props.executionKind === "local"}
            testId="workbench-execution-local"
            onSelect={() => {
              props.onSelectExecution("local");
              props.onOpenChange("execution", false);
            }}
          />
          <MenuRow
            icon={<GitFork size={18} />}
            label={i18n.t("workbench.executionWorktree")}
            description={i18n.t("workbench.executionWorktreeDescription")}
            selected={props.executionKind === "managed_worktree"}
            disabled={!worktreeAvailable()}
            testId="workbench-execution-worktree"
            onSelect={() => {
              props.onSelectExecution("managed_worktree");
              props.onOpenChange("execution", false);
            }}
          />
        </div>
      </FloatingPopover>

      <Show when={props.executionKind === "local"}>
        <div class="composer-git-state" data-testid="workbench-project-git-state">
          <GitBranch size={15} aria-hidden="true" />
          <span>{gitLabel()}</span>
          <Show when={props.gitSnapshot?.state.kind === "unborn"}>
            <Button
              type="button"
              variant="ghost"
              size="small"
              data-testid="project-git-create-initial"
              onClick={props.onCreateInitialCommit}
            >
              {copy("创建首提", "Create initial commit")}
            </Button>
          </Show>
          <Button
            type="button"
            variant="ghost"
            size="small"
            aria-label={copy("刷新 Git 状态", "Refresh Git status")}
            disabled={props.gitLoading || !props.selectedProject}
            onClick={props.onRefreshGit}
          >
            <RefreshCw size={14} classList={{ "is-spinning": props.gitLoading }} />
          </Button>
        </div>
      </Show>

      <Show when={props.executionKind === "managed_worktree"}>
        <FloatingPopover
          open={props.activePopover === "branch"}
          onOpenChange={(open) => props.onOpenChange("branch", open)}
          label={i18n.t("workbench.baseBranch")}
          disabled={props.gitRefs.length === 0}
          triggerClass="composer-control-trigger"
          triggerTestId="workbench-base-branch"
          contentClass="composer-popover composer-branch-popover"
          contentTestId="workbench-branch-popover"
          trigger={
            <ContextTrigger
              icon={<GitBranch size={16} />}
              label={props.baseRevision || i18n.t("workbench.baseBranch")}
            />
          }
        >
          <div class="composer-popover-heading">{i18n.t("workbench.branchPopoverTitle")}</div>
          <Button
            type="button"
            variant="ghost"
            size="small"
            class="composer-branch-refresh"
            disabled={props.gitLoading || !props.selectedProject}
            onClick={props.onRefreshGit}
          >
            <RefreshCw size={14} classList={{ "is-spinning": props.gitLoading }} />
            {copy("刷新分支", "Refresh branches")}
          </Button>
          <div class="composer-popover-list composer-branch-list" role="listbox">
            <For each={props.gitRefs}>
              {(entry) => (
                <MenuRow
                  icon={<GitBranch size={18} />}
                  label={entry.name}
                  description={`${entry.revision.slice(0, 8)} · ${
                    entry.remote ? copy("远程", "Remote") : copy("本地", "Local")
                  }`}
                  selected={entry.name === props.baseRevision}
                  onSelect={() => {
                    props.onSelectBranch(entry.name);
                    props.onOpenChange("branch", false);
                  }}
                />
              )}
            </For>
          </div>
        </FloatingPopover>
      </Show>
    </div>
  );
}

export function ComposerOptionsPopover(
  props: PopoverStateProps & {
    behaviorMode: "default" | "plan";
    skills: SkillRecord[];
    skillsLoading: boolean;
    skillsError: string | undefined;
    selectedSkillIds: string[];
    onChooseAttachments: () => void;
    onTogglePlanMode: () => void;
    onToggleSkill: (skillId: string) => void;
  },
) {
  const i18n = useI18n();
  const copy = (zh: string, en: string) => (i18n.locale() === "zh-CN" ? zh : en);

  return (
    <FloatingPopover
      open={props.activePopover === "options"}
      onOpenChange={(open) => props.onOpenChange("options", open)}
      label={i18n.t("workbench.taskOptions")}
      triggerClass="composer-plus composer-icon-button"
      triggerTestId="workbench-task-options"
      contentClass="composer-popover composer-options-popover"
      contentTestId="workbench-options-popover"
      trigger={<Plus size={19} />}
    >
      <div class="composer-popover-heading">{i18n.t("workbench.addPopoverTitle")}</div>
      <div class="composer-popover-list">
        <div class="composer-attachment-menu-row">
          <Button
            type="button"
            class="composer-attachment-file-button"
            data-testid="workbench-add-attachment"
            onClick={props.onChooseAttachments}
          >
            <span class="composer-popover-row-icon" aria-hidden="true">
              <Paperclip size={18} />
            </span>
            <span class="composer-popover-row-copy">
              <strong>{i18n.t("workbench.addAttachment")}</strong>
              <small>{i18n.t("workbench.addAttachmentDescription")}</small>
            </span>
          </Button>
        </div>
        <MenuRow
          icon={<Lightbulb size={18} />}
          label={i18n.t("workbench.planMode")}
          description={i18n.t("workbench.planModeDescription")}
          selected={props.behaviorMode === "plan"}
          testId="workbench-plan-mode"
          onSelect={props.onTogglePlanMode}
        />
      </div>

      <div class="composer-popover-section-heading">
        <Sparkles size={14} />
        <span>{i18n.t("workbench.skills")}</span>
      </div>
      <div class="composer-popover-list composer-skill-list">
        <Show when={props.skillsLoading}>
          <div class="composer-popover-status">{i18n.t("workbench.skillsLoading")}</div>
        </Show>
        <Show when={!props.skillsLoading && props.skillsError}>
          <div class="composer-popover-status error" role="status">
            <AlertTriangle size={15} />
            <span>{props.skillsError}</span>
          </div>
        </Show>
        <Show when={!props.skillsLoading && !props.skillsError && props.skills.length === 0}>
          <div class="composer-popover-status">{i18n.t("workbench.skillsEmpty")}</div>
        </Show>
        <For each={props.skills}>
          {(skill, index) => {
            const hasErrors = () => skill.diagnostics.some((item) => item.severity === "error");
            const unavailable = () => !skill.enabled || hasErrors();
            const displayName = () => skillDisplayName(skill, i18n.locale() === "zh-CN");
            const description = () =>
              unavailable()
                ? !skill.enabled
                  ? i18n.t("workbench.skillDisabled")
                  : copy(
                      "技能存在错误，请先前往设置修复",
                      "This Skill has errors; fix it in Settings",
                    )
                : skill.description;
            return (
              <MenuRow
                icon={
                  <span class={`composer-skill-icon skill-color-${index() % 5}`}>
                    <FileText size={15} />
                  </span>
                }
                label={displayName()}
                description={description()}
                selected={props.selectedSkillIds.includes(skill.id)}
                disabled={unavailable()}
                testId={`workbench-skill-${skill.name}`}
                onSelect={() => props.onToggleSkill(skill.id)}
              />
            );
          }}
        </For>
      </div>
    </FloatingPopover>
  );
}

export function ApprovalPolicyPopover(
  props: PopoverStateProps & {
    value: ApprovalPolicy;
    onChange: (policy: ApprovalPolicy) => void;
  },
) {
  const i18n = useI18n();
  const policies: Array<{
    value: ApprovalPolicy;
    icon: JSX.Element;
    label: string;
    description: string;
    tone: MenuRowTone;
  }> = [
    {
      value: "always_ask_side_effects",
      icon: <Hand size={18} />,
      label: i18n.t("workbench.approvalAlwaysAsk"),
      description: i18n.t("workbench.approvalAlwaysAskDescription"),
      tone: "neutral",
    },
    {
      value: "only_when_needed",
      icon: <ShieldCheck size={18} />,
      label: i18n.t("workbench.approvalOnlyWhenNeeded"),
      description: i18n.t("workbench.approvalOnlyWhenNeededDescription"),
      tone: "recommended",
    },
    {
      value: "never_prompt",
      icon: <ShieldOff size={18} />,
      label: i18n.t("workbench.approvalNeverPrompt"),
      description: i18n.t("workbench.approvalNeverPromptDescription"),
      tone: "danger",
    },
  ];
  const selected = () => policies.find((policy) => policy.value === props.value) ?? policies[1]!;

  return (
    <FloatingPopover
      open={props.activePopover === "approval"}
      onOpenChange={(open) => props.onOpenChange("approval", open)}
      label={i18n.t("workbench.approvalPolicy")}
      triggerClass={`composer-approval-trigger composer-policy policy-${selected().tone}`}
      triggerTestId="workbench-approval-policy"
      contentClass="composer-popover composer-approval-popover"
      contentTestId="workbench-approval-popover"
      trigger={
        <>
          {selected().icon}
          <span>{selected().label}</span>
          <ChevronDown class="composer-trigger-chevron" size={13} aria-hidden="true" />
        </>
      }
    >
      <div class="composer-popover-heading">{i18n.t("workbench.approvalPopoverTitle")}</div>
      <div class="composer-popover-list">
        <For each={policies}>
          {(policy) => (
            <MenuRow
              icon={policy.icon}
              label={policy.label}
              description={policy.description}
              selected={props.value === policy.value}
              tone={policy.tone}
              testId={`workbench-approval-policy-${policy.value}`}
              onSelect={() => {
                props.onChange(policy.value);
                props.onOpenChange("approval", false);
              }}
            />
          )}
        </For>
      </div>
    </FloatingPopover>
  );
}

export function PlanModeChip(props: { onDisable: () => void }) {
  const i18n = useI18n();
  const disableLabel = () => i18n.t("workbench.disablePlanMode");

  return (
    <>
      <span class="composer-options-divider" aria-hidden="true" />
      <Tooltip label={disableLabel()}>
        <Button
          type="button"
          class="composer-plan-mode-chip"
          data-testid="workbench-plan-mode-chip"
          aria-label={disableLabel()}
          onClick={props.onDisable}
        >
          <span class="composer-plan-mode-icon" aria-hidden="true">
            <Lightbulb class="composer-plan-mode-default-icon" size={17} />
            <X class="composer-plan-mode-remove-icon" size={16} />
          </span>
          <span>{i18n.t("workbench.planModeChip")}</span>
        </Button>
      </Tooltip>
    </>
  );
}

export function SkillReferenceList(props: {
  skills: SkillRecord[];
  onRemove: (skillId: string) => void;
}) {
  const i18n = useI18n();
  return (
    <Show when={props.skills.length > 0}>
      <div class="composer-skill-references" aria-label={i18n.t("workbench.skillReferences")}>
        <For each={props.skills}>
          {(skill, index) => {
            const displayName = skillDisplayName(skill, i18n.locale() === "zh-CN");
            return (
              <Button
                type="button"
                class="composer-skill-reference"
                aria-label={i18n.t("workbench.removeSkillReference").replace("{name}", displayName)}
                title={i18n.t("workbench.removeSkillReference").replace("{name}", displayName)}
                onClick={() => props.onRemove(skill.id)}
              >
                <span class={`composer-skill-icon skill-color-${index() % 5}`} aria-hidden="true">
                  <FileText size={14} />
                </span>
                <span>{displayName}</span>
              </Button>
            );
          }}
        </For>
      </div>
    </Show>
  );
}
