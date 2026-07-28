const icon = (name, className = "icon") =>
  `<svg class="${className}" aria-hidden="true"><use href="#icon-${name}"></use></svg>`;

const savedAppearance = window.HachimiAppearance?.read();
const lightAppearance = document.documentElement.dataset.appearanceTheme === "light";
const appearancePalette = lightAppearance
  ? {
      canvas: "#f8f7f3",
      surface: "#ffffff",
      control: "#ebe9e3",
      text: "#24272d",
      faint: "#626975",
      success: "#267a50",
      danger: "#c94f59",
    }
  : {
      canvas: "#111316",
      surface: "#191c20",
      control: "#252a31",
      text: "#f1f3f5",
      faint: "#858f9b",
      success: "#54b986",
      danger: "#ed747c",
    };
const appearanceAccent = savedAppearance?.accent ?? "#7062d5";

const customSelect = ({ id, options, selected = options[0], compact = false }) => `
  <div class="ui-select${compact ? " compact" : ""}" data-select>
    <button class="ui-select-trigger" id="${id}" type="button" aria-haspopup="listbox" aria-controls="${id}-options" aria-expanded="false">
      <span data-select-value>${selected}</span>${icon("chevron-down")}
    </button>
    <div class="ui-select-popover" id="${id}-options" role="listbox" aria-labelledby="${id}" hidden>
      ${options
        .map(
          (option) =>
            `<button class="ui-select-option" type="button" role="option" aria-selected="${String(option === selected)}" data-value="${option}"><span>${option}</span>${icon("check")}</button>`,
        )
        .join("")}
    </div>
  </div>
`;

