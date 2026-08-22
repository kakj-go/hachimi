import { commandFailure, commands, type AppSettings } from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import {
  BUILTIN_THEMES,
  DEFAULT_CODE_FONT,
  DEFAULT_UI_FONT,
  PageHeading,
  RangeField,
  SegmentedControl,
  SelectField,
  SettingsCard,
  SettingsRow,
  SettingsSection,
  Switch as Toggle,
  ThemeCard,
  Toast,
  useTheme,
  type AppearancePreferences,
  type DiffMarkerMode,
  type ReducedMotion,
  type UiDensity,
} from "@hachimi/ui";
import { For, createEffect, createSignal, onCleanup, untrack } from "solid-js";
import { createSerializedAutosave, type AutosaveStatus } from "./appearance-save";
import "./settings-appearance.css";

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

export function AppearanceSettings(props: {
  settings: AppSettings;
  setSettings: (settings: AppSettings) => void;
  fail: (message: string) => void;
}) {
  const i18n = useI18n();
  const theme = useTheme();
  const [toast, setToast] = createSignal<{ tone: "success" | "danger"; text: string }>();
  const [saveStatus, setSaveStatus] = createSignal<AutosaveStatus>("idle");
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

  function chooseTheme(themeId: string) {
    theme.setTheme(themeId);
    persist(
      {
        ...props.settings,
        appearance: { ...props.settings.appearance, activeThemeId: themeId },
      },
      true,
    );
  }

  function updatePreferences(partial: Partial<AppearancePreferences>, immediate = false) {
    theme.setPreferences(partial);
    persist(
      {
        ...props.settings,
        appearance: {
          ...props.settings.appearance,
          preferences: { ...props.settings.appearance.preferences, ...partial },
        },
      },
      immediate,
    );
  }

  const preferences = () => props.settings.appearance.preferences;

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
      <SettingsSection title={i18n.t("settings.appearance.themes")}>
        <div class="theme-grid" role="radiogroup" aria-label={i18n.t("settings.appearance.themes")}>
          <For each={[...BUILTIN_THEMES]}>
            {(profile) => (
              <ThemeCard
                class="theme-option-card"
                selected={theme.profile().id === profile.id}
                label={profile.name}
                previewClass={`theme-option-preview theme-option-${profile.id}`}
                previewStyle={{
                  "--thumb-accent": profile.accent,
                  "--thumb-background": profile.background,
                  "--thumb-foreground": profile.foreground,
                }}
                preview={
                  <>
                    <i class="theme-option-line" />
                    <i class="theme-option-line short" />
                    <span class="theme-option-deco" aria-hidden="true" />
                    <span class="theme-option-scheme">
                      {profile.scheme === "dark"
                        ? i18n.t("settings.theme.dark")
                        : i18n.t("settings.theme.light")}
                    </span>
                  </>
                }
                aria-checked={theme.profile().id === profile.id}
                role="radio"
                onClick={() => chooseTheme(profile.id)}
              />
            )}
          </For>
        </div>
      </SettingsSection>
      <SettingsSection title={i18n.t("settings.appearance.diffPreview")}>
        <div class="diff-preview" aria-label={i18n.t("settings.appearance.diffPreview")}>
          <div
            class="diff-preview-pane"
            data-preview-background={theme.profile().background}
            data-preview-foreground={theme.profile().foreground}
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
      <SettingsSection title={i18n.t("settings.appearance.preferences")}>
        <SettingsCard class="settings-card appearance-preferences">
          <SettingsRow label={i18n.t("settings.appearance.density")}>
            <SegmentedControl<UiDensity>
              label={i18n.t("settings.appearance.density")}
              value={preferences().density}
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
              onChange={(density) => updatePreferences({ density }, true)}
            />
          </SettingsRow>
          <SettingsRow label={i18n.t("settings.appearance.translucentSidebar")}>
            <Toggle
              checked={preferences().translucentSidebar}
              label={i18n.t("settings.appearance.translucentSidebar")}
              onChange={(translucentSidebar) => updatePreferences({ translucentSidebar }, true)}
            />
          </SettingsRow>
          <SettingsRow label={i18n.t("settings.appearance.motion")}>
            <SelectField
              label={i18n.t("settings.appearance.motion")}
              value={preferences().reducedMotion}
              options={[
                { value: "system", label: i18n.t("settings.theme.system") },
                { value: "on", label: i18n.t("common.enabled") },
                { value: "off", label: i18n.t("common.disabled") },
              ]}
              onChange={(reducedMotion) =>
                updatePreferences({ reducedMotion: reducedMotion as ReducedMotion }, true)
              }
            />
          </SettingsRow>
          <SettingsRow label={i18n.t("settings.appearance.uiSize")}>
            <RangeField
              label={i18n.t("settings.appearance.uiSize")}
              value={preferences().uiFontSize}
              min={12}
              max={20}
              unit="px"
              onInput={(uiFontSize) => updatePreferences({ uiFontSize })}
              onCommit={(uiFontSize) => updatePreferences({ uiFontSize }, true)}
            />
          </SettingsRow>
          <SettingsRow label={i18n.t("settings.appearance.codeSize")}>
            <RangeField
              label={i18n.t("settings.appearance.codeSize")}
              value={preferences().codeFontSize}
              min={10}
              max={20}
              unit="px"
              onInput={(codeFontSize) => updatePreferences({ codeFontSize })}
              onCommit={(codeFontSize) => updatePreferences({ codeFontSize }, true)}
            />
          </SettingsRow>
          <SettingsRow label={i18n.t("settings.appearance.pointer")}>
            <Toggle
              checked={preferences().pointerCursor}
              label={i18n.t("settings.appearance.pointer")}
              onChange={(pointerCursor) => updatePreferences({ pointerCursor }, true)}
            />
          </SettingsRow>
          <SettingsRow label={i18n.t("settings.appearance.diffMarkers")}>
            <SegmentedControl<DiffMarkerMode>
              label={i18n.t("settings.appearance.diffMarkers")}
              value={preferences().diffMarkers}
              options={[
                { value: "color", label: i18n.t("settings.appearance.diffColor") },
                { value: "signs", label: i18n.t("settings.appearance.diffSigns") },
              ]}
              onChange={(diffMarkers) => updatePreferences({ diffMarkers }, true)}
            />
          </SettingsRow>
          <SettingsRow label={i18n.t("settings.appearance.contrast")}>
            <RangeField
              label={i18n.t("settings.appearance.contrast")}
              value={preferences().contrast}
              min={0}
              max={100}
              unit="%"
              onInput={(contrast) => updatePreferences({ contrast })}
              onCommit={(contrast) => updatePreferences({ contrast }, true)}
            />
          </SettingsRow>
          <SettingsRow label={i18n.t("settings.appearance.uiFont")}>
            <SelectField
              label={i18n.t("settings.appearance.uiFont")}
              value={preferences().uiFont}
              options={fontSelectOptions(UI_FONT_OPTIONS, preferences().uiFont)}
              onChange={(uiFont) => updatePreferences({ uiFont }, true)}
            />
          </SettingsRow>
          <SettingsRow label={i18n.t("settings.appearance.codeFont")}>
            <SelectField
              label={i18n.t("settings.appearance.codeFont")}
              value={preferences().codeFont}
              options={fontSelectOptions(CODE_FONT_OPTIONS, preferences().codeFont)}
              onChange={(codeFont) => updatePreferences({ codeFont }, true)}
            />
          </SettingsRow>
        </SettingsCard>
      </SettingsSection>
      <Toast open={Boolean(toast())} tone={toast()?.tone} onClose={() => setToast(undefined)}>
        {toast()?.text}
      </Toast>
    </div>
  );
}
