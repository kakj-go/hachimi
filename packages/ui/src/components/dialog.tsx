import { Dialog as KDialog } from "@kobalte/core";
import { Show, type JSX } from "solid-js";
import { Button } from "./button";
import { X } from "lucide-solid";

export interface DialogProps {
  open: boolean;
  title: string;
  description?: string;
  children: JSX.Element;
  onOpenChange: (open: boolean) => void;
}

export function Dialog(props: DialogProps) {
  return (
    <KDialog.Root open={props.open} onOpenChange={props.onOpenChange}>
      <KDialog.Portal>
        <KDialog.Overlay data-component="dialog-overlay" />
        <KDialog.Content
          data-component="dialog-content"
          data-variant="default"
          data-size="normal"
          data-state="open"
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
            aria-label="Close"
          >
            <X size={16} aria-hidden="true" />
            <span class="sr-only">Close</span>
          </KDialog.CloseButton>
        </KDialog.Content>
      </KDialog.Portal>
    </KDialog.Root>
  );
}
