import { Dialog as KDialog } from "@kobalte/core";
import { Show, type JSX } from "solid-js";
import { Button } from "./button";
import { X } from "lucide-solid";
import type { UiDensity } from "../theme/context";
import { componentState, type ComponentStateProps } from "./types";

export interface DialogProps {
  open: boolean;
  title: string;
  description?: string;
  closeLabel?: string;
  size?: "normal" | "wide";
  children: JSX.Element;
  onOpenChange: (open: boolean) => void;
  variant?: ComponentStateProps["variant"];
  tone?: ComponentStateProps["tone"];
  density?: UiDensity;
  disabled?: boolean;
  loading?: boolean;
  invalid?: boolean;
}

export function Dialog(props: DialogProps) {
  return (
    <KDialog.Root open={props.open} onOpenChange={props.onOpenChange} modal>
      <KDialog.Portal>
        <KDialog.Overlay data-component="dialog-overlay" />
        <KDialog.Content
          data-component="dialog-content"
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
        >
          <KDialog.Title data-component="dialog-title">{props.title}</KDialog.Title>
          <Show when={props.description}>
            <KDialog.Description data-component="dialog-description">
              {props.description}
            </KDialog.Description>
          </Show>
          {props.children}
          <KDialog.CloseButton
            as={Button}
            data-component="dialog-close"
            variant="ghost"
            size="small"
            aria-label={props.closeLabel ?? "Close"}
          >
            <X size={18} aria-hidden="true" />
            <span class="sr-only">{props.closeLabel ?? "Close"}</span>
          </KDialog.CloseButton>
        </KDialog.Content>
      </KDialog.Portal>
    </KDialog.Root>
  );
}
