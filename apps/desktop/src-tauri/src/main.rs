#![cfg_attr(windows, windows_subsystem = "windows")]
#![cfg_attr(all(feature = "desktop-e2e", not(debug_assertions)), allow(dead_code))]
#[cfg(all(feature = "desktop-e2e", not(debug_assertions)))]
compile_error!("the desktop-e2e feature is forbidden in release builds");
mod agent_commands;
mod agent_git_forge_tools;
mod agent_host_tools;
mod agent_host_tools_support;
mod agent_runtime_host;
mod app_domain_handler;
mod app_shell;
mod browser_detection;
mod browser_extension_server;
mod browser_extension_trust;
mod browser_router;
mod browser_settings_commands;
mod browser_tool_policy;
mod browser_workspace_commands;
mod channel_agent_dispatch;
mod command_error;
mod computer_settings_commands;
mod computer_targeting;
mod desktop_e2e;
#[cfg(all(debug_assertions, feature = "desktop-e2e"))]
mod desktop_e2e_agent_tools;
mod domain_run_launcher;
mod embedded_browser;
mod embedded_browser_agent;
mod environment_commands;
mod extension_commands;
mod forge_commands;
mod gateway_process;
mod gateway_runtime;
mod git_forge_host;
mod host_domain_commands;
mod host_revision_snapshots;
mod integration_commands;
mod local_host_commands;
mod managed_sandbox_runtime;
mod mcp_commands;
mod mcp_runtime;
mod media_commands;
mod optional_resource_runtime;
mod permission_runtime;
mod permission_settings_commands;
mod plugin_content_protocol;
mod process_commands;
mod project_git_commands;
mod project_tool_commands;
mod review_commands;
mod runtime_supervisor;
mod sandbox_commands;
mod scheduler_commands;
mod scheduler_runtime;
mod shutdown_coordinator;
mod skill_drop;
mod startup_error;
mod startup_timeline;
mod storage_layout;
mod workbench_commands;
mod workbench_plan_commands;
mod workspace_commands;
mod workspace_mutation_commands;
use agent_commands::*;
use app_shell::*;
use browser_settings_commands::*;
use browser_workspace_commands::*;
use command_error::CommandError;
use computer_settings_commands::*;
use desktop_e2e::*;
use environment_commands::*;
use extension_commands::*;
use forge_commands::*;
use host_domain_commands::*;
use integration_commands::*;
use mcp_commands::*;
use mcp_runtime::*;
use media_commands::*;
use permission_settings_commands::*;
use process_commands::*;
use project_git_commands::*;
use project_tool_commands::*;
use review_commands::*;
use runtime_supervisor::*;
use sandbox_commands::*;
use scheduler_commands::*;
use scheduler_runtime::start_desktop_scheduler;
use skill_drop::{PendingSkillDrop, handle_skill_drag_event};
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
use storage_layout::*;
use workbench_commands::*;
use workbench_plan_commands::*;
use workspace_commands::*;
use workspace_mutation_commands::*;

