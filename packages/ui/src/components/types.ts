import type { UiDensity } from "../theme/context";

/** Shared state vocabulary used by every interactive Hachimi UI primitive. */
export type ComponentVariant = "default" | "primary" | "ghost" | "danger" | "filled";
export type ComponentSize = "small" | "normal" | "large";
export type ComponentTone = "neutral" | "accent" | "danger";

export interface ComponentStateProps {
  variant?: ComponentVariant | undefined;
  size?: ComponentSize | undefined;
  tone?: ComponentTone | undefined;
  density?: UiDensity | undefined;
  disabled?: boolean | undefined;
  loading?: boolean | undefined;
  invalid?: boolean | undefined;
}

export function componentState(state: {
  disabled?: boolean | undefined;
  loading?: boolean | undefined;
  invalid?: boolean | undefined;
}): "disabled" | "loading" | "invalid" | "idle" {
  if (state.loading) return "loading";
  if (state.disabled) return "disabled";
  if (state.invalid) return "invalid";
  return "idle";
}
