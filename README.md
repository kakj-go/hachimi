# Hachimi Desktop

Hachimi 是一个 Windows 先行、保持跨平台边界的透明 3D 桌面宠物。当前开发预览版由 Tauri 2、Rust、SolidJS 和 Three.js/three-vrm 构建，包含独立 Workbench、设置中心、VRM 桌宠、单轮流式 LLM 聊天、离线语音识别和离线语音合成。

> 项目尚未发布正式 Release，也没有托管 CI。当前仓库没有项目级 `LICENSE`，公开 fork、分发或接收外部贡献前应先明确项目许可证。

## 已实现

- Pet 与 Workbench 是相互协调的独立窗口；Workbench 显示时隐藏 Pet，最小化或关闭后恢复 Pet。
- Workbench 包含主页、完整设置路由、自绘标题栏、窗口控制、动作设置和动作库实验室。
- 仅接受通过 Detector 4 严格检测的 Runtime Ready VRM 0.x/1.0；普通 GLB 不会进入 V4 Catalog，加载失败时回退到内置 SVG 故障角色。
- Avatar Motion Runtime V4 提供 163 个去重内置 VRMA、用户 VRMA 两阶段导入、Humanoid 重定向、手指保留、惯性化、平衡/接触/IK/关节/碰撞约束，以及独立的口型、注视与 SpringBone 通道。
- Pet 可调用 OpenAI-compatible `/chat/completions` 做无历史、无 Tool Call 的单轮流式聊天；API Key 保存在系统 Keyring，并支持由 Rust 发起连接测试。
- SenseVoice-Small INT8 在本地识别中文、英文、日语、韩语和粤语。
- sherpa-onnx VITS/MeloTTS 在 Rust 进程内完成中英双语合成；支持导入 `.tar.bz2` VITS 模型、选择 Speaker ID、50%–200% 语速及静音。
- Windows 下 STT 与 TTS 可分别选择 Auto、DirectML 或 CPU。DirectML 会按独立显存枚举并尝试真实 DXGI Adapter，失败后重建 CPU Session。

项目、会话、发送、Monaco、终端、Git、Browser、Computer 与 Agent 工作流目前仍未实现。macOS 和 Linux X11 尚未验证；Windows 的透明闪烁、点击穿透、拖动与混合 DPI 仍需要人工验收。

Bundle Identifier：`com.hachimi.desktop`

## 发布前必须注意的资源许可

`assets/avatar-default/3800386813668044008` 下的本地测试 VRM 来自 VRoid Hub，其嵌入许可禁止再分发和修改。因此：

- `.gitignore` 会排除该 `.vrm`，只提交用于审计的 `manifest.json` 与 `NOTICE.md`。
- 缺少该模型时应用会使用 SVG 故障角色；开发者可以在本机放入相同哈希的测试文件。
- MSI、NSIS 和便携 ZIP 只打包该目录的 manifest 与 NOTICE，明确排除本地 VRM；公开安装包在没有用户模型时使用 SVG 故障角色。
- 正式发布前必须换成作者明确授权再分发的默认模型，并同步更新资源路径、manifest、代码常量与说明。

内置 VRMA 动作来自 Clawatar 和 OpenMaiWaifu，采用 MIT License，来源、固定 commit 与许可证副本位于 `assets/avatar-motions-v4/notices`。语音模型、sherpa-onnx、ONNX Runtime 和 DirectML 的来源与许可证见 `apps/desktop/src-tauri/resources/ai-models/THIRD-PARTY-NOTICES.md`。

## 开发环境

- Windows 11 与 WebView2 Runtime
- Rust `1.97.1`（由 `rust-toolchain.toml` 固定）
- Node.js `24.10.x`
- pnpm `11.15.1`（由 `packageManager` 固定，建议通过 Corepack 使用）
- Git LFS（内置 ONNX 语音模型通过 LFS 版本化）

```powershell
git lfs install
git lfs pull
corepack enable
corepack pnpm install --frozen-lockfile
corepack pnpm dev
```

正常 clone 并执行 `git lfs pull` 后，仓库已经包含打包所需的 SenseVoice-Small 与 MeloTTS 全部运行文件，不需要在构建时联网下载。构建前会按 manifest 校验文件大小和 SHA-256；资源缺失或损坏时仍可联网修复：

