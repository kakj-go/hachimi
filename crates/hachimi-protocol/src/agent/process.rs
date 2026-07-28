use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use specta::Type;

use super::{
    CheckoutId, MutationContext, ProcessSessionId, ProcessSessionRecord, RunId, SessionId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProcessTerminalSize {
    pub rows: u16,
    pub cols: u16,
}

impl Default for ProcessTerminalSize {
    fn default() -> Self {
        Self { rows: 24, cols: 80 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ProcessOutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProcessSpawnRequest {
    pub context: MutationContext,
    pub session_id: SessionId,
    pub checkout_id: CheckoutId,
    pub command: Vec<String>,
    pub tty: bool,
    pub stream_stdin: bool,
    pub stream_output: bool,
    #[specta(type = Option<specta_typescript::Number>)]
    pub output_bytes_cap: Option<u64>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub timeout_ms: Option<u64>,
    pub environment: BTreeMap<String, Option<String>>,
    pub size: Option<ProcessTerminalSize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProcessWriteRequest {
    pub context: MutationContext,
    pub process_session_id: ProcessSessionId,
    pub write_id: String,
    pub delta_base64: Option<String>,
    pub close_stdin: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProcessResizeRequest {
    pub context: MutationContext,
    pub process_session_id: ProcessSessionId,
    pub size: ProcessTerminalSize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProcessTerminateRequest {
    pub context: MutationContext,
    pub process_session_id: ProcessSessionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProcessReadRequest {
    pub process_session_id: ProcessSessionId,
    #[specta(type = Option<specta_typescript::Number>)]
    pub after_sequence: Option<u64>,
    pub max_bytes: Option<u32>,
    pub wait_ms: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProcessListRequest {
    pub session_id: Option<SessionId>,
    pub run_id: Option<RunId>,
    pub include_terminal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProcessOutputChunk {
    #[specta(type = specta_typescript::Number)]
    pub sequence: u64,
    pub stream: ProcessOutputStream,
    pub delta_base64: String,
    pub cap_reached: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProcessReadSnapshot {
    pub process: ProcessSessionRecord,
    pub chunks: Vec<ProcessOutputChunk>,
    #[specta(type = specta_typescript::Number)]
    pub next_sequence: u64,
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProcessEvent {
    Output {
        process_session_id: ProcessSessionId,
        chunk: ProcessOutputChunk,
    },
    Exited {
        process_session_id: ProcessSessionId,
        #[specta(type = specta_typescript::Number)]
        sequence: u64,
        exit_code: i32,
    },
    Closed {
        process_session_id: ProcessSessionId,
        #[specta(type = specta_typescript::Number)]
        sequence: u64,
    },
}
