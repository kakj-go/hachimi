# Harness Agent 代码实施计划（当前基线）

产品/API 行为固定参考：[ref:OAI-PRODUCT-BROWSER-20260730] [ref:OAI-PRODUCT-CHROME-20260730] [ref:OAI-PRODUCT-COMPUTER-20260730] [ref:OAI-PRODUCT-PLUGINS-20260730] [ref:OAI-PRODUCT-SCHEDULED-20260730] [ref:OAI-PRODUCT-RESPONSES-20260730] [ref:GITHUB-API-20260730] [ref:GITLAB-API-20260730] [ref:GITEE-API-20260730] [ref:GITEA-FORGEJO-API-20260730] [ref:WECOM-API-20260730] [ref:DINGTALK-STREAM-SDK-GO-20260731] [ref:FEISHU-SDK-GO-20260731]。

更新时间：2026-07-31

实现状态只以 `docs/ROADMAP.md` 为准；本文描述代码结构、实施顺序和验收映射，不单独定义发布状态。

## 1. 固定约束

- 唯一执行入口是 `AgentRunExecutor::execute(AgentRunRequest)`。
- `Run = Codex Turn`、`TranscriptItem = Codex Item`，不引入第二套 Thread/Turn 模型。
- Provider 只支持公开 OpenAI `/v1/chat/completions`、`/v1/responses`、`/v1/embeddings` 三类协议和显式登记、probe 匹配的兼容档案；不接私有 Codex、Realtime 或多媒体协议。
- 每次采样捕获不可变 `StepContext`，其中包含 world state、分层 `AGENTS.md`、SkillActivation、MCP binding、Host/Sandbox readiness、Model View、预算和 `ToolPlan`。
- `step_revision + tool_plan_hash + registry_revision` 是 Tool Call 的前置条件；旧 revision、未知工具、Schema 不匹配、Grant/Sandbox 收窄失败均 fail closed。
- Tauri 只处理 principal、DTO、系统对话框和 event bridge；Scheduler 只计算时间、claim invocation、提交 fresh Run。
- Pet 使用 `EntryProfile::PetConversation` 创建统一 Session/fresh Run，并只把最终输出交给 Motion/TTS presentation；不创建专用 ToolLoop、模型循环或权限模型。
- Browser、Computer、Plugin/Connector、Channel/Gateway 和 Session-bound Scheduled continuation 只能作为 typed Host/Domain/transport 接入同一 `AgentRunExecutor`，不得创建第二套 Agent Loop、权限模型或事件模型。
- 前后端新增文件低于 2000 行；不把逻辑堆回 `main.rs`、`workbench_commands.rs` 或 `home.tsx`。

## 2. Fresh 数据库基线

当前基线使用二十二个增量 migration，不提供 typed/untyped 双读：

