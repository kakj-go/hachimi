# Workbench Windows 验收清单

更新时间：2026-07-28

## 普通 Desktop E2E

- [x] 添加临时 Git Project，支持先添加项目后 `git init`；
- [x] Git snapshot 已验证 `NotRepository`、`Unborn(main · 尚无提交)`、`Ready` 和 `Detached`；Detached 时 Managed Worktree 禁用，切回分支后立即恢复；
- [x] 创建空 root commit 后无需刷新即可选择 Managed Worktree；
- [x] 新任务使用草稿态，发送前不写 Session/Run，发送只创建一个 Checkout/Session/Run；
- [x] Plan mode 无写入/Exec，接受 ProposedPlan 后创建新的 Default Run；
- [x] Approval、secret UserInput、取消、断线补发和 generation fencing；
- [x] 真实 WebView2 验证文件树 lazy load、Watch 新文件投影和逐文件 Run Diff/hunk；Workbench adapter 与 Workspace Rust 测试验证 Watch invalidate/generation fencing、搜索取消、Checkout Diff 和大 Diff 分块；
- [x] 五个 Built-in Office Skill 已验证显式 activation 和真实容器 Artifact；模型通过 `skills.list`/`skills.read` 隐式激活 Office overlay，无效资源读取失败后可恢复且不扩大 Grant；
- [x] WebView reload 后从 `snapshot_sequence + active_event_replay` 恢复，Run 继续执行；
- [x] 应用关闭/重启后 Transcript、Diff、Evidence、TaskRun 可查看；旧 Run 转 interrupted/lost，不恢复 secret、Approval 或 Sandbox lease；
- [x] Task Center 创建/授权、Run Now、取消、General/Office Run 和重启恢复；
- [x] Task Center 高级 Cron/IANA timezone 序列化、Cancelled retry 和 NeedsAttention continuation 已有 Workbench 测试；
- [x] MCP schema 漂移 → `NeedsAttention` → fresh interactive continuation 已有真实 WebView2 E2E；
- [x] Scheduler Rust 测试验证 retry 终态约束、fresh TaskRun/invocation key，以及通知失败不改变成功执行状态；
- [x] 真实 `SystemClock` release soak 覆盖短期 At、anchored Every、6-field Cron 和 20+ occurrence；
- [x] Terminal Vitest/真实 WebView2 覆盖 base64 bytes、分段 UTF-8、非法字节、output cap、resize、stdin echo、reload reattach、kill 和孙进程终止；
- [x] Desktop runner 在 restart/session/failure/finally 精确回收 E2E 应用和 WebDriver 进程树；Workspace Worker/MCP stdio 使用 `CREATE_NO_WINDOW`。完整 4-spec 回归中应用实例最大为 1，结束后残留为 0；
- [ ] 真实墙钟自然触发 At/Every/Cron、系统通知展示和 UI retry 的长时间 WebView2 E2E 仍待发布环境执行。

## 管理员 Windows Runner

- [x] `pnpm test:windows:release` 与受保护 self-hosted workflow 已建立，测试不自动重试；
- [x] restricted process 显式 security-capabilities + handle-list，包含未列 inheritable sentinel handle smoke；
- [ ] MSI/NSIS setup helper、marker、ACL、身份/SID 和 policy 版本；
- [ ] restricted token canary、Job Object 子孙进程、Checkout/TEMP ACL；
- [ ] junction/reparse、symlink、UNC/设备路径、ADS、保留名、尾随点空格、8.3 路径、跨盘符矩阵；
- [ ] deny-read 和 deny-all-network；
- [ ] MCP stdio restricted Host 与网络隔离；
- [ ] Workspace Worker、Agent `workspace_exec` 与 Terminal/ConPTY 分别通过真实 restricted-process smoke；
- [ ] linked worktree 的 stage 与 commit 使用两个独立 ACL lease，分别覆盖 shared common-dir 和 per-worktree git-dir；代码与双 lease smoke 已完成，等待管理员执行；
- [ ] portable target repair/UAC cancel/重启 attestation；
- [ ] Windows Shell UI Automation 验证系统 Toast 只包含任务名和终态；
- [ ] 没有四项 enforced 时写入、Exec、stdio MCP 和后台副作用 fail closed。

## 固定命令

```text
pnpm check
pnpm test:windows:standard-user
pnpm test:windows:release
```

`pnpm check` 是普通本地/托管 CI 的完整聚合入口，包含 format、typecheck、lint/Clippy、workspace tests、contracts、provenance/architecture、P0、Storybook、视觉/可访问性、build 和 Desktop E2E。

## 真实环境测试归属

下列测试保留 `#[ignore]` 是为了防止普通 `cargo test --workspace` 修改系统状态或误用交互桌面；它们不是遗漏，而是由发布 Gate 使用 `--ignored` 精确执行：

| 环境测试                                                              | 所属 Gate                 | 自动入口                          |
| --------------------------------------------------------------------- | ------------------------- | --------------------------------- |
| managed Chromium 真实页面、上传和下载                                 | standard-user             | `pnpm test:windows:standard-user` |
| WGC/Notepad 真实截图和输入                                            | standard-user             | `pnpm test:windows:standard-user` |
| Gateway 当前用户启动项往返                                            | standard-user             | `pnpm test:windows:standard-user` |
| SystemClock At/Every/Cron soak                                        | standard-user、elevated   | 两个 Windows Gate 均显式执行      |
| Sandbox ACL/路径攻击、Workspace Worker、Agent Exec、MCP stdio、ConPTY | standard-user 或 elevated | 对应 Windows Gate 显式执行        |

长时间 WebView2 自然触发、Toast UI Automation 和 retry 仍是上述发布环境证据的一部分；未取得对应 `summary.json` 前保持未勾选，不降低已经完成的代码能力，也不得据此创建正式发布标记。

失败产物只保留脱敏截图、WebDriver 日志和运行状态；不得保存 Prompt、secret、完整 Tool 输出或用户路径。

当前 portable 已构建到 `target/portable/Hachimi-portable.zip`，并包含 Desktop、Workspace Worker、setup/launcher/canary/attest 与五个 Built-in Office Skill。当前非管理员会话运行真实 setup 时在受信 Git runtime ACL 阶段以 Windows error 5 fail closed；该结果不视为管理员验收通过。
