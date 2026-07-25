//! Approval contract with a fail-closed non-interactive implementation.

use hachimi_protocol::{ClientContext, ControlMethod};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalRequirement {
    NotRequired,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    NotRequired,
    Denied,
}

pub trait ApprovalBroker: Send + Sync {
    fn decide(
        &self,
        client: &ClientContext,
        method: ControlMethod,
        requirement: ApprovalRequirement,
    ) -> ApprovalDecision;
}

#[derive(Debug, Default)]
pub struct NonInteractiveApproval;

impl ApprovalBroker for NonInteractiveApproval {
    fn decide(
        &self,
        _client: &ClientContext,
        _method: ControlMethod,
        requirement: ApprovalRequirement,
    ) -> ApprovalDecision {
        match requirement {
            ApprovalRequirement::NotRequired => ApprovalDecision::NotRequired,
            ApprovalRequirement::Required => ApprovalDecision::Denied,
        }
    }
}

#[cfg(test)]
mod tests {
    use hachimi_core::WindowKind;

    use super::*;

    #[test]
    fn noninteractive_approval_fails_closed() {
        let broker = NonInteractiveApproval;
        let client = ClientContext::for_window(WindowKind::Pet);
        assert_eq!(
            broker.decide(
                &client,
                ControlMethod::WindowInteract,
                ApprovalRequirement::Required
            ),
            ApprovalDecision::Denied
        );
    }
}
