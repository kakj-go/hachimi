//! App-scoped Computer Host with frame and user-input epoch fencing.

use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use hachimi_core::FeatureAvailability;
use hachimi_protocol::{
    CapabilityGrantSet, ComputerAction, ComputerActionRequest, ComputerActionResult,
    ComputerAppRule, ComputerFrame, ComputerFrameId, ComputerWindowIdentity, RunId,
    SandboxCapabilityReport, SandboxReadiness, SessionId,
};
use parking_lot::Mutex;
use sha2::Digest as _;
use thiserror::Error;

mod platform;
pub use platform::PlatformComputerBroker;

const FRAME_TTL_MS: i64 = 15_000;
pub(crate) const MAX_FRAME_IMAGE_BYTES: usize = 20 * 1024 * 1024;

#[must_use]
pub const fn availability() -> FeatureAvailability {
    FeatureAvailability::Available
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ComputerHostError {
    #[error("computer sandbox readiness is not enforced")]
    SandboxNotReady,
    #[error("computer observe permission is missing")]
    ObserveNotGranted,
    #[error("computer act permission is missing")]
    ActNotGranted,
    #[error("computer target application is not allowed")]
    AppNotAllowed,
    #[error("computer target is elevated or belongs to a protected desktop")]
    ProtectedTarget,
    #[error("computer target is owned by Hachimi")]
    SelfTarget,
    #[error("computer frame was not found")]
    FrameNotFound,
    #[error("computer frame is stale")]
    StaleFrame,
    #[error("computer request belongs to a stale Run generation")]
    StaleRunGeneration,
    #[error("computer target identity changed")]
    TargetChanged,
    #[error("computer action was invalidated by user input or takeover")]
    UserTakeover,
    #[error("computer action limit was reached for this Run")]
    ActionLimitReached,
    #[error("computer input was rejected by the broker: {0}")]
    Broker(String),
    #[error("computer action is invalid")]
    InvalidAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedWindow {
    pub target: ComputerWindowIdentity,
    pub image_token: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerFrameImage {
    pub media_type: String,
    pub bytes: Vec<u8>,
    pub sha256: String,
}

pub type ComputerBrokerFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, ComputerHostError>> + Send + 'a>>;

pub trait ComputerBroker: Send + Sync {
    fn list_windows<'a>(&'a self) -> ComputerBrokerFuture<'a, Vec<ComputerWindowIdentity>> {
        Box::pin(async {
            Err(ComputerHostError::Broker(
                "the Computer broker cannot enumerate windows".into(),
            ))
        })
    }

    fn capture<'a>(&'a self, window_handle: &'a str) -> ComputerBrokerFuture<'a, CapturedWindow>;

    fn current_identity<'a>(
        &'a self,
        window_handle: &'a str,
    ) -> ComputerBrokerFuture<'a, ComputerWindowIdentity>;

    fn perform<'a>(
        &'a self,
        target: &'a ComputerWindowIdentity,
        action: &'a ComputerAction,
    ) -> ComputerBrokerFuture<'a, ()>;

    fn read_frame<'a>(&'a self, _image_token: &'a str) -> ComputerBrokerFuture<'a, Vec<u8>> {
        Box::pin(async {
            Err(ComputerHostError::Broker(
                "the Computer broker does not expose frame bytes".into(),
            ))
        })
    }

    /// Releases the ephemeral frame backing storage as soon as the bytes have
    /// been consumed or the frame is fenced out. Implementations must be
    /// idempotent because cleanup also runs during expiry and takeover.
    fn release_frame(&self, _image_token: &str) {}

    fn user_input_marker(&self) -> Option<u64> {
        None
    }
}

pub trait ComputerClock: Send + Sync {
    fn now_ms(&self) -> i64;
}

#[derive(Debug)]
pub struct SystemComputerClock;

impl ComputerClock for SystemComputerClock {
    fn now_ms(&self) -> i64 {
        now_ms()
    }
}

#[derive(Debug, Clone)]
struct FrameState {
    frame: ComputerFrame,
    window_handle: String,
    user_input_marker: Option<u64>,
}

#[derive(Debug, Default)]
struct ComputerState {
    frames: BTreeMap<ComputerFrameId, FrameState>,
    rules: BTreeMap<(SessionId, String), ComputerAppRule>,
    input_epochs: BTreeMap<SessionId, u64>,
    action_counts: BTreeMap<RunId, u32>,
}

