const icon = (name, className = "icon") =>
  `<svg class="${className}" aria-hidden="true"><use href="#icon-${name}"></use></svg>`;

document.body.innerHTML = `
  <svg width="0" height="0" style="position:absolute" aria-hidden="true">
    <symbol id="icon-panel" viewBox="0 0 24 24"><rect x="3" y="3" width="18" height="18" rx="2"/><path d="M9 3v18"/></symbol>
    <symbol id="icon-chevron-left" viewBox="0 0 24 24"><path d="m15 18-6-6 6-6"/></symbol>
    <symbol id="icon-chevron-right" viewBox="0 0 24 24"><path d="m9 18 6-6-6-6"/></symbol>
    <symbol id="icon-minus" viewBox="0 0 24 24"><path d="M5 12h14"/></symbol>
    <symbol id="icon-square" viewBox="0 0 24 24"><rect x="5" y="5" width="14" height="14" rx="1"/></symbol>
    <symbol id="icon-x" viewBox="0 0 24 24"><path d="m6 6 12 12M18 6 6 18"/></symbol>
    <symbol id="icon-search" viewBox="0 0 24 24"><circle cx="11" cy="11" r="7"/><path d="m20 20-4-4"/></symbol>
    <symbol id="icon-settings" viewBox="0 0 24 24"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .34 1.88l.06.06-2.83 2.83-.06-.06A1.7 1.7 0 0 0 15 19.4a1.7 1.7 0 0 0-1 .6 1.7 1.7 0 0 0-.4 1.1V21H9.6v-.1A1.7 1.7 0 0 0 8.5 19.4a1.7 1.7 0 0 0-1.88.34l-.06.06-2.83-2.83.06-.06A1.7 1.7 0 0 0 4.6 15a1.7 1.7 0 0 0-.6-1 1.7 1.7 0 0 0-1.1-.4H3V9.6h.1A1.7 1.7 0 0 0 4.6 8.5a1.7 1.7 0 0 0-.34-1.88l-.06-.06 2.83-2.83.06.06A1.7 1.7 0 0 0 9 4.6a1.7 1.7 0 0 0 1-.6 1.7 1.7 0 0 0 .4-1.1V3h4v.1A1.7 1.7 0 0 0 15.5 4.6a1.7 1.7 0 0 0 1.88-.34l.06-.06 2.83 2.83-.06.06A1.7 1.7 0 0 0 19.4 9c.4.3.7.6.8 1 .2.4.3.8.3 1.1v.1H21v4h-.1a1.7 1.7 0 0 0-1.5 1.1z"/></symbol>
    <symbol id="icon-palette" viewBox="0 0 24 24"><path d="M12 3a9 9 0 0 0 0 18h1.5a2 2 0 0 0 0-4H12a2 2 0 0 1 0-4h4a5 5 0 0 0 5-5c0-2.8-4-5-9-5z"/><circle cx="7.5" cy="10" r=".8"/><circle cx="9" cy="6.5" r=".8"/><circle cx="14" cy="6" r=".8"/><circle cx="17.5" cy="9" r=".8"/></symbol>
    <symbol id="icon-volume" viewBox="0 0 24 24"><path d="M11 5 6 9H3v6h3l5 4zM15 9a4 4 0 0 1 0 6M18 6a8 8 0 0 1 0 12"/></symbol>
    <symbol id="icon-bot" viewBox="0 0 24 24"><rect x="4" y="7" width="16" height="13" rx="2"/><path d="M9 3h6M12 3v4M8 12h.01M16 12h.01M8 16h8"/></symbol>
    <symbol id="icon-box" viewBox="0 0 24 24"><path d="m12 3 8 4.5v9L12 21l-8-4.5v-9zM4 7.5l8 4.5 8-4.5M12 12v9"/></symbol>
    <symbol id="icon-plug" viewBox="0 0 24 24"><path d="m8 12 8-8M14 3l7 7M5 11l8 8M3 14l7 7M8 17l-4 4"/></symbol>
    <symbol id="icon-shield" viewBox="0 0 24 24"><path d="M12 3 20 7v5c0 5-3.5 8-8 9-4.5-1-8-4-8-9V7z"/><path d="m9 12 2 2 4-4"/></symbol>
    <symbol id="icon-language" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c2.3 2.5 3.5 5.5 3.5 9S14.3 18.5 12 21M12 3c-2.3 2.5-3.5 5.5-3.5 9s1.2 6.5 3.5 9"/></symbol>
    <symbol id="icon-spark" viewBox="0 0 24 24"><path d="m12 3 1.4 4.6L18 9l-4.6 1.4L12 15l-1.4-4.6L6 9l4.6-1.4z"/><path d="m19 15 .7 2.3L22 18l-2.3.7L19 21l-.7-2.3L16 18l2.3-.7z"/></symbol>
    <symbol id="icon-check" viewBox="0 0 24 24"><path d="m5 12 4 4L19 6"/></symbol>
  </svg>
  <div class="app settings-app" id="settings-app" data-density="default">
    <header class="titlebar">
      <div class="titlebar-start">
        <button class="brand-menu" type="button" aria-label="Hachimi 菜单"><span class="mini-logo">H</span><span>Hachimi</span></button>
        <button type="button" aria-label="后退">${icon("chevron-left")}</button>
        <button type="button" aria-label="前进">${icon("chevron-right")}</button>
      </div>
      <div class="titlebar-center"><strong>设置</strong><span>·</span><span>外观</span></div>
      <div class="titlebar-end">
        <button id="toggle-settings-sidebar" type="button" aria-label="切换设置导航">${icon("panel")}</button>
        <div class="window-controls" aria-hidden="true">
          <button type="button" tabindex="-1">${icon("minus")}</button><button type="button" tabindex="-1">${icon("square")}</button><button type="button" tabindex="-1">${icon("x")}</button>
        </div>
      </div>
    </header>

    <div class="settings-workspace">
      <aside class="settings-sidebar">
        <div class="settings-sidebar-head"><button class="settings-back" type="button" aria-label="返回工作台">${icon("chevron-left")}</button><strong>设置</strong></div>
        <label class="settings-search">${icon("search")}<input type="search" aria-label="搜索设置" placeholder="搜索设置…" /></label>
        <nav class="settings-nav-scroll" aria-label="设置导航">
          <section class="settings-nav-group">
            <h2>个人</h2>
            <button class="settings-nav-row" type="button">${icon("settings")}<span>通用</span></button>
            <button class="settings-nav-row active" type="button">${icon("palette")}<span>外观</span></button>
            <button class="settings-nav-row" type="button">${icon("volume")}<span>语音</span></button>
          </section>
          <section class="settings-nav-group">
            <h2>Agent 与工具</h2>
            <button class="settings-nav-row" type="button">${icon("bot")}<span>模型与服务</span></button>
            <button class="settings-nav-row" type="button">${icon("plug")}<span>MCP 连接</span></button>
            <button class="settings-nav-row" type="button">${icon("spark")}<span>技能与扩展</span></button>
          </section>
          <section class="settings-nav-group">
            <h2>桌面体验</h2>
            <button class="settings-nav-row" type="button">${icon("box")}<span>桌宠与动作</span></button>
            <button class="settings-nav-row" type="button">${icon("shield")}<span>权限与安全</span></button>
            <button class="settings-nav-row" type="button">${icon("language")}<span>语言与地区</span></button>
          </section>
        </nav>
        <a class="sidebar-footer" href="demo-01-quiet-graphite.html"><span class="mini-logo">A</span><span class="profile-copy"><strong>静谧石墨</strong><small>返回工作台 Demo</small></span>${icon("chevron-right")}</a>
      </aside>

      <main class="settings-main">
        <header class="settings-header">
          <strong>外观</strong><span>控制主题、密度和常用界面行为</span>
          <div class="settings-header-actions"><span class="saved-state"><i class="status-dot"></i>所有更改已保存</span></div>
        </header>
        <div class="settings-scroll">
          <div class="settings-page-demo">
            <header class="settings-page-heading">
              <div><h1>外观</h1><p>保持桌面工作台清晰、安静，并确保主题变化不会改变组件的信息层级。</p></div>
              <span class="demo-label">${icon("spark")}方案 A</span>
            </header>

            <section class="settings-section-demo">
              <div class="settings-section-heading"><div><h2>界面主题</h2><p>主题只改变颜色关系，不改变信息层级；更改会同步到工作台和组件目录。</p></div></div>
              <div class="theme-layout">
                <div class="theme-picker">
                  <div class="theme-grid" role="group" aria-label="主题模式">
                    <button class="theme-card" type="button" data-theme="system" aria-pressed="false"><span class="theme-thumbnail thumbnail-system"><span></span><span></span><span></span></span><strong>跟随系统</strong></button>
                    <button class="theme-card" type="button" data-theme="light" aria-pressed="false"><span class="theme-thumbnail thumbnail-light"><span></span><span></span><span></span></span><strong>浅色</strong></button>
                    <button class="theme-card selected" type="button" data-theme="dark" aria-pressed="true"><span class="theme-thumbnail thumbnail-dark"><span></span><span></span><span></span></span><strong>深色</strong></button>
                  </div>
                  <div class="theme-profile"><span>主题配置 <strong>Quiet Graphite</strong></span><div class="accent-picker" aria-label="强调色"><button class="accent-chip selected" style="--chip-color:#7062d5" data-accent="#7062d5" data-rgb="112 98 213" aria-label="靛紫" type="button"></button><button class="accent-chip" style="--chip-color:#3573c8" data-accent="#3573c8" data-rgb="53 115 200" aria-label="深蓝" type="button"></button><button class="accent-chip" style="--chip-color:#307a67" data-accent="#307a67" data-rgb="48 122 103" aria-label="松绿" type="button"></button></div></div>
                </div>
                <aside class="live-preview-card">
                  <header class="preview-header"><strong>实时预览</strong><span id="preview-label">深色 · 默认密度</span></header>
                  <div class="workbench-preview" id="workbench-preview"><div class="preview-sidebar"><span class="preview-mark">H</span><i></i><i></i><i></i></div><div class="preview-content"><div class="preview-copy"><i></i><i></i><i></i></div><div class="preview-composer"></div></div></div>
                </aside>
              </div>
            </section>

            <section class="settings-section-demo">
              <div class="settings-section-heading"><div><h2>界面偏好</h2><p>所有控件共享同一高度、边框和焦点规则。</p></div></div>
              <div class="settings-card-demo">
                <div class="settings-row-demo"><span class="settings-row-copy"><strong>界面密度</strong><small>改变列表和设置行高度，不缩小正文。</small></span><div class="segmented settings-control" id="density-control" role="group" aria-label="界面密度"><button type="button" data-density="compact">紧凑</button><button class="selected" type="button" data-density="default">默认</button><button type="button" data-density="comfortable">宽松</button></div></div>
                <div class="settings-row-demo"><span class="settings-row-copy"><strong>半透明侧栏</strong><small>在支持的桌面环境中使用轻量背景模糊。</small></span><button class="demo-switch checked settings-control" type="button" role="switch" aria-checked="true" aria-label="半透明侧栏"><span></span></button></div>
                <div class="settings-row-demo"><span class="settings-row-copy"><strong>减少动态效果</strong><small>保留状态反馈，关闭非必要位移和缩放动画。</small></span><button class="demo-switch settings-control" type="button" role="switch" aria-checked="false" aria-label="减少动态效果"><span></span></button></div>
                <div class="settings-row-demo"><span class="settings-row-copy"><strong>界面语言</strong><small>影响导航、设置和 Agent 系统消息。</small></span><select class="demo-select settings-control" aria-label="界面语言"><option selected>简体中文</option><option>English</option><option>日本語</option></select></div>
              </div>
            </section>

            <aside class="settings-footer-note">${icon("check")}<span>方案 A 将设置页的卡片层级压缩为两层：页面分区与设置组。单个设置行不再重复使用独立卡片。</span></aside>
          </div>
        </div>
      </main>
    </div>
  </div>
`;

