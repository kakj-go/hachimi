//! Metadata-only audit events. Raw prompts, screenshots and tool payloads are excluded.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEvent {
    pub operation: &'static str,
    pub outcome: &'static str,
    pub principal: Option<String>,
}

impl AuditEvent {
    #[must_use]
    pub const fn decision(operation: &'static str, outcome: &'static str) -> Self {
        Self {
            operation,
            outcome,
            principal: None,
        }
    }

    #[must_use]
    pub fn with_principal(mut self, principal: impl Into<String>) -> Self {
        self.principal = Some(principal.into());
        self
    }
}

pub trait AuditSink: Send + Sync {
    fn record(&self, event: AuditEvent);
}

#[derive(Debug, Default)]
pub struct NoopAudit;

impl AuditSink for NoopAudit {
    fn record(&self, _event: AuditEvent) {}
}
