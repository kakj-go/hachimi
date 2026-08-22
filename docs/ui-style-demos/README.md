# Hachimi UI 唯一视觉来源

本目录的 `index.html` 是正式前端的组件目录与视觉验收入口。

## 维护规则

1. 新增或修改组件时，先在 `index.html` / `component-gallery.js` 中补齐完整状态。
2. 通用令牌、设置布局和组件样式只修改本目录对应 CSS。
3. `packages/ui/src/styles/demo-contract.css` 直接导入这些 CSS，禁止复制令牌或重新实现一套近似样式。
4. 正式页面通过 `@hachimi/ui` 组件和稳定的 `data-component` 标识使用该规范。
5. 页面级 CSS 只处理业务布局，不得重新定义通用控件的颜色、字号、间距、圆角和交互状态。

## 文件职责

- `demo-shared.css`：Quiet Graphite 主题、排版、间距、密度、页面 Shell。
- `component-gallery.css`：基础组件、Agent、资源、数据和设置模式。
- `component-composer.css`：Composer、附件、上下文和审批控件。
- `demo-settings.css`：完整设置页面结构。
- `demo-appearance.js`：主题、强调色、密度和动效偏好的跨页面同步。
- `theme-decorations.css`：五套二次元主题（px/crm/nya/tora/maho）的纯增量装饰层，按 `html[data-appearance-theme]` 激活。
- `theme-deco-fonts.css` + `fonts/`：装饰层用到的本地化显示字体（ZCOOL KuaiLe / Press Start 2P，woff2 分包）。

正式前端与 Demo 出现差异时，以本目录渲染结果为准。
