# Harness Agent 架构与实现（唯一 Runtime）

产品/API 行为固定参考：[ref:OAI-PRODUCT-BROWSER-20260730] [ref:OAI-PRODUCT-CHROME-20260730] [ref:OAI-PRODUCT-COMPUTER-20260730] [ref:OAI-PRODUCT-PLUGINS-20260730] [ref:OAI-PRODUCT-SCHEDULED-20260730] [ref:OAI-PRODUCT-RESPONSES-20260730] [ref:GITHUB-API-20260730] [ref:GITLAB-API-20260730] [ref:GITEE-API-20260730] [ref:GITEA-FORGEJO-API-20260730] [ref:WECOM-API-20260730] [ref:DINGTALK-STREAM-SDK-GO-20260731] [ref:FEISHU-SDK-GO-20260731]。

更新时间：2026-07-31

## 1. 边界与来源

Hachimi 以 Codex 为统一 Agent 与权限模型主基线：当前选择性适配 Session/Turn/Item、Tool Orchestrator、`AGENTS.md`、Skills、MCP、Session/Thread 持久化恢复、Process、Workspace、Compaction 和 Sandbox 控制流，并以 Codex Browser/Chrome、Computer Use、Plugins/Connectors 和 Scheduled Tasks 的公开产品行为约束本地 Host。OpenClaw 只作为本地常驻 Gateway、Channel 插件与确定性消息路由、Cron/Heartbeat/事件触发、Task ledger、投递和后台任务重启 reconciliation 的深度参考；实际代码派生仍以来源登记为准。Compaction 仅把 Claude Code Best 用作 clean-room 行为参考。没有嵌入任一完整产品 Runtime、产品提示词或授权模型。

当前实现边界已扩展到活动 Run 安全续跑、公开 OpenAI Chat/Responses/Embeddings、Remote Compaction/公开 summary、Multi-Agent、通用 Git/Forge、完整 Plugin lifecycle、企业 REST/事件安全层，以及统一 Workbench 会话中的 Browser/Computer 原语。本地代码完成不代表 alpha/GA 已发布；真实 Provider/Forge、外部企业组织与 Windows 证据仍按路线图阻塞。Memory 为远期，不采用 Codex Memory 方案。Hachimi 保持本机单用户运行，不增加登录或租户体系。

## 2. Kernel 结构

