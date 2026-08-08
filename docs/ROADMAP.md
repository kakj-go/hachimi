# Hachimi Agent 路线图（唯一实现状态源）

更新时间：2026-08-07

本文件是 Agent、Provider、Thread/Workbench、Git/Forge、Browser/Computer、Plugins/Connectors、Channel/Gateway 与 Scheduled Tasks 的唯一实现状态源。README、实现规格、架构说明和代码计划只引用本文件，不维护第二套状态。

## 统一四态

实现项与外部 Gate 都只能使用下列四态；同一能力可以同时是“实现状态 `Local Verified`、外部 Gate `Environment Blocked`”。

| 状态                  | 含义                                                                                |
| --------------------- | ----------------------------------------------------------------------------------- |
| `Implemented`         | 已形成可验收代码，但当前要求的本地矩阵尚未全部运行或通过                            |
| `Local Verified`      | 代码、协议/产品入口及当前机器可执行的确定性本地测试已通过                           |
| `Environment Blocked` | 需要真实账号、组织、凭据、特定 Windows 身份或浏览器环境；当前未运行，不计为测试失败 |
| `Not Planned`         | 当前范围明确不建设或不开放；不作为缺失项或发布阻塞项                                |

`Local Verified` 不等于 alpha、RC 或 GA 已发布。fixture、mock、loopback transport 和确定性 Host 只证明本地实现，不构成真实 OpenAI、Forge、外部企业组织或 Windows 发布证据。发布说明必须逐项披露 `Environment Blocked` 及缺少的环境，但不得把它记为测试失败。Hachimi 不增加登录或用户租户体系；企业字段中的 tenant/corp 仅指外部平台组织身份。

## 固定范围与参考基线

- Codex 固定提交 `4c43465133428898aa84f0bfc02c306ed65fb66a` 是统一 Agent、编程、办公、Browser/Computer、Skills、MCP、Plugins/Connectors、Session/Thread 和 Scheduled Tasks 的主基线。
- Codex manual 2026-08-05 快照 SHA-256 固定为 `3528f93bacfae29be08f757d0f24be468b52f058fff698d78d73495cc660b147`，只用于产品行为与权限模型基线。
- OpenClaw 固定提交 `f6d456235cf011004f7cffc71a95acf6fbf1fa0a` 只用于本地 Gateway/Channel、确定性路由、Cron/Event、Task ledger、投递与重启 reconciliation。
- Claude Code Best 固定提交 `34b3dc99bf40c57c0b78f3b5b1d70471ebc2d06d` 只作为公开 Compaction 行为的 clean-room 验收参考。
- OpenAI、Forge 与企业 API 快照分别登记在 `docs/references/openai/registry.json`、`docs/references/forge/registry.json` 和 `docs/references/enterprise/registry.json`。
- 钉钉 Stream wire 来源固定为 SDK Go `v0.9.1` / `d1cc841e6013c3f6513a5bb01dfe3219b9c37d17` [ref:DINGTALK-STREAM-SDK-GO-20260731]；飞书长连接 wire 来源固定为 Go SDK `v3.9.9` / `ff207b774541a195f0a98c5bfda1507905e45431` [ref:FEISHU-SDK-GO-20260731]。两者仅作为协议来源，不作为运行时依赖。

Memory、Agent Heartbeat、Plugin Marketplace/远程 Catalog/第三方安装管理均为 `Not Planned`。不实现在线 Microsoft 365/Google Workspace、Remote Workspace 或远程多租户 Control Plane。本地 `hachimi-control-plane` 只是单机内部编排层，Agent、AppServer 与 Gateway 保持本机单用户运行。

## 版本与交付状态

