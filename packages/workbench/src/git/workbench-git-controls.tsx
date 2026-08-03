import {
  commandFailure,
  type WorkbenchEnvironmentSnapshot,
  type WorkbenchGitAction,
  type WorkbenchGitResponse,
} from "@hachimi/contracts";
import {
  Badge,
  Button,
  Check,
  Checkbox,
  FloatingPopover,
  GitBranch,
  GitFork,
  SearchField,
  TextField,
  Upload,
} from "@hachimi/ui";
import { For, Show, createMemo, createSignal } from "solid-js";

import type { WorkbenchEnvironmentController } from "../state/workbench-environment-controller";
import "./workbench-git-controls.css";

export function WorkbenchGitControls(props: {
  environment: WorkbenchEnvironmentSnapshot;
  controller: WorkbenchEnvironmentController;
  locale: "zh-CN" | "en-US";
  remotePushEnabled: boolean;
  onOpenDiff: (branch?: string, branches?: string[]) => void;
}) {
  const [branchOpen, setBranchOpen] = createSignal(false);
  const [commitOpen, setCommitOpen] = createSignal(false);
  const [branchQuery, setBranchQuery] = createSignal("");
  const [newBranch, setNewBranch] = createSignal("");
  const [message, setMessage] = createSignal("");
  const [includeUnstaged, setIncludeUnstaged] = createSignal(true);
  const [busy, setBusy] = createSignal(false);
  const [failure, setFailure] = createSignal<string>();
  const [result, setResult] = createSignal<WorkbenchGitResponse>();
  const [pushState, setPushState] = createSignal<"idle" | "succeeded" | "retry">("idle");

  const zh = () => props.locale === "zh-CN";
  const filteredRefs = createMemo(() => {
    const query = branchQuery().trim().toLocaleLowerCase();
    return query
      ? props.environment.git.refs.filter((reference) =>
          reference.name.toLocaleLowerCase().includes(query),
        )
      : props.environment.git.refs;
  });
  const canPush = () =>
    props.remotePushEnabled &&
    !props.environment.git.detached &&
    Boolean(props.environment.git.branch) &&
    Boolean(props.environment.git.headSha) &&
    props.environment.git.remotes.length > 0;

  async function executeBranch(action: WorkbenchGitAction) {
    setBusy(true);
    setFailure(undefined);
    try {
      const response = await props.controller.executeGit(action, false);
      if (response.stage.status === "failed") {
        setFailure(response.stage.message ?? (zh() ? "分支操作失败" : "Branch operation failed"));
        return;
      }
      setBranchOpen(false);
      setBranchQuery("");
      setNewBranch("");
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
    }
  }

  async function commit(pushAfter: boolean) {
    setBusy(true);
    setFailure(undefined);
    setResult(undefined);
    setPushState("idle");
    try {
      const response = await props.controller.executeGit(
        { kind: "commit", message: message().trim() || null },
        includeUnstaged(),
      );
      setResult(response);
      if (response.commit.status !== "succeeded" || !pushAfter) return;
      try {
        await props.controller.pushGit({ head: response.head, branch: response.branch });
        setPushState("succeeded");
      } catch (error) {
        setFailure(commandFailure(error).message);
        setPushState("retry");
      }
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
    }
  }

  async function pushOnly() {
    setBusy(true);
    setFailure(undefined);
    try {
      await props.controller.pushGit();
      setPushState("succeeded");
    } catch (error) {
      setFailure(commandFailure(error).message);
      setPushState("retry");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div class="workbench-git-controls" aria-label={zh() ? "Git 操作" : "Git actions"}>
      <FloatingPopover
        open={branchOpen()}
        onOpenChange={(open) => {
          setBranchOpen(open);
          if (open) setFailure(undefined);
        }}
        label={zh() ? "切换分支" : "Switch branch"}
        placement="bottom-start"
        contentClass="workbench-git-popover branch"
        triggerClass="environment-summary-row workbench-git-trigger"
        triggerTestId="workbench-git-branch-trigger"
        trigger={
          <>
            <GitBranch size={16} />
            <strong title={props.environment.git.branch ?? "detached HEAD"}>
              {props.environment.git.branch ?? "detached HEAD"}
            </strong>
            <span class="environment-row-tail">›</span>
          </>
        }
      >
        <div class="workbench-git-popover-heading">
          <strong>{zh() ? "分支" : "Branches"}</strong>
          <Badge tone={props.environment.git.uncommittedFiles ? "warning" : "success"}>
            {props.environment.git.uncommittedFiles} {zh() ? "个未提交文件" : "uncommitted"}
          </Badge>
        </div>
        <SearchField
          label={zh() ? "搜索分支" : "Search branches"}
          value={branchQuery()}
          onInput={(event) => setBranchQuery(event.currentTarget.value)}
        />
        <div class="workbench-branch-list">
          <For each={filteredRefs()}>
            {(reference) => (
              <Button
                class="workbench-branch-row"
                classList={{ current: reference.current }}
                disabled={
                  busy() ||
                  reference.current ||
                  props.environment.git.uncommittedFiles > 0
                }
                title={reference.name}
                onClick={() =>
                  void executeBranch({
                    kind: "switch_branch",
                    branch: reference.name,
                    remote: reference.remote,
                  })
                }
              >
                <GitBranch size={15} />
                <span>
                  <strong>{reference.name}</strong>
                  <small>{reference.remote ? (zh() ? "远程" : "Remote") : "Local"}</small>
                </span>
                {reference.current ? <Check size={15} /> : null}
              </Button>
            )}
          </For>
        </div>
        <Show when={props.environment.git.uncommittedFiles > 0}>
          <p class="workbench-git-hint">
            {zh()
              ? "存在未提交改动时不能切换已有分支；可以创建新分支并携带这些改动。"
              : "Commit changes before switching, or create a new branch and carry them over."}
          </p>
        </Show>
        <div class="workbench-create-branch">
          <TextField
            label={zh() ? "新分支名称" : "New branch name"}
            value={newBranch()}
            onInput={(event) => setNewBranch(event.currentTarget.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && newBranch().trim()) {
                void executeBranch({ kind: "create_branch", branch: newBranch().trim() });
              }
            }}
          />
          <Button
            disabled={busy() || !newBranch().trim()}
            onClick={() =>
              void executeBranch({ kind: "create_branch", branch: newBranch().trim() })
            }
          >
            {zh() ? "创建并检出" : "Create and checkout"}
          </Button>
        </div>
        <Show when={failure()}>{(value) => <p class="workbench-git-error">{value()}</p>}</Show>
      </FloatingPopover>

      <FloatingPopover
        open={commitOpen()}
        onOpenChange={(open) => {
          setCommitOpen(open);
          if (open) setFailure(undefined);
        }}
        label={zh() ? "提交或推送" : "Commit or push"}
        placement="bottom-start"
        contentClass="workbench-git-popover commit"
        triggerClass="environment-summary-row workbench-git-trigger"
        triggerTestId="workbench-git-commit-trigger"
        trigger={
          <>
            <Upload size={16} />
            <strong>{zh() ? "提交或推送" : "Commit or push"}</strong>
            <span class="environment-row-tail">›</span>
          </>
        }
      >
        <div class="workbench-git-popover-heading">
          <span>
            <strong>{props.environment.git.branch ?? "detached HEAD"}</strong>
            <small>
              {props.environment.git.headSha?.slice(0, 8) ?? (zh() ? "尚无提交" : "No commits")}
            </small>
          </span>
          <span class="workbench-git-diff-stat">
            {props.environment.changes.changedFiles} {zh() ? "文件" : "files"} ·{" "}
            <b class="diff-additions">+{props.environment.changes.additions}</b>{" "}
            <b class="diff-deletions">-{props.environment.changes.deletions}</b>
          </span>
        </div>
        <TextField
          testId="workbench-git-commit-message"
          label={zh() ? "提交说明（留空则自动生成）" : "Commit message (auto when empty)"}
          value={message()}
          onInput={(event) => setMessage(event.currentTarget.value)}
        />
        <Checkbox
          label={zh() ? "包含未暂存更改" : "Include unstaged changes"}
          checked={includeUnstaged()}
          onChange={(event) => setIncludeUnstaged(event.currentTarget.checked)}
        />
        <Show when={failure()}>{(value) => <p class="workbench-git-error">{value()}</p>}</Show>
        <Show when={result()}>
          {(value) => (
            <div class="workbench-git-phases" role="status">
              <Phase label={zh() ? "暂存" : "Stage"} value={value().stage} />
              <Phase label={zh() ? "提交" : "Commit"} value={value().commit} />
            </div>
          )}
        </Show>
        <Show when={pushState() === "succeeded"}>
          <p class="workbench-git-success">{zh() ? "推送成功" : "Push succeeded"}</p>
        </Show>
        <div class="workbench-git-submit-row">
          <Button
            data-testid="workbench-git-commit"
            disabled={busy() || props.environment.git.uncommittedFiles === 0}
            onClick={() => void commit(false)}
          >
            {zh() ? "提交" : "Commit"}
          </Button>
          <Show when={canPush()}>
            <Button
              data-testid="workbench-git-commit-and-push"
              variant="primary"
              disabled={busy() || props.environment.git.uncommittedFiles === 0}
              onClick={() => void commit(true)}
            >
              {zh() ? "提交并推送" : "Commit & push"}
            </Button>
            <Button
              data-testid="workbench-git-push"
              disabled={busy()}
              onClick={() => void pushOnly()}
            >
              {pushState() === "retry"
                ? zh()
                  ? "重试推送"
                  : "Retry push"
                : zh()
                  ? "仅推送"
                  : "Push only"}
            </Button>
          </Show>
        </div>
      </FloatingPopover>

      <Button
        class="environment-summary-row workbench-compare-row"
        data-testid="workbench-git-compare"
        disabled={!props.environment.git.defaultComparisonRef}
        onClick={() =>
          props.onOpenDiff(
            props.environment.git.defaultComparisonRef ?? undefined,
            [...new Set(props.environment.git.refs.map((reference) => reference.name))],
          )
        }
      >
        <GitFork size={16} />
        <strong>{zh() ? "比较分支" : "Compare branch"}</strong>
        <span class="environment-row-tail" title={props.environment.git.defaultComparisonRef ?? ""}>
          {props.environment.git.defaultComparisonRef ?? "-"} ›
        </span>
      </Button>
    </div>
  );
}

function Phase(props: { label: string; value: WorkbenchGitResponse["stage"] }) {
  return (
    <div data-status={props.value.status}>
      <strong>{props.label}</strong>
      <span>{props.value.status}</span>
      <Show when={props.value.message}>{(message) => <small>{message()}</small>}</Show>
    </div>
  );
}