#[derive(Clone)]
pub struct ComputerHost {
    broker: Arc<dyn ComputerBroker>,
    clock: Arc<dyn ComputerClock>,
    state: Arc<Mutex<ComputerState>>,
}

impl std::fmt::Debug for ComputerHost {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ComputerHost")
            .finish_non_exhaustive()
    }
}

impl ComputerHost {
    #[must_use]
    pub fn new(broker: Arc<dyn ComputerBroker>, clock: Arc<dyn ComputerClock>) -> Self {
        Self {
            broker,
            clock,
            state: Arc::new(Mutex::new(ComputerState::default())),
        }
    }

    pub fn set_app_rule(&self, session_id: &SessionId, rule: ComputerAppRule) {
        self.state
            .lock()
            .rules
            .insert((session_id.clone(), rule.app_id.clone()), rule);
    }

    pub async fn list_windows(&self) -> Result<Vec<ComputerWindowIdentity>, ComputerHostError> {
        let mut windows = self.broker.list_windows().await?;
        windows.retain(|target| validate_target(target).is_ok());
        windows.sort_by(|left, right| {
            left.app_id
                .cmp(&right.app_id)
                .then_with(|| left.title.cmp(&right.title))
                .then_with(|| left.window_handle.cmp(&right.window_handle))
        });
        windows.dedup_by(|left, right| left.window_handle == right.window_handle);
        Ok(windows)
    }

    pub async fn observe(
        &self,
        session_id: SessionId,
        run_id: RunId,
        run_generation: u64,
        window_handle: &str,
        grants: &CapabilityGrantSet,
        sandbox: &SandboxCapabilityReport,
    ) -> Result<ComputerFrame, ComputerHostError> {
        require_sandbox(sandbox)?;
        if !grants.computer.observe
            || grants.session_id != session_id
            || grants.run_id.as_ref() != Some(&run_id)
        {
            return Err(ComputerHostError::ObserveNotGranted);
        }
        let captured = self.broker.capture(window_handle).await?;
        if let Err(error) = validate_target(&captured.target) {
            self.broker.release_frame(&captured.image_token);
            return Err(error);
        }
        let now = self.clock.now_ms();
        let mut state = self.state.lock();
        let Some(rule) = state
            .rules
            .get(&(session_id.clone(), captured.target.app_id.clone()))
            .filter(|rule| rule.observe)
        else {
            drop(state);
            self.broker.release_frame(&captured.image_token);
            return Err(ComputerHostError::AppNotAllowed);
        };
        if !grants.computer.target_windows.is_empty()
            && !grants
                .computer
                .target_windows
                .iter()
                .any(|target| target == &captured.target.app_id)
        {
            drop(state);
            self.broker.release_frame(&captured.image_token);
            return Err(ComputerHostError::AppNotAllowed);
        }
        let _always_allowed = rule.always_allowed;
        let input_epoch = *state.input_epochs.entry(session_id.clone()).or_insert(1);
        let frame = ComputerFrame {
            id: ComputerFrameId::random(),
            session_id,
            run_id,
            run_generation,
            target: captured.target,
            image_token: captured.image_token,
            width: captured.width,
            height: captured.height,
            input_epoch,
            created_at_ms: now,
            expires_at_ms: now.saturating_add(FRAME_TTL_MS),
        };
        let stale_tokens = state
            .frames
            .values()
            .filter(|value| value.frame.expires_at_ms <= now)
            .map(|value| value.frame.image_token.clone())
            .collect::<Vec<_>>();
        state
            .frames
            .retain(|_, value| value.frame.expires_at_ms > now);
        state.frames.insert(
            frame.id.clone(),
            FrameState {
                frame: frame.clone(),
                window_handle: window_handle.to_owned(),
                user_input_marker: self.broker.user_input_marker(),
            },
        );
        drop(state);
        for token in stale_tokens {
            self.broker.release_frame(&token);
        }
        Ok(frame)
    }

