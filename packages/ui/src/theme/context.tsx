import {
  createContext,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  useContext,
  type Accessor,
  type JSX,
} from "solid-js";

export type ThemeMode = "light" | "dark" | "system";
export type ResolvedTheme = "light" | "dark";
export type ThemeScheme = "light" | "dark";
export type ReducedMotion = "system" | "on" | "off";
export type DiffMarkerMode = "color" | "signs";
export type UiDensity = "compact" | "default" | "comfortable";

export interface ThemeProfile {
  id: string;
  name: string;
  scheme: ThemeScheme;
  builtin: boolean;
  accent: string;
  background: string;
  foreground: string;
  uiFont: string;
  codeFont: string;
  translucentSidebar: boolean;
  contrast: number;
}

export interface AppearancePreferences {
  pointerCursor: boolean;
  reducedMotion: ReducedMotion;
  uiFontSize: number;
  codeFontSize: number;
  diffMarkers: DiffMarkerMode;
  /** Always present after legacy settings are normalized by the provider. */
  density: UiDensity;
}

export interface AppearanceConfig {
  lightThemeId: string;
  darkThemeId: string;
  themes: ThemeProfile[];
  preferences: AppearancePreferences;
}

export const THEME_STORAGE_KEY = "hachimi.theme";
export const APPEARANCE_STORAGE_KEY = "hachimi.appearance";

export const DEFAULT_UI_FONT = 'Inter, "Noto Sans SC", "Segoe UI", system-ui, sans-serif';
export const DEFAULT_CODE_FONT = '"JetBrains Mono", "Cascadia Code", monospace';

function builtinTheme(
  id: string,
  name: string,
  scheme: ThemeScheme,
  accent: string,
  background: string,
  foreground: string,
  translucentSidebar: boolean,
  contrast: number,
): ThemeProfile {
  return {
    id,
    name,
    scheme,
    builtin: true,
    accent,
    background,
    foreground,
    uiFont: DEFAULT_UI_FONT,
    codeFont: DEFAULT_CODE_FONT,
    translucentSidebar,
    contrast,
  };
}

export const DEFAULT_APPEARANCE: AppearanceConfig = {
  lightThemeId: "codex-light",
  darkThemeId: "codex-dark",
  themes: [
    builtinTheme(
      "codex-light",
      "Quiet Graphite Light",
      "light",
      "#4358C5",
      "#F8F7F3",
      "#24272D",
      true,
      54,
    ),
    builtinTheme("codex-dark", "Quiet Graphite", "dark", "#7062D5", "#111316", "#F1F3F5", true, 60),
    builtinTheme(
      "catppuccin-light",
      "Catppuccin Latte",
      "light",
      "#8839EF",
      "#EFF1F5",
      "#4C4F69",
      true,
      52,
    ),
    builtinTheme(
      "catppuccin-dark",
      "Catppuccin Mocha",
      "dark",
      "#CBA6F7",
      "#1E1E2E",
      "#CDD6F4",
      true,
      62,
    ),
    builtinTheme(
      "github-light",
      "GitHub Light",
      "light",
      "#0969DA",
      "#FFFFFF",
      "#1F2328",
      false,
      55,
    ),
    builtinTheme("github-dark", "GitHub Dark", "dark", "#2F81F7", "#0D1117", "#E6EDF3", false, 64),
    builtinTheme(
      "gruvbox-light",
      "Gruvbox Light",
      "light",
      "#D65D0E",
      "#FBF1C7",
      "#3C3836",
      true,
      58,
    ),
    builtinTheme("gruvbox-dark", "Gruvbox Dark", "dark", "#FABD2F", "#282828", "#EBDBB2", true, 62),
    builtinTheme(
      "everforest-light",
      "Everforest Light",
      "light",
      "#8DA101",
      "#FDF6E3",
      "#5C6A72",
      true,
      52,
    ),
    builtinTheme(
      "everforest-dark",
      "Everforest Dark",
      "dark",
      "#A7C080",
      "#2D353B",
      "#D3C6AA",
      true,
      58,
    ),
    builtinTheme(
      "linear-light",
      "Linear Light",
      "light",
      "#5E6AD2",
      "#F7F8FA",
      "#1F232B",
      true,
      60,
    ),
    builtinTheme("linear-dark", "Linear Dark", "dark", "#7C85F6", "#111318", "#F3F4F6", true, 68),
    builtinTheme(
      "notion-light",
      "Notion Light",
      "light",
      "#37352F",
      "#FFFFFF",
      "#37352F",
      false,
      48,
    ),
    builtinTheme("notion-dark", "Notion Dark", "dark", "#D3D1CB", "#191919", "#EDECE9", false, 52),
    builtinTheme("one-light", "One Light", "light", "#4078F2", "#FAFAFA", "#383A42", true, 56),
    builtinTheme("one-dark", "One Dark", "dark", "#61AFEF", "#282C34", "#ABB2BF", true, 60),
    builtinTheme(
      "absolutely-light",
      "Absolutely Light",
      "light",
      "#D2694B",
      "#FFF9F5",
      "#382F2A",
      true,
      54,
    ),
    builtinTheme(
      "absolutely-dark",
      "Absolutely Dark",
      "dark",
      "#FF9E64",
      "#1A1718",
      "#F4EDEB",
      true,
      66,
    ),
  ],
  preferences: {
    pointerCursor: false,
    reducedMotion: "system",
    uiFontSize: 14,
    codeFontSize: 12,
    diffMarkers: "color",
    density: "default",
  },
};

