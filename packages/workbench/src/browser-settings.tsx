import {
  commandFailure,
  commands,
  type BrowserDataKind,
  type EmbeddedBrowserSettings,
  type FeatureFlags,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import {
  Badge,
  Button,
  Dialog,
  SettingsCard,
  SettingsRow,
  SettingsSection,
  StatusBanner,
  Switch as Toggle,
} from "@hachimi/ui";
import { Show, createSignal, onMount, untrack } from "solid-js";

export function BrowserSettingsSection(props: { featureFlags: FeatureFlags }) {
  const i18n = useI18n();
  const zh = () => i18n.locale() === "zh-CN";
  const [settings, setSettings] = createSignal<EmbeddedBrowserSettings>();
  const [dataSelection, setDataSelection] = createSignal<BrowserDataKind[]>([
    "history",
    "cookies",
    "cache",
  ]);
  const [busy, setBusy] = createSignal(false);
  const [failure, setFailure] = createSignal<string>();
  const [notice, setNotice] = createSignal<string>();
  const [clearDialogOpen, setClearDialogOpen] = createSignal(false);

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
      setSettings(await commands.getEmbeddedBrowserSettings());
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
      const current = untrack(settings);
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
      setNotice(zh() ? "选中的浏览器数据已清除。" : "Selected browser data was cleared.");
      setClearDialogOpen(false);
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
          <div class="host-domain-actions">
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
        <Show when={settings()?.fullCdpAccessAllowed}>
          <SettingsRow
            label={zh() ? "开发者 CDP" : "Developer CDP"}
            description={
              zh()
                ? "完整 CDP 每次仍经过 Run、站点权限和审批校验。"
                : "Full CDP still requires Run, site-permission, and approval checks."
            }
          >
            <Toggle
              label={zh() ? "启用完整 CDP 访问" : "Enable full CDP access"}
              checked={settings()?.fullCdpAccess ?? false}
              disabled={busy()}
              onChange={(enabled) => void updateSettings({ fullCdpAccess: enabled })}
            />
          </SettingsRow>
        </Show>
        <SettingsRow
          label={zh() ? "清除浏览数据" : "Clear browsing data"}
          description={
            zh()
              ? "选择需要移除的数据类型，确认后立即清除。"
              : "Choose which data types to remove, then confirm."
          }
        >
          <Button
            size="small"
            variant="danger"
            disabled={busy()}
            data-testid="embedded-browser-clear-data"
            onClick={() => setClearDialogOpen(true)}
          >
            {zh() ? "选择并清除…" : "Choose and clear…"}
          </Button>
        </SettingsRow>
      </SettingsCard>
      <Dialog
        open={clearDialogOpen()}
        title={zh() ? "清除浏览数据" : "Clear browsing data"}
        description={
          zh()
            ? "此操作无法撤销。请选择本次要清除的数据。"
            : "This cannot be undone. Choose the data to clear."
        }
        closeLabel={zh() ? "关闭" : "Close"}
        tone="danger"
        loading={busy()}
        onOpenChange={(open) => !busy() && setClearDialogOpen(open)}
      >
        <div class="browser-clear-dialog">
          <BrowserDataOption
            label={zh() ? "浏览历史" : "Browsing history"}
            description={
              zh()
                ? "清除 Hachimi 保存的访问记录，不会删除书签或下载文件。"
                : "Removes visits saved by Hachimi, without deleting bookmarks or downloads."
            }
            checked={dataSelection().includes("history")}
            disabled={busy()}
            onChange={(value) => toggleData("history", value)}
          />
          <BrowserDataOption
            label={zh() ? "Cookie 与站点数据" : "Cookies and site data"}
            description={
              zh()
                ? "清除登录状态和站点偏好，之后可能需要重新登录。"
                : "Removes sign-in sessions and site preferences; sites may require sign-in again."
            }
            checked={dataSelection().includes("cookies")}
            disabled={busy()}
            onChange={(value) => toggleData("cookies", value)}
          />
          <BrowserDataOption
            label={zh() ? "缓存文件" : "Cached files"}
            description={
              zh()
                ? "清除 CEF Profile 缓存；网页下次打开时会重新加载资源。"
                : "Clears the CEF profile cache; pages reload resources on the next visit."
            }
            checked={dataSelection().includes("cache")}
            disabled={busy()}
            onChange={(value) => toggleData("cache", value)}
          />
          <div class="browser-clear-dialog-actions">
            <Button disabled={busy()} onClick={() => setClearDialogOpen(false)}>
              {zh() ? "取消" : "Cancel"}
            </Button>
            <Button
              variant="danger"
              disabled={busy() || dataSelection().length === 0}
              data-testid="embedded-browser-clear-confirm"
              onClick={() => void clearData()}
            >
              {zh() ? "清除所选数据" : "Clear selected data"}
            </Button>
          </div>
        </div>
      </Dialog>
    </SettingsSection>
  );
}

function BrowserDataOption(props: {
  label: string;
  description: string;
  checked: boolean;
  disabled: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <div class="browser-clear-option">
      <span>
        <strong>{props.label}</strong>
        <small>{props.description}</small>
      </span>
      <Toggle
        label={props.label}
        checked={props.checked}
        disabled={props.disabled}
        onChange={props.onChange}
      />
    </div>
  );
}
