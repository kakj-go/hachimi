import type {
  SessionSourceRecord,
  WorkbenchEnvironmentSnapshot,
  WorkbenchHandoffResponse,
  WorkbenchSessionSnapshot,
} from "@hachimi/contracts";
import { commandFailure } from "@hachimi/contracts";
import { Box, Button, Plus } from "@hachimi/ui";
import { For, Show, createSignal, untrack } from "solid-js";

import { WorkbenchGitControls } from "../git/workbench-git-controls";
import {
  createWorkbenchEnvironmentController,
  type WorkbenchEnvironmentController,
} from "../state/workbench-environment-controller";
import type { InspectorResource } from "../state/workbench-layout";
import type { WorkbenchCommandPort } from "../workbench-command-port";
import { EnvironmentActivityRow } from "./environment-activity-row";
import { EnvironmentLocationMenu } from "./environment-location-menu";
import { EnvironmentSources, sourceTitle } from "./environment-sources";

export function ConnectedEnvironmentSummary(props: {
  snapshot: WorkbenchSessionSnapshot;
  commandPort: WorkbenchCommandPort;
  locale: "zh-CN" | "en-US";
  remotePushEnabled: boolean;
  onOpenInspector: (resource: InspectorResource) => void;
  onHandoff: (response: WorkbenchHandoffResponse) => void;
}) {
  const [handoffBusy, setHandoffBusy] = createSignal(false);
  const [handoffFailure, setHandoffFailure] = createSignal<string>();
  const controller = createWorkbenchEnvironmentController({
    snapshot: () => props.snapshot,
    commandPort: untrack(() => props.commandPort),
    onHandoff: (response) => untrack(() => props.onHandoff(response)),
  });

  async function handoff(kind: "local" | "managed_worktree") {
    setHandoffBusy(true);
    setHandoffFailure(undefined);
    try {
      await controller.handoff(kind);
    } catch (error) {
      setHandoffFailure(commandFailure(error).message);
      throw error;
    } finally {
      setHandoffBusy(false);
    }
  }

  return (
    <Show
      when={controller.environment()}
      fallback={
        <div class="environment-summary environment-summary-loading" aria-busy="true">
          {controller.failure() ??
            (props.locale === "zh-CN" ? "正在读取环境…" : "Loading environment…")}
        </div>
      }
    >
      {(environment) => (
        <EnvironmentSummary
          environment={environment()}
          controller={controller}
          locale={props.locale}
          remotePushEnabled={props.remotePushEnabled}
          handoffBusy={handoffBusy()}
          handoffFailure={handoffFailure()}
          onHandoff={handoff}
          onOpenInspector={props.onOpenInspector}
        />
      )}
    </Show>
  );
}

export function EnvironmentSummary(props: {
  environment: WorkbenchEnvironmentSnapshot;
  controller: WorkbenchEnvironmentController;
  locale: "zh-CN" | "en-US";
  remotePushEnabled: boolean;
  handoffBusy: boolean;
  handoffFailure: string | undefined;
  onHandoff: (kind: "local" | "managed_worktree") => Promise<void>;
  onOpenInspector: (resource: InspectorResource) => void;
}) {
  const zh = () => props.locale === "zh-CN";

  function openSource(source: SessionSourceRecord) {
    if (source.kind === "upload" && source.attachmentId) {
      props.onOpenInspector({
        kind: "attachment",
        attachmentId: source.attachmentId,
        name: sourceTitle(source),
      });
      return;
    }
    if (source.kind === "web" && source.url) {
      props.onOpenInspector({
        kind: "browser",
        ...(source.browserTabId ? { browserTabId: source.browserTabId } : {}),
        initialUrl: source.url,
      });
    }
  }

  return (
    <div class="environment-summary">
      <section class="environment-summary-section">
        <header>
          <strong>{zh() ? "环境信息" : "Environment"}</strong>
          <Button
            aria-label={zh() ? "打开文件" : "Open files"}
            data-testid="workbench-summary-files"
            onClick={() => props.onOpenInspector({ kind: "files" })}
          >
            <Plus size={16} />
          </Button>
        </header>
        <Button
          class="environment-summary-row"
          data-testid="workbench-summary-diff"
          onClick={() => props.onOpenInspector({ kind: "review", diffScope: "session" })}
        >
          <Box size={16} />
          <strong>
            {zh() ? "变更" : "Changes"} · {props.environment.changes.changedFiles}
          </strong>
          <span class="environment-row-tail">
            <b class="diff-additions">+{props.environment.changes.additions}</b>
            <b class="diff-deletions">-{props.environment.changes.deletions}</b>
          </span>
        </Button>
        <Show when={props.environment.checkout}>
          <EnvironmentLocationMenu
            environment={props.environment}
            locale={props.locale}
            busy={props.handoffBusy}
            failure={props.handoffFailure}
            onHandoff={props.onHandoff}
          />
          <WorkbenchGitControls
            environment={props.environment}
            controller={props.controller}
            locale={props.locale}
            remotePushEnabled={props.remotePushEnabled}
            onOpenDiff={(branch, branches) =>
              props.onOpenInspector({
                kind: "review",
                diffScope: "branch",
                ...(branch ? { diffBaseBranch: branch } : {}),
                ...(branches ? { diffBranches: branches } : {}),
              })
            }
          />
        </Show>
      </section>

      <For each={props.environment.activities}>
        {(activity) => (
          <section class="environment-summary-section">
            <header>
              <strong>
                {activity.kind === "browser"
                  ? zh()
                    ? "浏览器"
                    : "Browser"
                  : activity.kind === "computer"
                    ? "Computer Use"
                    : zh()
                      ? "计划"
                      : "Plan"}
              </strong>
            </header>
            <EnvironmentActivityRow
              activity={activity}
              locale={props.locale}
              onOpenPlan={(planId) => props.onOpenInspector({ kind: "plan", planId })}
              onOpenBrowser={(browser) =>
                props.onOpenInspector({
                  kind: "browser",
                  leaseId: browser.lease_id,
                  surface: browser.surface,
                  ...(browser.browser_tab_id ? { browserTabId: browser.browser_tab_id } : {}),
                  ...(browser.browser_session_id
                    ? { browserSessionId: browser.browser_session_id }
                    : {}),
                })
              }
              onOpenComputer={(computer) =>
                props.onOpenInspector({
                  kind: "computer",
                  controlSessionId: computer.control_session_id,
                })
              }
            />
          </section>
        )}
      </For>

      <Show when={props.environment.sources.length > 0}>
        <EnvironmentSources
          sources={props.environment.sources}
          locale={props.locale}
          onOpenSource={openSource}
          onViewAll={() => props.onOpenInspector({ kind: "sources" })}
        />
      </Show>
    </div>
  );
}
