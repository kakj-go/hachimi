//! OpenAI-compatible connectivity testing and OS-backed API-key storage.
//!
//! This crate deliberately does not register a Pet provider or an Agent runtime.

use std::time::{Duration, Instant};

use futures_util::StreamExt;
use hachimi_core::FeatureAvailability;
use hachimi_protocol::{LlmSettings, LlmSettingsInput, LlmTestResult};
use reqwest::StatusCode;
use serde_json::{Value, json};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use url::Url;

const API_KEY_SERVICE: &str = "com.hachimi.desktop";
const API_KEY_ACCOUNT: &str = "llm-api-key";
const RESPONSE_PREVIEW_LIMIT: usize = 512;

#[must_use]
pub const fn availability() -> FeatureAvailability {
    FeatureAvailability::Available
}

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("配置无效：{0}")]
    InvalidConfiguration(String),
    #[error("系统密钥存储不可用")]
    SecretStore,
    #[error("连接请求失败：{0}")]
    Request(String),
    #[error("服务返回 HTTP {0}{1}")]
    Http(StatusCode, String),
    #[error("服务返回了无效的 JSON")]
    InvalidResponse,
    #[error("请求已取消")]
    Cancelled,
}

pub trait ApiKeyStore: Send + Sync {
    fn get(&self) -> Result<Option<String>, LlmError>;
    fn set(&self, secret: &str) -> Result<(), LlmError>;
    fn clear(&self) -> Result<(), LlmError>;

    fn is_configured(&self) -> Result<bool, LlmError> {
        Ok(self.get()?.is_some_and(|value| !value.is_empty()))
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemApiKeyStore;

impl SystemApiKeyStore {
    fn entry() -> Result<keyring::Entry, LlmError> {
        keyring::Entry::new(API_KEY_SERVICE, API_KEY_ACCOUNT).map_err(|_| LlmError::SecretStore)
    }
}

impl ApiKeyStore for SystemApiKeyStore {
    fn get(&self) -> Result<Option<String>, LlmError> {
        match Self::entry()?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(LlmError::SecretStore),
        }
    }

    fn set(&self, secret: &str) -> Result<(), LlmError> {
        Self::entry()?
            .set_password(secret)
            .map_err(|_| LlmError::SecretStore)
    }

    fn clear(&self) -> Result<(), LlmError> {
        match Self::entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(LlmError::SecretStore),
        }
    }
}

