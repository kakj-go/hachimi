import { listen } from "@tauri-apps/api/event";
import {
  commandFailure,
  commands,
  type AppSettings,
  type AppearanceConfig,
  type AvatarAssessment,
  type AvatarCatalogSnapshot,
  type AvatarCapability,
  type AvatarEntry,
  type AvatarFormat,
  type AvatarImportInspection,
  type BootstrapState,
  type LlmSettingsInput,
  type LlmSettingsView,
  type LlmTestResult,
  type Locale,
  type DiffMarkerMode,
  type ReducedMotion,
  type SpeechRecognitionRuntimeState,
  type ThemeProfile,
  type ThemeScheme,
  type ThemeMode as ContractThemeMode,
  type VoiceCatalogSnapshot,
  type VoiceComputeMode,
  type VoiceModelEntry,
  type VoiceModelInspection,
  type VoiceRuntimeState,
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
  ChevronDown,
  ColorField,
  Copy,
  Dialog,
  DEFAULT_CODE_FONT,
  DEFAULT_UI_FONT,
  Dropdown,
  FolderOpen,
  GitBranch,
  Maximize2,
  MessageCircle,
  Minus,
  Monitor,
  MoreHorizontal,
  Moon,
  NumberField,
  Palette,
  PanelLeftClose,
  Play,
  Plus,
  PromptCard,
  RangeField,
  ResourceCard,
  ResourceList,
  SearchField,
  Search,
  SegmentedControl,
  SelectField,
  Send,
  Settings,
  ShieldCheck,
  SettingsRow,
  SettingsSection,
  StatusBanner,
  Square,
  Sun,
  Switch as Toggle,
  TerminalSquare,
  TextField,
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
} from "solid-js";
import "./workbench.css";
import { createSerializedAutosave, type AutosaveStatus } from "./appearance-save";
import { MotionLabPage } from "./motion-lab";
import { MotionSettingsPage } from "./motion-settings";
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
}) {
  const i18n = useI18n();
  const menuKeys = [
    "workbench.menu.file",
    "workbench.menu.edit",
    "workbench.menu.view",
    "workbench.menu.help",
  ] as const;
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
        <button
          type="button"
          class="title-sidebar-toggle"
          aria-label="Sidebar"
          onClick={() => props.onToggleSidebar()}
        >
          <PanelLeftClose size={16} />
        </button>
        <div class="title-history">
          <button
            type="button"
            aria-label={i18n.t("workbench.back")}
            disabled={!props.canGoBack}
            onClick={() => props.onBack()}
          >
            <ArrowLeft size={15} />
          </button>
          <button
            type="button"
            aria-label={i18n.t("workbench.forward")}
            disabled={!props.canGoForward}
            onClick={() => props.onForward()}
          >
            <ArrowRight size={15} />
          </button>
        </div>
        <div class="title-menus" aria-label={i18n.t("workbench.appMenu")}>
          <For each={menuKeys}>
            {(key) => (
              <button type="button" disabled title={i18n.t("workbench.menuDisabled")}>
                {i18n.t(key)}
              </button>
            )}
          </For>
        </div>
        <div class="window-controls">
          <button
            type="button"
            aria-label={i18n.t("workbench.window.minimize")}
            onClick={() => void commands.minimizeWorkbench()}
          >
            <Minus size={16} />
          </button>
          <button
            type="button"
            aria-label={i18n.t("workbench.window.maximize")}
            onClick={() => void commands.toggleMaximizeWorkbench()}
          >
            <Maximize2 size={14} />
          </button>
          <button
            type="button"
            class="window-close"
            aria-label={i18n.t("workbench.window.close")}
            onClick={() => void commands.hideWorkbench()}
          >
            <X size={16} />
          </button>
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

function ProjectSidebar(props: {
  openSettings: () => void;
  openMotionLab: () => void;
  motionLabEnabled: boolean;
  onNewTask: () => void;
}) {
  const i18n = useI18n();
  const [search, setSearch] = createSignal("");
  const [searchOpen, setSearchOpen] = createSignal(false);
  const [expanded, setExpanded] = createSignal(true);
  const [selected, setSelected] = createSignal("session-1");
  const sessions = createMemo(() =>
    [
      { id: "session-1", label: i18n.t("workbench.sample.session1") },
      { id: "session-2", label: i18n.t("workbench.sample.session2") },
      { id: "session-3", label: i18n.t("workbench.sample.session3") },
    ].filter((session) =>
      session.label.toLocaleLowerCase().includes(search().trim().toLocaleLowerCase()),
    ),
  );
  return (
    <aside class="project-sidebar">
      <div class="project-sidebar-brand">
        <span class="hachimi-mini-mark">H</span>
        <strong>Hachimi</strong>
        <button
          type="button"
          aria-label={i18n.t("settings.search")}
          aria-expanded={searchOpen()}
          onClick={() => setSearchOpen((value) => !value)}
        >
          <Search size={17} />
        </button>
      </div>
      <Show when={searchOpen()}>
        <SearchField
          label={i18n.t("settings.search")}
          placeholder={i18n.t("settings.search")}
          value={search()}
          onInput={(event) => setSearch(event.currentTarget.value)}
        />
      </Show>
      <nav class="project-quick-nav" aria-label={i18n.t("workbench.home")}>
        <button type="button" class="active" onClick={() => props.onNewTask()}>
          <Plus size={17} /> <span>{i18n.t("workbench.newTask")}</span>
        </button>
        <Show when={props.motionLabEnabled}>
          <button type="button" onClick={() => props.openMotionLab()}>
            <Play size={17} />{" "}
            <span>{i18n.locale() === "zh-CN" ? "动作库实验室" : "Motion Library Lab"}</span>
          </button>
        </Show>
      </nav>
      <div class="project-sidebar-scroll">
        <section class="project-list-section">
          <h2>{i18n.t("workbench.projects")}</h2>
          <button
            type="button"
            class="project-row selected"
            aria-expanded={expanded()}
            onClick={() => setExpanded((value) => !value)}
          >
            <span>
              <FolderOpen size={16} /> hachimi-code
            </span>
            <ChevronDown size={15} classList={{ collapsed: !expanded() }} />
          </button>
          <Show when={expanded()}>
            <div class="project-sessions">
              <For each={sessions()}>
                {(session) => (
                  <button
                    type="button"
                    classList={{ selected: selected() === session.id }}
                    onClick={() => setSelected(session.id)}
                  >
                    <MessageCircle size={14} />
                    <span>{session.label}</span>
                  </button>
                )}
              </For>
            </div>
          </Show>
        </section>
      </div>
      <button
        type="button"
        class="sidebar-account"
        aria-label={i18n.t("settings.title")}
        onClick={() => props.openSettings()}
      >
        <span class="account-avatar">M</span>
        <span>
          <strong>my_codex</strong>
          <small>{i18n.t("settings.title")}</small>
        </span>
        <Settings size={17} />
      </button>
    </aside>
  );
}

function HomePage(props: {
  navigate: (route: WorkbenchRoute) => void;
  settings: AppSettings;
  motionLabEnabled: boolean;
}) {
  const i18n = useI18n();
  const [draft, setDraft] = createSignal("");
  const cards = createMemo(() => [
    {
      icon: <Bot size={20} />,
      title: i18n.t("workbench.guide.llm"),
      description: i18n.t("workbench.guide.llmDescription"),
      route: "settings/llm" as const,
    },
    {
      icon: <Box size={20} />,
      title: i18n.t("workbench.guide.avatar"),
      description: i18n.t("workbench.guide.avatarDescription"),
      route: "settings/avatar" as const,
    },
    {
      icon: <Volume2 size={20} />,
      title: i18n.t("workbench.guide.voice"),
      description: i18n.t("workbench.guide.voiceDescription"),
      route: "settings/voice" as const,
    },
    {
      icon: <Palette size={20} />,
      title: i18n.t("workbench.guide.general"),
      description: i18n.t("workbench.guide.generalDescription"),
      route: "settings/general" as const,
    },
  ]);
  return (
    <div class="home-layout">
      <ProjectSidebar
        openSettings={() => props.navigate("settings/general")}
        openMotionLab={() => props.navigate("developer/motion-lab")}
        motionLabEnabled={props.motionLabEnabled}
        onNewTask={() => setDraft("")}
      />
      <main class="home-main">
        <div class="home-layout-actions">
          <button
            type="button"
            disabled
            aria-label="Layout"
            title={i18n.t("workbench.menuDisabled")}
          >
            <PanelLeftClose size={17} />
          </button>
        </div>
        <div class="welcome-block">
          <div class="welcome-mark">
            <span>H</span>
          </div>
          <h1>{i18n.t("workbench.buildPrompt")}</h1>
          <div class="guide-cards">
            <For each={cards()}>
              {(card) => (
                <PromptCard onClick={() => props.navigate(card.route)}>
                  <span>{card.icon}</span>
                  <strong>{card.title}</strong>
                  <small>{card.description}</small>
                </PromptCard>
              )}
            </For>
          </div>
        </div>
        <div class="composer-wrap">
          <div class="composer-context">
            <button type="button">
              <FolderOpen size={16} /> hachimi-code
            </button>
            <button type="button" disabled>
              <TerminalSquare size={16} /> {i18n.t("workbench.localEnvironment")}
            </button>
            <button type="button" disabled>
              <GitBranch size={16} /> main
            </button>
          </div>
          <div class="composer">
            <textarea
              aria-label={i18n.t("workbench.draft")}
              placeholder={i18n.t("workbench.draft")}
              value={draft()}
              onInput={(event) => setDraft(event.currentTarget.value)}
            />
            <div class="composer-footer">
              <div class="composer-options">
                <button type="button" disabled>
                  <ShieldCheck size={16} /> {i18n.t("workbench.permission")}
                </button>
                <button type="button" disabled>
                  <Bot size={16} /> {props.settings.llm.modelName}
                </button>
                <button type="button" disabled>
                  {i18n.t("settings.theme.system")}
                </button>
              </div>
              <button
                class="composer-send"
                type="button"
                disabled
                title={i18n.t("workbench.submitUnavailable")}
              >
                <Send size={16} />
              </button>
            </div>
            <p class="composer-capability-note">{i18n.t("workbench.submitUnavailable")}</p>
          </div>
        </div>
      </main>
    </div>
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
}) {
  const i18n = useI18n();
  const [search, setSearch] = createSignal("");
  const groups = createMemo(() => {
    const all = [
      {
        label: i18n.t("settings.group.personal"),
        items: [
          ["settings/general", i18n.t("settings.general"), <Settings size={17} />],
          ["settings/appearance", i18n.t("settings.appearance"), <Palette size={17} />],
          ["settings/voice", i18n.t("settings.voice"), <Volume2 size={17} />],
          ["settings/llm", i18n.t("settings.configuration"), <Bot size={17} />],
          ["settings/avatar", i18n.t("settings.pet"), <Box size={17} />],
          [
            "settings/motion",
            i18n.locale() === "zh-CN" ? "交互" : "Interactions",
            <Play size={17} />,
          ],
        ],
      },
    ] as const;
    const query = search().trim().toLocaleLowerCase();
    return all
      .map((group) => ({
        ...group,
        items: group.items.filter((item) => item[1].toLocaleLowerCase().includes(query)),
      }))
      .filter((group) => group.items.length > 0);
  });
  return (
    <div class="settings-layout">
      <aside class="settings-sidebar">
        <div class="settings-sidebar-brand">
          <button
            type="button"
            class="back-home"
            aria-label={i18n.t("settings.backHome")}
            onClick={() => props.navigate("home")}
          >
            <ArrowLeft size={16} />
          </button>
          <strong>Hachimi</strong>
        </div>
        <SearchField
          label={i18n.t("settings.search")}
          placeholder={i18n.t("settings.search")}
          value={search()}
          onInput={(event) => setSearch(event.currentTarget.value)}
        />
        <nav class="settings-nav" aria-label={i18n.t("settings.title")}>
          <For each={groups()}>
            {(group) => (
              <section class="settings-nav-group">
                <h2>{group.label}</h2>
                <For each={group.items}>
                  {(item) => (
                    <button
                      type="button"
                      classList={{ selected: props.route === item[0] }}
                      aria-current={props.route === item[0] ? "page" : undefined}
                      onClick={() => props.navigate(item[0])}
                    >
                      {item[2]}
                      <span>{item[1]}</span>
                    </button>
                  )}
                </For>
              </section>
            )}
          </For>
        </nav>
      </aside>
      <main class="settings-main">
        <div class="settings-workspace-topline" aria-hidden="true" />
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
              <LlmSettingsPage />
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
          </Switch>
        </div>
      </main>
    </div>
  );
}

function PageHeading(props: {
  title: string;
  description: string;
  badge?: string | undefined;
  badgeTone?: "neutral" | "success" | "warning" | "danger" | undefined;
}) {
  return (
    <header class="page-heading">
      <div>
        <h1>{props.title}</h1>
        <p>{props.description}</p>
      </div>
      <Show when={props.badge}>
        <Badge tone={props.badgeTone ?? "neutral"}>{props.badge}</Badge>
      </Show>
    </header>
  );
}

function GeneralSettings(props: {
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
  async function changeAlwaysOnTop(enabled: boolean) {
    try {
      props.setSettings(await commands.setAlwaysOnTop(enabled));
    } catch (error) {
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
    <div class="settings-page">
      <PageHeading
        title={i18n.t("settings.general")}
        description={i18n.t("settings.general.description")}
      />
      <SettingsSection title={i18n.t("settings.general")}>
        <div class="settings-card">
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
          <SettingsRow
            label={i18n.locale() === "zh-CN" ? "开发者模式" : "Developer mode"}
            description={
              i18n.locale() === "zh-CN"
                ? "Release 构建重启后显示程序实验室；Debug 构建始终可用。"
                : "Shows Motion Library Lab after restarting a Release build; Debug builds always enable it."
            }
          >
            <Toggle
              checked={props.settings.developerMode ?? false}
              label={i18n.locale() === "zh-CN" ? "开发者模式" : "Developer mode"}
              onChange={(enabled) => void persist({ developerMode: enabled })}
            />
          </SettingsRow>
        </div>
      </SettingsSection>
      <SettingsSection title={i18n.t("settings.securityBoundary")}>
        <StatusBanner tone="neutral">{i18n.t("settings.securityDescription")}</StatusBanner>
      </SettingsSection>
      <SettingsSection title={i18n.t("settings.data.title")}>
        <div class="settings-card">
          <SettingsRow
            label={i18n.t("settings.data.reset")}
            description={i18n.t("settings.data.description")}
          >
            <Button variant="danger" onClick={() => setResetOpen(true)}>
              <Trash2 size={15} />
              {i18n.t("settings.data.reset")}
            </Button>
          </SettingsRow>
        </div>
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
    <div class="settings-page appearance-page">
      <PageHeading
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
        <div class="appearance-mode-grid">
          <For each={modes()}>
            {(item) => (
              <button
                type="button"
                class="appearance-mode-card"
                classList={{ selected: theme.mode() === item.value }}
                aria-pressed={theme.mode() === item.value}
                onClick={() => void choose(item.value)}
              >
                <span
                  class={`appearance-mode-preview appearance-mode-${item.value}`}
                  style={previewStyle()}
                >
                  <i>{item.icon}</i>
                  <span />
                  <span />
                  <span />
                </span>
                <strong>{item.label}</strong>
              </button>
            )}
          </For>
        </div>
      </SettingsSection>
      <SettingsSection title={i18n.t("settings.appearance.diffPreview")}>
        <div class="diff-preview" aria-label={i18n.t("settings.appearance.diffPreview")}>
          <div
            class="diff-preview-pane light"
            style={{
              "--preview-background": lightPreview().background,
              "--preview-foreground": lightPreview().foreground,
            }}
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
            style={{
              "--preview-background": darkPreview().background,
              "--preview-foreground": darkPreview().foreground,
            }}
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
        <div class="settings-card appearance-preferences">
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
        </div>
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
      <div class="theme-profile-card">
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
      </div>
    </SettingsSection>
  );
}

function LlmSettingsPage() {
  const i18n = useI18n();
  const [view, setView] = createSignal<LlmSettingsView>();
  const [baseUrl, setBaseUrl] = createSignal("");
  const [modelName, setModelName] = createSignal("");
  const [maxInput, setMaxInput] = createSignal(0);
  const [maxOutput, setMaxOutput] = createSignal(0);
  const [apiKey, setApiKey] = createSignal("");
  const [clearKey, setClearKey] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [notice, setNotice] = createSignal<{ tone: "success" | "danger"; text: string }>();
  const [testResult, setTestResult] = createSignal<LlmTestResult>();

  function useView(next: LlmSettingsView) {
    setView(next);
    setBaseUrl(next.baseUrl);
    setModelName(next.modelName);
    setMaxInput(next.maxInputTokens);
    setMaxOutput(next.maxOutputTokens);
    setApiKey("");
    setClearKey(false);
  }
  function input(): LlmSettingsInput {
    return {
      baseUrl: baseUrl(),
      modelName: modelName(),
      maxInputTokens: maxInput(),
      maxOutputTokens: maxOutput(),
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
    <div class="settings-page">
      <PageHeading
        title={i18n.t("settings.llm")}
        description={i18n.t("settings.llm.description")}
        badge="OpenAI-compatible"
      />
      <SettingsSection title={i18n.t("settings.connection")}>
        <div class="settings-card unified-settings-card">
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
        </div>
      </SettingsSection>
      <Show when={notice()}>
        {(value) => <StatusBanner tone={value().tone}>{value().text}</StatusBanner>}
      </Show>
      <Show when={testResult()}>
        {(result) => (
          <SettingsSection title={i18n.t("settings.responsePreview")}>
            <pre class="response-preview">{result().responsePreview}</pre>
          </SettingsSection>
        )}
      </Show>
    </div>
  );
}

type CatalogEntry = AvatarEntry;
type CatalogSnapshot = AvatarCatalogSnapshot;

function ResourceSettingsPage() {
  const i18n = useI18n();
  const [snapshot, setSnapshot] = createSignal<CatalogSnapshot>({ entries: [], currentId: null });
  const [name, setName] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [notice, setNotice] = createSignal<{ tone: "success" | "danger"; text: string }>();
  const [pendingDelete, setPendingDelete] = createSignal<CatalogEntry>();
  const [avatarInspection, setAvatarInspection] = createSignal<AvatarImportInspection>();
  const isAvatar = () => true;
  const entries = () => snapshot().entries as CatalogEntry[];
  async function load() {
    setSnapshot(await commands.listAvatarModels());
  }
  async function importResource() {
    if (!name().trim()) {
      setNotice({ tone: "danger", text: i18n.t("settings.resource.invalidName") });
      return;
    }
    setBusy(true);
    setNotice(undefined);
    try {
      const inspection = await commands.inspectAvatarModel();
      if (inspection) {
        setAvatarInspection(inspection);
      } else {
        setNotice({ tone: "success", text: i18n.t("settings.resource.cancelled") });
      }
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    } finally {
      setBusy(false);
    }
  }
  async function closeAvatarInspection() {
    const token = avatarInspection()?.token;
    setAvatarInspection(undefined);
    if (token) await commands.cancelAvatarModelImport(token).catch(() => undefined);
  }
  async function confirmAvatarImport() {
    const inspection = avatarInspection();
    if (!inspection?.token) return;
    setBusy(true);
    try {
      const next = await commands.commitAvatarModelImport({
        token: inspection.token,
        name: name().trim(),
      });
      setSnapshot(next);
      setAvatarInspection(undefined);
      setName("");
      setNotice({ tone: "success", text: i18n.t("settings.resource.imported") });
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    } finally {
      setBusy(false);
    }
  }
  async function selectResource(id: string) {
    try {
      setSnapshot(await commands.selectAvatarModel(id));
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    }
  }
  async function deleteResource(entry: CatalogEntry) {
    setPendingDelete(entry);
  }
  async function confirmDelete() {
    const entry = pendingDelete();
    if (!entry) return;
    try {
      setSnapshot(await commands.deleteAvatarModel(entry.id));
      setPendingDelete(undefined);
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    }
  }
  onMount(() => {
    void load().catch((error) =>
      setNotice({ tone: "danger", text: commandFailure(error).message }),
    );
  });
  return (
    <div class="settings-page">
      <PageHeading
        title={i18n.t("settings.avatar")}
        description={i18n.t("settings.avatar.description")}
        badge="VRM 0.x / 1.0 · Runtime Ready · ≤ 200MB"
      />
      <Show when={isAvatar()}>
        <SettingsSection title={i18n.t("settings.avatar.importTitle")}>
          <StatusBanner>{i18n.t("settings.avatar.sketchfabHint")}</StatusBanner>
          <div class="settings-card unified-settings-card resource-import-card">
            <SettingsRow
              label={i18n.t("settings.resourceName")}
              description={i18n.t("settings.resource.sharedBlob")}
            >
              <TextField
                label={i18n.t("settings.resourceName")}
                value={name()}
                placeholder={i18n.t("settings.avatar.nameExample")}
                onInput={(event) => setName(event.currentTarget.value)}
              />
            </SettingsRow>
            <div class="settings-card-actions">
              <Button variant="primary" disabled={busy()} onClick={() => void importResource()}>
                {i18n.t("settings.avatar.import")}
              </Button>
            </div>
          </div>
        </SettingsSection>
      </Show>
      <Show when={notice()}>
        {(value) => <StatusBanner tone={value().tone}>{value().text}</StatusBanner>}
      </Show>
      <Show when={isAvatar()}>
        <SettingsSection title={`${i18n.t("settings.catalog")} · ${entries().length}`}>
          <Show
            when={entries().length > 0}
            fallback={<div class="empty-resource">{i18n.t("common.noResources")}</div>}
          >
            <ResourceList label={i18n.t("settings.avatar")}>
              <For each={entries()}>
                {(entry) => (
                  <ResourceCard
                    title={entry.name}
                    subtitle={`${entry.originalFileName} · ${formatBytes(entry.sizeBytes)}`}
                    current={entry.isCurrent}
                    tone={
                      resolvedAvatarAssessment(entry).compatibility !== "runtime_ready"
                        ? "danger"
                        : "default"
                    }
                    meta={
                      <div class="avatar-resource-meta">
                        <div class="avatar-badge-row">
                          <Badge>{avatarFormatLabel(entry.format ?? "glb")}</Badge>
                          <Badge
                            tone={avatarCompatibilityTone(
                              resolvedAvatarAssessment(entry).compatibility,
                            )}
                          >
                            {avatarCompatibilityLabel(
                              resolvedAvatarAssessment(entry).compatibility,
                              i18n.locale(),
                            )}
                          </Badge>
                          <Show when={resolvedAvatarAssessment(entry).issues.length > 0}>
                            <Badge tone="warning">
                              {i18n
                                .t("settings.avatar.warningSummary")
                                .replace(
                                  "{count}",
                                  String(resolvedAvatarAssessment(entry).issues.length),
                                )}
                            </Badge>
                          </Show>
                        </div>
                        <span>{`${entry.sha256.slice(0, 16)}… · ${formatDate(entry.importedAt)}`}</span>
                      </div>
                    }
                    details={
                      <details class="avatar-assessment-details">
                        <summary>{i18n.t("settings.avatar.assessmentDetails")}</summary>
                        <AvatarAssessmentDetails assessment={resolvedAvatarAssessment(entry)} />
                      </details>
                    }
                    actions={
                      <>
                        <Show
                          when={entry.isCurrent}
                          fallback={
                            <Button
                              size="small"
                              disabled={
                                resolvedAvatarAssessment(entry).compatibility !== "runtime_ready"
                              }
                              onClick={() => void selectResource(entry.id)}
                            >
                              {i18n.t("common.select")}
                            </Button>
                          }
                        >
                          <Badge tone="success">{i18n.t("common.current")}</Badge>
                        </Show>
                        <Show when={!entry.protected}>
                          <Button
                            size="small"
                            variant="danger"
                            onClick={() => void deleteResource(entry)}
                          >
                            <Trash2 size={14} /> {i18n.t("common.delete")}
                          </Button>
                        </Show>
                      </>
                    }
                  />
                )}
              </For>
            </ResourceList>
          </Show>
        </SettingsSection>
        <Dialog
          open={Boolean(avatarInspection())}
          title={i18n.t("settings.avatar.inspectionTitle")}
          description={i18n.t("settings.avatar.inspectionDescription")}
          onOpenChange={(open) => {
            if (!open) void closeAvatarInspection();
          }}
        >
          <Show when={avatarInspection()}>
            {(inspection) => (
              <div class="avatar-inspection-dialog">
                <div class="avatar-inspection-heading">
                  <div>
                    <strong>{inspection().originalFileName}</strong>
                    <span>{formatBytes(inspection().sizeBytes)}</span>
                  </div>
                  <div class="avatar-badge-row">
                    <Badge>{avatarFormatLabel(inspection().format)}</Badge>
                    <Badge tone={avatarCompatibilityTone(inspection().assessment.compatibility)}>
                      {avatarCompatibilityLabel(
                        inspection().assessment.compatibility,
                        i18n.locale(),
                      )}
                    </Badge>
                  </div>
                </div>
                <AvatarAssessmentDetails assessment={inspection().assessment} />
                <div class="dialog-actions">
                  <Button variant="ghost" onClick={() => void closeAvatarInspection()}>
                    {i18n.t("common.cancel")}
                  </Button>
                  <Button
                    variant="primary"
                    disabled={!inspection().token || busy()}
                    onClick={() => void confirmAvatarImport()}
                  >
                    {inspection().token
                      ? i18n.t("settings.avatar.confirmImport")
                      : i18n.t("settings.avatar.importBlocked")}
                  </Button>
                </div>
              </div>
            )}
          </Show>
        </Dialog>
        <Dialog
          open={Boolean(pendingDelete())}
          title={i18n.t("settings.resource.deleteTitle")}
          description={i18n
            .t("settings.resource.confirmDelete")
            .replace("{name}", pendingDelete()?.name ?? "")}
          onOpenChange={(open) => {
            if (!open) setPendingDelete(undefined);
          }}
        >
          <div class="dialog-actions">
            <Button variant="ghost" onClick={() => setPendingDelete(undefined)}>
              {i18n.t("common.cancel")}
            </Button>
            <Button variant="danger" onClick={() => void confirmDelete()}>
              {i18n.t("common.delete")}
            </Button>
          </div>
        </Dialog>
      </Show>
    </div>
  );
}

function VoiceSettingsPage() {
  const i18n = useI18n();
  const [catalog, setCatalog] = createSignal<VoiceCatalogSnapshot>({
    entries: [],
    currentId: "builtin-melo-zh-en",
  });
  const [runtime, setRuntime] = createSignal<VoiceRuntimeState>();
  const [recognition, setRecognition] = createSignal<SpeechRecognitionRuntimeState>();
  const [inspection, setInspection] = createSignal<VoiceModelInspection>();
  const [pendingDelete, setPendingDelete] = createSignal<VoiceModelEntry>();
  const [name, setName] = createSignal("");
  const [licenseAcknowledged, setLicenseAcknowledged] = createSignal(false);
  const [speakerId, setSpeakerId] = createSignal(0);
  const [speed, setSpeed] = createSignal(100);
  const [computeMode, setComputeMode] = createSignal<VoiceComputeMode>("auto");
  const [recognitionComputeMode, setRecognitionComputeMode] =
    createSignal<VoiceComputeMode>("auto");
  const [busy, setBusy] = createSignal(false);
  const [recognitionBusy, setRecognitionBusy] = createSignal(false);
  const [notice, setNotice] = createSignal<{ tone: "success" | "danger"; text: string }>();
  const unlisteners: Array<() => void> = [];

  async function load() {
    const [nextCatalog, nextRuntime, nextRecognition] = await Promise.all([
      commands.listVoiceModels(),
      commands.getVoiceRuntimeState(),
      commands.getSpeechRecognitionState(),
    ]);
    setCatalog(nextCatalog);
    setRuntime(nextRuntime);
    setSpeed(nextRuntime.speedPercent);
    setComputeMode(nextRuntime.computeMode);
    setRecognition(nextRecognition);
    setRecognitionComputeMode(nextRecognition.computeMode);
  }

  async function updateRecognitionSettings(nextComputeMode: VoiceComputeMode) {
    setRecognitionBusy(true);
    setNotice(undefined);
    try {
      const next = await commands.updateSpeechRecognitionSettings({
        computeMode: nextComputeMode,
      });
      setRecognition(next);
      setRecognitionComputeMode(next.computeMode);
      setNotice({ tone: "success", text: i18n.t("settings.voice.inputBackendSaved") });
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
      await commands
        .getSpeechRecognitionState()
        .then((value) => {
          setRecognition(value);
          setRecognitionComputeMode(value.computeMode);
        })
        .catch(() => undefined);
    } finally {
      setRecognitionBusy(false);
    }
  }

  async function updateVoiceSettings(speedPercent: number, nextComputeMode: VoiceComputeMode) {
    setBusy(true);
    try {
      const next = await commands.updateVoiceSettings({
        speedPercent,
        computeMode: nextComputeMode,
      });
      setRuntime(next);
      setSpeed(next.speedPercent);
      setComputeMode(next.computeMode);
      setNotice({ tone: "success", text: i18n.t("settings.voice.profileSaved") });
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
      const previous = runtime();
      if (previous) {
        setSpeed(previous.speedPercent);
        setComputeMode(previous.computeMode);
      }
    } finally {
      setBusy(false);
    }
  }

  async function inspectModel() {
    if (!name().trim()) {
      setNotice({ tone: "danger", text: i18n.t("settings.resource.invalidName") });
      return;
    }
    setBusy(true);
    setNotice(undefined);
    try {
      const next = await commands.inspectVoiceModel();
      if (next) {
        setInspection(next);
        setLicenseAcknowledged(false);
        setSpeakerId(next.suggestedSpeakerId);
      } else {
        setNotice({ tone: "success", text: i18n.t("settings.resource.cancelled") });
      }
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    } finally {
      setBusy(false);
    }
  }

  async function closeInspection() {
    const token = inspection()?.token;
    setInspection(undefined);
    setSpeakerId(0);
    if (token) await commands.cancelVoiceModelImport(token).catch(() => undefined);
  }

  async function commitImport() {
    const current = inspection();
    if (!current?.token) return;
    setBusy(true);
    try {
      const next = await commands.commitVoiceModelImport({
        token: current.token,
        name: name().trim(),
        licenseAcknowledged: licenseAcknowledged(),
        speakerId: speakerId(),
      });
      setCatalog(next);
      setInspection(undefined);
      setName("");
      setLicenseAcknowledged(false);
      setSpeakerId(0);
      setNotice({ tone: "success", text: i18n.t("settings.resource.imported") });
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    } finally {
      setBusy(false);
    }
  }

  async function selectModel(id: string) {
    setBusy(true);
    try {
      setCatalog(await commands.selectVoiceModel(id));
      setRuntime(await commands.getVoiceRuntimeState());
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    } finally {
      setBusy(false);
    }
  }

  async function confirmDelete() {
    const entry = pendingDelete();
    if (!entry) return;
    try {
      setCatalog(await commands.deleteVoiceModel(entry.id));
      setPendingDelete(undefined);
      setRuntime(await commands.getVoiceRuntimeState());
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    }
  }

  async function setMuted(muted: boolean) {
    try {
      setRuntime(await commands.setMuted(muted));
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    }
  }

  async function preview() {
    try {
      setRuntime(await commands.previewDefaultVoice());
      setNotice({ tone: "success", text: i18n.t("settings.voice.previewStarted") });
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    }
  }

  onMount(() => {
    void load().catch((error) =>
      setNotice({ tone: "danger", text: commandFailure(error).message }),
    );
    void Promise.all([
      listen<VoiceCatalogSnapshot>("voice:catalog-changed", ({ payload }) => setCatalog(payload)),
      listen<VoiceRuntimeState>("voice-runtime-changed", ({ payload }) => {
        setRuntime(payload);
        setSpeed(payload.speedPercent);
        setComputeMode(payload.computeMode);
      }),
      listen<SpeechRecognitionRuntimeState>("speech-recognition-state-changed", ({ payload }) => {
        setRecognition(payload);
        setRecognitionComputeMode(payload.computeMode);
      }),
    ]).then((values) => unlisteners.push(...values));
  });
  onCleanup(() => unlisteners.forEach((unlisten) => unlisten()));

  return (
    <div class="settings-page">
      <PageHeading
        title={i18n.t("settings.voice")}
        description={i18n.t("settings.voice.description")}
        badge="sherpa-onnx 1.13.4"
      />

      <SettingsSection title={i18n.t("settings.voice.inputTitle")}>
        <StatusBanner>{i18n.t("settings.voice.inputDescription")}</StatusBanner>
        <div class="settings-card voice-runtime-card">
          <SettingsRow
            label={recognition()?.modelName ?? "SenseVoice-Small INT8"}
            description={i18n.t("settings.voice.recognitionDescription")}
          >
            <Badge tone={recognition()?.installed ? "success" : "danger"}>
              {recognition()?.installed
                ? i18n.t("settings.voice.inputBundledState")
                : i18n.t("settings.voice.unavailable")}
            </Badge>
          </SettingsRow>
          <SettingsRow
            label={i18n.t("settings.voice.inputRuntime")}
            description={recognition()?.provider ?? "sherpa-onnx 1.13.4"}
          >
            <SelectField
              label={i18n.t("settings.voice.computeMode")}
              value={recognitionComputeMode()}
              disabled={recognitionBusy() || Boolean(recognition()?.loading)}
              options={[
                { value: "auto", label: i18n.t("settings.voice.computeAuto") },
                { value: "direct_ml", label: "DirectML" },
                { value: "cpu", label: "CPU" },
              ]}
              onChange={(value) => void updateRecognitionSettings(value as VoiceComputeMode)}
            />
          </SettingsRow>
          <SettingsRow
            label={i18n.t("settings.voice.backend")}
            description={
              recognition()?.computeDevice
                ? `${recognition()!.computeDevice!.name} · Adapter ${recognition()!.computeDevice!.deviceId} · ${recognition()!.computeDevice!.dedicatedMemoryMb} MB`
                : (recognition()?.fallbackReason ?? i18n.t("settings.voice.backendDescription"))
            }
          >
            <Badge tone={recognition()?.fallbackReason ? "warning" : "info"}>
              {recognition()?.loading
                ? i18n.t("settings.voice.loading")
                : recognition()?.backend === "direct_ml"
                  ? "DirectML"
                  : recognition()?.backend === "cpu"
                    ? "CPU"
                    : i18n.t("settings.voice.detecting")}
            </Badge>
          </SettingsRow>
          <SettingsRow
            label={i18n.t("settings.voice.inputLanguages")}
            description={(
              recognition()?.languages ?? ["zh-CN", "en-US", "ja-JP", "ko-KR", "yue"]
            ).join(" / ")}
          >
            <Badge>
              {recognition()?.installed ? formatBytes(recognition()?.sizeBytes ?? 0) : "—"}
            </Badge>
          </SettingsRow>
          <Show when={recognition()?.error}>
            {(error) => <StatusBanner tone="danger">{error()}</StatusBanner>}
          </Show>
        </div>
      </SettingsSection>

      <SettingsSection title={i18n.t("settings.voice.outputTitle")}>
        <div class="settings-card voice-runtime-card">
          <SettingsRow
            label={runtime()?.voiceName || i18n.t("settings.voice.builtIn")}
            description={i18n.t("settings.voice.offlineDescription")}
          >
            <Badge
              tone={runtime()?.available ? "success" : runtime()?.loading ? "warning" : "danger"}
            >
              {runtime()?.loading
                ? i18n.t("settings.voice.loading")
                : runtime()?.available
                  ? i18n.t("settings.voice.ready")
                  : i18n.t("settings.voice.unavailable")}
            </Badge>
          </SettingsRow>
          <SettingsRow
            label={i18n.t("settings.voice.computeMode")}
            description={i18n.t("settings.voice.computeDescription")}
          >
            <SelectField
              label={i18n.t("settings.voice.computeMode")}
              value={computeMode()}
              disabled={busy()}
              options={[
                { value: "auto", label: i18n.t("settings.voice.computeAuto") },
                { value: "direct_ml", label: "DirectML" },
                { value: "cpu", label: "CPU" },
              ]}
              onChange={(value) => void updateVoiceSettings(speed(), value as VoiceComputeMode)}
            />
          </SettingsRow>
          <SettingsRow
            label={i18n.t("settings.voice.backend")}
            description={
              runtime()?.computeDevice
                ? `${runtime()!.computeDevice!.name} · Adapter ${runtime()!.computeDevice!.deviceId} · ${runtime()!.computeDevice!.dedicatedMemoryMb} MB`
                : (runtime()?.fallbackReason ?? i18n.t("settings.voice.backendDescription"))
            }
          >
            <Badge tone={runtime()?.fallbackReason ? "warning" : "info"}>
              {runtime()?.backend === "direct_ml" ? "DirectML" : "CPU"}
            </Badge>
          </SettingsRow>
          <Show when={(runtime()?.speakerCount ?? 1) > 1}>
            <SettingsRow
              label={i18n.t("settings.voice.speakerId")}
              description={i18n
                .t("settings.voice.speakerSummary")
                .replace("{id}", String(runtime()?.speakerId ?? 0))
                .replace("{count}", String(runtime()?.speakerCount ?? 1))}
            >
              <Badge tone="info">Speaker {runtime()?.speakerId ?? 0}</Badge>
            </SettingsRow>
          </Show>
          <SettingsRow
            label={i18n.t("settings.voice.speed")}
            description={i18n.t("settings.voice.speedDescription")}
          >
            <RangeField
              label={i18n.t("settings.voice.speed")}
              min={50}
              max={200}
              step={5}
              unit="%"
              value={speed()}
              disabled={busy()}
              onInput={setSpeed}
              onCommit={(value) => void updateVoiceSettings(value, computeMode())}
            />
          </SettingsRow>
          <SettingsRow label={i18n.t("settings.voice.muted")}>
            <Toggle
              checked={runtime()?.muted ?? false}
              label={i18n.t("settings.voice.muted")}
              onChange={(value) => void setMuted(value)}
            />
          </SettingsRow>
          <SettingsRow label={i18n.t("settings.voice.preview")}>
            <div class="voice-preview-actions">
              <Button
                size="small"
                disabled={!runtime()?.available || runtime()?.muted || busy()}
                onClick={() => void preview()}
              >
                <Play size={14} /> {i18n.t("settings.voice.preview")}
              </Button>
              <Button size="small" variant="ghost" onClick={() => void commands.stopSpeech()}>
                <Square size={12} /> {i18n.t("pet.stop")}
              </Button>
            </div>
          </SettingsRow>
        </div>
      </SettingsSection>

      <SettingsSection title={i18n.t("settings.voice.importTitle")}>
        <StatusBanner>{i18n.t("settings.voice.importDescription")}</StatusBanner>
        <div class="settings-card unified-settings-card resource-import-card">
          <SettingsRow label={i18n.t("settings.resourceName")}>
            <TextField
              label={i18n.t("settings.resourceName")}
              value={name()}
              placeholder={i18n.t("settings.voice.nameExample")}
              onInput={(event) => setName(event.currentTarget.value)}
            />
          </SettingsRow>
          <div class="settings-card-actions">
            <Button variant="primary" disabled={busy()} onClick={() => void inspectModel()}>
              {i18n.t("settings.voice.inspect")}
            </Button>
          </div>
        </div>
      </SettingsSection>

      <Show when={notice()}>
        {(value) => <StatusBanner tone={value().tone}>{value().text}</StatusBanner>}
      </Show>

      <SettingsSection title={`${i18n.t("settings.catalog")} · ${catalog().entries.length}`}>
        <ResourceList label={i18n.t("settings.voice")}>
          <For each={catalog().entries}>
            {(entry) => (
              <ResourceCard
                title={entry.name}
                subtitle={`${entry.originalFileName} · ${formatBytes(entry.sizeBytes)}`}
                current={entry.id === catalog().currentId}
                meta={
                  <div class="avatar-resource-meta">
                    <div class="avatar-badge-row">
                      <Badge tone="info">{entry.modelType}</Badge>
                      <Badge>{entry.languages.join(" / ")}</Badge>
                      <Badge>{`${entry.sampleRate.toLocaleString()} Hz`}</Badge>
                      <Show when={(entry.speakerCount ?? 1) > 1}>
                        <Badge tone="info">
                          {i18n
                            .t("settings.voice.speakerSummary")
                            .replace("{id}", String(entry.speakerId ?? 0))
                            .replace("{count}", String(entry.speakerCount ?? 1))}
                        </Badge>
                      </Show>
                      <Badge tone={entry.origin === "built_in" ? "warning" : "neutral"}>
                        {entry.origin === "built_in"
                          ? i18n.t("settings.voice.builtInOrigin")
                          : i18n.t("settings.voice.importedOrigin")}
                      </Badge>
                    </div>
                    <span>{entry.licenseSummary}</span>
                  </div>
                }
                actions={
                  <>
                    <Show
                      when={entry.id === catalog().currentId}
                      fallback={
                        <Button
                          size="small"
                          disabled={busy()}
                          onClick={() => void selectModel(entry.id)}
                        >
                          {i18n.t("common.select")}
                        </Button>
                      }
                    >
                      <Badge tone="success">{i18n.t("common.current")}</Badge>
                    </Show>
                    <Show when={!entry.protected}>
                      <Button size="small" variant="danger" onClick={() => setPendingDelete(entry)}>
                        <Trash2 size={14} /> {i18n.t("common.delete")}
                      </Button>
                    </Show>
                  </>
                }
              />
            )}
          </For>
        </ResourceList>
      </SettingsSection>

      <Dialog
        open={Boolean(inspection())}
        title={i18n.t("settings.voice.inspectionTitle")}
        description={i18n.t("settings.voice.inspectionDescription")}
        onOpenChange={(open) => {
          if (!open) void closeInspection();
        }}
      >
        <Show when={inspection()}>
          {(value) => (
            <div class="avatar-inspection-dialog">
              <div class="avatar-inspection-heading">
                <div>
                  <strong>{value().originalFileName}</strong>
                  <span>{formatBytes(value().sizeBytes)}</span>
                </div>
                <div class="avatar-badge-row">
                  <Badge tone={value().compatible ? "success" : "danger"}>
                    {value().modelType}
                  </Badge>
                  <Badge>{value().languages.join(" / ") || "Unknown"}</Badge>
                  <Badge>{`${value().sampleRate.toLocaleString()} Hz`}</Badge>
                  <Show when={value().speakerCount > 1}>
                    <Badge tone="info">{value().speakerCount.toLocaleString()} Speakers</Badge>
                  </Show>
                </div>
              </div>
              <StatusBanner tone={value().licenseWarning ? "warning" : "success"}>
                {value().licenseSummary}
              </StatusBanner>
              <div class="voice-required-files">
                <strong>{i18n.t("settings.voice.requiredFiles")}</strong>
                <ul>
                  <For each={value().requiredFiles}>
                    {(path) => (
                      <li>
                        <code>{path}</code>
                      </li>
                    )}
                  </For>
                </ul>
              </div>
              <For each={value().issues}>
                {(issue) => <StatusBanner tone="danger">{issue}</StatusBanner>}
              </For>
              <Show when={value().speakerCount > 1}>
                <SettingsRow
                  label={i18n.t("settings.voice.speakerId")}
                  description={i18n
                    .t("settings.voice.speakerIdDescription")
                    .replace("{max}", String(value().speakerCount - 1))}
                >
                  <NumberField
                    label={i18n.t("settings.voice.speakerId")}
                    min={0}
                    max={value().speakerCount - 1}
                    step={1}
                    value={speakerId()}
                    disabled={busy()}
                    onInput={(event) => setSpeakerId(Number(event.currentTarget.value))}
                  />
                </SettingsRow>
              </Show>
              <SettingsRow
                label={i18n.t("settings.voice.licenseConfirm")}
                description={i18n.t("settings.voice.licenseConfirmDescription")}
              >
                <Toggle
                  checked={licenseAcknowledged()}
                  label={i18n.t("settings.voice.licenseConfirm")}
                  onChange={setLicenseAcknowledged}
                />
              </SettingsRow>
              <div class="dialog-actions">
                <Button variant="ghost" onClick={() => void closeInspection()}>
                  {i18n.t("common.cancel")}
                </Button>
                <Button
                  variant="primary"
                  disabled={
                    !value().token ||
                    !licenseAcknowledged() ||
                    !Number.isInteger(speakerId()) ||
                    speakerId() < 0 ||
                    speakerId() >= value().speakerCount ||
                    busy()
                  }
                  onClick={() => void commitImport()}
                >
                  {i18n.t("settings.voice.confirmImport")}
                </Button>
              </div>
            </div>
          )}
        </Show>
      </Dialog>

      <Dialog
        open={Boolean(pendingDelete())}
        title={i18n.t("settings.resource.deleteTitle")}
        description={i18n
          .t("settings.resource.confirmDelete")
          .replace("{name}", pendingDelete()?.name ?? "")}
        onOpenChange={(open) => {
          if (!open) setPendingDelete(undefined);
        }}
      >
        <div class="dialog-actions">
          <Button variant="ghost" onClick={() => setPendingDelete(undefined)}>
            {i18n.t("common.cancel")}
          </Button>
          <Button variant="danger" onClick={() => void confirmDelete()}>
            {i18n.t("common.delete")}
          </Button>
        </div>
      </Dialog>
    </div>
  );
}

function AvatarAssessmentDetails(props: { assessment: AvatarAssessment }) {
  const i18n = useI18n();
  const stats = () => props.assessment.statistics;
  return (
    <div class="avatar-assessment-report">
      <p class="avatar-level-description">
        {avatarCompatibilityDescription(props.assessment.compatibility, i18n.locale())}
      </p>
      <Show when={(props.assessment.requirements?.length ?? 0) > 0}>
        <div class="avatar-requirement-list">
          <For each={props.assessment.requirements ?? []}>
            {(requirement) => (
              <div class="avatar-requirement-row" data-passed={requirement.passed}>
                <Badge tone={requirement.passed ? "success" : "danger"}>
                  {requirement.passed ? "✓" : "!"}
                </Badge>
                <span>{avatarRequirementLabel(requirement.requirement, i18n.locale())}</span>
                <Show when={!requirement.passed && requirement.detail}>
                  <code>{requirement.detail}</code>
                </Show>
              </div>
            )}
          </For>
        </div>
      </Show>
      <dl class="avatar-statistics-grid">
        <div>
          <dt>{i18n.t("settings.avatar.stats.meshes")}</dt>
          <dd>{stats().meshCount}</dd>
        </div>
        <div>
          <dt>{i18n.t("settings.avatar.stats.triangles")}</dt>
          <dd>{(stats().triangleCount ?? 0).toLocaleString()}</dd>
        </div>
        <div>
          <dt>{i18n.t("settings.avatar.stats.bones")}</dt>
          <dd>{stats().boneCount}</dd>
        </div>
        <div>
          <dt>{i18n.t("settings.avatar.stats.animations")}</dt>
          <dd>{stats().animationCount}</dd>
        </div>
        <div>
          <dt>{i18n.t("settings.avatar.stats.materials")}</dt>
          <dd>{stats().materialCount}</dd>
        </div>
        <div>
          <dt>{i18n.t("settings.avatar.stats.textures")}</dt>
          <dd>{stats().textureCount}</dd>
        </div>
        <div>
          <dt>{i18n.t("settings.avatar.stats.morphs")}</dt>
          <dd>{stats().morphTargetCount}</dd>
        </div>
      </dl>
      <Show when={props.assessment.capabilities.length > 0}>
        <div class="avatar-capability-list">
          <For each={props.assessment.capabilities}>
            {(capability) => (
              <Badge tone="info">{avatarCapabilityLabel(capability, i18n.locale())}</Badge>
            )}
          </For>
        </div>
      </Show>
      <For each={props.assessment.issues}>
        {(issue) => (
          <p class="avatar-assessment-issue" data-severity={issue.severity}>
            {avatarIssueLabel(issue.code, i18n.locale())}
          </p>
        )}
      </For>
    </div>
  );
}

function resolvedAvatarAssessment(entry: AvatarEntry): AvatarAssessment {
  return entry.assessment;
}

function avatarFormatLabel(format: AvatarFormat): string {
  if (format === "vrm0") return "VRM 0.x";
  if (format === "vrm1") return "VRM 1.0";
  return "GLB 2.0";
}

function avatarCompatibilityTone(
  compatibility: AvatarAssessment["compatibility"],
): "neutral" | "success" | "danger" {
  if (compatibility === "runtime_ready") return "success";
  return "danger";
}

function avatarCompatibilityLabel(
  compatibility: AvatarAssessment["compatibility"],
  locale: AppLocale,
): string {
  if (compatibility === "runtime_ready") return "Runtime Ready";
  return locale === "zh-CN" ? "缺少运行能力" : "Missing runtime capability";
}

function avatarCompatibilityDescription(
  compatibility: AvatarAssessment["compatibility"],
  locale: AppLocale,
): string {
  if (compatibility === "runtime_ready") {
    return locale === "zh-CN"
      ? "该 VRM 具备标准动作、神态、视线、口型和二级物理运行能力。"
      : "This VRM is ready for standard motion, expression, gaze, lip-sync, and secondary physics.";
  }
  return locale === "zh-CN"
    ? "模型缺少必要运行能力，不能导入或选择。"
    : "The model is missing required runtime capabilities and cannot be selected.";
}

function avatarRequirementLabel(requirement: string, locale: AppLocale): string {
  const zh: Record<string, string> = {
    vrm_format: "VRM 0.x / 1.0 格式",
    skinned_mesh: "完整蒙皮网格",
    complete_humanoid: "核心 Humanoid 骨骼",
    chest_bone: "胸部骨骼（可选）",
    toe_bones: "脚趾骨骼（可选）",
    finger_bones: "手指骨骼（可选）",
    standard_blinks: "Neutral 与左右眨眼",
    jaw_lip_sync: "基础嘴型（可选）",
    five_visemes: "五元音口型",
    standard_emotions: "标准情绪表情",
    look_at: "VRM LookAt",
    mtoon: "MToon 材质",
    spring_bone: "SpringBone",
    spring_collider: "SpringBone Collider",
    skin_weights: "最多四个有效蒙皮权重",
    resource_budget: "资源预算",
  };
  const en: Record<string, string> = {
    vrm_format: "VRM 0.x / 1.0 format",
    skinned_mesh: "Complete skinned mesh",
    complete_humanoid: "Core humanoid bones",
    chest_bone: "Chest bone (optional)",
    toe_bones: "Toe bones (optional)",
    finger_bones: "Finger bones (optional)",
    standard_blinks: "Neutral and left/right blink",
    jaw_lip_sync: "Basic lip sync (optional)",
    five_visemes: "Five vowel visemes",
    standard_emotions: "Standard emotion expressions",
    look_at: "VRM LookAt",
    mtoon: "MToon materials",
    spring_bone: "SpringBone",
    spring_collider: "SpringBone collider",
    skin_weights: "At most four valid skin weights",
    resource_budget: "Resource budget",
  };
  return (locale === "zh-CN" ? zh : en)[requirement] ?? requirement;
}

function avatarCapabilityLabel(capability: AvatarCapability, locale: AppLocale): string {
  const zh: Record<AvatarCapability, string> = {
    renderable_mesh: "可渲染网格",
    skinned_mesh: "蒙皮网格",
    built_in_animations: "内置动画",
    humanoid_skeleton: "人形骨骼",
    blink: "眨眼",
    viseme: "嘴型",
    look_at: "视线",
    happy_expression: "开心表情",
    sad_expression: "难过表情",
    angry_expression: "生气表情",
    spring_bone: "弹性骨骼",
    standard_motion_retarget: "标准动作",
    runtime_ready: "Runtime Ready",
    m_toon: "MToon 材质",
    spring_bone_collider: "二级物理碰撞体",
    five_finger_hands: "完整手指骨骼",
    five_visemes: "五元音口型",
    standard_expressions: "标准表情集",
    lip_sync_jaw: "基础下颌口型",
    lip_sync_five_viseme: "五元音同步口型",
  };
  const en: Record<AvatarCapability, string> = {
    renderable_mesh: "Renderable mesh",
    skinned_mesh: "Skinned mesh",
    built_in_animations: "Built-in animations",
    humanoid_skeleton: "Humanoid skeleton",
    blink: "Blink",
    viseme: "Visemes",
    look_at: "Look at",
    happy_expression: "Happy expression",
    sad_expression: "Sad expression",
    angry_expression: "Angry expression",
    spring_bone: "Spring bones",
    standard_motion_retarget: "Standard motions",
    runtime_ready: "Runtime Ready",
    m_toon: "MToon materials",
    spring_bone_collider: "Secondary-motion colliders",
    five_finger_hands: "Complete finger bones",
    five_visemes: "Five visemes",
    standard_expressions: "Standard expressions",
    lip_sync_jaw: "Jaw lip sync",
    lip_sync_five_viseme: "Five-viseme lip sync",
  };
  return (locale === "zh-CN" ? zh : en)[capability];
}

function avatarIssueLabel(code: string, locale: AppLocale): string {
  const zh: Record<string, string> = {
    invalid_glb: "文件不是有效的 GLB 2.0 模型。",
    unsupported_glb_version: "仅支持 GLB 2.0。",
    glb_length_mismatch: "模型声明长度与实际文件不一致。",
    invalid_glb_json: "模型 JSON 数据损坏。",
    valid_scene_missing: "没有找到引用可渲染网格的有效场景。",
    resource_out_of_bounds: "模型缓冲区或资源范围超出了文件边界。",
    external_resource: "模型引用了外部资源，必须将贴图和缓冲区内嵌。",
    unsupported_required_extension: "模型使用了当前运行时不支持的必需压缩扩展。",
    renderable_mesh_missing: "没有检测到可渲染三角形网格。",
    position_bounds_missing: "模型缺少有效的 POSITION 边界数据。",
    model_bounds_empty: "模型空间边界为空。",
    humanoid_mapping_incomplete: "骨骼存在，但无法可靠映射为标准人形骨骼。",
    blink_missing: "未检测到标准眨眼表情。",
    five_visemes: "模型缺少 aa/ih/ou/ee/oh 五元音口型。",
    vrm_metadata_missing: "文件扩展名为 VRM，但没有检测到 VRM 元数据。",
    legacy_asset_unreadable: "旧模型无法重新检测，已保留并隔离。",
  };
  const en: Record<string, string> = {
    invalid_glb: "The file is not a valid GLB 2.0 model.",
    unsupported_glb_version: "Only GLB 2.0 is supported.",
    glb_length_mismatch: "The declared model length does not match the file.",
    invalid_glb_json: "The model JSON is damaged.",
    valid_scene_missing: "No valid scene references a renderable mesh.",
    resource_out_of_bounds: "A model buffer or resource range exceeds the file bounds.",
    external_resource: "External resources are not allowed; embed all textures and buffers.",
    unsupported_required_extension: "The model requires an unsupported compression extension.",
    renderable_mesh_missing: "No renderable triangle mesh was detected.",
    position_bounds_missing: "Valid POSITION bounds are missing.",
    model_bounds_empty: "The model bounds are empty.",
    humanoid_mapping_incomplete: "The skeleton cannot be mapped reliably to a humanoid.",
    blink_missing: "No standard blink expression was detected.",
    five_visemes: "The model does not provide all aa/ih/ou/ee/oh vowel visemes.",
    vrm_metadata_missing: "The file uses a VRM extension but contains no VRM metadata.",
    legacy_asset_unreadable: "The legacy model could not be reassessed and was quarantined.",
  };
  return (locale === "zh-CN" ? zh : en)[code] ?? code;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function formatDate(epochMillis: string): string {
  const value = Number(epochMillis);
  return Number.isFinite(value) ? new Date(value).toLocaleString() : epochMillis;
}

function LoadedWorkbench(props: {
  bootstrap: BootstrapState;
  initialSettings: AppSettings;
  initialRoute: WorkbenchRoute;
}) {
  const motionLabEnabled = untrack(() => props.bootstrap.featureFlags.motionLab);
  const initialRoute =
    props.initialRoute === "developer/motion-lab" && !motionLabEnabled
      ? "home"
      : props.initialRoute;
  const [settings, setSettings] = createSignal(props.initialSettings);
  const [history, setHistory] = createSignal<WorkbenchRoute[]>([initialRoute]);
  const [historyIndex, setHistoryIndex] = createSignal(0);
  const [failure, setFailure] = createSignal<string>();
  const [sidebarCollapsed, setSidebarCollapsed] = createSignal(false);
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
              />
            </Match>
            <Match when={route() === "developer/motion-lab" && motionLabEnabled}>
              <MotionLabPage />
            </Match>
            <Match when={true}>
              <HomePage
                navigate={navigate}
                settings={settings()}
                motionLabEnabled={motionLabEnabled}
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
