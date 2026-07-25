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

export interface TextFieldProps {
  label: string;
  value?: string;
  placeholder?: string;
  description?: string;
  disabled?: boolean;
  type?: "text" | "password";
  onInput?: JSX.EventHandler<HTMLInputElement, InputEvent>;
}

export function TextField(props: TextFieldProps) {
  return (
    <KTextField.Root
      data-component="form-field"
      data-variant="default"
      data-size="normal"
      data-state={props.disabled ? "disabled" : "idle"}
      disabled={props.disabled ?? false}
      value={props.value ?? ""}
    >
      <KTextField.Label data-component="form-label">{props.label}</KTextField.Label>
      <KTextField.Input
        data-component="text-field-input"
        data-variant="default"
        data-size="normal"
        data-state={props.disabled ? "disabled" : "idle"}
        type={props.type ?? "text"}
        placeholder={props.placeholder ?? ""}
        onInput={props.onInput ?? (() => undefined)}
      />
      <Show when={props.description}>
        <KTextField.Description data-component="field-description">
          {props.description}
        </KTextField.Description>
      </Show>
    </KTextField.Root>
  );
}

export interface FormFieldProps {
  label: string;
  description?: string;
  children: JSX.Element;
}

export function FormField(props: FormFieldProps) {
  return (
    <div data-component="form-field">
      <span data-component="form-label">{props.label}</span>
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
  disabled?: boolean;
  onChange?: (checked: boolean) => void;
}

export function Switch(props: SwitchProps) {
  return (
    <KSwitch.Root
      checked={props.checked}
      disabled={props.disabled ?? false}
      onChange={props.onChange ?? (() => undefined)}
      data-component="switch-root"
      data-variant="default"
      data-size="normal"
      data-state={props.checked ? "checked" : "unchecked"}
    >
      <KSwitch.Input />
      <KSwitch.Control data-component="switch">
        <KSwitch.Thumb data-component="switch-thumb" />
      </KSwitch.Control>
      <KSwitch.Label class="sr-only">{props.label}</KSwitch.Label>
    </KSwitch.Root>
  );
}

export interface NativeSelectProps extends JSX.SelectHTMLAttributes<HTMLSelectElement> {
  label: string;
}

export function NativeSelect(props: NativeSelectProps) {
  const [local, rest] = splitProps(props, ["label", "children", "value"]);
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
    <label data-component="form-field">
      <span data-component="form-label">{local.label}</span>
      <select
        ref={selectElement}
        data-component="text-field-input"
        data-variant="default"
        data-size="normal"
        data-state={props.disabled ? "disabled" : "idle"}
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
  value: string;
  options: readonly SelectOption[];
  description?: string;
  placeholder?: string;
  disabled?: boolean;
  onChange?: (value: string) => void;
}

export function SelectField(props: SelectFieldProps) {
  const options = createMemo<ResolvedSelectOption[]>(() =>
    props.options.map((option) => ({ ...option, key: selectOptionKey(option.value) })),
  );
  const selected = createMemo(
    () => options().find((option) => option.value === props.value) ?? null,
  );
  return (
    <KSelect.Root<ResolvedSelectOption>
      options={options()}
      optionValue="key"
      optionTextValue="label"
      optionDisabled="disabled"
      multiple={false}
      value={selected()}
      placeholder={props.placeholder ?? ""}
      disabled={props.disabled ?? false}
      onChange={(option) => {
        if (option && option.value !== props.value) props.onChange?.(option.value);
      }}
      itemComponent={(itemProps) => (
        <KSelect.Item item={itemProps.item} data-component="select-item">
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
      <KSelect.HiddenSelect />
      <KSelect.Label data-component="form-label">{props.label}</KSelect.Label>
      <KSelect.Trigger data-component="select-trigger">
        <KSelect.Value<ResolvedSelectOption>>
          {(state) => (
            <span data-component="select-value">
              <SelectPreview option={state.selectedOption()} />
              <span>{state.selectedOption().label}</span>
            </span>
          )}
        </KSelect.Value>
        <KSelect.Icon data-component="select-icon">
          <ChevronDown size={15} />
        </KSelect.Icon>
      </KSelect.Trigger>
      <Show when={props.description}>
        <KSelect.Description data-component="field-description">
          {props.description}
        </KSelect.Description>
      </Show>
      <KSelect.Portal>
        <KSelect.Content data-component="select-content">
          <KSelect.Listbox data-component="select-listbox" />
        </KSelect.Content>
      </KSelect.Portal>
    </KSelect.Root>
  );
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
  error?: string | undefined;
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
    <div data-component="form-field" data-invalid={Boolean(props.error)}>
      <label data-component="form-label" for={id}>
        {props.label}
      </label>
      <div data-component="color-field">
        <input
          id={id}
          type="color"
          value={/^#[\dA-Fa-f]{6}$/.test(draft()) ? draft() : "#000000"}
          disabled={props.disabled}
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
          disabled={props.disabled}
          aria-invalid={Boolean(props.error)}
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
      value={[props.value]}
      minValue={props.min}
      maxValue={props.max}
      step={props.step ?? 1}
      disabled={props.disabled ?? false}
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
  onChange?: (value: T) => void;
}) {
  return (
    <div data-component="segmented-control" role="group" aria-label={props.label}>
      <For each={props.options}>
        {(option) => (
          <button
            type="button"
            aria-pressed={props.value === option.value}
            disabled={props.disabled}
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
  danger?: boolean;
  disabled?: boolean;
  separatorBefore?: boolean;
}

export function Dropdown(props: {
  label: string;
  actions: readonly DropdownAction[];
  onSelect: (id: string) => void;
  children: JSX.Element;
}) {
  let trigger!: HTMLButtonElement;

  return (
    <KDropdownMenu.Root>
      <KDropdownMenu.Trigger
        ref={trigger}
        data-component="dropdown-trigger"
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
                  data-danger={action.danger ?? false}
                  disabled={action.disabled ?? false}
                  onSelect={() => {
                    props.onSelect(action.id);
                    window.setTimeout(() => trigger.focus());
                  }}
                >
                  {action.label}
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
}) {
  return (
    <Show when={props.open}>
      <div data-component="toast" data-tone={props.tone ?? "neutral"} role="status">
        <span>{props.children}</span>
        <button type="button" aria-label="Close" onClick={() => props.onClose?.()}>
          <X size={14} />
        </button>
      </div>
    </Show>
  );
}
