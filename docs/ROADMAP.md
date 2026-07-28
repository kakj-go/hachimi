# Hachimi Agent 路线图（Codex 产品与 Runtime 主基线）

更新时间：2026-07-28

本路线图描述当前唯一架构及已排期的后续 Host，不保留旧 Runtime、旧 DTO 或旧数据库兼容路线。代码派生固定参考 Codex `4c43465133428898aa84f0bfc02c306ed65fb66a` 和 OpenClaw `f6d456235cf011004f7cffc71a95acf6fbf1fa0a`。Codex 是统一 Agent、Browser/Computer、Skills、MCP、Plugins/Connectors、Session/Thread 恢复和 Scheduled Tasks 产品行为与权限模型的主基线；OpenClaw 只作为本地常驻 Gateway、Channel 插件与确定性消息路由、Cron/Heartbeat/事件触发、Task ledger、投递和后台任务重启 reconciliation 的深度参考；Claude Code Best 只用于 clean-room Compaction 行为研究。产品文档不是代码来源，代码派生范围以 `HARNESS_AGENT_SOURCE_PROVENANCE.md` 为准。

## 参考分工

| 能力                          | 主基线                                                                                                                                | 补充基线                                                                                                                 | 当前状态                                                |
| ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------- |
| Agent Kernel、编程、办公      | Codex Session/Turn/Item、Tool Orchestrator、Skills、MCP、Workspace、Process、Sandbox                                                  | Claude Code Best Compaction 公开行为                                                                                     | 当前阶段已落地，仍有 Windows 发布验收                   |
| Browser/Chrome                | Codex Browser、Chrome extension、站点授权、敏感操作确认和受控 CDP 产品行为与权限模型                                                  | Codex 未公开的 Host 细节只选择性研究 OpenClaw 的 navigation guard、request policy、Profile、CDP session 与 tab ownership | 下一阶段；当前 `hachimi-browser` 关闭                   |
| Computer Use                  | Codex App-scoped observe/act、用户接管和系统权限边界                                                                                  | Hachimi 原创 typed Computer Host                                                                                         | 下一阶段；当前 `DesktopControl` 关闭                    |
| Session/Thread 恢复           | Codex rollout reconstruction、thread resume 和持久历史恢复                                                                            | Hachimi SQLite snapshot/watermark 与 fail-closed recovery                                                                | 已落地；活跃 Run 重启后标记 interrupted，不伪造原地续跑 |
| Plugins/Connectors 与三方软件 | Codex Plugin manifest/marketplace、Skills/Hooks、Connectors/MCP、Browser extension、Scheduled task template、custom UI 和运行时权限链 | Hachimi Connector Host                                                                                                   | MCP 已落地；完整 Plugin/Connector 后置                  |
| Channels/Gateway              | OpenClaw Channel plugin、确定性 session/message routing、常驻 Gateway、pairing 和 delivery                                            | Codex 式 Tool/Approval/Sandbox/Host 权限约束                                                                             | 下一阶段；只在确有跨消息平台需求时启用                  |
| 定时与提示词日常任务          | Codex Scheduled Tasks 的独立任务、对话续接、Skills/Plugins 和 Worktree 产品语义                                                       | OpenClaw 单 timer、Task ledger、并发与重启 reconciliation                                                                | 独立 Schedule/Task 已落地；对话内定时续接后置           |
| 上下文压缩                    | Codex `compact.rs` 代码基线                                                                                                           | Claude Code Best recent tail、未完成事项和失败保留旧 checkpoint 行为                                                     | 已落地                                                  |

## 目标架构

```text
Tauri / Scheduler / future Browser · Computer · Connector · Channel/Gateway Hosts
                           │ typed AppServer + authenticated principal
                           ▼
AgentRunExecutor → TurnRuntime → immutable StepContext + ToolPlan
                           │
                           ├─ ModelClientSession
                           ├─ Tool Orchestrator (schema → policy → approval → sandbox → host)
                           ├─ Skills progressive disclosure + WorkloadResolver
                           ├─ CompactionService + token reconciliation
                           └─ Projector → SQLite Transcript/Event + active delta hub
```

