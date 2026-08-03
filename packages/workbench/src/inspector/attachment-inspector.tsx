import {
  commandFailure,
  type AttachmentId,
  type WorkbenchAttachmentPreview,
} from "@hachimi/contracts";
import { Badge } from "@hachimi/ui";
import { Match, Show, Switch, createEffect, createSignal } from "solid-js";

import type { WorkbenchCommandPort } from "../workbench-command-port";

export function AttachmentInspector(props: {
  attachmentId: AttachmentId;
  commandPort: WorkbenchCommandPort;
  locale: "zh-CN" | "en-US";
}) {
  const [preview, setPreview] = createSignal<WorkbenchAttachmentPreview>();
  const [failure, setFailure] = createSignal<string>();
  const zh = () => props.locale === "zh-CN";

  createEffect(() => {
    const attachmentId = props.attachmentId;
    setPreview(undefined);
    setFailure(undefined);
    void props.commandPort
      .readWorkbenchAttachment(attachmentId)
      .then(setPreview)
      .catch((error) => setFailure(commandFailure(error).message));
  });

  return (
    <div class="attachment-inspector">
      <Show when={failure()}>{(message) => <p class="composer-error">{message()}</p>}</Show>
      <Show when={preview()}>
        {(value) => (
          <>
            <header>
              <strong>{value().attachment.originalName}</strong>
              <Badge>{value().attachment.mimeType}</Badge>
            </header>
            <Switch
              fallback={
                <p>
                  {zh()
                    ? "该二进制格式暂无内嵌预览，可在输入来源中继续使用。"
                    : "This binary format has no inline preview."}
                </p>
              }
            >
              <Match when={value().dataUrl}>
                {(url) => <img src={url()} alt={value().attachment.originalName} />}
              </Match>
              <Match when={value().utf8Text !== null}>
                <pre>{value().utf8Text}</pre>
              </Match>
            </Switch>
            <Show when={value().truncated}>
              <small>{zh() ? "预览已截断为前 4 MiB。" : "Preview truncated to 4 MiB."}</small>
            </Show>
          </>
        )}
      </Show>
    </div>
  );
}
