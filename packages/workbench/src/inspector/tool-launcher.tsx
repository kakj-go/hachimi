import { Button, File, GitPullRequest, Globe, TerminalSquare } from "@hachimi/ui";
import { Show, type JSX } from "solid-js";

import type { InspectorResource } from "../state/workbench-layout";

export function InspectorToolLauncher(props: {
  locale: "zh-CN" | "en-US";
  hasProject: boolean;
  onOpenInspector: (resource: InspectorResource) => void;
  onOpenTerminal: () => void;
}) {
  const zh = () => props.locale === "zh-CN";
  return (
    <div class="inspector-tool-launcher" data-testid="workbench-resource-menu">
      <ToolAction
        icon={<GitPullRequest size={17} />}
        label={zh() ? "审阅" : "Review"}
        shortcut="Ctrl+Shift+G"
        disabled={!props.hasProject}
        onClick={() => props.onOpenInspector({ kind: "review", diffScope: "checkout" })}
      />
      <ToolAction
        icon={<TerminalSquare size={17} />}
        label={zh() ? "终端" : "Terminal"}
        disabled={!props.hasProject}
        onClick={props.onOpenTerminal}
      />
      <ToolAction
        icon={<Globe size={17} />}
        label={zh() ? "浏览器" : "Browser"}
        shortcut="Ctrl+T"
        disabled={!props.hasProject}
        onClick={() => props.onOpenInspector({ kind: "browser" })}
      />
      <ToolAction
        icon={<File size={17} />}
        label={zh() ? "文件" : "Files"}
        shortcut="Ctrl+P"
        disabled={!props.hasProject}
        onClick={() => props.onOpenInspector({ kind: "files" })}
      />
    </div>
  );
}

function ToolAction(props: {
  icon: JSX.Element;
  label: string;
  shortcut?: string;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <Button
      class="workbench-resource-action"
      disabled={props.disabled}
      title={props.disabled ? props.label : undefined}
      onClick={props.onClick}
    >
      {props.icon}
      <span>{props.label}</span>
      <Show when={props.shortcut}>{(shortcut) => <kbd>{shortcut()}</kbd>}</Show>
    </Button>
  );
}
