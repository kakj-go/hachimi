# Hachimi AI 实施规格（Agent/任务当前基线）

更新时间：2026-08-08

本规格与 `HARNESS_AGENT_ARCHITECTURE_AND_IMPLEMENTATION.md`、`HARNESS_AGENT_CODE_IMPLEMENTATION_PLAN.md` 和唯一状态源 `ROADMAP.md` 一致。当前实现包含活动 Run 安全续跑、三类公开 OpenAI Provider、Remote Compaction/公开 reasoning summary、Multi-Agent Runtime、Git/Forge Runtime、内置 Bundle 生命周期、五个 Channel provider，以及统一 Workbench 会话中的 Browser/Computer Host 原语。Multi-Agent 和独立 Agent Review 的正式 Workbench 入口仍需收口；fixture 不能作为真实 Provider、外部组织或 Windows 发布证据。Hachimi 保持本机单用户边界。

## 1. 产品边界

Hachimi 支持 General、Coding、Office、Scheduled Task 和父子 Agent Task；桌面控制能力属于 Workbench Session，不再是独立入口。所有入口都调用同一个 `AgentRunExecutor`；办公知识来自用户选择或模型渐进激活的 Skill，第三方 API 来自受控 MCP/Connector。Kernel 不硬编码在线 Office 服务或应用 SOP。

总体参考原则：Codex 是 Agent Runtime、编程/办公、Skills、MCP、Plugins/Connectors、Session/Thread 恢复、Browser/Computer 和 Scheduled Tasks 产品语义的主基线；OpenClaw 只用于常驻 Gateway、Channels/消息路由、Cron/事件触发、Task ledger、投递和后台任务重启 reconciliation；Claude Code Best 只影响 Compaction 的 clean-room 行为验收。

首批内置 Skill：

- `office-documents`：文档创建、修改、预览、导出和验证；
- `office-spreadsheets`：表格公式、结构、导入导出和验证；
- `office-presentations`：演示文稿结构、布局、预览和导出；
- `office-pdf`：PDF 生成、读取、渲染和校验；
- `office-file-organizer`：授权目录内盘点、预览计划、归类/重命名和回滚 manifest。

Skill 只提供知识、模板和验证要求，不创建 Scope、Capability Grant、Approval 或 Schedule policy。

## 2. 唯一 Runtime

```text
AppServer → AgentRunFactory → AgentRunExecutor → TurnRuntime
                                             ├─ StepContext + ToolPlan
                                             ├─ ModelClientSession
                                             ├─ Tool Orchestrator
                                             ├─ Skills/MCP/WorkloadResolver
                                             ├─ CompactionService
                                             └─ Central Projector → AgentStore
```

`Run` 对应 Codex Turn，`TranscriptItem` 对应 Codex Item。Tauri 和 Scheduler 不创建 Model、Tool Registry、Compactor 或 ToolLoop；它们只提交 typed request。

每次采样前创建 immutable `StepContext`，包含 Session/Run/generation、profile/context revision、world state、分层 `AGENTS.md`、Skill/MCP activation、Host/Sandbox、Model View、预算和 ToolPlan。Tool Call 固定绑定 `step_revision + tool_plan_hash`。

## 3. Entry/Workload

- `EntryProfile::Workbench` 是编码/办公/General 入口；
- Browser/Computer 是 Workbench Session 的 Host 能力，提供 Observe-first、授权、接管、审批与恢复 UI；
- `WorkloadKind::{General,Coding,Office}` 是行为 overlay；
- `workload_override` 用户指定时优先级最高。

Pet 使用 `EntryProfile::PetConversation` 和独立的持久 Session/fresh Run，但与 Workbench、Scheduler 共用唯一 `AgentRunExecutor`、Tool Orchestrator、权限模型和 Item lifecycle。Pet 只使用受控 Motion Catalog 与本地 TTS 作为 Delivery/Presentation Host；不拥有专用模型循环或授权模型，Scheduled Run 默认不触发动作或语音。

WorkloadResolver 顺序：用户 override → 显式且分类一致 Skill → Built-in Office Skill activation → Prompt/冲突 Skill 的 strict classifier → General。分类失败、Provider 不支持 strict JSON Schema 或置信度不足时保持 General。Workload 变化只递增下一 Step revision，不修改 generation 或扩大 Grant。

