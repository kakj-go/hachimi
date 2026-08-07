import type { SkillEditorKind, SkillPreviewResource } from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import { Button, Dialog, SelectField, StatusBanner, TextField } from "@hachimi/ui";
import {
  For,
  Show,
  createEffect,
  createMemo,
  createResource,
  createSignal,
  onCleanup,
  onMount,
} from "solid-js";

export function TextEditor(props: {
  path: string;
  kind: SkillEditorKind;
  value: string;
  contentAvailable?: boolean;
  dirty: boolean;
  saving: boolean;
  readOnly?: boolean;
  conflict: boolean;
  diagnostics: readonly { severity: string; message: string }[];
  onInput: (value: string) => void;
  onSave: () => void;
  onReload: () => void;
  onKeepLocal: () => void;
  resolveReference: (destination: string) => Promise<SkillPreviewResource>;
  referenceFiles: readonly string[];
}) {
  const i18n = useI18n();
  const copy = (zh: string, en: string) => (i18n.locale() === "zh-CN" ? zh : en);
  const references = createMemo(() => extractMarkdownReferences(props.value));
  const shortcut = (event: KeyboardEvent) => {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
      event.preventDefault();
      if (!props.readOnly && props.kind === "markdown" && props.dirty && !props.saving)
        props.onSave();
    }
  };
  onMount(() => window.addEventListener("keydown", shortcut));
  onCleanup(() => window.removeEventListener("keydown", shortcut));

  return (
    <section class="skill-editor" aria-label={props.path}>
      <header class="skill-editor-header">
        <div>
          <strong>{props.path}</strong>
          <span>
            {props.kind === "markdown"
              ? props.readOnly
                ? copy("只读", "Read only")
                : props.dirty
                  ? copy("未保存", "Unsaved")
                  : copy("已保存", "Saved")
              : copy("只读", "Read only")}
          </span>
        </div>
        <Show when={props.kind === "markdown" && !props.readOnly}>
          <Button
            data-testid="skill-save"
            size="small"
            variant="primary"
            disabled={!props.dirty || props.saving}
            onClick={props.onSave}
          >
            {props.saving ? copy("保存中…", "Saving…") : copy("保存", "Save")}
          </Button>
        </Show>
      </header>
      <Show when={props.conflict}>
        <StatusBanner tone="warning">
          {copy(
            "文件已在外部修改。当前草稿不会被覆盖。",
            "The file changed externally. Your draft was not overwritten.",
          )}
          <Button size="small" onClick={props.onReload}>
            {copy("重新加载磁盘版本", "Reload disk version")}
          </Button>
          <Button size="small" data-testid="skill-conflict-keep-local" onClick={props.onKeepLocal}>
            {copy("保留本地草稿", "Keep local draft")}
          </Button>
        </StatusBanner>
      </Show>
      <Show when={props.diagnostics.length > 0}>
        <div class="skill-editor-diagnostics" role="status">
          <For each={props.diagnostics}>
            {(diagnostic) => <span data-tone={diagnostic.severity}>{diagnostic.message}</span>}
          </For>
        </div>
      </Show>
      <Show
        when={props.kind !== "unsupported" || props.contentAvailable !== false}
        fallback={
          <div class="skill-editor-unsupported">
            {copy(
              "该文件无法作为受支持的文本安全显示。",
              "This file cannot be displayed safely as supported text.",
            )}
          </div>
        }
      >
        <Show
          when={props.kind === "markdown" && !props.readOnly}
          fallback={<ReadOnlyCode path={props.path} value={props.value} copy={copy} />}
        >
          <MarkdownWysiwyg
            path={props.path}
            value={props.value}
            onInput={props.onInput}
            isEntry={props.path === "SKILL.md"}
            referenceFiles={props.referenceFiles}
            copy={copy}
          />
          <Show when={references().length > 0}>
            <section class="skill-markdown-resources" aria-label={copy("引用资源", "References")}>
              <h3>{copy("引用资源", "Referenced resources")}</h3>
              <For each={references()}>
                {(reference) => (
                  <MarkdownResourcePreview reference={reference} resolve={props.resolveReference} />
                )}
              </For>
            </section>
          </Show>
        </Show>
      </Show>
    </section>
  );
}

