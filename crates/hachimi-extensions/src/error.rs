use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExtensionHostError {
    #[error("plugin source is invalid")]
    InvalidSource,
    #[error("plugin manifest is missing or invalid: {0}")]
    InvalidManifest(String),
    #[error("plugin contribution path escapes the bundle")]
    ContributionEscape,
    #[error("plugin bundle contains a symbolic link or reparse point")]
    SymbolicLink,
    #[error("plugin bundle exceeds its file or size limit")]
    BundleTooLarge,
    #[error("plugin I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("plugin storage failed: {0}")]
    Store(#[from] hachimi_storage::AgentStoreError),
    #[error("plugin database failed: {0}")]
    Database(#[from] sqlx::Error),
    #[error("plugin serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("plugin is not installed")]
    PluginNotFound,
    #[error("plugin revision is not available")]
    PluginRevisionNotFound,
    #[error("plugin lifecycle operation conflicts with another active operation")]
    LifecycleConflict,
    #[error("connector account is not healthy")]
    ConnectorNotHealthy,
    #[error("connector revision drifted")]
    ConnectorDrift,
    #[error("plugin contribution is unavailable or its pinned revision drifted")]
    ContributionDrift,
    #[error("connector action or arguments are invalid")]
    InvalidInvocation,
    #[error("connector rate limit is active; retry after the persisted backoff window")]
    RateLimited,
    #[error("connector idempotency key was reused with different input")]
    IdempotencyConflict,
    #[error("enterprise connector outcome is indeterminate")]
    EnterpriseIndeterminate,
    #[error("enterprise connector provider rejected the operation: {0}")]
    EnterpriseProvider(String),
    #[error("enterprise connector transport is unavailable")]
    EnterpriseTransport,
    #[error("enterprise attachment metadata or Run generation drifted")]
    EnterpriseAttachmentDrift,
    #[error("enterprise attachment request belongs to a stale Run generation")]
    StaleRunGeneration,
    #[error("enterprise attachment exceeds the 25 MiB limit")]
    EnterpriseAttachmentTooLarge,
    #[error("enterprise attachment type is denied or does not match its metadata")]
    EnterpriseAttachmentTypeDenied,
    #[error("connector credential storage is unavailable")]
    SecretStore,
    #[error("plugin contribution is unsupported in this release: {0}")]
    UnsupportedContribution(String),
    #[error("connector sidecar failed closed: {0}")]
    Sidecar(&'static str),
}

impl ExtensionHostError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidSource => "plugin_source_invalid",
            Self::InvalidManifest(_) => "plugin_manifest_invalid",
            Self::ContributionEscape => "plugin_contribution_escape",
            Self::SymbolicLink => "plugin_symbolic_link_denied",
            Self::BundleTooLarge => "plugin_bundle_too_large",
            Self::Io(_) => "plugin_io_failed",
            Self::Store(_) | Self::Database(_) => "plugin_storage_failed",
            Self::Serialization(_) => "plugin_serialization_failed",
            Self::PluginNotFound => "plugin_not_found",
            Self::PluginRevisionNotFound => "plugin_revision_not_found",
            Self::LifecycleConflict => "plugin_lifecycle_conflict",
            Self::ConnectorNotHealthy => "connector_not_healthy",
            Self::ConnectorDrift | Self::ContributionDrift => "connector_revision_drift",
            Self::InvalidInvocation => "connector_invalid_invocation",
            Self::RateLimited => "connector_rate_limited",
            Self::IdempotencyConflict => "connector_idempotency_conflict",
            Self::EnterpriseIndeterminate => "enterprise_outcome_indeterminate",
            Self::EnterpriseProvider(_) => "enterprise_provider_rejected",
            Self::EnterpriseTransport => "enterprise_transport_failed",
            Self::EnterpriseAttachmentDrift => "enterprise_attachment_drift",
            Self::StaleRunGeneration => "stale_run_generation",
            Self::EnterpriseAttachmentTooLarge => "enterprise_attachment_too_large",
            Self::EnterpriseAttachmentTypeDenied => "enterprise_attachment_type_denied",
            Self::SecretStore => "connector_secret_store_failed",
            Self::UnsupportedContribution(_) => "plugin_contribution_unsupported",
            Self::Sidecar(_) => "connector_sidecar_failed",
        }
    }
}

pub(crate) fn connector_error_code(error: &ExtensionHostError) -> (&'static str, bool) {
    match error {
        ExtensionHostError::RateLimited => ("connector_rate_limited", true),
        ExtensionHostError::EnterpriseTransport => ("enterprise_transport_failed", true),
        ExtensionHostError::EnterpriseIndeterminate => ("enterprise_outcome_indeterminate", false),
        ExtensionHostError::EnterpriseAttachmentDrift => ("enterprise_attachment_drift", false),
        ExtensionHostError::StaleRunGeneration => ("stale_run_generation", false),
        ExtensionHostError::EnterpriseAttachmentTooLarge => {
            ("enterprise_attachment_too_large", false)
        }
        ExtensionHostError::EnterpriseAttachmentTypeDenied => {
            ("enterprise_attachment_type_denied", false)
        }
        ExtensionHostError::EnterpriseProvider(_) => ("enterprise_provider_rejected", false),
        ExtensionHostError::ConnectorNotHealthy => ("connector_not_healthy", false),
        ExtensionHostError::ConnectorDrift | ExtensionHostError::ContributionDrift => {
            ("connector_revision_drift", false)
        }
        ExtensionHostError::InvalidInvocation => ("connector_invalid_invocation", false),
        ExtensionHostError::IdempotencyConflict => ("connector_idempotency_conflict", false),
        ExtensionHostError::Sidecar(_) => ("connector_sidecar_failed", true),
        _ => ("connector_operation_failed", false),
    }
}