pub fn validate_input(input: &LlmSettingsInput) -> Result<LlmSettings, LlmError> {
    let parsed = Url::parse(input.base_url.trim()).map_err(|_| {
        LlmError::InvalidConfiguration("接口地址必须是有效的 HTTP/HTTPS URL".into())
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(LlmError::InvalidConfiguration(
            "接口地址仅支持 HTTP 或 HTTPS".into(),
        ));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(LlmError::InvalidConfiguration(
            "接口地址不能包含查询参数或 fragment".into(),
        ));
    }
    let model_name = input.model_name.trim();
    if model_name.is_empty() || model_name.chars().count() > 128 {
        return Err(LlmError::InvalidConfiguration(
            "模型名称长度必须为 1–128 个字符".into(),
        ));
    }
    if input.max_input_tokens > 2_000_000 {
        return Err(LlmError::InvalidConfiguration(
            "最大输入 Token 必须在 0–2,000,000 之间".into(),
        ));
    }
    if input.max_output_tokens > 200_000 {
        return Err(LlmError::InvalidConfiguration(
            "最大输出 Token 必须在 0–200,000 之间".into(),
        ));
    }
    if input.clear_api_key
        && input
            .api_key
            .as_deref()
            .is_some_and(|secret| !secret.trim().is_empty())
    {
        return Err(LlmError::InvalidConfiguration(
            "不能同时设置并清除 API 密钥".into(),
        ));
    }

    Ok(LlmSettings {
        base_url: input.base_url.trim().trim_end_matches('/').into(),
        model_name: model_name.into(),
        max_input_tokens: input.max_input_tokens,
        max_output_tokens: input.max_output_tokens,
    })
}

pub fn apply_secret_change(
    store: &dyn ApiKeyStore,
    input: &LlmSettingsInput,
) -> Result<(), LlmError> {
    if input.clear_api_key {
        return store.clear();
    }
    if let Some(secret) = input.api_key.as_deref().filter(|value| !value.is_empty()) {
        store.set(secret)?;
    }
    Ok(())
}

pub async fn test_connection(
    settings: &LlmSettings,
    api_key: Option<&str>,
) -> Result<LlmTestResult, LlmError> {
    let endpoint = format!(
        "{}/chat/completions",
        settings.base_url.trim_end_matches('/')
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| LlmError::Request("无法创建 HTTP 客户端".into()))?;
    let body = json!({
        "model": settings.model_name,
        "messages": [{"role": "user", "content": "请仅回复 HACHIMI_OK"}],
        "temperature": 0,
        "stream": false,
        "max_tokens": 16
    });
    let mut request = client.post(endpoint).json(&body);
    if let Some(secret) = api_key.filter(|value| !value.is_empty()) {
        request = request.bearer_auth(secret);
    }

    let started = Instant::now();
    let response = request.send().await.map_err(|error| {
        let reason = if error.is_timeout() {
            "请求超时（30 秒）"
        } else if error.is_connect() {
            "无法连接到服务"
        } else {
            "网络请求失败"
        };
        LlmError::Request(reason.into())
    })?;
    let status = response.status();
    if !status.is_success() {
        let response_body = response.text().await.unwrap_or_default();
        let detail = provider_error_detail(&response_body)
            .map(|value| format!("：{value}"))
            .unwrap_or_default();
        return Err(LlmError::Http(status, detail));
    }
    let value: Value = response
        .json()
        .await
        .map_err(|_| LlmError::InvalidResponse)?;
    let content = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or(LlmError::InvalidResponse)?;
    Ok(LlmTestResult {
        success: true,
        latency_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
        response_preview: truncate_chars(content, RESPONSE_PREVIEW_LIMIT),
    })
}

pub async fn stream_pet_turn(
    settings: &LlmSettings,
    api_key: Option<&str>,
    input: &str,
    cancellation: &CancellationToken,
    mut on_delta: impl FnMut(&str),
) -> Result<String, LlmError> {
    let input = input.trim();
    if input.is_empty() || input.chars().count() > 8_000 {
        return Err(LlmError::InvalidConfiguration(
            "消息长度必须为 1–8,000 个字符".into(),
        ));
    }
    let endpoint = format!(
        "{}/chat/completions",
        settings.base_url.trim_end_matches('/')
    );
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|_| LlmError::Request("无法创建 HTTP 客户端".into()))?;
    let mut body = json!({
        "model": settings.model_name,
        "messages": [
            {
                "role": "system",
                "content": "你是 Hachimi，一只友好、简洁的桌面宠物。请使用用户的语言直接回答，不要声称可以操作文件、终端或桌面。"
            },
            {"role": "user", "content": input}
        ],
        "stream": true
    });
    if settings.max_output_tokens > 0 {
        body["max_tokens"] = Value::from(settings.max_output_tokens);
    }
    let mut request = client.post(endpoint).json(&body);
    if let Some(secret) = api_key.filter(|value| !value.is_empty()) {
        request = request.bearer_auth(secret);
    }
    let response = tokio::select! {
        () = cancellation.cancelled() => return Err(LlmError::Cancelled),
        response = request.send() => response.map_err(|error| request_error(&error))?,
    };
    let status = response.status();
    if !status.is_success() {
        let response_body = response.text().await.unwrap_or_default();
        let detail = provider_error_detail(&response_body)
            .map(|value| format!("：{value}"))
            .unwrap_or_default();
        return Err(LlmError::Http(status, detail));
    }
    let is_event_stream = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("text/event-stream"));
    if !is_event_stream {
        let value: Value = response
            .json()
            .await
            .map_err(|_| LlmError::InvalidResponse)?;
        let content = response_content(&value).ok_or(LlmError::InvalidResponse)?;
        on_delta(content);
        return Ok(content.to_owned());
    }

    let mut stream = response.bytes_stream();
    let mut pending = Vec::<u8>::new();
    let mut completed = String::new();
    loop {
        let next = tokio::select! {
            () = cancellation.cancelled() => return Err(LlmError::Cancelled),
            next = stream.next() => next,
        };
        let Some(chunk) = next else {
            break;
        };
        let chunk = chunk.map_err(|error| request_error(&error))?;
        pending.extend_from_slice(&chunk);
        while let Some(position) = pending.iter().position(|byte| *byte == b'\n') {
            let mut line = pending.drain(..=position).collect::<Vec<_>>();
            while matches!(line.last(), Some(b'\n' | b'\r')) {
                line.pop();
            }
            let Ok(line) = std::str::from_utf8(&line) else {
                return Err(LlmError::InvalidResponse);
            };
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data == "[DONE]" {
                return (!completed.is_empty())
                    .then_some(completed)
                    .ok_or(LlmError::InvalidResponse);
            }
            if data.is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(data).map_err(|_| LlmError::InvalidResponse)?;
            if let Some(delta) = value
                .pointer("/choices/0/delta/content")
                .and_then(Value::as_str)
                .or_else(|| response_content(&value))
            {
                completed.push_str(delta);
                on_delta(delta);
            }
        }
    }
    (!completed.is_empty())
        .then_some(completed)
        .ok_or(LlmError::InvalidResponse)
}

