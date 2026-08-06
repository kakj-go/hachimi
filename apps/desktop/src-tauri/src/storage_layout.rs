use crate::app_shell::absolute_path;
#[cfg(not(windows))]
use hachimi_llm::{ApiKeyStore, SystemApiKeyStore};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub(crate) const APP_IDENTIFIER: &str = "com.hachimi.desktop";
const PORTABLE_MARKER_FILE: &str = "hachimi.portable";
const RESET_MARKER_FILE: &str = "hachimi-reset-all-v1.marker";
const SCHEMA_EPOCH_RESET_JOURNAL_FILE: &str = "hachimi-schema-epoch-v2-reset.journal";
const SCHEMA_EPOCH_FILE: &str = ".hachimi-schema-epoch";
const SCHEMA_EPOCH: u32 = 2;
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct SchemaEpochResetJournal {
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

fn schema_epoch_reset_journal_path() -> PathBuf {
    std::env::temp_dir().join(SCHEMA_EPOCH_RESET_JOURNAL_FILE)
}

pub(crate) fn prepare_schema_epoch(layout: &StorageLayout) -> Result<(), String> {
    prepare_schema_epoch_with(layout, clear_managed_credentials)
}

fn prepare_schema_epoch_with(
    layout: &StorageLayout,
    clear_credentials: impl Fn() -> Result<(), String>,
) -> Result<(), String> {
    perform_pending_reset(layout)?;
    let journal_path = schema_epoch_reset_journal_path();
    if std::fs::read_to_string(layout.root.join(SCHEMA_EPOCH_FILE))
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        == Some(SCHEMA_EPOCH)
        && !journal_path.is_file()
    {
        return Ok(());
    }

    ensure_owned_data_root(&layout.root)?;
    let journal = if journal_path.is_file() {
        serde_json::from_slice::<SchemaEpochResetJournal>(
            &std::fs::read(&journal_path)
                .map_err(|error| format!("failed to read schema reset journal: {error}"))?,
        )
        .map_err(|error| format!("invalid schema reset journal: {error}"))?
    } else {
        let journal = SchemaEpochResetJournal {
            version: SCHEMA_EPOCH,
            root: layout.root.clone(),
            webview: layout.webview.clone(),
        };
        std::fs::write(
            &journal_path,
            serde_json::to_vec(&journal)
                .map_err(|error| format!("failed to encode schema reset journal: {error}"))?,
        )
        .map_err(|error| format!("failed to write schema reset journal: {error}"))?;
        journal
    };
    if journal.version != SCHEMA_EPOCH
        || journal.root != layout.root
        || journal.webview != layout.webview
    {
        return Err("schema reset journal belongs to another Hachimi storage root".into());
    }

    clear_credentials()?;
    clear_schema_epoch_agent_data(layout)?;
    std::fs::write(
        layout.root.join(SCHEMA_EPOCH_FILE),
        SCHEMA_EPOCH.to_string(),
    )
    .map_err(|error| format!("failed to persist schema epoch: {error}"))?;
    std::fs::remove_file(&journal_path)
        .map_err(|error| format!("failed to consume schema reset journal: {error}"))?;
    Ok(())
}

fn ensure_owned_data_root(root: &Path) -> Result<(), String> {
    if !root.exists() {
        std::fs::create_dir_all(root)
            .map_err(|error| format!("failed to create Hachimi data root: {error}"))?;
        std::fs::write(root.join(DATA_ROOT_SENTINEL_FILE), APP_IDENTIFIER)
            .map_err(|error| format!("failed to mark Hachimi data root: {error}"))?;
        return Ok(());
    }
    let sentinel = std::fs::read_to_string(root.join(DATA_ROOT_SENTINEL_FILE))
        .map_err(|error| format!("refusing to reset unverified data root: {error}"))?;
    if sentinel.trim() != APP_IDENTIFIER {
        return Err("refusing to reset a data root not owned by Hachimi".into());
    }
    Ok(())
}

fn clear_schema_epoch_agent_data(layout: &StorageLayout) -> Result<(), String> {
    for directory in [
        "agent-artifacts",
        "agent-workspaces",
        "attachments",
        "browser",
        "runtime",
        "worktrees",
    ] {
        remove_reset_directory(&layout.root.join(directory)).map_err(|error| {
            format!("failed to clear managed {directory} data during V2 reset: {error}")
        })?;
    }
    if layout.webview == layout.root {
        return Err(
            "refusing to clear schema data because WebView root equals Hachimi data root".into(),
        );
    }
    remove_reset_directory(&layout.webview).map_err(|error| {
        format!("failed to clear managed WebView data during V2 reset: {error}")
    })?;
    for path in [
        layout.root.join("agent-v2.sqlite3"),
        layout.root.join("agent-v2.sqlite3-shm"),
        layout.root.join("agent-v2.sqlite3-wal"),
        layout.root.join("agent-v2.sqlite3.migrate.lock"),
    ] {
        remove_reset_file(&path)?;
    }
    remove_reset_directory(&layout.root.join("agent-v2.sqlite3.backups"))
        .map_err(|error| format!("failed to clear migration backups: {error}"))?;
    for directory in [
        std::env::temp_dir().join("hachimi-agent-runs"),
        std::env::temp_dir().join("hachimi-computer-frames"),
        std::env::temp_dir().join("hachimi-process-runs"),
    ] {
        remove_reset_directory(&directory)
            .map_err(|error| format!("failed to clear managed run data: {error}"))?;
    }
    Ok(())
}

fn remove_reset_file(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to clear {}: {error}", path.display())),
    }
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
    clear_managed_credentials()?;
    std::fs::remove_file(&marker)
        .map_err(|error| format!("failed to consume reset marker: {error}"))?;
    Ok(())
}

