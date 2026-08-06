use hachimi_protocol::{
    DeliveryPolicy, DeliveryStatus, ScheduleDefinition, ScheduleSpec, TaskRunId, TaskRunRecord,
    TaskRunStatus, TaskRunTrigger,
};
use hachimi_storage::AgentStore;

use crate::{SchedulerError, TimeZoneResolver, error_code, occurrences_after};

pub(super) fn authority_configuration_changed(
    current: &ScheduleDefinition,
    candidate: &ScheduleDefinition,
) -> bool {
    current.entry_profile != candidate.entry_profile
        || current.workload_override != candidate.workload_override
        || current.context_template != candidate.context_template
        || current.skill_allowlist != candidate.skill_allowlist
        || current.skill_revisions != candidate.skill_revisions
        || current.mcp_tool_allowlist != candidate.mcp_tool_allowlist
        || current.permission_policy != candidate.permission_policy
        || current.contribution_revisions != candidate.contribution_revisions
        || current.host_revision_snapshot != candidate.host_revision_snapshot
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
        requester_session_id: None,
        execution_session_id: None,
        run_id: None,
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
