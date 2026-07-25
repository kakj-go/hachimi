import { listen } from "@tauri-apps/api/event";
import {
  commandFailure,
  commands,
  type AppSettings,
  type BootstrapState,
  type Locale,
  type ThemeMode as ContractThemeMode,
} from "@hachimi/contracts";
import { I18nProvider, useI18n, type AppLocale } from "@hachimi/i18n";
import {
  Badge,
  NativeSelect,
  SettingsRow,
  SettingsSection,
  Switch,
  Tabs,
  AppearanceProvider,
  useTheme,
  type TabDefinition,
  type ThemeMode,
} from "@hachimi/ui";
import { Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import "./settings.css";

type SettingsTab = "general" | "llm" | "avatar" | "voice";

function Placeholder(props: { text: string; phase: string }) {
  const i18n = useI18n();
  return (
    <section class="settings-placeholder" aria-labelledby={`placeholder-${props.phase}`}>
      <Badge tone="neutral">{props.phase}</Badge>
      <h2 id={`placeholder-${props.phase}`}>{i18n.t("common.comingSoon")}</h2>
      <p>{props.text}</p>
    </section>
  );
}

function SettingsContent(props: {
  settings: AppSettings;
  onAlwaysOnTopChange: (enabled: boolean) => Promise<void>;
}) {
  const i18n = useI18n();
  const theme = useTheme();
  const [activeTab, setActiveTab] = createSignal<SettingsTab>("general");

  const tabs = createMemo<readonly TabDefinition[]>(() => [
    {
      value: "general",
      label: i18n.t("settings.general"),
      content: (
        <div class="settings-sections">
          <SettingsSection title={i18n.t("settings.appearance")}>
            <SettingsRow label={i18n.t("settings.theme")}>
              <NativeSelect
                label={i18n.t("settings.theme")}
                aria-label={i18n.t("settings.theme")}
                value={theme.mode()}
                onChange={(event) => theme.setMode(event.currentTarget.value as ThemeMode)}
              >
                <option value="system">{i18n.t("settings.theme.system")}</option>
                <option value="light">{i18n.t("settings.theme.light")}</option>
                <option value="dark">{i18n.t("settings.theme.dark")}</option>
              </NativeSelect>
            </SettingsRow>
            <SettingsRow label={i18n.t("settings.language")}>
              <NativeSelect
                label={i18n.t("settings.language")}
                aria-label={i18n.t("settings.language")}
                value={i18n.locale()}
                onChange={(event) => i18n.setLocale(event.currentTarget.value as AppLocale)}
              >
                <option value="zh-CN">简体中文</option>
                <option value="en-US">English</option>
              </NativeSelect>
            </SettingsRow>
            <SettingsRow
              label={i18n.t("settings.alwaysOnTop")}
              description={i18n.t("settings.alwaysOnTopDescription")}
            >
              <Switch
                checked={props.settings.alwaysOnTop}
                label={i18n.t("settings.alwaysOnTop")}
                onChange={(enabled) => void props.onAlwaysOnTopChange(enabled)}
              />
            </SettingsRow>
          </SettingsSection>
          <SettingsSection title={i18n.t("settings.phase")}>
            <SettingsRow label={i18n.t("settings.phase")}>
              <Badge tone="neutral">{i18n.t("settings.phaseValue")}</Badge>
            </SettingsRow>
          </SettingsSection>
        </div>
      ),
    },
    {
      value: "llm",
      label: i18n.t("settings.llm"),
      content: <Placeholder phase="Phase 4" text={i18n.t("settings.placeholder.llm")} />,
    },
    {
      value: "avatar",
      label: i18n.t("settings.avatar"),
      content: <Placeholder phase="Phase 2–3" text={i18n.t("settings.placeholder.avatar")} />,
    },
    {
      value: "voice",
      label: i18n.t("settings.voice"),
      content: <Placeholder phase="Phase 6" text={i18n.t("settings.placeholder.voice")} />,
    },
  ]);

  return (
    <main class="settings-window">
      <header class="settings-header">
        <div>
          <p class="settings-eyebrow">Hachimi Desktop</p>
          <h1>{i18n.t("settings.title")}</h1>
        </div>
        <Badge tone="neutral">Windows Preview</Badge>
      </header>
      <Tabs
        value={activeTab()}
        tabs={tabs()}
        onChange={(value) => setActiveTab(value as SettingsTab)}
      />
    </main>
  );
}

function LoadedSettings(props: {
  bootstrap: BootstrapState;
  initialSettings: AppSettings;
  onFailure: (message: string) => void;
}) {
  const [settings, setSettings] = createSignal(props.initialSettings);
  let stopListening: (() => void) | undefined;

  async function persist(patch: Partial<AppSettings>) {
    const previous = settings();
    const next = { ...previous, ...patch };
    setSettings(next);
    try {
      setSettings(await commands.updateSettings(next));
    } catch (error) {
      setSettings(previous);
      props.onFailure(commandFailure(error).message);
    }
  }

  async function changeAlwaysOnTop(enabled: boolean) {
    const previous = settings();
    setSettings({ ...previous, alwaysOnTop: enabled });
    try {
      setSettings(await commands.setAlwaysOnTop(enabled));
    } catch (error) {
      setSettings(previous);
      props.onFailure(commandFailure(error).message);
    }
  }

  onMount(() => {
    void listen<AppSettings>("settings-changed", ({ payload }) => setSettings(payload)).then(
      (unlisten) => {
        stopListening = unlisten;
      },
    );
  });
  onCleanup(() => stopListening?.());

  return (
    <AppearanceProvider
      initialMode={props.bootstrap.theme as ThemeMode}
      initialAppearance={props.bootstrap.appearance}
      mode={settings().theme as ThemeMode}
      appearance={settings().appearance}
      onModeChange={(theme) => void persist({ theme: theme as ContractThemeMode })}
    >
      <I18nProvider
        initialLocale={props.bootstrap.locale as AppLocale}
        onLocaleChange={(locale) => void persist({ locale: locale as Locale })}
      >
        <SettingsContent settings={settings()} onAlwaysOnTopChange={changeAlwaysOnTop} />
      </I18nProvider>
    </AppearanceProvider>
  );
}

export function SettingsApp() {
  const [bootstrap, setBootstrap] = createSignal<BootstrapState>();
  const [settings, setSettings] = createSignal<AppSettings>();
  const [failure, setFailure] = createSignal<string>();

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

  onMount(() => {
    const preventNativeClose = (event: KeyboardEvent) => {
      if (event.key === "Escape") void commands.hideWorkbench();
    };
    window.addEventListener("keydown", preventNativeClose);
    onCleanup(() => window.removeEventListener("keydown", preventNativeClose));
  });

  return (
    <Show
      when={bootstrap() && settings()}
      fallback={
        <main class="settings-loading" role="status">
          {failure() ?? "Hachimi…"}
        </main>
      }
    >
      <LoadedSettings
        bootstrap={bootstrap()!}
        initialSettings={settings()!}
        onFailure={setFailure}
      />
    </Show>
  );
}
