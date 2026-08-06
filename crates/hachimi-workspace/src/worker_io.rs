use super::*;

pub(super) struct SearchState<'a> {
    pub(super) root: &'a Path,
    pub(super) query: &'a str,
    pub(super) folded_query: Option<String>,
    pub(super) max_results: usize,
    pub(super) visited_files: usize,
    pub(super) matches: Vec<SearchMatch>,
    pub(super) truncated: bool,
}

pub(super) fn search_path(path: &Path, state: &mut SearchState<'_>) -> Result<(), WorkspaceError> {
    if state.matches.len() >= state.max_results || state.visited_files >= MAX_SEARCHED_FILES {
        state.truncated = true;
        return Ok(());
    }
    let metadata = std::fs::symlink_metadata(path).map_err(io_error)?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_dir() {
        let mut entries = std::fs::read_dir(path)
            .map_err(io_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(io_error)?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            search_path(&entry.path(), state)?;
            if state.truncated {
                break;
            }
        }
        return Ok(());
    }
    if !metadata.is_file() || metadata.len() > MAX_TEXT_BYTES {
        return Ok(());
    }
    state.visited_files += 1;
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or_default());
    std::fs::File::open(path)
        .map_err(io_error)?
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    let Ok(content) = String::from_utf8(bytes) else {
        return Ok(());
    };
    for (index, line) in content.lines().enumerate() {
        let matches = if let Some(query) = &state.folded_query {
            line.to_lowercase().contains(query)
        } else {
            line.contains(state.query)
        };
        if matches {
            state.matches.push(SearchMatch {
                path: relative_display(state.root, path),
                line: index + 1,
                text: bounded(line, 500),
            });
            if state.matches.len() >= state.max_results {
                state.truncated = true;
                break;
            }
        }
    }
    Ok(())
}

pub(super) fn read_bounded(path: &Path) -> Result<Vec<u8>, WorkspaceError> {
    let metadata = std::fs::metadata(path).map_err(io_error)?;
    if !metadata.is_file() {
        return Err(WorkspaceError::new(
            WorkspaceErrorCode::NotFound,
            "workspace path is not a file",
        ));
    }
    if metadata.len() > MAX_TEXT_BYTES {
        return Err(WorkspaceError::new(
            WorkspaceErrorCode::TooLarge,
            "workspace file exceeds the 2 MiB text limit",
        ));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or_default());
    std::fs::File::open(path)
        .map_err(io_error)?
        .take(MAX_TEXT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_TEXT_BYTES {
        return Err(WorkspaceError::new(
            WorkspaceErrorCode::TooLarge,
            "workspace file exceeds the 2 MiB text limit",
        ));
    }
    Ok(bytes)
}

pub(super) fn ensure_content_size(content: &str) -> Result<(), WorkspaceError> {
    if u64::try_from(content.len()).unwrap_or(u64::MAX) > MAX_TEXT_BYTES {
        return Err(WorkspaceError::new(
            WorkspaceErrorCode::TooLarge,
            "workspace content exceeds the 2 MiB text limit",
        ));
    }
    Ok(())
}

pub(super) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), WorkspaceError> {
    let mut file = AtomicWriteFile::open(path).map_err(io_error)?;
    file.write_all(bytes).map_err(io_error)?;
    file.commit().map_err(io_error)
}

pub(super) fn write_output(
    root: &Path,
    path: &Path,
    bytes: &[u8],
    replacements: usize,
) -> WorkspaceOutput {
    WorkspaceOutput::Write {
        path: relative_display(root, path),
        sha256: sha256(bytes),
        byte_size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        replacements,
    }
}

pub(super) fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

pub(super) fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

pub(super) fn io_error(error: std::io::Error) -> WorkspaceError {
    WorkspaceError::new(WorkspaceErrorCode::Io, error.to_string())
}

pub(super) fn path_security_error(error: PathSecurityError) -> WorkspaceError {
    match error {
        PathSecurityError::NotFound => {
            WorkspaceError::new(WorkspaceErrorCode::NotFound, error.to_string())
        }
        PathSecurityError::Io(error) => io_error(error),
        PathSecurityError::UnsupportedRoot => WorkspaceError::new(
            WorkspaceErrorCode::UnsupportedWorkspaceRoot,
            "Workspace must be moved to a current-user-owned local NTFS directory",
        ),
        PathSecurityError::OwnershipMismatch => WorkspaceError::new(
            WorkspaceErrorCode::WorkspaceOwnershipMismatch,
            "Workspace must be owned by the current user; move it to a user-owned directory",
        ),
        PathSecurityError::ProtectedRoot => WorkspaceError::new(
            WorkspaceErrorCode::ProtectedWorkspaceRoot,
            "Workspace is inside a protected Windows directory; move it to a user-owned directory",
        ),
        PathSecurityError::EscapesCheckout
        | PathSecurityError::UnsupportedPathForm
        | PathSecurityError::ReservedDeviceName
        | PathSecurityError::ReparsePoint
        | PathSecurityError::HardLink => {
            WorkspaceError::new(WorkspaceErrorCode::PathOutsideCheckout, error.to_string())
        }
    }
}

pub(super) fn bounded(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

pub(super) fn bounded_bytes(bytes: &[u8]) -> (String, bool) {
    let truncated = bytes.len() > MAX_PROCESS_OUTPUT_BYTES;
    let bytes = &bytes[..bytes.len().min(MAX_PROCESS_OUTPUT_BYTES)];
    (String::from_utf8_lossy(bytes).into_owned(), truncated)
}

pub(super) fn copy_process_environment(command: &mut Command) {
    const ALLOWED: &[&str] = &[
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "WINDIR",
        "TEMP",
        "TMP",
        "USERPROFILE",
        "LOCALAPPDATA",
        "APPDATA",
        "CARGO_HOME",
        "RUSTUP_HOME",
        "SSH_AUTH_SOCK",
    ];
    for key in ALLOWED {
        if let Some(value) = std::env::var_os(key) {
            command.env(OsStr::new(key), value);
        }
    }
    configure_restricted_git_environment(command);
}
