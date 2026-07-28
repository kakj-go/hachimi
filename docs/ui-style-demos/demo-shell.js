const concept = document.documentElement.dataset.concept ?? "graphite";

const concepts = {
  graphite: {
    label: "A · 静谧石墨",
    note: "推荐",
    accent: "#7062D5",
    background: "#111316",
    surface: "#191C20",
  },
  mist: {
    label: "B · 雾紫 Hachimi",
    note: "品牌感",
    accent: "#C5A7FF",
    background: "#16141A",
    surface: "#1E1B23",
  },
  paper: {
    label: "C · 纸白专注",
    note: "办公感",
    accent: "#4358C5",
    background: "#F8F7F3",
    surface: "#FFFFFF",
  },
};

const savedAppearance = concept === "graphite" ? window.HachimiAppearance?.read() : undefined;
const resolvedTheme = savedAppearance
  ? window.HachimiAppearance?.resolveTheme(savedAppearance.themeMode)
  : concept === "paper"
    ? "light"
    : "dark";
const current = {
  ...(concepts[concept] ?? concepts.graphite),
  ...(savedAppearance
    ? {
        accent: savedAppearance.accent.toUpperCase(),
        background: resolvedTheme === "light" ? "#F8F7F3" : "#111316",
        surface: resolvedTheme === "light" ? "#FFFFFF" : "#191C20",
      }
    : {}),
};

const icon = (name, className = "icon") =>
  `<svg class="${className}" aria-hidden="true"><use href="#icon-${name}"></use></svg>`;