Session 是唯一长生命周期容器，Run 对应 Codex Turn，Transcript Item 对应 Codex Item。Interactive、Coding、Office、General 和 Scheduled Run 不得创建第二套 loop。

## 已完成

- [x] 三个 fresh migration baseline：`0001_agent_kernel.sql`、`0002_workspace_extensions.sql`、`0003_automation.sql`。
- [x] `hachimi-model-runtime` 与唯一 `AgentRunExecutor`；Workbench 和 Scheduler 使用同一入口。
- [x] Session lane、generation fencing、取消、后台并发、恢复 reconciliation 和幂等副作用 ledger。
- [x] typed Item/RunEvent；Tool、Assistant、Reasoning 支持 `started → delta → completed`，completed payload 是权威值。
- [x] SQLite 持久 sequence、snapshot watermark、断线 catch-up 与有界 active delta replay；delta 不落盘。
- [x] typed asynchronous AppServer façade；Tauri 生命周期命令只做认证、DTO 和事件桥接。
- [x] metadata-only SQLite Audit；principal、目标摘要、决策和稳定结果码入库，Prompt/secret/完整 Tool Payload 不入库。
- [x] Codex 式分层 `AGENTS.md`、原子 `apply_patch`、Workspace Host 边界、Run Diff、Process/Review 基础协议。
- [x] Codex progressive Skill Catalog：分页 list、显式 `$name`/ID、首次读取激活、资源 revision fencing、User Skill 结构化分类。
- [x] Provider capability negotiation：`Auto|Enabled|Disabled`、静态 strict-schema probe、Run probe/degradation 持久化和严格 User Skill workload classifier；unsupported 固定回退 General。
- [x] `WorkloadResolver`：用户 override > 显式 Skill > Built-in Office Skill > 结构化任务分类 > General；分类不产生授权。
- [x] 五个 Built-in Office Skill：`office-documents`、`office-spreadsheets`、`office-presentations`、`office-pdf`、`office-file-organizer`。它们只提供知识、模板和验证要求，实际操作走 Workspace Tool/MCP。
- [x] MCP Tools、Resources、Templates、Prompts、OAuth/Keyring、Elicitation、progress、媒体引用和 stdio/HTTP Host 边界。
- [x] 本地 Schedule/Task：产品语义与 Codex Scheduled Tasks 的后台任务、Skills/MCP 和 Worktree 模式对齐；持久调度实现参考 OpenClaw 的 At/Every/Cron、单 timer、TaskRun ledger、并发和重启 reconciliation，并支持通知、Worktree 上限和交互 continuation。
- [x] ScheduleGrant：Skill content/tree revision、MCP schema/Host identity、权限快照和漂移 `NeedsAttention`；自动加载不得扩大授权。
- [x] Compaction 生命周期：Local/Remote、overflow 分组删除、checkpoint 原子安装、失败保留旧 checkpoint、billed/active/remaining token reconciliation。
- [x] D2a 文件树、分块读取、Watch、取消搜索和 Run/Checkout Diff；C2.1 Sandbox runtime manager、restricted process 和 fail-closed 路径策略。

## 当前收口项（仍属于本轮 Agent/任务范围）

### A：Runtime 与 AppServer 收口

