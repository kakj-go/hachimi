use std::path::PathBuf;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use hachimi_protocol::{SystemBrowserInstallation, SystemBrowserKind};

const MINIMUM_CHROME_MAJOR: u32 = 120;
const MINIMUM_EDGE_MAJOR: u32 = 120;

#[must_use]
pub(super) fn detect_system_browsers() -> Vec<SystemBrowserInstallation> {
    [SystemBrowserKind::Chrome, SystemBrowserKind::Edge]
        .into_iter()
        .filter_map(detect_browser)
        .collect()
}

fn detect_browser(kind: SystemBrowserKind) -> Option<SystemBrowserInstallation> {
    let executable_name = match kind {
        SystemBrowserKind::Chrome => "chrome.exe",
        SystemBrowserKind::Edge => "msedge.exe",
    };
    let mut candidates = registry_app_paths(executable_name);
    candidates.extend(standard_paths(kind));
    candidates.extend(path_candidates(executable_name));
    let executable = candidates
        .into_iter()
        .find(|candidate| candidate.is_file())?;
    let version = registry_version(kind).or_else(|| executable_version(&executable));
    Some(SystemBrowserInstallation {
        kind,
        executable_path: executable.to_string_lossy().into_owned(),
        supported: version
            .as_deref()
            .is_some_and(|version| browser_version_supported(kind, version)),
        version,
        extension_store_url: extension_store_url(kind),
    })
}

fn browser_version_supported(kind: SystemBrowserKind, version: &str) -> bool {
    let minimum = match kind {
        SystemBrowserKind::Chrome => MINIMUM_CHROME_MAJOR,
        SystemBrowserKind::Edge => MINIMUM_EDGE_MAJOR,
    };
    version
        .trim()
        .split('.')
        .next()
        .and_then(|major| major.parse::<u32>().ok())
        .is_some_and(|major| major >= minimum)
}

#[cfg(windows)]
fn executable_version(executable: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new(executable)
        .arg("--version")
        .creation_flags(0x0800_0000)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    text.split_whitespace()
        .find(|part| {
            part.chars()
                .next()
                .is_some_and(|value| value.is_ascii_digit())
        })
        .map(|value| value.trim().to_owned())
}

#[cfg(not(windows))]
fn executable_version(_executable: &std::path::Path) -> Option<String> {
    None
}

#[cfg(windows)]
fn registry_app_paths(executable: &str) -> Vec<PathBuf> {
    [
        format!(r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{executable}"),
        format!(r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{executable}"),
        format!(
            r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths\{executable}"
        ),
    ]
    .into_iter()
    .filter_map(|key| registry_value(&key, None))
    .map(PathBuf::from)
    .collect()
}

#[cfg(not(windows))]
fn registry_app_paths(_executable: &str) -> Vec<PathBuf> {
    Vec::new()
}

fn standard_paths(kind: SystemBrowserKind) -> Vec<PathBuf> {
    let relative = match kind {
        SystemBrowserKind::Chrome => "Google/Chrome/Application/chrome.exe",
        SystemBrowserKind::Edge => "Microsoft/Edge/Application/msedge.exe",
    };
    ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .map(|root| root.join(relative))
        .collect()
}

#[cfg(windows)]
fn path_candidates(executable: &str) -> Vec<PathBuf> {
    let output = std::process::Command::new("where.exe")
        .arg(executable)
        .creation_flags(0x0800_0000)
        .output();
    output
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(not(windows))]
fn path_candidates(_executable: &str) -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(windows)]
fn registry_version(kind: SystemBrowserKind) -> Option<String> {
    let vendor = match kind {
        SystemBrowserKind::Chrome => "Google\\Chrome",
        SystemBrowserKind::Edge => "Microsoft\\Edge",
    };
    ["HKCU", "HKLM"].into_iter().find_map(|root| {
        registry_value(
            &format!(r"{root}\SOFTWARE\{vendor}\BLBeacon"),
            Some("version"),
        )
    })
}

#[cfg(not(windows))]
fn registry_version(_kind: SystemBrowserKind) -> Option<String> {
    None
}

#[cfg(windows)]
fn registry_value(key: &str, name: Option<&str>) -> Option<String> {
    use std::os::windows::process::CommandExt;
    let mut command = std::process::Command::new("reg.exe");
    command.arg("query").arg(key);
    match name {
        Some(name) => {
            command.arg("/v").arg(name);
        }
        None => {
            command.arg("/ve");
        }
    }
    let output = command.creation_flags(0x0800_0000).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    text.lines().find_map(|line| {
        ["REG_SZ", "REG_EXPAND_SZ"]
            .into_iter()
            .find_map(|marker| {
                line.split_once(marker)
                    .map(|(_, value)| value.trim().to_owned())
            })
            .filter(|value| !value.is_empty())
    })
}

fn extension_store_url(kind: SystemBrowserKind) -> Option<String> {
    let id = match kind {
        SystemBrowserKind::Chrome => option_env!("HACHIMI_CHROME_EXTENSION_ID"),
        SystemBrowserKind::Edge => option_env!("HACHIMI_EDGE_EXTENSION_ID"),
    }?
    .trim();
    if id.is_empty() {
        return None;
    }
    Some(match kind {
        SystemBrowserKind::Chrome => format!("https://chromewebstore.google.com/detail/{id}"),
        SystemBrowserKind::Edge => {
            format!("https://microsoftedge.microsoft.com/addons/detail/{id}")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn standard_browser_paths_are_deduplicatable() {
        let paths = standard_paths(SystemBrowserKind::Chrome);
        assert_eq!(paths.iter().collect::<BTreeSet<_>>().len(), paths.len());
    }

    #[test]
    fn browser_versions_enforce_the_supported_baseline() {
        assert!(browser_version_supported(
            SystemBrowserKind::Chrome,
            "120.0.6099.110"
        ));
        assert!(browser_version_supported(
            SystemBrowserKind::Edge,
            "135.0.3179.98"
        ));
        assert!(!browser_version_supported(
            SystemBrowserKind::Chrome,
            "119.0.0.0"
        ));
        assert!(!browser_version_supported(
            SystemBrowserKind::Edge,
            "not-a-version"
        ));
        assert!(!browser_version_supported(SystemBrowserKind::Edge, ""));
    }
}
