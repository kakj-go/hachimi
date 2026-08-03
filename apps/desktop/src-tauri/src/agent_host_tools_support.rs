use hachimi_protocol::{ComputerAction, ComputerWindowIdentity};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub(super) fn browser_target_summary(origin: &str, action: &str) -> String {
    format!(
        "browser:origin_sha256:{}:action:{action}",
        stable_hash(origin.as_bytes())
    )
}

pub(super) const fn browser_error_code(error: &hachimi_browser::BrowserHostError) -> &'static str {
    match error {
        hachimi_browser::BrowserHostError::SandboxNotReady => "sandbox_not_ready",
        hachimi_browser::BrowserHostError::InvalidOrigin => "invalid_origin",
        hachimi_browser::BrowserHostError::SessionNotFound => "session_not_found",
        hachimi_browser::BrowserHostError::SessionOwnershipMismatch => "ownership_mismatch",
        hachimi_browser::BrowserHostError::SessionInactive => "session_inactive",
        hachimi_browser::BrowserHostError::StaleObservation => "stale_observation",
        hachimi_browser::BrowserHostError::StaleRunGeneration => "stale_run_generation",
        hachimi_browser::BrowserHostError::PermissionMissing => "permission_missing",
        hachimi_browser::BrowserHostError::PairingInvalid => "pairing_invalid",
        hachimi_browser::BrowserHostError::InvalidInput => "invalid_input",
        hachimi_browser::BrowserHostError::BrokerUnavailable => "broker_unavailable",
        hachimi_browser::BrowserHostError::BrokerUnsupportedMode => "broker_mode_unsupported",
        hachimi_browser::BrowserHostError::Broker(_) => "broker_failed",
        hachimi_browser::BrowserHostError::ActionInFlight => "action_in_flight",
        hachimi_browser::BrowserHostError::UploadTokenInvalid => "upload_token_invalid",
        hachimi_browser::BrowserHostError::DownloadFailed => "download_failed",
        hachimi_browser::BrowserHostError::DownloadConfirmationRequired => {
            "download_confirmation_required"
        }
        hachimi_browser::BrowserHostError::NetworkOriginDenied => "network_origin_denied",
        hachimi_browser::BrowserHostError::PrivateNetworkDenied => "private_network_denied",
        hachimi_browser::BrowserHostError::NetworkResolutionDenied => "network_resolution_denied",
        hachimi_browser::BrowserHostError::CdpMethodUnsupported => "cdp_method_unsupported",
        hachimi_browser::BrowserHostError::ExtensionAuthenticationFailed => {
            "extension_authentication_failed"
        }
        hachimi_browser::BrowserHostError::ExtensionCommandInvalid => "extension_command_invalid",
        hachimi_browser::BrowserHostError::ExtensionCommandTimeout => "extension_command_timeout",
    }
}

pub(super) fn computer_target_summary(target: &ComputerWindowIdentity, action: &str) -> String {
    format!(
        "computer:app_sha256:{}:window_sha256:{}:action:{action}",
        stable_hash(target.app_id.as_bytes()),
        stable_hash(target.fingerprint.as_bytes())
    )
}

pub(super) const fn computer_action_category(action: &ComputerAction) -> &'static str {
    match action {
        ComputerAction::MouseMove { .. } => "mouse_move",
        ComputerAction::MouseClick { .. } => "mouse_click",
        ComputerAction::MouseDown { .. } => "mouse_down",
        ComputerAction::MouseUp { .. } => "mouse_up",
        ComputerAction::MouseDoubleClick { .. } => "mouse_double_click",
        ComputerAction::MouseDrag { .. } => "mouse_drag",
        ComputerAction::Scroll { .. } => "scroll",
        ComputerAction::KeyPress { .. } => "key_press",
        ComputerAction::KeyDown { .. } => "key_down",
        ComputerAction::KeyUp { .. } => "key_up",
        ComputerAction::KeyChord { .. } => "key_chord",
        ComputerAction::TypeText { .. } => "type_text",
        ComputerAction::WindowFocus => "window_focus",
        ComputerAction::WindowMove { .. } => "window_move",
        ComputerAction::WindowResize { .. } => "window_resize",
        ComputerAction::WindowMinimize => "window_minimize",
        ComputerAction::WindowMaximize => "window_maximize",
        ComputerAction::WindowRestore => "window_restore",
        ComputerAction::WindowClose => "window_close",
        ComputerAction::LaunchApp { .. } => "launch_app",
    }
}

pub(super) const fn computer_error_code(
    error: &hachimi_computer::ComputerHostError,
) -> &'static str {
    match error {
        hachimi_computer::ComputerHostError::SandboxNotReady => "sandbox_not_ready",
        hachimi_computer::ComputerHostError::ObserveNotGranted => "observe_not_granted",
        hachimi_computer::ComputerHostError::ActNotGranted => "act_not_granted",
        hachimi_computer::ComputerHostError::AppNotAllowed => "app_not_allowed",
        hachimi_computer::ComputerHostError::ProtectedTarget => "protected_target",
        hachimi_computer::ComputerHostError::SelfTarget => "self_target",
        hachimi_computer::ComputerHostError::FrameNotFound => "frame_not_found",
        hachimi_computer::ComputerHostError::StaleFrame => "stale_frame",
        hachimi_computer::ComputerHostError::StaleRunGeneration => "stale_run_generation",
        hachimi_computer::ComputerHostError::TargetChanged => "target_changed",
        hachimi_computer::ComputerHostError::UserTakeover => "user_takeover",
        hachimi_computer::ComputerHostError::ActionLimitReached => "action_limit_reached",
        hachimi_computer::ComputerHostError::Broker(_) => "broker_failed",
        hachimi_computer::ComputerHostError::InvalidAction => "invalid_action",
    }
}

pub(super) fn stable_hash(value: &[u8]) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn connector_source(metadata: &Value) -> Option<(String, Option<String>)> {
    let url = metadata
        .get("sourceUrl")
        .and_then(Value::as_str)
        .and_then(hachimi_storage::canonical_session_source_url)?;
    let title = metadata
        .get("sourceTitle")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Some((url, title))
}

pub(super) fn object_schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

pub(super) fn now_ms() -> i64 {
    i64::try_from(crate::epoch_millis()).unwrap_or(i64::MAX)
}
