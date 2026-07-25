import { For, createSignal, onMount, untrack } from "solid-js";
import { render } from "solid-js/web";
import { describe, expect, it, vi } from "vitest";
import userEvent from "@testing-library/user-event";

import {
  ColorField,
  Dropdown,
  NativeSelect,
  RangeField,
  SegmentedControl,
  SelectField,
  Toast,
} from "./forms";

describe("NativeSelect", () => {
  it("selects a persisted value when its options arrive asynchronously", async () => {
    const host = document.createElement("div");
    document.body.append(host);

    function Harness() {
      const [value, setValue] = createSignal("47");
      const [options, setOptions] = createSignal<string[]>([]);

      onMount(() => {
        setValue("48");
        setOptions(["47", "48"]);
      });

      return (
        <NativeSelect label="Language and voice" value={value()}>
          <For each={options()}>{(option) => <option value={option}>{option}</option>}</For>
        </NativeSelect>
      );
    }

    const dispose = render(() => <Harness />, host);
    await Promise.resolve();

    expect(host.querySelector("select")).toHaveValue("48");

    dispose();
    host.remove();
  });
});

describe("custom form controls", () => {
  it("lets the Kobalte select choose an option with the keyboard", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const [value, setValue] = createSignal("system");
    const changed = vi.fn((next: string) => setValue(next));
    const dispose = render(
      () => (
        <SelectField
          label="Motion"
          value={value()}
          options={[
            { value: "system", label: "System" },
            { value: "on", label: "On" },
            { value: "off", label: "Off" },
          ]}
          onChange={changed}
        />
      ),
      host,
    );
    const trigger = host.querySelector<HTMLElement>('[data-component="select-trigger"]')!;
    trigger.focus();
    await userEvent.keyboard("{ArrowDown}{ArrowDown}{Enter}");
    expect(untrack(value)).toBe("on");
    expect(changed).toHaveBeenCalledTimes(1);
    dispose();
    host.remove();
  });

  it("keeps invalid color text visible for correction", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const [draft, setDraft] = createSignal("#112233");
    const dispose = render(
      () => (
        <ColorField
          label="Accent"
          value="#112233"
          error={/^#[\dA-Fa-f]{6}$/.test(draft()) ? undefined : "Invalid color"}
          onInput={setDraft}
        />
      ),
      host,
    );
    const input = host.querySelector<HTMLInputElement>('input[type="text"]')!;
    await userEvent.clear(input);
    await userEvent.type(input, "#12");
    expect(input).toHaveValue("#12");
    expect(host).toHaveTextContent("Invalid color");
    dispose();
    host.remove();
  });

  it("refreshes color text after an external reset", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const [value, setValue] = createSignal("#112233");
    const dispose = render(
      () => (
        <ColorField
          label="Accent"
          value={value()}
          onInput={(next) => {
            if (/^#[\dA-Fa-f]{6}$/.test(next)) setValue(next);
          }}
        />
      ),
      host,
    );
    const input = host.querySelector<HTMLInputElement>('input[type="text"]')!;
    await userEvent.clear(input);
    await userEvent.type(input, "#AABBCC");
    input.blur();
    setValue("#445566");
    await Promise.resolve();
    expect(input).toHaveValue("#445566");
    dispose();
    host.remove();
  });

  it("exposes segmented state and a dismissible toast", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const [marker, setMarker] = createSignal<"color" | "signs">("color");
    const [open, setOpen] = createSignal(true);
    const dispose = render(
      () => (
        <>
          <SegmentedControl
            label="Diff markers"
            value={marker()}
            options={[
              { value: "color", label: "Color" },
              { value: "signs", label: "Signs" },
            ]}
            onChange={setMarker}
          />
          <Toast open={open()} onClose={() => setOpen(false)}>
            Saved
          </Toast>
        </>
      ),
      host,
    );
    await userEvent.click(host.querySelectorAll("button")[1]!);
    expect(untrack(marker)).toBe("signs");
    await userEvent.click(host.querySelector('button[aria-label="Close"]')!);
    expect(host.querySelector('[data-component="toast"]')).toBeNull();
    dispose();
    host.remove();
  });

  it("opens a dropdown, skips disabled actions, and returns focus after selection", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const selected = vi.fn();
    const dispose = render(
      () => (
        <Dropdown
          label="Theme actions"
          actions={[
            { id: "reset", label: "Reset" },
            { id: "delete", label: "Delete", disabled: true },
          ]}
          onSelect={selected}
        >
          Actions
        </Dropdown>
      ),
      host,
    );
    const trigger = host.querySelector<HTMLElement>('[data-component="dropdown-trigger"]')!;
    await userEvent.click(trigger);
    const items = document.body.querySelectorAll<HTMLElement>('[data-component="dropdown-item"]');
    expect(items).toHaveLength(2);
    expect(items[1]).toHaveAttribute("data-disabled");
    await userEvent.click(items[0]!);
    await new Promise((resolve) => window.setTimeout(resolve));
    expect(selected).toHaveBeenCalledWith("reset");
    expect(trigger).toHaveFocus();
    dispose();
    host.remove();
  });

  it("updates and commits a range with the keyboard", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const [value, setValue] = createSignal(50);
    const committed = vi.fn();
    const dispose = render(
      () => (
        <RangeField
          label="Contrast"
          value={value()}
          min={0}
          max={100}
          onInput={setValue}
          onCommit={committed}
        />
      ),
      host,
    );
    const slider = host.querySelector<HTMLElement>('[role="slider"]')!;
    expect(slider.style.transform).toBe("translate(-50%, -50%)");
    slider.focus();
    slider.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    slider.dispatchEvent(new KeyboardEvent("keyup", { key: "ArrowRight", bubbles: true }));
    expect(untrack(value)).toBe(51);
    expect(committed).toHaveBeenLastCalledWith(51);
    dispose();
    host.remove();
  });
});
