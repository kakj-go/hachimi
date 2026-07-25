# Workbench 与设置中心 Windows 人工验收

记录日期：2026-07-24

自动化结果由 `pnpm check`、`pnpm test:visual` 和 Rust 单测记录；下列项目必须在 Windows 实机逐项观察后填写，未填写项不得宣称已人工验收。

## 2026-07-24 自动化与构建记录

- [x] `pnpm check` 的全部子检查：格式、ESLint、Clippy、TypeScript、Vitest、Rust workspace tests、Specta dirty-check、Storybook build、Vite/Tauri Debug build 全部通过。
- [x] `pnpm test:visual`：覆盖 Workbench 首页、设置路由、Motion Library Lab、Light/Dark、zh-CN/en-US、多种缩放以及 Pet 状态。
- [x] `hachimi-voice` 自动化实模测试：内置中英双语 MeloTTS VITS 返回有效 PCM，SenseVoice-Small 成功识别中文测试 WAV。
- [x] Avatar Motion Runtime V4：内置 VRMA 在构建时校验格式、轨道、时长、SHA-256 与来源；动态重定向保留完整手指，切换惯性化、媒体口型时钟和 24 项模型编译 LRU 均由新运行时负责。
- [x] 口型自动验收：20ms RMS attack/release、prepared-only Timeline、单调 sequence/media position、无嘴型禁止 Pet PCM，以及 `LipSyncProvider` 有效时间线采用/无效时间线回退均有自动测试。
- [x] Debug 与 Release 冒烟：`hachimi-desktop.exe` 启动 5 秒后均存活且可响应，主窗口标题为 `Hachimi`；测试进程随后已停止。
- [x] 安装包包含 MIT 许可的 MeloTTS VITS、SenseVoice-Small 与 sherpa-onnx 1.13.4 DirectML Windows DLL；STT/TTS 实模热身均已在 DirectML 后端通过。
- [ ] 尚未在无 Rust、Node、LLVM 的干净 Windows 虚拟机中完成安装、卸载与语音试听。

构建产物：

- `target/debug/hachimi-desktop.exe`
- `target/release/hachimi-desktop.exe`
- `target/release/bundle/msi/Hachimi_0.1.0_x64_en-US.msi`
- `target/release/bundle/nsis/Hachimi_0.1.0_x64-setup.exe`

## 环境

- Windows 版本：待填写
- WebView2 版本：待填写
- 显示器与 DPI：待填写
- Ollama/OpenAI-compatible 测试服务与模型：待填写

## 窗口与导航

- [ ] Pet 的“工作台”打开唯一 Workbench 并进入主页。
- [ ] LLM、3D 模型、语音菜单复用同一窗口并进入对应路由。
- [ ] 自绘标题栏可拖动、双击最大化、最小化、还原与关闭。
- [ ] 八方向边缘缩放可用，且不能缩小到 `960×640` 以下。
- [ ] 关闭 Workbench 后 Pet 和主进程保持运行，再打开时继续复用窗口。
- [ ] Workbench 显示后 Pet 隐藏；最小化或关闭后 Pet 无抢焦点恢复。
- [ ] 主页右下角设置按钮与 `Ctrl+,` 都进入通用设置。
- [ ] 静态草稿 Enter 只换行，发送按钮禁用且不产生网络请求或会话。

## 设置与安全

- [ ] 主题、语言和 Pet 始终置顶可保存，重启后仍生效。
- [ ] API Key 输入留空保持原密钥；显式清除后状态变为未配置。
- [ ] `settings.json`、日志和 WebView 返回值中不出现 API Key。
- [ ] 使用本地服务“保存并测试”成功，显示延迟和截断响应预览。
- [ ] 连接失败仍保留非敏感配置，错误不包含密钥或完整响应。
- [ ] Workbench 未获得 Workspace、Shell、Git、Browser 或 Computer 权限。

## 资源目录

- [ ] Runtime Ready VRM 0.x / VRM 1.0 可检测、导入、命名、选择和删除，并展示核心门禁、可选能力、体型、接触点与口型等级。
- [ ] 普通 GLB、核心 Humanoid 不完整的 VRM、无效二进制或超过 200MB 的文件被拒绝且无残留。
- [ ] 当前角色在 Workbench 关闭后动态显示，透明背景、自动取景和 Relaxed Base Pose 正确。
- [ ] VRM 引用外部 HTTP/文件资源、未知必需扩展、越界 BufferView 或超出三角形/纹理预算时被明确拒绝。
- [ ] V4 Catalog 不读取或迁移旧 Catalog；启动与导入不会主动删除旧目录、旧 Blob 或旧 motion-pack 文件。
- [ ] 语音设置展示 SenseVoice-Small、VITS 模型库、实际 Backend、GPU 回退原因和语速。
- [ ] 链接、嵌套归档、路径穿越、条目数或解压大小超限的 VITS `.tar.bz2` 被拒绝且无残留。
- [ ] 同一文件使用不同名称导入时复用 SHA-256 目录。
- [ ] 删除当前项后自动选择最新剩余项；删除最后引用后资源目录被清理。
- [ ] 取消原生文件选择器不会修改目录或列表。

