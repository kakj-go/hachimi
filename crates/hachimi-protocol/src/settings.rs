use serde::{Deserialize, Serialize};
use specta::Type;

use crate::{ProviderAccountId, ProviderEndpointId, ProviderProtocolKind};

fn default_provider_profile() -> String {
    "openai-strict".into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum StructuredOutputMode {
    #[default]
    Auto,
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningSummaryMode {
    #[default]
    Auto,
    Concise,
    Detailed,
    None,
}

impl ReasoningSummaryMode {
    #[must_use]
    pub const fn is_enabled(self) -> bool {
        !matches!(self, Self::None)
    }

    #[must_use]
    pub const fn as_provider_value(self) -> Option<&'static str> {
        match self {
            Self::Auto => Some("auto"),
            Self::Concise => Some("concise"),
            Self::Detailed => Some("detailed"),
            Self::None => None,
        }
    }
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
    #[serde(default)]
    pub protocol: ProviderProtocolKind,
    #[serde(default = "default_provider_profile")]
    pub compatibility_profile_id: String,
    #[serde(default)]
    pub provider_endpoint_id: Option<ProviderEndpointId>,
    #[serde(default)]
    pub provider_account_id: Option<ProviderAccountId>,
    #[serde(default)]
    pub embedding_model_name: String,
    #[serde(default)]
    pub reasoning_summary: ReasoningSummaryMode,
    #[serde(default)]
    pub remote_compaction: bool,
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    #[serde(default)]
    pub structured_output_mode: StructuredOutputMode,
}

impl Default for LlmSettings {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:11434/v1".into(),
            model_name: "gpt-5.6-sol".into(),
            protocol: ProviderProtocolKind::ChatCompletions,
            compatibility_profile_id: default_provider_profile(),
            provider_endpoint_id: None,
            provider_account_id: None,
            embedding_model_name: String::new(),
            reasoning_summary: ReasoningSummaryMode::Auto,
            remote_compaction: false,
            max_input_tokens: 1_050_000,
            max_output_tokens: 128_000,
            structured_output_mode: StructuredOutputMode::Auto,
        }
    }
}

impl LlmSettings {
    /// Updates only the previous built-in defaults, leaving custom provider settings intact.
    pub fn upgrade_legacy_defaults(&mut self) -> bool {
        if self.base_url != "http://localhost:11434/v1"
            || self.model_name != "gemma4:e4b"
            || self.protocol != ProviderProtocolKind::ChatCompletions
            || self.compatibility_profile_id != "openai-strict"
            || self.provider_endpoint_id.is_some()
            || self.provider_account_id.is_some()
            || !self.embedding_model_name.is_empty()
            || self.max_input_tokens != 0
            || self.max_output_tokens != 0
            || self.remote_compaction
            || self.structured_output_mode != StructuredOutputMode::Auto
        {
            return false;
        }
        let defaults = Self::default();
        self.model_name = defaults.model_name;
        self.max_input_tokens = defaults.max_input_tokens;
        self.max_output_tokens = defaults.max_output_tokens;
        true
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LlmSettingsView {
    pub base_url: String,
    pub model_name: String,
    pub protocol: ProviderProtocolKind,
    pub compatibility_profile_id: String,
    pub provider_endpoint_id: Option<ProviderEndpointId>,
    pub provider_account_id: Option<ProviderAccountId>,
    pub embedding_model_name: String,
    pub reasoning_summary: ReasoningSummaryMode,
    pub remote_compaction: bool,
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
            protocol: settings.protocol,
            compatibility_profile_id: settings.compatibility_profile_id.clone(),
            provider_endpoint_id: settings.provider_endpoint_id.clone(),
            provider_account_id: settings.provider_account_id.clone(),
            embedding_model_name: settings.embedding_model_name.clone(),
            reasoning_summary: settings.reasoning_summary,
            remote_compaction: settings.remote_compaction,
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
    pub protocol: ProviderProtocolKind,
    pub compatibility_profile_id: String,
    pub provider_endpoint_id: Option<ProviderEndpointId>,
    pub provider_account_id: Option<ProviderAccountId>,
    pub embedding_model_name: String,
    pub reasoning_summary: ReasoningSummaryMode,
    pub remote_compaction: bool,
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
