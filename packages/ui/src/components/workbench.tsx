import { For, Show, splitProps, type JSX } from "solid-js";
import { Search } from "lucide-solid";

export interface TitleBarProps {
  brand: string;
  children?: JSX.Element;
  onPointerDown?: JSX.EventHandler<HTMLElement, PointerEvent>;
  onDoubleClick?: JSX.EventHandler<HTMLElement, MouseEvent>;
}

export function TitleBar(props: TitleBarProps) {
  return (
    <header
      data-component="title-bar"
      data-variant="workbench"
      data-size="normal"
      data-state="idle"
      onPointerDown={(event) => props.onPointerDown?.(event)}
      onDblClick={(event) => props.onDoubleClick?.(event)}
    >
      <strong data-component="title-bar-brand">{props.brand}</strong>
      {props.children}
    </header>
  );
}

export interface SidebarNavItem<T extends string> {
  value: T;
  label: string;
  icon?: JSX.Element;
}

export function SidebarNav<T extends string>(props: {
  label: string;
  items: readonly SidebarNavItem<T>[];
  value: T;
  onChange: (value: T) => void;
}) {
  return (
    <nav data-component="sidebar-nav" aria-label={props.label}>
      <For each={props.items}>
        {(item) => (
          <button
            type="button"
            data-component="sidebar-nav-item"
            data-state={props.value === item.value ? "selected" : "idle"}
            aria-current={props.value === item.value ? "page" : undefined}
            onClick={() => props.onChange(item.value)}
          >
            <Show when={item.icon}>{item.icon}</Show>
            <span>{item.label}</span>
          </button>
        )}
      </For>
    </nav>
  );
}

export interface SearchFieldProps extends JSX.InputHTMLAttributes<HTMLInputElement> {
  label: string;
}

export function SearchField(props: SearchFieldProps) {
  const [local, rest] = splitProps(props, ["label"]);
  return (
    <label data-component="search-field">
      <Search size={15} aria-hidden="true" />
      <span class="sr-only">{local.label}</span>
      <input type="search" aria-label={local.label} {...rest} />
    </label>
  );
}

export interface TextAreaProps extends JSX.TextareaHTMLAttributes<HTMLTextAreaElement> {
  label: string;
  description?: string;
}

export function TextArea(props: TextAreaProps) {
  const [local, rest] = splitProps(props, ["label", "description"]);
  return (
    <label data-component="form-field">
      <span data-component="form-label">{local.label}</span>
      <textarea
        data-component="text-area"
        data-state={props.disabled ? "disabled" : "idle"}
        {...rest}
      />
      <Show when={local.description}>
        <span data-component="field-description">{local.description}</span>
      </Show>
    </label>
  );
}

export interface NumberFieldProps extends JSX.InputHTMLAttributes<HTMLInputElement> {
  label: string;
  description?: string;
}

export function NumberField(props: NumberFieldProps) {
  const [local, rest] = splitProps(props, ["label", "description"]);
  return (
    <label data-component="form-field">
      <span data-component="form-label">{local.label}</span>
      <input type="number" data-component="text-field-input" data-state="idle" {...rest} />
      <Show when={local.description}>
        <span data-component="field-description">{local.description}</span>
      </Show>
    </label>
  );
}

export function StatusBanner(props: {
  tone?: "neutral" | "success" | "warning" | "danger";
  children: JSX.Element;
}) {
  return (
    <div
      data-component="status-banner"
      data-variant={props.tone ?? "neutral"}
      data-state="visible"
      role={props.tone === "danger" ? "alert" : "status"}
    >
      {props.children}
    </div>
  );
}

export function ResourceList(props: { label: string; children: JSX.Element }) {
  return (
    <div data-component="resource-list" role="list" aria-label={props.label}>
      {props.children}
    </div>
  );
}

export function ResourceCard(props: {
  title: string;
  subtitle: string;
  current?: boolean;
  tone?: "default" | "danger";
  media?: JSX.Element;
  meta?: JSX.Element;
  details?: JSX.Element;
  actions?: JSX.Element;
}) {
  return (
    <article
      data-component="resource-card"
      data-variant={props.current ? "current" : (props.tone ?? "default")}
      data-state={props.current ? "selected" : "idle"}
      role="listitem"
    >
      <Show when={props.media}>
        <div data-component="resource-card-media">{props.media}</div>
      </Show>
      <div data-component="resource-card-copy">
        <strong>{props.title}</strong>
        <span>{props.subtitle}</span>
        <Show when={props.meta}>
          <small>{props.meta}</small>
        </Show>
        <Show when={props.details}>
          <div data-component="resource-card-details">{props.details}</div>
        </Show>
      </div>
      <Show when={props.actions}>
        <div data-component="resource-card-actions">{props.actions}</div>
      </Show>
    </article>
  );
}
