# Harness Agent 代码实施计划（当前基线）

更新时间：2026-07-28

## 1. 固定约束

- 唯一执行入口是 `AgentRunExecutor::execute(AgentRunRequest)`。
- `Run = Codex Turn`、`TranscriptItem = Codex Item`，不引入第二套 Thread/Turn 模型。
- 每次采样捕获不可变 `StepContext`，其中包含 world state、分层 `AGENTS.md`、SkillActivation、MCP binding、Host/Sandbox readiness、Model View、预算和 `ToolPlan`。
- `step_revision + tool_plan_hash + registry_revision` 是 Tool Call 的前置条件；旧 revision、未知工具、Schema 不匹配、Grant/Sandbox 收窄失败均 fail closed。
- Tauri 只处理 principal、DTO、系统对话框和 event bridge；Scheduler 只计算时间、claim invocation、提交 fresh Run。
- Pet 输出只作为 Delivery/Presentation Host 接入现有本地 Motion/TTS runtime，不创建专用 EntryProfile、Session、Run、ToolLoop 或权限模型。
- Browser、Computer、Plugin/Connector、Channel/Gateway 和 Session-bound Scheduled continuation 只能作为 typed Host/Domain/transport 接入同一 `AgentRunExecutor`，不得创建第二套 Agent Loop、权限模型或事件模型。
- 前后端新增文件低于 2000 行；不把逻辑堆回 `main.rs`、`workbench_commands.rs` 或 `home.tsx`。

## 2. Fresh 数据库基线

开发基线只有三个 migration，不提供旧数据库升级或双读：

| Migration                       | 内容                                                                                              |
| ------------------------------- | ------------------------------------------------------------------------------------------------- |
| `0001_agent_kernel.sql`         | Session/Run、typed Item、Event、Approval、UserInput、Audit、usage、Compaction、side-effect ledger |
| `0002_workspace_extensions.sql` | Project/Checkout、Diff、Process、Review、MCP、Skills、MCP Host identity                           |
| `0003_automation.sql`           | ScheduleDefinition、ScheduleGrant、TaskRun、Delivery、invocation key                              |

Transcript 只有 `payload_json`；secret、原始 Process 输出、连接订阅、delta 和高权限 lease 不持久化。

## 3. 已落地模块

### Runtime

`hachimi-model-runtime` 提供 ModelRuntime、ModelClientSession、Factory、stream/compact/classifier/token contract。`hachimi-agent` 的 `TurnRuntime`/`ToolLoopDriver` 只消费这个抽象；Model adapter 在 `hachimi-llm`。

`hachimi-llm` 已实现 `Auto|Enabled|Disabled` structured-output 协商、无用户内容的静态 strict-schema probe、base URL/model/settings revision 进程缓存和严格 User Skill workload classifier。Run 持久化 requested/negotiated capability、probe source/error 与 degradation；Unsupported/非法响应固定回退 General。

### Tool Orchestrator

Tool schema planner、model-visible router、execution router 和 Orchestrator 统一处理参数 Schema、Policy、Approval、Sandbox、side-effect claim、Host dispatch、取消、结果预算和终态 Projector。Tool Item 的最终 `ToolExecutionResult` 是唯一权威。

### Projector 与事件

SQLite 使用 Session 单调 sequence。`item.started` 和 `item.completed` 进入持久 Event；`item.delta` 进入 `AgentStore` 的有界 process-local active replay hub。Resume 返回 `snapshot_sequence + active_event_replay`；Subscribe 从持久和 active 两个来源合并、排序、去重。Run 完成或恢复时 active delta 清理，未完成 Item 被 interrupted/lost。

### Skills 与 Workload

Skill Catalog 支持 Built-in/User/Repo/System/Admin roots、路径稳定身份、namespace、分页 list、显式 ID/`$name`、lazy SKILL.md activation、资源 revision fencing、依赖诊断和 Watch。Built-in Skill metadata 只声明 workload/兼容性，不授予 Scope/Grant/Approval。User Skill 仅 frontmatter + 有界 SKILL.md 参与 strict classifier；失败或低置信度回到 General。

### Scheduler

`hachimi-scheduler` 使用 Codex Scheduled Tasks 的后台任务、Skills/MCP、Worktree 和 NeedsAttention 产品语义，以及 OpenClaw 式持久定义 + runtime state + 单个最近到期 timer。当前每次 invocation 创建 fresh TaskRun/Session/Run。ScheduleGrant 固化 Skill content/tree revision、MCP schema/Host identity、权限上限和 workload/context；任何漂移、Sandbox 缺失、Approval/UserInput 或越界自动 Skill 都进入 `NeedsAttention`，不在后台等待。

### Compaction

CompactionService 采用 Claude Code Best clean-room 行为：Local/Remote 显式协商，旧 checkpoint 只在新 checkpoint 完整通过验证后替换；overflow 每次删除一个完整历史组，保留 recent tail/未完成事项/动态上下文。`billed_usage` 只增不减，`active_context_tokens` 和 `remaining_context_tokens` 在 checkpoint 安装后重算。