use hachimi_agent::{AgentExecutorRegistry, AgentRunExecutor};
use hachimi_approvals::{ApprovalBroker, PersistentApprovalBroker};
use hachimi_avatar::{AvatarCatalog, InspectedAvatar, inspect_avatar};
use hachimi_capabilities::McpEchoServer;
use hachimi_control_plane::{
    AgentLifecycleService, AppServer, ControlPlane, McpControlService, PersistentControlAuditSink,
};
use hachimi_core::{FeatureFlags, RuntimeFeatureSet, WindowKind};
use hachimi_llm::{
    ApiKeyStore, SystemApiKeyStore, apply_secret_change, test_connection, validate_input,
};
use hachimi_motion::{InspectedMotion, MotionCatalog, inspect_motion};
use hachimi_process::ProcessRegistry;
use hachimi_protocol::{
    AppSettings, ApprovalDecisionRequest, ApprovalResolution, ApprovalStatus, AttachmentRecord,
    AvatarAdaptationProfile, AvatarCatalogSnapshot, AvatarImportCommitRequest,
    AvatarImportInspection, AvatarRuntimeAsset, BootstrapState, CONTROL_PROTOCOL_VERSION,
    ClientContext, ClientId, ControlMethod, FrontendLogEntry, FrontendLogLevel, GitRefRecord,
    InteractionMotionBindingUpdateRequest, InteractiveRegionsUpdate, LipSyncCapability,
    LlmSettingsInput, LlmSettingsView, LlmTestResult, Locale, MAX_THEME_PROFILES,
    McpServerHealthRecord, McpServerRecord, McpServerUpsertRequest, McpServerView,
    MotionAssetBindingsClearRequest, MotionBindingResetRequest, MotionCatalogSnapshot,
    MotionEnabledUpdateRequest, MotionImportCommitRequest, MotionImportInspection,
    MotionMetadataUpdateRequest, MotionRuntimeAsset, PetContextMenuRequest, PetTurnEvent,
    PetTurnRequest, PlanAcceptanceRequest, ProjectId, ProjectRecord, ResourceEntryRequest, RunId,
    RunRecord, SETTINGS_SCHEMA_VERSION, SessionId, SkillSubscriptionId,
    SpeechRecognitionRuntimeState, SpeechRecognitionSettingsInput, ThemeProfile,
    ThemeProfileDocument, ThemeScheme, VoiceCatalogSnapshot, VoiceImportCommitRequest,
    VoiceModelInspection, VoiceRuntimeState, VoiceSettingsInput, WindowPlacementV1,
    WorkbenchPlanAcceptanceSnapshot, WorkbenchRoute, WorkbenchSessionSnapshot,
    WorkbenchTaskSnapshot, WorkbenchTaskStartRequest,
};
use hachimi_sandbox::{
    SandboxBackend, SandboxRuntimeManager, SandboxStatus, WindowsSandboxReadinessProbe,
};
use hachimi_storage::{AgentStore, SettingsStore};
use hachimi_user_input::{PersistentUserInputBroker, UserInputBroker};
use hachimi_voice::{
    InspectedVoiceModel, SpeechRecognizerRuntime, VoiceCatalog, VoiceEventSink, VoiceRuntime,
    VoiceRuntimeEventSinks, VoiceRuntimeStateSink, VoiceTurnEventSink, inspect_voice_archive,
};
use hachimi_windowing::{
    InteractiveRegionState, MonitorGeometry, PhysicalPoint, PhysicalRect,
    restore_or_default_placement,
};
use hachimi_workbench::WorkbenchService;
use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use tauri::{
    AppHandle, Emitter, LogicalPosition, Manager, PhysicalPosition, Runtime, State, WebviewWindow,
    WebviewWindowBuilder, WindowEvent, Wry,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_runtime::ResizeDirection;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const PET_TURN_EVENT: &str = "pet:turn";
const VOICE_PLAYBACK_EVENT: &str = "voice:playback";
const VOICE_TURN_EVENT: &str = "voice:turn";
const VOICE_CATALOG_EVENT: &str = "voice:catalog-changed";
const SPEECH_RECOGNITION_STATE_EVENT: &str = "speech-recognition-state-changed";
const AVATAR_CATALOG_EVENT: &str = "avatar:catalog-changed";
const MOTION_CATALOG_EVENT: &str = "motion:catalog-changed";
const WORKBENCH_RUN_EVENT: &str = "workbench:run-updated";
const WORKBENCH_SESSION_ACTIVITY_EVENT: &str = "workbench:session-activity-changed";
const DEFAULT_AVATAR_RESOURCE: &str =
    "resources/avatar-default/2639776812528692620/2639776812528692620.vrm";
const PET_VISIBILITY_EVENT: &str = "pet:visibility";
const MAX_THEME_FILE_BYTES: u64 = 64 * 1024;
const AVATAR_IMPORT_TOKEN_TTL: Duration = Duration::from_secs(10 * 60);
const MOTION_IMPORT_TOKEN_TTL: Duration = Duration::from_secs(10 * 60);
const VOICE_IMPORT_TOKEN_TTL: Duration = Duration::from_secs(10 * 60);
#[derive(Debug)]
struct ActivePetRun {
    run_id: String,
    session_id: SessionId,
    agent_run_id: RunId,
    run_generation: u64,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PetWindowMotionEvent {
    x: i32,
    y: i32,
    velocity_x: f32,
    velocity_y: f32,
}

struct DesktopState {
    storage_layout: StorageLayout,
    agent_store: AgentStore,
    settings: RwLock<AppSettings>,
    settings_store: SettingsStore,
    workbench: WorkbenchService,
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
    agent_executor: AgentRunExecutor,
    process_registry: Arc<ProcessRegistry>,
    process_event_bridges: ProcessEventBridgeRegistry,
    scheduler_handle: Mutex<Option<hachimi_scheduler::SchedulerHandle>>,
    runtime_supervisor: RuntimeSupervisor,
    workspace_watches: Arc<Mutex<BTreeMap<hachimi_protocol::FsWatchId, ActiveWorkspaceWatch>>>,
    workspace_searches: Arc<Mutex<BTreeMap<hachimi_protocol::FsSearchId, ActiveWorkspaceSearch>>>,
    agent_event_streams: Mutex<BTreeMap<hachimi_protocol::EventSubscriptionId, CancellationToken>>,
    approval_broker: PersistentApprovalBroker,
    user_input_broker: PersistentUserInputBroker,
    app_server: AppServer,
    gateway: hachimi_gateway::GatewayHost,
    plugin_host: hachimi_extensions::PluginHost,
    plugin_surfaces: plugin_content_protocol::PluginSurfaceRegistry,
    browser_host: Arc<hachimi_browser::BrowserHost>,
    computer_host: Arc<hachimi_computer::ComputerHost>,
    embedded_browser: Arc<embedded_browser::EmbeddedBrowserService<Wry>>,
    sandbox_runtime: Arc<SandboxRuntimeManager>,
    sandbox_activity: SandboxActivityTracker,
    control_plane: Arc<ControlPlane>,
    mcp_control: McpControlService,
    mcp_secrets: McpKeyring,
    mcp_echo_server: McpEchoServer,
    skill_host: hachimi_skills::SkillHost,
    skill_subscriptions: Mutex<BTreeMap<SkillSubscriptionId, String>>,
    pending_skill_drops: Mutex<BTreeMap<String, PendingSkillDrop>>,
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
    fn sandbox_snapshot(&self) -> hachimi_protocol::SandboxRuntimeSnapshot {
        self.sandbox_runtime.snapshot()
    }

    fn sandbox_status(&self) -> SandboxStatus {
        SandboxStatus::from_report(&self.sandbox_snapshot().report)
    }

    fn sandbox_backend(&self) -> Option<Arc<dyn SandboxBackend>> {
        let snapshot = self.sandbox_snapshot();
        (snapshot.report.backend != "desktop-e2e-deterministic")
            .then(|| Arc::clone(&self.sandbox_runtime) as Arc<dyn SandboxBackend>)
    }

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
        let settings = workbench_commands::provider_settings_for_runtime(
            self.settings.read().llm.clone(),
            self.control_plane.feature_flags().runtime_features,
        );
        Ok(LlmSettingsView::from_settings(&settings, configured))
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
        WindowKind::Service => {
            return Err(CommandError::new(
                "invalid_window",
                "internal service principals do not map to a Webview window",
            ));
        }
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
    cancel_all_workspace_transients(&state);
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

fn load_or_create_loopback_token(data_root: &Path) -> std::io::Result<String> {
    let credential = keyring::Entry::new("com.hachimi.channel", "loopback-webhook:local")
        .map_err(keyring_io_error)?;
    match credential.get_password() {
        Ok(token) if (32..=128).contains(&token.len()) => return Ok(token),
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "loopback credential is invalid",
            ));
        }
        Err(keyring::Error::NoEntry) => {}
        Err(error) => return Err(keyring_io_error(error)),
    }
    let gateway_root = data_root.join("gateway");
    std::fs::create_dir_all(&gateway_root)?;
    let token_path = gateway_root.join("loopback.token");
    let token = if token_path.is_file() {
        let token = std::fs::read_to_string(&token_path)?;
        let token = token.trim().to_owned();
        if !(32..=128).contains(&token.len()) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "legacy loopback token is invalid",
            ));
        }
        token
    } else {
        Uuid::new_v4().to_string()
    };
    credential.set_password(&token).map_err(keyring_io_error)?;
    if token_path.is_file() {
        std::fs::remove_file(token_path)?;
    }
    Ok(token)
}
fn keyring_io_error(error: keyring::Error) -> std::io::Error {
    std::io::Error::other(format!("channel credential storage failed: {error}"))
}