document.body.innerHTML = `
  <svg width="0" height="0" style="position:absolute" aria-hidden="true">
    <symbol id="icon-panel" viewBox="0 0 24 24"><rect x="3" y="3" width="18" height="18" rx="2"/><path d="M9 3v18"/></symbol>
    <symbol id="icon-chevron-left" viewBox="0 0 24 24"><path d="m15 18-6-6 6-6"/></symbol>
    <symbol id="icon-chevron-right" viewBox="0 0 24 24"><path d="m9 18 6-6-6-6"/></symbol>
    <symbol id="icon-chevron-down" viewBox="0 0 24 24"><path d="m6 9 6 6 6-6"/></symbol>
    <symbol id="icon-minus" viewBox="0 0 24 24"><path d="M5 12h14"/></symbol>
    <symbol id="icon-square" viewBox="0 0 24 24"><rect x="5" y="5" width="14" height="14" rx="1"/></symbol>
    <symbol id="icon-x" viewBox="0 0 24 24"><path d="m6 6 12 12M18 6 6 18"/></symbol>
    <symbol id="icon-plus" viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></symbol>
    <symbol id="icon-search" viewBox="0 0 24 24"><circle cx="11" cy="11" r="7"/><path d="m20 20-4-4"/></symbol>
    <symbol id="icon-settings" viewBox="0 0 24 24"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .34 1.88l.06.06-2.83 2.83-.06-.06A1.7 1.7 0 0 0 15 19.4a1.7 1.7 0 0 0-1 .6 1.7 1.7 0 0 0-.4 1.1V21H9.6v-.1A1.7 1.7 0 0 0 8.5 19.4a1.7 1.7 0 0 0-1.88.34l-.06.06-2.83-2.83.06-.06A1.7 1.7 0 0 0 4.6 15a1.7 1.7 0 0 0-.6-1 1.7 1.7 0 0 0-1.1-.4H3V9.6h.1A1.7 1.7 0 0 0 4.6 8.5a1.7 1.7 0 0 0-.34-1.88l-.06-.06 2.83-2.83.06.06A1.7 1.7 0 0 0 9 4.6a1.7 1.7 0 0 0 1-.6 1.7 1.7 0 0 0 .4-1.1V3h4v.1A1.7 1.7 0 0 0 15.5 4.6a1.7 1.7 0 0 0 1.88-.34l.06-.06 2.83 2.83-.06.06A1.7 1.7 0 0 0 19.4 9c.4.3.7.6.8 1 .2.4.3.8.3 1.1v.1H21v4h-.1a1.7 1.7 0 0 0-1.5 1.1z"/></symbol>
    <symbol id="icon-palette" viewBox="0 0 24 24"><path d="M12 3a9 9 0 0 0 0 18h1.5a2 2 0 0 0 0-4H12a2 2 0 0 1 0-4h4a5 5 0 0 0 5-5c0-2.8-4-5-9-5z"/><circle cx="7.5" cy="10" r=".8"/><circle cx="9" cy="6.5" r=".8"/><circle cx="14" cy="6" r=".8"/><circle cx="17.5" cy="9" r=".8"/></symbol>
    <symbol id="icon-mouse" viewBox="0 0 24 24"><rect x="7" y="3" width="10" height="18" rx="5"/><path d="M12 3v6"/></symbol>
    <symbol id="icon-form" viewBox="0 0 24 24"><path d="M4 5h16M4 12h10M4 19h16"/><circle cx="18" cy="12" r="2"/></symbol>
    <symbol id="icon-navigation" viewBox="0 0 24 24"><path d="m4 4 16 7-7 3-3 6z"/></symbol>
    <symbol id="icon-bell" viewBox="0 0 24 24"><path d="M18 8a6 6 0 0 0-12 0c0 7-3 7-3 9h18c0-2-3-2-3-9M10 21h4"/></symbol>
    <symbol id="icon-bot" viewBox="0 0 24 24"><rect x="4" y="7" width="16" height="13" rx="2"/><path d="M9 3h6M12 3v4M8 12h.01M16 12h.01M8 16h8"/></symbol>
    <symbol id="icon-box" viewBox="0 0 24 24"><path d="m12 3 8 4.5v9L12 21l-8-4.5v-9zM4 7.5l8 4.5 8-4.5M12 12v9"/></symbol>
    <symbol id="icon-code" viewBox="0 0 24 24"><path d="m8 9-4 3 4 3M16 9l4 3-4 3M14 5l-4 14"/></symbol>
    <symbol id="icon-layout" viewBox="0 0 24 24"><rect x="3" y="3" width="18" height="18" rx="2"/><path d="M3 9h18M9 9v12"/></symbol>
    <symbol id="icon-spark" viewBox="0 0 24 24"><path d="m12 3 1.4 4.6L18 9l-4.6 1.4L12 15l-1.4-4.6L6 9l4.6-1.4z"/><path d="m19 15 .7 2.3L22 18l-2.3.7L19 21l-.7-2.3L16 18l2.3-.7z"/></symbol>
    <symbol id="icon-check" viewBox="0 0 24 24"><path d="m5 12 4 4L19 6"/></symbol>
    <symbol id="icon-alert" viewBox="0 0 24 24"><path d="M12 3 2 21h20zM12 9v5M12 18h.01"/></symbol>
    <symbol id="icon-info" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7h.01"/></symbol>
    <symbol id="icon-trash" viewBox="0 0 24 24"><path d="M4 7h16M9 7V4h6v3M7 7l1 14h8l1-14M10 11v6M14 11v6"/></symbol>
    <symbol id="icon-download" viewBox="0 0 24 24"><path d="M12 3v12M7 10l5 5 5-5M4 21h16"/></symbol>
    <symbol id="icon-more" viewBox="0 0 24 24"><circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/></symbol>
    <symbol id="icon-terminal" viewBox="0 0 24 24"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="m7 9 3 3-3 3M13 15h4"/></symbol>
    <symbol id="icon-shield" viewBox="0 0 24 24"><path d="M12 3 20 7v5c0 5-3.5 8-8 9-4.5-1-8-4-8-9V7z"/><path d="m9 12 2 2 4-4"/></symbol>
    <symbol id="icon-send" viewBox="0 0 24 24"><path d="m22 2-7 20-4-9-9-4z"/><path d="M22 2 11 13"/></symbol>
    <symbol id="icon-paperclip" viewBox="0 0 24 24"><path d="m20.5 11.5-8.6 8.6a6 6 0 0 1-8.5-8.5l9.2-9.2a4 4 0 0 1 5.7 5.7l-9.2 9.2a2 2 0 0 1-2.8-2.8l8.5-8.5"/></symbol>
    <symbol id="icon-folder" viewBox="0 0 24 24"><path d="M3 7.5h7l2 2h9v8.5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><path d="M3 9.5v-3a2 2 0 0 1 2-2h5l2 3"/></symbol>
    <symbol id="icon-file" viewBox="0 0 24 24"><path d="M6 2h8l4 4v16H6z"/><path d="M14 2v5h5"/></symbol>
    <symbol id="icon-branch" viewBox="0 0 24 24"><circle cx="6" cy="5" r="2"/><circle cx="6" cy="19" r="2"/><circle cx="18" cy="7" r="2"/><path d="M6 7v10M8 17c5 0 3-10 8-10"/></symbol>
    <symbol id="icon-volume" viewBox="0 0 24 24"><path d="M11 5 6 9H3v6h3l5 4zM15 9a4 4 0 0 1 0 6M18 6a8 8 0 0 1 0 12"/></symbol>
    <symbol id="icon-play" viewBox="0 0 24 24"><path d="m7 4 13 8-13 8z"/></symbol>
    <symbol id="icon-plug" viewBox="0 0 24 24"><path d="m8 12 8-8M14 3l7 7M5 11l8 8M3 14l7 7M8 17l-4 4"/></symbol>
    <symbol id="icon-tree" viewBox="0 0 24 24"><path d="M5 4v16M5 8h6M5 16h6M11 6h8v4h-8zM11 14h8v4h-8z"/></symbol>
  </svg>

  <div class="catalog-app" id="catalog-app">
    <header class="titlebar">
      <div class="titlebar-start"><button class="brand-menu" type="button" aria-label="Hachimi 菜单"><span class="mini-logo">H</span><span>Hachimi</span></button><button type="button" aria-label="后退">${icon("chevron-left")}</button><button type="button" aria-label="前进">${icon("chevron-right")}</button></div>
      <div class="titlebar-center"><strong>Design System</strong><span>·</span><span>方案 A / Quiet Graphite</span></div>
      <div class="titlebar-end"><button id="toggle-catalog-nav" type="button" aria-label="切换目录">${icon("panel")}</button><div class="window-controls" aria-hidden="true"><button type="button" tabindex="-1">${icon("minus")}</button><button type="button" tabindex="-1">${icon("square")}</button><button type="button" tabindex="-1">${icon("x")}</button></div></div>
    </header>

    <div class="catalog-workspace">
      <aside class="catalog-sidebar">
        <div class="catalog-brand"><span class="mini-logo">A</span><span class="catalog-brand-copy"><strong>组件与模式</strong><small>Hachimi UI 0.3 draft</small></span></div>
        <label class="catalog-search">${icon("search")}<input id="catalog-search" type="search" aria-label="搜索目录" placeholder="搜索组件…" /></label>
        <nav class="catalog-nav" aria-label="组件目录">
          <section class="catalog-nav-group"><h2>规范</h2><a class="active" href="#overview" data-keywords="总览 原则 overview">${icon("spark")}<span>总览</span></a><a href="#foundations" data-keywords="基础 颜色 字体 间距 圆角 tokens">${icon("palette")}<span>基础令牌</span></a></section>
          <section class="catalog-nav-group"><h2>基础组件</h2><a href="#actions" data-keywords="按钮 图标 操作 button">${icon("mouse")}<span>操作与按钮</span></a><a href="#forms" data-keywords="表单 输入 开关 选择 form input">${icon("form")}<span>表单控件</span></a><a href="#navigation" data-keywords="导航 标签 菜单 tabs menu">${icon("navigation")}<span>导航与菜单</span></a><a href="#feedback" data-keywords="反馈 徽标 提示 对话 toast dialog">${icon("bell")}<span>反馈与浮层</span></a></section>
          <section class="catalog-nav-group"><h2>业务模式</h2><a href="#agent" data-keywords="agent 消息 工具 审批 计划 composer">${icon("bot")}<span>Agent 工作流</span></a><a href="#resources" data-keywords="资源 模型 语音 动作 mcp 技能">${icon("box")}<span>资源与扩展</span></a><a href="#data-code" data-keywords="文件树 diff 编辑器 代码">${icon("code")}<span>数据与代码</span></a><a href="#settings-patterns" data-keywords="设置 行 卡片 settings">${icon("settings")}<span>设置模式</span></a></section>
          <section class="catalog-nav-group"><h2>组合</h2><a href="#templates" data-keywords="页面 工作台 设置 模板 layout">${icon("layout")}<span>页面模板</span></a></section>
        </nav>
        <div class="catalog-sidebar-footer"><span>方案覆盖状态</span><span class="status-dot"></span><strong>完整</strong></div>
      </aside>

      <main class="catalog-main">
        <header class="catalog-header"><div class="catalog-header-copy"><strong>Hachimi 统一组件规范</strong><small>同一套令牌覆盖桌面工作台、设置、资源管理与 Agent 工作流</small></div><div class="catalog-header-actions"><a class="ui-button ghost small" href="demo-01-quiet-graphite.html" aria-label="查看工作台页面">${icon("layout")}<span>工作台</span></a><a class="ui-button small" href="demo-01-settings.html" aria-label="查看设置页面">${icon("settings")}<span>设置页</span></a></div></header>
        <div class="catalog-scroll" id="catalog-scroll">
          <div class="catalog-content">
            <section class="catalog-hero" id="overview" data-section>
              <div><p class="catalog-eyebrow">Quiet Graphite · Component Library</p><h1>一套克制、清晰、适合长期工作的桌面生产力语言</h1><p>这里集中展示项目当前需要的基础组件、Agent 专用模式、资源管理模式与页面组合。所有样式统一使用 6 档字号、4px 间距网格、4 档圆角和单一强调色。</p></div>
              <aside class="catalog-hero-note"><header>${icon("spark")}<strong>方案 A 的核心取舍</strong></header><p>品牌色不再铺满界面，只用于选中、主要操作与焦点；背景层级承担信息分组，让长时间阅读更安静。</p></aside>
            </section>
            <div class="coverage-grid"><div class="coverage-card"><strong>10</strong><span>组件与业务分组</span></div><div class="coverage-card"><strong>6</strong><span>固定字号层级</span></div><div class="coverage-card"><strong>4</strong><span>表面与圆角层级</span></div><div class="coverage-card"><strong>2</strong><span>完整页面模板</span></div></div>

            <section class="catalog-section" id="foundations" data-section>
              <header class="catalog-section-heading"><div><h2>基础令牌</h2><p>业务组件只引用语义令牌，不直接决定颜色、字号、阴影和圆角。</p></div><span class="component-count">Color · Type · Space · Radius</span></header>
              <div class="spec-card wide"><header class="spec-card-header"><strong>语义颜色</strong><code>surface / content / state</code></header><div class="token-colors"><div class="color-token"><i style="--token-color:${appearancePalette.canvas}"></i><strong>Canvas</strong><code>${appearancePalette.canvas.toUpperCase()}</code></div><div class="color-token"><i style="--token-color:${appearancePalette.surface}"></i><strong>Surface</strong><code>${appearancePalette.surface.toUpperCase()}</code></div><div class="color-token"><i style="--token-color:${appearancePalette.control}"></i><strong>Control</strong><code>${appearancePalette.control.toUpperCase()}</code></div><div class="color-token"><i style="--token-color:var(--accent)"></i><strong>Accent</strong><code>${appearanceAccent.toUpperCase()}</code></div><div class="color-token"><i style="--token-color:${appearancePalette.text}"></i><strong>Text</strong><code>${appearancePalette.text.toUpperCase()}</code></div><div class="color-token"><i style="--token-color:${appearancePalette.faint}"></i><strong>Faint</strong><code>${appearancePalette.faint.toUpperCase()}</code></div><div class="color-token"><i style="--token-color:${appearancePalette.success}"></i><strong>Success</strong><code>${appearancePalette.success.toUpperCase()}</code></div><div class="color-token"><i style="--token-color:${appearancePalette.danger}"></i><strong>Danger</strong><code>${appearancePalette.danger.toUpperCase()}</code></div></div></div>
              <div class="spec-grid"><article class="spec-card"><header class="spec-card-header"><strong>排版层级</strong><code>12 / 13 / 14 / 16 / 20 / 28</code></header><div class="spec-demo"><div class="type-scale"><div class="type-row" style="--sample-size:28px"><span>页面标题</span><strong>外观设置</strong><code>28/35</code></div><div class="type-row" style="--sample-size:20px"><span>分区标题</span><strong>Agent 工作流</strong><code>20/25</code></div><div class="type-row" style="--sample-size:16px"><span>组件标题</span><strong>界面偏好</strong><code>16/20</code></div><div class="type-row" style="--sample-size:14px"><span>正文控件</span><strong>统一的信息密度</strong><code>14/21</code></div><div class="type-row" style="--sample-size:12px"><span>辅助信息</span><strong>所有更改已保存</strong><code>12/18</code></div></div></div></article>
                <article class="spec-card"><header class="spec-card-header"><strong>间距与圆角</strong><code>4px grid</code></header><div class="spec-demo column"><div class="space-scale"><div class="space-row"><span>space-1</span><i style="--sample-width:16px"></i><code>4px</code></div><div class="space-row"><span>space-2</span><i style="--sample-width:32px"></i><code>8px</code></div><div class="space-row"><span>space-3</span><i style="--sample-width:48px"></i><code>12px</code></div><div class="space-row"><span>space-4</span><i style="--sample-width:64px"></i><code>16px</code></div><div class="space-row"><span>space-5</span><i style="--sample-width:96px"></i><code>24px</code></div></div><div class="radius-scale"><div class="radius-token" style="--sample-radius:6px">6px</div><div class="radius-token" style="--sample-radius:10px">10px</div><div class="radius-token" style="--sample-radius:14px">14px</div><div class="radius-token" style="--sample-radius:18px">18px</div></div></div></article></div>
            </section>

            <section class="catalog-section" id="actions" data-section>
              <header class="catalog-section-heading"><div><h2>操作与按钮</h2><p>一个页面只保留一个视觉主操作；危险操作不与主操作共用颜色。</p></div><span class="component-count">Button · IconButton · Segmented · Dropdown</span></header>
              <div class="spec-grid"><article class="spec-card"><header class="spec-card-header"><strong>按钮变体</strong><code>default / primary / ghost / danger</code></header><div class="spec-demo"><button class="ui-button" type="button">默认按钮</button><button class="ui-button primary" type="button">${icon("spark")}主要操作</button><button class="ui-button ghost" type="button">幽灵按钮</button><button class="ui-button danger" type="button">${icon("trash")}删除</button><button class="ui-button" type="button" disabled>不可用</button></div></article>
                <article class="spec-card"><header class="spec-card-header"><strong>尺寸与图标按钮</strong><code>28 / 34 / 40</code></header><div class="spec-demo"><button class="ui-button small" type="button">小按钮</button><button class="ui-button" type="button">默认按钮</button><button class="ui-button large" type="button">大按钮</button><button class="ui-icon-button" type="button" aria-label="下载">${icon("download")}</button><button class="ui-icon-button ghost" type="button" aria-label="更多">${icon("more")}</button></div></article>
                <article class="spec-card"><header class="spec-card-header"><strong>分段控件</strong><code>single selection</code></header><div class="spec-demo"><div class="ui-segmented" data-segmented><button type="button">紧凑</button><button class="selected" type="button">默认</button><button type="button">宽松</button></div></div></article>
                <article class="spec-card"><header class="spec-card-header"><strong>下拉操作</strong><code>menu / separator / shortcut</code></header><div class="spec-demo"><div class="dropdown-wrap" id="action-dropdown"><button class="ui-button" id="dropdown-trigger" type="button" aria-expanded="false">更多操作 ${icon("chevron-down")}</button><div class="menu-demo dropdown-popover"><button class="menu-item" type="button">${icon("download")}导出主题<kbd>Ctrl E</kbd></button><button class="menu-item" type="button">${icon("file")}复制配置</button><i class="menu-separator"></i><button class="menu-item danger" type="button">${icon("trash")}删除配置</button></div></div></div></article></div>
            </section>

            <section class="catalog-section" id="forms" data-section>
              <header class="catalog-section-heading"><div><h2>表单控件</h2><p>输入组件统一 40px 高度；说明文字和错误信息始终占据稳定位置。</p></div><span class="component-count">Input · Select · Textarea · Switch · Range · Color</span></header>
              <article class="spec-card wide"><header class="spec-card-header"><strong>文本与选择输入</strong><code>label / control / description</code></header><div class="spec-demo"><div class="form-grid"><div class="field-stack"><label for="model-input">模型名称</label><input class="ui-input" id="model-input" value="gpt-5.6" /><small>用于新的 Agent 运行。</small></div><div class="field-stack"><label for="provider-select">服务提供方</label>${customSelect({ id: "provider-select", options: ["OpenAI Compatible", "Local Ollama"] })}<small>连接信息保存在本机。</small></div><div class="field-stack"><label for="api-input">API 地址</label><input class="ui-input invalid" id="api-input" value="http://localhost" aria-invalid="true" /><small class="field-error">地址必须包含版本路径，例如 /v1。</small></div><div class="field-stack"><label for="resource-search">搜索资源</label><div class="ui-search">${icon("search")}<input id="resource-search" type="search" placeholder="搜索模型、技能或文件…" /></div><small>按名称快速筛选当前资源。</small></div><div class="field-stack" style="grid-column:1/-1"><label for="prompt-input">系统提示</label><textarea class="ui-textarea" id="prompt-input" placeholder="输入 Agent 的默认行为说明…"></textarea><small>建议不超过 600 字。</small></div></div></div></article>
              <div class="spec-grid"><article class="spec-card"><header class="spec-card-header"><strong>选择与开关</strong><code>checkbox / radio / switch</code></header><div class="spec-demo column"><label class="ui-checkbox"><input type="checkbox" checked />启用工作区工具</label><label class="ui-checkbox"><input type="checkbox" />允许浏览器控制</label><label class="ui-radio"><input type="radio" name="policy" checked />按需批准</label><label class="ui-radio"><input type="radio" name="policy" />完全只读</label><div style="display:flex;align-items:center;justify-content:space-between"><span class="field-label">半透明侧栏</span><button class="ui-switch checked" type="button" role="switch" aria-checked="true" aria-label="半透明侧栏"><span></span></button></div></div></article>
                <article class="spec-card"><header class="spec-card-header"><strong>范围与颜色</strong><code>range / color field / file</code></header><div class="spec-demo column"><div class="range-wrap"><header class="range-header"><span>语音速度</span><output id="range-output">100%</output></header><input id="range-input" type="range" min="50" max="150" value="100" aria-label="语音速度" /></div><div class="field-stack"><span class="field-label">强调色</span><div style="display:flex;gap:8px;align-items:center"><input type="color" value="${appearanceAccent}" aria-label="强调色选择器" /><input class="ui-input" value="${appearanceAccent.toUpperCase()}" aria-label="强调色十六进制" /></div></div><button class="ui-button" type="button">${icon("paperclip")}选择本地文件</button></div></article></div>
            </section>

            <section class="catalog-section" id="navigation" data-section>
              <header class="catalog-section-heading"><div><h2>导航与菜单</h2><p>导航选中态使用表面变化，强调色只作为图标或小型状态提示。</p></div><span class="component-count">Tabs · Breadcrumb · SidebarNav · Menu</span></header>
              <div class="spec-grid"><article class="spec-card"><header class="spec-card-header"><strong>标签页</strong><code>horizontal tabs</code></header><div class="spec-demo"><div class="ui-tabs" id="tabs-demo"><div class="ui-tab-list"><button class="selected" type="button" data-tab="files">文件</button><button type="button" data-tab="changes">更改</button><button type="button" data-tab="search">搜索</button></div><div class="ui-tab-panel" data-tab-panel>显示工作区文件树和当前文件预览。</div></div></div></article>
                <article class="spec-card"><header class="spec-card-header"><strong>面包屑与快捷导航</strong><code>hierarchy / current</code></header><div class="spec-demo column"><nav class="breadcrumb" aria-label="文件路径"><span>hachimi-code</span>${icon("chevron-right")}<span>packages</span>${icon("chevron-right")}<strong>workbench</strong></nav><div class="menu-demo" style="width:100%"><button class="menu-item highlighted" type="button">${icon("plus")}新建任务<kbd>Ctrl N</kbd></button><button class="menu-item" type="button">${icon("search")}搜索任务<kbd>Ctrl K</kbd></button><button class="menu-item" type="button">${icon("settings")}打开设置<kbd>Ctrl ,</kbd></button></div></div></article>
                <article class="spec-card wide"><header class="spec-card-header"><strong>侧栏导航模式</strong><code>project / session / settings</code></header><div class="spec-demo"><div style="width:240px;display:grid;gap:3px"><button class="settings-nav-row active" type="button">${icon("folder")}hachimi-code</button><button class="settings-nav-row" type="button">${icon("bot")}Agent 与工具</button><button class="settings-nav-row" type="button">${icon("settings")}设置</button></div><div style="width:280px;display:grid;gap:3px"><button class="settings-nav-row active" type="button">${icon("palette")}外观</button><button class="settings-nav-row" type="button">${icon("volume")}语音</button><button class="settings-nav-row" type="button">${icon("box")}桌宠与动作</button></div></div></article><article class="spec-card wide"><header class="spec-card-header"><strong>快速开始卡片</strong><code>prompt card / suggested action</code></header><div class="spec-demo"><div class="prompt-grid"><button class="prompt-card-demo" type="button">${icon("bot")}<strong>连接大语言模型</strong><small>配置兼容服务与默认模型。</small></button><button class="prompt-card-demo" type="button">${icon("box")}<strong>添加 3D 角色</strong><small>导入并检查本地 VRM 资源。</small></button><button class="prompt-card-demo" type="button">${icon("volume")}<strong>配置本地语音</strong><small>管理 TTS 模型与试听状态。</small></button></div></div></article></div>
            </section>

            <section class="catalog-section" id="feedback" data-section>
              <header class="catalog-section-heading"><div><h2>反馈与浮层</h2><p>临时反馈、需要注意和阻塞操作使用不同强度，避免所有状态都抢夺注意力。</p></div><span class="component-count">Badge · Banner · Progress · Toast · Dialog · Tooltip</span></header>
              <div class="spec-grid"><article class="spec-card"><header class="spec-card-header"><strong>徽标与状态</strong><code>neutral / info / success / warning / danger</code></header><div class="spec-demo"><div class="badge-row"><span class="ui-badge">默认</span><span class="ui-badge info">运行中</span><span class="ui-badge success">已连接</span><span class="ui-badge warning">需批准</span><span class="ui-badge danger">失败</span></div></div></article>
                <article class="spec-card"><header class="spec-card-header"><strong>进度与加载</strong><code>determinate progress</code></header><div class="spec-demo"><div class="progress-stack"><div class="progress-item"><header><span>下载语音模型</span><span>68%</span></header><div class="progress-track"><i style="--progress:68%"></i></div></div><div class="progress-item"><header><span>扫描技能目录</span><span>完成</span></header><div class="progress-track"><i style="--progress:100%"></i></div></div></div></div></article>
                <article class="spec-card"><header class="spec-card-header"><strong>状态横幅</strong><code>info / warning</code></header><div class="spec-demo column"><div class="ui-alert info">${icon("info")}<span>设置已保存，新任务将使用更新后的模型。</span></div><div class="ui-alert warning">${icon("alert")}<span>Windows Sandbox 尚未通过运行时验证，写入工具暂不可用。</span></div></div></article>
                <article class="spec-card"><header class="spec-card-header"><strong>浮层与对话框</strong><code>toast / dialog / tooltip / popover</code></header><div class="spec-demo"><button class="ui-button" id="show-toast" type="button">显示 Toast</button><button class="ui-button primary" id="show-dialog" type="button">打开对话框</button><button class="ui-icon-button" type="button" aria-label="更多信息" title="悬浮提示：查看组件说明">${icon("info")}</button></div></article></div>
            </section>

            <section class="catalog-section" id="agent" data-section>
              <header class="catalog-section-heading"><div><h2>Agent 工作流</h2><p>对话正文保持低装饰；工具、审批、计划和用户输入使用可识别但克制的容器。</p></div><span class="component-count">Message · ToolCall · Approval · Plan · UserInput · Composer</span></header>
              <div class="spec-grid"><article class="spec-card"><header class="spec-card-header"><strong>消息与工具执行</strong><code>transcript / tool event</code></header><div class="spec-demo top"><div class="agent-thread"><article class="agent-message user"><div class="agent-message-meta"><span class="avatar">M</span><strong>你</strong><span>10:24</span></div><p>请统一前端所有组件的视觉规范，并提供完整预览。</p></article><article class="agent-message"><div class="agent-message-meta"><span class="mini-logo">H</span><strong>Hachimi</strong><span>正在分析</span></div><p>我会先盘点基础组件与业务模式，再用同一套令牌进行组合。</p></article><div class="tool-call open"><button class="tool-call-toggle" type="button" aria-expanded="true">${icon("terminal")}<strong>检查前端组件</strong><span class="ui-badge success">完成</span><small>0.8s</small></button><div class="tool-call-details">rg data-component packages/ui/src<br />42 shared patterns discovered</div></div></div></div></article>
                <article class="spec-card"><header class="spec-card-header"><strong>审批、计划与用户输入</strong><code>blocking agent states</code></header><div class="spec-demo column"><article class="agent-card approval"><header><span>${icon("shield")}<strong>需要批准</strong></span><span class="ui-badge warning">命令执行</span></header><p>Agent 请求运行 <code>pnpm test</code>，仅访问当前工作区。</p><footer class="agent-card-actions"><button class="ui-button small" type="button">拒绝</button><button class="ui-button primary small" type="button">批准一次</button></footer></article><article class="agent-card plan"><header><span>${icon("spark")}<strong>实施计划</strong></span><span class="ui-badge info">3 步</span></header><p>统一令牌 → 迁移基础组件 → 更新业务页面与视觉测试。</p><footer class="agent-card-actions"><button class="ui-button primary small" type="button">执行计划</button></footer></article><div class="field-stack"><label for="agent-input">选择迁移范围</label>${customSelect({ id: "agent-input", options: ["全部前端组件（推荐）", "仅工作台"] })}<small>回答只提供给当前运行。</small></div></div></article>
                <article class="spec-card wide"><header class="spec-card-header"><strong>Composer 与附件</strong><code>context / attachment / approval / send</code></header><div class="spec-demo"><div class="composer-demo-stage"><div class="composer-notice">${icon("alert")}<span>Windows Sandbox 尚未通过运行时验证，写入与命令执行将安全拒绝。</span></div><div class="composer-context-row"><button type="button">${icon("folder")}<span>hachimi-code</span>${icon("chevron-down")}</button><button type="button">${icon("terminal")}<span>本地执行</span>${icon("chevron-down")}</button></div><div class="composer-demo"><div class="attachment-list"><article class="attachment-card"><span class="attachment-file-icon">${icon("file")}</span><span class="attachment-copy"><strong>ui-audit.md</strong><small>MD · 4.8 KB</small></span><button type="button" aria-label="移除 ui-audit.md">${icon("x")}</button></article><article class="attachment-card image"><span class="attachment-preview" aria-hidden="true"><i>H</i></span><button type="button" aria-label="移除图片附件">${icon("x")}</button></article></div><textarea aria-label="任务输入" placeholder="描述你希望 Hachimi 完成的任务…"></textarea><footer><div class="composer-actions"><button class="composer-icon-button" type="button" aria-label="添加附件">${icon("plus")}</button><button class="composer-policy" type="button">${icon("shield")}<span>按需批准</span>${icon("chevron-down")}</button></div><button class="composer-send" type="button" aria-label="发送任务">${icon("send")}</button></footer></div><small class="composer-hint">上下文位于输入框外；附件、正文和执行策略在输入框内形成稳定层级。</small></div></div></article></div>
            </section>

            <section class="catalog-section" id="resources" data-section>
              <header class="catalog-section-heading"><div><h2>资源与扩展</h2><p>模型、语音、动作、MCP 与技能共享资源卡骨架，通过元数据和状态标签区分领域。</p></div><span class="component-count">ResourceCard · MotionCard · MCPTool · SkillTree</span></header>
              <div class="spec-grid"><article class="spec-card"><header class="spec-card-header"><strong>模型、角色与语音资源</strong><code>current / runtime / metadata</code></header><div class="spec-demo top"><div class="resource-list-demo"><article class="resource-card-demo current"><div class="resource-card-body"><span class="resource-icon">${icon("bot")}</span><span class="resource-copy"><strong>gpt-5.6</strong><span>OpenAI Compatible · 128k context</span></span><span class="ui-badge success">当前</span></div></article><article class="resource-card-demo"><div class="resource-card-body"><span class="resource-icon">${icon("box")}</span><span class="resource-copy"><strong>Mimi</strong><span>VRM 1.0 · Runtime Ready · 42.8k tris</span></span><span class="ui-badge success">当前角色</span></div></article><article class="resource-card-demo"><div class="resource-card-body"><span class="resource-icon">${icon("volume")}</span><span class="resource-copy"><strong>Hachimi 中英女声</strong><span>MeloTTS · 44.1 kHz · 本地</span></span><button class="ui-button small" type="button">试听</button></div></article></div></div></article>
                <article class="spec-card"><header class="spec-card-header"><strong>动作资源</strong><code>motion / binding / playback</code></header><div class="spec-demo top"><div class="motion-list"><article class="motion-card"><header><span>${icon("play")}<strong>标准待机</strong></span><span class="ui-badge">内置锁定</span></header><div class="motion-meta"><span>Idle</span><span>11.6s</span><span>全身</span><span>30 fingers</span></div><div class="agent-card-actions"><button class="ui-button ghost small" type="button">预览</button><button class="ui-button small" type="button">编辑绑定</button></div></article><article class="motion-card"><header><span>${icon("play")}<strong>挥手问候</strong></span><span class="ui-badge info">用户</span></header><div class="motion-meta"><span>Gesture</span><span>2.4s</span><span>右手</span></div></article></div></div></article>
                <article class="spec-card"><header class="spec-card-header"><strong>MCP 服务与工具</strong><code>server / exposure / schema</code></header><div class="spec-demo top"><div class="extension-list-demo"><article class="extension-card"><header><span>${icon("plug")}<strong>filesystem</strong></span><span class="ui-badge success">已连接</span></header><div class="extension-meta"><span>stdio</span><span>5 tools</span><span>按需批准</span></div></article><article class="extension-card"><header><span>${icon("terminal")}<strong>read_file</strong></span><button class="ui-switch checked" type="button" role="switch" aria-label="公开 read_file 工具" aria-checked="true"><span></span></button></header><div class="extension-meta"><span>读取工作区内的文本文件</span></div></article></div></div></article>
                <article class="spec-card"><header class="spec-card-header"><strong>技能树</strong><code>tree / diagnostics / controls</code></header><div class="spec-demo top"><div class="extension-list-demo"><div class="tree-row selected">${icon("tree")}内置技能</div><div class="tree-row indent">${icon("file")}documents <span class="ui-badge success" style="margin-left:auto">有效</span></div><div class="tree-row indent">${icon("file")}pdf <span class="ui-badge success" style="margin-left:auto">有效</span></div><div class="tree-row">${icon("tree")}个人技能</div><div class="tree-row indent">${icon("file")}office-helper <span class="ui-badge warning" style="margin-left:auto">1 个诊断</span></div></div></div></article></div>
            </section>

            <section class="catalog-section" id="data-code" data-section>
              <header class="catalog-section-heading"><div><h2>数据与代码</h2><p>文件树、编辑器、Diff 和诊断指标使用更紧凑的信息密度，但不低于 12px。</p></div><span class="component-count">FileTree · Editor · Diff · Metrics · Search</span></header>
              <article class="spec-card wide"><header class="spec-card-header"><strong>工作区浏览器与 Diff</strong><code>tree / viewer / added / removed</code></header><div class="spec-demo"><div class="split-demo"><aside class="tree-panel"><div class="tree-row">${icon("folder")}packages</div><div class="tree-row indent">${icon("folder")}ui</div><div class="tree-row indent selected">${icon("file")}tokens.css</div><div class="tree-row indent">${icon("file")}button.tsx</div><div class="tree-row">${icon("folder")}workbench</div><div class="tree-row">${icon("file")}package.json</div></aside><section class="code-panel"><header class="code-header"><span>packages/ui/tokens.css</span><span>+3 −2</span></header><div class="code-body"><div class="code-line"><span>18</span><code>:root {</code></div><div class="code-line removed"><span>19</span><code>- --radius-card: 18px;</code></div><div class="code-line added"><span>19</span><code>+ --radius-card: 14px;</code></div><div class="code-line added"><span>20</span><code>+ --text-body: 14px;</code></div><div class="code-line added"><span>21</span><code>+ --surface-selected: rgb(112 98 213 / 12%);</code></div><div class="code-line"><span>22</span><code>}</code></div></div></section></div></div></article>
              <div class="spec-grid"><article class="spec-card"><header class="spec-card-header"><strong>运行指标</strong><code>motion / model diagnostics</code></header><div class="spec-demo"><div class="metric-grid"><div class="metric-card"><span>活动骨骼</span><strong>42</strong></div><div class="metric-card"><span>帧率</span><strong>60</strong></div><div class="metric-card"><span>时长</span><strong>2.4s</strong></div><div class="metric-card"><span>警告</span><strong>0</strong></div></div></div></article><article class="spec-card"><header class="spec-card-header"><strong>搜索与空状态</strong><code>query / empty / refresh</code></header><div class="spec-demo column"><label class="ui-search">${icon("search")}<input aria-label="搜索工作区" placeholder="在工作区中搜索…" /></label><div class="ui-alert">${icon("search")}<span>没有匹配 “legacy-radius” 的结果。尝试更短的关键词。</span></div><button class="ui-button small" type="button">刷新索引</button></div></article></div>
            </section>

            <section class="catalog-section" id="settings-patterns" data-section>
              <header class="catalog-section-heading"><div><h2>设置模式</h2><p>页面只保留分区和设置组两层容器，单个设置行不重复套卡片。</p></div><span class="component-count">SettingsSection · SettingsRow · ThemeCard</span></header>
              <div class="spec-grid"><article class="spec-card wide"><header class="spec-card-header"><strong>设置组与设置行</strong><code>label / description / control</code></header><div class="spec-demo"><div class="settings-group-demo"><div class="settings-row-gallery"><span><strong>界面密度</strong><small>改变列表和设置行高度，不缩小正文。</small></span><div class="ui-segmented" data-segmented><button type="button">紧凑</button><button class="selected" type="button">默认</button><button type="button">宽松</button></div></div><div class="settings-row-gallery"><span><strong>半透明侧栏</strong><small>在支持的桌面环境中使用轻量背景模糊。</small></span><button class="ui-switch checked" type="button" role="switch" aria-label="设置模式半透明侧栏" aria-checked="true"><span></span></button></div><div class="settings-row-gallery"><span><strong>界面语言</strong><small>影响导航、设置和 Agent 系统消息。</small></span>${customSelect({ id: "settings-language", options: ["简体中文", "English"], compact: true })}</div></div></div></article>
                <article class="spec-card"><header class="spec-card-header"><strong>主题卡</strong><code>idle / selected</code></header><div class="spec-demo"><button class="theme-card" type="button"><span class="theme-thumbnail thumbnail-light"><span></span><span></span><span></span></span><strong>浅色</strong></button><button class="theme-card selected" type="button"><span class="theme-thumbnail thumbnail-dark"><span></span><span></span><span></span></span><strong>深色</strong></button></div></article><article class="spec-card"><header class="spec-card-header"><strong>资源导入</strong><code>file / inspection / confirmation</code></header><div class="spec-demo column"><div class="ui-alert info">${icon("info")}选择文件后先执行本地兼容性检查。</div><button class="ui-button" type="button">${icon("paperclip")}选择 VRM 模型</button><button class="ui-button primary" type="button" disabled>确认导入</button></div></article></div>
            </section>

            <section class="catalog-section" id="templates" data-section>
              <header class="catalog-section-heading"><div><h2>页面模板</h2><p>用同一套组件和令牌组合出完整工作台与设置界面。</p></div><span class="component-count">Workbench · Settings</span></header>
              <div class="page-template-grid"><a class="page-template" href="demo-01-quiet-graphite.html"><div class="page-template-preview"><i class="template-side"></i><span class="template-main"><i></i><i></i><b></b></span><i class="template-rail"></i></div><span class="page-template-copy"><strong>Agent 工作台</strong><span>项目导航、消息流、工具执行、右侧上下文与 Composer。</span></span></a><a class="page-template" href="demo-01-settings.html"><div class="page-template-preview settings"><i class="template-side"></i><span class="template-main"><i></i><i></i><b></b></span></div><span class="page-template-copy"><strong>设置界面</strong><span>设置导航、主题配置、表单行和实时预览。</span></span></a></div>
            </section>
          </div>
        </div>
      </main>
    </div>
  </div>

  <div class="catalog-toast" id="catalog-toast" role="status">设置已保存</div>
  <div class="modal-overlay" id="modal-overlay" role="presentation">
    <section class="inline-dialog" role="dialog" aria-modal="true" aria-labelledby="dialog-title" aria-describedby="dialog-description">
      <header><div><h3 id="dialog-title">重置主题设置？</h3></div><button class="ui-icon-button ghost" id="dialog-close" type="button" aria-label="关闭对话框">${icon("x")}</button></header>
      <p id="dialog-description">颜色、密度和界面偏好将恢复为 Quiet Graphite 默认值，此操作不会影响工作区数据。</p>
      <footer class="dialog-actions"><button class="ui-button" id="dialog-cancel" type="button">取消</button><button class="ui-button primary" id="dialog-confirm" type="button">确认重置</button></footer>
    </section>
  </div>
`;

