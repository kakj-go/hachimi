//! Browser and desktop-computer Host contracts.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use super::{
    BrowserObservationId, BrowserPairingId, BrowserSessionId, ComputerFrameId, ItemId, RunId,
    SessionId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum BrowserProfileKind {
    Isolated,
    ChromeExtension,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum BrowserCapability {
    Observe,
    Act,
    Upload,
    Download,
    CookieStorage,
    Cdp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum BrowserPermissionDecision {
    Deny,
    AllowOnce,
    AllowSession,
    AllowPersisted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum BrowserNetworkRuleKind {
    Document,
    Resource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BrowserNetworkRule {
    pub origin: String,
    pub kind: BrowserNetworkRuleKind,
    pub allow_private_network: bool,
    #[specta(type = Option<specta_typescript::Number>)]
    pub expires_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "camelCase")]
pub struct BrowserNetworkPolicy {
    pub rules: Vec<BrowserNetworkRule>,
    pub deny_private_network_by_default: bool,
    #[specta(type = specta_typescript::Number)]
    pub revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum BrowserPermissionRequestStatus {
    Pending,
    Allowed,
    Denied,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BrowserPermissionRequest {
    pub id: ItemId,
    pub browser_session_id: BrowserSessionId,
    pub owner_session_id: SessionId,
    pub owner_run_id: RunId,
    #[specta(type = specta_typescript::Number)]
    pub run_generation: u64,
    pub origin: String,
    pub capabilities: Vec<BrowserCapability>,
    pub network_kind: BrowserNetworkRuleKind,
    pub private_network: bool,
    pub status: BrowserPermissionRequestStatus,
    #[specta(type = specta_typescript::Number)]
    pub expected_browser_revision: u64,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = specta_typescript::Number)]
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BrowserPermissionRequiredEvent {
    pub request: BrowserPermissionRequest,
    pub reason_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSitePermission {
    pub origin: String,
    pub capabilities: Vec<BrowserCapability>,
    pub decision: BrowserPermissionDecision,
    pub granted_by: String,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = Option<specta_typescript::Number>)]
    pub expires_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BrowserPermissionLedgerEntry {
    pub browser_session_id: BrowserSessionId,
    pub owner_session_id: SessionId,
    pub owner_run_id: RunId,
    #[specta(type = specta_typescript::Number)]
    pub run_generation: u64,
    #[specta(type = specta_typescript::Number)]
    pub browser_revision: u64,
    pub permission: BrowserSitePermission,
    pub network_rules: Vec<BrowserNetworkRule>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSessionStatus {
    Starting,
    Ready,
    TakenOver,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSession {
    pub id: BrowserSessionId,
    pub profile_kind: BrowserProfileKind,
    pub owner_session_id: SessionId,
    pub owner_run_id: RunId,
    #[specta(type = specta_typescript::Number)]
    pub run_generation: u64,
    pub origin: Option<String>,
    pub task_tab_group: String,
    #[specta(type = specta_typescript::Number)]
    pub revision: u64,
    pub status: BrowserSessionStatus,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BrowserObservation {
    pub id: BrowserObservationId,
    pub browser_session_id: BrowserSessionId,
    #[specta(type = specta_typescript::Number)]
    pub run_generation: u64,
    #[specta(type = specta_typescript::Number)]
    pub browser_revision: u64,
    pub origin: String,
    pub title: String,
    pub text: String,
    pub external_content: bool,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserAction {
    Navigate {
        url: String,
    },
    Back,
    Forward,
    Reload {
        #[serde(default)]
        ignore_cache: bool,
    },
    Stop,
    Click {
        selector: String,
    },
    Hover {
        selector: String,
    },
    DoubleClick {
        selector: String,
    },
    Scroll {
        selector: Option<String>,
        delta_x: i32,
        delta_y: i32,
    },
    DragDrop {
        source_selector: String,
        target_selector: String,
    },
    Clear {
        selector: String,
    },
    Fill {
        selector: String,
        text: String,
    },
    SelectOption {
        selector: String,
        value: String,
    },
    PressKeys {
        keys: Vec<String>,
    },
    WaitFor {
        selector: Option<String>,
        state: BrowserWaitState,
        #[specta(type = specta_typescript::Number)]
        timeout_ms: u64,
    },
    TabList,
    TabNew {
        url: Option<String>,
    },
    TabSwitch {
        tab_id: String,
    },
    TabClose {
        tab_id: String,
    },
    TypeText {
        selector: String,
        text: String,
    },
    Upload {
        selector: String,
        file_token: String,
    },
    Download {
        selector: String,
        #[serde(default)]
        allow_unknown_type: bool,
    },
    ReadStorage,
    WriteStorage {
        #[specta(type = specta_typescript::Unknown)]
        entries: Value,
    },
    Cdp {
        method: String,
        #[specta(type = specta_typescript::Unknown)]
        params: Value,
    },
}

impl BrowserAction {
    #[must_use]
    pub const fn required_capability(&self) -> BrowserCapability {
        match self {
            Self::WaitFor { .. } | Self::TabList => BrowserCapability::Observe,
            Self::Navigate { .. }
            | Self::Back
            | Self::Forward
            | Self::Reload { .. }
            | Self::Stop
            | Self::Click { .. }
            | Self::Hover { .. }
            | Self::DoubleClick { .. }
            | Self::Scroll { .. }
            | Self::DragDrop { .. }
            | Self::Clear { .. }
            | Self::Fill { .. }
            | Self::SelectOption { .. }
            | Self::PressKeys { .. }
            | Self::TabNew { .. }
            | Self::TabSwitch { .. }
            | Self::TabClose { .. }
            | Self::TypeText { .. } => BrowserCapability::Act,
            Self::Upload { .. } => BrowserCapability::Upload,
            Self::Download { .. } => BrowserCapability::Download,
            Self::ReadStorage | Self::WriteStorage { .. } => BrowserCapability::CookieStorage,
            Self::Cdp { .. } => BrowserCapability::Cdp,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum BrowserWaitState {
    Attached,
    Visible,
    Hidden,
    NavigationComplete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BrowserActionRequest {
    pub browser_session_id: BrowserSessionId,
    pub observation_id: BrowserObservationId,
    #[serde(default)]
    #[specta(type = specta_typescript::Number)]
    pub run_generation: u64,
    #[specta(type = specta_typescript::Number)]
    pub expected_revision: u64,
    pub action: BrowserAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BrowserActionResult {
    pub browser_session_id: BrowserSessionId,
    #[specta(type = specta_typescript::Number)]
    pub revision: u64,
    pub accepted: bool,
    pub result_code: String,
    #[specta(type = Option<specta_typescript::Unknown>)]
    pub output: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BrowserFileToken {
    pub browser_session_id: BrowserSessionId,
    pub token: String,
    pub file_name: String,
    #[specta(type = specta_typescript::Number)]
    pub size: u64,
    pub sha256: String,
    #[specta(type = specta_typescript::Number)]
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BrowserImportedDownload {
    pub browser_session_id: BrowserSessionId,
    pub download_token: String,
    pub destination: String,
    #[specta(type = specta_typescript::Number)]
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BrowserPairing {
    pub id: BrowserPairingId,
    pub nonce: String,
    pub extension_identity: Option<String>,
    pub confirmed: bool,
    #[specta(type = specta_typescript::Number)]
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BrowserHostSettings {
    pub preferred_profile_kind: BrowserProfileKind,
    pub latest_pairing: Option<BrowserPairing>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ComputerWindowIdentity {
    pub app_id: String,
    pub process_id: u32,
    pub window_handle: String,
    pub fingerprint: String,
    pub title: String,
    pub elevated: bool,
    pub protected_desktop: bool,
    pub hachimi_owned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ComputerFrame {
    pub id: ComputerFrameId,
    pub session_id: SessionId,
    pub run_id: RunId,
    #[specta(type = specta_typescript::Number)]
    pub run_generation: u64,
    pub target: ComputerWindowIdentity,
    pub image_token: String,
    pub width: u32,
    pub height: u32,
    #[specta(type = specta_typescript::Number)]
    pub input_epoch: u64,
    #[specta(type = specta_typescript::Number)]
    pub created_at_ms: i64,
    #[specta(type = specta_typescript::Number)]
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ComputerAction {
    MouseMove {
        x: i32,
        y: i32,
    },
    MouseClick {
        x: i32,
        y: i32,
        button: String,
    },
    MouseDown {
        x: i32,
        y: i32,
        button: String,
    },
    MouseUp {
        x: i32,
        y: i32,
        button: String,
    },
    MouseDoubleClick {
        x: i32,
        y: i32,
        button: String,
    },
    MouseDrag {
        from_x: i32,
        from_y: i32,
        to_x: i32,
        to_y: i32,
        button: String,
    },
    Scroll {
        delta_x: i32,
        delta_y: i32,
    },
    KeyPress {
        key: String,
        modifiers: Vec<String>,
    },
    KeyDown {
        key: String,
    },
    KeyUp {
        key: String,
    },
    KeyChord {
        keys: Vec<String>,
    },
    TypeText {
        text: String,
    },
    WindowFocus,
    WindowMove {
        x: i32,
        y: i32,
    },
    WindowResize {
        width: u32,
        height: u32,
    },
    WindowMinimize,
    WindowMaximize,
    WindowRestore,
    WindowClose,
    LaunchApp {
        app_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ComputerActionRequest {
    pub frame_id: ComputerFrameId,
    #[serde(default)]
    #[specta(type = specta_typescript::Number)]
    pub run_generation: u64,
    pub target_fingerprint: String,
    #[specta(type = specta_typescript::Number)]
    pub expected_input_epoch: u64,
    pub action: ComputerAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ComputerActionResult {
    pub frame_id: ComputerFrameId,
    pub accepted: bool,
    pub result_code: String,
    #[specta(type = specta_typescript::Number)]
    pub next_input_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ComputerAppRule {
    pub app_id: String,
    pub observe: bool,
    pub act: bool,
    pub always_allowed: bool,
    pub granted_by: String,
    #[specta(type = specta_typescript::Number)]
    pub updated_at_ms: i64,
}