const settingsApp = document.querySelector("#settings-app");
const preview = document.querySelector("#workbench-preview");
const previewLabel = document.querySelector("#preview-label");
const appearance = window.HachimiAppearance;
let currentAppearance = appearance?.read() ?? {
  themeMode: "dark",
  accent: "#7062d5",
  accentRgb: "112 98 213",
  density: "default",
  translucentSidebar: true,
  reducedMotion: false,
};
let selectedTheme =
  currentAppearance.themeMode === "light"
    ? "浅色"
    : currentAppearance.themeMode === "system"
      ? "跟随系统"
      : "深色";
let selectedDensity =
  currentAppearance.density === "compact"
    ? "紧凑密度"
    : currentAppearance.density === "comfortable"
      ? "宽松密度"
      : "默认密度";

const syncPreviewLabel = () => {
  if (previewLabel) previewLabel.textContent = `${selectedTheme} · ${selectedDensity}`;
};

const syncPreview = (themeMode) => {
  if (!(preview instanceof HTMLElement)) return;
  const light = appearance?.resolveTheme(themeMode) === "light";
  preview.style.setProperty("--preview-bg", light ? "#f8f7f3" : "var(--canvas)");
  preview.style.setProperty("--preview-sidebar", light ? "#eeece7" : "var(--sidebar)");
  preview.style.setProperty("--preview-surface", light ? "#ffffff" : "var(--surface)");
  preview.style.setProperty("--preview-text", light ? "#24272d" : "var(--text)");
  preview.style.setProperty("--preview-line", light ? "#d8d6d0" : "var(--control)");
};

