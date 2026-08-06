import type { BrowserSitePolicy } from "@hachimi/contracts";
import { I18nProvider } from "@hachimi/i18n";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { PrivateBrowserSitePolicySettings } from "./browser-site-policy-settings";

const commandMocks = vi.hoisted(() => ({
  listBrowserSitePolicies: vi.fn(),
  updatePrivateBrowserSitePolicy: vi.fn(),
  removeBrowserSitePolicy: vi.fn(),
}));

vi.mock("@hachimi/contracts", () => ({
  commands: commandMocks,
  commandFailure: (reason: unknown) => ({ code: "command_failed", message: String(reason) }),
}));

vi.mock("@hachimi/ui", () => ({
  Badge: (props: { children?: JSX.Element }) => <span>{props.children}</span>,
  Button: (props: {
    children?: JSX.Element;
    disabled?: boolean;
    "data-testid"?: string;
    onClick?: () => void;
  }) => (
    <button
      disabled={props.disabled}
      data-testid={props["data-testid"]}
      onClick={() => props.onClick?.()}
    >
      {props.children}
    </button>
  ),
  Select: (props: {
    label: string;
    value: string;
    options: { value: string; label: string }[];
    disabled?: boolean;
    onChange?: (value: string) => void;
  }) => (
    <select
      aria-label={props.label}
      value={props.value}
      disabled={props.disabled}
      onChange={(event) => props.onChange?.(event.currentTarget.value)}
    >
      <For each={props.options}>
        {(option) => <option value={option.value}>{option.label}</option>}
      </For>
    </select>
  ),
  SettingsCard: (props: { children?: JSX.Element }) => <div>{props.children}</div>,
  SettingsRow: (props: { label: string; description?: JSX.Element; children?: JSX.Element }) => (
    <section>
      <strong>{props.label}</strong>
      <span>{props.description}</span>
      {props.children}
    </section>
  ),
  SettingsSection: (props: { title: string; children?: JSX.Element }) => (
    <section aria-label={props.title}>{props.children}</section>
  ),
  StatusBanner: (props: { children?: JSX.Element }) => <div role="status">{props.children}</div>,
  TextField: (props: {
    label: string;
    value: string;
    placeholder?: string;
    onInput?: (event: InputEvent & { currentTarget: HTMLInputElement }) => void;
  }) => (
    <input
      aria-label={props.label}
      value={props.value}
      placeholder={props.placeholder}
      onInput={(event) => props.onInput?.(event)}
    />
  ),
}));

const privatePolicy: BrowserSitePolicy = {
  origin: "http://127.0.0.1:3000",
  decision: "allow",
  capabilities: ["observe", "act"],
  privateNetwork: true,
  revision: 3,
  updatedAtMs: 10,
};

const publicPolicy: BrowserSitePolicy = {
  ...privatePolicy,
  origin: "https://example.com",
  privateNetwork: false,
};

describe("PrivateBrowserSitePolicySettings", () => {
  beforeEach(() => {
    commandMocks.listBrowserSitePolicies.mockResolvedValue([publicPolicy, privatePolicy]);
    commandMocks.updatePrivateBrowserSitePolicy.mockResolvedValue(privatePolicy);
    commandMocks.removeBrowserSitePolicy.mockResolvedValue(true);
  });

  afterEach(() => {
    document.body.innerHTML = "";
    vi.clearAllMocks();
  });

  it("shows only private policies and saves through the Developer-only command", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="zh-CN">
          <PrivateBrowserSitePolicySettings />
        </I18nProvider>
      ),
      host,
    );

    await vi.waitFor(() => expect(host.textContent).toContain(privatePolicy.origin));
    expect(host.textContent).not.toContain(publicPolicy.origin);

    const origin = host.querySelector<HTMLInputElement>('input[aria-label="Origin"]')!;
    origin.value = "http://localhost:4173";
    origin.dispatchEvent(new InputEvent("input", { bubbles: true }));
    host.querySelector<HTMLButtonElement>('[data-testid="private-browser-policy-save"]')!.click();

    await vi.waitFor(() =>
      expect(commandMocks.updatePrivateBrowserSitePolicy).toHaveBeenCalledWith({
        origin: "http://localhost:4173",
        decision: "allow",
        capabilities: ["observe", "act"],
        expectedRevision: null,
      }),
    );
    dispose();
  });
});