## 4. Item 与事件

Assistant、Reasoning、ToolExecution 的生命周期是：

```text
item.started(InProgress) → item.delta(active memory only)
                         → item.completed(authoritative payload)
```

SQLite 只保存 typed started/completed、审计 metadata、usage snapshot、checkpoint 和副作用 ledger。delta 进入每 Session 有界 active replay buffer，Resume 附带 `active_event_replay`，Subscribe 合并持久事件并按 Session sequence 去重。completed 后清理对应 delta；客户端永远不能从 delta 拼接权威内容。进程重启后未完成 Run 先进入 `Recovering`/`WaitingRecoveryDecision`：同 Run 增加 generation，旧 lease/token 失效，只读或有可靠幂等回执的步骤才可续接；未知外部副作用保持 `indeterminate`。delta、secret、Approval、Worker token、临时 Grant 和 lease 均不恢复。

## 5. 安全链

```text
Tool schema → StepContext/allowlist → Policy/Plan read-only
→ Approval → CapabilityGrant → Sandbox → Workspace/MCP Host
→ side-effect claim → bounded result/artifact → Projector
```

重复 idempotency key 只回放规范结果；参数不一致返回 conflict；dispatch 后未知结果标记 indeterminate，不自动重跑。取消、超时或 generation 变化终止整棵受控进程树。Elicitation 仅提供数据，不能替代 Approval；Prompt、Skill、MCP 输出和模型文本不能授权。

## 6. Skills progressive disclosure

StepContext 只放有界 metadata。模型通过分页 `skills.list` 发现允许隐式调用的 Skill，首次读取 `SKILL.md` 才创建 `SkillActivation`，激活后才能读相对资源。每次读取校验 authority、namespace、package、path 和 content revision。User Skill 分类不执行脚本、不读 secret 资源。

## 7. Compaction

Compaction 支持 Auto、Manual、ProviderOverflow、本地 checkpoint 和 Responses-only Remote Compaction [ref:OAI-PRODUCT-RESPONSES-20260730]。远程能力必须同时通过配置、probe 和响应校验；失败、超时、越界或 drift 回退本地 checkpoint。reasoning summary 只保存 Provider 明确标记为可展示的有界 summary，不读取、推断或持久化隐藏 reasoning。overflow 保护最新目标、未完成事项、recent tail、Tool 状态和动态上下文；`billed_usage` 只增不减。

## 8. Scheduler

ScheduleDefinition 保存 Prompt、At/Every/Cron/Event、Workspace context、workload override、Tool/Skill/MCP allowlist、Skill content/tree revision、Connector/Host revision、统一权限 policy、permission revision、misfire、delivery、停止条件和 config revision。Codex Scheduled Tasks 定义后台任务、Skills/Plugins、Workspace、对话续接和权限产品语义 [ref:OAI-PRODUCT-SCHEDULED-20260730]；当前本地 timer、事件入口、Task ledger、并发和重启 reconciliation 实现参考固定版本 OpenClaw。每次 invocation 创建 fresh TaskRun 和 Run，Run 创建服务再生成不可变 `RunAuthoritySnapshot`；Event 使用 `(source,event_id)`、fingerprint 和包含 schedule revision 的 invocation key 去重。Prompt/时间/名称变化不改变权限 revision，权限/context/Skill/MCP/Host 集合变化由用户保存新的 definition revision；运行时漂移进入 `NeedsAttention`。

Event 入口只接受 source/type/subject/最多 16 个 exact labels 和可选 typed resource reference，不接受 Prompt、Grant、Approval 或任意正文。AppServer 使用已认证 principal 绑定 source identity；Workspace、Plugin、Connector、Channel 和 Gateway 适配器只能经由同一入口投递。Event metadata 不提升权限，Agent 需要正文时必须通过已授权 Tool 读取 resource reference。

后台 Run 不等待 Approval/UserInput；越界、漂移、Sandbox 缺失或自动 Skill 不在 pinned allowlist 时进入 `NeedsAttention`，交互 continuation 新建 Run/generation。重启只 reconciliation ledger，不恢复旧临时授权。

## 9. 持久化

当前 fresh schema 使用 V2 schema epoch，迁移由 `crates/hachimi-storage/migrations/` 顺序管理，最新文件为 `0042_permission_skill_allowlist.sql`。文档不再逐个复制 migration 清单，避免与代码目录漂移。

