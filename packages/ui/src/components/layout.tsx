import { Show, type JSX } from "solid-js";

export interface BadgeProps {
  tone?: "neutral" | "info" | "success" | "warning" | "danger";
  children: JSX.Element;
}

export function Badge(props: BadgeProps) {
  return (
    <span
      data-component="badge"
      data-variant={props.tone ?? "neutral"}
      data-size="normal"
      data-state="idle"
    >
      {props.children}
    </span>
  );
}

export interface SettingsSectionProps {
  title: string;
  children: JSX.Element;
}

export function SettingsSection(props: SettingsSectionProps) {
  return (
    <section data-component="settings-section" data-variant="default" data-state="idle">
      <h2 data-component="settings-section-title">{props.title}</h2>
      {props.children}
    </section>
  );
}

export interface SettingsRowProps {
  label: string;
  description?: string;
  children: JSX.Element;
}

export function SettingsRow(props: SettingsRowProps) {
  return (
    <div data-component="settings-row" data-variant="default" data-state="idle">
      <div>
        <div>{props.label}</div>
        <Show when={props.description}>
          <div data-component="settings-row-description">{props.description}</div>
        </Show>
      </div>
      <div>{props.children}</div>
    </div>
  );
}
