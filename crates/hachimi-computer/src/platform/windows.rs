use std::{
    collections::HashMap,
    fs::File,
    io::Read,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, UNIX_EPOCH},
};

use hachimi_process_policy::ProcessPolicy;
use hachimi_protocol::{
    ComputerAction, ComputerAppDescriptor, ComputerRuntimeHealth, ComputerWindowIdentity,
};
use parking_lot::{Condvar, Mutex};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use windows_capture::{
    capture::{CaptureControl, Context, GraphicsCaptureApiHandler},
    frame::Frame,
    graphics_capture_api::InternalCaptureControl,
    settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    },
    window::Window,
};

use crate::ComputerHostError;

use ::windows::{
    Graphics::Capture::GraphicsCaptureSession,
    Win32::{
        Foundation::{HANDLE, HWND, LPARAM, PROPERTYKEY, RECT, WPARAM},
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
            DIB_RGB_COLORS, DeleteDC, DeleteObject, HGDIOBJ, SelectObject,
        },
        Security::{
            GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation,
            TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenIntegrityLevel,
        },
        Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW},
        System::{
            Com::StructuredStorage::PropVariantToString,
            StationsAndDesktops::{
                DESKTOP_CONTROL_FLAGS, DESKTOP_READOBJECTS, GetThreadDesktop,
                GetUserObjectInformationW, OpenInputDesktop, UOI_NAME,
            },
            SystemServices::SECURITY_MANDATORY_HIGH_RID,
            Threading::{
                GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
            },
        },
        UI::{
            Input::KeyboardAndMouse::{
                INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
                KEYEVENTF_UNICODE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
                MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTDOWN,
                MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput, VIRTUAL_KEY,
                VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_INSERT,
                VK_LEFT, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE,
                VK_TAB, VK_UP,
            },
            Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow},
            Shell::{SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGetFileInfoW},
            WindowsAndMessaging::{
                DI_NORMAL, DestroyIcon, DrawIconEx, EnumWindows, GetClassNameW,
                GetForegroundWindow, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId,
                IsWindow, IsWindowVisible, MoveWindow, PostMessageW, SW_MAXIMIZE, SW_MINIMIZE,
                SW_RESTORE, SetCursorPos, SetForegroundWindow, ShowWindow, WM_CLOSE,
            },
        },
    },
    core::{BOOL, GUID, Owned, PCWSTR, PWSTR},
};

const MAX_CAPTURE_DIMENSION: u32 = 16_384;
const PKEY_APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
    pid: 5,
};
static APP_DESCRIPTOR_CACHE: OnceLock<Mutex<HashMap<String, CachedAppDescriptor>>> =
    OnceLock::new();
static APP_ICON_CACHE: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();
static WINDOW_CAPTURE: OnceLock<Mutex<WindowCaptureCache>> = OnceLock::new();

pub(super) fn runtime_health() -> ComputerRuntimeHealth {
    let graphics_capture_available = GraphicsCaptureSession::IsSupported().unwrap_or(false);
    let input_desktop_available =
        input_desktop_name().is_ok_and(|name| name.eq_ignore_ascii_case("default"));
    // SAFETY: GetCurrentProcess returns a valid pseudo-handle which must not be closed.
    let process_elevated = process_integrity(unsafe { GetCurrentProcess() })
        .is_ok_and(|level| level >= SECURITY_MANDATORY_HIGH_RID as u32);
    let error_code = if !graphics_capture_available {
        Some("computer_capture_unavailable".into())
    } else if !input_desktop_available {
        Some("computer_protected_desktop".into())
    } else {
        None
    };
    ComputerRuntimeHealth {
        os_supported: true,
        graphics_capture_available,
        input_desktop_available,
        process_elevated,
        error_code,
    }
}

#[derive(Clone)]
struct CachedAppDescriptor {
    len: u64,
    modified_ms: u128,
    descriptor: ComputerAppDescriptor,
}

pub(super) fn list_windows() -> Result<Vec<ComputerWindowIdentity>, ComputerHostError> {
    unsafe extern "system" fn collect(hwnd: HWND, parameter: LPARAM) -> BOOL {
        // SAFETY: parameter is a live Vec<HWND> pointer for the duration of
        // EnumWindows and this callback runs synchronously on the same thread.
        let handles = unsafe { &mut *(parameter.0 as *mut Vec<HWND>) };
        // SAFETY: EnumWindows supplies a valid HWND value for inspection.
        if unsafe { IsWindowVisible(hwnd) }.as_bool() && handles.len() < 256 {
            handles.push(hwnd);
        }
        true.into()
    }

    let mut handles: Vec<HWND> = Vec::new();
    // SAFETY: collect does not retain the pointer and EnumWindows is
    // synchronous. The vector remains alive until the call returns.
    unsafe {
        EnumWindows(
            Some(collect),
            LPARAM((&mut handles as *mut Vec<HWND>).cast::<()>() as isize),
        )
    }
    .map_err(|error| broker(format!("enum_windows:{error}")))?;
    Ok(handles
        .into_iter()
        .filter_map(|hwnd| read_identity(&format!("0x{:x}", hwnd.0 as usize)).ok())
        .filter(|identity| !identity.title.trim().is_empty())
        .filter(|identity| is_user_application(&identity.app_id))
        .collect())
}

pub(super) fn foreground_window() -> Result<ComputerWindowIdentity, ComputerHostError> {
    // SAFETY: GetForegroundWindow returns a borrowed handle or null.
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return Err(broker("foreground_window_missing"));
    }
    read_identity(&format!("0x{:x}", hwnd.0 as usize))
}

pub(super) fn read_identity(
    window_handle: &str,
) -> Result<ComputerWindowIdentity, ComputerHostError> {
    let handle = parse_handle(window_handle)?;
    let hwnd = HWND(handle as *mut _);
    // SAFETY: HWND is an opaque value and IsWindow performs the validity check.
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return Err(broker("window_not_found"));
    }

    let mut process_id = 0_u32;
    // SAFETY: process_id remains valid for the call and hwnd was validated above.
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
    if thread_id == 0 || process_id == 0 {
        return Err(broker("window_process_not_found"));
    }
    // SAFETY: the returned HANDLE is wrapped in Owned immediately.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }
        .map_err(|error| broker(format!("open_process:{error}")))?;
    // SAFETY: process is a newly owned kernel handle.
    let process = unsafe { Owned::new(process) };
    let executable_path = process_image_path(*process)?;
    let app = app_descriptor(hwnd, &executable_path)?;
    let app_id = app.app_id.clone();
    let title = window_text(hwnd);
    let class_name = window_class(hwnd);
    let rect = window_rect(hwnd)?;
    let target_integrity = process_integrity(*process).unwrap_or(u32::MAX);
    // SAFETY: GetCurrentProcess returns a valid pseudo-handle which must not be closed.
    let current_integrity = process_integrity(unsafe { GetCurrentProcess() }).unwrap_or(0);
    let target_desktop = thread_desktop_name(thread_id).unwrap_or_else(|_| "protected".into());
    let input_desktop = input_desktop_name().unwrap_or_else(|_| "protected".into());
    let protected_desktop = !target_desktop.eq_ignore_ascii_case("default")
        || !input_desktop.eq_ignore_ascii_case("default")
        || !target_desktop.eq_ignore_ascii_case(&input_desktop);
    let elevated = target_integrity >= SECURITY_MANDATORY_HIGH_RID as u32
        || target_integrity > current_integrity;
    let hachimi_owned = process_id == std::process::id() || app_id.starts_with("hachimi");
    let fingerprint = fingerprint(&(
        handle,
        process_id,
        &app_id,
        &title,
        &class_name,
        [rect.left, rect.top, rect.right, rect.bottom],
        target_integrity,
        &target_desktop,
    ));

    Ok(ComputerWindowIdentity {
        app_id,
        app,
        process_id,
        window_handle: format!("0x{handle:x}"),
        fingerprint,
        title,
        elevated,
        protected_desktop,
        hachimi_owned,
    })
}

