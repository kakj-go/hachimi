import { splitProps, type JSX } from "solid-js";

export type ButtonVariant = "default" | "primary" | "ghost" | "danger";
export type ButtonSize = "small" | "normal" | "large";

export interface ButtonProps extends JSX.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
}

export function Button(props: ButtonProps) {
  const [local, rest] = splitProps(props, ["variant", "size", "children", "disabled"]);
  return (
    <button
      type="button"
      data-component="button"
      data-variant={local.variant ?? "default"}
      data-size={local.size ?? "normal"}
      data-state={local.disabled ? "disabled" : "idle"}
      disabled={local.disabled}
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
  const [local, rest] = splitProps(props, ["label"]);
  return (
    <Button aria-label={local.label} title={local.label} {...rest} data-component="icon-button" />
  );
}
