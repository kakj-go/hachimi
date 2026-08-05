# Hachimi Agent 路线图（唯一实现状态源）

更新时间：2026-07-31

本文件是 Agent、Provider、Thread/Workbench、Git/Forge、Browser/Computer、Plugins/Connectors、Channel/Gateway 与 Scheduled Tasks 的唯一实现状态源。README、实现规格、架构说明和代码计划只引用本文件，不维护第二套状态。

## 双状态定义

| 维度         | 状态               | 含义                                                                                |
| ------------ | ------------------ | ----------------------------------------------------------------------------------- |
| 实现状态     | 未实现             | 尚未形成可验收代码                                                                  |
| 实现状态     | 部分完成           | 已有代码，但 migration、产品入口、测试或本地 Gate 仍有缺口                          |
| 实现状态     | 代码与本地测试完成 | 代码、增量 migration/协议、产品入口、单元测试和当前机器可执行的确定性本地测试已完成 |
| 实现状态     | 来源阻塞           | 固定官方来源不足，禁止通过猜测补协议；当前没有此状态的 P1–P8 项                     |
| 实现状态     | 远期               | 当前版本不实施，也不固定实现方案                                                    |
| 真实环境验证 | 本地已验证         | 该项只要求本地验证且已经完成                                                        |
| 真实环境验证 | 真实环境待验证     | 代码和 fixture 不能替代真实服务、租户、系统身份或 Windows Runner 证据               |
| 真实环境验证 | 真实环境已验证     | 对应真实服务/外部组织/系统身份产生的脱敏 artifact 已通过统一证据校验                |
| 真实环境验证 | 不适用             | 该项没有真实外部环境 Gate                                                           |

“代码与本地测试完成”不等于 alpha、RC 或 GA 已发布。fixture、mock、loopback transport 和确定性 Host 只证明本地实现，不构成真实 OpenAI、Forge、外部企业组织或 Windows 发布证据。Hachimi 不增加登录或用户租户体系；企业字段中的 tenant/corp 仅指外部平台组织身份。

## 固定范围与参考基线

- Codex 固定提交 `4c43465133428898aa84f0bfc02c306ed65fb66a` 是统一 Agent、编程、办公、Browser/Computer、Skills、MCP、Plugins/Connectors、Session/Thread 和 Scheduled Tasks 的主基线。
- OpenClaw 固定提交 `f6d456235cf011004f7cffc71a95acf6fbf1fa0a` 只用于本地 Gateway/Channel、确定性路由、Cron/Heartbeat/Event、Task ledger、投递与重启 reconciliation。
- Claude Code Best 固定提交 `34b3dc99bf40c57c0b78f3b5b1d70471ebc2d06d` 只作为公开 Compaction 行为的 clean-room 验收参考。
- OpenAI、Forge 与企业 API 快照分别登记在 `docs/references/openai/registry.json`、`docs/references/forge/registry.json` 和 `docs/references/enterprise/registry.json`。
- 钉钉 Stream wire 来源固定为 SDK Go `v0.9.1` / `d1cc841e6013c3f6513a5bb01dfe3219b9c37d17` [ref:DINGTALK-STREAM-SDK-GO-20260731]；飞书长连接 wire 来源固定为 Go SDK `v3.9.9` / `ff207b774541a195f0a98c5bfda1507905e45431` [ref:FEISHU-SDK-GO-20260731]。两者仅作为协议来源，不作为运行时依赖。

Memory 保持远期。不实现 Marketplace、在线 Microsoft 365/Google Workspace、Remote Workspace 或远程多租户 Control Plane。本地 `hachimi-control-plane` 只是单机内部编排层，Agent、AppServer 与 Gateway 保持本机单用户运行。

## 版本与交付状态

