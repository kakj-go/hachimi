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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SandboxBootstrapPhase {
    NotStarted,
    StagingRuntime,
    InstallingProfile,
    Attesting,
    Ready,
    RepairRequired,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SandboxBootstrapState {
    pub phase: SandboxBootstrapPhase,
    pub runtime_root: String,
    pub profile_sid: Option<String>,
    pub snapshot: SandboxRuntimeSnapshot,
    pub stable_error_code: Option<String>,
}
