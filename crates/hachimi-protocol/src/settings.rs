use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum StructuredOutputMode {
    #[default]
    Auto,
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCapabilityProbeSource {
    Probe,
    UserOverride,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilityProbe {
    pub strict_json_schema: bool,
    pub output_schema: bool,
    pub source: ProviderCapabilityProbeSource,
    pub stable_error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LlmSettings {
    pub base_url: String,
    pub model_name: String,
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    #[serde(default)]
    pub structured_output_mode: StructuredOutputMode,
}

impl Default for LlmSettings {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:11434/v1".into(),
            model_name: "gemma4:e4b".into(),
            max_input_tokens: 0,
            max_output_tokens: 0,
            structured_output_mode: StructuredOutputMode::Auto,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LlmSettingsView {
    pub base_url: String,
    pub model_name: String,
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    pub structured_output_mode: StructuredOutputMode,
    pub api_key_configured: bool,
}

impl LlmSettingsView {
    #[must_use]
    pub fn from_settings(settings: &LlmSettings, api_key_configured: bool) -> Self {
        Self {
            base_url: settings.base_url.clone(),
            model_name: settings.model_name.clone(),
            max_input_tokens: settings.max_input_tokens,
            max_output_tokens: settings.max_output_tokens,
            structured_output_mode: settings.structured_output_mode,
            api_key_configured,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LlmSettingsInput {
    pub base_url: String,
    pub model_name: String,
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    pub structured_output_mode: StructuredOutputMode,
    /// Missing or blank keeps the existing secret. Secrets are never returned to the WebView.
    pub api_key: Option<String>,
    pub clear_api_key: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LlmTestResult {
    pub success: bool,
    pub latency_ms: u32,
    pub response_preview: String,
    pub capability_probe: ProviderCapabilityProbe,
}
