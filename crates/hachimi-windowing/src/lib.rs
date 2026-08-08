//! Window geometry, interactive-region validation, and platform host abstraction.

use hachimi_protocol::{InteractiveRegionsUpdate, LogicalRect, WindowPlacementV1};
use thiserror::Error;

pub const MAX_INTERACTIVE_REGIONS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhysicalPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MonitorGeometry {
    pub name: Option<String>,
    pub bounds: PhysicalRect,
    pub scale_factor: f64,
    pub primary: bool,
}

pub trait DesktopWindowHost {
    type Error;

    fn set_transparent(&self, enabled: bool) -> Result<(), Self::Error>;
    fn set_always_on_top(&self, enabled: bool) -> Result<(), Self::Error>;
    fn set_click_through(&self, enabled: bool) -> Result<(), Self::Error>;
    fn set_bounds(&self, bounds: PhysicalRect) -> Result<(), Self::Error>;
    fn cursor_position(&self) -> Result<PhysicalPoint, Self::Error>;
    fn monitors(&self) -> Result<Vec<MonitorGeometry>, Self::Error>;
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum RegionError {
    #[error("interactive regions are only accepted for the pet window")]
    InvalidWindow,
    #[error("interactive region revision is stale")]
    StaleRevision,
    #[error("too many interactive regions")]
    TooManyRegions,
    #[error("window dimensions are invalid")]
    InvalidWindowBounds,
    #[error("interactive region is invalid")]
    InvalidRegion,
}

#[derive(Debug, Clone, Default)]
pub struct InteractiveRegionState {
    revision: u32,
    regions: Vec<LogicalRect>,
}

impl InteractiveRegionState {
    #[must_use]
    pub const fn revision(&self) -> u32 {
        self.revision
    }

    #[must_use]
    pub fn regions(&self) -> &[LogicalRect] {
        &self.regions
    }

    pub fn update(&mut self, update: InteractiveRegionsUpdate) -> Result<(), RegionError> {
        if update.window_label != "pet" {
            return Err(RegionError::InvalidWindow);
        }
        if update.revision <= self.revision {
            return Err(RegionError::StaleRevision);
        }
        if update.regions.len() > MAX_INTERACTIVE_REGIONS {
            return Err(RegionError::TooManyRegions);
        }
        if !update.window_width.is_finite()
            || !update.window_height.is_finite()
            || update.window_width <= 0.0
            || update.window_height <= 0.0
        {
            return Err(RegionError::InvalidWindowBounds);
        }
        if update.regions.iter().any(|region| {
            !region.has_finite_values()
                || region.width <= 0.0
                || region.height <= 0.0
                || region.x < 0.0
                || region.y < 0.0
                || region.x + region.width > update.window_width
                || region.y + region.height > update.window_height
        }) {
            return Err(RegionError::InvalidRegion);
        }

        self.revision = update.revision;
        self.regions = update.regions;
        Ok(())
    }

