use super::*;

impl AgentStore {
    pub async fn accept_proposed_plan_idempotent(
        &self,
        principal: &str,
        idempotency_key: &str,
        plan_id: &PlanId,
        run: &RunRecord,
    ) -> Result<(PlanDocument, PlanConfirmation, RunRecord), AgentStoreError> {
        self.accept_proposed_plan_inner(principal, idempotency_key, plan_id, run, None)
            .await
    }

    pub async fn accept_proposed_plan_authorized_idempotent(
        &self,
        principal: &str,
        idempotency_key: &str,
        plan_id: &PlanId,
        run: &RunRecord,
        launch: AtomicRunLaunchInput<'_>,
    ) -> Result<(PlanDocument, PlanConfirmation, RunRecord), AgentStoreError> {
        self.accept_proposed_plan_inner(principal, idempotency_key, plan_id, run, Some(launch))
            .await
    }

    async fn accept_proposed_plan_inner(
        &self,
        principal: &str,
        idempotency_key: &str,
        plan_id: &PlanId,
        run: &RunRecord,
        launch: Option<AtomicRunLaunchInput<'_>>,
    ) -> Result<(PlanDocument, PlanConfirmation, RunRecord), AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        if let Some(existing_id) = sqlx::query_scalar::<_, String>(
            "SELECT resource_id FROM idempotency_records WHERE principal = ? AND method = 'plan.accept' AND idempotency_key = ?",
        )
        .bind(principal)
        .bind(idempotency_key)
        .fetch_optional(&mut *transaction)
        .await?
        {
            let existing_run = get_run_tx(&mut transaction, &RunId::new(existing_id))
                .await?
                .ok_or_else(|| AgentStoreError::RunNotFound(run.id.clone()))?;
            let accepted_plan_id = existing_run
                .configuration
                .accepted_plan_id
                .as_ref()
                .ok_or(AgentStoreError::ProposedPlanRunMismatch)?;
            let plan = get_plan_document_tx(&mut transaction, accepted_plan_id)
                .await?
                .ok_or_else(|| AgentStoreError::ProposedPlanNotFound(accepted_plan_id.clone()))?;
            let confirmation = get_plan_confirmation_tx(&mut transaction, accepted_plan_id)
                .await?
                .ok_or_else(|| AgentStoreError::ProposedPlanNotFound(accepted_plan_id.clone()))?;
            if launch.is_some() {
                run_bundle::require_authority_snapshot_tx(&mut transaction, &existing_run.id)
                    .await?;
            }
            transaction.commit().await?;
            return Ok((plan, confirmation, existing_run));
        }

        let plan = get_plan_document_tx(&mut transaction, plan_id)
            .await?
            .ok_or_else(|| AgentStoreError::ProposedPlanNotFound(plan_id.clone()))?;
        let mut confirmation = get_plan_confirmation_tx(&mut transaction, plan_id)
            .await?
            .ok_or_else(|| AgentStoreError::ProposedPlanNotFound(plan_id.clone()))?;
        if confirmation.status != PlanConfirmationStatus::Pending {
            return Err(AgentStoreError::ProposedPlanNotAcceptable(plan.id));
        }
        if run.session_id != plan.session_id
            || run.status != RunStatus::Queued
            || run.configuration.accepted_plan_id.as_ref() != Some(&plan.id)
            || run.configuration.accepted_plan_revision != Some(plan.revision)
        {
            return Err(AgentStoreError::ProposedPlanRunMismatch);
        }
        let configuration_json = serde_json::to_string(&run.configuration)?;
        sqlx::query(
            "INSERT INTO runs (id, session_id, status, purpose, origin_json, generation, configuration_json, requested_capabilities_json, negotiated_capabilities_json, provider_capability_probe_json, capability_degradations_json, failure_code, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(run.id.as_str())
        .bind(run.session_id.as_str())
        .bind(run.status.as_str())
        .bind(enum_to_db(&run.purpose)?)
        .bind(serde_json::to_string(&run.origin)?)
        .bind(i64::try_from(run.generation).unwrap_or(i64::MAX))
        .bind(configuration_json)
        .bind(serde_json::to_string(&run.requested_capabilities)?)
        .bind(serde_json::to_string(&run.negotiated_capabilities)?)
        .bind(serde_json::to_string(&run.provider_capability_probe)?)
        .bind(serde_json::to_string(&run.capability_degradations)?)
        .bind(&run.failure_code)
        .bind(run.created_at_ms)
        .bind(run.updated_at_ms)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO run_attachments (run_id, attachment_id) SELECT ?, attachment_id FROM run_attachments WHERE run_id = ?",
        )
        .bind(run.id.as_str())
        .bind(plan.source_run_id.as_str())
        .execute(&mut *transaction)
        .await?;
        if let Some(launch) = launch {
            let session_row = sqlx::query("SELECT * FROM sessions WHERE id = ?")
                .bind(plan.session_id.as_str())
                .fetch_one(&mut *transaction)
                .await?;
            let session = session_from_row(&session_row)?;
            run_bundle::insert_atomic_launch_state_tx(&mut transaction, &session, run, launch)
                .await?;
        }
        let accepted_at_ms = run.created_at_ms;
        sqlx::query(
            "UPDATE plan_confirmations SET status = 'accepted', accepted_run_id = ?, resolved_at_ms = ? WHERE plan_id = ? AND status = 'pending'",
        )
        .bind(run.id.as_str())
        .bind(accepted_at_ms)
        .bind(plan.id.as_str())
        .execute(&mut *transaction)
        .await?;
        confirmation.status = PlanConfirmationStatus::Accepted;
        confirmation.accepted_run_id = Some(run.id.clone());
        confirmation.resolved_at_ms = Some(accepted_at_ms);
        append_event_tx(
            &mut transaction,
            &run.session_id,
            Some(&run.id),
            "run.queued",
            json!({ "status": run.status, "acceptedPlanId": plan.id, "planRevision": plan.revision }),
            run.created_at_ms,
        )
        .await?;
        append_event_tx(
            &mut transaction,
            &run.session_id,
            Some(&run.id),
            "plan.accepted",
            json!({ "planId": plan.id, "revision": plan.revision, "executionRunId": run.id }),
            accepted_at_ms,
        )
        .await?;
        sqlx::query(
            "INSERT INTO idempotency_records (principal, method, idempotency_key, resource_id, response_json, created_at_ms) VALUES (?, 'plan.accept', ?, ?, '{}', ?)",
        )
        .bind(principal)
        .bind(idempotency_key)
        .bind(run.id.as_str())
        .bind(run.created_at_ms)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.bump_session_environment_revision(&run.session_id)
            .await?;
        Ok((plan, confirmation, run.clone()))
    }
}
