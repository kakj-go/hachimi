use std::collections::{BTreeMap, BTreeSet};

use hachimi_protocol::{
    ItemPayload, ItemStatus, RunId, RunStatus, SessionId, UserInputAnswer, UserInputRequestId,
    UserInputRequestRecord, UserInputResolution, UserInputResolutionAction, UserInputStatus,
};
use serde_json::json;
use sqlx::Row;

use super::{AgentStore, AgentStoreError, append_event_tx, get_run_tx, next_sequence_tx};

impl AgentStore {
    pub async fn create_user_input_request(
        &self,
        request: &UserInputRequestRecord,
    ) -> Result<UserInputRequestRecord, AgentStoreError> {
        validate_questions(request)?;
        let mut transaction = self.pool.begin().await?;
        let run = get_run_tx(&mut transaction, &request.run_id)
            .await?
            .ok_or_else(|| AgentStoreError::RunNotFound(request.run_id.clone()))?;
        if run.session_id != request.session_id
            || run.generation != request.run_generation
            || run.status != RunStatus::Running
            || request.status != UserInputStatus::Pending
        {
            return Err(AgentStoreError::RunPreconditionFailed);
        }

        let sequence =
            next_sequence_tx(&mut transaction, &request.session_id, request.created_at_ms).await?;
        let payload = ItemPayload::UserInputRequest {
            request_id: request.id.clone(),
            questions: request.questions.clone(),
        };
        sqlx::query(
            "INSERT INTO transcript_items (id, session_id, run_id, sequence, kind, status, payload_json, relations_json, created_at_ms) VALUES (?, ?, ?, ?, 'user_input_request', 'in_progress', ?, ?, ?)",
        )
        .bind(request.item_id.as_str())
        .bind(request.session_id.as_str())
        .bind(request.run_id.as_str())
        .bind(i64::try_from(sequence).unwrap_or(i64::MAX))
        .bind(serde_json::to_string(&payload)?)
        .bind(serde_json::to_string(&json!({
            "userInputRequestId": request.id,
        }))?)
        .bind(request.created_at_ms)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO user_input_requests (id, session_id, run_id, run_generation, item_id, questions_json, status, expires_at_ms, created_at_ms, resolved_at_ms, resolved_by) VALUES (?, ?, ?, ?, ?, ?, 'pending', ?, ?, NULL, NULL)",
        )
        .bind(request.id.as_str())
        .bind(request.session_id.as_str())
        .bind(request.run_id.as_str())
        .bind(i64::try_from(request.run_generation).unwrap_or(i64::MAX))
        .bind(request.item_id.as_str())
        .bind(serde_json::to_string(&request.questions)?)
        .bind(request.expires_at_ms)
        .bind(request.created_at_ms)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE runs SET status = 'waiting_user_input', updated_at_ms = ? WHERE id = ?",
        )
        .bind(request.created_at_ms)
        .bind(request.run_id.as_str())
        .execute(&mut *transaction)
        .await?;
        append_event_tx(
            &mut transaction,
            &request.session_id,
            Some(&request.run_id),
            "user_input.requested",
            json!({
                "requestId": request.id,
                "itemId": request.item_id,
                "questionCount": request.questions.len(),
            }),
            request.created_at_ms,
        )
        .await?;
        transaction.commit().await?;
        Ok(request.clone())
    }

    pub async fn get_user_input_request(
        &self,
        request_id: &UserInputRequestId,
    ) -> Result<Option<UserInputRequestRecord>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM user_input_requests WHERE id = ?")
            .bind(request_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(user_input_from_row).transpose()
    }

    pub async fn list_pending_user_inputs(
        &self,
        session_id: Option<&SessionId>,
    ) -> Result<Vec<UserInputRequestRecord>, AgentStoreError> {
        let rows = if let Some(session_id) = session_id {
            sqlx::query(
                "SELECT * FROM user_input_requests WHERE status = 'pending' AND session_id = ? ORDER BY created_at_ms ASC, id ASC",
            )
            .bind(session_id.as_str())
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT * FROM user_input_requests WHERE status = 'pending' ORDER BY created_at_ms ASC, id ASC",
            )
            .fetch_all(&self.pool)
            .await?
        };
        rows.iter().map(user_input_from_row).collect()
    }

    /// Resolves metadata only. Answers are deliberately validated in memory and never persisted.
    pub async fn resolve_user_input(
        &self,
        resolution: &UserInputResolution,
    ) -> Result<UserInputRequestRecord, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query("SELECT * FROM user_input_requests WHERE id = ?")
            .bind(resolution.request_id.as_str())
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| AgentStoreError::UserInputNotFound(resolution.request_id.clone()))?;
        let mut request = user_input_from_row(&row)?;
        if request.status != UserInputStatus::Pending {
            return Err(AgentStoreError::UserInputNotPending(request.id));
        }
        if request.run_id != resolution.expected_run_id
            || request.run_generation != resolution.expected_generation
        {
            return Err(AgentStoreError::StaleUserInputResolution);
        }
        let run = get_run_tx(&mut transaction, &request.run_id)
            .await?
            .ok_or_else(|| AgentStoreError::RunNotFound(request.run_id.clone()))?;
        if run.generation != request.run_generation || run.status != RunStatus::WaitingUserInput {
            return Err(AgentStoreError::StaleUserInputResolution);
        }
        if resolution.action == UserInputResolutionAction::Submit {
            validate_answers(&request, &resolution.answers)?;
        } else if !resolution.answers.is_empty() {
            return Err(AgentStoreError::InvalidUserInputAnswer(
                "decline and cancel resolutions cannot contain answers",
            ));
        }

        let (request_status, item_status, event_name) = match resolution.action {
            UserInputResolutionAction::Submit => (
                UserInputStatus::Resolved,
                ItemStatus::Completed,
                "user_input.resolved",
            ),
            UserInputResolutionAction::Decline => (
                UserInputStatus::Cancelled,
                ItemStatus::Interrupted,
                "user_input.declined",
            ),
            UserInputResolutionAction::Cancel => (
                UserInputStatus::Cancelled,
                ItemStatus::Interrupted,
                "user_input.cancelled",
            ),
        };

        let updated = sqlx::query(
            "UPDATE user_input_requests SET status = ?, resolved_at_ms = ?, resolved_by = ? WHERE id = ? AND status = 'pending'",
        )
        .bind(request_status.as_str())
        .bind(resolution.resolved_at_ms)
        .bind(&resolution.resolved_by)
        .bind(request.id.as_str())
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(AgentStoreError::UserInputNotPending(request.id));
        }
        sqlx::query("UPDATE transcript_items SET status = ? WHERE id = ?")
            .bind(item_status.as_str())
            .bind(request.item_id.as_str())
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE runs SET status = 'running', updated_at_ms = ? WHERE id = ?")
            .bind(resolution.resolved_at_ms)
            .bind(request.run_id.as_str())
            .execute(&mut *transaction)
            .await?;
        append_event_tx(
            &mut transaction,
            &request.session_id,
            Some(&request.run_id),
            event_name,
            json!({
                "requestId": request.id,
                "itemId": request.item_id,
                "answerCount": resolution.answers.len(),
                "action": resolution.action,
            }),
            resolution.resolved_at_ms,
        )
        .await?;
        transaction.commit().await?;
        request.status = request_status;
        request.resolved_at_ms = Some(resolution.resolved_at_ms);
        request.resolved_by = Some(resolution.resolved_by.clone());
        Ok(request)
    }

    pub async fn cancel_run_user_inputs(
        &self,
        run_id: &RunId,
        resolved_at_ms: i64,
        resolved_by: &str,
    ) -> Result<u64, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let rows = sqlx::query(
            "SELECT id, session_id, item_id FROM user_input_requests WHERE run_id = ? AND status = 'pending'",
        )
        .bind(run_id.as_str())
        .fetch_all(&mut *transaction)
        .await?;
        for row in &rows {
            let request_id = UserInputRequestId::new(row.get::<String, _>("id"));
            let session_id = SessionId::new(row.get::<String, _>("session_id"));
            let item_id: String = row.get("item_id");
            sqlx::query(
                "UPDATE user_input_requests SET status = 'cancelled', resolved_at_ms = ?, resolved_by = ? WHERE id = ? AND status = 'pending'",
            )
            .bind(resolved_at_ms)
            .bind(resolved_by)
            .bind(request_id.as_str())
            .execute(&mut *transaction)
            .await?;
            sqlx::query("UPDATE transcript_items SET status = ? WHERE id = ?")
                .bind(ItemStatus::Interrupted.as_str())
                .bind(item_id)
                .execute(&mut *transaction)
                .await?;
            append_event_tx(
                &mut transaction,
                &session_id,
                Some(run_id),
                "user_input.cancelled",
                json!({ "requestId": request_id }),
                resolved_at_ms,
            )
            .await?;
        }
        transaction.commit().await?;
        Ok(u64::try_from(rows.len()).unwrap_or(u64::MAX))
    }

    pub async fn expire_user_input(
        &self,
        request_id: &UserInputRequestId,
        resolved_at_ms: i64,
    ) -> Result<UserInputRequestRecord, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query("SELECT * FROM user_input_requests WHERE id = ?")
            .bind(request_id.as_str())
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| AgentStoreError::UserInputNotFound(request_id.clone()))?;
        let mut request = user_input_from_row(&row)?;
        if request.status != UserInputStatus::Pending {
            return Err(AgentStoreError::UserInputNotPending(request.id));
        }
        sqlx::query(
            "UPDATE user_input_requests SET status = 'expired', resolved_at_ms = ?, resolved_by = 'system:expired' WHERE id = ? AND status = 'pending'",
        )
        .bind(resolved_at_ms)
        .bind(request.id.as_str())
        .execute(&mut *transaction)
        .await?;
        sqlx::query("UPDATE transcript_items SET status = 'interrupted' WHERE id = ?")
            .bind(request.item_id.as_str())
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "UPDATE runs SET status = 'running', updated_at_ms = ? WHERE id = ? AND status = 'waiting_user_input'",
        )
        .bind(resolved_at_ms)
        .bind(request.run_id.as_str())
        .execute(&mut *transaction)
        .await?;
        append_event_tx(
            &mut transaction,
            &request.session_id,
            Some(&request.run_id),
            "user_input.expired",
            json!({ "requestId": request.id, "itemId": request.item_id }),
            resolved_at_ms,
        )
        .await?;
        transaction.commit().await?;
        request.status = UserInputStatus::Expired;
        request.resolved_at_ms = Some(resolved_at_ms);
        request.resolved_by = Some("system:expired".into());
        Ok(request)
    }
}

