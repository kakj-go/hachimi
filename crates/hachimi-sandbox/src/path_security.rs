use std::path::{Component, Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathAccess {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsFileIdentity {
    pub volume_serial_number: u32,
    pub file_index: u64,
}

#[derive(Debug, Error)]
pub enum PathSecurityError {
    #[error("checkout root is not a supported local NTFS path")]
    UnsupportedRoot,
    #[error("checkout root is not owned by the current user")]
    OwnershipMismatch,
    #[error("checkout root is inside a protected Windows system directory")]
    ProtectedRoot,
    #[error("workspace path is absolute or escapes the checkout")]
    EscapesCheckout,
    #[error("workspace path contains an unsupported Windows path form")]
    UnsupportedPathForm,
    #[error("workspace path contains a reserved Windows device name")]
    ReservedDeviceName,
    #[error("workspace path traverses a reparse point")]
    ReparsePoint,
    #[error("workspace write target has multiple hard links")]
    HardLink,
    #[error("workspace path does not exist")]
    NotFound,
    #[error("workspace path validation failed: {0}")]
    Io(#[from] std::io::Error),
}

pub fn validate_checkout_root(root: &Path) -> Result<PathBuf, PathSecurityError> {
    #[cfg(windows)]
    reject_checkout_root_reparse_chain(root)
        .map_err(|error| path_error_context(error, "checkout ancestor validation"))?;
    let canonical = std::fs::canonicalize(root).map_err(|error| {
        PathSecurityError::Io(std::io::Error::new(
            error.kind(),
            format!("checkout canonicalization: {error}"),
        ))
    })?;
    if !canonical.is_dir() {
        return Err(PathSecurityError::UnsupportedRoot);
    }
    #[cfg(windows)]
    {
        ensure_local_drive_path(&canonical)?;
        ensure_ntfs(&canonical)
            .map_err(|error| path_error_context(error, "checkout filesystem validation"))?;
        ensure_not_protected_system_path(&canonical)?;
        reject_reparse(&canonical)
            .map_err(|error| path_error_context(error, "checkout reparse validation"))?;
        ensure_owned_by_current_user(&canonical)
            .map_err(|error| path_error_context(error, "checkout ownership validation"))?;
    }
    Ok(canonical)
}

#[cfg(windows)]
fn ensure_not_protected_system_path(path: &Path) -> Result<(), PathSecurityError> {
    let value = path.to_string_lossy();
    let value = value.strip_prefix(r"\\?\").unwrap_or(&value);
    let first = value
        .split(['\\', '/'])
        .nth(1)
        .unwrap_or_default()
        .trim_end_matches([' ', '.']);
    if [
        "Windows",
        "Program Files",
        "Program Files (x86)",
        "ProgramData",
        "$Recycle.Bin",
        "System Volume Information",
    ]
    .iter()
    .any(|protected| first.eq_ignore_ascii_case(protected))
    {
        return Err(PathSecurityError::ProtectedRoot);
    }
    Ok(())
}

#[cfg(windows)]
fn ensure_owned_by_current_user(path: &Path) -> Result<(), PathSecurityError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_INSUFFICIENT_BUFFER, LocalFree},
        Security::{
            Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT},
            EqualSid, GetTokenInformation, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
            TOKEN_QUERY, TOKEN_USER, TokenUser,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut owner: PSID = std::ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 || owner.is_null() || descriptor.is_null() {
        return Err(std::io::Error::from_raw_os_error(status as i32).into());
    }
    let result = (|| {
        let mut token = std::ptr::null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(PathSecurityError::Io(std::io::Error::last_os_error()));
        }
        let token_result = (|| {
            let mut required = 0_u32;
            unsafe {
                GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut required);
            }
            if required == 0
                || std::io::Error::last_os_error().raw_os_error()
                    != Some(ERROR_INSUFFICIENT_BUFFER as i32)
            {
                return Err(PathSecurityError::Io(std::io::Error::last_os_error()));
            }
            let mut buffer = vec![0_u8; usize::try_from(required).unwrap_or(usize::MAX)];
            if unsafe {
                GetTokenInformation(
                    token,
                    TokenUser,
                    buffer.as_mut_ptr().cast(),
                    required,
                    &mut required,
                )
            } == 0
            {
                return Err(PathSecurityError::Io(std::io::Error::last_os_error()));
            }
            let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
            if unsafe { EqualSid(owner, user.User.Sid) } == 0 {
                return Err(PathSecurityError::OwnershipMismatch);
            }
            Ok(())
        })();
        unsafe { CloseHandle(token) };
        token_result
    })();
    unsafe { LocalFree(descriptor) };
    result
}