const shell = `
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
    <symbol id="icon-folder" viewBox="0 0 24 24"><path d="M3 7.5h7l2 2h9v8.5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><path d="M3 9.5v-3a2 2 0 0 1 2-2h5l2 3"/></symbol>
    <symbol id="icon-settings" viewBox="0 0 24 24"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .34 1.88l.06.06-2.83 2.83-.06-.06A1.7 1.7 0 0 0 15 19.4a1.7 1.7 0 0 0-1 .6 1.7 1.7 0 0 0-.4 1.1V21H9.6v-.1A1.7 1.7 0 0 0 8.5 19.4a1.7 1.7 0 0 0-1.88.34l-.06.06-2.83-2.83.06-.06A1.7 1.7 0 0 0 4.6 15a1.7 1.7 0 0 0-.6-1 1.7 1.7 0 0 0-1.1-.4H3V9.6h.1A1.7 1.7 0 0 0 4.6 8.5a1.7 1.7 0 0 0-.34-1.88l-.06-.06 2.83-2.83.06.06A1.7 1.7 0 0 0 9 4.6a1.7 1.7 0 0 0 1-.6 1.7 1.7 0 0 0 .4-1.1V3h4v.1A1.7 1.7 0 0 0 15.5 4.6a1.7 1.7 0 0 0 1.88-.34l.06-.06 2.83 2.83-.06.06A1.7 1.7 0 0 0 19.4 9c.4.3.7.6.8 1 .2.4.3.8.3 1.1v.1H21v4h-.1a1.7 1.7 0 0 0-1.5 1.1z"/></symbol>
    <symbol id="icon-branch" viewBox="0 0 24 24"><circle cx="6" cy="5" r="2"/><circle cx="6" cy="19" r="2"/><circle cx="18" cy="7" r="2"/><path d="M6 7v10M8 17c5 0 3-10 8-10"/></symbol>
    <symbol id="icon-shield" viewBox="0 0 24 24"><path d="M12 3 20 7v5c0 5-3.5 8-8 9-4.5-1-8-4-8-9V7z"/><path d="m9 12 2 2 4-4"/></symbol>
    <symbol id="icon-paperclip" viewBox="0 0 24 24"><path d="m20.5 11.5-8.6 8.6a6 6 0 0 1-8.5-8.5l9.2-9.2a4 4 0 0 1 5.7 5.7l-9.2 9.2a2 2 0 0 1-2.8-2.8l8.5-8.5"/></symbol>
    <symbol id="icon-send" viewBox="0 0 24 24"><path d="m22 2-7 20-4-9-9-4z"/><path d="M22 2 11 13"/></symbol>
    <symbol id="icon-spark" viewBox="0 0 24 24"><path d="m12 3 1.4 4.6L18 9l-4.6 1.4L12 15l-1.4-4.6L6 9l4.6-1.4z"/><path d="m19 15 .7 2.3L22 18l-2.3.7L19 21l-.7-2.3L16 18l2.3-.7z"/></symbol>
    <symbol id="icon-terminal" viewBox="0 0 24 24"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="m7 9 3 3-3 3M13 15h4"/></symbol>
    <symbol id="icon-file" viewBox="0 0 24 24"><path d="M6 2h8l4 4v16H6z"/><path d="M14 2v5h5"/></symbol>
    <symbol id="icon-alert" viewBox="0 0 24 24"><path d="M12 3 2 21h20zM12 9v5M12 18h.01"/></symbol>
    <symbol id="icon-check" viewBox="0 0 24 24"><path d="m5 12 4 4L19 6"/></symbol>
    <symbol id="icon-layout" viewBox="0 0 24 24"><rect x="3" y="3" width="18" height="18" rx="2"/><path d="M3 9h18M15 9v12"/></symbol>
    <symbol id="icon-command" viewBox="0 0 24 24"><path d="M18 9a3 3 0 1 0-3-3v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3V6a3 3 0 1 0-3 3z"/></symbol>
  </svg>
  <div class="app" id="app">
    <header class="titlebar">
      <div class="titlebar-start">
        <button class="brand-menu" type="button" aria-label="Hachimi 菜单">
          <span class="mini-logo">H</span>
          <span>Hachimi</span>
        </button>
        <button type="button" aria-label="后退">${icon("chevron-left")}</button>
        <button type="button" aria-label="前进">${icon("chevron-right")}</button>
      </div>
      <div class="titlebar-center">
        <strong>统一前端视觉规范与组件样式</strong>
        <span>·</span>
        <span>hachimi-code</span>
      </div>
      <div class="titlebar-end">
        <button id="toggle-sidebar" type="button" aria-label="切换侧边栏">${icon("panel")}</button>
        <button id="toggle-rail" type="button" aria-label="切换上下文面板">${icon("layout")}</button>
        <div class="window-controls" aria-hidden="true">
          <button type="button" tabindex="-1">${icon("minus")}</button>
          <button type="button" tabindex="-1">${icon("square")}</button>
          <button type="button" tabindex="-1">${icon("x")}</button>
        </div>
      </div>
    </header>

    <div class="workspace">
      <aside class="sidebar">
        <div class="sidebar-top">
          <div class="sidebar-title"><span class="mini-logo">H</span><span>工作台</span></div>
          <button class="icon-button" type="button" aria-label="搜索">${icon("search")}</button>
        </div>
        <button class="new-task" type="button">${icon("plus")}<span>新建任务</span></button>
        <div class="sidebar-scroll">
          <section class="nav-section">
            <h2 class="section-kicker">项目</h2>
            <button class="nav-row active" type="button">
              ${icon("folder")}<span>hachimi-code</span>${icon("chevron-down", "icon chevron")}
            </button>
            <div class="thread-list">
              <button class="thread-row active" type="button"><span>统一前端视觉规范与组件样式</span></button>
              <button class="thread-row" type="button"><span>完善 Agent 上下文压缩策略</span></button>
              <button class="thread-row" type="button"><span>优化桌面控制权限提示</span></button>
              <button class="thread-row" type="button"><span>VRM 动作库交互评审</span></button>
            </div>
          </section>
          <section class="nav-section">
            <h2 class="section-kicker">快捷入口</h2>
            <button class="nav-row" type="button">${icon("command")}<span>技能与扩展</span></button>
            <button class="nav-row" type="button">${icon("terminal")}<span>运行记录</span></button>
            <button class="nav-row" type="button">${icon("settings")}<span>设置</span></button>
          </section>
        </div>
        <button class="sidebar-footer" type="button">
          <span class="avatar">M</span>
          <span class="profile-copy"><strong>my_codex</strong><small>本地工作区</small></span>
          ${icon("settings")}
        </button>
      </aside>

      <main class="main">
        <header class="task-header">
          <button class="icon-button" type="button" aria-label="返回">${icon("chevron-left")}</button>
          <div class="task-heading">
            <strong>统一前端视觉规范与组件样式</strong>
            <small>codex/ui-system-demo · 当前工作区</small>
          </div>
          <div class="header-meta"><span class="status-dot"></span><span>已保存</span></div>
          <button class="ghost-button" type="button">${icon("branch")}<span>查看更改</span></button>
        </header>

        <section class="conversation" aria-label="任务对话">
          <div class="conversation-inner">
            <article class="message user-message">
              <div class="message-meta"><span class="avatar">M</span><strong>你</strong><span>10:24</span></div>
              <p>分析一下现在前端各个组件的风格、颜色、排版和文字大小。先给几个界面方案，我想直观看到统一后的效果。</p>
            </article>

            <article class="message assistant-copy">
              <div class="message-meta"><span class="mini-logo">H</span><strong>Hachimi</strong><span>正在分析</span></div>
              <h2>问题不在于缺少 token，而在于业务样式经常绕过它</h2>
              <p>现有基础层已经定义了颜色、字号、间距与圆角，但组件和页面仍大量使用一次性数值，导致同级信息看起来像来自不同产品。</p>
              <div class="audit-grid" aria-label="样式审计摘要">
                <div class="audit-card"><strong>57</strong><span>处硬编码字号</span></div>
                <div class="audit-card"><strong>99</strong><span>处硬编码圆角</span></div>
                <div class="audit-card"><strong>15</strong><span>份独立 CSS 文件</span></div>
              </div>
            </article>

            <section class="activity open" id="activity">
              <button class="activity-toggle" id="activity-toggle" type="button" aria-expanded="true">
                ${icon("terminal")}<strong>检查前端样式</strong><span>3 项完成</span><small>0.8s</small>${icon("chevron-down", "icon chevron")}
              </button>
              <div class="activity-details">
                <div class="activity-step"><span>读取基础 design token</span><code>packages/ui/src/styles</code></div>
                <div class="activity-step"><span>统计原始字号与圆角</span><code>packages/workbench/src/*.css</code></div>
                <div class="activity-step"><span>核对主工作台与设置页</span><code>visual snapshots</code></div>
              </div>
            </section>

            <article class="recommendation">
              <div class="recommendation-header">${icon("spark")}<strong>建议建立一套“克制的桌面生产力”语言</strong></div>
              <ul>
                <li>正文固定 14px；辅助信息 12–13px；页面标题只保留 20px 与 28px 两档。</li>
                <li>间距只使用 4 的倍数；小控件、卡片、浮层分别使用 6/10/14px 圆角。</li>
                <li>强调色只承担选中、主要操作和焦点，不再同时用多种紫色与蓝色表达同一状态。</li>
              </ul>
            </article>

            <article class="diff-card">
              <header class="diff-header"><code>packages/ui/src/styles/tokens.css</code><span>建议结构</span></header>
              <div class="diff-body">
                <div class="diff-line"><span>18</span><code>:root {</code></div>
                <div class="diff-line added"><span>19</span><code>+ --text-body: 14px;</code></div>
                <div class="diff-line added"><span>20</span><code>+ --radius-control: 10px;</code></div>
                <div class="diff-line added"><span>21</span><code>+ --surface-selected: rgb(var(--accent-rgb) / 12%);</code></div>
                <div class="diff-line"><span>22</span><code>}</code></div>
              </div>
            </article>
          </div>
        </section>

        <div class="composer-shell">
          <div class="composer-context">
            <div class="composer-warning">${icon("alert")}<span>Windows Sandbox 尚未通过运行时验证，写入与命令执行将安全拒绝。</span></div>
            <div class="composer-context-tools">
              <button type="button">${icon("folder")}<span>hachimi-code</span>${icon("chevron-down")}</button>
              <button type="button">${icon("terminal")}<span>本地执行</span>${icon("chevron-down")}</button>
            </div>
          </div>
          <div class="composer">
            <div class="composer-attachments">
              <article class="composer-attachment file">
                <span class="composer-file-icon">${icon("file")}</span>
                <span class="composer-attachment-copy"><strong>ui-audit.md</strong><small>MD · 4.8 KB</small></span>
                <button type="button" aria-label="移除 ui-audit.md">${icon("x")}</button>
              </article>
              <article class="composer-attachment image">
                <span class="composer-image-preview" aria-hidden="true"><i>H</i></span>
                <button type="button" aria-label="移除图片附件">${icon("x")}</button>
              </article>
            </div>
            <textarea id="composer-input" aria-label="继续讨论" placeholder="继续讨论或提出修改要求…"></textarea>
            <footer class="composer-footer">
              <div class="composer-tools">
                <button class="tool-button icon-only" type="button" aria-label="添加附件">${icon("plus")}</button>
                <button class="tool-button approval-mode" type="button">${icon("shield")}<span>按需批准</span>${icon("chevron-down")}</button>
              </div>
              <button class="primary-button" id="send-button" type="button" aria-label="发送">${icon("send")}</button>
            </footer>
          </div>
          <small class="composer-footnote">任务将使用当前工作区上下文；执行位置、计划模式和审批策略分别保存。</small>
        </div>
      </main>

      <aside class="context-rail">
        <header class="rail-header"><strong>界面规范</strong><button class="icon-button" id="close-rail" type="button" aria-label="关闭上下文面板">${icon("x")}</button></header>
        <div class="rail-scroll">
          <section class="rail-section">
            <h3>当前方案</h3>
            <div class="context-card">
              <div class="context-row">${icon("spark")}<span>${current.label}</span><strong>${current.note}</strong></div>
              <div class="density-bars" aria-hidden="true"><span class="density-bar"></span><span class="density-bar"></span><span class="density-bar"></span></div>
            </div>
          </section>
          <section class="rail-section">
            <h3>核心颜色</h3>
            <div class="context-card token-list">
              <div class="token-row"><i class="swatch" style="--swatch:${current.accent}"></i><span>Accent</span><code>${current.accent}</code></div>
              <div class="token-row"><i class="swatch" style="--swatch:${current.background}"></i><span>Canvas</span><code>${current.background}</code></div>
              <div class="token-row"><i class="swatch" style="--swatch:${current.surface}"></i><span>Surface</span><code>${current.surface}</code></div>
            </div>
          </section>
          <section class="rail-section">
            <h3>排版层级</h3>
            <div class="context-card">
              <div class="context-row"><span>页面标题</span><strong>28 / 35</strong></div>
              <div class="context-row"><span>分区标题</span><strong>20 / 25</strong></div>
              <div class="context-row"><span>正文与控件</span><strong>14 / 21</strong></div>
              <div class="context-row"><span>辅助信息</span><strong>12 / 18</strong></div>
            </div>
          </section>
          <section class="rail-section">
            <h3>切换方案</h3>
            <nav class="concept-switcher" aria-label="视觉方案">
              <a data-concept-link="graphite" href="demo-01-quiet-graphite.html">A</a>
              <a data-concept-link="mist" href="demo-02-hachimi-mist.html">B</a>
              <a data-concept-link="paper" href="demo-03-paper-focus.html">C</a>
            </nav>
          </section>
        </div>
      </aside>
    </div>
  </div>
  <div class="toast" id="toast" role="status">演示消息已发送</div>
`;