interface RuntimeAppearanceMirror {
  version: 1;
  mode: ThemeMode;
  appearance: AppearanceConfig;
}

export function isThemeMode(value: unknown): value is ThemeMode {
  return value === "light" || value === "dark" || value === "system";
}

export function isHexColor(value: unknown): value is string {
  return typeof value === "string" && /^#[\dA-Fa-f]{6}$/.test(value);
}

export function isUiDensity(value: unknown): value is UiDensity {
  return value === "compact" || value === "default" || value === "comfortable";
}

export function normalizeAppearanceConfig(config: AppearanceConfig): AppearanceConfig {
  const themes = config.themes.map((profile) => {
    const isLegacyLight =
      profile.id === "codex-light" &&
      profile.builtin &&
      profile.accent.toUpperCase() === "#1677D2" &&
      profile.background.toUpperCase() === "#F5F4F7" &&
      profile.foreground.toUpperCase() === "#202126";
    const isLegacyDark =
      profile.id === "codex-dark" &&
      profile.builtin &&
      profile.accent.toUpperCase() === "#2EA8FF" &&
      profile.background.toUpperCase() === "#151616" &&
      profile.foreground.toUpperCase() === "#F1F1F3";
    if (!isLegacyLight && !isLegacyDark) return profile;
    return {
      ...DEFAULT_APPEARANCE.themes.find((candidate) => candidate.id === profile.id)!,
    };
  });
  return {
    ...config,
    themes,
    preferences: {
      ...config.preferences,
      density: isUiDensity(config.preferences.density) ? config.preferences.density : "default",
    },
  };
}

export function isAppearanceConfig(value: unknown): value is AppearanceConfig {
  if (!value || typeof value !== "object") return false;
  const config = value as Partial<AppearanceConfig>;
  if (
    typeof config.lightThemeId !== "string" ||
    typeof config.darkThemeId !== "string" ||
    !Array.isArray(config.themes) ||
    config.themes.length === 0 ||
    config.themes.length > 32 ||
    !config.preferences
  ) {
    return false;
  }
  const ids = new Set<string>();
  for (const profile of config.themes) {
    if (
      !profile ||
      typeof profile.id !== "string" ||
      profile.id.length === 0 ||
      profile.id.length > 64 ||
      !/^[\dA-Za-z_-]+$/.test(profile.id) ||
      ids.has(profile.id) ||
      typeof profile.name !== "string" ||
      profile.name.length === 0 ||
      profile.name.length > 64 ||
      (profile.scheme !== "light" && profile.scheme !== "dark") ||
      typeof profile.builtin !== "boolean" ||
      !isHexColor(profile.accent) ||
      !isHexColor(profile.background) ||
      !isHexColor(profile.foreground) ||
      typeof profile.uiFont !== "string" ||
      !profile.uiFont ||
      profile.uiFont.length > 256 ||
      typeof profile.codeFont !== "string" ||
      !profile.codeFont ||
      profile.codeFont.length > 256 ||
      typeof profile.translucentSidebar !== "boolean" ||
      !Number.isInteger(profile.contrast) ||
      profile.contrast < 0 ||
      profile.contrast > 100
    ) {
      return false;
    }
    ids.add(profile.id);
  }
  const preferences = config.preferences;
  return (
    config.themes.some(
      (profile) => profile.id === "codex-light" && profile.scheme === "light" && profile.builtin,
    ) &&
    config.themes.some(
      (profile) => profile.id === "codex-dark" && profile.scheme === "dark" && profile.builtin,
    ) &&
    ids.has(config.lightThemeId) &&
    ids.has(config.darkThemeId) &&
    config.themes.some(
      (profile) => profile.id === config.lightThemeId && profile.scheme === "light",
    ) &&
    config.themes.some(
      (profile) => profile.id === config.darkThemeId && profile.scheme === "dark",
    ) &&
    typeof preferences.pointerCursor === "boolean" &&
    ["system", "on", "off"].includes(preferences.reducedMotion) &&
    Number.isInteger(preferences.uiFontSize) &&
    preferences.uiFontSize >= 12 &&
    preferences.uiFontSize <= 20 &&
    Number.isInteger(preferences.codeFontSize) &&
    preferences.codeFontSize >= 10 &&
    preferences.codeFontSize <= 20 &&
    ["color", "signs"].includes(preferences.diffMarkers) &&
    (preferences.density === undefined || isUiDensity(preferences.density))
  );
}