/// Validates a short-lived drive alias created by the trusted Workspace Host
/// against the already validated real Checkout root. This avoids granting
/// traversal over a user's profile ancestors while preserving a handle-based
/// boundary check inside the restricted worker.
#[cfg(windows)]
pub fn validate_checkout_alias_root(
    alias: &Path,
    expected_identity: WindowsFileIdentity,
) -> Result<PathBuf, PathSecurityError> {
    let value = alias.to_string_lossy();
    let bytes = value.as_bytes();
    if bytes.len() != 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'\\'
    {
        return Err(PathSecurityError::UnsupportedRoot);
    }
    let actual_identity = path_file_identity(alias)
        .map_err(|error| path_error_context(error, "checkout alias handle validation"))?;
    if actual_identity != expected_identity {
        return Err(PathSecurityError::EscapesCheckout);
    }
    Ok(alias.to_owned())
}

#[cfg(not(windows))]
pub fn validate_checkout_alias_root(
    _alias: &Path,
    _expected_identity: WindowsFileIdentity,
) -> Result<PathBuf, PathSecurityError> {
    Err(PathSecurityError::UnsupportedRoot)
}

fn path_error_context(error: PathSecurityError, context: &'static str) -> PathSecurityError {
    match error {
        PathSecurityError::Io(source) => PathSecurityError::Io(std::io::Error::new(
            source.kind(),
            format!("{context}: {source}"),
        )),
        error => error,
    }
}

#[cfg(windows)]
fn reject_checkout_root_reparse_chain(path: &Path) -> Result<(), PathSecurityError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    for ancestor in absolute.ancestors().collect::<Vec<_>>().into_iter().rev() {
        if ancestor.parent().is_none() || ancestor.as_os_str().is_empty() {
            continue;
        }
        match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) => reject_metadata_reparse(&metadata)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub fn resolve_checkout_path(
    root: &Path,
    relative: &str,
    access: PathAccess,
    allow_missing_leaf: bool,
) -> Result<PathBuf, PathSecurityError> {
    validate_relative_form(relative)?;
    let relative_path = Path::new(relative);
    let joined = root.join(relative_path);
    reject_reparse_chain(root, relative_path, allow_missing_leaf)?;
    #[cfg(windows)]
    {
        if joined.exists() {
            validate_windows_path_handle(&joined, access)?;
        } else if allow_missing_leaf {
            let parent = joined.parent().ok_or(PathSecurityError::EscapesCheckout)?;
            validate_windows_path_handle(parent, PathAccess::Read)?;
        } else {
            return Err(PathSecurityError::NotFound);
        }
        Ok(joined)
    }
    #[cfg(not(windows))]
    {
        let resolved = if joined.exists() {
            final_existing_path(&joined, access)?
        } else if allow_missing_leaf {
            let parent = joined.parent().ok_or(PathSecurityError::EscapesCheckout)?;
            let parent = final_existing_path(parent, PathAccess::Read)?;
            let name = joined
                .file_name()
                .ok_or(PathSecurityError::UnsupportedPathForm)?;
            parent.join(name)
        } else {
            return Err(PathSecurityError::NotFound);
        };
        let boundary_root = root.to_owned();
        if !component_prefix(&boundary_root, &resolved) {
            return Err(PathSecurityError::EscapesCheckout);
        }
        Ok(resolved)
    }
}

fn validate_relative_form(relative: &str) -> Result<(), PathSecurityError> {
    if relative.contains(['\0', ':', '%'])
        || relative.starts_with("\\\\")
        || relative.starts_with("//")
    {
        return Err(PathSecurityError::UnsupportedPathForm);
    }
    let path = Path::new(relative);
    for component in path.components() {
        match component {
            Component::Normal(value) => validate_component(&value.to_string_lossy())?,
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(PathSecurityError::EscapesCheckout);
            }
        }
    }
    Ok(())
}

fn validate_component(value: &str) -> Result<(), PathSecurityError> {
    if value.is_empty()
        || value.ends_with([' ', '.'])
        || value.chars().any(|character| {
            character.is_control() || matches!(character, '<' | '>' | '"' | '|' | '?' | '*')
        })
    {
        return Err(PathSecurityError::UnsupportedPathForm);
    }
    let stem = value.split('.').next().unwrap_or_default();
    let upper = stem.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
    ) || (upper.len() == 4
        && (upper.starts_with("COM") || upper.starts_with("LPT"))
        && upper.as_bytes()[3].is_ascii_digit()
        && upper.as_bytes()[3] != b'0')
    {
        return Err(PathSecurityError::ReservedDeviceName);
    }
    Ok(())
}