pub(super) struct CapturedImage {
    pub width: u32,
    pub height: u32,
    pub png_bytes: Vec<u8>,
}

#[derive(Default)]
struct WindowCaptureCache {
    active: Option<ActiveWindowCapture>,
}

struct ActiveWindowCapture {
    window_handle: isize,
    requested: Arc<AtomicBool>,
    state: Arc<Mutex<FrameState>>,
    wake: Arc<Condvar>,
    last_generation: u64,
    control: Option<CaptureControl<StreamingCapture, String>>,
}

impl ActiveWindowCapture {
    fn finished(&self) -> bool {
        self.control
            .as_ref()
            .is_none_or(CaptureControl::is_finished)
    }

    fn stop(mut self) {
        let Some(control) = self.control.take() else {
            return;
        };
        if control.is_finished() {
            let _ = control.wait();
        } else {
            let _ = control.stop();
        }
    }
}

pub(super) fn capture_window(window_handle: &str) -> Result<CapturedImage, ComputerHostError> {
    let handle = parse_handle(window_handle)?;
    let hwnd = HWND(handle as *mut _);
    let window = Window::from_raw_hwnd(hwnd.0);
    if !window.is_valid() {
        return Err(broker("window_not_capturable"));
    }
    let width = u32::try_from(window.width().map_err(|error| broker(error.to_string()))?)
        .map_err(|_| broker("window_width_invalid"))?;
    let height = u32::try_from(window.height().map_err(|error| broker(error.to_string()))?)
        .map_err(|_| broker("window_height_invalid"))?;
    validate_dimensions(width, height)?;

    let mut cache = WINDOW_CAPTURE
        .get_or_init(|| Mutex::new(WindowCaptureCache::default()))
        .lock();
    let replace = cache
        .active
        .as_ref()
        .is_none_or(|active| active.window_handle != handle || active.finished());
    if replace {
        if let Some(active) = cache.active.take() {
            active.stop();
        }
        cache.active = Some(start_window_capture(handle, window)?);
    }
    let active = cache.active.as_mut().expect("capture initialized");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut state = active.state.lock();
    if state.generation <= active.last_generation {
        active.requested.store(true, Ordering::Release);
    }
    while state.generation <= active.last_generation {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(broker(
                "windows_graphics_capture_no_frame:timed out waiting for frame",
            ));
        }
        active.wake.wait_for(&mut state, remaining);
    }
    active.last_generation = state.generation;
    let captured = state
        .image
        .take()
        .ok_or_else(|| broker("windows_graphics_capture_no_frame:frame payload missing"))?;
    validate_dimensions(captured.width, captured.height)?;
    Ok(captured)
}

fn start_window_capture(
    window_handle: isize,
    window: Window,
) -> Result<ActiveWindowCapture, ComputerHostError> {
    let requested = Arc::new(AtomicBool::new(false));
    let state = Arc::new(Mutex::new(FrameState::default()));
    let wake = Arc::new(Condvar::new());
    let settings = Settings::new(
        window,
        CursorCaptureSettings::WithoutCursor,
        DrawBorderSettings::Default,
        SecondaryWindowSettings::Exclude,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Rgba8,
        CaptureFlags {
            requested: Arc::clone(&requested),
            state: Arc::clone(&state),
            wake: Arc::clone(&wake),
        },
    );
    let control = StreamingCapture::start_free_threaded(settings)
        .map_err(|error| broker(format!("windows_graphics_capture:{error}")))?;
    Ok(ActiveWindowCapture {
        window_handle,
        requested,
        state,
        wake,
        last_generation: 0,
        control: Some(control),
    })
}

pub(super) fn perform_action(
    target: &ComputerWindowIdentity,
    action: &ComputerAction,
) -> Result<(), ComputerHostError> {
    let current = read_identity(&target.window_handle)?;
    if current.fingerprint != target.fingerprint
        || current.process_id != target.process_id
        || current.elevated
        || current.protected_desktop
        || current.hachimi_owned
    {
        return Err(ComputerHostError::TargetChanged);
    }
    let hwnd = HWND(parse_handle(&target.window_handle)? as *mut _);
    // Never steal focus from the user. A frame is actionable only while its
    // window remains the foreground target; a user switch therefore fails
    // closed instead of bringing a background window to the front.
    // SAFETY: GetForegroundWindow has no preconditions.
    if unsafe { GetForegroundWindow() } != hwnd {
        return Err(ComputerHostError::UserTakeover);
    }
    let rect = window_rect(hwnd)?;
    arm_next_window_frame(hwnd.0 as isize);

    match action {
        ComputerAction::MouseMove { x, y } => move_pointer(rect, *x, *y),
        ComputerAction::MouseClick { x, y, button } => {
            move_pointer(rect, *x, *y)?;
            let (down, up) = mouse_button_flags(button)?;
            send_inputs(&[mouse_input(down, 0), mouse_input(up, 0)])
        }
        ComputerAction::MouseDown { x, y, button } => {
            move_pointer(rect, *x, *y)?;
            let (down, _) = mouse_button_flags(button)?;
            send_inputs(&[mouse_input(down, 0)])
        }
        ComputerAction::MouseUp { x, y, button } => {
            move_pointer(rect, *x, *y)?;
            let (_, up) = mouse_button_flags(button)?;
            send_inputs(&[mouse_input(up, 0)])
        }
        ComputerAction::MouseDoubleClick { x, y, button } => {
            move_pointer(rect, *x, *y)?;
            let (down, up) = mouse_button_flags(button)?;
            send_inputs(&[
                mouse_input(down, 0),
                mouse_input(up, 0),
                mouse_input(down, 0),
                mouse_input(up, 0),
            ])
        }
        ComputerAction::MouseDrag {
            from_x,
            from_y,
            to_x,
            to_y,
            button,
        } => {
            move_pointer(rect, *from_x, *from_y)?;
            let (down, up) = mouse_button_flags(button)?;
            send_inputs(&[mouse_input(down, 0)])?;
            move_pointer(rect, *to_x, *to_y)?;
            send_inputs(&[mouse_input(up, 0)])
        }
        ComputerAction::Scroll { delta_x, delta_y } => {
            move_pointer(
                rect,
                (rect.right - rect.left) / 2,
                (rect.bottom - rect.top) / 2,
            )?;
            let mut inputs = Vec::with_capacity(2);
            if *delta_x != 0 {
                inputs.push(mouse_input(MOUSEEVENTF_HWHEEL, *delta_x as u32));
            }
            if *delta_y != 0 {
                inputs.push(mouse_input(MOUSEEVENTF_WHEEL, *delta_y as u32));
            }
            send_inputs(&inputs)
        }
        ComputerAction::KeyPress { key, modifiers } => send_key(key, modifiers),
        ComputerAction::KeyDown { key } => send_key_transition(key, false),
        ComputerAction::KeyUp { key } => send_key_transition(key, true),
        ComputerAction::KeyChord { keys } => send_chord(keys),
        ComputerAction::TypeText { text } => send_text(text),
        ComputerAction::WindowFocus => {
            // SAFETY: hwnd was revalidated immediately above.
            unsafe { SetForegroundWindow(hwnd) }
                .as_bool()
                .then_some(())
                .ok_or(ComputerHostError::UserTakeover)
        }
        ComputerAction::WindowMove { x, y } => {
            // SAFETY: hwnd and the current rectangle were validated above.
            unsafe {
                MoveWindow(
                    hwnd,
                    *x,
                    *y,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    true,
                )
            }
            .map_err(|error| broker(format!("move_window:{error}")))
        }
        ComputerAction::WindowResize { width, height } => {
            let width = i32::try_from(*width).map_err(|_| ComputerHostError::InvalidAction)?;
            let height = i32::try_from(*height).map_err(|_| ComputerHostError::InvalidAction)?;
            // SAFETY: hwnd and the current rectangle were validated above.
            unsafe { MoveWindow(hwnd, rect.left, rect.top, width, height, true) }
                .map_err(|error| broker(format!("resize_window:{error}")))
        }
        ComputerAction::WindowMinimize => {
            // SAFETY: hwnd was revalidated immediately above.
            let _ = unsafe { ShowWindow(hwnd, SW_MINIMIZE) };
            Ok(())
        }
        ComputerAction::WindowMaximize => {
            // SAFETY: hwnd was revalidated immediately above.
            let _ = unsafe { ShowWindow(hwnd, SW_MAXIMIZE) };
            Ok(())
        }
        ComputerAction::WindowRestore => {
            // SAFETY: hwnd was revalidated immediately above.
            let _ = unsafe { ShowWindow(hwnd, SW_RESTORE) };
            Ok(())
        }
        ComputerAction::WindowClose => {
            // SAFETY: hwnd was revalidated immediately above and WM_CLOSE is asynchronous.
            unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }
                .map_err(|error| broker(format!("close_window:{error}")))
        }
        ComputerAction::LaunchApp { app_id } => {
            let mut command = Command::new(app_id);
            ProcessPolicy::VisibleApplication.apply_std(&mut command);
            command
                .spawn()
                .map(|_| ())
                .map_err(|error| broker(format!("launch_app:{error}")))
        }
    }
}