    pub async fn act(
        &self,
        request: &ComputerActionRequest,
        grants: &CapabilityGrantSet,
    ) -> Result<ComputerActionResult, ComputerHostError> {
        validate_action(&request.action)?;
        if !grants.computer.act {
            return Err(ComputerHostError::ActNotGranted);
        }
        let frame_state = {
            let state = self.state.lock();
            state
                .frames
                .get(&request.frame_id)
                .cloned()
                .ok_or(ComputerHostError::FrameNotFound)?
        };
        if frame_state.frame.expires_at_ms <= self.clock.now_ms()
            || frame_state.frame.input_epoch != request.expected_input_epoch
            || grants.session_id != frame_state.frame.session_id
            || grants.run_id.as_ref() != Some(&frame_state.frame.run_id)
        {
            if frame_state.frame.expires_at_ms <= self.clock.now_ms() {
                self.take_over(&frame_state.frame.session_id);
            }
            return Err(ComputerHostError::StaleFrame);
        }
        if frame_state.frame.run_generation != request.run_generation {
            return Err(ComputerHostError::StaleRunGeneration);
        }
        if frame_state.frame.target.fingerprint != request.target_fingerprint {
            return Err(ComputerHostError::TargetChanged);
        }
        if frame_state.user_input_marker.is_some()
            && frame_state.user_input_marker != self.broker.user_input_marker()
        {
            self.take_over(&frame_state.frame.session_id);
            return Err(ComputerHostError::UserTakeover);
        }
        let rule = self
            .state
            .lock()
            .rules
            .get(&(
                frame_state.frame.session_id.clone(),
                frame_state.frame.target.app_id.clone(),
            ))
            .cloned()
            .filter(|rule| rule.act)
            .ok_or(ComputerHostError::AppNotAllowed)?;
        if let ComputerAction::LaunchApp { app_id } = &request.action {
            let launch_allowed = self
                .state
                .lock()
                .rules
                .get(&(frame_state.frame.session_id.clone(), app_id.clone()))
                .is_some_and(|rule| rule.act)
                && (grants.computer.target_windows.is_empty()
                    || grants
                        .computer
                        .target_windows
                        .iter()
                        .any(|value| value == app_id));
            if !launch_allowed {
                return Err(ComputerHostError::AppNotAllowed);
            }
        }
        if grants.max_actions_reached(
            self.state
                .lock()
                .action_counts
                .get(&frame_state.frame.run_id)
                .copied()
                .unwrap_or_default(),
        ) {
            return Err(ComputerHostError::ActionLimitReached);
        }
        let current = self
            .broker
            .current_identity(&frame_state.window_handle)
            .await?;
        validate_target(&current)?;
        if current.fingerprint != frame_state.frame.target.fingerprint
            || current.process_id != frame_state.frame.target.process_id
            || current.app_id != frame_state.frame.target.app_id
        {
            self.take_over(&frame_state.frame.session_id);
            return Err(ComputerHostError::TargetChanged);
        }
        let _always_allowed = rule.always_allowed;
        self.broker.perform(&current, &request.action).await?;
        *self
            .state
            .lock()
            .action_counts
            .entry(frame_state.frame.run_id.clone())
            .or_default() += 1;
        let next_epoch = self.take_over(&frame_state.frame.session_id);
        Ok(ComputerActionResult {
            frame_id: request.frame_id.clone(),
            accepted: true,
            result_code: "performed".into(),
            next_input_epoch: next_epoch,
        })
    }

    pub async fn frame_image(
        &self,
        frame_id: &ComputerFrameId,
        grants: &CapabilityGrantSet,
    ) -> Result<ComputerFrameImage, ComputerHostError> {
        let frame = self
            .state
            .lock()
            .frames
            .get(frame_id)
            .map(|state| state.frame.clone())
            .ok_or(ComputerHostError::FrameNotFound)?;
        if frame.expires_at_ms <= self.clock.now_ms()
            || grants.session_id != frame.session_id
            || grants.run_id.as_ref() != Some(&frame.run_id)
            || !grants.computer.observe
        {
            return Err(ComputerHostError::StaleFrame);
        }
        let bytes = match self.broker.read_frame(&frame.image_token).await {
            Ok(bytes) => bytes,
            Err(error) => {
                self.broker.release_frame(&frame.image_token);
                return Err(error);
            }
        };
        self.broker.release_frame(&frame.image_token);
        if bytes.is_empty() || bytes.len() > MAX_FRAME_IMAGE_BYTES {
            return Err(ComputerHostError::Broker(
                "computer_frame_image_size_invalid".into(),
            ));
        }
        let sha256 = sha2::Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok(ComputerFrameImage {
            media_type: "image/png".into(),
            bytes,
            sha256,
        })
    }

    #[must_use]
    pub fn frame_snapshot(&self, frame_id: &ComputerFrameId) -> Option<ComputerFrame> {
        self.state
            .lock()
            .frames
            .get(frame_id)
            .map(|state| state.frame.clone())
    }