| Migration                                       | 内容                                                                                                                   |
| ----------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `0001_agent_kernel.sql`                         | Session/Run、typed Item、Event、Approval、UserInput、Audit、usage、Compaction、side-effect ledger                      |
| `0002_workspace_extensions.sql`                 | Project/Checkout、Diff、Process、Review、MCP、Skills、MCP Host identity                                                |
| `0003_automation.sql`                           | ScheduleDefinition、ScheduleGrant、TaskRun、Delivery、invocation key                                                   |
| `0004_local_hosts.sql`                          | Browser/Computer 授权、Plugin/Connector metadata、Channel ingress/outbox、continuation binding                         |
| `0005_connector_transport.sql`                  | Connector transport ledger、持久 mock-poll inbox、Browser 首选模式和 Computer 全局规则                                 |
| `0006_local_host_completion.sql`                | Browser document/resource 网络规则与待授权请求、Plugin runtime revision、Channel provider/account、Schedule Host Grant |
| `0007_plugin_permission_review.sql`             | Plugin scope 差异与显式权限复核                                                                                        |
| `0008_plugin_runtime_bindings.sql`              | Hook execution/subscription 及 MCP、Browser extension、Scheduled template、Asset、Custom UI、Channel runtime binding   |
| `0009_channel_provider_runtime.sql`             | Channel contribution 启用状态和 provider runtime 索引                                                                  |
| `0010_schedule_events.sql`                      | Event Schedule matcher、metadata-only 去重 ledger、typed resource reference、TaskRun event context 与关联              |
| `0011_run_recovery.sql`                         | durable step checkpoint、Run recovery record、generation-safe decision                                                 |
| `0012_provider_registry.sql`                    | Provider endpoint/account/protocol/profile/capability registry                                                         |
| `0013_provider_remote_context.sql`              | Remote Compaction state、公开 summary 来源与降级 metadata                                                              |
| `0014_agent_tasks.sql`                          | 父子 Agent Task/Run、预算、消息、取消与产物 lineage                                                                    |
| `0015_git_forge_operations.sql`                 | Git/Forge mutation、expected revision/OID、idempotency 与未知结果 ledger                                               |
| `0016_plugin_revisions.sql`                     | Plugin revision head、known-good、lifecycle journal 与 rollback                                                        |
| `0017_enterprise_integrations.sql`              | 企业账号、token/rate-limit、event receipt、attachment metadata、operation ledger                                       |
| `0018_desktop_control_state.sql`                | 历史 Host 状态 migration；当前由 `0029_unified_host_policies.sql` 迁移到统一 ledger                                    |
| `0019_recovery_alignment.sql`                   | Run recovery 状态、revision snapshot 与六阶段 checkpoint 对齐                                                          |
| `0020_agent_task_recovery.sql`                  | AgentTask execution generation、lease owner/expiry 与 reconciliation metadata                                          |
| `0021_enterprise_content.sql`                   | 结构化 mention、附件下载 fencing 和 EnterpriseAttachment Artifact 关联                                                 |
| `0022_desktop_generation_fencing.sql`           | Browser Session owner Run generation 与 Browser/Computer action generation fencing                                     |
| `0023`–`0028`                                   | Workbench v2、Project Tool context、环境摘要与持久 Embedded Browser Workspace/权限/设置                                |
| `0029_unified_host_policies.sql`                | Browser/Computer 统一 Host policy、可信应用身份与 action ledger                                                        |
| `0030_enterprise_integration_orchestration.sql` | Connector/Channel 收敛为统一企业集成账户和 lifecycle journal                                                           |
| `0031_plugin_builtin_channel_binding.sql`       | Plugin Runtime 对官方内置企业 Channel 绑定的持久化约束                                                                 |

`CONTROL_PROTOCOL_VERSION = 31`。文件数据库 pending migration 使用 Desktop/Gateway 共享 `<database>.migrate.lock`；SQLite Online Backup、manifest/SHA-256、最近三份保留、30 秒 `database_migration_busy` 和失败事务回滚均由同一实现提供。内存数据库不备份。

Transcript 只有 `payload_json`；secret、原始 Process 输出、附件正文、连接订阅、delta 和高权限 lease 不持久化。

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

`hachimi-scheduler` 使用 Codex Scheduled Tasks 的后台任务、Skills/MCP、Worktree 和 NeedsAttention 产品语义，以及 OpenClaw 式持久定义 + runtime state + 单个最近到期 timer。`Standalone` 创建 fresh TaskRun/Session/Run；`SessionContinuation` 在既有 Session lane 创建 fresh TaskRun/Run。ScheduleGrant 固化 Skill content/tree revision、MCP schema/Host identity、Plugin/Connector contribution、权限上限和 workload/context；任何漂移、Sandbox 缺失、Approval/UserInput 或越界自动 Skill 都进入 `NeedsAttention`，不在后台等待。

### Compaction

CompactionService 保留 Claude Code Best clean-room 行为并接入 Responses-only Remote Compaction [ref:OAI-PRODUCT-RESPONSES-20260730]。旧 checkpoint 只在新结果完整验证后替换；远程失败、超时、越界或 drift 回退本地。公开 summary 只来自 Provider 明确可展示字段，不从隐藏 reasoning 推断。

### Workspace/Sandbox

Workspace Host 负责文件、Git、Watch、Search、Diff 和 Patch；Kernel 不直接访问文件系统。Windows Sandbox runtime manager 在 per-user data root staging 固定 SHA-256 的 sidecar/portable Git，创建当前用户 AppContainer Profile，并进行 ACL、restricted token、Job Object、handle、filesystem 与 deny-all network canary。setup/repair 不使用 `runas` 或 UAC；Workspace 必须是当前用户所有的本地 NTFS 非保护目录。未达到四项 enforced 时写入、Exec、stdio MCP、Browser/Computer 和后台副作用全部拒绝。

