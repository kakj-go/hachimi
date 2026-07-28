// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex/codex-rs/core/src/tasks/review.rs
// @ 4c43465133428898aa84f0bfc02c306ed65fb66a.
// Modified for Hachimi: SQLite Review records, typed items, and finding state.

use hachimi_protocol::{
    ItemId, ItemPayload, ItemRelations, ItemStatus, ReviewDelivery, ReviewFinding, ReviewFindingId,
    ReviewFindingStatus, ReviewId, ReviewOutput, ReviewRecord, ReviewSeverity, ReviewSnapshot,
    ReviewTarget, RunEventPayload, RunId, SessionId, TranscriptItem, TranscriptItemKind,
};
use serde_json::json;
use sqlx::Row;

use super::{
    AgentStore, AgentStoreError, append_event_typed_tx, next_sequence_tx, transcript_kind_db,
};

impl AgentStore {
    pub async fn create_review_record(
        &self,
        review: &ReviewRecord,
    ) -> Result<ReviewRecord, AgentStoreError> {
        let target_json = serde_json::to_string(&review.target)?;
        sqlx::query(
            "INSERT INTO review_runs (id, session_id, run_id, target_json, delivery, created_at_ms) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT(run_id) DO NOTHING",
        )
        .bind(review.id.as_str())
        .bind(review.session_id.as_str())
        .bind(review.run_id.as_str())
        .bind(target_json)
        .bind(review_delivery_db(review.delivery))
        .bind(review.created_at_ms)
        .execute(&self.pool)
        .await?;
        self.get_review_by_run(&review.run_id)
            .await?
            .ok_or_else(|| AgentStoreError::ReviewNotFound(review.id.clone()))
    }

    pub async fn get_review(
        &self,
        review_id: &ReviewId,
    ) -> Result<Option<ReviewRecord>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM review_runs WHERE id = ?")
            .bind(review_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(review_from_row).transpose()
    }

    pub async fn get_review_by_run(
        &self,
        run_id: &RunId,
    ) -> Result<Option<ReviewRecord>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM review_runs WHERE run_id = ?")
            .bind(run_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(review_from_row).transpose()
    }

    pub async fn list_reviews(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<ReviewRecord>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM review_runs WHERE session_id = ? ORDER BY created_at_ms DESC, id DESC",
        )
        .bind(session_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(review_from_row).collect()
    }

