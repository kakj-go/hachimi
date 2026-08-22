import { listen } from "@tauri-apps/api/event";
import {
  commandFailure,
  commands,
  type AppSettings,
  type BootstrapState,
  type DiagnosticsPaths,
  type LlmSettingsInput,
  type LlmSettingsView,
  type LlmTestResult,
  type ProviderProtocolKind,
  type ReasoningSummaryMode,
  type StructuredOutputMode,
  type Locale,
  type WorkbenchRoute,
} from "@hachimi/contracts";
import { I18nProvider, useI18n, type AppLocale } from "@hachimi/i18n";
import {
  Badge,
  AppearanceProvider,
  AlertTriangle,
  AppShell,
  ArrowLeft,
  ArrowRight,
  Bot,
  Box,
  Button,
  Copy,
  Dialog,
  Dropdown,
  Globe,
  FolderOpen,
  Maximize2,
  Minus,
  Monitor,
  NumberField,
  PageHeading,
  Palette,
  PanelLeftClose,
  Play,
  Plug,
  Plus,
  Puzzle,
  SearchField,
  SelectField,
  Sidebar,
  SettingsCard,
  Settings,
  SettingsRow,
  SettingsSection,
  StatusBanner,
  Switch as Toggle,
  TextField,
  TitleBar,
  Trash2,
  Volume2,
  X,
} from "@hachimi/ui";
import {
  For,
  Match,
  Show,
  Switch,
  createMemo,
  createSignal,
  onCleanup,
  onMount,
  untrack,
  type JSX,
} from "solid-js";

import { runtimeFeatureVisibility } from "./runtime-feature-visibility";
import { normalizeRemoteContextFields } from "./llm-settings-normalization";
import "./workbench.css";
import "./diagnostics-settings.css";
import "./appearance-workbench.css";
import "./workspace-browser.css";
import "./resource-settings.css";
import "./motion.css";
import "./extensions-settings.css";
import "./ui-contract.css";
import "./host-domain-settings.css";
import "./host-inspectors.css";
import "./platform-integrations-settings.css";
import "./runtime-health.css";
import { AppearanceSettings } from "./settings-appearance";
import { HomePage } from "./home";
import { MotionLabPage } from "./motion-lab";
import { MotionSettingsPage } from "./motion-settings";
import { McpSettingsPage } from "./mcp-settings";
import { HostDomainSettingsPage } from "./host-domain-settings";
import { PlatformIntegrationsSettings } from "./platform-integrations-settings";
import { ResourceSettingsPage, VoiceSettingsPage } from "./resource-settings";
import { SkillsSettingsPage } from "./skills-settings";
import { PetPermissionSettings } from "./pet-permission-settings";
import { normalizeWorkbenchRoute, SETTINGS_ROUTES } from "./routing";

function initialRoute(): WorkbenchRoute {
  return normalizeWorkbenchRoute(new URLSearchParams(window.location.search).get("route"));
}