fn request_error(error: &reqwest::Error) -> LlmError {
    let reason = if error.is_timeout() {
        "请求超时"
    } else if error.is_connect() {
        "无法连接到服务"
    } else {
        "网络请求失败"
    };
    LlmError::Request(reason.into())
}

fn response_content(value: &Value) -> Option<&str> {
    value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
}

fn truncate_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn provider_error_detail(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    let code = value
        .pointer("/error/code")
        .or_else(|| value.get("code"))
        .and_then(Value::as_str)
        .and_then(|value| safe_provider_text(value, 64));
    let message = value
        .pointer("/error/message")
        .or_else(|| value.get("message"))
        .and_then(Value::as_str)
        .and_then(|value| safe_provider_text(value, 256));
    match (code, message) {
        (Some(code), Some(message)) => Some(format!("{code}: {message}")),
        (Some(code), None) => Some(code),
        (None, Some(message)) => Some(message),
        (None, None) => None,
    }
}

fn safe_provider_text(value: &str, limit: usize) -> Option<String> {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    Some(truncate_chars(&redact_api_keys(&collapsed), limit))
}

fn redact_api_keys(value: &str) -> String {
    let mut remaining = value;
    let mut redacted = String::with_capacity(value.len());
    while let Some(index) = remaining.find("sk-") {
        redacted.push_str(&remaining[..index]);
        redacted.push_str("[REDACTED]");
        let secret = &remaining[index + 3..];
        let secret_end = secret
            .char_indices()
            .find_map(|(index, character)| {
                (!character.is_ascii_alphanumeric() && !matches!(character, '-' | '_'))
                    .then_some(index)
            })
            .unwrap_or(secret.len());
        remaining = &secret[secret_end..];
    }
    redacted.push_str(remaining);
    redacted
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct MemoryStore(Mutex<Option<String>>);

    impl ApiKeyStore for MemoryStore {
        fn get(&self) -> Result<Option<String>, LlmError> {
            Ok(self.0.lock().expect("lock").clone())
        }
        fn set(&self, secret: &str) -> Result<(), LlmError> {
            *self.0.lock().expect("lock") = Some(secret.into());
            Ok(())
        }
        fn clear(&self) -> Result<(), LlmError> {
            *self.0.lock().expect("lock") = None;
            Ok(())
        }
    }

    fn input() -> LlmSettingsInput {
        LlmSettingsInput {
            base_url: "http://localhost:11434/v1/".into(),
            model_name: "gemma4:e4b".into(),
            max_input_tokens: 0,
            max_output_tokens: 0,
            api_key: None,
            clear_api_key: false,
        }
    }

    #[test]
    fn validates_and_normalizes_settings() {
        let settings = validate_input(&input()).expect("valid");
        assert_eq!(settings.base_url, "http://localhost:11434/v1");
        let mut invalid = input();
        invalid.base_url = "file:///secret".into();
        assert!(validate_input(&invalid).is_err());
        invalid.base_url = "https://example.com/v1?token=secret".into();
        assert!(validate_input(&invalid).is_err());
    }

    #[test]
    fn blank_secret_keeps_existing_and_clear_is_explicit() {
        let store = MemoryStore::default();
        store.set("secret").expect("seed");
        let mut value = input();
        value.api_key = Some(String::new());
        apply_secret_change(&store, &value).expect("keep");
        assert_eq!(store.get().expect("get").as_deref(), Some("secret"));
        value.clear_api_key = true;
        apply_secret_change(&store, &value).expect("clear");
        assert_eq!(store.get().expect("get"), None);
    }

    #[test]
    fn response_preview_is_unicode_safe() {
        assert_eq!(truncate_chars("哈奇米abcdef", 3), "哈奇米");
    }

    #[test]
    fn persisted_llm_settings_have_no_secret_field() {
        let json = serde_json::to_string(&LlmSettings::default()).expect("serialize");
        assert!(!json.to_lowercase().contains("api"));
        assert!(!json.contains("secret"));
    }

    #[test]
    fn extracts_safe_provider_errors_and_redacts_keys() {
        assert_eq!(
            provider_error_detail(
                r#"{"code":"INVALID_API_KEY","message":"Invalid API key sk-secret123"}"#
            )
            .as_deref(),
            Some("INVALID_API_KEY: Invalid API key [REDACTED]")
        );
        assert_eq!(provider_error_detail("<html>proxy error</html>"), None);
    }
}