fn arm_next_window_frame(window_handle: isize) {
    let Some(cache) = WINDOW_CAPTURE.get() else {
        return;
    };
    let cache = cache.lock();
    if let Some(active) = cache.active.as_ref()
        && active.window_handle == window_handle
        && !active.finished()
    {
        active.requested.store(true, Ordering::Release);
    }
}

fn mouse_button_flags(
    button: &str,
) -> Result<
    (
        ::windows::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS,
        ::windows::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS,
    ),
    ComputerHostError,
> {
    match button {
        "left" => Ok((MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP)),
        "right" => Ok((MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP)),
        "middle" => Ok((MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP)),
        _ => Err(ComputerHostError::InvalidAction),
    }
}

#[derive(Clone)]
struct CaptureFlags {
    requested: Arc<AtomicBool>,
    state: Arc<Mutex<FrameState>>,
    wake: Arc<Condvar>,
}

#[derive(Default)]
struct FrameState {
    generation: u64,
    image: Option<CapturedImage>,
}

struct StreamingCapture {
    flags: CaptureFlags,
}

impl GraphicsCaptureApiHandler for StreamingCapture {
    type Flags = CaptureFlags;
    type Error = String;

    fn new(context: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self {
            flags: context.flags,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        if !self.flags.requested.swap(false, Ordering::AcqRel) {
            return Ok(());
        }
        let width = frame.width();
        let height = frame.height();
        validate_dimensions(width, height).map_err(|error| error.to_string())?;
        let buffer = frame.buffer().map_err(|error| error.to_string())?;
        let mut packed = Vec::new();
        let png_bytes = encode_rgba_png(buffer.as_nopadding_buffer(&mut packed), width, height)?;
        let mut state = self.flags.state.lock();
        state.generation = state.generation.saturating_add(1);
        state.image = Some(CapturedImage {
            width,
            height,
            png_bytes,
        });
        self.flags.wake.notify_all();
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn encode_rgba_png(buffer: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut output, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
        writer
            .write_image_data(buffer)
            .map_err(|error| error.to_string())?;
    }
    Ok(output)
}

fn process_image_path(process: HANDLE) -> Result<PathBuf, ComputerHostError> {
    let mut buffer = vec![0_u16; 32_768];
    let mut length = u32::try_from(buffer.len()).map_err(|_| broker("process_name_too_long"))?;
    // SAFETY: the mutable UTF-16 buffer and its length are valid for the call.
    unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    }
    .map_err(|error| broker(format!("process_name:{error}")))?;
    let path = PathBuf::from(String::from_utf16_lossy(&buffer[..length as usize]));
    std::fs::canonicalize(&path)
        .or_else(|_| {
            path.is_absolute()
                .then_some(path)
                .ok_or_else(|| std::io::Error::other("process path is not absolute"))
        })
        .map_err(|error| broker(format!("process_path:{error}")))
}

fn app_descriptor(hwnd: HWND, path: &Path) -> Result<ComputerAppDescriptor, ComputerHostError> {
    let metadata =
        std::fs::metadata(path).map_err(|error| broker(format!("process_metadata:{error}")))?;
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_millis())
        .unwrap_or_default();
    let normalized_path = path.to_string_lossy().replace('/', "\\");
    let app_user_model_id = window_app_user_model_id(hwnd);
    let cache_key = format!(
        "{}|{}",
        normalized_path.to_lowercase(),
        app_user_model_id.as_deref().unwrap_or_default()
    );
    let cache = APP_DESCRIPTOR_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(cached) = cache.lock().get(&cache_key)
        && cached.len == metadata.len()
        && cached.modified_ms == modified_ms
    {
        return Ok(cached.descriptor.clone());
    }

