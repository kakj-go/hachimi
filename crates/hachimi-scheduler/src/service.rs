// SPDX-License-Identifier: MIT
// Copyright (c) 2026 OpenClaw Foundation
// Adapted from openclaw/openclaw src/cron/service/{timer-scheduler,timer-catchup,task-runs}.ts
// Commit: f6d456235cf011004f7cffc71a95acf6fbf1fa0a
// Modified for Hachimi: SQLite invocation claims, fresh Session/Run execution, grants, and Tokio timers.

use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc, time::Duration};

use hachimi_protocol::{
    ArtifactId, DeliveryPolicy, DeliveryStatus, EntryProfile, MisfirePolicy,
    ScheduleAuthorizationScope, ScheduleDefinition, ScheduleGrantId, ScheduleGrantRecord,
    ScheduleGrantStatus, ScheduleHealth, ScheduleId, SchedulePreview, ScheduleSnapshot,
    ScheduleSpec, TaskRunId, TaskRunRecord, TaskRunStatus, TaskRunTrigger, WorkloadKind,
};
use hachimi_storage::{AgentStore, AgentStoreError, ScheduleInvocationClaim};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::{TimeZoneResolver, error_code, occurrences_after, preview_schedule};

const MIN_TASK_TIMEOUT_MS: u64 = 60_000;
const MAX_TASK_TIMEOUT_MS: u64 = 24 * 60 * 60 * 1_000;
const MISFIRE_GRACE_MS: i64 = 60_000;
const MAX_SCHEDULER_SLEEP_MS: u64 = 60_000;
const MIN_REFIRE_GAP_MS: u64 = 250;

pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleRunCompletion {
    pub status: TaskRunStatus,
    pub result_summary: Option<String>,
    pub error_code: Option<String>,
    pub error_summary: Option<String>,
    pub artifact_ids: Vec<ArtifactId>,
}

pub type ScheduleLaunchFuture = Pin<
    Box<dyn Future<Output = Result<ScheduleRunCompletion, ScheduleLaunchError>> + Send + 'static>,
>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{code}: {message}")]
pub struct ScheduleLaunchError {
    pub code: String,
    pub message: String,
}

pub trait ScheduleRunLauncher: Send + Sync {
    fn launch(
        &self,
        schedule: ScheduleDefinition,
        task_run: TaskRunRecord,
        cancellation: CancellationToken,
    ) -> ScheduleLaunchFuture;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskNotification {
    pub schedule_id: ScheduleId,
    pub task_run_id: TaskRunId,
    pub task_name: String,
    pub status: TaskRunStatus,
}

pub type NotificationFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'static>>;

pub trait NotificationAdapter: Send + Sync {
    fn deliver(&self, notification: TaskNotification) -> NotificationFuture;
}

#[derive(Debug, Default)]
pub struct NoopNotificationAdapter;

impl NotificationAdapter for NoopNotificationAdapter {
    fn deliver(&self, _notification: TaskNotification) -> NotificationFuture {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error("scheduler storage failed: {0}")]
    Store(#[from] AgentStoreError),
    #[error("invalid schedule: {0}")]
    InvalidSchedule(String),
    #[error("schedule does not have a future occurrence")]
    NoFutureOccurrence,
    #[error("schedule permission scope serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct SchedulerService {
    store: AgentStore,
    clock: Arc<dyn Clock>,
    timezone: Arc<dyn TimeZoneResolver>,
    launcher: Arc<dyn ScheduleRunLauncher>,
    notifications: Arc<dyn NotificationAdapter>,
    wake: Arc<Notify>,
    active_launches: Arc<Mutex<BTreeMap<TaskRunId, CancellationToken>>>,
}

impl std::fmt::Debug for SchedulerService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SchedulerService")
            .finish_non_exhaustive()
    }
}