const catalogApp = document.querySelector("#catalog-app");
const catalogScroll = document.querySelector("#catalog-scroll");
const navLinks = [...document.querySelectorAll(".catalog-nav a")];
const toast = document.querySelector("#catalog-toast");
const modal = document.querySelector("#modal-overlay");
let lastFocusedElement;

const showToast = (message = "设置已保存") => {
  if (!toast) return;
  toast.textContent = message;
  toast.classList.add("visible");
  window.setTimeout(() => toast.classList.remove("visible"), 1600);
};

const closeDialog = () => {
  modal?.classList.remove("open");
  if (lastFocusedElement instanceof HTMLElement) lastFocusedElement.focus();
};

document.querySelector("#toggle-catalog-nav")?.addEventListener("click", () => {
  catalogApp?.classList.toggle("nav-hidden");
});

document.querySelector("#catalog-search")?.addEventListener("input", (event) => {
  const query = event.currentTarget.value.trim().toLocaleLowerCase();
  for (const link of navLinks) {
    const text =
      `${link.textContent} ${link.getAttribute("data-keywords") ?? ""}`.toLocaleLowerCase();
    link.hidden = Boolean(query) && !text.includes(query);
  }
});

navLinks.forEach((link) => {
  link.addEventListener("click", () => {
    navLinks.forEach((item) => item.classList.remove("active"));
    link.classList.add("active");
  });
});