### Workspace/Sandbox

Workspace Host 负责文件、Git、Watch、Search、Diff 和 Patch；Kernel 不直接访问文件系统。Windows Sandbox runtime manager 进行 setup marker、ACL、restricted token、Job Object、canary 和 deny-all network attestation；restricted process 使用 security-capabilities + explicit handle-list，sentinel smoke 验证未列句柄不可继承。未达到四项 enforced 时所有写入、Exec、stdio MCP 和后台副作用拒绝。

## 4. 收口实施顺序

1. **已完成：AppServer 全域挂接**：生命周期、Approval/UserInput、fs/process/review/mcp/skills/schedule/task 均由可注入 typed handler 处理。
2. **已完成：实时 UI reducer**：Assistant/Reasoning/Tool delta 使用 active item reducer；completed payload 删除 delta 并保持权威。
3. **已完成：普通 Desktop E2E**：真实 WebView2 覆盖 Git/草稿、Plan、Approval、secret UserInput、Diff/Evidence、Review、重启恢复、Skills/MCP、Task Center 和 Office fixture。
4. **已完成：无窗口后台 Runtime**：`service:scheduler` principal 直接进入唯一 `AgentRunExecutor`，验证 background priority 及 Run、Transcript、Usage、终态持久化。
5. **已完成：Skill/Task 漂移收口**：Scheduled MCP 使用精确 server/tool/schema/Host identity fencing；隐式 Office activation 只改变下一 Step workload，不扩大 Grant；漂移进入 `NeedsAttention` 并创建 fresh continuation。
6. **已完成：Task 高级终态**：成功 TaskRun 禁止 retry，Failed/TimedOut/Cancelled/Lost 使用 fresh TaskRun/invocation key retry；通知失败只影响 DeliveryStatus。
7. **代码完成、执行待管理员 Runner：Windows release gate**：`pnpm test:windows:release` 已串联真实安装/修复、handle sentinel、Workspace Worker、Agent Exec、MCP stdio、Terminal/ConPTY、portable 重启、NTFS 路径矩阵、进程树、SystemClock soak 和 Desktop/Toast；本机非管理员未宣称通过。
8. **代码完成、执行待管理员 Runner：生产 extension 验收**：真实 restricted stdio MCP 路径已覆盖修改、预览、Diff、导出、Artifact 校验、中断 → NeedsAttention → 重启恢复和文件整理回滚；最终 release 结论等待受保护 Runner。
9. **已完成：Desktop 测试进程生命周期**：restart/session/failure/finally 只保留最新的精确 E2E 应用实例并终止 WebDriver 进程树；Workspace Worker/MCP stdio 后台启动使用 `CREATE_NO_WINDOW`，完整 Desktop gate 结束后无残留进程。
10. **下一阶段：Pet 输出适配**：通过受控 Host 播放/停止 Motion Catalog 中已有 VRMA，并用现有本地 TTS 播放/停止 Assistant 稳定文本；拒绝任意动作路径及 secret、Approval、UserInput、原始 Tool Result 的语音输出，Scheduled Run 默认禁用。
11. **下一阶段：Browser Host**：实现隔离 Profile、可选 Chrome extension 配对、站点 allowlist、Observe/Act、Download/Upload、Cookie/Storage 和显式 CDP 权限；Codex Browser/Chrome 是产品与安全主基线，底层 Host 可在逐文件登记后选择性研究 OpenClaw 的 CDP/Playwright、SSRF、Profile 和 tab ownership 实现。
12. **下一阶段：Computer Host**：实现 App-scoped Observe/Act、Frame fencing、用户接管、Always-allow App 决策、敏感操作确认和 Windows 前台真实 E2E；Codex Computer Use 是主基线。
13. **下一阶段：Plugins/Connectors**：以 Codex Plugin manifest/marketplace/bundle、Skills/Hooks、Connectors/MCP、Browser extension、Scheduled task template 和 custom UI 为主基线，实现 Plugin bundle、第三方账号/auth、MCP exposure、Action permission、健康检查、事件同步、漂移、重试/幂等和审计。
14. **下一阶段：Channels/Gateway**：仅针对外部消息入口深度参考 OpenClaw Channel plugin、确定性 session/message routing、pairing/allowlist、durable ingress/delivery 和常驻 Gateway；所有入站消息进入 authenticated AppServer，不创建第二套 Agent Runtime。
15. **下一阶段：Session-bound Scheduled continuation**：在现有 Session lane 创建 fresh Run，复用已压缩上下文但重新固定 StepContext、ToolPlan、ScheduleGrant 和 Host readiness；支持 Plugin/Connector、停止条件和线程内 heartbeat，不恢复任何临时授权。

## 5. Control Plane 接口

当前 typed façade 已覆盖 `initialize`、Session search/resume/fork/metadata、Run steer/interrupt precondition、Event subscribe/unsubscribe，以及以下全部 domain：

```text
approval.*  user_input.*  fs.*  process.*  review.*
mcp.*       skills.*      schedule.*  task.*
```

