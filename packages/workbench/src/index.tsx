import { listen } from "@tauri-apps/api/event";
import {
  commandFailure,
  commands,
  type AppSettings,
  type AppearanceConfig,
  type BootstrapState,
  type LlmSettingsInput,
  type LlmSettingsView,
  type LlmTestResult,
  type ProviderProtocolKind,
  type StructuredOutputMode,
  type Locale,
  type DiffMarkerMode,
  type ReducedMotion,
  type ThemeProfile,
  type ThemeScheme,
  type ThemeMode as ContractThemeMode,
  type UiDensity,
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
  ColorField,
  Copy,
  Dialog,
  DEFAULT_CODE_FONT,
  DEFAULT_UI_FONT,
  Dropdown,
  Globe,
  Maximize2,
  Minus,
  Monitor,
  MoreHorizontal,
  Moon,
  NumberField,
  PageHeading,
  Palette,
  PanelLeftClose,
  Play,
  Plug,
  Plus,
  Puzzle,
  RangeField,
  SearchField,
  SegmentedControl,
  SelectField,
  Sidebar,
  SettingsCard,
  Settings,
  SettingsRow,
  SettingsSection,
  StatusBanner,
  Sun,
  Switch as Toggle,
  TextField,
  ThemeCard,
  TitleBar,
  Toast,
  Trash2,
  Upload,
  Volume2,
  X,
  contrastRatio,
  isHexColor,
  selectedTheme,
  useTheme,
  type ThemeMode,
} from "@hachimi/ui";
import {
  For,
  Match,
  Show,
  Switch,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  onMount,
  untrack,
  type JSX,
} from "solid-js";

import { runtimeFeatureVisibility } from "./runtime-feature-visibility";
import "./workbench.css";
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
import { createSerializedAutosave, type AutosaveStatus } from "./appearance-save";
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
              {
                id: "motion",
                label: text("动作库实验室", "Motion Library Lab"),
                icon: <Play size={15} />,
              },
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
              else window.alert(text("Hachimi 0.3.0-alpha.8", "Hachimi 0.3.0-alpha.8"));
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

const UI_FONT_OPTIONS = [
  { value: DEFAULT_UI_FONT, label: "Inter", accent: "#1677D2" },
  {
    value: '"Segoe UI", "Microsoft YaHei UI", system-ui, sans-serif',
    label: "Segoe UI",
    accent: "#2563EB",
  },
  {
    value: '"Microsoft YaHei UI", "Microsoft YaHei", sans-serif',
    label: "Microsoft YaHei",
    accent: "#0891B2",
  },
  {
    value: '"Noto Sans SC", "Microsoft YaHei UI", sans-serif',
    label: "Noto Sans SC",
    accent: "#7C3AED",
  },
  { value: "Arial, Helvetica, sans-serif", label: "Arial", accent: "#D2694B" },
] as const;

const CODE_FONT_OPTIONS = [
  { value: DEFAULT_CODE_FONT, label: "JetBrains Mono", accent: "#7C3AED" },
  {
    value: '"Cascadia Code", "Cascadia Mono", Consolas, monospace',
    label: "Cascadia Code",
    accent: "#1677D2",
  },
  { value: "Consolas, monospace", label: "Consolas", accent: "#0891B2" },
  {
    value: '"Fira Code", "Cascadia Code", Consolas, monospace',
    label: "Fira Code",
    accent: "#D2694B",
  },
  {
    value: '"Source Code Pro", "Cascadia Code", Consolas, monospace',
    label: "Source Code Pro",
    accent: "#8DA101",
  },
  {
    value: '"IBM Plex Mono", "Cascadia Code", Consolas, monospace',
    label: "IBM Plex Mono",
    accent: "#5E6AD2",
  },
] as const;

function fontSelectOptions(
  presets: readonly { value: string; label: string; accent: string }[],
  current: string,
) {
  const options = presets.map((preset) => ({
    value: preset.value,
    label: preset.label,
    preview: {
      accent: preset.accent,
      background: "#F8F8FA",
      foreground: "#202126",
      fontFamily: preset.value,
    },
  }));
  if (options.some((option) => option.value === current)) return options;
  return [
    {
      value: current,
      label: current,
      preview: {
        accent: "#2EA8FF",
        background: "#F8F8FA",
        foreground: "#202126",
        fontFamily: current,
      },
    },
    ...options,
  ];
}

