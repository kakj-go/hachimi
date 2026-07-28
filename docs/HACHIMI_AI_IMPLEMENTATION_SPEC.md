# Hachimi AI 实施规格（Agent/任务当前基线）

更新时间：2026-07-28

本规格与 `HARNESS_AGENT_ARCHITECTURE_AND_IMPLEMENTATION.md`、`HARNESS_AGENT_CODE_IMPLEMENTATION_PLAN.md` 和 `ROADMAP.md` 一致，直接描述当前唯一 Agent Runtime。Pet 输出适配、Browser/Computer、完整 Plugin/Connector 与 Channels/Gateway 不属于当前发布实现，但已经进入路线图下一阶段并在第 10 节固定参考边界；长期 Memory、多 Agent、远程 Control Plane、push/PR 和完整 Provider 扩展更后置。

## 1. 产品边界

Hachimi 支持 General、Coding、Office 和 Scheduled Prompt Task。编码、办公和后台任务都调用同一个 `AgentRunExecutor`；办公知识来自用户选择或模型渐进激活的 Skill，第三方 API 来自用户配置 MCP。Kernel 不硬编码 Outlook、Excel、邮件、日历或任何应用 SOP。

总体参考原则：Codex 是 Agent Runtime、编程/办公、Skills、MCP、Plugins/Connectors、Session/Thread 恢复、Browser/Computer 和 Scheduled Tasks 产品语义的主基线；OpenClaw 只用于常驻 Gateway、Channels/消息路由、Cron/Heartbeat/事件触发、Task ledger、投递和后台任务重启 reconciliation；Claude Code Best 只影响 Compaction 的 clean-room 行为验收。

首批内置 Skill：

- `office-documents`：文档创建、修改、预览、导出和验证；
- `office-spreadsheets`：表格公式、结构、导入导出和验证；
- `office-presentations`：演示文稿结构、布局、预览和导出；
- `office-pdf`：PDF 生成、读取、渲染和校验；
- `office-file-organizer`：授权目录内盘点、预览计划、归类/重命名和回滚 manifest。

Skill 只提供知识、模板和验证要求，不创建 Scope、Capability Grant、Approval 或 ScheduleGrant。

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
- `DesktopControl` 只保留协议并关闭执行；
- `WorkloadKind::{General,Coding,Office}` 是行为 overlay；
- `workload_override` 用户指定时优先级最高。

Pet 输出不属于 EntryProfile 或 Workload。后续只增加受控 Motion Catalog 播放/停止与现有本地 TTS 播放/停止，作为统一 Agent 的 Delivery/Presentation Host；不增加专用 Session、Run、模型循环或授权模型，Scheduled Run 默认不触发动作或语音。

WorkloadResolver 顺序：用户 override → 显式且分类一致 Skill → Built-in Office Skill activation → Prompt/冲突 Skill 的 strict classifier → General。分类失败、Provider 不支持 strict JSON Schema 或置信度不足时保持 General。Workload 变化只递增下一 Step revision，不修改 generation 或扩大 Grant。

## 4. Item 与事件

Assistant、Reasoning、ToolExecution 的生命周期是：

```text
item.started(InProgress) → item.delta(active memory only)
                         → item.completed(authoritative payload)
```

SQLite 只保存 typed started/completed、审计 metadata、usage snapshot、checkpoint 和副作用 ledger。delta 进入每 Session 有界 active replay buffer，Resume 附带 `active_event_replay`，Subscribe 合并持久事件并按 Session sequence 去重。completed 后清理对应 delta；客户端永远不能从 delta 拼接权威内容。进程重启后未完成 Run/Item 转 interrupted/lost，不恢复 delta、secret、Approval、Worker token 或 lease。

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

