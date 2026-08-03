import type { WorkbenchSessionSnapshot } from "@hachimi/contracts";
import { Badge } from "@hachimi/ui";
import { Show, createMemo } from "solid-js";

import { timelineItemText } from "../timeline/timeline-text";
import type { InspectorResource } from "../state/workbench-layout";
import type { WorkbenchCommandPort } from "../workbench-command-port";
import { WorkspaceBrowser } from "../workspace-browser";
import { AttachmentInspector } from "./attachment-inspector";
import { BrowserInspector } from "./browser-inspector";
import { InspectorShell } from "./inspector-shell";
import { SourcesInspector } from "./sources-inspector";
import { InspectorToolLauncher } from "./tool-launcher";

export function SessionInspector(props: {
  snapshot: WorkbenchSessionSnapshot;
  resource: InspectorResource | undefined;
  commandPort: WorkbenchCommandPort;
  locale: "zh-CN" | "en-US";
  onOpenInspector: (resource: InspectorResource) => void;
  onOpenTerminal: () => void;
}) {
  const zh = () => props.locale === "zh-CN";
  const review = createMemo(() => (props.resource?.kind === "review" ? props.resource : undefined));
  const files = createMemo(() => (props.resource?.kind === "files" ? props.resource : undefined));
  const attachment = createMemo(() =>
    props.resource?.kind === "attachment" ? props.resource : undefined,
  );
  const artifact = createMemo(() =>
    (() => {
      const resource = props.resource;
      return resource?.kind === "artifact"
        ? props.snapshot.artifacts.find((item) => item.id === resource.artifactId)
        : undefined;
    })(),
  );
  const plan = createMemo(() =>
    (() => {
      const resource = props.resource;
      return resource?.kind === "plan"
        ? props.snapshot.proposedPlans.find((item) => item.id === resource.planId)
        : undefined;
    })(),
  );
  const title = () => {
    if (props.resource?.kind === "tools") return zh() ? "工具" : "Tools";
    if (props.resource?.kind === "plan") return zh() ? "计划" : "Plan";
    if (props.resource?.kind === "attachment") return props.resource.name;
    if (props.resource?.kind === "artifact") return artifact()?.displayName ?? "Artifact";
    if (props.resource?.kind === "review") return zh() ? "审阅" : "Review";
    if (props.resource?.kind === "files") return zh() ? "文件" : "Files";
    if (props.resource?.kind === "browser") return zh() ? "浏览器" : "Browser";
    if (props.resource?.kind === "sources") return zh() ? "来源" : "Sources";
    return zh() ? "工作区" : "Workspace";
  };

  return (
    <InspectorShell
      title={title()}
      resourceKind={props.resource?.kind === "tools" ? "tools" : "resource"}
      wide={
        props.resource?.kind === "tools" ||
        props.resource?.kind === "review" ||
        props.resource?.kind === "files" ||
        props.resource?.kind === "browser" ||
        props.resource?.kind === "sources"
      }
    >
      <Show when={props.resource?.kind === "tools"}>
        <InspectorToolLauncher
          locale={props.locale}
          hasProject={props.snapshot.session.context.kind === "project"}
          onOpenInspector={props.onOpenInspector}
          onOpenTerminal={props.onOpenTerminal}
        />
      </Show>
      <Show when={review()}>
        {(resource) => (
          <WorkspaceBrowser
            mode="review"
            snapshot={props.snapshot}
            commandPort={props.commandPort}
            {...(resource().path ? { initialPath: resource().path } : {})}
            {...(resource().diffRunId ? { initialDiffRunId: resource().diffRunId } : {})}
            {...(resource().diffBaseBranch
              ? { initialDiffBaseBranch: resource().diffBaseBranch }
              : {})}
            {...(resource().diffBranches ? { diffBranches: resource().diffBranches } : {})}
            {...(resource().diffScope ? { initialDiffScope: resource().diffScope } : {})}
          />
        )}
      </Show>
      <Show when={files()}>
        {(resource) => (
          <WorkspaceBrowser
            mode="files"
            snapshot={props.snapshot}
            commandPort={props.commandPort}
            {...(resource().path ? { initialPath: resource().path } : {})}
          />
        )}
      </Show>
      <Show when={attachment()}>
        {(resource) => (
          <AttachmentInspector
            attachmentId={resource().attachmentId}
            commandPort={props.commandPort}
            locale={props.locale}
          />
        )}
      </Show>
      <Show when={artifact()}>
        {(item) => (
          <div class="inspector-artifact-view">
            <Badge>{item().kind}</Badge>
            <h3>{item().displayName}</h3>
            <pre>{timelineItemText(item().metadata)}</pre>
          </div>
        )}
      </Show>
      <Show when={plan()}>
        {(item) => (
          <div class="inspector-plan-view">
            <pre>{item().contentMarkdown}</pre>
          </div>
        )}
      </Show>
      <Show when={props.resource?.kind === "browser"}>
        <BrowserInspector
          snapshot={props.snapshot}
          commandPort={props.commandPort}
          locale={props.locale}
          {...(props.resource?.kind === "browser" && props.resource.browserTabId
            ? { browserTabId: props.resource.browserTabId }
            : {})}
          {...(props.resource?.kind === "browser" && props.resource.initialUrl
            ? { initialUrl: props.resource.initialUrl }
            : {})}
        />
      </Show>
      <Show when={props.resource?.kind === "sources"}>
        <SourcesInspector
          sessionId={props.snapshot.session.id}
          initialSources={props.snapshot.sources}
          commandPort={props.commandPort}
          locale={props.locale}
          onOpenInspector={props.onOpenInspector}
        />
      </Show>
    </InspectorShell>
  );
}
