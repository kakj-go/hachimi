use super::*;

pub(super) fn avatar_source_is_unchanged(
    previous: &InspectedAvatar,
    current: &InspectedAvatar,
) -> bool {
    current.sha256 == previous.sha256
        && current.size_bytes == previous.size_bytes
        && current.modified_millis == previous.modified_millis
        && current.is_compatible()
}

pub(super) fn voice_source_is_unchanged(
    previous: &InspectedVoiceModel,
    current: &InspectedVoiceModel,
) -> bool {
    current.sha256 == previous.sha256
        && current.size_bytes == previous.size_bytes
        && current.modified_millis == previous.modified_millis
        && current.compatible
}

#[tauri::command]
pub(super) fn get_llm_settings(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<LlmSettingsView, CommandError> {
    state.authorize(&window, ControlMethod::LlmRead)?;
    state.llm_view()
}

pub(super) fn save_llm(
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
pub(super) fn save_llm_settings(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    input: LlmSettingsInput,
) -> Result<LlmSettingsView, CommandError> {
    state.authorize(&window, ControlMethod::LlmWrite)?;
    save_llm(&app, &state, &input)
}

#[tauri::command]
pub(super) async fn save_and_test_llm_settings(
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
pub(super) fn list_avatar_models(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<AvatarCatalogSnapshot, CommandError> {
    state.authorize(&window, ControlMethod::AvatarRead)?;
    Ok(state.avatar_catalog.read().snapshot())
}

#[tauri::command]
pub(super) fn inspect_avatar_model(
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
pub(super) fn commit_avatar_model_import(
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
pub(super) fn cancel_avatar_model_import(
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
pub(super) fn select_avatar_model(
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
pub(super) fn delete_avatar_model(
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
pub(super) fn get_current_avatar_asset(
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
pub(super) fn get_avatar_runtime_asset(
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
pub(super) fn list_motion_catalog(
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
pub(super) fn inspect_motion_file(
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
pub(super) fn commit_motion_import(
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
pub(super) fn cancel_motion_import(
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
pub(super) fn update_motion_metadata(
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
pub(super) fn delete_user_motion(
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
pub(super) fn set_interaction_motion_binding(
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
pub(super) fn clear_motion_interaction_bindings(
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
pub(super) fn set_motion_enabled(
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
pub(super) fn reset_motion_bindings(
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
pub(super) fn reset_motion_binding(
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
pub(super) fn get_motion_runtime_asset(
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

pub(super) fn profile_supports_pet_voice(profile: &AvatarAdaptationProfile) -> bool {
    !matches!(profile.lip_sync, LipSyncCapability::None)
}

#[tauri::command]
pub(super) fn start_pet_turn(
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
pub(super) fn cancel_pet_turn(
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
pub(super) fn list_voice_models(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<VoiceCatalogSnapshot, CommandError> {
    state.authorize(&window, ControlMethod::VoiceRead)?;
    Ok(state.voice_catalog.read().snapshot())
}

#[tauri::command]
pub(super) fn inspect_voice_model(
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
pub(super) fn commit_voice_model_import(
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
pub(super) fn cancel_voice_model_import(
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
pub(super) fn select_voice_model(
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
pub(super) fn delete_voice_model(
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
pub(super) fn get_voice_runtime_state(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<VoiceRuntimeState, CommandError> {
    state.authorize(&window, ControlMethod::VoicePlayback)?;
    Ok(state.voice_runtime.state())
}

#[tauri::command]
pub(super) fn get_speech_recognition_state(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<SpeechRecognitionRuntimeState, CommandError> {
    state.authorize(&window, ControlMethod::VoiceRead)?;
    Ok(state.speech_recognizer.state())
}

#[tauri::command]
pub(super) async fn update_speech_recognition_settings(
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
pub(super) fn update_voice_settings(
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
pub(super) fn set_muted(
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
pub(super) fn preview_default_voice(
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
pub(super) fn stop_speech(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<VoiceRuntimeState, CommandError> {
    state.authorize(&window, ControlMethod::VoicePlayback)?;
    state.voice_runtime.stop();
    Ok(state.voice_runtime.state())
}

#[tauri::command]
pub(super) async fn recognize_pet_speech(
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
