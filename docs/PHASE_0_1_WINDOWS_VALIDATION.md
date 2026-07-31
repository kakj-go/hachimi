# Phase 0 + Phase 1 Windows 验证记录

记录日期：2026-07-22（Asia/Shanghai）

## 阶段状态

| 标记                      | 状态   | 说明                                                                       |
| ------------------------- | ------ | -------------------------------------------------------------------------- |
| `phase0-local-complete`   | 已完成 | 本地检查可执行且通过；仓库未配置托管 CI，因此不声明规范中的完整 Phase 0。  |
| Phase 1 自动化            | 已完成 | 前端生产构建、Storybook、视觉回归、Rust/Tauri debug build 和启动烟测通过。 |
| `phase1-windows-complete` | 未标记 | 仍需人在真实桌面上观察透明、穿透、交互延迟和窗口恢复。                     |
| macOS / Linux X11         | 未验证 | 只保留抽象接口，不声明跨平台 Phase 1 完成。                                |

## 本次机器环境

| 项目               | 记录                                          |
| ------------------ | --------------------------------------------- |
| Windows            | Windows 11 专业版 `10.0.26200`（Build 26200） |
| 用户 DPI           | 168 DPI（175%）                               |
| 主显示器           | `DISPLAY1`，逻辑区域 `(0, 0) 2194×1234`       |
| 副显示器           | `DISPLAY2`，逻辑区域 `(-2194, 0) 2194×1234`   |
| 混合 DPI           | 未验证；当前记录只能确认负坐标双显示器布局    |
| Rust / Node / pnpm | `1.97.1` / `24.10.0` / `11.15.1`（Corepack）  |

启动烟测确认 `target/debug/hachimi-desktop.exe` 启动 4 秒后进程仍存活，并创建标题为 `Hachimi` 的顶层窗口。烟测结束后已终止该测试进程。

## 自动化结果

- `cargo test --workspace`：通过。
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`：通过。
- TypeScript workspace typecheck、Vitest 与 ESLint：通过。
- Specta 合约重新生成内容检查：通过。
- Storybook production build：通过。
- Playwright：12 个 Light/Dark × zh-CN/en-US × 100/125/150% 截图，以及 Button、TextField、ContextMenu、Dialog 键盘和 WCAG 检查通过。
- Vite production build：只有 Pet 与 Settings 两个 HTML 入口；Workbench 占位未进入 bundle。
- Tauri `--debug --no-bundle`：通过，产物为 `target/debug/hachimi-desktop.exe`。

视觉基线位于 `packages/ui/src/visual/__screenshots__`。

## Windows 人工验收清单

以下项目必须由验收人逐项操作；通过前不得添加 `phase1-windows-complete` 标记。

- [ ] 冷启动过程中没有白底、黑色矩形或不透明 WebView 闪烁。
- [ ] Pet 不出现在任务栏，首次启动默认置顶且显示在主显示器右下角 24 px 边距位置。
- [ ] 左键剪影出现反馈；仅剪影区域的主鼠标按键能够开始拖动。
- [ ] 右键剪影出现菜单，顺序与禁用 Workbench 状态正确。
- [ ] 透明空白处的点击落到后方应用；剪影和菜单仍可交互。
- [ ] 从透明区进入交互区后 60 ms 内恢复交互，未观察到明显抖动或卡死。
- [ ] “始终置顶”可从 Pet 菜单和 Settings 同步切换，重启后保留。
- [ ] Settings 可重复打开；关闭按钮和 Escape 只隐藏 Settings，不退出 Pet 或主进程。
- [ ] 主题与语言切换即时生效且重启后保留。
- [ ] Pet 移动时不主动抢占其他应用焦点。
- [ ] 在双显示器负坐标布局拖动并重启，位置恢复正确。
- [ ] 在不同 DPI 的显示器之间移动，交互矩形与鼠标命中一致。
- [ ] 拔掉保存 Pet 的副显示器后重启，Pet 回退到主显示器右下角且仍可见。
- [ ] “退出”保存位置并终止进程。

## 截图与已知问题

- Storybook 的自动截图已保存；Windows 真实透明桌面的验收截图待人工执行清单时补充。
- 当前穿透方案使用 30 ms 后台轮询和 `set_ignore_cursor_events`，尚未实测 60 ms 目标；若不稳定，再评估 Windows `WM_NCHITTEST` 优化。
- Windows OS 沙箱 Runtime、attestation 和 `Disabled`/`Degraded`/`Enforced` fail-closed 状态已经实现；真实 standard-user 与 elevated 证据仍必须分别通过 `pnpm test:windows:standard-user` 和 `pnpm test:windows:release` 取得，不能由本人工 Pet/窗口清单替代。
- Kobalte ContextMenu 打开时会插入 focus-guard 元素；Playwright 对此禁用了 Axe 的 `aria-required-children` 单项规则，其余选定 WCAG 2 A/AA 规则保持启用。
- macOS、Linux X11、Wayland 均未进行窗口行为验证。
