use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeComponentId {
    Gateway,
    InternalResources,
    Mcp,
    Scheduler,
    BrowserExtension,
    Cef,
    ComputerUse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeComponentState {
    Starting,
    Ready,
    Retrying,
    Degraded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeComponentHealth {
    pub component: RuntimeComponentId,
    pub state: RuntimeComponentState,
    pub error_code: Option<String>,
    pub retryable: bool,
    pub attempt: u32,
    #[specta(type = Option<specta_typescript::Number>)]
    pub next_retry_at_ms: Option<i64>,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeHealthSnapshot {
    pub components: Vec<RuntimeComponentHealth>,
    #[specta(type = specta_typescript::Number)]
    pub revision: u64,
}
