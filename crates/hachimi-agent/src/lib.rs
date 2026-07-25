//! Agent runtime boundary. No agent loop is registered in Phase 0 or Phase 1.

use hachimi_core::FeatureAvailability;

#[must_use]
pub const fn availability() -> FeatureAvailability {
    FeatureAvailability::Disabled
}
