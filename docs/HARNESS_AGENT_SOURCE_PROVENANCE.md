# Harness Agent 第三方来源登记

更新时间：2026-07-31

本文只登记实际复制、翻译或实质改写的文件。候选研究路径不等于已移植代码；新增派生文件必须先更新本文件，再进入实现。

## 固定来源

| ID                              | 仓库与 commit                                                     | 许可证与边界                                                                                       |
| ------------------------------- | ----------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| `codex-4c434651`                | OpenAI Codex `4c43465133428898aa84f0bfc02c306ed65fb66a`           | Apache-2.0；选择性移植，保留 SPDX、commit、源路径和修改说明                                        |
| `openclaw-f6d45623`             | OpenClaw `f6d456235cf011004f7cffc71a95acf6fbf1fa0a`               | MIT；选择性移植，保留版权、commit、源路径和修改说明                                                |
| `claude-clean-room-34b3dc99`    | Claude Code Best `34b3dc99bf40c57c0b78f3b5b1d70471ebc2d06d`       | 只研究公开可观察 Compaction 行为；不复制源码、提示词、注释、测试或内部标识符                       |
| `dingtalk-stream-sdk-go-v0.9.1` | DingTalk Stream SDK Go `d1cc841e6013c3f6513a5bb01dfe3219b9c37d17` | MIT；只保存握手、WebSocket、心跳、ACK 和重连的有界 wire 快照，不作为运行时依赖或源码移植           |
| `feishu-sdk-go-v3.9.9`          | Feishu Go SDK `ff207b774541a195f0a98c5bfda1507905e45431`          | MIT；只保存长连接 WebSocket/protobuf、心跳、ACK 和重连的有界 wire 快照，不作为运行时依赖或源码移植 |

## 产品行为与候选研究范围（不是已派生代码）

产品文档只用于定义目标行为、安全边界和验收，不是实现代码来源。候选源码目录只表示研究范围；只有下文列出的精确 Hachimi 目标文件才是已发生的派生。未来适配任何候选文件前，必须先登记精确源路径、目标路径、许可证和修改说明。

