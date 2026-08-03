import type { SessionSourceRecord } from "@hachimi/contracts";
import { Button, File, Globe, Link2, Plus } from "@hachimi/ui";
import { For } from "solid-js";

export function EnvironmentSources(props: {
  sources: SessionSourceRecord[];
  locale: "zh-CN" | "en-US";
  onOpenSource: (source: SessionSourceRecord) => void;
  onViewAll: () => void;
}) {
  const zh = () => props.locale === "zh-CN";
  return (
    <section class="environment-summary-section">
      <header>
        <strong>{zh() ? "来源" : "Sources"}</strong>
        <Plus size={16} aria-hidden="true" />
      </header>
      <For each={props.sources.slice(0, 5)}>
        {(source) => (
          <Button
            class="environment-summary-row"
            title={sourceTitle(source)}
            onClick={() => props.onOpenSource(source)}
          >
            {source.kind === "web" ? <Globe size={15} /> : <File size={15} />}
            <span>{sourceTitle(source)}</span>
            <span class="environment-row-tail">›</span>
          </Button>
        )}
      </For>
      <Button
        class="environment-summary-row environment-view-all"
        data-testid="workbench-summary-sources-all"
        onClick={props.onViewAll}
      >
        <Link2 size={15} />
        <span>{zh() ? "查看全部" : "View all"}</span>
        <span class="environment-row-tail">{props.sources.length}</span>
      </Button>
    </section>
  );
}

export function sourceTitle(source: SessionSourceRecord) {
  if (source.title?.trim()) return source.title;
  if (source.url) {
    try {
      const url = new URL(source.url);
      return `${url.host}${url.pathname === "/" ? "" : url.pathname}`;
    } catch {
      return source.url;
    }
  }
  return source.attachmentId ?? source.id;
}