function WindowChrome(props: {
  canGoBack: boolean;
  canGoForward: boolean;
  motionLabEnabled: boolean;
  onBack: () => void;
  onForward: () => void;
  onToggleSidebar: () => void;
  onNavigate: (route: WorkbenchRoute) => void;
}) {
  const i18n = useI18n();
  const text = (zh: string, en: string) => (i18n.locale() === "zh-CN" ? zh : en);
  const drag = (event: PointerEvent & { currentTarget: HTMLElement; target: Element }) => {
    if (event.button === 0 && !event.target.closest("button")) {
      void commands.startWorkbenchDragging();
    }
  };
  return (
    <>
      <TitleBar
        brand=""
        onPointerDown={drag}
        onDoubleClick={(event) => {
          if (!(event.target as Element).closest("button")) {
            void commands.toggleMaximizeWorkbench();
          }
        }}
      >
        <Button
          type="button"
          class="title-sidebar-toggle"
          aria-label="Sidebar"
          onClick={() => props.onToggleSidebar()}
        >
          <PanelLeftClose size={16} />
        </Button>
        <div class="title-history">
          <Button
            type="button"
            aria-label={i18n.t("workbench.back")}
            disabled={!props.canGoBack}
            onClick={() => props.onBack()}
          >
            <ArrowLeft size={15} />
          </Button>
          <Button
            type="button"
            aria-label={i18n.t("workbench.forward")}
            disabled={!props.canGoForward}
            onClick={() => props.onForward()}
          >
            <ArrowRight size={15} />
          </Button>
        </div>
        <div class="title-menus" aria-label={i18n.t("workbench.appMenu")}>
          <Dropdown
            label={i18n.t("workbench.menu.file")}
            actions={[
              { id: "new_task", label: i18n.t("workbench.newTask"), icon: <Plus size={15} /> },
              {
                id: "add_project",
                label: i18n.t("workbench.addProject"),
                icon: <Box size={15} />,
              },
              {
                id: "settings",
                label: i18n.t("settings.title"),
                icon: <Settings size={15} />,
                separatorBefore: true,
              },
              {
                id: "close",
                label: text("关闭工作台", "Close Workbench"),
                icon: <X size={15} />,
              },
            ]}
            onSelect={(id) => {
              if (id === "new_task") window.dispatchEvent(new Event("hachimi:new-task"));
              else if (id === "add_project") window.dispatchEvent(new Event("hachimi:add-project"));
              else if (id === "settings") props.onNavigate("settings/general");
              else if (id === "close") void commands.hideWorkbench();
            }}
          >
            {i18n.t("workbench.menu.file")}
          </Dropdown>
          <Dropdown
            label={i18n.t("workbench.menu.edit")}
            actions={[
              { id: "undo", label: text("撤销", "Undo") },
              { id: "redo", label: text("重做", "Redo") },
              { id: "cut", label: text("剪切", "Cut"), separatorBefore: true },
              { id: "copy", label: text("复制", "Copy"), icon: <Copy size={15} /> },
              { id: "paste", label: text("粘贴", "Paste") },
              { id: "select_all", label: text("全选", "Select all"), separatorBefore: true },
            ]}
            onSelect={(id) => {
              const command = id === "select_all" ? "selectAll" : id;
              document.execCommand(command);
            }}
          >
            {i18n.t("workbench.menu.edit")}
          </Dropdown>
          <Dropdown
            label={i18n.t("workbench.menu.view")}
            actions={[
              {
                id: "toggle_sidebar",
                label: text("切换侧栏", "Toggle sidebar"),
                icon: <PanelLeftClose size={15} />,
              },
              {
                id: "back",
                label: i18n.t("workbench.back"),
                icon: <ArrowLeft size={15} />,
                disabled: !props.canGoBack,
              },
              {
                id: "forward",
                label: i18n.t("workbench.forward"),
                icon: <ArrowRight size={15} />,
                disabled: !props.canGoForward,
              },
              {
                id: "appearance",
                label: i18n.t("settings.appearance"),
                icon: <Palette size={15} />,
                separatorBefore: true,
              },
              ...(props.motionLabEnabled
                ? [
                    {
                      id: "motion",
                      label: text("动作库实验室", "Motion Library Lab"),
                      icon: <Play size={15} />,
                    },
                  ]
                : []),
            ]}
            onSelect={(id) => {
              if (id === "toggle_sidebar") props.onToggleSidebar();
              else if (id === "back") props.onBack();
              else if (id === "forward") props.onForward();
              else if (id === "appearance") props.onNavigate("settings/appearance");
              else if (id === "motion") props.onNavigate("developer/motion-lab");
            }}
          >
            {i18n.t("workbench.menu.view")}
          </Dropdown>
          <Dropdown
            label={i18n.t("workbench.menu.help")}
            actions={[
              { id: "settings", label: text("打开设置", "Open settings") },
              { id: "about", label: text("关于 Hachimi", "About Hachimi") },
            ]}
            onSelect={(id) => {
              if (id === "settings") props.onNavigate("settings/general");
              else window.alert(text("Hachimi 1.0.0", "Hachimi 1.0.0"));
            }}
          >
            {i18n.t("workbench.menu.help")}
          </Dropdown>
        </div>
        <div class="window-controls">
          <Button
            type="button"
            aria-label={i18n.t("workbench.window.minimize")}
            onClick={() => void commands.minimizeWorkbench()}
          >
            <Minus size={16} />
          </Button>
          <Button
            type="button"
            aria-label={i18n.t("workbench.window.maximize")}
            onClick={() => void commands.toggleMaximizeWorkbench()}
          >
            <Maximize2 size={14} />
          </Button>
          <Button
            type="button"
            class="window-close"
            aria-label={i18n.t("workbench.window.close")}
            onClick={() => void commands.hideWorkbench()}
          >
            <X size={16} />
          </Button>
        </div>
      </TitleBar>
      <For
        each={[
          "north",
          "north-east",
          "east",
          "south-east",
          "south",
          "south-west",
          "west",
          "north-west",
        ]}
      >
        {(direction) => (
          <div
            class="resize-handle"
            data-direction={direction}
            aria-hidden="true"
            onPointerDown={(event) => {
              if (event.button === 0) void commands.startWorkbenchResize(direction);
            }}
          />
        )}
      </For>
    </>
  );
}

const settingsRoutes: readonly WorkbenchRoute[] = SETTINGS_ROUTES;

