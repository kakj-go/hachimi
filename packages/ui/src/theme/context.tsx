import {
  createContext,
  createEffect,
  createMemo,
  createSignal,
  useContext,
  type Accessor,
  type JSX,
} from "solid-js";

export type ThemeScheme = "light" | "dark";
export type ReducedMotion = "system" | "on" | "off";
export type DiffMarkerMode = "color" | "signs";
export type UiDensity = "compact" | "default" | "comfortable";

export interface ThemeProfile {
  id: string;
  name: string;
  scheme: ThemeScheme;
  accent: string;
  background: string;
  foreground: string;
}

export interface AppearancePreferences {
  pointerCursor: boolean;
  reducedMotion: ReducedMotion;
  uiFontSize: number;
  codeFontSize: number;
  diffMarkers: DiffMarkerMode;
  density: UiDensity;
  uiFont: string;
  codeFont: string;
  translucentSidebar: boolean;
  contrast: number;
}

export interface AppearanceConfig {
  activeThemeId: string;
  preferences: AppearancePreferences;
}

export const APPEARANCE_STORAGE_KEY = "hachimi.appearance";

export const DEFAULT_UI_FONT = 'Inter, "Noto Sans SC", "Segoe UI", system-ui, sans-serif';
export const DEFAULT_CODE_FONT = '"JetBrains Mono", "Cascadia Code", monospace';

export const DEFAULT_THEME_ID = "nya";

export const BUILTIN_THEMES: readonly ThemeProfile[] = [
  {
    id: "px",
    name: "像素物语",
    scheme: "dark",
    accent: "#FF6BA0",
    background: "#171030",
    foreground: "#F4EFFF",
  },
  {
    id: "crm",
    name: "奶油手帐",
    scheme: "light",
    accent: "#FF8FAB",
    background: "#FDF3EA",
    foreground: "#3C302C",
  },
  {
    id: "nya",
    name: "黑猫夜行",
    scheme: "dark",
    accent: "#FFB64D",
    background: "#161110",
    foreground: "#F7EAD9",
  },
  {
    id: "tora",
    name: "橘猫咖啡",
    scheme: "light",
    accent: "#FF8C2E",
    background: "#FFF4E6",
    foreground: "#3B2817",
  },
  {
    id: "maho",
    name: "魔法星辰",
    scheme: "light",
    accent: "#FF7AD9",
    background: "#F2EAFB",
    foreground: "#392D40",
  },
];

export const DEFAULT_PREFERENCES: AppearancePreferences = {
  pointerCursor: false,
  reducedMotion: "system",
  uiFontSize: 14,
  codeFontSize: 12,
  diffMarkers: "color",
  density: "default",
  uiFont: DEFAULT_UI_FONT,
  codeFont: DEFAULT_CODE_FONT,
  translucentSidebar: true,
  contrast: 60,
};

export const DEFAULT_APPEARANCE: AppearanceConfig = {
  activeThemeId: DEFAULT_THEME_ID,
  preferences: DEFAULT_PREFERENCES,
};

interface RuntimeAppearanceMirror {
  version: 2;
  appearance: AppearanceConfig;
}

export function isHexColor(value: unknown): value is string {
  return typeof value === "string" && /^#[\dA-Fa-f]{6}$/.test(value);
}

export function isUiDensity(value: unknown): value is UiDensity {
  return value === "compact" || value === "default" || value === "comfortable";
}

export function builtinThemeById(id: string): ThemeProfile | undefined {
  return BUILTIN_THEMES.find((profile) => profile.id === id);
}

export function activeThemeProfile(appearance: AppearanceConfig): ThemeProfile {
  return builtinThemeById(appearance.activeThemeId) ?? builtinThemeById(DEFAULT_THEME_ID)!;
}