Compaction 支持 Auto、Manual、ProviderOverflow 和 Local/Remote。Remote 必须由 Provider capability 明确协商；输出必须通过角色、Tool 配对、媒体引用和预算校验。overflow 每次删除一个完整历史组，保护最新目标、未完成事项、recent tail 和动态上下文。只有 Completed checkpoint 替换 Model View；失败或取消保留旧 checkpoint。`billed_usage` 只增不减，`active_context_tokens` 和 `remaining_context_tokens` 在替换后重新计算。

## 8. Scheduler

ScheduleDefinition 保存 Prompt、At/Every/Cron、时区、General/Project context、workload override、Tool/Skill/MCP allowlist、权限 revision、misfire、delivery 和 config revision。Codex Scheduled Tasks 定义后台任务、Skills/Plugins、Worktree、对话续接和权限产品语义；当前本地 timer、Task ledger、并发和重启 reconciliation 实现参考 OpenClaw。每次 invocation 创建 fresh TaskRun、Session、Run，唯一 invocation key 防重复。ScheduleGrant 固化 Skill content/tree revision、MCP schema/Host identity 和权限上限；Prompt/时间/名称变化不撤销，权限/context/Skill/MCP 集合变化必须重新授权。

后台 Run 不等待 Approval/UserInput；越界、漂移、Sandbox 缺失或自动 Skill 不在 pinned allowlist 时进入 `NeedsAttention`，交互 continuation 新建 Run/generation。重启只 reconciliation ledger，不恢复旧临时授权。

## 9. 持久化

开发基线只有：

- `0001_agent_kernel.sql`；
- `0002_workspace_extensions.sql`；
- `0003_automation.sql`。

不保留旧 `content_json`、旧 Profile 字段、双读或升级 fixture。UUIDv7、append-only Transcript/Event、secret 不落盘和 TaskRun lineage 保持不变。

## 10. 下一阶段参考边界

- Pet 输出：Motion ID 只能来自受控 Catalog，不能接受任意路径；TTS 只消费可展示的 Assistant 稳定文本，不得朗读 secret、Approval、UserInput 或原始 Tool Result；全部调用复用现有 Tool Orchestrator 和 Host 权限边界。
- Browser/Chrome：产品行为、站点授权、敏感操作确认和 CDP 权限以 Codex 为主；Codex 未公开的 Host 细节可逐文件研究 OpenClaw 的 Playwright/CDP、SSRF、Profile、navigation guard 和 tab ownership，不采用其 Agent Runtime。
- Computer Use：App-scoped Observe/Act、用户接管、Always-allow App 与系统权限边界只以 Codex 为主，Windows Host 使用 Hachimi typed contract 独立实现。
- Plugins/Connectors：采用 Codex Plugin manifest/marketplace/bundle、Skills/Hooks、Connectors/MCP、Browser extension、Scheduled task template、custom UI 和权限分层；普通第三方 API 不经过常驻 Gateway。
- Channels/Gateway：仅外部消息入口深度参考 OpenClaw 的 Channel plugin、Account/Peer/Thread 路由、pairing/allowlist、durable ingress/delivery 和常驻 Gateway；Gateway 只提交 authenticated AppServer request。
- Scheduler：Codex 定义 Scheduled Tasks 产品语义，OpenClaw 只提供 Cron/Heartbeat/事件触发、Task ledger、投递和后台任务重启 reconciliation。OpenClaw standing orders 不能从 Prompt 或 `AGENTS.md` 直接生成永久授权，仍必须创建显式 ScheduleGrant。
- Restart：Codex 式 Session/Thread resume 与 rollout reconstruction 用于恢复历史；当前活跃 Run 重启后保持 interrupted/lost。不得照搬 OpenClaw 对有副作用 turn 的自动重放，只可后续研究经过审计的只读续接和幂等投递回执。

## 11. 验收

Rust/PNPM 门槛、P0 对抗测试、真实 Desktop E2E、管理员 Windows Sandbox/MCP stdio smoke 和 provenance/architecture check 是发布条件。管理员权限或 WebView2 driver 不可用时必须明确阻塞，不能把 deterministic test backend 宣称为 OS 隔离。