| 版本                       | 计划内容            | 实现状态         | 外部 Gate             | 发布状态与结论                                                                                                                                                                                 |
| -------------------------- | ------------------- | ---------------- | --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `v0.2.1`                   | 原 R0 封板          | `Not Planned`    | `Not Planned`         | 已取消，不再发布；升级验证基线继续使用 `v0.2.0`                                                                                                                                                |
| `v0.3.0-alpha.1`–`alpha.7` | 原 P1–P7 分批 alpha | `Local Verified` | `Environment Blocked` | 未发布，由 alpha.8 合并取代；不补造历史 tag                                                                                                                                                    |
| `v0.3.0-alpha.8`           | P1–P8 合并预发布    | `Local Verified` | `Environment Blocked` | 源码版本、许可、候选构建/校验与独立 alpha prerelease workflow 已接入；候选状态由对应 commit 的 immutable manifest/Windows run artifact 记录；未创建 tag 或发布，alpha 不携带真实 Gate 完成声明 |
| `v0.3.0-rc.1`              | 外部集成收口        | `Implemented`    | `Environment Blocked` | 未发布；真实 Provider、Forge、三个外部企业组织和两类 Windows artifact 均未执行                                                                                                                 |
| `v0.3.0`                   | GA                  | `Implemented`    | `Environment Blocked` | 未发布；最终 commit/hash 必须重跑全部 Gate                                                                                                                                                     |

## P1–P8 实现状态

| 阶段 | 能力                                   | 实现状态         | 外部 Gate             | 已落地与边界                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| ---- | -------------------------------------- | ---------------- | --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| P1   | 崩溃前活动 Run 安全续跑                | `Local Verified` | `Environment Blocked` | `Recovering`/`WaitingRecoveryDecision`、六阶段 durable checkpoint、真实 side-effect ID、revision snapshot、同 Run generation 递增、旧 lease/token/Approval/临时 Grant 失效、只读/可靠幂等回执恢复、未知 mutation `indeterminate` 与 Resume UI；逐阶段文件数据库 crash injection 已覆盖                                                                                                                                                                                                                                                              |
| P2   | OpenAI 标准 Provider                   | `Local Verified` | `Environment Blocked` | capability registry、严格 wire 校验、`/v1/chat/completions`、`/v1/responses`、`/v1/embeddings`、Responses tool/usage/cancel/error、兼容档案和 Keyring `secret_ref` [ref:OAI-PRODUCT-CHATCOMPLETIONS-20260730] [ref:OAI-PRODUCT-RESPONSES-20260730] [ref:OAI-PRODUCT-EMBEDDINGS-20260730]                                                                                                                                                                                                                                                            |
| P3   | Remote Compaction / reasoning summary  | `Local Verified` | `Environment Blocked` | Responses-only capability、配置/probe/响应三重校验、越界/超时/drift 本地回退、只接收 Provider 明确公开的有界 summary，不持久化隐藏 reasoning                                                                                                                                                                                                                                                                                                                                                                                                        |
| P4   | Multi-Agent 与重启恢复                 | `Local Verified` | `Environment Blocked` | 父子 Task/Run/Session lineage、spawn/send/wait/cancel/collect、权限/allowlist/预算单调收窄、深度/并发限制、execution generation/lease、启动 reconciliation、后台 `NeedsAttention`、Usage/Artifact 回收与 UI；工具可见性不再按来源、EntryProfile 或 workload 裁剪；child Run 使用 transient authority，不覆盖来源 owner policy                                                                                                                                                                                                                       |
| P5   | 标准 Git push                          | `Local Verified` | `Environment Blocked` | 任意标准 Remote URL、通用 Git Workspace Host、GCM/SSH Agent、remote hash/ref/OID fencing、side-effect ledger 与未知结果 reconciliation；Project 与普通 Workspace 只要实际目录是 Git 仓库且权限、Host readiness 满足即可使用 Git/Forge；关闭远程 mutation 后仍保留本地 stage/commit                                                                                                                                                                                                                                                                  |
| P5   | Forge PR/MR                            | `Local Verified` | `Environment Blocked` | GitHub、GitLab、Gitee、Gitea/Forgejo create/query/update/close/merge、Credential Manager、expected revision/OID、幂等与合并独立高风险审批 [ref:GITHUB-API-20260730] [ref:GITLAB-API-20260730] [ref:GITEE-API-20260730] [ref:GITEA-FORGEJO-API-20260730]；Agent 原生 `forge.change.query`/`forge.change.mutate` 与 Workbench UI 复用同一 Host/transport；未知响应返回 executor error，使统一 ledger 保持 `Indeterminate`，不包装为可重放普通失败；supplied Approval 会重新核对 generation/tool/参数/主体/一次性 scope/有效期；换源分支必须新建 PR/MR |
| P6   | Plugin contribution 生命周期           | `Local Verified` | `Environment Blocked` | 内置 Bundle 的 Skill、Hook、EventSource、MCP、Connector、BrowserExtension、ScheduledTaskTemplate、Asset、CustomUI、Channel 共用完整 lifecycle；Marketplace、远程 Catalog 和第三方安装管理为 `Not Planned`                                                                                                                                                                                                                                                                                                                                           |
| P7   | 企业 Connector / Channel / EventSource | `Local Verified` | `Environment Blocked` | 企业微信 loopback listener 支持官方 GET `echostr` 和 POST 加密 XML，再由 Provider 做 AES/签名/组织/重放校验；钉钉 Stream、飞书 WebSocket/protobuf 的 heartbeat/reconnect/ACK/dedup、REST Connector、Gateway ledger 和 Scheduler typed ingress 已接入 [ref:WECOM-API-20260730] [ref:DINGTALK-STREAM-SDK-GO-20260731] [ref:FEISHU-SDK-GO-20260731]                                                                                                                                                                                                    |
| P7   | mention 与附件                         | `Local Verified` | `Environment Blocked` | 结构化 User/Bot/All mention；显式 `enterprise.download_attachment` 经统一权限与 Tool Orchestrator 暴露；25 MiB、MIME/magic/扩展名校验、分块临时下载、原子 Artifact 移入、Run/generation/account/event/remote ID/metadata hash/幂等 fencing；后台来源必须精确固定 account、`download_attachment` action 和 contribution revision，缺失或漂移进入 `NeedsAttention`；通用 `connector_invoke` 不能借保留动作绕过附件链；SQLite 不存附件正文                                                                                                             |
| P8   | Unified Host 与双 Browser 原语         | `Local Verified` | `Environment Blocked` | Workbench Session、Browser/Computer Inspector、Observe-first/接管/恢复、Browser history/input/wait/tab/transfer/storage 与静态 CDP allowlist、Computer 鼠标/键盘/窗口/受控启动；Session/lease/observation/frame/action 全部绑定持久 `run_generation`，旧 generation 稳定返回 `stale_run_generation`                                                                                                                                                                                                                                                 |