if (catalogScroll && "IntersectionObserver" in window) {
  const observer = new IntersectionObserver(
    (entries) => {
      const visible = entries
        .filter((entry) => entry.isIntersecting)
        .sort((left, right) => right.intersectionRatio - left.intersectionRatio)[0];
      if (!visible) return;
      navLinks.forEach((link) =>
        link.classList.toggle("active", link.hash === `#${visible.target.id}`),
      );
    },
    { root: catalogScroll, rootMargin: "-15% 0px -70%", threshold: [0, 0.2, 0.6] },
  );
  document.querySelectorAll("[data-section]").forEach((section) => observer.observe(section));
}

document.querySelectorAll("[data-segmented]").forEach((control) => {
  control.querySelectorAll("button").forEach((button) => {
    button.addEventListener("click", () => {
      control.querySelectorAll("button").forEach((item) => item.classList.remove("selected"));
      button.classList.add("selected");
    });
  });
});

document.querySelectorAll(".ui-switch").forEach((control) => {
  control.addEventListener("click", () => {
    const checked = control.classList.toggle("checked");
    control.setAttribute("aria-checked", String(checked));
  });
});

const closeCustomSelect = (select, restoreFocus = false) => {
  const trigger = select.querySelector(".ui-select-trigger");
  const popover = select.querySelector(".ui-select-popover");
  select.classList.remove("open");
  trigger?.setAttribute("aria-expanded", "false");
  if (popover instanceof HTMLElement) popover.hidden = true;
  if (restoreFocus && trigger instanceof HTMLElement) trigger.focus();
};

