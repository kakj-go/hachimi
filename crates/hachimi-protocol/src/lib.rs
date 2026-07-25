//! Transport-neutral protocol and serializable desktop contracts.

use std::collections::BTreeSet;

use hachimi_core::{FeatureFlags, WindowKind};
use serde::{Deserialize, Serialize};
use specta::Type;

pub const CONTROL_PROTOCOL_VERSION: u32 = 17;
pub const SETTINGS_SCHEMA_VERSION: u32 = 8;
pub const THEME_PROFILE_FORMAT: &str = "hachimi-theme";
pub const THEME_PROFILE_FORMAT_VERSION: u32 = 1;
pub const MAX_THEME_PROFILES: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
pub struct RequestId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
pub struct ClientId(pub String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ControlRequest<T> {
    pub protocol_version: u32,
    pub id: RequestId,
    pub client_id: ClientId,
    pub method: String,
    pub params: T,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ControlResponse<T> {
    pub id: RequestId,
    pub ok: bool,
    pub payload: Option<T>,
    pub error: Option<ControlError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ControlEvent<T> {
    pub event: String,
    pub payload: T,
    pub seq: u64,
    pub state_version: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ControlErrorCode {
    InvalidProtocolVersion,
    InvalidRequest,
    UnknownMethod,
    PermissionDenied,
    ApprovalRequired,
    FeatureDisabled,
    Conflict,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ControlError {
    pub code: ControlErrorCode,
    pub message: String,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
pub enum Scope {
    #[serde(rename = "pet.interact")]
    PetInteract,
    #[serde(rename = "agent.run")]
    AgentRun,
    #[serde(rename = "settings.read")]
    SettingsRead,
    #[serde(rename = "settings.write")]
    SettingsWrite,
    #[serde(rename = "llm.read")]
    LlmRead,
    #[serde(rename = "llm.write")]
    LlmWrite,
    #[serde(rename = "llm.test")]
    LlmTest,
    #[serde(rename = "llm.chat")]
    LlmChat,
    #[serde(rename = "avatar.read")]
    AvatarRead,
    #[serde(rename = "avatar.manage")]
    AvatarManage,
    #[serde(rename = "avatar.runtime")]
    AvatarRuntime,
    #[serde(rename = "motion.read")]
    MotionRead,
    #[serde(rename = "motion.manage")]
    MotionManage,
    #[serde(rename = "motion.runtime")]
    MotionRuntime,
    #[serde(rename = "voice.read")]
    VoiceRead,
    #[serde(rename = "voice.manage")]
    VoiceManage,
    #[serde(rename = "voice.playback")]
    VoicePlayback,
    #[serde(rename = "voice.capture")]
    VoiceCapture,
    #[serde(rename = "workbench.open")]
    WorkbenchOpen,
    #[serde(rename = "workbench.window")]
    WorkbenchWindow,
    #[serde(rename = "workspace.read")]
    WorkspaceRead,
    #[serde(rename = "workspace.write")]
    WorkspaceWrite,
    #[serde(rename = "workspace.exec")]
    WorkspaceExec,
    #[serde(rename = "browser.observe")]
    BrowserObserve,
    #[serde(rename = "browser.control")]
    BrowserControl,
    #[serde(rename = "computer.observe")]
    ComputerObserve,
    #[serde(rename = "computer.control")]
    ComputerControl,
    #[serde(rename = "connectors.invoke")]
    ConnectorsInvoke,
    #[serde(rename = "devices.pair")]
    DevicesPair,
    #[serde(rename = "admin.policy")]
    AdminPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientContext {
    pub client_id: ClientId,
    pub window_kind: WindowKind,
    pub scopes: BTreeSet<Scope>,
}

impl ClientContext {
    #[must_use]
    pub fn for_window(window_kind: WindowKind) -> Self {
        let scopes = match window_kind {
            WindowKind::Pet => BTreeSet::from([
                Scope::PetInteract,
                Scope::WorkbenchOpen,
                Scope::LlmChat,
                Scope::AvatarRuntime,
                Scope::MotionRuntime,
                Scope::VoicePlayback,
                Scope::VoiceCapture,
            ]),
            WindowKind::Settings => BTreeSet::from([Scope::SettingsRead, Scope::SettingsWrite]),
            WindowKind::Workbench => BTreeSet::from([
                Scope::SettingsRead,
                Scope::SettingsWrite,
                Scope::LlmRead,
                Scope::LlmWrite,
                Scope::LlmTest,
                Scope::AvatarRead,
                Scope::AvatarManage,
                Scope::MotionRead,
                Scope::MotionManage,
                Scope::VoiceRead,
                Scope::VoiceManage,
                Scope::VoicePlayback,
                Scope::WorkbenchWindow,
            ]),
        };
        Self {
            client_id: ClientId(format!("window:{}", window_kind.label())),
            window_kind,
            scopes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    Light,
    Dark,
    #[default]
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ThemeScheme {
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReducedMotion {
    #[default]
    System,
    On,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum DiffMarkerMode {
    #[default]
    Color,
    Signs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ThemeProfile {
    pub id: String,
    pub name: String,
    pub scheme: ThemeScheme,
    pub builtin: bool,
    pub accent: String,
    pub background: String,
    pub foreground: String,
    pub ui_font: String,
    pub code_font: String,
    pub translucent_sidebar: bool,
    pub contrast: u8,
}

impl ThemeProfile {
    #[allow(clippy::too_many_arguments)]
    fn builtin(
        id: &str,
        name: &str,
        scheme: ThemeScheme,
        accent: &str,
        background: &str,
        foreground: &str,
        translucent_sidebar: bool,
        contrast: u8,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            scheme,
            builtin: true,
            accent: accent.into(),
            background: background.into(),
            foreground: foreground.into(),
            ui_font: "Inter, \"Noto Sans SC\", \"Segoe UI\", system-ui, sans-serif".into(),
            code_font: "\"JetBrains Mono\", \"Cascadia Code\", monospace".into(),
            translucent_sidebar,
            contrast,
        }
    }

    #[must_use]
    pub fn codex_light() -> Self {
        Self::builtin(
            "codex-light",
            "Codex Light",
            ThemeScheme::Light,
            "#1677D2",
            "#F5F4F7",
            "#202126",
            true,
            45,
        )
    }

    #[must_use]
    pub fn codex_dark() -> Self {
        Self::builtin(
            "codex-dark",
            "Codex Dark",
            ThemeScheme::Dark,
            "#2EA8FF",
            "#151616",
            "#F1F1F3",
            true,
            60,
        )
    }

    #[must_use]
    pub fn builtin_profiles() -> Vec<Self> {
        vec![
            Self::codex_light(),
            Self::codex_dark(),
            Self::builtin(
                "catppuccin-light",
                "Catppuccin Latte",
                ThemeScheme::Light,
                "#8839EF",
                "#EFF1F5",
                "#4C4F69",
                true,
                52,
            ),
            Self::builtin(
                "catppuccin-dark",
                "Catppuccin Mocha",
                ThemeScheme::Dark,
                "#CBA6F7",
                "#1E1E2E",
                "#CDD6F4",
                true,
                62,
            ),
            Self::builtin(
                "github-light",
                "GitHub Light",
                ThemeScheme::Light,
                "#0969DA",
                "#FFFFFF",
                "#1F2328",
                false,
                55,
            ),
            Self::builtin(
                "github-dark",
                "GitHub Dark",
                ThemeScheme::Dark,
                "#2F81F7",
                "#0D1117",
                "#E6EDF3",
                false,
                64,
            ),
            Self::builtin(
                "gruvbox-light",
                "Gruvbox Light",
                ThemeScheme::Light,
                "#D65D0E",
                "#FBF1C7",
                "#3C3836",
                true,
                58,
            ),
            Self::builtin(
                "gruvbox-dark",
                "Gruvbox Dark",
                ThemeScheme::Dark,
                "#FABD2F",
                "#282828",
                "#EBDBB2",
                true,
                62,
            ),
            Self::builtin(
                "everforest-light",
                "Everforest Light",
                ThemeScheme::Light,
                "#8DA101",
                "#FDF6E3",
                "#5C6A72",
                true,
                52,
            ),
            Self::builtin(
                "everforest-dark",
                "Everforest Dark",
                ThemeScheme::Dark,
                "#A7C080",
                "#2D353B",
                "#D3C6AA",
                true,
                58,
            ),
            Self::builtin(
                "linear-light",
                "Linear Light",
                ThemeScheme::Light,
                "#5E6AD2",
                "#F7F8FA",
                "#1F232B",
                true,
                60,
            ),
            Self::builtin(
                "linear-dark",
                "Linear Dark",
                ThemeScheme::Dark,
                "#7C85F6",
                "#111318",
                "#F3F4F6",
                true,
                68,
            ),
            Self::builtin(
                "notion-light",
                "Notion Light",
                ThemeScheme::Light,
                "#37352F",
                "#FFFFFF",
                "#37352F",
                false,
                48,
            ),
            Self::builtin(
                "notion-dark",
                "Notion Dark",
                ThemeScheme::Dark,
                "#D3D1CB",
                "#191919",
                "#EDECE9",
                false,
                52,
            ),
            Self::builtin(
                "one-light",
                "One Light",
                ThemeScheme::Light,
                "#4078F2",
                "#FAFAFA",
                "#383A42",
                true,
                56,
            ),
            Self::builtin(
                "one-dark",
                "One Dark",
                ThemeScheme::Dark,
                "#61AFEF",
                "#282C34",
                "#ABB2BF",
                true,
                60,
            ),
            Self::builtin(
                "absolutely-light",
                "Absolutely Light",
                ThemeScheme::Light,
                "#D2694B",
                "#FFF9F5",
                "#382F2A",
                true,
                54,
            ),
            Self::builtin(
                "absolutely-dark",
                "Absolutely Dark",
                ThemeScheme::Dark,
                "#FF9E64",
                "#1A1718",
                "#F4EDEB",
                true,
                66,
            ),
        ]
    }

    #[must_use]
    pub fn builtin_by_id(id: &str) -> Option<Self> {
        Self::builtin_profiles()
            .into_iter()
            .find(|profile| profile.id == id)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty()
            || self.id.len() > 64
            || !self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err("theme id must contain 1-64 ASCII letters, numbers, '-' or '_'".into());
        }
        let name_length = self.name.chars().count();
        if !(1..=64).contains(&name_length) {
            return Err("theme name must contain 1-64 characters".into());
        }
        for (label, color) in [
            ("accent", self.accent.as_str()),
            ("background", self.background.as_str()),
            ("foreground", self.foreground.as_str()),
        ] {
            if !is_hex_color(color) {
                return Err(format!("{label} must be a #RRGGBB color"));
            }
        }
        if self.ui_font.trim().is_empty() || self.ui_font.chars().count() > 256 {
            return Err("UI font stack must contain 1-256 characters".into());
        }
        if self.code_font.trim().is_empty() || self.code_font.chars().count() > 256 {
            return Err("code font stack must contain 1-256 characters".into());
        }
        if self.contrast > 100 {
            return Err("theme contrast must be between 0 and 100".into());
        }
        Ok(())
    }
}

#[must_use]
pub fn is_hex_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppearancePreferences {
    pub pointer_cursor: bool,
    pub reduced_motion: ReducedMotion,
    pub ui_font_size: u8,
    pub code_font_size: u8,
    pub diff_markers: DiffMarkerMode,
}

impl Default for AppearancePreferences {
    fn default() -> Self {
        Self {
            pointer_cursor: false,
            reduced_motion: ReducedMotion::System,
            ui_font_size: 14,
            code_font_size: 12,
            diff_markers: DiffMarkerMode::Color,
        }
    }
}

impl AppearancePreferences {
    pub fn validate(&self) -> Result<(), String> {
        if !(12..=20).contains(&self.ui_font_size) {
            return Err("UI font size must be between 12 and 20".into());
        }
        if !(10..=20).contains(&self.code_font_size) {
            return Err("code font size must be between 10 and 20".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceConfig {
    pub light_theme_id: String,
    pub dark_theme_id: String,
    pub themes: Vec<ThemeProfile>,
    pub preferences: AppearancePreferences,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            light_theme_id: "codex-light".into(),
            dark_theme_id: "codex-dark".into(),
            themes: ThemeProfile::builtin_profiles(),
            preferences: AppearancePreferences::default(),
        }
    }
}

impl AppearanceConfig {
    pub fn merge_missing_builtin_profiles(&mut self) -> bool {
        let mut changed = false;
        for profile in ThemeProfile::builtin_profiles() {
            if self.themes.len() >= MAX_THEME_PROFILES {
                break;
            }
            if self.themes.iter().all(|current| current.id != profile.id) {
                self.themes.push(profile);
                changed = true;
            }
        }
        changed
    }

    #[must_use]
    pub fn selected_id(&self, scheme: ThemeScheme) -> &str {
        match scheme {
            ThemeScheme::Light => &self.light_theme_id,
            ThemeScheme::Dark => &self.dark_theme_id,
        }
    }

    pub fn set_selected_id(&mut self, scheme: ThemeScheme, id: String) {
        match scheme {
            ThemeScheme::Light => self.light_theme_id = id,
            ThemeScheme::Dark => self.dark_theme_id = id,
        }
    }

    #[must_use]
    pub fn profile(&self, id: &str) -> Option<&ThemeProfile> {
        self.themes.iter().find(|profile| profile.id == id)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.themes.is_empty() || self.themes.len() > MAX_THEME_PROFILES {
            return Err(format!(
                "appearance must contain between 1 and {MAX_THEME_PROFILES} themes"
            ));
        }
        let mut ids = BTreeSet::new();
        for profile in &self.themes {
            profile.validate()?;
            if !ids.insert(profile.id.as_str()) {
                return Err(format!("duplicate theme id: {}", profile.id));
            }
        }
        for (id, scheme) in [
            ("codex-light", ThemeScheme::Light),
            ("codex-dark", ThemeScheme::Dark),
        ] {
            let Some(profile) = self.profile(id) else {
                return Err(format!("missing built-in theme: {id}"));
            };
            if !profile.builtin || profile.scheme != scheme {
                return Err(format!("invalid built-in theme identity: {id}"));
            }
        }
        for (scheme, selected_id) in [
            (ThemeScheme::Light, self.light_theme_id.as_str()),
            (ThemeScheme::Dark, self.dark_theme_id.as_str()),
        ] {
            let Some(profile) = self.profile(selected_id) else {
                return Err(format!("selected theme does not exist: {selected_id}"));
            };
            if profile.scheme != scheme {
                return Err(format!(
                    "selected theme has the wrong color scheme: {selected_id}"
                ));
            }
        }
        self.preferences.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ThemeProfileDocument {
    pub format: String,
    pub version: u32,
    pub profile: ThemeProfile,
}

impl ThemeProfileDocument {
    #[must_use]
    pub fn new(profile: ThemeProfile) -> Self {
        Self {
            format: THEME_PROFILE_FORMAT.into(),
            version: THEME_PROFILE_FORMAT_VERSION,
            profile,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.format != THEME_PROFILE_FORMAT || self.version != THEME_PROFILE_FORMAT_VERSION {
            return Err("unsupported Hachimi theme format".into());
        }
        self.profile.validate()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
pub enum Locale {
    #[serde(rename = "zh-CN")]
    #[default]
    ZhCn,
    #[serde(rename = "en-US")]
    EnUs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, Default)]
pub enum WorkbenchRoute {
    #[serde(rename = "home")]
    #[default]
    Home,
    #[serde(rename = "settings/general")]
    SettingsGeneral,
    #[serde(rename = "settings/appearance")]
    SettingsAppearance,
    #[serde(rename = "settings/llm")]
    SettingsLlm,
    #[serde(rename = "settings/avatar")]
    SettingsAvatar,
    #[serde(rename = "settings/motion")]
    SettingsMotion,
    #[serde(rename = "settings/voice")]
    SettingsVoice,
    #[serde(rename = "developer/motion-lab")]
    DeveloperMotionLab,
}

impl WorkbenchRoute {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::SettingsGeneral => "settings/general",
            Self::SettingsAppearance => "settings/appearance",
            Self::SettingsLlm => "settings/llm",
            Self::SettingsAvatar => "settings/avatar",
            Self::SettingsMotion => "settings/motion",
            Self::SettingsVoice => "settings/voice",
            Self::DeveloperMotionLab => "developer/motion-lab",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LlmSettings {
    pub base_url: String,
    pub model_name: String,
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
}

impl Default for LlmSettings {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:11434/v1".into(),
            model_name: "gemma4:e4b".into(),
            max_input_tokens: 0,
            max_output_tokens: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LlmSettingsView {
    pub base_url: String,
    pub model_name: String,
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    pub api_key_configured: bool,
}

impl LlmSettingsView {
    #[must_use]
    pub fn from_settings(settings: &LlmSettings, api_key_configured: bool) -> Self {
        Self {
            base_url: settings.base_url.clone(),
            model_name: settings.model_name.clone(),
            max_input_tokens: settings.max_input_tokens,
            max_output_tokens: settings.max_output_tokens,
            api_key_configured,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LlmSettingsInput {
    pub base_url: String,
    pub model_name: String,
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    /// Missing or blank keeps the existing secret. Secrets are never returned to the WebView.
    pub api_key: Option<String>,
    pub clear_api_key: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LlmTestResult {
    pub success: bool,
    pub latency_ms: u32,
    pub response_preview: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AvatarEntry {
    pub id: String,
    pub name: String,
    pub original_file_name: String,
    pub size_bytes: u32,
    pub sha256: String,
    pub imported_at: String,
    pub is_current: bool,
    pub protected: bool,
    pub format: AvatarFormat,
    pub assessment: AvatarAssessment,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AvatarFormat {
    #[default]
    Glb,
    Vrm0,
    Vrm1,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AvatarCompatibility {
    RuntimeReady,
    #[default]
    Incompatible,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "snake_case")]
pub enum BehaviorChannel {
    Root,
    Locomotion,
    FullBody,
    LowerBody,
    UpperBody,
    LeftArm,
    RightArm,
    Fingers,
    Head,
    Gaze,
    Face,
    Mouth,
    Spring,
}

pub type MotionAssetId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MotionSource {
    Builtin,
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MotionCategory {
    Idle,
    Reaction,
    Gesture,
    Speech,
    Locomotion,
    Performance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MotionPlaybackMode {
    Once,
    Loop,
    Hold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MotionRootMode {
    Discard,
    InPlace,
    Stage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum InteractionRegion {
    HeadTop,
    Face,
    Chest,
    Belly,
    Hips,
    LeftHand,
    RightHand,
    LeftArm,
    RightArm,
    LeftLeg,
    RightLeg,
    Foot,
    Generic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InteractionMotionBinding {
    pub region: InteractionRegion,
    pub motion_id: MotionAssetId,
    pub cooldown_ms: u32,
    pub mirror_by_side: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InteractionMotionPreviewRequest {
    pub region: InteractionRegion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MotionCatalogEntry {
    pub id: MotionAssetId,
    pub source: MotionSource,
    pub protected: bool,
    pub name: String,
    #[serde(default)]
    pub name_zh: String,
    pub description: String,
    #[serde(default)]
    pub description_zh: String,
    pub file_name: String,
    pub sha256: String,
    pub size_bytes: u32,
    pub duration_ms: u32,
    pub category: MotionCategory,
    pub tags: Vec<String>,
    pub playback_mode: MotionPlaybackMode,
    pub root_mode: MotionRootMode,
    pub channels: Vec<BehaviorChannel>,
    pub animated_bones: Vec<String>,
    pub finger_bone_count: u16,
    pub has_finger_motion: bool,
    pub has_expression: bool,
    pub has_look_at: bool,
    pub mirrorable: bool,
    pub transition_in_ms: u16,
    pub transition_out_ms: u16,
    pub source_project: String,
    pub source_paths: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MotionCatalogSnapshot {
    pub entries: Vec<MotionCatalogEntry>,
    pub bindings: Vec<InteractionMotionBinding>,
    pub disabled_motion_ids: Vec<MotionAssetId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MotionImportInspection {
    pub token: Option<String>,
    pub original_file_name: String,
    pub size_bytes: u32,
    pub sha256: String,
    pub duration_ms: u32,
    pub animated_bones: Vec<String>,
    pub finger_bone_count: u16,
    pub has_expression: bool,
    pub has_look_at: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MotionImportCommitRequest {
    pub token: String,
    pub name: String,
    pub description: String,
    pub category: MotionCategory,
    pub playback_mode: MotionPlaybackMode,
    pub root_mode: MotionRootMode,
    #[serde(default)]
    pub interaction_region: Option<InteractionRegion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MotionMetadataUpdateRequest {
    pub id: MotionAssetId,
    pub name: String,
    pub description: String,
    pub category: MotionCategory,
    pub playback_mode: MotionPlaybackMode,
    pub root_mode: MotionRootMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InteractionMotionBindingUpdateRequest {
    pub region: InteractionRegion,
    #[serde(default)]
    pub motion_id: Option<MotionAssetId>,
    #[serde(default)]
    pub cooldown_ms: Option<u32>,
    #[serde(default)]
    pub mirror_by_side: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MotionEnabledUpdateRequest {
    pub id: MotionAssetId,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MotionAssetBindingsClearRequest {
    pub motion_id: MotionAssetId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MotionBindingResetRequest {
    pub region: InteractionRegion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MotionRuntimeAsset {
    pub entry: MotionCatalogEntry,
    pub asset_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MotionChannelWeight {
    pub channel: BehaviorChannel,
    /// Semantic channel weight in permille, clamped by consumers to 0..=1000.
    pub weight: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ClipMotionRequest {
    pub request_id: String,
    pub motion_id: MotionAssetId,
    pub active: bool,
    pub priority: u16,
    pub mirror: bool,
    pub channel_weights: Vec<MotionChannelWeight>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeControllerKind {
    Cursor,
    Drag,
    Contact,
    RootMotion,
    Locomotion,
    Speech,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeControllerRequest {
    pub kind: RuntimeControllerKind,
    pub active: bool,
    /// Controller-specific normalized target. Locomotion currently consumes the X component.
    pub target: [f32; 3],
    pub intensity: f32,
    pub sequence: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AvatarCapability {
    RenderableMesh,
    SkinnedMesh,
    BuiltInAnimations,
    HumanoidSkeleton,
    Blink,
    Viseme,
    LookAt,
    HappyExpression,
    SadExpression,
    AngryExpression,
    SpringBone,
    StandardMotionRetarget,
    RuntimeReady,
    MToon,
    SpringBoneCollider,
    FiveFingerHands,
    FiveVisemes,
    StandardExpressions,
    LipSyncJaw,
    LipSyncFiveViseme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AvatarIssueSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AvatarIssue {
    pub code: String,
    pub severity: AvatarIssueSeverity,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(default, rename_all = "camelCase")]
pub struct AvatarStatistics {
    pub node_count: u32,
    pub mesh_count: u32,
    pub primitive_count: u32,
    pub triangle_count: u32,
    pub material_count: u32,
    pub texture_count: u32,
    pub bone_count: u32,
    pub animation_count: u32,
    pub morph_target_count: u32,
    #[serde(default)]
    pub max_texture_dimension: u32,
    #[serde(default)]
    pub estimated_texture_memory_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AvatarRequirementResult {
    pub requirement: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AvatarAssessment {
    pub compatibility: AvatarCompatibility,
    pub detector_version: u32,
    pub capabilities: Vec<AvatarCapability>,
    pub statistics: AvatarStatistics,
    pub requirements: Vec<AvatarRequirementResult>,
    pub issues: Vec<AvatarIssue>,
}

impl Default for AvatarAssessment {
    fn default() -> Self {
        Self {
            compatibility: AvatarCompatibility::Incompatible,
            detector_version: 0,
            capabilities: Vec::new(),
            statistics: AvatarStatistics::default(),
            requirements: Vec::new(),
            issues: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AvatarExpressionBinding {
    pub expression: String,
    pub node_index: u32,
    pub morph_index: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AvatarRestBone {
    pub bone: String,
    pub node_index: u32,
    pub parent_bone: Option<String>,
    pub local_translation: [f32; 3],
    pub local_rotation: [f32; 4],
    pub length: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AvatarJointLimit {
    pub bone: String,
    pub swing_degrees: f32,
    pub twist_min_degrees: f32,
    pub twist_max_degrees: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AvatarCollisionCapsule {
    pub bone: String,
    pub radius: f32,
    pub half_height: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum LipSyncCapability {
    #[default]
    None,
    Jaw,
    FiveViseme,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AvatarBodyProportions {
    pub height: f32,
    pub shoulder_width: f32,
    pub hip_width: f32,
    pub spine_length: f32,
    pub left_upper_arm_length: f32,
    pub left_lower_arm_length: f32,
    pub right_upper_arm_length: f32,
    pub right_lower_arm_length: f32,
    pub left_upper_leg_length: f32,
    pub left_lower_leg_length: f32,
    pub right_upper_leg_length: f32,
    pub right_lower_leg_length: f32,
    pub left_hand_length: f32,
    pub right_hand_length: f32,
    pub left_foot_length: f32,
    pub right_foot_length: f32,
    pub foot_height: f32,
}

impl Default for AvatarBodyProportions {
    fn default() -> Self {
        Self {
            height: 1.6,
            shoulder_width: 0.36,
            hip_width: 0.28,
            spine_length: 0.48,
            left_upper_arm_length: 0.26,
            left_lower_arm_length: 0.26,
            right_upper_arm_length: 0.26,
            right_lower_arm_length: 0.26,
            left_upper_leg_length: 0.4,
            left_lower_leg_length: 0.4,
            right_upper_leg_length: 0.4,
            right_lower_leg_length: 0.4,
            left_hand_length: 0.16,
            right_hand_length: 0.16,
            left_foot_length: 0.22,
            right_foot_length: 0.22,
            foot_height: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AvatarContactPoint {
    pub id: String,
    pub bone: String,
    pub local_position: [f32; 3],
    pub local_normal: [f32; 3],
    pub radius: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AvatarLookAtProfile {
    pub eye_forward_axis: [f32; 3],
    pub horizontal_inner_degrees: f32,
    pub horizontal_outer_degrees: f32,
    pub vertical_up_degrees: f32,
    pub vertical_down_degrees: f32,
}

impl Default for AvatarLookAtProfile {
    fn default() -> Self {
        Self {
            eye_forward_axis: [0.0, 0.0, -1.0],
            horizontal_inner_degrees: 20.0,
            horizontal_outer_degrees: 30.0,
            vertical_up_degrees: 15.0,
            vertical_down_degrees: 12.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AvatarAdaptationProfile {
    pub vrm_version: AvatarFormat,
    pub bones: Vec<AvatarRestBone>,
    pub expressions: Vec<AvatarExpressionBinding>,
    pub look_at: AvatarLookAtProfile,
    pub spring_bone_group_count: u32,
    pub collider_count: u32,
    pub joint_limits: Vec<AvatarJointLimit>,
    pub proportions: AvatarBodyProportions,
    pub contacts: Vec<AvatarContactPoint>,
    pub collision_capsules: Vec<AvatarCollisionCapsule>,
    pub left_knee_pole: [f32; 3],
    pub right_knee_pole: [f32; 3],
    pub left_elbow_pole: [f32; 3],
    pub right_elbow_pole: [f32; 3],
    pub lip_sync: LipSyncCapability,
    pub has_finger_bones: bool,
    pub has_toe_bones: bool,
}

impl Default for AvatarAdaptationProfile {
    fn default() -> Self {
        Self {
            vrm_version: AvatarFormat::Glb,
            bones: Vec::new(),
            expressions: Vec::new(),
            look_at: AvatarLookAtProfile::default(),
            spring_bone_group_count: 0,
            collider_count: 0,
            joint_limits: Vec::new(),
            proportions: AvatarBodyProportions::default(),
            contacts: Vec::new(),
            collision_capsules: Vec::new(),
            left_knee_pole: [0.0, 0.0, 1.0],
            right_knee_pole: [0.0, 0.0, 1.0],
            left_elbow_pole: [0.0, 0.0, 1.0],
            right_elbow_pole: [0.0, 0.0, 1.0],
            lip_sync: LipSyncCapability::None,
            has_finger_bones: false,
            has_toe_bones: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AvatarImportInspection {
    pub token: Option<String>,
    pub original_file_name: String,
    pub size_bytes: u32,
    pub sha256: String,
    pub format: AvatarFormat,
    pub assessment: AvatarAssessment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AvatarImportCommitRequest {
    pub token: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AvatarCatalogSnapshot {
    pub entries: Vec<AvatarEntry>,
    pub current_id: Option<String>,
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ResourceImportRequest {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ResourceEntryRequest {
    pub id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LogicalRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl LogicalRect {
    #[must_use]
    pub fn has_finite_values(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
    }

    #[must_use]
    pub fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x <= self.x + self.width && y <= self.y + self.height
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InteractiveRegionsUpdate {
    pub window_label: String,
    pub revision: u32,
    pub window_width: f64,
    pub window_height: f64,
    pub regions: Vec<LogicalRect>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WindowPlacementV1 {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub monitor_name: Option<String>,
    pub scale_factor: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub schema_version: u32,
    pub theme: ThemeMode,
    pub locale: Locale,
    pub always_on_top: bool,
    pub pet_placement: Option<WindowPlacementV1>,
    pub llm: LlmSettings,
    pub voice: VoiceSettings,
    pub appearance: AppearanceConfig,
    /// Enables developer-only Workbench surfaces after restart without changing schema 7.
    #[serde(default)]
    pub developer_mode: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            theme: ThemeMode::System,
            locale: Locale::ZhCn,
            always_on_top: true,
            pet_placement: None,
            llm: LlmSettings::default(),
            voice: VoiceSettings::default(),
            appearance: AppearanceConfig::default(),
            developer_mode: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapState {
    pub protocol_version: u32,
    pub window_kind: WindowKind,
    pub locale: Locale,
    pub theme: ThemeMode,
    pub appearance: AppearanceConfig,
    pub always_on_top: bool,
    pub feature_flags: FeatureFlags,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlMethod {
    SystemBootstrap,
    SettingsRead,
    SettingsWrite,
    WindowInteract,
    WorkbenchOpen,
    WorkbenchWindow,
    LlmRead,
    LlmWrite,
    LlmTest,
    LlmChat,
    AvatarRead,
    AvatarManage,
    AvatarRuntime,
    MotionRead,
    MotionManage,
    MotionRuntime,
    VoiceRead,
    VoiceManage,
    VoicePlayback,
    VoiceCapture,
}

impl ControlMethod {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SystemBootstrap => "system.bootstrap",
            Self::SettingsRead => "settings.read",
            Self::SettingsWrite => "settings.write",
            Self::WindowInteract => "window.interact",
            Self::WorkbenchOpen => "workbench.open",
            Self::WorkbenchWindow => "workbench.window",
            Self::LlmRead => "llm.read",
            Self::LlmWrite => "llm.write",
            Self::LlmTest => "llm.test",
            Self::LlmChat => "llm.chat",
            Self::AvatarRead => "avatar.read",
            Self::AvatarManage => "avatar.manage",
            Self::AvatarRuntime => "avatar.runtime",
            Self::MotionRead => "motion.read",
            Self::MotionManage => "motion.manage",
            Self::MotionRuntime => "motion.runtime",
            Self::VoiceRead => "voice.read",
            Self::VoiceManage => "voice.manage",
            Self::VoicePlayback => "voice.playback",
            Self::VoiceCapture => "voice.capture",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "system.bootstrap" => Some(Self::SystemBootstrap),
            "settings.read" => Some(Self::SettingsRead),
            "settings.write" => Some(Self::SettingsWrite),
            "window.interact" => Some(Self::WindowInteract),
            "workbench.open" => Some(Self::WorkbenchOpen),
            "workbench.window" => Some(Self::WorkbenchWindow),
            "llm.read" => Some(Self::LlmRead),
            "llm.write" => Some(Self::LlmWrite),
            "llm.test" => Some(Self::LlmTest),
            "llm.chat" => Some(Self::LlmChat),
            "avatar.read" => Some(Self::AvatarRead),
            "avatar.manage" => Some(Self::AvatarManage),
            "avatar.runtime" => Some(Self::AvatarRuntime),
            "motion.read" => Some(Self::MotionRead),
            "motion.manage" => Some(Self::MotionManage),
            "motion.runtime" => Some(Self::MotionRuntime),
            "voice.read" => Some(Self::VoiceRead),
            "voice.manage" => Some(Self::VoiceManage),
            "voice.playback" => Some(Self::VoicePlayback),
            "voice.capture" => Some(Self::VoiceCapture),
            _ => None,
        }
    }

    #[must_use]
    pub const fn required_scope(self, window_kind: WindowKind) -> Scope {
        match self {
            Self::SystemBootstrap => match window_kind {
                WindowKind::Pet => Scope::PetInteract,
                WindowKind::Settings | WindowKind::Workbench => Scope::SettingsRead,
            },
            Self::SettingsRead => Scope::SettingsRead,
            Self::SettingsWrite => Scope::SettingsWrite,
            Self::WindowInteract => Scope::PetInteract,
            Self::WorkbenchOpen => Scope::WorkbenchOpen,
            Self::WorkbenchWindow => Scope::WorkbenchWindow,
            Self::LlmRead => Scope::LlmRead,
            Self::LlmWrite => Scope::LlmWrite,
            Self::LlmTest => Scope::LlmTest,
            Self::LlmChat => Scope::LlmChat,
            Self::AvatarRead => Scope::AvatarRead,
            Self::AvatarManage => Scope::AvatarManage,
            Self::AvatarRuntime => Scope::AvatarRuntime,
            Self::MotionRead => Scope::MotionRead,
            Self::MotionManage => Scope::MotionManage,
            Self::MotionRuntime => Scope::MotionRuntime,
            Self::VoiceRead => Scope::VoiceRead,
            Self::VoiceManage => Scope::VoiceManage,
            Self::VoicePlayback => Scope::VoicePlayback,
            Self::VoiceCapture => Scope::VoiceCapture,
        }
    }
}

pub fn registered_types() -> specta::Types {
    specta::Types::default()
        .register::<RequestId>()
        .register::<ClientId>()
        .register::<ControlErrorCode>()
        .register::<ControlError>()
        .register::<Scope>()
        .register::<ThemeMode>()
        .register::<ThemeScheme>()
        .register::<ReducedMotion>()
        .register::<DiffMarkerMode>()
        .register::<ThemeProfile>()
        .register::<AppearancePreferences>()
        .register::<AppearanceConfig>()
        .register::<ThemeProfileDocument>()
        .register::<Locale>()
        .register::<WorkbenchRoute>()
        .register::<LlmSettings>()
        .register::<LlmSettingsView>()
        .register::<LlmSettingsInput>()
        .register::<LlmTestResult>()
        .register::<AvatarFormat>()
        .register::<AvatarCompatibility>()
        .register::<BehaviorChannel>()
        .register::<MotionSource>()
        .register::<MotionCategory>()
        .register::<MotionPlaybackMode>()
        .register::<MotionRootMode>()
        .register::<InteractionRegion>()
        .register::<InteractionMotionBinding>()
        .register::<InteractionMotionPreviewRequest>()
        .register::<MotionCatalogEntry>()
        .register::<MotionCatalogSnapshot>()
        .register::<MotionImportInspection>()
        .register::<MotionImportCommitRequest>()
        .register::<MotionMetadataUpdateRequest>()
        .register::<InteractionMotionBindingUpdateRequest>()
        .register::<MotionEnabledUpdateRequest>()
        .register::<MotionAssetBindingsClearRequest>()
        .register::<MotionBindingResetRequest>()
        .register::<MotionRuntimeAsset>()
        .register::<MotionChannelWeight>()
        .register::<ClipMotionRequest>()
        .register::<RuntimeControllerKind>()
        .register::<RuntimeControllerRequest>()
        .register::<AvatarCapability>()
        .register::<AvatarIssueSeverity>()
        .register::<AvatarIssue>()
        .register::<AvatarStatistics>()
        .register::<AvatarRequirementResult>()
        .register::<AvatarAssessment>()
        .register::<AvatarExpressionBinding>()
        .register::<AvatarRestBone>()
        .register::<AvatarJointLimit>()
        .register::<AvatarCollisionCapsule>()
        .register::<LipSyncCapability>()
        .register::<AvatarBodyProportions>()
        .register::<AvatarContactPoint>()
        .register::<AvatarLookAtProfile>()
        .register::<AvatarAdaptationProfile>()
        .register::<AvatarImportInspection>()
        .register::<AvatarImportCommitRequest>()
        .register::<AvatarEntry>()
        .register::<AvatarCatalogSnapshot>()
        .register::<VoiceSettings>()
        .register::<VoiceSettingsInput>()
        .register::<SpeechRecognitionSettingsInput>()
        .register::<VoiceComputeMode>()
        .register::<VoiceComputeBackend>()
        .register::<VoiceModelOrigin>()
        .register::<VoiceModelEntry>()
        .register::<VoiceCatalogSnapshot>()
        .register::<VoiceModelInspection>()
        .register::<VoiceImportCommitRequest>()
        .register::<AvatarRuntimeAsset>()
        .register::<SpeechTimelineQuality>()
        .register::<SpeechVisemeFrame>()
        .register::<SpeechTimeline>()
        .register::<SpeechPlaybackSource>()
        .register::<SpeechPlaybackPhase>()
        .register::<SpeechPlaybackEvent>()
        .register::<SpeechTurnPhase>()
        .register::<SpeechTurnEvent>()
        .register::<VoiceRuntimeState>()
        .register::<SpeechRecognitionRuntimeState>()
        .register::<PetTurnRequest>()
        .register::<PetContextMenuRequest>()
        .register::<PetTurnEvent>()
        .register::<FrontendLogLevel>()
        .register::<FrontendLogEntry>()
        .register::<ResourceImportRequest>()
        .register::<ResourceEntryRequest>()
        .register::<LogicalRect>()
        .register::<InteractiveRegionsUpdate>()
        .register::<WindowPlacementV1>()
        .register::<AppSettings>()
        .register::<BootstrapState>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workbench_has_only_settings_and_catalog_scopes() {
        let scopes = ClientContext::for_window(WindowKind::Workbench).scopes;
        assert!(scopes.contains(&Scope::LlmTest));
        assert!(scopes.contains(&Scope::AvatarManage));
        assert!(scopes.contains(&Scope::VoiceManage));
        assert!(!scopes.contains(&Scope::VoiceCapture));
        assert!(!scopes.contains(&Scope::WorkspaceRead));
        assert!(!scopes.contains(&Scope::WorkspaceExec));
    }

    #[test]
    fn only_pet_can_capture_speech() {
        let pet = ClientContext::for_window(WindowKind::Pet);
        let workbench = ClientContext::for_window(WindowKind::Workbench);
        assert!(pet.scopes.contains(&Scope::VoiceCapture));
        assert!(!workbench.scopes.contains(&Scope::VoiceCapture));
    }

    #[test]
    fn scope_deserialization_is_exact() {
        assert!(serde_json::from_str::<Scope>("\"computer.control\"").is_ok());
        assert!(serde_json::from_str::<Scope>("\"computer\"").is_err());
        assert!(serde_json::from_str::<Scope>("\"computer.control.extra\"").is_err());
    }

    #[test]
    fn default_settings_are_versioned() {
        assert_eq!(
            AppSettings::default().schema_version,
            SETTINGS_SCHEMA_VERSION
        );
    }

    #[test]
    fn default_appearance_is_valid_and_has_both_schemes() {
        let appearance = AppearanceConfig::default();
        assert!(appearance.validate().is_ok());
        assert_eq!(appearance.themes.len(), 18);
        assert!(appearance.profile("catppuccin-dark").is_some());
        assert!(appearance.profile("github-light").is_some());
        assert_eq!(
            appearance
                .profile(&appearance.light_theme_id)
                .unwrap()
                .scheme,
            ThemeScheme::Light
        );
        assert_eq!(
            appearance
                .profile(&appearance.dark_theme_id)
                .unwrap()
                .scheme,
            ThemeScheme::Dark
        );
    }

    #[test]
    fn missing_builtins_are_added_without_overwriting_existing_profiles() {
        let mut appearance = AppearanceConfig::default();
        appearance
            .themes
            .retain(|profile| profile.id.starts_with("codex-"));
        appearance.themes[0].accent = "#123456".into();
        assert!(appearance.merge_missing_builtin_profiles());
        assert_eq!(appearance.themes.len(), 18);
        assert_eq!(appearance.profile("codex-light").unwrap().accent, "#123456");
        assert!(!appearance.merge_missing_builtin_profiles());
    }

    #[test]
    fn theme_document_validation_is_strict() {
        let mut document = ThemeProfileDocument::new(ThemeProfile::codex_dark());
        assert!(document.validate().is_ok());
        document.profile.accent = "red".into();
        assert!(document.validate().is_err());
        document.profile.accent = "#2EA8FF".into();
        document.version += 1;
        assert!(document.validate().is_err());
    }
}
