use serde::{Deserialize, Serialize};
use specta::Type;

use super::{MutationContext, SandboxCapabilityReport};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SandboxRuntimeSnapshot {
    #[specta(type = specta_typescript::Number)]
    pub revision: u64,
    pub report: SandboxCapabilityReport,
    pub repairing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SandboxRepairRequest {
    pub context: MutationContext,
}
