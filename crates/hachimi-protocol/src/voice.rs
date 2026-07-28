use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VoiceSettings {
    pub muted: bool,
    #[serde(default = "default_voice_speed_percent")]
    pub speed_percent: u16,
    #[serde(default)]
    pub compute_mode: VoiceComputeMode,
    /// SenseVoice has its own session and can fall back independently from TTS.
    #[serde(default)]
    pub recognition_compute_mode: VoiceComputeMode,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            muted: false,
            speed_percent: default_voice_speed_percent(),
            compute_mode: VoiceComputeMode::Auto,
            recognition_compute_mode: VoiceComputeMode::Auto,
        }
    }
}

#[must_use]
pub const fn default_voice_speed_percent() -> u16 {
    100
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VoiceSettingsInput {
    pub speed_percent: u16,
    pub compute_mode: VoiceComputeMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SpeechRecognitionSettingsInput {
    pub compute_mode: VoiceComputeMode,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum VoiceComputeMode {
    #[default]
    Auto,
    DirectMl,
    Cpu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum VoiceComputeBackend {
    DirectMl,
    Cpu,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VoiceComputeDevice {
    pub device_id: u32,
    pub name: String,
    pub dedicated_memory_mb: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum VoiceModelOrigin {
    BuiltIn,
    Imported,
}

const fn default_voice_speaker_count() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VoiceModelEntry {
    pub id: String,
    pub name: String,
    pub sha256: String,
    pub original_file_name: String,
    pub size_bytes: u32,
    pub origin: VoiceModelOrigin,
    pub model_type: String,
    pub languages: Vec<String>,
    pub sample_rate: u32,
    #[serde(default = "default_voice_speaker_count")]
    pub speaker_count: u32,
    #[serde(default)]
    pub speaker_id: u32,
    pub license_summary: String,
    pub license_warning: bool,
    pub protected: bool,
    pub imported_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VoiceCatalogSnapshot {
    pub entries: Vec<VoiceModelEntry>,
    pub current_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VoiceModelInspection {
    pub token: Option<String>,
    pub original_file_name: String,
    pub size_bytes: u32,
    pub sha256: String,
    pub model_type: String,
    pub languages: Vec<String>,
    pub sample_rate: u32,
    pub speaker_count: u32,
    pub suggested_speaker_id: u32,
    pub required_files: Vec<String>,
    pub license_summary: String,
    pub license_warning: bool,
    pub compatible: bool,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VoiceImportCommitRequest {
    pub token: String,
    pub name: String,
    pub license_acknowledged: bool,
    pub speaker_id: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AvatarRuntimeAsset {
    pub entry_id: String,
    pub name: String,
    pub sha256: String,
    pub asset_url: String,
    pub format: AvatarFormat,
    pub profile: AvatarAdaptationProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SpeechTimelineQuality {
    EnergyLocked,
    PhonemeTimed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SpeechVisemeFrame {
    pub time_ms: u32,
    pub aa: u8,
    pub ih: u8,
    pub ou: u8,
    pub ee: u8,
    pub oh: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SpeechTimeline {
    pub frame_duration_ms: u16,
    pub jaw_open: Vec<u8>,
    pub visemes: Option<Vec<SpeechVisemeFrame>>,
    pub quality: SpeechTimelineQuality,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SpeechPlaybackSource {
    PetTurn,
    WorkbenchPreview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SpeechPlaybackPhase {
    Prepared,
    Playing,
    Progress,
    Completed,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SpeechPlaybackEvent {
    pub playback_id: String,
    pub run_id: Option<String>,
    pub source: SpeechPlaybackSource,
    pub phase: SpeechPlaybackPhase,
    pub media_position_ms: u32,
    pub duration_ms: u32,
    pub sequence: u32,
    pub timeline: Option<SpeechTimeline>,
    pub segment_index: u32,
    pub text_start: u32,
    pub text_end: u32,
    /// Display text is attached to Pet playback preparation so the WebView
    /// can reveal the complete sentence when its PCM segment starts.
    pub display_text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SpeechTurnPhase {
    Started,
    Completed,
    Stopped,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SpeechTurnEvent {
    pub run_id: String,
    pub phase: SpeechTurnPhase,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VoiceRuntimeState {
    pub available: bool,
    pub muted: bool,
    pub model_id: Option<String>,
    pub voice_name: String,
    pub speaking: bool,
    pub speed_percent: u16,
    pub provider: String,
    pub compute_mode: VoiceComputeMode,
    pub backend: Option<VoiceComputeBackend>,
    pub compute_device: Option<VoiceComputeDevice>,
    pub fallback_reason: Option<String>,
    pub loading: bool,
    pub languages: Vec<String>,
    pub speaker_count: u32,
    pub speaker_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SpeechRecognitionRuntimeState {
    pub installed: bool,
    pub installing: bool,
    pub bundled: bool,
    pub model_name: String,
    pub provider: String,
    pub languages: Vec<String>,
    pub size_bytes: u32,
    pub compute_mode: VoiceComputeMode,
    pub backend: Option<VoiceComputeBackend>,
    pub compute_device: Option<VoiceComputeDevice>,
    pub fallback_reason: Option<String>,
    pub loading: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PetTurnRequest {
    pub run_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PetContextMenuRequest {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FrontendLogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FrontendLogEntry {
    pub level: FrontendLogLevel,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PetTurnEvent {
    Started {
        #[serde(rename = "runId")]
        run_id: String,
    },
    TextDelta {
        #[serde(rename = "runId")]
        run_id: String,
        delta: String,
    },
    Completed {
        #[serde(rename = "runId")]
        run_id: String,
        text: String,
        #[serde(rename = "speechQueued")]
        speech_queued: bool,
    },
    Cancelled {
        #[serde(rename = "runId")]
        run_id: String,
    },
    Failed {
        #[serde(rename = "runId")]
        run_id: String,
        code: String,
        message: String,
    },
}