### Pet、Browser 与 Computer

Pet 的 `start_pet_turn` 只负责创建统一 Run 并提交唯一 `AgentRunExecutor`；Approval/UserInput 使用同一持久 Item，Pet/Workbench 竞争解析只有一个 CAS 胜者。TTS 只消费稳定 Assistant 文本，secret UserInput Run 强制无声。

Browser Host 已实现 managed Chromium 与 Chrome extension、history/input/wait/tab/transfer/storage 原语、owned-tab DNR 和静态 CDP allowlist；拒绝任意 `Runtime.evaluate`、Target attach、网络拦截与调试器逃逸。Computer Host 已实现鼠标/键盘、窗口管理和受控应用启动；动作绑定 Frame/App/Window fingerprint、input epoch、前台窗口和 generation，拒绝高完整性、安全桌面、登录桌面与 Hachimi 窗口。截图仍只保存在受限内存仓库。

### Plugin/Connector、Channel/Gateway

Plugin Bundle 已实现统一 lifecycle journal：stage/validate/permission review/activate/health/commit，以及 install/enable/disable/update/rollback/uninstall、known-good revision、崩溃 reconciliation、账号撤销和跨产品清理。所有 contribution 继续通过 Sandbox/唯一 Runtime。企业微信官方 GET `echostr`/POST 加密 callback、钉钉 Stream、飞书 WebSocket/protobuf、结构化 mention、受控附件下载和 durable ledger 已接入 [ref:WECOM-API-20260730] [ref:DINGTALK-STREAM-SDK-GO-20260731] [ref:FEISHU-SDK-GO-20260731]；三个外部企业组织 Gate 真实环境待验证。

Workbench 设置只暴露“平台集成”：由正式 manifest 生成企业微信、钉钉、飞书 Tab，并通过类型化凭据表单编排同一逻辑账户下的 Connector/API 与 Channel/消息能力。Gateway 的登录启动和进程需求由消息账户数量自动 reconcile；底层 Connector、Channel、Gateway domain 仅供 Runtime、诊断与测试。Plugins 用户入口在所有版本置灰，P6 Runtime/lifecycle 的完成结论不变，第三方 Bundle 用户管理界面后续开放。

## 4. 收口实施顺序

当前实现状态、真实环境验证和发布边界只以 `docs/ROADMAP.md` 的双状态为准。以下编号保留实施顺序和验收映射，不另建状态体系。

