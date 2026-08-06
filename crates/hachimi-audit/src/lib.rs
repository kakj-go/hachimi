//! Metadata-only audit events. Raw prompts, screenshots and tool payloads are excluded.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEvent {
    pub operation: &'static str,
    pub outcome: &'static str,
    pub principal: Option<String>,
    pub target_summary: Option<String>,
    pub result_code: Option<String>,
}

impl AuditEvent {
    #[must_use]
    pub const fn decision(operation: &'static str, outcome: &'static str) -> Self {
        Self {
            operation,
            outcome,
            principal: None,
            target_summary: None,
            result_code: None,
        }
    }

    #[must_use]
    pub fn with_principal(mut self, principal: impl Into<String>) -> Self {
        self.principal = Some(principal.into());
        self
    }

    #[must_use]
    pub fn with_metadata(
        mut self,
        target_summary: impl Into<String>,
        result_code: impl Into<String>,
    ) -> Self {
        self.target_summary = Some(target_summary.into());
        self.result_code = Some(result_code.into());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_is_owned_by_the_audit_event_without_payload_fields() {
        let event = AuditEvent::decision("computer.act", "allowed")
            .with_principal("local-host-broker")
            .with_metadata(
                "computer:app_sha256:abc:window_sha256:def:action:type_text",
                "succeeded",
            );
        assert_eq!(event.principal.as_deref(), Some("local-host-broker"));
        assert_eq!(event.result_code.as_deref(), Some("succeeded"));
        let summary = event.target_summary.expect("target metadata");
        assert!(summary.contains("app_sha256"));
        assert!(summary.contains("window_sha256"));
        assert!(!summary.contains("input text"));
        assert!(!summary.contains("screenshot"));
    }
}
