# Harness Agent 架构与实现（唯一 Runtime）

更新时间：2026-07-28

## 1. 边界与来源

Hachimi 以 Codex 为统一 Agent 与权限模型主基线：当前选择性适配 Session/Turn/Item、Tool Orchestrator、`AGENTS.md`、Skills、MCP、Session/Thread 持久化恢复、Process、Workspace、Compaction 和 Sandbox 控制流；下一阶段对标 Codex Browser/Chrome、Computer Use、Plugins/Connectors 和 Scheduled Tasks 产品行为。OpenClaw 只作为本地常驻 Gateway、Channel 插件与确定性消息路由、Cron/Heartbeat/事件触发、Task ledger、投递和后台任务重启 reconciliation 的深度参考；当前实际派生范围仍只有 Session lane 与 Scheduler。Compaction 仅把 Claude Code Best 用作 clean-room 行为参考。没有嵌入任一完整产品 Runtime、产品提示词或授权模型。

当前发布边界：Workbench 的 General/Coding/Office、Skills/MCP 和独立 Scheduled Task 已实现；Pet 输出适配、DesktopControl、Browser Host、Computer Host、完整 Plugin/Connector、Channels/Gateway 与 Session-bound Scheduled continuation 仍关闭，属于路线图下一阶段。长期 Memory、多 Agent、远程 Workspace/Control Plane、push/PR 和完整 Provider 扩展更后置。

## 2. Kernel 结构

```text
Authenticated AppServer request
       ↑ current Tauri/Scheduler; future Browser/Computer/Connector/Channel Hosts
       ↓
AgentRunFactory (事务化 Session/Run/User Item/lineage)
       ↓
AgentRunExecutor (lane/generation/cancel/registry/reconcile)
       ↓
TurnRuntime
       ├─ StepContextFactory → immutable StepContext + ToolPlan
       ├─ ModelClientSession
       ├─ ToolOrchestrator
       ├─ WorkloadResolver + SkillActivation + MCP bindings
       ├─ CompactionService
       └─ Central Projector → AgentStore
```

Kernel 不直接访问文件系统、Windows token、Keyring、浏览器 Profile、屏幕输入、Channel 连接或 Tauri window。Workspace Host、Sandbox Backend、MCP Host、未来 Browser/Computer/Connector/Channel Host 和 App Server 都是边界。

### Profile、Workload 与权限

`EntryProfile::Workbench` 是当前执行入口，`DesktopControl` 只保留关闭的未来协议；`WorkloadKind::{General,Coding,Office}` 描述任务 overlay，`workload_override` 可由用户显式指定。Pet 不属于 EntryProfile 或 Workload，而是统一 Agent 的 Delivery/Presentation 输出适配。Profile prompt/context/tool candidates、Skill/MCP allowlist、Provider、Host readiness、Capability Grant、Mode 和 Run allowlist 求交集后才生成 ToolPlan。

WorkloadResolver 优先级固定为：用户 override → 显式且分类一致 Skill → Built-in Office Skill activation → Prompt/冲突 metadata 的 strict 分类 → General。分类、Prompt、Skill、MCP、Compaction 和模型输出永远不能产生授权。

Provider 的结构化输出能力由 `StructuredOutputMode::{Auto,Enabled,Disabled}` 控制。`Auto` 在连接测试和首个 ModelClientSession 使用不含用户内容的静态 strict-schema 请求探测，并按 base URL、model 和 settings revision 做进程内缓存；`Enabled` 是用户确认而不是伪造成功，调用失败仍降级；`Disabled` 禁止 strict classifier 和要求 strict schema 的动态 Tool。Run 持久保存请求能力、协商能力、probe source/error 和 degradation。User Skill workload classifier 只接受严格反序列化后的 `General|Coding|Office`、有界 reason 和合法 confidence；不支持或响应非法时保持 General。

### 下一阶段 Host 组合