function storedMirror(storage: Pick<Storage, "getItem">): RuntimeAppearanceMirror | undefined {
  try {
    const raw = storage.getItem(APPEARANCE_STORAGE_KEY);
    if (!raw) return undefined;
    const value = JSON.parse(raw) as Partial<RuntimeAppearanceMirror>;
    if (value.version !== 1 || !isThemeMode(value.mode) || !isAppearanceConfig(value.appearance)) {
      return undefined;
    }
    return value as RuntimeAppearanceMirror;
  } catch {
    return undefined;
  }
}

export function storedTheme(storage: Pick<Storage, "getItem"> = localStorage): ThemeMode {
  const mirror = storedMirror(storage);
  if (mirror) return mirror.mode;
  const value = storage.getItem(THEME_STORAGE_KEY);
  return isThemeMode(value) ? value : "system";
}

export function resolveTheme(mode: ThemeMode, systemDark: boolean): ResolvedTheme {
  return mode === "system" ? (systemDark ? "dark" : "light") : mode;
}

export function selectedTheme(appearance: AppearanceConfig, scheme: ThemeScheme): ThemeProfile {
  const selectedId = scheme === "light" ? appearance.lightThemeId : appearance.darkThemeId;
  return (
    appearance.themes.find((profile) => profile.id === selectedId && profile.scheme === scheme) ??
    DEFAULT_APPEARANCE.themes.find((profile) => profile.scheme === scheme)!
  );
}

interface Rgb {
  r: number;
  g: number;
  b: number;
}

function hexToRgb(value: string): Rgb {
  return {
    r: Number.parseInt(value.slice(1, 3), 16),
    g: Number.parseInt(value.slice(3, 5), 16),
    b: Number.parseInt(value.slice(5, 7), 16),
  };
}

function rgbToHex(color: Rgb): string {
  return `#${[color.r, color.g, color.b]
    .map((channel) => Math.round(channel).toString(16).padStart(2, "0"))
    .join("")}`.toUpperCase();
}

function mix(left: string, right: string, rightWeight: number): string {
  const a = hexToRgb(left);
  const b = hexToRgb(right);
  const weight = Math.max(0, Math.min(1, rightWeight));
  return rgbToHex({
    r: a.r * (1 - weight) + b.r * weight,
    g: a.g * (1 - weight) + b.g * weight,
    b: a.b * (1 - weight) + b.b * weight,
  });
}

function luminance(value: string): number {
  const color = hexToRgb(value);
  const linear = [color.r, color.g, color.b].map((channel) => {
    const normalized = channel / 255;
    return normalized <= 0.04045 ? normalized / 12.92 : ((normalized + 0.055) / 1.055) ** 2.4;
  });
  return linear[0]! * 0.2126 + linear[1]! * 0.7152 + linear[2]! * 0.0722;
}

export function contrastRatio(left: string, right: string): number {
  const lighter = Math.max(luminance(left), luminance(right));
  const darker = Math.min(luminance(left), luminance(right));
  return (lighter + 0.05) / (darker + 0.05);
}

export function accentForeground(accent: string): "#000000" | "#FFFFFF" {
  return contrastRatio(accent, "#000000") >= contrastRatio(accent, "#FFFFFF")
    ? "#000000"
    : "#FFFFFF";
}

