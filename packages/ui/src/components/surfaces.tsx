import { Tooltip as KobalteTooltip } from "@kobalte/core/tooltip";
import { Show, splitProps, type JSX } from "solid-js";
import { IconButton, type IconButtonProps } from "./button";

export function AppShell(props: JSX.HTMLAttributes<HTMLDivElement>) {
  return <div data-component="app-shell" {...props} />;
}

export function Sidebar(props: JSX.HTMLAttributes<HTMLElement>) {
  return <aside data-component="sidebar" {...props} />;
}

export interface NavItemProps extends JSX.ButtonHTMLAttributes<HTMLButtonElement> {
  selected?: boolean;
  icon?: JSX.Element;
}

export function NavItem(props: NavItemProps) {
  const [local, rest] = splitProps(props, ["selected", "icon", "children"]);
  return (
    <button
      type="button"
      data-component="nav-item"
      data-state={local.selected ? "selected" : "idle"}
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

export function PromptCard(props: JSX.ButtonHTMLAttributes<HTMLButtonElement>) {
  return <button type="button" data-component="prompt-card" {...props} />;
}

export function Composer(props: JSX.HTMLAttributes<HTMLDivElement>) {
  return <div data-component="composer" {...props} />;
}

export function SettingsCard(props: JSX.HTMLAttributes<HTMLElement>) {
  return <section data-component="settings-card" {...props} />;
}

export interface ThemeCardProps extends JSX.ButtonHTMLAttributes<HTMLButtonElement> {
  selected?: boolean;
  preview: JSX.Element;
  label: string;
}

export function ThemeCard(props: ThemeCardProps) {
  const [local, rest] = splitProps(props, ["selected", "preview", "label"]);
  return (
    <button
      type="button"
      data-component="theme-card"
      data-state={local.selected ? "selected" : "idle"}
      aria-pressed={local.selected}
      {...rest}
    >
      <span data-component="theme-card-preview">{local.preview}</span>
      <span>{local.label}</span>
    </button>
  );
}

export function ResourceRow(props: JSX.HTMLAttributes<HTMLElement>) {
  return <article data-component="resource-row" {...props} />;
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
