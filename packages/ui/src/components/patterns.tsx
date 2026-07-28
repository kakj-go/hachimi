import { For, Show, splitProps, type JSX } from "solid-js";
import type { UiDensity } from "../theme/context";
import type { ControlSize, ControlTone } from "./forms";
import { componentState, type ComponentStateProps } from "./types";

export interface ChoiceControlProps extends Omit<
  JSX.InputHTMLAttributes<HTMLInputElement>,
  "type"
> {
  label: string;
  invalid?: boolean;
  loading?: boolean;
  size?: ControlSize;
  tone?: ControlTone;
  density?: UiDensity;
}

export function Checkbox(props: ChoiceControlProps) {
  const [local, rest] = splitProps(props, [
    "label",
    "class",
    "invalid",
    "loading",
    "size",
    "tone",
    "density",
  ]);
  return (
    <label
      class={["ui-checkbox", local.class].filter(Boolean).join(" ")}
      data-component="checkbox"
      data-size={local.size ?? "normal"}
      data-tone={local.tone ?? "neutral"}
      data-density={local.density}
      data-state={local.loading ? "loading" : local.invalid ? "invalid" : "idle"}
    >
      <input
        type="checkbox"
        {...rest}
        disabled={rest.disabled || local.loading}
        aria-label={props.label}
        aria-invalid={local.invalid || undefined}
        aria-busy={local.loading || undefined}
      />
      <span>{props.label}</span>
    </label>
  );
}

export function Radio(props: ChoiceControlProps) {
  const [local, rest] = splitProps(props, [
    "label",
    "class",
    "invalid",
    "loading",
    "size",
    "tone",
    "density",
  ]);
  return (
    <label
      class={["ui-radio", local.class].filter(Boolean).join(" ")}
      data-component="radio"
      data-size={local.size ?? "normal"}
      data-tone={local.tone ?? "neutral"}
      data-density={local.density}
      data-state={local.loading ? "loading" : local.invalid ? "invalid" : "idle"}
    >
      <input
        type="radio"
        {...rest}
        disabled={rest.disabled || local.loading}
        aria-label={props.label}
        aria-invalid={local.invalid || undefined}
        aria-busy={local.loading || undefined}
      />
      <span>{props.label}</span>
    </label>
  );
}

export interface AlertBannerProps {
  tone?: "info" | "warning" | "danger" | "success";
  variant?: ComponentStateProps["variant"];
  size?: ControlSize;
  density?: UiDensity;
  disabled?: boolean;
  loading?: boolean;
  invalid?: boolean;
  icon?: JSX.Element;
  children: JSX.Element;
}

export function AlertBanner(props: AlertBannerProps) {
  return (
    <div
      class={`ui-alert ${props.tone ?? "info"}`}
      data-component="alert-banner"
      data-variant={props.variant ?? "default"}
      data-size={props.size ?? "normal"}
      data-tone={props.tone ?? "info"}
      data-density={props.density}
      data-state={componentState(props)}
      data-invalid={props.invalid || undefined}
      aria-busy={props.loading || undefined}
      role={props.tone === "danger" ? "alert" : "status"}
    >
      <Show when={props.icon}>{props.icon}</Show>
      <span>{props.children}</span>
    </div>
  );
}

export function WarningBanner(props: Omit<AlertBannerProps, "tone">) {
  return <AlertBanner {...props} tone="warning" />;
}

export function Progress(props: { label: string; value: number; max?: number }) {
  const max = () => Math.max(1, props.max ?? 100);
  const percent = () => Math.min(100, Math.max(0, (props.value / max()) * 100));
  return (
    <div class="progress-item" data-component="progress">
      <header>
        <span>{props.label}</span>
        <span>{Math.round(percent())}%</span>
      </header>
      <div
        class="progress-track"
        role="progressbar"
        aria-label={props.label}
        aria-valuenow={props.value}
        aria-valuemin={0}
        aria-valuemax={max()}
      >
        <i style={{ "--progress": `${percent()}%` }} />
      </div>
    </div>
  );
}