const setSelected = (selector, predicate) => {
  document.querySelectorAll(selector).forEach((item) => {
    const selected = predicate(item);
    item.classList.toggle("selected", selected);
    if (item.hasAttribute("aria-pressed")) item.setAttribute("aria-pressed", String(selected));
  });
};

setSelected(
  ".theme-card",
  (card) => card.getAttribute("data-theme") === currentAppearance.themeMode,
);
setSelected(
  ".accent-chip",
  (chip) => chip.getAttribute("data-accent")?.toLowerCase() === currentAppearance.accent,
);
settingsApp?.setAttribute("data-density", currentAppearance.density);
setSelected(
  "#density-control button",
  (button) => button.getAttribute("data-density") === currentAppearance.density,
);
const translucentSidebar = document.querySelector('[aria-label="半透明侧栏"]');
const reducedMotion = document.querySelector('[aria-label="减少动态效果"]');
const languageSelect = document.querySelector('[aria-label="界面语言"]');
if (translucentSidebar) {
  translucentSidebar.classList.toggle("checked", currentAppearance.translucentSidebar);
  translucentSidebar.setAttribute("aria-checked", String(currentAppearance.translucentSidebar));
}
if (reducedMotion) {
  reducedMotion.classList.toggle("checked", currentAppearance.reducedMotion);
  reducedMotion.setAttribute("aria-checked", String(currentAppearance.reducedMotion));
}
if (languageSelect instanceof HTMLSelectElement && currentAppearance.language) {
  languageSelect.value = currentAppearance.language;
}
syncPreview(currentAppearance.themeMode);
syncPreviewLabel();

