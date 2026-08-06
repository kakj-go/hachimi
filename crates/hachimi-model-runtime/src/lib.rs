//! Provider-neutral model runtime contracts used by the Agent kernel.

use std::{future::Future, pin::Pin, sync::Arc};

use futures_util::Stream;
use hachimi_protocol::{
    ModelCompactionRequest, ModelCompactionResult, ModelEvent, ModelMessage, ModelRequest,
    ProviderCapabilities, ProviderCapabilityProbe, ProviderEmbeddingRequest,
    ProviderEmbeddingResult, RunConfiguration, TokenCountSource, WorkloadKind,
};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

pub type ModelEventStream =
    Pin<Box<dyn Stream<Item = Result<ModelEvent, ModelRuntimeError>> + Send>>;
pub type ModelCompactionFuture =
    Pin<Box<dyn Future<Output = Result<ModelCompactionResult, ModelRuntimeError>> + Send>>;
pub type ModelEmbeddingFuture =
    Pin<Box<dyn Future<Output = Result<ProviderEmbeddingResult, ModelRuntimeError>> + Send>>;
pub type ModelClientFuture =
    Pin<Box<dyn Future<Output = Result<Arc<dyn ModelClientSession>, ModelRuntimeError>> + Send>>;
pub type WorkloadClassificationFuture =
    Pin<Box<dyn Future<Output = Result<WorkloadClassificationResult, ModelRuntimeError>> + Send>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadClassificationRequest {
    pub prompt: String,
    pub skill_name: Option<String>,
    pub skill_description: Option<String>,
    pub bounded_skill_markdown: Option<String>,
    pub classifier_revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadClassificationResult {
    pub workload: WorkloadKind,
    pub confidence_basis_points: u16,
    pub reason: String,
    pub classifier_revision: String,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ModelRuntimeError {
    #[error("model request was cancelled")]
    Cancelled,
    #[error("provider does not support required capability: {0}")]
    UnsupportedCapability(&'static str),
    #[error("model provider failed: {0}")]
    Provider(String),
    #[error("model provider rejected the request because its context window was exceeded")]
    ContextOverflow,
    #[error("model provider returned an invalid stream: {0}")]
    InvalidStream(String),
    #[error("agent run requires attention: {0}")]
    NeedsAttention(String),
}

pub trait ModelRuntime: Send + Sync {
    fn capabilities(&self) -> ProviderCapabilities;

    fn capability_probe(&self) -> Option<ProviderCapabilityProbe> {
        None
    }

    fn stream(&self, request: ModelRequest, cancellation: CancellationToken) -> ModelEventStream;

    fn compact(
        &self,
        _request: ModelCompactionRequest,
        _cancellation: CancellationToken,
    ) -> ModelCompactionFuture {
        Box::pin(async {
            Err(ModelRuntimeError::UnsupportedCapability(
                "remote_compaction",
            ))
        })
    }

    fn embed(
        &self,
        _request: ProviderEmbeddingRequest,
        _cancellation: CancellationToken,
    ) -> ModelEmbeddingFuture {
        Box::pin(async { Err(ModelRuntimeError::UnsupportedCapability("embeddings")) })
    }

    /// Strict structured classifier surface. Providers must explicitly advertise
    /// both strict JSON Schema and output-schema support before implementing it.
    fn classify_workload(
        &self,
        _request: WorkloadClassificationRequest,
        _cancellation: CancellationToken,
    ) -> WorkloadClassificationFuture {
        Box::pin(async {
            Err(ModelRuntimeError::UnsupportedCapability(
                "strict_workload_classification",
            ))
        })
    }

    fn count_tokens(&self, messages: &[ModelMessage]) -> (u64, TokenCountSource) {
        (
            conservative_token_estimate(messages),
            TokenCountSource::ConservativeEstimate,
        )
    }
}

/// A Turn-scoped model client. Providers may keep sticky routing or transport
/// state behind this object while the Agent reuses it for every step in a Run.
pub trait ModelClientSession: ModelRuntime {}

impl<T> ModelClientSession for T where T: ModelRuntime + ?Sized {}

pub trait ModelRuntimeFactory: Send + Sync {
    fn create_session(&self, configuration: &RunConfiguration) -> ModelClientFuture;
}

#[must_use]
pub fn conservative_token_estimate(messages: &[ModelMessage]) -> u64 {
    let bytes = messages
        .iter()
        .map(|message| {
            message.content.len()
                + message.name.as_ref().map_or(0, String::len)
                + message
                    .tool_calls
                    .iter()
                    .map(|call| call.name.len() + call.arguments.to_string().len() + 16)
                    .sum::<usize>()
                + 12
        })
        .sum::<usize>();
    u64::try_from(bytes.saturating_add(3) / 4).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use hachimi_protocol::ModelMessage;

    use super::conservative_token_estimate;

    #[test]
    fn conservative_estimate_is_non_zero_for_visible_content() {
        assert!(conservative_token_estimate(&[ModelMessage::user("hello")]) > 0);
    }
}