所有 mutation 使用 request/client/protocol/idempotency；活跃 Run 使用 expected Run/generation；Schedule 使用 expected config revision。unsubscribe 只停止推送，不取消 Run。

下一阶段新增 `browser.*`、`computer.*`、`plugin/connector.*` 和 `channel/gateway.*` typed domain；Scheduled continuation 复用现有 `schedule.*`、Session lane 和 Run lifecycle。Gateway 是认证 transport/Host，不拥有独立 Agent 协议。

## 6. 验收矩阵

- Runtime：Interactive/Scheduled StepContext hash、ToolPlan、Policy/Sandbox decision 和 Item lifecycle 一致。
- 安全：重复副作用、取消竞争、Prompt Injection、Windows path/reparse、Sandbox fail-closed、secret 不落盘。
- 生命周期：断线继续执行、snapshot watermark 无缺口/重复、stale generation 拒绝、重启 interrupted/lost。
- Skills：显式、`$name`、隐式激活；资源未激活拒绝；revision 漂移不扩大授权。
- Tasks：At/Every/Cron、DST、Skip/CatchUpOnce、并发、reconciliation、NeedsAttention、continuation 新 Run。
- Office：五个 Skill 的真实格式 Artifact fixture E2E，操作均通过普通 Tool/MCP，不增加 Office Kernel 分支；生产 extension 另走发布环境验收。
- Pet 输出（下一阶段）：Motion ID 只能来自受控 Catalog；动作可停止/替换；TTS 只消费可展示的 Assistant 稳定文本；无专用 Agent Runtime，后台任务默认无声且不播放动作。
- Browser（下一阶段）：隔离/现有 Profile、域名授权、Prompt Injection、CDP/Download/Upload 分权、取消和 Profile 数据清理。
- Computer（下一阶段）：App allowlist、Frame 过期、用户接管、自身审批界面拒绝、截图/剪贴板脱敏和 Windows 前台限制。
- Plugin/Connector（下一阶段）：安装与账号授权分层、Action 权限、Schema/Host identity 漂移、Webhook/Poll、撤销、重试/幂等和 metadata-only Audit。
- Channel/Gateway（下一阶段）：Account/Peer/Thread 路由、DM/group 隔离、pairing/allowlist、bot-loop protection、入站去重、投递回执/重试、重启 drain 和默认 loopback/auth。
- Session-bound Scheduled continuation（下一阶段）：同 Session lane 的 fresh Run、压缩上下文续接、权限重新固定、明确停止条件和无临时授权恢复。

本轮新增完成证据：

- `hachimi-agent::run_runtime_tests::service_principal_executes_a_background_run_without_a_window_transport` 验证 Scheduler 不依赖 WebView/Tauri window。
- `hachimi-scheduler` 测试验证 retry 状态边界、fresh invocation 和 DeliveryStatus 隔离。
- Workbench/Vitest 验证高级 Cron/IANA timezone、Cancelled retry 和 NeedsAttention continuation。
- Desktop/WebView2 E2E 验证 Detached Git、Office Skill 隐式激活/资源失败恢复，以及 MCP schema 漂移后的 NeedsAttention/continuation。
- Scheduled Run 的空 MCP allowlist 不再解释为“允许全部”；Interactive binding 进入 StepContext revision，Review 不接受未固定 MCP。
- `hachimi-scheduler` ignored release soak 使用真实 `SystemClock` 覆盖短期 At、anchored Every、6-field Cron 和 20+ occurrence，无重复 key、无漂移和无泄漏 active launch。
- Windows release 脚本分别运行 handle sentinel、Workspace Worker、Agent `workspace_exec`、restricted stdio MCP 和 Terminal/ConPTY smoke；Toast 由 Windows Shell UI Automation 校验任务名与终态。
- linked worktree Git mutation smoke 强制先用独立 lease stage 并恢复 RX，再用第二 lease commit；ACL 同时显式升级 shared common-dir 与 per-worktree git-dir，任一步准备失败都会回滚。
- Desktop E2E 进程监控回归证明完整 3-spec gate 中应用实例最大为 1，结束后 `desktop-e2e-build/tools` 相关进程为 0。

2026-07-28 本机非管理员门槛已通过 format、typecheck、lint/clippy、全量 Rust workspace tests、contracts、provenance/architecture、P0 adversarial；完整 `pnpm test:desktop:e2e` 单次执行通过 3 个 spec、9 个场景，覆盖真实 WebView2 核心生命周期、Terminal/重启、Skills/MCP、Task/Office 和隐式 Office recovery。Portable target 已成功构建且包含全部 Sandbox/Workspace sidecar 与五个 Built-in Office Skill。当前 PowerShell 非管理员，真实 setup 在受信 Git runtime ACL 阶段以 Windows error 5 fail closed；管理员 Sandbox/restricted stdio/Toast/portable attestation gate 继续保持未完成。

固定门槛见 `docs/ROADMAP.md`。管理员 Windows 和真实 Desktop driver 不可用时必须保留截图/日志并报告环境阻塞。