export function appearanceTokens(profile: ThemeProfile): Record<string, string> {
  const isQuietGraphiteDark =
    profile.id === "codex-dark" &&
    profile.background.toUpperCase() === "#111316" &&
    profile.foreground.toUpperCase() === "#F1F3F5";
  const isQuietGraphiteLight =
    profile.id === "codex-light" &&
    profile.background.toUpperCase() === "#F8F7F3" &&
    profile.foreground.toUpperCase() === "#24272D";
  const isDark = profile.scheme === "dark";
  const panelWeight = isDark ? 0.055 : 0.035;
  const controlWeight = isDark ? 0.115 : 0.08;
  const contrastScale = 0.25 + profile.contrast / 80;
  const accentRgb = hexToRgb(profile.accent);
  const adjustQuietGraphiteSurface = (value: string, highContrastWeight: number) => {
    if (profile.contrast === 60) return value;
    if (profile.contrast < 60) {
      return mix(value, profile.background, (60 - profile.contrast) / 60);
    }
    return mix(value, profile.foreground, ((profile.contrast - 60) / 40) * highContrastWeight);
  };
  return {
    "--appearance-background": profile.background,
    "--appearance-foreground": profile.foreground,
    "--appearance-accent": profile.accent,
    "--appearance-accent-rgb": `${accentRgb.r} ${accentRgb.g} ${accentRgb.b}`,
    "--appearance-accent-foreground": accentForeground(profile.accent),
    "--appearance-panel": isQuietGraphiteDark
      ? adjustQuietGraphiteSurface("#191C20", 0.18)
      : isQuietGraphiteLight
        ? "#FFFFFF"
        : mix(profile.background, profile.foreground, panelWeight * contrastScale),
    "--appearance-surface-strong": isQuietGraphiteDark
      ? adjustQuietGraphiteSurface("#20242A", 0.2)
      : isQuietGraphiteLight
        ? "#F1EFE9"
        : mix(profile.background, profile.foreground, (isDark ? 0.09 : 0.055) * contrastScale),
    "--appearance-control": isQuietGraphiteDark
      ? adjustQuietGraphiteSurface("#252A31", 0.22)
      : isQuietGraphiteLight
        ? "#EBE9E3"
        : mix(profile.background, profile.foreground, controlWeight * contrastScale),
    "--appearance-titlebar": isQuietGraphiteDark
      ? adjustQuietGraphiteSurface("#15171B", 0.14)
      : isQuietGraphiteLight
        ? "#F2F0EB"
        : mix(profile.background, profile.foreground, (isDark ? 0.07 : 0.045) * contrastScale),
    "--appearance-sidebar-start": isQuietGraphiteDark
      ? adjustQuietGraphiteSurface("#17191D", 0.14)
      : isQuietGraphiteLight
        ? "#EEECE7"
        : mix(profile.background, profile.accent, (isDark ? 0.08 : 0.055) * contrastScale),
    "--appearance-sidebar-end": isQuietGraphiteDark
      ? adjustQuietGraphiteSurface("#17191D", 0.16)
      : isQuietGraphiteLight
        ? "#EEECE7"
        : mix(profile.background, profile.accent, (isDark ? 0.14 : 0.09) * contrastScale),
    "--appearance-border-muted": mix(
      profile.background,
      profile.foreground,
      (isDark ? 0.07 : 0.055) * contrastScale,
    ),
    "--appearance-border": mix(
      profile.background,
      profile.foreground,
      (isDark ? 0.15 : 0.12) * contrastScale,
    ),
    "--appearance-muted": isQuietGraphiteDark
      ? "#A5ADB8"
      : isQuietGraphiteLight
        ? "#505762"
        : mix(
            profile.background,
            profile.foreground,
            (isDark ? 0.48 : 0.44) + (profile.contrast / 100) * 0.3,
          ),
    "--appearance-contrast": String(profile.contrast),
    "--font-ui": profile.uiFont,
    "--font-code": profile.codeFont,
  };
}

export function applyAppearance(
  root: HTMLElement,
  mode: ThemeMode,
  appearance: AppearanceConfig,
  systemDark: boolean,
): ResolvedTheme {
  const resolved = resolveTheme(mode, systemDark);
  const profile = selectedTheme(appearance, resolved);
  root.dataset.themeMode = mode;
  root.dataset.colorScheme = resolved;
  root.dataset.appearanceTheme = resolved;
  root.dataset.pointerCursor = appearance.preferences.pointerCursor ? "on" : "off";
  root.dataset.reducedMotion = appearance.preferences.reducedMotion;
  root.dataset.diffMarkers = appearance.preferences.diffMarkers;
  root.dataset.translucentSidebar = profile.translucentSidebar ? "on" : "off";
  root.dataset.appearanceDensity = appearance.preferences.density ?? "default";
  if (root.style) {
    for (const [name, value] of Object.entries(appearanceTokens(profile))) {
      root.style.setProperty(name, value);
    }
    const uiSize = appearance.preferences.uiFontSize;
    const typeScale = {
      xs: `${Math.max(10, uiSize - 2)}px`,
      sm: `${Math.max(11, uiSize - 1)}px`,
      md: `${uiSize}px`,
      lg: `${uiSize + 2}px`,
      xl: `${uiSize + 6}px`,
      "2xl": `${uiSize + 14}px`,
    };
    for (const [step, value] of Object.entries(typeScale)) {
      root.style.setProperty(`--font-size-${step}`, value);
      root.style.setProperty(`--font-${step}`, value);
    }
    root.style.setProperty("--font-size-code", `${appearance.preferences.codeFontSize}px`);
  }
  return resolved;
}

