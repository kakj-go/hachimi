import {
  commandFailure,
  type GitFileStatus,
  type GitMutation,
  type GitWorkspaceSnapshot,
  type MutationContext,
  type RunRecord,
  type WorkbenchSessionSnapshot,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import { Badge, Button, Check, GitBranch, RefreshCw, TextField } from "@hachimi/ui";
import { For, Show, createEffect, createMemo, createSignal } from "solid-js";

import type { WorkbenchCommandPort } from "./workbench-command-port";
import "./workspace-git-panel.css";

function latestRun(snapshot: WorkbenchSessionSnapshot): RunRecord | undefined {
  return snapshot.runs[snapshot.runs.length - 1];
}

function mutationContext(run: RunRecord): MutationContext {
  return {
    requestId: crypto.randomUUID(),
    clientId: "window:workbench",
    protocolVersion: 18,
    idempotencyKey: crypto.randomUUID(),
    expectedRunId: run.id,
    expectedGeneration: run.generation,
  };
}

function staged(entry: GitFileStatus): boolean {
  return entry.indexStatus !== " " && entry.indexStatus !== "?";
}

function unstaged(entry: GitFileStatus): boolean {
  return entry.worktreeStatus !== " " || entry.indexStatus === "?";
}

export function WorkspaceGitPanel(props: {
  snapshot: WorkbenchSessionSnapshot;
  commandPort: WorkbenchCommandPort;
  revision: number;
}) {
  const i18n = useI18n();
  const [git, setGit] = createSignal<GitWorkspaceSnapshot>();
  const [message, setMessage] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [failure, setFailure] = createSignal<string>();
  const [notice, setNotice] = createSignal<string>();
  let requestGeneration = 0;

  const zh = () => i18n.locale() === "zh-CN";
  const checkoutId = () =>
    props.snapshot.session.context.kind === "project"
      ? props.snapshot.session.context.checkout_id
      : undefined;
  const run = () => latestRun(props.snapshot);
  const stagedEntries = createMemo(() => git()?.status.filter(staged) ?? []);
  const unstagedEntries = createMemo(() => git()?.status.filter(unstaged) ?? []);

  async function load(dependency?: {
    sessionId: string;
    checkoutId: string | undefined;
    revision: number;
  }) {
    const currentCheckout = dependency?.checkoutId ?? checkoutId();
    if (!currentCheckout) return;
    const generation = ++requestGeneration;
    setBusy(true);
    try {
      const snapshot = await props.commandPort.getWorkspaceGit({
        sessionId: dependency?.sessionId ?? props.snapshot.session.id,
        checkoutId: currentCheckout,
        historyLimit: 20,
      });
      if (generation === requestGeneration) setGit(snapshot);
    } catch (error) {
      if (generation === requestGeneration) setFailure(commandFailure(error).message);
    } finally {
      if (generation === requestGeneration) setBusy(false);
    }
  }

  async function mutate(mutation: GitMutation) {
    const currentRun = run();
    const currentCheckout = checkoutId();
    if (!currentRun || !currentCheckout) return;
    setBusy(true);
    setFailure(undefined);
    setNotice(undefined);
    try {
      const response = await props.commandPort.mutateWorkspaceGit({
        context: mutationContext(currentRun),
        sessionId: props.snapshot.session.id,
        checkoutId: currentCheckout,
        mutation,
      });
      setGit(response.snapshot);
      if (mutation.kind === "commit") {
        setMessage("");
        setNotice(
          response.commitSha
            ? `${zh() ? "已创建本地提交" : "Created local commit"} ${response.commitSha.slice(0, 8)}`
            : zh()
              ? "本地提交已完成"
              : "Local commit completed",
        );
      }
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
    }
  }

  createEffect(() => {
    void load({
      sessionId: props.snapshot.session.id,
      checkoutId: checkoutId(),
      revision: props.revision,
    });
  });

  return (
    <div class="workspace-git-panel" data-component="git-panel">
      <header>
        <div>
          <GitBranch size={14} />
          <strong>{git()?.branch ?? (git()?.detached ? "Detached HEAD" : "Git")}</strong>
          <Show when={git()?.headSha}>{(sha) => <code>{sha().slice(0, 8)}</code>}</Show>
        </div>
        <Button
          aria-label={zh() ? "刷新 Git" : "Refresh Git"}
          title={zh() ? "刷新 Git" : "Refresh Git"}
          disabled={busy()}
          onClick={() => void load()}
        >
          <RefreshCw size={13} />
        </Button>
      </header>

      <Show when={failure()}>
        {(value) => <div class="workspace-git-message error">{value()}</div>}
      </Show>
      <Show when={notice()}>
        {(value) => <div class="workspace-git-message success">{value()}</div>}
      </Show>

      <section class="workspace-git-changes">
        <div class="workspace-git-section-heading">
          <strong>{zh() ? "变更" : "Changes"}</strong>
          <span>{git()?.status.length ?? 0}</span>
          <Show when={unstagedEntries().length > 0}>
            <Button
              data-testid="workspace-git-stage-all"
              disabled={busy()}
              onClick={() =>
                void mutate({ kind: "stage", paths: unstagedEntries().map((entry) => entry.path) })
              }
            >
              {zh() ? "全部暂存" : "Stage all"}
            </Button>
          </Show>
          <Show when={stagedEntries().length > 0}>
            <Button
              data-testid="workspace-git-unstage-all"
              disabled={busy()}
              onClick={() =>
                void mutate({
                  kind: "unstage",
                  paths: stagedEntries().map((entry) => entry.path),
                })
              }
            >
              {zh() ? "全部取消暂存" : "Unstage all"}
            </Button>
          </Show>
        </div>
        <Show
          when={(git()?.status.length ?? 0) > 0}
          fallback={
            <p>{zh() ? "工作区没有本地变更。" : "The working tree has no local changes."}</p>
          }
        >
          <For each={git()?.status ?? []}>
            {(entry) => (
              <div class="workspace-git-change" data-testid="workspace-git-change">
                <div>
                  <strong>{entry.path}</strong>
                  <Show when={entry.previousPath}>{(path) => <small>{path()} →</small>}</Show>
                </div>
                <div class="workspace-git-statuses">
                  <Show when={staged(entry)}>
                    <Badge tone="success">{entry.indexStatus}</Badge>
                  </Show>
                  <Show when={unstaged(entry)}>
                    <Badge tone="warning">{entry.worktreeStatus}</Badge>
                  </Show>
                </div>
                <Show when={unstaged(entry)}>
                  <Button
                    disabled={busy()}
                    onClick={() => void mutate({ kind: "stage", paths: [entry.path] })}
                  >
                    {zh() ? "暂存" : "Stage"}
                  </Button>
                </Show>
                <Show when={staged(entry)}>
                  <Button
                    disabled={busy()}
                    onClick={() => void mutate({ kind: "unstage", paths: [entry.path] })}
                  >
                    {zh() ? "取消暂存" : "Unstage"}
                  </Button>
                </Show>
              </div>
            )}
          </For>
        </Show>
      </section>

      <section class="workspace-git-commit">
        <TextField
          label={zh() ? "提交说明" : "Commit message"}
          testId="workspace-git-commit-message"
          value={message()}
          placeholder={zh() ? "描述这次本地变更" : "Describe the local changes"}
          onInput={(event) => setMessage(event.currentTarget.value)}
        />
        <Button
          data-testid="workspace-git-commit"
          variant="primary"
          disabled={busy() || stagedEntries().length === 0 || !message().trim()}
          onClick={() => void mutate({ kind: "commit", message: message().trim() })}
        >
          <Check size={13} /> {zh() ? "创建本地提交" : "Create local commit"}
        </Button>
        <small>
          {zh()
            ? "提交仅保存在本地；push 和 PR 仍保持关闭。Repository hooks 不会执行。"
            : "Commits remain local; push and PR stay disabled. Repository hooks are not executed."}
        </small>
      </section>

      <section class="workspace-git-history">
        <div class="workspace-git-section-heading">
          <strong>{zh() ? "最近提交" : "Recent commits"}</strong>
        </div>
        <For each={git()?.recentCommits ?? []}>
          {(commit) => (
            <div class="workspace-git-commit-row">
              <code>{commit.abbreviatedSha}</code>
              <div>
                <strong>{commit.subject}</strong>
                <small>
                  {commit.authorName} ·{" "}
                  {new Date(commit.committedAtMs).toLocaleString(i18n.locale())}
                </small>
              </div>
            </div>
          )}
        </For>
      </section>
    </div>
  );
}