| 能力                       | 主参考                                                                                                                                                                                                                                                                                                                    | 边界                                                                                                                                                                                            |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Browser/Chrome             | Codex Browser [ref:OAI-PRODUCT-BROWSER-20260730] 与 Chrome extension [ref:OAI-PRODUCT-CHROME-20260730] 产品文档                                                                                                                                                                                                           | 产品行为和权限以 Codex 为准；当前 `hachimi-browser` 与扩展 transport 为 Hachimi 原创实现。Codex 未公开的 Host 细节只候选研究 OpenClaw 对应文件，不得整体移植 `extensions/browser/**`            |
| Computer Use               | Codex Computer Use [ref:OAI-PRODUCT-COMPUTER-20260730] 产品文档                                                                                                                                                                                                                                                           | App-scoped Observe/Act、用户接管和系统权限以 Codex 为准；Hachimi Windows Host 独立实现                                                                                                          |
| OpenAI Provider            | Chat Completions、Responses 与 Embeddings 官方固定快照 [ref:OAI-PRODUCT-CHATCOMPLETIONS-20260730] [ref:OAI-PRODUCT-RESPONSES-20260730] [ref:OAI-PRODUCT-EMBEDDINGS-20260730]                                                                                                                                              | 只实现公开 wire shape、Remote Compaction 与公开 summary；不复制私有 Codex protocol、Prompt 或隐藏 reasoning                                                                                     |
| Git/Forge                  | GitHub、GitLab、Gitee、Gitea/Forgejo 官方 API 快照 [ref:GITHUB-API-20260730] [ref:GITLAB-API-20260730] [ref:GITEE-API-20260730] [ref:GITEA-FORGEJO-API-20260730]                                                                                                                                                          | 快照只固定 create/query/update/close/merge wire contract；adapter 为 Hachimi 原创，不复制厂商 SDK                                                                                               |
| 企业平台                   | 企业微信官方 API、DingTalk Stream SDK Go v0.9.1 与 Feishu Go SDK v3.9.9 固定快照 [ref:WECOM-API-20260730] [ref:DINGTALK-STREAM-SDK-GO-20260731] [ref:FEISHU-SDK-GO-20260731]                                                                                                                                              | Rust 原创实现 REST、loopback AES callback、Stream/长连接 transport、mention 与附件 Artifact fencing；SDK 快照只提供 wire contract，不链接或复制 vendor SDK 运行时代码                           |
| Plugins/Connectors         | Codex Plugins [ref:OAI-PRODUCT-PLUGINS-20260730]；固定 commit 的 `codex-rs/core-plugins/**`、`codex-rs/core/src/connectors.rs`、`codex-rs/codex-mcp/**`                                                                                                                                                                   | Codex 是本地 bundle、Skills/Hooks、Connectors/MCP、Browser extension、Scheduled task template、custom UI 和权限分层主基线；Hachimi 只建设本地 Bundle 分发                                       |
| Session/Thread 恢复        | Codex `codex-rs/core/src/session/rollout_reconstruction.rs`、`codex-rs/app-server/tests/suite/v2/thread_resume.rs`                                                                                                                                                                                                        | 恢复持久历史，不等于自动重放崩溃前有副作用的活跃 Turn                                                                                                                                           |
| Scheduled Tasks            | Codex Scheduled Tasks [ref:OAI-PRODUCT-SCHEDULED-20260730] 产品文档                                                                                                                                                                                                                                                       | Codex 定义独立/对话内任务、Skills/Plugins、Worktree 和权限产品语义；开源固定 commit 没有完整调度引擎                                                                                            |
| Channels/Gateway           | OpenClaw `src/channels/plugins/binding-routing.ts`、`src/channels/message/{durable-receive,ingress-queue,ingress-retry-policy,receipt,send}.ts`、`src/channels/turn/durable-delivery.ts`、`src/routing/session-key.ts`、`src/gateway/{auth,auth-rate-limit,connection-auth,channel-health-monitor,server-session-key}.ts` | 只参考 Channel plugin contract、确定性消息/Session 路由、pairing/allowlist、bot-loop protection、durable ingress/delivery 和常驻 Gateway；不把整个 `src/gateway/**` 纳入候选范围                |
| Cron/Heartbeat/Task ledger | OpenClaw `src/cron/service/{timer-scheduler,timer-catchup,task-runs}.ts`、`src/cron/{heartbeat-policy,heartbeat-task}.ts`、`src/cron/delivery*.ts`、`src/tasks/{task-registry.store.sqlite,task-registry.maintenance,task-registry.reconcile}.ts`、`docs/automation/{cron-jobs,tasks}.md`                                 | 参考本地 timer、事件触发、Task ledger、投递和后台任务重启 reconciliation；不把 `src/cron/**` 或 `src/tasks/**` 整体作为 Agent Runtime 参考，也不允许 standing orders 或 Prompt 直接授予永久权限 |

OpenClaw 的 Agent Core、通用 Plugin/Provider Runtime、Memory、多 Agent、产品 Prompt，以及有副作用 Turn 的自动恢复均不在默认参考范围。Browser 候选只补足 Codex 未公开的底层 Host 工程细节；一旦 Codex 提供等价公开实现，应优先重新评估 Codex 基线。

P1–P8 新增代码均沿用上方固定派生边界或作为 Hachimi 原创模块实现。OpenAI 三类协议、四类 Forge adapter、企业微信 callback，以及 DingTalk Stream/Feishu 长连接 wire contract 已登记离线快照；普通 CI 校验路径、commit、许可证和 SHA-256。DingTalk/Feishu SDK 只作为协议来源，Rust transport 未引入 vendor SDK 运行时依赖。真实外部企业组织仍待验证，fixture、loopback WebSocket 和 protobuf/ACK 测试不能冒充真实平台连接；外部组织身份不构成 Hachimi 租户。