| 版本                       | 计划内容            | 实现状态           | 真实环境验证   | 发布状态与结论                                                                                                                                                                                 |
| -------------------------- | ------------------- | ------------------ | -------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `v0.2.1`                   | 原 R0 封板          | 未实现             | 不适用         | 已取消，不再发布；升级验证基线继续使用 `v0.2.0`                                                                                                                                                |
| `v0.3.0-alpha.1`–`alpha.7` | 原 P1–P7 分批 alpha | 代码与本地测试完成 | 真实环境待验证 | 未发布，由 alpha.8 合并取代；不补造历史 tag                                                                                                                                                    |
| `v0.3.0-alpha.8`           | P1–P8 合并预发布    | 代码与本地测试完成 | 真实环境待验证 | 源码版本、许可、候选构建/校验与独立 alpha prerelease workflow 已接入；候选状态由对应 commit 的 immutable manifest/Windows run artifact 记录；未创建 tag 或发布，alpha 不携带真实 Gate 完成声明 |
| `v0.3.0-rc.1`              | 外部集成收口        | 未实现             | 真实环境待验证 | 未发布；真实 Provider、Forge、三个外部企业组织和两类 Windows artifact 均未执行                                                                                                                 |
| `v0.3.0`                   | GA                  | 未实现             | 真实环境待验证 | 未发布；最终 commit/hash 必须重跑全部 Gate                                                                                                                                                     |

## P1–P8 实现状态

| 阶段 | 能力                                   | 实现状态           | 真实环境验证   | 已落地与边界                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| ---- | -------------------------------------- | ------------------ | -------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| P1   | 崩溃前活动 Run 安全续跑                | 代码与本地测试完成 | 真实环境待验证 | `Recovering`/`WaitingRecoveryDecision`、六阶段 durable checkpoint、真实 side-effect ID、revision snapshot、同 Run generation 递增、旧 lease/token/Approval/临时 Grant 失效、只读/可靠幂等回执恢复、未知 mutation `indeterminate` 与 Resume UI；逐阶段文件数据库 crash injection 已覆盖                                                                                                                                                                                                                                                              |
| P2   | OpenAI 标准 Provider                   | 代码与本地测试完成 | 真实环境待验证 | capability registry、严格 wire 校验、`/v1/chat/completions`、`/v1/responses`、`/v1/embeddings`、Responses tool/usage/cancel/error、兼容档案和 Keyring `secret_ref` [ref:OAI-PRODUCT-CHATCOMPLETIONS-20260730] [ref:OAI-PRODUCT-RESPONSES-20260730] [ref:OAI-PRODUCT-EMBEDDINGS-20260730]                                                                                                                                                                                                                                                            |
| P3   | Remote Compaction / reasoning summary  | 代码与本地测试完成 | 真实环境待验证 | Responses-only capability、配置/probe/响应三重校验、越界/超时/drift 本地回退、只接收 Provider 明确公开的有界 summary，不持久化隐藏 reasoning                                                                                                                                                                                                                                                                                                                                                                                                        |
| P4   | Multi-Agent 与重启恢复                 | 代码与本地测试完成 | 真实环境待验证 | 父子 Task/Run/Session lineage、spawn/send/wait/cancel/collect、权限/allowlist/预算单调收窄、深度/并发限制、execution generation/lease、启动 reconciliation、Scheduled `NeedsAttention`、Usage/Artifact 回收与 UI；五个 `agent.*` 已进入 Workbench General/Coding/Office 的真实 ToolPlan，Scheduled 只接受持久化精确 allowlist，Pet 不开放                                                                                                                                                                                                           |
| P5   | 标准 Git push                          | 代码与本地测试完成 | 真实环境待验证 | 任意标准 Remote URL、managed Git/Workspace Host、GCM/SSH Agent、remote hash/ref/OID fencing、side-effect ledger 与未知结果 reconciliation；Agent 原生 `git.remotes`/`git.push` 仅注册到交互式 Project Coding Run，当前 Project Remote 推导出的精确 host/protocol Grant 只供 Git/Forge 授权链使用；关闭远程 mutation 后仍保留本地 stage/commit                                                                                                                                                                                                       |
| P5   | Forge PR/MR                            | 代码与本地测试完成 | 真实环境待验证 | GitHub、GitLab、Gitee、Gitea/Forgejo create/query/update/close/merge、Credential Manager、expected revision/OID、幂等与合并独立高风险审批 [ref:GITHUB-API-20260730] [ref:GITLAB-API-20260730] [ref:GITEE-API-20260730] [ref:GITEA-FORGEJO-API-20260730]；Agent 原生 `forge.change.query`/`forge.change.mutate` 与 Workbench UI 复用同一 Host/transport；未知响应返回 executor error，使统一 ledger 保持 `Indeterminate`，不包装为可重放普通失败；supplied Approval 会重新核对 generation/tool/参数/主体/一次性 scope/有效期；换源分支必须新建 PR/MR |
| P6   | Plugin contribution 生命周期           | 代码与本地测试完成 | 真实环境待验证 | Skill、Hook、EventSource、MCP、Connector、BrowserExtension、ScheduledTaskTemplate、Asset、CustomUI、Channel 共用 stage/validate/review/activate/health/commit，完整 install/enable/update/rollback/disable/uninstall、known-good、crash reconciliation 和十类无残留矩阵；不实现 Marketplace                                                                                                                                                                                                                                                         |
| P7   | 企业 Connector / Channel / EventSource | 代码与本地测试完成 | 真实环境待验证 | 企业微信 loopback listener 支持官方 GET `echostr` 和 POST 加密 XML，再由 Provider 做 AES/签名/组织/重放校验；钉钉 Stream、飞书 WebSocket/protobuf 的 heartbeat/reconnect/ACK/dedup、REST Connector、Gateway ledger 和 Scheduler typed ingress 已接入 [ref:WECOM-API-20260730] [ref:DINGTALK-STREAM-SDK-GO-20260731] [ref:FEISHU-SDK-GO-20260731]                                                                                                                                                                                                    |
| P7   | mention 与附件                         | 代码与本地测试完成 | 真实环境待验证 | 结构化 User/Bot/All mention；显式 `enterprise.download_attachment` 已进入 Workbench General/Office ToolPlan，Coding/Pet 不开放；25 MiB、MIME/magic/扩展名校验、分块临时下载、原子 Artifact 移入、Run/generation/account/event/remote ID/metadata hash/幂等 fencing；Scheduled 必须精确固定 account、`download_attachment` action 和 contribution revision，缺失或漂移进入 `NeedsAttention`；通用 `connector_invoke` 不能借保留动作绕过附件链；SQLite 不存附件正文                                                                                   |
| P8   | Unified Host 与双 Browser 原语         | 代码与本地测试完成 | 真实环境待验证 | Workbench Session、Browser/Computer Inspector、Observe-first/接管/恢复、Browser history/input/wait/tab/transfer/storage 与静态 CDP allowlist、Computer 鼠标/键盘/窗口/受控启动；Session/lease/observation/frame/action 全部绑定持久 `run_generation`，旧 generation 稳定返回 `stale_run_generation`                                                                                                                                                                                                                                                 |

