import { Box, Button, File, GitPullRequest, Globe, Monitor, Paperclip, Plus, X } from "@hachimi/ui";
import { For, Match, Switch } from "solid-js";

import type { InspectorTab } from "../state/inspector-tabs";
import { inspectorTabLabel } from "../state/inspector-tabs";
import type { InspectorResource } from "../state/workbench-layout";

export function InspectorWorkspaceTabs(props: {
  tabs: InspectorTab[];
  activeTabId: string | undefined;
  launcherVisible: boolean;
  locale: "zh-CN" | "en-US";
  onSelect: (tabId: string) => void;
  onClose: (tabId: string) => void;
  onOpenLauncher: () => void;
}) {
  const zh = () => props.locale === "zh-CN";
  return (
    <div class="inspector-workspace-tabs">
      <div class="inspector-workspace-tab-strip" role="tablist">
        <For each={props.tabs}>
          {(tab) => (
            <div
              class="inspector-workspace-tab"
              classList={{
                active: !props.launcherVisible && tab.id === props.activeTabId,
              }}
              role="tab"
              aria-selected={!props.launcherVisible && tab.id === props.activeTabId}
            >
              <Button
                class="inspector-workspace-tab-select"
                title={inspectorTabLabel(tab.resource, props.locale)}
                data-testid={`workbench-inspector-tab-${tab.id}`}
                onClick={() => props.onSelect(tab.id)}
              >
                <ResourceIcon resource={tab.resource} />
                <span>{inspectorTabLabel(tab.resource, props.locale)}</span>
              </Button>
              <Button
                class="inspector-workspace-tab-close"
                aria-label={zh() ? "关闭工作区标签" : "Close workspace tab"}
                title={zh() ? "关闭工作区标签" : "Close workspace tab"}
                data-testid={`workbench-inspector-tab-close-${tab.id}`}
                onClick={() => props.onClose(tab.id)}
              >
                <X size={13} />
              </Button>
            </div>
          )}
        </For>
      </div>
      <Button
        class="inspector-workspace-new-tab"
        classList={{ active: props.launcherVisible }}
        aria-label={zh() ? "新建工作区标签" : "New workspace tab"}
        title={zh() ? "新建工作区标签" : "New workspace tab"}
        data-testid="workbench-inspector-new-tab"
        onClick={props.onOpenLauncher}
      >
        <Plus size={16} />
      </Button>
    </div>
  );
}

function ResourceIcon(props: { resource: InspectorResource }) {
  return (
    <Switch fallback={<Box size={14} />}>
      <Match when={props.resource.kind === "review"}>
        <GitPullRequest size={14} />
      </Match>
      <Match when={props.resource.kind === "files"}>
        <File size={14} />
      </Match>
      <Match when={props.resource.kind === "browser"}>
        <Globe size={14} />
      </Match>
      <Match when={props.resource.kind === "computer"}>
        <Monitor size={14} />
      </Match>
      <Match when={props.resource.kind === "attachment"}>
        <Paperclip size={14} />
      </Match>
    </Switch>
  );
}
