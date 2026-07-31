# Hachimi 0.3.0 发布 Gate

本文说明 `v0.3.0-alpha.8 → v0.3.0-rc.1 → v0.3.0` 的可执行 Gate、受保护配置和证据边界。当前只完成了 Gate/发布代码与本地验证；真实外部环境和两类 Windows Runner 尚未执行，不能据此创建 tag 或声明发布完成。

## 身份与“租户”边界

Hachimi 保持本机单用户产品，不增加 Hachimi 账号、登录、用户租户、云端控制面或远程多租户体系。文档和配置中的 `tenantId`、`corpId`、组织或“企业三环境”只表示企业微信、钉钉、飞书各自返回的外部组织标识，用于凭据绑定、事件验签和跨组织隔离。

OpenAI、Forge 与企业平台凭据只存 Windows Credential Manager。Staging JSON 只允许 endpoint、模型、仓库/组织、测试 Peer/Group、文件路径和 `secretRef`；任何 `apiKey`、`token`、`password`、带用户名密码的 URL 都会以稳定错误码拒绝。

## 命令与证据

| 命令                                                                       | 用途                                                                          | 当前状态                         |
| -------------------------------------------------------------------------- | ----------------------------------------------------------------------------- | -------------------------------- |
| `corepack pnpm release:version-check`                                      | 校验 Cargo/package/Tauri 版本、Apache-2.0 元数据、NOTICE 边界及安装包资源声明 | 本地可执行                       |
| `corepack pnpm release:artifact-manifest -- target/release-candidate`      | 对 MSI、NSIS、便携 ZIP、来源 registry、LICENSE/NOTICE 生成 SHA-256            | 本地可执行                       |
| `corepack pnpm release:artifact-verify -- --root target/release-candidate` | 下载后重新哈希候选，拒绝 manifest、commit、version、来源或许可漂移            | 本地可执行                       |
| `corepack pnpm test:staging:openai`                                        | 真实 OpenAI 与同次确定性故障 conformance                                      | 环境阻塞                         |
| `corepack pnpm test:staging:forge`                                         | managed Git Host 与五个 Forge 环境                                            | 环境阻塞                         |
| `corepack pnpm test:staging:enterprise`                                    | 三个外部企业组织的 REST/Stream/长连接验证                                     | 环境阻塞                         |
| `corepack pnpm release:evidence:verify`                                    | 聚合五类原始 `summary.json`，fail closed                                      | 本地聚合逻辑已验证；真实证据缺失 |

证据位于 `target/release-evidence/<run-id>/`。只保存 schema、Gate 状态、版本、commit、候选/来源哈希、脱敏环境指纹、稳定检查 ID、详情哈希、时间和稳定失败码；不保存 Secret、原始消息、隐藏 reasoning、附件正文、完整远端响应或本机敏感路径。

## OpenAI 配置

环境变量 `HACHIMI_STAGING_OPENAI_CONFIG` 指向 JSON：

```json
{
  "schemaVersion": 1,
  "gateKind": "openai",
  "environmentFingerprint": "protected-openai-staging",
  "secretRefs": ["credential-manager:provider:default"],
  "baseUrl": "https://api.openai.com/v1",
  "chatModel": "<protected-chat-model>",
  "responsesModel": "<protected-responses-model>",
  "embeddingModel": "<protected-embedding-model>",
  "requireReasoningSummary": true,
  "requireRemoteCompaction": true,
  "overflowProbeChars": 1200000
}
```

真实测试通过产品 `OpenAiCompatibleRuntime` 和 Provider registry 覆盖 Chat/Responses/Embeddings、stream、Tool、usage、取消、Provider 错误、overflow、远程压缩与公开 summary。Capability drift、畸形响应、超时、本地 fallback 和隐藏 reasoning 过滤由同一 Gate 先运行确定性 conformance，再与真实运行写入同一份 summary。

## Forge 配置

环境变量 `HACHIMI_STAGING_FORGE_CONFIG` 指向包含五项 `repositories` 的 JSON。`platformLabel` 必须分别为 `github`、`gitlab`、`gitee`、`gitea`、`forgejo`；Gitea 与 Forgejo 都使用 `forgeKind: "gitea_forgejo"`，但必须是两个独立环境。

每项仓库配置必须包含：

```json
{
  "platformLabel": "github",
  "forgeKind": "github",
  "apiBaseUrl": "https://api.github.com/",
  "faultApiBaseUrl": "https://<protected-fault-proxy>/github/",
  "owner": "<test-owner>",
  "repository": "<test-repository>",
  "remoteUrlHash": "<sha256>",
  "secretRef": "release-github",
  "sourceRef": "hachimi-gate/<run-id>/change",
  "targetRef": "main",
  "expectedCommitOid": "<40-hex-oid>",
  "mergeSourceRef": "hachimi-gate/<run-id>/merge",
  "mergeCommitOid": "<40-hex-oid>",
  "checkoutPath": "<protected-disposable-checkout>",
  "remoteName": "origin"
}
```

Git fetch/push 通过 `WorkspaceHostClient` 与固定 managed Git 执行，凭据由 GCM/SSH Agent 提供。Forge token 使用 Credential Manager。create/query/update/close/merge 使用产品 adapter；确定性 ledger 测试与真实 mutation 属于同一 Gate。传入的 Forge Approval 会重新校验 Session、Run generation、Tool call、参数哈希、一次性 scope、解析主体和过期时间，旧 Approval 不能复用于 merge。

