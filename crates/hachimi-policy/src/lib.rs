//! Deterministic policy decisions. This crate never delegates authorization to an LLM.

use hachimi_protocol::{ClientContext, ControlMethod, Scope};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Deny,
}

pub trait PolicyEngine: Send + Sync {
    fn evaluate(
        &self,
        client: &ClientContext,
        method: ControlMethod,
        required_scope: Scope,
    ) -> PolicyDecision;
}

#[derive(Debug, Default)]
pub struct DefaultPolicy;

impl PolicyEngine for DefaultPolicy {
    fn evaluate(
        &self,
        client: &ClientContext,
        _method: ControlMethod,
        required_scope: Scope,
    ) -> PolicyDecision {
        if client.scopes.contains(&required_scope) {
            PolicyDecision::Allow
        } else {
            PolicyDecision::Deny
        }
    }
}

#[cfg(test)]
mod tests {
    use hachimi_core::WindowKind;

    use super::*;

    #[test]
    fn exact_scope_is_required() {
        let policy = DefaultPolicy;
        let pet = ClientContext::for_window(WindowKind::Pet);
        assert_eq!(
            policy.evaluate(&pet, ControlMethod::SettingsRead, Scope::SettingsRead),
            PolicyDecision::Deny
        );
    }
}
