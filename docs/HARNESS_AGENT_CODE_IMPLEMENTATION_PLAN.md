# Harness Agent 代码实施映射

更新时间：2026-08-08

本文记录代码模块与验收入口，不维护独立功能状态；状态只看 [路线图](ROADMAP.md)。产品行为固定参考：[ref:OAI-PRODUCT-BROWSER-20260730] [ref:OAI-PRODUCT-CHROME-20260730] [ref:OAI-PRODUCT-COMPUTER-20260730] [ref:OAI-PRODUCT-PLUGINS-20260730] [ref:OAI-PRODUCT-SCHEDULED-20260730] [ref:OAI-PRODUCT-RESPONSES-20260730] [ref:GITHUB-API-20260730] [ref:GITLAB-API-20260730] [ref:GITEE-API-20260730] [ref:GITEA-FORGEJO-API-20260730] [ref:WECOM-API-20260730] [ref:DINGTALK-STREAM-SDK-GO-20260731] [ref:FEISHU-SDK-GO-20260731]。

## 1. 固定约束

- 唯一执行入口是 `AgentRunExecutor::execute(AgentRunRequest)`。
- `Run = Codex Turn`、`TranscriptItem = Codex Item`，Pet、Workbench、Scheduler 和 Gateway 不创建第二套模型循环。
- Provider 只承诺公开 OpenAI `/v1/chat/completions`、`/v1/responses`、`/v1/embeddings` 和显式登记的兼容档案。
- 每次采样捕获不可变 `StepContext`；Tool Call 绑定 `step_revision + tool_plan_hash + registry_revision`。
- Tauri 只处理 principal、DTO、系统对话框和 event bridge；Scheduler 只负责 occurrence、claim 和提交 fresh Run。
- Rust、TypeScript、TSX、CSS 单文件必须保持在 2000 行以内，前端复用 `@hachimi/ui` 的组件和 token。

## 2. 模块映射

| 领域               | 主要模块                                                                                                    | 关键职责                                                                                                          |
| ------------------ | ----------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| Agent Kernel       | `crates/hachimi-agent`                                                                                      | Run/Turn、ToolPlan、权限求交、Item 投影、Approval/UserInput、Compaction、Multi-Agent。                            |
| Storage            | `crates/hachimi-storage`                                                                                    | Session/Run/Item、recovery、artifact metadata、迁移与 generation fencing。                                        |
| Control plane      | `crates/hachimi-control-plane`                                                                              | typed AppServer domain handler，统一转发 fs/process/review/mcp/skills/schedule/task/host/plugin/channel/gateway。 |
| Workspace          | `crates/hachimi-workspace`、`crates/hachimi-sandbox`                                                        | 文件、Diff、Patch、Git、受限进程、Windows 路径与 ACL 校验。                                                       |
| Provider           | `crates/hachimi-llm`、`crates/hachimi-model-runtime`                                                        | 三类公开协议、stream/tool/usage/cancel、capability probe 和上下文计数。                                           |
| Scheduler          | `crates/hachimi-scheduler`                                                                                  | At/Every/Cron/Event、invocation ledger、retry、通知和启动 reconciliation。                                        |
| Browser/Computer   | `crates/hachimi-browser`、`crates/hachimi-computer`、`apps/desktop/src-tauri/src/embedded_browser_agent.rs` | observation/lease、浏览器动作、下载、Win32 Observe/Act 和接管。                                                   |
| Plugins/Connectors | `crates/hachimi-extensions`、`crates/hachimi-enterprise`                                                    | 内置 Bundle lifecycle、Connector action/revision fencing、企业 API。                                              |
| Gateway/Channels   | `crates/hachimi-gateway`、`crates/hachimi-channel-providers`                                                | 五个内置 provider、durable ingress/outbox、ACK、重试、去重和消息投递。                                            |
| Desktop UI         | `packages/workbench`、`packages/pet`、`packages/ui`                                                         | Workbench/Pet 路由、设置、任务中心、Inspector、共享组件与视觉测试。                                               |

## 3. 数据与安全边界

每个 Run 在采样前重新读取 `AGENTS.md`、Skill/MCP/Host revision、Workspace/Git 状态和 Sandbox readiness。权限只能收窄，不能由 Prompt、Skill、Hook 或模型文本扩大。旧 generation、lease、Approval、临时 Grant 和 secret 在重启后失效；未知副作用保持 `indeterminate`，不得自动重放。

Browser 使用 origin/capability allowlist 和静态 CDP allowlist；Computer 使用 App/Window fingerprint、Frame/input epoch 和前台 fencing。后台 Computer 只允许结构化预授权的应用身份与动作上限，真实 Windows Gate 仍待验证。Scheduled Browser upload 当前由命令层稳定拒绝，不得在文档中当作已支持能力。

MCP 支持 stdio 与 HTTPS/loopback Streamable HTTP，服务器 schema、OAuth 能力和 Host identity 必须固定；不允许重定向、凭据注入或绕过 Tool Orchestrator。

## 4. 当前收口项

代码层的主要收口顺序如下，详细缺口与验收命令见 [路线图](ROADMAP.md)：

1. 将 Multi-Agent task tree、预算/用量、NeedsAttention 和 Artifact lineage 接入 Workbench。
2. 接入独立 Agent ReviewPanel 的计划审阅、评论、修订和批准流程。
3. 决定 Scheduled Browser upload 是否建设；若建设，补齐预授权、文件 token、站点范围、审计和重启恢复。
4. 拆分接近 2000 行的 Workbench 页面，避免新功能继续堆积在单文件。

## 5. 数据库基线

当前 fresh schema 使用 V2 schema epoch，SQLx 迁移由 `crates/hachimi-storage/migrations/` 顺序管理，最新迁移为 `0042_permission_skill_allowlist.sql`。开发阶段不承诺旧 DesktopControl 数据兼容；升级时以 reset journal、备份和回滚策略保护用户 Project、资源和外观设置。

## 6. 验收入口

```powershell
corepack pnpm check
corepack pnpm test:desktop:e2e
corepack pnpm test:windows:standard-user
corepack pnpm test:windows:release
corepack pnpm test:staging:openai
corepack pnpm test:staging:forge
corepack pnpm test:staging:enterprise
corepack pnpm test:staging:channels
```

普通本地测试证明确定性实现；真实账号、组织、浏览器和 Windows 身份缺失时由路线图记录为 `Environment Blocked`。来源文件、固定提交和派生边界见 [来源登记](HARNESS_AGENT_SOURCE_PROVENANCE.md)。