1. **已完成：AppServer 全域挂接**：生命周期、Approval/UserInput、fs/process/review/mcp/skills/schedule/task 均由可注入 typed handler 处理。
2. **已完成：实时 UI reducer**：Assistant/Reasoning/Tool delta 使用 active item reducer；completed payload 删除 delta 并保持权威。
3. **已完成：普通 Desktop E2E**：真实 WebView2 spec 使用 deterministic Provider/Sandbox/Browser fixture，覆盖 Git/草稿、Plan、Approval、secret UserInput、Diff/Evidence、Review、重启投影、Skills/MCP、Thread Fork/Steer、Task Center Event accepted/replayed/conflict、Office fixture、Chrome 扩展 broker、内置 `sample-crm`/Connector、两个 Channel provider、Gateway reconciliation，以及 Pet/Workbench 同一 Run 的跨窗口 UserInput/Approval。真实 Sandbox、managed Chromium、WGC 与 Gateway startup 只由 Windows 系统 Gate 验证。
4. **已完成：无窗口后台 Runtime**：`service:scheduler` principal 直接进入唯一 `AgentRunExecutor`，验证 background priority 及 Run、Transcript、Usage、终态持久化。
5. **已完成：Skill/Task 漂移收口**：Scheduled MCP 使用精确 server/tool/schema/Host identity fencing；隐式 Office activation 只改变下一 Step workload，不扩大 Grant；漂移进入 `NeedsAttention` 并创建 fresh continuation。
6. **已完成：Task 高级终态**：成功 TaskRun 禁止 retry，Failed/TimedOut/Cancelled/Lost 使用 fresh TaskRun/invocation key retry；通知失败只影响 DeliveryStatus。
7. **环境阻塞：Windows release Gate**：MSI/NSIS/便携 ZIP 只构建一次，两个 Runner 下载并重新哈希同一候选。`test:windows:standard-user` 必须在真实非 Administrators 且未提升的 Runner 上，使用真实发布的 0.2.0 NSIS 执行跨版本升级、managed Chromium、真实 Chrome extension、记事本 WGC/前台受控输入、HKCU Gateway startup 和不跳过的 Desktop E2E。`test:windows:release -SkipBuild` 必须在真正 elevated Runner 上执行 linked-worktree ACL、handle sentinel、restricted Office/MCP、真实 Scheduler soak、系统 Toast、便携恢复与高权限拒绝。
8. **已完成：本地 Office 与 extension 验收**：restricted stdio MCP、本地 DOCX/XLSX/PPTX/PDF 修改/预览/Diff/导出/Artifact、中断恢复与文件整理回滚已覆盖；不增加在线 Office 服务依赖。
9. **已完成：Desktop 测试进程生命周期**：restart/session/failure/finally 只保留最新的精确 E2E 应用实例并终止 WebDriver 进程树；Workspace Worker/MCP stdio 后台启动使用 `CREATE_NO_WINDOW`，完整 Desktop gate 结束后无残留进程。
10. **已完成：Pet 统一执行与输出安全**：删除独立模型循环；稳定 Assistant 输出、取消、Approval/UserInput 跨界面接续及 secret 语音隔离已接通。
11. **已完成：Browser Host 原语**：managed Chromium/Chrome extension、站点授权、owned tabs、history/input/wait/tab、transfer/storage 和静态 CDP allowlist 已实现；真实 Windows 证据环境阻塞。
12. **已完成：Computer Host 原语**：WGC、鼠标/键盘/窗口/受控启动、Frame/input epoch fencing、接管与高权限拒绝已实现；真实记事本 smoke 环境阻塞。
13. **已完成：Plugin lifecycle**：完整 contribution transaction、revision/known-good、升级/回滚/卸载与重启 reconciliation 已实现。
14. **代码与本地测试完成：企业 Channel/Gateway**：本地 Gateway、三个企业 REST/事件安全层、WeCom URL 验证与加密 XML callback、DingTalk Stream、Feishu 长连接、mention/附件和 Bundle 已实现；三个外部组织 Gate 后置。
15. **已完成：Session-bound Scheduled continuation**：既有 Session lane 的 fresh Run、压缩上下文续接、fresh Grant/Host snapshot、Task Center Connector selection、isolated Browser unattended Grant、Plugin/Connector 漂移、停止条件和 heartbeat 已实现；Computer unattended 稳定返回 `computer_unattended_unsupported`。
16. **已完成：Thread/Workbench 延续**：终态 Run 边界 Fork 只复制可安全重建的完成历史并生成新 Item/sequence；General/Project Session 均可继续，活动 Run 走 generation-fenced Steer，列表支持 rename/pin/archive/search 和 lineage。
17. **已完成：Computer 内存截图**：WGC Frame 使用 `Frame::buffer()` 与 PNG encoder 直接写入受 TTL/容量限制的内存仓库；read/release/expiry/takeover/错误路径均不创建新临时截图文件，启动只清理旧版明确匹配的 UUID PNG。
18. **已完成：Event Schedule**：Control protocol v20、`0010` migration、事务 fan-out/claim、去重/conflict、重启 reconciliation、五类 AppServer adapter、Plugin EventSource、Task Center Event 表单/receipt 和 Desktop E2E 已接通。
19. **已完成 P1**：durable checkpoint、可信恢复分类、generation/revision fencing、`indeterminate` 与 Resume UI。
20. **已完成 P2**：capability Provider registry 与 Chat/Responses/Embeddings strict adapter；真实 OpenAI Gate 环境阻塞。
21. **已完成 P3**：Remote Compaction 本地回退与公开 summary 边界；真实 staging 环境阻塞。
22. **已完成 P4**：父子 Agent Task/Run、权限/预算收窄、消息/取消/产物与 lineage UI；五个 `agent.*` 已加入 Workbench General/Coding/Office ToolPlan，Scheduled 只接受持久化精确 allowlist，Pet 不开放；独立 Desktop E2E 已真实执行 spawn→wait→collect。
23. **已完成 P5 本地实现**：标准 Git push、Agent 原生 `git.remotes/git.push/forge.change.query/forge.change.mutate` 与四类 Forge adapter 已接入；Agent 与 Workbench command 复用 Remote/Workspace/transport/Credential/reconciliation Host，Agent 只在交互式 Project Coding 注册；Project Remote 推导的精确 host/protocol Grant 只进入 Git/Forge 授权上下文；Credential Manager/GCM/SSH、审批、revision/OID/idempotency/side-effect ledger 已覆盖 [ref:GITHUB-API-20260730] [ref:GITLAB-API-20260730] [ref:GITEE-API-20260730] [ref:GITEA-FORGEJO-API-20260730]；supplied Approval 重新校验 generation/Tool/参数/主体/一次性 scope/有效期，保证 merge 使用独立精确审批；mutation 响应未知时返回 executor error 并保持统一 ledger `Indeterminate`，只按 source/target/字段/状态/OID 查询证明，不重复 mutation。真实 staging 环境阻塞。
24. **已完成 P6**：完整 Plugin contribution lifecycle 与产品入口 reconciliation。
25. **代码与本地测试完成 P7**：企业 REST、事件验证、Bundle、Channel/EventSource、transport supervisor、结构化 mention、附件下载与 ledger 已完成；`enterprise.download_attachment` 已进入 General/Office ToolPlan，Scheduled 复用共享验证器精确校验 account、`download_attachment` action 和 contribution revision，缺失/漂移映射 `NeedsAttention`，通用 `connector_invoke` 不能绕过专用附件链；真实三个外部组织待验证。
26. **已完成 P8 本地实现**：统一 Workbench Host Session、Browser/Computer Inspector、Observe-first、双 Browser lease 与 durable action ledger；真实 Windows smoke 环境阻塞。