### P4/P5/P7 模型工具入口矩阵

| 运行入口            | Multi-Agent                                                | 企业附件                                          | Git/Forge                                                                      |
| ------------------- | ---------------------------------------------------------- | ------------------------------------------------- | ------------------------------------------------------------------------------ |
| Workbench General   | `agent.spawn/send/wait/cancel/collect`                     | `enterprise.download_attachment`                  | 不开放                                                                         |
| Workbench Coding    | `agent.spawn/send/wait/cancel/collect`                     | 不开放                                            | 仅交互式 Project Session；Plan mode 只保留 `git.remotes`、`forge.change.query` |
| Workbench Office    | `agent.spawn/send/wait/cancel/collect`                     | `enterprise.download_attachment`                  | 不开放                                                                         |
| Scheduled Workbench | 仅持久化精确 Tool allowlist；Plan mode 只保留 wait/collect | 仅 General/Office，且固定 account/action/revision | 全部不开放，包括 query                                                         |
| Pet                 | 不开放                                                     | 不开放                                            | 不开放                                                                         |

以上矩阵还会与 Feature Flag、Run allowlist、Capability Grant、Plan mode 和 Host readiness 逐次求交集；Project Remote 派生的 Git/Forge Grant 不写回其他 Host 的授权上下文，也不扩大 Connector、Browser 或 Computer 权限。

## 公共基础状态