function themeProfileLabel(profile: ThemeProfile): string {
  return profile.builtin ? profile.name.replace(/ (Light|Dark|Latte|Mocha)$/, "") : profile.name;
}

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
            status: i18n.locale() === "zh-CN" ? "规划中" : "Planned",
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
            <Badge>v0.3.0-alpha.8</Badge>
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

function AppearanceSettings(props: {
  settings: AppSettings;
  setSettings: (settings: AppSettings) => void;
  fail: (message: string) => void;
}) {
  const i18n = useI18n();
  const theme = useTheme();
  const modes = createMemo(() => [
    {
      value: "system" as const,
      label: i18n.t("settings.theme.system"),
      icon: <Monitor size={17} />,
    },
    { value: "light" as const, label: i18n.t("settings.theme.light"), icon: <Sun size={17} /> },
    { value: "dark" as const, label: i18n.t("settings.theme.dark"), icon: <Moon size={17} /> },
  ]);
  const lightPreview = () => selectedTheme(props.settings.appearance, "light");
  const darkPreview = () => selectedTheme(props.settings.appearance, "dark");
  const previewStyle = () => ({
    "--preview-light-background": lightPreview().background,
    "--preview-light-foreground": lightPreview().foreground,
    "--preview-light-accent": lightPreview().accent,
    "--preview-dark-background": darkPreview().background,
    "--preview-dark-foreground": darkPreview().foreground,
    "--preview-dark-accent": darkPreview().accent,
  });
  const [toast, setToast] = createSignal<{ tone: "success" | "danger"; text: string }>();
  const [saveStatus, setSaveStatus] = createSignal<AutosaveStatus>("idle");
  const [pendingThemeAction, setPendingThemeAction] = createSignal<{
    action: "reset" | "delete";
    profile: ThemeProfile;
  }>();
  let toastTimer: ReturnType<typeof setTimeout> | undefined;
  const notify = (tone: "success" | "danger", text: string) => {
    if (toastTimer) clearTimeout(toastTimer);
    setToast({ tone, text });
    toastTimer = setTimeout(() => setToast(undefined), 2600);
  };
  const autosave = createSerializedAutosave<AppSettings>({
    initial: untrack(() => props.settings),
    save: commands.updateSettings,
    onConfirmed: (value) => props.setSettings(value),
    onRollback: (confirmed, error) => {
      props.setSettings(confirmed);
      theme.setMode(confirmed.theme as ThemeMode);
      theme.setAppearance(confirmed.appearance);
      const message = commandFailure(error).message;
      notify("danger", message);
      props.fail(message);
    },
    onStatusChange: setSaveStatus,
  });
  createEffect(() => autosave.accept(props.settings));
  onCleanup(() => {
    if (toastTimer) clearTimeout(toastTimer);
    autosave.dispose();
  });

  function persist(next: AppSettings, immediate = false) {
    autosave.schedule(next, immediate);
    props.setSettings(next);
    theme.setAppearance(next.appearance);
  }

  function choose(mode: ThemeMode) {
    theme.setMode(mode);
    persist({ ...props.settings, theme: mode as ContractThemeMode }, true);
  }

  function updateAppearance(
    transform: (appearance: AppearanceConfig) => AppearanceConfig,
    immediate = false,
  ) {
    persist({ ...props.settings, appearance: transform(props.settings.appearance) }, immediate);
  }

  function updateProfile(
    scheme: ThemeScheme,
    profileId: string,
    patch: Partial<ThemeProfile>,
    immediate = false,
  ) {
    const appearance = {
      ...props.settings.appearance,
      themes: props.settings.appearance.themes.map((profile) =>
        profile.id === profileId ? { ...profile, ...patch } : profile,
      ),
    };
    theme.setMode(scheme);
    persist(
      {
        ...props.settings,
        theme: scheme as ContractThemeMode,
        appearance,
      },
      immediate,
    );
  }

  function selectThemeProfile(scheme: ThemeScheme, id: string) {
    const appearance = {
      ...props.settings.appearance,
      ...(scheme === "light" ? { lightThemeId: id } : { darkThemeId: id }),
    };
    theme.setMode(scheme);
    persist(
      {
        ...props.settings,
        theme: scheme as ContractThemeMode,
        appearance,
      },
      true,
    );
  }

  async function runThemeCommand(action: "import" | "copy" | "reset" | "delete", value: string) {
    try {
      await autosave.flush();
      if (action === "copy") {
        await commands.copyThemeProfile(value);
        notify("success", i18n.t("settings.appearance.copied"));
        return;
      }
      const next =
        action === "import"
          ? await commands.importThemeProfile(value as ThemeScheme)
          : action === "reset"
            ? await commands.resetThemeProfile(value)
            : await commands.deleteThemeProfile(value);
      if (next) {
        props.setSettings(next);
        theme.setAppearance(next.appearance);
        autosave.accept(next);
        notify(
          "success",
          action === "import" ? i18n.t("settings.appearance.imported") : i18n.t("settings.saved"),
        );
      }
    } catch (error) {
      const message = commandFailure(error).message;
      notify("danger", message);
      props.fail(message);
    }
  }
  return (
    <div class="settings-page settings-page-demo appearance-page">
      <PageHeading
        class="settings-page-heading"
        title={i18n.t("settings.appearance")}
        description={i18n.t("settings.appearance.description")}
        badge={
          saveStatus() === "pending"
            ? i18n.t("settings.appearance.savePending")
            : saveStatus() === "saving"
              ? i18n.t("settings.appearance.saving")
              : saveStatus() === "saved"
                ? i18n.t("settings.appearance.saved")
                : saveStatus() === "error"
                  ? i18n.t("settings.appearance.saveFailed")
                  : undefined
        }
        badgeTone={
          saveStatus() === "error" ? "danger" : saveStatus() === "saved" ? "success" : "neutral"
        }
      />
      <SettingsSection title={i18n.t("settings.theme")}>
        <div class="theme-layout">
          <div class="theme-picker">
            <div class="appearance-mode-grid" role="group" aria-label={i18n.t("settings.theme")}>
              <For each={modes()}>
                {(item) => (
                  <ThemeCard
                    class="appearance-mode-card"
                    selected={theme.mode() === item.value}
                    label={item.label}
                    previewClass={[
                      "appearance-mode-preview",
                      "theme-thumbnail",
                      "appearance-mode-" + item.value,
                      "thumbnail-" + item.value,
                    ].join(" ")}
                    previewStyle={previewStyle()}
                    preview={
                      <>
                        <span />
                        <span />
                        <span />
                      </>
                    }
                    onClick={() => void choose(item.value)}
                  />
                )}
              </For>
            </div>
            <div class="theme-profile">
              <span>
                {i18n.t("settings.appearance.themeProfile")} <strong>{theme.profile().name}</strong>
              </span>
              <div class="accent-picker" aria-label={i18n.t("settings.appearance.accent")}>
                <For each={[theme.profile().accent, "#3573C8", "#307A67"]}>
                  {(accent) => (
                    <Button
                      class="accent-chip"
                      classList={{
                        selected: accent.toUpperCase() === theme.profile().accent.toUpperCase(),
                      }}
                      data-chip-color={accent}
                      type="button"
                      aria-label={accent}
                      onClick={() =>
                        updateProfile(theme.resolved(), theme.profile().id, { accent }, true)
                      }
                    />
                  )}
                </For>
              </div>
            </div>
          </div>
          <aside class="live-preview-card">
            <header class="preview-header">
              <strong>{i18n.locale() === "zh-CN" ? "实时预览" : "Live preview"}</strong>
              <span>
                {theme.resolved() === "dark"
                  ? i18n.t("settings.theme.dark")
                  : i18n.t("settings.theme.light")}
                {" · "}
                {props.settings.appearance.preferences.density === "compact"
                  ? i18n.t("settings.appearance.densityCompact")
                  : props.settings.appearance.preferences.density === "comfortable"
                    ? i18n.t("settings.appearance.densityComfortable")
                    : i18n.t("settings.appearance.densityDefault")}
              </span>
            </header>
            <div class="workbench-preview">
              <div class="preview-sidebar">
                <span class="preview-mark">H</span>
                <i />
                <i />
                <i />
              </div>
              <div class="preview-content">
                <div class="preview-copy">
                  <i />
                  <i />
                  <i />
                </div>
                <div class="preview-composer" />
              </div>
            </div>
          </aside>
        </div>
      </SettingsSection>
      <SettingsSection title={i18n.t("settings.appearance.diffPreview")}>
        <div class="diff-preview" aria-label={i18n.t("settings.appearance.diffPreview")}>
          <div
            class="diff-preview-pane light"
            data-preview-background={lightPreview().background}
            data-preview-foreground={lightPreview().foreground}
          >
            <span>src/theme.ts</span>
            <code>
              <i>-</i> color: #766cf7;
            </code>
            <code>
              <b>+</b> color: #2ea8ff;
            </code>
          </div>
          <div
            class="diff-preview-pane dark"
            data-preview-background={darkPreview().background}
            data-preview-foreground={darkPreview().foreground}
          >
            <span>src/theme.ts</span>
            <code>
              <i>-</i> color: #766cf7;
            </code>
            <code>
              <b>+</b> color: #2ea8ff;
            </code>
          </div>
        </div>
      </SettingsSection>
      <For each={["light", "dark"] as const}>
        {(scheme) => {
          const selectedId = () =>
            scheme === "light"
              ? props.settings.appearance.lightThemeId
              : props.settings.appearance.darkThemeId;
          const profile = () =>
            props.settings.appearance.themes.find((item) => item.id === selectedId())!;
          return (
            <ThemeProfileEditor
              scheme={scheme}
              appearance={props.settings.appearance}
              profile={profile()}
              onSelect={(id) => selectThemeProfile(scheme, id)}
              onUpdate={(patch, immediate) => updateProfile(scheme, profile().id, patch, immediate)}
              onImport={() => void runThemeCommand("import", scheme)}
              onCopy={() => void runThemeCommand("copy", profile().id)}
              onReset={() => setPendingThemeAction({ action: "reset", profile: profile() })}
              onDelete={() => setPendingThemeAction({ action: "delete", profile: profile() })}
            />
          );
        }}
      </For>
      <SettingsSection title={i18n.t("settings.appearance.preferences")}>
        <SettingsCard class="settings-card appearance-preferences">
          <SettingsRow label={i18n.t("settings.appearance.pointer")}>
            <Toggle
              checked={props.settings.appearance.preferences.pointerCursor}
              label={i18n.t("settings.appearance.pointer")}
              onChange={(pointerCursor) =>
                updateAppearance(
                  (appearance) => ({
                    ...appearance,
                    preferences: { ...appearance.preferences, pointerCursor },
                  }),
                  true,
                )
              }
            />
          </SettingsRow>
          <SettingsRow label={i18n.t("settings.appearance.motion")}>
            <SelectField
              label={i18n.t("settings.appearance.motion")}
              value={props.settings.appearance.preferences.reducedMotion}
              options={[
                { value: "system", label: i18n.t("settings.theme.system") },
                { value: "on", label: i18n.t("common.enabled") },
                { value: "off", label: i18n.t("common.disabled") },
              ]}
              onChange={(reducedMotion) =>
                updateAppearance(
                  (appearance) => ({
                    ...appearance,
                    preferences: {
                      ...appearance.preferences,
                      reducedMotion: reducedMotion as ReducedMotion,
                    },
                  }),
                  true,
                )
              }
            />
          </SettingsRow>
          <SettingsRow label={i18n.t("settings.appearance.density")}>
            <SegmentedControl<UiDensity>
              label={i18n.t("settings.appearance.density")}
              value={props.settings.appearance.preferences.density ?? "default"}
              options={[
                {
                  value: "compact",
                  label: i18n.t("settings.appearance.densityCompact"),
                },
                {
                  value: "default",
                  label: i18n.t("settings.appearance.densityDefault"),
                },
                {
                  value: "comfortable",
                  label: i18n.t("settings.appearance.densityComfortable"),
                },
              ]}
              onChange={(density) =>
                updateAppearance(
                  (appearance) => ({
                    ...appearance,
                    preferences: { ...appearance.preferences, density },
                  }),
                  true,
                )
              }
            />
          </SettingsRow>
          <SettingsRow label={i18n.t("settings.appearance.uiSize")}>
            <RangeField
              label={i18n.t("settings.appearance.uiSize")}
              value={props.settings.appearance.preferences.uiFontSize}
              min={12}
              max={20}
              unit="px"
              onInput={(uiFontSize) =>
                updateAppearance((appearance) => ({
                  ...appearance,
                  preferences: { ...appearance.preferences, uiFontSize },
                }))
              }
              onCommit={(uiFontSize) =>
                updateAppearance(
                  (appearance) => ({
                    ...appearance,
                    preferences: { ...appearance.preferences, uiFontSize },
                  }),
                  true,
                )
              }
            />
          </SettingsRow>
          <SettingsRow label={i18n.t("settings.appearance.codeSize")}>
            <RangeField
              label={i18n.t("settings.appearance.codeSize")}
              value={props.settings.appearance.preferences.codeFontSize}
              min={10}
              max={20}
              unit="px"
              onInput={(codeFontSize) =>
                updateAppearance((appearance) => ({
                  ...appearance,
                  preferences: { ...appearance.preferences, codeFontSize },
                }))
              }
              onCommit={(codeFontSize) =>
                updateAppearance(
                  (appearance) => ({
                    ...appearance,
                    preferences: { ...appearance.preferences, codeFontSize },
                  }),
                  true,
                )
              }
            />
          </SettingsRow>
          <SettingsRow label={i18n.t("settings.appearance.diffMarkers")}>
            <SegmentedControl<DiffMarkerMode>
              label={i18n.t("settings.appearance.diffMarkers")}
              value={props.settings.appearance.preferences.diffMarkers}
              options={[
                { value: "color", label: i18n.t("settings.appearance.diffColor") },
                { value: "signs", label: i18n.t("settings.appearance.diffSigns") },
              ]}
              onChange={(diffMarkers) =>
                updateAppearance(
                  (appearance) => ({
                    ...appearance,
                    preferences: { ...appearance.preferences, diffMarkers },
                  }),
                  true,
                )
              }
            />
          </SettingsRow>
        </SettingsCard>
      </SettingsSection>
      <Toast open={Boolean(toast())} tone={toast()?.tone} onClose={() => setToast(undefined)}>
        {toast()?.text}
      </Toast>
      <Dialog
        open={Boolean(pendingThemeAction())}
        title={
          pendingThemeAction()?.action === "delete"
            ? i18n.t("settings.appearance.deleteTitle")
            : i18n.t("settings.appearance.resetTitle")
        }
        description={i18n
          .t(
            pendingThemeAction()?.action === "delete"
              ? "settings.appearance.deleteConfirm"
              : "settings.appearance.resetConfirm",
          )
          .replace("{name}", pendingThemeAction()?.profile.name ?? "")}
        onOpenChange={(open) => {
          if (!open) setPendingThemeAction(undefined);
        }}
      >
        <div class="dialog-actions">
          <Button variant="ghost" onClick={() => setPendingThemeAction(undefined)}>
            {i18n.t("common.cancel")}
          </Button>
          <Button
            variant={pendingThemeAction()?.action === "delete" ? "danger" : "primary"}
            onClick={() => {
              const pending = pendingThemeAction();
              if (!pending) return;
              setPendingThemeAction(undefined);
              void runThemeCommand(pending.action, pending.profile.id);
            }}
          >
            {i18n.t("common.confirm")}
          </Button>
        </div>
      </Dialog>
    </div>
  );
}

