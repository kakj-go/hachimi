//! Transport-neutral protocol and serializable desktop contracts.

mod agent;
mod control;
mod motion;
mod provider;
mod settings;
mod transport;
mod voice;
mod workspace;

pub use control::ControlMethod;
pub use motion::*;
pub use provider::*;

pub use agent::*;
pub use settings::*;
pub use transport::*;
pub use voice::*;
pub use workspace::*;

use std::collections::BTreeSet;

use hachimi_core::{FeatureFlags, RuntimeFeatureSet, WindowKind};
use serde::{Deserialize, Serialize};
use specta::Type;

pub const CONTROL_PROTOCOL_VERSION: u32 = 31;
pub const PLUGIN_UI_BRIDGE_PROTOCOL_VERSION: u32 = 1;
pub const SETTINGS_SCHEMA_VERSION: u32 = 8;
pub const THEME_PROFILE_FORMAT: &str = "hachimi-theme";
pub const THEME_PROFILE_FORMAT_VERSION: u32 = 1;
pub const MAX_THEME_PROFILES: usize = 32;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum UiDensity {
    Compact,
    #[default]
    Default,
    Comfortable,
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
            "Quiet Graphite Light",
            ThemeScheme::Light,
            "#4358C5",
            "#F8F7F3",
            "#24272D",
            true,
            54,
        )
    }

    #[must_use]
    pub fn codex_dark() -> Self {
        Self::builtin(
            "codex-dark",
            "Quiet Graphite",
            ThemeScheme::Dark,
            "#7062D5",
            "#111316",
            "#F1F3F5",
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
    #[serde(default)]
    pub density: UiDensity,
}

impl Default for AppearancePreferences {
    fn default() -> Self {
        Self {
            pointer_cursor: false,
            reduced_motion: ReducedMotion::System,
            ui_font_size: 14,
            code_font_size: 12,
            diff_markers: DiffMarkerMode::Color,
            density: UiDensity::Default,
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
        for profile in &mut self.themes {
            let replacement = match profile.id.as_str() {
                "codex-light"
                    if profile.builtin
                        && profile.accent.eq_ignore_ascii_case("#1677D2")
                        && profile.background.eq_ignore_ascii_case("#F5F4F7")
                        && profile.foreground.eq_ignore_ascii_case("#202126") =>
                {
                    Some(ThemeProfile::codex_light())
                }
                "codex-dark"
                    if profile.builtin
                        && profile.accent.eq_ignore_ascii_case("#2EA8FF")
                        && profile.background.eq_ignore_ascii_case("#151616")
                        && profile.foreground.eq_ignore_ascii_case("#F1F1F3") =>
                {
                    Some(ThemeProfile::codex_dark())
                }
                _ => None,
            };
            if let Some(replacement) = replacement {
                *profile = replacement;
                changed = true;
            }
        }
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
    #[serde(rename = "settings/skills")]
    SettingsSkills,
    #[serde(rename = "settings/mcp")]
    SettingsMcp,
    #[serde(rename = "settings/integrations")]
    SettingsIntegrations,
    #[serde(rename = "settings/browser")]
    SettingsBrowser,
    #[serde(rename = "settings/computer-use")]
    SettingsComputerUse,
    #[serde(rename = "settings/runtime-security")]
    SettingsRuntimeSecurity,
    #[serde(rename = "settings/diagnostics")]
    SettingsDiagnostics,
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
            Self::SettingsSkills => "settings/skills",
            Self::SettingsMcp => "settings/mcp",
            Self::SettingsIntegrations => "settings/integrations",
            Self::SettingsBrowser => "settings/browser",
            Self::SettingsComputerUse => "settings/computer-use",
            Self::SettingsRuntimeSecurity => "settings/runtime-security",
            Self::SettingsDiagnostics => "settings/diagnostics",
            Self::DeveloperMotionLab => "developer/motion-lab",
        }
    }
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MotionAnalysisStatus {
    Pending,
    #[default]
    Ready,
    Failed,
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
    #[serde(default)]
    pub analysis_status: MotionAnalysisStatus,
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
    pub family: MotionFamily,
    pub tags: Vec<String>,
    pub loop_mode: MotionLoopMode,
    #[serde(default)]
    pub derived_from_motion_id: Option<MotionAssetId>,
    #[serde(default)]
    pub motion_role: Option<MotionRole>,
    #[serde(default)]
    pub source_start_ms: Option<u32>,
    #[serde(default)]
    pub source_end_ms: Option<u32>,
    #[serde(default)]
    pub procedural_yaw_degrees: Option<i16>,
    pub root_mode: MotionRootMode,
    pub slot: MotionSlot,
    pub channel_mask: Vec<BehaviorChannel>,
    pub transition_profile_id: String,
    pub fallback_motion_id: MotionAssetId,
    pub animated_bones: Vec<String>,
    pub finger_bone_count: u16,
    pub has_finger_motion: bool,
    pub has_expression: bool,
    pub has_look_at: bool,
    pub mirrorable: bool,
    pub source_project: String,
    pub source_paths: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MotionCatalogSnapshot {
    pub entries: Vec<MotionCatalogEntry>,
    pub transition_profiles: Vec<MotionTransitionProfile>,
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
    pub loop_mode: MotionLoopMode,
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
    pub family: MotionFamily,
    pub loop_mode: MotionLoopMode,
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

pub fn registered_types() -> specta::Types {
    specta::Types::default()
        .register::<RequestId>()
        .register::<ClientId>()
        .register::<ControlErrorCode>()
        .register::<ControlError>()
        .register::<Scope>()
        .register::<ProjectId>()
        .register::<CheckoutId>()
        .register::<SessionId>()
        .register::<RunId>()
        .register::<ItemId>()
        .register::<ToolCallId>()
        .register::<AttachmentId>()
        .register::<SessionSourceId>()
        .register::<ApprovalId>()
        .register::<TaskRunId>()
        .register::<AgentTaskId>()
        .register::<AgentTaskMessageId>()
        .register::<ForgeOperationId>()
        .register::<PlanId>()
        .register::<ArtifactId>()
        .register::<CompactionCheckpointId>()
        .register::<McpServerId>()
        .register::<SkillId>()
        .register::<SkillSubscriptionId>()
        .register::<UserInputRequestId>()
        .register::<ProcessSessionId>()
        .register::<ReviewId>()
        .register::<ReviewFindingId>()
        .register::<RealtimeSessionId>()
        .register::<PlanStepId>()
        .register::<EventSubscriptionId>()
        .register::<FsWatchId>()
        .register::<FsSearchId>()
        .register::<SideEffectExecutionId>()
        .register::<RunStepCheckpointId>()
        .register::<RunRecoveryId>()
        .register::<AvatarId>()
        .register::<ScheduleId>()
        .register::<BrowserSessionId>()
        .register::<BrowserObservationId>()
        .register::<BrowserPairingId>()
        .register::<BrowserWorkspaceId>()
        .register::<BrowserTabId>()
        .register::<BrowserAutomationLeaseId>()
        .register::<ComputerFrameId>()
        .register::<ComputerControlSessionId>()
        .register::<PluginId>()
        .register::<ConnectorAccountId>()
        .register::<ChannelMessageId>()
        .register::<ChannelDeliveryId>()
        .register::<SkillActivationId>()
        .register::<ExecutionTarget>()
        .register::<RunDriverKind>()
        .register::<RunPurpose>()
        .register::<EntryProfile>()
        .register::<WorkloadKind>()
        .register::<WorkloadResolutionSource>()
        .register::<WorkloadResolution>()
        .register::<SessionContextBinding>()
        .register::<RunOrigin>()
        .register::<BehaviorMode>()
        .register::<ApprovalPolicy>()
        .register::<PermissionProfile>()
        .register::<SessionPermissionConfig>()
        .register::<SessionExtraAuthorizationSummary>()
        .register::<SessionPermissionConfigRequest>()
        .register::<SessionPermissionConfigUpdate>()
        .register::<RunBudget>()
        .register::<RunConfiguration>()
        .register::<CheckoutKind>()
        .register::<CheckoutStatus>()
        .register::<RunStatus>()
        .register::<TaskRunStatus>()
        .register::<ScheduleSpec>()
        .register::<ScheduleEventSourceKind>()
        .register::<ScheduleEventSource>()
        .register::<ScheduleEventResourceRef>()
        .register::<ScheduleEventMatcher>()
        .register::<ScheduleEventEnvelope>()
        .register::<ScheduleEventIngressRequest>()
        .register::<ScheduleEventContext>()
        .register::<ScheduleEventReceiptStatus>()
        .register::<ScheduleEventReceipt>()
        .register::<ScheduleContextTemplate>()
        .register::<ScheduleStopConditions>()
        .register::<ConnectorRevisionSelection>()
        .register::<HostRevisionSnapshot>()
        .register::<MisfirePolicy>()
        .register::<DeliveryPolicy>()
        .register::<ScheduleHealth>()
        .register::<McpToolSelection>()
        .register::<ScheduleSkillSelection>()
        .register::<ScheduleDefinition>()
        .register::<TaskRunTrigger>()
        .register::<DeliveryStatus>()
        .register::<ScheduleCreateRequest>()
        .register::<ScheduleUpdateRequest>()
        .register::<SchedulePreview>()
        .register::<ScheduleSnapshot>()
        .register::<TaskInteractiveContinuation>()
        .register::<UserInputStatus>()
        .register::<UserInputOption>()
        .register::<UserInputQuestion>()
        .register::<UserInputAnswer>()
        .register::<UserInputDisplayAnswer>()
        .register::<UserInputRequestRecord>()
        .register::<UserInputResolution>()
        .register::<UserInputResolutionAction>()
        .register::<ItemStatus>()
        .register::<PlanStepStatus>()
        .register::<PlanStep>()
        .register::<ItemRelations>()
        .register::<TranscriptItemKind>()
        .register::<AgentMessagePhase>()
        .register::<ItemPayload>()
        .register::<ToolExecutionResult>()
        .register::<TranscriptItem>()
        .register::<ItemDeltaPayload>()
        .register::<CompactionReason>()
        .register::<CompactionTrigger>()
        .register::<CompactionPhase>()
        .register::<CompactionImplementation>()
        .register::<CompactionSummarySource>()
        .register::<ReasoningSummarySource>()
        .register::<TokenCountSource>()
        .register::<CompactionTokenSnapshot>()
        .register::<CompactionLifecycle>()
        .register::<CompactionSummary>()
        .register::<CompactionQuality>()
        .register::<CompactionCheckpoint>()
        .register::<RunUsageSnapshot>()
        .register::<SideEffectExecutionStatus>()
        .register::<SideEffectExecutionRecord>()
        .register::<RunStepPhase>()
        .register::<ToolRecoveryPolicy>()
        .register::<RunRecoveryState>()
        .register::<RunRecoveryDecisionAction>()
        .register::<RecoveryRevisionSnapshot>()
        .register::<RunStepCheckpoint>()
        .register::<RunRecoveryRecord>()
        .register::<RunRecoveryDecisionRequest>()
        .register::<RunRecoverySnapshot>()
        .register::<FsEntryKind>()
        .register::<FsEntry>()
        .register::<FsListRequest>()
        .register::<FsListPage>()
        .register::<FsReadChunkRequest>()
        .register::<FsFileChunk>()
        .register::<FsWriteRequest>()
        .register::<FsWriteResponse>()
        .register::<GitFileStatus>()
        .register::<GitCommitSummary>()
        .register::<GitWorkspaceRequest>()
        .register::<GitWorkspaceSnapshot>()
        .register::<GitMutation>()
        .register::<GitMutationRequest>()
        .register::<GitMutationResponse>()
        .register::<FsChangeKind>()
        .register::<FsWatchRequest>()
        .register::<FsWatchRegistration>()
        .register::<FsChangeEvent>()
        .register::<FsSearchStartRequest>()
        .register::<FsSearchUpdateRequest>()
        .register::<FsSearchResult>()
        .register::<FsSearchSnapshot>()
        .register::<DiffScope>()
        .register::<FileDiffStatus>()
        .register::<DiffLine>()
        .register::<DiffHunk>()
        .register::<FileDiffSummary>()
        .register::<RunDiffSnapshot>()
        .register::<DiffReadFileRequest>()
        .register::<DiffReadFileResponse>()
        .register::<RunEventEnvelope>()
        .register::<RunEventPayload>()
        .register::<PlanConfirmationStatus>()
        .register::<PlanDocument>()
        .register::<PlanConfirmation>()
        .register::<ExecutionPlanState>()
        .register::<ProjectRecord>()
        .register::<ProjectGitState>()
        .register::<ProjectGitSnapshot>()
        .register::<ProjectGitInitialCommitRequest>()
        .register::<ProjectGitInitialCommitResponse>()
        .register::<AttachmentRecord>()
        .register::<CheckoutRecord>()
        .register::<SessionRecord>()
        .register::<RunRecord>()
        .register::<TaskRunRecord>()
        .register::<AgentTaskStatus>()
        .register::<AgentTaskRecord>()
        .register::<AgentTaskMessageRecord>()
        .register::<AgentTaskCollection>()
        .register::<ForgeKind>()
        .register::<GitRemoteRecord>()
        .register::<GitRemoteListRequest>()
        .register::<GitPushRequest>()
        .register::<GitPushResponse>()
        .register::<ForgeRepositoryIdentity>()
        .register::<ForgeChangeState>()
        .register::<ForgeChangeRecord>()
        .register::<ForgeChangeMutation>()
        .register::<ForgeChangeQueryRequest>()
        .register::<ForgeChangeMutationRequest>()
        .register::<ForgeCredentialUpdateRequest>()
        .register::<ForgeCredentialState>()
        .register::<ForgeOperationStatus>()
        .register::<ForgeOperationRecord>()
        .register::<SkillDiagnosticSeverity>()
        .register::<SkillDiagnostic>()
        .register::<SkillEntryKind>()
        .register::<SkillEditorKind>()
        .register::<SkillScope>()
        .register::<SkillToolDependency>()
        .register::<SkillTreeNode>()
        .register::<SkillRecord>()
        .register::<SkillActivationSource>()
        .register::<SkillClassification>()
        .register::<SkillActivation>()
        .register::<SkillFileSnapshot>()
        .register::<SkillFileWriteRequest>()
        .register::<SkillPreviewResourceRequest>()
        .register::<SkillPreviewResource>()
        .register::<SkillEntryCreateRequest>()
        .register::<SkillEntryRenameRequest>()
        .register::<SkillChangeKind>()
        .register::<SkillChangeEvent>()
        .register::<McpServerTransport>()
        .register::<McpHeaderView>()
        .register::<McpHeaderInput>()
        .register::<McpServerRecord>()
        .register::<McpServerUpsertRequest>()
        .register::<McpServerHealthState>()
        .register::<McpServerHealthRecord>()
        .register::<McpServerView>()
        .register::<McpToolView>()
        .register::<McpToolOverride>()
        .register::<McpConnectionTestResult>()
        .register::<McpResource>()
        .register::<McpResourceTemplate>()
        .register::<McpResourceContent>()
        .register::<McpMediaReference>()
        .register::<McpPromptArgument>()
        .register::<McpPrompt>()
        .register::<McpPromptRole>()
        .register::<McpPromptMessage>()
        .register::<McpPromptResult>()
        .register::<McpInventorySnapshot>()
        .register::<McpResourceReadRequest>()
        .register::<McpPromptGetRequest>()
        .register::<McpCallOutcome>()
        .register::<McpCallSummaryRecord>()
        .register::<McpCallSummaryListRequest>()
        .register::<McpToolProgressRecord>()
        .register::<McpAuthStatus>()
        .register::<McpAuthStatusRecord>()
        .register::<McpOAuthLoginRequest>()
        .register::<McpOAuthLoginResponse>()
        .register::<ArtifactKind>()
        .register::<ArtifactRecord>()
        .register::<ApprovalStatus>()
        .register::<ApprovalGrantScope>()
        .register::<ApprovalRequestRecord>()
        .register::<ApprovalResolution>()
        .register::<ToolEffect>()
        .register::<ToolDescriptor>()
        .register::<DynamicToolRegistration>()
        .register::<DynamicToolValidationError>()
        .register::<ModelRole>()
        .register::<ModelToolCall>()
        .register::<ModelInputImage>()
        .register::<ModelMessage>()
        .register::<ModelRequest>()
        .register::<ModelCompactionRequest>()
        .register::<ModelCompactionResult>()
        .register::<TokenUsage>()
        .register::<ModelFinishReason>()
        .register::<ModelEvent>()
        .register::<ProviderCapabilities>()
        .register::<ProviderProtocolKind>()
        .register::<ProviderCompatibilityProfileKind>()
        .register::<ProviderCompatibilityProfile>()
        .register::<ProviderEndpointRecord>()
        .register::<ProviderAccountRecord>()
        .register::<ProviderEndpointUpsertRequest>()
        .register::<ProviderProbeStatus>()
        .register::<ProviderProbeReport>()
        .register::<ProviderRegistrySnapshot>()
        .register::<ProviderEmbeddingRequest>()
        .register::<ProviderEmbeddingVector>()
        .register::<ProviderEmbeddingResult>()
        .register::<CapabilityDegradation>()
        .register::<MutationContext>()
        .register::<PermissionGrantScope>()
        .register::<FileSystemAccess>()
        .register::<FileSystemGrant>()
        .register::<NetworkGrant>()
        .register::<ProcessGrant>()
        .register::<BrowserGrant>()
        .register::<ComputerGrant>()
        .register::<CapabilityGrantSet>()
        .register::<SandboxReadiness>()
        .register::<SandboxCapabilityReport>()
        .register::<SandboxRuntimeSnapshot>()
        .register::<SandboxRepairRequest>()
        .register::<SandboxBootstrapPhase>()
        .register::<SandboxBootstrapState>()
        .register::<BrowserProfileKind>()
        .register::<BrowserAutomationSurfaceKind>()
        .register::<BrowserAutomationPreference>()
        .register::<HostPolicyDecision>()
        .register::<BrowserSitePolicy>()
        .register::<BrowserSitePolicyUpdate>()
        .register::<HostAccessRequestStatus>()
        .register::<HostAccessTarget>()
        .register::<HostAccessRequestRecord>()
        .register::<HostAccessDecision>()
        .register::<HostAccessDecisionRequest>()
        .register::<BrowserOpenTarget>()
        .register::<BrowserWorkspaceRuntimeState>()
        .register::<BrowserNavigationErrorKind>()
        .register::<BrowserNavigationError>()
        .register::<BrowserTabSnapshot>()
        .register::<BrowserWorkspace>()
        .register::<BrowserAutomationLeaseStatus>()
        .register::<BrowserAutomationLease>()
        .register::<BrowserSurfaceBounds>()
        .register::<BrowserWorkspaceMutation>()
        .register::<BrowserWorkspaceMutationRequest>()
        .register::<BrowserSurfaceLayoutRequest>()
        .register::<BrowserHistoryEntry>()
        .register::<EmbeddedBrowserSettings>()
        .register::<EmbeddedBrowserSettingsUpdate>()
        .register::<BrowserDataKind>()
        .register::<ClearEmbeddedBrowserDataRequest>()
        .register::<BrowserDownloadStatus>()
        .register::<BrowserDownloadSnapshot>()
        .register::<BrowserDownloadAction>()
        .register::<BrowserDownloadActionRequest>()
        .register::<BrowserWorkspaceChangeReason>()
        .register::<BrowserWorkspaceChanged>()
        .register::<BrowserCapability>()
        .register::<BrowserPermissionDecision>()
        .register::<EmbeddedBrowserPermissionScope>()
        .register::<EmbeddedBrowserPermissionRequest>()
        .register::<EmbeddedBrowserSitePermission>()
        .register::<EmbeddedBrowserPermissionResolutionRequest>()
        .register::<EmbeddedBrowserPermissionRequiredEvent>()
        .register::<BrowserNetworkRuleKind>()
        .register::<BrowserNetworkRule>()
        .register::<BrowserNetworkPolicy>()
        .register::<BrowserPermissionRequestStatus>()
        .register::<BrowserPermissionRequest>()
        .register::<BrowserPermissionRequiredEvent>()
        .register::<BrowserSitePermission>()
        .register::<BrowserPermissionLedgerEntry>()
        .register::<BrowserSessionStatus>()
        .register::<BrowserSession>()
        .register::<BrowserObservation>()
        .register::<ExternalBrowserLeaseObservation>()
        .register::<BrowserWaitState>()
        .register::<BrowserAction>()
        .register::<BrowserActionRequest>()
        .register::<BrowserActionResult>()
        .register::<BrowserPairing>()
        .register::<BrowserHostSettings>()
        .register::<BrowserHostSettingsUpdate>()
        .register::<SystemBrowserKind>()
        .register::<SystemBrowserInstallation>()
        .register::<ComputerWindowIdentity>()
        .register::<ComputerAppDescriptor>()
        .register::<ComputerAppCandidate>()
        .register::<PermissionCommandCandidate>()
        .register::<ComputerAppPolicy>()
        .register::<ComputerAppPolicyUpdate>()
        .register::<ComputerHostSettings>()
        .register::<ComputerRuntimeHealth>()
        .register::<ComputerHostSettingsUpdate>()
        .register::<ComputerFrame>()
        .register::<ComputerFramePreview>()
        .register::<ComputerControlStatus>()
        .register::<ComputerControlSession>()
        .register::<ComputerAction>()
        .register::<ComputerActionRequest>()
        .register::<ComputerActionResult>()
        .register::<ComputerAppRule>()
        .register::<PluginContribution>()
        .register::<PluginContributionKind>()
        .register::<PluginHookEvent>()
        .register::<PluginHookRuntimeKind>()
        .register::<PluginHookDescriptor>()
        .register::<PluginHookInvocation>()
        .register::<PluginHookMetadataEntry>()
        .register::<PluginHookOutcome>()
        .register::<PluginUiBridgeMethod>()
        .register::<PluginContributionSurface>()
        .register::<PluginUiContext>()
        .register::<PluginUiBridgeRequest>()
        .register::<PluginUiBridgeResponse>()
        .register::<ContributionRuntimeState>()
        .register::<PluginRevisionStatus>()
        .register::<PluginLifecycleOperation>()
        .register::<PluginLifecyclePhase>()
        .register::<PluginLifecycleJournalStatus>()
        .register::<PluginRevisionRecord>()
        .register::<PluginRevisionHead>()
        .register::<PluginLifecycleJournalRecord>()
        .register::<InstalledContribution>()
        .register::<PluginPermissionDiff>()
        .register::<PluginManifest>()
        .register::<PluginStatus>()
        .register::<InstalledPlugin>()
        .register::<ConnectorHealth>()
        .register::<ConnectorRevision>()
        .register::<ConnectorRuntimeKind>()
        .register::<ConnectorActionEffect>()
        .register::<ConnectorActionDescriptor>()
        .register::<ConnectorDriverDescriptor>()
        .register::<ConnectorAccount>()
        .register::<ConnectorAccountUpsert>()
        .register::<ConnectorInvocationRequest>()
        .register::<ConnectorInvocationResult>()
        .register::<ContributionRevision>()
        .register::<IntegrationProviderId>()
        .register::<IntegrationTransport>()
        .register::<ChannelAccountState>()
        .register::<IntegrationCapability>()
        .register::<IntegrationAuthMethod>()
        .register::<CredentialFieldKind>()
        .register::<CredentialFieldDefinition>()
        .register::<IntegrationProviderDefinition>()
        .register::<IntegrationCredentialInput>()
        .register::<IntegrationProviderAccount>()
        .register::<IntegrationAccountUpsert>()
        .register::<IntegrationAccountCapabilitiesUpdate>()
        .register::<IntegrationProbeDimension>()
        .register::<IntegrationAccountProbeSnapshot>()
        .register::<IntegrationAccountProbeResult>()
        .register::<IlinkQrSession>()
        .register::<IlinkQrLoginRequest>()
        .register::<EnterpriseAttachmentDownloadRequest>()
        .register::<EnterpriseAttachmentDownloadResult>()
        .register::<EnterprisePluginIdentity>()
        .register::<ChannelChatKind>()
        .register::<ChannelConversationAddress>()
        .register::<ChannelEventKey>()
        .register::<ChannelActor>()
        .register::<ChannelMessagePartKind>()
        .register::<RemoteMediaDescriptor>()
        .register::<ChannelMessagePart>()
        .register::<ChannelMentionKind>()
        .register::<ChannelMention>()
        .register::<ChannelQuoteContext>()
        .register::<ChannelDmPolicy>()
        .register::<ChannelGroupHistoryPolicy>()
        .register::<ChannelTopicPolicy>()
        .register::<ChannelMentionPolicy>()
        .register::<ChannelAuthorizationTarget>()
        .register::<ChannelGrant>()
        .register::<ChannelAccessPolicy>()
        .register::<ChannelAccessPolicyUpsert>()
        .register::<ChannelAuthorization>()
        .register::<ChannelAuthorizationUpsert>()
        .register::<ChannelPairingCodeRequest>()
        .register::<ChannelPairingCode>()
        .register::<ChannelIdentityLinkCodeRequest>()
        .register::<ChannelIdentityLinkCode>()
        .register::<ChannelIdentityGroup>()
        .register::<ChannelIdentityTransferMember>()
        .register::<ChannelIdentityTransferPreview>()
        .register::<ChannelIdentityTransferCommitRequest>()
        .register::<ChannelIdentityTransferResult>()
        .register::<ChannelProviderRuntimeKind>()
        .register::<ChannelProviderManifest>()
        .register::<ChannelProviderHealthState>()
        .register::<ChannelProviderHealth>()
        .register::<ChannelProviderAccount>()
        .register::<ChannelProviderAccountUpsert>()
        .register::<ChannelConfigRevision>()
        .register::<VerifiedChannelMessage>()
        .register::<IngressStatus>()
        .register::<IngressReceipt>()
        .register::<DeliveryAttemptStatus>()
        .register::<ChannelOutboundPayload>()
        .register::<DeliveryAttempt>()
        .register::<GatewayHealth>()
        .register::<RuntimeComponentId>()
        .register::<RuntimeComponentState>()
        .register::<RuntimeComponentHealth>()
        .register::<RuntimeHealthSnapshot>()
        .register::<ProcessStatus>()
        .register::<ProcessSessionRecord>()
        .register::<ProcessTerminalSize>()
        .register::<ProcessOutputStream>()
        .register::<ProcessSpawnRequest>()
        .register::<ProcessWriteRequest>()
        .register::<ProcessResizeRequest>()
        .register::<ProcessTerminateRequest>()
        .register::<ProcessReadRequest>()
        .register::<ProcessListRequest>()
        .register::<ProcessOutputChunk>()
        .register::<ProcessReadSnapshot>()
        .register::<ProcessEvent>()
        .register::<ReviewDelivery>()
        .register::<ReviewTarget>()
        .register::<ReviewSeverity>()
        .register::<ReviewFindingStatus>()
        .register::<ReviewRecord>()
        .register::<ReviewFinding>()
        .register::<ReviewStartRequest>()
        .register::<ReviewStartSnapshot>()
        .register::<ReviewSnapshot>()
        .register::<ReviewFindingUpdateRequest>()
        .register::<ControlInitializeRequest>()
        .register::<ControlInitializeResponse>()
        .register::<SessionCursor>()
        .register::<SessionSearchRequest>()
        .register::<SessionPage>()
        .register::<RunSteerStatus>()
        .register::<RunSteerRecord>()
        .register::<SessionResumeRequest>()
        .register::<SessionResumeSnapshot>()
        .register::<EventSubscriptionRequest>()
        .register::<EventSubscriptionRecord>()
        .register::<EventSubscriptionSnapshot>()
        .register::<SessionForkRequest>()
        .register::<SessionMetadataUpdateRequest>()
        .register::<RunControlRequest>()
        .register::<GitRefRecord>()
        .register::<WorkbenchGitAction>()
        .register::<WorkbenchGitRequest>()
        .register::<WorkbenchGitPhaseStatus>()
        .register::<WorkbenchGitPhaseResult>()
        .register::<WorkbenchGitResponse>()
        .register::<SessionSourceKind>()
        .register::<SessionSourceOrigin>()
        .register::<SessionSourceRecord>()
        .register::<EnvironmentChangeSummary>()
        .register::<EnvironmentGitSummary>()
        .register::<EnvironmentActivity>()
        .register::<EnvironmentHandoffState>()
        .register::<WorkbenchEnvironmentSnapshot>()
        .register::<WorkbenchEnvironmentChangeReason>()
        .register::<WorkbenchEnvironmentChanged>()
        .register::<WorkbenchHandoffRequest>()
        .register::<WorkbenchHandoffResponse>()
        .register::<WorkbenchTaskStartRequest>()
        .register::<WorkbenchTaskSnapshot>()
        .register::<ManualCompactionRequest>()
        .register::<ManualCompactionStatus>()
        .register::<ManualCompactionResult>()
        .register::<WorkbenchAttachmentPreview>()
        .register::<SessionRunActivity>()
        .register::<WorkbenchSessionListItem>()
        .register::<RunSummaryFile>()
        .register::<RunSummaryRecord>()
        .register::<WorkbenchSessionSnapshot>()
        .register::<ApprovalDecisionRequest>()
        .register::<PlanAcceptanceRequest>()
        .register::<PlanRevisionRequest>()
        .register::<PlanSkipRequest>()
        .register::<WorkbenchPlanAcceptanceSnapshot>()
        .register::<WorkbenchPlanSkipSnapshot>()
        .register::<ThemeMode>()
        .register::<ThemeScheme>()
        .register::<ReducedMotion>()
        .register::<DiffMarkerMode>()
        .register::<UiDensity>()
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
        .register::<ReasoningSummaryMode>()
        .register::<StructuredOutputMode>()
        .register::<ProviderCapabilityProbeSource>()
        .register::<ProviderCapabilityProbe>()
        .register::<AvatarFormat>()
        .register::<AvatarCompatibility>()
        .register::<BehaviorChannel>()
        .register::<MotionSource>()
        .register::<MotionAnalysisStatus>()
        .register::<MotionFamily>()
        .register::<MotionLoopMode>()
        .register::<MotionRole>()
        .register::<MotionSlot>()
        .register::<MotionInterruptPolicy>()
        .register::<MotionRootMode>()
        .register::<InteractionRegion>()
        .register::<InteractionMotionBinding>()
        .register::<InteractionMotionPreviewRequest>()
        .register::<MotionTransitionWindow>()
        .register::<MotionInertialHalfLives>()
        .register::<MotionTransitionProfile>()
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
        .register::<MotionLocomotionIntent>()
        .register::<MotionIntentRequest>()
        .register::<MotionFeatureCacheReadRequest>()
        .register::<MotionFeatureCacheWriteRequest>()
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
        .register::<RuntimeFeatureSet>()
        .register::<BootstrapState>()
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
