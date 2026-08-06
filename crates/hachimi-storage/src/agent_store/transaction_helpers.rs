use super::*;

pub(super) async fn next_sequence_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    session_id: &SessionId,
    updated_at_ms: i64,
) -> Result<u64, AgentStoreError> {
    let sequence = sqlx::query_scalar::<_, i64>(
        "UPDATE sessions SET next_sequence = next_sequence + 1, updated_at_ms = ? WHERE id = ? RETURNING next_sequence - 1",
    )
    .bind(updated_at_ms)
    .bind(session_id.as_str())
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| AgentStoreError::SessionNotFound(session_id.clone()))?;
    Ok(u64::try_from(sequence).unwrap_or_default())
}

pub(super) async fn append_event_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    session_id: &SessionId,
    run_id: Option<&RunId>,
    event: &str,
    payload: Value,
    created_at_ms: i64,
) -> Result<RunEventEnvelope, AgentStoreError> {
    append_event_typed_tx(
        transaction,
        session_id,
        run_id,
        event,
        None,
        payload,
        created_at_ms,
    )
    .await
}

pub(super) async fn append_event_typed_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    session_id: &SessionId,
    run_id: Option<&RunId>,
    event: &str,
    typed_payload: Option<RunEventPayload>,
    payload: Value,
    created_at_ms: i64,
) -> Result<RunEventEnvelope, AgentStoreError> {
    let sequence = next_sequence_tx(transaction, session_id, created_at_ms).await?;
    let payload = typed_payload.unwrap_or_else(|| RunEventPayload::Generic {
        event: event.to_owned(),
        data: payload,
    });
    sqlx::query(
        "INSERT INTO run_events (session_id, sequence, run_id, payload_json, created_at_ms) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(session_id.as_str())
    .bind(i64::try_from(sequence).unwrap_or(i64::MAX))
    .bind(run_id.map(RunId::as_str))
    .bind(serde_json::to_string(&payload)?)
    .bind(created_at_ms)
    .execute(&mut **transaction)
    .await?;
    Ok(RunEventEnvelope {
        protocol_version: CONTROL_PROTOCOL_VERSION,
        sequence,
        session_id: session_id.clone(),
        run_id: run_id.cloned(),
        payload,
        created_at_ms,
    })
}

pub(super) async fn get_run_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    run_id: &RunId,
) -> Result<Option<RunRecord>, AgentStoreError> {
    let row = sqlx::query("SELECT * FROM runs WHERE id = ?")
        .bind(run_id.as_str())
        .fetch_optional(&mut **transaction)
        .await?;
    row.as_ref().map(run_from_row).transpose()
}

pub(super) async fn get_run_connection(
    connection: &mut sqlx::pool::PoolConnection<Sqlite>,
    run_id: &RunId,
) -> Result<Option<RunRecord>, AgentStoreError> {
    let row = sqlx::query("SELECT * FROM runs WHERE id = ?")
        .bind(run_id.as_str())
        .fetch_optional(&mut **connection)
        .await?;
    row.as_ref().map(run_from_row).transpose()
}

pub(super) async fn get_proposed_plan_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    plan_id: &PlanId,
) -> Result<Option<ProposedPlan>, AgentStoreError> {
    let row = sqlx::query("SELECT * FROM proposed_plans WHERE id = ?")
        .bind(plan_id.as_str())
        .fetch_optional(&mut **transaction)
        .await?;
    row.as_ref().map(proposed_plan_from_row).transpose()
}

pub(super) async fn get_proposed_plan_by_run_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    run_id: &RunId,
) -> Result<Option<ProposedPlan>, AgentStoreError> {
    let row = sqlx::query("SELECT * FROM proposed_plans WHERE run_id = ?")
        .bind(run_id.as_str())
        .fetch_optional(&mut **transaction)
        .await?;
    row.as_ref().map(proposed_plan_from_row).transpose()
}

pub(super) async fn latest_compaction_checkpoint_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    session_id: &SessionId,
) -> Result<Option<CompactionCheckpoint>, AgentStoreError> {
    let row = sqlx::query(
        "SELECT * FROM compaction_checkpoints WHERE session_id = ? ORDER BY covered_through_sequence DESC LIMIT 1",
    )
    .bind(session_id.as_str())
    .fetch_optional(&mut **transaction)
    .await?;
    row.as_ref().map(compaction_checkpoint_from_row).transpose()
}
