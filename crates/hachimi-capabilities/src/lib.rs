//! Registry for future narrow Capability Hosts. It starts empty in the MVP.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDescriptor {
    pub host_id: String,
    pub host_kind: String,
    pub commands: Vec<String>,
}

#[derive(Debug, Default)]
pub struct CapabilityRegistry {
    hosts: RwLock<Vec<CapabilityDescriptor>>,
}

impl CapabilityRegistry {
    #[must_use]
    pub fn list(&self) -> Vec<CapabilityDescriptor> {
        self.hosts.read().clone()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hosts.read().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_starts_without_high_permission_hosts() {
        assert!(CapabilityRegistry::default().is_empty());
    }
}