pub(super) fn user_input_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<UserInputRequestRecord, AgentStoreError> {
    let status_value: String = row.get("status");
    let status = UserInputStatus::parse(&status_value).ok_or_else(|| {
        AgentStoreError::InvalidPersistedValue {
            kind: "user input status",
            value: status_value,
        }
    })?;
    Ok(UserInputRequestRecord {
        id: UserInputRequestId::new(row.get::<String, _>("id")),
        session_id: SessionId::new(row.get::<String, _>("session_id")),
        run_id: RunId::new(row.get::<String, _>("run_id")),
        run_generation: u64::try_from(row.get::<i64, _>("run_generation")).unwrap_or_default(),
        item_id: hachimi_protocol::ItemId::new(row.get::<String, _>("item_id")),
        questions: serde_json::from_str(row.get("questions_json"))?,
        status,
        expires_at_ms: row.get("expires_at_ms"),
        created_at_ms: row.get("created_at_ms"),
        resolved_at_ms: row.get("resolved_at_ms"),
        resolved_by: row.get("resolved_by"),
    })
}

fn validate_questions(request: &UserInputRequestRecord) -> Result<(), AgentStoreError> {
    if request.questions.is_empty() || request.questions.len() > 3 {
        return Err(AgentStoreError::InvalidUserInputAnswer(
            "question count must be between one and three",
        ));
    }
    let mut ids = BTreeSet::new();
    for question in &request.questions {
        if question.id.trim().is_empty()
            || question.id.len() > 64
            || !ids.insert(question.id.clone())
        {
            return Err(AgentStoreError::InvalidUserInputAnswer(
                "question IDs must be unique and bounded",
            ));
        }
        if question.header.trim().is_empty()
            || question.header.chars().count() > 64
            || question.prompt.trim().is_empty()
            || question.prompt.chars().count() > 4_096
        {
            return Err(AgentStoreError::InvalidUserInputAnswer(
                "question text is invalid",
            ));
        }
        if !question.options.is_empty() && !(2..=3).contains(&question.options.len()) {
            return Err(AgentStoreError::InvalidUserInputAnswer(
                "choice questions require two or three options",
            ));
        }
        if question.secret && (!question.options.is_empty() || question.default_answer.is_some()) {
            return Err(AgentStoreError::InvalidUserInputAnswer(
                "secret questions cannot contain options or defaults",
            ));
        }
        if question.auto_resolution_ms.is_some_and(|value| value == 0) {
            return Err(AgentStoreError::InvalidUserInputAnswer(
                "auto-resolution timeout must be positive",
            ));
        }
    }
    Ok(())
}

fn validate_answers(
    request: &UserInputRequestRecord,
    answers: &[UserInputAnswer],
) -> Result<(), AgentStoreError> {
    if answers.len() != request.questions.len() {
        return Err(AgentStoreError::InvalidUserInputAnswer(
            "all questions must be answered exactly once",
        ));
    }
    let mut by_id = BTreeMap::new();
    for answer in answers {
        if answer.value.chars().count() > 32_000
            || by_id
                .insert(answer.question_id.as_str(), answer.value.as_str())
                .is_some()
        {
            return Err(AgentStoreError::InvalidUserInputAnswer(
                "answer is duplicated or too large",
            ));
        }
    }
    for question in &request.questions {
        let answer =
            by_id
                .get(question.id.as_str())
                .ok_or(AgentStoreError::InvalidUserInputAnswer(
                    "answer question ID is unknown",
                ))?;
        if !question.options.is_empty()
            && !question
                .options
                .iter()
                .any(|option| option.value == *answer)
        {
            return Err(AgentStoreError::InvalidUserInputAnswer(
                "choice answer is not one of the declared options",
            ));
        }
    }
    Ok(())
}
