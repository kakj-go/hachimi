import {
  commandFailure,
  type ForgeRepositoryIdentity,
  type GitFileStatus,
  type GitMutation,
  type GitRemoteRecord,
  type GitWorkspaceSnapshot,
  type RunRecord,
  type WorkbenchSessionSnapshot,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import { Badge, Button, Check, GitBranch, RefreshCw, SelectField, TextField } from "@hachimi/ui";
import { For, Show, createEffect, createMemo, createSignal } from "solid-js";

import type { WorkbenchCommandPort } from "./workbench-command-port";
import { runMutationContext } from "./mutation-context";
import "./workspace-git-panel.css";

function latestRun(snapshot: WorkbenchSessionSnapshot): RunRecord | undefined {
  return snapshot.runs[snapshot.runs.length - 1];
}

function staged(entry: GitFileStatus): boolean {
  return entry.indexStatus !== " " && entry.indexStatus !== "?";
}

function unstaged(entry: GitFileStatus): boolean {
  return entry.worktreeStatus !== " " || entry.indexStatus === "?";
}

function forgeRepository(remote: GitRemoteRecord): ForgeRepositoryIdentity | undefined {
  if (remote.forgeKind === "unknown") return undefined;
  const scp = remote.displayUrl.match(/^(?:[^@]+@)?([^:]+):(.+)$/u);
  let host: string;
  let path: string;
  try {
    const url = new URL(remote.displayUrl);
    host = url.host;
    path = url.pathname;
  } catch {
    if (!scp) return undefined;
    host = scp[1]!;
    path = scp[2]!;
  }
  const parts = path
    .replace(/^\/+|\.git$/gu, "")
    .split("/")
    .filter(Boolean);
  const repository = parts.pop();
  const owner = parts.join("/");
  if (!owner || !repository) return undefined;
  const apiBaseUrl =
    remote.forgeKind === "git_hub"
      ? "https://api.github.com/"
      : remote.forgeKind === "git_lab"
        ? `https://${host}/api/v4/`
        : remote.forgeKind === "gitee"
          ? "https://gitee.com/api/v5/"
          : `https://${host}/api/v1/`;
  return {
    forgeKind: remote.forgeKind,
    apiBaseUrl,
    owner,
    repository,
    remoteUrlHash: remote.remoteUrlHash,
    secretRef: `forge:${remote.forgeKind}:${remote.remoteUrlHash.slice(0, 24)}`,
  };
}

export function WorkspaceGitPanel(props: {
  snapshot: WorkbenchSessionSnapshot;
  commandPort: WorkbenchCommandPort;
  revision: number;
  gitRemoteMutationsEnabled?: boolean;
}) {
  const i18n = useI18n();
  const [git, setGit] = createSignal<GitWorkspaceSnapshot>();
  const [message, setMessage] = createSignal("");
  const [remotes, setRemotes] = createSignal<GitRemoteRecord[]>([]);
  const [remoteName, setRemoteName] = createSignal("");
  const [forgeToken, setForgeToken] = createSignal("");
  const [prTitle, setPrTitle] = createSignal("");
  const [targetBranch, setTargetBranch] = createSignal("main");
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
  const remote = createMemo(
    () => remotes().find((entry) => entry.name === remoteName()) ?? remotes()[0],
  );
  const repository = createMemo(() => {
    const current = remote();
    return current ? forgeRepository(current) : undefined;
  });

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
      const remoteSnapshot = await props.commandPort.listGitRemotes({
        sessionId: dependency?.sessionId ?? props.snapshot.session.id,
        checkoutId: currentCheckout,
      });
      if (generation === requestGeneration) {
        setGit(snapshot);
        setRemotes(remoteSnapshot);
        if (!remoteSnapshot.some((entry) => entry.name === remoteName())) {
          setRemoteName(remoteSnapshot[0]?.name ?? "");
        }
      }
    } catch (error) {
      if (generation === requestGeneration) setFailure(commandFailure(error).message);
    } finally {
      if (generation === requestGeneration) setBusy(false);
    }
  }

  async function push() {
    const currentRun = run();
    const currentCheckout = checkoutId();
    const currentGit = git();
    const currentRemote = remote();
    if (!currentRun || !currentCheckout || !currentGit?.headSha || !currentRemote) return;
    setBusy(true);
    setFailure(undefined);
    setNotice(undefined);
    try {
      const response = await props.commandPort.pushGitRemote({
        context: runMutationContext(currentRun),
        sessionId: props.snapshot.session.id,
        checkoutId: currentCheckout,
        remoteName: currentRemote.name,
        expectedRemoteUrlHash: currentRemote.remoteUrlHash,
        sourceRef: "HEAD",
        targetRef: `refs/heads/${currentGit.branch ?? targetBranch()}`,
        expectedCommitOid: currentGit.headSha,
        approvalId: null,
      });
      setNotice(
        `${zh() ? "远程 push 已确认" : "Remote push confirmed"} ${response.commitOid.slice(0, 8)}`,
      );
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
    }
  }

  async function saveForgeToken() {
    const current = repository();
    if (!current?.secretRef || !forgeToken().trim()) return;
    setBusy(true);
    setFailure(undefined);
    try {
      await props.commandPort.updateForgeCredential({
        secretRef: current.secretRef,
        secret: forgeToken().trim(),
      });
      setForgeToken("");
      setNotice(
        zh() ? "Forge Token 已保存到系统凭据管理器。" : "Forge token saved in Credential Manager.",
      );
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
    }
  }

  async function createPullRequest() {
    const currentRun = run();
    const currentCheckout = checkoutId();
    const currentGit = git();
    const currentRepository = repository();
    if (
      !currentRun ||
      !currentCheckout ||
      !currentGit?.headSha ||
      !currentGit.branch ||
      !currentRepository ||
      !prTitle().trim() ||
      !targetBranch().trim()
    )
      return;
    setBusy(true);
    setFailure(undefined);
    setNotice(undefined);
    try {
      const change = await props.commandPort.mutateForgeChange({
        context: runMutationContext(currentRun),
        sessionId: props.snapshot.session.id,
        checkoutId: currentCheckout,
        repository: currentRepository,
        mutation: {
          kind: "create",
          title: prTitle().trim(),
          body: "",
          source_ref: currentGit.branch,
          target_ref: targetBranch().trim(),
        },
        expectedRevision: null,
        expectedCommitOid: currentGit.headSha,
        approvalId: null,
      });
      setNotice(
        `${zh() ? "已创建 PR/MR" : "Created PR/MR"} #${change.number}${change.webUrl ? ` · ${change.webUrl}` : ""}`,
      );
      setPrTitle("");
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
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
        context: runMutationContext(currentRun),
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
            ? "Repository hooks 不会执行；push 与 PR/MR 需要逐次审批。"
            : "Repository hooks are disabled; push and PR/MR require per-operation approval."}
        </small>
      </section>

      <Show when={props.gitRemoteMutationsEnabled !== false}>
        <section class="workspace-git-remote">
          <div class="workspace-git-section-heading">
            <strong>{zh() ? "远程与 PR/MR" : "Remote and PR/MR"}</strong>
            <span>{remotes().length}</span>
          </div>
          <Show
            when={remotes().length > 0}
            fallback={
              <small>{zh() ? "没有配置 Git Remote。" : "No Git remote is configured."}</small>
            }
          >
            <SelectField
              label="Remote"
              value={remote()?.name ?? ""}
              options={remotes().map((entry) => ({
                value: entry.name,
                label: `${entry.name} · ${entry.displayUrl}`,
              }))}
              disabled={busy()}
              onChange={setRemoteName}
            />
            <Button
              variant="primary"
              disabled={busy() || !git()?.headSha || !remote()}
              onClick={() => void push()}
            >
              {zh() ? "审批并 Push" : "Approve and push"}
            </Button>
            <Show when={repository()}>
              <div class="workspace-git-forge-fields">
                <TextField
                  label="Forge Token"
                  type="password"
                  maxLength={16384}
                  value={forgeToken()}
                  placeholder={zh() ? "只写入系统凭据管理器" : "Stored only in Credential Manager"}
                  onInput={(event) => setForgeToken(event.currentTarget.value)}
                />
                <Button
                  disabled={busy() || !forgeToken().trim()}
                  onClick={() => void saveForgeToken()}
                >
                  {zh() ? "保存 Token" : "Save token"}
                </Button>
                <TextField
                  label={zh() ? "目标分支" : "Target branch"}
                  value={targetBranch()}
                  maxLength={1024}
                  onInput={(event) => setTargetBranch(event.currentTarget.value)}
                />
                <TextField
                  label={zh() ? "PR/MR 标题" : "PR/MR title"}
                  value={prTitle()}
                  maxLength={512}
                  onInput={(event) => setPrTitle(event.currentTarget.value)}
                />
                <Button
                  variant="primary"
                  disabled={
                    busy() ||
                    !git()?.branch ||
                    !git()?.headSha ||
                    !targetBranch().trim() ||
                    !prTitle().trim()
                  }
                  onClick={() => void createPullRequest()}
                >
                  {zh() ? "审批并创建 PR/MR" : "Approve and create PR/MR"}
                </Button>
              </div>
            </Show>
          </Show>
        </section>
      </Show>

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
