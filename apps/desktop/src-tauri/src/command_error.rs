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
        tracing::error!(code, detail = %error, "Desktop operation failed");
        Self::new(code, public_message(code))
    }
}

impl From<tauri::Error> for CommandError {
    fn from(error: tauri::Error) -> Self {
        Self::operation("tauri_error", error)
    }
}

fn public_message(code: &str) -> &'static str {
    if code.contains("credential") || code.contains("secret") || code.contains("keyring") {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_hides_internal_error_details() {
        let error = CommandError::operation(
            "integration_account_store_failed",
            "database locked at C:\\private\\agent.sqlite3",
        );
        assert_eq!(error.code, "integration_account_store_failed");
        assert!(!error.message.contains("database"));
        assert!(!error.message.contains("private"));
        assert!(error.message.contains("无法保存"));
    }
}