- [x] 生产代码无 `AgentRunExecutor::execute_registered`、入口级 ToolLoop/Registry/Compactor、InMemoryAudit 和 Agent 事件轮询。
- [x] AppServer 统一生命周期、Approval、UserInput、fs、process、review、mcp、skills、schedule、task 的 typed request/response；Desktop 注入单一 domain handler，Tauri command 不组装 Agent Runtime。
- [x] scheduler service principal 的无窗口集成测试直接进入唯一 `AgentRunExecutor`，验证后台优先级、Run/Transcript/Usage/终态持久化，证明后台 Run 不依赖 Tauri WebView。
- [x] 每次 sampling/Tool boundary 动态刷新 `AGENTS.md`、Skill、固定 MCP、Host 和 Sandbox intersection；Tool Call 使用 `step_revision + tool_plan_hash + registry_revision` 三重 fencing。
- [x] Release 默认开启 `workspace_tools`、`mcp_runtime`、`scheduler`，并保留三个 `HACHIMI_DISABLE_*` emergency kill switch；默认启用不产生 Grant。

### B：实时 Item 投影

- [x] Assistant/Reasoning/Tool 的稳定 Item ID、started/completed 事件和 active delta replay。
- [x] UI active item reducer 按稳定 Item ID 投影 Assistant/Reasoning/Tool delta；completed 删除临时缓冲并以持久 payload 为权威。
- [x] 连接重建的 active replay、completed 清理和 Run interrupted/lost 的真实 WebView2 Desktop E2E。

### C：Windows 发布门槛

- [x] Restricted launch 同时使用 AppContainer security capabilities 和 explicit handle list；sentinel 单元/管理员 smoke 证明未列 inheritable handle 不可访问。
- [x] 统一 `pnpm test:windows:release`、脱敏报告和受保护的 `self-hosted/windows/x64/hachimi-sandbox` workflow 已实现，外部 PR 不执行管理员任务。
- [x] linked worktree Git mutation ACL 同时覆盖 shared common-dir 与 absolute per-worktree git-dir，stage/commit 使用两个独立 lease 并在每次操作后恢复显式 RX；管理员 smoke 已扩展但尚未执行。
- [ ] 在管理员 Runner 实际通过 setup helper、ACL、restricted token、Job Object、子孙进程、junction/reparse、deny-read、deny-all-network。
- [ ] 在管理员 Runner 实际通过 Workspace Worker、Agent Exec、MCP stdio、Terminal/ConPTY、portable target 重启 attestation 和真实 Desktop/Toast。
- [x] Sandbox readiness 未达到四项 enforced 时，后台副作用任务进入 `NeedsAttention`；General 只读任务可继续。

### D：Office Skill 验收

- [x] 五个 Skill 的 metadata、implicit activation、资源验证和 Workload overlay。
- [x] Desktop E2E 验证模型通过 `skills.list`/`skills.read` 隐式激活 `office-documents`，下一 Step 切换 Office overlay；无效资源读取失败后可恢复，且不扩大 CapabilityGrant。
- [x] Desktop fixture 验证五个 Office Skill 显式选择、四类真实容器 Artifact、文件整理 preview 和受控 delivery。
- [x] 文件整理 fixture 覆盖 preview plan、冲突后缀、回滚 manifest 和授权目录边界。
- [x] MCP schema 漂移会使后台任务进入 `NeedsAttention`，真实 WebView2 流程可创建 fresh interactive continuation；不会恢复旧 Grant、Tool Plan 或 generation。
- [x] Release E2E 已实现 restricted stdio MCP 的修改、预览、Diff、导出、Artifact 校验、文件整理回滚和中断恢复路径。
- [ ] 上述 restricted stdio/真实 Sandbox Office 路径仍待管理员 Windows runner 实际通过。

### E：Task 生命周期验收