Browser、Computer、Connector 与 Channel/Gateway 不创建新的 Agent Loop，也不把产品特判放进 Kernel。它们作为 typed Host/transport 进入同一 AppServer 与 Tool Orchestrator：

- Pet Output Host：只暴露受控 Motion Catalog 的播放/停止以及现有本地 TTS 的播放/停止；动作不接受任意路径，语音不读取 secret、Approval、UserInput 或原始 Tool Result。它只消费统一 Agent 的稳定输出或受控 Tool Call，不创建专用 Session、Run、Profile 或模型循环；Scheduled Run 默认不触发动作或语音。
- Browser Host：以 Codex 产品行为为基线，隔离 Browser Profile 与可选 Chrome extension 分开；站点授权、Act、Download/Upload、Cookie/Storage 和 CDP 权限分层。
- Computer Host：以 Codex 产品行为为基线，Observe/Act 分离，动作绑定稳定 Frame ID 和目标 App；用户接管、窗口变化或授权过期立即使旧动作失效。
- Plugin/Connector Host：以 Codex Plugin manifest/marketplace、Skills/Hooks、Connectors/MCP、Browser extension、Scheduled task template 和 custom UI 为主基线；Plugin 安装、第三方账号连接、MCP tool exposure、Source-system authorization 和 Runtime Grant 分层，Schema、Action 或 Host identity 漂移只能收窄权限或进入 NeedsAttention。
- Channel/Gateway Host：以 OpenClaw Channel plugin、Account/Peer/Thread session routing、durable ingress/delivery 和常驻 Gateway 为主基线；只把外部消息规范化为 authenticated AppServer request，不拥有模型循环。

Browser/Computer/Plugin/Connector 遵循 Codex 的目标产品行为和权限交互；Browser 底层 Host 可选择性研究 OpenClaw 的 CDP/Playwright 安全实现，Channel/Gateway 和本地调度深度参考 OpenClaw。最终暴露给模型的 MCP/Tool 都进入同一个 Codex 式 Orchestrator 安全链。任何 OpenClaw 源码只有在固定文件、许可证和修改说明登记后才能适配。当前空 crate、Feature Flag 或协议枚举不代表 Host 已实现。

## 3. StepContext 与 ToolPlan

每次采样前捕获不可变 StepContext：

- Session/Run/generation、step/profile/context revision；
- SessionContextBinding、RunOrigin、当前 world state；
- Project/Checkout、Git、分层 `AGENTS.md` 指令；
- SkillActivation、MCP binding、Host/Sandbox readiness；
- Model View、token reconciliation、预算；
- Provider capability snapshot、ToolPlan hash 和 Tool registry revision。

`StepWorldStateRefresher` 在每次采样前和每个 Tool 边界后重新读取分层 `AGENTS.md`、已激活 Skill revision/诊断、Run 创建时固定的 MCP schema/Host identity/健康、Workspace/MCP Host readiness，以及“Run 初始 Sandbox snapshot ∩ 当前 report”。新增 MCP Tool 不进入旧 Run；初始 Tool 恢复健康后可重新进入 ToolPlan；Sandbox repair 不能提升活跃 Run。动态 instructions/readiness/workload 由 `StepMessageBuilder` 替换，不把旧快照反复追加到历史消息。

Tool Call 同时带 `step_revision + tool_plan_hash + registry_revision`。Orchestrator 必须使用与调用相同的 registry snapshot；任何 context/profile/Skill/MCP/Host/AGENTS/registry 变化只能在 Tool 边界形成下一 revision，旧调用在 Host dispatch 前 fail closed，不动态扩大 Grant。Scheduled Run 的 pinned Skill/MCP 漂移直接进入 `NeedsAttention`。

## 4. Item、事件与恢复

`TranscriptItem` 是 typed payload，不再存在 `content_json` 双读。Assistant、Reasoning、ToolExecution 使用稳定 Item ID：

