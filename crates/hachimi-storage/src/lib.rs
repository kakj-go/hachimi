//! Versioned local settings with atomic replacement and corrupt-file recovery.

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use atomic_write_file::AtomicWriteFile;
use hachimi_protocol::{
    AppSettings, AppearanceConfig, LlmSettings, Locale, SETTINGS_SCHEMA_VERSION, ThemeMode,
    VoiceSettings, WindowPlacementV1,
};
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("settings serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported settings schema version: {0}")]
    UnsupportedSchema(u32),
    #[error("invalid settings: {0}")]
    InvalidSettings(String),
}

#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppSettingsV1 {
    schema_version: u32,
    theme: ThemeMode,
    locale: Locale,
    always_on_top: bool,
    pet_placement: Option<WindowPlacementV1>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppSettingsV2 {
    schema_version: u32,
    theme: ThemeMode,
    locale: Locale,
    always_on_top: bool,
    pet_placement: Option<WindowPlacementV1>,
    llm: LlmSettings,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppSettingsV3 {
    schema_version: u32,
    theme: ThemeMode,
    locale: Locale,
    always_on_top: bool,
    pet_placement: Option<WindowPlacementV1>,
    llm: LlmSettings,
    voice: VoiceSettingsV3,
}

#[derive(Debug, Deserialize)]
struct VoiceSettingsV3 {
    muted: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppSettingsV4 {
    schema_version: u32,
    theme: ThemeMode,
    locale: Locale,
    always_on_top: bool,
    pet_placement: Option<WindowPlacementV1>,
    llm: LlmSettings,
    voice: VoiceSettings,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppSettingsV5 {
    schema_version: u32,
    theme: ThemeMode,
    locale: Locale,
    always_on_top: bool,
    pet_placement: Option<WindowPlacementV1>,
    llm: LlmSettings,
    voice: VoiceSettingsV5,
    appearance: AppearanceConfig,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VoiceSettingsV5 {
    muted: bool,
    speed_percent: u16,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppSettingsV6 {
    schema_version: u32,
    theme: ThemeMode,
    locale: Locale,
    always_on_top: bool,
    pet_placement: Option<WindowPlacementV1>,
    llm: LlmSettings,
    #[allow(dead_code)]
    voice: serde_json::Value,
    appearance: AppearanceConfig,
}

impl SettingsStore {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<AppSettings, StorageError> {
        if !self.path.exists() {
            return Ok(AppSettings::default());
        }

        let bytes = fs::read(&self.path)?;
        let value: serde_json::Value = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(_) => return self.recover_corrupt(&bytes),
        };
        let Some(schema_version) = value
            .get("schemaVersion")
            .and_then(serde_json::Value::as_u64)
        else {
            return self.recover_corrupt(&bytes);
        };
        match u32::try_from(schema_version).unwrap_or(u32::MAX) {
            SETTINGS_SCHEMA_VERSION => match serde_json::from_value::<AppSettings>(value) {
                Ok(mut settings) if settings.appearance.validate().is_ok() => {
                    if settings.appearance.merge_missing_builtin_profiles() {
                        self.save(&settings)?;
                    }
                    Ok(settings)
                }
                Err(_) => self.recover_corrupt(&bytes),
                Ok(_) => self.recover_corrupt(&bytes),
            },
            1 => {
                let old: AppSettingsV1 = match serde_json::from_value::<AppSettingsV1>(value) {
                    Ok(settings) if settings.schema_version == 1 => settings,
                    _ => return self.recover_corrupt(&bytes),
                };
                let migrated = AppSettings {
                    schema_version: SETTINGS_SCHEMA_VERSION,
                    theme: old.theme,
                    locale: old.locale,
                    always_on_top: old.always_on_top,
                    pet_placement: old.pet_placement,
                    llm: LlmSettings::default(),
                    voice: VoiceSettings::default(),
                    appearance: AppearanceConfig::default(),
                    developer_mode: false,
                };
                self.save(&migrated)?;
                Ok(migrated)
            }
            2 => {
                let old: AppSettingsV2 = match serde_json::from_value::<AppSettingsV2>(value) {
                    Ok(settings) if settings.schema_version == 2 => settings,
                    _ => return self.recover_corrupt(&bytes),
                };
                let migrated = AppSettings {
                    schema_version: SETTINGS_SCHEMA_VERSION,
                    theme: old.theme,
                    locale: old.locale,
                    always_on_top: old.always_on_top,
                    pet_placement: old.pet_placement,
                    llm: old.llm,
                    voice: VoiceSettings::default(),
                    appearance: AppearanceConfig::default(),
                    developer_mode: false,
                };
                self.save(&migrated)?;
                Ok(migrated)
            }
            3 => {
                let old: AppSettingsV3 = match serde_json::from_value::<AppSettingsV3>(value) {
                    Ok(settings) if settings.schema_version == 3 => settings,
                    _ => return self.recover_corrupt(&bytes),
                };
                let migrated = AppSettings {
                    schema_version: SETTINGS_SCHEMA_VERSION,
                    theme: old.theme,
                    locale: old.locale,
                    always_on_top: old.always_on_top,
                    pet_placement: old.pet_placement,
                    llm: old.llm,
                    voice: VoiceSettings {
                        muted: old.voice.muted,
                        speed_percent: VoiceSettings::default().speed_percent,
                        ..VoiceSettings::default()
                    },
                    appearance: AppearanceConfig::default(),
                    developer_mode: false,
                };
                self.save(&migrated)?;
                Ok(migrated)
            }
            4 => {
                let old: AppSettingsV4 = match serde_json::from_value::<AppSettingsV4>(value) {
                    Ok(settings) if settings.schema_version == 4 => settings,
                    _ => return self.recover_corrupt(&bytes),
                };
                let migrated = AppSettings {
                    schema_version: SETTINGS_SCHEMA_VERSION,
                    theme: old.theme,
                    locale: old.locale,
                    always_on_top: old.always_on_top,
                    pet_placement: old.pet_placement,
                    llm: old.llm,
                    voice: old.voice,
                    appearance: AppearanceConfig::default(),
                    developer_mode: false,
                };
                self.save(&migrated)?;
                Ok(migrated)
            }
            5 => {
                let old: AppSettingsV5 = match serde_json::from_value::<AppSettingsV5>(value) {
                    Ok(settings) if settings.schema_version == 5 => settings,
                    _ => return self.recover_corrupt(&bytes),
                };
                let migrated = AppSettings {
                    schema_version: SETTINGS_SCHEMA_VERSION,
                    theme: old.theme,
                    locale: old.locale,
                    always_on_top: old.always_on_top,
                    pet_placement: old.pet_placement,
                    llm: old.llm,
                    voice: VoiceSettings {
                        muted: old.voice.muted,
                        speed_percent: old.voice.speed_percent.clamp(50, 200),
                        ..VoiceSettings::default()
                    },
                    appearance: old.appearance,
                    developer_mode: false,
                };
                self.save(&migrated)?;
                Ok(migrated)
            }
            6 => {
                let old: AppSettingsV6 = match serde_json::from_value::<AppSettingsV6>(value) {
                    Ok(settings) if settings.schema_version == 6 => settings,
                    _ => return self.recover_corrupt(&bytes),
                };
                let migrated = AppSettings {
                    schema_version: SETTINGS_SCHEMA_VERSION,
                    theme: old.theme,
                    locale: old.locale,
                    always_on_top: old.always_on_top,
                    pet_placement: old.pet_placement,
                    llm: old.llm,
                    voice: VoiceSettings::default(),
                    appearance: old.appearance,
                    developer_mode: false,
                };
                self.save(&migrated)?;
                Ok(migrated)
            }
            version => Err(StorageError::UnsupportedSchema(version)),
        }
    }

    pub fn save(&self, settings: &AppSettings) -> Result<(), StorageError> {
        if settings.schema_version != SETTINGS_SCHEMA_VERSION {
            return Err(StorageError::UnsupportedSchema(settings.schema_version));
        }
        settings
            .appearance
            .validate()
            .map_err(StorageError::InvalidSettings)?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let bytes = serde_json::to_vec_pretty(settings)?;
        let mut file = AtomicWriteFile::open(&self.path)?;
        file.write_all(&bytes)?;
        file.flush()?;
        file.commit()?;
        Ok(())
    }

    fn back_up_corrupt_file(&self, bytes: &[u8]) -> Result<(), StorageError> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let file_name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("settings.json");
        let backup = self
            .path
            .with_file_name(format!("{file_name}.corrupt-{timestamp}.bak"));
        fs::write(backup, bytes)?;
        Ok(())
    }

    fn recover_corrupt(&self, bytes: &[u8]) -> Result<AppSettings, StorageError> {
        self.back_up_corrupt_file(bytes)?;
        Ok(AppSettings::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_settings() {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = SettingsStore::new(directory.path().join("settings.json"));
        let settings = AppSettings {
            always_on_top: false,
            ..AppSettings::default()
        };
        store.save(&settings).expect("save");
        assert_eq!(store.load().expect("load"), settings);
    }

    #[test]
    fn enriches_existing_schema_five_settings_with_new_builtin_themes() {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = SettingsStore::new(directory.path().join("settings.json"));
        let mut settings = AppSettings::default();
        settings
            .appearance
            .themes
            .retain(|profile| profile.id.starts_with("codex-"));
        settings.appearance.themes[0].accent = "#123456".into();
        store
            .save(&settings)
            .expect("save old schema five appearance");

        let loaded = store.load().expect("load enriched settings");
        assert_eq!(loaded.appearance.themes.len(), 18);
        assert_eq!(
            loaded.appearance.profile("codex-light").unwrap().accent,
            "#123456"
        );
        assert!(loaded.appearance.profile("everforest-dark").is_some());
    }

    #[test]
    fn corrupt_settings_are_backed_up_and_defaulted() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("settings.json");
        fs::write(&path, b"not-json").expect("seed corrupt settings");
        let store = SettingsStore::new(path);
        assert_eq!(store.load().expect("fallback"), AppSettings::default());
        assert!(
            fs::read_dir(directory.path())
                .expect("list")
                .flatten()
                .any(|entry| entry.file_name().to_string_lossy().contains("corrupt"))
        );
    }

    #[test]
    fn unknown_schema_is_not_overwritten() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("settings.json");
        let store = SettingsStore::new(path);
        let settings = AppSettings {
            schema_version: 99,
            ..AppSettings::default()
        };
        assert!(matches!(
            store.save(&settings),
            Err(StorageError::UnsupportedSchema(99))
        ));
    }

    #[test]
    fn migrates_v1_and_preserves_existing_values() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("settings.json");
        fs::write(
            &path,
            br#"{
                "schemaVersion": 1,
                "theme": "dark",
                "locale": "en-US",
                "alwaysOnTop": false,
                "petPlacement": null
            }"#,
        )
        .expect("seed v1");
        let settings = SettingsStore::new(&path).load().expect("migrate");
        assert_eq!(settings.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(settings.theme, ThemeMode::Dark);
        assert_eq!(settings.locale, Locale::EnUs);
        assert!(!settings.always_on_top);
        assert_eq!(settings.llm, LlmSettings::default());

        let rewritten: serde_json::Value =
            serde_json::from_slice(&fs::read(path).expect("read migrated")).expect("json");
        assert_eq!(rewritten["schemaVersion"], SETTINGS_SCHEMA_VERSION);
    }

    #[test]
    fn migrates_v2_with_voice_unmuted() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("settings.json");
        fs::write(
            &path,
            br#"{
                "schemaVersion": 2,
                "theme": "system",
                "locale": "zh-CN",
                "alwaysOnTop": true,
                "petPlacement": null,
                "llm": {
                    "baseUrl": "http://localhost:11434/v1",
                    "modelName": "demo",
                    "maxInputTokens": 0,
                    "maxOutputTokens": 0
                }
            }"#,
        )
        .expect("seed v2");
        let settings = SettingsStore::new(&path).load().expect("migrate");
        assert_eq!(settings.schema_version, SETTINGS_SCHEMA_VERSION);
        assert!(!settings.voice.muted);
        assert_eq!(settings.voice.speed_percent, 100);
        assert_eq!(settings.llm.model_name, "demo");
    }

    #[test]
    fn migrates_v3_and_preserves_mute_state() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("settings.json");
        fs::write(
            &path,
            br#"{
                "schemaVersion": 3,
                "theme": "dark",
                "locale": "zh-CN",
                "alwaysOnTop": true,
                "petPlacement": null,
                "llm": {
                    "baseUrl": "http://localhost:11434/v1",
                    "modelName": "demo",
                    "maxInputTokens": 0,
                    "maxOutputTokens": 0
                },
                "voice": { "muted": true }
            }"#,
        )
        .expect("seed v3");
        let settings = SettingsStore::new(&path).load().expect("migrate");
        assert_eq!(settings.schema_version, SETTINGS_SCHEMA_VERSION);
        assert!(settings.voice.muted);
        assert_eq!(settings.voice.speed_percent, 100);
    }

    #[test]
    fn migrates_v4_and_adds_default_appearance() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("settings.json");
        fs::write(
            &path,
            br#"{
                "schemaVersion": 4,
                "theme": "light",
                "locale": "en-US",
                "alwaysOnTop": false,
                "petPlacement": null,
                "llm": {
                    "baseUrl": "http://localhost:11434/v1",
                    "modelName": "demo-v4",
                    "maxInputTokens": 0,
                    "maxOutputTokens": 0
                },
                "voice": { "muted": false, "voiceId": 47, "speedPercent": 105 }
            }"#,
        )
        .expect("seed v4");
        let settings = SettingsStore::new(&path).load().expect("migrate");
        assert_eq!(settings.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(settings.theme, ThemeMode::Light);
        assert_eq!(settings.llm.model_name, "demo-v4");
        assert_eq!(settings.voice.speed_percent, 105);
        assert_eq!(settings.appearance, AppearanceConfig::default());

        let rewritten: serde_json::Value =
            serde_json::from_slice(&fs::read(path).expect("read migrated")).expect("json");
        assert!(rewritten.get("appearance").is_some());
    }

    #[test]
    fn migrates_v5_and_drops_kokoro_voice_id() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("settings.json");
        let appearance = serde_json::to_value(AppearanceConfig::default()).expect("appearance");
        let old = serde_json::json!({
            "schemaVersion": 5,
            "theme": "dark",
            "locale": "zh-CN",
            "alwaysOnTop": true,
            "petPlacement": null,
            "llm": LlmSettings::default(),
            "voice": { "muted": true, "voiceId": 47, "speedPercent": 115 },
            "appearance": appearance,
        });
        fs::write(&path, serde_json::to_vec_pretty(&old).expect("json")).expect("seed v5");
        let settings = SettingsStore::new(&path).load().expect("migrate");
        assert_eq!(settings.schema_version, SETTINGS_SCHEMA_VERSION);
        assert!(settings.voice.muted);
        assert_eq!(settings.voice.speed_percent, 115);
        let rewritten: serde_json::Value =
            serde_json::from_slice(&fs::read(path).expect("read migrated")).expect("json");
        assert!(rewritten["voice"].get("voiceId").is_none());
        assert_eq!(rewritten["schemaVersion"], SETTINGS_SCHEMA_VERSION);
    }

    #[test]
    fn migrates_v6_and_resets_all_legacy_voice_fields() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("settings.json");
        let old = serde_json::json!({
            "schemaVersion": 6,
            "theme": "light",
            "locale": "en-US",
            "alwaysOnTop": false,
            "petPlacement": null,
            "llm": {
                "baseUrl": "https://example.test/v1",
                "modelName": "preserved-model",
                "maxInputTokens": 1234,
                "maxOutputTokens": 321
            },
            "voice": {
                "muted": true,
                "speedPercent": 175,
                "endpoint": "http://127.0.0.1:9880",
                "referenceAudio": "reference.wav"
            },
            "appearance": AppearanceConfig::default(),
            "legacyAvatarBehavior": { "ignored": true }
        });
        fs::write(&path, serde_json::to_vec_pretty(&old).expect("json")).expect("seed v6");
        let settings = SettingsStore::new(&path).load().expect("migrate");
        assert_eq!(settings.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(settings.theme, ThemeMode::Light);
        assert_eq!(settings.locale, Locale::EnUs);
        assert_eq!(settings.llm.model_name, "preserved-model");
        assert_eq!(settings.voice, VoiceSettings::default());
        assert_eq!(settings.appearance, AppearanceConfig::default());
    }

    #[test]
    fn refuses_to_save_invalid_appearance() {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = SettingsStore::new(directory.path().join("settings.json"));
        let mut settings = AppSettings::default();
        settings.appearance.themes[0].foreground = "not-a-color".into();
        assert!(matches!(
            store.save(&settings),
            Err(StorageError::InvalidSettings(_))
        ));
        assert!(!store.path().exists());
    }
}