- [x] Scheduler Rust 测试覆盖 At/Every/Cron、IANA timezone、重复 invocation、重叠跳过、retry 合法终态和 fresh invocation key。
- [x] ignored release soak 使用真实 `SystemClock` 覆盖短期 At、anchored Every、6-field Cron 和至少 20 次 occurrence，无重复 invocation、timer drift 或 active launch 泄漏。
- [x] 成功 TaskRun 禁止 retry；Failed/TimedOut/Cancelled/Lost 可 retry；通知失败只改变 DeliveryStatus，不改变成功执行状态。
- [x] Workbench 测试覆盖高级 Cron/timezone DTO、Cancelled retry 和 NeedsAttention continuation；真实 WebView2 覆盖 MCP schema 漂移后的 NeedsAttention/continuation。
- [x] Windows Shell UI Automation 的 Toast 断言已接入管理员 Desktop E2E，通知只匹配任务名和终态且排除 Hachimi 自身 WebView。
- [ ] 真实墙钟自然触发 At/Every/Cron、系统 Toast 和 UI retry 的完整管理员 WebView2 长时间执行仍属于发布环境验收。

### F：固定门槛

每次收口运行：

```text
pnpm format:check
pnpm typecheck
pnpm lint
pnpm test
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
pnpm contracts:check
pnpm provenance:check
pnpm test:p0-adversarial
pnpm test:desktop:e2e
pnpm test:windows:release
cargo test -p hachimi-sandbox --features windows-smoke -- --ignored --test-threads=1
```

本机未具备管理员 UAC、WebView2 driver 或真实 NTFS smoke 条件时，只报告阻塞，不伪造通过。

2026-07-28 本机非管理员门槛已通过：format、typecheck、lint/clippy、全量 Rust workspace tests、contracts、provenance/architecture、P0 adversarial；完整 `pnpm test:desktop:e2e` 单次执行通过 3 个 spec、9 个场景，覆盖真实 WebView2 核心生命周期、Terminal/重启、Skills/MCP 与 Task/Office。Desktop E2E 清理回归证明测试期间应用实例最大为 1、后台 Worker/MCP 无可见控制台、结束后相关进程为 0。最新 portable target 已构建且 sidecar/Office Skill 完整。当前 shell 非管理员，真实 setup 在受信 Git runtime ACL 阶段以 Windows error 5 fail closed；`test:windows:release` 与 Windows Sandbox ignored smoke 尚未执行。

## 下一阶段路线（当前未实现）

以下能力已经进入总路线，但不属于当前发布收口项，也不能因存在协议、Feature Flag 或空 Host crate 宣称完成。

### G：Pet 输出适配

- [ ] 统一 Agent 通过受控 Pet Host 播放 Motion Catalog 中已有的 VRMA，并支持停止或替换当前动作；Motion ID 必须来自受控 Catalog，不能接受任意文件路径。
- [ ] Assistant 的稳定文本可通过现有本地 TTS 输出并支持停止；不得朗读 secret、Approval、UserInput 或原始 Tool Result。
- [ ] Pet 输出只属于 Delivery/Presentation 层，复用现有 Session、Run、StepContext、Policy 和 Tool Orchestrator，不创建专用 Agent Runtime；Scheduled Run 默认不得自动播放动作或语音。

### H：Browser Host

- [ ] 提供隔离 Profile 的内置 Browser；如接入用户现有 Chrome Profile，必须使用独立扩展、显式配对和域名授权。
- [ ] Observe、Act、Download、Upload、Cookie/Storage 和 CDP 分权；页面内容始终视为不可信输入，敏感提交、购买、删除、权限变化单独确认。
- [ ] 以 Codex Browser/Chrome 产品行为与安全边界为主基线；Codex 未公开的 Host 细节只选择性研究 OpenClaw `extensions/browser/src/browser/{navigation-guard,request-policy,profiles,cdp-page-session,chrome-mcp-tabs}.ts` 及对应测试，但必须逐文件登记来源。

### I：Computer Host

- [ ] DesktopControl 使用 App-scoped observe/act、稳定 Frame ID、动作后重观察、用户真实输入接管和随时停止；不得控制 Hachimi 自身审批界面或代替系统权限确认。
- [ ] Windows 前台占用、截图/Accessibility/剪贴板数据边界、Always-allow App 决策和高风险操作确认必须有真实桌面 E2E。
- [ ] 以 Codex Computer Use 产品行为与权限模型为主基线；Computer Host 采用 Hachimi typed Host 独立实现，不能因存在协议枚举或 Feature Flag 宣称完成。

