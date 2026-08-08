import type { RunSummaryRecord } from "@hachimi/contracts";
import { Box, Button, ChevronDown } from "@hachimi/ui";
import { For, Show } from "solid-js";

export function TimelineRunCompletion(props: {
  summary: RunSummaryRecord | undefined;
  locale: "zh-CN" | "en-US";
  onOpenDiff: (runId: string, path?: string) => void;
}) {
  const zh = () => props.locale === "zh-CN";
  const visibleSummary = () => {
    const summary = props.summary;
    return summary &&
      (summary.changedFiles > 0 ||
        summary.files.length > 0 ||
        summary.additions > 0 ||
        summary.deletions > 0)
      ? summary
      : undefined;
  };
  return (
    <Show when={visibleSummary()}>
      {(summary) => (
        <article class="run-completion-summary" data-status={summary().status}>
          <header>
            <span class="run-summary-heading">
              <Box size={18} />
              <span>
                <strong>
                  {zh()
                    ? `已编辑 ${summary().changedFiles} 个文件`
                    : `Edited ${summary().changedFiles} files`}
                </strong>
                <small>
                  <b class="diff-additions">+{summary().additions}</b>{" "}
                  <b class="diff-deletions">-{summary().deletions}</b>
                </small>
              </span>
            </span>
            <Button
              size="small"
              data-testid="workbench-review-run-changes"
              data-run-id={summary().runId}
              onClick={() => props.onOpenDiff(summary().runId)}
            >
              {zh() ? "审核" : "Review"}
            </Button>
          </header>
          <Show when={summary().diffUnavailable}>
            <small>{zh() ? "Diff 暂不可用" : "Diff unavailable"}</small>
          </Show>
          <ul>
            <For each={summary().files.slice(0, 5)}>
              {(file) => (
                <li>
                  <Button
                    type="button"
                    data-testid="workbench-review-run-file"
                    data-run-id={summary().runId}
                    data-path={file.path}
                    onClick={() => props.onOpenDiff(summary().runId, file.path)}
                  >
                    <code>{file.path}</code>
                  </Button>
                  <span>
                    <b class="diff-additions">+{file.additions}</b>{" "}
                    <b class="diff-deletions">-{file.deletions}</b>
                  </span>
                </li>
              )}
            </For>
          </ul>
          <Show when={summary().files.length > 5}>
            <details class="run-summary-more">
              <summary>
                {zh()
                  ? `再显示 ${summary().files.length - 5} 个文件`
                  : `Show ${summary().files.length - 5} more files`}{" "}
                <ChevronDown size={14} />
              </summary>
              <ul>
                <For each={summary().files.slice(5)}>
                  {(file) => (
                    <li>
                      <Button
                        type="button"
                        data-testid="workbench-review-run-file"
                        data-run-id={summary().runId}
                        data-path={file.path}
                        onClick={() => props.onOpenDiff(summary().runId, file.path)}
                      >
                        <code>{file.path}</code>
                      </Button>
                      <span>
                        <b class="diff-additions">+{file.additions}</b>{" "}
                        <b class="diff-deletions">-{file.deletions}</b>
                      </span>
                    </li>
                  )}
                </For>
              </ul>
            </details>
          </Show>
        </article>
      )}
    </Show>
  );
}
