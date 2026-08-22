import { describe, expect, it } from "vitest";
import {
  accentForeground,
  activeThemeProfile,
  APPEARANCE_STORAGE_KEY,
  appearanceTokens,
  applyAppearance,
  BUILTIN_THEMES,
  contrastRatio,
  DEFAULT_APPEARANCE,
  DEFAULT_PREFERENCES,
  DEFAULT_THEME_ID,
  isAppearanceConfig,
  normalizeAppearanceConfig,
  preloadTheme,
} from "./context";

describe("built-in themes", () => {
  it("ships the five fixed profiles with the contract colors", () => {
    expect(BUILTIN_THEMES.map((profile) => profile.id)).toEqual([
      "px",
      "crm",
      "nya",
      "tora",
      "maho",
    ]);
    expect(BUILTIN_THEMES.find((profile) => profile.id === "px")).toMatchObject({
      scheme: "dark",
      accent: "#FF6BA0",
      background: "#171030",
      foreground: "#F4EFFF",
    });
    expect(BUILTIN_THEMES.find((profile) => profile.id === "crm")).toMatchObject({
      scheme: "light",
      accent: "#FF8FAB",
      background: "#FDF3EA",
      foreground: "#3C302C",
    });
  });

  it("defaults to the nya theme with the contract preference defaults", () => {
    expect(DEFAULT_THEME_ID).toBe("nya");
    expect(DEFAULT_APPEARANCE.activeThemeId).toBe("nya");
    expect(DEFAULT_APPEARANCE.preferences).toEqual({
      pointerCursor: false,
      reducedMotion: "system",
      uiFontSize: 14,
      codeFontSize: 12,
      diffMarkers: "color",
      density: "default",
      uiFont: DEFAULT_PREFERENCES.uiFont,
      codeFont: DEFAULT_PREFERENCES.codeFont,
      translucentSidebar: true,
      contrast: 60,
    });
  });

  it("resolves the active profile and falls back to nya for unknown ids", () => {
    expect(activeThemeProfile(DEFAULT_APPEARANCE).id).toBe("nya");
    expect(activeThemeProfile({ ...DEFAULT_APPEARANCE, activeThemeId: "px" }).id).toBe("px");
    expect(activeThemeProfile({ ...DEFAULT_APPEARANCE, activeThemeId: "nope" }).id).toBe("nya");
  });
});

describe("appearance validation and normalization", () => {
  it("accepts the default appearance and rejects malformed configs", () => {
    expect(isAppearanceConfig(DEFAULT_APPEARANCE)).toBe(true);
    expect(isAppearanceConfig(undefined)).toBe(false);
    expect(isAppearanceConfig({ activeThemeId: "nya" })).toBe(false);
    expect(
      isAppearanceConfig({
        ...DEFAULT_APPEARANCE,
        preferences: { ...DEFAULT_PREFERENCES, contrast: 120 },
      }),
    ).toBe(false);
    expect(
      isAppearanceConfig({
        ...DEFAULT_APPEARANCE,
        preferences: { ...DEFAULT_PREFERENCES, uiFontSize: 8 },
      }),
    ).toBe(false);
  });

  it("normalizes unknown theme ids and legacy density back to defaults", () => {
    const legacy = structuredClone(DEFAULT_APPEARANCE);
    legacy.activeThemeId = "not-a-theme";
    delete (legacy.preferences as unknown as { density?: string }).density;
    const normalized = normalizeAppearanceConfig(legacy);
    expect(normalized.activeThemeId).toBe("nya");
    expect(normalized.preferences.density).toBe("default");
    expect(normalized.preferences.translucentSidebar).toBe(true);
  });
});

describe("color helpers", () => {
  it("derives readable accent text and reports WCAG contrast", () => {
    expect(accentForeground("#FFB64D")).toBe("#000000");
    expect(accentForeground("#7062D5")).toBe("#FFFFFF");
    expect(contrastRatio("#161110", "#F7EAD9")).toBeGreaterThan(4.5);
    expect(contrastRatio("#FDF3EA", "#3C302C")).toBeGreaterThan(4.5);
  });
});

