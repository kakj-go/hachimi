use serde::Serialize;
use specta::Type;

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(super) struct CommandError {
    pub(super) code: String,
    pub(super) message: String,
}

impl CommandError {
    pub(super) fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub(super) fn operation(code: &'static str, error: impl std::fmt::Display) -> Self {
        let detail = error.to_string();
        tracing::error!(code, detail = %detail, "Desktop operation failed");
        Self::new(code, public_operation_message(code, &detail))
    }
}

impl From<tauri::Error> for CommandError {
    fn from(error: tauri::Error) -> Self {
        Self::operation("tauri_error", error)
    }
}

fn public_operation_message(code: &str, detail: &str) -> String {
    let summary = if code == "llm_test_failed" {
        "大语言模型连接测试失败。"
    } else if code == "invalid_llm_settings" {
        "大语言模型配置无效，无法保存或测试。"
    } else if code.contains("credential") || code.contains("secret") || code.contains("keyring") {
        "系统安全凭据不可用，请检查系统凭据服务后重试。"
    } else if code.contains("timeout") {
        "操作等待超时，请检查本地服务状态后重试。"
    } else if code.contains("load") || code.contains("read") || code.contains("query") {
        "无法读取所需的本地状态，请重试；若问题持续，请查看应用日志。"
    } else if code.contains("store")
        || code.contains("save")
        || code.contains("write")
        || code.contains("update")
        || code.contains("delete")
    {
        "无法保存本地状态，请重试；若问题持续，请查看应用日志。"
    } else if code.contains("start")
        || code.contains("spawn")
        || code.contains("runtime")
        || code.contains("launch")
    {
        "所需的本地运行服务无法启动，请重试或查看应用日志。"
    } else {
        "操作未完成，请重试；若问题持续，请查看应用日志。"
    };
    match public_reason(detail) {
        Some(reason) => format!("{summary} 原因：{reason}（错误代码：{code}）"),
        None => format!(
            "{summary} 原因：发生未分类的内部错误，请在诊断页打开日志进一步定位。（错误代码：{code}）"
        ),
    }
}

fn public_reason(detail: &str) -> Option<&'static str> {
    let detail = detail.to_ascii_lowercase();
    if detail.contains("http 401")
        || detail.contains("401 unauthorized")
        || detail.contains("invalid token")
        || detail.contains("invalid api key")
        || detail.contains("incorrect api key")
    {
        Some("服务拒绝了身份验证，请检查 API 密钥是否正确、有效，并确认密钥属于当前服务。")
    } else if detail.contains("http 403") || detail.contains("403 forbidden") {
        Some("服务拒绝访问，请检查 API 密钥权限和账户访问策略。")
    } else if detail.contains("http 404") || detail.contains("404 not found") {
        Some("请求的接口或模型不存在，请检查接口地址、Provider 协议和模型名称。")
    } else if detail.contains("http 429")
        || detail.contains("429 too many requests")
        || detail.contains("rate limit")
        || detail.contains("quota")
    {
        Some("服务触发了速率限制或账户额度不足，请稍后重试并检查服务账户额度。")
    } else if detail.contains("http 400") || detail.contains("400 bad request") {
        Some("服务拒绝了请求配置，请检查接口地址、Provider 协议和模型名称。")
    } else if detail.contains("http 500")
        || detail.contains("http 502")
        || detail.contains("http 503")
        || detail.contains("http 504")
    {
        Some("模型服务当前异常或暂时不可用，请稍后重试。")
    } else if detail.contains("timed out")
        || detail.contains("timeout")
        || detail.contains("deadline has elapsed")
    {
        Some("请求等待超时，请检查网络连接和服务状态。")
    } else if detail.contains("connection refused") || detail.contains("actively refused") {
        Some("目标服务拒绝连接，请检查接口地址、端口和服务是否已启动。")
    } else if detail.contains("dns")
        || detail.contains("failed to lookup address")
        || detail.contains("name or service not known")
    {
        Some("无法解析服务域名，请检查接口地址和 DNS 网络设置。")
    } else if detail.contains("certificate") || detail.contains("tls") {
        Some("无法验证服务的 TLS 证书，请检查 HTTPS 地址和系统时间。")
    } else if detail.contains("database is locked") || detail.contains("database locked") {
        Some("本地数据库正被占用，请稍后重试；若持续出现，请重启应用。")
    } else if detail.contains("permission denied") || detail.contains("access is denied") {
        Some("系统拒绝了文件或目录访问，请检查当前用户权限。")
    } else if detail.contains("no space left") || detail.contains("disk full") {
        Some("磁盘可用空间不足，请释放空间后重试。")
    } else if detail.contains("api key") && detail.contains("missing") {
        Some("尚未配置 API 密钥。")
    } else if detail.contains("invalid url") || detail.contains("relative url without a base") {
        Some("接口地址格式无效，请填写完整的 HTTP 或 HTTPS 地址。")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_hides_internal_error_details_but_exposes_a_stable_code() {
        let error = CommandError::operation(
            "integration_account_store_failed",
            "unexpected sqlite failure at C:\\private\\agent.sqlite3 with sk-secret",
        );
        assert_eq!(error.code, "integration_account_store_failed");
        assert!(!error.message.contains("database"));
        assert!(!error.message.contains("private"));
        assert!(!error.message.contains("secret"));
        assert!(error.message.contains("无法保存"));
        assert!(error.message.contains("integration_account_store_failed"));
    }

    #[test]
    fn operation_explains_safe_actionable_provider_failures() {
        let error = CommandError::operation(
            "llm_test_failed",
            "HTTP 401 Unauthorized: Invalid token (request id: private)",
        );
        assert!(error.message.contains("身份验证"));
        assert!(error.message.contains("API 密钥"));
        assert!(error.message.contains("llm_test_failed"));
        assert!(!error.message.contains("request id"));
        assert!(!error.message.contains("private"));
    }

    #[test]
    fn operation_explains_local_contention_without_disclosing_paths() {
        let error = CommandError::operation(
            "settings_save_failed",
            "database is locked at C:\\private\\agent.sqlite3",
        );
        assert!(error.message.contains("本地数据库正被占用"));
        assert!(!error.message.contains("private"));
    }
}