export function isAppearanceConfig(value: unknown): value is AppearanceConfig {
  if (!value || typeof value !== "object") return false;
  const config = value as Partial<AppearanceConfig>;
  if (
    typeof config.activeThemeId !== "string" ||
    config.activeThemeId.length === 0 ||
    config.activeThemeId.length > 64 ||
    !/^[\dA-Za-z_-]+$/.test(config.activeThemeId) ||
    !config.preferences
  ) {
    return false;
  }
  const preferences = config.preferences;
  return (
    typeof preferences.pointerCursor === "boolean" &&
    ["system", "on", "off"].includes(preferences.reducedMotion) &&
    Number.isInteger(preferences.uiFontSize) &&
    preferences.uiFontSize >= 12 &&
    preferences.uiFontSize <= 20 &&
    Number.isInteger(preferences.codeFontSize) &&
    preferences.codeFontSize >= 10 &&
    preferences.codeFontSize <= 20 &&
    ["color", "signs"].includes(preferences.diffMarkers) &&
    isUiDensity(preferences.density) &&
    typeof preferences.uiFont === "string" &&
    preferences.uiFont.trim().length > 0 &&
    preferences.uiFont.length <= 256 &&
    typeof preferences.codeFont === "string" &&
    preferences.codeFont.trim().length > 0 &&
    preferences.codeFont.length <= 256 &&
    typeof preferences.translucentSidebar === "boolean" &&
    Number.isInteger(preferences.contrast) &&
    preferences.contrast >= 0 &&
    preferences.contrast <= 100
  );
}

export function normalizeAppearanceConfig(config: AppearanceConfig): AppearanceConfig {
  return {
    activeThemeId: builtinThemeById(config.activeThemeId) ? config.activeThemeId : DEFAULT_THEME_ID,
    preferences: {
      ...DEFAULT_PREFERENCES,
      ...config.preferences,
      density: isUiDensity(config.preferences?.density)
        ? config.preferences.density
        : DEFAULT_PREFERENCES.density,
    },
  };
}