Memory 调整为远期，本轮不创建 migration、Store、检索方案或 Codex Memory 派生实现。

## 5. 本地 AppServer 接口

当前 typed façade 已覆盖 `initialize`、Session search/resume/fork/metadata、Run steer/interrupt precondition、Run Event subscribe/unsubscribe，以及以下全部 domain：

```text
approval.*  user_input.*  fs.*  process.*  review.*
mcp.*       skills.*      schedule.*  task.*
browser.*   computer.*    plugin.*    connector.*
channel.*   gateway.*
```

所有 mutation 使用 request/client/protocol/idempotency；活跃 Run 使用 expected Run/generation；Schedule 使用 expected config revision。unsubscribe 只停止推送，不取消 Run。

上述 typed domain 均已接入 Desktop domain handler；Scheduled continuation 复用现有 `schedule.*`、Session lane 和 Run lifecycle。`schedule.*` 还提供由 authenticated principal 绑定 source identity 的 typed Event ingress，AppServer 分别暴露 Workspace、Plugin、Connector、Channel 和 Gateway 五个本地 adapter，调用负载不能声明 principal，也不能绕过 Scheduler ledger/Grant。Gateway 是认证 transport/Host，不拥有独立 Agent 协议。

## 6. 验收矩阵

- Runtime：Interactive/Scheduled StepContext hash、ToolPlan、Policy/Sandbox decision 和 Item lifecycle 一致。
- 安全：重复副作用、取消竞争、Prompt Injection、Windows path/reparse、Sandbox fail-closed、secret 不落盘。
- 生命周期：断线继续执行、snapshot watermark 无缺口/重复、stale generation 拒绝、重启 recovery/人工决策与未知副作用不重放。
- Skills：显式、`$name`、隐式激活；资源未激活拒绝；revision 漂移不扩大授权。
- Tasks：At/Every/Cron/Event、DST、Skip/CatchUpOnce、source principal/matcher、去重/冲突/fan-out、并发、reconciliation、NeedsAttention、continuation 新 Run。
- Office：五个 Skill 的本地真实格式 Artifact E2E，操作均通过普通 Tool/MCP，不增加在线 Office 服务依赖或 Office Kernel 分支。
- Pet 输出：统一 Run、跨界面 Approval/UserInput CAS、受控 Catalog、稳定文本 TTS、secret 无声、取消同步停止。
- Browser：隔离/Chrome extension、origin/capability 授权、Prompt Injection、CDP/Download/Upload 分权、过期 observation 和清理。
- Computer：App allowlist、Frame 过期、用户接管、自身审批/管理员/安全桌面拒绝，以及截图不持久化。
- Plugin/Connector：安装与账号授权分层、Action 权限、Schema/Host identity/account 漂移、Webhook/Poll、撤销、重试/幂等和 metadata-only Audit。
- Channel/Gateway：Account/Peer/Thread 路由、DM/group/thread 隔离、auth、bot-loop protection、入站去重、投递回执/重试和重启 reconciliation。
- Session-bound continuation：同 Session lane 的 fresh Run、压缩上下文续接、权限重新固定、停止条件、heartbeat 和无临时授权恢复。