function SettingsShell(props: {
  route: WorkbenchRoute;
  navigate: (route: WorkbenchRoute) => void;
  settings: AppSettings;
  setSettings: (settings: AppSettings) => void;
  fail: (message: string) => void;
  featureFlags: BootstrapState["featureFlags"];
}) {
  const i18n = useI18n();
  const runtimeVisibility = () => runtimeFeatureVisibility(props.featureFlags);
  const [search, setSearch] = createSignal("");
  const groups = createMemo(() => {
    const item = (
      route: WorkbenchRoute | null,
      label: string,
      icon: JSX.Element,
      options: { id?: string; disabled?: boolean; status?: string } = {},
    ) => ({
      route,
      label,
      icon,
      id: options.id ?? route?.replace("settings/", "") ?? label.toLowerCase(),
      disabled: options.disabled ?? false,
      status: options.status,
    });
    const all = [
      {
        label: i18n.t("settings.group.personal"),
        items: [
          item("settings/general", i18n.t("settings.general"), <Settings size={17} />),
          item("settings/appearance", i18n.t("settings.appearance"), <Palette size={17} />),
          item("settings/voice", i18n.t("settings.voice"), <Volume2 size={17} />),
          item("settings/llm", i18n.t("settings.configuration"), <Bot size={17} />),
          item("settings/avatar", i18n.t("settings.pet"), <Box size={17} />),
          item(
            "settings/motion",
            i18n.locale() === "zh-CN" ? "交互" : "Interactions",
            <Play size={17} />,
          ),
        ],
      },
      {
        label: i18n.locale() === "zh-CN" ? "能力与集成" : "Capabilities & integrations",
        items: [
          item(
            "settings/integrations",
            i18n.locale() === "zh-CN" ? "平台集成" : "Platform integrations",
            <Plug size={17} />,
          ),
          item(
            "settings/browser",
            i18n.locale() === "zh-CN" ? "浏览器" : "Browser",
            <Globe size={17} />,
          ),
          item("settings/computer-use", "Computer Use", <Monitor size={17} />),
          item("settings/skills", i18n.t("settings.skills"), <Puzzle size={17} />),
          item("settings/mcp", i18n.t("settings.mcp"), <Plug size={17} />),
          item(null, "Plugins", <Puzzle size={17} />, {
            id: "plugins",
            disabled: true,
            status: i18n.locale() === "zh-CN" ? "暂不开放" : "Not available",
          }),
        ],
      },
      {
        label: i18n.locale() === "zh-CN" ? "系统" : "System",
        items: [
          item("settings/runtime-security", "Runtime & Security", <Globe size={17} />),
          item(
            "settings/diagnostics",
            i18n.locale() === "zh-CN" ? "诊断" : "Diagnostics",
            <AlertTriangle size={17} />,
          ),
        ],
      },
    ];
    const query = search().trim().toLocaleLowerCase();
    return all
      .map((group) => ({
        ...group,
        items: group.items.filter((entry) => entry.label.toLocaleLowerCase().includes(query)),
      }))
      .filter((group) => group.items.length > 0);
  });
  const activeLabel = createMemo(() => {
    for (const group of groups()) {
      const entry = group.items.find((entry) => entry.route === props.route);
      if (entry) return entry.label;
    }
    return i18n.t("settings.title");
  });
  return (
    <div
      class="settings-layout settings-workspace settings-app"
      data-density={props.settings.appearance.preferences.density ?? "default"}
    >
      <Sidebar class="settings-sidebar">
        <div class="settings-sidebar-brand settings-sidebar-head">
          <Button
            type="button"
            class="back-home"
            aria-label={i18n.t("settings.backHome")}
            onClick={() => props.navigate("home")}
          >
            <ArrowLeft size={16} />
          </Button>
          <strong>Hachimi</strong>
        </div>
        <SearchField
          class="settings-search"
          label={i18n.t("settings.search")}
          placeholder={i18n.t("settings.search")}
          value={search()}
          onInput={(event) => setSearch(event.currentTarget.value)}
        />
        <nav class="settings-nav settings-nav-scroll" aria-label={i18n.t("settings.title")}>
          <For each={groups()}>
            {(group) => (
              <section class="settings-nav-group">
                <h2>{group.label}</h2>
                <For each={group.items}>
                  {(item) => (
                    <Button
                      type="button"
                      class="settings-nav-row"
                      data-testid={`settings-nav-${item.id}`}
                      classList={{
                        selected: props.route === item.route,
                        active: props.route === item.route,
                      }}
                      disabled={item.disabled}
                      aria-disabled={item.disabled || undefined}
                      aria-current={props.route === item.route ? "page" : undefined}
                      onClick={() => item.route && props.navigate(item.route)}
                    >
                      {item.icon}
                      <span>{item.label}</span>
                      <Show when={item.status}>
                        <small class="settings-nav-status">{item.status}</small>
                      </Show>
                    </Button>
                  )}
                </For>
              </section>
            )}
          </For>
        </nav>
      </Sidebar>
      <main class="settings-main">
        <header class="settings-workspace-topline settings-header">
          <strong>{i18n.t("settings.title")}</strong>
          <span aria-hidden="true">·</span>
          <span>{activeLabel()}</span>
          <div class="settings-header-actions">
            <span class="saved-state">
              <i class="status-dot" aria-hidden="true" />
              {i18n.t("settings.appearance.saved")}
            </span>
          </div>
        </header>
        <div class="settings-scroll">
          <Switch>
            <Match when={props.route === "settings/general"}>
              <GeneralSettings
                settings={props.settings}
                setSettings={props.setSettings}
                fail={props.fail}
              />
            </Match>
            <Match when={props.route === "settings/appearance"}>
              <AppearanceSettings
                settings={props.settings}
                setSettings={props.setSettings}
                fail={props.fail}
              />
            </Match>
            <Match when={props.route === "settings/llm"}>
              <LlmSettingsPage
                providerExtensionsEnabled={runtimeVisibility().providerExtensions}
                providerRemoteContextEnabled={runtimeVisibility().providerRemoteContext}
              />
            </Match>
            <Match when={props.route === "settings/avatar"}>
              <ResourceSettingsPage />
            </Match>
            <Match when={props.route === "settings/motion"}>
              <MotionSettingsPage />
            </Match>
            <Match when={props.route === "settings/voice"}>
              <VoiceSettingsPage />
            </Match>
            <Match when={props.route === "settings/skills"}>
              <SkillsSettingsPage />
            </Match>
            <Match when={props.route === "settings/mcp"}>
              <McpSettingsPage connectorEnabled={props.featureFlags.mcpRuntime} />
            </Match>
            <Match when={props.route === "settings/integrations"}>
              <PlatformIntegrationsSettings />
            </Match>
            <Match when={props.route === "settings/browser"}>
              <HostDomainSettingsPage featureFlags={props.featureFlags} section="browser" />
            </Match>
            <Match when={props.route === "settings/computer-use"}>
              <HostDomainSettingsPage featureFlags={props.featureFlags} section="computer-use" />
            </Match>
            <Match when={props.route === "settings/runtime-security"}>
              <HostDomainSettingsPage
                featureFlags={props.featureFlags}
                section="runtime-security"
                developerMode={props.settings.developerMode ?? false}
              />
            </Match>
            <Match when={props.route === "settings/diagnostics"}>
              <DiagnosticsSettings
                settings={props.settings}
                setSettings={props.setSettings}
                fail={props.fail}
              />
            </Match>
          </Switch>
        </div>
      </main>
    </div>
  );
}

