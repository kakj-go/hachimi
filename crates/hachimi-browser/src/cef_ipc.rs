use hachimi_protocol::{BrowserNavigationError, BrowserTabId};
use serde::{Deserialize, Serialize};

pub const CEF_IPC_PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CefHostCommandEnvelope {
    pub protocol_version: u32,
    pub request_id: u64,
    pub command: CefHostCommand,
}

impl CefHostCommandEnvelope {
    #[must_use]
    pub fn new(request_id: u64, command: CefHostCommand) -> Self {
        Self {
            protocol_version: CEF_IPC_PROTOCOL_VERSION,
            request_id,
            command,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CefBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl CefBounds {
    #[must_use]
    pub fn validated(self) -> Option<Self> {
        (self.width > 0
            && self.height > 0
            && self.width <= 16_384
            && self.height <= 16_384
            && self.x.unsigned_abs() <= 100_000
            && self.y.unsigned_abs() <= 100_000)
            .then_some(self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CefHostCommand {
    SetParentWindow {
        parent_hwnd: u64,
    },
    CreateTab {
        tab_id: BrowserTabId,
        url: String,
        bounds: CefBounds,
        visible: bool,
    },
    CloseTab {
        tab_id: BrowserTabId,
    },
    ActivateTab {
        tab_id: BrowserTabId,
    },
    SetBounds {
        tab_id: BrowserTabId,
        bounds: CefBounds,
    },
    SetVisible {
        tab_id: BrowserTabId,
        visible: bool,
    },
    SetAgentNavigationPolicy {
        tab_id: BrowserTabId,
        allowed_origins: Vec<String>,
    },
    ClearAgentNavigationPolicy {
        tab_id: BrowserTabId,
    },
    Focus {
        tab_id: BrowserTabId,
    },
    Navigate {
        tab_id: BrowserTabId,
        url: String,
    },
    Back {
        tab_id: BrowserTabId,
    },
    Forward {
        tab_id: BrowserTabId,
    },
    Reload {
        tab_id: BrowserTabId,
        ignore_cache: bool,
    },
    Stop {
        tab_id: BrowserTabId,
    },
    ConfigureDownloads {
        directory: Option<String>,
        ask_where_to_save: bool,
    },
    ClearBrowsingData {
        cookies: bool,
        cache: bool,
    },
    CancelDownload {
        tab_id: BrowserTabId,
        download_id: u32,
    },
    Observe {
        tab_id: BrowserTabId,
    },
    DevTools {
        tab_id: BrowserTabId,
        method: String,
        params: serde_json::Value,
        full_access: bool,
    },
    Shutdown,
}

impl CefHostCommand {
    #[must_use]
    pub fn tab_id(&self) -> Option<&BrowserTabId> {
        match self {
            Self::SetParentWindow { .. }
            | Self::ConfigureDownloads { .. }
            | Self::ClearBrowsingData { .. }
            | Self::Shutdown => None,
            Self::CreateTab { tab_id, .. }
            | Self::CloseTab { tab_id }
            | Self::ActivateTab { tab_id }
            | Self::SetBounds { tab_id, .. }
            | Self::SetVisible { tab_id, .. }
            | Self::SetAgentNavigationPolicy { tab_id, .. }
            | Self::ClearAgentNavigationPolicy { tab_id }
            | Self::Focus { tab_id }
            | Self::Navigate { tab_id, .. }
            | Self::Back { tab_id }
            | Self::Forward { tab_id }
            | Self::Reload { tab_id, .. }
            | Self::Stop { tab_id }
            | Self::CancelDownload { tab_id, .. }
            | Self::Observe { tab_id }
            | Self::DevTools { tab_id, .. } => Some(tab_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CefHostMessage {
    Ready {
        protocol_version: u32,
        chromium_version: String,
    },
    Response {
        request_id: u64,
        result: Result<CefHostResponse, CefHostFailure>,
    },
    Event {
        event: CefHostEvent,
    },
    Fatal {
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CefHostResponse {
    Acknowledged,
    TabCreated { state: CefTabState },
    Observation { observation: CefObservation },
    DevTools { result: serde_json::Value },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CefHostFailure {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl CefHostFailure {
    #[must_use]
    pub fn new(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CefTabState {
    pub tab_id: BrowserTabId,
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub navigation_error: Option<BrowserNavigationError>,
    pub input_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CefObservation {
    pub state: CefTabState,
    pub text: String,
    pub accessibility_tree: serde_json::Value,
    pub screenshot_base64: Option<String>,
    pub screenshot_mime_type: Option<String>,
    pub viewport_width: Option<u32>,
    pub viewport_height: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CefHostEvent {
    TabStateChanged {
        state: CefTabState,
    },
    UserInput {
        tab_id: BrowserTabId,
        input_epoch: u64,
    },
    ShortcutRequested {
        tab_id: BrowserTabId,
        shortcut: CefBrowserShortcut,
    },
    PopupRequested {
        opener_tab_id: BrowserTabId,
        target_url: String,
    },
    AgentNavigationBlocked {
        tab_id: BrowserTabId,
        target_url: String,
    },
    DownloadUpdated {
        tab_id: BrowserTabId,
        download_id: u32,
        url: String,
        suggested_name: String,
        destination: Option<String>,
        received_bytes: u64,
        total_bytes: Option<u64>,
        complete: bool,
        cancelled: bool,
        interrupted: bool,
    },
    RenderProcessTerminated {
        tab_id: BrowserTabId,
        status: String,
    },
    RuntimeCrashed {
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CefBrowserShortcut {
    FocusAddress,
    NewTab,
    CloseTab,
    Reload,
    Back,
    Forward,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_envelope_round_trips_as_one_json_line() {
        let envelope = CefHostCommandEnvelope::new(
            42,
            CefHostCommand::CreateTab {
                tab_id: BrowserTabId::from("tab-1"),
                url: "https://example.com/".into(),
                bounds: CefBounds {
                    x: 10,
                    y: 20,
                    width: 800,
                    height: 600,
                },
                visible: true,
            },
        );
        let json = serde_json::to_string(&envelope).expect("serialize CEF command");
        assert!(!json.contains('\n'));
        assert_eq!(
            serde_json::from_str::<CefHostCommandEnvelope>(&json).expect("decode CEF command"),
            envelope
        );
    }

    #[test]
    fn native_surface_bounds_are_strictly_bounded() {
        assert!(
            CefBounds {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            }
            .validated()
            .is_some()
        );
        assert!(
            CefBounds {
                x: 0,
                y: 0,
                width: 0,
                height: 600,
            }
            .validated()
            .is_none()
        );
        assert!(
            CefBounds {
                x: 0,
                y: 0,
                width: 20_000,
                height: 600,
            }
            .validated()
            .is_none()
        );
    }

    #[test]
    fn parent_window_command_round_trips_without_a_tab_scope() {
        let envelope = CefHostCommandEnvelope::new(
            7,
            CefHostCommand::SetParentWindow {
                parent_hwnd: 0x1234,
            },
        );
        let json = serde_json::to_string(&envelope).expect("serialize parent window command");
        assert!(json.contains("\"kind\":\"set_parent_window\""));
        assert_eq!(
            serde_json::from_str::<CefHostCommandEnvelope>(&json)
                .expect("decode parent window command"),
            envelope
        );
        assert!(envelope.command.tab_id().is_none());
    }
}