    #[must_use]
    pub fn hit_test(
        &self,
        cursor: PhysicalPoint,
        window_origin: PhysicalPoint,
        scale_factor: f64,
    ) -> bool {
        if !scale_factor.is_finite() || scale_factor <= 0.0 {
            return false;
        }
        let logical_x = (cursor.x - window_origin.x) / scale_factor;
        let logical_y = (cursor.y - window_origin.y) / scale_factor;
        self.regions
            .iter()
            .any(|region| region.contains(logical_x, logical_y))
    }
}

#[must_use]
pub fn placement_intersects_monitor(
    placement: &WindowPlacementV1,
    monitor: &MonitorGeometry,
) -> bool {
    let left = i64::from(placement.x);
    let top = i64::from(placement.y);
    let right = left + i64::from(placement.width);
    let bottom = top + i64::from(placement.height);
    let monitor_left = i64::from(monitor.bounds.x);
    let monitor_top = i64::from(monitor.bounds.y);
    let monitor_right = monitor_left + i64::from(monitor.bounds.width);
    let monitor_bottom = monitor_top + i64::from(monitor.bounds.height);
    right > monitor_left && left < monitor_right && bottom > monitor_top && top < monitor_bottom
}

#[must_use]
pub fn restore_or_default_placement(
    saved: Option<&WindowPlacementV1>,
    monitors: &[MonitorGeometry],
    window_width: u32,
    window_height: u32,
    margin: u32,
) -> WindowPlacementV1 {
    if let Some(saved) = saved
        && monitors
            .iter()
            .any(|monitor| placement_intersects_monitor(saved, monitor))
    {
        return saved.clone();
    }

    let monitor = monitors
        .iter()
        .find(|monitor| monitor.primary)
        .or_else(|| monitors.first());
    let Some(monitor) = monitor else {
        return WindowPlacementV1 {
            x: 0,
            y: 0,
            width: window_width,
            height: window_height,
            monitor_name: None,
            scale_factor: 1.0,
        };
    };
    let available_width = monitor.bounds.width.saturating_sub(window_width);
    let available_height = monitor.bounds.height.saturating_sub(window_height);
    let biased_offset = |available: u32| {
        let preferred = u64::from(available) * 58 / 100;
        let gutter = u64::from(margin.min(available / 2));
        preferred.clamp(gutter, u64::from(available).saturating_sub(gutter))
    };
    let x = i64::from(monitor.bounds.x)
        + i64::try_from(biased_offset(available_width)).unwrap_or(i64::MAX);
    let y = i64::from(monitor.bounds.y)
        + i64::try_from(biased_offset(available_height)).unwrap_or(i64::MAX);
    WindowPlacementV1 {
        x: x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        y: y.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        width: window_width,
        height: window_height,
        monitor_name: monitor.name.clone(),
        scale_factor: monitor.scale_factor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_update(revision: u32) -> InteractiveRegionsUpdate {
        InteractiveRegionsUpdate {
            window_label: "pet".into(),
            revision,
            window_width: 360.0,
            window_height: 480.0,
            regions: vec![LogicalRect {
                x: 100.0,
                y: 100.0,
                width: 120.0,
                height: 200.0,
            }],
        }
    }

    #[test]
    fn rejects_stale_region_updates() {
        let mut state = InteractiveRegionState::default();
        state.update(valid_update(2)).expect("first update");
        assert_eq!(
            state.update(valid_update(1)),
            Err(RegionError::StaleRevision)
        );
    }

    #[test]
    fn rejects_invalid_region_bounds_and_non_finite_values() {
        let mut state = InteractiveRegionState::default();
        let mut outside = valid_update(1);
        outside.regions[0].x = 350.0;
        outside.regions[0].width = 20.0;
        assert_eq!(state.update(outside), Err(RegionError::InvalidRegion));

        let mut non_finite = valid_update(2);
        non_finite.regions[0].x = f64::NAN;
        assert_eq!(state.update(non_finite), Err(RegionError::InvalidRegion));

        let mut invalid_window = valid_update(3);
        invalid_window.window_width = 0.0;
        assert_eq!(
            state.update(invalid_window),
            Err(RegionError::InvalidWindowBounds)
        );
    }

    #[test]
    fn rejects_more_than_the_region_limit() {
        let mut state = InteractiveRegionState::default();
        let mut update = valid_update(1);
        update.regions = vec![update.regions[0]; MAX_INTERACTIVE_REGIONS + 1];
        assert_eq!(state.update(update), Err(RegionError::TooManyRegions));
    }

    #[test]
    fn hit_test_converts_physical_coordinates_by_scale() {
        let mut state = InteractiveRegionState::default();
        state.update(valid_update(1)).expect("update");
        assert!(state.hit_test(
            PhysicalPoint { x: 400.0, y: 500.0 },
            PhysicalPoint { x: 200.0, y: 300.0 },
            2.0
        ));
        assert!(!state.hit_test(
            PhysicalPoint { x: 210.0, y: 310.0 },
            PhysicalPoint { x: 200.0, y: 300.0 },
            2.0
        ));
    }

    #[test]
    fn offscreen_saved_position_uses_primary_center_with_lower_right_bias() {
        let monitors = vec![MonitorGeometry {
            name: Some("primary".into()),
            bounds: PhysicalRect {
                x: -1920,
                y: 0,
                width: 1920,
                height: 1080,
            },
            scale_factor: 1.0,
            primary: true,
        }];
        let saved = WindowPlacementV1 {
            x: 4000,
            y: 4000,
            width: 360,
            height: 480,
            monitor_name: None,
            scale_factor: 1.0,
        };
        let restored = restore_or_default_placement(Some(&saved), &monitors, 360, 480, 24);
        assert_eq!(restored.x, -1016);
        assert_eq!(restored.y, 348);
    }

    #[test]
    fn intersecting_negative_position_survives_scale_change() {
        let monitors = vec![MonitorGeometry {
            name: Some("left".into()),
            bounds: PhysicalRect {
                x: -2560,
                y: -180,
                width: 2560,
                height: 1440,
            },
            scale_factor: 1.5,
            primary: true,
        }];
        let saved = WindowPlacementV1 {
            x: -480,
            y: 240,
            width: 360,
            height: 480,
            monitor_name: Some("left".into()),
            scale_factor: 1.0,
        };
        assert_eq!(
            restore_or_default_placement(Some(&saved), &monitors, 360, 480, 24),
            saved
        );
    }
}
