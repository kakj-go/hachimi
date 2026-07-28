use hachimi_protocol::{
    CapabilityDegradation, ItemPayload, ItemRelations, ProviderCapabilities,
    ProviderCapabilityProbe, RunId, RunStatus, RunSteerRecord, RunSteerStatus, SessionCursor,
    SessionForkRequest, SessionId, SessionPage, SessionRecord, SessionResumeRequest,
    SessionResumeSnapshot, SessionSearchRequest,
};
use serde_json::json;
use sqlx::Row;

use super::usage::usage_from_row;
use super::user_input::user_input_from_row;
use super::{
    AgentStore, AgentStoreError, append_event_tx, approval_from_row, enum_to_db, get_run_tx,
    run_from_row, session_from_row, transcript_item_from_row,
};

impl AgentStore {
    pub async fn update_run_capabilities(
        &self,
        run_snapshot: &hachimi_protocol::RunRecord,
        negotiated: ProviderCapabilities,
        probe: Option<&ProviderCapabilityProbe>,
        degradations: &[CapabilityDegradation],
        updated_at_ms: i64,
    ) -> Result<hachimi_protocol::RunRecord, AgentStoreError> {
        let run_id = &run_snapshot.id;
        let expected_generation = run_snapshot.generation;
        let requested = run_snapshot.requested_capabilities;
        let mut transaction = self.pool.begin().await?;
        let run = get_run_tx(&mut transaction, run_id)
            .await?
            .ok_or_else(|| AgentStoreError::RunNotFound(run_id.clone()))?;
        if run.generation != expected_generation || run.status.is_terminal() {
            return Err(AgentStoreError::RunPreconditionFailed);
        }
        sqlx::query(
            "UPDATE runs SET requested_capabilities_json = ?, negotiated_capabilities_json = ?, provider_capability_probe_json = ?, capability_degradations_json = ?, updated_at_ms = ? WHERE id = ? AND generation = ?",
        )
        .bind(serde_json::to_string(&requested)?)
        .bind(serde_json::to_string(&negotiated)?)
        .bind(serde_json::to_string(&probe)?)
        .bind(serde_json::to_string(degradations)?)
        .bind(updated_at_ms)
        .bind(run_id.as_str())
        .bind(i64::try_from(expected_generation).unwrap_or(i64::MAX))
        .execute(&mut *transaction)
        .await?;
        append_event_tx(
            &mut transaction,
            &run.session_id,
            Some(run_id),
            "provider.capabilities_negotiated",
            json!({
                "negotiated": negotiated,
                "probe": probe,
                "degradations": degradations,
            }),
            updated_at_ms,
        )
        .await?;
        let updated = get_run_tx(&mut transaction, run_id)
            .await?
            .ok_or_else(|| AgentStoreError::RunNotFound(run_id.clone()))?;
        transaction.commit().await?;
        Ok(updated)
    }

    pub async fn search_sessions(
        &self,
        request: &SessionSearchRequest,
    ) -> Result<SessionPage, AgentStoreError> {
        let query = request
            .query
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| format!("%{value}%"));
        let project_id = request.project_id.as_ref().map(|id| id.as_str());
        let archived = request.archived.map(i64::from);
        let before_time = request.before.as_ref().map(|cursor| cursor.updated_at_ms);
        let before_id = request.before.as_ref().map(|cursor| cursor.id.as_str());
        let limit = request.limit.clamp(1, 200);
        let rows = sqlx::query(
            "SELECT * FROM sessions
             WHERE (? IS NULL OR (context_kind = 'project' AND json_extract(context_json, '$.project_id') = ?))
               AND (? IS NULL OR archived = ?)
               AND (? IS NULL OR title LIKE ?)
               AND (? IS NULL OR updated_at_ms < ? OR (updated_at_ms = ? AND id > ?))
             ORDER BY updated_at_ms DESC, id ASC
             LIMIT ?",
        )
        .bind(project_id)
        .bind(project_id)
        .bind(archived)
        .bind(archived)
        .bind(query.as_deref())
        .bind(query.as_deref())
        .bind(before_time)
        .bind(before_time)
        .bind(before_time)
        .bind(before_id)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        let items = rows
            .iter()
            .map(session_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let next_cursor = (items.len() == usize::try_from(limit).unwrap_or(usize::MAX))
            .then(|| items.last())
            .flatten()
            .map(|session| SessionCursor {
                updated_at_ms: session.updated_at_ms,
                id: session.id.clone(),
            });
        Ok(SessionPage { items, next_cursor })
    }

