import { Popover as KPopover } from "@kobalte/core";
import type { JSX } from "solid-js";
import type { UiDensity } from "../theme/context";
import { componentState, type ComponentStateProps } from "./types";

export interface FloatingPopoverProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  label: string;
  trigger: JSX.Element;
  children: JSX.Element;
  triggerClass?: string;
  triggerTestId?: string;
  contentClass?: string;
  contentTestId?: string;
  disabled?: boolean;
  placement?: "top-start" | "top-end" | "bottom-start" | "bottom-end";
  gutter?: number;
  variant?: ComponentStateProps["variant"];
  size?: ComponentStateProps["size"];
  tone?: ComponentStateProps["tone"];
  density?: UiDensity;
  loading?: boolean;
  invalid?: boolean;
}

/**
 * Theme-neutral floating surface with desktop-friendly viewport collision and
 * dismissal behavior. Product-specific layout stays with the caller.
 */
export function FloatingPopover(props: FloatingPopoverProps) {
  return (
    <KPopover.Root
      open={props.open}
      onOpenChange={props.onOpenChange}
      placement={props.placement ?? "top-start"}
      gutter={props.gutter ?? 8}
      flip="bottom-start bottom-end top-end top-start"
      slide
      overflowPadding={12}
    >
      <KPopover.Trigger
        data-component="floating-popover-trigger"
        data-variant={props.variant ?? "default"}
        data-size={props.size ?? "normal"}
        data-tone={props.tone ?? "neutral"}
        data-density={props.density}
        data-state={componentState({
          disabled: props.disabled,
          loading: props.loading,
          invalid: props.invalid,
        })}
        data-invalid={props.invalid || undefined}
        aria-busy={props.loading || undefined}
        aria-invalid={props.invalid || undefined}
        data-testid={props.triggerTestId}
        class={props.triggerClass}
        aria-label={props.label}
        disabled={props.disabled}
      >
        {props.trigger}
      </KPopover.Trigger>
      <KPopover.Portal>
        <KPopover.Content
          data-component="floating-popover-content"
          data-variant={props.variant ?? "default"}
          data-size={props.size ?? "normal"}
          data-tone={props.tone ?? "neutral"}
          data-density={props.density}
          data-testid={props.contentTestId}
          class={props.contentClass}
          onOpenAutoFocus={(event) => event.preventDefault()}
        >
          <KPopover.Title class="sr-only">{props.label}</KPopover.Title>
          {props.children}
        </KPopover.Content>
      </KPopover.Portal>
    </KPopover.Root>
  );
}

/** Canonical public name; FloatingPopover remains as a descriptive alias. */
export function Popover(props: FloatingPopoverProps) {
  return <FloatingPopover {...props} />;
}