const openCustomSelect = (select, focusSelected = false) => {
  document.querySelectorAll("[data-select].open").forEach((item) => {
    if (item !== select) closeCustomSelect(item);
  });
  const trigger = select.querySelector(".ui-select-trigger");
  const popover = select.querySelector(".ui-select-popover");
  select.classList.add("open");
  trigger?.setAttribute("aria-expanded", "true");
  if (popover instanceof HTMLElement) popover.hidden = false;
  if (focusSelected) {
    const selected = select.querySelector('[role="option"][aria-selected="true"]');
    (selected ?? select.querySelector('[role="option"]'))?.focus();
  }
};

document.querySelectorAll("[data-select]").forEach((select) => {
  const trigger = select.querySelector(".ui-select-trigger");
  const options = [...select.querySelectorAll('[role="option"]')];

  trigger?.addEventListener("click", () => {
    if (select.classList.contains("open")) closeCustomSelect(select);
    else openCustomSelect(select);
  });

  trigger?.addEventListener("keydown", (event) => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    openCustomSelect(select, true);
  });

  options.forEach((option, index) => {
    option.addEventListener("click", () => {
      options.forEach((item) => item.setAttribute("aria-selected", "false"));
      option.setAttribute("aria-selected", "true");
      const value = select.querySelector("[data-select-value]");
      if (value) value.textContent = option.dataset.value ?? option.textContent.trim();
      closeCustomSelect(select, true);
    });
    option.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        event.preventDefault();
        closeCustomSelect(select, true);
        return;
      }
      if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
      event.preventDefault();
      let nextIndex = index;
      if (event.key === "ArrowDown") nextIndex = (index + 1) % options.length;
      if (event.key === "ArrowUp") nextIndex = (index - 1 + options.length) % options.length;
      if (event.key === "Home") nextIndex = 0;
      if (event.key === "End") nextIndex = options.length - 1;
      options[nextIndex]?.focus();
    });
  });
});