    pub async fn resume_session(
        &self,
        request: &SessionResumeRequest,
    ) -> Result<SessionResumeSnapshot, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let session_row = sqlx::query("SELECT * FROM sessions WHERE id = ?")
            .bind(request.session_id.as_str())
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| AgentStoreError::SessionNotFound(request.session_id.clone()))?;
        let session = session_from_row(&session_row)?;
        let snapshot_sequence =
            u64::try_from(session_row.get::<i64, _>("next_sequence").saturating_sub(1))
                .unwrap_or_default();
        let active_run_row = sqlx::query(
            "SELECT * FROM runs WHERE session_id = ? AND status IN ('queued', 'preparing', 'running', 'waiting_approval', 'waiting_user_input', 'cancelling') ORDER BY created_at_ms DESC, id ASC LIMIT 1",
        )
        .bind(request.session_id.as_str())
        .fetch_optional(&mut *transaction)
        .await?;
        let active_run = active_run_row.as_ref().map(run_from_row).transpose()?;

        let mut transcript = Vec::new();
        let mut previous_transcript_cursor = None;
        if !request.metadata_only {
            let before = request
                .transcript_before_sequence
                .unwrap_or(snapshot_sequence.saturating_add(1));
            let limit = request.transcript_limit.clamp(1, 200);
            let rows = sqlx::query(
                "SELECT * FROM transcript_items WHERE session_id = ? AND sequence < ? ORDER BY sequence DESC LIMIT ?",
            )
            .bind(request.session_id.as_str())
            .bind(i64::try_from(before).unwrap_or(i64::MAX))
            .bind(i64::from(limit))
            .fetch_all(&mut *transaction)
            .await?;
            transcript = rows
                .iter()
                .rev()
                .map(|row| transcript_item_from_row(row, &request.session_id))
                .collect::<Result<Vec<_>, _>>()?;
            if transcript.len() == usize::try_from(limit).unwrap_or(usize::MAX) {
                previous_transcript_cursor = transcript.first().map(|item| item.sequence);
            }
        }

        let (pending_approvals, pending_user_inputs, usage_snapshot) = if let Some(run) =
            &active_run
        {
            let approval_rows = sqlx::query(
                "SELECT * FROM approval_requests WHERE session_id = ? AND run_id = ? AND run_generation = ? AND status = 'pending' ORDER BY created_at_ms ASC, id ASC",
            )
            .bind(request.session_id.as_str())
            .bind(run.id.as_str())
            .bind(i64::try_from(run.generation).unwrap_or(i64::MAX))
            .fetch_all(&mut *transaction)
            .await?;
            let user_input_rows = sqlx::query(
                "SELECT * FROM user_input_requests WHERE session_id = ? AND run_id = ? AND run_generation = ? AND status = 'pending' ORDER BY created_at_ms ASC, id ASC",
            )
            .bind(request.session_id.as_str())
            .bind(run.id.as_str())
            .bind(i64::try_from(run.generation).unwrap_or(i64::MAX))
            .fetch_all(&mut *transaction)
            .await?;
            let usage_row = sqlx::query("SELECT * FROM run_usage_snapshots WHERE run_id = ?")
                .bind(run.id.as_str())
                .fetch_optional(&mut *transaction)
                .await?;
            (
                approval_rows
                    .iter()
                    .map(approval_from_row)
                    .collect::<Result<Vec<_>, _>>()?,
                user_input_rows
                    .iter()
                    .map(user_input_from_row)
                    .collect::<Result<Vec<_>, _>>()?,
                usage_row.as_ref().map(usage_from_row).transpose()?,
            )
        } else {
            (Vec::new(), Vec::new(), None)
        };
        transaction.commit().await?;
        let active_event_replay = self.list_active_event_replay(&request.session_id, 0);
        Ok(SessionResumeSnapshot {
            session,
            active_run,
            transcript,
            pending_approvals,
            pending_user_inputs,
            usage_snapshot,
            active_event_replay,
            snapshot_sequence,
            previous_transcript_cursor,
        })
    }

    pub async fn fork_session_idempotent(
        &self,
        principal: &str,
        request: &SessionForkRequest,
        new_session_id: SessionId,
        created_at_ms: i64,
    ) -> Result<SessionRecord, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        if let Some(existing_id) = sqlx::query_scalar::<_, String>(
            "SELECT resource_id FROM idempotency_records WHERE principal = ? AND method = 'session.fork' AND idempotency_key = ?",
        )
        .bind(principal)
        .bind(&request.context.idempotency_key)
        .fetch_optional(&mut *transaction)
        .await?
        {
            let row = sqlx::query("SELECT * FROM sessions WHERE id = ?")
                .bind(existing_id)
                .fetch_optional(&mut *transaction)
                .await?
                .ok_or_else(|| AgentStoreError::SessionNotFound(new_session_id.clone()))?;
            transaction.commit().await?;
            return session_from_row(&row);
        }
        let source_row = sqlx::query("SELECT * FROM sessions WHERE id = ?")
            .bind(request.source_session_id.as_str())
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| AgentStoreError::SessionNotFound(request.source_session_id.clone()))?;
        let source = session_from_row(&source_row)?;
        let session = SessionRecord {
            id: new_session_id,
            context: source.context,
            entry_profile: source.entry_profile,
            title: request.title.trim().chars().take(200).collect(),
            archived: false,
            pinned: false,
            parent_session_id: Some(source.id.clone()),
            source_run_id: request.source_run_id.clone(),
            created_at_ms,
            updated_at_ms: created_at_ms,
        };
        sqlx::query(
            "INSERT INTO sessions (id, context_kind, context_json, entry_profile, title, archived, pinned, parent_session_id, source_run_id, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, 0, 0, ?, ?, ?, ?)",
        )
        .bind(session.id.as_str())
        .bind(super::session_context_kind(&session.context))
        .bind(serde_json::to_string(&session.context)?)
        .bind(enum_to_db(&session.entry_profile)?)
        .bind(&session.title)
        .bind(request.source_session_id.as_str())
        .bind(request.source_run_id.as_ref().map(RunId::as_str))
        .bind(created_at_ms)
        .bind(created_at_ms)
        .execute(&mut *transaction)
        .await?;
        append_event_tx(
            &mut transaction,
            &session.id,
            None,
            "session.forked",
            json!({
                "parentSessionId": request.source_session_id,
                "sourceRunId": request.source_run_id,
            }),
            created_at_ms,
        )
        .await?;
        sqlx::query(
            "INSERT INTO idempotency_records (principal, method, idempotency_key, resource_id, response_json, created_at_ms) VALUES (?, 'session.fork', ?, ?, '{}', ?)",
        )
        .bind(principal)
        .bind(&request.context.idempotency_key)
        .bind(session.id.as_str())
        .bind(created_at_ms)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(session)
    }

    pub async fn update_session_metadata(
        &self,
        session_id: &SessionId,
        title: Option<&str>,
        archived: Option<bool>,
        pinned: Option<bool>,
        updated_at_ms: i64,
    ) -> Result<SessionRecord, AgentStoreError> {
        let title = title.map(str::trim).filter(|value| !value.is_empty());
        if title.is_some_and(|value| value.chars().count() > 200) {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "session title",
                value: "title exceeds 200 characters".into(),
            });
        }
        let mut transaction = self.pool.begin().await?;
        let changed = sqlx::query(
            "UPDATE sessions SET title = COALESCE(?, title), archived = COALESCE(?, archived), pinned = COALESCE(?, pinned), updated_at_ms = ? WHERE id = ?",
        )
        .bind(title)
        .bind(archived.map(i64::from))
        .bind(pinned.map(i64::from))
        .bind(updated_at_ms)
        .bind(session_id.as_str())
        .execute(&mut *transaction)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(AgentStoreError::SessionNotFound(session_id.clone()));
        }
        append_event_tx(
            &mut transaction,
            session_id,
            None,
            "session.metadata_updated",
            json!({ "titleChanged": title.is_some(), "archived": archived, "pinned": pinned }),
            updated_at_ms,
        )
        .await?;
        let row = sqlx::query("SELECT * FROM sessions WHERE id = ?")
            .bind(session_id.as_str())
            .fetch_one(&mut *transaction)
            .await?;
        let session = session_from_row(&row)?;
        transaction.commit().await?;
        Ok(session)
    }

    pub async fn assert_run_precondition(
        &self,
        run_id: &RunId,
        expected_run_id: &RunId,
        expected_generation: u64,
    ) -> Result<hachimi_protocol::RunRecord, AgentStoreError> {
        if run_id != expected_run_id {
            return Err(AgentStoreError::RunPreconditionFailed);
        }
        let run = self
            .get_run(run_id)
            .await?
            .ok_or_else(|| AgentStoreError::RunNotFound(run_id.clone()))?;
        if run.generation != expected_generation || run.status.is_terminal() {
            return Err(AgentStoreError::RunPreconditionFailed);
        }
        Ok(run)
    }

    pub async fn enqueue_run_steer(
        &self,
        run_id: &RunId,
        expected_run_id: &RunId,
        expected_generation: u64,
        input: &str,
        created_at_ms: i64,
    ) -> Result<RunSteerRecord, AgentStoreError> {
        let input = input.trim();
        if input.is_empty() || input.chars().count() > 32_000 || run_id != expected_run_id {
            return Err(AgentStoreError::RunPreconditionFailed);
        }
        let mut transaction = self.pool.begin().await?;
        let run = get_run_tx(&mut transaction, run_id)
            .await?
            .ok_or_else(|| AgentStoreError::RunNotFound(run_id.clone()))?;
        if run.generation != expected_generation || run.status != RunStatus::Running {
            return Err(AgentStoreError::RunPreconditionFailed);
        }
        let id = hachimi_protocol::ItemId::random();
        let sequence =
            super::next_sequence_tx(&mut transaction, &run.session_id, created_at_ms).await?;
        let payload = ItemPayload::User {
            text: input.into(),
            attachment_ids: Vec::new(),
        };
        sqlx::query(
            "INSERT INTO transcript_items (id, session_id, run_id, sequence, kind, status, payload_json, relations_json, created_at_ms) VALUES (?, ?, ?, ?, 'user', 'completed', ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(run.session_id.as_str())
        .bind(run.id.as_str())
        .bind(i64::try_from(sequence).unwrap_or(i64::MAX))
        .bind(serde_json::to_string(&payload)?)
        .bind(serde_json::to_string(&ItemRelations::default())?)
        .bind(created_at_ms)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO run_steers (id, session_id, run_id, run_generation, item_id, input_text, status, created_at_ms, consumed_at_ms) VALUES (?, ?, ?, ?, ?, ?, 'pending', ?, NULL)",
        )
        .bind(id.as_str())
        .bind(run.session_id.as_str())
        .bind(run.id.as_str())
        .bind(i64::try_from(run.generation).unwrap_or(i64::MAX))
        .bind(id.as_str())
        .bind(input)
        .bind(created_at_ms)
        .execute(&mut *transaction)
        .await?;
        append_event_tx(
            &mut transaction,
            &run.session_id,
            Some(&run.id),
            "run.steer_queued",
            json!({ "itemId": id, "generation": run.generation }),
            created_at_ms,
        )
        .await?;
        transaction.commit().await?;
        Ok(RunSteerRecord {
            id,
            session_id: run.session_id,
            run_id: run.id,
            run_generation: run.generation,
            input: input.into(),
            status: RunSteerStatus::Pending,
            created_at_ms,
            consumed_at_ms: None,
        })
    }

    pub async fn drain_run_steers(
        &self,
        run_id: &RunId,
        run_generation: u64,
        consumed_at_ms: i64,
    ) -> Result<Vec<RunSteerRecord>, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let run = get_run_tx(&mut transaction, run_id)
            .await?
            .ok_or_else(|| AgentStoreError::RunNotFound(run_id.clone()))?;
        if run.generation != run_generation || run.status.is_terminal() {
            return Err(AgentStoreError::RunPreconditionFailed);
        }
        let rows = sqlx::query(
            "SELECT * FROM run_steers WHERE run_id = ? AND run_generation = ? AND status = 'pending' ORDER BY created_at_ms ASC, id ASC",
        )
        .bind(run_id.as_str())
        .bind(i64::try_from(run_generation).unwrap_or(i64::MAX))
        .fetch_all(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE run_steers SET status = 'consumed', consumed_at_ms = ? WHERE run_id = ? AND run_generation = ? AND status = 'pending'",
        )
        .bind(consumed_at_ms)
        .bind(run_id.as_str())
        .bind(i64::try_from(run_generation).unwrap_or(i64::MAX))
        .execute(&mut *transaction)
        .await?;
        let records: Vec<RunSteerRecord> = rows
            .iter()
            .map(|row| RunSteerRecord {
                id: hachimi_protocol::ItemId::new(row.get::<String, _>("id")),
                session_id: SessionId::new(row.get::<String, _>("session_id")),
                run_id: RunId::new(row.get::<String, _>("run_id")),
                run_generation: u64::try_from(row.get::<i64, _>("run_generation"))
                    .unwrap_or_default(),
                input: row.get("input_text"),
                status: RunSteerStatus::Consumed,
                created_at_ms: row.get("created_at_ms"),
                consumed_at_ms: Some(consumed_at_ms),
            })
            .collect();
        for record in &records {
            append_event_tx(
                &mut transaction,
                &record.session_id,
                Some(&record.run_id),
                "run.steer_consumed",
                json!({ "itemId": record.id, "generation": record.run_generation }),
                consumed_at_ms,
            )
            .await?;
        }
        transaction.commit().await?;
        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use hachimi_protocol::{
        ApprovalPolicy, BehaviorMode, CheckoutId, CheckoutKind, CheckoutRecord, CheckoutStatus,
        EntryProfile, ExecutionTarget, ItemId, ItemRelations, ItemStatus, LlmSettings,
        PermissionProfile, ProjectId, ProjectRecord, ProviderCapabilities, RunBudget,
        RunConfiguration, RunDriverKind, RunEventPayload, RunPurpose, RunUsageSnapshot,
        SessionResumeRequest, SessionSearchRequest, TokenCountSource, TokenUsage, TranscriptItem,
        TranscriptItemKind, UserInputAnswer, UserInputQuestion, UserInputRequestId,
        UserInputRequestRecord, UserInputResolution, UserInputStatus, WorkloadKind,
    };

    use super::*;

    async fn seed() -> (AgentStore, SessionRecord, hachimi_protocol::RunRecord) {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let now = 1_700_000_000_000_i64;
        let project = ProjectRecord {
            id: ProjectId::from("project-lifecycle"),
            display_name: "Lifecycle".into(),
            root_path: "C:\\lifecycle".into(),
            git_root: Some("C:\\lifecycle".into()),
            trusted: true,
            created_at_ms: now,
            updated_at_ms: now,
        };
        store.create_project(&project).await.expect("project");
        let checkout = CheckoutRecord {
            id: CheckoutId::from("checkout-lifecycle"),
            project_id: project.id.clone(),
            kind: CheckoutKind::Local,
            path: project.root_path.clone(),
            base_revision: Some("main".into()),
            head_revision: None,
            status: CheckoutStatus::Ready,
            pinned: false,
            created_at_ms: now,
            updated_at_ms: now,
        };
        store.create_checkout(&checkout).await.expect("checkout");
        let session = SessionRecord {
            id: SessionId::from("session-lifecycle"),
            context: hachimi_protocol::SessionContextBinding::Project {
                project_id: project.id,
                checkout_id: checkout.id,
            },
            entry_profile: EntryProfile::Workbench,
            title: "Lifecycle".into(),
            archived: false,
            pinned: false,
            parent_session_id: None,
            source_run_id: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        store.create_session(&session).await.expect("session");
        let run = hachimi_protocol::RunRecord {
            id: RunId::from("run-lifecycle"),
            session_id: session.id.clone(),
            status: RunStatus::Queued,
            purpose: RunPurpose::Task,
            origin: hachimi_protocol::RunOrigin::Interactive,
            generation: 4,
            configuration: RunConfiguration {
                model_snapshot: LlmSettings::default(),
                driver: RunDriverKind::ToolLoop,
                entry_profile: EntryProfile::Workbench,
                workload_override: Some(WorkloadKind::Coding),
                behavior_mode: BehaviorMode::Default,
                execution_target: Some(ExecutionTarget::Local {
                    project_id: session.context.project_id().expect("project").clone(),
                }),
                approval_policy: ApprovalPolicy::OnlyWhenNeeded,
                permission_profile: PermissionProfile::ReadOnly,
                budget: RunBudget::default(),
                accepted_plan_id: None,
                accepted_plan_revision: None,
            },
            requested_capabilities: ProviderCapabilities {
                text_input: true,
                ..ProviderCapabilities::default()
            },
            negotiated_capabilities: ProviderCapabilities::default(),
            provider_capability_probe: None,
            capability_degradations: Vec::new(),
            failure_code: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        store
            .create_run_idempotent("test", "lifecycle-run", &run)
            .await
            .expect("run");
        store
            .transition_run(&run.id, RunStatus::Preparing, None)
            .await
            .expect("preparing");
        store
            .transition_run(&run.id, RunStatus::Running, None)
            .await
            .expect("running");
        (store, session, run)
    }

    #[tokio::test]
    async fn resume_watermark_bridges_snapshot_and_catch_up_without_a_gap() {
        let (store, session, run) = seed().await;
        store
            .upsert_run_usage_snapshot(&RunUsageSnapshot {
                run_id: run.id.clone(),
                billed_usage: TokenUsage {
                    input_tokens: 41,
                    output_tokens: 7,
                },
                active_context_tokens: 23,
                remaining_context_tokens: 70,
                source: TokenCountSource::Tokenizer,
                updated_at_ms: 1_700_000_000_005,
            })
            .await
            .expect("usage");
        store
            .append_transcript_item(TranscriptItem {
                id: ItemId::from("resume-user"),
                session_id: session.id.clone(),
                run_id: Some(run.id.clone()),
                sequence: 0,
                kind: TranscriptItemKind::User,
                status: ItemStatus::Completed,
                payload: ItemPayload::User {
                    text: "before snapshot".into(),
                    attachment_ids: Vec::new(),
                },
                relations: ItemRelations::default(),
                created_at_ms: 1_700_000_000_010,
            })
            .await
            .expect("transcript");
        let snapshot = store
            .resume_session(&SessionResumeRequest {
                session_id: session.id.clone(),
                metadata_only: false,
                transcript_before_sequence: None,
                transcript_limit: 50,
            })
            .await
            .expect("resume");
        assert_eq!(snapshot.transcript.len(), 1);
        assert_eq!(
            snapshot
                .usage_snapshot
                .as_ref()
                .expect("resume usage")
                .billed_usage
                .input_tokens,
            41
        );
        store
            .append_event(
                &session.id,
                Some(&run.id),
                "after.snapshot",
                json!({ "ok": true }),
            )
            .await
            .expect("event");
        let catch_up = store
            .list_events(&session.id, snapshot.snapshot_sequence)
            .await
            .expect("catch up");
        assert_eq!(catch_up.len(), 1);
        assert_eq!(catch_up[0].sequence, snapshot.snapshot_sequence + 1);
        assert_eq!(catch_up[0].event_name(), "after.snapshot");

        let metadata = store
            .resume_session(&SessionResumeRequest {
                session_id: session.id,
                metadata_only: true,
                transcript_before_sequence: None,
                transcript_limit: 0,
            })
            .await
            .expect("metadata resume");
        assert!(metadata.transcript.is_empty());
        assert_eq!(
            metadata
                .usage_snapshot
                .as_ref()
                .expect("metadata usage")
                .active_context_tokens,
            23
        );
        assert!(metadata.snapshot_sequence >= catch_up[0].sequence);
        assert!(
            store
                .list_events(&metadata.session.id, 0)
                .await
                .expect("events")
                .iter()
                .all(|event| !event.event_name().contains("usage"))
        );
    }

    #[tokio::test]
    async fn active_item_deltas_replay_without_entering_sqlite() {
        let (store, session, run) = seed().await;
        let item_id = ItemId::from("streaming-assistant");
        store
            .append_transcript_item(TranscriptItem {
                id: item_id.clone(),
                session_id: session.id.clone(),
                run_id: Some(run.id.clone()),
                sequence: 0,
                kind: TranscriptItemKind::Assistant,
                status: ItemStatus::InProgress,
                payload: ItemPayload::Assistant {
                    text: String::new(),
                },
                relations: ItemRelations::default(),
                created_at_ms: 1_700_000_000_010,
            })
            .await
            .expect("started item");
        let delta = store
            .append_live_item_delta(&session.id, &run.id, &item_id, "streaming")
            .await
            .expect("live delta");
        assert!(matches!(delta.payload, RunEventPayload::ItemDelta { .. }));
        assert!(
            store
                .list_events(&session.id, 0)
                .await
                .expect("persistent events")
                .iter()
                .all(|event| !matches!(event.payload, RunEventPayload::ItemDelta { .. }))
        );

        let snapshot = store
            .resume_session(&SessionResumeRequest {
                session_id: session.id.clone(),
                metadata_only: true,
                transcript_before_sequence: None,
                transcript_limit: 0,
            })
            .await
            .expect("resume");
        assert_eq!(snapshot.active_event_replay, vec![delta]);

        store
            .complete_transcript_item(
                &item_id,
                ItemStatus::Completed,
                ItemPayload::Assistant {
                    text: "streaming".into(),
                },
            )
            .await
            .expect("complete");
        assert!(store.list_active_event_replay(&session.id, 0).is_empty());
    }

    #[tokio::test]
    async fn stale_generations_cannot_steer_or_interrupt_a_run() {
        let (store, _session, run) = seed().await;
        assert!(matches!(
            store.assert_run_precondition(&run.id, &run.id, 3).await,
            Err(AgentStoreError::RunPreconditionFailed)
        ));
        assert!(matches!(
            store
                .enqueue_run_steer(&run.id, &run.id, 3, "stale", 1_700_000_000_020)
                .await,
            Err(AgentStoreError::RunPreconditionFailed)
        ));
        store
            .enqueue_run_steer(&run.id, &run.id, 4, "continue safely", 1_700_000_000_021)
            .await
            .expect("steer");
        assert!(matches!(
            store.drain_run_steers(&run.id, 3, 1_700_000_000_022).await,
            Err(AgentStoreError::RunPreconditionFailed)
        ));
        let drained = store
            .drain_run_steers(&run.id, 4, 1_700_000_000_023)
            .await
            .expect("drain");
        assert_eq!(drained.len(), 1);
        assert!(
            store
                .drain_run_steers(&run.id, 4, 1_700_000_000_024)
                .await
                .expect("second drain")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn session_cursor_does_not_skip_equal_timestamps() {
        let (store, session, _run) = seed().await;
        for suffix in ["a", "b", "c"] {
            store
                .create_session(&SessionRecord {
                    id: SessionId::new(format!("session-{suffix}")),
                    context: session.context.clone(),
                    entry_profile: session.entry_profile,
                    title: format!("Session {suffix}"),
                    archived: false,
                    pinned: false,
                    parent_session_id: None,
                    source_run_id: None,
                    created_at_ms: session.created_at_ms,
                    updated_at_ms: session.updated_at_ms,
                })
                .await
                .expect("session");
        }
        let first = store
            .search_sessions(&SessionSearchRequest {
                project_id: session.context.project_id().cloned(),
                query: None,
                archived: Some(false),
                before: None,
                limit: 2,
            })
            .await
            .expect("first page");
        let second = store
            .search_sessions(&SessionSearchRequest {
                project_id: session.context.project_id().cloned(),
                query: None,
                archived: Some(false),
                before: first.next_cursor.clone(),
                limit: 2,
            })
            .await
            .expect("second page");
        let ids = first
            .items
            .iter()
            .chain(&second.items)
            .map(|item| item.id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(ids.len(), 4);
        assert_eq!(first.items.len(), 2);
        assert_eq!(second.items.len(), 2);
    }

    #[tokio::test]
    async fn secret_user_input_answers_never_enter_sqlite_transcript_or_events() {
        let (store, session, run) = seed().await;
        let request = UserInputRequestRecord {
            id: UserInputRequestId::from("secret-request"),
            session_id: session.id,
            run_id: run.id.clone(),
            run_generation: run.generation,
            item_id: ItemId::from("secret-item"),
            questions: vec![UserInputQuestion {
                id: "token".into(),
                header: "Token".into(),
                prompt: "Enter a temporary token".into(),
                options: Vec::new(),
                secret: true,
                auto_resolution_ms: None,
                default_answer: None,
            }],
            status: UserInputStatus::Pending,
            expires_at_ms: None,
            created_at_ms: 1_700_000_000_030,
            resolved_at_ms: None,
            resolved_by: None,
        };
        store
            .create_user_input_request(&request)
            .await
            .expect("request");
        let secret = "super-secret-answer-value";
        store
            .resolve_user_input(&UserInputResolution {
                request_id: request.id,
                expected_run_id: run.id,
                expected_generation: run.generation,
                action: hachimi_protocol::UserInputResolutionAction::Submit,
                answers: vec![UserInputAnswer {
                    question_id: "token".into(),
                    value: secret.into(),
                }],
                resolved_by: "test".into(),
                resolved_at_ms: 1_700_000_000_031,
            })
            .await
            .expect("resolve");
        let pattern = format!("%{secret}%");
        let request_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM user_input_requests WHERE questions_json || COALESCE(resolved_by, '') LIKE ?",
        )
        .bind(&pattern)
        .fetch_one(&store.pool)
        .await
        .expect("request secret scan");
        let transcript_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM transcript_items WHERE payload_json LIKE ?",
        )
        .bind(&pattern)
        .fetch_one(&store.pool)
        .await
        .expect("transcript secret scan");
        let event_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM run_events WHERE payload_json LIKE ?",
        )
        .bind(pattern)
        .fetch_one(&store.pool)
        .await
        .expect("event secret scan");
        assert_eq!(request_count, 0, "secret leaked into user_input_requests");
        assert_eq!(transcript_count, 0, "secret leaked into transcript_items");
        assert_eq!(event_count, 0, "secret leaked into run_events");
    }
}