P1–P8 新增验收矩阵：

- Run 恢复：进程在 sampling、Tool 等待、只读 dispatch、幂等 dispatch、未知副作用和 checkpoint 写入各边界崩溃，恢复后不得重复外部副作用或恢复临时授权。
- Provider：Chat Completions、Responses 与 Embeddings 共用 provider-neutral conformance，覆盖 stream、Tool、usage、cancel、overflow、Remote Compaction、公开 summary、capability drift 和降级；不测试未支持的私有 Codex/媒体协议。
- Multi-Agent：EntryProfile × Workload × Mode 正反矩阵、Feature Flag、Scheduled 精确 allowlist、子 Agent 权限/预算收窄、取消传播、重启恢复、Usage 汇总与 Artifact lineage 无缺口；产品 E2E 真实执行 spawn→wait→collect。
- Git push/PR：Agent 与 Workbench UI 复用同一 Host；任意标准 Remote 的 fetch/push conformance；Plan mode 只保留 remotes/query；GitHub、GitLab、Gitee、Gitea/Forgejo 的 PR adapter 覆盖 remote/ref/OID 漂移、凭据撤销、重复请求、网络未知结果和并发更新；未知结果进入 `Indeterminate` 且 mutation 不重放。未知平台只能生成草稿。
- Plugin/企业平台：升级/回滚/卸载无残留 contribution；企业微信、钉钉、飞书覆盖账号隔离、签名校验、入站去重、线程路由、限流、投递回执和重启恢复；Scheduled 企业附件覆盖 account/action/revision 成功、缺失和漂移的 `NeedsAttention` 路径。
- Unified Hosts：Workbench 正式入口只使用唯一 Runtime；Browser/Computer 动作覆盖 stale observation/frame、接管、敏感动作审批和高权限/安全桌面拒绝。

本轮新增完成证据：