export function preloadTheme(
  root: HTMLElement = document.documentElement,
  storage: Pick<Storage, "getItem"> = localStorage,
  systemDark = window.matchMedia("(prefers-color-scheme: dark)").matches,
): ResolvedTheme {
  const mirror = storedMirror(storage);
  const mode = mirror?.mode ?? storedTheme(storage);
  return applyAppearance(
    root,
    mode,
    normalizeAppearanceConfig(mirror?.appearance ?? DEFAULT_APPEARANCE),
    systemDark,
  );
}

interface ThemeContextValue {
  mode: Accessor<ThemeMode>;
  resolved: Accessor<ResolvedTheme>;
  appearance: Accessor<AppearanceConfig>;
  profile: Accessor<ThemeProfile>;
  setMode: (mode: ThemeMode) => void;
  setAppearance: (appearance: AppearanceConfig) => void;
}

const ThemeContext = createContext<ThemeContextValue>();

export interface AppearanceProviderProps {
  initialMode?: ThemeMode;
  initialAppearance?: AppearanceConfig;
  mode?: ThemeMode;
  appearance?: AppearanceConfig;
  onModeChange?: (mode: ThemeMode) => void;
  children: JSX.Element;
}

export function AppearanceProvider(props: AppearanceProviderProps) {
  const media = window.matchMedia("(prefers-color-scheme: dark)");
  const [mode, setModeValue] = createSignal<ThemeMode>(props.initialMode ?? "system");
  const [appearance, setAppearanceValue] = createSignal<AppearanceConfig>(
    normalizeAppearanceConfig(props.initialAppearance ?? DEFAULT_APPEARANCE),
  );
  const [systemDark, setSystemDark] = createSignal(media.matches);
  const resolved = () => resolveTheme(mode(), systemDark());
  const profile = createMemo(() => selectedTheme(appearance(), resolved()));
  const handleSystemChange = (event: MediaQueryListEvent) => setSystemDark(event.matches);
  media.addEventListener("change", handleSystemChange);
  onCleanup(() => media.removeEventListener("change", handleSystemChange));

  createEffect(() => {
    if (props.mode && isThemeMode(props.mode)) setModeValue(props.mode);
  });

  createEffect(() => {
    if (props.appearance && isAppearanceConfig(props.appearance)) {
      setAppearanceValue(normalizeAppearanceConfig(props.appearance));
    }
  });

  createEffect(() => {
    applyAppearance(document.documentElement, mode(), appearance(), systemDark());
    localStorage.setItem(THEME_STORAGE_KEY, mode());
    localStorage.setItem(
      APPEARANCE_STORAGE_KEY,
      JSON.stringify({ version: 1, mode: mode(), appearance: appearance() }),
    );
  });

  const value: ThemeContextValue = {
    mode,
    resolved,
    appearance,
    profile,
    setMode(nextMode) {
      setModeValue(nextMode);
      props.onModeChange?.(nextMode);
    },
    setAppearance(nextAppearance) {
      if (isAppearanceConfig(nextAppearance)) {
        setAppearanceValue(normalizeAppearanceConfig(nextAppearance));
      }
    },
  };
  return <ThemeContext.Provider value={value}>{props.children}</ThemeContext.Provider>;
}

export interface ThemeProviderProps {
  initialMode?: ThemeMode;
  onModeChange?: (mode: ThemeMode) => void;
  children: JSX.Element;
}

export function ThemeProvider(props: ThemeProviderProps) {
  return (
    <AppearanceProvider
      initialMode={props.initialMode ?? "system"}
      onModeChange={(mode) => props.onModeChange?.(mode)}
    >
      {props.children}
    </AppearanceProvider>
  );
}

export function useTheme(): ThemeContextValue {
  const context = useContext(ThemeContext);
  if (!context) throw new Error("useTheme must be used inside ThemeProvider");
  return context;
}