    let executable_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| broker("process_name_invalid"))?
        .to_owned();
    if let Some(packaged) = app_user_model_id.as_deref().and_then(packaged_app_metadata) {
        let publisher_verified = packaged.publisher.is_some();
        let identity_hash = packaged_identity_hash(
            &packaged.package_family_name,
            &packaged.app_user_model_id,
            packaged.publisher.as_deref(),
        );
        let descriptor = ComputerAppDescriptor {
            app_id: packaged.app_user_model_id.clone(),
            display_name: packaged
                .display_name
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| executable_name.clone()),
            executable_name,
            executable_path: Some(normalized_path),
            publisher: packaged.publisher,
            publisher_verified,
            package_family_name: Some(packaged.package_family_name),
            app_user_model_id: Some(packaged.app_user_model_id),
            file_identity: None,
            identity_hash,
        };
        cache.lock().insert(
            cache_key,
            CachedAppDescriptor {
                len: metadata.len(),
                modified_ms,
                descriptor: descriptor.clone(),
            },
        );
        return Ok(descriptor);
    }
    let app_id = executable_name.to_ascii_lowercase();
    let version = version_strings(path);
    let display_name = version
        .product_name
        .or(version.file_description)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| executable_name.clone());
    let verified_publisher = verified_publisher(path);
    let publisher_verified = verified_publisher.is_some();
    let publisher = verified_publisher.or_else(|| {
        version
            .company_name
            .filter(|value| !value.trim().is_empty())
    });
    let file_identity = sha256_file(path)?;
    let normalized_identity_path = normalized_path.to_lowercase();
    let identity_hash = win32_identity_hash(
        &normalized_identity_path,
        &file_identity,
        publisher.as_deref(),
        publisher_verified,
    );
    let descriptor = ComputerAppDescriptor {
        app_id,
        display_name,
        executable_name,
        executable_path: Some(normalized_path),
        publisher,
        publisher_verified,
        package_family_name: None,
        app_user_model_id,
        file_identity: Some(file_identity),
        identity_hash,
    };
    cache.lock().insert(
        cache_key,
        CachedAppDescriptor {
            len: metadata.len(),
            modified_ms,
            descriptor: descriptor.clone(),
        },
    );
    Ok(descriptor)
}

pub(super) fn app_icon_png(
    app: &ComputerAppDescriptor,
) -> Result<Option<Vec<u8>>, ComputerHostError> {
    let cache = APP_ICON_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(cached) = cache.lock().get(&app.identity_hash) {
        return Ok(Some(cached.clone()));
    }
    let Some(path) = app.executable_path.as_deref() else {
        return Ok(None);
    };
    let result = native_icon_png(Path::new(path))?;
    if let Some(bytes) = result.as_ref() {
        cache
            .lock()
            .insert(app.identity_hash.clone(), bytes.clone());
    }
    Ok(result)
}

fn native_icon_png(path: &Path) -> Result<Option<Vec<u8>>, ComputerHostError> {
    let path_wide = wide_null(path.as_os_str());
    let mut file_info = SHFILEINFOW::default();
    // SAFETY: `path_wide` is a valid, nul-terminated UTF-16 path and
    // `file_info` points to writable storage of the exact Shell structure size.
    let result = unsafe {
        SHGetFileInfoW(
            PCWSTR(path_wide.as_ptr()),
            ::windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(std::ptr::addr_of_mut!(file_info)),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        )
    };
    if result == 0 || file_info.hIcon.0.is_null() {
        return Ok(None);
    }
    let icon = file_info.hIcon;
    let result = render_icon_png(icon);
    // SAFETY: Shell returned an owned icon handle which must be destroyed once.
    let _ = unsafe { DestroyIcon(icon) };
    result
        .map(Some)
        .map_err(|error| broker(format!("app_icon_render:{error}")))
}

fn render_icon_png(
    icon: ::windows::Win32::UI::WindowsAndMessaging::HICON,
) -> Result<Vec<u8>, String> {
    const SIZE: i32 = 32;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: SIZE,
            biHeight: -SIZE,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..BITMAPINFOHEADER::default()
        },
        ..BITMAPINFO::default()
    };
    let hdc = unsafe { CreateCompatibleDC(None) };
    if hdc.0.is_null() {
        return Err("create_icon_dc".into());
    }
    let mut pixels = std::ptr::null_mut();
    let bitmap =
        unsafe { CreateDIBSection(Some(hdc), &info, DIB_RGB_COLORS, &mut pixels, None, 0) }
            .map_err(|error| format!("create_icon_bitmap:{error}"));
    let Ok(bitmap) = bitmap else {
        let _ = unsafe { DeleteDC(hdc) };
        return Err(bitmap.unwrap_err());
    };
    let previous = unsafe { SelectObject(hdc, HGDIOBJ(bitmap.0)) };
    let draw_result = unsafe { DrawIconEx(hdc, 0, 0, icon, SIZE, SIZE, 0, None, DI_NORMAL) };
    let output = if let Err(error) = draw_result {
        Err(format!("draw_icon:{error}"))
    } else if pixels.is_null() {
        Err("icon_pixels_null".into())
    } else {
        let bgra =
            unsafe { std::slice::from_raw_parts(pixels.cast::<u8>(), (SIZE * SIZE * 4) as usize) };
        let mut rgba = vec![0_u8; bgra.len()];
        for (source, target) in bgra.chunks_exact(4).zip(rgba.chunks_exact_mut(4)) {
            target[0] = source[2];
            target[1] = source[1];
            target[2] = source[0];
            target[3] = if source[3] == 0 { 255 } else { source[3] };
        }
        encode_rgba_png(&rgba, SIZE as u32, SIZE as u32)
            .map_err(|error| format!("encode_icon:{error}"))
    };
    if !previous.0.is_null() {
        unsafe { SelectObject(hdc, previous) };
    }
    unsafe {
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(hdc);
    }
    output
}

#[derive(Debug, Deserialize)]
struct PackagedAppMetadata {
    #[serde(rename = "family")]
    package_family_name: String,
    #[serde(rename = "aumid")]
    app_user_model_id: String,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    publisher: Option<String>,
}