- `hachimi-agent::run_runtime_tests::service_principal_executes_a_background_run_without_a_window_transport` 验证 Scheduler 不依赖 WebView/Tauri window。
- `hachimi-scheduler` 测试验证 retry 状态边界、fresh invocation 和 DeliveryStatus 隔离。
- Workbench/Vitest 验证高级 Cron/IANA timezone、Cancelled retry 和 NeedsAttention continuation。
- Desktop/WebView2 E2E 验证 Detached Git、Office Skill 隐式激活/资源失败恢复、MCP schema 漂移后的 NeedsAttention/continuation，以及 Connector credential 撤销后的同 Session fresh interactive continuation。
- Scheduled Run 的空 MCP allowlist 不再解释为“允许全部”；Interactive binding 进入 StepContext revision，Review 不接受未固定 MCP。
- `hachimi-scheduler` ignored release soak 使用真实 `SystemClock` 覆盖短期 At、anchored Every、6-field Cron 和 20+ occurrence，无重复 key、无漂移和无泄漏 active launch。
- Browser 默认模式由持久 Host settings 决定，Chrome extension 只使用最新未过期确认配对；真实 unpacked extension E2E 覆盖 popup nonce、任务 tab group、document/resource DNR 与 Take over 清理。Computer 提供窗口发现、Session 级审批规则、Always-allowed Apps UI、前台 fencing、metadata-only Audit 与截图即时释放测试。
- `sample-crm` Webhook/Poll/retry ledger 与 `mock-poll` 重启/重复消息测试已加入；Gateway transport 不再组装 Agent Runtime，claimed ingress 经 typed AppServer 后由唯一 `AgentRunExecutor` 执行。
- Windows release 脚本分别运行 handle sentinel、Workspace Worker、Agent `workspace_exec`、restricted stdio MCP 和 Terminal/ConPTY smoke；Toast 由 Windows Shell UI Automation 校验任务名与终态。
- linked worktree Git mutation smoke 强制先用独立 lease stage 并恢复 RX，再用第二 lease commit；ACL 同时显式升级 shared common-dir 与 per-worktree git-dir，任一步准备失败都会回滚。
- Desktop E2E 覆盖 Workbench core、Agent tools、Skills/MCP、按领域拆分的 Host 集成和 Task Center；独立 Agent tools spec 的 9 个场景验证 Multi-Agent 实际调用、General/Office 企业附件隔离、Git/Forge 全生命周期、Workbench UI/Agent 双入口共享 Host、一次性 Approval、Remote drift、凭据撤销、重复 mutation receipt 与未知结果不重放；独立 Feature Flag spec 在关闭 Multi-Agent、Git Remote Mutations 和 Enterprise Integrations 后验证真实模型 ToolPlan fail-closed，同时保留 Coding 只读 `git.remotes`。Plugin、Connector、Channel 与 Gateway E2E 会跨应用重启验证各自的类型化命令和 ledger，并在结束前撤销临时 Connector 凭据；Workbench core 验证 Pet/Workbench 对同一 Approval/UserInput 的 CAS 接续；Task Center 覆盖 Event accepted/replayed/conflict 投影。
- Desktop E2E 已消除 deprecated frame API、stale element 和滚动降级噪音；串行完整 Gate 结束后 Hachimi、WebDriver、Gateway、Browser broker 与 sidecar 进程/端口均无残留。
- Storybook 静态构建和 128 个严格视觉/可访问性场景通过，覆盖 light/dark、zh-CN/en-US、100%/125%/150%、生产 Workbench 响应式布局、Task Center、设置页、Pet 和 WCAG A/AA；Tool Execution 可滚动结果支持键盘聚焦，表单说明文字满足对比度门槛。

代码能力与可执行本地 Gate 的双状态见路线图。`v0.2.1` 已取消，当前源码统一为 `0.3.0-alpha.8`；许可、全 workspace 版本、候选 hash、staging harness、候选 Gateway callback、Forge 故障响应 reconciliation、三类包许可内容校验、六类证据聚合、Windows 单次构建和不可覆盖 tag 的发布 workflow 已实现。2026-07-31 当前 Windows 工作机已完整通过 `corepack pnpm check` 与独立 Feature Flag Desktop E2E。Desktop E2E 固定使用 `tauri-driver 2.0.6`，并按已安装 Edge 精确准备经过 Authenticode、版本和 SHA-256 校验的 Edge WebDriver；缓存可按需重建。真实 OpenAI/Forge/三个外部企业组织/五平台 Channel 及两个 Windows 系统 Gate 仍后置。

`publish-alpha-prerelease.yml` 只允许使用 `Windows Release Gate` 的成功候选构建产物发布 alpha，并在发布说明中固定披露真实外部 Gate 和两类 Windows 身份 Gate 尚未形成通过结论。`publish-release.yml` 只处理 RC/GA；六类真实证据不齐、commit/hash 漂移或存在 skip 时不得创建对应 tag 或发布。任何已存在 tag 都不得覆盖，失败后递增 alpha/RC 序号。

固定门槛见 `docs/ROADMAP.md`，Gate 配置见 `docs/RELEASE_GATES.md`。fixture、mock、loopback transport 和确定性 Host 只证明本地实现；真实标准用户 Runner、elevated Runner、真实 0.2.0 基线、外部服务凭据或组织环境不可用时必须保持“真实环境待验证”，不能补造或伪造 Gate 证据。企业组织标识不引入 Hachimi 登录或租户体系。