### 统一模型工具入口规则

| 决策维度              | 规则                                                                                                       |
| --------------------- | ---------------------------------------------------------------------------------------------------------- |
| 来源/Profile/workload | 不参与工具选择；Project、Manual、Channel、Scheduled、Pet 在同 Host 与同权限下获得相同 ToolPlan             |
| 授权升级              | `AuthorityMode` 独立控制逐次 Approval；后台权限 owner 只得到预配置 `Allow`，越权进入 `NeedsAttention`      |
| 用户输入              | `UserInputAvailability` 只表示交互 Host readiness；Pet 可提问但仍不可逐次提权，后台任务和子 Agent 不可提问 |
| Git/Forge             | 依赖通用 WorkspaceHandle、实际 Git 仓库、权限、凭据与 Host readiness，不依赖 `project_id` 或 Coding 分类   |
| Plan mode             | 始终收窄为只读；任何权限档位都不能恢复 mutation                                                            |

最终 ToolPlan 只与 Provider、Feature Flag、Host readiness、Plan mode 和统一权限决策求交集；执行器仍按结构化参数二次校验，不信任模型可见 schema。

## 不规划项与 Connector 当前边界

- Gateway process heartbeat、Channel transport/WebSocket heartbeat 与运行 lease heartbeat 已实现；它们分别表示进程存活、传输保活和执行所有权续租。
- Agent Heartbeat 为 `Not Planned`，已从 Roadmap、架构、规格和来源对标路径移除，不作为缺失项；`schedule.activity_marker` 仅标记 Scheduled Run 已启动。
- Connector 当前只保留账号、凭据引用、结构化 Action effect、权限、幂等、撤销、revision fencing 与 health。内置平台授权 UI 可用；Marketplace、远程 Catalog 和第三方 Connector/Plugin 安装管理为 `Not Planned`。

