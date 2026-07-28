import { For, Show, splitProps, type JSX } from "solid-js";
import { Search } from "lucide-solid";
import type { UiDensity } from "../theme/context";
import type { ControlSize, ControlTone } from "./forms";
import { componentState, type ComponentStateProps } from "./types";

export interface TitleBarProps {
  brand: string;
  children?: JSX.Element;
  onPointerDown?: JSX.EventHandler<HTMLElement, PointerEvent>;
  onDoubleClick?: JSX.EventHandler<HTMLElement, MouseEvent>;
}

export function TitleBar(props: TitleBarProps & ComponentStateProps) {
  return (
    <header
      class="titlebar"
      data-component="title-bar"
      data-size={props.size ?? "normal"}
      data-variant={props.variant ?? "workbench"}
      data-tone={props.tone ?? "neutral"}
      data-density={props.density}
      data-state={componentState(props)}
      data-invalid={props.invalid || undefined}
      aria-busy={props.loading || undefined}
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

export function SidebarNav<T extends string>(
  props: {
    label: string;
    items: readonly SidebarNavItem<T>[];
    value: T;
    onChange: (value: T) => void;
  } & ComponentStateProps,
) {
  return (
    <nav data-component="sidebar-nav" aria-label={props.label}>
      <For each={props.items}>
        {(item) => (
          <button
            type="button"
            data-component="sidebar-nav-item"
            data-variant={props.variant ?? "default"}
            data-size={props.size ?? "normal"}
            data-tone={props.tone ?? "neutral"}
            data-density={props.density}
            data-state={props.value === item.value ? "selected" : componentState(props)}
            data-invalid={props.invalid || undefined}
            disabled={props.disabled || props.loading}
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
  variant?: "default" | "filled";
  invalid?: boolean;
  loading?: boolean;
  size?: ControlSize;
  tone?: ControlTone;
  density?: UiDensity;
}

export function SearchField(props: SearchFieldProps) {
  const [local, rest] = splitProps(props, [
    "label",
    "class",
    "invalid",
    "loading",
    "size",
    "tone",
    "density",
    "variant",
  ]);
  return (
    <label
      class={["ui-search", local.class].filter(Boolean).join(" ")}
      data-component="search-field"
      data-variant={local.variant ?? "default"}
      data-size={local.size ?? "normal"}
      data-tone={local.tone ?? "neutral"}
      data-density={local.density}
      data-state={componentState({
        loading: local.loading,
        disabled: rest.disabled,
        invalid: local.invalid,
      })}
      data-invalid={local.invalid || undefined}
    >
      <Search size={15} aria-hidden="true" />
      <span class="sr-only">{local.label}</span>
      <input
        type="search"
        {...rest}
        aria-label={local.label}
        aria-invalid={local.invalid || undefined}
        aria-busy={local.loading || undefined}
        disabled={rest.disabled || local.loading}
      />
    </label>
  );
}

export interface TextAreaProps extends JSX.TextareaHTMLAttributes<HTMLTextAreaElement> {
  label: string;
  description?: string;
  variant?: "default" | "filled";
  invalid?: boolean;
  loading?: boolean;
  size?: ControlSize;
  tone?: ControlTone;
  density?: UiDensity;
}

export function TextArea(props: TextAreaProps) {
  const [local, rest] = splitProps(props, [
    "label",
    "description",
    "class",
    "invalid",
    "loading",
    "size",
    "tone",
    "density",
    "variant",
  ]);
  return (
    <label class="field-stack" data-component="form-field">
      <span class="field-label" data-component="form-label">
        {local.label}
      </span>
      <textarea
        class={["ui-textarea", local.class].filter(Boolean).join(" ")}
        data-component="text-area"
        data-variant={local.variant ?? "default"}
        data-size={local.size ?? "normal"}
        data-tone={local.tone ?? "neutral"}
        data-density={local.density}
        data-state={componentState({
          loading: local.loading,
          disabled: props.disabled,
          invalid: local.invalid,
        })}
        {...rest}
        aria-invalid={local.invalid || undefined}
        aria-busy={local.loading || undefined}
        disabled={rest.disabled || local.loading}
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
  variant?: "default" | "filled";
  invalid?: boolean;
  loading?: boolean;
  size?: ControlSize;
  tone?: ControlTone;
  density?: UiDensity;
}

export function NumberField(props: NumberFieldProps) {
  const [local, rest] = splitProps(props, [
    "label",
    "description",
    "invalid",
    "loading",
    "size",
    "tone",
    "density",
    "variant",
  ]);
  return (
    <label data-component="form-field">
      <span data-component="form-label">{local.label}</span>
      <input
        type="number"
        data-component="text-field-input"
        data-variant={local.variant ?? "default"}
        data-size={local.size ?? "normal"}
        data-tone={local.tone ?? "neutral"}
        data-density={local.density}
        data-state={componentState({
          loading: local.loading,
          disabled: rest.disabled,
          invalid: local.invalid,
        })}
        {...rest}
        aria-invalid={local.invalid || undefined}
        aria-busy={local.loading || undefined}
        disabled={rest.disabled || local.loading}
      />
      <Show when={local.description}>
        <span data-component="field-description">{local.description}</span>
      </Show>
    </label>
  );
}

export function StatusBanner(
  props: {
    tone?: "neutral" | "success" | "warning" | "danger";
    children: JSX.Element;
  } & Omit<ComponentStateProps, "tone">,
) {
  return (
    <div
      class={`ui-alert ${props.tone ?? "info"}`}
      data-component="status-banner"
      data-variant={props.tone ?? "neutral"}
      data-size={props.size ?? "normal"}
      data-density={props.density}
      data-state={componentState(props)}
      data-invalid={props.invalid || undefined}
      aria-busy={props.loading || undefined}
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

export function ResourceCard(
  props: {
    title: string;
    subtitle: string;
    current?: boolean;
    tone?: "default" | "neutral" | "accent" | "danger";
    media?: JSX.Element;
    meta?: JSX.Element;
    details?: JSX.Element;
    actions?: JSX.Element;
  } & Omit<ComponentStateProps, "tone">,
) {
  return (
    <article
      data-component="resource-card"
      data-variant={props.current ? "current" : (props.tone ?? "default")}
      data-size={props.size ?? "normal"}
      data-density={props.density}
      data-state={props.current ? "selected" : "idle"}
      data-invalid={props.invalid || undefined}
      aria-busy={props.loading || undefined}
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