document.addEventListener("click", (event) => {
  if (event.target.closest?.("[data-select]")) return;
  document.querySelectorAll("[data-select].open").forEach((select) => closeCustomSelect(select));
});

const rangeInput = document.querySelector("#range-input");
rangeInput?.addEventListener("input", () => {
  const output = document.querySelector("#range-output");
  if (output) output.textContent = `${rangeInput.value}%`;
});

document.querySelectorAll(".ui-tab-list button").forEach((button) => {
  button.addEventListener("click", () => {
    document
      .querySelectorAll(".ui-tab-list button")
      .forEach((item) => item.classList.remove("selected"));
    button.classList.add("selected");
    const panels = {
      files: "显示工作区文件树和当前文件预览。",
      changes: "显示本次运行产生的文件更改与 Diff。",
      search: "跨当前工作区搜索文件名与文本内容。",
    };
    const panel = document.querySelector("[data-tab-panel]");
    if (panel) panel.textContent = panels[button.dataset.tab] ?? panels.files;
  });
});

document.querySelector("#dropdown-trigger")?.addEventListener("click", (event) => {
  const dropdown = document.querySelector("#action-dropdown");
  const open = dropdown?.classList.toggle("open") ?? false;
  event.currentTarget.setAttribute("aria-expanded", String(open));
});