Memory 已调整为远期，当前不选择 Codex Memory 或其他实现方案，不登记候选派生路径；后续重新立项时再单独补充来源与 clean-room 边界。

## 动态产品文档快照 registry

动态官方文档分别登记在 `docs/references/openai/registry.json`、`docs/references/forge/registry.json` 和 `docs/references/enterprise/registry.json`。每条记录包含稳定 reference ID、产品/版本、canonical URL、retrieved timestamp、原始/规范化快照与 SHA-256。普通 CI 只读本地文件并校验 hash，不访问网络。更新文档必须创建新的 reference ID，不能就地改变已登记快照。

## Codex 派生文件

下列文件均标记 `SPDX-License-Identifier: Apache-2.0`，并在文件头记录固定 commit、源路径和 Hachimi 修改说明。

### Agent Kernel、Turn、Tool 与 Compaction

来源：`codex-rs/core/src/session/{turn,step_context,turn_context,world_state}.rs`、`core/src/tools/{registry,router,parallel,lifecycle,orchestrator,spec_plan,sandboxing}.rs`、`core/src/compact.rs`、`core/src/{agents_md,agents_md_manager}.rs`、`apply-patch/src/*`。

- `crates/hachimi-agent/src/agents_md.rs`
- `crates/hachimi-agent/src/apply_patch.rs`
- `crates/hachimi-agent/src/compaction.rs`
- `crates/hachimi-agent/src/profiles.rs`
- `crates/hachimi-agent/src/step_context.rs`
- `crates/hachimi-agent/src/tool_loop.rs`
- `crates/hachimi-agent/src/tool_orchestrator.rs`
- `crates/hachimi-agent/src/tool_registry.rs`
- `crates/hachimi-agent/src/tool_runtime.rs`
- `crates/hachimi-agent/src/turn_runtime.rs`
- `crates/hachimi-agent/src/workload_resolver.rs`
- `crates/hachimi-control-plane/src/app_server.rs`
- `crates/hachimi-control-plane/src/app_server_domain.rs`

修改：替换为 Hachimi Session/Run、immutable StepContext、ModelClientSession、Capability Grant、SQLite、Workspace Host、Schedule origin 和 Workload overlay；不复制 Codex 产品 Prompt、账户、云、Hook、多 Agent 或 telemetry。

测试：Step revision/ToolPlan hash、旧调用拒绝、并行读/串行副作用、取消、AGENTS 层级、Patch 原子性、Compaction checkpoint/token reconciliation、Workbench/Scheduler 同 Runtime、typed AppServer。

### MCP、Elicitation、Review 与 Diff

来源：`app-server-protocol/src/protocol/v2/{mcp,item,review,fs}.rs`、`rmcp-client/src/*`、`core/src/turn_diff_tracker.rs`、`core/src/review_format.rs`。

- `crates/hachimi-agent/src/mcp_elicitation.rs`
- `crates/hachimi-agent/src/mcp_progress.rs`
- `crates/hachimi-agent/src/mcp_resource_tools.rs`
- `crates/hachimi-agent/src/mcp_tools.rs`
- `crates/hachimi-agent/src/review.rs`
- `crates/hachimi-agent/src/review_tools.rs`
- `crates/hachimi-agent/src/run_diff.rs`
- `crates/hachimi-capabilities/src/mcp.rs`
- `crates/hachimi-capabilities/src/mcp_elicitation.rs`
- `crates/hachimi-capabilities/src/mcp_inventory.rs`
- `crates/hachimi-capabilities/src/mcp_media.rs`
- `crates/hachimi-capabilities/src/mcp_oauth.rs`
- `crates/hachimi-capabilities/src/mcp_progress.rs`
- `crates/hachimi-capabilities/src/mcp_supervisor.rs`
- `crates/hachimi-protocol/src/agent/review.rs`
- `crates/hachimi-storage/src/agent_store/review.rs`
- `crates/hachimi-storage/src/agent_store/workspace_diff.rs`

