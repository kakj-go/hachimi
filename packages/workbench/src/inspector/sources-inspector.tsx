import { commandFailure, type SessionSourceRecord } from "@hachimi/contracts";
import { Button, File, Globe } from "@hachimi/ui";
import { For, Show, createEffect, createSignal } from "solid-js";

import type { InspectorResource } from "../state/workbench-layout";
import type { WorkbenchCommandPort } from "../workbench-command-port";
import { sourceTitle } from "./environment-sources";

export function SourcesInspector(props: {
  sessionId: string;
  initialSources: SessionSourceRecord[];
  commandPort: WorkbenchCommandPort;
  locale: "zh-CN" | "en-US";
  onOpenInspector: (resource: InspectorResource) => void;
}) {
  const [sources, setSources] = createSignal(props.initialSources);
  const [failure, setFailure] = createSignal<string>();
  const zh = () => props.locale === "zh-CN";

  createEffect(() => {
    const sessionId = props.sessionId;
    void props.commandPort
      .getWorkbenchEnvironment(sessionId)
      .then((environment) => setSources(environment.sources))
      .catch((error) => setFailure(commandFailure(error).message));
  });

  function open(source: SessionSourceRecord) {
    if (source.kind === "upload" && source.attachmentId) {
      props.onOpenInspector({
        kind: "attachment",
        attachmentId: source.attachmentId,
        name: sourceTitle(source),
      });
    } else if (source.kind === "web" && source.url) {
      props.onOpenInspector({
        kind: "browser",
        ...(source.browserTabId ? { browserTabId: source.browserTabId } : {}),
        initialUrl: source.url,
      });
    }
  }

  return (
    <div class="sources-inspector">
      <Show when={failure()}>
        {(message) => <p class="environment-inline-error">{message()}</p>}
      </Show>
      <For
        each={sources()}
        fallback={
          <div class="inspector-empty-state">
            <Globe size={32} />
            <strong>{zh() ? "暂无来源" : "No sources"}</strong>
          </div>
        }
      >
        {(source) => (
          <Button
            class="sources-inspector-row"
            title={sourceTitle(source)}
            onClick={() => open(source)}
          >
            {source.kind === "web" ? <Globe size={17} /> : <File size={17} />}
            <span>
              <strong>{sourceTitle(source)}</strong>
              <small>{source.origin}</small>
            </span>
            <time>{new Date(source.lastUsedAtMs).toLocaleString(props.locale)}</time>
          </Button>
        )}
      </For>
    </div>
  );
}
