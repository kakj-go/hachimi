import {
  commandFailure,
  commands,
  type BrowserDataKind,
  type BrowserHistoryEntry,
  type EmbeddedBrowserSettings,
  type FeatureFlags,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import {
  Badge,
  Button,
  SettingsCard,
  SettingsRow,
  SettingsSection,
  StatusBanner,
  Switch as Toggle,
} from "@hachimi/ui";
import { For, Show, createSignal, onMount } from "solid-js";

export function BrowserSettingsSection(props: { featureFlags: FeatureFlags }) {
  const i18n = useI18n();
  const zh = () => i18n.locale() === "zh-CN";
  const [settings, setSettings] = createSignal<EmbeddedBrowserSettings>();
  const [history, setHistory] = createSignal<BrowserHistoryEntry[]>([]);
  const [dataSelection, setDataSelection] = createSignal<BrowserDataKind[]>([
    "history",
    "cookies",
    "cache",
  ]);
  const [busy, setBusy] = createSignal(false);
  const [failure, setFailure] = createSignal<string>();
  const [notice, setNotice] = createSignal<string>();

  async function run(operation: () => Promise<void>) {
    setBusy(true);
    setFailure(undefined);
    setNotice(undefined);
    try {
      await operation();
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
    }
  }

  async function load() {
    await run(async () => {
      const [browserSettings, browserHistory] = await Promise.all([
        commands.getEmbeddedBrowserSettings(),
        commands.getBrowserHistory("", 12),
      ]);
      setSettings(browserSettings);
      setHistory(browserHistory);
    });
  }

  async function updateSettings(
    patch: Partial<
      Pick<
        EmbeddedBrowserSettings,
        "downloadDirectory" | "askWhereToSaveDownloads" | "fullCdpAccess"
      >
    >,
  ) {
    const current = settings();
    if (!current) return;
    await run(async () => {
      const next = await commands.updateEmbeddedBrowserSettings({
        downloadDirectory:
          "downloadDirectory" in patch
            ? (patch.downloadDirectory ?? null)
            : current.downloadDirectory,
        askWhereToSaveDownloads: patch.askWhereToSaveDownloads ?? current.askWhereToSaveDownloads,
        fullCdpAccess: patch.fullCdpAccess ?? current.fullCdpAccess,
        expectedRevision: current.revision,
      });
      setSettings(next);
      setNotice(zh() ? "内置浏览器设置已更新。" : "Embedded browser settings updated.");
    });
  }

  async function chooseDownloadDirectory() {
    await run(async () => {
      const directory = await commands.chooseBrowserDownloadDirectory();
      const current = settings();
      if (!directory || !current) return;
      setSettings(
        await commands.updateEmbeddedBrowserSettings({
          downloadDirectory: directory,
          askWhereToSaveDownloads: current.askWhereToSaveDownloads,
          fullCdpAccess: current.fullCdpAccess,
          expectedRevision: current.revision,
        }),
      );
      setNotice(zh() ? "下载位置已更新。" : "Download location updated.");
    });
  }

  function toggleData(kind: BrowserDataKind, enabled: boolean) {
    setDataSelection((current) =>
      enabled ? [...new Set([...current, kind])] : current.filter((value) => value !== kind),
    );
  }

  async function clearData() {
    const data = dataSelection();
    if (data.length === 0) return;
    await run(async () => {
      await commands.clearEmbeddedBrowserData({ data });
      if (data.includes("history")) setHistory([]);
      setNotice(zh() ? "选中的浏览器数据已清除。" : "Selected browser data was cleared.");
    });
  }

  onMount(() => void load());

  return (
    <SettingsSection title={zh() ? "内置浏览器" : "Embedded browser"}>
      <Show when={failure()}>
        {(message) => <StatusBanner tone="danger">{message()}</StatusBanner>}
      </Show>
      <Show when={notice()}>
        {(message) => <StatusBanner tone="success">{message()}</StatusBanner>}
      </Show>
      <SettingsCard>
        <SettingsRow
          label={zh() ? "CEF Profile" : "CEF profile"}
          description={
            zh()
              ? "应用独立 Profile；用户浏览与 Agent 站点权限相互独立。"
              : "App-isolated profile; manual browsing and Agent site permission remain separate."
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
        <SettingsRow
          label={zh() ? "下载位置" : "Download location"}
          description={
            settings()?.downloadDirectory ??
            (zh() ? "系统 Downloads 文件夹" : "System Downloads folder")
          }
        >
          <div class="local-hosts-actions">
            <Button size="small" disabled={busy()} onClick={() => void chooseDownloadDirectory()}>
              {zh() ? "选择文件夹" : "Choose folder"}
            </Button>
            <Button
              size="small"
              disabled={busy() || !settings()?.downloadDirectory}
              onClick={() => void updateSettings({ downloadDirectory: null })}
            >
              {zh() ? "恢复默认" : "Use default"}
            </Button>
          </div>
        </SettingsRow>
        <SettingsRow
          label={zh() ? "下载前询问" : "Ask where to save downloads"}
          description={
            zh()
              ? "每次下载都显示系统保存对话框。"
              : "Show the system save dialog for every download."
          }
        >
          <Toggle
            label={zh() ? "每次询问" : "Ask every time"}
            checked={settings()?.askWhereToSaveDownloads ?? false}
            disabled={busy() || !settings()}
            onChange={(enabled) => void updateSettings({ askWhereToSaveDownloads: enabled })}
          />
        </SettingsRow>
        <SettingsRow
          label="Developer mode"
          description={
            settings()?.fullCdpAccessAllowed
              ? zh()
                ? "完整 CDP 每次仍经过 Run、站点权限和审批校验。"
                : "Full CDP still requires Run, site-permission, and approval checks."
              : zh()
                ? "先在应用设置中开启 Developer mode 并重启。"
                : "Enable application Developer mode and restart first."
          }
        >
          <Toggle
            label={zh() ? "启用完整 CDP 访问" : "Enable full CDP access"}
            checked={settings()?.fullCdpAccess ?? false}
            disabled={busy() || !settings()?.fullCdpAccessAllowed}
            onChange={(enabled) => void updateSettings({ fullCdpAccess: enabled })}
          />
        </SettingsRow>
        <SettingsRow
          label={zh() ? "清除浏览数据" : "Clear browsing data"}
          description={
            zh()
              ? "历史记录存于 Hachimi；Cookie 和缓存由 CEF Profile 管理。"
              : "History is stored by Hachimi; cookies and cache belong to the CEF profile."
          }
        >
          <div class="local-hosts-actions">
            <Toggle
              label={zh() ? "历史记录" : "History"}
              checked={dataSelection().includes("history")}
              disabled={busy()}
              onChange={(value) => toggleData("history", value)}
            />
            <Toggle
              label="Cookie"
              checked={dataSelection().includes("cookies")}
              disabled={busy()}
              onChange={(value) => toggleData("cookies", value)}
            />
            <Toggle
              label={zh() ? "缓存" : "Cache"}
              checked={dataSelection().includes("cache")}
              disabled={busy()}
              onChange={(value) => toggleData("cache", value)}
            />
            <Button
              size="small"
              variant="danger"
              disabled={busy() || dataSelection().length === 0}
              data-testid="embedded-browser-clear-data"
              onClick={() => void clearData()}
            >
              {zh() ? "清除所选数据" : "Clear selected data"}
            </Button>
          </div>
        </SettingsRow>
        <For each={history().slice(0, 8)}>
          {(entry) => (
            <SettingsRow
              label={entry.title || entry.url}
              description={`${entry.url} · ${zh() ? "访问" : "visited"} ${entry.visitCount}`}
            >
              <Badge>{new Date(entry.lastVisitedAtMs).toLocaleDateString()}</Badge>
            </SettingsRow>
          )}
        </For>
      </SettingsCard>
    </SettingsSection>
  );
}