```powershell
corepack pnpm models:prepare
```

本地测试默认 VRM 是可选资源。只有在你合法取得该文件且仅用于许可允许的场景时，才执行：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\prepare-default-avatar.ps1 `
  -SourcePath "C:\path\to\3800386813668044008.vrm"
```

脚本会校验文件的固定 SHA-256；模型保持为 Git ignored。

## 检查与测试

完整检查包含 Prettier、ESLint、Clippy、TypeScript、Rust/前端单测、协议生成一致性、Storybook 和 Tauri debug build：

```powershell
corepack pnpm check
```

视觉回归首次运行还需要 Chromium：

```powershell
corepack pnpm --filter @hachimi/ui exec playwright install chromium
corepack pnpm storybook:build
corepack pnpm test:visual
```

Windows 人工验收表见：

- [Phase 0/1 Windows 验收](docs/PHASE_0_1_WINDOWS_VALIDATION.md)
- [Workbench 与设置中心验收](docs/WORKBENCH_WINDOWS_VALIDATION.md)

## Windows 构建产物

生成 MSI 与 NSIS 安装程序：

```powershell
corepack pnpm build:installer
```

产物由 Tauri 写入：

- `target/release/bundle/msi/`
- `target/release/bundle/nsis/`

生成无需安装的便携 ZIP：

```powershell
corepack pnpm build:portable
```

便携包位于 `target/portable/Hachimi-portable.zip`。解压后可直接运行；其中的 `reset-portable-data.cmd` 会删除该便携实例的 `data` 并清除 Hachimi API Key。

Windows Debug 和 Release 构建均使用 GUI 子系统，从资源管理器启动时不会额外显示命令行窗口。

## 本地数据

设置、模型库、语音库、日志和 WebView 缓存使用统一的数据根目录：

- Debug 与本地测试：`target/hachimi-data`。
- 便携版：`Hachimi.exe` 同级的 `data`；程序同级存在 `hachimi.portable` 标记。
- MSI/NSIS：设置和资源使用 `%APPDATA%/com.hachimi.desktop`，WebView 缓存使用 `%LOCALAPPDATA%/com.hachimi.desktop/EBWebView`。
- 自动化或自定义运行：通过 `HACHIMI_DATA_DIR` 指定数据根目录。

API Key 不会明文写入这些目录，而是由 Windows Credential Manager 保护。“设置 → 通用 → 本地数据 → 重置全部本地数据”会让应用退出，并在下次启动前删除当前数据根目录、WebView 缓存和 API Key。

后端与 WebView 日志分别为 `logs/hachimi-backend.log` 和 `logs/hachimi-frontend.log`。日志限制长度并对 API Key/Bearer Token 脱敏，不记录聊天输入或完整模型响应。

## 仓库结构

- `apps/desktop/src-tauri`：Tauri 窗口适配层、资源和打包配置。
- `apps/desktop/web`：Vite 生产双入口，仅构建 `pet.html` 与 `workbench.html`。
- `packages/ui`：SolidJS + Kobalte 设计系统、Storybook 与视觉回归基线。
- `packages/pet`、`packages/workbench`：Pet 与 Workbench/设置中心 UI。
- `packages/avatar-motion-runtime`：浏览器侧动作采样、重定向、组合与约束运行时。
- `crates/hachimi-*`：Rust 协议、存储、LLM、Avatar、Motion、Voice、窗口和控制层。
- `assets/avatar-motions-v4`：内容寻址的内置动作、Catalog 和第三方许可。
- `apps/desktop/src-tauri/resources/ai-models`：STT/TTS manifest、许可和完整运行文件；大型 ONNX 权重使用 Git LFS。

## GitHub 首次提交建议

应提交源码、测试、视觉基线、文档、图标、Cargo/pnpm lockfile、动作资源与许可、完整语音模型运行文件、原生运行库及其哈希 manifest。两个大型 ONNX 必须以 Git LFS pointer 提交。不要提交 `target`、`node_modules`、`dist`、Storybook/Playwright 输出、IDE 配置、日志、本地数据、API Key/签名证书、安装程序，或许可禁止再分发的默认 VRM。
