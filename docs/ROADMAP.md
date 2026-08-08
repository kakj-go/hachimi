# Hachimi 路线图

更新时间：2026-08-08

本文是当前功能状态和后续迭代的唯一状态源。README 只做产品摘要；架构、实施规格和验收文档引用本文，不再维护第二套功能表。

## 状态说明

| 状态                  | 含义                                                                                |
| --------------------- | ----------------------------------------------------------------------------------- |
| `Local Verified`      | 代码、协议、产品入口和当前机器可执行的确定性测试已通过。                            |
| `Implemented`         | 已有可验收代码，但本地矩阵或产品入口仍需收口。                                      |
| `Environment Blocked` | 需要真实账号、外部组织、真实浏览器或指定 Windows 身份；缺少环境不计为代码测试失败。 |
| `Not Planned`         | 当前产品范围不包含的事项，不作为路线图缺口。                                        |

`Local Verified` 只表示本地实现完成，不等于真实外部服务连通，也不等于 alpha、RC 或 GA 已发布。外部产品行为以固定来源登记为准：[OpenAI](references/openai/registry.json)、[Forge](references/forge/registry.json)、[企业平台](references/enterprise/registry.json)。

## 已完成

| 能力                         | 状态             | 当前交付内容                                                                                                             | 证据/边界                                                                                                                                                        |
| ---------------------------- | ---------------- | ------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Pet 与 Workbench 双窗口      | `Local Verified` | 单实例、透明桌宠、工作台路由、窗口恢复、主题和语言设置。                                                                 | Windows 本地测试；macOS/Linux 尚未验证。                                                                                                                         |
| Session / Run / Item Runtime | `Local Verified` | 持久 Session、Run、Transcript、Approval、UserInput、Steer、Fork、归档、搜索、Compaction 和 crash recovery。              | `crates/hachimi-agent`、`crates/hachimi-storage`。                                                                                                               |
| Workspace 与编码工具         | `Local Verified` | 文件读写、搜索、watch、Diff、Patch、编辑器、Terminal、Git stage/commit/branch/compare/push。                             | 真实远程仓库 mutation 仍见下表。                                                                                                                                 |
| Provider 与模型上下文        | `Local Verified` | OpenAI-compatible Chat Completions、Responses、Embeddings；Responses-only remote compaction 和公开 reasoning summary。   | 真实 OpenAI staging 尚未执行 [ref:OAI-PRODUCT-CHATCOMPLETIONS-20260730] [ref:OAI-PRODUCT-RESPONSES-20260730] [ref:OAI-PRODUCT-EMBEDDINGS-20260730]。             |
| Skills 与 MCP                | `Local Verified` | Skills 分页/激活/导入、MCP stdio、HTTPS/loopback HTTP、OAuth、schema fencing 和调用历史。                                | 权限仍由统一 Tool Orchestrator 控制。                                                                                                                            |
| Scheduled Tasks              | `Local Verified` | At、Every、Cron、Event、独立/共享 Session、Worktree、权限、重试、停止条件、通知和重启 reconciliation。                   | 后台 Browser upload 当前明确拒绝 [ref:OAI-PRODUCT-SCHEDULED-20260730]。                                                                                          |
| Browser / Computer Host      | `Local Verified` | CEF Workspace、外部 Chrome observation、接管/恢复、站点权限、下载、Computer Observe/Act、应用规则。                      | 真实 Chrome Profile、CEF 配对和高完整性桌面待验证 [ref:OAI-PRODUCT-BROWSER-20260730] [ref:OAI-PRODUCT-COMPUTER-20260730] [ref:OAI-PRODUCT-CHROME-20260730]。     |
| Avatar / Motion / Voice      | `Local Verified` | VRM/VRMA 导入与重定向、动作约束、SenseVoice-Small、VITS/MeloTTS、设备选择和语速设置。                                    | Motion Lab 是开发 surface，默认产品入口关闭。                                                                                                                    |
| Git / Forge Runtime          | `Implemented`    | Git remote 与 GitHub/GitLab/Gitee/Gitea/Forgejo query/mutate/reconcile、凭据和副作用 ledger。                            | 继续沿用统一 Host、审批和未知结果 reconciliation [ref:GITHUB-API-20260730] [ref:GITLAB-API-20260730] [ref:GITEE-API-20260730] [ref:GITEA-FORGEJO-API-20260730]。 |
| Multi-Agent Runtime          | `Implemented`    | `agent.spawn/send/wait/cancel/collect`、父子 lineage、预算/深度/并发限制、lease 和启动 reconciliation。                  | 用户可见任务树、预算/用量和控制面尚未接入 Workbench。                                                                                                            |
| Channel / Gateway            | `Local Verified` | 5 个内置 provider：钉钉、飞书、企业微信 AI Bot、企业微信自建应用、微信 iLink；durable ingress/outbox、ACK、重试和去重。  | 真实账号、组织、回调、媒体和撤销 Gate 待验证。                                                                                                                   |
| 内置 Bundle Runtime          | `Local Verified` | 内置 Bundle 的 install/enable/disable/update/rollback/uninstall、known-good revision 和崩溃恢复。                        | 用户 Plugins 管理入口当前未开放；不把它宣传成第三方插件市场。                                                                                                    |
| Storage / 协议基线           | `Local Verified` | V2 schema epoch、迁移锁/备份/回滚、生成的 contracts 和 feature flags；最新迁移为 `0042_permission_skill_allowlist.sql`。 | fresh schema 不承诺旧 DesktopControl 数据兼容。                                                                                                                  |

## 继续迭代

下表回答“路线图还差哪些功能”。其中“功能入口缺失”是产品收口项，“环境阻塞”是验证或发布前置条件；二者不混为已完成。

