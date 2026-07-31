import {
  DropdownMenu as KDropdownMenu,
  Select as KSelect,
  Slider as KSlider,
  Switch as KSwitch,
  TextField as KTextField,
} from "@kobalte/core";
import { Check, ChevronDown, X } from "lucide-solid";
import {
  createEffect,
  createMemo,
  createSignal,
  createUniqueId,
  For,
  onCleanup,
  onMount,
  Show,
  splitProps,
  untrack,
  type JSX,
} from "solid-js";
import type { UiDensity } from "../theme/context";
import { componentState, type ComponentStateProps } from "./types";

export type ControlSize = ComponentStateProps["size"];
export type ControlTone = ComponentStateProps["tone"];

export interface TextFieldProps {
  label: string;
  testId?: string;
  value?: string;
  placeholder?: string;
  description?: string;
  disabled?: boolean;
  loading?: boolean;
  invalid?: boolean;
  error?: string;
  variant?: "default" | "filled";
  size?: ControlSize;
  tone?: ControlTone;
  density?: UiDensity;
  type?: JSX.InputHTMLAttributes<HTMLInputElement>["type"];
  maxLength?: number;
  autofocus?: boolean;
  onInput?: JSX.EventHandler<HTMLInputElement, InputEvent>;
  onKeyDown?: JSX.EventHandler<HTMLInputElement, KeyboardEvent>;
}

export function TextField(props: TextFieldProps) {
  return (
    <KTextField.Root
      class="field-stack"
      data-component="form-field"
      data-variant={props.variant ?? "default"}
      data-size={props.size ?? "normal"}
      data-tone={props.tone ?? "neutral"}
      data-density={props.density}
      data-state={
        props.loading
          ? "loading"
          : props.disabled
            ? "disabled"
            : props.invalid || props.error
              ? "invalid"
              : "idle"
      }
      data-invalid={props.invalid || Boolean(props.error)}
      disabled={Boolean(props.disabled || props.loading)}
      value={props.value ?? ""}
    >
      <KTextField.Label class="field-label" data-component="form-label">
        {props.label}
      </KTextField.Label>
      <KTextField.Input
        class="ui-input"
        data-component="text-field-input"
        data-variant={props.variant ?? "default"}
        data-size={props.size ?? "normal"}
        data-tone={props.tone ?? "neutral"}
        data-density={props.density}
        data-state={
          props.loading
            ? "loading"
            : props.disabled
              ? "disabled"
              : props.invalid || props.error
                ? "invalid"
                : "idle"
        }
        aria-busy={props.loading || undefined}
        data-testid={props.testId}
        classList={{ invalid: props.invalid || Boolean(props.error) }}
        aria-invalid={props.invalid || Boolean(props.error)}
        type={props.type ?? "text"}
        maxLength={props.maxLength}
        autofocus={props.autofocus}
        placeholder={props.placeholder ?? ""}
        onInput={props.onInput ?? (() => undefined)}
        onKeyDown={props.onKeyDown}
      />
      <Show when={props.description && !props.error}>
        <KTextField.Description data-component="field-description">
          {props.description}
        </KTextField.Description>
      </Show>
      <Show when={props.error}>
        <span class="field-error" data-component="field-error">
          {props.error}
        </span>
      </Show>
    </KTextField.Root>
  );
}

export interface FormFieldProps {
  label: string;
  description?: string;
  children: JSX.Element;
  variant?: ComponentStateProps["variant"];
  size?: ControlSize;
  tone?: ControlTone;
  density?: UiDensity;
  disabled?: boolean;
  loading?: boolean;
  invalid?: boolean;
}

export function FormField(props: FormFieldProps) {
  return (
    <div
      class="field-stack"
      data-component="form-field"
      data-variant={props.variant ?? "default"}
      data-size={props.size ?? "normal"}
      data-tone={props.tone ?? "neutral"}
      data-density={props.density}
      data-state={componentState(props)}
      data-invalid={props.invalid || undefined}
      aria-busy={props.loading || undefined}
    >
      <span class="field-label" data-component="form-label">
        {props.label}
      </span>
      {props.children}
      <Show when={props.description}>
        <span data-component="field-description">{props.description}</span>
      </Show>
    </div>
  );
}

