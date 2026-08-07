use std::{collections::BTreeMap, env, fs, path::Path};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use hachimi_protocol::{ComputerAppCandidate, PermissionCommandCandidate};
use tauri::{State, WebviewWindow};
use tokio::time::{Duration, sleep};

use crate::{CommandError, DesktopState, require_window};

#[tauri::command]
pub(super) async fn choose_permission_directory(
    window: WebviewWindow,
) -> Result<Option<String>, CommandError> {
    require_window(&window, "workbench")?;
    Ok(rfd::AsyncFileDialog::new()
        .set_title("选择授权目录")
        .pick_folder()
        .await
        .map(|folder| folder.path().to_string_lossy().into_owned()))
}

#[tauri::command]
pub(super) async fn choose_permission_files(
    window: WebviewWindow,
    root: String,
) -> Result<Vec<String>, CommandError> {
    require_window(&window, "workbench")?;
    let root = fs::canonicalize(&root)
        .map_err(|error| CommandError::operation("permission_root_invalid", error))?;
    let Some(files) = rfd::AsyncFileDialog::new()
        .set_title("选择授权文件")
        .pick_files()
        .await
    else {
        return Ok(Vec::new());
    };
    let mut selected = Vec::new();
    for file in files {
        let path = fs::canonicalize(file.path())
            .map_err(|error| CommandError::operation("permission_file_invalid", error))?;
        let relative = path.strip_prefix(&root).map_err(|_| {
            CommandError::new(
                "permission_file_outside_root",
                "Selected files must be inside the authorized directory.",
            )
        })?;
        if !path.is_file() {
            return Err(CommandError::new(
                "permission_file_not_regular",
                "Only regular files can be selected.",
            ));
        }
        selected.push(relative.to_string_lossy().replace('\\', "/"));
    }
    selected.sort_by_key(|value| value.to_ascii_lowercase());
    selected.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    Ok(selected)
}

#[tauri::command]
pub(super) fn search_permission_commands(
    window: WebviewWindow,
    prefix: String,
) -> Result<Vec<PermissionCommandCandidate>, CommandError> {
    require_window(&window, "workbench")?;
    let prefix = prefix.trim().to_ascii_lowercase();
    if prefix.is_empty() {
        return Ok(Vec::new());
    }
    let mut candidates = BTreeMap::<String, PermissionCommandCandidate>::new();
    let mut roots = env::var_os("PATH")
        .map(|value| env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();
    if let Ok(executable) = env::current_exe()
        && let Some(root) = executable.parent()
    {
        roots.push(root.join("managed-git").join("cmd"));
    }
    for root in roots {
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || !is_supported_command(&path) {
                continue;
            }
            let Some(name) = path
                .file_stem()
                .and_then(|value| value.to_str())
                .map(str::to_owned)
            else {
                continue;
            };
            if is_internal_sidecar(&name) {
                continue;
            }
            if !name.to_ascii_lowercase().starts_with(&prefix) {
                continue;
            }
            let canonical = fs::canonicalize(&path).unwrap_or(path);
            let key = canonical.to_string_lossy().to_ascii_lowercase();
            candidates
                .entry(key)
                .or_insert_with(|| PermissionCommandCandidate {
                    name,
                    executable_path: canonical.to_string_lossy().into_owned(),
                    source: if root.ends_with(Path::new("managed-git").join("cmd")) {
                        "Hachimi bundled".into()
                    } else {
                        "PATH".into()
                    },
                });
        }
    }
    let mut values = candidates.into_values().collect::<Vec<_>>();
    values.sort_by_key(|candidate| {
        (
            candidate.name.to_ascii_lowercase(),
            candidate.executable_path.clone(),
        )
    });
    values.truncate(50);
    Ok(values)
}

#[tauri::command]
pub(super) async fn choose_permission_foreground_application(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Option<ComputerAppCandidate>, CommandError> {
    require_window(&window, "workbench")?;
    sleep(Duration::from_secs(3)).await;
    let target = state
        .computer_host
        .foreground_window()
        .await
        .map_err(|error| CommandError::operation("computer_foreground_window_failed", error))?;
    let icon_png_base64 = state
        .computer_host
        .app_icon_png(&target.app)
        .await
        .ok()
        .flatten()
        .map(|bytes| STANDARD.encode(bytes));
    Ok(Some(ComputerAppCandidate {
        app: target.app,
        window_count: 1,
        icon_png_base64,
    }))
}

fn is_supported_command(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "exe" | "com" | "bat" | "cmd"
            )
        })
}

fn is_internal_sidecar(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("hachimi-sandbox-")
        || name == "hachimi-cef-host"
        || name == "hachimi-gateway"
        || name.starts_with("hachimi-channel-sidecar-")
        || name.starts_with("hachimi-connector-sidecar-")
        || name.starts_with("hachimi-hook-sidecar-")
}

#[cfg(test)]
mod tests {
    use super::is_internal_sidecar;

    #[test]
    fn internal_service_sidecars_are_not_user_command_candidates() {
        assert!(is_internal_sidecar("hachimi-sandbox-launcher"));
        assert!(is_internal_sidecar("Hachimi-CEF-Host"));
        assert!(is_internal_sidecar("hachimi-gateway"));
        assert!(is_internal_sidecar("hachimi-channel-sidecar-slack"));
        assert!(!is_internal_sidecar("git"));
        assert!(!is_internal_sidecar("hachimi-workspace-worker"));
    }
}