```text
Authenticated AppServer request
       ↑ Tauri/Pet/Scheduler/Browser/Computer/Connector/Channel Hosts
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

Kernel 不直接访问文件系统、Windows token、Keyring、浏览器 Profile、屏幕输入、Channel 连接或 Tauri window。Workspace、Sandbox、MCP、Browser、Computer、Plugin/Connector、Channel/Gateway 和 AppServer 都是 typed 边界。

### Profile、Workload 与权限

`EntryProfile::{Workbench,PetConversation}` 和 Multi-Agent 子 Run 都进入唯一 `AgentRunExecutor`；`WorkloadKind::{General,Coding,Office}` 描述任务 overlay。Pet 的窗口/TTS/动作和 Workbench 的 Browser/Computer Inspector 只是受控产品 Host，不拥有独立模型循环或 Grant。Profile、Skill/MCP/Host allowlist、Provider、Host readiness、Capability Grant、Mode 和 Run allowlist 求交集后才生成 ToolPlan。

P4/P5/P7 的模型入口按产品边界固定：五个 `agent.*` 只进入 Workbench General/Coding/Office，Scheduled 还必须命中持久化精确 Tool allowlist；企业附件只进入 General/Office；四个 Git/Forge 工具只进入交互式 Project Coding。Pet 不注册这些工具。Plan mode 对 Multi-Agent 只保留 wait/collect，对 Git/Forge 只保留 remotes/query，并继续移除全部 mutation。

WorkloadResolver 优先级固定为：用户 override → 显式且分类一致 Skill → Built-in Office Skill activation → Prompt/冲突 metadata 的 strict 分类 → General。分类、Prompt、Skill、MCP、Compaction 和模型输出永远不能产生授权。

Provider 的结构化输出能力由 `StructuredOutputMode::{Auto,Enabled,Disabled}` 控制。`Auto` 在连接测试和首个 ModelClientSession 使用不含用户内容的静态 strict-schema 请求探测，并按 base URL、model 和 settings revision 做进程内缓存；`Enabled` 是用户确认而不是伪造成功，调用失败仍降级；`Disabled` 禁止 strict classifier 和要求 strict schema 的动态 Tool。Run 持久保存请求能力、协商能力、probe source/error 和 degradation。User Skill workload classifier 只接受严格反序列化后的 `General|Coding|Office`、有界 reason 和合法 confidence；不支持或响应非法时保持 General。

Provider registry 当前接入公开 OpenAI `/v1/chat/completions`、`/v1/responses`、`/v1/embeddings` 三类协议和显式兼容档案。Responses adapter 映射公开 Item/function/usage/error/cancel；Remote Compaction 与 reasoning summary 只能由 capability probe 启用 [ref:OAI-PRODUCT-RESPONSES-20260730]。不接私有 Codex、Realtime、Images/Audio/Video 或厂商私有协议。

### Local Host 组合

Browser、Computer、Connector 与 Channel/Gateway 不创建新的 Agent Loop，也不把产品特判放进 Kernel。它们作为 typed Host/transport 进入同一 AppServer 与 Tool Orchestrator：

- Pet Output Host：只暴露受控 Motion Catalog 的播放/停止以及现有本地 TTS 的播放/停止；动作不接受任意路径，语音只消费最终稳定 Assistant 文本。存在 secret UserInput 的 Run 强制无声，Approval/UserInput/原始 Tool Result 不作为输出源；取消 Run 同步停止输出，Scheduled Run 默认无动作、无语音。
- Browser Host：managed Chromium 与任务 owned Chrome tabs 共享 deny-by-default 网络规则；history、input、wait、tab、transfer/storage 原语和静态 CDP allowlist 均绑定 origin、observation 和 Run generation。任意 `Runtime.evaluate`、Target attach、网络拦截和调试器逃逸拒绝。
- Computer Host：Observe/Act 分离，鼠标/键盘/窗口/受控应用启动绑定 Frame、App/Window fingerprint、input epoch、前台窗口与 generation；用户接管、窗口变化或授权过期立即失效。Audit 不保存标题、输入、坐标或截图。
- Plugin/Connector Host：typed Bundle 通过 lifecycle journal 执行 stage/validate/permission/activate/health/commit，支持 update/rollback/uninstall 与崩溃 reconciliation。Skill/Hook/EventSource/MCP/Connector/Browser extension/Scheduled template/assets/custom UI/Channel 的产品绑定在停用/卸载时清理；动态执行只能使用 Sandbox stdio sidecar。`sample-crm` 与三个企业 REST driver 复用统一 Connector registry。
- Git/Forge Host：Workbench UI 与 Agent executor 复用 Remote 解析、Workspace Host dispatch、Forge transport、Credential Manager、revision/OID fencing 和远端 reconciliation；窗口鉴权与 UI ledger 留在 command 入口，Agent 入口由 `AuthorizedTool` 统一执行 Policy、Approval 和 side-effect ledger。交互式 Project Run 从当前 Remote 推导精确 host/protocol Grant，并只安装到 Git/Forge 的授权上下文，不扩大 Connector、Browser 或其他 Host。mutation 返回未知时 executor 必须报错，使统一 ledger 保持 `Indeterminate`，不能转成普通失败或重放。
- Channel/Gateway Host：普通用户 `--gateway` 进程使用唯一 `ChannelProvider` registry，负责 Account/Peer/Thread、durable ingress/outbox、heartbeat、重试和 startup reconciliation。企业 Bundle 的内置 Channel contribution 只控制同一 Gateway provider，不启动第二个 sidecar。WeCom loopback listener 支持官方 GET `echostr` 与 POST 加密 XML，再由企业 Provider 完成 AES/签名/外部组织标识/重放校验；DingTalk Stream、Feishu WebSocket/protobuf 的 ACK/heartbeat/reconnect/dedup 与企业 ledger 已实现 [ref:WECOM-API-20260730] [ref:DINGTALK-STREAM-SDK-GO-20260731] [ref:FEISHU-SDK-GO-20260731]；真实外部组织连接仍待验证。

Browser/Computer/Plugin/Connector 遵循 Codex 的目标产品行为和权限交互；Browser 底层 Host 可选择性研究 OpenClaw 的 CDP/Playwright 安全实现，Channel/Gateway 和本地调度深度参考 OpenClaw。最终暴露给模型的 MCP/Tool 都进入同一个 Codex 式 Orchestrator 安全链。任何 OpenClaw 源码只有在固定文件、许可证和修改说明登记后才能适配；当前完成结论来自可运行 Host 和测试，不来自 Feature Flag 或协议枚举。

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

Session 的 `next_sequence` 为唯一单调水位。Resume 返回 metadata、活跃 Run/generation、pending Approval/UserInput、usage snapshot、`snapshot_sequence` 和有界 replay。进程重启后活动 Run 创建 durable recovery record，同 Run 增加 generation，旧进程/lease/token 失效；只读或能凭相同幂等键证明结果的步骤可续接，未知副作用必须人工决策。旧 delta、Approval、secret、Worker token、临时 Grant 和 lease 不恢复。

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

Windows runtime attestation 必须同时证明 OS、filesystem、process、network 四项 enforced，restricted token canary、Job Object、ACL、managed Runtime/Git SHA-256 和 deny-all network 均通过后才允许 Workspace 写入、Exec、stdio MCP、Browser/Computer broker 或后台副作用。Restricted launch 同时注册 `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES` 和 `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`，后者只包含本次启动创建的 child-side stdio handles；无 stdio 时关闭继承。AppContainer Profile、marker 和 managed Runtime 全部位于 per-user data root，首次启动/repair/upgrade 不使用 `runas`、UAC、系统 Git 或系统 Git ACL。Workspace 必须是当前用户所有的本地 NTFS 非保护目录，否则返回稳定迁移错误。Run 保存创建时 Sandbox snapshot，执行能力只能收窄。

## 7. Skills progressive disclosure

StepContext 只注入有界 metadata；模型可调用分页 `skills.list`，首次读取主 `SKILL.md` 才创建 SkillActivation，之后才能读取相对资源。每次读取校验 authority/package/namespace/path/revision。User Skill 分类只读取 frontmatter 和有界正文，不执行脚本或读取 secret。Skill scripts 只能经普通 Tool/Exec Policy 执行。

五个 Built-in Office Skill 分别负责文档、表格、演示、PDF 和文件整理的知识/验证/模板；实际文件/API 操作仍走 Workspace Tool/MCP。用户显式选择时在首次采样前激活，模型后续隐式激活时在同一 Run 递增 workload/profile/context revision；两者都不扩大权限。

## 8. Compaction

Compaction 是唯一上下文裁剪入口，支持 Auto/Manual/ProviderOverflow、本地 checkpoint 和 Responses-only Remote Compaction。远程 capability 必须由配置、probe 与运行时响应同时证明；失败、超时、越界或漂移回退本地，不静默伪造 Remote。summary 只接收 Provider 明确允许展示的内容，不读取隐藏 reasoning。

只有 checkpoint 完整 Completed 才安装新 Model View。overflow 每次删除一个完整历史组，保护最新用户目标、未完成事项、recent tail 和动态上下文；仍超限返回 `compaction_source_overflow` 并保留旧 checkpoint。`billed_usage` 只增不减，`active_context_tokens`/`remaining_context_tokens` 在安装后重算。

## 9. Scheduler 与后台任务

ScheduleDefinition 是持久主列表，Scheduler 只计算 occurrence、claim invocation、创建 fresh TaskRun/Run 并提交 AppServer。`Standalone` 创建新 Session；`SessionContinuation` 进入既有 Session lane 创建 fresh Run，并读取触发时已经压缩持久化的上下文。两者都重新捕获 generation、StepContext、ToolPlan、ScheduleGrant、Plugin/Connector revision 和 Host readiness，不恢复旧 Approval、UserInput secret、临时 Grant、Browser observation、Computer frame、MCP session 或进程 lease。

ScheduleGrant 持久有效，但 Prompt/名称/时间/通知变化不撤销；Context、workload、权限、Tool/Skill/MCP/贡献点集合变化必须 reauthorize。Task Center 固定 Connector account/action/content/Host/Schema revision，以及 isolated Browser unattended 的 document/resource origin 与 Observe/Act/Download 能力；Upload/Cookie/CDP 默认关闭，Computer unattended 稳定拒绝。企业附件把保留动作 `download_attachment` 作为独立授权：执行前必须同时匹配 account、contribution revision 和 action，授权缺失返回 `schedule_enterprise_attachment_not_authorized`，revision 漂移返回 `schedule_connector_action_drift`，两者都通过共享 Schedule Host Grant 验证器映射为 `NeedsAttention`；`connector_invoke` 显式拒绝该保留动作，不能绕过附件专用链。Skill content/tree、MCP schema/Host identity、Plugin/Connector/account、Browser network rule、Sandbox/Host readiness 漂移进入 NeedsAttention；后台不等待 Approval/UserInput。停止条件包括最大次数、截止时间、成功后停止与用户停用，每次 continuation 写入 thread heartbeat Item。

重启时 completed invocation 不重复、无执行器 running TaskRun → Lost、queued claim 可安全重新分派，旧临时 Grant/secret/Approval/UserInput/lease 不恢复。手动 Run 不改变 next occurrence；每个 Schedule 同时最多一个 invocation，全局后台并发上限为 2，交互优先。

## 10. App Server 与本地传输

`hachimi-control-plane::AppServer` 接收 authenticated `AppServerContext` 和 typed `AppServerRequest`，已挂接 initialize、Session search/resume/fork/metadata、Run steer/interrupt precondition、Run Event subscribe/unsubscribe、绑定 generation 的 Approval/UserInput resolution，以及 fs/process/review/mcp/skills/schedule/task/browser/computer/plugin/connector/channel/gateway typed domain。Schedule domain 还提供由 authenticated principal 绑定 source identity 的 typed Event ingress，并为 Workspace、Plugin、Connector、Channel 和 Gateway 暴露五个显式本地 adapter；adapter 负载没有 principal 字段，不能绕过 Scheduler ledger/Grant。Desktop 只把 DTO、principal 和系统窗口接缝交给 AppServer；Broker 负责持久化与唤醒，AppServer 不创建 Model/Tool loop。

Tauri 负责 DTO、系统对话框和 `agent:events` bridge；Scheduler 使用 `service:scheduler` principal，不依赖 Workbench 窗口。AppServer、Workspace 与 Gateway 保持本机、单用户边界。

Release 默认开启 Workspace、MCP、Scheduler 与本地 Host 框架，但默认开启不产生权限；Browser/Chrome 配对、Computer Act、Connector account 和外部 ingress 仍需用户显式授权。Tool 逐次与 Policy、Grant、Approval、Sandbox、Host readiness 和 Run allowlist 求交集。

控制协议统一为 v31。`0019`–`0031` 依次覆盖 Run recovery、AgentTask lease/reconciliation、企业内容、Workbench 环境、持久 CEF Workspace/Tab/Lease、统一 Host policy、企业平台账户编排与官方内置 Channel 绑定。Desktop 与 Gateway 共享文件迁移锁、SQLite Online Backup、manifest/SHA-256、最近三份保留和失败回滚。`RuntimeFeatureSet` 开关只影响入口、工具注册和命令执行，不跳过 migration；停用命令统一返回 `feature_disabled` 和 feature key。

设置的信息架构只保留“平台集成”产品页：正式 Provider catalog 仅包含企业微信、钉钉和飞书，一个逻辑账户关联可独立启停的 Connector/API 与 Channel/消息能力，凭据正文只进入 Windows Credential Manager。Gateway 不暴露总开关，而由首个/最后一个消息账户自动注册、启动、停止和注销。测试 Provider 不进入正式 UI；本地应用访问继续属于 Computer Use。

## 11. P1–P8 已落地架构与后置验证

- **P1–P4**：durable Run recovery、三类 Provider、Remote context 和父子 Agent Task 已接入统一 Store/Executor/Projector；`agent.spawn/send/wait/cancel/collect` 已通过实际 Workbench ToolPlan 和独立 Desktop E2E 调用，Scheduled 只允许持久化精确 allowlist；真实 OpenAI Gate仍环境阻塞。
- **P5**：标准 Git Remote、Agent 原生 `git.remotes/git.push/forge.change.query/forge.change.mutate` 与四类 Forge adapter 已接入共享 Host、side-effect ledger、Credential Manager/GCM/SSH、expected revision/OID 和审批 [ref:GITHUB-API-20260730] [ref:GITLAB-API-20260730] [ref:GITEE-API-20260730] [ref:GITEA-FORGEJO-API-20260730]。mutation 响应未知时只做有界远端查询，并按 source/target、可见字段、状态和 commit OID 精确证明结果；无法证明仍保持 `indeterminate`，不会重放。supplied Approval 重新匹配 Session/Run generation/Tool call/参数哈希/解析主体/一次性 scope/有效期，不能复用旧审批完成 merge；source ref 仍是不可变前置条件。真实 staging Gate 环境阻塞。
- **P6**：完整 contribution lifecycle 和跨产品 reconciliation 已接入；扩展只允许本地/内置/管理员 Bundle，不建设 Marketplace。Plugin Runtime 保留给官方集成，用户管理界面与第三方 Bundle 产品化后续开放。
- **P7**：三个企业 REST driver、事件认证、Channel contribution、EventSource、transport supervisor、结构化 mention、General/Office 可达的 25 MiB 受控附件下载和 Artifact fencing 已接入 [ref:WECOM-API-20260730] [ref:DINGTALK-STREAM-SDK-GO-20260731] [ref:FEISHU-SDK-GO-20260731]；Scheduled account/action/revision fencing 与 `NeedsAttention` 已复用同一验证器；三个外部企业组织 Gate 真实环境待验证，外部组织标识不构成 Hachimi 租户。
- **P8**：统一 Workbench Host Session、Browser/Computer Inspector、Observe-first、接管/恢复和双 Browser lease 原语已接入；终态 Run 必须 fresh generation，真实 Windows UI smoke 环境阻塞。
- **Office**：产品边界是本地 DOCX/XLSX/PPTX/PDF 和文件整理，通过 Skills、普通 Tool/MCP、Artifact 与格式验证完成，不增加在线 Office 服务依赖或专用 Agent Kernel。

Memory 不进入本轮架构；后续独立立项时再确定生命周期、隐私、检索和来源边界。

## 12. 验收与后置

实现状态与真实环境验证使用路线图定义的双状态；本架构文档不维护第二套状态。P1–P8 当前为“代码与本地测试完成／真实环境待验证”。2026-07-31 当前 Windows 工作机已完整通过统一 `corepack pnpm check`，并额外在三个 Runtime Feature Flag 关闭的独立 Desktop E2E 中验证模型 ToolPlan fail-closed；这些结果仍不替代后置真实外部与 Windows 身份 Gate。

系统 Gate 包含两个缺一不可的隔离 Windows Runner：standard-user Runner 必须确认账户不属于 `BUILTIN\\Administrators` 且进程未提升，使用真实发布的 0.2.0 NSIS 升级到同一不可变候选，验证 per-user setup/repair、Workspace/Exec/MCP、Pet、Browser、Computer、本地 Plugin/Connector/Gateway、Scheduled continuation 和不跳过的 Desktop E2E；elevated Runner 必须下载同一候选并验证 linked-worktree ACL/双 lease、handle sentinel、restricted Office/MCP、真实 Scheduler soak、系统 Toast、便携恢复和高权限边界。两个 Gate 都上传脱敏 `summary.json`、同一候选 SHA-256 和必要日志，失败不自动重试。

`v0.2.1` 已取消，升级基线固定为 `v0.2.0`。源码版本为 `0.3.0-alpha.8`；候选状态只由对应 clean commit 的 immutable manifest/Windows run artifact 记录，真实 Gate、tag 和 GitHub Release 尚未完成；alpha.1–alpha.7 未发布并由 alpha.8 合并取代。许可、候选哈希、来源哈希、六类 summary 聚合、候选 Gateway callback、故障代理 reconciliation、三类包解包后许可校验和不可覆盖 tag 的发布架构见 `docs/RELEASE_GATES.md`。管理员窗口、UAC 和安全桌面只验证稳定拒绝，不进入控制范围。

路线图唯一状态见 `docs/ROADMAP.md`；来源登记唯一权威见 `docs/HARNESS_AGENT_SOURCE_PROVENANCE.md`。
