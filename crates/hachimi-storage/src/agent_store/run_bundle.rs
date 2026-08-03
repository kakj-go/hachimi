use hachimi_protocol::{
    AttachmentId, RunEventPayload, RunId, RunRecord, SessionId, SessionRecord, TranscriptItem,
};
use serde_json::json;

use super::{
    AgentStore, AgentStoreError, append_event_tx, append_event_typed_tx, enum_to_db, get_run_tx,
    next_sequence_tx, session_context_kind, session_from_row, transcript_item_from_row,
    transcript_kind_db,
};

#[derive(Debug, Clone, PartialEq)]
pub struct CreatedAgentRun {
    pub session: SessionRecord,
    pub run: RunRecord,
    pub user_item: TranscriptItem,
}

impl AgentStore {
    /// Atomically creates a fresh Run and user Item in an existing Session.
    /// Used by scheduled/thread continuations; it never copies old grants or pending Items.
    pub async fn create_agent_run_in_session_idempotent(
        &self,
        principal: &str,
        idempotency_key: &str,
        session: &SessionRecord,
        run: &RunRecord,
        user_item: &TranscriptItem,
        attachment_ids: &[AttachmentId],
    ) -> Result<CreatedAgentRun, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        if let Some(existing_id) = sqlx::query_scalar::<_, String>(
            "SELECT resource_id FROM idempotency_records WHERE principal = ? AND method = 'run.start' AND idempotency_key = ?",
        )
        .bind(principal)
        .bind(idempotency_key)
        .fetch_optional(&mut *transaction)
        .await?
        {
            let existing_run = get_run_tx(&mut transaction, &RunId::new(existing_id))
                .await?
                .ok_or_else(|| AgentStoreError::RunNotFound(run.id.clone()))?;
            let item_row = sqlx::query(
                "SELECT * FROM transcript_items WHERE run_id = ? AND kind = 'user' ORDER BY sequence ASC LIMIT 1",
            )
            .bind(existing_run.id.as_str())
            .fetch_one(&mut *transaction)
            .await?;
            let existing_item = transcript_item_from_row(&item_row, &session.id)?;
            transaction.commit().await?;
            return Ok(CreatedAgentRun {
                session: session.clone(),
                run: existing_run,
                user_item: existing_item,
            });
        }
        let persisted_session = sqlx::query("SELECT * FROM sessions WHERE id = ?")
            .bind(session.id.as_str())
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| AgentStoreError::SessionNotFound(session.id.clone()))?;
        let persisted_session = session_from_row(&persisted_session)?;
        if persisted_session != *session
            || run.session_id != session.id
            || user_item.session_id != session.id
            || user_item.run_id.as_ref() != Some(&run.id)
        {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "agent continuation bundle",
                value: "persisted Session, Run and User Item lineage do not match".into(),
            });
        }
        let active_run_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM runs WHERE session_id = ? AND status NOT IN ('succeeded', 'failed', 'timed_out', 'cancelled', 'interrupted', 'lost')",
        )
        .bind(session.id.as_str())
        .fetch_one(&mut *transaction)
        .await?;
        if active_run_count != 0 {
            return Err(AgentStoreError::RunPreconditionFailed);
        }
        for attachment_id in attachment_ids {
            let exists =
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM attachments WHERE id = ?")
                    .bind(attachment_id.as_str())
                    .fetch_one(&mut *transaction)
                    .await?
                    > 0;
            if !exists {
                return Err(AgentStoreError::AttachmentNotFound(attachment_id.clone()));
            }
        }
        sqlx::query(
            "INSERT INTO runs (id, session_id, status, purpose, origin_json, generation, configuration_json, requested_capabilities_json, negotiated_capabilities_json, provider_capability_probe_json, capability_degradations_json, failure_code, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(run.id.as_str())
        .bind(run.session_id.as_str())
        .bind(run.status.as_str())
        .bind(enum_to_db(&run.purpose)?)
        .bind(serde_json::to_string(&run.origin)?)
        .bind(i64::try_from(run.generation).unwrap_or(i64::MAX))
        .bind(serde_json::to_string(&run.configuration)?)
        .bind(serde_json::to_string(&run.requested_capabilities)?)
        .bind(serde_json::to_string(&run.negotiated_capabilities)?)
        .bind(serde_json::to_string(&run.provider_capability_probe)?)
        .bind(serde_json::to_string(&run.capability_degradations)?)
        .bind(&run.failure_code)
        .bind(run.created_at_ms)
        .bind(run.updated_at_ms)
        .execute(&mut *transaction)
        .await?;
        append_event_tx(
            &mut transaction,
            &session.id,
            Some(&run.id),
            "run.queued",
            json!({ "status": run.status, "origin": run.origin, "continuation": true }),
            run.created_at_ms,
        )
        .await?;
        for attachment_id in attachment_ids {
            sqlx::query("INSERT INTO run_attachments (run_id, attachment_id) VALUES (?, ?)")
                .bind(run.id.as_str())
                .bind(attachment_id.as_str())
                .execute(&mut *transaction)
                .await?;
        }
        let mut persisted_item = user_item.clone();
        persisted_item.sequence =
            next_sequence_tx(&mut transaction, &session.id, user_item.created_at_ms).await?;
        sqlx::query(
            "INSERT INTO transcript_items (id, session_id, run_id, sequence, kind, status, payload_json, relations_json, created_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(persisted_item.id.as_str())
        .bind(session.id.as_str())
        .bind(run.id.as_str())
        .bind(i64::try_from(persisted_item.sequence).unwrap_or(i64::MAX))
        .bind(transcript_kind_db(persisted_item.kind))
        .bind(persisted_item.status.as_str())
        .bind(serde_json::to_string(&persisted_item.payload)?)
        .bind(serde_json::to_string(&persisted_item.relations)?)
        .bind(persisted_item.created_at_ms)
        .execute(&mut *transaction)
        .await?;
        append_event_typed_tx(
            &mut transaction,
            &session.id,
            Some(&run.id),
            "item.completed",
            Some(RunEventPayload::ItemCompleted {
                item: Box::new(persisted_item.clone()),
            }),
            json!({ "itemId": persisted_item.id, "status": persisted_item.status }),
            persisted_item.created_at_ms,
        )
        .await?;
        sqlx::query(
            "INSERT INTO idempotency_records (principal, method, idempotency_key, resource_id, response_json, created_at_ms) VALUES (?, 'run.start', ?, ?, '{}', ?)",
        )
        .bind(principal)
        .bind(idempotency_key)
        .bind(run.id.as_str())
        .bind(run.created_at_ms)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("UPDATE sessions SET updated_at_ms = ? WHERE id = ?")
            .bind(run.created_at_ms)
            .bind(session.id.as_str())
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(CreatedAgentRun {
            session: session.clone(),
            run: run.clone(),
            user_item: persisted_item,
        })
    }

    pub async fn create_agent_run_bundle_idempotent(
        &self,
        principal: &str,
        idempotency_key: &str,
        session: &SessionRecord,
        run: &RunRecord,
        user_item: &TranscriptItem,
        attachment_ids: &[AttachmentId],
    ) -> Result<CreatedAgentRun, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        if let Some(existing_id) = sqlx::query_scalar::<_, String>(
            "SELECT resource_id FROM idempotency_records WHERE principal = ? AND method = 'run.start' AND idempotency_key = ?",
        )
        .bind(principal)
        .bind(idempotency_key)
        .fetch_optional(&mut *transaction)
        .await?
        {
            let existing_run = get_run_tx(&mut transaction, &RunId::new(existing_id))
                .await?
                .ok_or_else(|| AgentStoreError::RunNotFound(run.id.clone()))?;
            let session_row = sqlx::query("SELECT * FROM sessions WHERE id = ?")
                .bind(existing_run.session_id.as_str())
                .fetch_one(&mut *transaction)
                .await?;
            let existing_session = session_from_row(&session_row)?;
            let item_row = sqlx::query(
                "SELECT * FROM transcript_items WHERE run_id = ? AND kind = 'user' ORDER BY sequence ASC LIMIT 1",
            )
            .bind(existing_run.id.as_str())
            .fetch_one(&mut *transaction)
            .await?;
            let existing_item = transcript_item_from_row(&item_row, &existing_session.id)?;
            transaction.commit().await?;
            return Ok(CreatedAgentRun {
                session: existing_session,
                run: existing_run,
                user_item: existing_item,
            });
        }
        if run.session_id != session.id
            || user_item.session_id != session.id
            || user_item.run_id.as_ref() != Some(&run.id)
        {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "agent run bundle",
                value: "Session, Run and User Item lineage do not match".into(),
            });
        }
        for attachment_id in attachment_ids {
            let exists =
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM attachments WHERE id = ?")
                    .bind(attachment_id.as_str())
                    .fetch_one(&mut *transaction)
                    .await?
                    > 0;
            if !exists {
                return Err(AgentStoreError::AttachmentNotFound(attachment_id.clone()));
            }
        }

        sqlx::query(
            "INSERT INTO sessions (id, context_kind, context_json, entry_profile, title, archived, pinned, parent_session_id, source_run_id, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(session.id.as_str())
        .bind(session_context_kind(&session.context))
        .bind(serde_json::to_string(&session.context)?)
        .bind(enum_to_db(&session.entry_profile)?)
        .bind(&session.title)
        .bind(session.archived)
        .bind(session.pinned)
        .bind(session.parent_session_id.as_ref().map(SessionId::as_str))
        .bind(session.source_run_id.as_ref().map(RunId::as_str))
        .bind(session.created_at_ms)
        .bind(session.updated_at_ms)
        .execute(&mut *transaction)
        .await?;
        append_event_tx(
            &mut transaction,
            &session.id,
            None,
            "session.created",
            json!({ "sessionId": session.id }),
            session.created_at_ms,
        )
        .await?;

        sqlx::query(
            "INSERT INTO runs (id, session_id, status, purpose, origin_json, generation, configuration_json, requested_capabilities_json, negotiated_capabilities_json, provider_capability_probe_json, capability_degradations_json, failure_code, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(run.id.as_str())
        .bind(run.session_id.as_str())
        .bind(run.status.as_str())
        .bind(enum_to_db(&run.purpose)?)
        .bind(serde_json::to_string(&run.origin)?)
        .bind(i64::try_from(run.generation).unwrap_or(i64::MAX))
        .bind(serde_json::to_string(&run.configuration)?)
        .bind(serde_json::to_string(&run.requested_capabilities)?)
        .bind(serde_json::to_string(&run.negotiated_capabilities)?)
        .bind(serde_json::to_string(&run.provider_capability_probe)?)
        .bind(serde_json::to_string(&run.capability_degradations)?)
        .bind(&run.failure_code)
        .bind(run.created_at_ms)
        .bind(run.updated_at_ms)
        .execute(&mut *transaction)
        .await?;
        append_event_tx(
            &mut transaction,
            &session.id,
            Some(&run.id),
            "run.queued",
            json!({ "status": run.status, "origin": run.origin }),
            run.created_at_ms,
        )
        .await?;

        for attachment_id in attachment_ids {
            sqlx::query("INSERT INTO run_attachments (run_id, attachment_id) VALUES (?, ?)")
                .bind(run.id.as_str())
                .bind(attachment_id.as_str())
                .execute(&mut *transaction)
                .await?;
        }

        let mut persisted_item = user_item.clone();
        persisted_item.sequence =
            next_sequence_tx(&mut transaction, &session.id, user_item.created_at_ms).await?;
        sqlx::query(
            "INSERT INTO transcript_items (id, session_id, run_id, sequence, kind, status, payload_json, relations_json, created_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(persisted_item.id.as_str())
        .bind(session.id.as_str())
        .bind(run.id.as_str())
        .bind(i64::try_from(persisted_item.sequence).unwrap_or(i64::MAX))
        .bind(transcript_kind_db(persisted_item.kind))
        .bind(persisted_item.status.as_str())
        .bind(serde_json::to_string(&persisted_item.payload)?)
        .bind(serde_json::to_string(&persisted_item.relations)?)
        .bind(persisted_item.created_at_ms)
        .execute(&mut *transaction)
        .await?;
        let completed_payload = RunEventPayload::ItemCompleted {
            item: Box::new(persisted_item.clone()),
        };
        append_event_typed_tx(
            &mut transaction,
            &session.id,
            Some(&run.id),
            "item.completed",
            Some(completed_payload),
            json!({
                "itemId": persisted_item.id,
                "status": persisted_item.status
            }),
            persisted_item.created_at_ms,
        )
        .await?;

        sqlx::query(
            "INSERT INTO idempotency_records (principal, method, idempotency_key, resource_id, response_json, created_at_ms) VALUES (?, 'run.start', ?, ?, '{}', ?)",
        )
        .bind(principal)
        .bind(idempotency_key)
        .bind(run.id.as_str())
        .bind(run.created_at_ms)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(CreatedAgentRun {
            session: session.clone(),
            run: run.clone(),
            user_item: persisted_item,
        })
    }
}
