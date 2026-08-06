use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ControlInitializeRequest {
    pub client_version: String,
    pub protocol_version: u32,
    pub supported_features: Vec<String>,
    pub experimental_features: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ControlInitializeResponse {
    pub protocol_version: u32,
    pub enabled_features: Vec<String>,
    pub experimental_features: Vec<String>,
    pub warnings: Vec<String>,
    pub sandbox: SandboxCapabilityReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionCursor {
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
    pub id: SessionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionSearchRequest {
    pub project_id: Option<ProjectId>,
    pub query: Option<String>,
    pub archived: Option<bool>,
    pub before: Option<SessionCursor>,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionPage {
    pub items: Vec<SessionRecord>,
    pub next_cursor: Option<SessionCursor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RunSteerStatus {
    Pending,
    Consumed,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunSteerRecord {
    pub id: ItemId,
    pub session_id: SessionId,
    pub run_id: RunId,
    #[specta(type = specta_typescript::Number)]
    pub run_generation: u64,
    pub input: String,
    pub status: RunSteerStatus,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = Option<specta_typescript::Number>)]
    pub consumed_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionResumeRequest {
    pub session_id: SessionId,
    pub metadata_only: bool,
    #[specta(type = Option<specta_typescript::Number>)]
    pub transcript_before_sequence: Option<u64>,
    pub transcript_limit: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionResumeSnapshot {
    pub session: SessionRecord,
    pub active_run: Option<RunRecord>,
    pub transcript: Vec<TranscriptItem>,
    pub pending_approvals: Vec<ApprovalRequestRecord>,
    pub pending_user_inputs: Vec<UserInputRequestRecord>,
    pub usage_snapshot: Option<RunUsageSnapshot>,
    #[serde(default)]
    pub active_event_replay: Vec<RunEventEnvelope>,
    #[specta(type = specta_typescript::Number)]
    pub snapshot_sequence: u64,
    #[specta(type = Option<specta_typescript::Number>)]
    pub previous_transcript_cursor: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EventSubscriptionRequest {
    pub session_id: SessionId,
    #[specta(type = specta_typescript::Number)]
    pub after_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EventSubscriptionRecord {
    pub id: EventSubscriptionId,
    pub session_id: SessionId,
    pub client_id: ClientId,
    #[specta(type = specta_typescript::Number)]
    pub after_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EventSubscriptionSnapshot {
    pub subscription: EventSubscriptionRecord,
    pub catch_up: Vec<RunEventEnvelope>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionForkRequest {
    pub context: MutationContext,
    pub source_session_id: SessionId,
    pub source_run_id: RunId,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionMetadataUpdateRequest {
    pub context: MutationContext,
    pub session_id: SessionId,
    pub title: Option<String>,
    pub archived: Option<bool>,
    pub pinned: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunControlRequest {
    pub context: MutationContext,
    pub run_id: RunId,
    pub input: Option<String>,
}