document.querySelector("#toggle-settings-sidebar")?.addEventListener("click", () => {
  settingsApp?.classList.toggle("sidebar-hidden");
});

document.querySelectorAll(".settings-nav-row").forEach((row) => {
  row.addEventListener("click", () => {
    document
      .querySelectorAll(".settings-nav-row")
      .forEach((item) => item.classList.remove("active"));
    row.classList.add("active");
  });
});

document.querySelectorAll(".theme-card").forEach((card) => {
  card.addEventListener("click", () => {
    document.querySelectorAll(".theme-card").forEach((item) => {
      item.classList.remove("selected");
      item.setAttribute("aria-pressed", "false");
    });
    card.classList.add("selected");
    card.setAttribute("aria-pressed", "true");
    const theme = card.getAttribute("data-theme") ?? "dark";
    currentAppearance = appearance?.save({ themeMode: theme }) ?? currentAppearance;
    selectedTheme = theme === "light" ? "浅色" : theme === "system" ? "跟随系统" : "深色";
    syncPreview(theme);
    syncPreviewLabel();
  });
});

document.querySelectorAll(".accent-chip").forEach((chip) => {
  chip.addEventListener("click", () => {
    document.querySelectorAll(".accent-chip").forEach((item) => item.classList.remove("selected"));
    chip.classList.add("selected");
    const accent = chip.getAttribute("data-accent");
    const rgb = chip.getAttribute("data-rgb");
    if (accent)
      currentAppearance = appearance?.save({ accent, accentRgb: rgb }) ?? currentAppearance;
  });
});

document.querySelectorAll("#density-control button").forEach((button) => {
  button.addEventListener("click", () => {
    document
      .querySelectorAll("#density-control button")
      .forEach((item) => item.classList.remove("selected"));
    button.classList.add("selected");
    const density = button.getAttribute("data-density") ?? "default";
    settingsApp?.setAttribute("data-density", density);
    currentAppearance = appearance?.save({ density }) ?? currentAppearance;
    selectedDensity =
      density === "compact" ? "紧凑密度" : density === "comfortable" ? "宽松密度" : "默认密度";
    syncPreviewLabel();
  });
});

document.querySelectorAll(".demo-switch").forEach((control) => {
  control.addEventListener("click", () => {
    const checked = control.classList.toggle("checked");
    control.setAttribute("aria-checked", String(checked));
    const key =
      control.getAttribute("aria-label") === "减少动态效果"
        ? "reducedMotion"
        : "translucentSidebar";
    currentAppearance = appearance?.save({ [key]: checked }) ?? currentAppearance;
  });
});

languageSelect?.addEventListener("change", (event) => {
  const language = event.currentTarget.value;
  currentAppearance = appearance?.save({ language }) ?? currentAppearance;
});
