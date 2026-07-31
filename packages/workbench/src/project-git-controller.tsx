import {
  commandFailure,
  type GitRefRecord,
  type ProjectGitSnapshot,
  type ProjectRecord,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import { Button, Dialog, TextField } from "@hachimi/ui";
import { Show, createEffect, createMemo, createSignal, untrack, type Accessor } from "solid-js";

import type { WorkbenchCommandPort } from "./workbench-command-port";
import { directUserMutationContext } from "./mutation-context";

export function createProjectGitController(options: {
  commandPort: WorkbenchCommandPort;
  selectedProject: Accessor<ProjectRecord | undefined>;
  onProjectReconciled: (projectId: string, gitRoot: string | null) => void;
  onFailure: (message: string | undefined) => void;
}) {
  const [snapshot, setSnapshot] = createSignal<ProjectGitSnapshot>();
  const [refs, setRefs] = createSignal<GitRefRecord[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [executionKind, setExecutionKind] = createSignal<"local" | "managed_worktree">("local");
  const [baseRevision, setBaseRevision] = createSignal("");
  const [initialCommitOpen, setInitialCommitOpen] = createSignal(false);
  const [initialCommitBusy, setInitialCommitBusy] = createSignal(false);
  const [authorName, setAuthorName] = createSignal("");
  const [authorEmail, setAuthorEmail] = createSignal("");

  const load = async (projectId: string, force = false) => {
    setLoading(true);
    try {
      const next = force
        ? await options.commandPort.refreshProjectGit(projectId)
        : await options.commandPort.inspectProjectGit(projectId);
      if (untrack(options.selectedProject)?.id !== projectId) return;
      setSnapshot(next);
      options.onProjectReconciled(projectId, next.gitRoot);
      if (next.state.kind !== "ready") {
        setRefs([]);
        setExecutionKind("local");
        setBaseRevision("");
        return;
      }
      const nextRefs = await options.commandPort.listProjectGitRefs(projectId);
      if (untrack(options.selectedProject)?.id !== projectId) return;
      setRefs(nextRefs);
      const preferred = nextRefs.find((entry) => entry.current) ?? nextRefs[0];
      setBaseRevision((current) =>
        nextRefs.some((entry) => entry.name === current) ? current : (preferred?.name ?? ""),
      );
    } catch (error) {
      if (untrack(options.selectedProject)?.id === projectId) {
        setSnapshot(undefined);
        setRefs([]);
        setExecutionKind("local");
        setBaseRevision("");
        options.onFailure(commandFailure(error).message);
      }
    } finally {
      if (untrack(options.selectedProject)?.id === projectId) setLoading(false);
    }
  };

  const selectedProjectId = createMemo(() => options.selectedProject()?.id);
  createEffect(() => {
    const projectId = selectedProjectId();
    if (!projectId) {
      setSnapshot(undefined);
      setRefs([]);
      setExecutionKind("local");
      setBaseRevision("");
      return;
    }
    void load(projectId);
  });

  const createInitialCommit = async () => {
    const project = options.selectedProject();
    if (!project) return;
    setInitialCommitBusy(true);
    options.onFailure(undefined);
    try {
      const response = await options.commandPort.createProjectEmptyInitialCommit({
        context: directUserMutationContext(),
        projectId: project.id,
        authorName: authorName(),
        authorEmail: authorEmail(),
      });
      setSnapshot(response.snapshot);
      options.onProjectReconciled(project.id, response.snapshot.gitRoot);
      setInitialCommitOpen(false);
      await load(project.id, true);
    } catch (error) {
      options.onFailure(commandFailure(error).message);
    } finally {
      setInitialCommitBusy(false);
    }
  };

  return {
    snapshot,
    refs,
    loading,
    executionKind,
    baseRevision,
    setExecutionKind,
    setBaseRevision,
    refresh: () => {
      const projectId = options.selectedProject()?.id;
      if (projectId) void load(projectId, true);
    },
    resetForDraft: () => {
      setExecutionKind("local");
    },
    openInitialCommit: () => setInitialCommitOpen(true),
    initialCommitDialog: () => (
      <ProjectGitInitialCommitDialog
        open={initialCommitOpen()}
        busy={initialCommitBusy()}
        authorName={authorName()}
        authorEmail={authorEmail()}
        onAuthorName={setAuthorName}
        onAuthorEmail={setAuthorEmail}
        onClose={() => setInitialCommitOpen(false)}
        onConfirm={() => void createInitialCommit()}
      />
    ),
  };
}

function ProjectGitInitialCommitDialog(props: {
  open: boolean;
  busy: boolean;
  authorName: string;
  authorEmail: string;
  onAuthorName: (value: string) => void;
  onAuthorEmail: (value: string) => void;
  onClose: () => void;
  onConfirm: () => void;
}) {
  const i18n = useI18n();
  const zh = () => i18n.locale() === "zh-CN";
  return (
    <Dialog
      open={props.open}
      title={zh() ? "创建空的初始提交" : "Create empty initial commit"}
      description={
        zh()
          ? "仅创建不包含任何文件的根提交。当前已暂存和未跟踪文件不会进入提交，也不会修改 Git 配置。"
          : "Creates a root commit with an empty tree. Staged and untracked files stay untouched, and Git configuration is not changed."
      }
      onOpenChange={(open) => {
        if (!open) props.onClose();
      }}
    >
      <div class="project-git-initial-fields">
        <TextField
          label={zh() ? "作者名称" : "Author name"}
          value={props.authorName}
          onInput={(event) => props.onAuthorName(event.currentTarget.value)}
        />
        <TextField
          label={zh() ? "作者邮箱（仅本次提交）" : "Author email (this commit only)"}
          value={props.authorEmail}
          onInput={(event) => props.onAuthorEmail(event.currentTarget.value)}
        />
        <Show when={props.authorEmail && !props.authorEmail.includes("@")}>
          <p class="composer-error">{zh() ? "请输入有效邮箱。" : "Enter a valid email address."}</p>
        </Show>
        <div class="dialog-actions">
          <Button type="button" variant="ghost" disabled={props.busy} onClick={props.onClose}>
            {zh() ? "取消" : "Cancel"}
          </Button>
          <Button
            type="button"
            disabled={props.busy || !props.authorName.trim() || !props.authorEmail.includes("@")}
            data-testid="project-git-create-initial-confirm"
            onClick={props.onConfirm}
          >
            {props.busy ? (zh() ? "正在创建…" : "Creating…") : zh() ? "创建提交" : "Create commit"}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
