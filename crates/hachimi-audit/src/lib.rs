//! Metadata-only audit events. Raw prompts, screenshots and tool payloads are excluded.

use parking_lot::RwLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEvent {
    pub operation: &'static str,
    pub outcome: &'static str,
}

pub trait AuditSink: Send + Sync {
    fn record(&self, event: AuditEvent);
}

#[derive(Debug, Default)]
pub struct InMemoryAudit {
    events: RwLock<Vec<AuditEvent>>,
}

impl AuditSink for InMemoryAudit {
    fn record(&self, event: AuditEvent) {
        self.events.write().push(event);
    }
}

impl InMemoryAudit {
    #[must_use]
    pub fn snapshot(&self) -> Vec<AuditEvent> {
        self.events.read().clone()
    }
}