function ReadOnlyCode(props: {
  path: string;
  value: string;
  copy: (zh: string, en: string) => string;
}) {
  const lines = createMemo(() => props.value.replace(/\r\n?/g, "\n").split("\n"));
  return (
    <div
      class="skill-readonly-code"
      data-testid="skill-text-viewer"
      aria-label={props.copy(`${props.path} 只读文本`, `${props.path} read-only text`)}
    >
      <pre class="skill-line-numbers" aria-hidden="true">
        {lines()
          .map((_, index) => index + 1)
          .join("\n")}
      </pre>
      <pre>
        <code>{props.value}</code>
      </pre>
    </div>
  );
}

interface MarkdownDocument {
  frontmatter: string[] | null;
  name: string;
  displayName: string;
  description: string;
  body: string;
}

function MarkdownWysiwyg(props: {
  path: string;
  value: string;
  onInput: (value: string) => void;
  isEntry: boolean;
  referenceFiles: readonly string[];
  copy: (zh: string, en: string) => string;
}) {
  let editor!: HTMLDivElement;
  let lastEmitted = "";
  let savedSelection: Range | undefined;
  const [documentState, setDocumentState] = createSignal(parseMarkdownDocument(""));
  const [referenceOpen, setReferenceOpen] = createSignal(false);
  const [referenceLabel, setReferenceLabel] = createSignal("");
  const [referencePath, setReferencePath] = createSignal("");

  function emit(nextState = documentState()) {
    const body = editor ? editorToMarkdown(editor) : nextState.body;
    const source = composeMarkdownDocument({ ...nextState, body });
    lastEmitted = source;
    props.onInput(source);
  }

  function format(command: string, value?: string) {
    editor.focus();
    document.execCommand(command, false, value);
    emit();
  }

  function openReferenceDialog() {
    const selection = window.getSelection();
    savedSelection = selection?.rangeCount ? selection.getRangeAt(0).cloneRange() : undefined;
    setReferenceLabel(selection?.toString() ?? "");
    setReferencePath(props.referenceFiles[0] ?? "");
    setReferenceOpen(true);
  }

  function insertReference() {
    const destination = referencePath().trim();
    if (!destination) return;
    editor.focus();
    const selection = window.getSelection();
    if (selection && savedSelection) {
      selection.removeAllRanges();
      selection.addRange(savedSelection);
    }
    if (!selection?.toString()) {
      document.execCommand("insertText", false, referenceLabel().trim() || destination);
    }
    document.execCommand("createLink", false, destination);
    const anchor = selection?.anchorNode?.parentElement?.closest("a");
    anchor?.setAttribute("data-destination", destination);
    anchor?.setAttribute("href", "#");
    setReferenceOpen(false);
    emit();
  }

  createEffect(() => {
    const incoming = props.value;
    if (!editor || incoming === lastEmitted) return;
    const parsed = parseMarkdownDocument(incoming, props.isEntry);
    setDocumentState(parsed);
    editor.innerHTML = markdownBodyToSafeHtml(parsed.body);
  });

  onMount(() => {
    const parsed = parseMarkdownDocument(props.value, props.isEntry);
    setDocumentState(parsed);
    editor.innerHTML = markdownBodyToSafeHtml(parsed.body);
    editor.focus();
  });

  return (
    <div class="skill-wysiwyg">
      <Show when={props.isEntry && documentState().frontmatter !== null}>
        <div class="skill-wysiwyg-metadata">
          <TextField
            label={props.copy("名称", "Name")}
            value={documentState().name}
            disabled
            description={props.copy("由技能目录名称决定", "Defined by the Skill directory")}
          />
          <TextField
            label={props.copy("别名", "Alias")}
            value={documentState().displayName}
            placeholder={props.copy("面向用户显示的简洁名称", "Concise user-facing name")}
            onInput={(event) => {
              const next = { ...documentState(), displayName: event.currentTarget.value };
              setDocumentState(next);
              emit(next);
            }}
          />
          <div class="skill-wysiwyg-description">
            <TextField
              label={props.copy("描述", "Description")}
              value={documentState().description}
              onInput={(event) => {
                const next = { ...documentState(), description: event.currentTarget.value };
                setDocumentState(next);
                emit(next);
              }}
            />
          </div>
        </div>
      </Show>
      <div
        class="skill-wysiwyg-toolbar"
        role="toolbar"
        aria-label={props.copy("文本格式", "Text formatting")}
      >
        <Button size="small" onClick={() => format("formatBlock", "P")}>
          {props.copy("正文", "Text")}
        </Button>
        <Button size="small" onClick={() => format("formatBlock", "H2")}>
          {props.copy("标题", "Heading")}
        </Button>
        <Button size="small" aria-label={props.copy("粗体", "Bold")} onClick={() => format("bold")}>
          <strong>B</strong>
        </Button>
        <Button
          size="small"
          aria-label={props.copy("斜体", "Italic")}
          onClick={() => format("italic")}
        >
          <em>I</em>
        </Button>
        <Button size="small" onClick={() => format("insertUnorderedList")}>
          {props.copy("列表", "List")}
        </Button>
        <Button size="small" onClick={() => format("formatBlock", "PRE")}>
          {props.copy("代码块", "Code")}
        </Button>
        <Button
          size="small"
          disabled={props.referenceFiles.length === 0}
          onClick={openReferenceDialog}
        >
          {props.copy("引用文件", "Reference")}
        </Button>
      </div>
      <div
        ref={editor}
        class="skill-wysiwyg-canvas"
        data-testid="skill-markdown-editor"
        contentEditable
        role="textbox"
        aria-multiline="true"
        aria-label={props.copy(`${props.path} 富文本编辑器`, `${props.path} rich text editor`)}
        spellcheck
        onInput={() => emit()}
        onClick={(event) => {
          if ((event.target as HTMLElement).closest("a")) event.preventDefault();
        }}
        onPaste={(event) => {
          event.preventDefault();
          document.execCommand(
            "insertText",
            false,
            event.clipboardData?.getData("text/plain") ?? "",
          );
        }}
      />
      <Dialog
        open={referenceOpen()}
        title={props.copy("引用 Skill 文件", "Reference a Skill file")}
        description={props.copy(
          "只能引用当前 Skill 目录内的相对路径。",
          "Only relative paths inside this Skill can be referenced.",
        )}
        onOpenChange={setReferenceOpen}
        closeLabel={props.copy("关闭", "Close")}
      >
        <div class="dialog-form">
          <TextField
            label={props.copy("显示文字", "Label")}
            value={referenceLabel()}
            onInput={(event) => setReferenceLabel(event.currentTarget.value)}
          />
          <SelectField
            label={props.copy("选择文件", "Choose file")}
            value={referencePath()}
            placeholder={props.copy("选择当前 Skill 中的文件", "Choose a file in this Skill")}
            options={props.referenceFiles.map((path) => ({ value: path, label: path }))}
            description={props.copy(
              "列表仅包含当前 Markdown 可以安全引用的文件。",
              "Only files this Markdown document can reference safely are listed.",
            )}
            onChange={setReferencePath}
          />
          <div class="dialog-actions">
            <Button onClick={() => setReferenceOpen(false)}>{props.copy("取消", "Cancel")}</Button>
            <Button variant="primary" disabled={!referencePath().trim()} onClick={insertReference}>
              {props.copy("插入引用", "Insert reference")}
            </Button>
          </div>
        </div>
      </Dialog>
    </div>
  );
}