document.querySelectorAll(".tool-call-toggle").forEach((button) => {
  button.addEventListener("click", () => {
    const toolCall = button.closest(".tool-call");
    const open = toolCall?.classList.toggle("open") ?? false;
    button.setAttribute("aria-expanded", String(open));
  });
});

document.querySelector("#show-toast")?.addEventListener("click", () => showToast());

document.querySelector("#show-dialog")?.addEventListener("click", (event) => {
  lastFocusedElement = event.currentTarget;
  modal?.classList.add("open");
  document.querySelector("#dialog-close")?.focus();
});

document.querySelector("#dialog-close")?.addEventListener("click", closeDialog);
document.querySelector("#dialog-cancel")?.addEventListener("click", closeDialog);
document.querySelector("#dialog-confirm")?.addEventListener("click", () => {
  closeDialog();
  showToast("主题设置已恢复");
});

modal?.addEventListener("click", (event) => {
  if (event.target === modal) closeDialog();
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    closeDialog();
    const dropdown = document.querySelector("#action-dropdown");
    dropdown?.classList.remove("open");
    document.querySelector("#dropdown-trigger")?.setAttribute("aria-expanded", "false");
    document
      .querySelectorAll("[data-select].open")
      .forEach((select) => closeCustomSelect(select, true));
  }
});
