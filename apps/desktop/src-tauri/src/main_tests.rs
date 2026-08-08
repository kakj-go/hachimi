use super::{
    PendingAvatarImport, PendingVoiceImport,
    app_shell::{SingleInstanceActivationTarget, single_instance_activation_target},
    avatar_source_is_unchanged, cancel_pending_avatar_import, cancel_pending_voice_import,
    consume_pending_avatar_import, consume_pending_voice_import, debug_data_root,
    delete_theme_in_settings, profile_supports_pet_voice, provider_settings_for_runtime,
    release_agent_feature_flags, release_feature_enabled, release_runtime_feature_set,
    reset_theme_in_settings, sanitize_log_message, validate_app_settings,
    voice_source_is_unchanged,
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

#[test]
fn secondary_launch_prefers_workbench_then_pet_while_startup_remains_safe() {
    assert_eq!(
        single_instance_activation_target(true, true),
        SingleInstanceActivationTarget::Workbench
    );
    assert_eq!(
        single_instance_activation_target(false, true),
        SingleInstanceActivationTarget::Pet
    );
    assert_eq!(
        single_instance_activation_target(false, false),
        SingleInstanceActivationTarget::StartupPending
    );
}

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

#[test]
fn release_features_are_enabled_by_default_and_have_an_emergency_kill_switch() {
    assert!(release_feature_enabled(false));
    assert!(!release_feature_enabled(true));
    let defaults = release_agent_feature_flags(false, false, false);
    assert!(defaults.workspace_tools);
    assert!(defaults.mcp_runtime);
    assert!(defaults.scheduler);
    assert_eq!(
        defaults.runtime_features,
        hachimi_core::RuntimeFeatureSet::all_enabled()
    );
    let disabled = release_agent_feature_flags(true, true, true);
    assert!(!disabled.workspace_tools);
    assert!(!disabled.mcp_runtime);
    assert!(!disabled.scheduler);

    let runtime = release_runtime_feature_set(hachimi_core::RuntimeFeatureSet {
        multi_agent: true,
        ..hachimi_core::RuntimeFeatureSet::all_disabled()
    });
    assert!(!runtime.multi_agent);
    assert!(runtime.run_recovery);
    assert_eq!(
        release_runtime_feature_set(hachimi_core::RuntimeFeatureSet::all_disabled()),
        hachimi_core::RuntimeFeatureSet::all_enabled()
    );
    assert_eq!(
        release_runtime_feature_set(hachimi_core::RuntimeFeatureSet::all_enabled()),
        hachimi_core::RuntimeFeatureSet::all_disabled()
    );
}

#[test]
fn provider_switches_force_legacy_chat_and_strip_remote_context() {
    let mut settings = hachimi_protocol::AppSettings::default().llm;
    settings.protocol = hachimi_protocol::ProviderProtocolKind::Responses;
    settings.embedding_model_name = "embedding-fixture".into();
    settings.reasoning_summary = hachimi_protocol::ReasoningSummaryMode::Detailed;
    settings.remote_compaction = true;
    let mut features = hachimi_core::RuntimeFeatureSet::all_enabled();
    features.provider_extensions = false;
    let legacy = provider_settings_for_runtime(settings.clone(), features);
    assert_eq!(
        legacy.protocol,
        hachimi_protocol::ProviderProtocolKind::ChatCompletions
    );
    assert!(legacy.embedding_model_name.is_empty());
    assert_eq!(
        legacy.reasoning_summary,
        hachimi_protocol::ReasoningSummaryMode::None
    );
    assert!(!legacy.remote_compaction);

    features.provider_extensions = true;
    features.provider_remote_context = false;
    let responses = provider_settings_for_runtime(settings, features);
    assert_eq!(
        responses.protocol,
        hachimi_protocol::ProviderProtocolKind::Responses
    );
    assert_eq!(
        responses.reasoning_summary,
        hachimi_protocol::ReasoningSummaryMode::None
    );
    assert!(!responses.remote_compaction);
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
        "#7062D5"
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
        consume_pending_avatar_import(&mut imports, "expired", &ClientId("workbench".into()), now)
            .is_none()
    );
    assert!(
        consume_pending_avatar_import(&mut imports, "valid", &ClientId("workbench".into()), now)
            .is_some()
    );
    assert!(
        consume_pending_avatar_import(&mut imports, "valid", &ClientId("workbench".into()), now)
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