    pub fn take_over(&self, session_id: &SessionId) -> u64 {
        let mut state = self.state.lock();
        let next = state
            .input_epochs
            .get(session_id)
            .copied()
            .unwrap_or(0)
            .saturating_add(1);
        state.input_epochs.insert(session_id.clone(), next);
        let released = state
            .frames
            .values()
            .filter(|value| &value.frame.session_id == session_id)
            .map(|value| value.frame.image_token.clone())
            .collect::<Vec<_>>();
        state
            .frames
            .retain(|_, value| &value.frame.session_id != session_id);
        drop(state);
        for token in released {
            self.broker.release_frame(&token);
        }
        next
    }
}

trait ComputerGrantLimit {
    fn max_actions_reached(&self, completed: u32) -> bool;
}

impl ComputerGrantLimit for CapabilityGrantSet {
    fn max_actions_reached(&self, completed: u32) -> bool {
        self.computer
            .max_actions
            .is_some_and(|limit| completed >= limit)
    }
}

fn require_sandbox(report: &SandboxCapabilityReport) -> Result<(), ComputerHostError> {
    (report.readiness == SandboxReadiness::Ready
        && report.os_enforced
        && report.filesystem_enforced
        && report.process_enforced
        && report.network_enforced)
        .then_some(())
        .ok_or(ComputerHostError::SandboxNotReady)
}

fn validate_target(target: &ComputerWindowIdentity) -> Result<(), ComputerHostError> {
    if target.elevated || target.protected_desktop {
        return Err(ComputerHostError::ProtectedTarget);
    }
    if target.hachimi_owned {
        return Err(ComputerHostError::SelfTarget);
    }
    if target.app_id.trim().is_empty()
        || target.fingerprint.trim().is_empty()
        || target.window_handle.trim().is_empty()
    {
        return Err(ComputerHostError::ProtectedTarget);
    }
    Ok(())
}