export interface SwitchProps {
  checked: boolean;
  label: string;
  testId?: string;
  disabled?: boolean;
  loading?: boolean;
  size?: ControlSize;
  tone?: ControlTone;
  density?: UiDensity;
  variant?: "default" | "filled";
  invalid?: boolean;
  onChange?: (checked: boolean) => void;
}

export function Switch(props: SwitchProps) {
  return (
    <KSwitch.Root
      checked={props.checked}
      disabled={Boolean(props.disabled || props.loading)}
      onChange={props.onChange ?? (() => undefined)}
      data-component="switch-root"
      data-testid={props.testId}
      data-variant={props.variant ?? "default"}
      data-size={props.size ?? "normal"}
      data-tone={props.tone ?? "neutral"}
      data-density={props.density}
      data-state={componentState(props)}
      data-invalid={props.invalid || undefined}
      aria-busy={props.loading || undefined}
      aria-invalid={props.invalid || undefined}
    >
      <KSwitch.Input />
      <KSwitch.Control
        class="ui-switch"
        classList={{ checked: props.checked }}
        data-component="switch"
      >
        <KSwitch.Thumb data-component="switch-thumb" />
      </KSwitch.Control>
      <KSwitch.Label class="sr-only">{props.label}</KSwitch.Label>
    </KSwitch.Root>
  );
}

export interface NativeSelectProps extends JSX.SelectHTMLAttributes<HTMLSelectElement> {
  label: string;
  variant?: "default" | "filled";
  size?: ControlSize;
  tone?: ControlTone;
  density?: UiDensity;
  loading?: boolean;
  invalid?: boolean;
}

export function NativeSelect(props: NativeSelectProps) {
  const [local, rest] = splitProps(props, [
    "label",
    "children",
    "value",
    "variant",
    "size",
    "tone",
    "density",
    "loading",
    "invalid",
  ]);
  let selectElement!: HTMLSelectElement;

  const syncSelection = (value: NativeSelectProps["value"]) => {
    queueMicrotask(() => {
      if (!selectElement || value === undefined || value === null) return;
      if (Array.isArray(value)) {
        const selectedValues = new Set(value.map(String));
        for (const option of selectElement.options) {
          option.selected = selectedValues.has(option.value);
        }
        return;
      }
      selectElement.value = String(value);
    });
  };

  createEffect(() => syncSelection(local.value));
  onMount(() => {
    const observer = new MutationObserver(() => syncSelection(local.value));
    observer.observe(selectElement, { childList: true });
    onCleanup(() => observer.disconnect());
  });

  return (
    <label
      class="field-stack"
      data-component="form-field"
      data-variant={local.variant ?? "default"}
      data-size={local.size ?? "normal"}
      data-tone={local.tone ?? "neutral"}
      data-density={local.density}
      data-state={componentState(local)}
      data-invalid={local.invalid || undefined}
    >
      <span class="field-label" data-component="form-label">
        {local.label}
      </span>
      <select
        ref={selectElement}
        class="ui-input"
        data-component="text-field-input"
        data-variant={local.variant ?? "default"}
        data-size={local.size ?? "normal"}
        data-tone={local.tone ?? "neutral"}
        data-density={local.density}
        data-state={componentState({
          loading: local.loading,
          disabled: rest.disabled,
          invalid: local.invalid,
        })}
        data-invalid={local.invalid || undefined}
        aria-invalid={local.invalid || undefined}
        aria-busy={local.loading || undefined}
        disabled={rest.disabled || local.loading}
        value={local.value}
        {...rest}
      >
        {local.children}
      </select>
    </label>
  );
}

export interface SelectOption {
  value: string;
  label: string;
  description?: string;
  disabled?: boolean;
  preview?: {
    accent: string;
    background: string;
    foreground?: string;
    fontFamily?: string;
  };
}

type ResolvedSelectOption = SelectOption & { key: string };

function selectOptionKey(value: string): string {
  let hash = 2_166_136_261;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 16_777_619);
  }
  return `option-${hash >>> 0}`;
}

export interface SelectFieldProps {
  label: string;
  testId?: string;
  value: string;
  options: readonly SelectOption[];
  description?: string;
  placeholder?: string;
  disabled?: boolean;
  loading?: boolean;
  invalid?: boolean;
  error?: string;
  variant?: "default" | "filled";
  size?: ControlSize;
  tone?: ControlTone;
  density?: UiDensity;
  onChange?: (value: string) => void;
}