```text
item.started(InProgress)
  → item.delta (仅 active event hub)
  → item.completed(Completed|Failed|Interrupted, authoritative payload)
```

Tool 的 arguments、result、Approval、UserInput、Plan、Diff、Evidence 和 Compaction 使用 relations 稳定关联。delta 有界、只在活跃进程内 replay；完成后清理，客户端不得把 delta 拼接当最终值。SQLite 只保存 started/completed 和 metadata-only Event。

Session 的 `next_sequence` 为唯一单调水位。Resume 在一致持久快照上返回 metadata、活跃 Run/generation、pending Approval/UserInput、usage snapshot、`snapshot_sequence` 和有界 `active_event_replay`。Subscribe 合并 SQLite catch-up 与 active replay、按 sequence 去重；断线不取消 Run，unsubscribe 只停止推送。进程重启后旧 Run/未完成 Item 转为 interrupted/lost，不恢复旧 delta、Approval、secret、Worker token 或 lease。

## 5. Tool Orchestrator 安全链

```text
Model-visible schema
 → exact name/namespace/schema validation
 → StepContext + Run allowlist
 → Policy/Plan mode read-only intersection
 → Approval (必要时)
 → Capability Grant
 → Sandbox readiness + restricted process
 → Host dispatch + side_effect_executions claim
 → bounded result/artifact
 → central Projector
```

重复 idempotency key 只返回规范化旧结果；参数变化返回 conflict；已 dispatched 未确认标为 indeterminate，不自动重放。取消/超时/generation 变化终止整棵受控进程树，迟到结果不能写 Transcript/Diff/Event。

Elicitation 只提供数据，不等同 Approval；Skill metadata、Hook（后置）、Prompt 或模型文本都不能创建 Scope、Grant 或 Approval。

## 6. Workspace、Patch 与 Sandbox

Workspace Host 负责 `fs.list/read_chunk/watch/search/diff`、Git plumbing、`apply_patch`、Exec/Process metadata。路径先 native normalization，再通过句柄验证最终对象；reparse/junction/symlink、UNC/设备路径、ADS、保留设备名、尾随点空格和越界 `..` fail closed。

Windows runtime attestation 必须同时证明 OS、filesystem、process、network 四项 enforced，restricted token canary、Job Object、ACL 和 deny-all network 均通过后才允许写入/Exec/stdio MCP。Restricted launch 同时注册 `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES` 和 `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`，后者只包含本次启动创建的 child-side stdio handles；无 stdio 时关闭继承。Sentinel smoke 证明一个虽标记 inheritable、但未进入 handle list 的句柄无法被 Sandbox 子进程使用。Portable/debug 只能显式管理员 setup；marker 不是 readiness。Run 保存创建时 Sandbox snapshot，执行能力只能收窄。

## 7. Skills progressive disclosure

StepContext 只注入有界 metadata；模型可调用分页 `skills.list`，首次读取主 `SKILL.md` 才创建 SkillActivation，之后才能读取相对资源。每次读取校验 authority/package/namespace/path/revision。User Skill 分类只读取 frontmatter 和有界正文，不执行脚本或读取 secret。Skill scripts 只能经普通 Tool/Exec Policy 执行。

五个 Built-in Office Skill 分别负责文档、表格、演示、PDF 和文件整理的知识/验证/模板；实际文件/API 操作仍走 Workspace Tool/MCP。用户显式选择时在首次采样前激活，模型后续隐式激活时在同一 Run 递增 workload/profile/context revision；两者都不扩大权限。

## 8. Compaction

Compaction 是唯一上下文裁剪入口，支持 Auto/Manual/ProviderOverflow、Local/Remote。Remote 必须由 Provider capability 明确协商，输出先校验消息角色、Tool 配对、媒体引用和预算；失败按配置记录后回退 Local，不静默伪造 Remote。

