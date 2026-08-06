use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Once},
    time::{Duration, Instant},
};

use hachimi_protocol::{
    ComputerAction, ComputerAppDescriptor, ComputerRuntimeHealth, ComputerWindowIdentity,
};
use parking_lot::Mutex;

use crate::{
    CapturedWindow, ComputerBroker, ComputerBrokerFuture, ComputerHostError, MAX_FRAME_IMAGE_BYTES,
};

#[cfg(windows)]
mod windows;

const FRAME_STORE_TTL: Duration = Duration::from_secs(30);
const MAX_STORED_FRAMES: usize = 8;
const MAX_STORED_FRAME_BYTES: usize = 64 * 1024 * 1024;
static LEGACY_FRAME_CLEANUP: Once = Once::new();

#[must_use]
pub fn computer_runtime_health() -> ComputerRuntimeHealth {
    #[cfg(windows)]
    {
        windows::runtime_health()
    }
    #[cfg(not(windows))]
    {
        ComputerRuntimeHealth {
            os_supported: false,
            graphics_capture_available: false,
            input_desktop_available: false,
            process_elevated: false,
            error_code: Some("computer_unsupported_os".into()),
        }
    }
}

#[derive(Debug, Clone)]
struct StoredFrame {
    bytes: Arc<[u8]>,
    stored_at: Instant,
}

#[derive(Debug, Default)]
struct FrameStore {
    frames: BTreeMap<String, StoredFrame>,
    total_bytes: usize,
}

impl FrameStore {
    fn insert(
        &mut self,
        token: String,
        bytes: Vec<u8>,
        now: Instant,
    ) -> Result<(), ComputerHostError> {
        if bytes.is_empty() || bytes.len() > MAX_FRAME_IMAGE_BYTES {
            return Err(ComputerHostError::Broker(
                "computer_frame_image_size_invalid".into(),
            ));
        }
        self.prune_expired(now);
        while self.frames.len() >= MAX_STORED_FRAMES
            || self.total_bytes.saturating_add(bytes.len()) > MAX_STORED_FRAME_BYTES
        {
            let Some(oldest) = self
                .frames
                .iter()
                .min_by_key(|(_, frame)| frame.stored_at)
                .map(|(token, _)| token.clone())
            else {
                break;
            };
            self.remove(&oldest);
        }
        self.total_bytes = self.total_bytes.saturating_add(bytes.len());
        self.frames.insert(
            token,
            StoredFrame {
                bytes: bytes.into(),
                stored_at: now,
            },
        );
        Ok(())
    }

    fn read(&mut self, token: &str, now: Instant) -> Result<Vec<u8>, ComputerHostError> {
        self.prune_expired(now);
        self.frames
            .get(token)
            .map(|frame| frame.bytes.as_ref().to_vec())
            .ok_or(ComputerHostError::FrameNotFound)
    }

    fn remove(&mut self, token: &str) {
        if let Some(frame) = self.frames.remove(token) {
            self.total_bytes = self.total_bytes.saturating_sub(frame.bytes.len());
        }
    }

    fn prune_expired(&mut self, now: Instant) {
        let expired = self
            .frames
            .iter()
            .filter(|(_, frame)| now.saturating_duration_since(frame.stored_at) >= FRAME_STORE_TTL)
            .map(|(token, _)| token.clone())
            .collect::<Vec<_>>();
        for token in expired {
            self.remove(&token);
        }
    }
}

#[derive(Debug)]
pub struct PlatformComputerBroker {
    frame_tokens: Arc<Mutex<FrameStore>>,
}

impl PlatformComputerBroker {
    #[must_use]
    pub fn new() -> Self {
        LEGACY_FRAME_CLEANUP.call_once(|| {
            cleanup_legacy_frame_dir(&std::env::temp_dir().join("hachimi-computer-frames"));
        });
        Self {
            frame_tokens: Arc::new(Mutex::new(FrameStore::default())),
        }
    }
}

impl Default for PlatformComputerBroker {
    fn default() -> Self {
        Self::new()
    }
}

impl ComputerBroker for PlatformComputerBroker {
    fn list_windows<'a>(&'a self) -> ComputerBrokerFuture<'a, Vec<ComputerWindowIdentity>> {
        Box::pin(async move {
            tokio::task::spawn_blocking(list_windows)
                .await
                .map_err(|error| ComputerHostError::Broker(error.to_string()))?
        })
    }

    fn capture<'a>(&'a self, window_handle: &'a str) -> ComputerBrokerFuture<'a, CapturedWindow> {
        let window_handle = window_handle.to_owned();
        let frame_tokens = Arc::clone(&self.frame_tokens);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || capture_window(&window_handle, &frame_tokens))
                .await
                .map_err(|error| ComputerHostError::Broker(error.to_string()))?
        })
    }

    fn app_icon_png<'a>(
        &'a self,
        app: &'a ComputerAppDescriptor,
    ) -> ComputerBrokerFuture<'a, Option<Vec<u8>>> {
        let descriptor = app.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || app_icon_png(&descriptor))
                .await
                .map_err(|error| ComputerHostError::Broker(error.to_string()))?
        })
    }

    fn current_identity<'a>(
        &'a self,
        window_handle: &'a str,
    ) -> ComputerBrokerFuture<'a, ComputerWindowIdentity> {
        let window_handle = window_handle.to_owned();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || read_identity(&window_handle))
                .await
                .map_err(|error| ComputerHostError::Broker(error.to_string()))?
        })
    }

    fn perform<'a>(
        &'a self,
        target: &'a ComputerWindowIdentity,
        action: &'a ComputerAction,
    ) -> ComputerBrokerFuture<'a, ()> {
        let target = target.clone();
        let action = action.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || perform_action(&target, &action))
                .await
                .map_err(|error| ComputerHostError::Broker(error.to_string()))?
        })
    }

    fn read_frame<'a>(&'a self, image_token: &'a str) -> ComputerBrokerFuture<'a, Vec<u8>> {
        let image_token = image_token.to_owned();
        let frame_tokens = Arc::clone(&self.frame_tokens);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                frame_tokens.lock().read(&image_token, Instant::now())
            })
            .await
            .map_err(|error| ComputerHostError::Broker(error.to_string()))?
        })
    }

    fn release_frame(&self, image_token: &str) {
        self.frame_tokens.lock().remove(image_token);
    }

    fn user_input_marker(&self) -> Option<u64> {
        platform_user_input_marker()
    }
}