fn window_app_user_model_id(hwnd: HWND) -> Option<String> {
    // The Shell property store is the reliable source for both packaged
    // windows hosted by ApplicationFrameHost and classic windows with an
    // explicit AppUserModelID.
    let store = unsafe { SHGetPropertyStoreForWindow::<IPropertyStore>(hwnd) }.ok()?;
    let value = unsafe { store.GetValue(&PKEY_APP_USER_MODEL_ID) }.ok()?;
    let mut buffer = [0_u16; 512];
    unsafe { PropVariantToString(std::ptr::addr_of!(value), &mut buffer) }.ok()?;
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    let value = String::from_utf16_lossy(&buffer[..end]).trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn packaged_app_metadata(app_user_model_id: &str) -> Option<PackagedAppMetadata> {
    let package_family_name = package_family_from_aumid(app_user_model_id)?;
    let script = r#"
$OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$package = Get-AppxPackage -PackageFamilyName $env:HACHIMI_POWERSHELL_ARG_0 | Select-Object -First 1
if ($null -eq $package) { exit 0 }
$manifest = Get-AppxPackageManifest -Package $package
$applicationId = $env:HACHIMI_POWERSHELL_ARG_1.Substring($env:HACHIMI_POWERSHELL_ARG_1.IndexOf('!') + 1)
$application = @($manifest.Package.Applications.Application) | Where-Object { $_.Id -eq $applicationId } | Select-Object -First 1
$startApp = @(Get-StartApps -ErrorAction SilentlyContinue) | Where-Object { $_.AppID -eq $env:HACHIMI_POWERSHELL_ARG_1 } | Select-Object -First 1
$displayName = if ($null -ne $startApp) { $startApp.Name } elseif ($null -ne $application.VisualElements.DisplayName) { $application.VisualElements.DisplayName } else { $package.Name }
[Console]::Out.Write(( [pscustomobject]@{ family = $package.PackageFamilyName; aumid = $env:HACHIMI_POWERSHELL_ARG_1; displayName = $displayName; publisher = $package.Publisher } | ConvertTo-Json -Compress ))
"#;
    let output = powershell_args(
        script,
        &[
            std::ffi::OsStr::new(package_family_name),
            std::ffi::OsStr::new(app_user_model_id),
        ],
    )?;
    serde_json::from_str(output.trim()).ok()
}

fn package_family_from_aumid(app_user_model_id: &str) -> Option<&str> {
    let (package_family_name, application_id) = app_user_model_id.split_once('!')?;
    let package_family_name = package_family_name.trim();
    (!package_family_name.is_empty() && !application_id.trim().is_empty())
        .then_some(package_family_name)
}

fn packaged_identity_hash(
    package_family_name: &str,
    app_user_model_id: &str,
    publisher: Option<&str>,
) -> String {
    fingerprint(&(
        "packaged",
        package_family_name,
        app_user_model_id,
        publisher.unwrap_or_default(),
    ))
}

fn win32_identity_hash(
    normalized_path: &str,
    file_identity: &str,
    publisher: Option<&str>,
    publisher_verified: bool,
) -> String {
    if publisher_verified {
        fingerprint(&(
            "signed_win32",
            normalized_path,
            publisher.unwrap_or_default(),
        ))
    } else {
        fingerprint(&("unsigned_win32", normalized_path, file_identity))
    }
}

fn verified_publisher(path: &Path) -> Option<String> {
    let script = r#"$signature = Get-AuthenticodeSignature -LiteralPath $env:HACHIMI_POWERSHELL_ARG_0; if ($signature.Status -eq 'Valid' -and $null -ne $signature.SignerCertificate) { [Console]::Out.Write($signature.SignerCertificate.GetNameInfo('SimpleName', $false)) }"#;
    powershell_text(script, path).filter(|value| !value.trim().is_empty())
}

fn powershell_text(script: &str, path: &Path) -> Option<String> {
    powershell_args(script, &[path.as_os_str()])
}

fn powershell_args(script: &str, args: &[&std::ffi::OsStr]) -> Option<String> {
    let mut command = hachimi_process_policy::std_command(
        "powershell.exe",
        hachimi_process_policy::ProcessPolicy::HiddenBackground,
    );
    // `-Command` treats trailing argv values as PowerShell source text. Pass
    // paths through the child environment so spaces and non-ASCII characters
    // remain intact instead of being parsed as code.
    for (index, argument) in args.iter().enumerate() {
        command.env(format!("HACHIMI_POWERSHELL_ARG_{index}"), argument);
    }
    let output = command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.len() > 512 * 1024 {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

#[derive(Default)]
struct VersionStrings {
    file_description: Option<String>,
    product_name: Option<String>,
    company_name: Option<String>,
}

fn version_strings(path: &Path) -> VersionStrings {
    let path_wide = wide_null(path.as_os_str());
    // SAFETY: path_wide is NUL-terminated and remains alive for both calls.
    let size = unsafe { GetFileVersionInfoSizeW(PCWSTR(path_wide.as_ptr()), None) };
    if size == 0 || size > 16 * 1024 * 1024 {
        return VersionStrings::default();
    }
    let mut data = vec![0_u8; size as usize];
    // SAFETY: data has exactly the size requested by Windows.
    if unsafe {
        GetFileVersionInfoW(
            PCWSTR(path_wide.as_ptr()),
            None,
            size,
            data.as_mut_ptr().cast(),
        )
    }
    .is_err()
    {
        return VersionStrings::default();
    }
    let translation = version_translation(&data).unwrap_or((0x0409, 0x04b0));
    VersionStrings {
        file_description: version_string(&data, translation, "FileDescription"),
        product_name: version_string(&data, translation, "ProductName"),
        company_name: version_string(&data, translation, "CompanyName"),
    }
}

fn version_translation(data: &[u8]) -> Option<(u16, u16)> {
    let query = wide_null(std::ffi::OsStr::new("\\VarFileInfo\\Translation"));
    let mut value = std::ptr::null_mut();
    let mut len = 0_u32;
    // SAFETY: data is a successful version resource buffer and output pointers are writable.
    let ok = unsafe {
        VerQueryValueW(
            data.as_ptr().cast(),
            PCWSTR(query.as_ptr()),
            &mut value,
            &mut len,
        )
    };
    if !ok.as_bool() || value.is_null() || len < 4 {
        return None;
    }
    // SAFETY: Windows reported at least two u16 values.
    let values = unsafe { std::slice::from_raw_parts(value.cast::<u16>(), 2) };
    Some((values[0], values[1]))
}

fn version_string(data: &[u8], translation: (u16, u16), key: &str) -> Option<String> {
    let query = format!(
        "\\StringFileInfo\\{:04x}{:04x}\\{key}",
        translation.0, translation.1
    );
    let query = wide_null(std::ffi::OsStr::new(&query));
    let mut value = std::ptr::null_mut();
    let mut len = 0_u32;
    // SAFETY: data is a successful version resource buffer and output pointers are writable.
    let ok = unsafe {
        VerQueryValueW(
            data.as_ptr().cast(),
            PCWSTR(query.as_ptr()),
            &mut value,
            &mut len,
        )
    };
    if !ok.as_bool() || value.is_null() || len <= 1 {
        return None;
    }
    // SAFETY: Windows reports the UTF-16 element count, including the trailing NUL.
    let units = unsafe { std::slice::from_raw_parts(value.cast::<u16>(), len as usize) };
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units.len());
    let value = String::from_utf16_lossy(&units[..end]).trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn wide_null(value: &std::ffi::OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

fn sha256_file(path: &Path) -> Result<String, ComputerHostError> {
    let mut file =
        File::open(path).map_err(|error| broker(format!("process_hash_open:{error}")))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| broker(format!("process_hash_read:{error}")))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn is_user_application(app_id: &str) -> bool {
    !matches!(
        app_id,
        "ctfmon.exe"
            | "dllhost.exe"
            | "dwm.exe"
            | "fontdrvhost.exe"
            | "searchhost.exe"
            | "shellexperiencehost.exe"
            | "sihost.exe"
            | "startmenuexperiencehost.exe"
            | "taskhostw.exe"
            | "textinputhost.exe"
    )
}

fn process_integrity(process: HANDLE) -> Result<u32, ComputerHostError> {
    let mut token = HANDLE::default();
    // SAFETY: token points to writable storage for the returned handle.
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }
        .map_err(|error| broker(format!("open_process_token:{error}")))?;
    // SAFETY: token is a newly owned kernel handle.
    let token = unsafe { Owned::new(token) };
    let mut length = 0_u32;
    // SAFETY: the first call intentionally supplies no buffer to obtain the required byte count.
    let _ = unsafe { GetTokenInformation(*token, TokenIntegrityLevel, None, 0, &mut length) };
    if length < u32::try_from(std::mem::size_of::<TOKEN_MANDATORY_LABEL>()).unwrap_or(u32::MAX) {
        return Err(broker("token_integrity_size_invalid"));
    }
    let mut buffer = vec![0_u8; length as usize];
    // SAFETY: buffer has the size returned by Windows and remains valid for the call.
    unsafe {
        GetTokenInformation(
            *token,
            TokenIntegrityLevel,
            Some(buffer.as_mut_ptr().cast()),
            length,
            &mut length,
        )
    }
    .map_err(|error| broker(format!("token_integrity:{error}")))?;
    // SAFETY: successful TokenIntegrityLevel queries return TOKEN_MANDATORY_LABEL at the start.
    let label = unsafe { &*(buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()) };
    let sid = label.Label.Sid;
    // SAFETY: the SID belongs to the validated token information buffer.
    let count = unsafe { GetSidSubAuthorityCount(sid).as_ref() }
        .copied()
        .filter(|count| *count > 0)
        .ok_or_else(|| broker("token_integrity_sid_invalid"))?;
    // SAFETY: count was read from the valid SID and is greater than zero.
    unsafe { GetSidSubAuthority(sid, u32::from(count - 1)).as_ref() }
        .copied()
        .ok_or_else(|| broker("token_integrity_rid_invalid"))
}

fn thread_desktop_name(thread_id: u32) -> Result<String, ComputerHostError> {
    // SAFETY: the thread ID came from GetWindowThreadProcessId.
    let desktop = unsafe { GetThreadDesktop(thread_id) }
        .map_err(|error| broker(format!("thread_desktop:{error}")))?;
    desktop_name(HANDLE(desktop.0))
}

fn input_desktop_name() -> Result<String, ComputerHostError> {
    // SAFETY: the access mask is read-only and the returned desktop is wrapped in Owned.
    let desktop = unsafe { OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_READOBJECTS) }
        .map_err(|error| broker(format!("input_desktop:{error}")))?;
    // SAFETY: desktop is a newly opened desktop handle.
    let desktop = unsafe { Owned::new(desktop) };
    desktop_name(HANDLE(desktop.0))
}

fn desktop_name(handle: HANDLE) -> Result<String, ComputerHostError> {
    let mut needed = 0_u32;
    // SAFETY: the first call intentionally supplies no buffer to obtain the required byte count.
    let _ = unsafe { GetUserObjectInformationW(handle, UOI_NAME, None, 0, Some(&mut needed)) };
    if !(2..=32_768).contains(&needed) {
        return Err(broker("desktop_name_size_invalid"));
    }
    let mut buffer = vec![0_u16; needed.div_ceil(2) as usize];
    // SAFETY: buffer is at least the byte size returned by the first call.
    unsafe {
        GetUserObjectInformationW(
            handle,
            UOI_NAME,
            Some(buffer.as_mut_ptr().cast()),
            needed,
            Some(&mut needed),
        )
    }
    .map_err(|error| broker(format!("desktop_name:{error}")))?;
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    let name = String::from_utf16_lossy(&buffer[..end]);
    (!name.trim().is_empty())
        .then_some(name)
        .ok_or_else(|| broker("desktop_name_empty"))
}

fn window_text(hwnd: HWND) -> String {
    let mut buffer = vec![0_u16; 2_048];
    // SAFETY: buffer is valid and hwnd was checked before this helper is called.
    let length = unsafe { GetWindowTextW(hwnd, &mut buffer) }.max(0) as usize;
    String::from_utf16_lossy(&buffer[..length])
}

fn window_class(hwnd: HWND) -> String {
    let mut buffer = vec![0_u16; 512];
    // SAFETY: buffer is valid and hwnd was checked before this helper is called.
    let length = unsafe { GetClassNameW(hwnd, &mut buffer) }.max(0) as usize;
    String::from_utf16_lossy(&buffer[..length])
}

fn window_rect(hwnd: HWND) -> Result<RECT, ComputerHostError> {
    let mut rect = RECT::default();
    // SAFETY: rect points to writable storage and hwnd has been validated.
    unsafe { GetWindowRect(hwnd, &mut rect) }
        .map_err(|error| broker(format!("window_rect:{error}")))?;
    if rect.right <= rect.left || rect.bottom <= rect.top {
        return Err(broker("window_rect_invalid"));
    }
    Ok(rect)
}

fn move_pointer(rect: RECT, x: i32, y: i32) -> Result<(), ComputerHostError> {
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if x < 0 || y < 0 || x >= width || y >= height {
        return Err(ComputerHostError::InvalidAction);
    }
    // SAFETY: coordinates were bounded to the currently fenced window rectangle.
    unsafe { SetCursorPos(rect.left.saturating_add(x), rect.top.saturating_add(y)) }
        .map_err(|error| broker(format!("set_cursor:{error}")))
}

fn send_key(key: &str, modifiers: &[String]) -> Result<(), ComputerHostError> {
    let key = virtual_key(key).ok_or(ComputerHostError::InvalidAction)?;
    let mut held = Vec::with_capacity(modifiers.len());
    for modifier in modifiers {
        let value = match modifier.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => VK_CONTROL,
            "alt" => VK_MENU,
            "shift" => VK_SHIFT,
            _ => return Err(ComputerHostError::InvalidAction),
        };
        if held.contains(&value) {
            return Err(ComputerHostError::InvalidAction);
        }
        held.push(value);
    }
    let mut inputs = Vec::with_capacity(held.len() * 2 + 2);
    inputs.extend(
        held.iter()
            .copied()
            .map(|value| keyboard_input(value, false)),
    );
    inputs.push(keyboard_input(key, false));
    inputs.push(keyboard_input(key, true));
    inputs.extend(
        held.iter()
            .rev()
            .copied()
            .map(|value| keyboard_input(value, true)),
    );
    send_inputs(&inputs)
}

fn send_key_transition(key: &str, key_up: bool) -> Result<(), ComputerHostError> {
    let key = virtual_key(key).ok_or(ComputerHostError::InvalidAction)?;
    send_inputs(&[keyboard_input(key, key_up)])
}

fn send_chord(keys: &[String]) -> Result<(), ComputerHostError> {
    let keys = keys
        .iter()
        .map(|key| virtual_key(key).ok_or(ComputerHostError::InvalidAction))
        .collect::<Result<Vec<_>, _>>()?;
    let mut inputs = Vec::with_capacity(keys.len() * 2);
    inputs.extend(keys.iter().copied().map(|key| keyboard_input(key, false)));
    inputs.extend(
        keys.iter()
            .rev()
            .copied()
            .map(|key| keyboard_input(key, true)),
    );
    send_inputs(&inputs)
}

fn send_text(text: &str) -> Result<(), ComputerHostError> {
    for unit in text.encode_utf16() {
        let down = unicode_input(unit, false);
        let up = unicode_input(unit, true);
        send_inputs(&[down, up])?;
    }
    Ok(())
}

fn send_inputs(inputs: &[INPUT]) -> Result<(), ComputerHostError> {
    if inputs.is_empty() {
        return Ok(());
    }
    let size = i32::try_from(std::mem::size_of::<INPUT>())
        .map_err(|_| ComputerHostError::InvalidAction)?;
    // SAFETY: the INPUT slice is fully initialized and remains valid for the call.
    let sent = unsafe { SendInput(inputs, size) };
    if sent != u32::try_from(inputs.len()).unwrap_or(u32::MAX) {
        return Err(broker("send_input_rejected"));
    }
    Ok(())
}

fn mouse_input(
    flags: ::windows::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS,
    data: u32,
) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                mouseData: data,
                dwFlags: flags,
                ..Default::default()
            },
        },
    }
}