export function SelectField(props: SelectFieldProps) {
  const options = createMemo<ResolvedSelectOption[]>(() =>
    props.options.map((option) => ({ ...option, key: selectOptionKey(option.value) })),
  );
  const selected = createMemo(
    () => options().find((option) => option.value === props.value) ?? null,
  );
  const state = () =>
    props.loading
      ? "loading"
      : props.disabled
        ? "disabled"
        : props.invalid || props.error
          ? "invalid"
          : "idle";
  return (
    <div
      class="ui-select field-stack"
      data-component="select"
      data-variant={props.variant ?? "default"}
      data-size={props.size ?? "normal"}
      data-tone={props.tone ?? "neutral"}
      data-density={props.density}
      data-state={state()}
      data-invalid={props.invalid || Boolean(props.error)}
    >
      <KSelect.Root<ResolvedSelectOption>
        class="ui-select-control"
        style={{ width: "100%", "min-width": "0", "max-width": "100%" }}
        options={options()}
        optionValue="key"
        optionTextValue="label"
        optionDisabled="disabled"
        multiple={false}
        value={selected()}
        placeholder={props.placeholder ?? ""}
        disabled={Boolean(props.disabled || props.loading)}
        onChange={(option) => {
          if (option && option.value !== props.value) props.onChange?.(option.value);
        }}
        itemComponent={(itemProps) => (
          <KSelect.Item item={itemProps.item} class="ui-select-option" data-component="select-item">
            <span data-component="select-item-main">
              <SelectPreview option={itemProps.item.rawValue} />
              <span data-component="select-item-copy">
                <KSelect.ItemLabel>{itemProps.item.rawValue.label}</KSelect.ItemLabel>
                <Show when={itemProps.item.rawValue.description}>
                  <span>{itemProps.item.rawValue.description}</span>
                </Show>
              </span>
            </span>
            <KSelect.ItemIndicator data-component="select-indicator">
              <Check size={14} />
            </KSelect.ItemIndicator>
          </KSelect.Item>
        )}
      >
        <KSelect.Label class="field-label" data-component="form-label">
          {props.label}
        </KSelect.Label>
        <KSelect.Trigger
          class="ui-select-trigger"
          classList={{ invalid: props.invalid || Boolean(props.error) }}
          style={{ width: "100%", "min-width": "0", "max-width": "100%" }}
          data-component="select-trigger"
          data-value={props.value}
          data-testid={props.testId}
          aria-invalid={props.invalid || Boolean(props.error)}
          aria-busy={props.loading || undefined}
        >
          <KSelect.Value<ResolvedSelectOption>>
            {(selectState) => (
              <span data-component="select-value">
                <SelectPreview option={selectState.selectedOption()} />
                <span>{selectState.selectedOption().label}</span>
              </span>
            )}
          </KSelect.Value>
          <KSelect.Icon data-component="select-icon">
            <ChevronDown size={15} />
          </KSelect.Icon>
        </KSelect.Trigger>
        <Show when={props.description && !props.error}>
          <KSelect.Description data-component="field-description">
            {props.description}
          </KSelect.Description>
        </Show>
        <Show when={props.error}>
          <span class="field-error" data-component="field-error">
            {props.error}
          </span>
        </Show>
        <KSelect.Portal>
          <KSelect.Content class="ui-select-popover" data-component="select-content">
            <KSelect.Listbox data-component="select-listbox" />
          </KSelect.Content>
        </KSelect.Portal>
      </KSelect.Root>
    </div>
  );
}

/** Canonical public name; SelectField remains as a compatibility alias. */
export function Select(props: SelectFieldProps) {
  return <SelectField {...props} />;
}

function SelectPreview(props: { option: SelectOption }) {
  return (
    <Show when={props.option.preview}>
      {(preview) => (
        <span
          data-component="select-preview"
          style={{
            "--select-preview-accent": preview().accent,
            "--select-preview-background": preview().background,
            "--select-preview-foreground": preview().foreground ?? preview().accent,
            "--select-preview-font": preview().fontFamily ?? "inherit",
          }}
          aria-hidden="true"
        >
          Aa
        </span>
      )}
    </Show>
  );
}

export interface ColorFieldProps {
  label: string;
  value: string;
  description?: string;
  disabled?: boolean;
  loading?: boolean;
  error?: string | undefined;
  size?: ControlSize;
  tone?: ControlTone;
  density?: UiDensity;
  variant?: "default" | "filled";
  invalid?: boolean;
  onInput?: (value: string) => void;
  onCommit?: (value: string) => void;
}

