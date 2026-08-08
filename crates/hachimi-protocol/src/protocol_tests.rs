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
fn permission_update_round_trips_revision_and_resource_scopes() {
    let value = serde_json::json!({
        "sessionId": null,
        "entryProfile": "pet_conversation",
        "expectedRevision": 7,
        "config": {
            "skillIds": ["office-documents", "office-pdf"],
            "policy": {
                "level": "writable",
                "rules": {
                    "fileSystem": [{
                        "access": "read",
                        "roots": ["C:\\workspace"],
                        "globs": ["src/**/*.rs"],
                        "files": ["Cargo.toml"],
                        "specialRoots": []
                    }],
                    "fileSystemUnrestrictedRead": false,
                    "fileSystemUnrestrictedWrite": false,
                    "network": { "enabled": true, "unrestrictedHosts": false, "hosts": ["example.com"], "protocols": ["https"] },
                    "process": { "spawn": true, "interactive": false, "unrestrictedCommands": false, "allowedCommands": [] },
                    "browser": { "observe": true, "act": false, "upload": false, "download": false, "cookieStorage": false, "cdp": false, "unrestrictedOrigins": false, "origins": ["https://example.com"] },
                    "computer": { "observe": true, "act": false, "unrestrictedTargets": false, "allowedApplications": ["sha256:app"], "maxActions": null },
                    "mcp": [],
                    "connectors": []
                },
                "revision": 7
            },
            "extraAuthorizations": []
        }
    });
    let update: SessionPermissionConfigUpdate =
        serde_json::from_value(value).expect("deserialize permission update");
    assert_eq!(update.expected_revision, 7);
    assert_eq!(
        update.config.skill_ids,
        [
            SkillId::from("office-documents"),
            SkillId::from("office-pdf")
        ]
    );
    assert_eq!(
        update.config.policy.rules.file_system[0].files,
        ["Cargo.toml"]
    );
    assert_eq!(
        update.config.policy.rules.computer.allowed_applications,
        ["sha256:app"]
    );
    let encoded = serde_json::to_value(update).expect("serialize permission update");
    assert_eq!(encoded["expectedRevision"], 7);
    assert_eq!(encoded["config"]["skillIds"][0], "office-documents");
    assert_eq!(
        encoded["config"]["policy"]["rules"]["computer"]["allowedApplications"][0],
        "sha256:app"
    );
}

#[test]
fn default_settings_are_versioned() {
    let settings = AppSettings::default();
    assert_eq!(settings.schema_version, SETTINGS_SCHEMA_VERSION);
    assert_eq!(settings.llm.model_name, "gpt-5.6-sol");
    assert_eq!(settings.llm.max_input_tokens, 1_050_000);
    assert_eq!(settings.llm.max_output_tokens, 128_000);
}

#[test]
fn default_appearance_is_valid_and_has_both_schemes() {
    let appearance = AppearanceConfig::default();
    assert!(appearance.validate().is_ok());
    assert_eq!(appearance.preferences.density, UiDensity::Default);
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
fn legacy_appearance_without_density_uses_default_density() {
    let mut value = serde_json::to_value(AppearanceConfig::default()).unwrap();
    value["preferences"]
        .as_object_mut()
        .unwrap()
        .remove("density");
    let appearance: AppearanceConfig = serde_json::from_value(value).unwrap();
    assert_eq!(appearance.preferences.density, UiDensity::Default);
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
fn legacy_codex_defaults_upgrade_to_quiet_graphite_without_overwriting_custom_colors() {
    let mut appearance = AppearanceConfig::default();
    let light = appearance.profile("codex-light").unwrap().clone();
    let dark = appearance.profile("codex-dark").unwrap().clone();
    appearance.themes[0] = ThemeProfile {
        name: "Codex Light".into(),
        accent: "#1677D2".into(),
        background: "#F5F4F7".into(),
        foreground: "#202126".into(),
        ..light
    };
    appearance.themes[1] = ThemeProfile {
        name: "Custom dark".into(),
        accent: "#123456".into(),
        ..dark
    };
    assert!(appearance.merge_missing_builtin_profiles());
    assert_eq!(
        appearance.profile("codex-light").unwrap().name,
        "Quiet Graphite Light"
    );
    assert_eq!(appearance.profile("codex-dark").unwrap().accent, "#123456");
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
