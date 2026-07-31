use super::*;

pub(super) const fn release_feature_enabled(explicitly_disabled: bool) -> bool {
    !explicitly_disabled
}

pub(super) fn env_disabled(name: &str) -> bool {
    matches!(std::env::var(name).as_deref(), Ok("1" | "true" | "TRUE"))
}

pub(super) const fn release_runtime_feature_set(disabled: RuntimeFeatureSet) -> RuntimeFeatureSet {
    RuntimeFeatureSet {
        run_recovery: !disabled.run_recovery,
        provider_extensions: !disabled.provider_extensions,
        provider_remote_context: !disabled.provider_remote_context,
        multi_agent: !disabled.multi_agent,
        git_remote_mutations: !disabled.git_remote_mutations,
        plugin_runtime: !disabled.plugin_runtime,
        enterprise_integrations: !disabled.enterprise_integrations,
        desktop_control: !disabled.desktop_control,
    }
}

pub(super) fn release_agent_feature_flags(
    workspace_disabled: bool,
    mcp_disabled: bool,
    scheduler_disabled: bool,
) -> FeatureFlags {
    FeatureFlags {
        workbench: true,
        workspace_tools: release_feature_enabled(workspace_disabled),
        browser_control: true,
        computer_observe: true,
        computer_control: true,
        plugin_runtime: true,
        local_gateway: true,
        mcp_runtime: release_feature_enabled(mcp_disabled),
        scheduler: release_feature_enabled(scheduler_disabled),
        runtime_features: RuntimeFeatureSet::all_enabled(),
        ..FeatureFlags::all_disabled()
    }
}