    pub async fn complete_review(
        &self,
        review: &ReviewRecord,
        output: &ReviewOutput,
        findings: &[ReviewFinding],
        used_plain_text_fallback: bool,
        created_at_ms: i64,
    ) -> Result<ReviewSnapshot, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM review_runs WHERE id = ?")
            .bind(review.id.as_str())
            .fetch_one(&mut *transaction)
            .await?
            > 0;
        if !exists {
            return Err(AgentStoreError::ReviewNotFound(review.id.clone()));
        }
        let already_completed = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM transcript_items WHERE run_id = ? AND kind = 'review'",
        )
        .bind(review.run_id.as_str())
        .fetch_one(&mut *transaction)
        .await?
            > 0;
        if !already_completed {
            for finding in findings {
                if finding.review_id != review.id {
                    return Err(AgentStoreError::InvalidPersistedValue {
                        kind: "review finding",
                        value: "finding belongs to another Review".into(),
                    });
                }
                sqlx::query(
                    "INSERT INTO review_findings (id, review_id, severity, file, line, message, evidence, status) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(finding.id.as_str())
                .bind(review.id.as_str())
                .bind(review_severity_db(finding.severity))
                .bind(&finding.file)
                .bind(finding.line.map(i64::from))
                .bind(&finding.message)
                .bind(&finding.evidence)
                .bind(review_finding_status_db(finding.status))
                .execute(&mut *transaction)
                .await?;
            }
            let summary = bounded_summary(&output.overall_explanation);
            let mut item = TranscriptItem {
                id: ItemId::random(),
                session_id: review.session_id.clone(),
                run_id: Some(review.run_id.clone()),
                sequence: 0,
                kind: TranscriptItemKind::Review,
                status: ItemStatus::Completed,
                payload: ItemPayload::Review {
                    review_id: review.id.clone(),
                    summary: summary.clone(),
                    overall_correctness: output.overall_correctness.clone(),
                    overall_confidence_score: output.overall_confidence_score,
                    finding_count: u32::try_from(findings.len()).unwrap_or(u32::MAX),
                    used_plain_text_fallback,
                },
                relations: ItemRelations::default(),
                created_at_ms,
            };
            item.sequence =
                next_sequence_tx(&mut transaction, &review.session_id, created_at_ms).await?;
            sqlx::query(
                "INSERT INTO transcript_items (id, session_id, run_id, sequence, kind, status, payload_json, relations_json, created_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(item.id.as_str())
            .bind(item.session_id.as_str())
            .bind(item.run_id.as_ref().map(RunId::as_str))
            .bind(i64::try_from(item.sequence).unwrap_or(i64::MAX))
            .bind(transcript_kind_db(item.kind))
            .bind(item.status.as_str())
            .bind(serde_json::to_string(&item.payload)?)
            .bind(serde_json::to_string(&item.relations)?)
            .bind(item.created_at_ms)
            .execute(&mut *transaction)
            .await?;
            append_event_typed_tx(
                &mut transaction,
                &review.session_id,
                Some(&review.run_id),
                "item.completed",
                Some(RunEventPayload::ItemCompleted {
                    item_id: item.id,
                    status: ItemStatus::Completed,
                    payload: Box::new(item.payload),
                }),
                json!({ "reviewId": review.id, "findingCount": findings.len() }),
                created_at_ms,
            )
            .await?;
        }
        transaction.commit().await?;
        self.review_snapshot(&review.id).await
    }

    pub async fn review_snapshot(
        &self,
        review_id: &ReviewId,
    ) -> Result<ReviewSnapshot, AgentStoreError> {
        let review = self
            .get_review(review_id)
            .await?
            .ok_or_else(|| AgentStoreError::ReviewNotFound(review_id.clone()))?;
        let run = self
            .get_run(&review.run_id)
            .await?
            .ok_or_else(|| AgentStoreError::RunNotFound(review.run_id.clone()))?;
        let findings = self.list_review_findings(review_id).await?;
        let payload = sqlx::query_scalar::<_, String>(
            "SELECT payload_json FROM transcript_items WHERE run_id = ? AND kind = 'review' ORDER BY sequence DESC LIMIT 1",
        )
        .bind(review.run_id.as_str())
        .fetch_optional(&self.pool)
        .await?
        .map(|value| serde_json::from_str::<ItemPayload>(&value))
        .transpose()?;
        let (summary, overall_correctness, overall_confidence_score) = match payload {
            Some(ItemPayload::Review {
                summary,
                overall_correctness,
                overall_confidence_score,
                ..
            }) => (
                Some(summary),
                Some(overall_correctness),
                Some(overall_confidence_score),
            ),
            _ => (None, None, None),
        };
        Ok(ReviewSnapshot {
            review,
            run,
            findings,
            summary,
            overall_correctness,
            overall_confidence_score,
        })
    }

    pub async fn list_review_findings(
        &self,
        review_id: &ReviewId,
    ) -> Result<Vec<ReviewFinding>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM review_findings WHERE review_id = ? ORDER BY CASE severity WHEN 'critical' THEN 0 WHEN 'error' THEN 1 WHEN 'warning' THEN 2 ELSE 3 END, id",
        )
        .bind(review_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(review_finding_from_row).collect()
    }

    pub async fn update_review_finding_status(
        &self,
        review_id: &ReviewId,
        finding_id: &ReviewFindingId,
        status: ReviewFindingStatus,
    ) -> Result<ReviewFinding, AgentStoreError> {
        let result =
            sqlx::query("UPDATE review_findings SET status = ? WHERE id = ? AND review_id = ?")
                .bind(review_finding_status_db(status))
                .bind(finding_id.as_str())
                .bind(review_id.as_str())
                .execute(&self.pool)
                .await?;
        if result.rows_affected() != 1 {
            return Err(AgentStoreError::ReviewFindingNotFound(finding_id.clone()));
        }
        let row = sqlx::query("SELECT * FROM review_findings WHERE id = ?")
            .bind(finding_id.as_str())
            .fetch_one(&self.pool)
            .await?;
        review_finding_from_row(&row)
    }
}