function GeneralSettings(props: {
  settings: AppSettings;
  setSettings: (settings: AppSettings) => void;
  fail: (message: string) => void;
}) {
  const i18n = useI18n();
  async function persist(patch: Partial<AppSettings>) {
    const previous = props.settings;
    const next = { ...previous, ...patch };
    props.setSettings(next);
    try {
      props.setSettings(await commands.updateSettings(next));
    } catch (error) {
      props.setSettings(previous);
      props.fail(commandFailure(error).message);
    }
  }
  async function changeAlwaysOnTop(enabled: boolean) {
    try {
      props.setSettings(await commands.setAlwaysOnTop(enabled));
    } catch (error) {
      props.fail(commandFailure(error).message);
    }
  }
  return (
    <div class="settings-page settings-page-demo">
      <PageHeading
        class="settings-page-heading"
        title={i18n.t("settings.general")}
        description={i18n.t("settings.general.description")}
      />
      <SettingsSection title={i18n.t("settings.general")}>
        <SettingsCard class="settings-card settings-card-demo">
          <SettingsRow label={i18n.t("settings.language")}>
            <SelectField
              label={i18n.t("settings.language")}
              value={i18n.locale()}
              options={[
                { value: "zh-CN", label: "简体中文" },
                { value: "en-US", label: "English" },
              ]}
              onChange={(value) => {
                const locale = value as AppLocale;
                i18n.setLocale(locale);
                void persist({ locale: locale as Locale });
              }}
            />
          </SettingsRow>
          <SettingsRow
            label={i18n.t("settings.alwaysOnTop")}
            description={i18n.t("settings.petOnly")}
          >
            <Toggle
              checked={props.settings.alwaysOnTop}
              label={i18n.t("settings.alwaysOnTop")}
              onChange={(enabled) => void changeAlwaysOnTop(enabled)}
            />
          </SettingsRow>
        </SettingsCard>
      </SettingsSection>
      <PetPermissionSettings />
    </div>
  );
}

