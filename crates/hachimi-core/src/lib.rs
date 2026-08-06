//! Shared, platform-independent Hachimi primitives.

use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    Pet,
    Settings,
    Workbench,
    Service,
}

impl WindowKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pet => "pet",
            Self::Settings => "settings",
            Self::Workbench => "workbench",
            Self::Service => "service",
        }
    }

    #[must_use]
    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "pet" => Some(Self::Pet),
            "settings" => Some(Self::Settings),
            "workbench" => Some(Self::Workbench),
            "service" => Some(Self::Service),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeFeatureSet {
    pub run_recovery: bool,
    pub provider_extensions: bool,
    pub provider_remote_context: bool,
    pub multi_agent: bool,
    pub git_remote_mutations: bool,
    pub plugin_runtime: bool,
    pub enterprise_integrations: bool,
}

impl RuntimeFeatureSet {
    #[must_use]
    pub const fn all_enabled() -> Self {
        Self {
            run_recovery: true,
            provider_extensions: true,
            provider_remote_context: true,
            multi_agent: true,
            git_remote_mutations: true,
            plugin_runtime: true,
            enterprise_integrations: true,
        }
    }

    #[must_use]
    pub const fn all_disabled() -> Self {
        Self {
            run_recovery: false,
            provider_extensions: false,
            provider_remote_context: false,
            multi_agent: false,
            git_remote_mutations: false,
            plugin_runtime: false,
            enterprise_integrations: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "camelCase")]
pub struct FeatureFlags {
    pub workbench: bool,
    pub motion_lab: bool,
    pub workspace_tools: bool,
    pub browser_control: bool,
    pub computer_observe: bool,
    pub computer_control: bool,
    pub remote_tts: bool,
    pub remote_gateway: bool,
    pub plugin_runtime: bool,
    pub local_gateway: bool,
    pub mcp_runtime: bool,
    pub scheduler: bool,
    pub runtime_features: RuntimeFeatureSet,
}

impl FeatureFlags {
    #[must_use]
    pub const fn all_disabled() -> Self {
        Self {
            workbench: false,
            motion_lab: false,
            workspace_tools: false,
            browser_control: false,
            computer_observe: false,
            computer_control: false,
            remote_tts: false,
            remote_gateway: false,
            plugin_runtime: false,
            local_gateway: false,
            mcp_runtime: false,
            scheduler: false,
            runtime_features: RuntimeFeatureSet::all_disabled(),
        }
    }

    #[must_use]
    pub const fn any_privileged_enabled(self) -> bool {
        self.workspace_tools
            || self.browser_control
            || self.computer_observe
            || self.computer_control
            || self.remote_gateway
            || self.plugin_runtime
            || self.local_gateway
            || self.mcp_runtime
            || self.scheduler
            || self.runtime_features.run_recovery
            || self.runtime_features.provider_extensions
            || self.runtime_features.provider_remote_context
            || self.runtime_features.multi_agent
            || self.runtime_features.git_remote_mutations
            || self.runtime_features.plugin_runtime
            || self.runtime_features.enterprise_integrations
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FeatureAvailability {
    Disabled,
    Available,
}

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("unknown window label: {0}")]
    UnknownWindow(String),
    #[error("feature is disabled: {0}")]
    FeatureDisabled(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_flags_are_closed_by_default() {
        let flags = FeatureFlags::default();
        assert_eq!(flags, FeatureFlags::all_disabled());
        assert!(!flags.any_privileged_enabled());
    }

    #[test]
    fn workbench_shell_is_not_a_privileged_runtime() {
        let flags = FeatureFlags {
            workbench: true,
            ..FeatureFlags::all_disabled()
        };
        assert!(!flags.any_privileged_enabled());
    }

    #[test]
    fn only_known_window_labels_are_accepted() {
        assert_eq!(WindowKind::from_label("pet"), Some(WindowKind::Pet));
        assert_eq!(WindowKind::from_label("other"), None);
    }
}