fn main() {
    if let Err(error) = reject_release_e2e_environment() {
        panic!("{error}");
    }
    let storage_layout = resolve_storage_layout();
    if let Err(error) = prepare_schema_epoch(&storage_layout) {
        eprintln!("Hachimi V2 initialization could not be completed: {error}");
        std::process::exit(1);
    }
    if std::env::args_os().any(|argument| argument == "--gateway") {
        gateway_process::run(&storage_layout.root);
        return;
    }
    configure_webview_storage(&storage_layout);
    let log_dir = initialize_logging(storage_layout.logs());
    let startup_log_dir = log_dir.clone();

    let builder = tauri::Builder::default();
    #[cfg(not(feature = "desktop-e2e"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        activate_existing_instance(app);
    }));
    let result = builder
        .plugin(tauri_plugin_notification::init())
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
        .register_uri_scheme_protocol("hachimi-plugin-asset", plugin_content_protocol::asset_protocol)
        .register_uri_scheme_protocol("hachimi-plugin-ui", plugin_content_protocol::ui_protocol)
        .on_webview_event(handle_skill_drag_event)
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
            list_mcp_servers,
            get_mcp_server,
            get_mcp_echo_server_url,
            test_mcp_server,
            upsert_mcp_server,
            set_mcp_server_enabled,
            refresh_mcp_server,
            remove_mcp_server,
            list_mcp_tools,
            discover_mcp_tools,
            set_mcp_tool_enabled,
            get_mcp_inventory,
            refresh_mcp_inventory,
            read_mcp_resource,
            get_mcp_prompt,
            list_mcp_call_summaries,
            get_mcp_auth_status,
            start_mcp_oauth_login,
            logout_mcp_oauth,
            list_skills,
            create_skill,
            import_skill_archive,
            import_skill_dropped_files,
            rename_skill,
            remove_skill,
            set_skill_enabled,
            get_skill_tree,
            read_skill_file,
            write_skill_file,
            create_skill_entry,
            rename_skill_entry,
            remove_skill_entry,
            validate_skill,
            read_skill_preview_resource,
            subscribe_skills,
            unsubscribe_skills,
            list_workbench_projects,
            get_workbench_project_tool_context,
            list_run_recoveries,
            resolve_run_recovery,
            add_workbench_project,
            manage_workbench_project,
            import_workbench_attachment,
            read_workbench_attachment,
            list_workbench_sessions,
            get_workbench_session,
            get_workbench_environment,
            open_browser_workspace,
            mutate_browser_workspace,
            update_browser_surface_layout,
            get_browser_history,
            get_embedded_browser_settings,
            choose_browser_download_directory,
            update_embedded_browser_settings,
            clear_embedded_browser_data,
            get_browser_downloads,
            manage_browser_download,
            list_embedded_browser_permission_requests,
            list_embedded_browser_site_permissions,
            resolve_embedded_browser_permission,
            revoke_embedded_browser_site_permission,
            open_system_browser,
            handoff_workbench_session,
            resolve_workbench_approval,
            accept_workbench_plan,
            revise_workbench_plan,
            skip_workbench_plan,
            execute_workbench_git,
            list_project_git_refs,
            inspect_project_git,
            refresh_project_git,
            create_project_empty_initial_commit,
            list_git_remotes,
            push_git_remote,
            query_forge_change,
            mutate_forge_change,
            update_forge_credential,
            get_sandbox_status,
            get_sandbox_bootstrap_state,
            refresh_sandbox_status,
            attest_sandbox,
            repair_sandbox,
            pin_workbench_checkout,
            cleanup_workbench_checkout,
            start_workbench_task,
            compact_workbench_session,
            cancel_workbench_run,
            list_workspace_files,
            read_workspace_file_chunk,
            write_workspace_file,
            get_workspace_git,
            mutate_workspace_git,
            watch_workspace_files,
            unwatch_workspace_files,
            start_workspace_file_search,
            update_workspace_file_search,
            cancel_workspace_file_search,
            get_workspace_diff,
            read_workspace_diff_file,
            spawn_process,
            write_process_stdin,
            resize_process,
            terminate_process,
            read_process,
            list_processes,
            start_review,
            get_review,
            list_reviews,
            update_review_finding,
            create_schedule,
            choose_schedule_workspace_directory,
            get_schedule,
            list_schedules,
            preview_schedule,
            update_schedule,
            set_schedule_enabled,
            remove_schedule,
            run_schedule_now,
            ingest_schedule_event,
            list_schedule_event_receipts,
            get_task_run,
            list_task_runs,
            cancel_task_run,
            retry_task_run,
            continue_task_interactively,
            list_installed_plugins,
            list_installed_plugin_contributions,
            get_plugin_contribution_surface,
            check_plugin_health,
            get_plugin_permission_diff,
            list_plugin_revisions,
            list_plugin_lifecycle_journal,
            list_connector_accounts,
            get_connector_driver_descriptor,
            get_gateway_health,
            get_runtime_health,
            retry_runtime_component,
            list_channel_provider_manifests,
            list_channel_provider_health,
            list_channel_provider_accounts,
            list_integration_providers,
            list_enterprise_integrations,
            begin_ilink_qr_login,
            poll_ilink_qr_login,
            cancel_ilink_qr_login,
            upsert_enterprise_integration,
            set_enterprise_integration_capabilities,
            probe_enterprise_integration,
            remove_enterprise_integration,
            list_channel_authorizations,
            upsert_channel_authorization,
            create_channel_pairing_code,
            create_channel_identity_link_code,
            list_channel_identity_transfer_previews,
            transfer_channel_identity,
            get_channel_access_policy,
            update_channel_access_policy,
            approve_browser_extension,
            install_browser_extension,
            get_browser_host_settings,
            update_browser_host_settings,
            list_browser_site_policies,
            update_browser_site_policy,
            update_private_browser_site_policy,
            remove_browser_site_policy,
            list_host_access_requests,
            resolve_host_access_request,
            stop_browser_automation,
            take_over_browser_automation,
            resume_browser_automation,
            get_computer_host_settings,
            update_computer_host_settings,
            list_computer_app_candidates,
            list_computer_app_policies,
            update_computer_app_policy,
            choose_permission_directory,
            choose_permission_files,
            search_permission_commands,
            choose_permission_foreground_application,
            take_over_computer_control,
            resume_computer_control,
            stop_computer_control,
            get_computer_control_frame,
            initialize_agent_control,
            search_agent_sessions,
            resume_agent_session,
            fork_agent_session,
            update_agent_session_metadata,
            steer_agent_run,
            interrupt_agent_run,
            subscribe_agent_events,
            unsubscribe_agent_events,
            list_pending_user_input,
            resolve_user_input,
            list_pending_approvals,
            resolve_agent_approval,
            cancel_user_input,
            get_llm_settings,
            get_provider_registry,
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
            get_session_permission_config,
            update_session_permission_config,
            clear_session_extra_authorizations,
            start_pet_turn,
            recover_pet_turn,
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
            let mut startup_timeline = startup_timeline::StartupTimeline::new();
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
            let runtime_features = release_runtime_feature_set(RuntimeFeatureSet {
                run_recovery: env_disabled("HACHIMI_DISABLE_RUN_RECOVERY"),
                provider_extensions: env_disabled("HACHIMI_DISABLE_PROVIDER_EXTENSIONS"),
                provider_remote_context: env_disabled(
                    "HACHIMI_DISABLE_PROVIDER_REMOTE_CONTEXT",
                ),
                multi_agent: env_disabled("HACHIMI_DISABLE_MULTI_AGENT"),
                git_remote_mutations: env_disabled("HACHIMI_DISABLE_GIT_REMOTE_MUTATIONS"),
                plugin_runtime: env_disabled("HACHIMI_DISABLE_PLUGIN_RUNTIME"),
                enterprise_integrations: env_disabled(
                    "HACHIMI_DISABLE_ENTERPRISE_INTEGRATIONS",
                ),
            });
            let agent_store = tauri::async_runtime::block_on(AgentStore::connect(
                data_dir.join("agent-v2.sqlite3"),
            ))?;
            tauri::async_runtime::block_on(agent_store.reconcile_managed_workspaces())?;
            let runtime_supervisor = RuntimeSupervisor::new(app.handle().clone());
            let run_activity = agent_store.subscribe_run_activity();
            let mut recovery = tauri::async_runtime::block_on(
                agent_store.recover_interrupted_with_run_recovery(
                    runtime_features.run_recovery,
                ),
            )?;
            if recovery.interrupted_runs > 0
                || recovery.lost_tasks > 0
                || recovery.expired_processes > 0
                || recovery.expired_approvals > 0
                || recovery.interrupted_user_inputs > 0
                || recovery.stopped_mcp_servers > 0
            {
                tracing::warn!(
                interrupted_runs = recovery.interrupted_runs,
                lost_tasks = recovery.lost_tasks,
                    expired_processes = recovery.expired_processes,
                expired_approvals = recovery.expired_approvals,
                    interrupted_user_inputs = recovery.interrupted_user_inputs,
                    stopped_mcp_servers = recovery.stopped_mcp_servers,
                    "recovered interrupted agent state"
                );
            }
            let approval_broker = PersistentApprovalBroker::new(agent_store.clone());
            let user_input_broker = PersistentUserInputBroker::new(agent_store.clone());
            let resource_dir = app.path().resource_dir()?;
            startup_timeline.checkpoint("storage_and_recovery");
            tauri::async_runtime::block_on(agent_store.reconcile_browser_startup())?;
            let embedded_browser = Arc::new(embedded_browser::EmbeddedBrowserService::new(
                app.handle().clone(),
                agent_store.clone(),
                &data_dir,
                &resource_dir,
            ));
            let embedded_agent_browser = Arc::new(
                embedded_browser_agent::EmbeddedAgentBrowser::new(
                    app.handle().clone(),
                    agent_store.clone(),
                    Arc::clone(&embedded_browser),
                    settings.developer_mode,
                ),
            );
            let managed_sandbox = managed_sandbox_runtime::stage_or_degrade(
                &data_dir, &resource_dir, runtime_supervisor.clone(),
            );
            startup_timeline.checkpoint("sandbox_resources");
            let sandbox_probe = Arc::new(
                WindowsSandboxReadinessProbe::new(storage_layout.sandbox_setup_marker())
                    .with_runtime(
                        managed_sandbox.launcher.clone(),
                        managed_sandbox.canary.clone(),
                        data_dir.join("sandbox/windows/attestation"),
                    )
                    .with_runtime_integrity(managed_sandbox.expected_integrity.clone()),
            );
            let deterministic_report = deterministic_e2e_sandbox_report();
            let sandbox_runtime = Arc::new(SandboxRuntimeManager::new_with_report(
                Arc::clone(&sandbox_probe),
                managed_sandbox.setup.clone(),
                storage_layout.sandbox_setup_marker(),
                managed_sandbox.launcher.clone(),
                deterministic_report.clone(),
            ));
            if deterministic_report.is_none()
                && SandboxStatus::from_report(&sandbox_runtime.snapshot().report)
                    != SandboxStatus::Enforced
                && let Err(error) = tauri::async_runtime::block_on(sandbox_runtime.repair())
            {
                tracing::warn!(code = error.code, message = %error.message, "per-user Sandbox bootstrap/repair did not attest");
            }
            let sandbox_report = sandbox_runtime.snapshot().report;
            startup_timeline.checkpoint("sandbox_attestation");
            let sandbox_backend: Option<Arc<dyn SandboxBackend>> =
                deterministic_e2e_sandbox_backend().or_else(|| {
                    deterministic_report
                        .is_none()
                        .then(|| Arc::clone(&sandbox_runtime) as Arc<dyn SandboxBackend>)
                });
            tracing::info!(
                backend = %sandbox_report.backend,
                readiness = ?sandbox_report.readiness,
                os_enforced = sandbox_report.os_enforced,
                error_code = ?sandbox_report.stable_error_code,
                "workspace sandbox readiness probed"
            );
            let workbench = WorkbenchService::new(
                agent_store.clone(),
                data_dir.join("worktrees"),
                data_dir.join("attachments"),
            );
            let reconciled_handoffs =
                tauri::async_runtime::block_on(workbench.reconcile_handoffs())?;
            if reconciled_handoffs > 0 {
                tracing::warn!(
                    count = reconciled_handoffs,
                    "unfinished Workbench Handoffs were reconciled during startup"
                );
            }
            let skill_host = hachimi_skills::SkillHost::new(
                data_dir.join("skills/user"),
                agent_store.clone(),
            )?;
            let bundled_skill_root = resolve_resource(app.handle(), "resources/skills/builtin");
            let development_skill_root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("resources/skills/builtin");
            let builtin_skill_root = if bundled_skill_root.is_dir() {
                bundled_skill_root
            } else {
                development_skill_root
            };
            let mut skill_catalog_roots = vec![
                hachimi_skills::SkillCatalogRoot::new(
                    builtin_skill_root,
                    hachimi_protocol::SkillScope::BuiltIn,
                ),
                hachimi_skills::SkillCatalogRoot::new(
                    data_dir.join("skills/system"),
                    hachimi_protocol::SkillScope::System,
                ),
            ];
            if let Some(user_profile) = std::env::var_os("USERPROFILE") {
                skill_catalog_roots.push(hachimi_skills::SkillCatalogRoot::new(
                    PathBuf::from(user_profile).join(".agents/skills"),
                    hachimi_protocol::SkillScope::User,
                ));
            }
            if let Some(program_data) = std::env::var_os("PROGRAMDATA") {
                skill_catalog_roots.push(hachimi_skills::SkillCatalogRoot::new(
                    PathBuf::from(program_data).join("Hachimi/skills"),
                    hachimi_protocol::SkillScope::Admin,
                ));
            }
            skill_host.set_catalog_roots(skill_catalog_roots)?;
            let skill_change_host = skill_host.clone();
            let bundled_default_avatar = resolve_resource(app.handle(), DEFAULT_AVATAR_RESOURCE);
            let development_default_avatar = Path::new(env!("CARGO_MANIFEST_DIR")).join(
                "../../../assets/avatar-default/2639776812528692620/2639776812528692620.vrm",
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
            startup_timeline.checkpoint("skills_and_avatar");
            let optional_resources = optional_resource_runtime::stage_optional_resources(
                app.handle(),
                &data_dir,
                runtime_supervisor.clone(),
            );
            let motion_catalog = optional_resource_runtime::load_motion_catalog(
                &optional_resources.motion_catalog,
                &data_dir,
                &runtime_supervisor,
            )?;
            let sensevoice_dir = optional_resources.speech_model;
            let vits_dir = optional_resources.voice_model;
            let voice_catalog = VoiceCatalog::load(data_dir.join("voice-models"), &vits_dir)?;
            startup_timeline.checkpoint("optional_resources");
            let current_voice_asset = voice_catalog.current_asset();
            let voice_model_available = current_voice_asset.is_some();
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
            optional_resource_runtime::publish_voice_health(
                &runtime_supervisor,
                speech_recognizer.available(),
                voice_model_available,
                &sensevoice_dir,
            );
            let mut feature_flags = release_agent_feature_flags(
                std::env::var("HACHIMI_DISABLE_WORKSPACE_TOOLS").as_deref() == Ok("1"),
                std::env::var("HACHIMI_DISABLE_MCP_RUNTIME").as_deref() == Ok("1"),
                std::env::var("HACHIMI_DISABLE_SCHEDULER").as_deref() == Ok("1"),
            );
            feature_flags.runtime_features = runtime_features;
            feature_flags.plugin_runtime = feature_flags.runtime_features.plugin_runtime;
            feature_flags.motion_lab = cfg!(debug_assertions);
            let control_plane = Arc::new(ControlPlane::with_audit(
                feature_flags,
                Arc::new(PersistentControlAuditSink::new(agent_store.clone())),
            ));
            let agent_lifecycle = AgentLifecycleService::new(
                agent_store.clone(),
                feature_flags,
                sandbox_report.clone(),
            );
            let mcp_secrets = McpKeyring;
            let mcp_echo_server = McpEchoServer::start()?;
            tracing::info!(url = %mcp_echo_server.url(), "loopback MCP echo server started");
            tauri::async_runtime::block_on(retry_deferred_mcp_secret_cleanup(
                &agent_store,
                mcp_secrets,
            ));
            let mcp_control = configured_mcp_control(
                &agent_store,
                &control_plane,
                sandbox_backend.as_ref(),
                &data_dir,
                mcp_secrets,
            );
            let browser = browser_extension_server::create_browser_host(
                app.handle().clone(), &data_dir, feature_flags.browser_control,
                runtime_supervisor.clone(),
            );
            let computer = Arc::new(hachimi_computer::ComputerHost::new(
                Arc::new(hachimi_computer::PlatformComputerBroker::new()),
                Arc::new(hachimi_computer::SystemComputerClock),
            ));
            let plugins = hachimi_extensions::PluginHost::new(
                agent_store.clone(),
                data_dir.join("plugins"),
            );
            if feature_flags.runtime_features.plugin_runtime
                && let Some(backend) = sandbox_backend.as_ref()
            {
                tauri::async_runtime::block_on(
                    plugins.register_sidecar_drivers(Arc::clone(backend)),
                )?;
            }
            let mut plugin_skill_catalog_roots = skill_host.catalog_roots();
            if feature_flags.runtime_features.plugin_runtime {
                for (plugin_id, path) in
                    tauri::async_runtime::block_on(plugins.enabled_skill_roots())?
                {
                    let namespace = app_domain_handler::plugin_skill_namespace(plugin_id.as_str());
                    plugin_skill_catalog_roots.push(
                        hachimi_skills::SkillCatalogRoot::new(
                            path,
                            hachimi_protocol::SkillScope::System,
                        )
                        .with_namespace(namespace),
                    );
                }
            }
            skill_host.set_catalog_roots(plugin_skill_catalog_roots)?;
            let multi_agent = hachimi_agent::MultiAgentCoordinator::new(agent_store.clone());
            let agent_preparer = Arc::new(agent_runtime_host::DesktopAgentRunPreparer::new(
                agent_runtime_host::DesktopAgentRunDependencies {
                    app: app.handle().clone(),
                    store: agent_store.clone(),
                    workbench: workbench.clone(),
                    approvals: approval_broker.clone(),
                    user_input: user_input_broker.clone(),
                    skills: skill_host.clone(),
                    mcp: mcp_control.clone(),
                    sandbox_backend: sandbox_backend.clone(),
                    browser: Arc::clone(&browser),
                    embedded_browser: embedded_agent_browser,
                    computer: Arc::clone(&computer),
                    plugins: plugins.clone(),
                    multi_agent: multi_agent.clone(),
                    runtime_features: feature_flags.runtime_features,
                    browser_control: feature_flags.browser_control,
                    computer_observe: feature_flags.computer_observe,
                    computer_control: feature_flags.computer_control,
                },
            ));
            let agent_executor = AgentRunExecutor::new(
                agent_store.clone(),
                Arc::new(AgentExecutorRegistry::default()),
                Arc::new(workbench_commands::DesktopModelRuntimeFactory::new(
                    agent_store.clone(),
                    feature_flags.runtime_features,
                )),
                agent_preparer,
            );
            multi_agent
                .install_executor(agent_executor.clone())
                .map_err(|_| "Multi-Agent executor was already installed")?;
            if runtime_features.multi_agent {
                let report = tauri::async_runtime::block_on(multi_agent.reconcile_startup())
                    .map_err(|error| format!("Multi-Agent startup reconciliation failed: {error}"))?;
                recovery.auto_resume_run_ids.retain(|run_id| {
                    !report.handled_recovery_run_ids.iter().any(|handled| handled == run_id)
                });
                if report.inspected > 0 {
                    tracing::info!(
                        inspected = report.inspected,
                        resumed = report.resumed,
                        synchronized_terminal = report.synchronized_terminal,
                        needs_attention = report.needs_attention,
                        failed = report.failed,
                        cancelled = report.cancelled,
                        "reconciled durable Multi-Agent Tasks"
                    );
                }
            }
            let recognition_runtime = speech_recognizer.clone();
            let scheduler = Arc::new(hachimi_scheduler::SchedulerService::new(
                agent_store.clone(),
                Arc::new(hachimi_scheduler::SystemClock),
                Arc::new(hachimi_scheduler::BundledIanaTimeZoneResolver),
                Arc::new(DesktopScheduleRunLauncher::new(app.handle().clone())),
                Arc::new(DesktopTaskNotificationAdapter::new(app.handle().clone())),
            ));
            let sandbox_activity = SandboxActivityTracker::default();
            let process_registry = Arc::new(ProcessRegistry::default());
            let workspace_watches = Arc::new(Mutex::new(BTreeMap::new()));
            let workspace_searches = Arc::new(Mutex::new(BTreeMap::new()));
            let loopback_token = load_or_create_loopback_token(&data_dir)?;
            let builtins = hachimi_gateway::local_builtin_providers_with_enterprise(
                agent_store.clone(),
                &loopback_token,
                feature_flags.runtime_features.enterprise_integrations,
            )?;
            let channel_providers = builtins.registry.clone();
            if feature_flags.runtime_features.plugin_runtime
                && let Some(backend) = sandbox_backend.as_ref()
            {
                for definition in
                    tauri::async_runtime::block_on(plugins.enabled_channel_sidecars())?
                {
                    let provider = hachimi_gateway::SandboxedStdioChannelProvider::new(
                        Arc::clone(backend),
                        definition.manifest,
                        definition.bundle_root,
                        definition.executable,
                        definition.args,
                    )?;
                    channel_providers.register(Arc::new(provider))?;
                }
            }
            let gateway = hachimi_gateway::GatewayHost::with_registry(
                agent_store.clone(),
                channel_providers,
            );
            tauri::async_runtime::block_on(
                gateway.bootstrap_provider_accounts(&builtins.accounts),
            )?;
            let plugin_surfaces = plugin_content_protocol::PluginSurfaceRegistry::default();
            let domain_handler = app_domain_handler::DesktopAppDomainHandler::new(
                app_domain_handler::DesktopAppDomainDependencies {
                    store: agent_store.clone(),
                    mcp: mcp_control.clone(),
                    skills: skill_host.clone(),
                    scheduler: Arc::clone(&scheduler),
                    processes: Arc::clone(&process_registry),
                    sandbox_runtime: Arc::clone(&sandbox_runtime),
                    run_launcher: Arc::new(
                        domain_run_launcher::DesktopDomainRunLauncherAdapter::new(
                            app.handle().clone(),
                        ),
                    ),
                    workspace_watches: Arc::clone(&workspace_watches),
                    workspace_searches: Arc::clone(&workspace_searches),
                    browser: Arc::clone(&browser),
                    computer: Arc::clone(&computer),
                    plugins: plugins.clone(),
                    plugin_surfaces: plugin_surfaces.clone(),
                    gateway: gateway.clone(),
                    loopback_channel: builtins.loopback,
                    mock_poll_channel: builtins.mock_poll,
                },
                feature_flags,
                sandbox_activity.clone(),
            );
            if feature_flags.runtime_features.plugin_runtime {
                tauri::async_runtime::block_on(domain_handler.reconcile_plugin_startup()).map_err(
                    |error| std::io::Error::other(format!("{}: {}", error.code, error.message)),
                )?;
            }
            startup_timeline.checkpoint("agent_services");
            let app_server = AppServer::new(Arc::clone(&control_plane), agent_lifecycle)
                .with_brokers(
                    Arc::new(approval_broker.clone()),
                    Arc::new(user_input_broker.clone()),
                )
                .with_domain_handler(Arc::new(domain_handler));
            let state = DesktopState {
                storage_layout: storage_layout.clone(),
                agent_store: agent_store.clone(),
                settings: RwLock::new(settings.clone()),
                settings_store: store,
                workbench,
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
                agent_executor,
                process_registry,
                process_event_bridges: ProcessEventBridgeRegistry::default(),
                scheduler_handle: Mutex::new(None),
                runtime_supervisor: runtime_supervisor.clone(),
                workspace_watches,
                workspace_searches,
                agent_event_streams: Mutex::new(BTreeMap::new()),
                approval_broker,
                user_input_broker,
                app_server,
                gateway: gateway.clone(),
                plugin_host: plugins,
                plugin_surfaces,
                browser_host: browser,
                computer_host: computer,
                embedded_browser: Arc::clone(&embedded_browser),
                sandbox_runtime,
                sandbox_activity,
                control_plane,
                mcp_control: mcp_control.clone(),
                mcp_secrets,
                mcp_echo_server,
                skill_host,
                skill_subscriptions: Mutex::new(BTreeMap::new()),
                pending_skill_drops: Mutex::new(BTreeMap::new()),
                interactive_regions: RwLock::new(InteractiveRegionState::default()),
                click_through: AtomicBool::new(false),
                pet_hidden_by_user: AtomicBool::new(false),
                placement_revision: AtomicU64::new(0),
            };
            app.manage(state);
            let reconciliation_store = agent_store.clone();
            let reconciliation_gateway = gateway.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = integration_commands::reconcile_integration_startup(
                    &reconciliation_store,
                    &reconciliation_gateway,
                )
                .await
                {
                    tracing::warn!(code = error.code, message = %error.message, "Integration startup reconciliation degraded");
                }
                if let Err(error) = reconciliation_gateway
                    .reconcile_startup(i64::try_from(epoch_millis()).unwrap_or(i64::MAX))
                    .await
                {
                    tracing::warn!(%error, "Gateway durable startup reconciliation degraded");
                }
            });
            embedded_browser.start_supervision();
            computer_settings_commands::start_computer_runtime(runtime_supervisor.clone());
            start_mcp_runtime(mcp_control.clone(), runtime_supervisor.clone(), feature_flags.mcp_runtime);
            workbench_plan_commands::start_workbench_activity_bridge(
                app.handle().clone(),
                run_activity,
            );
            schedule_auto_resume_runs(app.handle().clone(), recovery.auto_resume_run_ids);
            start_desktop_scheduler(app.handle(), scheduler, feature_flags.scheduler);
            gateway_runtime::start_gateway_runtime(
                app.handle().clone(),
                feature_flags.local_gateway,
                loopback_token,
                runtime_supervisor,
            );
            start_skill_change_bridge(app.handle().clone(), skill_change_host);
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
            let pet_builder = WebviewWindowBuilder::from_config(app.handle(), &pet_config)?;
            #[cfg(not(all(debug_assertions, feature = "desktop-e2e")))]
            let pet_builder = pet_builder.data_directory(storage_layout.webview.clone());
            let pet = pet_builder.build()?;
            pet.set_always_on_top(settings.always_on_top)?;
            pet.set_skip_taskbar(true)?;
            pet.set_shadow(false)?;
            let managed = app.state::<DesktopState>();
            restore_pet_placement(&pet, &managed)
                .map_err(|error| std::io::Error::other(error.message))?;
            startup_timeline.checkpoint("pet_window");
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
            if deterministic_e2e_provider_enabled() {
                open_workbench_route(app.handle(), &managed, WorkbenchRoute::Home)
                    .map_err(|error| std::io::Error::other(error.message))?;
            }
            startup_timeline.checkpoint("ready");
            Ok(())
        })
        .build(tauri::generate_context!());
    match result {
        Ok(app) => app.run(|app, event| {
            if matches!(event, tauri::RunEvent::ExitRequested { .. })
                && let Some(state) = app.try_state::<DesktopState>()
            {
                tauri::async_runtime::block_on(shutdown_coordinator::shutdown(&state));
            }
        }),
        Err(error) => startup_error::show(&error, &startup_log_dir),
    }
}
#[cfg(test)]
#[path = "main_tests.rs"]
mod logging_tests;
