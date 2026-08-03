import { AttachmentCard, FileText, FolderOpen } from "@hachimi/ui";
import { For, Show } from "solid-js";

import { useI18n } from "@hachimi/i18n";

export interface ComposerAttachmentPreview {
  id: string;
  sourceKey: string;
  kind: "file" | "folder";
  name: string;
  mimeType: string;
  byteSize: number;
  fileCount: number;
  previewUrl?: string;
  attachmentId?: string;
}

export function ComposerAttachmentList(props: {
  attachments: ComposerAttachmentPreview[];
  onRemove: (attachmentId: string) => void;
  onOpen?: (attachment: ComposerAttachmentPreview) => void;
}) {
  const i18n = useI18n();
  const folderSummary = (count: number) =>
    i18n.t("workbench.attachmentFolderSummary").replace("{count}", String(count));

  return (
    <Show when={props.attachments.length > 0}>
      <div
        class="composer-attachment-list attachment-list"
        role="list"
        aria-label={i18n.t("workbench.attachments")}
      >
        <For each={props.attachments}>
          {(attachment) => {
            const imagePreview = () =>
              attachment.kind === "file" &&
              attachment.mimeType.startsWith("image/") &&
              attachment.previewUrl;
            const typeLabel = () => fileTypeLabel(attachment.name, attachment.mimeType);
            const meta = () =>
              attachment.kind === "folder"
                ? `${folderSummary(attachment.fileCount)} · ${formatBytes(attachment.byteSize)}`
                : `${typeLabel()} · ${formatBytes(attachment.byteSize)}`;
            return (
              <AttachmentCard
                class="composer-attachment-card"
                name={attachment.name}
                meta={meta()}
                image={Boolean(imagePreview())}
                kind={attachment.kind}
                testId={`workbench-attachment-${attachment.id}`}
                title={attachment.name}
                preview={
                  <Show
                    when={imagePreview()}
                    fallback={
                      <Show when={attachment.kind === "folder"} fallback={<FileText size={22} />}>
                        <FolderOpen size={22} />
                      </Show>
                    }
                  >
                    {(previewUrl) => <img src={previewUrl()} alt="" />}
                  </Show>
                }
                removeLabel={`${i18n.t("workbench.removeAttachment")}: ${attachment.name}`}
                removeClass="composer-attachment-remove"
                onRemove={() => props.onRemove(attachment.id)}
                {...(props.onOpen ? { onOpen: () => props.onOpen!(attachment) } : {})}
              />
            );
          }}
        </For>
      </div>
    </Show>
  );
}

function fileTypeLabel(name: string, mimeType: string): string {
  const extension = name.includes(".") ? name.split(".").pop()?.trim() : undefined;
  if (extension) return extension.slice(0, 8).toLocaleUpperCase();
  const mimeSubtype = mimeType.split("/")[1]?.split("+")[0]?.trim();
  return mimeSubtype ? mimeSubtype.slice(0, 8).toLocaleUpperCase() : "FILE";
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
