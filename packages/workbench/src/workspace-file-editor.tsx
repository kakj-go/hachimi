import {
  AlertTriangle,
  Button,
  Check,
  Code2,
  ComposerInput,
  File,
  FileText,
  RefreshCw,
  X,
} from "@hachimi/ui";
import { Show, createEffect, createSignal, onCleanup, untrack } from "solid-js";

import { MarkdownContent } from "./timeline/message-markdown";
import "./workspace-file-editor.css";

interface MonacoDisposable {
  dispose(): void;
}

interface MonacoEditor {
  getValue(): string;
  setValue(value: string): void;
  layout(): void;
  addAction(action: {
    id: string;
    label: string;
    keybindings: number[];
    run: () => void;
  }): MonacoDisposable;
  onDidChangeModelContent(listener: () => void): MonacoDisposable;
  dispose(): void;
}

export function WorkspaceFileEditor(props: {
  path: string;
  value: string;
  editable: boolean;
  dirty: boolean;
  saving: boolean;
  conflict: boolean;
  readOnlyMessage?: string | undefined;
  showHeader?: boolean;
  workspaceRoot: string | undefined;
  locale: "zh-CN" | "en-US";
  onInput: (value: string) => void;
  onSave: () => void;
  onReload: () => void;
  onKeepLocal: () => void;
  onClose: () => void;
  onOpenPath: ((path: string) => void) | undefined;
}) {
  const [monacoReady, setMonacoReady] = createSignal(false);
  const [showMarkdownSource, setShowMarkdownSource] = createSignal(false);
  const zh = () => props.locale === "zh-CN";
  const markdownFile = () => isMarkdownPath(props.path);
  let container: HTMLDivElement | undefined;
  let editor: MonacoEditor | undefined;
  let contentChange: MonacoDisposable | undefined;
  let saveAction: MonacoDisposable | undefined;
  let themeObserver: MutationObserver | undefined;
  let disposed = false;
  let applyingValue = false;
  let loadingMonaco = false;

  createEffect(() => {
    const sourceVisible = !isMarkdownPath(props.path) || showMarkdownSource();
    if (
      !sourceVisible ||
      !props.editable ||
      editor ||
      loadingMonaco ||
      !container ||
      typeof ResizeObserver !== "function"
    ) {
      return;
    }
    loadingMonaco = true;
    const initialPath = untrack(() => props.path);
    const saveLabel = untrack(() => (zh() ? "保存文件" : "Save file"));
    const onInput = props.onInput;
    const onSave = props.onSave;
    void import("monaco-editor/esm/vs/editor/editor.api")
      .then((monaco) => {
        if (disposed || !container) return;
        editor = monaco.editor.create(container, {
          value: untrack(() => props.value),
          language: languageForPath(initialPath),
          theme: monacoTheme(),
          automaticLayout: true,
          minimap: { enabled: false },
          fontFamily: "var(--font-code)",
          fontSize: 12,
          lineHeight: 19,
          scrollBeyondLastLine: false,
          renderWhitespace: "selection",
          wordWrap: "off",
          accessibilitySupport: "auto",
          tabSize: 2,
          insertSpaces: true,
        }) as MonacoEditor;
        if (typeof MutationObserver === "function") {
          themeObserver = new MutationObserver(() => monaco.editor.setTheme(monacoTheme()));
          themeObserver.observe(document.documentElement, {
            attributes: true,
            attributeFilter: ["data-color-scheme"],
          });
        }
        contentChange = editor.onDidChangeModelContent(() => {
          if (!applyingValue) onInput(editor?.getValue() ?? "");
        });
        saveAction = editor.addAction({
          id: "hachimi.workspace.save",
          label: saveLabel,
          keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS],
          run: onSave,
        });
        setMonacoReady(true);
        editor.layout();
      })
      .catch(() => setMonacoReady(false))
      .finally(() => {
        loadingMonaco = false;
      });
  });

  createEffect(() => {
    if (props.path) setShowMarkdownSource(false);
  });

  createEffect(() => {
    const value = props.value;
    if (!editor || editor.getValue() === value) return;
    applyingValue = true;
    editor.setValue(value);
    applyingValue = false;
  });

  onCleanup(() => {
    disposed = true;
    saveAction?.dispose();
    contentChange?.dispose();
    themeObserver?.disconnect();
    editor?.dispose();
  });

  return (
    <section class="workspace-file-editor" data-component="workspace-editor">
      <Show when={props.showHeader !== false}>
        <header>
          <File size={13} />
          <strong>{props.path}</strong>
          <Show when={props.dirty}>
            <span class="workspace-editor-dirty">{zh() ? "未保存" : "Unsaved"}</span>
          </Show>
          <Show when={props.editable}>
            <Button
              data-testid="workspace-save-file"
              disabled={!props.dirty || props.saving || props.conflict}
              title={zh() ? "保存 (Ctrl+S)" : "Save (Ctrl+S)"}
              onClick={props.onSave}
            >
              <Check size={12} />{" "}
              {props.saving ? (zh() ? "保存中" : "Saving") : zh() ? "保存" : "Save"}
            </Button>
          </Show>
          <Button
            aria-label={zh() ? "关闭文件" : "Close file"}
            title={zh() ? "关闭文件" : "Close file"}
            onClick={props.onClose}
          >
            <X size={12} />
          </Button>
        </header>
      </Show>
      <Show when={props.conflict}>
        <div class="workspace-editor-conflict" role="alert">
          <AlertTriangle size={13} />
          <span>
            {zh()
              ? "文件在读取后发生变化。请选择重新加载，或保留本地内容并基于最新版本再次保存。"
              : "The file changed after it was read. Reload it, or keep the local draft and save again against the latest version."}
          </span>
          <Button onClick={props.onReload}>
            <RefreshCw size={12} /> {zh() ? "重新加载" : "Reload"}
          </Button>
          <Button onClick={props.onKeepLocal}>{zh() ? "保留本地" : "Keep local"}</Button>
        </div>
      </Show>
      <Show when={markdownFile()}>
        <div class="workspace-markdown-toolbar" role="toolbar">
          <Button
            classList={{ active: !showMarkdownSource() }}
            aria-label={zh() ? "预览 Markdown" : "Preview Markdown"}
            title={zh() ? "预览 Markdown" : "Preview Markdown"}
            aria-pressed={!showMarkdownSource()}
            onClick={() => setShowMarkdownSource(false)}
          >
            <FileText size={14} />
          </Button>
          <Show when={props.editable}>
            <Button
              classList={{ active: showMarkdownSource() }}
              aria-label={zh() ? "编辑 Markdown 源码" : "Edit Markdown source"}
              title={zh() ? "编辑 Markdown 源码" : "Edit Markdown source"}
              aria-pressed={showMarkdownSource()}
              onClick={() => setShowMarkdownSource(true)}
            >
              <Code2 size={14} />
            </Button>
          </Show>
        </div>
      </Show>
      <Show
        when={props.editable}
        fallback={
          <Show
            when={markdownFile()}
            fallback={
              <div class="workspace-editor-readonly">
                <p>{props.readOnlyMessage}</p>
                <pre>{props.value}</pre>
              </div>
            }
          >
            <MarkdownContent
              class="workspace-markdown-preview timeline-message-text"
              text={props.value}
              workspaceRoot={props.workspaceRoot}
              onOpenPath={props.onOpenPath}
            />
          </Show>
        }
      >
        <Show
          when={!markdownFile() || showMarkdownSource()}
          fallback={
            <MarkdownContent
              class="workspace-markdown-preview timeline-message-text"
              text={props.value}
              workspaceRoot={props.workspaceRoot}
              onOpenPath={props.onOpenPath}
            />
          }
        >
          <div
            ref={container}
            class="workspace-monaco-editor"
            classList={{ ready: monacoReady() }}
            data-testid="workspace-monaco-editor"
          />
          <ComposerInput
            label={zh() ? `${props.path} 编辑器` : `${props.path} editor`}
            class="workspace-editor-fallback"
            classList={{ hidden: monacoReady() }}
            aria-hidden={monacoReady() || undefined}
            tabIndex={monacoReady() ? -1 : undefined}
            data-testid="workspace-editor-fallback"
            value={props.value}
            onInput={(event) => props.onInput(event.currentTarget.value)}
            onKeyDown={(event) => {
              if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
                event.preventDefault();
                props.onSave();
              }
            }}
          />
        </Show>
      </Show>
    </section>
  );
}

function isMarkdownPath(path: string): boolean {
  return /\.(?:md|markdown)$/i.test(path);
}

function monacoTheme(): "vs" | "vs-dark" {
  return document.documentElement.dataset.colorScheme === "light" ? "vs" : "vs-dark";
}

function languageForPath(path: string): string {
  const extension = path.split(".").pop()?.toLowerCase();
  return (
    {
      c: "c",
      cc: "cpp",
      cpp: "cpp",
      css: "css",
      go: "go",
      html: "html",
      java: "java",
      js: "javascript",
      json: "json",
      jsx: "javascript",
      md: "markdown",
      py: "python",
      rs: "rust",
      sh: "shell",
      sql: "sql",
      toml: "ini",
      ts: "typescript",
      tsx: "typescript",
      xml: "xml",
      yaml: "yaml",
      yml: "yaml",
    }[extension ?? ""] ?? "plaintext"
  );
}