impl SchedulerService {
    #[must_use]
    pub fn new(
        store: AgentStore,
        clock: Arc<dyn Clock>,
        timezone: Arc<dyn TimeZoneResolver>,
        launcher: Arc<dyn ScheduleRunLauncher>,
        notifications: Arc<dyn NotificationAdapter>,
    ) -> Self {
        Self {
            store,
            clock,
            timezone,
            launcher,
            notifications,
            wake: Arc::new(Notify::new()),
            active_launches: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn preview(&self, schedule: &ScheduleSpec, count: usize) -> SchedulePreview {
        preview_schedule(self.timezone.as_ref(), schedule, self.clock.now_ms(), count)
    }

    pub async fn create(
        &self,
        principal: &str,
        idempotency_key: &str,
        mut definition: ScheduleDefinition,
        authorize: bool,
    ) -> Result<ScheduleSnapshot, SchedulerError> {
        normalize_definition(&mut definition);
        let grant_scope = authorize.then(|| authorization_scope(&definition));
        self.create_with_grant_scope(principal, idempotency_key, definition, grant_scope)
            .await
    }

    pub async fn create_with_grant_scope(
        &self,
        principal: &str,
        idempotency_key: &str,
        mut definition: ScheduleDefinition,
        grant_scope: Option<ScheduleAuthorizationScope>,
    ) -> Result<ScheduleSnapshot, SchedulerError> {
        normalize_definition(&mut definition);
        validate_definition(&definition)?;
        definition.config_revision = 1;
        definition.permission_revision = 1;
        definition.created_by = principal.to_owned();
        let now = self.clock.now_ms();
        definition.created_at_ms = now;
        definition.updated_at_ms = now;
        definition.next_run_at_ms = if definition.enabled {
            Some(required_next_occurrence(
                self.timezone.as_ref(),
                &definition.schedule,
                now,
            )?)
        } else {
            None
        };
        let grant = grant_scope
            .map(|scope| build_grant(&definition, principal, now, scope))
            .transpose()?;
        if grant.is_some() {
            definition.health = ScheduleHealth::Healthy;
            definition.health_reason = None;
        } else {
            definition.health = ScheduleHealth::NeedsAuthorization;
            definition.health_reason = Some("schedule_authorization_required".into());
        }
        let snapshot = self
            .store
            .create_schedule_idempotent(principal, idempotency_key, &definition, grant.as_ref())
            .await?;
        self.wake.notify_one();
        Ok(snapshot)
    }

    pub async fn get(
        &self,
        schedule_id: &ScheduleId,
    ) -> Result<Option<ScheduleSnapshot>, SchedulerError> {
        Ok(self.store.get_schedule_snapshot(schedule_id).await?)
    }

    pub async fn list(&self) -> Result<Vec<ScheduleDefinition>, SchedulerError> {
        Ok(self.store.list_schedules().await?)
    }

    pub async fn update(
        &self,
        mut definition: ScheduleDefinition,
        expected_config_revision: u64,
    ) -> Result<ScheduleDefinition, SchedulerError> {
        let current = self
            .store
            .get_schedule(&definition.id)
            .await?
            .ok_or_else(|| AgentStoreError::ScheduleNotFound(definition.id.clone()))?;
        if current.config_revision != expected_config_revision {
            return Err(AgentStoreError::ScheduleRevisionConflict.into());
        }
        normalize_definition(&mut definition);
        validate_definition(&definition)?;
        let scope_changed = authorization_scope(&current) != authorization_scope(&definition);
        definition.created_by = current.created_by;
        definition.created_at_ms = current.created_at_ms;
        definition.config_revision = current.config_revision.saturating_add(1);
        definition.updated_at_ms = self.clock.now_ms();
        if scope_changed {
            definition.permission_revision = current.permission_revision.saturating_add(1);
            definition.health = ScheduleHealth::NeedsAuthorization;
            definition.health_reason = Some("schedule_permission_scope_changed".into());
        } else {
            definition.permission_revision = current.permission_revision;
            definition.health = current.health;
            definition.health_reason = current.health_reason;
        }
        definition.next_run_at_ms = if definition.enabled {
            Some(required_next_occurrence(
                self.timezone.as_ref(),
                &definition.schedule,
                self.clock.now_ms(),
            )?)
        } else {
            None
        };
        let updated = self
            .store
            .update_schedule(&definition, expected_config_revision)
            .await?;
        self.wake.notify_one();
        Ok(updated)
    }

    pub async fn set_enabled(
        &self,
        schedule_id: &ScheduleId,
        enabled: bool,
        expected_config_revision: u64,
    ) -> Result<ScheduleDefinition, SchedulerError> {
        let schedule = self
            .store
            .get_schedule(schedule_id)
            .await?
            .ok_or_else(|| AgentStoreError::ScheduleNotFound(schedule_id.clone()))?;
        let now = self.clock.now_ms();
        let next = if enabled {
            Some(required_next_occurrence(
                self.timezone.as_ref(),
                &schedule.schedule,
                now,
            )?)
        } else {
            None
        };
        let updated = self
            .store
            .set_schedule_enabled(schedule_id, enabled, expected_config_revision, next, now)
            .await?;
        self.wake.notify_one();
        Ok(updated)
    }

    pub async fn remove(&self, schedule_id: &ScheduleId) -> Result<bool, SchedulerError> {
        let removed = self.store.remove_schedule(schedule_id).await?;
        self.wake.notify_one();
        Ok(removed)
    }

    pub async fn reauthorize(
        &self,
        schedule_id: &ScheduleId,
        principal: &str,
    ) -> Result<ScheduleGrantRecord, SchedulerError> {
        let schedule = self
            .store
            .get_schedule(schedule_id)
            .await?
            .ok_or_else(|| AgentStoreError::ScheduleNotFound(schedule_id.clone()))?;
        let scope = authorization_scope(&schedule);
        self.reauthorize_with_grant_scope(schedule_id, principal, scope)
            .await
    }

    pub async fn reauthorize_with_grant_scope(
        &self,
        schedule_id: &ScheduleId,
        principal: &str,
        scope: ScheduleAuthorizationScope,
    ) -> Result<ScheduleGrantRecord, SchedulerError> {
        let schedule = self
            .store
            .get_schedule(schedule_id)
            .await?
            .ok_or_else(|| AgentStoreError::ScheduleNotFound(schedule_id.clone()))?;
        let grant = build_grant(&schedule, principal, self.clock.now_ms(), scope)?;
        let grant = self.store.reauthorize_schedule(&grant).await?;
        self.wake.notify_one();
        Ok(grant)
    }

    pub async fn revoke_grant(
        &self,
        schedule_id: &ScheduleId,
    ) -> Result<Option<ScheduleGrantRecord>, SchedulerError> {
        let grant = self
            .store
            .revoke_schedule_grant(schedule_id, self.clock.now_ms())
            .await?;
        self.wake.notify_one();
        Ok(grant)
    }

    pub async fn run_now(&self, schedule_id: &ScheduleId) -> Result<TaskRunRecord, SchedulerError> {
        let schedule = self
            .store
            .get_schedule(schedule_id)
            .await?
            .ok_or_else(|| AgentStoreError::ScheduleNotFound(schedule_id.clone()))?;
        let now = self.clock.now_ms();
        let task_id = TaskRunId::random();
        let task = task_record(
            &schedule,
            task_id.clone(),
            TaskRunTrigger::Manual,
            now,
            format!("manual:{}:{}", schedule.id, task_id),
            TaskRunStatus::Queued,
            None,
            now,
        );
        let claim = self
            .store
            .claim_schedule_invocation(&schedule.id, schedule.config_revision, &task)
            .await?;
        let claimed_task = claim.task_run.clone();
        self.launch_claim(schedule, claim);
        Ok(claimed_task)
    }

    pub async fn retry(&self, task_run_id: &TaskRunId) -> Result<TaskRunRecord, SchedulerError> {
        let previous = self
            .store
            .get_task_run(task_run_id)
            .await?
            .ok_or_else(|| AgentStoreError::TaskRunNotFound(task_run_id.clone()))?;
        if !matches!(
            previous.status,
            TaskRunStatus::Failed
                | TaskRunStatus::TimedOut
                | TaskRunStatus::Cancelled
                | TaskRunStatus::Lost
        ) {
            return Err(SchedulerError::InvalidSchedule(format!(
                "TaskRun {} with status {} is not retryable",
                previous.id,
                previous.status.as_str()
            )));
        }
        let schedule_id = previous
            .schedule_id
            .ok_or_else(|| SchedulerError::InvalidSchedule("task has no Schedule".into()))?;
        let schedule = self
            .store
            .get_schedule(&schedule_id)
            .await?
            .ok_or_else(|| AgentStoreError::ScheduleNotFound(schedule_id.clone()))?;
        let now = self.clock.now_ms();
        let retry_id = TaskRunId::random();
        let task = task_record(
            &schedule,
            retry_id.clone(),
            TaskRunTrigger::Retry,
            now,
            format!("retry:{task_run_id}:{retry_id}"),
            TaskRunStatus::Queued,
            None,
            now,
        );
        let claim = self
            .store
            .claim_schedule_invocation(&schedule.id, schedule.config_revision, &task)
            .await?;
        let claimed_task = claim.task_run.clone();
        self.launch_claim(schedule, claim);
        Ok(claimed_task)
    }

    pub async fn cancel_task(
        &self,
        task_run_id: &TaskRunId,
    ) -> Result<TaskRunRecord, SchedulerError> {
        let current = self
            .store
            .get_task_run(task_run_id)
            .await?
            .ok_or_else(|| AgentStoreError::TaskRunNotFound(task_run_id.clone()))?;
        if current.status.is_terminal() {
            return Ok(current);
        }
        if let Some(cancellation) = self.active_launches.lock().get(task_run_id).cloned() {
            cancellation.cancel();
        }
        Ok(self
            .store
            .transition_task_run(
                task_run_id,
                TaskRunStatus::Cancelled,
                current.progress_percent,
                current.result_summary.as_deref(),
                Some("task_cancelled"),
                Some("the scheduled invocation was cancelled"),
                &current.artifact_ids,
                self.clock.now_ms(),
            )
            .await?)
    }

    pub async fn reconcile_startup(&self) -> Result<Vec<TaskRunRecord>, SchedulerError> {
        let queued = self
            .store
            .list_task_runs(None, 500)
            .await?
            .into_iter()
            .filter(|task| task.status == TaskRunStatus::Queued)
            .collect::<Vec<_>>();
        let mut relaunched = Vec::new();
        for task in queued {
            if self.active_launches.lock().contains_key(&task.id) {
                continue;
            }
            let Some(schedule_id) = task.schedule_id.clone() else {
                continue;
            };
            let Some(schedule) = self.store.get_schedule(&schedule_id).await? else {
                let _ = self
                    .store
                    .transition_task_run(
                        &task.id,
                        TaskRunStatus::NeedsAttention,
                        None,
                        None,
                        Some("schedule_missing"),
                        Some("the Schedule was removed before its queued invocation started"),
                        &[],
                        self.clock.now_ms(),
                    )
                    .await;
                continue;
            };
            self.launch_claim(
                schedule,
                ScheduleInvocationClaim {
                    task_run: task.clone(),
                    should_launch: true,
                },
            );
            relaunched.push(task);
        }
        Ok(relaunched)
    }

    pub async fn trigger_due(&self) -> Result<Vec<TaskRunRecord>, SchedulerError> {
        let now = self.clock.now_ms();
        let due = self.store.list_due_schedules(now).await?;
        let mut tasks = Vec::with_capacity(due.len());
        for schedule in due {
            let scheduled_for = schedule.next_run_at_ms.unwrap_or(now);
            let misfired = now.saturating_sub(scheduled_for) > MISFIRE_GRACE_MS;
            let (trigger, status, error) =
                if misfired && schedule.misfire_policy == MisfirePolicy::Skip {
                    (
                        TaskRunTrigger::Scheduled,
                        TaskRunStatus::Skipped,
                        Some((
                            "schedule_misfire_skipped".into(),
                            "the occurrence was missed while the scheduler was not running".into(),
                        )),
                    )
                } else {
                    (
                        if misfired {
                            TaskRunTrigger::CatchUp
                        } else {
                            TaskRunTrigger::Scheduled
                        },
                        TaskRunStatus::Queued,
                        None,
                    )
                };
            let task_id = TaskRunId::random();
            let task = task_record(
                &schedule,
                task_id,
                trigger,
                scheduled_for,
                format!("schedule:{}:{}", schedule.id, scheduled_for),
                status,
                error,
                now,
            );
            let claim = self
                .store
                .claim_schedule_invocation(&schedule.id, schedule.config_revision, &task)
                .await?;
            let next = next_occurrence(self.timezone.as_ref(), &schedule.schedule, now)?;
            match schedule.schedule {
                ScheduleSpec::At { .. } => {
                    let _ = self
                        .store
                        .set_schedule_enabled(
                            &schedule.id,
                            false,
                            schedule.config_revision,
                            None,
                            now,
                        )
                        .await?;
                }
                ScheduleSpec::Every { .. } | ScheduleSpec::Cron { .. } => {
                    self.store
                        .update_schedule_next_run(&schedule.id, next, now)
                        .await?;
                }
            }
            tasks.push(claim.task_run.clone());
            self.launch_claim(schedule, claim);
        }
        Ok(tasks)
    }

    #[must_use]
    pub fn start(self: Arc<Self>) -> SchedulerHandle {
        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let service = Arc::clone(&self);
        let join = tokio::spawn(async move {
            loop {
                if task_cancellation.is_cancelled() {
                    break;
                }
                let now = service.clock.now_ms();
                let wake_at = service.store.next_schedule_wakeup().await.ok().flatten();
                let delay = Duration::from_millis(scheduler_delay_ms(wake_at, now));
                tokio::select! {
                    () = task_cancellation.cancelled() => break,
                    () = service.wake.notified() => continue,
                    () = tokio::time::sleep(delay) => {
                        if let Err(error) = service.trigger_due().await {
                            tracing_fallback(&error);
                        }
                    }
                }
            }
        });
        SchedulerHandle {
            cancellation,
            join: Some(join),
        }
    }

    fn launch_claim(&self, schedule: ScheduleDefinition, claim: ScheduleInvocationClaim) {
        if !claim.should_launch {
            return;
        }
        let launcher = Arc::clone(&self.launcher);
        let notifications = Arc::clone(&self.notifications);
        let store = self.store.clone();
        let clock = Arc::clone(&self.clock);
        let active_launches = Arc::clone(&self.active_launches);
        let cancellation = CancellationToken::new();
        active_launches
            .lock()
            .insert(claim.task_run.id.clone(), cancellation.clone());
        tokio::spawn(async move {
            let task_id = claim.task_run.id.clone();
            let completion = launcher
                .launch(schedule.clone(), claim.task_run, cancellation)
                .await;
            let completion = match completion {
                Ok(completion) => completion,
                Err(error) => ScheduleRunCompletion {
                    status: TaskRunStatus::Failed,
                    result_summary: None,
                    error_code: Some(error.code),
                    error_summary: Some(error.message),
                    artifact_ids: Vec::new(),
                },
            };
            let now = clock.now_ms();
            let current = store.get_task_run(&task_id).await.ok().flatten();
            if current
                .as_ref()
                .is_some_and(|task| task.status == TaskRunStatus::Queued)
            {
                let _ = store
                    .transition_task_run(
                        &task_id,
                        TaskRunStatus::Preparing,
                        None,
                        None,
                        None,
                        None,
                        &[],
                        now,
                    )
                    .await;
            }
            let current = store.get_task_run(&task_id).await.ok().flatten();
            if completion.status == TaskRunStatus::Succeeded
                && current
                    .as_ref()
                    .is_some_and(|task| task.status == TaskRunStatus::Preparing)
            {
                let _ = store
                    .transition_task_run(
                        &task_id,
                        TaskRunStatus::Running,
                        Some(0),
                        None,
                        None,
                        None,
                        &[],
                        now,
                    )
                    .await;
            }
            let updated = store
                .transition_task_run(
                    &task_id,
                    completion.status,
                    Some(100),
                    completion.result_summary.as_deref(),
                    completion.error_code.as_deref(),
                    completion.error_summary.as_deref(),
                    &completion.artifact_ids,
                    now,
                )
                .await;
            if schedule.delivery_policy == DeliveryPolicy::TaskTabAndSystemNotification
                && let Ok(task) = updated
            {
                let delivered = notifications
                    .deliver(TaskNotification {
                        schedule_id: schedule.id,
                        task_run_id: task.id.clone(),
                        task_name: schedule.name,
                        status: task.status,
                    })
                    .await;
                let (status, code) = match delivered {
                    Ok(()) => (DeliveryStatus::Delivered, None),
                    Err(_) => (DeliveryStatus::Failed, Some("system_notification_failed")),
                };
                let _ = store
                    .update_task_delivery(&task.id, status, code, now)
                    .await;
            }
            active_launches.lock().remove(&task_id);
        });
    }
}

#[derive(Debug)]
pub struct SchedulerHandle {
    cancellation: CancellationToken,
    join: Option<tokio::task::JoinHandle<()>>,
}

impl SchedulerHandle {
    pub async fn stop(mut self) {
        self.cancellation.cancel();
        if let Some(join) = self.join.take() {
            let _ = join.await;
        }
    }
}

impl Drop for SchedulerHandle {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

fn scheduler_delay_ms(wake_at_ms: Option<i64>, now_ms: i64) -> u64 {
    let raw_delay = wake_at_ms
        .map(|timestamp| u64::try_from(timestamp.saturating_sub(now_ms).max(0)).unwrap_or(0))
        .unwrap_or(MAX_SCHEDULER_SLEEP_MS);
    if raw_delay == 0 {
        MIN_REFIRE_GAP_MS
    } else {
        raw_delay.min(MAX_SCHEDULER_SLEEP_MS)
    }
}

fn normalize_definition(definition: &mut ScheduleDefinition) {
    definition.name = definition.name.trim().chars().take(200).collect();
    definition.prompt = definition.prompt.trim().to_owned();
    definition.tool_allowlist.sort();
    definition.tool_allowlist.dedup();
    definition.skill_allowlist.sort();
    definition.skill_allowlist.dedup();
    definition.mcp_tool_allowlist.sort_by(|left, right| {
        (
            &left.server_id,
            &left.tool_name,
            &left.schema_hash,
            &left.host_identity_hash,
        )
            .cmp(&(
                &right.server_id,
                &right.tool_name,
                &right.schema_hash,
                &right.host_identity_hash,
            ))
    });
    definition.mcp_tool_allowlist.dedup();
    definition.permission_config.external_targets.sort();
    definition.permission_config.external_targets.dedup();
}

fn validate_definition(definition: &ScheduleDefinition) -> Result<(), SchedulerError> {
    if definition.name.is_empty() {
        return Err(SchedulerError::InvalidSchedule("name is required".into()));
    }
    if definition.prompt.is_empty() || definition.prompt.chars().count() > 32_000 {
        return Err(SchedulerError::InvalidSchedule(
            "prompt must contain 1-32000 characters".into(),
        ));
    }
    if !(MIN_TASK_TIMEOUT_MS..=MAX_TASK_TIMEOUT_MS).contains(&definition.timeout_ms) {
        return Err(SchedulerError::InvalidSchedule(
            "timeout must be between one minute and 24 hours".into(),
        ));
    }
    if definition.entry_profile != EntryProfile::Workbench {
        return Err(SchedulerError::InvalidSchedule(
            "only the Workbench entry profile can be scheduled".into(),
        ));
    }
    match (&definition.workload_override, &definition.context_template) {
        (
            Some(WorkloadKind::Coding),
            hachimi_protocol::ScheduleContextTemplate::Project {
                project_id,
                execution_target,
            },
        ) if execution_target.project_id() == project_id => {}
        (Some(WorkloadKind::Coding), _) => {
            return Err(SchedulerError::InvalidSchedule(
                "coding schedules require a matching Project target".into(),
            ));
        }
        (Some(WorkloadKind::Office | WorkloadKind::General) | None, _) => {}
    }
    if matches!(
        definition.context_template,
        hachimi_protocol::ScheduleContextTemplate::General
    ) && (definition.permission_config.allow_file_read
        || definition.permission_config.allow_file_write
        || definition.permission_config.allow_exec)
    {
        return Err(SchedulerError::InvalidSchedule(
            "General schedules cannot request Workspace file or process access".into(),
        ));
    }
    Ok(())
}

fn authorization_scope(definition: &ScheduleDefinition) -> ScheduleAuthorizationScope {
    ScheduleAuthorizationScope {
        entry_profile: definition.entry_profile,
        workload_override: definition.workload_override,
        context_template: definition.context_template.clone(),
        tool_allowlist: definition.tool_allowlist.clone(),
        skill_allowlist: definition.skill_allowlist.clone(),
        skill_revisions: Vec::new(),
        mcp_tool_allowlist: definition.mcp_tool_allowlist.clone(),
        permission_config: definition.permission_config.clone(),
    }
}

fn build_grant(
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

fn next_occurrence(
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

fn required_next_occurrence(
    timezone: &dyn TimeZoneResolver,
    schedule: &ScheduleSpec,
    after_ms: i64,
) -> Result<i64, SchedulerError> {
    next_occurrence(timezone, schedule, after_ms)?.ok_or(SchedulerError::NoFutureOccurrence)
}

#[allow(clippy::too_many_arguments)]
fn task_record(
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
        invocation_key,
        requester_session_id: None,
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

fn tracing_fallback(error: &SchedulerError) {
    eprintln!("Hachimi scheduler tick failed: {error}");
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

    use hachimi_protocol::{
        ApprovalPolicy, DeliveryPolicy, MisfirePolicy, PermissionProfile, ScheduleContextTemplate,
        SchedulePermissionConfig, ScheduleSkillSelection, SkillId,
    };

    use super::*;
    use crate::BundledIanaTimeZoneResolver;

    #[derive(Debug)]
    struct TestClock(AtomicI64);

    impl Clock for TestClock {
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    #[derive(Debug)]
    struct ImmediateLauncher;

    impl ScheduleRunLauncher for ImmediateLauncher {
        fn launch(
            &self,
            _schedule: ScheduleDefinition,
            _task_run: TaskRunRecord,
            _cancellation: CancellationToken,
        ) -> ScheduleLaunchFuture {
            Box::pin(async {
                Ok(ScheduleRunCompletion {
                    status: TaskRunStatus::Succeeded,
                    result_summary: Some("done".into()),
                    error_code: None,
                    error_summary: None,
                    artifact_ids: Vec::new(),
                })
            })
        }
    }

    #[derive(Debug)]
    struct LateSuccessLauncher {
        launches: Arc<AtomicUsize>,
    }

    impl ScheduleRunLauncher for LateSuccessLauncher {
        fn launch(
            &self,
            _schedule: ScheduleDefinition,
            _task_run: TaskRunRecord,
            cancellation: CancellationToken,
        ) -> ScheduleLaunchFuture {
            let launches = Arc::clone(&self.launches);
            Box::pin(async move {
                launches.fetch_add(1, Ordering::SeqCst);
                cancellation.cancelled().await;
                Ok(ScheduleRunCompletion {
                    status: TaskRunStatus::Succeeded,
                    result_summary: Some("late success must be fenced".into()),
                    error_code: None,
                    error_summary: None,
                    artifact_ids: Vec::new(),
                })
            })
        }
    }

    #[derive(Debug)]
    struct CountingLauncher(Arc<AtomicUsize>);

    impl ScheduleRunLauncher for CountingLauncher {
        fn launch(
            &self,
            _schedule: ScheduleDefinition,
            _task_run: TaskRunRecord,
            _cancellation: CancellationToken,
        ) -> ScheduleLaunchFuture {
            let launches = Arc::clone(&self.0);
            Box::pin(async move {
                launches.fetch_add(1, Ordering::SeqCst);
                Ok(ScheduleRunCompletion {
                    status: TaskRunStatus::Succeeded,
                    result_summary: Some("reconciled".into()),
                    error_code: None,
                    error_summary: None,
                    artifact_ids: Vec::new(),
                })
            })
        }
    }

    #[derive(Debug)]
    struct FailedLauncher;

    impl ScheduleRunLauncher for FailedLauncher {
        fn launch(
            &self,
            _schedule: ScheduleDefinition,
            _task_run: TaskRunRecord,
            _cancellation: CancellationToken,
        ) -> ScheduleLaunchFuture {
            Box::pin(async {
                Ok(ScheduleRunCompletion {
                    status: TaskRunStatus::Failed,
                    result_summary: None,
                    error_code: Some("fixture_failed".into()),
                    error_summary: Some("deterministic failure".into()),
                    artifact_ids: Vec::new(),
                })
            })
        }
    }

    #[derive(Debug)]
    struct FailedNotificationAdapter;

    impl NotificationAdapter for FailedNotificationAdapter {
        fn deliver(&self, _notification: TaskNotification) -> NotificationFuture {
            Box::pin(async { Err("notification backend unavailable".into()) })
        }
    }

    fn definition(now: i64) -> ScheduleDefinition {
        ScheduleDefinition {
            id: ScheduleId::from("schedule-test"),
            name: "Daily summary".into(),
            enabled: true,
            prompt: "Summarize the configured inputs.".into(),
            schedule: ScheduleSpec::Every {
                interval_ms: 86_400_000,
                anchor_ms: now + 1_000,
            },
            entry_profile: EntryProfile::Workbench,
            workload_override: Some(WorkloadKind::Office),
            context_template: ScheduleContextTemplate::General,
            tool_allowlist: vec!["request_user_input".into()],
            skill_allowlist: Vec::new(),
            mcp_tool_allowlist: Vec::new(),
            permission_config: SchedulePermissionConfig {
                permission_profile: PermissionProfile::ReadOnly,
                ..SchedulePermissionConfig::default()
            },
            permission_revision: 99,
            timeout_ms: 120_000,
            misfire_policy: MisfirePolicy::Skip,
            delivery_policy: DeliveryPolicy::TaskTabOnly,
            config_revision: 99,
            created_by: "ignored".into(),
            next_run_at_ms: None,
            health: ScheduleHealth::Invalid,
            health_reason: Some("ignored".into()),
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    async fn service(now: i64) -> (SchedulerService, Arc<TestClock>) {
        let clock = Arc::new(TestClock(AtomicI64::new(now)));
        let service = SchedulerService::new(
            AgentStore::connect_in_memory().await.expect("store"),
            clock.clone(),
            Arc::new(BundledIanaTimeZoneResolver),
            Arc::new(ImmediateLauncher),
            Arc::new(NoopNotificationAdapter),
        );
        (service, clock)
    }

    #[tokio::test]
    async fn prompt_and_timing_edits_keep_the_existing_permission_revision() {
        let now = 1_800_000_000_000;
        let (service, _) = service(now).await;
        let created = service
            .create("user", "create-1", definition(now), true)
            .await
            .expect("create");
        assert_eq!(created.definition.permission_revision, 1);
        assert_eq!(created.definition.health, ScheduleHealth::Healthy);
        let mut edited = created.definition.clone();
        edited.prompt = "Use the same scope with a new prompt.".into();
        edited.schedule = ScheduleSpec::Every {
            interval_ms: 7 * 86_400_000,
            anchor_ms: now + 2_000,
        };
        let edited = service
            .update(edited, created.definition.config_revision)
            .await
            .expect("update");
        assert_eq!(edited.permission_revision, 1);
        assert_eq!(edited.health, ScheduleHealth::Healthy);
        assert!(
            service
                .get(&edited.id)
                .await
                .expect("get")
                .expect("snapshot")
                .active_grant
                .is_some()
        );
    }

    #[tokio::test]
    async fn authority_scope_edits_require_a_new_user_grant() {
        let now = 1_800_000_000_000;
        let (service, _) = service(now).await;
        let created = service
            .create("user", "create-2", definition(now), true)
            .await
            .expect("create");
        let mut edited = created.definition.clone();
        edited.tool_allowlist.push("mcp:send_mail".into());
        let edited = service
            .update(edited, created.definition.config_revision)
            .await
            .expect("update");
        assert_eq!(edited.permission_revision, 2);
        assert_eq!(edited.health, ScheduleHealth::NeedsAuthorization);
        let grant = service
            .reauthorize(&edited.id, "user")
            .await
            .expect("reauthorize");
        assert_eq!(grant.permission_revision, 2);
        assert_eq!(
            service
                .get(&edited.id)
                .await
                .expect("get")
                .expect("snapshot")
                .definition
                .health,
            ScheduleHealth::Healthy
        );
    }

    #[tokio::test]
    async fn skill_content_drift_changes_the_grant_without_reusing_old_authority() {
        let now = 1_800_000_000_000;
        let (service, _) = service(now).await;
        let mut requested = definition(now);
        requested.skill_allowlist = vec![SkillId::from("daily-office")];
        let mut first_scope = authorization_scope(&requested);
        first_scope.skill_revisions = vec![ScheduleSkillSelection {
            skill_id: SkillId::from("daily-office"),
            content_hash: "content-v1".into(),
            tree_revision: "tree-v1".into(),
        }];
        let created = service
            .create_with_grant_scope("user", "skill-drift", requested, Some(first_scope))
            .await
            .expect("create with Skill revision");
        let first_grant = created.active_grant.expect("grant");

        let mut second_scope = authorization_scope(&created.definition);
        second_scope.skill_revisions = vec![ScheduleSkillSelection {
            skill_id: SkillId::from("daily-office"),
            content_hash: "content-v2".into(),
            tree_revision: "tree-v2".into(),
        }];
        let second_grant = service
            .reauthorize_with_grant_scope(&created.definition.id, "user", second_scope)
            .await
            .expect("reauthorize changed Skill");

        assert_eq!(
            first_grant.permission_revision,
            second_grant.permission_revision
        );
        assert_ne!(first_grant.scope_hash, second_grant.scope_hash);
        assert_eq!(
            second_grant.scope.skill_revisions[0].content_hash,
            "content-v2"
        );
    }

    #[tokio::test]
    async fn unauthorized_background_invocations_need_attention_without_waiting() {
        let now = 1_800_000_000_000;
        let (service, _) = service(now).await;
        let created = service
            .create("user", "create-3", definition(now), false)
            .await
            .expect("create");
        let task = service
            .run_now(&created.definition.id)
            .await
            .expect("run now");
        let stored = service
            .store
            .get_task_run(&task.id)
            .await
            .expect("task")
            .expect("task row");
        assert_eq!(stored.status, TaskRunStatus::NeedsAttention);
        assert_eq!(
            stored.error_code.as_deref(),
            Some("schedule_authorization_required")
        );
    }

    #[tokio::test]
    async fn authorized_background_invocation_runs_without_a_window_or_transport_context() {
        let now = 1_800_000_000_000;
        let store = AgentStore::connect_in_memory().await.expect("store");
        let launches = Arc::new(AtomicUsize::new(0));
        let service = SchedulerService::new(
            store.clone(),
            Arc::new(TestClock(AtomicI64::new(now))),
            Arc::new(BundledIanaTimeZoneResolver),
            Arc::new(CountingLauncher(Arc::clone(&launches))),
            Arc::new(NoopNotificationAdapter),
        );
        let schedule = service
            .create(
                "service:scheduler-test",
                "windowless-create",
                definition(now),
                true,
            )
            .await
            .expect("create")
            .definition;
        let task = service.run_now(&schedule.id).await.expect("run now");

        for _ in 0..100 {
            if store
                .get_task_run(&task.id)
                .await
                .expect("task")
                .is_some_and(|task| task.status == TaskRunStatus::Succeeded)
            {
                break;
            }
            tokio::task::yield_now().await;
        }

        assert_eq!(launches.load(Ordering::SeqCst), 1);
        assert_eq!(
            store
                .get_task_run(&task.id)
                .await
                .expect("task")
                .expect("row")
                .status,
            TaskRunStatus::Succeeded
        );
    }

    #[tokio::test]
    async fn enabled_at_schedule_must_have_a_future_occurrence() {
        let now = 1_800_000_000_000;
        let (service, _) = service(now).await;
        let mut past = definition(now);
        past.schedule = ScheduleSpec::At {
            timestamp_ms: now - 1,
        };
        let error = service
            .create("user", "past-at", past, true)
            .await
            .expect_err("past At schedule must fail closed");
        assert!(matches!(error, SchedulerError::NoFutureOccurrence));
    }

    #[tokio::test]
    async fn repeated_invocation_claim_never_dispatches_twice() {
        let now = 1_800_000_000_000;
        let (service, _) = service(now).await;
        let schedule = service
            .create("user", "duplicate-claim", definition(now), true)
            .await
            .expect("create")
            .definition;
        let task = task_record(
            &schedule,
            TaskRunId::from("task-duplicate"),
            TaskRunTrigger::Scheduled,
            now + 1_000,
            "schedule:schedule-test:1800000001000".into(),
            TaskRunStatus::Queued,
            None,
            now,
        );
        let first = service
            .store
            .claim_schedule_invocation(&schedule.id, schedule.config_revision, &task)
            .await
            .expect("first claim");
        let repeated = service
            .store
            .claim_schedule_invocation(&schedule.id, schedule.config_revision, &task)
            .await
            .expect("repeated claim");
        assert!(first.should_launch);
        assert!(!repeated.should_launch);
        assert_eq!(first.task_run.id, repeated.task_run.id);
    }

    #[tokio::test]
    async fn cancellation_wins_over_a_late_launcher_success() {
        let now = 1_800_000_000_000;
        let clock = Arc::new(TestClock(AtomicI64::new(now)));
        let launches = Arc::new(AtomicUsize::new(0));
        let service = SchedulerService::new(
            AgentStore::connect_in_memory().await.expect("store"),
            clock,
            Arc::new(BundledIanaTimeZoneResolver),
            Arc::new(LateSuccessLauncher {
                launches: Arc::clone(&launches),
            }),
            Arc::new(NoopNotificationAdapter),
        );
        let schedule = service
            .create("user", "cancel-race", definition(now), true)
            .await
            .expect("create")
            .definition;
        let task = service.run_now(&schedule.id).await.expect("run now");
        for _ in 0..100 {
            if launches.load(Ordering::SeqCst) == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(launches.load(Ordering::SeqCst), 1);
        service.cancel_task(&task.id).await.expect("cancel");
        for _ in 0..100 {
            let stored = service
                .store
                .get_task_run(&task.id)
                .await
                .expect("task")
                .expect("row");
            if stored.status.is_terminal() {
                tokio::task::yield_now().await;
                break;
            }
            tokio::task::yield_now().await;
        }
        let stored = service
            .store
            .get_task_run(&task.id)
            .await
            .expect("task")
            .expect("row");
        assert_eq!(stored.status, TaskRunStatus::Cancelled);
        assert_ne!(
            stored.result_summary.as_deref(),
            Some("late success must be fenced")
        );
    }

    #[tokio::test]
    async fn retry_uses_a_fresh_task_and_invocation_key_and_rejects_successful_runs() {
        let now = 1_800_000_000_000;
        let store = AgentStore::connect_in_memory().await.expect("store");
        let service = SchedulerService::new(
            store.clone(),
            Arc::new(TestClock(AtomicI64::new(now))),
            Arc::new(BundledIanaTimeZoneResolver),
            Arc::new(FailedLauncher),
            Arc::new(NoopNotificationAdapter),
        );
        let schedule = service
            .create("user", "retry-fresh", definition(now), true)
            .await
            .expect("create")
            .definition;
        let first = service.run_now(&schedule.id).await.expect("first Run");
        for _ in 0..100 {
            if store
                .get_task_run(&first.id)
                .await
                .expect("task")
                .is_some_and(|task| task.status == TaskRunStatus::Failed)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        let retry = service.retry(&first.id).await.expect("retry");
        assert_ne!(first.id, retry.id);
        assert_ne!(first.invocation_key, retry.invocation_key);
        assert_eq!(retry.trigger, TaskRunTrigger::Retry);

        let succeeded_service = SchedulerService::new(
            store.clone(),
            Arc::new(TestClock(AtomicI64::new(now))),
            Arc::new(BundledIanaTimeZoneResolver),
            Arc::new(ImmediateLauncher),
            Arc::new(NoopNotificationAdapter),
        );
        let successful = succeeded_service
            .run_now(&schedule.id)
            .await
            .expect("successful Run");
        for _ in 0..100 {
            if store
                .get_task_run(&successful.id)
                .await
                .expect("task")
                .is_some_and(|task| task.status == TaskRunStatus::Succeeded)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        let error = succeeded_service
            .retry(&successful.id)
            .await
            .expect_err("successful Run must not retry");
        assert!(matches!(error, SchedulerError::InvalidSchedule(_)));
    }

    #[tokio::test]
    async fn notification_failure_changes_delivery_only_not_execution_status() {
        let now = 1_800_000_000_000;
        let store = AgentStore::connect_in_memory().await.expect("store");
        let service = SchedulerService::new(
            store.clone(),
            Arc::new(TestClock(AtomicI64::new(now))),
            Arc::new(BundledIanaTimeZoneResolver),
            Arc::new(ImmediateLauncher),
            Arc::new(FailedNotificationAdapter),
        );
        let mut requested = definition(now);
        requested.delivery_policy = DeliveryPolicy::TaskTabAndSystemNotification;
        let schedule = service
            .create("user", "notification-failure", requested, true)
            .await
            .expect("create")
            .definition;
        let task = service.run_now(&schedule.id).await.expect("run now");
        let stored = loop {
            let stored = store
                .get_task_run(&task.id)
                .await
                .expect("task")
                .expect("task row");
            if stored.delivery_status == DeliveryStatus::Failed {
                break stored;
            }
            tokio::task::yield_now().await;
        };
        assert_eq!(stored.status, TaskRunStatus::Succeeded);
        assert_eq!(stored.delivery_status, DeliveryStatus::Failed);
        assert_eq!(
            stored.delivery_error_code.as_deref(),
            Some("system_notification_failed")
        );
    }

    #[tokio::test]
    async fn startup_reconciliation_dispatches_each_safe_queued_claim_once() {
        let now = 1_800_000_000_000;
        let store = AgentStore::connect_in_memory().await.expect("store");
        let clock = Arc::new(TestClock(AtomicI64::new(now)));
        let setup = SchedulerService::new(
            store.clone(),
            clock.clone(),
            Arc::new(BundledIanaTimeZoneResolver),
            Arc::new(ImmediateLauncher),
            Arc::new(NoopNotificationAdapter),
        );
        let schedule = setup
            .create("user", "reconcile", definition(now), true)
            .await
            .expect("create")
            .definition;
        let queued = task_record(
            &schedule,
            TaskRunId::from("task-reconcile"),
            TaskRunTrigger::Scheduled,
            now + 1_000,
            "schedule:schedule-test:reconcile".into(),
            TaskRunStatus::Queued,
            None,
            now,
        );
        let claim = store
            .claim_schedule_invocation(&schedule.id, schedule.config_revision, &queued)
            .await
            .expect("claim");
        assert!(claim.should_launch);

        let launches = Arc::new(AtomicUsize::new(0));
        let restarted = SchedulerService::new(
            store.clone(),
            clock,
            Arc::new(BundledIanaTimeZoneResolver),
            Arc::new(CountingLauncher(Arc::clone(&launches))),
            Arc::new(NoopNotificationAdapter),
        );
        assert_eq!(
            restarted
                .reconcile_startup()
                .await
                .expect("reconcile")
                .len(),
            1
        );
        assert_eq!(
            restarted.reconcile_startup().await.expect("dedupe").len(),
            0
        );
        for _ in 0..100 {
            if store
                .get_task_run(&queued.id)
                .await
                .expect("task")
                .is_some_and(|task| task.status == TaskRunStatus::Succeeded)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(launches.load(Ordering::SeqCst), 1);
        assert_eq!(
            store
                .get_task_run(&queued.id)
                .await
                .expect("task")
                .expect("row")
                .status,
            TaskRunStatus::Succeeded
        );
    }

    #[test]
    fn schedule_permissions_do_not_reuse_interactive_approval_policy() {
        assert_eq!(ApprovalPolicy::OnlyWhenNeeded, ApprovalPolicy::default());
        assert_ne!(
            serde_json::to_string(&SchedulePermissionConfig::default()).expect("JSON"),
            serde_json::to_string(&ApprovalPolicy::OnlyWhenNeeded).expect("JSON")
        );
    }

    #[test]
    fn scheduler_timer_is_bounded_and_avoids_zero_delay_refire_loops() {
        let now = 1_800_000_000_000;
        assert_eq!(scheduler_delay_ms(None, now), MAX_SCHEDULER_SLEEP_MS);
        assert_eq!(
            scheduler_delay_ms(Some(now + MAX_SCHEDULER_SLEEP_MS as i64 + 1), now),
            MAX_SCHEDULER_SLEEP_MS
        );
        assert_eq!(scheduler_delay_ms(Some(now), now), MIN_REFIRE_GAP_MS);
        assert_eq!(scheduler_delay_ms(Some(now - 1), now), MIN_REFIRE_GAP_MS);
        assert_eq!(scheduler_delay_ms(Some(now + 1_000), now), 1_000);
    }

    #[tokio::test]
    #[ignore = "real SystemClock release soak"]
    async fn system_clock_at_every_and_six_field_cron_soak_without_duplicate_invocations() {
        let _short_interval_guard = crate::calendar::enable_release_soak_short_intervals();
        let store = AgentStore::connect_in_memory().await.expect("store");
        let launches = Arc::new(AtomicUsize::new(0));
        let service = Arc::new(SchedulerService::new(
            store.clone(),
            Arc::new(SystemClock),
            Arc::new(BundledIanaTimeZoneResolver),
            Arc::new(CountingLauncher(Arc::clone(&launches))),
            Arc::new(NoopNotificationAdapter),
        ));
        let now = SystemClock.now_ms();

        let mut at = definition(now);
        at.id = ScheduleId::from("system-clock-at");
        at.name = "System clock At".into();
        at.schedule = ScheduleSpec::At {
            timestamp_ms: now + 800,
        };
        service
            .create("release-soak", "system-clock-at", at, true)
            .await
            .expect("At schedule");

        let mut every = definition(now);
        every.id = ScheduleId::from("system-clock-every");
        every.name = "System clock Every".into();
        every.schedule = ScheduleSpec::Every {
            interval_ms: 300,
            anchor_ms: now + 300,
        };
        service
            .create("release-soak", "system-clock-every", every, true)
            .await
            .expect("Every schedule");

        let mut cron = definition(now);
        cron.id = ScheduleId::from("system-clock-cron");
        cron.name = "System clock Cron".into();
        cron.schedule = ScheduleSpec::Cron {
            expression: "*/1 * * * * *".into(),
            timezone: "UTC".into(),
        };
        service
            .create("release-soak", "system-clock-cron", cron, true)
            .await
            .expect("Cron schedule");

        let handle = Arc::clone(&service).start();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        while tokio::time::Instant::now() < deadline {
            let tasks = store.list_task_runs(None, 500).await.expect("soak tasks");
            let every_count = tasks
                .iter()
                .filter(|task| {
                    task.schedule_id.as_ref() == Some(&ScheduleId::from("system-clock-every"))
                })
                .count();
            let at_seen = tasks.iter().any(|task| {
                task.schedule_id.as_ref() == Some(&ScheduleId::from("system-clock-at"))
            });
            let cron_seen = tasks.iter().any(|task| {
                task.schedule_id.as_ref() == Some(&ScheduleId::from("system-clock-cron"))
            });
            if every_count >= 20 && at_seen && cron_seen {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        drop(handle);
        assert!(
            launches.load(Ordering::SeqCst) >= 22,
            "natural timer did not produce the required 20+ occurrence soak"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;

        let tasks = store.list_task_runs(None, 500).await.expect("task runs");
        let keys = tasks
            .iter()
            .map(|task| task.invocation_key.clone())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            keys.len(),
            tasks.len(),
            "an occurrence was invoked more than once"
        );
        assert!(tasks.iter().any(|task| {
            task.schedule_id.as_ref() == Some(&ScheduleId::from("system-clock-at"))
        }));
        assert!(tasks.iter().any(|task| {
            task.schedule_id.as_ref() == Some(&ScheduleId::from("system-clock-cron"))
        }));
        let mut every_occurrences = tasks
            .iter()
            .filter(|task| {
                task.schedule_id.as_ref() == Some(&ScheduleId::from("system-clock-every"))
            })
            .filter_map(|task| task.scheduled_for_ms)
            .collect::<Vec<_>>();
        every_occurrences.sort_unstable();
        assert!(every_occurrences.len() >= 20);
        assert!(
            every_occurrences
                .windows(2)
                .all(|pair| pair[1] > pair[0] && (pair[1] - pair[0]) % 300 == 0),
            "Every occurrences drifted away from their fixed anchor"
        );
        assert!(
            service.active_launches.lock().is_empty(),
            "completed natural-clock invocations leaked active workers"
        );
        let at = store
            .get_schedule(&ScheduleId::from("system-clock-at"))
            .await
            .expect("At schedule")
            .expect("At row");
        assert!(
            !at.enabled,
            "one-shot At schedule was not disabled after firing"
        );
    }
}
