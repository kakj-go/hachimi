import type { SkillPreviewResource } from "@hachimi/contracts";
import { I18nProvider } from "@hachimi/i18n";
import { render } from "solid-js/web";
import { For, Show, type JSX } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import { TextEditor } from "./text-editor";

vi.mock("@hachimi/ui", () => ({
  Button: (props: {
    children?: JSX.Element;
    disabled?: boolean;
    onClick?: () => void;
    "data-testid"?: string;
  }) => (
    <button
      type="button"
      data-testid={props["data-testid"]}
      disabled={props.disabled}
      onClick={() => props.onClick?.()}
    >
      {props.children}
    </button>
  ),
  StatusBanner: (props: { children?: JSX.Element }) => <div role="status">{props.children}</div>,
  Dialog: (props: { open: boolean; children?: JSX.Element; title: string }) => (
    <Show when={props.open}>
      <div role="dialog" aria-label={props.title}>
        {props.children}
      </div>
    </Show>
  ),
  TextField: (props: {
    label: string;
    value?: string;
    onInput?: JSX.EventHandler<HTMLInputElement, InputEvent>;
  }) => (
    <label>
      {props.label}
      <input value={props.value ?? ""} onInput={(event) => props.onInput?.(event)} />
    </label>
  ),
  SelectField: (props: {
    label: string;
    value: string;
    options: readonly { value: string; label: string }[];
    onChange?: (value: string) => void;
  }) => (
    <label>
      {props.label}
      <select value={props.value} onChange={(event) => props.onChange?.(event.currentTarget.value)}>
        <For each={props.options}>
          {(option) => <option value={option.value}>{option.label}</option>}
        </For>
      </select>
    </label>
  ),
}));

function resource(destination: string): SkillPreviewResource {
  return {
    skillId: "skill-1",
    sourcePath: "SKILL.md",
    relativePath: destination,
    editorKind: "text",
    text: "resolved only through SkillHost",
    mediaType: null,
    dataBase64: null,
    sizeBytes: 31,
    revision: "sha256:resource",
  };
}

async function settle() {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

function mount(overrides: Partial<Parameters<typeof TextEditor>[0]> = {}) {
  const host = document.createElement("div");
  document.body.append(host);
  const onSave = vi.fn();
  const onInput = vi.fn();
  const onReload = vi.fn();
  const onKeepLocal = vi.fn();
  const resolveReference = vi.fn(async (destination: string) => resource(destination));
  const dispose = render(
    () => (
      <I18nProvider initialLocale="en-US">
        <TextEditor
          path="SKILL.md"
          kind="markdown"
          value={
            "---\nname: safe-skill\ndescription: Safe editing\n---\n\n# Safe\n\n<script>never execute</script>\n\n[Reference](reference.md)"
          }
          dirty
          saving={false}
          conflict={false}
          diagnostics={[]}
          onInput={onInput}
          onSave={onSave}
          onReload={onReload}
          onKeepLocal={onKeepLocal}
          resolveReference={resolveReference}
          referenceFiles={["reference.md", "scripts/helper.py"]}
          {...overrides}
        />
      </I18nProvider>
    ),
    host,
  );
  return { host, dispose, onSave, onInput, onReload, onKeepLocal, resolveReference };
}

afterEach(() => {
  document.body.replaceChildren();
  vi.clearAllMocks();
});

describe("Skill text editor", () => {
  it("renders one safe WYSIWYG canvas and resolves references through the host", async () => {
    const mounted = mount();
    await settle();

    const editor = mounted.host.querySelector<HTMLElement>('[data-testid="skill-markdown-editor"]');
    expect(editor).not.toBeNull();
    expect(mounted.host.querySelector('[role="tab"]')).toBeNull();
    expect(mounted.host.querySelector('[data-testid="skill-editor-input"]')).toBeNull();
    expect(editor?.textContent).toContain("Safe");
    expect(editor?.textContent).not.toContain("name: safe-skill");
    expect(mounted.resolveReference).toHaveBeenCalledWith("reference.md");
    expect(mounted.host.textContent).toContain("resolved only through SkillHost");
    expect(mounted.host.querySelector("script")).toBeNull();

    if (editor) {
      editor.innerHTML = "<h2>Visible title</h2><p>Rich body</p>";
      editor.dispatchEvent(new InputEvent("input", { bubbles: true }));
    }
    expect(mounted.onInput).toHaveBeenCalledWith(
      expect.stringContaining("## Visible title\n\nRich body"),
    );

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "s", ctrlKey: true }));
    expect(mounted.onSave).toHaveBeenCalledTimes(1);
    mounted.dispose();
  });

  it("shows JavaScript and Python as read-only text", () => {
    const mounted = mount({ path: "script.py", kind: "text", value: "print('safe')" });
    expect(mounted.host.querySelector('[data-testid="skill-text-viewer"]')?.textContent).toContain(
      "print('safe')",
    );
    expect(mounted.host.querySelector('[data-testid="skill-markdown-editor"]')).toBeNull();
    expect(mounted.host.querySelector('[data-testid="skill-save"]')).toBeNull();
    mounted.dispose();
  });

  it("shows Skill metadata only for the root SKILL.md", () => {
    const mounted = mount({
      path: "references/notes.md",
      value: "---\nname: resource\ndescription: ordinary content\n---\n\n# Notes",
      referenceFiles: ["api.md"],
    });
    expect(mounted.host.textContent).not.toContain("Skill name");
    expect(mounted.host.textContent).not.toContain("Purpose and trigger conditions");
    mounted.dispose();
  });

  it("writes the user-facing alias into Skill frontmatter", () => {
    const mounted = mount();
    const alias = [...mounted.host.querySelectorAll("label")].find((label) =>
      label.textContent?.startsWith("Alias"),
    );
    const input = alias?.querySelector("input");
    expect(input).not.toBeNull();
    if (input) {
      input.value = "Safe Writer";
      input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    }
    expect(mounted.onInput).toHaveBeenLastCalledWith(
      expect.stringContaining("display_name: Safe Writer"),
    );
    mounted.dispose();
  });

  it("offers existing Skill files in the reference picker", () => {
    const mounted = mount();
    [...mounted.host.querySelectorAll("button")]
      .find((button) => button.textContent === "Reference")
      ?.click();
    const dialog = mounted.host.querySelector<HTMLElement>('[role="dialog"]');
    const options = [...(dialog?.querySelectorAll("option") ?? [])].map(
      (option) => option.textContent,
    );
    expect(options).toEqual(["reference.md", "scripts/helper.py"]);
    mounted.dispose();
  });

  it("keeps conflict resolution choices explicit", () => {
    const mounted = mount({ conflict: true });
    const buttons = [...mounted.host.querySelectorAll("button")];
    buttons.find((button) => button.textContent?.includes("Reload"))?.click();
    buttons.find((button) => button.textContent?.includes("Keep local"))?.click();
    expect(mounted.onReload).toHaveBeenCalledTimes(1);
    expect(mounted.onKeepLocal).toHaveBeenCalledTimes(1);
    mounted.dispose();
  });
});