## 公共基础状态

| 项目                                            | 实现状态         | 外部 Gate             | 结论                                                                                                                                                                                                                                                                |
| ----------------------------------------------- | ---------------- | --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Control protocol                                | `Local Verified` | `Not Planned`         | `CONTROL_PROTOCOL_VERSION = 31`，TypeScript contracts 由 Rust 重新生成                                                                                                                                                                                              |
| migrations `0011`–`0018`                        | `Local Verified` | `Not Planned`         | `0018` 保留历史文件名，但 fresh schema 已直接使用 `computer_control_sessions` 与 `host_action_ledger`；当前阶段不兼容旧 DesktopControl 数据                                                                                                                         |
| `0019_recovery_alignment.sql`                   | `Local Verified` | `Not Planned`         | Run recovery 状态、revision snapshot 与 checkpoint 对齐                                                                                                                                                                                                             |
| `0020_agent_task_recovery.sql`                  | `Local Verified` | `Not Planned`         | AgentTask execution generation、lease 与 reconciliation metadata                                                                                                                                                                                                    |
| `0021_enterprise_content.sql`                   | `Local Verified` | `Not Planned`         | mention、附件下载与 Artifact 关联                                                                                                                                                                                                                                   |
| `0022_desktop_generation_fencing.sql`           | `Local Verified` | `Not Planned`         | Browser Session owner Run generation 与 Desktop action fencing                                                                                                                                                                                                      |
| migrations `0023`–`0028`                        | `Local Verified` | `Not Planned`         | Workbench、Project Tool context、环境摘要和持久 Embedded Browser Workspace/权限/设置                                                                                                                                                                                |
| `0029_unified_host_policies.sql`                | `Local Verified` | `Not Planned`         | Browser/Computer 统一 Host policy、可信应用身份与 action ledger                                                                                                                                                                                                     |
| `0030_enterprise_integration_orchestration.sql` | `Local Verified` | `Not Planned`         | Connector/Channel 收敛为统一企业集成账户和 lifecycle journal                                                                                                                                                                                                        |
| `0031_plugin_builtin_channel_binding.sql`       | `Local Verified` | `Not Planned`         | 官方内置企业 Channel 的 Plugin Runtime binding 约束                                                                                                                                                                                                                 |
| `0040_unified_agent_authority.sql`              | `Local Verified` | `Environment Blocked` | AgentWorkspace、统一 permission owner/policy、来源 owner 与 Run bundle 原子提交、不可变 RunAuthoritySnapshot、创建补偿与 managed orphan reconciliation 已完成；发布环境证据仍待完成                                                                                 |
| `0041_schema_epoch_v2.sql`                      | `Local Verified` | `Environment Blocked` | 破坏性 V2 初始化与 reset journal；保留用户 Project、Scheduled 显式目录及 Avatar/外观资产，发布环境重试证据尚待完成                                                                                                                                                  |
| Provider-neutral Compaction 闭环                | `Local Verified` | `Environment Blocked` | 完整 ConversationGroup、策略化 Microcompact、continuity reinjection、固定 overflow 状态机、最多三次重建、Provider overflow 单次重试、typed `PromptTooLong` 和 `/compact` 已接入；不新增 migration 或持久熔断                                                        |
| Scheduler/Gateway/Connector reconciliation      | `Local Verified` | `Environment Blocked` | Scheduler Run bundle 与启动投影、Gateway dispatch-without-receipt fencing、既有 Host side-effect ledger 和重启 reconciliation 已接入；不声明不存在的 Connector `dispatched` schema                                                                                  |
| App Server resume/rejoin matrix                 | `Local Verified` | `Environment Blocked` | Storage 1/200 分页、cursor、usage、active Run/delta、fork/stale generation、双客户端 watermark catch-up/unsubscribe，以及五类 Desktop 进程重启（approval、user input、read-only checkpoint、幂等 receipt、unknown side effect）均已通过；真实外部身份 Gate 仍未运行 |
| Office Artifact deterministic E2E               | `Local Verified` | `Not Planned`         | 低资源 OOXML/PDF 结构化语义、安全负例、Compaction 二进制隔离、Scheduled schema drift/中断/重启幂等与 Desktop `task-center` 已通过；不依赖 Office、GPU 或外部账号                                                                                                    |
| Browser/Computer opt-in stress                  | `Local Verified` | `Environment Blocked` | `test:desktop:stress` 已实现真实 managed Chromium 经 BrowserHost API 与专用 Win32 fixture 的顺序短时压力路径，单实例、最长 900 秒、2.5 GiB 熔断；本地短时组合可运行，完整 600 秒发布级 soak 按范围不执行；高完整性桌面/真实 Profile 仍阻塞                          |
| 五来源统一 Agent/Workspace/权限体系             | `Implemented`    | `Environment Blocked` | 五来源统一 launcher/executor/orchestrator；三档与七类资源执行校验、来源 owner 原子持久化、child transient policy、共享编辑器和 Pet MCP/Connector 配置已落地；Desktop E2E 仍有显式 skip，真实 Gate 未运行                                                            |
| migration 安全                                  | `Local Verified` | `Not Planned`         | 文件数据库共享 `<database>.migrate.lock`、30 秒 busy 失败、SQLite Online Backup、manifest/SHA-256、最近 3 份保留、失败事务回滚；内存库不备份                                                                                                                        |
| 七类 Runtime 功能停用开关                       | `Local Verified` | `Not Planned`         | 默认开启；UI 隐藏入口、相关工具不注册、命令返回 `feature_disabled` 和 feature key；Browser/Computer 使用各自独立 capability flag                                                                                                                                    |

