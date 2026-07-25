//! Workbench boundary. No file, Git, shell, PTY, or MCP tools exist in the MVP.

use hachimi_core::FeatureAvailability;

#[must_use]
pub const fn availability() -> FeatureAvailability {
    FeatureAvailability::Disabled
}
