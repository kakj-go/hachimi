import type { JSX } from "solid-js";

import type { InspectorTab } from "../state/inspector-tabs";
import { InspectorWorkspaceTabs } from "./workspace-tabs";

export function InspectorShell(props: {
  title: string;
  resourceKind?: "tools" | "resource";
  wide?: boolean;
  tabs: InspectorTab[];
  activeTabId: string | undefined;
  locale: "zh-CN" | "en-US";
  onSelectTab: (tabId: string) => void;
  onCloseTab: (tabId: string) => void;
  onOpenLauncher: () => void;
  children: JSX.Element;
}) {
  return (
    <aside
      class="workbench-inspector"
      classList={{ "workbench-inspector-wide": props.wide }}
      data-resource={props.resourceKind ?? "resource"}
      aria-label={props.title}
    >
      <InspectorWorkspaceTabs
        tabs={props.tabs}
        activeTabId={props.activeTabId}
        launcherVisible={props.resourceKind === "tools"}
        locale={props.locale}
        onSelect={props.onSelectTab}
        onClose={props.onCloseTab}
        onOpenLauncher={props.onOpenLauncher}
      />
      <div class="workbench-inspector-body">{props.children}</div>
    </aside>
  );
}