#[cfg(windows)]
fn clear_managed_credentials() -> Result<(), String> {
    use windows::{
        Win32::Security::Credentials::{CREDENTIALW, CredDeleteW, CredEnumerateW, CredFree},
        core::{HRESULT, PCWSTR},
    };

    let mut count = 0_u32;
    let mut credentials = std::ptr::null_mut::<*mut CREDENTIALW>();
    // SAFETY: Windows allocates the returned array. It is inspected read-only and always released
    // with CredFree before this function returns.
    if let Err(error) =
        unsafe { CredEnumerateW(PCWSTR::null(), None, &mut count, &mut credentials) }
    {
        if error.code() == HRESULT::from_win32(1168) {
            return Ok(());
        }
        return Err(format!("failed to enumerate Hachimi credentials: {error}"));
    }
    struct CredentialBuffer(*mut *mut CREDENTIALW);
    impl Drop for CredentialBuffer {
        fn drop(&mut self) {
            // SAFETY: the pointer was returned by CredEnumerateW and is released exactly once.
            unsafe { CredFree(self.0.cast()) };
        }
    }
    let _buffer = CredentialBuffer(credentials);
    // SAFETY: CredEnumerateW returned `count` initialized pointers in this allocation.
    let entries = unsafe { std::slice::from_raw_parts(credentials, count as usize) };
    for credential in entries.iter().copied() {
        // SAFETY: each pointer belongs to the live enumeration buffer.
        let record = unsafe { &*credential };
        let target = unsafe { record.TargetName.to_string() }
            .map_err(|error| format!("failed to decode credential target: {error}"))?;
        if !is_managed_credential_target(&target) {
            continue;
        }
        // SAFETY: TargetName and Type come from the live CREDENTIALW record.
        if let Err(error) = unsafe { CredDeleteW(PCWSTR(record.TargetName.0), record.Type, None) }
            && error.code() != HRESULT::from_win32(1168)
        {
            return Err(format!("failed to delete Hachimi credential: {error}"));
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn clear_managed_credentials() -> Result<(), String> {
    SystemApiKeyStore
        .clear()
        .map_err(|error| format!("failed to clear Hachimi credentials: {error}"))
}

fn is_managed_credential_target(target: &str) -> bool {
    const SERVICES: [&str; 7] = [
        "com.hachimi.desktop",
        "com.hachimi.forge",
        "com.hachimi.connector",
        "com.hachimi.channel",
        "com.hachimi.integration",
        "com.hachimi.mcp-header",
        "com.hachimi.desktop.browser-extension",
    ];
    SERVICES.iter().any(|service| {
        target == *service
            || target
                .strip_suffix(service)
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_target_filter_covers_only_hachimi_services() {
        assert!(is_managed_credential_target(
            "account-1.com.hachimi.connector"
        ));
        assert!(is_managed_credential_target(
            "approved-installation-digest.com.hachimi.desktop.browser-extension"
        ));
        assert!(!is_managed_credential_target("account-1.github.com"));
        assert!(!is_managed_credential_target(
            "com.hachimi.connector.attacker"
        ));
    }

    #[test]
    fn schema_epoch_reset_preserves_appearance_avatar_and_selected_directories() {
        let temporary = tempfile::tempdir().expect("temporary root");
        let root = temporary.path().join("data");
        let selected = temporary.path().join("selected-schedule");
        std::fs::create_dir_all(root.join("agent-workspaces/session-1")).expect("workspace");
        std::fs::create_dir_all(root.join("worktrees/project-1")).expect("worktree");
        std::fs::create_dir_all(root.join("browser/cef-profile")).expect("browser");
        std::fs::create_dir_all(root.join("webview/EBWebView/Default")).expect("webview");
        std::fs::create_dir_all(root.join("models/avatar-1")).expect("avatar");
        std::fs::create_dir_all(&selected).expect("selected directory");
        std::fs::write(root.join("settings.json"), b"appearance").expect("settings");
        std::fs::write(root.join("agent-v2.sqlite3"), b"database").expect("database");
        std::fs::write(root.join("models/avatar-1/model.vrm"), b"avatar").expect("avatar file");
        std::fs::write(selected.join("user.txt"), b"user").expect("selected file");
        let layout = StorageLayout {
            root: root.clone(),
            webview: root.join("webview"),
            mode: StorageMode::Override,
            redirect_webview: true,
        };

        clear_schema_epoch_agent_data(&layout).expect("epoch reset");

        assert!(!root.join("agent-v2.sqlite3").exists());
        assert!(!root.join("agent-workspaces").exists());
        assert!(!root.join("worktrees").exists());
        assert!(!root.join("browser").exists());
        assert!(!root.join("webview").exists());
        assert_eq!(
            std::fs::read(root.join("settings.json")).expect("settings preserved"),
            b"appearance"
        );
        assert!(root.join("models/avatar-1/model.vrm").is_file());
        assert!(selected.join("user.txt").is_file());
    }

    #[test]
    fn schema_epoch_reset_journal_survives_credential_failure_and_retries() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };

        let temporary = tempfile::tempdir().expect("temporary root");
        let root = temporary.path().join("data");
        let layout = StorageLayout {
            root: root.clone(),
            webview: temporary.path().join("webview"),
            mode: StorageMode::Override,
            redirect_webview: true,
        };
        let should_fail = Arc::new(AtomicBool::new(true));
        let first_flag = Arc::clone(&should_fail);
        let first = prepare_schema_epoch_with(&layout, move || {
            if first_flag.swap(false, Ordering::SeqCst) {
                Err("credential manager unavailable".into())
            } else {
                Ok(())
            }
        });
        assert!(first.is_err());
        assert!(schema_epoch_reset_journal_path().is_file());

        prepare_schema_epoch_with(&layout, || Ok(())).expect("retry reset");
        assert!(!schema_epoch_reset_journal_path().exists());
        assert_eq!(
            std::fs::read_to_string(root.join(SCHEMA_EPOCH_FILE)).expect("epoch"),
            SCHEMA_EPOCH.to_string()
        );
    }
}
