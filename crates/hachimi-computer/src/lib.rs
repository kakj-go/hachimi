//! Computer Host boundary. Screen capture and input injection are absent and disabled.

use hachimi_core::FeatureAvailability;

#[must_use]
pub const fn availability() -> FeatureAvailability {
    FeatureAvailability::Disabled
}