## 桌宠聊天与离线语音

- [ ] 模型普通单击只触发命中部位互动，消息按钮才打开输入框；移动超过 5 CSS px 只拖动窗口。
- [ ] Enter 发送、Shift+Enter 换行、Escape 关闭，停止按钮能取消当前流式请求。
- [ ] 输入框麦克风按钮可调用本地 SenseVoice-Small，并把识别文字填入草稿。
- [ ] 每次请求只包含固定 System Prompt 与本次 User 消息，不包含历史或 Tool Call。
- [ ] 401、网络失败和无效响应只显示脱敏错误且不触发语音。
- [ ] 第一完整短句生成后使用当前 VITS 模型朗读，字幕、说话身体动作和嘴型按同一 `playbackId + mediaPositionMs` 同步；静音立即停止并在重启后保持。
- [ ] `jaw`/`five_viseme` 模型的可视口型偏差 p95 ≤ 80ms；停止后 120ms 内嘴型低于 0.05；`none` 模型断言 Pet PCM 未入队且文字仍显示。
- [ ] 语音页可导入、选择和删除 VITS 模型；多说话人模型可选择有效 Speaker ID，并可选择 50%–200% 语速及 Auto/DirectML/CPU。

## Avatar Motion Runtime V4 与长待机

- [ ] 当前 VRM 启动后双臂进入自然姿态，动作结束回到 Relaxed Base Pose，不回到 T-Pose。
- [ ] 12 个行为均可从程序实验室选择和调参；Mask、相位、重心、支撑脚、接触、碰撞与求解耗时诊断正确。
- [ ] 自然模式连续观察至少 30 分钟；呼吸、换重心、眨眼、视线、倾听和原地行走无明显机械重复。
- [ ] 调度优先级符合 `drag > reaction > gesture > locomotion > speech_body > attention > ambient/base`；拖拽不停止语音，只压制走路与身体手势。
- [ ] 程序切换只在开始、停止、换相和打断时惯性化；最终 IK/接触没有被后续混合破坏，动作结束不回到 T-Pose。
- [ ] 16 个本地 QA 模型（VRM0/VRM1 各八个）全部执行 12 个行为并从正面、侧面、斜面录制，分类与哈希记录在 QA 清单中。
- [ ] 单帧骨骼跳变 ≤ 8°、Root 切换 ≤ 身高 0.5%、循环接缝 ≤ 2°、脚漂 ≤ 身高 0.5%、地面穿透 ≤ 0.3%、关节超限 ≤ 1°。
- [ ] 非设计行为重心不离开支撑区域；无末帧冻结、脚滑、肘穿躯干、明显硬切或 T-Pose 泄漏。
- [ ] 60fps 单模型 Motion Runtime p95 求解 ≤ 2ms（不含渲染和 SpringBone）。
- [ ] 连续运行 60 分钟无持续内存、Mixer、Texture、Clip 或事件监听器增长；记录 Iris Xe/同级核显 p95 帧时间。

## 已知范围

- 新导入只支持 Detector 4 判定为 Runtime Ready 的 VRM 0.x/1.0；未知模型自带 Clip 不会自动播放。
- V4 提供内置只读动作库和用户 VRMA 1.0 导入，不接入在线 Hub Motion API；点击绑定全局应用于所有 Runtime Ready 模型。
- 缺胸骨、脚趾、手指、LookAt、SpringBone 或高级表情时自动降级对应自由度；缺嘴型时 Pet 只显示文字。
- QA 模型来自用户依法下载的本地 VRoid Hub 文件；仓库只保存分类与 SHA-256，不提交禁止再分发的模型。
- 中英双语 MeloTTS VITS 与 SenseVoice-Small 均随安装包提供；旧的按需下载目录、旧语音 Profile、ZIP 和 GPT-SoVITS 不再参与运行。
- Workbench 的项目、会话、消息发送、Monaco、终端、Git、Agent 和桌面控制均未实现。
- macOS 与 Linux 未进行本阶段实机验收。
