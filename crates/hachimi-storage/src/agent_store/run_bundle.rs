use hachimi_protocol::{
    AttachmentId, ChannelEventKey, RunEventPayload, RunId, RunRecord, SessionId, SessionRecord,
    TranscriptItem,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelRunBindingInput {
    pub event_key: ChannelEventKey,
    pub binding_key_hash: String,
    pub binding_key_json: String,
    pub account_id: String,
    pub authorization_id: Option<String>,
    pub authorization_revision: u64,
    pub identity_group_id: Option<String>,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, Copy)]
pub struct ChannelAgentRunCreateInput<'a> {
    pub principal: &'a str,
    pub idempotency_key: &'a str,
    pub proposed_session: &'a SessionRecord,
    pub proposed_run: &'a RunRecord,
    pub proposed_user_item: &'a TranscriptItem,
    pub attachment_ids: &'a [AttachmentId],
    pub binding: &'a ChannelRunBindingInput,
}

impl AgentStore {
    /// Creates the Channel Session/Run/User Item, updates its deterministic
    /// binding, and records the ingress Run in one SQLite transaction.
    pub async fn create_channel_agent_run_idempotent(
        &self,
        input: ChannelAgentRunCreateInput<'_>,
    ) -> Result<CreatedAgentRun, AgentStoreError> {
        let ChannelAgentRunCreateInput {
            principal,
            idempotency_key,
            proposed_session,
            proposed_run,
            proposed_user_item,
            attachment_ids,
            binding,
        } = input;
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
                .ok_or_else(|| AgentStoreError::RunNotFound(proposed_run.id.clone()))?;
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
            let updated = sqlx::query("UPDATE channel_ingress SET status = 'run_created', session_id = ?, run_id = ?, result_code = 'run_created', updated_at_ms = ? WHERE provider_id = ? AND account_id = ? AND external_message_id = ? AND status IN ('claimed', 'run_created') AND (run_id IS NULL OR run_id = ?)")
                .bind(existing_session.id.as_str())
                .bind(existing_run.id.as_str())
                .bind(binding.timestamp_ms)
                .bind(&binding.event_key.provider_id)
                .bind(&binding.event_key.account_id)
                .bind(&binding.event_key.external_message_id)
                .bind(existing_run.id.as_str())
                .execute(&mut *transaction)
                .await?;
            if updated.rows_affected() != 1 {
                return Err(AgentStoreError::InvalidPersistedValue {
                    kind: "channel ingress",
                    value: "idempotent Run does not match the claimed ingress".into(),
                });
            }
            transaction.commit().await?;
            return Ok(CreatedAgentRun {
                session: existing_session,
                run: existing_run,
                user_item: existing_item,
            });
        }
        if proposed_run.session_id != proposed_session.id
            || proposed_user_item.session_id != proposed_session.id
            || proposed_user_item.run_id.as_ref() != Some(&proposed_run.id)
            || binding.binding_key_hash.trim().is_empty()
            || binding.binding_key_json.trim().is_empty()
            || binding.account_id != binding.event_key.account_id
            || binding.authorization_revision == 0
        {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "channel run bundle",
                value: "Session, Run, binding, ingress, and User Item lineage do not match".into(),
            });
        }

        let bound_session = sqlx::query("SELECT session.* FROM channel_session_bindings AS binding INNER JOIN sessions AS session ON session.id = binding.session_id WHERE binding.binding_key_hash = ?")
            .bind(&binding.binding_key_hash)
            .fetch_optional(&mut *transaction)
            .await?
            .map(|row| session_from_row(&row))
            .transpose()?;
        let (session, create_session) = match bound_session {
            Some(session) if !session.archived => (session, false),
            Some(_) => {
                sqlx::query("DELETE FROM channel_session_bindings WHERE binding_key_hash = ?")
                    .bind(&binding.binding_key_hash)
                    .execute(&mut *transaction)
                    .await?;
                (proposed_session.clone(), true)
            }
            None => (proposed_session.clone(), true),
        };
        if session.context != proposed_session.context
            || session.entry_profile != proposed_session.entry_profile
        {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "channel Session binding",
                value: "bound Session is incompatible with the Channel Run".into(),
            });
        }
        if !create_session {
            let active_run_count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM runs WHERE session_id = ? AND status NOT IN ('succeeded', 'failed', 'timed_out', 'cancelled', 'interrupted', 'lost')",
            )
            .bind(session.id.as_str())
            .fetch_one(&mut *transaction)
            .await?;
            if active_run_count != 0 {
                return Err(AgentStoreError::RunPreconditionFailed);
            }
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
        if create_session {
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
                json!({ "sessionId": session.id, "origin": "channel" }),
                session.created_at_ms,
            )
            .await?;
        }

        let mut run = proposed_run.clone();
        run.session_id = session.id.clone();
        sqlx::query(
            "INSERT INTO runs (id, session_id, status, purpose, origin_json, generation, configuration_json, requested_capabilities_json, negotiated_capabilities_json, provider_capability_probe_json, capability_degradations_json, failure_code, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(run.id.as_str())
        .bind(session.id.as_str())
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
            json!({ "status": run.status, "origin": run.origin, "channel": true }),
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
        let mut user_item = proposed_user_item.clone();
        user_item.session_id = session.id.clone();
        user_item.run_id = Some(run.id.clone());
        user_item.sequence =
            next_sequence_tx(&mut transaction, &session.id, user_item.created_at_ms).await?;
        sqlx::query(
            "INSERT INTO transcript_items (id, session_id, run_id, sequence, kind, status, payload_json, relations_json, created_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(user_item.id.as_str())
        .bind(session.id.as_str())
        .bind(run.id.as_str())
        .bind(i64::try_from(user_item.sequence).unwrap_or(i64::MAX))
        .bind(transcript_kind_db(user_item.kind))
        .bind(user_item.status.as_str())
        .bind(serde_json::to_string(&user_item.payload)?)
        .bind(serde_json::to_string(&user_item.relations)?)
        .bind(user_item.created_at_ms)
        .execute(&mut *transaction)
        .await?;
        append_event_typed_tx(
            &mut transaction,
            &session.id,
            Some(&run.id),
            "item.completed",
            Some(RunEventPayload::ItemCompleted {
                item: Box::new(user_item.clone()),
            }),
            json!({ "itemId": user_item.id, "status": user_item.status }),
            user_item.created_at_ms,
        )
        .await?;
        sqlx::query("INSERT INTO channel_session_bindings(binding_key_hash, binding_key_json, account_id, authorization_id, authorization_revision, identity_group_id, session_id, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(binding_key_hash) DO UPDATE SET binding_key_json = excluded.binding_key_json, account_id = excluded.account_id, authorization_id = excluded.authorization_id, authorization_revision = excluded.authorization_revision, identity_group_id = excluded.identity_group_id, session_id = excluded.session_id, updated_at_ms = excluded.updated_at_ms")
            .bind(&binding.binding_key_hash)
            .bind(&binding.binding_key_json)
            .bind(&binding.account_id)
            .bind(&binding.authorization_id)
            .bind(i64::try_from(binding.authorization_revision).unwrap_or(i64::MAX))
            .bind(&binding.identity_group_id)
            .bind(session.id.as_str())
            .bind(binding.timestamp_ms)
            .bind(binding.timestamp_ms)
            .execute(&mut *transaction)
            .await?;
        let ingress = sqlx::query("UPDATE channel_ingress SET status = 'run_created', session_id = ?, run_id = ?, result_code = 'run_created', updated_at_ms = ? WHERE provider_id = ? AND account_id = ? AND external_message_id = ? AND status = 'claimed' AND run_id IS NULL")
            .bind(session.id.as_str())
            .bind(run.id.as_str())
            .bind(binding.timestamp_ms)
            .bind(&binding.event_key.provider_id)
            .bind(&binding.event_key.account_id)
            .bind(&binding.event_key.external_message_id)
            .execute(&mut *transaction)
            .await?;
        if ingress.rows_affected() != 1 {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "channel ingress",
                value: "Run creation requires one claimed ingress".into(),
            });
        }
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
            session,
            run,
            user_item,
        })
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use hachimi_protocol::{
        ApprovalPolicy, BehaviorMode, ChannelMessageId, EntryProfile, ItemId, ItemPayload,
        ItemRelations, ItemStatus, LlmSettings, PermissionProfile, ProviderCapabilities, RunBudget,
        RunConfiguration, RunDriverKind, RunOrigin, RunPurpose, RunStatus, SessionContextBinding,
        TranscriptItemKind, WorkloadKind,
    };

    async fn seed_account(store: &AgentStore) {
        sqlx::query("INSERT INTO integration_provider_accounts(id, provider_id, display_name, tenant_key, tenant_identity_hash, transport, state, diagnostic, connector_account_id, credential_ref, credential_fingerprint, api_access_enabled, messaging_enabled, config_json, credential_revision, config_revision, last_event_at_ms, last_delivery_at_ms, next_reconnect_at_ms, consecutive_failures, created_at_ms, updated_at_ms) VALUES('account-1', 'dingtalk', 'DingTalk', 'tenant-1', 'tenant-hash', 'stream', 'healthy', NULL, NULL, NULL, NULL, 0, 1, '{}', 1, 1, NULL, NULL, NULL, 0, 1, 1)")
            .execute(store.pool())
            .await
            .expect("account");
    }

    async fn seed_ingress(store: &AgentStore, message_id: &str) {
        sqlx::query("INSERT INTO channel_ingress(provider_id, account_id, external_message_id, address_json, actor_id, payload_hash, normalized_payload_json, status, claim_token, claim_expires_at_ms, session_id, run_id, authorization_id, authorization_revision, grant_snapshot_json, result_code, provider_receipt, received_at_ms, updated_at_ms) VALUES('dingtalk', 'account-1', ?, '{}', 'user-1', 'hash', '{}', 'claimed', 'claim', 60001, NULL, NULL, NULL, NULL, '{}', 'claimed', NULL, 1, 1)")
            .bind(message_id)
            .execute(store.pool())
            .await
            .expect("ingress");
    }

    fn proposed(suffix: &str) -> (SessionRecord, RunRecord, TranscriptItem) {
        let session = SessionRecord {
            id: SessionId::new(format!("session-{suffix}")),
            context: SessionContextBinding::General,
            entry_profile: EntryProfile::Workbench,
            title: "Channel".into(),
            archived: false,
            pinned: false,
            parent_session_id: None,
            source_run_id: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        let run = RunRecord {
            id: RunId::new(format!("run-{suffix}")),
            session_id: session.id.clone(),
            status: RunStatus::Queued,
            purpose: RunPurpose::Task,
            origin: RunOrigin::Channel {
                channel: "dingtalk".into(),
                account: "account-1".into(),
                peer: "user-1".into(),
                thread: String::new(),
                message_id: ChannelMessageId::new(format!("message-{suffix}")),
            },
            generation: 1,
            configuration: RunConfiguration {
                model_snapshot: LlmSettings::default(),
                driver: RunDriverKind::ToolLoop,
                entry_profile: EntryProfile::Workbench,
                workload_override: Some(WorkloadKind::General),
                behavior_mode: BehaviorMode::Default,
                execution_target: None,
                approval_policy: ApprovalPolicy::NeverPrompt,
                permission_profile: PermissionProfile::ReadOnly,
                budget: RunBudget::default(),
                accepted_plan_id: None,
                accepted_plan_revision: None,
            },
            requested_capabilities: ProviderCapabilities::default(),
            negotiated_capabilities: ProviderCapabilities::default(),
            provider_capability_probe: None,
            capability_degradations: Vec::new(),
            failure_code: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        let item = TranscriptItem {
            id: ItemId::new(format!("item-{suffix}")),
            session_id: session.id.clone(),
            run_id: Some(run.id.clone()),
            sequence: 0,
            kind: TranscriptItemKind::User,
            status: ItemStatus::Completed,
            payload: ItemPayload::User {
                text: "hello".into(),
                attachment_ids: Vec::new(),
            },
            relations: ItemRelations::default(),
            created_at_ms: 1,
        };
        (session, run, item)
    }

    fn binding(message_id: &str) -> ChannelRunBindingInput {
        ChannelRunBindingInput {
            event_key: ChannelEventKey {
                provider_id: "dingtalk".into(),
                account_id: "account-1".into(),
                external_message_id: message_id.into(),
            },
            binding_key_hash: "stable-binding".into(),
            binding_key_json: r#"{"scope":"provider_dm"}"#.into(),
            account_id: "account-1".into(),
            authorization_id: None,
            authorization_revision: 1,
            identity_group_id: None,
            timestamp_ms: 2,
        }
    }

    async fn complete_run(store: &AgentStore, run_id: &RunId) {
        store
            .transition_run(run_id, RunStatus::Preparing, None)
            .await
            .expect("preparing");
        store
            .transition_run(run_id, RunStatus::Running, None)
            .await
            .expect("running");
        store
            .transition_run(run_id, RunStatus::Succeeded, None)
            .await
            .expect("succeeded");
    }

    #[tokio::test]
    async fn channel_run_binding_and_ingress_commit_atomically() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        seed_account(&store).await;
        seed_ingress(&store, "message-1").await;
        let (session, run, item) = proposed("1");
        let created = store
            .create_channel_agent_run_idempotent(ChannelAgentRunCreateInput {
                principal: "channel",
                idempotency_key: "event-1",
                proposed_session: &session,
                proposed_run: &run,
                proposed_user_item: &item,
                attachment_ids: &[],
                binding: &binding("message-1"),
            })
            .await
            .expect("created");
        assert_eq!(created.session.id, session.id);
        let ingress: (String, String, String) = sqlx::query_as(
            "SELECT status, session_id, run_id FROM channel_ingress WHERE external_message_id = 'message-1'",
        )
        .fetch_one(store.pool())
        .await
        .expect("ingress");
        assert_eq!(
            ingress,
            (
                "run_created".into(),
                session.id.to_string(),
                run.id.to_string()
            )
        );
        let binding_session: String = sqlx::query_scalar(
            "SELECT session_id FROM channel_session_bindings WHERE binding_key_hash = 'stable-binding'",
        )
        .fetch_one(store.pool())
        .await
        .expect("binding");
        assert_eq!(binding_session, session.id.as_str());

        let (duplicate_session, duplicate_run, duplicate_item) = proposed("duplicate");
        let duplicate = store
            .create_channel_agent_run_idempotent(ChannelAgentRunCreateInput {
                principal: "channel",
                idempotency_key: "event-1",
                proposed_session: &duplicate_session,
                proposed_run: &duplicate_run,
                proposed_user_item: &duplicate_item,
                attachment_ids: &[],
                binding: &binding("message-1"),
            })
            .await
            .expect("idempotent");
        assert_eq!(duplicate.run.id, run.id);
    }

    #[tokio::test]
    async fn channel_binding_reuses_active_session_and_replaces_archived_session() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        seed_account(&store).await;
        seed_ingress(&store, "message-1").await;
        let (first_session, first_run, first_item) = proposed("1");
        let first = store
            .create_channel_agent_run_idempotent(ChannelAgentRunCreateInput {
                principal: "channel",
                idempotency_key: "event-1",
                proposed_session: &first_session,
                proposed_run: &first_run,
                proposed_user_item: &first_item,
                attachment_ids: &[],
                binding: &binding("message-1"),
            })
            .await
            .expect("first");
        complete_run(&store, &first.run.id).await;
        seed_ingress(&store, "message-2").await;
        let (second_session, second_run, second_item) = proposed("2");
        let second = store
            .create_channel_agent_run_idempotent(ChannelAgentRunCreateInput {
                principal: "channel",
                idempotency_key: "event-2",
                proposed_session: &second_session,
                proposed_run: &second_run,
                proposed_user_item: &second_item,
                attachment_ids: &[],
                binding: &binding("message-2"),
            })
            .await
            .expect("second");
        assert_eq!(second.session.id, first.session.id);
        assert!(
            store
                .get_session(&second_session.id)
                .await
                .expect("lookup")
                .is_none()
        );

        complete_run(&store, &second.run.id).await;
        store
            .update_session_metadata(&first.session.id, None, Some(true), None, 3)
            .await
            .expect("archive");
        seed_ingress(&store, "message-3").await;
        let (third_session, third_run, third_item) = proposed("3");
        let third = store
            .create_channel_agent_run_idempotent(ChannelAgentRunCreateInput {
                principal: "channel",
                idempotency_key: "event-3",
                proposed_session: &third_session,
                proposed_run: &third_run,
                proposed_user_item: &third_item,
                attachment_ids: &[],
                binding: &binding("message-3"),
            })
            .await
            .expect("third");
        assert_eq!(third.session.id, third_session.id);
        assert_ne!(third.session.id, first.session.id);
    }

    #[tokio::test]
    async fn missing_claimed_ingress_rolls_back_entire_channel_bundle() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        seed_account(&store).await;
        let (session, run, item) = proposed("rollback");
        assert!(
            store
                .create_channel_agent_run_idempotent(ChannelAgentRunCreateInput {
                    principal: "channel",
                    idempotency_key: "event-rollback",
                    proposed_session: &session,
                    proposed_run: &run,
                    proposed_user_item: &item,
                    attachment_ids: &[],
                    binding: &binding("missing"),
                })
                .await
                .is_err()
        );
        let sessions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
            .fetch_one(store.pool())
            .await
            .expect("sessions");
        let runs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runs")
            .fetch_one(store.pool())
            .await
            .expect("runs");
        let bindings: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channel_session_bindings")
            .fetch_one(store.pool())
            .await
            .expect("bindings");
        assert_eq!((sessions, runs, bindings), (0, 0, 0));
    }
}