function storedMirror(storage: Pick<Storage, "getItem">): RuntimeAppearanceMirror | undefined {
  try {
    const raw = storage.getItem(APPEARANCE_STORAGE_KEY);
    if (!raw) return undefined;
    const value = JSON.parse(raw) as Partial<RuntimeAppearanceMirror>;
    if (value.version !== 2 || !isAppearanceConfig(value.appearance)) return undefined;
    return value as RuntimeAppearanceMirror;
  } catch {
    return undefined;
  }
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

export function appearanceTokens(
  profile: ThemeProfile,
  preferences: AppearancePreferences,
): Record<string, string> {
  const isDark = profile.scheme === "dark";
  const panelWeight = isDark ? 0.055 : 0.035;
  const controlWeight = isDark ? 0.115 : 0.08;
  const contrastScale = 0.25 + preferences.contrast / 80;
  const accentRgb = hexToRgb(profile.accent);
  return {
    "--appearance-background": profile.background,
    "--appearance-foreground": profile.foreground,
    "--appearance-accent": profile.accent,
    "--appearance-accent-rgb": `${accentRgb.r} ${accentRgb.g} ${accentRgb.b}`,
    "--appearance-accent-foreground": accentForeground(profile.accent),
    "--appearance-panel": mix(profile.background, profile.foreground, panelWeight * contrastScale),
    "--appearance-surface-strong": mix(
      profile.background,
      profile.foreground,
      (isDark ? 0.09 : 0.055) * contrastScale,
    ),
    "--appearance-control": mix(
      profile.background,
      profile.foreground,
      controlWeight * contrastScale,
    ),
    "--appearance-titlebar": mix(
      profile.background,
      profile.foreground,
      (isDark ? 0.07 : 0.045) * contrastScale,
    ),
    "--appearance-sidebar-start": mix(
      profile.background,
      profile.accent,
      (isDark ? 0.08 : 0.055) * contrastScale,
    ),
    "--appearance-sidebar-end": mix(
      profile.background,
      profile.accent,
      (isDark ? 0.14 : 0.09) * contrastScale,
    ),
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
    "--appearance-muted": mix(
      profile.background,
      profile.foreground,
      (isDark ? 0.48 : 0.57) + (preferences.contrast / 100) * 0.3,
    ),
    "--appearance-contrast": String(preferences.contrast),
    "--font-ui": preferences.uiFont,
    "--font-code": preferences.codeFont,
  };
}

export function applyAppearance(root: HTMLElement, appearance: AppearanceConfig): ThemeScheme {
  const profile = activeThemeProfile(appearance);
  const preferences = appearance.preferences;
  root.dataset.colorScheme = profile.scheme;
  root.dataset.appearanceTheme = profile.id;
  root.dataset.pointerCursor = preferences.pointerCursor ? "on" : "off";
  root.dataset.reducedMotion = preferences.reducedMotion;
  root.dataset.diffMarkers = preferences.diffMarkers;
  root.dataset.translucentSidebar = preferences.translucentSidebar ? "on" : "off";
  root.dataset.appearanceDensity = preferences.density;
  if (root.style) {
    for (const [name, value] of Object.entries(appearanceTokens(profile, preferences))) {
      root.style.setProperty(name, value);
    }
    const uiSize = preferences.uiFontSize;
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
    root.style.setProperty("--font-size-code", `${preferences.codeFontSize}px`);
  }
  return profile.scheme;
}

export function preloadTheme(
  root: HTMLElement = document.documentElement,
  storage: Pick<Storage, "getItem"> = localStorage,
): ThemeScheme {
  const mirror = storedMirror(storage);
  return applyAppearance(root, normalizeAppearanceConfig(mirror?.appearance ?? DEFAULT_APPEARANCE));
}

interface ThemeContextValue {
  appearance: Accessor<AppearanceConfig>;
  profile: Accessor<ThemeProfile>;
  setTheme: (themeId: string) => void;
  setPreferences: (partial: Partial<AppearancePreferences>) => void;
  setAppearance: (appearance: AppearanceConfig) => void;
}

const ThemeContext = createContext<ThemeContextValue>();

export interface AppearanceProviderProps {
  initialAppearance?: AppearanceConfig;
  appearance?: AppearanceConfig;
  children: JSX.Element;
}

export function AppearanceProvider(props: AppearanceProviderProps) {
  const [appearance, setAppearanceValue] = createSignal<AppearanceConfig>(
    normalizeAppearanceConfig(props.initialAppearance ?? DEFAULT_APPEARANCE),
  );
  const profile = createMemo(() => activeThemeProfile(appearance()));

  createEffect(() => {
    if (props.appearance && isAppearanceConfig(props.appearance)) {
      setAppearanceValue(normalizeAppearanceConfig(props.appearance));
    }
  });

  createEffect(() => {
    applyAppearance(document.documentElement, appearance());
    localStorage.setItem(
      APPEARANCE_STORAGE_KEY,
      JSON.stringify({ version: 2, appearance: appearance() } satisfies RuntimeAppearanceMirror),
    );
  });

  const value: ThemeContextValue = {
    appearance,
    profile,
    setTheme(themeId) {
      setAppearanceValue((current) =>
        normalizeAppearanceConfig({ ...current, activeThemeId: themeId }),
      );
    },
    setPreferences(partial) {
      setAppearanceValue((current) =>
        normalizeAppearanceConfig({
          ...current,
          preferences: { ...current.preferences, ...partial },
        }),
      );
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
  initialAppearance?: AppearanceConfig;
  children: JSX.Element;
}

export function ThemeProvider(props: ThemeProviderProps) {
  return (
    <AppearanceProvider initialAppearance={props.initialAppearance ?? DEFAULT_APPEARANCE}>
      {props.children}
    </AppearanceProvider>
  );
}

export function useTheme(): ThemeContextValue {
  const context = useContext(ThemeContext);
  if (!context) throw new Error("useTheme must be used inside ThemeProvider");
  return context;
}
