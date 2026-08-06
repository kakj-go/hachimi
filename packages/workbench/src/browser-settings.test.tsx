import type { EmbeddedBrowserSettings, FeatureFlags } from "@hachimi/contracts";
import { I18nProvider } from "@hachimi/i18n";
import { Show, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { BrowserSettingsSection } from "./browser-settings";

const commandMocks = vi.hoisted(() => ({
  getEmbeddedBrowserSettings: vi.fn(),
  getBrowserHistory: vi.fn(),
  chooseBrowserDownloadDirectory: vi.fn(),
  updateEmbeddedBrowserSettings: vi.fn(),
  clearEmbeddedBrowserData: vi.fn(),
}));

vi.mock("@hachimi/contracts", () => ({
  commands: commandMocks,
  commandFailure: (reason: unknown) => ({ code: "command_failed", message: String(reason) }),
}));

vi.mock("@hachimi/ui", () => ({
  Badge: (props: { children?: JSX.Element }) => <span>{props.children}</span>,
  Dialog: (props: { open: boolean; title: string; children?: JSX.Element }) => (
    <Show when={props.open}>
      <section role="dialog" aria-label={props.title}>
        {props.children}
      </section>
    </Show>
  ),
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
  Switch: (props: {
    label: string;
    checked: boolean;
    disabled?: boolean;
    onChange?: (checked: boolean) => void;
  }) => (
    <button
      role="switch"
      aria-label={props.label}
      aria-checked={props.checked}
      disabled={props.disabled}
      onClick={() => props.onChange?.(!props.checked)}
    />
  ),
}));

const initialSettings: EmbeddedBrowserSettings = {
  downloadDirectory: "D:\\Downloads",
  askWhereToSaveDownloads: false,
  fullCdpAccess: false,
  fullCdpAccessAllowed: false,
  revision: 7,
};

const featureFlags = { browserControl: true } as FeatureFlags;

function nextSettings(
  patch: Partial<EmbeddedBrowserSettings>,
  revision = initialSettings.revision + 1,
): EmbeddedBrowserSettings {
  return { ...initialSettings, ...patch, revision };
}

function mount() {
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(
    () => (
      <I18nProvider initialLocale="zh-CN">
        <BrowserSettingsSection featureFlags={featureFlags} />
      </I18nProvider>
    ),
    host,
  );
  return { host, dispose };
}

beforeEach(() => {
  commandMocks.getEmbeddedBrowserSettings.mockResolvedValue(initialSettings);
  commandMocks.getBrowserHistory.mockResolvedValue([]);
  commandMocks.chooseBrowserDownloadDirectory.mockResolvedValue("E:\\BrowserDownloads");
  commandMocks.updateEmbeddedBrowserSettings.mockImplementation(async (request) =>
    nextSettings(
      {
        downloadDirectory: request.downloadDirectory,
        askWhereToSaveDownloads: request.askWhereToSaveDownloads,
        fullCdpAccess: request.fullCdpAccess,
      },
      request.expectedRevision + 1,
    ),
  );
  commandMocks.clearEmbeddedBrowserData.mockResolvedValue(undefined);
});

afterEach(() => {
  document.body.replaceChildren();
  vi.clearAllMocks();
});

describe("BrowserSettingsSection", () => {
  it("updates and restores the download directory with revision fencing", async () => {
    const mounted = mount();
    await vi.waitFor(() => expect(mounted.host.textContent).toContain("D:\\Downloads"));
    [...mounted.host.querySelectorAll("button")]
      .find((button) => button.textContent === "选择文件夹")
      ?.click();
    await vi.waitFor(() =>
      expect(commandMocks.updateEmbeddedBrowserSettings).toHaveBeenCalledWith({
        downloadDirectory: "E:\\BrowserDownloads",
        askWhereToSaveDownloads: false,
        fullCdpAccess: false,
        expectedRevision: 7,
      }),
    );
    expect(mounted.host.textContent).toContain("E:\\BrowserDownloads");

    [...mounted.host.querySelectorAll("button")]
      .find((button) => button.textContent === "恢复默认")
      ?.click();
    await vi.waitFor(() =>
      expect(commandMocks.updateEmbeddedBrowserSettings).toHaveBeenLastCalledWith({
        downloadDirectory: null,
        askWhereToSaveDownloads: false,
        fullCdpAccess: false,
        expectedRevision: 8,
      }),
    );
    mounted.dispose();
  });

  it("updates ask-before-save and hides full CDP without Developer mode", async () => {
    const mounted = mount();
    await vi.waitFor(() =>
      expect(
        mounted.host.querySelector<HTMLButtonElement>('[aria-label="每次询问"]')?.disabled,
      ).toBe(false),
    );
    const ask = mounted.host.querySelector<HTMLButtonElement>('[aria-label="每次询问"]')!;
    expect(
      mounted.host.querySelector<HTMLButtonElement>('[aria-label="启用完整 CDP 访问"]'),
    ).toBeNull();
    ask.click();
    await vi.waitFor(() =>
      expect(commandMocks.updateEmbeddedBrowserSettings).toHaveBeenCalledWith({
        downloadDirectory: "D:\\Downloads",
        askWhereToSaveDownloads: true,
        fullCdpAccess: false,
        expectedRevision: 7,
      }),
    );
    mounted.dispose();
  });

  it("clears only the selected browser data kinds", async () => {
    const mounted = mount();
    await vi.waitFor(() =>
      expect(
        mounted.host.querySelector<HTMLButtonElement>('[data-testid="embedded-browser-clear-data"]')
          ?.disabled,
      ).toBe(false),
    );
    mounted.host.querySelector<HTMLButtonElement>('[aria-label="Cookie"]')?.click();
    mounted.host
      .querySelector<HTMLButtonElement>('[data-testid="embedded-browser-clear-data"]')
      ?.click();
    mounted.host.querySelector<HTMLButtonElement>('[aria-label="Cookie 与站点数据"]')?.click();
    mounted.host.querySelector<HTMLButtonElement>('[aria-label="缓存文件"]')?.click();
    mounted.host
      .querySelector<HTMLButtonElement>('[data-testid="embedded-browser-clear-confirm"]')
      ?.click();
    await vi.waitFor(() =>
      expect(commandMocks.clearEmbeddedBrowserData).toHaveBeenCalledWith({ data: ["history"] }),
    );
    mounted.dispose();
  });
});
