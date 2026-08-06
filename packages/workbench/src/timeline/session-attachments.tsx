import type { AttachmentRecord, ItemPayload } from "@hachimi/contracts";
import { Button, FileText } from "@hachimi/ui";
import { For, Show } from "solid-js";

export function SessionAttachments(props: {
  payload: ItemPayload;
  attachments: AttachmentRecord[];
  onOpen: (attachment: AttachmentRecord) => void;
}) {
  const ids = () => (props.payload.type === "user" ? props.payload.data.attachment_ids : []);
  const records = () =>
    ids().flatMap((id) => {
      const attachment = props.attachments.find((candidate) => candidate.id === id);
      return attachment ? [attachment] : [];
    });
  return (
    <Show when={records().length > 0}>
      <div class="timeline-attachment-list">
        <For each={records()}>
          {(attachment) => (
            <Button size="small" onClick={() => props.onOpen(attachment)}>
              <FileText size={14} />
              {attachment.originalName}
            </Button>
          )}
        </For>
      </div>
    </Show>
  );
}