每项还必须提供 `faultApiBaseUrl`，指向受保护的透明故障代理。代理把 mutation 完整转发到原 Forge 后丢弃响应，但继续放行只读查询；adapter 随后按 source/target、可见字段、状态和 source commit OID 做远端 reconciliation。只有精确匹配才确认成功，Create/Close/Merge 的 staging 测试还会断言本次结果确由未知响应恢复，避免把普通成功响应误算为故障验证。

## 企业平台配置

环境变量 `HACHIMI_STAGING_ENTERPRISE_CONFIG` 指向三个 `connections`：

```json
{
  "schemaVersion": 1,
  "gateKind": "enterprise",
  "environmentFingerprint": "protected-enterprise-organizations",
  "secretRefs": [
    "keyring:connector:release-wecom",
    "keyring:connector:release-dingtalk",
    "keyring:connector:release-feishu"
  ],
  "connections": [
    {
      "platform": "wecom",
      "accountId": "release-wecom",
      "credentialRef": "keyring:connector:release-wecom",
      "departmentId": "<test-department>",
      "peerId": "<test-peer>",
      "groupId": "<test-group>",
      "expectInboundEvent": true,
      "callbackPublicUrl": "https://<protected-reverse-proxy>/v1/channels/wecom/callback?account_id=release-wecom"
    }
  ]
}
```

钉钉、飞书对应 `platform` 为 `ding_talk`、`feishu`，三个 connection 的 `expectInboundEvent` 都必须为 `true`。企业微信 HTTPS reverse proxy 只能转发到 `127.0.0.1:42371/v1/channels/wecom/callback`；query 必须保留 `msg_signature`、`timestamp`、`nonce` 并增加受控的 `account_id`。Gate 会启动同一便携候选的 `--gateway`，只有真实 callback 在其 ledger 中形成带结构化 mention 和附件 metadata 的 receipt 才通过。Gateway 支持官方 GET `echostr` 验证和 POST 加密 XML，解密、组织标识、重放窗口与事件内容仍由企业 Provider 验证。

钉钉/飞书受保护入站 fixture 同样必须带结构化 mention 和允许类型附件；真实下载使用产品 `EnterpriseApiClient`、25 MiB 上限和远端 ID。MIME/magic、拒绝 HTML/可执行文件、Artifact fencing、重复下载和未知结果仍由同一 Gate 的确定性 `PluginHost` 测试验证，fixture 不替代真实传输。

这里的三个 connection 是三个外部平台组织，不是 Hachimi 租户。真实入站 callback、mention、附件、限流、凭据撤销和重启 reconciliation 在受保护环境不存在时保持“真实环境待验证”。

## Windows 与发布

`windows-release-gate.yml` 只构建一次 MSI、NSIS 和便携 ZIP。alpha 的 NSIS 和源码版本保持 `0.3.0-alpha.N`；由于 Wix/MSI 只接受数值 prerelease，MSI 打包阶段使用确定性 `0.3.0-N` overlay，artifact manifest 仍绑定完整源码版本、commit 和三类包哈希。手动运行默认 `candidate_only: true`，只生成不可变候选并跳过已后置的 standard-user/elevated 身份 Gate；显式关闭该输入或正式 push/tag 流程才会进入两类身份 Gate。standard-user 与 elevated Runner 下载同一候选并重新哈希；前者必须是真实非 Administrators、未提升的交互账户并执行 `v0.2.0 → 候选`，后者必须是真正提升的交互管理员环境。NSIS 安装目录、MSI administrative image 和便携 ZIP 都会按源文件 SHA-256 验证 Apache LICENSE、根 NOTICE、默认 VRM/动作许可、语音第三方 NOTICE、模型许可及默认 VRM 本体，不能只依赖 Tauri 配置声明。

确定性 Desktop E2E 在 `target/desktop-e2e-tools/` 使用固定 `tauri-driver 2.0.6` 和与 Runner 已安装 Edge 精确匹配的 Microsoft Edge WebDriver。准备脚本校验 driver 版本、Microsoft Authenticode 签名和本地 SHA-256 manifest；缓存被安全清理后可重新获取，不要求 Runner 长期保留多套 driver。managed Chromium 已展开目录若仍通过逐文件 manifest 校验，也不再要求同时保留额外下载 ZIP。

`publish-alpha-prerelease.yml` 只接受 `Windows Release Gate` 中已经成功的 `build-candidate` 作业产物，重新校验候选 commit/version/hash/source/license 后创建 alpha prerelease。alpha 发布说明固定声明不携带真实 OpenAI、Forge、企业组织或两类 Windows 身份 Gate 的通过结论，不能作为 RC/GA 证据；已有 tag 永不覆盖，失败后必须递增 alpha 序号。

`publish-release.yml` 仅处理 RC/GA，需要成功的 Windows run ID、外部 staging run ID、全新 tag 和 channel。它下载五类原始 summary 与候选，验证同一 commit/version/artifact/source/license 和证据时效后才调用 `gh release create`。已有 tag 永不覆盖；Gate 失败需递增 RC 序号。发布页固定说明 Windows 二进制未签名，以及默认 VRM 使官方包只能非商业发行。
