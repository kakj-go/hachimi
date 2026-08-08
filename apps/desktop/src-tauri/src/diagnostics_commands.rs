use super::*;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DiagnosticsPaths {
    data_directory: String,
    log_directory: String,
    backend_log: String,
    frontend_log: String,
}

#[tauri::command]
pub(super) fn get_diagnostics_paths(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<DiagnosticsPaths, CommandError> {
    state.authorize(&window, ControlMethod::SettingsRead)?;
    let log_directory = &state.log_directory;
    Ok(DiagnosticsPaths {
        data_directory: state.storage_layout.root.display().to_string(),
        backend_log: log_directory
            .join("hachimi-backend.log")
            .display()
            .to_string(),
        frontend_log: log_directory
            .join("hachimi-frontend.log")
            .display()
            .to_string(),
        log_directory: log_directory.display().to_string(),
    })
}

#[tauri::command]
pub(super) fn open_logs_directory(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<(), CommandError> {
    state.authorize(&window, ControlMethod::SettingsRead)?;
    let directory = &state.log_directory;
    std::fs::create_dir_all(directory)
        .map_err(|error| CommandError::operation("log_directory_create_failed", error))?;
    open_directory(directory)
        .map_err(|error| CommandError::operation("log_directory_open_failed", error))
}

#[cfg(target_os = "windows")]
fn open_directory(directory: &Path) -> std::io::Result<()> {
    std::process::Command::new("explorer.exe")
        .arg(directory)
        .spawn()
        .map(|_| ())
}

#[cfg(target_os = "macos")]
fn open_directory(directory: &Path) -> std::io::Result<()> {
    std::process::Command::new("open")
        .arg(directory)
        .spawn()
        .map(|_| ())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_directory(directory: &Path) -> std::io::Result<()> {
    std::process::Command::new("xdg-open")
        .arg(directory)
        .spawn()
        .map(|_| ())
}
