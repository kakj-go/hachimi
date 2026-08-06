import type { ComputerAppDescriptor, ComputerAppPolicy } from "@hachimi/contracts";
import { I18nProvider } from "@hachimi/i18n";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ComputerUseSettings } from "./computer-use-settings";

const commandMocks = vi.hoisted(() => ({
  getComputerHostSettings: vi.fn(),
  listComputerAppCandidates: vi.fn(),
  listComputerAppPolicies: vi.fn(),
  listHostAccessRequests: vi.fn(),
  updateComputerHostSettings: vi.fn(),
  updateComputerAppPolicy: vi.fn(),
  resolveHostAccessRequest: vi.fn(),
}));

vi.mock("@hachimi/contracts", () => ({
  commands: commandMocks,
  commandFailure: (reason: unknown) => ({ code: "command_failed", message: String(reason) }),
}));

vi.mock("@hachimi/ui", () => ({
  Badge: (props: { children?: JSX.Element }) => <span>{props.children}</span>,
  Button: (props: { children?: JSX.Element; disabled?: boolean; onClick?: () => void }) => (
    <button disabled={props.disabled} onClick={() => props.onClick?.()}>
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
  SettingsRow: (props: {
    label: JSX.Element;
    description?: JSX.Element;
    children?: JSX.Element;
  }) => (
    <section>
      <span>{props.label}</span>
      <span>{props.description}</span>
      {props.children}
    </section>
  ),
  SettingsSection: (props: { title: string; children?: JSX.Element }) => (
    <section aria-label={props.title}>{props.children}</section>
  ),
  StatusBanner: (props: { children?: JSX.Element }) => <div role="alert">{props.children}</div>,
  Switch: (props: {
    label: string;
    testId?: string;
    checked: boolean;
    disabled?: boolean;
    onChange?: (checked: boolean) => void;
  }) => (
    <button
      role="switch"
      data-testid={props.testId}
      aria-label={props.label}
      aria-checked={props.checked}
      disabled={props.disabled}
      onClick={() => props.onChange?.(!props.checked)}
    />
  ),
}));

const app: ComputerAppDescriptor = {
  appId: "win32:test",
  displayName: "Test Editor",
  executableName: "editor.exe",
  executablePath: "C:\\Tools\\editor.exe",
  publisher: "Test Publisher",
  publisherVerified: true,
  packageFamilyName: null,
  appUserModelId: null,
  fileIdentity: "file-1",
  identityHash: "app-hash-1",
};

function mount() {
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(
    () => (
      <I18nProvider initialLocale="zh-CN">
        <ComputerUseSettings />
      </I18nProvider>
    ),
    host,
  );
  return { host, dispose };
}

beforeEach(() => {
  const runtimeHealth = {
    osSupported: true,
    graphicsCaptureAvailable: true,
    inputDesktopAvailable: true,
    processElevated: false,
    errorCode: null,
  };
  commandMocks.getComputerHostSettings.mockResolvedValue({
    automationEnabled: false,
    runtimeHealth,
  });
  commandMocks.listComputerAppCandidates.mockResolvedValue([]);
  commandMocks.listComputerAppPolicies.mockResolvedValue([]);
  commandMocks.listHostAccessRequests.mockResolvedValue([]);
  commandMocks.updateComputerHostSettings.mockImplementation(
    async ({ automationEnabled }: { automationEnabled: boolean }) => ({
      automationEnabled,
      runtimeHealth,
    }),
  );
  commandMocks.updateComputerAppPolicy.mockResolvedValue({
    app,
    decision: "allow",
    revision: 1,
    updatedAtMs: 1,
  } satisfies ComputerAppPolicy);
});

afterEach(() => {
  document.body.replaceChildren();
  vi.clearAllMocks();
});

describe("ComputerUseSettings", () => {
  it("enables the automation switch before slow application discovery finishes", async () => {
    let finishDiscovery!: (value: never[]) => void;
    commandMocks.listComputerAppCandidates.mockReturnValue(
      new Promise<never[]>((resolve) => {
        finishDiscovery = resolve;
      }),
    );
    const mounted = mount();
    const toggle = () =>
      mounted.host.querySelector<HTMLButtonElement>('[data-testid="computer-automation-toggle"]')!;

    await vi.waitFor(() => expect(toggle().disabled).toBe(false));
    toggle().click();
    await vi.waitFor(() =>
      expect(commandMocks.updateComputerHostSettings).toHaveBeenCalledWith({
        automationEnabled: true,
      }),
    );

    finishDiscovery([]);
    mounted.dispose();
  });

  it("keeps the switch usable when application discovery fails", async () => {
    commandMocks.listComputerAppCandidates.mockRejectedValue(new Error("scan failed"));
    const mounted = mount();

    await vi.waitFor(() => expect(mounted.host.textContent).toContain("scan failed"));
    expect(
      mounted.host.querySelector<HTMLButtonElement>('[data-testid="computer-automation-toggle"]')
        ?.disabled,
    ).toBe(false);
    mounted.dispose();
  });

  it("keeps automation independent when an application policy update fails", async () => {
    commandMocks.listComputerAppCandidates.mockResolvedValue([{ app, iconPngBase64: null }]);
    commandMocks.updateComputerAppPolicy.mockRejectedValue(new Error("policy conflict"));
    const mounted = mount();

    await vi.waitFor(() => expect(mounted.host.textContent).toContain("Test Editor"));
    const policy = mounted.host.querySelector<HTMLSelectElement>('[aria-label="访问策略"]')!;
    policy.value = "allow";
    policy.dispatchEvent(new Event("change", { bubbles: true }));

    await vi.waitFor(() => expect(mounted.host.textContent).toContain("policy conflict"));
    expect(
      mounted.host.querySelector<HTMLButtonElement>('[data-testid="computer-automation-toggle"]')
        ?.disabled,
    ).toBe(false);
    mounted.dispose();
  });
});
