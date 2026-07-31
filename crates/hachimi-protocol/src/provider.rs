use serde::{Deserialize, Serialize};
use specta::Type;

use crate::{
    MutationContext, ProviderAccountId, ProviderCapabilities, ProviderCapabilityProbeId,
    ProviderEndpointId, TokenUsage,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProtocolKind {
    #[default]
    ChatCompletions,
    Responses,
    Embeddings,
}

impl ProviderProtocolKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ChatCompletions => "chat_completions",
            Self::Responses => "responses",
            Self::Embeddings => "embeddings",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "chat_completions" => Self::ChatCompletions,
            "responses" => Self::Responses,
            "embeddings" => Self::Embeddings,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCompatibilityProfileKind {
    #[default]
    OpenAiStrict,
    RegisteredDialect,
}

impl ProviderCompatibilityProfileKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiStrict => "openai_strict",
            Self::RegisteredDialect => "registered_dialect",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "openai_strict" => Self::OpenAiStrict,
            "registered_dialect" => Self::RegisteredDialect,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCompatibilityProfile {
    pub id: String,
    pub display_name: String,
    pub kind: ProviderCompatibilityProfileKind,
    pub protocols: Vec<ProviderProtocolKind>,
    pub profile_revision: String,
    pub builtin: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderEndpointRecord {
    pub id: ProviderEndpointId,
    pub display_name: String,
    pub base_url: String,
    pub compatibility_profile_id: String,
    pub enabled: bool,
    #[specta(type = specta_typescript::Number)]
    pub config_revision: u64,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccountRecord {
    pub id: ProviderAccountId,
    pub endpoint_id: ProviderEndpointId,
    pub display_name: String,
    pub secret_ref: String,
    pub enabled: bool,
    #[specta(type = specta_typescript::Number)]
    pub config_revision: u64,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderEndpointUpsertRequest {
    pub context: MutationContext,
    pub endpoint: ProviderEndpointRecord,
    #[specta(type = Option<specta_typescript::Number>)]
    pub expected_config_revision: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProbeStatus {
    Succeeded,
    Failed,
}

impl ProviderProbeStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderProbeReport {
    pub id: ProviderCapabilityProbeId,
    pub endpoint_id: ProviderEndpointId,
    pub account_id: Option<ProviderAccountId>,
    pub status: ProviderProbeStatus,
    pub protocols: Vec<ProviderProtocolKind>,
    pub capabilities: ProviderCapabilities,
    pub capability_revision: String,
    pub stable_error_code: Option<String>,
    #[specta(type = specta_typescript::Number)]
    pub probed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRegistrySnapshot {
    pub profiles: Vec<ProviderCompatibilityProfile>,
    pub endpoints: Vec<ProviderEndpointRecord>,
    pub accounts: Vec<ProviderAccountRecord>,
    pub latest_probes: Vec<ProviderProbeReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderEmbeddingRequest {
    pub model: String,
    pub input: Vec<String>,
    pub dimensions: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderEmbeddingVector {
    pub index: u32,
    pub values: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderEmbeddingResult {
    pub model: String,
    pub vectors: Vec<ProviderEmbeddingVector>,
    pub usage: TokenUsage,
}
