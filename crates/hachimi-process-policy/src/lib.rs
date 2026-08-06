//! Central child-process presentation policy for GUI hosts.
//!
//! Windows GUI applications must opt out of console allocation for captured
//! helpers. Interactive PTYs and user-requested applications are deliberately
//! left alone because they own their presentation surface.

use std::ffi::OsStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessPolicy {
    HiddenCaptured,
    HiddenBackground,
    InteractivePty,
    VisibleApplication,
}

impl ProcessPolicy {
    pub fn apply_std(self, command: &mut std::process::Command) -> &mut std::process::Command {
        #[cfg(windows)]
        if matches!(self, Self::HiddenCaptured | Self::HiddenBackground) {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command
    }

    pub fn apply_tokio(
        self,
        command: &mut tokio::process::Command,
    ) -> &mut tokio::process::Command {
        self.apply_std(command.as_std_mut());
        command
    }
}

#[must_use]
pub fn std_command(program: impl AsRef<OsStr>, policy: ProcessPolicy) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    policy.apply_std(&mut command);
    command
}

#[must_use]
pub fn tokio_command(program: impl AsRef<OsStr>, policy: ProcessPolicy) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    policy.apply_tokio(&mut command);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_policies_configure_without_changing_program() {
        for policy in [
            ProcessPolicy::HiddenCaptured,
            ProcessPolicy::HiddenBackground,
            ProcessPolicy::InteractivePty,
            ProcessPolicy::VisibleApplication,
        ] {
            let command = std_command("hachimi-policy-probe", policy);
            assert_eq!(command.get_program(), "hachimi-policy-probe");
        }
    }
}
