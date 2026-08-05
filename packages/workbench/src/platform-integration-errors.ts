import { commandFailure } from "@hachimi/contracts";

const INTEGRATION_FAILURE_MESSAGES: Record<string, [string, string]> = {
  integration_input_invalid: [
    "账户名称和标识不能为空，且不能超过允许长度。请检查后重试。",
    "Account name and ID are required and must fit the allowed length. Check them and try again.",
  ],
  integration_credentials_invalid: [
    "平台凭据缺失或格式不正确。请检查应用 ID、密钥及该平台要求的回调配置。",
    "Platform credentials are missing or invalid. Check the app ID, secret, and any callback settings required by the provider.",
  ],
  integration_capability_required: [
    "请至少启用消息收发或平台 API 访问中的一项。",
    "Enable at least messaging or platform API access.",
  ],
  integration_api_unsupported: [
    "该平台目前不支持企业 API 访问，请仅启用消息收发。",
    "This provider does not support enterprise API access. Enable messaging only.",
  ],
  integration_account_provider_conflict: [
    "该账户标识已被其他平台使用，请更换账户标识。",
    "This account ID is already used by another provider. Choose another account ID.",
  ],
  integration_account_not_found: [
    "该平台账户已不存在，请刷新列表后重试。",
    "This platform account no longer exists. Refresh the list and try again.",
  ],
  integration_config_invalid: [
    "保存的平台配置已损坏，请删除该账户后重新添加。",
    "The saved provider configuration is invalid. Remove the account and add it again.",
  ],
  integration_identity_conflict: [
    "该平台账号已连接到另一个账户。请更新现有账户的凭据，或先断开现有账户后重试。",
    "This platform account is already connected. Update its credentials or disconnect it before trying again.",
  ],
  integration_revision_conflict: [
    "该账户刚刚被其他操作修改，请关闭窗口并重新打开后再试。",
    "This account was changed by another operation. Close and reopen it before trying again.",
  ],
  integration_secret_store_failed: [
    "无法将凭据写入系统安全存储，请检查系统凭据服务后重试。",
    "The credentials could not be written to secure system storage. Check the credential service and try again.",
  ],
  integration_account_store_failed: [
    "无法保存平台账户，请重试；若问题持续，请查看应用日志。",
    "The platform account could not be saved. Retry, then check the application logs if the problem continues.",
  ],
  integration_runtime_configure_failed: [
    "账户已保存，但消息运行时启动失败。请检查 Gateway 状态后重试。",
    "The account was saved, but its messaging runtime could not start. Check Gateway status and try again.",
  ],
  integration_gateway_unavailable: [
    "本地消息服务未能在 10 秒内启动。Hachimi 会继续自动恢复，你可以稍后重新检测。",
    "The local messaging service did not start within 10 seconds. Hachimi will keep recovering; retry the check shortly.",
  ],
  integration_runtime_health_unavailable: [
    "暂时无法读取消息连接状态，请稍后刷新。",
    "Messaging connection status is temporarily unavailable. Refresh shortly.",
  ],
  ilink_qr_request_invalid: [
    "微信账户名称或标识不正确，请检查后重新生成二维码。",
    "The WeChat account name or ID is invalid. Check it and request a new QR code.",
  ],
  ilink_qr_session_not_found: [
    "二维码登录会话已失效，请重新生成二维码。",
    "The QR sign-in session is no longer available. Request a new QR code.",
  ],
  ilink_qr_session_invalid: [
    "二维码登录会话不完整，请重新生成二维码。",
    "The QR sign-in session is incomplete. Request a new QR code.",
  ],
};

export function integrationFailureMessage(error: unknown, zh: boolean): string {
  const failure = commandFailure(error);
  const localized = INTEGRATION_FAILURE_MESSAGES[failure.code];
  return localized?.[zh ? 0 : 1] ?? failure.message;
}