function DiagnosticsSettings(props: {
  settings: AppSettings;
  setSettings: (settings: AppSettings) => void;
  fail: (message: string) => void;
}) {
  const i18n = useI18n();
  const [resetOpen, setResetOpen] = createSignal(false);
  const [resetting, setResetting] = createSignal(false);
  const [paths, setPaths] = createSignal<DiagnosticsPaths>();
  const [openingLogs, setOpeningLogs] = createSignal(false);
  async function persist(patch: Partial<AppSettings>) {
    const previous = props.settings;
    const next = { ...previous, ...patch };
    props.setSettings(next);
    try {
      props.setSettings(await commands.updateSettings(next));
    } catch (error) {
      props.setSettings(previous);
      props.fail(commandFailure(error).message);
    }
  }
  async function resetAllLocalData() {
    setResetting(true);
    try {
      await commands.resetLocalData();
    } catch (error) {
      setResetting(false);
      setResetOpen(false);
      props.fail(commandFailure(error).message);
    }
  }
  async function openLogs() {
    setOpeningLogs(true);
    try {
      await commands.openLogsDirectory();
    } catch (error) {
      props.fail(commandFailure(error).message);
    } finally {
      setOpeningLogs(false);
    }
  }
  onMount(() => {
    void commands
      .getDiagnosticsPaths()
      .then(setPaths)
      // eslint-disable-next-line solid/reactivity -- Promise failures are reported outside rendering.
      .catch((error) => props.fail(commandFailure(error).message));
  });
  return (
    <div class="settings-page settings-page-demo">
      <PageHeading
        class="settings-page-heading"
        title={i18n.locale() === "zh-CN" ? "诊断" : "Diagnostics"}
        description={
          i18n.locale() === "zh-CN"
            ? "查看构建信息、开发者能力并管理本地数据。"
            : "Inspect build information, developer capabilities, and local data."
        }
      />
      <SettingsSection title={i18n.locale() === "zh-CN" ? "运行日志" : "Application logs"}>
        <SettingsCard class="settings-card settings-card-demo">
          <SettingsRow
            label={i18n.locale() === "zh-CN" ? "日志目录" : "Log directory"}
            description={
              i18n.locale() === "zh-CN"
                ? "后端、前端和本地服务日志保存在应用数据目录，不在安装目录。"
                : "Backend, frontend, and local-service logs are stored with app data, not in the installation directory."
            }
          >
            <div class="settings-path-action">
              <code title={paths()?.logDirectory}>{paths()?.logDirectory ?? "..."}</code>
              <Button
                size="small"
                disabled={!paths() || openingLogs()}
                title={i18n.locale() === "zh-CN" ? "打开日志目录" : "Open log directory"}
                onClick={() => void openLogs()}
              >
                <FolderOpen size={14} />
                {openingLogs()
                  ? i18n.locale() === "zh-CN"
                    ? "正在打开"
                    : "Opening"
                  : i18n.locale() === "zh-CN"
                    ? "打开"
                    : "Open"}
              </Button>
            </div>
          </SettingsRow>
        </SettingsCard>
      </SettingsSection>
      <SettingsSection title={i18n.locale() === "zh-CN" ? "开发者" : "Developer"}>
        <SettingsCard class="settings-card settings-card-demo">
          <SettingsRow
            label={i18n.locale() === "zh-CN" ? "开发者模式" : "Developer mode"}
            description={
              i18n.locale() === "zh-CN"
                ? "重启后显示开发者专用界面和完整 Browser CDP 能力。"
                : "Shows developer-only surfaces and full Browser CDP controls after restart."
            }
          >
            <Toggle
              checked={props.settings.developerMode ?? false}
              label={i18n.locale() === "zh-CN" ? "开发者模式" : "Developer mode"}
              onChange={(enabled) => void persist({ developerMode: enabled })}
            />
          </SettingsRow>
        </SettingsCard>
      </SettingsSection>
      <SettingsSection title={i18n.t("settings.about.title")}>
        <SettingsCard class="settings-card settings-card-demo">
          <SettingsRow label={i18n.t("settings.about.version")}>
            <Badge>v1.0.0</Badge>
          </SettingsRow>
          <SettingsRow
            label={i18n.t("settings.about.sourceLicense")}
            description={i18n.t("settings.about.sourceLicenseDescription")}
          >
            <Badge>Apache-2.0</Badge>
          </SettingsRow>
        </SettingsCard>
        <StatusBanner tone="warning">
          <strong>{i18n.t("settings.about.binaryBoundary")}</strong>
          <span>{i18n.t("settings.about.binaryBoundaryDescription")}</span>
          <small>{i18n.t("settings.about.notice")}</small>
        </StatusBanner>
      </SettingsSection>
      <SettingsSection title={i18n.t("settings.data.title")}>
        <SettingsCard class="settings-card settings-card-demo">
          <SettingsRow
            label={i18n.t("settings.data.reset")}
            description={i18n.t("settings.data.description")}
          >
            <Button variant="danger" onClick={() => setResetOpen(true)}>
              <Trash2 size={15} />
              {i18n.t("settings.data.reset")}
            </Button>
          </SettingsRow>
        </SettingsCard>
      </SettingsSection>
      <Dialog
        open={resetOpen()}
        title={i18n.t("settings.data.resetTitle")}
        description={i18n.t("settings.data.resetConfirm")}
        onOpenChange={(open) => {
          if (!resetting()) setResetOpen(open);
        }}
      >
        <div class="dialog-actions">
          <Button variant="ghost" disabled={resetting()} onClick={() => setResetOpen(false)}>
            {i18n.t("common.cancel")}
          </Button>
          <Button variant="danger" disabled={resetting()} onClick={() => void resetAllLocalData()}>
            {resetting() ? i18n.t("settings.data.resetting") : i18n.t("common.confirm")}
          </Button>
        </div>
      </Dialog>
    </div>
  );
}