修改：替换为 Hachimi typed Item、UserInput Broker、Keyring reference、media hash reference、ScheduleGrant、restricted MCP Host 和 Review lineage。Elicitation 不等于 Approval，服务器 annotation 不授予权限。

测试：Resources/Templates/Prompts、OAuth、progress、媒体边界、Elicitation accept/decline/cancel、Review target/Finding、Diff baseline/restart。

### Workspace、Process、Git 与 Windows Sandbox

来源：`app-server/src/fuzzy_file_search.rs`、`core/src/file_watcher.rs`、`apply-patch/src/*`、`git-utils/src/*`、`exec-server/src/server/{process_handler,session_registry}.rs`、`windows-sandbox-rs/src/*`。

- `apps/desktop/src-tauri/src/project_git_commands.rs`
- `apps/desktop/src-tauri/src/app_domain_handler/process.rs`
- `apps/desktop/src-tauri/src/review_commands.rs`
- `apps/desktop/src-tauri/src/workspace_mutation_commands.rs`
- `crates/hachimi-process/src/lib.rs`
- `crates/hachimi-process/src/pty.rs`
- `crates/hachimi-process/src/pty/conpty.rs`
- `crates/hachimi-protocol/src/workspace.rs`
- `crates/hachimi-sandbox/src/process_backend.rs`
- `crates/hachimi-sandbox/src/restricted_process.rs`
- `crates/hachimi-sandbox/src/runtime_attestation.rs`
- `crates/hachimi-sandbox/src/runtime_manager.rs`
- `crates/hachimi-sandbox/src/setup.rs`
- `crates/hachimi-sandbox/tests/windows_smoke.rs`
- `crates/hachimi-workspace/src/diff.rs`
- `crates/hachimi-workspace/src/file_search.rs`
- `crates/hachimi-workspace/src/git.rs`
- `crates/hachimi-workspace/src/git_alias.rs`
- `crates/hachimi-workspace/src/patch.rs`
- `crates/hachimi-workspace/src/review_diff.rs`
- `crates/hachimi-workspace/src/watch.rs`

修改：路径和进程操作改为 Checkout-bound Workspace Worker、Hachimi AppContainer/restricted token、Job Object、security-capabilities + explicit handle-list、native final-path validation、deny-all network、Git plumbing、side-effect ledger 和 Tauri DTO。未嵌入 Codex Core 或其产品策略。`restricted_process.rs` 的 Hachimi handle allowlist/stdio ownership plumbing 是围绕上述来源边界的本地实现；登记在此不把未复制的 Codex 候选文件伪装成逐行移植。

测试：Watch 去重/invalidations、搜索取消、Patch rollback、PTY bytes/resize/kill、Unborn Git、NTFS/reparse/path matrix、restricted process/network smoke。

### Skills progressive disclosure

来源：`core-skills/src/{loader,model,service,render,injection,skill_instructions,invocation_utils,root_loader}.rs`、`ext/skills/src/tools/{list,read}.rs`、`core/src/skills.rs`。

- `crates/hachimi-agent/src/skill_runtime.rs`
- `crates/hachimi-skills/src/catalog.rs`
- `crates/hachimi-skills/src/metadata.rs`
- `crates/hachimi-skills/src/watcher.rs`

修改：使用 Hachimi Run-scoped Catalog、SkillActivation、revision fencing、Workload classification、Built-in/User/Repo/System/Admin roots 和 ScheduleGrant；Skill metadata 只能声明兼容性/诊断，永远不授予权限。

测试：显式 ID、`$name`、隐式 activation、分页、资源越界、namespace、revision 漂移、Worktree 继承、Watcher 和 Office overlay。

## OpenClaw 派生文件

下列文件保留 MIT SPDX、OpenClaw Foundation 版权、固定 commit 和源路径。

来源：`src/process/command-queue.ts`、`src/cron/service/{timer-scheduler,timer-catchup,task-runs}.ts`、`src/tasks/task-registry.store.sqlite.ts`、`src/cron/config-revision.ts`。

