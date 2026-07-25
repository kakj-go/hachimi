//! Sandbox capability reporting. Phase 0 intentionally has no OS-enforced backend.

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum SandboxStatus {
    #[default]
    Disabled,
}

impl SandboxStatus {
    #[must_use]
    pub const fn is_os_enforced(self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_backend_never_claims_os_enforcement() {
        assert!(!SandboxStatus::Disabled.is_os_enforced());
    }
}