| 项目                                            | 实现状态           | 真实环境验证 | 结论                                                                                                                                         |
| ----------------------------------------------- | ------------------ | ------------ | -------------------------------------------------------------------------------------------------------------------------------------------- |
| Control protocol                                | 代码与本地测试完成 | 本地已验证   | `CONTROL_PROTOCOL_VERSION = 31`，TypeScript contracts 由 Rust 重新生成                                                                       |
| migrations `0011`–`0018`                        | 代码与本地测试完成 | 本地已验证   | `0018` 保留历史文件名，但 fresh schema 已直接使用 `computer_control_sessions` 与 `host_action_ledger`；当前阶段不兼容旧 DesktopControl 数据  |
| `0019_recovery_alignment.sql`                   | 代码与本地测试完成 | 本地已验证   | Run recovery 状态、revision snapshot 与 checkpoint 对齐                                                                                      |
| `0020_agent_task_recovery.sql`                  | 代码与本地测试完成 | 本地已验证   | AgentTask execution generation、lease 与 reconciliation metadata                                                                             |
| `0021_enterprise_content.sql`                   | 代码与本地测试完成 | 本地已验证   | mention、附件下载与 Artifact 关联                                                                                                            |
| `0022_desktop_generation_fencing.sql`           | 代码与本地测试完成 | 本地已验证   | Browser Session owner Run generation 与 Desktop action fencing                                                                               |
| migrations `0023`–`0028`                        | 代码与本地测试完成 | 本地已验证   | Workbench、Project Tool context、环境摘要和持久 Embedded Browser Workspace/权限/设置                                                         |
| `0029_unified_host_policies.sql`                | 代码与本地测试完成 | 本地已验证   | Browser/Computer 统一 Host policy、可信应用身份与 action ledger                                                                              |
| `0030_enterprise_integration_orchestration.sql` | 代码与本地测试完成 | 本地已验证   | Connector/Channel 收敛为统一企业集成账户和 lifecycle journal                                                                                 |
| `0031_plugin_builtin_channel_binding.sql`       | 代码与本地测试完成 | 本地已验证   | 官方内置企业 Channel 的 Plugin Runtime binding 约束                                                                                          |
| migration 安全                                  | 代码与本地测试完成 | 本地已验证   | 文件数据库共享 `<database>.migrate.lock`、30 秒 busy 失败、SQLite Online Backup、manifest/SHA-256、最近 3 份保留、失败事务回滚；内存库不备份 |
| 七类 Runtime 功能停用开关                       | 代码与本地测试完成 | 本地已验证   | 默认开启；UI 隐藏入口、相关工具不注册、命令返回 `feature_disabled` 和 feature key；Browser/Computer 使用各自独立 capability flag             |

七个 Runtime 开关为：`HACHIMI_DISABLE_RUN_RECOVERY`、`HACHIMI_DISABLE_PROVIDER_EXTENSIONS`、`HACHIMI_DISABLE_PROVIDER_REMOTE_CONTEXT`、`HACHIMI_DISABLE_MULTI_AGENT`、`HACHIMI_DISABLE_GIT_REMOTE_MUTATIONS`、`HACHIMI_DISABLE_PLUGIN_RUNTIME`、`HACHIMI_DISABLE_ENTERPRISE_INTEGRATIONS`。Browser、Computer Observe 与 Computer Act 不再受单一桌面模式开关捆绑。

普通 SQLite 不保存 API/Forge/企业凭据正文、原始或隐藏 reasoning、附件正文、截图、Cookie、临时 Grant、Approval/Host token 或进程输出正文。密钥存 Credential Manager，Git 使用 GCM/SSH Agent。

## 本地验收与后置真实 Gate

本地统一入口为：

```powershell
corepack pnpm check
```

本地矩阵覆盖迁移 18→31、备份/锁/回滚、Run 六阶段 crash injection、Provider conformance、Multi-Agent reconciliation、Git/Forge ledger、十类 Plugin lifecycle、WeCom/DingTalk/Feishu fixture transport、mention/附件 Artifact fencing、Desktop stale generation/observation/frame 与八类功能开关。独立 `agent-tools.e2e.mjs` 通过真实 ToolPlan 验证 spawn→wait→collect、Git/Forge 工具展示、Workbench UI/Agent 双入口共享 Host、一次性 Approval、Remote drift fencing、未知结果不重放和 General/Office 隔离；`agent-feature-flags.e2e.mjs` 在关闭 Multi-Agent、Git Remote Mutations 和 Enterprise Integrations 后验证模型 ToolPlan 不暴露相应工具，同时保留 Coding 只读 `git.remotes`。2026-07-31 当前 Windows 工作机已完整通过 `corepack pnpm check` 及该独立 Feature Flag E2E。Rust、TypeScript 与 TSX 单文件保持不超过 2000 行；前端复用共享组件和样式系统。

