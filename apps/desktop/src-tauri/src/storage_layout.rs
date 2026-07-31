use crate::app_shell::absolute_path;
use hachimi_llm::{ApiKeyStore, SystemApiKeyStore};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub(crate) const APP_IDENTIFIER: &str = "com.hachimi.desktop";
const PORTABLE_MARKER_FILE: &str = "hachimi.portable";
const RESET_MARKER_FILE: &str = "hachimi-reset-all-v1.marker";
pub(crate) const DATA_ROOT_SENTINEL_FILE: &str = ".hachimi-data-root";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StorageMode {
    Debug,
    Portable,
    Installed,
    Override,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StorageLayout {
    pub(crate) root: PathBuf,
    pub(crate) webview: PathBuf,
    pub(crate) mode: StorageMode,
    redirect_webview: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ResetMarker {
    version: u32,
    root: PathBuf,
    webview: PathBuf,
}

impl StorageLayout {
    pub(crate) fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub(crate) fn sandbox_setup_marker(&self) -> PathBuf {
        self.root.join("sandbox/windows/setup.json")
    }
}

pub(crate) fn debug_data_root(executable: &Path) -> Option<PathBuf> {
    let executable_dir = executable.parent()?;
    if executable_dir
        .file_name()
        .is_some_and(|name| name == "debug")
    {
        return executable_dir
            .parent()
            .map(|target| target.join("hachimi-data"));
    }
    if executable_dir
        .file_name()
        .is_some_and(|name| name == "deps")
    {
        let debug = executable_dir.parent()?;
        if debug.file_name().is_some_and(|name| name == "debug") {
            return debug.parent().map(|target| target.join("hachimi-data"));
        }
    }
    None
}

pub(crate) fn resolve_storage_layout() -> StorageLayout {
    let executable = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("hachimi-desktop"));
    let executable_dir = executable
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    if let Some(override_root) = std::env::var_os("HACHIMI_DATA_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        let root = absolute_path(override_root);
        return StorageLayout {
            webview: root.join("webview"),
            root,
            mode: StorageMode::Override,
            redirect_webview: true,
        };
    }

    if executable_dir.join(PORTABLE_MARKER_FILE).is_file() {
        let root = executable_dir.join("data");
        return StorageLayout {
            webview: root.join("webview"),
            root,
            mode: StorageMode::Portable,
            redirect_webview: true,
        };
    }

    if cfg!(debug_assertions)
        && let Some(root) = debug_data_root(&executable)
    {
        return StorageLayout {
            webview: root.join("webview"),
            root,
            mode: StorageMode::Debug,
            redirect_webview: true,
        };
    }

    let root = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| executable_dir.join("data"))
        .join(APP_IDENTIFIER);
    let webview_base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| executable_dir.join("data-local"));
    let webview = webview_base.join(APP_IDENTIFIER).join("EBWebView");
    StorageLayout {
        root,
        webview,
        mode: StorageMode::Installed,
        redirect_webview: false,
    }
}

#[cfg(all(debug_assertions, feature = "desktop-e2e"))]
pub(crate) fn desktop_e2e_path(variable: &str) -> Option<PathBuf> {
    std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(absolute_path)
}

#[cfg(not(all(debug_assertions, feature = "desktop-e2e")))]
pub(crate) fn desktop_e2e_path(_variable: &str) -> Option<PathBuf> {
    None
}

fn reset_marker_path() -> PathBuf {
    std::env::temp_dir().join(RESET_MARKER_FILE)
}

pub(crate) fn write_reset_marker(layout: &StorageLayout) -> Result<(), String> {
    let marker = ResetMarker {
        version: 1,
        root: layout.root.clone(),
        webview: layout.webview.clone(),
    };
    let encoded = serde_json::to_vec(&marker)
        .map_err(|error| format!("failed to serialize reset marker: {error}"))?;
    std::fs::write(reset_marker_path(), encoded)
        .map_err(|error| format!("failed to write reset marker: {error}"))
}

fn remove_reset_directory(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub(crate) fn perform_pending_reset(layout: &StorageLayout) -> Result<(), String> {
    let marker = reset_marker_path();
    if !marker.is_file() {
        return Ok(());
    }

    let pending: ResetMarker = serde_json::from_slice(
        &std::fs::read(&marker).map_err(|error| format!("failed to read reset marker: {error}"))?,
    )
    .map_err(|error| format!("invalid reset marker: {error}"))?;
    if pending.version != 1 || pending.root != layout.root || pending.webview != layout.webview {
        return Ok(());
    }

    if layout.root.exists() {
        let sentinel = std::fs::read_to_string(layout.root.join(DATA_ROOT_SENTINEL_FILE))
            .map_err(|error| format!("refusing to clear unverified data root: {error}"))?;
        if sentinel.trim() != APP_IDENTIFIER {
            return Err("refusing to clear a data root not owned by Hachimi".into());
        }
        remove_reset_directory(&layout.root)
            .map_err(|error| format!("failed to clear {}: {error}", layout.root.display()))?;
    }
    if !layout.webview.starts_with(&layout.root) {
        remove_reset_directory(&layout.webview)
            .map_err(|error| format!("failed to clear {}: {error}", layout.webview.display()))?;
    }
    SystemApiKeyStore
        .clear()
        .map_err(|error| format!("failed to clear Hachimi credentials: {error}"))?;
    std::fs::remove_file(&marker)
        .map_err(|error| format!("failed to consume reset marker: {error}"))?;
    Ok(())
}

pub(crate) fn configure_webview_storage(layout: &StorageLayout) {
    #[cfg(all(debug_assertions, feature = "desktop-e2e"))]
    {
        let _ = layout;
    }
    #[cfg(not(all(debug_assertions, feature = "desktop-e2e")))]
    {
        if !layout.redirect_webview {
            return;
        }
        unsafe {
            std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", &layout.webview);
        }
    }
}
