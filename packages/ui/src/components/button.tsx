import { splitProps, type JSX } from "solid-js";
import type { UiDensity } from "../theme/context";
import type { ComponentStateProps } from "./types";

export type ButtonVariant = "default" | "primary" | "ghost" | "danger";
export type ButtonSize = "small" | "normal" | "large";

export interface ButtonProps
  extends
    Omit<JSX.ButtonHTMLAttributes<HTMLButtonElement>, "disabled">,
    Pick<ComponentStateProps, "disabled" | "loading" | "invalid"> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  tone?: "neutral" | "accent" | "danger";
  density?: UiDensity;
  loading?: boolean;
  invalid?: boolean;
}

export function Button(props: ButtonProps) {
  const [local, rest] = splitProps(props, [
    "variant",
    "size",
    "tone",
    "density",
    "loading",
    "invalid",
    "children",
    "disabled",
    "class",
  ]);
  const className = () =>
    [
      "ui-button",
      local.variant && local.variant !== "default" ? local.variant : undefined,
      local.size && local.size !== "normal" ? local.size : undefined,
      local.class,
    ]
      .filter(Boolean)
      .join(" ");
  return (
    <button
      type="button"
      class={className()}
      data-component="button"
      data-variant={local.variant ?? "default"}
      data-size={local.size ?? "normal"}
      data-tone={local.tone ?? "neutral"}
      data-density={local.density}
      data-state={
        local.loading ? "loading" : local.disabled ? "disabled" : local.invalid ? "invalid" : "idle"
      }
      data-invalid={local.invalid || undefined}
      aria-busy={local.loading || undefined}
      aria-invalid={local.invalid || undefined}
      disabled={local.disabled || local.loading}
      {...rest}
    >
      {local.children}
    </button>
  );
}

export interface IconButtonProps extends ButtonProps {
  label: string;
}

export function IconButton(props: IconButtonProps) {
  const [local, rest] = splitProps(props, ["label", "class"]);
  return (
    <Button
      aria-label={local.label}
      title={local.label}
      class={["ui-icon-button", local.class].filter(Boolean).join(" ")}
      {...rest}
      data-component="icon-button"
    />
  );
}
