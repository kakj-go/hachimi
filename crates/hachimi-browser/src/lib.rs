//! Browser Host boundary. No browser process, profile, or control tool is created in the MVP.

use hachimi_core::FeatureAvailability;

#[must_use]
pub const fn availability() -> FeatureAvailability {
    FeatureAvailability::Disabled
}