只有 checkpoint 完整 Completed 才安装新 Model View。overflow 每次删除一个完整历史组，保护最新用户目标、未完成事项、recent tail 和动态上下文；仍超限返回 `compaction_source_overflow` 并保留旧 checkpoint。`billed_usage` 只增不减，`active_context_tokens`/`remaining_context_tokens` 在安装后重算。

## 9. Scheduler 与后台任务

ScheduleDefinition 是持久主列表，Scheduler 只计算 occurrence、claim invocation、创建 fresh TaskRun/Session/Run 并提交 AppServer。这个独立任务模式是当前唯一已实现模式。ScheduleGrant 永久有效，Prompt/名称/时间/通知变化不撤销；Context、workload、权限、Tool/Skill/MCP 集合变化必须 reauthorize。Skill content/tree revision、MCP schema/Host identity、Sandbox/Host readiness 漂移进入 NeedsAttention；后台不等待 Approval/UserInput。

下一阶段增加 Codex Scheduled Tasks 式 Session-bound continuation：occurrence 进入现有 Session lane 并创建 fresh Run，使用已压缩对话上下文，但重新捕获 generation、StepContext、ToolPlan、ScheduleGrant 和全部 Host readiness。它不得恢复旧 Approval、UserInput secret、临时 Grant 或 lease；与独立任务模式使用不同的显式 context template，禁止静默互换。

重启时 completed invocation 不重复、无执行器 running TaskRun → Lost、queued claim 可安全重新分派，旧临时 Grant/secret/Approval/UserInput/lease 不恢复。手动 Run 不改变 next occurrence；每个 Schedule 同时最多一个 invocation，全局后台并发上限为 2，交互优先。

## 10. App Server 与传输

`hachimi-control-plane::AppServer` 接收 authenticated `AppServerContext` 和 typed `AppServerRequest`，已挂接 initialize、Session search/resume/fork/metadata、Run steer/interrupt precondition、Event subscribe/unsubscribe，以及绑定 generation 的 Approval/UserInput resolution。Desktop 只把 Tauri DTO、principal 和系统窗口接缝交给 AppServer；Broker 负责持久化与唤醒，AppServer 不创建 Model/Tool loop。fs、process、review、mcp、skills、schedule、task 已通过各自 typed handler 注入；未来 browser、computer、plugin/connector 继续沿用同一 domain handler 边界，禁止在 Tauri 命令中重新组装 Agent runtime。

Tauri 负责 DTO、系统对话框和 `agent:events` bridge；Scheduler 使用 `service:scheduler` principal，不依赖 Workbench 窗口。未来远程 Control Plane transport 复用同一 typed contract。

Release 默认开启 `workspace_tools`、`mcp_runtime` 和 `scheduler`，但默认开启不产生权限；Tool 仍逐次与 Policy、Grant、Approval、Sandbox、Host readiness 和 Run allowlist 求交集。只保留 `HACHIMI_DISABLE_WORKSPACE_TOOLS`、`HACHIMI_DISABLE_MCP_RUNTIME`、`HACHIMI_DISABLE_SCHEDULER` 三个 emergency kill switch；Plugin 能力没有借用 `mcp_runtime` 名称或被隐式启用。

## 11. 验收与后置

当前阶段必须通过固定 Rust/PNPM/provenance/architecture/P0/Desktop E2E 门槛；`pnpm test:windows:release` 在受保护的 `self-hosted/windows/x64/hachimi-sandbox` 管理员 Runner 上串行验证 setup/attestation、handle sentinel、Workspace Worker、Agent Exec、MCP stdio、Terminal/ConPTY、真实 Desktop/Toast、SystemClock soak 和 portable 重启。Browser、Computer、Connector 与 Session-bound Scheduled continuation 分别建立独立安全/E2E 门槛后才能从路线图下一阶段移入“已完成”。所有失败保留脱敏截图/日志，不自动重试掩盖问题。

路线图唯一状态见 `docs/ROADMAP.md`；来源登记唯一权威见 `docs/HARNESS_AGENT_SOURCE_PROVENANCE.md`。