export function ColorField(props: ColorFieldProps) {
  const id = `color-${createUniqueId()}`;
  const [draft, setDraft] = createSignal(untrack(() => props.value));
  let textInput!: HTMLInputElement;
  createEffect(() => {
    const value = props.value;
    if (document.activeElement !== textInput) setDraft(value);
  });
  return (
    <div
      class="field-stack"
      data-component="form-field"
      data-variant={props.variant ?? "default"}
      data-size={props.size ?? "normal"}
      data-tone={props.tone ?? "neutral"}
      data-density={props.density}
      data-state={componentState({ ...props, invalid: props.invalid || Boolean(props.error) })}
      data-invalid={props.invalid || Boolean(props.error) || undefined}
      aria-busy={props.loading || undefined}
    >
      <label class="field-label" data-component="form-label" for={id}>
        {props.label}
      </label>
      <div data-component="color-field">
        <input
          id={id}
          type="color"
          value={/^#[\dA-Fa-f]{6}$/.test(draft()) ? draft() : "#000000"}
          disabled={props.disabled || props.loading}
          aria-label={`${props.label} color picker`}
          onInput={(event) => {
            const value = event.currentTarget.value.toUpperCase();
            setDraft(value);
            props.onInput?.(value);
          }}
          onChange={(event) => props.onCommit?.(event.currentTarget.value.toUpperCase())}
        />
        <input
          ref={textInput}
          type="text"
          value={draft()}
          disabled={props.disabled || props.loading}
          aria-invalid={props.invalid || Boolean(props.error) || undefined}
          aria-label={props.label}
          maxlength={7}
          spellcheck={false}
          onInput={(event) => {
            setDraft(event.currentTarget.value);
            props.onInput?.(event.currentTarget.value);
          }}
          onBlur={() => props.onCommit?.(draft())}
        />
      </div>
      <Show when={props.description && !props.error}>
        <span data-component="field-description">{props.description}</span>
      </Show>
      <Show when={props.error}>
        <span data-component="field-error">{props.error}</span>
      </Show>
    </div>
  );
}

export interface RangeFieldProps {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  unit?: string;
  disabled?: boolean;
  loading?: boolean;
  size?: ControlSize;
  tone?: ControlTone;
  density?: UiDensity;
  variant?: "default" | "filled";
  invalid?: boolean;
  onInput?: (value: number) => void;
  onCommit?: (value: number) => void;
}

export function RangeField(props: RangeFieldProps) {
  const commitKeys = new Set([
    "ArrowLeft",
    "ArrowRight",
    "ArrowUp",
    "ArrowDown",
    "Home",
    "End",
    "PageUp",
    "PageDown",
  ]);

  return (
    <KSlider.Root
      data-component="range-field"
      data-variant={props.variant ?? "default"}
      data-size={props.size ?? "normal"}
      data-tone={props.tone ?? "neutral"}
      data-density={props.density}
      data-state={componentState(props)}
      data-invalid={props.invalid || undefined}
      aria-busy={props.loading || undefined}
      aria-invalid={props.invalid || undefined}
      value={[props.value]}
      minValue={props.min}
      maxValue={props.max}
      step={props.step ?? 1}
      disabled={Boolean(props.disabled || props.loading)}
      onChange={(values) => props.onInput?.(values[0] ?? props.value)}
      onChangeEnd={(values) => props.onCommit?.(values[0] ?? props.value)}
    >
      <div data-component="range-header">
        <KSlider.Label data-component="form-label">{props.label}</KSlider.Label>
        <output>
          {props.value}
          {props.unit ?? ""}
        </output>
      </div>
      <KSlider.Track data-component="range-track">
        <KSlider.Fill data-component="range-fill" />
        <KSlider.Thumb
          data-component="range-thumb"
          style={{
            left: `${((props.value - props.min) / (props.max - props.min)) * 100}%`,
            transform: "translate(-50%, -50%)",
          }}
          onKeyUp={(event) => {
            if (commitKeys.has(event.key)) props.onCommit?.(props.value);
          }}
        >
          <KSlider.Input />
        </KSlider.Thumb>
      </KSlider.Track>
    </KSlider.Root>
  );
}

export interface SegmentOption<T extends string> {
  value: T;
  label: string;
}

export function SegmentedControl<T extends string>(props: {
  label: string;
  value: T;
  options: readonly SegmentOption<T>[];
  disabled?: boolean;
  loading?: boolean;
  size?: ControlSize;
  tone?: ControlTone;
  density?: UiDensity;
  variant?: "default" | "filled";
  invalid?: boolean;
  onChange?: (value: T) => void;
}) {
  return (
    <div
      class="ui-segmented"
      data-component="segmented-control"
      data-variant={props.variant ?? "default"}
      data-size={props.size ?? "normal"}
      data-tone={props.tone ?? "neutral"}
      data-density={props.density}
      data-state={componentState(props)}
      data-invalid={props.invalid || undefined}
      role="group"
      aria-label={props.label}
      aria-busy={props.loading || undefined}
    >
      <For each={props.options}>
        {(option) => (
          <button
            classList={{ selected: props.value === option.value }}
            type="button"
            aria-pressed={props.value === option.value}
            disabled={props.disabled || props.loading}
            onClick={() => props.onChange?.(option.value)}
          >
            {option.label}
          </button>
        )}
      </For>
    </div>
  );
}

export interface DropdownAction {
  id: string;
  label: string;
  icon?: JSX.Element;
  testId?: string;
  danger?: boolean;
  disabled?: boolean;
  separatorBefore?: boolean;
}

export function Dropdown(props: {
  label: string;
  triggerTestId?: string;
  actions: readonly DropdownAction[];
  onSelect: (id: string) => void;
  children: JSX.Element;
  variant?: ComponentStateProps["variant"];
  size?: ControlSize;
  tone?: ControlTone;
  density?: UiDensity;
  disabled?: boolean;
  loading?: boolean;
  invalid?: boolean;
}) {
  let trigger!: HTMLButtonElement;

  return (
    <KDropdownMenu.Root>
      <KDropdownMenu.Trigger
        ref={trigger}
        data-component="dropdown-trigger"
        data-variant={props.variant ?? "default"}
        data-size={props.size ?? "normal"}
        data-tone={props.tone ?? "neutral"}
        data-density={props.density}
        data-state={componentState(props)}
        data-invalid={props.invalid || undefined}
        aria-busy={props.loading || undefined}
        aria-invalid={props.invalid || undefined}
        disabled={props.disabled || props.loading}
        data-testid={props.triggerTestId}
        aria-label={props.label}
      >
        {props.children}
      </KDropdownMenu.Trigger>
      <KDropdownMenu.Portal>
        <KDropdownMenu.Content data-component="dropdown-content">
          <For each={props.actions}>
            {(action) => (
              <>
                <Show when={action.separatorBefore}>
                  <KDropdownMenu.Separator data-component="dropdown-separator" />
                </Show>
                <KDropdownMenu.Item
                  data-component="dropdown-item"
                  data-testid={action.testId}
                  data-danger={action.danger ?? false}
                  disabled={action.disabled ?? false}
                  onSelect={() => {
                    props.onSelect(action.id);
                    window.setTimeout(() => trigger.focus());
                  }}
                >
                  <span data-component="dropdown-item-label">
                    <Show when={action.icon}>
                      <span data-component="dropdown-item-icon">{action.icon}</span>
                    </Show>
                    <span>{action.label}</span>
                  </span>
                </KDropdownMenu.Item>
              </>
            )}
          </For>
        </KDropdownMenu.Content>
      </KDropdownMenu.Portal>
    </KDropdownMenu.Root>
  );
}

export function Toast(props: {
  open: boolean;
  tone?: "neutral" | "success" | "danger" | undefined;
  onClose?: () => void;
  children: JSX.Element;
  variant?: ComponentStateProps["variant"];
  size?: ControlSize;
  density?: UiDensity;
  disabled?: boolean;
  loading?: boolean;
  invalid?: boolean;
}) {
  return (
    <Show when={props.open}>
      <div
        data-component="toast"
        data-variant={props.variant ?? "default"}
        data-size={props.size ?? "normal"}
        data-tone={props.tone ?? "neutral"}
        data-density={props.density}
        data-state={componentState(props)}
        data-invalid={props.invalid || undefined}
        role="status"
      >
        <span>{props.children}</span>
        <button type="button" aria-label="Close" onClick={() => props.onClose?.()}>
          <X size={14} />
        </button>
      </div>
    </Show>
  );
}