`CONTROL_PROTOCOL_VERSION = 31`。文件数据库发现 pending migration 后使用共享 `<database>.migrate.lock`，以 SQLite Online Backup API 生成同级备份、manifest 和 SHA-256，只保留最近三份；30 秒未取得锁返回 `database_migration_busy`，失败回滚事务、保留备份并拒绝启动。内存数据库不创建备份。Desktop 与 Gateway 复用同一实现。

不保留旧 `content_json`、旧 Profile 字段或 typed/untyped 双读。UUIDv7、append-only Transcript/Event、secret 不落盘和 TaskRun lineage 保持不变。

## 10. 当前 Host 参考边界

- Pet 输出：Motion ID 只能来自受控 Catalog，不能接受任意路径；TTS 只消费可展示的 Assistant 稳定文本，不得朗读 secret、Approval、UserInput 或原始 Tool Result；全部调用复用现有 Tool Orchestrator 和 Host 权限边界。
- Browser/Chrome：产品行为、站点授权、敏感操作确认和 CDP 权限以 Codex 为主 [ref:OAI-PRODUCT-BROWSER-20260730] [ref:OAI-PRODUCT-CHROME-20260730]；Codex 未公开的 Host 细节可逐文件研究 OpenClaw 的 Playwright/CDP、SSRF、Profile、navigation guard 和 tab ownership，不采用其 Agent Runtime。
- Computer Use：App-scoped Observe/Act、用户接管、Always-allow App 与系统权限边界只以 Codex 为主 [ref:OAI-PRODUCT-COMPUTER-20260730]，Windows Host 使用 Hachimi typed contract 独立实现，截图只保存在受 TTL/容量限制的内存 PNG 仓库。
- Plugins/Connectors：采用 Codex Plugin bundle、Skills/Hooks/EventSource、Connectors/MCP、Browser extension、Scheduled task template、custom UI 和权限分层 [ref:OAI-PRODUCT-PLUGINS-20260730]；统一 lifecycle journal 覆盖 install/enable/disable/update/rollback/uninstall 和崩溃 reconciliation。企业微信 GET `echostr`/POST 加密 XML callback、钉钉 Stream、飞书 WebSocket/protobuf、结构化 mention 和受控附件 Artifact 已用 Rust 实现并通过确定性 fixture [ref:WECOM-API-20260730] [ref:DINGTALK-STREAM-SDK-GO-20260731] [ref:FEISHU-SDK-GO-20260731]；三个真实外部组织仍待验证。
- Channels/Gateway：仅外部消息入口深度参考 OpenClaw 的 Channel plugin、Account/Peer/Thread 路由、pairing/allowlist、durable ingress/delivery 和常驻 Gateway；Gateway 只提交 authenticated AppServer request。
- 平台集成设置：生产 manifest 提供钉钉、飞书、企业微信 AI Bot、企业微信自建应用和微信 iLink 五个 provider；其中企业微信自建应用、钉钉和飞书同时提供企业 API 配置。同一账户可独立选择 API 与消息能力，凭据只写入 Windows Credential Manager。Gateway 随消息账户需求自动托管；fixture Provider 不进入正式 UI。
- Plugins 产品状态：内置 Bundle lifecycle 继续由 Runtime 使用；用户管理入口当前置灰并显示“暂不开放 / Not available”，因此不能把内部 lifecycle 宣传成用户可用的第三方安装能力。
- Computer Use：只枚举当前可控制的可见窗口与已有策略；Windows 描述符包含产品名、规范路径、发布者验证状态、文件身份和 Shell 图标。策略写入只接受后端候选的 `identityHash`，不扫描全部已安装应用。
- Scheduler：Codex 定义 Scheduled Tasks 产品语义，OpenClaw 只提供 Cron/事件触发、Task ledger、投递和后台任务重启 reconciliation。Gateway/Transport/Lease heartbeat 已实现；`schedule.activity_marker` 仅为内部 Run 启动标签。OpenClaw standing orders 不能从 Prompt 或 `AGENTS.md` 直接生成永久权限；权限只能来自 ScheduleDefinition 的统一 policy。
- Restart：Codex 式 Session/Thread resume 与 rollout reconstruction 用于恢复历史；活动 Run 依据 durable checkpoint、可信 Host recovery policy、revision 和 side-effect receipt 安全续跑。不得照搬 OpenClaw 对有副作用 turn 的自动重放；dispatch 后结果未知的动作必须由用户确认或放弃。

