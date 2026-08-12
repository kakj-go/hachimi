use serde::{Deserialize, Serialize};
use specta::Type;

use crate::{BehaviorChannel, MotionAssetId, MotionChannelWeight};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MotionFamily {
    Idle,
    Reaction,
    Gesture,
    Speech,
    Locomotion,
    Performance,
    Recovery,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MotionLoopMode {
    Once,
    Loop,
    Hold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MotionRole {
    WalkStart,
    WalkLoop,
    WalkStop,
    TurnLeft,
    TurnRight,
    LocomotionRecoverToIdle,
    ActionRecoverToIdle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MotionSlot {
    Base,
    Locomotion,
    Speech,
    Action,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MotionInterruptPolicy {
    Immediate,
    SafePoint,
    Finish,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MotionTransitionWindow {
    pub start_ms: u32,
    pub end_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MotionInertialHalfLives {
    pub root_ms: u16,
    pub body_ms: u16,
    pub arms_ms: u16,
    pub look_at_ms: u16,
    pub expression_ms: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MotionTransitionProfile {
    pub id: String,
    pub family: MotionFamily,
    pub preferred_duration_ms: u16,
    pub minimum_duration_ms: u16,
    pub maximum_duration_ms: u16,
    pub interrupt_policy: MotionInterruptPolicy,
    pub blend_profile_id: String,
    pub sync_group: Option<String>,
    pub entry_windows: Vec<MotionTransitionWindow>,
    pub exit_windows: Vec<MotionTransitionWindow>,
    pub channel_mask: Vec<BehaviorChannel>,
    #[serde(default)]
    pub inertial_half_lives: Option<MotionInertialHalfLives>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MotionLocomotionIntent {
    pub direction: Vec<f32>,
    pub desired_speed: f32,
    pub remaining_distance: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MotionIntentRequest {
    pub request_id: String,
    pub motion_id: MotionAssetId,
    pub slot: MotionSlot,
    pub active: bool,
    pub priority: u16,
    pub interrupt_policy: MotionInterruptPolicy,
    pub mirror: bool,
    pub channel_weights: Vec<MotionChannelWeight>,
    #[serde(default)]
    pub locomotion: Option<MotionLocomotionIntent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MotionFeatureCacheReadRequest {
    pub cache_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MotionFeatureCacheWriteRequest {
    pub cache_key: String,
    pub payload: String,
}
