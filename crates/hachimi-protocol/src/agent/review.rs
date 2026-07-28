// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex/codex-rs/app-server-protocol/src/protocol/v2/review.rs
// and codex-rs/protocol/src/protocol.rs
// @ 4c43465133428898aa84f0bfc02c306ed65fb66a.
// Modified for Hachimi: Session/Run lineage, mutation fencing, and persisted findings.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::{
    MutationContext, ReviewDelivery, ReviewFinding, ReviewFindingId, ReviewFindingStatus, ReviewId,
    ReviewRecord, ReviewTarget, RunRecord, SessionId, SessionRecord,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReviewStartRequest {
    pub context: MutationContext,
    pub session_id: SessionId,
    pub target: ReviewTarget,
    pub delivery: ReviewDelivery,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReviewStartSnapshot {
    pub review: ReviewRecord,
    pub session: SessionRecord,
    pub run: RunRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSnapshot {
    pub review: ReviewRecord,
    pub run: RunRecord,
    pub findings: Vec<ReviewFinding>,
    pub summary: Option<String>,
    pub overall_correctness: Option<String>,
    pub overall_confidence_score: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFindingUpdateRequest {
    pub context: MutationContext,
    pub review_id: ReviewId,
    pub finding_id: ReviewFindingId,
    pub status: ReviewFindingStatus,
}

/// Provider-facing final Review shape. A completed Assistant item may contain
/// this object directly or wrap it in surrounding text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReviewOutput {
    #[serde(default)]
    pub findings: Vec<ReviewOutputFinding>,
    #[serde(default, alias = "overall_correctness")]
    pub overall_correctness: String,
    #[serde(default, alias = "overall_explanation")]
    pub overall_explanation: String,
    #[serde(default, alias = "overall_confidence_score")]
    pub overall_confidence_score: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewOutputFinding {
    pub title: String,
    pub body: String,
    #[serde(default, alias = "confidence_score")]
    pub confidence_score: f32,
    pub priority: i32,
    #[serde(alias = "code_location")]
    pub code_location: ReviewCodeLocation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCodeLocation {
    #[serde(alias = "absolute_file_path", alias = "file_path")]
    pub file_path: String,
    #[serde(alias = "line_range")]
    pub line_range: ReviewLineRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewLineRange {
    pub start: u32,
    pub end: u32,
}