## 11. 验收

本地门槛是 Rust/PNPM、P0 对抗、确定性 Desktop E2E、contracts、provenance、architecture、发布 harness 和 Rust/TypeScript/TSX/CSS 单文件不超过 2000 行。当前提交的结果必须由本次 `corepack pnpm check` 或对应 CI artifact 证明，不能沿用旧日期的通过结论。真实 OpenAI/Forge/外部组织以及 standard-user/elevated Windows Gate 明确后置；deterministic backend、fixture、loopback listener 和 mock conformance 只能证明本地实现，不能宣称 OS 隔离或外部服务连通。

当前 Provider registry 只支持公开 OpenAI `/v1/chat/completions`、`/v1/responses` 和 `/v1/embeddings` 三类协议及显式登记、probe 匹配的兼容档案 [ref:OAI-PRODUCT-CHATCOMPLETIONS-20260730] [ref:OAI-PRODUCT-RESPONSES-20260730] [ref:OAI-PRODUCT-EMBEDDINGS-20260730]。不接入私有 Codex 协议、Realtime、Images、Audio、Video 或厂商私有模型 wire shape。真实 OpenAI staging Gate 仍为环境阻塞。

## 12. P1–P8 当前边界

- 崩溃续跑只允许只读或能以同一幂等键证明回执的步骤；dispatch 后未知结果固定为 `indeterminate`。
- Multi-Agent 使用父子 Task/Run 和统一 Projector；`agent.spawn/send/wait/cancel/collect` 与其他能力一样只按权限、Host readiness、Plan mode 和父 Run 的单调收窄规则进入 ToolPlan，不按来源、EntryProfile 或 workload 建白名单。子 Agent 的 Tool/Skill/MCP/Host/预算只能继承后收窄，取消和审计向下传播。
- Git push 使用标准 Remote 与 GCM/SSH Agent。Agent 原生 `git.remotes/git.push/forge.change.query/forge.change.mutate` 依赖通用 WorkspaceHandle 和实际 Git 仓库，并与 Workbench UI 复用同一 Host；Plan mode 只保留 remotes/query。Remote 只为 Git/Forge 授权上下文产生精确 host/protocol Grant，不扩大其他 Host。Forge adapter 支持 GitHub、GitLab、Gitee、Gitea/Forgejo [ref:GITHUB-API-20260730] [ref:GITLAB-API-20260730] [ref:GITEE-API-20260730] [ref:GITEA-FORGEJO-API-20260730]；未知平台只完成 push 并生成草稿。mutation 响应未知时返回 executor error，使统一 side-effect ledger 保持 `Indeterminate`；只允许查询远端，source/target、可见字段、状态与 commit OID 全部匹配才把原操作确认为成功，否则不重放。官方 API 不支持原地替换源分支，因此更新操作把 source ref 作为不可变前置条件。
- 企业附件与通用 Connector 能力按统一权限和 Host readiness 进入所有来源/Profile/workload 的 ToolPlan；Scheduled、Channel 和 Pet 的后台授权必须固定 Connector account、action 与 contribution revision，缺失或漂移进入 `NeedsAttention`。通用 `connector_invoke` 明确拒绝保留动作，不能绕过 `enterprise.download_attachment` 的下载、校验和 Artifact fencing。
- Browser 与 Computer 继续绑定 Run generation、lease、observation/frame、App/Window fingerprint、站点/能力授权和 Sandbox readiness；终态 Run 不能复用旧 generation。
- 七类 `RuntimeFeatureSet` 能力默认开启；关闭时 UI 隐藏入口、对应工具不注册，命令返回 `feature_disabled` 和 feature key。Provider Extensions 关闭后只保留 legacy Chat Completions，Git Remote Mutations 关闭后仍允许本地 stage/commit；migration 不受开关影响。
- Office 产品边界是本地 DOCX/XLSX/PPTX/PDF 与文件整理，不增加在线 Office 服务依赖或 Office 专用 Kernel 分支。

> 实现状态统一以 `docs/ROADMAP.md` 为准；本规格只描述当前 contract，不单独宣称发布或真实环境验证完成。