fn validate_action(action: &ComputerAction) -> Result<(), ComputerHostError> {
    let valid_button = |button: &str| matches!(button, "left" | "right" | "middle");
    let valid_key = |key: &str| !key.trim().is_empty() && key.chars().count() <= 64;
    let valid = match action {
        ComputerAction::MouseMove { x, y } => *x >= 0 && *y >= 0,
        ComputerAction::MouseClick { x, y, button } => *x >= 0 && *y >= 0 && valid_button(button),
        ComputerAction::MouseDown { x, y, button }
        | ComputerAction::MouseUp { x, y, button }
        | ComputerAction::MouseDoubleClick { x, y, button } => {
            *x >= 0 && *y >= 0 && valid_button(button)
        }
        ComputerAction::MouseDrag {
            from_x,
            from_y,
            to_x,
            to_y,
            button,
        } => {
            *from_x >= 0
                && *from_y >= 0
                && *to_x >= 0
                && *to_y >= 0
                && valid_button(button)
                && (from_x != to_x || from_y != to_y)
        }
        ComputerAction::Scroll { delta_x, delta_y } => *delta_x != 0 || *delta_y != 0,
        ComputerAction::KeyPress { key, modifiers } => {
            valid_key(key)
                && modifiers.len() <= 3
                && modifiers.iter().all(|value| {
                    matches!(
                        value.to_ascii_lowercase().as_str(),
                        "ctrl" | "control" | "alt" | "shift"
                    )
                })
        }
        ComputerAction::KeyDown { key } | ComputerAction::KeyUp { key } => valid_key(key),
        ComputerAction::KeyChord { keys } => {
            (2..=5).contains(&keys.len()) && keys.iter().all(|key| valid_key(key))
        }
        ComputerAction::TypeText { text } => !text.is_empty() && text.chars().count() <= 8_000,
        ComputerAction::WindowFocus
        | ComputerAction::WindowMinimize
        | ComputerAction::WindowMaximize
        | ComputerAction::WindowRestore
        | ComputerAction::WindowClose => true,
        ComputerAction::WindowMove { x, y } => {
            (-32_768..=32_768).contains(x) && (-32_768..=32_768).contains(y)
        }
        ComputerAction::WindowResize { width, height } => {
            (1..=16_384).contains(width) && (1..=16_384).contains(height)
        }
        ComputerAction::LaunchApp { app_id } => {
            app_id.len() <= 128
                && app_id.to_ascii_lowercase().ends_with(".exe")
                && app_id
                    .chars()
                    .all(|value| value.is_ascii_alphanumeric() || matches!(value, '.' | '_' | '-'))
        }
    };
    valid.then_some(()).ok_or(ComputerHostError::InvalidAction)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hachimi_protocol::{
        ComputerGrant, FileSystemAccess, FileSystemGrant, NetworkGrant, PermissionGrantScope,
        PermissionProfile, ProcessGrant,
    };

    #[derive(Debug)]
    struct TestBroker {
        target: ComputerWindowIdentity,
        released: Option<Arc<Mutex<Vec<String>>>>,
    }

    impl ComputerBroker for TestBroker {
        fn capture<'a>(
            &'a self,
            _window_handle: &'a str,
        ) -> ComputerBrokerFuture<'a, CapturedWindow> {
            let target = self.target.clone();
            Box::pin(async move {
                Ok(CapturedWindow {
                    target,
                    image_token: "ephemeral:image".into(),
                    width: 800,
                    height: 600,
                })
            })
        }

        fn current_identity<'a>(
            &'a self,
            _window_handle: &'a str,
        ) -> ComputerBrokerFuture<'a, ComputerWindowIdentity> {
            let target = self.target.clone();
            Box::pin(async move { Ok(target) })
        }

        fn perform<'a>(
            &'a self,
            _target: &'a ComputerWindowIdentity,
            _action: &'a ComputerAction,
        ) -> ComputerBrokerFuture<'a, ()> {
            Box::pin(async move { Ok(()) })
        }

        fn read_frame<'a>(&'a self, _image_token: &'a str) -> ComputerBrokerFuture<'a, Vec<u8>> {
            Box::pin(async move { Ok(vec![137, 80, 78, 71, 13, 10, 26, 10]) })
        }

        fn release_frame(&self, image_token: &str) {
            if let Some(released) = &self.released {
                released.lock().push(image_token.to_owned());
            }
        }
    }

    fn target() -> ComputerWindowIdentity {
        ComputerWindowIdentity {
            app_id: "notepad.exe".into(),
            process_id: 42,
            window_handle: "0x1234".into(),
            fingerprint: "window-fingerprint".into(),
            title: "Notes".into(),
            elevated: false,
            protected_desktop: false,
            hachimi_owned: false,
        }
    }

    fn grants(session_id: &SessionId, run_id: &RunId) -> CapabilityGrantSet {
        CapabilityGrantSet {
            profile: PermissionProfile::ExternalSandbox,
            scope: PermissionGrantScope::Run,
            session_id: session_id.clone(),
            run_id: Some(run_id.clone()),
            source: "test".into(),
            file_system: vec![FileSystemGrant {
                access: FileSystemAccess::Read,
                roots: vec!["C:\\workspace".into()],
                globs: Vec::new(),
                special_roots: Vec::new(),
            }],
            network: NetworkGrant::default(),
            process: ProcessGrant::default(),
            browser: Default::default(),
            computer: ComputerGrant {
                observe: true,
                act: true,
                target_windows: vec!["notepad.exe".into()],
                max_actions: Some(2),
            },
            review_each_command: true,
            expires_at_ms: None,
        }
    }

    fn sandbox() -> SandboxCapabilityReport {
        SandboxCapabilityReport {
            backend: "test".into(),
            readiness: SandboxReadiness::Ready,
            os_enforced: true,
            filesystem_enforced: true,
            process_enforced: true,
            network_enforced: true,
            version: Some("test".into()),
            stable_error_code: None,
            diagnostics: Vec::new(),
        }
    }

    #[tokio::test]
    async fn every_action_consumes_the_frame_and_input_epoch() {
        let host = ComputerHost::new(
            Arc::new(TestBroker {
                target: target(),
                released: None,
            }),
            Arc::new(SystemComputerClock),
        );
        let session_id = SessionId::from("session");
        let run_id = RunId::from("run");
        host.set_app_rule(
            &session_id,
            ComputerAppRule {
                app_id: "notepad.exe".into(),
                observe: true,
                act: true,
                always_allowed: false,
                granted_by: "user:test".into(),
                updated_at_ms: 1,
            },
        );
        let active_grants = grants(&session_id, &run_id);
        let frame = host
            .observe(session_id, run_id, 5, "0x1234", &active_grants, &sandbox())
            .await
            .expect("frame");
        let request = ComputerActionRequest {
            frame_id: frame.id,
            run_generation: 5,
            target_fingerprint: frame.target.fingerprint,
            expected_input_epoch: frame.input_epoch,
            action: ComputerAction::TypeText {
                text: "hello".into(),
            },
        };
        let stale_request = ComputerActionRequest {
            run_generation: 4,
            ..request.clone()
        };
        assert_eq!(
            host.act(&stale_request, &active_grants).await,
            Err(ComputerHostError::StaleRunGeneration)
        );
        assert!(host.act(&request, &active_grants).await.is_ok());
        assert_eq!(
            host.act(&request, &active_grants).await,
            Err(ComputerHostError::FrameNotFound)
        );
    }

    #[tokio::test]
    async fn frame_bytes_are_ephemeral_and_bound_to_the_active_run() {
        let host = ComputerHost::new(
            Arc::new(TestBroker {
                target: target(),
                released: None,
            }),
            Arc::new(SystemComputerClock),
        );
        let session_id = SessionId::from("session");
        let run_id = RunId::from("run");
        host.set_app_rule(
            &session_id,
            ComputerAppRule {
                app_id: "notepad.exe".into(),
                observe: true,
                act: true,
                always_allowed: false,
                granted_by: "user:test".into(),
                updated_at_ms: 1,
            },
        );
        let active_grants = grants(&session_id, &run_id);
        let frame = host
            .observe(
                session_id.clone(),
                run_id.clone(),
                6,
                "0x1234",
                &active_grants,
                &sandbox(),
            )
            .await
            .expect("frame");
        let image = host
            .frame_image(&frame.id, &active_grants)
            .await
            .expect("ephemeral image");
        assert_eq!(image.media_type, "image/png");
        assert_eq!(image.bytes, vec![137, 80, 78, 71, 13, 10, 26, 10]);

        let other_run = RunId::from("other-run");
        assert_eq!(
            host.frame_image(&frame.id, &grants(&session_id, &other_run))
                .await,
            Err(ComputerHostError::StaleFrame)
        );
    }

    #[tokio::test]
    async fn frame_backing_storage_is_released_after_model_consumption() {
        let released = Arc::new(Mutex::new(Vec::new()));
        let host = ComputerHost::new(
            Arc::new(TestBroker {
                target: target(),
                released: Some(Arc::clone(&released)),
            }),
            Arc::new(SystemComputerClock),
        );
        let session_id = SessionId::from("session");
        let run_id = RunId::from("run");
        host.set_app_rule(
            &session_id,
            ComputerAppRule {
                app_id: "notepad.exe".into(),
                observe: true,
                act: true,
                always_allowed: false,
                granted_by: "user:test".into(),
                updated_at_ms: 1,
            },
        );
        let active_grants = grants(&session_id, &run_id);
        let frame = host
            .observe(session_id, run_id, 7, "0x1234", &active_grants, &sandbox())
            .await
            .expect("frame");
        host.frame_image(&frame.id, &active_grants)
            .await
            .expect("image");
        assert_eq!(released.lock().as_slice(), ["ephemeral:image"]);
    }

    #[tokio::test]
    async fn elevated_and_hachimi_windows_are_rejected() {
        for target in [
            ComputerWindowIdentity {
                elevated: true,
                ..target()
            },
            ComputerWindowIdentity {
                protected_desktop: true,
                ..target()
            },
            ComputerWindowIdentity {
                hachimi_owned: true,
                ..target()
            },
        ] {
            let host = ComputerHost::new(
                Arc::new(TestBroker {
                    target,
                    released: None,
                }),
                Arc::new(SystemComputerClock),
            );
            let session_id = SessionId::from("session");
            let run_id = RunId::from("run");
            host.set_app_rule(
                &session_id,
                ComputerAppRule {
                    app_id: "notepad.exe".into(),
                    observe: true,
                    act: true,
                    always_allowed: false,
                    granted_by: "user:test".into(),
                    updated_at_ms: 1,
                },
            );
            assert!(
                host.observe(
                    session_id.clone(),
                    run_id.clone(),
                    8,
                    "0x1234",
                    &grants(&session_id, &run_id),
                    &sandbox(),
                )
                .await
                .is_err()
            );
        }
    }

    #[test]
    fn controlled_app_launch_accepts_only_a_simple_executable_basename() {
        assert!(
            validate_action(&ComputerAction::LaunchApp {
                app_id: "notepad.exe".into(),
            })
            .is_ok()
        );
        for app_id in [
            "C:\\Windows\\notepad.exe",
            "..\\notepad.exe",
            "notepad.exe --argument",
            "notepad.cmd",
            "/usr/bin/app.exe",
        ] {
            assert_eq!(
                validate_action(&ComputerAction::LaunchApp {
                    app_id: app_id.into(),
                }),
                Err(ComputerHostError::InvalidAction),
                "{app_id}"
            );
        }
    }
}