export function EmptyState(
  props: {
    title: string;
    description?: string;
    icon?: JSX.Element;
    actions?: JSX.Element;
  } & ComponentStateProps,
) {
  return (
    <section
      class="agent-card"
      data-component="empty-state"
      data-variant={props.variant ?? "default"}
      data-size={props.size ?? "normal"}
      data-tone={props.tone ?? "neutral"}
      data-density={props.density}
      data-state={componentState(props)}
      data-invalid={props.invalid || undefined}
      aria-busy={props.loading || undefined}
    >
      <Show when={props.icon}>
        <header>{props.icon}</header>
      </Show>
      <strong>{props.title}</strong>
      <Show when={props.description}>
        <p>{props.description}</p>
      </Show>
      <Show when={props.actions}>
        <div class="agent-card-actions">{props.actions}</div>
      </Show>
    </section>
  );
}

export function AgentMessage(
  props: {
    role: "user" | "assistant";
    author?: string;
    meta?: JSX.Element;
    class?: string;
    component?: "agent-message" | "tool-call";
    children: JSX.Element;
  } & ComponentStateProps,
) {
  return (
    <article
      class={["agent-message", props.role === "user" ? "user" : "assistant", props.class]
        .filter(Boolean)
        .join(" ")}
      data-component={props.component ?? "agent-message"}
      data-role={props.role}
      data-variant={props.variant ?? "default"}
      data-size={props.size ?? "normal"}
      data-tone={props.tone ?? "neutral"}
      data-density={props.density}
      data-state={componentState(props)}
      data-invalid={props.invalid || undefined}
      aria-busy={props.loading || undefined}
    >
      <header class="agent-message-meta">
        <strong>{props.author ?? (props.role === "user" ? "You" : "Hachimi")}</strong>
        <Show when={props.meta}>{props.meta}</Show>
      </header>
      <div>{props.children}</div>
    </article>
  );
}

export function ToolCall(
  props: {
    title: string;
    summary?: string;
    open?: boolean;
    onToggle?: () => void;
    children?: JSX.Element;
  } & ComponentStateProps,
) {
  return (
    <section
      class="tool-call"
      classList={{ open: props.open }}
      data-component="tool-call"
      data-variant={props.variant ?? "default"}
      data-size={props.size ?? "normal"}
      data-tone={props.tone ?? "neutral"}
      data-density={props.density}
      data-state={componentState(props)}
      data-invalid={props.invalid || undefined}
      aria-busy={props.loading || undefined}
    >
      <button
        class="tool-call-toggle"
        type="button"
        aria-expanded={props.open}
        aria-invalid={props.invalid || undefined}
        disabled={props.disabled || props.loading}
        onClick={() => props.onToggle?.()}
      >
        <strong>{props.title}</strong>
        <Show when={props.summary}>
          <small>{props.summary}</small>
        </Show>
      </button>
      <Show when={props.children}>
        <div class="tool-call-details">{props.children}</div>
      </Show>
    </section>
  );
}

function AgentCard(
  props: {
    kind: "approval" | "plan";
    title: string;
    description?: string;
    icon?: JSX.Element;
    actions?: JSX.Element;
    children?: JSX.Element;
  } & ComponentStateProps,
) {
  return (
    <section
      class={`agent-card ${props.kind}`}
      data-component={props.kind}
      data-variant={props.variant ?? "default"}
      data-size={props.size ?? "normal"}
      data-tone={props.tone ?? "neutral"}
      data-density={props.density}
      data-state={componentState(props)}
      data-invalid={props.invalid || undefined}
      aria-busy={props.loading || undefined}
    >
      <header>
        <Show when={props.icon}>{props.icon}</Show>
        <strong>{props.title}</strong>
      </header>
      <Show when={props.description}>
        <p>{props.description}</p>
      </Show>
      <Show when={props.children}>{props.children}</Show>
      <Show when={props.actions}>
        <div class="agent-card-actions">{props.actions}</div>
      </Show>
    </section>
  );
}

export function ApprovalCard(props: Omit<Parameters<typeof AgentCard>[0], "kind">) {
  return <AgentCard kind="approval" {...props} />;
}

export function PlanCard(props: Omit<Parameters<typeof AgentCard>[0], "kind">) {
  return <AgentCard kind="plan" {...props} />;
}

export function Approval(props: Omit<Parameters<typeof AgentCard>[0], "kind">) {
  return <ApprovalCard {...props} />;
}