七个 Runtime 开关为：`HACHIMI_DISABLE_RUN_RECOVERY`、`HACHIMI_DISABLE_PROVIDER_EXTENSIONS`、`HACHIMI_DISABLE_PROVIDER_REMOTE_CONTEXT`、`HACHIMI_DISABLE_MULTI_AGENT`、`HACHIMI_DISABLE_GIT_REMOTE_MUTATIONS`、`HACHIMI_DISABLE_PLUGIN_RUNTIME`、`HACHIMI_DISABLE_ENTERPRISE_INTEGRATIONS`。Browser、Computer Observe 与 Computer Act 不再受单一桌面模式开关捆绑。

普通 SQLite 不保存 API/Forge/企业凭据正文、原始或隐藏 reasoning、附件正文、截图、Cookie、临时 Grant、Approval/Host token 或进程输出正文。密钥存 Credential Manager，Git 使用 GCM/SSH Agent。

## 本地验收与后置真实 Gate

本地统一入口为：

```powershell
corepack pnpm check
```

本地矩阵覆盖迁移 18→41、V2 reset journal、备份/锁/回滚、Run 六阶段 crash injection、Provider conformance、Multi-Agent reconciliation、五来源 Workspace/权限、Git/Forge ledger、十类内置 Bundle lifecycle、企业 fixture transport、mention/附件 Artifact fencing、Desktop stale generation/observation/frame 与功能开关。`resume-rejoin.e2e.mjs` 已通过五类进程重启恢复；`task-center.e2e.mjs` 覆盖 Office Artifact 完整生命周期、负例、运行中 stdio MCP 中断恢复和重启幂等；`test:desktop:stress` 已切换为 BrowserHost API、managed Chromium 和专用 Win32 fixture 的低资源短时路径。完整 600 秒发布级 soak 按范围不执行，真实外部/Windows 身份 Gate 保持 `Environment Blocked`，不计为测试失败。Rust、TypeScript 与 TSX 单文件保持不超过 2000 行；前端复用共享组件和样式系统。