- `crates/hachimi-agent/src/session_lane.rs`
- `crates/hachimi-scheduler/src/service.rs`
- `crates/hachimi-storage/src/agent_store/schedule.rs`

修改：改为 Tokio lane/单 timer、SQLite invocation claim、ScheduleDefinition/Grant/TaskRun、fresh Hachimi Session/Run、后台并发、通知、typed Event ledger 和重启 reconciliation。不复制 OpenClaw Agent Core、Prompt 或 Connector 产品逻辑。

测试：lane reset/generation、At/Every/Cron/DST、Skip/CatchUpOnce、重复 invocation、取消、重启、授权/Skill/MCP 漂移和脏 Worktree 上限。

## Claude clean-room 边界

`crates/hachimi-agent/src/compaction.rs` 的代码来源仍是上方登记的 Codex Apache-2.0 `core/src/compact.rs`；Claude Code Best 只影响公开行为验收：保护 recent tail/未完成事项、反复压缩质量警告、失败保留旧 checkpoint 和 token reconciliation。没有 Claude 源码、提示词、测试、注释或内部标识符进入仓库。

## 原创模块说明

`hachimi-model-runtime`、AgentStore migrations、active delta hub、metadata-only Audit、Capability Grant、UserInput secret broker、Office Skills 内容、Workbench UI、Task Center、Tauri event bridge 和 StorageLayout 是按 Hachimi contract 独立实现；其行为可能与上述架构约束互操作，但未标记为 copied/translated/adapted。

本期 Local Host 的以下实现同样是 Hachimi 原创代码，只使用上表的产品行为/安全边界参考，没有复制候选源码：

- `crates/hachimi-browser/**` 与 `assets/browser-extension/**`；
- `crates/hachimi-computer/**`；
- `crates/hachimi-extensions/**` 与 `assets/plugins/sample-crm/**`；
- `crates/hachimi-gateway/**`、`apps/desktop/src-tauri/src/{gateway_process,gateway_runtime}.rs`；
- `crates/hachimi-protocol/src/agent/{hosts,plugins,gateway,local_hosts}.rs`；
- `apps/desktop/src-tauri/src/{agent_host_tools,browser_extension_server,local_host_commands,managed_sandbox_runtime}.rs` 与 `app_domain_handler/local_hosts.rs`；
- `packages/workbench/src/local-hosts-settings.{tsx,css}`。
- `crates/hachimi-scheduler/src/event.rs`、`crates/hachimi-storage/src/agent_store/schedule_event.rs` 与 `packages/workbench/src/task-event-form.tsx`。
- `crates/hachimi-agent/src/multi_agent.rs`、`crates/hachimi-protocol/src/agent/{agent_tasks,recovery,git_forge,enterprise}.rs`；
- `crates/hachimi-llm/src/{responses,embeddings}.rs`、`crates/hachimi-model-runtime/src/lib.rs` 中的公开 Provider adapter；
- `crates/hachimi-forge/**`、`apps/desktop/src-tauri/src/forge_commands.rs` 与 Git/Forge operation ledger；
- `crates/hachimi-enterprise/**`、`crates/hachimi-extensions/src/enterprise_connector.rs`、`crates/hachimi-gateway/src/enterprise_provider.rs` 与 `assets/plugins/{wecom,dingtalk,feishu}/**`；
- `packages/workbench/src/desktop-control.{tsx,css}` 与 DesktopControl durable state/action ledger。

OpenClaw 的 Channel/Gateway 文件用于确定性 route、durable ledger 和 reconciliation 行为对照；当前 `hachimi-gateway` 没有标记为 OpenClaw 派生文件。若后续实质改写候选实现，必须先把精确目标文件移入上方“OpenClaw 派生文件”清单并补齐 MIT SPDX/来源头。

`pnpm provenance:check` 扫描所有带 `Adapted from`、`Translated from` 或 `Modified for Hachimi:` 的源码，要求本文件存在精确目标路径、文件含 SPDX 和固定 commit。候选来源不能伪装成已移植代码。
