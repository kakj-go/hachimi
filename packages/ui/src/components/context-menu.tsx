import { ContextMenu as KContextMenu } from "@kobalte/core";
import { For, Show, onCleanup, onMount, type JSX } from "solid-js";
import type { UiDensity } from "../theme/context";
import { componentState, type ComponentStateProps } from "./types";

export interface ContextMenuAction {
  kind?: "item";
  id: string;
  label: string;
  disabled?: boolean;
  checked?: boolean;
  onSelect?: () => void;
}

export interface ContextMenuSeparator {
  kind: "separator";
  id: string;
}

export type ContextMenuEntry = ContextMenuAction | ContextMenuSeparator;

export interface ContextMenuProps {
  trigger: JSX.Element;
  entries: readonly ContextMenuEntry[];
  onOpenChange?: (open: boolean) => void;
  contentRef?: (element: HTMLDivElement) => void;
  variant?: ComponentStateProps["variant"];
  size?: ComponentStateProps["size"];
  tone?: ComponentStateProps["tone"];
  density?: UiDensity;
  disabled?: boolean;
  loading?: boolean;
  invalid?: boolean;
}

export function ContextMenu(props: ContextMenuProps) {
  let contentElement: HTMLDivElement | undefined;
  const dismiss = () => {
    contentElement?.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Escape",
        code: "Escape",
        bubbles: true,
        cancelable: true,
      }),
    );
  };
  const closeOnWindowBlur = () => {
    dismiss();
  };

  onMount(() => window.addEventListener("blur", closeOnWindowBlur));
  onCleanup(() => window.removeEventListener("blur", closeOnWindowBlur));

  return (
    <KContextMenu.Root
      onOpenChange={(open) => {
        if (!open) contentElement = undefined;
        props.onOpenChange?.(open);
      }}
      // A desktop pet window is intentionally small and can sit at any edge
      // of a monitor. Let Kobalte/Floating UI flip and slide the menu into the
      // available viewport instead of allowing it to be clipped by the
      // WebView's transparent bounds.
      placement="bottom-start"
      flip="top-start top-end bottom-end bottom-start"
      slide
      overflowPadding={12}
    >
      <KContextMenu.Trigger
        data-component="context-menu-trigger"
        data-variant={props.variant ?? "default"}
        data-size={props.size ?? "normal"}
        data-tone={props.tone ?? "neutral"}
        data-density={props.density}
        data-state={componentState(props)}
        data-invalid={props.invalid || undefined}
        aria-busy={props.loading || undefined}
        aria-invalid={props.invalid || undefined}
        disabled={Boolean(props.disabled || props.loading)}
      >
        {props.trigger}
      </KContextMenu.Trigger>
      <KContextMenu.Portal>
        <KContextMenu.Content
          data-component="menu-content"
          data-variant={props.variant ?? "default"}
          data-size={props.size ?? "normal"}
          data-tone={props.tone ?? "neutral"}
          data-density={props.density}
          data-state="open"
          ref={(element) => {
            contentElement = element;
            props.contentRef?.(element);
          }}
        >
          <For each={props.entries}>
            {(entry) => (
              <Show
                when={entry.kind !== "separator"}
                fallback={<KContextMenu.Separator data-component="menu-separator" />}
              >
                <KContextMenu.Item
                  data-component="menu-item"
                  data-variant={(entry as ContextMenuAction).checked ? "checked" : "default"}
                  data-size="normal"
                  data-state={(entry as ContextMenuAction).disabled ? "disabled" : "idle"}
                  disabled={(entry as ContextMenuAction).disabled ?? false}
                  onSelect={() => (entry as ContextMenuAction).onSelect?.()}
                >
                  <span>{(entry as ContextMenuAction).label}</span>
                  <Show when={(entry as ContextMenuAction).checked}>✓</Show>
                </KContextMenu.Item>
              </Show>
            )}
          </For>
        </KContextMenu.Content>
      </KContextMenu.Portal>
    </KContextMenu.Root>
  );
}
