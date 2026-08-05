import {
  commandFailure,
  commands,
  type BrowserAutomationPreference,
  type BrowserHostSettings,
  type BrowserPairing,
  type FeatureFlags,
  type SandboxBootstrapState,
  type SystemBrowserKind,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import {
  Badge,
  Button,
  PageHeading,
  RefreshCw,
  SettingsCard,
  SettingsRow,
  SettingsSection,
  ShieldCheck,
  Select,
  StatusBanner,
  Switch as Toggle,
} from "@hachimi/ui";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { For, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";

import { BrowserSettingsSection } from "./browser-settings";
import {
  BrowserSitePolicySettings,
  PrivateBrowserSitePolicySettings,
} from "./browser-site-policy-settings";
import { ComputerUseSettings } from "./computer-use-settings";
import { RuntimeHealthBanner } from "./runtime-health";

type BusyAction = "load" | "sandbox" | "pairing";

export type HostDomainSettingsSection = "browser" | "computer-use" | "runtime-security";

export function HostDomainSettingsPage(props: {
  featureFlags: FeatureFlags;
  section: HostDomainSettingsSection;
  developerMode?: boolean;
}) {
  const i18n = useI18n();
  const zh = () => i18n.locale() === "zh-CN";
  const [sandbox, setSandbox] = createSignal<SandboxBootstrapState>();
  const [browserSettings, setBrowserSettings] = createSignal<BrowserHostSettings>();
  const [busy, setBusy] = createSignal<BusyAction>();
  const [failure, setFailure] = createSignal<string>();
  const [notice, setNotice] = createSignal<string>();
  const [computerRefreshRevision, setComputerRefreshRevision] = createSignal(0);

  const pageCopy = createMemo(() => {
    const copy = {
      browser: [
        "浏览器",
        "Browser",
        "管理 Agent 自动化、双 Browser 路由、网站访问与内置 CEF 数据。",
        "Manage Agent automation, dual-Browser routing, website access, and embedded CEF data.",
      ],
      "computer-use": [
        "Computer Use",
        "Computer Use",
        "管理 Agent 对本地 GUI 应用的稳定身份策略。",
        "Manage stable application identity policies for local GUI automation.",
      ],
      "runtime-security": [
        "Runtime & Security",
        "Runtime & Security",
        "查看普通用户沙箱、安全边界与修复状态。",
        "Inspect per-user sandbox enforcement, security boundaries, and repair state.",
      ],
    } as const;
    const value = copy[props.section];
    return {
      title: zh() ? value[0] : value[1],
      description: zh() ? value[2] : value[3],
    };
  });

  const sandboxReady = createMemo(() => {
    const report = sandbox()?.snapshot.report;
    return (
      sandbox()?.phase === "ready" &&
      report?.readiness === "ready" &&
      report.osEnforced &&
      report.filesystemEnforced &&
      report.processEnforced &&
      report.networkEnforced
    );
  });

  async function run(action: BusyAction, operation: () => Promise<void>) {
    setBusy(action);
    setFailure(undefined);
    setNotice(undefined);
    try {
      await operation();
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(undefined);
    }
  }

  async function load() {
    const section = props.section;
    const browserControlEnabled = props.featureFlags.browserControl;
    await run("load", async () => {
      const tasks: Promise<void>[] = [];
      if (section === "runtime-security") {
        tasks.push(
          commands.getSandboxBootstrapState().then((value) => {
            setSandbox(value);
          }),
        );
      }
      if (section === "browser" && browserControlEnabled) {
        tasks.push(
          commands.getBrowserHostSettings().then((settings) => {
            setBrowserSettings(settings);
          }),
        );
      }
      await Promise.all(tasks);
    });
  }

  async function repairSandbox() {
    await run("sandbox", async () => {
      const bootstrap = await commands.getBootstrapState();
      await commands.repairSandbox({
        context: {
          requestId: crypto.randomUUID(),
          clientId: "window:workbench",
          protocolVersion: bootstrap.protocolVersion,
          idempotencyKey: crypto.randomUUID(),
          expectedRunId: null,
          expectedGeneration: null,
        },
      });
      setSandbox(await commands.getSandboxBootstrapState());
      setNotice(zh() ? "沙箱修复和重新证明已完成。" : "Sandbox repair and attestation completed.");
    });
  }

  async function installExtension(browser: SystemBrowserKind) {
    await run("pairing", async () => {
      await commands.installBrowserExtension(browser);
      setNotice(
        zh()
          ? "已打开扩展商店。安装后，Hachimi 会自动检测并请求一次授权。"
          : "The extension store is open. After installation, Hachimi will detect the extension and request authorization once.",
      );
    });
  }

  async function approveExtension(pairing: BrowserPairing) {
    await run("pairing", async () => {
      setBrowserSettings(await commands.approveBrowserExtension(pairing.id));
      setNotice(zh() ? "浏览器扩展已授权。" : "Browser extension authorized.");
    });
  }

  async function updateBrowserSettings(update: {
    automationEnabled?: boolean;
    automationPreference?: BrowserAutomationPreference;
  }) {
    const current = browserSettings();
    if (!current) return;
    await run("pairing", async () => {
      const settings = await commands.updateBrowserHostSettings({
        automationEnabled: update.automationEnabled ?? current.automationEnabled,
        automationPreference: update.automationPreference ?? current.automationPreference,
      });
      setBrowserSettings(settings);
    });
  }

  let stopAuthorizationListener: UnlistenFn | undefined;
  onMount(() => {
    void load();
    if (props.section === "browser") {
      // eslint-disable-next-line solid/reactivity -- Tauri invokes this external event callback.
      void listen<BrowserPairing>("browser-extension-authorization-requested", () => {
        void load();
      }).then((stop) => {
        stopAuthorizationListener = stop;
      });
    }
  });
  onCleanup(() => stopAuthorizationListener?.());

  return (
    <div class="settings-page host-domain-settings" data-testid={`settings-${props.section}-page`}>
      <PageHeading
        class="settings-page-heading"
        title={pageCopy().title}
        description={pageCopy().description}
        actions={
          <Button
            disabled={Boolean(busy())}
            data-testid="host-domain-refresh"
            onClick={() => {
              if (props.section === "computer-use") {
                setComputerRefreshRevision((revision) => revision + 1);
              } else {
                void load();
              }
            }}
          >
            <RefreshCw size={14} />
            {zh() ? "刷新" : "Refresh"}
          </Button>
        }
      />

      <Show when={failure()}>
        {(message) => (
          <div data-testid="host-domain-failure">
            <StatusBanner tone="danger">{message()}</StatusBanner>
          </div>
        )}
      </Show>
      <Show when={notice()}>
        {(message) => (
          <div data-testid="host-domain-notice">
            <StatusBanner tone="success">{message()}</StatusBanner>
          </div>
        )}
      </Show>

      <Show when={props.section === "runtime-security"}>
        <>
          <SettingsSection title={zh() ? "普通用户沙箱" : "Per-user sandbox"}>
            <RuntimeHealthBanner component="internal_resources" zh={zh()} />
            <StatusBanner tone="neutral">
              {zh()
                ? "网站或应用访问授权不会跳过发送、删除、购买、上传或下载等副作用审批。"
                : "Website or application access never bypasses side-effect approval for sending, deleting, purchasing, uploading, or downloading."}
            </StatusBanner>
            <SettingsCard>
              <SettingsRow
                label={zh() ? "安装与证明状态" : "Bootstrap and attestation"}
                description={`${sandbox()?.phase ?? "not_started"} · ${sandbox()?.stableErrorCode ?? "no_error"}`}
              >
                <Badge tone={sandboxReady() ? "success" : "warning"}>
                  {sandboxReady()
                    ? zh()
                      ? "已就绪"
                      : "Ready"
                    : zh()
                      ? "需要处理"
                      : "Needs action"}
                </Badge>
              </SettingsRow>
              <SettingsRow
                label={zh() ? "四项强制能力" : "Four enforcement capabilities"}
                description={
                  zh()
                    ? "OS、文件系统、进程和网络必须全部有效。"
                    : "OS, filesystem, process, and network must all be enforced."
                }
              >
                <span class="host-domain-metrics" data-testid="host-domain-sandbox-capabilities">
                  {
                    [
                      sandbox()?.snapshot.report.osEnforced,
                      sandbox()?.snapshot.report.filesystemEnforced,
                      sandbox()?.snapshot.report.processEnforced,
                      sandbox()?.snapshot.report.networkEnforced,
                    ].filter(Boolean).length
                  }
                  /4
                </span>
              </SettingsRow>
              <SettingsRow
                label={zh() ? "无 UAC 修复" : "Repair without UAC"}
                description={
                  sandbox()?.runtimeRoot ??
                  (zh() ? "运行目录尚未创建" : "Runtime directory not created")
                }
              >
                <Button disabled={Boolean(busy())} onClick={() => void repairSandbox()}>
                  <ShieldCheck size={14} />
                  {busy() === "sandbox"
                    ? zh()
                      ? "正在修复…"
                      : "Repairing…"
                    : zh()
                      ? "安装/修复"
                      : "Install/repair"}
                </Button>
              </SettingsRow>
            </SettingsCard>
          </SettingsSection>
          <Show when={props.developerMode}>
            <PrivateBrowserSitePolicySettings />
          </Show>
        </>
      </Show>

      <Show when={props.section === "browser"}>
        <SettingsSection title={zh() ? "Agent 浏览器" : "Agent browser"}>
          <RuntimeHealthBanner component="cef" zh={zh()} />
          <RuntimeHealthBanner component="browser_extension" zh={zh()} />
          <SettingsCard>
            <SettingsRow
              label={zh() ? "Browser Agent Router" : "Browser Agent router"}
              description={
                zh()
                  ? "统一路由到内置 CEF 或已配对的外置 Chrome。"
                  : "Routes automation to embedded CEF or paired external Chrome."
              }
            >
              <Badge tone={props.featureFlags.browserControl ? "success" : "warning"}>
                {props.featureFlags.browserControl
                  ? zh()
                    ? "可用"
                    : "Available"
                  : zh()
                    ? "已关闭"
                    : "Off"}
              </Badge>
            </SettingsRow>
            <For each={browserSettings()?.detectedBrowsers ?? []}>
              {(browser) => (
                <SettingsRow
                  label={`${browser.kind === "chrome" ? "Chrome" : "Edge"}${browser.version ? ` ${browser.version}` : ""}`}
                  description={
                    browser.supported
                      ? zh()
                        ? "已自动检测，可按需复用系统浏览器登录态。"
                        : "Detected automatically and available for reusing system browser sessions."
                      : zh()
                        ? "当前版本不受支持，自动模式将使用内置浏览器。"
                        : "This version is unsupported; auto mode will use the embedded browser."
                  }
                >
                  <Button
                    disabled={Boolean(busy()) || !browser.supported || !browser.extensionStoreUrl}
                    onClick={() => void installExtension(browser.kind)}
                  >
                    {zh() ? "安装扩展" : "Install extension"}
                  </Button>
                </SettingsRow>
              )}
            </For>
            <Show when={(browserSettings()?.detectedBrowsers.length ?? 0) === 0}>
              <SettingsRow
                label={zh() ? "系统浏览器" : "System browser"}
                description={
                  zh()
                    ? "未检测到 Chrome 或 Edge；Browser Control 将继续使用内置浏览器。"
                    : "Chrome or Edge was not detected; Browser Control will continue with the embedded browser."
                }
              >
                <Badge tone="neutral">CEF</Badge>
              </SettingsRow>
            </Show>
            <Show when={browserSettings()?.pendingAuthorization}>
              {(pending) => (
                <SettingsRow
                  label={zh() ? "扩展授权" : "Extension authorization"}
                  description={
                    zh()
                      ? "检测到新的浏览器扩展安装。确认后会自动交换连接令牌。"
                      : "A new browser extension installation was detected. Confirm to exchange a connection token automatically."
                  }
                >
                  <Button
                    variant="primary"
                    disabled={Boolean(busy())}
                    onClick={() => void approveExtension(pending())}
                  >
                    <ShieldCheck size={14} /> {zh() ? "允许连接" : "Allow connection"}
                  </Button>
                </SettingsRow>
              )}
            </Show>
            <Show when={browserSettings()?.latestPairing}>
              <SettingsRow
                label={zh() ? "扩展连接" : "Extension connection"}
                description={
                  zh()
                    ? "授权已保存在系统安全存储中，应用重启后会自动恢复。"
                    : "Authorization is stored in the OS credential store and reconnects after restart."
                }
              >
                <Badge tone="success">{zh() ? "已授权" : "Authorized"}</Badge>
              </SettingsRow>
            </Show>
            <SettingsRow
              label={zh() ? "Agent 浏览器" : "Agent browser"}
              description={
                zh()
                  ? "关闭后所有交互式与计划任务 Browser 自动化均不可用。"
                  : "Disables Browser automation for interactive and scheduled runs."
              }
            >
              <Toggle
                label={zh() ? "允许 Agent 自动化" : "Allow Agent automation"}
                checked={browserSettings()?.automationEnabled ?? false}
                disabled={Boolean(busy()) || !browserSettings()}
                onChange={(enabled) => void updateBrowserSettings({ automationEnabled: enabled })}
              />
            </SettingsRow>
            <SettingsRow
              label={zh() ? "自动化表面偏好" : "Automation surface preference"}
              description={
                zh()
                  ? "自动模式优先内置 CEF；选择外置 Chrome 时仅在扩展配对健康时使用。"
                  : "Auto prefers embedded CEF; external Chrome is used only while extension pairing is healthy."
              }
            >
              <Select
                label={zh() ? "浏览器表面" : "Browser surface"}
                value={browserSettings()?.automationPreference ?? "auto"}
                options={[
                  { value: "auto", label: zh() ? "自动" : "Auto" },
                  { value: "embedded", label: zh() ? "内置浏览器" : "Embedded browser" },
                  { value: "external_chrome", label: zh() ? "外置 Chrome" : "External Chrome" },
                ]}
                disabled={Boolean(busy()) || !browserSettings()}
                onChange={(value) =>
                  void updateBrowserSettings({
                    automationPreference: value as BrowserAutomationPreference,
                  })
                }
              />
            </SettingsRow>
          </SettingsCard>
        </SettingsSection>
        <BrowserSettingsSection featureFlags={props.featureFlags} />
        <BrowserSitePolicySettings />
      </Show>

      <Show when={props.section === "computer-use"}>
        <ComputerUseSettings refreshRevision={computerRefreshRevision()} />
      </Show>
    </div>
  );
}