fn keyboard_input(key: VIRTUAL_KEY, key_up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                dwFlags: if key_up {
                    KEYEVENTF_KEYUP
                } else {
                    Default::default()
                },
                ..Default::default()
            },
        },
    }
}

fn unicode_input(unit: u16, key_up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: unit,
                dwFlags: if key_up {
                    KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
                } else {
                    KEYEVENTF_UNICODE
                },
                ..Default::default()
            },
        },
    }
}

fn virtual_key(key: &str) -> Option<VIRTUAL_KEY> {
    let normalized = key.trim().to_ascii_lowercase();
    if normalized.len() == 1 {
        let value = normalized.as_bytes()[0];
        if value.is_ascii_alphanumeric() {
            return Some(VIRTUAL_KEY(u16::from(value.to_ascii_uppercase())));
        }
    }
    let value = match normalized.as_str() {
        "backspace" => VK_BACK,
        "tab" => VK_TAB,
        "ctrl" | "control" => VK_CONTROL,
        "alt" => VK_MENU,
        "shift" => VK_SHIFT,
        "enter" | "return" => VK_RETURN,
        "escape" | "esc" => VK_ESCAPE,
        "space" => VK_SPACE,
        "pageup" | "page_up" => VK_PRIOR,
        "pagedown" | "page_down" => VK_NEXT,
        "end" => VK_END,
        "home" => VK_HOME,
        "left" | "arrowleft" => VK_LEFT,
        "up" | "arrowup" => VK_UP,
        "right" | "arrowright" => VK_RIGHT,
        "down" | "arrowdown" => VK_DOWN,
        "insert" => VK_INSERT,
        "delete" => VK_DELETE,
        value if value.len() <= 3 && value.starts_with('f') => {
            let number = value[1..].parse::<u16>().ok()?;
            if !(1..=12).contains(&number) {
                return None;
            }
            VIRTUAL_KEY(111 + number)
        }
        _ => return None,
    };
    Some(value)
}