function parseMarkdownDocument(source: string, parseEntryFrontmatter = true): MarkdownDocument {
  const normalized = source.replace(/\r\n?/g, "\n");
  if (!parseEntryFrontmatter || !normalized.startsWith("---\n")) {
    return { frontmatter: null, name: "", displayName: "", description: "", body: normalized };
  }
  const end = normalized.indexOf("\n---", 4);
  if (end < 0)
    return { frontmatter: null, name: "", displayName: "", description: "", body: normalized };
  const frontmatter = normalized.slice(4, end).split("\n");
  const body = normalized.slice(end + 4).replace(/^\n+/, "");
  const field = (name: string) =>
    frontmatter
      .find((line) => line.startsWith(`${name}:`))
      ?.slice(name.length + 1)
      .trim()
      .replace(/^(['"])(.*)\1$/, "$2") ?? "";
  return {
    frontmatter,
    name: field("name"),
    displayName: field("display_name"),
    description: field("description"),
    body,
  };
}

function composeMarkdownDocument(value: MarkdownDocument): string {
  const body = value.body.trimEnd();
  if (!value.frontmatter) return body ? `${body}\n` : "";
  let hasName = false;
  let hasDisplayName = false;
  let hasDescription = false;
  const frontmatter = value.frontmatter.map((line) => {
    if (line.startsWith("name:")) {
      hasName = true;
      return `name: ${value.name}`;
    }
    if (line.startsWith("display_name:")) {
      hasDisplayName = true;
      return `display_name: ${value.displayName.replace(/\s+/g, " ").trim()}`;
    }
    if (line.startsWith("description:")) {
      hasDescription = true;
      return `description: ${value.description.replace(/\s+/g, " ").trim()}`;
    }
    return line;
  });
  if (!hasName) frontmatter.push(`name: ${value.name}`);
  if (!hasDisplayName && value.displayName.trim())
    frontmatter.push(`display_name: ${value.displayName.replace(/\s+/g, " ").trim()}`);
  if (!hasDescription)
    frontmatter.push(`description: ${value.description.replace(/\s+/g, " ").trim()}`);
  return `---\n${frontmatter.join("\n")}\n---\n\n${body}${body ? "\n" : ""}`;
}

function markdownBodyToSafeHtml(source: string): string {
  const lines = source.replace(/\r\n?/g, "\n").split("\n");
  const html: string[] = [];
  for (let index = 0; index < lines.length; ) {
    const line = lines[index] ?? "";
    if (!line.trim()) {
      index += 1;
      continue;
    }
    if (line.startsWith("```")) {
      const code: string[] = [];
      index += 1;
      while (index < lines.length && !(lines[index] ?? "").startsWith("```")) {
        code.push(lines[index] ?? "");
        index += 1;
      }
      if (index < lines.length) index += 1;
      html.push(`<pre><code>${escapeHtml(code.join("\n"))}</code></pre>`);
      continue;
    }
    const heading = /^(#{1,6})\s+(.+)$/.exec(line);
    if (heading) {
      const level = heading[1]?.length ?? 2;
      html.push(`<h${level}>${inlineMarkdownToSafeHtml(heading[2] ?? "")}</h${level}>`);
      index += 1;
      continue;
    }
    if (/^[-*]\s+/.test(line)) {
      const items: string[] = [];
      while (index < lines.length && /^[-*]\s+/.test(lines[index] ?? "")) {
        items.push(
          `<li>${inlineMarkdownToSafeHtml((lines[index] ?? "").replace(/^[-*]\s+/, ""))}</li>`,
        );
        index += 1;
      }
      html.push(`<ul>${items.join("")}</ul>`);
      continue;
    }
    const paragraph = [line];
    index += 1;
    while (
      index < lines.length &&
      (lines[index] ?? "").trim() &&
      !/^(#{1,6})\s+/.test(lines[index] ?? "") &&
      !/^[-*]\s+/.test(lines[index] ?? "") &&
      !(lines[index] ?? "").startsWith("```")
    ) {
      paragraph.push(lines[index] ?? "");
      index += 1;
    }
    html.push(`<p>${paragraph.map(inlineMarkdownToSafeHtml).join("<br>")}</p>`);
  }
  return html.join("") || "<p><br></p>";
}

function inlineMarkdownToSafeHtml(source: string): string {
  let value = escapeHtml(source);
  value = value.replace(
    /\[([^\]]+)\]\(([^)\s]+)\)/g,
    (_match, label: string, destination: string) =>
      `<a href="#" data-destination="${escapeAttribute(destination)}">${label}</a>`,
  );
  value = value.replace(/`([^`]+)`/g, "<code>$1</code>");
  value = value.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  value = value.replace(/\*([^*]+)\*/g, "<em>$1</em>");
  return value;
}

function editorToMarkdown(editor: HTMLElement): string {
  const blocks = [...editor.childNodes]
    .map((node) => blockToMarkdown(node))
    .filter((value) => value.length > 0);
  return blocks.join("\n\n");
}

function blockToMarkdown(node: Node): string {
  if (node.nodeType === Node.TEXT_NODE) return escapeMarkdownText(node.textContent ?? "").trim();
  if (!(node instanceof HTMLElement)) return "";
  const tag = node.tagName.toLowerCase();
  if (/^h[1-6]$/.test(tag)) {
    return `${"#".repeat(Number(tag[1]))} ${inlineNodeToMarkdown(node).trim()}`;
  }
  if (tag === "pre") return `\`\`\`\n${node.textContent ?? ""}\n\`\`\``;
  if (tag === "ul" || tag === "ol") {
    return [...node.children]
      .filter((child) => child.tagName.toLowerCase() === "li")
      .map(
        (child, index) =>
          `${tag === "ol" ? `${index + 1}.` : "-"} ${inlineNodeToMarkdown(child).trim()}`,
      )
      .join("\n");
  }
  return inlineNodeToMarkdown(node).trim();
}

function inlineNodeToMarkdown(node: Node): string {
  return [...node.childNodes]
    .map((child) => {
      if (child.nodeType === Node.TEXT_NODE) return escapeMarkdownText(child.textContent ?? "");
      if (!(child instanceof HTMLElement)) return "";
      const content = inlineNodeToMarkdown(child);
      switch (child.tagName.toLowerCase()) {
        case "br":
          return "\n";
        case "strong":
        case "b":
          return `**${content}**`;
        case "em":
        case "i":
          return `*${content}*`;
        case "code":
          return `\`${child.textContent ?? ""}\``;
        case "a": {
          const destination = child.dataset.destination || child.getAttribute("href") || "";
          return destination && destination !== "#" ? `[${content}](${destination})` : content;
        }
        default:
          return content;
      }
    })
    .join("");
}

function escapeMarkdownText(value: string): string {
  return value.replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function escapeAttribute(value: string): string {
  return escapeHtml(value).replace(/`/g, "&#96;");
}

interface MarkdownReference {
  label: string;
  destination: string;
}

function extractMarkdownReferences(source: string): MarkdownReference[] {
  const references = new Map<string, MarkdownReference>();
  const pattern = /!?\[([^\]]*)\]\(([^)]+)\)/g;
  for (const match of source.matchAll(pattern)) {
    const destination = (match[2] ?? "").trim().split(/\s+/)[0] ?? "";
    if (!destination || destination.startsWith("#")) continue;
    references.set(destination, { label: match[1] || destination, destination });
  }
  return [...references.values()];
}

function MarkdownResourcePreview(props: {
  reference: MarkdownReference;
  resolve: (destination: string) => Promise<SkillPreviewResource>;
}) {
  const i18n = useI18n();
  const copy = (zh: string, en: string) => (i18n.locale() === "zh-CN" ? zh : en);
  const [resource] = createResource(
    () => props.reference.destination,
    // eslint-disable-next-line solid/reactivity -- createResource tracks the source accessor.
    (destination) => props.resolve(destination),
  );
  return (
    <div class="skill-markdown-resource">
      <strong>{props.reference.label}</strong>
      <code>{props.reference.destination}</code>
      <Show when={resource.loading}>
        <span>{copy("正在通过 SkillHost 加载…", "Loading through SkillHost…")}</span>
      </Show>
      <Show when={resource.error}>
        <span data-tone="error">{copy("引用无法加载", "Reference could not be loaded")}</span>
      </Show>
      <Show when={resource()?.text !== null && resource()?.text !== undefined}>
        <details>
          <summary>{copy("查看引用文本", "View referenced text")}</summary>
          <pre>{resource()?.text}</pre>
        </details>
      </Show>
    </div>
  );
}
