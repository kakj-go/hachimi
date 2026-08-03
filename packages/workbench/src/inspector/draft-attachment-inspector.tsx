import { MessageCircle, RefreshCw } from "@hachimi/ui";
import { Show } from "solid-js";

import type { WorkbenchCommandPort } from "../workbench-command-port";
import type { InspectorResource } from "../state/workbench-layout";
import { AttachmentInspector } from "./attachment-inspector";
import { InspectorShell } from "./inspector-shell";
import { InspectorToolLauncher } from "./tool-launcher";

export function DraftAttachmentInspector(props: {
  resource: InspectorResource | undefined;
  hasProject: boolean;
  loading: boolean;
  commandPort: WorkbenchCommandPort;
  locale: "zh-CN" | "en-US";
  onOpenInspector: (resource: InspectorResource) => void;
  onOpenTerminal: () => void;
}) {
  const attachment = () => (props.resource?.kind === "attachment" ? props.resource : undefined);
  const tools = () => props.resource?.kind === "tools";

  return (
    <InspectorShell
      title={
        attachment()?.name ??
        (tools()
          ? props.locale === "zh-CN"
            ? "工具"
            : "Tools"
          : props.locale === "zh-CN"
            ? "工作区工具"
            : "Workspace tools")
      }
      resourceKind={tools() ? "tools" : "resource"}
      wide={tools()}
    >
      <Show
        when={attachment()}
        fallback={
          <Show
            when={tools()}
            fallback={
              <div class="inspector-empty-state">
                <Show when={props.loading} fallback={<MessageCircle size={34} />}>
                  <RefreshCw class="workbench-spin" size={34} />
                </Show>
                <strong>
                  {props.loading
                    ? props.locale === "zh-CN"
                      ? "正在准备项目工具"
                      : "Preparing project tools"
                    : props.locale === "zh-CN"
                      ? "选择右侧工作区中的工具"
                      : "Choose a tool in the right workspace"}
                </strong>
              </div>
            }
          >
            <InspectorToolLauncher
              locale={props.locale}
              hasProject={props.hasProject}
              onOpenInspector={props.onOpenInspector}
              onOpenTerminal={props.onOpenTerminal}
            />
          </Show>
        }
      >
        {(resource) => (
          <AttachmentInspector
            attachmentId={resource().attachmentId}
            commandPort={props.commandPort}
            locale={props.locale}
          />
        )}
      </Show>
    </InspectorShell>
  );
}