| 后置 Gate                   | 实现状态           | 真实环境验证   | 本轮处理                                                                                                                              |
| --------------------------- | ------------------ | -------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| 真实 OpenAI staging         | 代码与本地测试完成 | 真实环境待验证 | Harness 通过产品 Runtime 组合真实 conformance 与确定性 drift/timeout/fallback；无凭据，未执行                                         |
| Forge staging               | 代码与本地测试完成 | 真实环境待验证 | Harness 使用 managed Git Host，并分别要求 GitHub/GitLab/Gitee/Gitea/Forgejo；无环境，未执行                                           |
| 三个外部企业组织            | 代码与本地测试完成 | 真实环境待验证 | REST/钉钉/飞书 stream 与候选 Gateway 的企业微信 HTTPS callback harness 已接入；真实 callback/mention/附件/限流/撤销证据待外部环境执行 |
| standard-user Windows       | 代码与本地测试完成 | 真实环境待验证 | 接受不可变候选并禁止跳过 Desktop E2E；缺真实非 Administrators 交互 Runner                                                             |
| elevated Windows            | 代码与本地测试完成 | 真实环境待验证 | 支持 `-SkipBuild` 并验证同一候选；缺真正提升的交互 Runner                                                                             |
| 许可/版本/候选/证据 harness | 代码与本地测试完成 | 本地已验证     | Apache-2.0/NOTICE、三包哈希、安装/解包后第三方许可与默认 VRM 校验、来源哈希、脱敏、时效与 mismatch/skip fail-closed                   |
| alpha prerelease            | 部分完成           | 不适用         | `publish-alpha-prerelease.yml` 已实现；仅接受成功候选构建并强制披露未完成真实 Gate，当前尚未执行                                      |
| RC/GA tag / GitHub Release  | 部分完成           | 不适用         | `publish-release.yml` 已实现；真实六类 Gate 未齐、commit/hash 不一致或存在 skip 时禁止执行                                            |

## 固定不实现与远期项

| 项目                                      | 实现状态 | 结论                                                                        |
| ----------------------------------------- | -------- | --------------------------------------------------------------------------- |
| Memory                                    | 远期     | 当前不采用 Codex Memory 方案，不创建 Store、检索、migration 或派生实现      |
| Marketplace                               | 不实现   | 只支持内置、本地和管理员分发的内容寻址 Bundle                               |
| Plugin 用户管理界面与第三方 Bundle 产品化 | 后续计划 | Runtime 与 P6 lifecycle 保持可用；当前所有版本入口置灰且不可导航            |
| 在线 Office 服务                          | 不实现   | 不接 Microsoft 365/Google Workspace；保留本地 DOCX/XLSX/PPTX/PDF 与文件整理 |
| Remote Workspace                          | 不实现   | 不建设远程 Workspace 产品                                                   |
| 远程多租户 Control Plane                  | 不实现   | 本地 crate 仅为单机内部编排                                                 |
| 其他企业 Channel                          | 远期     | 当前只支持企业微信、钉钉、飞书                                              |
| 私有 Codex Provider/Realtime/多媒体协议   | 远期     | Provider 只承诺三类公开 OpenAI 标准协议和显式登记兼容档案                   |

## 完成与发布定义

能力只有在代码、migration、产品入口、单元测试、确定性本地测试和来源校验一致后，才可标为“代码与本地测试完成”。真实外部环境缺失不否定本地代码完成度，但必须保持“真实环境待验证”，不得以 fixture 冒充真实连接。

`v0.3.0` 仍需真实 OpenAI/Forge/企业 staging、standard-user/elevated Windows、干净提交、最终候选哈希、来源与许可证据和文档状态全部一致后才能发布。统一接口与配置见 `docs/RELEASE_GATES.md`；当前不得创建 tag 或声称 alpha.8/RC/GA 已发布。
