import { Tabs as KTabs } from "@kobalte/core";
import { For, type JSX } from "solid-js";

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
}

export function Tabs(props: TabsProps) {
  return (
    <KTabs.Root
      value={props.value}
      orientation={props.orientation ?? "horizontal"}
      onChange={props.onChange ?? (() => undefined)}
      data-component="tabs"
      data-variant="default"
      data-size="normal"
      data-state="idle"
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
