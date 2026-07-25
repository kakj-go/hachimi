// Hachimi is a GUI application in both Debug and Release builds. Keeping the
// Windows GUI subsystem enabled prevents an extra console window on startup.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use hachimi_avatar::{AvatarCatalog, InspectedAvatar, inspect_avatar};
use hachimi_control_plane::ControlPlane;
use hachimi_core::{FeatureFlags, WindowKind};
use hachimi_llm::{
    ApiKeyStore, LlmError, SystemApiKeyStore, apply_secret_change, stream_pet_turn,
    test_connection, validate_input,
};
use hachimi_motion::{InspectedMotion, MotionCatalog, inspect_motion};
use hachimi_protocol::{
    AppSettings, AvatarAdaptationProfile, AvatarCatalogSnapshot, AvatarImportCommitRequest,
    AvatarImportInspection, AvatarRuntimeAsset, BootstrapState, CONTROL_PROTOCOL_VERSION,
    ClientContext, ClientId, ControlMethod, FrontendLogEntry, FrontendLogLevel,
    InteractionMotionBindingUpdateRequest, InteractiveRegionsUpdate, LipSyncCapability,
    LlmSettingsInput, LlmSettingsView, LlmTestResult, Locale, MAX_THEME_PROFILES,
    MotionAssetBindingsClearRequest, MotionBindingResetRequest, MotionCatalogSnapshot,
    MotionEnabledUpdateRequest, MotionImportCommitRequest, MotionImportInspection,
    MotionMetadataUpdateRequest, MotionRuntimeAsset, PetContextMenuRequest, PetTurnEvent,
    PetTurnRequest, ResourceEntryRequest, SETTINGS_SCHEMA_VERSION, SpeechRecognitionRuntimeState,
    SpeechRecognitionSettingsInput, ThemeProfile, ThemeProfileDocument, ThemeScheme,
    VoiceCatalogSnapshot, VoiceImportCommitRequest, VoiceModelInspection, VoiceRuntimeState,
    VoiceSettingsInput, WindowPlacementV1, WorkbenchRoute,
};
use hachimi_storage::SettingsStore;
use hachimi_voice::{
    InspectedVoiceModel, SpeechRecognizerRuntime, VoiceCatalog, VoiceEventSink, VoiceRuntime,
    VoiceRuntimeEventSinks, VoiceRuntimeStateSink, VoiceTurnEventSink, inspect_voice_archive,
};
use hachimi_windowing::{
    InteractiveRegionState, MonitorGeometry, PhysicalPoint, PhysicalRect,
    restore_or_default_placement,
};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{
    AppHandle, Emitter, LogicalPosition, Manager, PhysicalPosition, Runtime, State, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder, WindowEvent, Wry,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_runtime::ResizeDirection;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const VITS_RESOURCE: &str = "resources/ai-models/text-to-speech/vits-melo-zh-en";
const SENSEVOICE_RESOURCE: &str = "resources/ai-models/speech-to-text/sensevoice-small";
const PET_TURN_EVENT: &str = "pet:turn";
const VOICE_PLAYBACK_EVENT: &str = "voice:playback";
const VOICE_TURN_EVENT: &str = "voice:turn";
const VOICE_CATALOG_EVENT: &str = "voice:catalog-changed";
const SPEECH_RECOGNITION_STATE_EVENT: &str = "speech-recognition-state-changed";
const AVATAR_CATALOG_EVENT: &str = "avatar:catalog-changed";
const MOTION_CATALOG_EVENT: &str = "motion:catalog-changed";
const DEFAULT_AVATAR_RESOURCE: &str =
    "resources/avatar-default/3800386813668044008/3800386813668044008.vrm";
const MOTION_CATALOG_RESOURCE: &str = "resources/avatar-motions-v4/catalog.json";
const PET_VISIBILITY_EVENT: &str = "pet:visibility";
const MAX_THEME_FILE_BYTES: u64 = 64 * 1024;
const AVATAR_IMPORT_TOKEN_TTL: Duration = Duration::from_secs(10 * 60);
const MOTION_IMPORT_TOKEN_TTL: Duration = Duration::from_secs(10 * 60);
const VOICE_IMPORT_TOKEN_TTL: Duration = Duration::from_secs(10 * 60);
const APP_IDENTIFIER: &str = "com.hachimi.desktop";
const PORTABLE_MARKER_FILE: &str = "hachimi.portable";
const RESET_MARKER_FILE: &str = "hachimi-reset-all-v1.marker";
const DATA_ROOT_SENTINEL_FILE: &str = ".hachimi-data-root";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StorageMode {
    Debug,
    Portable,
    Installed,
    Override,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StorageLayout {
    root: PathBuf,
    webview: PathBuf,
    mode: StorageMode,
    redirect_webview: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ResetMarker {
    version: u32,
    root: PathBuf,
    webview: PathBuf,
}

impl StorageLayout {
    fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }
}

fn absolute_path(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn debug_data_root(executable: &Path) -> Option<PathBuf> {
    let executable_dir = executable.parent()?;
    if executable_dir
        .file_name()
        .is_some_and(|name| name == "debug")
    {
        return executable_dir
            .parent()
            .map(|target| target.join("hachimi-data"));
    }
    if executable_dir
        .file_name()
        .is_some_and(|name| name == "deps")
    {
        let debug = executable_dir.parent()?;
        if debug.file_name().is_some_and(|name| name == "debug") {
            return debug.parent().map(|target| target.join("hachimi-data"));
        }
    }
    None
}

fn resolve_storage_layout() -> StorageLayout {
    let executable = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("hachimi-desktop"));
    let executable_dir = executable
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    if let Some(override_root) = std::env::var_os("HACHIMI_DATA_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        let root = absolute_path(override_root);
        return StorageLayout {
            webview: root.join("webview"),
            root,
            mode: StorageMode::Override,
            redirect_webview: true,
        };
    }

    if executable_dir.join(PORTABLE_MARKER_FILE).is_file() {
        let root = executable_dir.join("data");
        return StorageLayout {
            webview: root.join("webview"),
            root,
            mode: StorageMode::Portable,
            redirect_webview: true,
        };
    }

    if cfg!(debug_assertions)
        && let Some(root) = debug_data_root(&executable)
    {
        return StorageLayout {
            webview: root.join("webview"),
            root,
            mode: StorageMode::Debug,
            redirect_webview: true,
        };
    }

    let root = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| executable_dir.join("data"))
        .join(APP_IDENTIFIER);
    let webview_base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| executable_dir.join("data-local"));
    let webview = webview_base.join(APP_IDENTIFIER).join("EBWebView");
    StorageLayout {
        root,
        webview,
        mode: StorageMode::Installed,
        redirect_webview: false,
    }
}

fn reset_marker_path() -> PathBuf {
    std::env::temp_dir().join(RESET_MARKER_FILE)
}

fn write_reset_marker(layout: &StorageLayout) -> Result<(), String> {
    let marker = ResetMarker {
        version: 1,
        root: layout.root.clone(),
        webview: layout.webview.clone(),
    };
    let encoded = serde_json::to_vec(&marker)
        .map_err(|error| format!("failed to serialize reset marker: {error}"))?;
    std::fs::write(reset_marker_path(), encoded)
        .map_err(|error| format!("failed to write reset marker: {error}"))
}

