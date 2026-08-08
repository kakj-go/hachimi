import { Button } from "@hachimi/ui";
import { For, Show, type JSX } from "solid-js";

import "./authorization-workspace.css";

export interface AuthorizationWorkspaceSection<T extends string> {
  value: T;
  label: string;
  description: string;
  icon: JSX.Element;
  count?: number;
}

export function AuthorizationWorkspace<T extends string>(props: {
  ariaLabel: string;
  class?: string;
  sections: readonly AuthorizationWorkspaceSection<T>[];
  value: T;
  disabled?: boolean;
  onChange: (value: T) => void;
  children: JSX.Element;
  footer: JSX.Element;
}) {
  const selected = () => props.sections.find((section) => section.value === props.value);

  return (
    <section
      class={["authorization-workspace", props.class].filter(Boolean).join(" ")}
      data-component="authorization-workspace"
    >
      <nav class="authorization-workspace-nav" aria-label={props.ariaLabel}>
        <span class="authorization-workspace-nav-label">{props.ariaLabel}</span>
        <For each={props.sections}>
          {(section) => (
            <Button
              type="button"
              variant="ghost"
              class="authorization-workspace-nav-item"
              classList={{ selected: props.value === section.value }}
              aria-current={props.value === section.value ? "page" : undefined}
              disabled={props.disabled}
              title={section.label}
              onClick={() => props.onChange(section.value)}
            >
              <span class="authorization-workspace-nav-icon">{section.icon}</span>
              <span>{section.label}</span>
              <Show when={section.count !== undefined}>
                <small>{section.count}</small>
              </Show>
            </Button>
          )}
        </For>
      </nav>
      <div class="authorization-workspace-main">
        <div class="authorization-workspace-scroll">
          <header class="authorization-workspace-heading">
            <h3>{selected()?.label}</h3>
            <p>{selected()?.description}</p>
          </header>
          {props.children}
        </div>
        <footer class="authorization-workspace-footer">{props.footer}</footer>
      </div>
    </section>
  );
}