pub(super) fn absolute_path(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

#[tauri::command]
pub(super) fn exit_app(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<(), CommandError> {
    state.authorize(&window, ControlMethod::WindowInteract)?;
    require_window(&window, "pet")?;
    exit_application(&app, &state)
}

pub(super) fn exit_application(app: &AppHandle, state: &DesktopState) -> Result<(), CommandError> {
    if let Some(pet) = app.get_webview_window("pet") {
        capture_pet_placement(&pet, state)?;
    }
    state.save_settings()?;
    // Match Codex's process-session shutdown semantics: terminate all owned
    // process trees before handing control to the platform exit path.
    tauri::async_runtime::block_on(state.process_registry.shutdown());
    state.scheduler_handle.lock().take();
    app.exit(0);
    Ok(())
}

#[tauri::command]
pub(super) fn hide_pet_window(
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
pub(super) fn show_pet_context_menu(
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

pub(super) fn require_window(window: &WebviewWindow, expected: &str) -> Result<(), CommandError> {
    if window.label() == expected {
        Ok(())
    } else {
        Err(CommandError::new(
            "permission_denied",
            format!("command is only available to the {expected} window"),
        ))
    }
}

pub(super) fn avatar_asset_url(entry_id: &str) -> String {
    if cfg!(windows) {
        format!("http://hachimi-avatar.localhost/{entry_id}")
    } else {
        format!("hachimi-avatar://localhost/{entry_id}")
    }
}

pub(super) fn motion_asset_url(entry_id: &str) -> String {
    if cfg!(windows) {
        format!("http://hachimi-motion.localhost/{entry_id}")
    } else {
        format!("hachimi-motion://localhost/{entry_id}")
    }
}

pub(super) fn cancel_pet_activity(app: &AppHandle, state: &DesktopState, emit_cancelled: bool) {
    if let Some(active) = state.pet_run.lock().take() {
        active.cancellation.cancel();
        let _ = state
            .agent_executor
            .registry()
            .cancel(&active.agent_run_id, active.run_generation);
        let approval_broker = state.approval_broker.clone();
        let user_input_broker = state.user_input_broker.clone();
        let authority_run_id = active.agent_run_id.clone();
        tauri::async_runtime::spawn(async move {
            let _ = approval_broker.cancel_run(authority_run_id.clone()).await;
            let _ = user_input_broker.cancel_run(authority_run_id).await;
        });
        if emit_cancelled {
            let _ = app.emit_to(
                "pet",
                PET_TURN_EVENT,
                PetTurnEvent::Cancelled {
                    run_id: active.run_id,
                    session_id: active.session_id,
                    agent_run_id: active.agent_run_id,
                },
            );
        }
    }
    state.voice_runtime.stop();
}

pub(super) fn enter_workbench_mode(app: &AppHandle, state: &DesktopState) {
    // Workbench and Pet are two presentations over the same persistent Run.
    // Switching surfaces must not cancel pending Approval/UserInput or revoke
    // the Run; only explicit Stop/cancel owns that transition.
    state.voice_runtime.stop();
    let _ = app.emit_to("pet", "pet:close-composer", ());
    hide_pet(app);
}

pub(super) fn restore_pet<R: Runtime>(app: &AppHandle<R>) {
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

pub(super) fn hide_pet<R: Runtime>(app: &AppHandle<R>) {
    let _ = app.emit_to("pet", PET_VISIBILITY_EVENT, false);
    if let Some(pet) = app.get_webview_window("pet") {
        let _ = pet.hide();
    }
    refresh_tray_menu(app);
}

pub(super) fn show_pet_by_user<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<DesktopState>();
    state.pet_hidden_by_user.store(false, Ordering::SeqCst);
    restore_pet(app);
}

pub(super) fn toggle_pet_visibility<R: Runtime>(app: &AppHandle<R>) {
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

pub(super) fn refresh_tray_menu<R: Runtime>(app: &AppHandle<R>) {
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

pub(super) fn refresh_pet_context_menu<R: Runtime>(app: &AppHandle<R>) {
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

pub(super) fn handle_pet_context_menu_action(
    app: &AppHandle,
    id: &str,
) -> Result<(), CommandError> {
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

pub(super) fn create_pet_context_menu(app: &tauri::App) -> Result<(), tauri::Error> {
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

pub(super) fn create_tray(app: &tauri::App) -> Result<(), tauri::Error> {
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

pub(super) fn resolve_resource<R: Runtime>(app: &AppHandle<R>, relative: &str) -> PathBuf {
    if let Ok(resource_dir) = app.path().resource_dir() {
        let candidate = resource_dir.join(relative);
        if candidate.exists() {
            return candidate;
        }
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

pub(super) fn avatar_protocol_response(
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

pub(super) fn create_workbench_window<R: Runtime>(
    app: &AppHandle<R>,
    route: WorkbenchRoute,
    webview_data_directory: &Path,
) -> Result<(), CommandError> {
    #[cfg(all(debug_assertions, feature = "desktop-e2e"))]
    let _ = webview_data_directory;
    let url = format!("workbench.html?route={}", route.as_str());
    let builder = WebviewWindowBuilder::new(app, "workbench", WebviewUrl::App(url.into()))
        .title("Hachimi Workbench")
        .inner_size(1280.0, 800.0)
        .min_inner_size(960.0, 640.0)
        .resizable(true)
        .decorations(false)
        .transparent(false)
        .shadow(true);
    #[cfg(not(all(debug_assertions, feature = "desktop-e2e")))]
    let builder = builder.data_directory(webview_data_directory.to_path_buf());
    let builder = builder.on_page_load(|window, payload| {
        let registry = Arc::clone(&window.state::<DesktopState>().process_registry);
        let owner = ClientId("window:workbench".into());
        match payload.event() {
            tauri::webview::PageLoadEvent::Started => {
                tauri::async_runtime::spawn(async move {
                    registry.detach_owner(&owner).await;
                });
            }
            tauri::webview::PageLoadEvent::Finished => {
                tauri::async_runtime::spawn(async move {
                    registry.attach_owner(&owner).await;
                });
            }
        }
    });
    let workbench = builder
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
            cancel_all_workspace_transients(&event_app.state::<DesktopState>());
            #[cfg(all(debug_assertions, feature = "desktop-e2e"))]
            {
                // WebDriver ends a session by closing the application WebView.
                // The production hide-to-tray behavior would keep the Pet and
                // native process alive, so the replacement session could start
                // beside an owner of the previous WebView2 profile. Let the
                // debug-only E2E process terminate and exercise a real restart.
                let _ = api;
                event_app.exit(0);
            }
            #[cfg(not(all(debug_assertions, feature = "desktop-e2e")))]
            {
                api.prevent_close();
                let _ = close_target.hide();
                restore_pet(&event_app);
            }
        }
        WindowEvent::Focused(true) => {
            hide_pet(&event_app);
        }
        WindowEvent::Resized(_) => {
            if close_target.is_minimized().unwrap_or(false) {
                restore_pet(&event_app);
            }
        }
        WindowEvent::Destroyed => {
            cancel_all_workspace_transients(&event_app.state::<DesktopState>());
            restore_pet(&event_app);
        }
        _ => {}
    });
    workbench.show()?;
    workbench.set_focus()?;
    Ok(())
}

pub(super) fn capture_pet_placement<R: Runtime>(
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

pub(super) fn monitor_geometries<R: Runtime>(window: &WebviewWindow<R>) -> Vec<MonitorGeometry> {
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

pub(super) fn restore_pet_placement<R: Runtime>(
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

pub(super) fn start_click_through_loop(app: AppHandle) {
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

pub(super) fn persist_pet_placement_after_move(app: AppHandle, revision: u64) {
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

pub(super) fn open_append_log(path: PathBuf) -> std::io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

pub(super) fn epoch_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub(super) fn redact_prefixed_token(mut value: String, prefix: &str) -> String {
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

pub(super) fn sanitize_log_message(message: &str) -> String {
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

pub(super) fn initialize_logging(preferred: PathBuf) -> PathBuf {
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