fn remove_reset_directory(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn perform_pending_reset(layout: &StorageLayout) -> Result<(), String> {
    let marker = reset_marker_path();
    if !marker.is_file() {
        return Ok(());
    }

    let pending: ResetMarker = serde_json::from_slice(
        &std::fs::read(&marker).map_err(|error| format!("failed to read reset marker: {error}"))?,
    )
    .map_err(|error| format!("invalid reset marker: {error}"))?;
    if pending.version != 1 || pending.root != layout.root || pending.webview != layout.webview {
        // A different installed/portable/debug instance owns this marker. It
        // must remain pending until that same storage layout is launched.
        return Ok(());
    }

    if layout.root.exists() {
        let sentinel = std::fs::read_to_string(layout.root.join(DATA_ROOT_SENTINEL_FILE))
            .map_err(|error| format!("refusing to clear unverified data root: {error}"))?;
        if sentinel.trim() != APP_IDENTIFIER {
            return Err("refusing to clear a data root not owned by Hachimi".into());
        }
        remove_reset_directory(&layout.root)
            .map_err(|error| format!("failed to clear {}: {error}", layout.root.display()))?;
    }
    if !layout.webview.starts_with(&layout.root) {
        remove_reset_directory(&layout.webview)
            .map_err(|error| format!("failed to clear {}: {error}", layout.webview.display()))?;
    }
    SystemApiKeyStore
        .clear()
        .map_err(|error| format!("failed to clear Hachimi credentials: {error}"))?;
    std::fs::remove_file(&marker)
        .map_err(|error| format!("failed to consume reset marker: {error}"))?;
    Ok(())
}

fn configure_webview_storage(layout: &StorageLayout) {
    if !layout.redirect_webview {
        return;
    }
    // This runs at the very beginning of main, before Tauri/WebView2 creates
    // threads or reads its environment. That makes changing the process-local
    // WebView2 data root safe for Debug and explicit portable runs.
    unsafe {
        std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", &layout.webview);
    }
}

#[derive(Debug)]
struct ActivePetRun {
    run_id: String,
    cancellation: CancellationToken,
}

#[derive(Debug)]
struct PendingAvatarImport {
    owner: ClientId,
    source: PathBuf,
    inspection: InspectedAvatar,
    expires_at: Instant,
}

#[derive(Debug)]
struct PendingVoiceImport {
    owner: ClientId,
    source: PathBuf,
    inspection: InspectedVoiceModel,
    expires_at: Instant,
}

#[derive(Debug)]
struct PendingMotionImport {
    owner: ClientId,
    source: PathBuf,
    inspection: InspectedMotion,
    expires_at: Instant,
}

fn consume_pending_avatar_import(
    imports: &mut BTreeMap<String, PendingAvatarImport>,
    token: &str,
    owner: &ClientId,
    now: Instant,
) -> Option<PendingAvatarImport> {
    imports.retain(|_, value| value.expires_at > now);
    if imports.get(token).is_none_or(|value| &value.owner != owner) {
        return None;
    }
    imports.remove(token)
}

fn cancel_pending_avatar_import(
    imports: &mut BTreeMap<String, PendingAvatarImport>,
    token: &str,
    owner: &ClientId,
    now: Instant,
) -> bool {
    imports.retain(|_, value| value.expires_at > now);
    if imports
        .get(token)
        .is_some_and(|value| &value.owner == owner)
    {
        imports.remove(token);
        true
    } else {
        false
    }
}

fn avatar_source_is_unchanged(previous: &InspectedAvatar, current: &InspectedAvatar) -> bool {
    current.sha256 == previous.sha256
        && current.size_bytes == previous.size_bytes
        && current.modified_millis == previous.modified_millis
        && current.is_compatible()
}

fn consume_pending_motion_import(
    imports: &mut BTreeMap<String, PendingMotionImport>,
    token: &str,
    owner: &ClientId,
    now: Instant,
) -> Option<PendingMotionImport> {
    imports.retain(|_, value| value.expires_at > now);
    if imports.get(token).is_none_or(|value| &value.owner != owner) {
        return None;
    }
    imports.remove(token)
}

fn cancel_pending_motion_import(
    imports: &mut BTreeMap<String, PendingMotionImport>,
    token: &str,
    owner: &ClientId,
    now: Instant,
) -> bool {
    imports.retain(|_, value| value.expires_at > now);
    if imports
        .get(token)
        .is_some_and(|value| &value.owner == owner)
    {
        imports.remove(token);
        true
    } else {
        false
    }
}

fn consume_pending_voice_import(
    imports: &mut BTreeMap<String, PendingVoiceImport>,
    token: &str,
    owner: &ClientId,
    now: Instant,
) -> Option<PendingVoiceImport> {
    imports.retain(|_, value| value.expires_at > now);
    if imports.get(token).is_none_or(|value| &value.owner != owner) {
        return None;
    }
    imports.remove(token)
}

fn cancel_pending_voice_import(
    imports: &mut BTreeMap<String, PendingVoiceImport>,
    token: &str,
    owner: &ClientId,
    now: Instant,
) -> bool {
    imports.retain(|_, value| value.expires_at > now);
    if imports
        .get(token)
        .is_some_and(|value| &value.owner == owner)
    {
        imports.remove(token);
        true
    } else {
        false
    }
}

fn voice_source_is_unchanged(
    previous: &InspectedVoiceModel,
    current: &InspectedVoiceModel,
) -> bool {
    current.sha256 == previous.sha256
        && current.size_bytes == previous.size_bytes
        && current.modified_millis == previous.modified_millis
        && current.compatible
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
struct CommandError {
    code: String,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PetWindowMotionEvent {
    x: i32,
    y: i32,
    velocity_x: f32,
    velocity_y: f32,
}

impl CommandError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    fn operation(code: &'static str, error: impl std::fmt::Display) -> Self {
        Self::new(code, error.to_string())
    }
}

impl From<tauri::Error> for CommandError {
    fn from(error: tauri::Error) -> Self {
        Self::operation("tauri_error", error)
    }
}

struct DesktopState {
    storage_layout: StorageLayout,
    settings: RwLock<AppSettings>,
    settings_store: SettingsStore,
    api_key_store: SystemApiKeyStore,
    avatar_catalog: RwLock<AvatarCatalog>,
    pending_avatar_imports: Mutex<BTreeMap<String, PendingAvatarImport>>,
    motion_catalog: RwLock<MotionCatalog>,
    pending_motion_imports: Mutex<BTreeMap<String, PendingMotionImport>>,
    voice_catalog: RwLock<VoiceCatalog>,
    pending_voice_imports: Mutex<BTreeMap<String, PendingVoiceImport>>,
    voice_runtime: VoiceRuntime,
    speech_recognizer: SpeechRecognizerRuntime,
    frontend_log: FrontendLog,
    pet_run: Mutex<Option<ActivePetRun>>,
    control_plane: ControlPlane,
    interactive_regions: RwLock<InteractiveRegionState>,
    click_through: AtomicBool,
    pet_hidden_by_user: AtomicBool,
    placement_revision: AtomicU64,
}

struct TrayMenuState {
    visibility: MenuItem<Wry>,
    send_message: MenuItem<Wry>,
    workbench: MenuItem<Wry>,
    llm_settings: MenuItem<Wry>,
    avatar_settings: MenuItem<Wry>,
    voice_settings: MenuItem<Wry>,
    interaction_settings: MenuItem<Wry>,
    always_on_top: CheckMenuItem<Wry>,
    exit: MenuItem<Wry>,
}

struct PetContextMenuState {
    menu: Menu<Wry>,
    send_message: MenuItem<Wry>,
    hide: MenuItem<Wry>,
    workbench: MenuItem<Wry>,
    llm_settings: MenuItem<Wry>,
    avatar_settings: MenuItem<Wry>,
    voice_settings: MenuItem<Wry>,
    interaction_settings: MenuItem<Wry>,
    always_on_top: CheckMenuItem<Wry>,
    exit: MenuItem<Wry>,
}

#[derive(Debug)]
struct FrontendLog {
    file: Mutex<File>,
}

impl FrontendLog {
    fn open(log_dir: &Path) -> std::io::Result<Self> {
        Ok(Self {
            file: Mutex::new(open_append_log(log_dir.join("hachimi-frontend.log"))?),
        })
    }

    fn write(&self, window_label: &str, entry: &FrontendLogEntry) -> std::io::Result<()> {
        let level = match entry.level {
            FrontendLogLevel::Info => "INFO",
            FrontendLogLevel::Warn => "WARN",
            FrontendLogLevel::Error => "ERROR",
        };
        let message = sanitize_log_message(&entry.message);
        let mut file = self.file.lock();
        writeln!(
            file,
            "[{}] [{level}] [{window_label}] {message}",
            epoch_millis()
        )?;
        file.flush()
    }
}

impl DesktopState {
    fn authorize(
        &self,
        window: &WebviewWindow,
        method: ControlMethod,
    ) -> Result<ClientContext, CommandError> {
        let kind = WindowKind::from_label(window.label()).ok_or_else(|| {
            CommandError::new(
                "unknown_window",
                format!("unknown window: {}", window.label()),
            )
        })?;
        let client = ClientContext::for_window(kind);
        self.control_plane
            .authorize(&client, method)
            .map_err(|error| CommandError::new(format!("{:?}", error.code), error.message))?;
        Ok(client)
    }

    fn save_settings(&self) -> Result<(), CommandError> {
        self.settings_store
            .save(&self.settings.read())
            .map_err(|error| CommandError::operation("settings_save_failed", error))
    }

    fn llm_view(&self) -> Result<LlmSettingsView, CommandError> {
        let configured = self
            .api_key_store
            .is_configured()
            .map_err(|error| CommandError::operation("secret_store_failed", error))?;
        Ok(LlmSettingsView::from_settings(
            &self.settings.read().llm,
            configured,
        ))
    }
}

#[tauri::command]
fn get_bootstrap_state(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<BootstrapState, CommandError> {
    let client = state.authorize(&window, ControlMethod::SystemBootstrap)?;
    let settings = state.settings.read();
    Ok(BootstrapState {
        protocol_version: CONTROL_PROTOCOL_VERSION,
        window_kind: client.window_kind,
        locale: settings.locale,
        theme: settings.theme,
        appearance: settings.appearance.clone(),
        always_on_top: settings.always_on_top,
        feature_flags: state.control_plane.feature_flags(),
    })
}

#[tauri::command]
fn frontend_ready(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<(), CommandError> {
    state.authorize(&window, ControlMethod::SystemBootstrap)?;
    window.show()?;
    refresh_tray_menu(&app);
    Ok(())
}

#[tauri::command]
fn write_frontend_log(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    entry: FrontendLogEntry,
) -> Result<(), CommandError> {
    state.authorize(&window, ControlMethod::SystemBootstrap)?;
    state
        .frontend_log
        .write(window.label(), &entry)
        .map_err(|error| CommandError::operation("frontend_log_failed", error))
}

#[tauri::command]
fn get_settings(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<AppSettings, CommandError> {
    state.authorize(&window, ControlMethod::SettingsRead)?;
    Ok(state.settings.read().clone())
}

#[tauri::command]
fn update_settings(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    settings: AppSettings,
) -> Result<AppSettings, CommandError> {
    state.authorize(&window, ControlMethod::SettingsWrite)?;
    validate_app_settings(&settings)?;
    let current_voice_asset = state.voice_catalog.read().current_asset();
    state
        .voice_runtime
        .update_settings(
            settings.voice.speed_percent,
            settings.voice.compute_mode,
            current_voice_asset,
        )
        .map_err(|error| CommandError::operation("invalid_voice_settings", error))?;
    state
        .settings_store
        .save(&settings)
        .map_err(|error| CommandError::operation("settings_save_failed", error))?;
    *state.settings.write() = settings.clone();
    state.voice_runtime.set_muted(settings.voice.muted);
    if let Some(pet) = app.get_webview_window("pet") {
        pet.set_always_on_top(settings.always_on_top)?;
    }
    let _ = app.emit("settings-changed", &settings);
    refresh_tray_menu(&app);
    Ok(settings)
}

#[tauri::command]
fn reset_local_data(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<(), CommandError> {
    state.authorize(&window, ControlMethod::SettingsWrite)?;
    write_reset_marker(&state.storage_layout)
        .map_err(|error| CommandError::operation("reset_marker_failed", error))?;
    if let Err(error) = state.api_key_store.clear() {
        tracing::warn!(%error, "credential cleanup deferred until next launch");
    }
    state.voice_runtime.stop();
    if let Some(active) = state.pet_run.lock().take() {
        active.cancellation.cancel();
    }
    app.exit(0);
    Ok(())
}

fn validate_app_settings(settings: &AppSettings) -> Result<(), CommandError> {
    if settings.schema_version != SETTINGS_SCHEMA_VERSION {
        return Err(CommandError::new(
            "invalid_settings_schema",
            format!(
                "settings schema must be {SETTINGS_SCHEMA_VERSION}, got {}",
                settings.schema_version
            ),
        ));
    }
    settings
        .appearance
        .validate()
        .map_err(|error| CommandError::new("invalid_appearance", error))?;
    Ok(())
}

fn save_appearance_settings(
    app: &AppHandle,
    state: &DesktopState,
    settings: AppSettings,
) -> Result<AppSettings, CommandError> {
    validate_app_settings(&settings)?;
    state
        .settings_store
        .save(&settings)
        .map_err(|error| CommandError::operation("settings_save_failed", error))?;
    *state.settings.write() = settings.clone();
    let _ = app.emit("settings-changed", &settings);
    Ok(settings)
}

#[tauri::command]
fn import_theme_profile(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    scheme: ThemeScheme,
) -> Result<Option<AppSettings>, CommandError> {
    state.authorize(&window, ControlMethod::SettingsWrite)?;
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Hachimi theme", &["json"])
        .pick_file()
    else {
        return Ok(None);
    };
    let metadata = fs::metadata(&path)
        .map_err(|error| CommandError::operation("theme_import_failed", error))?;
    if metadata.len() > MAX_THEME_FILE_BYTES {
        return Err(CommandError::new(
            "theme_file_too_large",
            "theme file must not exceed 64 KiB",
        ));
    }
    let bytes =
        fs::read(&path).map_err(|error| CommandError::operation("theme_import_failed", error))?;
    if bytes.len() as u64 > MAX_THEME_FILE_BYTES {
        return Err(CommandError::new(
            "theme_file_too_large",
            "theme file must not exceed 64 KiB",
        ));
    }
    let mut document: ThemeProfileDocument = serde_json::from_slice(&bytes)
        .map_err(|error| CommandError::operation("invalid_theme_file", error))?;
    document
        .validate()
        .map_err(|error| CommandError::new("invalid_theme_file", error))?;
    if document.profile.scheme != scheme {
        return Err(CommandError::new(
            "theme_scheme_mismatch",
            "the imported theme has a different color scheme",
        ));
    }

    let mut settings = state.settings.read().clone();
    if settings.appearance.themes.len() >= MAX_THEME_PROFILES {
        return Err(CommandError::new(
            "theme_limit_reached",
            format!("no more than {MAX_THEME_PROFILES} themes can be stored"),
        ));
    }
    document.profile.id = format!("theme-{}", uuid::Uuid::new_v4().simple());
    document.profile.builtin = false;
    let selected_id = document.profile.id.clone();
    settings.appearance.themes.push(document.profile);
    settings.appearance.set_selected_id(scheme, selected_id);
    save_appearance_settings(&app, &state, settings).map(Some)
}

#[tauri::command]
fn copy_theme_profile(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    profile_id: String,
) -> Result<(), CommandError> {
    state.authorize(&window, ControlMethod::SettingsRead)?;
    let settings = state.settings.read();
    let profile = settings
        .appearance
        .profile(&profile_id)
        .cloned()
        .ok_or_else(|| CommandError::new("theme_not_found", "theme profile was not found"))?;
    let json = serde_json::to_string_pretty(&ThemeProfileDocument::new(profile))
        .map_err(|error| CommandError::operation("theme_copy_failed", error))?;
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.set_text(json))
        .map_err(|error| CommandError::operation("theme_copy_failed", error))
}

#[tauri::command]
fn reset_theme_profile(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    profile_id: String,
) -> Result<AppSettings, CommandError> {
    state.authorize(&window, ControlMethod::SettingsWrite)?;
    let mut settings = state.settings.read().clone();
    reset_theme_in_settings(&mut settings, &profile_id)?;
    save_appearance_settings(&app, &state, settings)
}

fn reset_theme_in_settings(
    settings: &mut AppSettings,
    profile_id: &str,
) -> Result<(), CommandError> {
    let default = ThemeProfile::builtin_by_id(profile_id).ok_or_else(|| {
        CommandError::new("theme_not_builtin", "only built-in themes can be reset")
    })?;
    let profile = settings
        .appearance
        .themes
        .iter_mut()
        .find(|profile| profile.id == profile_id && profile.builtin)
        .ok_or_else(|| CommandError::new("theme_not_found", "built-in theme was not found"))?;
    *profile = default;
    Ok(())
}

#[tauri::command]
fn delete_theme_profile(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    profile_id: String,
) -> Result<AppSettings, CommandError> {
    state.authorize(&window, ControlMethod::SettingsWrite)?;
    let mut settings = state.settings.read().clone();
    delete_theme_in_settings(&mut settings, &profile_id)?;
    save_appearance_settings(&app, &state, settings)
}

fn delete_theme_in_settings(
    settings: &mut AppSettings,
    profile_id: &str,
) -> Result<(), CommandError> {
    let profile = settings
        .appearance
        .profile(profile_id)
        .cloned()
        .ok_or_else(|| CommandError::new("theme_not_found", "theme profile was not found"))?;
    if profile.builtin {
        return Err(CommandError::new(
            "theme_is_builtin",
            "built-in themes cannot be deleted",
        ));
    }
    settings
        .appearance
        .themes
        .retain(|candidate| candidate.id != profile_id);
    if settings.appearance.selected_id(profile.scheme) == profile_id {
        let fallback = match profile.scheme {
            ThemeScheme::Light => "codex-light",
            ThemeScheme::Dark => "codex-dark",
        };
        settings
            .appearance
            .set_selected_id(profile.scheme, fallback.into());
    }
    settings
        .appearance
        .validate()
        .map_err(|error| CommandError::new("invalid_appearance", error))?;
    Ok(())
}

#[tauri::command]
fn set_interactive_regions(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    update: InteractiveRegionsUpdate,
) -> Result<(), CommandError> {
    state.authorize(&window, ControlMethod::WindowInteract)?;
    if update.window_label != window.label() {
        return Err(CommandError::new(
            "window_mismatch",
            "interactive regions cannot target another window",
        ));
    }
    state
        .interactive_regions
        .write()
        .update(update)
        .map_err(|error| CommandError::operation("invalid_regions", error))
}

#[tauri::command]
fn set_always_on_top(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    enabled: bool,
) -> Result<AppSettings, CommandError> {
    let kind = WindowKind::from_label(window.label())
        .ok_or_else(|| CommandError::new("unknown_window", "unknown window label"))?;
    let method = match kind {
        WindowKind::Pet => ControlMethod::WindowInteract,
        WindowKind::Settings | WindowKind::Workbench => ControlMethod::SettingsWrite,
    };
    state.authorize(&window, method)?;
    update_always_on_top(&app, &state, enabled)
}

fn update_always_on_top(
    app: &AppHandle,
    state: &DesktopState,
    enabled: bool,
) -> Result<AppSettings, CommandError> {
    if let Some(pet) = app.get_webview_window("pet") {
        pet.set_always_on_top(enabled)?;
    }
    state.settings.write().always_on_top = enabled;
    state.save_settings()?;
    let settings = state.settings.read().clone();
    let _ = app.emit("settings-changed", &settings);
    refresh_tray_menu(app);
    Ok(settings)
}

#[tauri::command]
fn start_pet_dragging(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<(), CommandError> {
    state.authorize(&window, ControlMethod::WindowInteract)?;
    require_window(&window, "pet")?;
    window.start_dragging()?;
    Ok(())
}

#[tauri::command]
async fn open_workbench(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    route: WorkbenchRoute,
) -> Result<(), CommandError> {
    state.authorize(&window, ControlMethod::WorkbenchOpen)?;
    // Creating a second WebView synchronously from the Pet's IPC callback can
    // keep WebView2 inside the initiating input dispatch. The async boundary
    // lets that dispatch unwind before native window creation/focus begins.
    tokio::task::yield_now().await;
    open_workbench_route(&app, &state, route)
}

fn open_workbench_route(
    app: &AppHandle,
    state: &DesktopState,
    route: WorkbenchRoute,
) -> Result<(), CommandError> {
    if let Some(workbench) = app.get_webview_window("workbench") {
        workbench.unminimize()?;
        workbench.show()?;
        workbench.set_focus()?;
        app.emit_to("workbench", "workbench:navigate", route)?;
        enter_workbench_mode(app, state);
        return Ok(());
    }
    create_workbench_window(app, route, &state.storage_layout.webview)?;
    enter_workbench_mode(app, state);
    Ok(())
}

#[tauri::command]
fn hide_workbench(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<(), CommandError> {
    state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    window.hide()?;
    restore_pet(&app);
    Ok(())
}

#[tauri::command]
fn minimize_workbench(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<(), CommandError> {
    state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    window.minimize()?;
    restore_pet(&app);
    Ok(())
}

#[tauri::command]
fn toggle_maximize_workbench(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<(), CommandError> {
    state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    if window.is_maximized()? {
        window.unmaximize()?;
    } else {
        window.maximize()?;
    }
    Ok(())
}

#[tauri::command]
fn start_workbench_dragging(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<(), CommandError> {
    state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    window.start_dragging()?;
    Ok(())
}

#[tauri::command]
fn start_workbench_resize(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    direction: String,
) -> Result<(), CommandError> {
    state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    let direction = match direction.as_str() {
        "north" => ResizeDirection::North,
        "north-east" => ResizeDirection::NorthEast,
        "east" => ResizeDirection::East,
        "south-east" => ResizeDirection::SouthEast,
        "south" => ResizeDirection::South,
        "south-west" => ResizeDirection::SouthWest,
        "west" => ResizeDirection::West,
        "north-west" => ResizeDirection::NorthWest,
        _ => {
            return Err(CommandError::new(
                "invalid_resize_direction",
                "unknown resize direction",
            ));
        }
    };
    window.as_ref().window().start_resize_dragging(direction)?;
    Ok(())
}

#[tauri::command]
fn get_llm_settings(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<LlmSettingsView, CommandError> {
    state.authorize(&window, ControlMethod::LlmRead)?;
    state.llm_view()
}

fn save_llm(
    app: &AppHandle,
    state: &DesktopState,
    input: &LlmSettingsInput,
) -> Result<LlmSettingsView, CommandError> {
    let settings = validate_input(input)
        .map_err(|error| CommandError::operation("invalid_llm_settings", error))?;
    {
        let mut app_settings = state.settings.write();
        let previous = app_settings.llm.clone();
        app_settings.llm = settings;
        if let Err(error) = state.settings_store.save(&app_settings) {
            app_settings.llm = previous;
            return Err(CommandError::operation("settings_save_failed", error));
        }
    }
    apply_secret_change(&state.api_key_store, input)
        .map_err(|error| CommandError::operation("secret_store_failed", error))?;
    let app_settings = state.settings.read().clone();
    let _ = app.emit("settings-changed", &app_settings);
    state.llm_view()
}

#[tauri::command]
fn save_llm_settings(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    input: LlmSettingsInput,
) -> Result<LlmSettingsView, CommandError> {
    state.authorize(&window, ControlMethod::LlmWrite)?;
    save_llm(&app, &state, &input)
}

#[tauri::command]
async fn save_and_test_llm_settings(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    input: LlmSettingsInput,
) -> Result<LlmTestResult, CommandError> {
    state.authorize(&window, ControlMethod::LlmWrite)?;
    state.authorize(&window, ControlMethod::LlmTest)?;
    save_llm(&app, &state, &input)?;
    let settings = state.settings.read().llm.clone();
    let secret = state
        .api_key_store
        .get()
        .map_err(|error| CommandError::operation("secret_store_failed", error))?;
    test_connection(&settings, secret.as_deref())
        .await
        .map_err(|error| CommandError::operation("llm_test_failed", error))
}

#[tauri::command]
fn list_avatar_models(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<AvatarCatalogSnapshot, CommandError> {
    state.authorize(&window, ControlMethod::AvatarRead)?;
    Ok(state.avatar_catalog.read().snapshot())
}

#[tauri::command]
fn inspect_avatar_model(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Option<AvatarImportInspection>, CommandError> {
    let client = state.authorize(&window, ControlMethod::AvatarManage)?;
    let Some(source) = rfd::FileDialog::new()
        .set_title("检测 Runtime Ready VRM 角色模型")
        .add_filter("VRM 角色模型", &["vrm"])
        .pick_file()
    else {
        return Ok(None);
    };
    let inspection = inspect_avatar(&source)
        .map_err(|error| CommandError::operation("avatar_inspection_failed", error))?;
    let token = inspection
        .is_compatible()
        .then(|| Uuid::new_v4().to_string());
    if let Some(token) = &token {
        let mut pending = state.pending_avatar_imports.lock();
        let now = Instant::now();
        pending.retain(|_, value| value.expires_at > now);
        pending.insert(
            token.clone(),
            PendingAvatarImport {
                owner: client.client_id,
                source,
                inspection: inspection.clone(),
                expires_at: now + AVATAR_IMPORT_TOKEN_TTL,
            },
        );
    }
    Ok(Some(inspection.view(token)))
}

#[tauri::command]
fn commit_avatar_model_import(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: AvatarImportCommitRequest,
) -> Result<AvatarCatalogSnapshot, CommandError> {
    let client = state.authorize(&window, ControlMethod::AvatarManage)?;
    let pending = {
        let mut imports = state.pending_avatar_imports.lock();
        let Some(pending) = consume_pending_avatar_import(
            &mut imports,
            &request.token,
            &client.client_id,
            Instant::now(),
        ) else {
            return Err(CommandError::new(
                "avatar_import_token_invalid",
                "模型导入已过期，请重新选择文件",
            ));
        };
        pending
    };
    let refreshed = inspect_avatar(&pending.source)
        .map_err(|error| CommandError::operation("avatar_import_source_changed", error))?;
    if !avatar_source_is_unchanged(&pending.inspection, &refreshed) {
        return Err(CommandError::new(
            "avatar_import_source_changed",
            "源模型在检测后发生变化，请重新选择文件",
        ));
    }
    let snapshot = state
        .avatar_catalog
        .write()
        .import_inspected(&request.name, &pending.source, &refreshed)
        .map_err(|error| CommandError::operation("avatar_import_failed", error))?;
    let _ = app.emit(AVATAR_CATALOG_EVENT, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
fn cancel_avatar_model_import(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    token: String,
) -> Result<(), CommandError> {
    let client = state.authorize(&window, ControlMethod::AvatarManage)?;
    let mut imports = state.pending_avatar_imports.lock();
    if !cancel_pending_avatar_import(&mut imports, &token, &client.client_id, Instant::now()) {
        return Err(CommandError::new(
            "avatar_import_token_invalid",
            "模型导入已过期或不属于当前窗口",
        ));
    }
    Ok(())
}

#[tauri::command]
fn select_avatar_model(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ResourceEntryRequest,
) -> Result<AvatarCatalogSnapshot, CommandError> {
    state.authorize(&window, ControlMethod::AvatarManage)?;
    let changes_current =
        state.avatar_catalog.read().snapshot().current_id.as_deref() != Some(request.id.as_str());
    let snapshot = state
        .avatar_catalog
        .write()
        .select(&request.id)
        .map_err(|error| CommandError::operation("avatar_select_failed", error))?;
    if changes_current {
        state.voice_runtime.stop();
    }
    let _ = app.emit(AVATAR_CATALOG_EVENT, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
fn delete_avatar_model(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ResourceEntryRequest,
) -> Result<AvatarCatalogSnapshot, CommandError> {
    state.authorize(&window, ControlMethod::AvatarManage)?;
    let deletes_current =
        state.avatar_catalog.read().snapshot().current_id.as_deref() == Some(request.id.as_str());
    let snapshot = state
        .avatar_catalog
        .write()
        .delete(&request.id)
        .map_err(|error| CommandError::operation("avatar_delete_failed", error))?;
    if deletes_current {
        state.voice_runtime.stop();
    }
    let _ = app.emit(AVATAR_CATALOG_EVENT, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
fn get_current_avatar_asset(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Option<AvatarRuntimeAsset>, CommandError> {
    match window.label() {
        "pet" => {
            state.authorize(&window, ControlMethod::AvatarRuntime)?;
        }
        "workbench" => {
            state.authorize(&window, ControlMethod::AvatarRead)?;
        }
        _ => return Err(CommandError::new("unknown_window", "不允许的窗口")),
    }
    Ok(state.avatar_catalog.read().current_asset().map(|asset| {
        let entry_id = asset.entry.id;
        let asset_url = avatar_asset_url(&entry_id);
        AvatarRuntimeAsset {
            entry_id,
            name: asset.entry.name,
            sha256: asset.entry.sha256,
            asset_url,
            format: asset.entry.format,
            profile: asset.profile,
        }
    }))
}

#[tauri::command]
fn get_avatar_runtime_asset(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ResourceEntryRequest,
) -> Result<Option<AvatarRuntimeAsset>, CommandError> {
    state.authorize(&window, ControlMethod::AvatarRead)?;
    let asset = state.avatar_catalog.read().asset_for(&request.id);
    Ok(asset.map(|asset| {
        let entry_id = asset.entry.id;
        let asset_url = avatar_asset_url(&entry_id);
        AvatarRuntimeAsset {
            entry_id,
            name: asset.entry.name,
            sha256: asset.entry.sha256,
            asset_url,
            format: asset.entry.format,
            profile: asset.profile,
        }
    }))
}

#[tauri::command]
fn list_motion_catalog(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<MotionCatalogSnapshot, CommandError> {
    match window.label() {
        "pet" => {
            state.authorize(&window, ControlMethod::MotionRuntime)?;
        }
        "workbench" => {
            state.authorize(&window, ControlMethod::MotionRead)?;
        }
        _ => return Err(CommandError::new("unknown_window", "不允许的窗口")),
    }
    Ok(state.motion_catalog.read().snapshot())
}

#[tauri::command]
fn inspect_motion_file(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Option<MotionImportInspection>, CommandError> {
    let client = state.authorize(&window, ControlMethod::MotionManage)?;
    let Some(source) = rfd::FileDialog::new()
        .set_title("检测 VRMA 1.0 动作")
        .add_filter("VRM Animation", &["vrma"])
        .pick_file()
    else {
        return Ok(None);
    };
    let inspection = inspect_motion(&source)
        .map_err(|error| CommandError::operation("motion_inspection_failed", error))?;
    let token = Uuid::new_v4().to_string();
    let view = inspection.view(Some(token.clone()));
    let mut pending = state.pending_motion_imports.lock();
    let now = Instant::now();
    pending.retain(|_, value| value.expires_at > now);
    pending.insert(
        token,
        PendingMotionImport {
            owner: client.client_id,
            source,
            inspection,
            expires_at: now + MOTION_IMPORT_TOKEN_TTL,
        },
    );
    Ok(Some(view))
}

#[tauri::command]
fn commit_motion_import(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: MotionImportCommitRequest,
) -> Result<MotionCatalogSnapshot, CommandError> {
    let client = state.authorize(&window, ControlMethod::MotionManage)?;
    let pending = consume_pending_motion_import(
        &mut state.pending_motion_imports.lock(),
        &request.token,
        &client.client_id,
        Instant::now(),
    )
    .ok_or_else(|| {
        CommandError::new(
            "motion_import_token_invalid",
            "动作导入已过期，请重新选择文件",
        )
    })?;
    let snapshot = state
        .motion_catalog
        .write()
        .import_inspected(&pending.source, &pending.inspection, &request)
        .map_err(|error| CommandError::operation("motion_import_failed", error))?;
    let _ = app.emit(MOTION_CATALOG_EVENT, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
fn cancel_motion_import(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    token: String,
) -> Result<(), CommandError> {
    let client = state.authorize(&window, ControlMethod::MotionManage)?;
    if !cancel_pending_motion_import(
        &mut state.pending_motion_imports.lock(),
        &token,
        &client.client_id,
        Instant::now(),
    ) {
        return Err(CommandError::new(
            "motion_import_token_invalid",
            "动作导入已过期或不属于当前窗口",
        ));
    }
    Ok(())
}

#[tauri::command]
fn update_motion_metadata(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: MotionMetadataUpdateRequest,
) -> Result<MotionCatalogSnapshot, CommandError> {
    state.authorize(&window, ControlMethod::MotionManage)?;
    let snapshot = state
        .motion_catalog
        .write()
        .update_metadata(&request)
        .map_err(|error| CommandError::operation("motion_update_failed", error))?;
    let _ = app.emit(MOTION_CATALOG_EVENT, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
fn delete_user_motion(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ResourceEntryRequest,
) -> Result<MotionCatalogSnapshot, CommandError> {
    state.authorize(&window, ControlMethod::MotionManage)?;
    let snapshot = state
        .motion_catalog
        .write()
        .delete_user(&request.id)
        .map_err(|error| CommandError::operation("motion_delete_failed", error))?;
    let _ = app.emit(MOTION_CATALOG_EVENT, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
fn set_interaction_motion_binding(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: InteractionMotionBindingUpdateRequest,
) -> Result<MotionCatalogSnapshot, CommandError> {
    state.authorize(&window, ControlMethod::MotionManage)?;
    let snapshot = state
        .motion_catalog
        .write()
        .update_binding(&request)
        .map_err(|error| CommandError::operation("motion_bindings_invalid", error))?;
    let _ = app.emit(MOTION_CATALOG_EVENT, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
fn clear_motion_interaction_bindings(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: MotionAssetBindingsClearRequest,
) -> Result<MotionCatalogSnapshot, CommandError> {
    state.authorize(&window, ControlMethod::MotionManage)?;
    let snapshot = state
        .motion_catalog
        .write()
        .clear_motion_bindings(&request)
        .map_err(|error| CommandError::operation("motion_bindings_clear_failed", error))?;
    let _ = app.emit(MOTION_CATALOG_EVENT, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
fn set_motion_enabled(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: MotionEnabledUpdateRequest,
) -> Result<MotionCatalogSnapshot, CommandError> {
    state.authorize(&window, ControlMethod::MotionManage)?;
    let snapshot = state
        .motion_catalog
        .write()
        .set_motion_enabled(&request)
        .map_err(|error| CommandError::operation("motion_enabled_update_failed", error))?;
    let _ = app.emit(MOTION_CATALOG_EVENT, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
fn reset_motion_bindings(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<MotionCatalogSnapshot, CommandError> {
    state.authorize(&window, ControlMethod::MotionManage)?;
    let snapshot = state
        .motion_catalog
        .write()
        .reset_bindings()
        .map_err(|error| CommandError::operation("motion_bindings_reset_failed", error))?;
    let _ = app.emit(MOTION_CATALOG_EVENT, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
fn reset_motion_binding(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: MotionBindingResetRequest,
) -> Result<MotionCatalogSnapshot, CommandError> {
    state.authorize(&window, ControlMethod::MotionManage)?;
    let snapshot = state
        .motion_catalog
        .write()
        .reset_binding(request.region)
        .map_err(|error| CommandError::operation("motion_binding_reset_failed", error))?;
    let _ = app.emit(MOTION_CATALOG_EVENT, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
fn get_motion_runtime_asset(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ResourceEntryRequest,
) -> Result<Option<MotionRuntimeAsset>, CommandError> {
    match window.label() {
        "pet" => {
            state.authorize(&window, ControlMethod::MotionRuntime)?;
        }
        "workbench" => {
            state.authorize(&window, ControlMethod::MotionRead)?;
        }
        _ => return Err(CommandError::new("unknown_window", "不允许的窗口")),
    }
    Ok(state
        .motion_catalog
        .read()
        .asset_for(&request.id)
        .map(|asset| {
            let asset_url = motion_asset_url(&asset.entry.id);
            MotionRuntimeAsset {
                entry: asset.entry,
                asset_url,
            }
        }))
}

fn profile_supports_pet_voice(profile: &AvatarAdaptationProfile) -> bool {
    !matches!(profile.lip_sync, LipSyncCapability::None)
}

#[tauri::command]
fn start_pet_turn(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    request: PetTurnRequest,
) -> Result<(), CommandError> {
    state.authorize(&window, ControlMethod::LlmChat)?;
    require_window(&window, "pet")?;
    let text = request.text.trim().to_owned();
    if text.is_empty() || text.chars().count() > 8_000 {
        return Err(CommandError::new(
            "invalid_pet_message",
            "消息长度必须为 1–8,000 个字符",
        ));
    }
    let settings = state.settings.read().llm.clone();
    let secret = state
        .api_key_store
        .get()
        .map_err(|error| CommandError::operation("secret_store_failed", error))?;
    cancel_pet_activity(&app, &state, false);
    let cancellation = CancellationToken::new();
    *state.pet_run.lock() = Some(ActivePetRun {
        run_id: request.run_id.clone(),
        cancellation: cancellation.clone(),
    });
    app.emit_to(
        "pet",
        PET_TURN_EVENT,
        PetTurnEvent::Started {
            run_id: request.run_id.clone(),
        },
    )?;
    let avatar_supports_lip_sync = state
        .avatar_catalog
        .read()
        .current_asset()
        .is_some_and(|asset| profile_supports_pet_voice(&asset.profile));
    let voice_streaming =
        avatar_supports_lip_sync && state.voice_runtime.begin_pet_turn(request.run_id.clone());

    let task_app = app.clone();
    let run_id = request.run_id;
    tauri::async_runtime::spawn(async move {
        let delta_app = task_app.clone();
        let delta_run_id = run_id.clone();
        let result = stream_pet_turn(
            &settings,
            secret.as_deref(),
            &text,
            &cancellation,
            move |delta| {
                if voice_streaming {
                    delta_app
                        .state::<DesktopState>()
                        .voice_runtime
                        .push_pet_delta(&delta_run_id, delta);
                } else {
                    let _ = delta_app.emit_to(
                        "pet",
                        PET_TURN_EVENT,
                        PetTurnEvent::TextDelta {
                            run_id: delta_run_id.clone(),
                            delta: delta.to_owned(),
                        },
                    );
                }
            },
        )
        .await;
        let state = task_app.state::<DesktopState>();
        let is_current = state
            .pet_run
            .lock()
            .as_ref()
            .is_some_and(|active| active.run_id == run_id);
        if !is_current {
            return;
        }
        state.pet_run.lock().take();
        match result {
            Ok(text) => {
                let speech_queued = voice_streaming && state.voice_runtime.finish_pet_turn(&run_id);
                let _ = task_app.emit_to(
                    "pet",
                    PET_TURN_EVENT,
                    PetTurnEvent::Completed {
                        run_id: run_id.clone(),
                        text,
                        speech_queued,
                    },
                );
            }
            Err(LlmError::Cancelled) => {
                state.voice_runtime.stop();
                let _ = task_app.emit_to("pet", PET_TURN_EVENT, PetTurnEvent::Cancelled { run_id });
            }
            Err(error) => {
                state.voice_runtime.stop();
                let _ = task_app.emit_to(
                    "pet",
                    PET_TURN_EVENT,
                    PetTurnEvent::Failed {
                        run_id,
                        code: "llm_request_failed".into(),
                        message: error.to_string(),
                    },
                );
            }
        }
    });
    Ok(())
}

#[tauri::command]
fn cancel_pet_turn(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<(), CommandError> {
    state.authorize(&window, ControlMethod::LlmChat)?;
    require_window(&window, "pet")?;
    cancel_pet_activity(&app, &state, true);
    Ok(())
}

#[tauri::command]
fn list_voice_models(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<VoiceCatalogSnapshot, CommandError> {
    state.authorize(&window, ControlMethod::VoiceRead)?;
    Ok(state.voice_catalog.read().snapshot())
}

#[tauri::command]
fn inspect_voice_model(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Option<VoiceModelInspection>, CommandError> {
    let client = state.authorize(&window, ControlMethod::VoiceManage)?;
    let Some(source) = rfd::FileDialog::new()
        .set_title("检测 sherpa-onnx VITS 模型")
        .add_filter("VITS 模型归档", &["bz2"])
        .pick_file()
    else {
        return Ok(None);
    };
    let inspection = inspect_voice_archive(&source)
        .map_err(|error| CommandError::operation("voice_inspection_failed", error))?;
    let token = inspection.compatible.then(|| Uuid::new_v4().to_string());
    if let Some(token) = &token {
        let mut pending = state.pending_voice_imports.lock();
        let now = Instant::now();
        pending.retain(|_, value| value.expires_at > now);
        pending.insert(
            token.clone(),
            PendingVoiceImport {
                owner: client.client_id,
                source,
                inspection: inspection.clone(),
                expires_at: now + VOICE_IMPORT_TOKEN_TTL,
            },
        );
    }
    Ok(Some(inspection.view(token)))
}

#[tauri::command]
fn commit_voice_model_import(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: VoiceImportCommitRequest,
) -> Result<VoiceCatalogSnapshot, CommandError> {
    let client = state.authorize(&window, ControlMethod::VoiceManage)?;
    let pending = {
        let mut imports = state.pending_voice_imports.lock();
        consume_pending_voice_import(
            &mut imports,
            &request.token,
            &client.client_id,
            Instant::now(),
        )
        .ok_or_else(|| {
            CommandError::new(
                "voice_import_token_invalid",
                "语音模型导入已过期，请重新选择文件",
            )
        })?
    };
    let refreshed = inspect_voice_archive(&pending.source)
        .map_err(|error| CommandError::operation("voice_import_source_changed", error))?;
    if !voice_source_is_unchanged(&pending.inspection, &refreshed) {
        return Err(CommandError::new(
            "voice_import_source_changed",
            "源语音模型在检测后发生变化，请重新选择文件",
        ));
    }
    let snapshot = state
        .voice_catalog
        .write()
        .import_inspected(
            &request.name,
            &pending.source,
            &refreshed,
            request.license_acknowledged,
            request.speaker_id,
        )
        .map_err(|error| CommandError::operation("voice_import_failed", error))?;
    let _ = app.emit(VOICE_CATALOG_EVENT, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
fn cancel_voice_model_import(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    token: String,
) -> Result<(), CommandError> {
    let client = state.authorize(&window, ControlMethod::VoiceManage)?;
    let mut imports = state.pending_voice_imports.lock();
    if !cancel_pending_voice_import(&mut imports, &token, &client.client_id, Instant::now()) {
        return Err(CommandError::new(
            "voice_import_token_invalid",
            "语音模型导入已过期或不属于当前窗口",
        ));
    }
    Ok(())
}

#[tauri::command]
fn select_voice_model(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ResourceEntryRequest,
) -> Result<VoiceCatalogSnapshot, CommandError> {
    state.authorize(&window, ControlMethod::VoiceManage)?;
    let (asset, previous_asset) = {
        let catalog = state.voice_catalog.read();
        let asset = catalog.asset(&request.id).ok_or_else(|| {
            CommandError::new("voice_model_not_found", "找不到可用的语音模型文件")
        })?;
        (asset, catalog.current_asset())
    };
    let mode = state.settings.read().voice.compute_mode;
    state
        .voice_runtime
        .load_model_with_rollback(Some(asset), previous_asset.clone(), mode)
        .map_err(|error| CommandError::operation("voice_model_warmup_failed", error))?;
    let snapshot = match state.voice_catalog.write().select(&request.id) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let _ = state.voice_runtime.load_model(previous_asset, mode);
            return Err(CommandError::operation("voice_model_select_failed", error));
        }
    };
    let runtime = state.voice_runtime.state();
    let _ = app.emit(VOICE_CATALOG_EVENT, &snapshot);
    let _ = app.emit("voice-runtime-changed", &runtime);
    Ok(snapshot)
}

#[tauri::command]
fn delete_voice_model(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ResourceEntryRequest,
) -> Result<VoiceCatalogSnapshot, CommandError> {
    state.authorize(&window, ControlMethod::VoiceManage)?;
    let was_current = state.voice_catalog.read().snapshot().current_id == request.id;
    let snapshot = state
        .voice_catalog
        .write()
        .delete(&request.id)
        .map_err(|error| CommandError::operation("voice_model_delete_failed", error))?;
    if was_current {
        let asset = state.voice_catalog.read().current_asset();
        let mode = state.settings.read().voice.compute_mode;
        let _ = state.voice_runtime.load_model(asset, mode);
    }
    let runtime = state.voice_runtime.state();
    let _ = app.emit(VOICE_CATALOG_EVENT, &snapshot);
    let _ = app.emit("voice-runtime-changed", &runtime);
    Ok(snapshot)
}

#[tauri::command]
fn get_voice_runtime_state(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<VoiceRuntimeState, CommandError> {
    state.authorize(&window, ControlMethod::VoicePlayback)?;
    Ok(state.voice_runtime.state())
}

#[tauri::command]
fn get_speech_recognition_state(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<SpeechRecognitionRuntimeState, CommandError> {
    state.authorize(&window, ControlMethod::VoiceRead)?;
    Ok(state.speech_recognizer.state())
}

#[tauri::command]
async fn update_speech_recognition_settings(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: SpeechRecognitionSettingsInput,
) -> Result<SpeechRecognitionRuntimeState, CommandError> {
    state.authorize(&window, ControlMethod::VoiceManage)?;
    require_window(&window, "workbench")?;
    let recognizer = state.speech_recognizer.clone();
    let compute_mode = input.compute_mode;
    let result = tokio::task::spawn_blocking(move || recognizer.update_compute_mode(compute_mode))
        .await
        .map_err(|error| CommandError::operation("speech_backend_update_failed", error))?
        .map_err(|error| CommandError::operation("speech_backend_update_failed", error))?;
    state.settings.write().voice.recognition_compute_mode = compute_mode;
    state.save_settings()?;
    let settings = state.settings.read().clone();
    let _ = app.emit("settings-changed", &settings);
    let _ = app.emit(SPEECH_RECOGNITION_STATE_EVENT, &result);
    Ok(result)
}

#[tauri::command]
fn update_voice_settings(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    input: VoiceSettingsInput,
) -> Result<VoiceRuntimeState, CommandError> {
    state.authorize(&window, ControlMethod::VoiceManage)?;
    tracing::info!(
        speed_percent = input.speed_percent,
        window = window.label(),
        compute_mode = ?input.compute_mode,
        "native VITS settings update requested"
    );
    let current_asset = state.voice_catalog.read().current_asset();
    state
        .voice_runtime
        .update_settings(input.speed_percent, input.compute_mode, current_asset)
        .map_err(|error| CommandError::operation("invalid_voice_settings", error))?;
    {
        let mut settings = state.settings.write();
        settings.voice.speed_percent = input.speed_percent;
        settings.voice.compute_mode = input.compute_mode;
    }
    state.save_settings()?;
    let settings = state.settings.read().clone();
    let runtime = state.voice_runtime.state();
    let _ = app.emit("settings-changed", &settings);
    let _ = app.emit("voice-runtime-changed", &runtime);
    Ok(runtime)
}

#[tauri::command]
fn set_muted(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    muted: bool,
) -> Result<VoiceRuntimeState, CommandError> {
    state.authorize(&window, ControlMethod::VoicePlayback)?;
    state.settings.write().voice.muted = muted;
    state.save_settings()?;
    state.voice_runtime.set_muted(muted);
    let settings = state.settings.read().clone();
    let runtime = state.voice_runtime.state();
    let _ = app.emit("settings-changed", &settings);
    let _ = app.emit("voice-runtime-changed", &runtime);
    Ok(runtime)
}

#[tauri::command]
fn preview_default_voice(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<VoiceRuntimeState, CommandError> {
    state.authorize(&window, ControlMethod::VoicePlayback)?;
    let state_view = state.voice_runtime.state();
    tracing::info!(
        speed_percent = state_view.speed_percent,
        muted = state_view.muted,
        window = window.label(),
        "voice preview requested"
    );
    let sample = "你好，我是 Hachimi。很高兴在桌面上陪着你。";
    state.voice_runtime.speak(sample);
    tracing::info!("native VITS voice preview queued");
    Ok(state.voice_runtime.state())
}

#[tauri::command]
fn stop_speech(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<VoiceRuntimeState, CommandError> {
    state.authorize(&window, ControlMethod::VoicePlayback)?;
    state.voice_runtime.stop();
    Ok(state.voice_runtime.state())
}

#[tauri::command]
async fn recognize_pet_speech(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<String, CommandError> {
    state.authorize(&window, ControlMethod::VoiceCapture)?;
    require_window(&window, "pet")?;
    let recognizer = state.speech_recognizer.clone();
    tokio::task::spawn_blocking(move || recognizer.recognize_once())
        .await
        .map_err(|error| CommandError::operation("speech_recognition_failed", error))?
        .map_err(|error| CommandError::operation("speech_recognition_failed", error))
}

#[cfg(any())]
mod legacy_windows_speech {
    use super::*;

    #[cfg(windows)]
    macro_rules! wait_for_windows_operation {
        ($operation:expr, $timeout:expr) => {{
            let started = std::time::Instant::now();
            loop {
                let status = $operation.Status().map_err(|error| {
                    CommandError::operation(
                        "speech_recognition_failed",
                        sanitize_windows_error(error),
                    )
                })?;
                match status.0 {
                    0 => {
                        if started.elapsed() >= $timeout {
                            let _ = $operation.Cancel();
                            break Err(CommandError::new(
                                "speech_recognition_timeout",
                                "语音识别超时，请检查麦克风后重试",
                            ));
                        }
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    1 => break Ok(()),
                    2 => {
                        break Err(CommandError::new(
                            "speech_recognition_cancelled",
                            "语音识别已取消",
                        ));
                    }
                    _ => {
                        break Err(CommandError::new(
                            "speech_recognition_failed",
                            "Windows 语音识别执行失败",
                        ));
                    }
                }
            }
        }};
    }

    #[cfg(windows)]
    fn recognize_windows_speech() -> Result<String, CommandError> {
        use windows::{
            Media::SpeechRecognition::{
                ISpeechRecognitionConstraint, SpeechRecognitionResultStatus,
                SpeechRecognitionScenario, SpeechRecognitionTopicConstraint, SpeechRecognizer,
            },
            Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
            core::{HSTRING, Interface},
        };

        struct ComApartment;
        impl Drop for ComApartment {
            fn drop(&mut self) {
                // SAFETY: paired with the successful CoInitializeEx call on this worker thread.
                unsafe { CoUninitialize() };
            }
        }
        // SAFETY: this is a dedicated blocking worker thread and the null reserved pointer is required.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .map_err(|error| {
                CommandError::operation(
                    "speech_recognition_unavailable",
                    sanitize_windows_error(error),
                )
            })?;
        let _apartment = ComApartment;
        let recognizer = SpeechRecognizer::new().map_err(|error| {
            CommandError::operation(
                "speech_recognition_unavailable",
                sanitize_windows_error(error),
            )
        })?;
        let topic = SpeechRecognitionTopicConstraint::Create(
            SpeechRecognitionScenario::Dictation,
            &HSTRING::from("hachimi-dictation"),
        )
        .map_err(|error| {
            CommandError::operation(
                "speech_recognition_unavailable",
                sanitize_windows_error(error),
            )
        })?;
        let constraint = topic
            .cast::<ISpeechRecognitionConstraint>()
            .map_err(|error| {
                CommandError::operation(
                    "speech_recognition_unavailable",
                    sanitize_windows_error(error),
                )
            })?;
        recognizer
            .Constraints()
            .and_then(|constraints| constraints.Append(&constraint))
            .map_err(|error| {
                CommandError::operation(
                    "speech_recognition_unavailable",
                    sanitize_windows_error(error),
                )
            })?;

        let compilation = recognizer.CompileConstraintsAsync().map_err(|error| {
            CommandError::operation(
                "speech_recognition_unavailable",
                sanitize_windows_error(error),
            )
        })?;
        wait_for_windows_operation!(compilation, Duration::from_secs(15))?;
        let compilation = compilation.GetResults().map_err(|error| {
            CommandError::operation(
                "speech_recognition_unavailable",
                sanitize_windows_error(error),
            )
        })?;
        let compilation_status = compilation.Status().map_err(|error| {
            CommandError::operation(
                "speech_recognition_unavailable",
                sanitize_windows_error(error),
            )
        })?;
        if compilation_status != SpeechRecognitionResultStatus::Success {
            return Err(speech_status_error(compilation_status));
        }

        let operation = recognizer.RecognizeAsync().map_err(|error| {
            CommandError::operation("speech_recognition_failed", sanitize_windows_error(error))
        })?;
        wait_for_windows_operation!(operation, Duration::from_secs(30))?;
        let result = operation.GetResults().map_err(|error| {
            CommandError::operation("speech_recognition_failed", sanitize_windows_error(error))
        })?;
        let status = result.Status().map_err(|error| {
            CommandError::operation("speech_recognition_failed", sanitize_windows_error(error))
        })?;
        if status != SpeechRecognitionResultStatus::Success {
            return Err(speech_status_error(status));
        }
        let text = result
            .Text()
            .map_err(|error| {
                CommandError::operation("speech_recognition_failed", sanitize_windows_error(error))
            })?
            .to_string();
        let _ = recognizer.Close();
        let text = text.trim().to_owned();
        if text.is_empty() {
            return Err(CommandError::new(
                "speech_not_recognized",
                "没有识别到语音，请靠近麦克风后重试",
            ));
        }
        Ok(text)
    }

    #[cfg(windows)]
    fn speech_status_error(
        status: windows::Media::SpeechRecognition::SpeechRecognitionResultStatus,
    ) -> CommandError {
        use windows::Media::SpeechRecognition::SpeechRecognitionResultStatus;
        let (code, message) = if status == SpeechRecognitionResultStatus::MicrophoneUnavailable {
            ("microphone_unavailable", "麦克风不可用或未授予访问权限")
        } else if status == SpeechRecognitionResultStatus::TopicLanguageNotSupported
            || status == SpeechRecognitionResultStatus::GrammarLanguageMismatch
        {
            (
                "speech_language_unavailable",
                "Windows 未安装当前系统语言的语音识别包，请在系统语言设置中安装语音组件",
            )
        } else if status == SpeechRecognitionResultStatus::TimeoutExceeded
            || status == SpeechRecognitionResultStatus::PauseLimitExceeded
        {
            ("speech_not_recognized", "没有听到清晰语音，请重试")
        } else if status == SpeechRecognitionResultStatus::NetworkFailure {
            (
                "speech_service_unavailable",
                "Windows 语音识别服务不可用，请检查系统的在线语音识别设置",
            )
        } else if status == SpeechRecognitionResultStatus::UserCanceled {
            ("speech_recognition_cancelled", "语音识别已取消")
        } else {
            ("speech_recognition_failed", "Windows 无法完成本次语音识别")
        };
        CommandError::new(code, message)
    }

    #[cfg(windows)]
    fn sanitize_windows_error(error: windows::core::Error) -> String {
        let code = error.code();
        format!("Windows 语音服务错误（0x{:08X}）", code.0 as u32)
    }
}

#[tauri::command]
fn exit_app(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<(), CommandError> {
    state.authorize(&window, ControlMethod::WindowInteract)?;
    require_window(&window, "pet")?;
    exit_application(&app, &state)
}

fn exit_application(app: &AppHandle, state: &DesktopState) -> Result<(), CommandError> {
    if let Some(pet) = app.get_webview_window("pet") {
        capture_pet_placement(&pet, state)?;
    }
    state.save_settings()?;
    app.exit(0);
    Ok(())
}

#[tauri::command]
fn hide_pet_window(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<(), CommandError> {
    state.authorize(&window, ControlMethod::WindowInteract)?;
    require_window(&window, "pet")?;
    state.pet_hidden_by_user.store(true, Ordering::SeqCst);
    hide_pet(&app);
    Ok(())
}

#[tauri::command]
fn show_pet_context_menu(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    menu: State<'_, PetContextMenuState>,
    request: PetContextMenuRequest,
) -> Result<(), CommandError> {
    state.authorize(&window, ControlMethod::WindowInteract)?;
    require_window(&window, "pet")?;
    if !request.x.is_finite() || !request.y.is_finite() {
        return Err(CommandError::new(
            "invalid_menu_position",
            "右键菜单位置无效",
        ));
    }
    refresh_pet_context_menu(&app);
    window
        .popup_menu_at(
            &menu.menu,
            LogicalPosition::new(request.x.max(0.0), request.y.max(0.0)),
        )
        .map_err(CommandError::from)
}

fn require_window(window: &WebviewWindow, expected: &str) -> Result<(), CommandError> {
    if window.label() == expected {
        Ok(())
    } else {
        Err(CommandError::new(
            "permission_denied",
            format!("command is only available to the {expected} window"),
        ))
    }
}

fn avatar_asset_url(entry_id: &str) -> String {
    if cfg!(windows) {
        format!("http://hachimi-avatar.localhost/{entry_id}")
    } else {
        format!("hachimi-avatar://localhost/{entry_id}")
    }
}

fn motion_asset_url(entry_id: &str) -> String {
    if cfg!(windows) {
        format!("http://hachimi-motion.localhost/{entry_id}")
    } else {
        format!("hachimi-motion://localhost/{entry_id}")
    }
}

fn cancel_pet_activity(app: &AppHandle, state: &DesktopState, emit_cancelled: bool) {
    if let Some(active) = state.pet_run.lock().take() {
        active.cancellation.cancel();
        if emit_cancelled {
            let _ = app.emit_to(
                "pet",
                PET_TURN_EVENT,
                PetTurnEvent::Cancelled {
                    run_id: active.run_id,
                },
            );
        }
    }
    state.voice_runtime.stop();
}

fn enter_workbench_mode(app: &AppHandle, state: &DesktopState) {
    cancel_pet_activity(app, state, true);
    let _ = app.emit_to("pet", "pet:close-composer", ());
    hide_pet(app);
}

fn restore_pet<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<DesktopState>();
    if state.pet_hidden_by_user.load(Ordering::SeqCst) {
        refresh_tray_menu(app);
        return;
    }
    if let Some(pet) = app.get_webview_window("pet") {
        let _ = app.emit_to("pet", "pet:refresh-avatar", ());
        let _ = pet.show();
        let _ = app.emit_to("pet", PET_VISIBILITY_EVENT, true);
    }
    refresh_tray_menu(app);
}

fn hide_pet<R: Runtime>(app: &AppHandle<R>) {
    let _ = app.emit_to("pet", PET_VISIBILITY_EVENT, false);
    if let Some(pet) = app.get_webview_window("pet") {
        let _ = pet.hide();
    }
    refresh_tray_menu(app);
}

fn show_pet_by_user<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<DesktopState>();
    state.pet_hidden_by_user.store(false, Ordering::SeqCst);
    restore_pet(app);
}

fn toggle_pet_visibility<R: Runtime>(app: &AppHandle<R>) {
    let visible = app
        .get_webview_window("pet")
        .and_then(|pet| pet.is_visible().ok())
        .unwrap_or(false);
    if visible {
        let state = app.state::<DesktopState>();
        state.pet_hidden_by_user.store(true, Ordering::SeqCst);
        hide_pet(app);
    } else {
        show_pet_by_user(app);
    }
}

fn refresh_tray_menu<R: Runtime>(app: &AppHandle<R>) {
    let Some(tray) = app.try_state::<TrayMenuState>() else {
        return;
    };
    let state = app.state::<DesktopState>();
    let settings = state.settings.read();
    let zh = settings.locale == Locale::ZhCn;
    let visible = app
        .get_webview_window("pet")
        .and_then(|pet| pet.is_visible().ok())
        .unwrap_or(false);
    let _ = tray.visibility.set_text(if visible {
        if zh { "隐藏桌宠" } else { "Hide Pet" }
    } else if zh {
        "显示桌宠"
    } else {
        "Show Pet"
    });
    let _ = tray
        .send_message
        .set_text(if zh { "发送消息" } else { "Send message" });
    let _ = tray
        .workbench
        .set_text(if zh { "工作台" } else { "Workbench" });
    let _ = tray
        .llm_settings
        .set_text(if zh { "LLM 设置" } else { "LLM settings" });
    let _ = tray.avatar_settings.set_text(if zh {
        "角色模型"
    } else {
        "Avatar settings"
    });
    let _ = tray
        .voice_settings
        .set_text(if zh { "语音设置" } else { "Voice settings" });
    let _ = tray.interaction_settings.set_text(if zh {
        "交互设置"
    } else {
        "Interaction settings"
    });
    let _ = tray.always_on_top.set_text(if zh {
        "始终显示在其他窗口上方"
    } else {
        "Keep above other windows"
    });
    let _ = tray.always_on_top.set_checked(settings.always_on_top);
    let _ = tray.exit.set_text(if zh { "退出" } else { "Exit" });
    drop(settings);
    refresh_pet_context_menu(app);
}

fn refresh_pet_context_menu<R: Runtime>(app: &AppHandle<R>) {
    let Some(menu) = app.try_state::<PetContextMenuState>() else {
        return;
    };
    let state = app.state::<DesktopState>();
    let settings = state.settings.read();
    let zh = settings.locale == Locale::ZhCn;
    let _ = menu
        .send_message
        .set_text(if zh { "发送消息" } else { "Send message" });
    let _ = menu.hide.set_text(if zh { "隐藏桌宠" } else { "Hide Pet" });
    let _ = menu
        .workbench
        .set_text(if zh { "工作台" } else { "Workbench" });
    let _ = menu
        .llm_settings
        .set_text(if zh { "LLM 设置" } else { "LLM settings" });
    let _ = menu.avatar_settings.set_text(if zh {
        "角色模型"
    } else {
        "Avatar settings"
    });
    let _ = menu
        .voice_settings
        .set_text(if zh { "语音设置" } else { "Voice settings" });
    let _ = menu.interaction_settings.set_text(if zh {
        "交互设置"
    } else {
        "Interaction settings"
    });
    let _ = menu.always_on_top.set_text(if zh {
        "始终显示在其他窗口上方"
    } else {
        "Keep above other windows"
    });
    let _ = menu.always_on_top.set_checked(settings.always_on_top);
    let _ = menu.exit.set_text(if zh { "退出" } else { "Exit" });
}

fn handle_pet_context_menu_action(app: &AppHandle, id: &str) -> Result<(), CommandError> {
    let state = app.state::<DesktopState>();
    match id {
        "pet-menu.send-message" => {
            show_pet_by_user(app);
            let _ = app.emit_to("pet", "pet:open-composer", ());
            Ok(())
        }
        "pet-menu.hide" => {
            state.pet_hidden_by_user.store(true, Ordering::SeqCst);
            hide_pet(app);
            Ok(())
        }
        "pet-menu.workbench" => open_workbench_route(app, &state, WorkbenchRoute::Home),
        "pet-menu.settings-llm" => open_workbench_route(app, &state, WorkbenchRoute::SettingsLlm),
        "pet-menu.settings-avatar" => {
            open_workbench_route(app, &state, WorkbenchRoute::SettingsAvatar)
        }
        "pet-menu.settings-voice" => {
            open_workbench_route(app, &state, WorkbenchRoute::SettingsVoice)
        }
        "pet-menu.settings-interaction" => {
            open_workbench_route(app, &state, WorkbenchRoute::SettingsMotion)
        }
        "pet-menu.always-on-top" => {
            let enabled = !state.settings.read().always_on_top;
            update_always_on_top(app, &state, enabled).map(|_| ())
        }
        "pet-menu.exit" => exit_application(app, &state),
        _ => Ok(()),
    }
}

fn create_pet_context_menu(app: &tauri::App) -> Result<(), tauri::Error> {
    let state = app.state::<DesktopState>();
    let settings = state.settings.read();
    let zh = settings.locale == Locale::ZhCn;
    let send_message = MenuItem::with_id(
        app,
        "pet-menu.send-message",
        if zh { "发送消息" } else { "Send message" },
        true,
        None::<&str>,
    )?;
    let hide = MenuItem::with_id(
        app,
        "pet-menu.hide",
        if zh { "隐藏桌宠" } else { "Hide Pet" },
        true,
        None::<&str>,
    )?;
    let workbench = MenuItem::with_id(
        app,
        "pet-menu.workbench",
        if zh { "工作台" } else { "Workbench" },
        true,
        None::<&str>,
    )?;
    let llm_settings = MenuItem::with_id(
        app,
        "pet-menu.settings-llm",
        if zh { "LLM 设置" } else { "LLM settings" },
        true,
        None::<&str>,
    )?;
    let avatar_settings = MenuItem::with_id(
        app,
        "pet-menu.settings-avatar",
        if zh {
            "角色模型"
        } else {
            "Avatar settings"
        },
        true,
        None::<&str>,
    )?;
    let voice_settings = MenuItem::with_id(
        app,
        "pet-menu.settings-voice",
        if zh { "语音设置" } else { "Voice settings" },
        true,
        None::<&str>,
    )?;
    let interaction_settings = MenuItem::with_id(
        app,
        "pet-menu.settings-interaction",
        if zh {
            "交互设置"
        } else {
            "Interaction settings"
        },
        true,
        None::<&str>,
    )?;
    let always_on_top = CheckMenuItem::with_id(
        app,
        "pet-menu.always-on-top",
        if zh {
            "始终显示在其他窗口上方"
        } else {
            "Keep above other windows"
        },
        true,
        settings.always_on_top,
        None::<&str>,
    )?;
    let exit = MenuItem::with_id(
        app,
        "pet-menu.exit",
        if zh { "退出" } else { "Exit" },
        true,
        None::<&str>,
    )?;
    drop(settings);
    let separator_one = PredefinedMenuItem::separator(app)?;
    let separator_two = PredefinedMenuItem::separator(app)?;
    let separator_three = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &send_message,
            &hide,
            &separator_one,
            &workbench,
            &llm_settings,
            &avatar_settings,
            &voice_settings,
            &interaction_settings,
            &separator_two,
            &always_on_top,
            &separator_three,
            &exit,
        ],
    )?;
    app.manage(PetContextMenuState {
        menu,
        send_message,
        hide,
        workbench,
        llm_settings,
        avatar_settings,
        voice_settings,
        interaction_settings,
        always_on_top,
        exit,
    });
    Ok(())
}

fn create_tray(app: &tauri::App) -> Result<(), tauri::Error> {
    let state = app.state::<DesktopState>();
    let settings = state.settings.read();
    let zh = settings.locale == Locale::ZhCn;
    let visibility = MenuItem::with_id(
        app,
        "tray.visibility",
        if zh { "隐藏桌宠" } else { "Hide Pet" },
        true,
        None::<&str>,
    )?;
    let send_message = MenuItem::with_id(
        app,
        "tray.send-message",
        if zh { "发送消息" } else { "Send message" },
        true,
        None::<&str>,
    )?;
    let workbench = MenuItem::with_id(
        app,
        "tray.workbench",
        if zh { "工作台" } else { "Workbench" },
        true,
        None::<&str>,
    )?;
    let llm_settings = MenuItem::with_id(
        app,
        "tray.settings-llm",
        if zh { "LLM 设置" } else { "LLM settings" },
        true,
        None::<&str>,
    )?;
    let avatar_settings = MenuItem::with_id(
        app,
        "tray.settings-avatar",
        if zh {
            "角色模型"
        } else {
            "Avatar settings"
        },
        true,
        None::<&str>,
    )?;
    let voice_settings = MenuItem::with_id(
        app,
        "tray.settings-voice",
        if zh { "语音设置" } else { "Voice settings" },
        true,
        None::<&str>,
    )?;
    let interaction_settings = MenuItem::with_id(
        app,
        "tray.settings-interaction",
        if zh {
            "交互设置"
        } else {
            "Interaction settings"
        },
        true,
        None::<&str>,
    )?;
    let always_on_top = CheckMenuItem::with_id(
        app,
        "tray.always-on-top",
        if zh {
            "始终显示在其他窗口上方"
        } else {
            "Keep above other windows"
        },
        true,
        settings.always_on_top,
        None::<&str>,
    )?;
    let exit = MenuItem::with_id(
        app,
        "tray.exit",
        if zh { "退出" } else { "Exit" },
        true,
        None::<&str>,
    )?;
    drop(settings);
    let separator_one = PredefinedMenuItem::separator(app)?;
    let separator_two = PredefinedMenuItem::separator(app)?;
    let separator_three = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &visibility,
            &send_message,
            &separator_one,
            &workbench,
            &llm_settings,
            &avatar_settings,
            &voice_settings,
            &interaction_settings,
            &separator_two,
            &always_on_top,
            &separator_three,
            &exit,
        ],
    )?;
    app.manage(TrayMenuState {
        visibility,
        send_message,
        workbench,
        llm_settings,
        avatar_settings,
        voice_settings,
        interaction_settings,
        always_on_top,
        exit,
    });
    let mut builder = TrayIconBuilder::with_id("hachimi-main")
        .menu(&menu)
        .tooltip("Hachimi")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let state = app.state::<DesktopState>();
            let result = match event.id().as_ref() {
                "tray.visibility" => {
                    toggle_pet_visibility(app);
                    Ok(())
                }
                "tray.send-message" => {
                    show_pet_by_user(app);
                    let _ = app.emit_to("pet", "pet:open-composer", ());
                    Ok(())
                }
                "tray.workbench" => open_workbench_route(app, &state, WorkbenchRoute::Home),
                "tray.settings-llm" => {
                    open_workbench_route(app, &state, WorkbenchRoute::SettingsLlm)
                }
                "tray.settings-avatar" => {
                    open_workbench_route(app, &state, WorkbenchRoute::SettingsAvatar)
                }
                "tray.settings-voice" => {
                    open_workbench_route(app, &state, WorkbenchRoute::SettingsVoice)
                }
                "tray.settings-interaction" => {
                    open_workbench_route(app, &state, WorkbenchRoute::SettingsMotion)
                }
                "tray.always-on-top" => {
                    let enabled = !state.settings.read().always_on_top;
                    update_always_on_top(app, &state, enabled).map(|_| ())
                }
                "tray.exit" => exit_application(app, &state),
                _ => Ok(()),
            };
            if let Err(error) = result {
                tracing::error!(code = %error.code, message = %error.message, "tray menu action failed");
            }
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                toggle_pet_visibility(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    refresh_tray_menu(app.handle());
    Ok(())
}

fn resolve_resource<R: Runtime>(app: &AppHandle<R>, relative: &str) -> PathBuf {
    if let Ok(resource_dir) = app.path().resource_dir() {
        let candidate = resource_dir.join(relative);
        if candidate.exists() {
            return candidate;
        }
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn avatar_protocol_response(
    status: tauri::http::StatusCode,
    content_type: &'static str,
    body: Vec<u8>,
) -> tauri::http::Response<Vec<u8>> {
    tauri::http::Response::builder()
        .status(status)
        .header(tauri::http::header::CONTENT_TYPE, content_type)
        .header(tauri::http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(tauri::http::header::CACHE_CONTROL, "no-store")
        .body(body)
        .expect("static avatar protocol response headers are valid")
}

fn create_workbench_window<R: Runtime>(
    app: &AppHandle<R>,
    route: WorkbenchRoute,
    webview_data_directory: &Path,
) -> Result<(), CommandError> {
    let url = format!("workbench.html?route={}", route.as_str());
    let workbench = WebviewWindowBuilder::new(app, "workbench", WebviewUrl::App(url.into()))
        .title("Hachimi Workbench")
        .inner_size(1280.0, 800.0)
        .min_inner_size(960.0, 640.0)
        .resizable(true)
        .decorations(false)
        .transparent(false)
        .shadow(true)
        .data_directory(webview_data_directory.to_path_buf())
        // The workbench is an opaque application window, so it does not need
        // the Pet window's hidden-until-ready flash prevention. Keeping it
        // hidden here makes any frontend bootstrap error look like the menu
        // command did nothing.
        .visible(true)
        .build()?;
    let close_target = workbench.clone();
    let event_app = app.clone();
    workbench.on_window_event(move |event| match event {
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            let _ = close_target.hide();
            restore_pet(&event_app);
        }
        WindowEvent::Focused(true) => {
            hide_pet(&event_app);
        }
        WindowEvent::Resized(_) => {
            if close_target.is_minimized().unwrap_or(false) {
                restore_pet(&event_app);
            }
        }
        WindowEvent::Destroyed => restore_pet(&event_app),
        _ => {}
    });
    workbench.show()?;
    workbench.set_focus()?;
    Ok(())
}

fn capture_pet_placement<R: Runtime>(
    window: &WebviewWindow<R>,
    state: &DesktopState,
) -> Result<(), CommandError> {
    let position = window.outer_position()?;
    let size = window.outer_size()?;
    let scale_factor = window.scale_factor()?;
    let monitor_name = window
        .current_monitor()?
        .and_then(|monitor| monitor.name().cloned());
    state.settings.write().pet_placement = Some(WindowPlacementV1 {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
        monitor_name,
        scale_factor,
    });
    Ok(())
}

fn monitor_geometries<R: Runtime>(window: &WebviewWindow<R>) -> Vec<MonitorGeometry> {
    let primary = window.primary_monitor().ok().flatten();
    window
        .available_monitors()
        .unwrap_or_default()
        .into_iter()
        .map(|monitor| {
            let position = monitor.position();
            let size = monitor.size();
            let is_primary = primary.as_ref().is_some_and(|candidate| {
                candidate.position() == position && candidate.size() == size
            });
            MonitorGeometry {
                name: monitor.name().cloned(),
                bounds: PhysicalRect {
                    x: position.x,
                    y: position.y,
                    width: size.width,
                    height: size.height,
                },
                scale_factor: monitor.scale_factor(),
                primary: is_primary,
            }
        })
        .collect()
}

fn restore_pet_placement<R: Runtime>(
    window: &WebviewWindow<R>,
    state: &DesktopState,
) -> Result<(), CommandError> {
    let monitors = monitor_geometries(window);
    let placement = restore_or_default_placement(
        state.settings.read().pet_placement.as_ref(),
        &monitors,
        360,
        480,
        24,
    );
    window.set_position(PhysicalPosition::new(placement.x, placement.y))?;
    Ok(())
}

fn start_click_through_loop(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(30));
        loop {
            interval.tick().await;
            let Some(window) = app.get_webview_window("pet") else {
                break;
            };
            let Some(state) = app.try_state::<DesktopState>() else {
                continue;
            };
            if !window.is_visible().unwrap_or(false) {
                continue;
            }
            let (Ok(cursor), Ok(origin), Ok(scale_factor)) = (
                window.cursor_position(),
                window.outer_position(),
                window.scale_factor(),
            ) else {
                continue;
            };
            let hit = state.interactive_regions.read().hit_test(
                PhysicalPoint {
                    x: cursor.x,
                    y: cursor.y,
                },
                PhysicalPoint {
                    x: f64::from(origin.x),
                    y: f64::from(origin.y),
                },
                scale_factor,
            );
            let should_ignore = !hit;
            if state.click_through.swap(should_ignore, Ordering::SeqCst) != should_ignore {
                let _ = window.set_ignore_cursor_events(should_ignore);
            }
        }
    });
}

fn persist_pet_placement_after_move(app: AppHandle, revision: u64) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let Some(state) = app.try_state::<DesktopState>() else {
            return;
        };
        if state.placement_revision.load(Ordering::SeqCst) != revision {
            return;
        }
        let Some(pet) = app.get_webview_window("pet") else {
            return;
        };
        if let Err(error) = capture_pet_placement(&pet, &state).and_then(|()| state.save_settings())
        {
            tracing::warn!(message = %error.message, "failed to persist pet placement");
        }
    });
}

fn open_append_log(path: PathBuf) -> std::io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

fn epoch_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn redact_prefixed_token(mut value: String, prefix: &str) -> String {
    let mut search_from = 0;
    while let Some(relative) = value[search_from..].find(prefix) {
        let token_start = search_from + relative + prefix.len();
        let token_end = value[token_start..]
            .char_indices()
            .find_map(|(offset, character)| {
                (!character.is_ascii_alphanumeric() && !matches!(character, '-' | '_' | '.'))
                    .then_some(token_start + offset)
            })
            .unwrap_or(value.len());
        if token_end == token_start {
            search_from = token_start;
            continue;
        }
        value.replace_range(token_start..token_end, "[REDACTED]");
        search_from = token_start + "[REDACTED]".len();
    }
    value
}

fn sanitize_log_message(message: &str) -> String {
    let mut value = message
        .chars()
        .take(4_096)
        .collect::<String>()
        .replace(['\r', '\n'], " ");
    for prefix in [
        "sk-",
        "Bearer ",
        "bearer ",
        "apiKey=",
        "api_key=",
        "apiKey\":\"",
        "api_key\":\"",
    ] {
        value = redact_prefixed_token(value, prefix);
    }
    value
}

fn initialize_logging(preferred: PathBuf) -> PathBuf {
    let fallback = std::env::temp_dir().join("Hachimi").join("logs");
    let (log_dir, backend_log, used_fallback) = [(&preferred, false), (&fallback, true)]
        .into_iter()
        .find_map(|(directory, fallback)| {
            std::fs::create_dir_all(directory)
                .and_then(|()| open_append_log(directory.join("hachimi-backend.log")))
                .ok()
                .map(|file| (directory.clone(), file, fallback))
        })
        .expect("unable to create the Hachimi logs directory");

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(
            "warn,hachimi_desktop=info,hachimi_voice=info,hachimi_llm=info",
        ))
        .with_ansi(false)
        .with_thread_names(true)
        .with_writer(StdMutex::new(backend_log))
        .init();
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        tracing::error!(panic = %panic_info, "unhandled Rust panic");
        previous_hook(panic_info);
    }));
    if used_fallback {
        tracing::warn!(
            preferred = %preferred.display(),
            fallback = %log_dir.display(),
            "the executable directory was not writable; using a fallback logs directory"
        );
    }
    tracing::info!(directory = %log_dir.display(), "file logging initialized");
    log_dir
}

fn main() {
    let storage_layout = resolve_storage_layout();
    if let Err(error) = perform_pending_reset(&storage_layout) {
        eprintln!("Hachimi reset could not be completed: {error}");
    }
    configure_webview_storage(&storage_layout);
    let log_dir = initialize_logging(storage_layout.logs());

    tauri::Builder::default()
        .register_uri_scheme_protocol("hachimi-avatar", |context, request| {
            if !matches!(context.webview_label(), "pet" | "workbench")
                || request.method() != tauri::http::Method::GET
            {
                return avatar_protocol_response(
                    tauri::http::StatusCode::FORBIDDEN,
                    "text/plain; charset=utf-8",
                    b"forbidden".to_vec(),
                );
            }
            let entry_id = request.uri().path().trim_matches('/');
            if entry_id.is_empty()
                || entry_id.len() > 64
                || !entry_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            {
                return avatar_protocol_response(
                    tauri::http::StatusCode::BAD_REQUEST,
                    "text/plain; charset=utf-8",
                    b"invalid avatar id".to_vec(),
                );
            }
            let Some(state) = context.app_handle().try_state::<DesktopState>() else {
                return avatar_protocol_response(
                    tauri::http::StatusCode::SERVICE_UNAVAILABLE,
                    "text/plain; charset=utf-8",
                    b"avatar runtime is starting".to_vec(),
                );
            };
            let asset = if context.webview_label() == "workbench" {
                state.avatar_catalog.read().asset_for(entry_id)
            } else {
                state.avatar_catalog.read().current_asset_for(entry_id)
            };
            match asset.and_then(|asset| std::fs::read(asset.path).ok()) {
                Some(bytes) => avatar_protocol_response(
                    tauri::http::StatusCode::OK,
                    "model/gltf-binary",
                    bytes,
                ),
                None => avatar_protocol_response(
                    tauri::http::StatusCode::NOT_FOUND,
                    "text/plain; charset=utf-8",
                    b"avatar not found".to_vec(),
                ),
            }
        })
        .register_uri_scheme_protocol("hachimi-motion", |context, request| {
            if !matches!(context.webview_label(), "pet" | "workbench")
                || request.method() != tauri::http::Method::GET
            {
                return avatar_protocol_response(
                    tauri::http::StatusCode::FORBIDDEN,
                    "text/plain; charset=utf-8",
                    b"forbidden".to_vec(),
                );
            }
            let id = request.uri().path().trim_matches('/');
            if id.is_empty()
                || id.len() > 128
                || !id.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'.' | b'_' | b'-')
                })
            {
                return avatar_protocol_response(
                    tauri::http::StatusCode::BAD_REQUEST,
                    "text/plain; charset=utf-8",
                    b"invalid motion id".to_vec(),
                );
            }
            let Some(state) = context.app_handle().try_state::<DesktopState>() else {
                return avatar_protocol_response(
                    tauri::http::StatusCode::SERVICE_UNAVAILABLE,
                    "text/plain; charset=utf-8",
                    b"motion runtime is starting".to_vec(),
                );
            };
            match state
                .motion_catalog
                .read()
                .asset_for(id)
                .and_then(|asset| std::fs::read(asset.path).ok())
            {
                Some(bytes) => avatar_protocol_response(
                    tauri::http::StatusCode::OK,
                    "model/gltf-binary",
                    bytes,
                ),
                None => avatar_protocol_response(
                    tauri::http::StatusCode::NOT_FOUND,
                    "text/plain; charset=utf-8",
                    b"motion asset not found".to_vec(),
                ),
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_bootstrap_state,
            frontend_ready,
            write_frontend_log,
            get_settings,
            update_settings,
            reset_local_data,
            import_theme_profile,
            copy_theme_profile,
            reset_theme_profile,
            delete_theme_profile,
            set_interactive_regions,
            set_always_on_top,
            start_pet_dragging,
            open_workbench,
            hide_workbench,
            minimize_workbench,
            toggle_maximize_workbench,
            start_workbench_dragging,
            start_workbench_resize,
            get_llm_settings,
            save_llm_settings,
            save_and_test_llm_settings,
            list_avatar_models,
            inspect_avatar_model,
            commit_avatar_model_import,
            cancel_avatar_model_import,
            select_avatar_model,
            delete_avatar_model,
            get_current_avatar_asset,
            get_avatar_runtime_asset,
            list_motion_catalog,
            inspect_motion_file,
            commit_motion_import,
            cancel_motion_import,
            update_motion_metadata,
            delete_user_motion,
            set_interaction_motion_binding,
            clear_motion_interaction_bindings,
            set_motion_enabled,
            reset_motion_bindings,
            reset_motion_binding,
            get_motion_runtime_asset,
            start_pet_turn,
            cancel_pet_turn,
            list_voice_models,
            inspect_voice_model,
            commit_voice_model_import,
            cancel_voice_model_import,
            select_voice_model,
            delete_voice_model,
            get_voice_runtime_state,
            get_speech_recognition_state,
            update_speech_recognition_settings,
            update_voice_settings,
            set_muted,
            preview_default_voice,
            stop_speech,
            recognize_pet_speech,
            hide_pet_window,
            show_pet_context_menu,
            exit_app,
        ])
        .setup(move |app| {
            let data_dir = storage_layout.root.clone();
            std::fs::create_dir_all(&data_dir)?;
            std::fs::write(data_dir.join(DATA_ROOT_SENTINEL_FILE), APP_IDENTIFIER)?;
            tracing::info!(
                mode = ?storage_layout.mode,
                directory = %data_dir.display(),
                "application storage initialized"
            );
            let store = SettingsStore::new(data_dir.join("settings.json"));
            let settings = store.load().unwrap_or_else(|error| {
                tracing::error!(%error, "failed to load settings; using defaults");
                AppSettings::default()
            });
            let bundled_default_avatar = resolve_resource(app.handle(), DEFAULT_AVATAR_RESOURCE);
            let development_default_avatar = Path::new(env!("CARGO_MANIFEST_DIR")).join(
                "../../../assets/avatar-default/3800386813668044008/3800386813668044008.vrm",
            );
            let default_avatar = if bundled_default_avatar.is_file() {
                bundled_default_avatar
            } else {
                development_default_avatar
            };
            let avatar_root = data_dir.join("models");
            let avatar_catalog =
                match AvatarCatalog::load_with_default(&avatar_root, default_avatar) {
                    Ok(catalog) => catalog,
                    Err(error) => {
                        tracing::error!(%error, "bundled default VRM is unavailable; Pet will use the SVG fault fallback when no user Runtime Ready model exists");
                        AvatarCatalog::load(avatar_root)?
                    }
                };
            let bundled_motion_catalog =
                resolve_resource(app.handle(), MOTION_CATALOG_RESOURCE);
            let development_motion_catalog = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../assets/avatar-motions-v4/catalog.json");
            let builtin_motion_catalog = if bundled_motion_catalog.is_file() {
                bundled_motion_catalog
            } else {
                development_motion_catalog
            };
            let motion_catalog =
                MotionCatalog::load(data_dir.join("motions-v3"), builtin_motion_catalog)?;
            let bundled_sensevoice = resolve_resource(app.handle(), SENSEVOICE_RESOURCE);
            let development_sensevoice = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("resources/ai-models/speech-to-text/sensevoice-small");
            let sensevoice_dir = if bundled_sensevoice.join("model.int8.onnx").is_file() {
                bundled_sensevoice
            } else {
                development_sensevoice
            };
            let vits_dir = resolve_resource(app.handle(), VITS_RESOURCE);
            let voice_catalog = VoiceCatalog::load(data_dir.join("voice-models"), &vits_dir)?;
            let current_voice_asset = voice_catalog.current_asset();
            let speech_recognizer = SpeechRecognizerRuntime::new(
                sensevoice_dir.clone(),
                settings.voice.recognition_compute_mode,
            );
            let playback_app = app.handle().clone();
            let voice_event_sink: VoiceEventSink = Arc::new(move |event| {
                let _ = playback_app.emit_to("pet", VOICE_PLAYBACK_EVENT, event);
            });
            let turn_app = app.handle().clone();
            let voice_turn_event_sink: VoiceTurnEventSink = Arc::new(move |event| {
                let _ = turn_app.emit_to("pet", VOICE_TURN_EVENT, event);
            });
            let runtime_app = app.handle().clone();
            let voice_runtime_state_sink: VoiceRuntimeStateSink = Arc::new(move |runtime| {
                let _ = runtime_app.emit("voice-runtime-changed", runtime);
            });
            let voice_runtime = VoiceRuntime::start_with_event_sink(
                current_voice_asset,
                settings.voice.muted,
                settings.voice.speed_percent,
                settings.voice.compute_mode,
                VoiceRuntimeEventSinks {
                    playback: Some(voice_event_sink),
                    turn: Some(voice_turn_event_sink),
                    state: Some(voice_runtime_state_sink),
                },
            );
            if !speech_recognizer.available() {
                tracing::error!(path = %sensevoice_dir.display(), "bundled SenseVoice-Small model is missing");
            }
            let feature_flags = FeatureFlags {
                workbench: true,
                motion_lab: cfg!(debug_assertions)
                    || settings.developer_mode
                    || std::env::var("HACHIMI_ENABLE_MOTION_LAB").as_deref() == Ok("1"),
                ..FeatureFlags::all_disabled()
            };
            let recognition_runtime = speech_recognizer.clone();
            let state = DesktopState {
                storage_layout: storage_layout.clone(),
                settings: RwLock::new(settings.clone()),
                settings_store: store,
                api_key_store: SystemApiKeyStore,
                avatar_catalog: RwLock::new(avatar_catalog),
                pending_avatar_imports: Mutex::new(BTreeMap::new()),
                motion_catalog: RwLock::new(motion_catalog),
                pending_motion_imports: Mutex::new(BTreeMap::new()),
                voice_catalog: RwLock::new(voice_catalog),
                pending_voice_imports: Mutex::new(BTreeMap::new()),
                voice_runtime,
                speech_recognizer,
                frontend_log: FrontendLog::open(&log_dir)?,
                pet_run: Mutex::new(None),
                control_plane: ControlPlane::new(feature_flags),
                interactive_regions: RwLock::new(InteractiveRegionState::default()),
                click_through: AtomicBool::new(false),
                pet_hidden_by_user: AtomicBool::new(false),
                placement_revision: AtomicU64::new(0),
            };
            app.manage(state);
            create_pet_context_menu(app)?;
            let recognition_app = app.handle().clone();
            std::thread::spawn(move || {
                let result = recognition_runtime.initialize();
                let runtime = match result {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        tracing::warn!(%error, "SenseVoice background warm-up failed");
                        recognition_runtime.state()
                    }
                };
                let _ = recognition_app.emit(SPEECH_RECOGNITION_STATE_EVENT, &runtime);
            });
            let pet_config = app
                .config()
                .app
                .windows
                .iter()
                .find(|window| window.label == "pet")
                .cloned()
                .ok_or_else(|| std::io::Error::other("pet window config is missing"))?;
            let pet = WebviewWindowBuilder::from_config(app.handle(), &pet_config)?
                .data_directory(storage_layout.webview.clone())
                .build()?;
            pet.set_always_on_top(settings.always_on_top)?;
            pet.set_skip_taskbar(true)?;
            pet.set_shadow(false)?;
            let managed = app.state::<DesktopState>();
            restore_pet_placement(&pet, &managed)
                .map_err(|error| std::io::Error::other(error.message))?;
            let moved_app = app.handle().clone();
            let previous_motion = Arc::new(StdMutex::new(None::<(i32, i32, Instant)>));
            let moved_motion = Arc::clone(&previous_motion);
            pet.on_window_event(move |event| {
                if let WindowEvent::Moved(position) = event {
                    let now = Instant::now();
                    let (velocity_x, velocity_y) = moved_motion
                        .lock()
                        .ok()
                        .and_then(|mut previous| {
                            let velocity = previous.map(|(x, y, timestamp)| {
                                let seconds = now.duration_since(timestamp).as_secs_f32().max(0.001);
                                (
                                    (position.x - x) as f32 / seconds,
                                    (position.y - y) as f32 / seconds,
                                )
                            });
                            *previous = Some((position.x, position.y, now));
                            velocity
                        })
                        .unwrap_or((0.0, 0.0));
                    let _ = moved_app.emit_to(
                        "pet",
                        "pet:window-motion",
                        PetWindowMotionEvent {
                            x: position.x,
                            y: position.y,
                            velocity_x,
                            velocity_y,
                        },
                    );
                    let state = moved_app.state::<DesktopState>();
                    let revision = state.placement_revision.fetch_add(1, Ordering::SeqCst) + 1;
                    persist_pet_placement_after_move(moved_app.clone(), revision);
                }
            });
            pet.on_menu_event(|window, event| {
                if let Err(error) =
                    handle_pet_context_menu_action(window.app_handle(), event.id().as_ref())
                {
                    tracing::error!(code = %error.code, message = %error.message, "pet context menu action failed");
                }
            });
            create_tray(app)?;
            start_click_through_loop(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run Hachimi");
}

#[cfg(test)]
mod logging_tests {
    use super::{
        PendingAvatarImport, PendingVoiceImport, avatar_source_is_unchanged,
        cancel_pending_avatar_import, cancel_pending_voice_import, consume_pending_avatar_import,
        consume_pending_voice_import, debug_data_root, delete_theme_in_settings,
        profile_supports_pet_voice, reset_theme_in_settings, sanitize_log_message,
        validate_app_settings, voice_source_is_unchanged,
    };
    use std::{
        collections::BTreeMap,
        path::{Path, PathBuf},
        time::{Duration, Instant},
    };

    use hachimi_avatar::InspectedAvatar;
    use hachimi_protocol::{
        AppSettings, AvatarAdaptationProfile, AvatarAssessment, AvatarCompatibility, AvatarFormat,
        ClientId, LipSyncCapability, ThemeProfile, ThemeScheme,
    };
    use hachimi_voice::{InspectedVoiceModel, VoiceAssetPaths};

    fn compatible_inspection(sha256: &str, modified_millis: u128) -> InspectedAvatar {
        InspectedAvatar {
            original_file_name: "avatar.vrm".into(),
            size_bytes: 42,
            sha256: sha256.into(),
            format: AvatarFormat::Vrm1,
            assessment: AvatarAssessment {
                compatibility: AvatarCompatibility::RuntimeReady,
                ..AvatarAssessment::default()
            },
            profile: AvatarAdaptationProfile::default(),
            modified_millis,
        }
    }

    #[test]
    fn pet_pcm_is_allowed_only_when_the_avatar_can_move_its_mouth() {
        let mut profile = AvatarAdaptationProfile::default();
        assert!(!profile_supports_pet_voice(&profile));
        profile.lip_sync = LipSyncCapability::Jaw;
        assert!(profile_supports_pet_voice(&profile));
        profile.lip_sync = LipSyncCapability::FiveViseme;
        assert!(profile_supports_pet_voice(&profile));
    }

    fn pending(owner: &str, expires_at: Instant) -> PendingAvatarImport {
        PendingAvatarImport {
            owner: ClientId(owner.into()),
            source: PathBuf::from("avatar.vrm"),
            inspection: compatible_inspection("sha", 7),
            expires_at,
        }
    }

    fn compatible_voice_inspection(sha256: &str, modified_millis: u128) -> InspectedVoiceModel {
        InspectedVoiceModel {
            original_file_name: "voice.tar.bz2".into(),
            size_bytes: 42,
            sha256: sha256.into(),
            modified_millis,
            model_type: "vits".into(),
            languages: vec!["zh-CN".into()],
            sample_rate: 22_050,
            speaker_count: 1,
            suggested_speaker_id: 0,
            license_summary: "License: Test".into(),
            license_warning: false,
            compatible: true,
            issues: Vec::new(),
            paths: VoiceAssetPaths::default(),
        }
    }

    fn pending_voice(owner: &str, expires_at: Instant) -> PendingVoiceImport {
        PendingVoiceImport {
            owner: ClientId(owner.into()),
            source: PathBuf::from("voice.tar.bz2"),
            inspection: compatible_voice_inspection("sha", 7),
            expires_at,
        }
    }

    #[test]
    fn frontend_logs_are_bounded_and_secrets_are_redacted() {
        let value = sanitize_log_message(
            "Bearer token-value sk-example-secret apiKey=another-secret\nnext line",
        );
        assert!(!value.contains("token-value"));
        assert!(!value.contains("sk-example-secret"));
        assert!(!value.contains("another-secret"));
        assert!(!value.contains('\n'));
        assert!(value.contains("[REDACTED]"));
    }

    #[test]
    fn debug_storage_is_kept_under_target_for_binaries_and_tests() {
        assert_eq!(
            debug_data_root(Path::new(
                r"D:\workspace\hachimi\target\debug\hachimi-desktop.exe"
            )),
            Some(PathBuf::from(r"D:\workspace\hachimi\target\hachimi-data"))
        );
        assert_eq!(
            debug_data_root(Path::new(
                r"D:\workspace\hachimi\target\debug\deps\hachimi-desktop.exe"
            )),
            Some(PathBuf::from(r"D:\workspace\hachimi\target\hachimi-data"))
        );
    }

    #[test]
    fn reset_restores_an_edited_builtin_theme() {
        let mut settings = AppSettings::default();
        settings.appearance.themes[1].accent = "#FF00FF".into();
        reset_theme_in_settings(&mut settings, "codex-dark").expect("reset");
        assert_eq!(
            settings
                .appearance
                .profile("codex-dark")
                .expect("dark")
                .accent,
            "#2EA8FF"
        );
    }

    #[test]
    fn reset_supports_every_builtin_theme() {
        let mut settings = AppSettings::default();
        settings
            .appearance
            .themes
            .iter_mut()
            .find(|profile| profile.id == "github-dark")
            .expect("github dark")
            .accent = "#FF00FF".into();
        reset_theme_in_settings(&mut settings, "github-dark").expect("reset");
        assert_eq!(
            settings
                .appearance
                .profile("github-dark")
                .expect("github dark")
                .accent,
            "#2F81F7"
        );
    }

    #[test]
    fn deleting_selected_custom_theme_falls_back_safely() {
        let mut settings = AppSettings::default();
        let mut custom = ThemeProfile::codex_dark();
        custom.id = "theme-custom".into();
        custom.name = "Custom".into();
        custom.builtin = false;
        custom.scheme = ThemeScheme::Dark;
        settings.appearance.dark_theme_id = custom.id.clone();
        settings.appearance.themes.push(custom);
        delete_theme_in_settings(&mut settings, "theme-custom").expect("delete");
        assert_eq!(settings.appearance.dark_theme_id, "codex-dark");
        assert!(settings.appearance.profile("theme-custom").is_none());
    }

    #[test]
    fn app_settings_validation_rejects_invalid_appearance() {
        let mut settings = AppSettings::default();
        settings.appearance.themes[0].background = "white".into();
        assert_eq!(
            validate_app_settings(&settings).expect_err("invalid").code,
            "invalid_appearance"
        );
    }

    #[test]
    fn avatar_import_tokens_expire_are_client_bound_and_single_use() {
        let now = Instant::now();
        let mut imports = BTreeMap::from([
            (
                "valid".into(),
                pending("workbench", now + Duration::from_secs(60)),
            ),
            (
                "expired".into(),
                pending("workbench", now - Duration::from_secs(1)),
            ),
        ]);
        assert!(
            consume_pending_avatar_import(&mut imports, "valid", &ClientId("other".into()), now)
                .is_none()
        );
        assert!(
            consume_pending_avatar_import(
                &mut imports,
                "expired",
                &ClientId("workbench".into()),
                now
            )
            .is_none()
        );
        assert!(
            consume_pending_avatar_import(
                &mut imports,
                "valid",
                &ClientId("workbench".into()),
                now
            )
            .is_some()
        );
        assert!(
            consume_pending_avatar_import(
                &mut imports,
                "valid",
                &ClientId("workbench".into()),
                now
            )
            .is_none()
        );
    }

    #[test]
    fn avatar_import_token_cancel_and_source_change_fail_closed() {
        let now = Instant::now();
        let mut imports = BTreeMap::from([(
            "token".into(),
            pending("workbench", now + Duration::from_secs(60)),
        )]);
        assert!(!cancel_pending_avatar_import(
            &mut imports,
            "token",
            &ClientId("other".into()),
            now,
        ));
        assert!(cancel_pending_avatar_import(
            &mut imports,
            "token",
            &ClientId("workbench".into()),
            now,
        ));
        assert!(!imports.contains_key("token"));

        let original = compatible_inspection("sha", 7);
        assert!(avatar_source_is_unchanged(
            &original,
            &compatible_inspection("sha", 7)
        ));
        assert!(!avatar_source_is_unchanged(
            &original,
            &compatible_inspection("different", 7)
        ));
        assert!(!avatar_source_is_unchanged(
            &original,
            &compatible_inspection("sha", 8)
        ));
    }

    #[test]
    fn voice_import_tokens_expire_are_client_bound_single_use_and_cancellable() {
        let now = Instant::now();
        let owner = ClientId("workbench".into());
        let other = ClientId("other".into());
        let mut imports = BTreeMap::from([
            (
                "valid".into(),
                pending_voice("workbench", now + Duration::from_secs(60)),
            ),
            (
                "expired".into(),
                pending_voice("workbench", now - Duration::from_secs(1)),
            ),
            (
                "cancel".into(),
                pending_voice("workbench", now + Duration::from_secs(60)),
            ),
        ]);

        assert!(consume_pending_voice_import(&mut imports, "valid", &other, now).is_none());
        assert!(consume_pending_voice_import(&mut imports, "expired", &owner, now).is_none());
        assert!(consume_pending_voice_import(&mut imports, "valid", &owner, now).is_some());
        assert!(consume_pending_voice_import(&mut imports, "valid", &owner, now).is_none());
        assert!(!cancel_pending_voice_import(
            &mut imports,
            "cancel",
            &other,
            now
        ));
        assert!(cancel_pending_voice_import(
            &mut imports,
            "cancel",
            &owner,
            now
        ));
        assert!(!imports.contains_key("cancel"));
    }

    #[test]
    fn voice_import_source_changes_fail_closed() {
        let original = compatible_voice_inspection("sha", 7);
        assert!(voice_source_is_unchanged(
            &original,
            &compatible_voice_inspection("sha", 7)
        ));
        assert!(!voice_source_is_unchanged(
            &original,
            &compatible_voice_inspection("different", 7)
        ));
        assert!(!voice_source_is_unchanged(
            &original,
            &compatible_voice_inspection("sha", 8)
        ));
        let mut incompatible = compatible_voice_inspection("sha", 7);
        incompatible.compatible = false;
        assert!(!voice_source_is_unchanged(&original, &incompatible));
    }
}