fn parse_handle(value: &str) -> Result<isize, ComputerHostError> {
    let value = value.trim();
    let parsed = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .map_or_else(
            || value.parse::<isize>(),
            |hex| isize::from_str_radix(hex, 16),
        )
        .map_err(|_| ComputerHostError::InvalidAction)?;
    (parsed > 0)
        .then_some(parsed)
        .ok_or(ComputerHostError::InvalidAction)
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), ComputerHostError> {
    (width > 0 && height > 0 && width <= MAX_CAPTURE_DIMENSION && height <= MAX_CAPTURE_DIMENSION)
        .then_some(())
        .ok_or_else(|| broker("window_size_invalid"))
}

fn fingerprint(value: &impl serde::Serialize) -> String {
    Sha256::digest(serde_json::to_vec(value).unwrap_or_default())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn broker(message: impl Into<String>) -> ComputerHostError {
    ComputerHostError::Broker(message.into())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        process::{Child, Command},
        thread,
        time::{Duration, Instant},
    };

    use ::windows::Win32::{
        System::Threading::{
            AttachThreadInput, GetCurrentProcess, GetCurrentThreadId, GetProcessHandleCount,
            OpenProcess, PROCESS_TERMINATE, TerminateProcess,
        },
        UI::WindowsAndMessaging::{
            BringWindowToTop, MSG, PM_NOREMOVE, PeekMessageW, SetForegroundWindow,
        },
    };

    use super::*;

    #[test]
    fn packaged_identity_is_scoped_to_family_aumid_and_publisher() {
        let family = "Contoso.Notes_abcd1234";
        let aumid = "Contoso.Notes_abcd1234!App";
        assert_eq!(package_family_from_aumid(aumid), Some(family));
        assert_eq!(package_family_from_aumid("not-packaged"), None);
        assert_eq!(package_family_from_aumid("family!"), None);

        let identity = packaged_identity_hash(family, aumid, Some("CN=Contoso"));
        assert_eq!(
            identity,
            packaged_identity_hash(family, aumid, Some("CN=Contoso"))
        );
        assert_ne!(
            identity,
            packaged_identity_hash(family, "Contoso.Notes_abcd1234!Admin", Some("CN=Contoso"))
        );
        assert_ne!(
            identity,
            packaged_identity_hash(family, aumid, Some("CN=Other"))
        );
    }

    #[test]
    fn extracts_a_png_from_a_real_windows_executable() {
        let path = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join("Explorer.exe");
        let icon = native_icon_png(&path)
            .expect("shell icon extraction")
            .expect("Explorer icon");
        assert!(icon.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]));
        assert!(icon.len() <= 256 * 1024);
    }

    #[test]
    fn win32_identity_separates_paths_and_reprompts_only_unsigned_file_changes() {
        let first_path = win32_identity_hash(
            r"c:\apps\first\tool.exe",
            "content-a",
            Some("Contoso"),
            true,
        );
        let second_path = win32_identity_hash(
            r"c:\apps\second\tool.exe",
            "content-a",
            Some("Contoso"),
            true,
        );
        assert_ne!(first_path, second_path);
        assert_eq!(
            first_path,
            win32_identity_hash(
                r"c:\apps\first\tool.exe",
                "content-b",
                Some("Contoso"),
                true,
            )
        );

        let unsigned = win32_identity_hash(r"c:\apps\tool.exe", "content-a", None, false);
        assert_ne!(
            unsigned,
            win32_identity_hash(r"c:\apps\tool.exe", "content-b", None, false)
        );
    }

    struct TestProcessGuard {
        child: Child,
        window_process_id: Option<u32>,
    }

    impl Drop for TestProcessGuard {
        fn drop(&mut self) {
            if let Some(process_id) = self.window_process_id {
                // SAFETY: the process ID belongs to the test-created fixture
                // window and the returned handle is immediately owned.
                if let Ok(process) = unsafe { OpenProcess(PROCESS_TERMINATE, false, process_id) } {
                    // SAFETY: OpenProcess returned a newly owned HANDLE.
                    let process = unsafe { Owned::new(process) };
                    // SAFETY: process was opened with PROCESS_TERMINATE solely
                    // to keep this ignored interactive smoke leak-free.
                    let _ = unsafe { TerminateProcess(*process, 0) };
                }
            }
            let _ = self.child.kill();
        }
    }

    #[test]
    #[ignore = "requires an interactive default Windows desktop"]
    fn captures_and_controls_the_win32_stress_fixture_with_wgc() {
        let seconds = std::env::var("HACHIMI_STRESS_PHASE_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok());
        let stress_deadline =
            seconds.map(|seconds| Instant::now() + Duration::from_secs(seconds.clamp(1, 450)));
        let mut iterations = 0_u64;
        loop {
            log_stress_handles(iterations, "before_iteration");
            let fixture_deadline = stress_deadline
                .map(|deadline| deadline.min(Instant::now() + Duration::from_secs(25)));
            run_real_fixture_wgc_iteration(iterations, fixture_deadline);
            log_stress_handles(iterations, "after_iteration");
            iterations = iterations.saturating_add(1);
            if stress_deadline.is_none_or(|deadline| Instant::now() >= deadline) {
                break;
            }
        }
        eprintln!("computer_real_stress_iterations={iterations}");
    }

    fn log_stress_handles(iteration: u64, stage: &str) {
        if std::env::var_os("HACHIMI_COMPUTER_HANDLE_DIAGNOSTICS").is_none() {
            return;
        }
        let mut count = 0_u32;
        // SAFETY: GetCurrentProcess returns a valid pseudo-handle and count is writable.
        if unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) }.is_ok() {
            eprintln!("computer_handle iteration={iteration} stage={stage} count={count}");
        }
    }

    fn run_real_fixture_wgc_iteration(iteration: u64, fixture_deadline: Option<Instant>) {
        let existing = Window::enumerate()
            .expect("enumerate existing windows")
            .into_iter()
            .map(|window| window.as_raw_hwnd() as usize)
            .collect::<BTreeSet<_>>();
        let fixture = std::env::var("HACHIMI_COMPUTER_STRESS_FIXTURE")
            .expect("HACHIMI_COMPUTER_STRESS_FIXTURE must name the built Win32 fixture");
        let child = Command::new(&fixture)
            .spawn()
            .expect("launch Hachimi Computer stress fixture");
        let mut process_guard = TestProcessGuard {
            child,
            window_process_id: None,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        let window = loop {
            let candidate = Window::enumerate()
                .expect("enumerate windows")
                .into_iter()
                .find(|window| {
                    !existing.contains(&(window.as_raw_hwnd() as usize))
                        && window
                            .title()
                            .is_ok_and(|title| title == "Hachimi Computer Stress Fixture")
                });
            if let Some(window) = candidate {
                break window;
            }
            if Instant::now() >= deadline {
                panic!("Hachimi Computer stress fixture did not expose a capturable window");
            }
            thread::sleep(Duration::from_millis(100));
        };
        let handle = format!("0x{:x}", window.as_raw_hwnd() as usize);
        let identity = read_identity(&handle).expect("window identity");
        process_guard.window_process_id = Some(identity.process_id);
        assert!(!identity.elevated);
        assert!(!identity.protected_desktop);
        assert!(!identity.hachimi_owned);

        let mut previous_frame = capture_window(&handle).expect("WGC frame");
        log_stress_handles(iteration, "after_capture_initial");
        assert!(previous_frame.width > 0 && previous_frame.height > 0);
        assert!(
            previous_frame
                .png_bytes
                .starts_with(&[137, 80, 78, 71, 13, 10, 26, 10])
        );

        let hwnd = HWND(window.as_raw_hwnd());
        let mut cycle = 0_u64;
        loop {
            select_stress_window(hwnd);
            let actionable_identity = read_identity(&handle).expect("foreground window identity");
            perform_action(
                &actionable_identity,
                &ComputerAction::TypeText {
                    text: format!("Hachimi Computer Host stress {iteration}:{cycle} "),
                },
            )
            .expect("fenced SendInput action");
            let mutation_deadline = Instant::now() + Duration::from_secs(5);
            let after_text_frame = loop {
                let frame = capture_window(&handle).expect("WGC frame after text input");
                if frame.png_bytes != previous_frame.png_bytes {
                    break frame;
                }
                assert!(
                    Instant::now() < mutation_deadline,
                    "Computer fixture did not expose a changed WGC frame after controlled input"
                );
                thread::sleep(Duration::from_millis(100));
            };
            log_stress_handles(iteration, "after_capture_text");
            select_stress_window(hwnd);
            let after_text = read_identity(&handle).expect("identity after text input");
            perform_action(
                &after_text,
                &ComputerAction::Scroll {
                    delta_x: 0,
                    delta_y: -120,
                },
            )
            .expect("fenced scroll action");
            select_stress_window(hwnd);
            let before_resize = read_identity(&handle).expect("identity before resize");
            perform_action(
                &before_resize,
                &ComputerAction::WindowResize {
                    width: 720 + u32::try_from((iteration + cycle) % 2).unwrap_or_default() * 40,
                    height: 520,
                },
            )
            .expect("fenced resize action");
            let resize_deadline = Instant::now() + Duration::from_secs(5);
            let recaptured = loop {
                let frame = capture_window(&handle).expect("WGC frame after controlled actions");
                if frame.png_bytes != after_text_frame.png_bytes
                    && frame.width >= 700
                    && frame.height >= 500
                {
                    break frame;
                }
                assert!(
                    Instant::now() < resize_deadline,
                    "Computer fixture did not expose a changed WGC frame after resize"
                );
            };
            log_stress_handles(iteration, "after_capture_resize");
            previous_frame = recaptured;
            cycle = cycle.saturating_add(1);
            if fixture_deadline.is_none_or(|deadline| Instant::now() >= deadline) {
                break;
            }
        }
        assert!(previous_frame.width >= 700 && previous_frame.height >= 500);
    }

    fn select_stress_window(hwnd: HWND) {
        let deadline = Instant::now() + Duration::from_secs(10);
        // SAFETY: GetForegroundWindow has no preconditions and returns a borrowed HWND value.
        while unsafe { GetForegroundWindow() } != hwnd {
            send_inputs(&[
                keyboard_input(VK_MENU, false),
                keyboard_input(VK_MENU, true),
            ])
            .expect("test harness Alt activation");
            focus_owned_stress_window(hwnd);
            if Instant::now() >= deadline {
                let foreground = unsafe { GetForegroundWindow() };
                let mut target_process = 0_u32;
                let mut foreground_process = 0_u32;
                let target_thread =
                    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut target_process)) };
                let foreground_thread =
                    unsafe { GetWindowThreadProcessId(foreground, Some(&mut foreground_process)) };
                panic!(
                    "computer_stress_foreground_contended: target={:?} pid={target_process} tid={target_thread}; foreground={:?} pid={foreground_process} tid={foreground_thread}",
                    hwnd.0, foreground.0
                );
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn focus_owned_stress_window(hwnd: HWND) {
        // This ignored harness owns the fixture HWND. Temporarily sharing its
        // input queues models explicit user selection without weakening the
        // production broker's fail-closed foreground fencing.
        let mut message = MSG::default();
        // SAFETY: a no-remove peek initializes this test thread's USER32
        // message queue without consuming application messages.
        let _ = unsafe { PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE) };
        let current_thread = unsafe { GetCurrentThreadId() };
        let foreground = unsafe { GetForegroundWindow() };
        let foreground_thread = if !foreground.0.is_null() {
            unsafe { GetWindowThreadProcessId(foreground, None) }
        } else {
            0
        };
        let target_thread = unsafe { GetWindowThreadProcessId(hwnd, None) };
        let foreground_attached = foreground_thread != 0
            && foreground_thread != current_thread
            && unsafe { AttachThreadInput(current_thread, foreground_thread, true) }.as_bool();
        let target_attached = target_thread != 0
            && target_thread != current_thread
            && target_thread != foreground_thread
            && unsafe { AttachThreadInput(current_thread, target_thread, true) }.as_bool();

        // SAFETY: hwnd belongs to the live Computer fixture created by this test.
        unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = BringWindowToTop(hwnd);
            let _ = SetForegroundWindow(hwnd);
        }

        if target_attached {
            // SAFETY: this exactly balances the successful attachment above.
            let _ = unsafe { AttachThreadInput(current_thread, target_thread, false) };
        }
        if foreground_attached {
            // SAFETY: this exactly balances the successful attachment above.
            let _ = unsafe { AttachThreadInput(current_thread, foreground_thread, false) };
        }
    }
}