function LlmSettingsPage(props: {
  providerExtensionsEnabled: boolean;
  providerRemoteContextEnabled: boolean;
}) {
  const i18n = useI18n();
  const [view, setView] = createSignal<LlmSettingsView>();
  const [baseUrl, setBaseUrl] = createSignal("");
  const [modelName, setModelName] = createSignal("");
  const [protocol, setProtocol] = createSignal<ProviderProtocolKind>("chat_completions");
  const [compatibilityProfileId, setCompatibilityProfileId] = createSignal("openai-strict");
  const [reasoningSummary, setReasoningSummary] = createSignal<ReasoningSummaryMode>("none");
  const [remoteCompaction, setRemoteCompaction] = createSignal(false);
  const [maxInput, setMaxInput] = createSignal(0);
  const [maxOutput, setMaxOutput] = createSignal(0);
  const [structuredOutputMode, setStructuredOutputMode] =
    createSignal<StructuredOutputMode>("auto");
  const [apiKey, setApiKey] = createSignal("");
  const [clearKey, setClearKey] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [notice, setNotice] = createSignal<{ tone: "success" | "danger"; text: string }>();
  const [testResult, setTestResult] = createSignal<LlmTestResult>();

  function useView(next: LlmSettingsView) {
    setView(next);
    setBaseUrl(next.baseUrl);
    setModelName(next.modelName);
    setProtocol(props.providerExtensionsEnabled ? next.protocol : "chat_completions");
    setCompatibilityProfileId(next.compatibilityProfileId);
    const remoteContext = normalizeRemoteContextFields(
      next.protocol,
      props.providerExtensionsEnabled && props.providerRemoteContextEnabled,
      next.reasoningSummary,
      next.remoteCompaction,
    );
    setReasoningSummary(remoteContext.reasoningSummary);
    setRemoteCompaction(remoteContext.remoteCompaction);
    setMaxInput(next.maxInputTokens);
    setMaxOutput(next.maxOutputTokens);
    setStructuredOutputMode(next.structuredOutputMode);
    setApiKey("");
    setClearKey(false);
  }
  function input(): LlmSettingsInput {
    const remoteContext = normalizeRemoteContextFields(
      protocol(),
      props.providerExtensionsEnabled && props.providerRemoteContextEnabled,
      reasoningSummary(),
      remoteCompaction(),
    );
    return {
      baseUrl: baseUrl(),
      modelName: modelName(),
      protocol: props.providerExtensionsEnabled ? protocol() : "chat_completions",
      compatibilityProfileId: compatibilityProfileId(),
      providerEndpointId: view()?.providerEndpointId ?? null,
      providerAccountId: view()?.providerAccountId ?? null,
      embeddingModelName: "",
      reasoningSummary: remoteContext.reasoningSummary,
      remoteCompaction: remoteContext.remoteCompaction,
      maxInputTokens: maxInput(),
      maxOutputTokens: maxOutput(),
      structuredOutputMode: structuredOutputMode(),
      apiKey: apiKey() || null,
      clearApiKey: clearKey(),
    };
  }
  async function save(test: boolean) {
    setBusy(true);
    setNotice(undefined);
    setTestResult(undefined);
    try {
      if (test) {
        const result = await commands.saveAndTestLlmSettings(input());
        setTestResult(result);
        setNotice({
          tone: "success",
          text: i18n.t("settings.testSuccess").replace("{latency}", String(result.latencyMs)),
        });
        useView(await commands.getLlmSettings());
      } else {
        useView(await commands.saveLlmSettings(input()));
        setNotice({ tone: "success", text: i18n.t("settings.saved") });
      }
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    } finally {
      setBusy(false);
    }
  }
  onMount(async () => {
    try {
      useView(await commands.getLlmSettings());
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    }
  });
  return (
    <div class="settings-page settings-page-demo">
      <PageHeading
        class="settings-page-heading"
        title={i18n.t("settings.llm")}
        description={i18n.t("settings.llm.description")}
        badge="OpenAI-compatible"
      />
      <SettingsSection title={i18n.t("settings.connection")}>
        <SettingsCard class="settings-card unified-settings-card">
          <SettingsRow
            label={i18n.t("settings.providerProtocol")}
            description={i18n.t("settings.providerProtocol.description")}
          >
            <SelectField
              label={i18n.t("settings.providerProtocol")}
              value={protocol()}
              options={[
                { value: "chat_completions", label: "OpenAI Chat Completions" },
                ...(props.providerExtensionsEnabled
                  ? [{ value: "responses", label: "OpenAI Responses" }]
                  : []),
              ]}
              onChange={(value) => {
                const next = value as ProviderProtocolKind;
                setProtocol(next);
                if (next !== "responses") {
                  setReasoningSummary("none");
                  setRemoteCompaction(false);
                }
              }}
            />
          </SettingsRow>
          <Show when={props.providerExtensionsEnabled}>
            <SettingsRow
              label={i18n.t("settings.compatibilityProfile")}
              description={i18n.t("settings.compatibilityProfile.description")}
            >
              <TextField
                label={i18n.t("settings.compatibilityProfile")}
                value={compatibilityProfileId()}
                onInput={(event) => setCompatibilityProfileId(event.currentTarget.value)}
              />
            </SettingsRow>
          </Show>
          <SettingsRow label={i18n.t("settings.interfaceUrl")}>
            <TextField
              label={i18n.t("settings.interfaceUrl")}
              value={baseUrl()}
              placeholder="http://localhost:11434/v1"
              onInput={(event) => setBaseUrl(event.currentTarget.value)}
            />
          </SettingsRow>
          <SettingsRow
            label={i18n.t("settings.apiKey")}
            description={i18n.t("settings.apiKey.keep")}
          >
            <div class="settings-control-stack">
              <TextField
                label={i18n.t("settings.apiKey")}
                type="password"
                value={apiKey()}
                onInput={(event) => {
                  setApiKey(event.currentTarget.value);
                  if (event.currentTarget.value) setClearKey(false);
                }}
              />
              <Badge tone={view()?.apiKeyConfigured ? "success" : "neutral"}>
                {view()?.apiKeyConfigured
                  ? i18n.t("settings.apiKey.configured")
                  : i18n.t("settings.apiKey.notConfigured")}
              </Badge>
            </div>
          </SettingsRow>
          <SettingsRow label={i18n.t("settings.apiKey.clear")}>
            <Toggle
              checked={clearKey()}
              disabled={!view()?.apiKeyConfigured}
              label={i18n.t("settings.apiKey.clear")}
              onChange={(checked) => {
                setClearKey(checked);
                if (checked) setApiKey("");
              }}
            />
          </SettingsRow>
          <SettingsRow label={i18n.t("settings.modelName")}>
            <TextField
              label={i18n.t("settings.modelName")}
              value={modelName()}
              placeholder="gpt-5.6-sol"
              onInput={(event) => setModelName(event.currentTarget.value)}
            />
          </SettingsRow>
          <Show when={protocol() === "responses" && props.providerRemoteContextEnabled}>
            <SettingsRow
              label={i18n.t("settings.reasoningSummary")}
              description={i18n.t("settings.reasoningSummary.description")}
            >
              <SelectField
                value={reasoningSummary()}
                label={i18n.t("settings.reasoningSummary")}
                options={[
                  { value: "auto", label: i18n.t("settings.reasoningSummary.auto") },
                  { value: "concise", label: i18n.t("settings.reasoningSummary.concise") },
                  { value: "detailed", label: i18n.t("settings.reasoningSummary.detailed") },
                  { value: "none", label: i18n.t("settings.reasoningSummary.none") },
                ]}
                onChange={(value) => setReasoningSummary(value as ReasoningSummaryMode)}
              />
            </SettingsRow>
            <SettingsRow
              label={i18n.t("settings.remoteCompaction")}
              description={i18n.t("settings.remoteCompaction.description")}
            >
              <Toggle
                checked={remoteCompaction()}
                label={i18n.t("settings.remoteCompaction")}
                onChange={setRemoteCompaction}
              />
            </SettingsRow>
          </Show>
          <SettingsRow
            label={i18n.t("settings.structuredOutput")}
            description={i18n.t("settings.structuredOutput.description")}
          >
            <SelectField
              label={i18n.t("settings.structuredOutput")}
              value={structuredOutputMode()}
              options={[
                { value: "auto", label: i18n.t("settings.structuredOutput.auto") },
                { value: "enabled", label: i18n.t("settings.structuredOutput.enabled") },
                { value: "disabled", label: i18n.t("settings.structuredOutput.disabled") },
              ]}
              onChange={(value) => setStructuredOutputMode(value as StructuredOutputMode)}
            />
          </SettingsRow>
          <SettingsRow
            label={i18n.t("settings.maxInputTokens")}
            description={i18n.t("settings.serverDecides")}
          >
            <NumberField
              label={i18n.t("settings.maxInputTokens")}
              value={maxInput()}
              min={0}
              max={2_000_000}
              onInput={(event) => setMaxInput(event.currentTarget.valueAsNumber || 0)}
            />
          </SettingsRow>
          <SettingsRow
            label={i18n.t("settings.maxOutputTokens")}
            description={i18n.t("settings.serverDecides")}
          >
            <NumberField
              label={i18n.t("settings.maxOutputTokens")}
              value={maxOutput()}
              min={0}
              max={200_000}
              onInput={(event) => setMaxOutput(event.currentTarget.valueAsNumber || 0)}
            />
          </SettingsRow>
          <div class="settings-card-actions">
            <Button disabled={busy()} onClick={() => void save(false)}>
              {i18n.t("common.save")}
            </Button>
            <Button variant="primary" disabled={busy()} onClick={() => void save(true)}>
              {busy() ? "…" : i18n.t("common.saveAndTest")}
            </Button>
          </div>
        </SettingsCard>
      </SettingsSection>
      <Show when={notice()}>
        {(value) => <StatusBanner tone={value().tone}>{value().text}</StatusBanner>}
      </Show>
      <Show when={testResult()}>
        {(result) => (
          <SettingsSection title={i18n.t("settings.responsePreview")}>
            <pre class="response-preview">{result().responsePreview}</pre>
            <StatusBanner
              tone={
                result().capabilityProbe.strictJsonSchema && result().capabilityProbe.outputSchema
                  ? "success"
                  : "warning"
              }
            >
              {result().capabilityProbe.strictJsonSchema && result().capabilityProbe.outputSchema
                ? i18n.t("settings.structuredOutput.available")
                : i18n.t("settings.structuredOutput.unavailable")}
            </StatusBanner>
          </SettingsSection>
        )}
      </Show>
    </div>
  );
}

