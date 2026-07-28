import { Tooltip as KobalteTooltip } from "@kobalte/core/tooltip";
import { Show, splitProps, type JSX } from "solid-js";
import { IconButton, type IconButtonProps } from "./button";
import { componentState, type ComponentStateProps } from "./types";

export function AppShell(props: JSX.HTMLAttributes<HTMLDivElement>) {
  return <div data-component="app-shell" {...props} />;
}

export function Sidebar(props: JSX.HTMLAttributes<HTMLElement>) {
  return <aside data-component="sidebar" {...props} />;
}

export interface NavItemProps
  extends
    Omit<JSX.ButtonHTMLAttributes<HTMLButtonElement>, keyof ComponentStateProps>,
    ComponentStateProps {
  selected?: boolean;
  icon?: JSX.Element;
}

export function NavItem(props: NavItemProps) {
  const [local, rest] = splitProps(props, [
    "selected",
    "icon",
    "children",
    "variant",
    "size",
    "tone",
    "density",
    "loading",
    "invalid",
  ]);
  return (
    <button
      type="button"
      data-component="nav-item"
      data-variant={local.variant ?? "default"}
      data-size={local.size ?? "normal"}
      data-tone={local.tone ?? "neutral"}
      data-density={local.density}
      data-state={local.selected ? "selected" : componentState(local)}
      data-invalid={local.invalid || undefined}
      aria-busy={local.loading || undefined}
      aria-invalid={local.invalid || undefined}
      disabled={rest.disabled || local.loading}
      aria-current={local.selected ? "page" : undefined}
      {...rest}
    >
      <Show when={local.icon}>
        <span data-component="nav-item-icon">{local.icon}</span>
      </Show>
      <span>{local.children}</span>
    </button>
  );
}

export function PromptCard(
  props: Omit<JSX.ButtonHTMLAttributes<HTMLButtonElement>, keyof ComponentStateProps> &
    ComponentStateProps,
) {
  const [local, rest] = splitProps(props, [
    "class",
    "variant",
    "size",
    "tone",
    "density",
    "loading",
    "invalid",
  ]);
  return (
    <button
      type="button"
      class={["prompt-card-demo", local.class].filter(Boolean).join(" ")}
      data-component="prompt-card"
      data-variant={local.variant ?? "default"}
      data-size={local.size ?? "normal"}
      data-tone={local.tone ?? "neutral"}
      data-density={local.density}
      data-state={componentState(local)}
      data-invalid={local.invalid || undefined}
      aria-busy={local.loading || undefined}
      aria-invalid={local.invalid || undefined}
      disabled={rest.disabled || local.loading}
      {...rest}
    />
  );
}

export function Composer(
  props: Omit<JSX.HTMLAttributes<HTMLDivElement>, keyof ComponentStateProps> & ComponentStateProps,
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
    <div
      class={["composer-demo", local.class].filter(Boolean).join(" ")}
      data-component="composer"
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

export interface ComposerInputProps
  extends
    Omit<JSX.TextareaHTMLAttributes<HTMLTextAreaElement>, keyof ComponentStateProps>,
    ComponentStateProps {
  label: string;
}

export function ComposerInput(props: ComposerInputProps) {
  const [local, rest] = splitProps(props, [
    "label",
    "class",
    "variant",
    "size",
    "tone",
    "density",
    "loading",
    "invalid",
  ]);
  return (
    <textarea
      class={["composer-input", local.class].filter(Boolean).join(" ")}
      data-component="composer-input"
      data-variant={local.variant ?? "default"}
      data-size={local.size ?? "normal"}
      data-tone={local.tone ?? "neutral"}
      data-density={local.density}
      data-state={componentState(local)}
      data-invalid={local.invalid || undefined}
      aria-label={local.label}
      aria-busy={local.loading || undefined}
      aria-invalid={local.invalid || undefined}
      disabled={rest.disabled || local.loading}
      {...rest}
    />
  );
}

export function SettingsCard(
  props: Omit<JSX.HTMLAttributes<HTMLElement>, keyof ComponentStateProps> & ComponentStateProps,
) {
  const [local, rest] = splitProps(props, [
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
      data-component="settings-card"
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

export interface ThemeCardProps extends JSX.ButtonHTMLAttributes<HTMLButtonElement> {
  selected?: boolean;
  preview: JSX.Element;
  label: string;
  previewClass?: string;
  previewStyle?: JSX.CSSProperties;
  variant?: ComponentStateProps["variant"];
  size?: ComponentStateProps["size"];
  tone?: ComponentStateProps["tone"];
  density?: ComponentStateProps["density"];
  loading?: boolean;
  invalid?: boolean;
}

export function ThemeCard(props: ThemeCardProps) {
  const [local, rest] = splitProps(props, [
    "selected",
    "preview",
    "label",
    "class",
    "previewClass",
    "previewStyle",
    "variant",
    "size",
    "tone",
    "density",
    "loading",
    "invalid",
  ]);
  return (
    <button
      type="button"
      class={["theme-card", local.class].filter(Boolean).join(" ")}
      data-component="theme-card"
      data-variant={local.variant ?? "default"}
      data-size={local.size ?? "normal"}
      data-tone={local.tone ?? "neutral"}
      data-density={local.density}
      data-state={local.selected ? "selected" : "idle"}
      data-invalid={local.invalid || undefined}
      aria-busy={local.loading || undefined}
      aria-invalid={local.invalid || undefined}
      disabled={rest.disabled || local.loading}
      aria-pressed={local.selected}
      {...rest}
    >
      <span
        class={local.previewClass}
        style={local.previewStyle}
        data-component="theme-card-preview"
      >
        {local.preview}
      </span>
      <strong data-component="theme-card-label">{local.label}</strong>
    </button>
  );
}

export function ResourceRow(
  props: Omit<JSX.HTMLAttributes<HTMLElement>, keyof ComponentStateProps> & ComponentStateProps,
) {
  const [local, rest] = splitProps(props, [
    "variant",
    "size",
    "tone",
    "density",
    "disabled",
    "loading",
    "invalid",
  ]);
  return (
    <article
      data-component="resource-row"
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

export function Tooltip(props: { label: string; children: JSX.Element }) {
  return (
    <KobalteTooltip placement="top" openDelay={450} closeDelay={80}>
      <KobalteTooltip.Trigger as="span" data-component="tooltip-trigger">
        {props.children}
      </KobalteTooltip.Trigger>
      <KobalteTooltip.Portal>
        <KobalteTooltip.Content data-component="tooltip-content">
          {props.label}
        </KobalteTooltip.Content>
      </KobalteTooltip.Portal>
    </KobalteTooltip>
  );
}

export function FloatingIconButton(props: IconButtonProps) {
  return <IconButton {...props} data-component="floating-icon-button" />;
}