### J：Plugins、Connectors 与三方软件

- [ ] 采用 Codex Plugin manifest/marketplace/bundle 模型；Plugin 可组合 Skill、Hook、Connector/MCP、Browser extension、Scheduled task template 和 custom UI，安装、启用、账号连接、工具暴露与运行时权限保持分层。
- [ ] Connector Host 覆盖 OAuth/Keyring、Host identity、健康检查、Schema/Action 漂移、限流、重试、幂等、Webhook/Poll、撤销和 metadata-only Audit。
- [ ] 以 Codex Connector-to-MCP routing 和 MCP Tool/Approval/Sandbox/Host 权限链为主基线；结构化 API 能完成的流程不得自动退化为 Browser/Computer 坐标控制。
- [ ] 生产级邮件、日历、Office、Slack/飞书等 Connector 必须逐个声明数据范围、外部副作用和恢复路径；Kernel 不增加应用特判。

### K：Channels 与常驻 Gateway

- [ ] 仅在产品需要从 Slack、飞书、Telegram、Discord 等外部消息入口持续接收与回复时引入；普通 Connector API 不经过 Channel/Gateway。
- [ ] 深度参考 OpenClaw Channel plugin、Account/Peer/Thread session key、确定性 reply routing、DM/group/thread 隔离、pairing/allowlist、bot-loop protection 和 durable ingress/delivery。
- [ ] Gateway 只承担认证连接、Channel 生命周期、事件入口、投递队列和健康/重载，不创建第二套 Agent Runtime；所有消息最终提交 typed AppServer request。
- [ ] 默认 loopback、显式认证、速率限制、连接身份和 metadata-only Audit；远程 Gateway、移动节点与多租户继续后置。

### L：Scheduled Tasks 产品语义补齐

- [x] 独立 Schedule 每次 invocation 创建 fresh TaskRun/Session/Run，支持 At/Every/Cron、时区、Worktree、Skills/MCP allowlist 和 NeedsAttention。
- [ ] 增加 Session-bound scheduled continuation：在现有 Session lane 中创建 fresh Run，复用经压缩的对话上下文，但重新捕获 StepContext、ToolPlan、Grant 和 Host readiness，绝不恢复旧 Approval、secret 或 lease。
- [ ] Scheduled Task 可组合 Plugin/Connector，并在权限或 Schema 漂移时进入 NeedsAttention；支持用户明确的停止条件和线程内 heartbeat。

### M：更后阶段能力

长期 Memory、多 Agent、远程 Workspace/Control Plane、push/PR 和完整 Provider 扩展继续后置，需独立设计与授权模型。

## 来源与许可证

- Codex 开源代码当前只选择性适配 Session/Turn/Tool/Skills/MCP/Sandbox/Process/Workspace/Compaction 控制流，并固定到登记 commit。
- Codex Browser、Computer Use 和 Scheduled Tasks 官方文档定义产品行为与安全验收；固定 commit 同时提供 Session/Thread recovery、Plugins/Connectors 与 MCP 公开实现。产品文档不视为可复制的实现代码，也不证明对应 Host 已经落地。
- OpenClaw 当前已派生代码仍只包括 Session lane、Schedule、Task ledger、单 timer 和重启 reconciliation。Channels/Gateway 已进入下一阶段参考范围；Browser 只补充研究 Codex 未公开的底层 Host 细节。所有逐文件实现仍必须先完成来源登记。
- Claude Code Best 仅用于 Compaction 行为测试，不复制源码、提示词、注释、测试或内部标识符。

实际复制、翻译或实质改写前必须先更新 `HARNESS_AGENT_SOURCE_PROVENANCE.md`；所有派生文件保留对应许可证、固定 commit、源路径和修改说明。
