import { I18nProvider } from "@hachimi/i18n";
import type { JSX } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import { createPermissionPolicy } from "./permission-policy-editor";
import { PetPermissionSettings } from "./pet-permission-settings";

const commandMocks = vi.hoisted(() => ({
  getSessionPermissionConfig: vi.fn(),
  updateSessionPermissionConfig: vi.fn(),
  listSkills: vi.fn(),
  listComputerAppCandidates: vi.fn(),
  listComputerAppPolicies: vi.fn(),
}));

vi.mock("@hachimi/contracts", () => ({
  commands: commandMocks,
  commandFailure: (reason: unknown) => ({ code: "command_failed", message: String(reason) }),
}));

vi.mock("@hachimi/ui", () => {
  const Icon = () => <span aria-hidden="true" />;
  const Button = (props: {
    children?: JSX.Element;
    disabled?: boolean;
    onClick?: () => void;
    "aria-label"?: string;
    "data-testid"?: string;
  }) => (
    <button
      type="button"
      aria-label={props["aria-label"]}
      data-testid={props["data-testid"]}
      disabled={props.disabled}
      onClick={() => props.onClick?.()}
    >
      {props.children}
    </button>
  );
  return {
    Button,
    Checkbox: (props: {
      label: string;
      checked?: boolean;
      disabled?: boolean;
      onChange?: JSX.EventHandler<HTMLInputElement, Event>;
    }) => (
      <label>
        <input
          type="checkbox"
          aria-label={props.label}
          checked={props.checked}
          disabled={props.disabled}
          onChange={(event) => props.onChange?.(event)}
        />
        <span>{props.label}</span>
      </label>
    ),
    Dialog: () => null,
    History: Icon,
    IconButton: Button,
    PermissionPolicyEditor: () => <div data-testid="permission-editor" />,
    Puzzle: Icon,
    SearchField: (props: {
      label: string;
      value?: string;
      disabled?: boolean;
      onInput?: JSX.EventHandler<HTMLInputElement, InputEvent>;
    }) => (
      <input
        type="search"
        aria-label={props.label}
        value={props.value ?? ""}
        disabled={props.disabled}
        onInput={(event) => props.onInput?.(event)}
      />
    ),
    SettingsSection: (props: { title: string; children: JSX.Element }) => (
      <section aria-label={props.title}>{props.children}</section>
    ),
    ShieldCheck: Icon,
    Sparkles: Icon,
    StatusBanner: (props: { children: JSX.Element }) => <div>{props.children}</div>,
    X: Icon,
  };
});

afterEach(() => {
  document.body.replaceChildren();
  vi.clearAllMocks();
});

async function settle() {
  for (let index = 0; index < 8; index += 1) await Promise.resolve();
}

describe("PetPermissionSettings", () => {
  it("loads and saves the Pet Skill allowlist with friendly names", async () => {
    const policy = createPermissionPolicy();
    const skill = {
      id: "skill-word",
      scope: "built_in",
      namespace: null,
      name: "office documents",
      qualifiedName: "office-documents",
      description: "Edit Word documents",
      dependencies: [],
      editable: false,
      enabled: true,
      contentHash: "word-content",
      treeRevision: "word-tree",
      diagnostics: [],
      updatedAtMs: 1,
    };
    commandMocks.getSessionPermissionConfig.mockResolvedValue({
      policy,
      skillIds: [],
      extraAuthorizations: [],
    });
    commandMocks.listSkills.mockResolvedValue([skill]);
    commandMocks.updateSessionPermissionConfig.mockImplementation(
      async (request) => request.config,
    );

    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="zh-CN">
          <PetPermissionSettings />
        </I18nProvider>
      ),
      root,
    );

    await settle();
    const skillsNavigation = [...root.querySelectorAll<HTMLButtonElement>("button")].find(
      (button) => button.textContent?.includes("技能权限"),
    );
    expect(skillsNavigation).toBeTruthy();
    skillsNavigation!.click();
    const skillCheckbox = root.querySelector<HTMLInputElement>('input[aria-label="Word"]');
    expect(skillCheckbox).toBeTruthy();
    expect(root.querySelector(".skill-permission-copy code")?.textContent).toBe("office-documents");
    skillCheckbox!.click();
    await settle();
    root.querySelector<HTMLButtonElement>('[data-testid="pet-permission-save"]')!.click();

    await vi.waitFor(() => expect(commandMocks.updateSessionPermissionConfig).toHaveBeenCalled());
    expect(commandMocks.updateSessionPermissionConfig).toHaveBeenCalledWith({
      sessionId: null,
      entryProfile: "pet_conversation",
      expectedRevision: 0,
      config: {
        extraAuthorizations: [],
        policy,
        skillIds: ["skill-word"],
      },
    });
    dispose();
  });
});
