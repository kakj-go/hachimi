import { Show, type JSX } from "solid-js";
import type { UiDensity } from "../theme/context";
import { componentState, type ComponentStateProps } from "./types";

export interface BadgeProps {
  tone?: "neutral" | "info" | "success" | "warning" | "danger";
  variant?: ComponentStateProps["variant"];
  size?: ComponentStateProps["size"];
  density?: UiDensity;
  loading?: boolean;
  invalid?: boolean;
  children: JSX.Element;
}

export function Badge(props: BadgeProps) {
  return (
    <span
      class={`ui-badge ${props.tone ?? "neutral"}`}
      data-component="badge"
      data-variant={props.variant ?? props.tone ?? "neutral"}
      data-size={props.size ?? "normal"}
      data-density={props.density}
      data-state={componentState(props)}
      data-invalid={props.invalid || undefined}
    >
      {props.children}
    </span>
  );
}

export interface PageHeadingProps {
  title: JSX.Element;
  description?: JSX.Element;
  eyebrow?: JSX.Element;
  badge?: JSX.Element;
  badgeTone?: BadgeProps["tone"];
  actions?: JSX.Element;
  class?: string;
}

export function PageHeading(props: PageHeadingProps) {
  return (
    <header
      class={["page-heading", props.class].filter(Boolean).join(" ")}
      data-component="page-heading"
    >
      <div data-component="page-heading-copy">
        <Show when={props.eyebrow}>
          <span data-component="page-heading-eyebrow">{props.eyebrow}</span>
        </Show>
        <h1 data-component="page-heading-title">{props.title}</h1>
        <Show when={props.description}>
          <p data-component="page-heading-description">{props.description}</p>
        </Show>
      </div>
      <Show when={props.badge}>
        <Badge tone={props.badgeTone ?? "neutral"}>{props.badge}</Badge>
      </Show>
      <Show when={props.actions}>{props.actions}</Show>
    </header>
  );
}

export interface SettingsSectionProps {
  title: string;
  children: JSX.Element;
  variant?: ComponentStateProps["variant"];
  size?: ComponentStateProps["size"];
  density?: UiDensity;
  disabled?: boolean;
  loading?: boolean;
  invalid?: boolean;
}

export function SettingsSection(props: SettingsSectionProps) {
  return (
    <section
      class="settings-section-demo"
      data-component="settings-section"
      data-variant={props.variant ?? "default"}
      data-size={props.size ?? "normal"}
      data-density={props.density}
      data-state={componentState(props)}
      data-invalid={props.invalid || undefined}
    >
      <header class="settings-section-heading">
        <div>
          <h2 data-component="settings-section-title">{props.title}</h2>
        </div>
      </header>
      {props.children}
    </section>
  );
}

export interface SettingsRowProps {
  label: string;
  description?: string;
  children: JSX.Element;
  variant?: ComponentStateProps["variant"];
  size?: ComponentStateProps["size"];
  tone?: ComponentStateProps["tone"];
  density?: UiDensity;
  disabled?: boolean;
  loading?: boolean;
  invalid?: boolean;
}

export function SettingsRow(props: SettingsRowProps) {
  return (
    <div
      class="settings-row-gallery settings-row-demo"
      data-component="settings-row"
      data-variant={props.variant ?? "default"}
      data-size={props.size ?? "normal"}
      data-tone={props.tone ?? "neutral"}
      data-density={props.density}
      data-state={componentState(props)}
      data-invalid={props.invalid || undefined}
    >
      <div class="settings-row-copy">
        <div>{props.label}</div>
        <Show when={props.description}>
          <div data-component="settings-row-description">{props.description}</div>
        </Show>
      </div>
      <div class="settings-control">{props.children}</div>
    </div>
  );
}
