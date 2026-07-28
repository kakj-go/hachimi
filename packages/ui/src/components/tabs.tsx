import { Tabs as KTabs } from "@kobalte/core";
import { For, type JSX } from "solid-js";
import type { UiDensity } from "../theme/context";
import { componentState, type ComponentStateProps } from "./types";

export interface TabDefinition {
  value: string;
  label: string;
  content: JSX.Element;
}

export interface TabsProps {
  value: string;
  tabs: readonly TabDefinition[];
  orientation?: "horizontal" | "vertical";
  onChange?: (value: string) => void;
  variant?: ComponentStateProps["variant"];
  size?: ComponentStateProps["size"];
  tone?: ComponentStateProps["tone"];
  density?: UiDensity;
  disabled?: boolean;
  loading?: boolean;
  invalid?: boolean;
}

export function Tabs(props: TabsProps) {
  return (
    <KTabs.Root
      value={props.value}
      orientation={props.orientation ?? "horizontal"}
      onChange={props.onChange ?? (() => undefined)}
      data-component="tabs"
      data-variant={props.variant ?? "default"}
      data-size={props.size ?? "normal"}
      data-tone={props.tone ?? "neutral"}
      data-density={props.density}
      data-state={componentState(props)}
      data-invalid={props.invalid || undefined}
      disabled={Boolean(props.disabled || props.loading)}
    >
      <KTabs.List data-component="tabs-list">
        <For each={props.tabs}>
          {(tab) => (
            <KTabs.Trigger
              value={tab.value}
              data-component="tabs-trigger"
              data-variant="default"
              data-size="normal"
            >
              {tab.label}
            </KTabs.Trigger>
          )}
        </For>
        <KTabs.Indicator />
      </KTabs.List>
      <For each={props.tabs}>
        {(tab) => (
          <KTabs.Content value={tab.value} data-component="tabs-content">
            {tab.content}
          </KTabs.Content>
        )}
      </For>
    </KTabs.Root>
  );
}