| 优先级 | 缺口                      | 当前状态                                                      | 完成标准                                                                                            | 入口                                                 |
| ------ | ------------------------- | ------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- | ---------------------------------------------------- |
| P0     | Multi-Agent Workbench UI  | Runtime 已有，前端面板未挂载                                  | 任务树、预算/用量、消息、取消、NeedsAttention、Artifact lineage 和重启恢复可在工作台操作            | `packages/workbench/src/agent-task-panel.tsx`        |
| P0     | Agent Review 流程         | Diff Review 已有；独立 ReviewPanel 未接入                     | 计划审阅、评论、修订、批准和结果投影形成完整用户流程                                                | `packages/workbench/src/review-panel.tsx`            |
| P0     | Scheduled Browser upload  | 当前稳定返回 `schedule_browser_upload_unattended_unsupported` | 明确产品策略；若继续建设，需结构化预授权、文件 token、站点范围、审计和重启恢复                      | `apps/desktop/src-tauri/src/scheduler_commands.rs`   |
| P1     | 真实 OpenAI staging       | `Environment Blocked`                                         | 使用受保护 `secretRef` 完成 Chat/Responses/Embeddings、stream、取消、错误和 compaction Gate         | `test:staging:openai`                                |
| P1     | 3 个企业组织 Gate         | `Environment Blocked`                                         | 企业微信、钉钉、飞书真实 REST/callback/Stream、mention、附件、限流和撤销                            | `test:staging:enterprise`                            |
| P1     | 5 个 Channel Gate         | `Environment Blocked`                                         | 五个 provider 的真实账号、文本/媒体、重连、去重、投递和重启恢复                                     | `test:staging:channels`                              |
| P1     | Browser/Computer 真实环境 | `Environment Blocked`                                         | 隔离 Chrome Profile、CEF/扩展配对、standard-user 高完整性边界和长时低资源 soak                      | `test:windows:standard-user`、`test:desktop:stress`  |
| P1     | Windows 发布隔离 Gate     | `Environment Blocked`                                         | standard-user/elevated 的安装、ACL、restricted token、Job、MCP stdio、ConPTY、便携恢复和 Toast 证据 | `test:windows:standard-user`、`test:windows:release` |
| P1     | `v0.3.0` 发布             | `Implemented`                                                 | clean candidate、许可证/来源/哈希证据、必需证据汇总、alpha/RC/GA workflow 通过后再创建 tag          | `docs/RELEASE_GATES.md`                              |
| P2     | 前端文件拆分              | `Implemented`                                                 | 在继续增加功能前拆分接近 2000 行的 `home.tsx` 和 `index.tsx`，保持共享 UI/样式系统                  | `packages/workbench/src/`                            |
| P2     | 跨平台验证                | `Environment Blocked`                                         | 至少完成 macOS、Linux X11/Wayland 的启动、窗口和资源加载验证；不阻塞 Windows 0.3.0                  | `docs/PHASE_0_1_WINDOWS_VALIDATION.md`               |

## 下一阶段产品路线

以下四项是下一阶段新增的产品方向，当前均为设计/原型阶段，不计入已完成能力。

| 优先级 | 方向             | 目标                                                                                                             | 第一阶段验收                                                                       |
| ------ | ---------------- | ---------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| P0     | AI 动作选择工具  | 提供一个模型可调用的工具，让 AI 根据语义、当前状态和动作约束，从已接入的固定 VRMA Catalog 中选择并播放合适动作。 | 工具 schema、动作候选过滤、权限/冷却/冲突处理、播放结果回执和失败回退。            |
| P1     | 动态生成 VRMA    | 研究由 AI 根据动作意图生成或编辑 VRMA，再经过 Humanoid、轨道、时长、资源和许可校验后进入本地动作库。             | 生成结果可预览、可撤销、不会绕过 Detector/Runtime 约束，并保留来源和版本信息。     |
| P1     | 前端样式持续优化 | 继续统一字体、间距、色彩、控件状态、响应式布局和交互反馈，减少页面之间的视觉差异。                               | 共享 token/组件覆盖新增页面，视觉回归和可访问性检查通过。                          |
| P1     | 二次元主题界面   | 提供多套可切换的二次元主题风格，并同步调整按钮、图标、表单、卡片、导航和空状态等 UI，而不是只替换背景色。        | 主题包有明确 token，所有核心页面可切换，浅色/深色、中文/英文和缩放视觉基线均更新。 |

## 当前范围

Hachimi 当前是本机、单用户、Windows 先行产品。路线图只记录代码已有边界和明确的迭代项；未列入本文的设想不构成承诺。内置 Bundle、Gateway、Channels、Scheduled Tasks 和权限模型分别参考 Codex 的公开产品行为与 OpenClaw 的本地 Gateway/Channel/Task ledger 行为 [ref:OAI-PRODUCT-CODEXAPPSERVER-20260731] [ref:OAI-PRODUCT-PLUGINS-20260730] [ref:OAI-PRODUCT-SCHEDULED-20260730]。

所有真实 Gate 都必须使用脱敏 evidence，fixture、mock、loopback transport 和 deterministic Host 不代替真实服务或操作系统身份。来源提交、固定版本和派生边界见 [来源登记](HARNESS_AGENT_SOURCE_PROVENANCE.md)；架构约束见 [统一 Agent 架构](HARNESS_AGENT_ARCHITECTURE_AND_IMPLEMENTATION.md)。

## 交付定义

一项能力只有在代码、协议、产品入口和确定性测试一致后，才能标为 `Local Verified`。真实账号、组织、浏览器或 Windows 身份缺失时保持 `Environment Blocked`，不把它当作功能完成，也不把它计为测试失败。正式发布还需要 clean commit、候选哈希、许可证证据和 Gate 汇总全部一致。