document.body.innerHTML = shell;

const app = document.querySelector("#app");
const activity = document.querySelector("#activity");
const activityToggle = document.querySelector("#activity-toggle");
const composerInput = document.querySelector("#composer-input");
const toast = document.querySelector("#toast");

document.querySelector("#toggle-sidebar")?.addEventListener("click", () => {
  app?.classList.toggle("sidebar-hidden");
});

document.querySelector("#toggle-rail")?.addEventListener("click", () => {
  app?.classList.toggle("rail-hidden");
});

document.querySelector("#close-rail")?.addEventListener("click", () => {
  app?.classList.add("rail-hidden");
});

activityToggle?.addEventListener("click", () => {
  const isOpen = activity?.classList.toggle("open") ?? false;
  activityToggle.setAttribute("aria-expanded", String(isOpen));
});

document.querySelectorAll(".nav-row, .thread-row").forEach((row) => {
  row.addEventListener("click", () => {
    const group = row.classList.contains("thread-row") ? ".thread-row" : ".nav-row";
    document.querySelectorAll(group).forEach((item) => item.classList.remove("active"));
    row.classList.add("active");
  });
});

document.querySelector("#send-button")?.addEventListener("click", () => {
  if (composerInput instanceof HTMLTextAreaElement) composerInput.value = "";
  toast?.classList.add("visible");
  window.setTimeout(() => toast?.classList.remove("visible"), 1600);
});