function ThemeProfileEditor(props: {
  scheme: ThemeScheme;
  appearance: AppearanceConfig;
  profile: ThemeProfile;
  onSelect: (id: string) => void;
  onUpdate: (patch: Partial<ThemeProfile>, immediate?: boolean) => void;
  onImport: () => void;
  onCopy: () => void;
  onReset: () => void;
  onDelete: () => void;
}) {
  const i18n = useI18n();
  const [colorError, setColorError] = createSignal<"accent" | "background" | "foreground">();
  const warning = () => contrastRatio(props.profile.background, props.profile.foreground) < 4.5;
  const setColor = (key: "accent" | "background" | "foreground", value: string) => {
    const normalized = value.toUpperCase();
    if (!isHexColor(normalized)) {
      setColorError(key);
      return;
    }
    setColorError(undefined);
    props.onUpdate({ [key]: normalized });
  };
  return (
    <SettingsSection
      title={
        props.scheme === "light"
          ? i18n.t("settings.appearance.lightTheme")
          : i18n.t("settings.appearance.darkTheme")
      }
    >
      <SettingsCard class="theme-profile-card theme-picker">
        <div class="theme-profile-toolbar">
          <SelectField
            label={i18n.t("settings.appearance.themeProfile")}
            value={props.profile.id}
            options={props.appearance.themes
              .filter((profile) => profile.scheme === props.scheme)
              .map((profile) => ({
                value: profile.id,
                label: themeProfileLabel(profile),
                preview: {
                  accent: profile.accent,
                  background: profile.background,
                  foreground: profile.foreground,
                  fontFamily: profile.uiFont,
                },
              }))}
            onChange={props.onSelect}
          />
          <div class="theme-profile-actions">
            <Button size="small" onClick={props.onImport}>
              <Upload size={14} /> {i18n.t("common.import")}
            </Button>
            <Button size="small" onClick={props.onCopy}>
              <Copy size={14} /> {i18n.t("settings.appearance.copy")}
            </Button>
            <Dropdown
              label={i18n.t("settings.appearance.themeMenu")}
              actions={[
                {
                  id: "reset",
                  label: i18n.t("settings.appearance.reset"),
                  disabled: !props.profile.builtin,
                },
                {
                  id: "delete",
                  label: i18n.t("common.delete"),
                  danger: true,
                  disabled: props.profile.builtin,
                  separatorBefore: true,
                },
              ]}
              onSelect={(action) => (action === "reset" ? props.onReset() : props.onDelete())}
            >
              <MoreHorizontal size={17} />
            </Dropdown>
          </div>
        </div>
        <div class="appearance-color-rows">
          <SettingsRow label={i18n.t("settings.appearance.accent")}>
            <ColorField
              label={i18n.t("settings.appearance.accent")}
              value={props.profile.accent}
              error={
                colorError() === "accent" ? i18n.t("settings.appearance.invalidColor") : undefined
              }
              onInput={(value) => setColor("accent", value)}
            />
          </SettingsRow>
          <SettingsRow label={i18n.t("settings.appearance.background")}>
            <ColorField
              label={i18n.t("settings.appearance.background")}
              value={props.profile.background}
              error={
                colorError() === "background"
                  ? i18n.t("settings.appearance.invalidColor")
                  : undefined
              }
              onInput={(value) => setColor("background", value)}
            />
          </SettingsRow>
          <SettingsRow label={i18n.t("settings.appearance.foreground")}>
            <ColorField
              label={i18n.t("settings.appearance.foreground")}
              value={props.profile.foreground}
              error={
                colorError() === "foreground"
                  ? i18n.t("settings.appearance.invalidColor")
                  : undefined
              }
              onInput={(value) => setColor("foreground", value)}
            />
          </SettingsRow>
        </div>
        <Show when={warning()}>
          <div class="contrast-warning" role="status">
            <AlertTriangle size={16} /> {i18n.t("settings.appearance.contrastWarning")}
          </div>
        </Show>
        <SettingsRow label={i18n.t("settings.appearance.uiFont")}>
          <SelectField
            label={i18n.t("settings.appearance.uiFont")}
            value={props.profile.uiFont}
            options={fontSelectOptions(UI_FONT_OPTIONS, props.profile.uiFont)}
            onChange={(uiFont) => props.onUpdate({ uiFont }, true)}
          />
        </SettingsRow>
        <SettingsRow label={i18n.t("settings.appearance.codeFont")}>
          <SelectField
            label={i18n.t("settings.appearance.codeFont")}
            value={props.profile.codeFont}
            options={fontSelectOptions(CODE_FONT_OPTIONS, props.profile.codeFont)}
            onChange={(codeFont) => props.onUpdate({ codeFont }, true)}
          />
        </SettingsRow>
        <SettingsRow label={i18n.t("settings.appearance.translucentSidebar")}>
          <Toggle
            checked={props.profile.translucentSidebar}
            label={i18n.t("settings.appearance.translucentSidebar")}
            onChange={(translucentSidebar) => props.onUpdate({ translucentSidebar }, true)}
          />
        </SettingsRow>
        <SettingsRow label={i18n.t("settings.appearance.contrast")}>
          <RangeField
            label={i18n.t("settings.appearance.contrast")}
            value={props.profile.contrast}
            min={0}
            max={100}
            unit="%"
            onInput={(contrast) => props.onUpdate({ contrast })}
            onCommit={(contrast) => props.onUpdate({ contrast }, true)}
          />
        </SettingsRow>
      </SettingsCard>
    </SettingsSection>
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
  const [embeddingModelName, setEmbeddingModelName] = createSignal("");
  const [reasoningSummary, setReasoningSummary] = createSignal(false);
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
    setEmbeddingModelName(props.providerExtensionsEnabled ? next.embeddingModelName : "");
    setReasoningSummary(
      props.providerExtensionsEnabled && props.providerRemoteContextEnabled
        ? next.reasoningSummary
        : false,
    );
    setRemoteCompaction(
      props.providerExtensionsEnabled && props.providerRemoteContextEnabled
        ? next.remoteCompaction
        : false,
    );
    setMaxInput(next.maxInputTokens);
    setMaxOutput(next.maxOutputTokens);
    setStructuredOutputMode(next.structuredOutputMode);
    setApiKey("");
    setClearKey(false);
  }
  function input(): LlmSettingsInput {
    return {
      baseUrl: baseUrl(),
      modelName: modelName(),
      protocol: props.providerExtensionsEnabled ? protocol() : "chat_completions",
      compatibilityProfileId: compatibilityProfileId(),
      providerEndpointId: view()?.providerEndpointId ?? null,
      providerAccountId: view()?.providerAccountId ?? null,
      embeddingModelName: props.providerExtensionsEnabled ? embeddingModelName() : "",
      reasoningSummary: props.providerRemoteContextEnabled ? reasoningSummary() : false,
      remoteCompaction: props.providerRemoteContextEnabled ? remoteCompaction() : false,
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
                  setReasoningSummary(false);
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
              placeholder="gemma4:e4b"
              onInput={(event) => setModelName(event.currentTarget.value)}
            />
          </SettingsRow>
          <Show when={props.providerExtensionsEnabled}>
            <SettingsRow
              label={i18n.t("settings.embeddingModel")}
              description={i18n.t("settings.embeddingModel.description")}
            >
              <TextField
                label={i18n.t("settings.embeddingModel")}
                value={embeddingModelName()}
                placeholder="text-embedding-3-small"
                onInput={(event) => setEmbeddingModelName(event.currentTarget.value)}
              />
            </SettingsRow>
          </Show>
          <Show when={protocol() === "responses" && props.providerRemoteContextEnabled}>
            <SettingsRow
              label={i18n.t("settings.reasoningSummary")}
              description={i18n.t("settings.reasoningSummary.description")}
            >
              <Toggle
                checked={reasoningSummary()}
                label={i18n.t("settings.reasoningSummary")}
                onChange={setReasoningSummary}
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
  onMount(() => {
    window.addEventListener("keydown", handleShortcut);
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
    stopNavigation?.();
    stopSettings?.();
  });
  return (
    <AppearanceProvider
      initialMode={props.bootstrap.theme as ThemeMode}
      initialAppearance={props.bootstrap.appearance}
      mode={settings().theme as ThemeMode}
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