function LoadedWorkbench(props: {
  bootstrap: BootstrapState;
  initialSettings: AppSettings;
  initialRoute: WorkbenchRoute;
}) {
  const motionLabEnabled = untrack(() => props.bootstrap.featureFlags.motionLab);
  const releaseFeatures = untrack(() => runtimeFeatureVisibility(props.bootstrap.featureFlags));
  const initialRoute =
    props.initialRoute === "developer/motion-lab" && !motionLabEnabled
      ? "home"
      : props.initialRoute;
  const [settings, setSettings] = createSignal(props.initialSettings);
  const [history, setHistory] = createSignal<WorkbenchRoute[]>([initialRoute]);
  const [historyIndex, setHistoryIndex] = createSignal(0);
  const [failure, setFailure] = createSignal<string>();
  const [sidebarCollapsed, setSidebarCollapsed] = createSignal(window.innerWidth <= 760);
  let narrowViewport = window.innerWidth <= 760;
  const route = () => history()[historyIndex()] ?? "home";
  let stopNavigation: (() => void) | undefined;
  let stopSettings: (() => void) | undefined;

  function navigate(next: WorkbenchRoute) {
    if (next === "developer/motion-lab" && !motionLabEnabled) {
      setFailure("Motion Library Lab is disabled in this build.");
      return;
    }
    if (next === route()) return;
    setHistory((current) => [...current.slice(0, historyIndex() + 1), next]);
    setHistoryIndex((index) => index + 1);
  }
  function back() {
    setHistoryIndex((index) => Math.max(0, index - 1));
  }
  function forward() {
    setHistoryIndex((index) => Math.min(history().length - 1, index + 1));
  }
  const handleShortcut = (event: KeyboardEvent) => {
    if (event.ctrlKey && event.key === ",") {
      event.preventDefault();
      navigate("settings/general");
    }
  };
  const handleResize = () => {
    const nextNarrowViewport = window.innerWidth <= 760;
    if (nextNarrowViewport !== narrowViewport) {
      narrowViewport = nextNarrowViewport;
      setSidebarCollapsed(nextNarrowViewport);
    }
  };
  onMount(() => {
    window.addEventListener("keydown", handleShortcut);
    window.addEventListener("resize", handleResize);
    // eslint-disable-next-line solid/reactivity -- Tauri callbacks are live event handlers.
    void listen<WorkbenchRoute>("workbench:navigate", ({ payload }) => {
      // Tauri delivers this callback after mount; the router signals remain live.
      navigate(payload);
    }).then((unlisten) => {
      stopNavigation = unlisten;
    });
    void listen<AppSettings>("settings-changed", ({ payload }) => setSettings(payload)).then(
      (unlisten) => {
        stopSettings = unlisten;
      },
    );
  });
  onCleanup(() => {
    window.removeEventListener("keydown", handleShortcut);
    window.removeEventListener("resize", handleResize);
    stopNavigation?.();
    stopSettings?.();
  });
  return (
    <AppearanceProvider
      initialAppearance={props.bootstrap.appearance}
      appearance={settings().appearance}
    >
      <I18nProvider initialLocale={props.bootstrap.locale as AppLocale}>
        <AppShell
          class="workbench-window"
          classList={{ "sidebar-collapsed": sidebarCollapsed() }}
          data-route={route()}
        >
          <WindowChrome
            canGoBack={historyIndex() > 0}
            canGoForward={historyIndex() < history().length - 1}
            motionLabEnabled={motionLabEnabled}
            onBack={back}
            onForward={forward}
            onToggleSidebar={() => setSidebarCollapsed((value) => !value)}
            onNavigate={navigate}
          />
          <Show when={failure()}>{(message) => <div class="global-error">{message()}</div>}</Show>
          <Switch>
            <Match when={settingsRoutes.includes(route())}>
              <SettingsShell
                route={route()}
                navigate={navigate}
                settings={settings()}
                setSettings={setSettings}
                fail={setFailure}
                featureFlags={props.bootstrap.featureFlags}
              />
            </Match>
            <Match when={route() === "developer/motion-lab" && motionLabEnabled}>
              <MotionLabPage />
            </Match>
            <Match when={true}>
              <HomePage
                navigate={navigate}
                motionLabEnabled={motionLabEnabled}
                runRecoveryEnabled={releaseFeatures.runRecovery}
                multiAgentEnabled={releaseFeatures.multiAgent}
                gitRemoteMutationsEnabled={releaseFeatures.gitRemoteMutations}
                workspaceToolsEnabled={props.bootstrap.featureFlags.workspaceTools}
                schedulerEnabled={props.bootstrap.featureFlags.scheduler}
              />
            </Match>
          </Switch>
        </AppShell>
      </I18nProvider>
    </AppearanceProvider>
  );
}

export function WorkbenchApp() {
  const [bootstrap, setBootstrap] = createSignal<BootstrapState>();
  const [settings, setSettings] = createSignal<AppSettings>();
  const [failure, setFailure] = createSignal<string>();
  const route = initialRoute();
  onMount(async () => {
    try {
      const [nextBootstrap, nextSettings] = await Promise.all([
        commands.getBootstrapState(),
        commands.getSettings(),
      ]);
      setBootstrap(nextBootstrap);
      setSettings(nextSettings);
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      await commands.frontendReady().catch(() => undefined);
    }
  });
  return (
    <Show
      when={bootstrap() && settings()}
      fallback={
        <main class="workbench-loading" role="status">
          {failure() ?? "Hachimi…"}
        </main>
      }
    >
      <LoadedWorkbench
        bootstrap={bootstrap()!}
        initialSettings={settings()!}
        initialRoute={route}
      />
    </Show>
  );
}