fn reject_reparse_chain(
    root: &Path,
    relative: &Path,
    allow_missing_leaf: bool,
) -> Result<(), PathSecurityError> {
    let mut current = root.to_owned();
    for (index, component) in relative.components().enumerate() {
        if let Component::Normal(value) = component {
            current.push(value);
            match std::fs::symlink_metadata(&current) {
                Ok(metadata) => reject_metadata_reparse(&metadata)?,
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound && allow_missing_leaf =>
                {
                    if relative.components().count() != index + 1 {
                        return Err(PathSecurityError::NotFound);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(PathSecurityError::NotFound);
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(())
}

fn reject_reparse(path: &Path) -> Result<(), PathSecurityError> {
    reject_metadata_reparse(&std::fs::symlink_metadata(path)?)
}

fn reject_metadata_reparse(metadata: &std::fs::Metadata) -> Result<(), PathSecurityError> {
    if metadata.file_type().is_symlink() {
        return Err(PathSecurityError::ReparsePoint);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(PathSecurityError::ReparsePoint);
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn final_existing_path(path: &Path, access: PathAccess) -> Result<PathBuf, PathSecurityError> {
    let _ = access;
    Ok(std::fs::canonicalize(path)?)
}

#[cfg(any(not(windows), test))]
fn component_prefix(root: &Path, candidate: &Path) -> bool {
    let root = root.components().collect::<Vec<_>>();
    let candidate = candidate.components().collect::<Vec<_>>();
    candidate.len() >= root.len()
        && root
            .iter()
            .zip(candidate.iter())
            .all(|(left, right)| left == right)
}

#[cfg(windows)]
pub fn path_file_identity(path: &Path) -> Result<WindowsFileIdentity, PathSecurityError> {
    let information = windows_file_information(path)?;
    Ok(WindowsFileIdentity {
        volume_serial_number: information.dwVolumeSerialNumber,
        file_index: u64::from(information.nFileIndexHigh) << 32
            | u64::from(information.nFileIndexLow),
    })
}

#[cfg(not(windows))]
pub fn path_file_identity(_path: &Path) -> Result<WindowsFileIdentity, PathSecurityError> {
    Err(PathSecurityError::UnsupportedRoot)
}

#[cfg(windows)]
fn validate_windows_path_handle(path: &Path, access: PathAccess) -> Result<(), PathSecurityError> {
    let information = windows_file_information(path)?;
    if information.dwFileAttributes & 0x400 != 0 {
        return Err(PathSecurityError::ReparsePoint);
    }
    if access == PathAccess::Write && information.nNumberOfLinks > 1 {
        return Err(PathSecurityError::HardLink);
    }
    Ok(())
}

#[cfg(windows)]
fn windows_file_information(
    path: &Path,
) -> Result<windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION, PathSecurityError>
{
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE,
            FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle, OPEN_EXISTING,
        },
    };

    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    let result = if unsafe { GetFileInformationByHandle(handle, &mut information) } == 0 {
        Err(std::io::Error::last_os_error().into())
    } else {
        Ok(information)
    };
    unsafe {
        CloseHandle(handle);
    }
    result
}

#[cfg(windows)]
fn ensure_local_drive_path(path: &Path) -> Result<(), PathSecurityError> {
    let value = path.to_string_lossy();
    let stripped = value.strip_prefix(r"\\?\").unwrap_or(&value);
    if stripped.starts_with("UNC\\")
        || stripped.starts_with("Volume{")
        || stripped.starts_with(r"\\")
        || stripped.starts_with(r"\.\")
    {
        return Err(PathSecurityError::UnsupportedRoot);
    }
    let bytes = stripped.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'\\' {
        return Err(PathSecurityError::UnsupportedRoot);
    }
    Ok(())
}

#[cfg(windows)]
fn ensure_ntfs(path: &Path) -> Result<(), PathSecurityError> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Storage::FileSystem::{GetVolumeInformationW, GetVolumePathNameW};

    let path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut volume_path = vec![0_u16; 512];
    if unsafe {
        GetVolumePathNameW(
            path.as_ptr(),
            volume_path.as_mut_ptr(),
            u32::try_from(volume_path.len()).unwrap_or(u32::MAX),
        )
    } == 0
    {
        return Err(PathSecurityError::Io(std::io::Error::last_os_error()));
    }
    let mut filesystem = vec![0_u16; 64];
    if unsafe {
        GetVolumeInformationW(
            volume_path.as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            filesystem.as_mut_ptr(),
            u32::try_from(filesystem.len()).unwrap_or(u32::MAX),
        )
    } == 0
    {
        return Err(PathSecurityError::Io(std::io::Error::last_os_error()));
    }
    let length = filesystem
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(filesystem.len());
    if !String::from_utf16_lossy(&filesystem[..length]).eq_ignore_ascii_case("NTFS") {
        return Err(PathSecurityError::UnsupportedRoot);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_windows_special_forms_before_io() {
        let root = Path::new("root");
        for path in [
            "../secret",
            "C:\\secret",
            "\\\\server\\share",
            "file.txt:stream",
            "NUL.txt",
            "trailing. ",
            "%TEMP%\\file",
        ] {
            assert!(
                resolve_checkout_path(root, path, PathAccess::Read, false).is_err(),
                "{path}"
            );
        }
    }

    #[test]
    fn component_boundary_is_not_a_string_prefix() {
        assert!(component_prefix(
            Path::new("/work/root"),
            Path::new("/work/root/file")
        ));
        assert!(!component_prefix(
            Path::new("/work/root"),
            Path::new("/work/root-escape")
        ));
    }

    #[cfg(windows)]
    #[test]
    fn protected_windows_directories_are_rejected_before_acl_changes() {
        assert!(matches!(
            validate_checkout_root(Path::new(r"C:\Windows")),
            Err(PathSecurityError::ProtectedRoot)
        ));
    }
}