fn review_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<ReviewRecord, AgentStoreError> {
    Ok(ReviewRecord {
        id: ReviewId::new(row.get::<String, _>("id")),
        session_id: SessionId::new(row.get::<String, _>("session_id")),
        run_id: RunId::new(row.get::<String, _>("run_id")),
        target: serde_json::from_str::<ReviewTarget>(&row.get::<String, _>("target_json"))?,
        delivery: match row.get::<String, _>("delivery").as_str() {
            "inline" => ReviewDelivery::Inline,
            "detached" => ReviewDelivery::Detached,
            value => {
                return Err(AgentStoreError::InvalidPersistedValue {
                    kind: "review delivery",
                    value: value.into(),
                });
            }
        },
        created_at_ms: row.get("created_at_ms"),
    })
}

fn review_finding_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<ReviewFinding, AgentStoreError> {
    let persisted_line = row.get::<Option<i64>, _>("line");
    let line = persisted_line.map(u32::try_from).transpose().map_err(|_| {
        AgentStoreError::InvalidPersistedValue {
            kind: "review finding line",
            value: persisted_line.map_or_else(|| "null".into(), |value| value.to_string()),
        }
    })?;
    Ok(ReviewFinding {
        id: ReviewFindingId::new(row.get::<String, _>("id")),
        review_id: ReviewId::new(row.get::<String, _>("review_id")),
        severity: match row.get::<String, _>("severity").as_str() {
            "info" => ReviewSeverity::Info,
            "warning" => ReviewSeverity::Warning,
            "error" => ReviewSeverity::Error,
            "critical" => ReviewSeverity::Critical,
            value => {
                return Err(AgentStoreError::InvalidPersistedValue {
                    kind: "review severity",
                    value: value.into(),
                });
            }
        },
        file: row.get("file"),
        line,
        message: row.get("message"),
        evidence: row.get("evidence"),
        status: match row.get::<String, _>("status").as_str() {
            "open" => ReviewFindingStatus::Open,
            "acknowledged" => ReviewFindingStatus::Acknowledged,
            "resolved" => ReviewFindingStatus::Resolved,
            "dismissed" => ReviewFindingStatus::Dismissed,
            value => {
                return Err(AgentStoreError::InvalidPersistedValue {
                    kind: "review finding status",
                    value: value.into(),
                });
            }
        },
    })
}

const fn review_delivery_db(value: ReviewDelivery) -> &'static str {
    match value {
        ReviewDelivery::Inline => "inline",
        ReviewDelivery::Detached => "detached",
    }
}

const fn review_severity_db(value: ReviewSeverity) -> &'static str {
    match value {
        ReviewSeverity::Info => "info",
        ReviewSeverity::Warning => "warning",
        ReviewSeverity::Error => "error",
        ReviewSeverity::Critical => "critical",
    }
}

const fn review_finding_status_db(value: ReviewFindingStatus) -> &'static str {
    match value {
        ReviewFindingStatus::Open => "open",
        ReviewFindingStatus::Acknowledged => "acknowledged",
        ReviewFindingStatus::Resolved => "resolved",
        ReviewFindingStatus::Dismissed => "dismissed",
    }
}

fn bounded_summary(value: &str) -> String {
    value.trim().chars().take(32_000).collect()
}
