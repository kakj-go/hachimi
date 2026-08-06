//! Windows AppContainer identity used as the C2.1 filesystem and deny-all network boundary.

use std::path::Path;

pub const APP_CONTAINER_NAME: &str = "Hachimi.AgentSandbox.v1";

#[cfg(windows)]
pub struct AppContainerSid {
    sid: windows_sys::Win32::Security::PSID,
}

#[cfg(not(windows))]
pub struct AppContainerSid;

#[cfg(not(windows))]
impl AppContainerSid {
    pub fn resolve() -> Result<Self, String> {
        Err("AppContainer identities are only available on Windows".into())
    }

    pub(crate) fn ensure_profile_with_state() -> Result<(Self, bool), String> {
        Err("AppContainer identities are only available on Windows".into())
    }

    pub(crate) fn delete_profile() -> Result<(), String> {
        Err("AppContainer identities are only available on Windows".into())
    }

    pub fn to_string_sid(&self) -> Result<String, String> {
        Err("AppContainer identities are only available on Windows".into())
    }
}

#[cfg(windows)]
impl AppContainerSid {
    pub fn resolve() -> Result<Self, String> {
        use windows_sys::Win32::Security::Isolation::DeriveAppContainerSidFromAppContainerName;

        let name = wide_nul(APP_CONTAINER_NAME);
        let mut sid = std::ptr::null_mut();
        let result = unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) };
        if result < 0 || sid.is_null() {
            Err(format!(
                "DeriveAppContainerSidFromAppContainerName failed: HRESULT 0x{:08X}",
                result as u32
            ))
        } else {
            Ok(Self { sid })
        }
    }

    pub(crate) fn ensure_profile_with_state() -> Result<(Self, bool), String> {
        use windows_sys::Win32::Foundation::ERROR_ALREADY_EXISTS;
        use windows_sys::Win32::Security::Isolation::CreateAppContainerProfile;

        let name = wide_nul(APP_CONTAINER_NAME);
        let display = wide_nul("Hachimi Agent Sandbox");
        let description = wide_nul("Hachimi restricted workspace and process identity");
        let mut sid = std::ptr::null_mut();
        let result = unsafe {
            CreateAppContainerProfile(
                name.as_ptr(),
                display.as_ptr(),
                description.as_ptr(),
                std::ptr::null(),
                0,
                &mut sid,
            )
        };
        let already_exists = result as u32 == 0x8007_0000 | ERROR_ALREADY_EXISTS;
        if result >= 0 && !sid.is_null() {
            Ok((Self { sid }, true))
        } else if already_exists {
            Self::resolve().map(|identity| (identity, false))
        } else {
            Err(format!(
                "CreateAppContainerProfile failed: HRESULT 0x{:08X}",
                result as u32
            ))
        }
    }

    pub(crate) fn delete_profile() -> Result<(), String> {
        use windows_sys::Win32::Security::Isolation::DeleteAppContainerProfile;

        let name = wide_nul(APP_CONTAINER_NAME);
        let result = unsafe { DeleteAppContainerProfile(name.as_ptr()) };
        if result >= 0 || result as u32 == 0x8007_0490 {
            Ok(())
        } else {
            Err(format!(
                "DeleteAppContainerProfile failed: HRESULT 0x{:08X}",
                result as u32
            ))
        }
    }

    #[must_use]
    pub fn as_ptr(&self) -> windows_sys::Win32::Security::PSID {
        self.sid
    }

    pub fn to_string_sid(&self) -> Result<String, String> {
        use windows_sys::Win32::{
            Foundation::LocalFree, Security::Authorization::ConvertSidToStringSidW,
        };

        let mut value = std::ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(self.sid, &mut value) } == 0 || value.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mut length = 0_usize;
        while unsafe { *value.add(length) } != 0 {
            length += 1;
        }
        let string = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(value, length) });
        unsafe {
            LocalFree(value.cast());
        }
        Ok(string)
    }
}

#[cfg(windows)]
impl Drop for AppContainerSid {
    fn drop(&mut self) {
        if !self.sid.is_null() {
            unsafe {
                windows_sys::Win32::Security::FreeSid(self.sid);
            }
        }
    }
}

#[cfg(windows)]
pub fn grant_appcontainer_access(path: &Path, write: bool) -> Result<(), String> {
    let sid = AppContainerSid::resolve()?.to_string_sid()?;
    let permission = if write { "(OI)(CI)M" } else { "(OI)(CI)RX" };
    run_icacls(path, ["/grant:r", &format!("*{sid}:{permission}"), "/Q"])
}

#[cfg(windows)]
pub fn deny_appcontainer_read(path: &Path) -> Result<(), String> {
    let sid = AppContainerSid::resolve()?.to_string_sid()?;
    run_icacls(path, ["/deny", &format!("*{sid}:(OI)(CI)(RX)"), "/Q"])
}

#[cfg(windows)]
pub fn deny_appcontainer_write(path: &Path) -> Result<(), String> {
    let sid = AppContainerSid::resolve()?.to_string_sid()?;
    run_icacls(path, ["/deny", &format!("*{sid}:(OI)(CI)(W,D,DC)"), "/Q"])
}

#[cfg(windows)]
pub fn revoke_appcontainer_access(path: &Path) -> Result<(), String> {
    let sid = AppContainerSid::resolve()?.to_string_sid()?;
    let identity = format!("*{sid}");
    for mode in ["/remove:g", "/remove:d"] {
        let status = hachimi_process_policy::std_command(
            "icacls.exe",
            hachimi_process_policy::ProcessPolicy::HiddenCaptured,
        )
        .arg(path)
        .args([mode, identity.as_str(), "/Q"])
        .status()
        .map_err(|error| error.to_string())?;
        if !status.success() {
            return Err(format!("icacls {mode} exited with {status}"));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn run_icacls<const N: usize>(path: &Path, arguments: [&str; N]) -> Result<(), String> {
    let mut child = hachimi_process_policy::std_command(
        "icacls.exe",
        hachimi_process_policy::ProcessPolicy::HiddenCaptured,
    )
    .arg(path)
    .args(arguments)
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .spawn()
    .map_err(|error| error.to_string())?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            return status
                .success()
                .then_some(())
                .ok_or_else(|| format!("icacls exited with {status}"));
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "icacls timed out after 30 seconds for {}",
                path.display()
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

#[cfg(windows)]
fn wide_nul(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

#[cfg(not(windows))]
pub fn grant_appcontainer_access(_path: &Path, _write: bool) -> Result<(), String> {
    Err("AppContainer ACLs are only available on Windows".into())
}

#[cfg(not(windows))]
pub fn deny_appcontainer_read(_path: &Path) -> Result<(), String> {
    Err("AppContainer ACLs are only available on Windows".into())
}

#[cfg(not(windows))]
pub fn deny_appcontainer_write(_path: &Path) -> Result<(), String> {
    Err("AppContainer ACLs are only available on Windows".into())
}

#[cfg(not(windows))]
pub fn revoke_appcontainer_access(_path: &Path) -> Result<(), String> {
    Err("AppContainer ACLs are only available on Windows".into())
}