describe("appearanceTokens", () => {
  it("derives every surface token from the generic mix pipeline", () => {
    const nya = activeThemeProfile(DEFAULT_APPEARANCE);
    const tokens = appearanceTokens(nya, DEFAULT_PREFERENCES);
    expect(tokens["--appearance-background"]).toBe("#161110");
    expect(tokens["--appearance-foreground"]).toBe("#F7EAD9");
    expect(tokens["--appearance-accent"]).toBe("#FFB64D");
    expect(tokens["--appearance-accent-foreground"]).toBe("#000000");
    expect(tokens["--appearance-contrast"]).toBe("60");
    expect(tokens["--font-ui"]).toBe(DEFAULT_PREFERENCES.uiFont);
    expect(tokens["--font-code"]).toBe(DEFAULT_PREFERENCES.codeFont);
    for (const name of [
      "--appearance-panel",
      "--appearance-surface-strong",
      "--appearance-control",
      "--appearance-titlebar",
      "--appearance-sidebar-start",
      "--appearance-sidebar-end",
      "--appearance-border-muted",
      "--appearance-border",
      "--appearance-muted",
    ]) {
      expect(tokens[name]).toMatch(/^#[\dA-F]{6}$/);
    }
  });

  it("scales surfaces with the contrast preference", () => {
    const nya = activeThemeProfile(DEFAULT_APPEARANCE);
    const baseline = appearanceTokens(nya, DEFAULT_PREFERENCES);
    const low = appearanceTokens(nya, { ...DEFAULT_PREFERENCES, contrast: 20 });
    const high = appearanceTokens(nya, { ...DEFAULT_PREFERENCES, contrast: 90 });
    expect(low["--appearance-panel"]).not.toBe(baseline["--appearance-panel"]);
    expect(high["--appearance-control"]).not.toBe(baseline["--appearance-control"]);
    expect(low["--appearance-contrast"]).toBe("20");
    expect(high["--appearance-contrast"]).toBe("90");
  });

  it("takes the font stacks from preferences instead of the profile", () => {
    const nya = activeThemeProfile(DEFAULT_APPEARANCE);
    const tokens = appearanceTokens(nya, {
      ...DEFAULT_PREFERENCES,
      uiFont: "Arial, sans-serif",
      codeFont: "Consolas, monospace",
    });
    expect(tokens["--font-ui"]).toBe("Arial, sans-serif");
    expect(tokens["--font-code"]).toBe("Consolas, monospace");
  });
});

describe("applyAppearance", () => {
  it("writes the theme id and scheme to the root dataset", () => {
    const root = document.createElement("html");
    const scheme = applyAppearance(root, DEFAULT_APPEARANCE);
    expect(scheme).toBe("dark");
    expect(root.dataset.appearanceTheme).toBe("nya");
    expect(root.dataset.colorScheme).toBe("dark");
    const light = applyAppearance(root, { ...DEFAULT_APPEARANCE, activeThemeId: "crm" });
    expect(light).toBe("light");
    expect(root.dataset.appearanceTheme).toBe("crm");
    expect(root.dataset.colorScheme).toBe("light");
    expect(root.dataset.themeMode).toBeUndefined();
  });

  it("applies the complete type scale and behavioral attributes", () => {
    const root = document.createElement("html");
    const appearance = structuredClone(DEFAULT_APPEARANCE);
    appearance.preferences.uiFontSize = 18;
    appearance.preferences.codeFontSize = 16;
    appearance.preferences.pointerCursor = true;
    appearance.preferences.reducedMotion = "on";
    appearance.preferences.diffMarkers = "signs";
    appearance.preferences.density = "compact";
    appearance.preferences.translucentSidebar = false;
    applyAppearance(root, appearance);
    expect(root.style.getPropertyValue("--font-size-xs")).toBe("16px");
    expect(root.style.getPropertyValue("--font-size-sm")).toBe("17px");
    expect(root.style.getPropertyValue("--font-size-md")).toBe("18px");
    expect(root.style.getPropertyValue("--font-size-2xl")).toBe("32px");
    expect(root.style.getPropertyValue("--font-size-code")).toBe("16px");
    expect(root.dataset.pointerCursor).toBe("on");
    expect(root.dataset.reducedMotion).toBe("on");
    expect(root.dataset.diffMarkers).toBe("signs");
    expect(root.dataset.appearanceDensity).toBe("compact");
    expect(root.dataset.translucentSidebar).toBe("off");
  });

  it("uses 14px as the default interface font size", () => {
    const root = document.createElement("html");
    applyAppearance(root, DEFAULT_APPEARANCE);
    expect(root.style.getPropertyValue("--font-size-md")).toBe("14px");
  });
});

describe("preloadTheme", () => {
  it("applies the default theme when nothing is stored", () => {
    const root = document.createElement("html");
    const storage = { getItem: () => null };
    expect(preloadTheme(root, storage)).toBe("dark");
    expect(root.dataset.appearanceTheme).toBe("nya");
  });

  it("preloads the stored appearance mirror before rendering", () => {
    const root = document.createElement("html");
    const appearance = { ...structuredClone(DEFAULT_APPEARANCE), activeThemeId: "tora" };
    const storage = {
      getItem: (key: string) =>
        key === APPEARANCE_STORAGE_KEY ? JSON.stringify({ version: 2, appearance }) : null,
    };
    expect(preloadTheme(root, storage)).toBe("light");
    expect(root.dataset.appearanceTheme).toBe("tora");
    expect(root.style.getPropertyValue("--appearance-accent")).toBe("#FF8C2E");
  });

  it("ignores legacy v1 mirrors and falls back to defaults", () => {
    const root = document.createElement("html");
    const storage = {
      getItem: (key: string) =>
        key === APPEARANCE_STORAGE_KEY
          ? JSON.stringify({ version: 1, mode: "dark", appearance: { themes: [] } })
          : null,
    };
    expect(preloadTheme(root, storage)).toBe("dark");
    expect(root.dataset.appearanceTheme).toBe("nya");
  });
});
