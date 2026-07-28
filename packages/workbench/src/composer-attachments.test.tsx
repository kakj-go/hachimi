import { I18nProvider } from "@hachimi/i18n";
import { createSignal, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ComposerAttachmentList, type ComposerAttachmentPreview } from "./composer-attachments";

vi.mock("@hachimi/ui", () => {
  const Icon = () => <span aria-hidden="true" />;
  const AttachmentCard = (props: {
    class?: string;
    image?: boolean;
    kind?: "file" | "folder";
    testId?: string;
    title?: string;
    name: string;
    meta?: string;
    preview?: JSX.Element;
    removeLabel?: string;
    removeClass?: string;
    onRemove?: () => void;
  }) => (
    <article
      class={["attachment-card", props.class, props.image ? "image" : "", props.kind]
        .filter(Boolean)
        .join(" ")}
      data-testid={props.testId}
      title={props.title}
    >
      <span>{props.preview}</span>
      {!props.image && (
        <span>
          <strong>{props.name}</strong>
          <small>{props.meta}</small>
        </span>
      )}
      <button
        type="button"
        class={props.removeClass}
        aria-label={props.removeLabel}
        onClick={() => props.onRemove?.()}
      >
        ×
      </button>
    </article>
  );
  return { AttachmentCard, FileText: Icon, FolderOpen: Icon, X: Icon };
});

const attachments: ComposerAttachmentPreview[] = [
  {
    id: "image",
    sourceKey: "image",
    kind: "file",
    name: "shore.png",
    mimeType: "image/png",
    byteSize: 1024,
    fileCount: 1,
    previewUrl: "data:image/png;base64,iVBORw0KGgo=",
  },
  {
    id: "text",
    sourceKey: "text",
    kind: "file",
    name: "notes.txt",
    mimeType: "text/plain",
    byteSize: 20,
    fileCount: 1,
  },
  {
    id: "folder",
    sourceKey: "folder",
    kind: "folder",
    name: "references",
    mimeType: "inode/directory",
    byteSize: 2048,
    fileCount: 3,
  },
];

afterEach(() => {
  document.body.replaceChildren();
});

describe("ComposerAttachmentList", () => {
  it("renders image, file, and folder cards and removes an individual item", async () => {
    const host = document.createElement("div");
    document.body.append(host);

    function Harness() {
      const [items, setItems] = createSignal(attachments);
      return (
        <I18nProvider initialLocale="zh-CN">
          <ComposerAttachmentList
            attachments={items()}
            onRemove={(attachmentId) =>
              setItems((current) => current.filter((item) => item.id !== attachmentId))
            }
          />
        </I18nProvider>
      );
    }

    const dispose = render(() => <Harness />, host);
    await Promise.resolve();

    expect(host.querySelectorAll(".composer-attachment-card")).toHaveLength(3);
    expect(host.querySelector(".composer-attachment-card.image img")).not.toBeNull();
    expect(host.querySelector(".composer-attachment-card.folder")?.textContent).toContain(
      "3 个文件",
    );

    host
      .querySelector<HTMLButtonElement>(
        '[data-testid="workbench-attachment-text"] .composer-attachment-remove',
      )!
      .click();
    await Promise.resolve();
    expect(host.querySelectorAll(".composer-attachment-card")).toHaveLength(2);
    expect(host.textContent).not.toContain("notes.txt");

    dispose();
  });
});