#[cfg(windows)]
fn platform_user_input_marker() -> Option<u64> {
    use ::windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

    let mut input = LASTINPUTINFO {
        cbSize: u32::try_from(std::mem::size_of::<LASTINPUTINFO>()).ok()?,
        dwTime: 0,
    };
    // SAFETY: the structure has the documented size and remains valid for the call.
    (unsafe { GetLastInputInfo(&mut input) }.as_bool()).then_some(u64::from(input.dwTime))
}

#[cfg(not(windows))]
fn platform_user_input_marker() -> Option<u64> {
    None
}

#[cfg(windows)]
fn list_windows() -> Result<Vec<ComputerWindowIdentity>, ComputerHostError> {
    windows::list_windows()
}

#[cfg(windows)]
fn app_icon_png(app: &ComputerAppDescriptor) -> Result<Option<Vec<u8>>, ComputerHostError> {
    windows::app_icon_png(app)
}

#[cfg(not(windows))]
fn app_icon_png(_app: &ComputerAppDescriptor) -> Result<Option<Vec<u8>>, ComputerHostError> {
    Ok(None)
}

#[cfg(not(windows))]
fn list_windows() -> Result<Vec<ComputerWindowIdentity>, ComputerHostError> {
    Err(ComputerHostError::Broker(
        "the platform Computer broker is Windows-only".into(),
    ))
}

#[cfg(windows)]
fn read_identity(window_handle: &str) -> Result<ComputerWindowIdentity, ComputerHostError> {
    windows::read_identity(window_handle)
}

#[cfg(not(windows))]
fn read_identity(_window_handle: &str) -> Result<ComputerWindowIdentity, ComputerHostError> {
    Err(ComputerHostError::Broker(
        "the platform Computer broker is Windows-only".into(),
    ))
}

#[cfg(windows)]
fn capture_window(
    window_handle: &str,
    frame_tokens: &Mutex<FrameStore>,
) -> Result<CapturedWindow, ComputerHostError> {
    let identity = read_identity(window_handle)?;
    let captured = windows::capture_window(window_handle)?;
    let token = format!("computer-frame:{}", uuid::Uuid::new_v4());
    frame_tokens
        .lock()
        .insert(token.clone(), captured.png_bytes, Instant::now())?;
    Ok(CapturedWindow {
        target: identity,
        image_token: token,
        width: captured.width,
        height: captured.height,
    })
}

#[cfg(not(windows))]
fn capture_window(
    _window_handle: &str,
    _frame_tokens: &Mutex<FrameStore>,
) -> Result<CapturedWindow, ComputerHostError> {
    Err(ComputerHostError::Broker(
        "the platform Computer broker is Windows-only".into(),
    ))
}

#[cfg(windows)]
fn perform_action(
    target: &ComputerWindowIdentity,
    action: &ComputerAction,
) -> Result<(), ComputerHostError> {
    windows::perform_action(target, action)
}

#[cfg(not(windows))]
fn perform_action(
    _target: &ComputerWindowIdentity,
    _action: &ComputerAction,
) -> Result<(), ComputerHostError> {
    Err(ComputerHostError::Broker(
        "the platform Computer broker is Windows-only".into(),
    ))
}

fn cleanup_legacy_frame_dir(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_legacy_png = entry.file_type().is_ok_and(|kind| kind.is_file())
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
            && path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| uuid::Uuid::parse_str(stem).is_ok());
        if is_legacy_png {
            let _ = std::fs::remove_file(path);
        }
    }
    let _ = std::fs::remove_dir(root);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_frames_are_bounded_released_and_expired() {
        let now = Instant::now();
        let mut store = FrameStore::default();
        store.insert("first".into(), vec![1, 2, 3], now).unwrap();
        assert_eq!(store.read("first", now).unwrap(), [1, 2, 3]);
        store.remove("first");
        assert_eq!(
            store.read("first", now),
            Err(ComputerHostError::FrameNotFound)
        );

        for index in 0..=MAX_STORED_FRAMES {
            store
                .insert(format!("frame-{index}"), vec![index as u8], now)
                .unwrap();
        }
        assert_eq!(store.frames.len(), MAX_STORED_FRAMES);
        assert!(!store.frames.contains_key("frame-0"));

        store.prune_expired(now + FRAME_STORE_TTL);
        assert!(store.frames.is_empty());
        assert_eq!(store.total_bytes, 0);
    }

    #[test]
    fn legacy_cleanup_only_removes_uuid_png_files() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join(format!("{}.png", uuid::Uuid::new_v4()));
        let unrelated_png = root.path().join("keep.png");
        let unrelated_file = root.path().join(format!("{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&legacy, b"legacy").unwrap();
        std::fs::write(&unrelated_png, b"keep").unwrap();
        std::fs::write(&unrelated_file, b"keep").unwrap();

        cleanup_legacy_frame_dir(root.path());

        assert!(!legacy.exists());
        assert!(unrelated_png.exists());
        assert!(unrelated_file.exists());
    }
}
