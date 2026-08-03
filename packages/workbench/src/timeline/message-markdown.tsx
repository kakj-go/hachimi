/* eslint-disable solid/no-innerhtml -- The only HTML sink receives DOMPurify output from renderMarkdown. */
import DOMPurify from "dompurify";
import { Renderer, marked } from "marked";
import { createEffect, createMemo, createSignal, onCleanup } from "solid-js";

import { resolveLocalMarkdownPath } from "./local-file-links";

const ALLOWED_TAGS = [
  "a",
  "blockquote",
  "br",
  "code",
  "del",
  "em",
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "hr",
  "li",
  "ol",
  "p",
  "pre",
  "strong",
  "table",
  "tbody",
  "td",
  "th",
  "thead",
  "tr",
  "ul",
];

export function TimelineMessageText(props: {
  text: string;
  workspaceRoot: string | undefined;
  onOpenPath: (path: string) => void;
}) {
  const [renderedText, setRenderedText] = createSignal("");

  createEffect(() => {
    const nextText = props.text;
    let cancelled = false;
    const commit = () => {
      if (!cancelled) setRenderedText(nextText);
    };
    if (typeof requestAnimationFrame === "function") {
      const frame = requestAnimationFrame(commit);
      onCleanup(() => {
        cancelled = true;
        cancelAnimationFrame(frame);
      });
    } else {
      queueMicrotask(commit);
      onCleanup(() => {
        cancelled = true;
      });
    }
  });

  const html = createMemo(() => renderMarkdown(renderedText(), props.workspaceRoot));
  return (
    <div
      class="timeline-message-text"
      innerHTML={html()}
      onClick={(event) => {
        const target = event.target instanceof Element ? event.target.closest("a") : null;
        const path = target?.getAttribute("data-local-path");
        if (!path || !event.currentTarget.contains(target)) return;
        event.preventDefault();
        props.onOpenPath(path);
      }}
    />
  );
}

export function renderMarkdown(text: string, workspaceRoot?: string): string {
  const renderer = new Renderer();
  renderer.html = ({ text: raw }) => escapeHtml(raw);
  renderer.image = ({ text: alt }) => `<span>${escapeHtml(alt)}</span>`;
  renderer.link = ({ href, title, tokens }) => {
    const label = renderer.parser.parseInline(tokens);
    const localPath = resolveLocalMarkdownPath(href, workspaceRoot);
    if (localPath) {
      return `<a href="#" data-local-path="${escapeAttribute(localPath)}"${titleAttribute(
        title,
      )}>${label}</a>`;
    }
    const externalHref = safeExternalHref(href);
    if (!externalHref) return label;
    return `<a href="${escapeAttribute(externalHref)}" target="_blank" rel="noopener noreferrer"${titleAttribute(
      title,
    )}>${label}</a>`;
  };

  const parsed = marked.parse(text, {
    async: false,
    breaks: true,
    gfm: true,
    renderer,
  });
  const source = typeof parsed === "string" ? parsed : "";
  return DOMPurify.sanitize(source, {
    ALLOWED_TAGS,
    ALLOWED_ATTR: ["class", "data-local-path", "href", "rel", "target", "title"],
  });
}

function safeExternalHref(href: string): string | undefined {
  const value = href.trim();
  if (value.startsWith("#")) return value;
  try {
    const url = new URL(value);
    return ["http:", "https:", "mailto:"].includes(url.protocol) ? value : undefined;
  } catch {
    return undefined;
  }
}

function titleAttribute(title: string | null | undefined): string {
  return title ? ` title="${escapeAttribute(title)}"` : "";
}

function escapeHtml(value: string): string {
  return value.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");
}

function escapeAttribute(value: string): string {
  return escapeHtml(value).replaceAll('"', "&quot;").replaceAll("'", "&#39;");
}