export function Plan(props: Omit<Parameters<typeof AgentCard>[0], "kind">) {
  return <PlanCard {...props} />;
}

export function AttachmentCard(
  props: {
    name: string;
    meta?: string;
    image?: boolean;
    kind?: "file" | "folder";
    class?: string;
    testId?: string;
    title?: string;
    preview?: JSX.Element;
    icon?: JSX.Element;
    removeLabel?: string;
    removeClass?: string;
    onRemove?: () => void;
  } & ComponentStateProps,
) {
  return (
    <article
      class={["attachment-card", props.class].filter(Boolean).join(" ")}
      classList={{ image: props.image, folder: props.kind === "folder" }}
      data-component="attachment-card"
      data-testid={props.testId}
      title={props.title}
      role="listitem"
      data-variant={props.variant ?? "default"}
      data-size={props.size ?? "normal"}
      data-tone={props.tone ?? "neutral"}
      data-density={props.density}
      data-state={componentState(props)}
      data-invalid={props.invalid || undefined}
      aria-busy={props.loading || undefined}
    >
      <span class={props.image ? "attachment-preview" : "attachment-file-icon"}>
        {props.preview ?? props.icon}
      </span>
      <Show when={!props.image}>
        <span class="attachment-copy">
          <strong>{props.name}</strong>
          <Show when={props.meta}>
            <small>{props.meta}</small>
          </Show>
        </span>
      </Show>
      <Show when={props.onRemove}>
        <button
          type="button"
          class={props.removeClass}
          aria-label={props.removeLabel ?? `Remove ${props.name}`}
          disabled={props.disabled || props.loading}
          onClick={() => props.onRemove?.()}
        >
          ×
        </button>
      </Show>
    </article>
  );
}

export function ContextRow(
  props: { label: string; value?: JSX.Element; icon?: JSX.Element } & ComponentStateProps,
) {
  return (
    <div
      class="context-row"
      data-component="context-row"
      data-variant={props.variant ?? "default"}
      data-size={props.size ?? "normal"}
      data-tone={props.tone ?? "neutral"}
      data-density={props.density}
      data-state={componentState(props)}
      data-invalid={props.invalid || undefined}
    >
      <Show when={props.icon}>{props.icon}</Show>
      <span>{props.label}</span>
      <Show when={props.value}>
        <strong>{props.value}</strong>
      </Show>
    </div>
  );
}

export interface TreeItem {
  id: string;
  label: string;
  depth?: number;
  icon?: JSX.Element;
}

export interface TreeProps {
  label: string;
  items: readonly TreeItem[];
  selectedId?: string;
  onSelect?: (id: string) => void;
  component?: "tree" | "file-tree" | "skill-tree";
  variant?: ComponentStateProps["variant"];
  size?: ComponentStateProps["size"];
  tone?: ComponentStateProps["tone"];
  density?: ComponentStateProps["density"];
  disabled?: boolean;
  loading?: boolean;
  invalid?: boolean;
}

export function Tree(props: TreeProps) {
  return (
    <div
      class="tree-panel"
      data-component={props.component ?? "tree"}
      data-variant={props.variant ?? "default"}
      data-size={props.size ?? "normal"}
      data-tone={props.tone ?? "neutral"}
      data-density={props.density}
      data-state={componentState(props)}
      data-invalid={props.invalid || undefined}
      aria-busy={props.loading || undefined}
      role="tree"
      aria-label={props.label}
    >
      <For each={props.items}>
        {(item) => (
          <button
            type="button"
            class="tree-row"
            classList={{
              selected: props.selectedId === item.id,
              indent: Boolean(item.depth),
            }}
            style={{ "--tree-depth": item.depth ?? 0 }}
            role="treeitem"
            aria-selected={props.selectedId === item.id}
            disabled={props.disabled || props.loading}
            onClick={() => props.onSelect?.(item.id)}
          >
            <Show when={item.icon}>{item.icon}</Show>
            <span>{item.label}</span>
          </button>
        )}
      </For>
    </div>
  );
}

export function FileTree(props: TreeProps) {
  return <Tree {...props} component="file-tree" />;
}

export function SkillTree(props: TreeProps) {
  return <Tree {...props} component="skill-tree" />;
}