| 后置 Gate                   | 实现状态         | Gate 状态             | 缺失环境与本轮处理                                                                                                  |
| --------------------------- | ---------------- | --------------------- | ------------------------------------------------------------------------------------------------------------------- |
| 真实 OpenAI staging         | `Local Verified` | `Environment Blocked` | 缺 OpenAI staging 凭据；Harness 已组合真实 conformance 与确定性 drift/timeout/fallback，未执行真实请求              |
| Forge staging               | `Local Verified` | `Environment Blocked` | 缺 GitHub/GitLab/Gitee/Gitea/Forgejo 测试账号、仓库和凭据，未执行                                                   |
| 三个外部企业组织            | `Local Verified` | `Environment Blocked` | 缺企业微信、钉钉、飞书真实组织、应用凭据、HTTPS callback/Stream 环境；真实 mention/附件/限流/撤销未执行             |
| Chrome/CEF 与真实 Profile   | `Implemented`    | `Environment Blocked` | 缺可隔离的真实用户 Chrome Profile、扩展配对及 CEF/managed Chromium 长时环境；确定性 broker 不能替代真实浏览器       |
| standard-user Windows       | `Local Verified` | `Environment Blocked` | 缺不属于 `BUILTIN\\Administrators` 的交互 Runner 和不可变候选环境                                                   |
| elevated Windows            | `Local Verified` | `Environment Blocked` | 缺真正提升的交互 Runner、高完整性桌面和 security desktop 环境                                                       |
| 五渠道与企业组织 Gate       | `Implemented`    | `Environment Blocked` | 缺五渠道真实账号/组织、凭据、回调域名与组织管理员授权；当前只运行已登记的确定性 adapter/fixture                     |
| 许可/版本/候选/证据 harness | `Local Verified` | `Local Verified`      | Apache-2.0/NOTICE、三包哈希、安装/解包后第三方许可与默认 VRM 校验、来源哈希、脱敏、时效与 mismatch/skip fail-closed |
| alpha prerelease            | `Implemented`    | `Not Planned`         | workflow 已实现；仅接受成功候选构建并强制披露全部 `Environment Blocked`，当前不创建 tag                             |
| RC/GA tag / GitHub Release  | `Implemented`    | `Not Planned`         | workflow 已实现；真实 Gate 未齐、commit/hash 漂移或存在 skip 时禁止执行                                             |

## 固定 Not Planned 项

| 项目                                    | 实现状态      | 结论                                                                        |
| --------------------------------------- | ------------- | --------------------------------------------------------------------------- |
| Memory                                  | `Not Planned` | 当前不采用 Codex Memory 方案，不创建 Store、检索、migration 或派生实现      |
| Agent Heartbeat                         | `Not Planned` | 不建设周期唤醒、`HEARTBEAT_OK`、quiet hours 或主动通知去重                  |
| Marketplace/远程 Catalog                | `Not Planned` | 只保留内置 Bundle lifecycle；不建设远程发现、排序或分发                     |
| Plugin 用户管理与第三方安装             | `Not Planned` | 所有版本入口显示“暂不开放 / Not available”，不承诺后续迭代                  |
| 在线 Office 服务                        | `Not Planned` | 不接 Microsoft 365/Google Workspace；保留本地 DOCX/XLSX/PPTX/PDF 与文件整理 |
| Remote Workspace                        | `Not Planned` | 不建设远程 Workspace 产品                                                   |
| 远程多租户 Control Plane                | `Not Planned` | 本地 crate 仅为单机内部编排                                                 |
| 其他企业 Channel                        | `Not Planned` | 当前只支持已登记渠道，不扩展新的 Channel 产品面                             |
| 私有 Codex Provider/Realtime/多媒体协议 | `Not Planned` | Provider 只承诺三类公开 OpenAI 标准协议和显式登记兼容档案                   |

## 完成与发布定义

能力只有在代码、产品入口、单元测试、确定性本地测试和来源校验一致后，才可标为 `Local Verified`。真实外部环境缺失不否定本地代码完成度，但必须保持 `Environment Blocked`，记录所缺账号、组织、凭据或系统环境，不得以 fixture 冒充真实连接，也不得将其计为测试失败。

`v0.3.0` 仍需真实 OpenAI/Forge/企业 staging、standard-user/elevated Windows、干净提交、最终候选哈希、来源与许可证据和文档状态全部一致后才能发布。统一接口与配置见 `docs/RELEASE_GATES.md`；当前不得创建 tag 或声称 alpha.8/RC/GA 已发布。
