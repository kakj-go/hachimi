use hachimi_protocol::{
    DeliveryPolicy, DeliveryStatus, ScheduleAuthorizationScope, ScheduleDefinition,
    ScheduleGrantId, ScheduleGrantRecord, ScheduleGrantStatus, ScheduleSpec, TaskRunId,
    TaskRunRecord, TaskRunStatus, TaskRunTrigger,
};
use hachimi_storage::AgentStore;
use sha2::{Digest, Sha256};

use crate::{SchedulerError, TimeZoneResolver, error_code, occurrences_after};

pub(super) fn authorization_scope(definition: &ScheduleDefinition) -> ScheduleAuthorizationScope {
    ScheduleAuthorizationScope {
        entry_profile: definition.entry_profile,
        workload_override: definition.workload_override,
        context_template: definition.context_template.clone(),
        tool_allowlist: definition.tool_allowlist.clone(),
        skill_allowlist: definition.skill_allowlist.clone(),
        skill_revisions: Vec::new(),
        mcp_tool_allowlist: definition.mcp_tool_allowlist.clone(),
        permission_config: definition.permission_config.clone(),
        contribution_revisions: definition.contribution_revisions.clone(),
        host_grant: definition.host_grant.clone(),
    }
}

pub(super) fn build_grant(
    definition: &ScheduleDefinition,
    principal: &str,
    now_ms: i64,
    scope: ScheduleAuthorizationScope,
) -> Result<ScheduleGrantRecord, SchedulerError> {
    if scope.entry_profile != definition.entry_profile
        || scope.workload_override != definition.workload_override
        || scope.context_template != definition.context_template
        || scope.tool_allowlist != definition.tool_allowlist
        || scope.skill_allowlist != definition.skill_allowlist
        || scope.mcp_tool_allowlist != definition.mcp_tool_allowlist
        || scope.contribution_revisions != definition.contribution_revisions
        || scope.host_grant != definition.host_grant
        || scope.permission_config != definition.permission_config
    {
        return Err(SchedulerError::InvalidSchedule(
            "authorization scope does not match the Schedule definition".into(),
        ));
    }
    let bytes = serde_json::to_vec(&scope)?;
    let scope_hash = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(ScheduleGrantRecord {
        id: ScheduleGrantId::random(),
        schedule_id: definition.id.clone(),
        permission_revision: definition.permission_revision,
        scope_hash,
        scope,
        status: ScheduleGrantStatus::Active,
        granted_by: principal.into(),
        created_at_ms: now_ms,
        revoked_at_ms: None,
    })
}

pub(super) fn next_occurrence(
    timezone: &dyn TimeZoneResolver,
    schedule: &ScheduleSpec,
    after_ms: i64,
) -> Result<Option<i64>, SchedulerError> {
    occurrences_after(timezone, schedule, after_ms, 1)
        .map(|values| values.into_iter().next())
        .map_err(|error| {
            SchedulerError::InvalidSchedule(format!("{}: {error}", error_code(&error)))
        })
}

pub(super) fn required_next_occurrence(
    timezone: &dyn TimeZoneResolver,
    schedule: &ScheduleSpec,
    after_ms: i64,
) -> Result<i64, SchedulerError> {
    next_occurrence(timezone, schedule, after_ms)?.ok_or(SchedulerError::NoFutureOccurrence)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn task_record(
    schedule: &ScheduleDefinition,
    id: TaskRunId,
    trigger: TaskRunTrigger,
    scheduled_for_ms: i64,
    invocation_key: String,
    status: TaskRunStatus,
    error: Option<(String, String)>,
    now_ms: i64,
) -> TaskRunRecord {
    let (error_code, error_summary) = error.unzip();
    TaskRunRecord {
        id,
        schedule_id: Some(schedule.id.clone()),
        schedule_revision: Some(schedule.config_revision),
        trigger,
        scheduled_for_ms: Some(scheduled_for_ms),
        event_context: None,
        invocation_key,
        requester_session_id: match &schedule.context_template {
            hachimi_protocol::ScheduleContextTemplate::SessionContinuation { session_id } => {
                Some(session_id.clone())
            }
            _ => None,
        },
        execution_session_id: None,
        run_id: None,
        permission_snapshot_hash: None,
        status,
        progress_percent: None,
        result_summary: None,
        error_code,
        error_summary,
        artifact_ids: Vec::new(),
        delivery_status: if schedule.delivery_policy == DeliveryPolicy::TaskTabAndSystemNotification
        {
            DeliveryStatus::Pending
        } else {
            DeliveryStatus::NotRequested
        },
        delivery_error_code: None,
        created_at_ms: now_ms,
        started_at_ms: None,
        finished_at_ms: status.is_terminal().then_some(now_ms),
        updated_at_ms: now_ms,
    }
}

pub(super) async fn apply_stop_conditions(
    store: &AgentStore,
    schedule: &ScheduleDefinition,
    status: TaskRunStatus,
    now_ms: i64,
) -> Result<(), SchedulerError> {
    let Some(current) = store.get_schedule(&schedule.id).await? else {
        return Ok(());
    };
    if !current.enabled {
        return Ok(());
    }
    let occurrence_count = store.count_schedule_task_runs(&schedule.id).await?;
    let should_stop = (status == TaskRunStatus::Succeeded
        && current.stop_conditions.stop_after_success)
        || current
            .stop_conditions
            .max_occurrences
            .is_some_and(|limit| occurrence_count >= u64::from(limit))
        || current
            .stop_conditions
            .end_at_ms
            .is_some_and(|end_at| now_ms >= end_at);
    if should_stop {
        store
            .set_schedule_enabled(&current.id, false, current.config_revision, None, now_ms)
            .await?;
    }
    Ok(())
}

pub(super) fn tracing_fallback(error: &SchedulerError) {
    eprintln!("Hachimi scheduler tick failed: {error}");
}
