const APPEARANCE_KEY = "hachimi-ui-appearance";

const defaults = {
  themeMode: "dark",
  accent: "#7062d5",
  accentRgb: "112 98 213",
  density: "default",
  translucentSidebar: true,
  reducedMotion: false,
};

const isAccent = (value) => typeof value === "string" && /^#[0-9a-f]{6}$/i.test(value);
const isRgb = (value) => typeof value === "string" && /^\d{1,3} \d{1,3} \d{1,3}$/.test(value);

const normalize = (value) => {
  const next = { ...defaults, ...(value && typeof value === "object" ? value : {}) };
  next.themeMode = ["dark", "light", "system"].includes(next.themeMode)
    ? next.themeMode
    : defaults.themeMode;
  next.density = ["compact", "default", "comfortable"].includes(next.density)
    ? next.density
    : defaults.density;
  next.accent = isAccent(next.accent) ? next.accent.toLowerCase() : defaults.accent;
  next.accentRgb = isRgb(next.accentRgb) ? next.accentRgb : defaults.accentRgb;
  next.translucentSidebar = Boolean(next.translucentSidebar);
  next.reducedMotion = Boolean(next.reducedMotion);
  return next;
};

const read = () => {
  try {
    return normalize(JSON.parse(localStorage.getItem(APPEARANCE_KEY) ?? "null"));
  } catch {
    return { ...defaults };
  }
};

const resolveTheme = (themeMode) => {
  if (themeMode !== "system") return themeMode;
  return window.matchMedia?.("(prefers-color-scheme: light)").matches ? "light" : "dark";
};

const apply = (value = read()) => {
  const appearance = normalize(value);
  const theme = resolveTheme(appearance.themeMode);
  const root = document.documentElement;
  root.dataset.appearanceTheme = theme;
  root.dataset.appearanceDensity = appearance.density;
  root.dataset.translucentSidebar = String(appearance.translucentSidebar);
  root.dataset.reducedMotion = String(appearance.reducedMotion);
  root.style.colorScheme = theme;
  root.style.setProperty("--accent", appearance.accent);
  root.style.setProperty("--accent-rgb", appearance.accentRgb);
  return appearance;
};

const save = (patch) => {
  const next = normalize({ ...read(), ...patch });
  try {
    localStorage.setItem(APPEARANCE_KEY, JSON.stringify(next));
  } catch {
    // The demo remains usable when localStorage is unavailable (for example in a restricted preview).
  }
  apply(next);
  window.dispatchEvent(new CustomEvent("hachimi-appearance-change", { detail: next }));
  return next;
};

window.HachimiAppearance = { defaults, key: APPEARANCE_KEY, read, save, apply, resolveTheme };
apply();

window.matchMedia?.("(prefers-color-scheme: light)").addEventListener("change", () => {
  const appearance = read();
  if (appearance.themeMode === "system") apply(appearance);
});

window.addEventListener("storage", (event) => {
  if (event.key === APPEARANCE_KEY) apply();
});
