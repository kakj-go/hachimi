import { describe, expect, it } from "vitest";
import {
  accentForeground,
  applyAppearance,
  APPEARANCE_STORAGE_KEY,
  appearanceTokens,
  contrastRatio,
  DEFAULT_APPEARANCE,
  normalizeAppearanceConfig,
  preloadTheme,
  resolveTheme,
  storedTheme,
  THEME_STORAGE_KEY,
} from "./context";

describe("resolveTheme", () => {
  it("follows the system only in system mode", () => {
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
    expect(resolveTheme("light", true)).toBe("light");
  });

  it("falls back when a stored theme is invalid", () => {
    expect(storedTheme({ getItem: () => "sepia" })).toBe("system");
    expect(storedTheme({ getItem: () => "dark" })).toBe("dark");
  });

  it("preloads the stored theme before rendering", () => {
    const root = { dataset: {} } as HTMLElement;
    const storage = {
      getItem: (key: string) => (key === THEME_STORAGE_KEY ? "system" : null),
    };
    expect(preloadTheme(root, storage, true)).toBe("dark");
    expect(root.dataset.colorScheme).toBe("dark");
    expect(root.dataset.themeMode).toBe("system");
  });

  it("preloads the validated appearance mirror without changing user colors", () => {
    const root = document.createElement("html");
    const appearance = structuredClone(DEFAULT_APPEARANCE);
    appearance.themes[1]!.accent = "#FFCC00";
    const storage = {
      getItem: (key: string) =>
        key === APPEARANCE_STORAGE_KEY
          ? JSON.stringify({ version: 1, mode: "dark", appearance })
          : null,
    };
    expect(preloadTheme(root, storage, false)).toBe("dark");
    expect(root.style.getPropertyValue("--appearance-accent")).toBe("#FFCC00");
  });

  it("upgrades legacy Codex defaults while preserving customized colors", () => {
    const appearance = structuredClone(DEFAULT_APPEARANCE);
    Object.assign(appearance.themes[0]!, {
      name: "Codex Light",
      accent: "#1677D2",
      background: "#F5F4F7",
      foreground: "#202126",
    });
    appearance.themes[1]!.accent = "#123456";
    const normalized = normalizeAppearanceConfig(appearance);
    expect(normalized.themes[0]!.name).toBe("Quiet Graphite Light");
    expect(normalized.themes[1]!.accent).toBe("#123456");
  });

  it("normalizes legacy appearance settings without density", () => {
    const legacy = structuredClone(DEFAULT_APPEARANCE);
    delete (legacy.preferences as unknown as { density?: string }).density;
    expect(normalizeAppearanceConfig(legacy).preferences.density).toBe("default");
  });

  it("derives readable accent text and reports WCAG contrast", () => {
    expect(DEFAULT_APPEARANCE.themes).toHaveLength(18);
    expect(accentForeground("#7062D5")).toBe("#FFFFFF");
    expect(contrastRatio("#111316", "#F1F3F5")).toBeGreaterThan(4.5);
    expect(appearanceTokens(DEFAULT_APPEARANCE.themes[1]!)["--appearance-panel"]).toBe("#191C20");
    expect(appearanceTokens(DEFAULT_APPEARANCE.themes[1]!)["--appearance-surface-strong"]).toBe(
      "#20242A",
    );
  });

  it("keeps the demo surface at the default contrast and changes it at other values", () => {
    const profile = structuredClone(DEFAULT_APPEARANCE.themes[1]!);
    const baseline = appearanceTokens(profile);
    profile.contrast = 20;
    const lowContrast = appearanceTokens(profile);
    profile.contrast = 90;
    const highContrast = appearanceTokens(profile);
    expect(baseline["--appearance-panel"]).toBe("#191C20");
    expect(baseline["--appearance-surface-strong"]).toBe("#20242A");
    expect(lowContrast["--appearance-panel"]).not.toBe(baseline["--appearance-panel"]);
    expect(highContrast["--appearance-control"]).not.toBe(baseline["--appearance-control"]);
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
    appearance.themes[1]!.translucentSidebar = false;
    applyAppearance(root, "dark", appearance, false);
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
    applyAppearance(root, "dark", DEFAULT_APPEARANCE, false);
    expect(root.style.getPropertyValue("--font-size-md")).toBe("14px");
  });
});