export function Workspace(
  props: Omit<JSX.HTMLAttributes<HTMLElement>, keyof ComponentStateProps> & ComponentStateProps,
) {
  const [local, rest] = splitProps(props, [
    "class",
    "variant",
    "size",
    "tone",
    "density",
    "disabled",
    "loading",
    "invalid",
  ]);
  return (
    <section
      class={["workspace-panel", local.class].filter(Boolean).join(" ")}
      data-component="workspace"
      data-variant={local.variant ?? "default"}
      data-size={local.size ?? "normal"}
      data-tone={local.tone ?? "neutral"}
      data-density={local.density}
      data-state={componentState(local)}
      data-invalid={local.invalid || undefined}
      aria-busy={local.loading || undefined}
      {...rest}
    />
  );
}

export function MetricCard(
  props: { label: string; value: JSX.Element; detail?: string } & ComponentStateProps,
) {
  return (
    <article
      class="metric-card"
      data-component="metric-card"
      data-variant={props.variant ?? "default"}
      data-size={props.size ?? "normal"}
      data-tone={props.tone ?? "neutral"}
      data-density={props.density}
      data-state={componentState(props)}
      data-invalid={props.invalid || undefined}
    >
      <span>{props.label}</span>
      <strong>{props.value}</strong>
      <Show when={props.detail}>
        <small>{props.detail}</small>
      </Show>
    </article>
  );
}

export function Metrics(props: { label: string; children: JSX.Element } & ComponentStateProps) {
  return (
    <section
      class="metric-grid"
      data-component="metrics"
      data-variant={props.variant ?? "default"}
      data-size={props.size ?? "normal"}
      data-tone={props.tone ?? "neutral"}
      data-density={props.density}
      data-state={componentState(props)}
      data-invalid={props.invalid || undefined}
      aria-label={props.label}
    >
      {props.children}
    </section>
  );
}

export function McpCard(
  props: {
    title: string;
    status?: JSX.Element;
    description?: string;
    actions?: JSX.Element;
    children?: JSX.Element;
  } & ComponentStateProps,
) {
  return (
    <article
      class="resource-card"
      data-component="mcp-card"
      data-variant={props.variant ?? "default"}
      data-size={props.size ?? "normal"}
      data-tone={props.tone ?? "neutral"}
      data-density={props.density}
      data-state={componentState(props)}
      data-invalid={props.invalid || undefined}
      aria-busy={props.loading || undefined}
    >
      <header>
        <strong>{props.title}</strong>
        <Show when={props.status}>{props.status}</Show>
      </header>
      <Show when={props.description}>
        <p>{props.description}</p>
      </Show>
      <Show when={props.children}>{props.children}</Show>
      <Show when={props.actions}>
        <footer>{props.actions}</footer>
      </Show>
    </article>
  );
}

export const MCPCard = McpCard;

export function DiffPanel(
  props: { path: string; meta?: JSX.Element; children: JSX.Element } & ComponentStateProps,
) {
  return (
    <section
      class="code-panel"
      data-component="diff-panel"
      data-variant={props.variant ?? "default"}
      data-size={props.size ?? "normal"}
      data-tone={props.tone ?? "neutral"}
      data-density={props.density}
      data-state={componentState(props)}
      data-invalid={props.invalid || undefined}
    >
      <header class="code-header">
        <code>{props.path}</code>
        <Show when={props.meta}>{props.meta}</Show>
      </header>
      <div class="code-body">{props.children}</div>
    </section>
  );
}

export function Diff(props: Parameters<typeof DiffPanel>[0]) {
  return <DiffPanel {...props} />;
}

export function DiffLine(
  props: {
    line?: number;
    tone?: "added" | "removed" | "context";
    children: JSX.Element;
  } & ComponentStateProps,
) {
  return (
    <div
      class="code-line"
      classList={{ added: props.tone === "added", removed: props.tone === "removed" }}
      data-component="diff-line"
      data-tone={props.tone ?? "context"}
      data-variant={props.variant ?? "default"}
      data-size={props.size ?? "normal"}
      data-density={props.density}
      data-state={componentState(props)}
      data-invalid={props.invalid || undefined}
    >
      <span>{props.line ?? ""}</span>
      <code>{props.children}</code>
    </div>
  );
}
