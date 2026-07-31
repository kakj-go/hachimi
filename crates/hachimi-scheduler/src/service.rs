// SPDX-License-Identifier: MIT
// Copyright (c) 2026 OpenClaw Foundation
// Adapted from openclaw/openclaw src/cron/service/{timer-scheduler,timer-catchup,task-runs}.ts
// Commit: f6d456235cf011004f7cffc71a95acf6fbf1fa0a
// Modified for Hachimi: SQLite invocation claims, fresh Session/Run execution, grants, and Tokio timers.

use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc, time::Duration};

use hachimi_protocol::{
    ArtifactId, DeliveryPolicy, DeliveryStatus, EntryProfile, MisfirePolicy, PermissionProfile,
    ScheduleAuthorizationScope, ScheduleDefinition, ScheduleGrantRecord, ScheduleHealth,
    ScheduleId, SchedulePreview, ScheduleSnapshot, ScheduleSpec, TaskRunId, TaskRunRecord,
    TaskRunStatus, TaskRunTrigger, WorkloadKind,
};
use hachimi_storage::{AgentStore, AgentStoreError, ScheduleInvocationClaim};
use parking_lot::Mutex;
use thiserror::Error;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::service_helpers::{
    apply_stop_conditions, authorization_scope, build_grant, next_occurrence,
    required_next_occurrence, task_record, tracing_fallback,
};
use crate::{TimeZoneResolver, preview_schedule};

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
    pub(crate) store: AgentStore,
    pub(crate) clock: Arc<dyn Clock>,
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
        definition.next_run_at_ms =
            if definition.enabled && !matches!(definition.schedule, ScheduleSpec::Event { .. }) {
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
        definition.next_run_at_ms =
            if definition.enabled && !matches!(definition.schedule, ScheduleSpec::Event { .. }) {
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
        let next = if enabled && !matches!(schedule.schedule, ScheduleSpec::Event { .. }) {
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
            .clone()
            .ok_or_else(|| SchedulerError::InvalidSchedule("task has no Schedule".into()))?;
        let schedule = self
            .store
            .get_schedule(&schedule_id)
            .await?
            .ok_or_else(|| AgentStoreError::ScheduleNotFound(schedule_id.clone()))?;
        let now = self.clock.now_ms();
        let retry_id = TaskRunId::random();
        let mut task = task_record(
            &schedule,
            retry_id.clone(),
            TaskRunTrigger::Retry,
            now,
            format!("retry:{task_run_id}:{retry_id}"),
            TaskRunStatus::Queued,
            None,
            now,
        );
        task.event_context = previous.event_context.clone();
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
            let occurrence_count = self.store.count_schedule_task_runs(&schedule.id).await?;
            let reached_limit = schedule
                .stop_conditions
                .max_occurrences
                .is_some_and(|limit| occurrence_count >= u64::from(limit));
            let reached_end = schedule
                .stop_conditions
                .end_at_ms
                .is_some_and(|end_at| scheduled_for > end_at);
            if reached_limit || reached_end {
                let _ = self
                    .store
                    .set_schedule_enabled(&schedule.id, false, schedule.config_revision, None, now)
                    .await?;
                continue;
            }
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
            let next =
                next_occurrence(self.timezone.as_ref(), &schedule.schedule, now)?.filter(|next| {
                    schedule
                        .stop_conditions
                        .end_at_ms
                        .is_none_or(|end_at| *next <= end_at)
                });
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
                ScheduleSpec::Event { .. } => {}
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

    pub(crate) fn launch_claim(
        &self,
        schedule: ScheduleDefinition,
        claim: ScheduleInvocationClaim,
    ) {
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
            let hook_before = store
                .dispatch_plugin_hook_event(
                    &hachimi_storage::PluginHookEventRecord {
                        event: "schedule.before".into(),
                        session_id: None,
                        run_id: None,
                        run_generation: None,
                        subject: task_id.as_str().into(),
                        result_code: "started".into(),
                        created_at_ms: clock.now_ms(),
                    },
                    cancellation.child_token(),
                )
                .await;
            let completion = if let Err(error) = hook_before {
                Err(ScheduleLaunchError {
                    code: "plugin_schedule_before_hook_failed".into(),
                    message: error.to_string(),
                })
            } else {
                launcher
                    .launch(schedule.clone(), claim.task_run, cancellation.clone())
                    .await
            };
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
            let _ = store
                .dispatch_plugin_hook_event(
                    &hachimi_storage::PluginHookEventRecord {
                        event: "schedule.after".into(),
                        session_id: None,
                        run_id: None,
                        run_generation: None,
                        subject: task_id.as_str().into(),
                        result_code: if completion.status == TaskRunStatus::Succeeded {
                            "succeeded".into()
                        } else {
                            "failed".into()
                        },
                        created_at_ms: now,
                    },
                    cancellation.child_token(),
                )
                .await;
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
            if updated.is_ok() {
                let _ = apply_stop_conditions(&store, &schedule, completion.status, now).await;
            }
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
    definition.contribution_revisions.sort_by(|left, right| {
        (&left.plugin_id, &left.contribution_id, &left.account_id).cmp(&(
            &right.plugin_id,
            &right.contribution_id,
            &right.account_id,
        ))
    });
    definition.contribution_revisions.dedup_by(|left, right| {
        left.plugin_id == right.plugin_id
            && left.contribution_id == right.contribution_id
            && left.account_id == right.account_id
    });
    definition
        .host_grant
        .connectors
        .sort_by(|left, right| left.account_id.cmp(&right.account_id));
    for selection in &mut definition.host_grant.connectors {
        selection.allowed_actions = selection
            .allowed_actions
            .iter()
            .map(|action| action.trim().to_owned())
            .filter(|action| !action.is_empty())
            .collect();
        selection.allowed_actions.sort();
        selection.allowed_actions.dedup();
        definition
            .tool_allowlist
            .extend(["connector_list_accounts".into(), "connector_invoke".into()]);
        definition.permission_config.external_targets.extend(
            selection
                .allowed_actions
                .iter()
                .map(|action| format!("connector:{}:{action}", selection.account_id)),
        );
        if let Some(current) = definition
            .contribution_revisions
            .iter_mut()
            .find(|revision| {
                revision.plugin_id == selection.contribution_revision.plugin_id
                    && revision.contribution_id == selection.contribution_revision.contribution_id
                    && revision.account_id == selection.contribution_revision.account_id
            })
        {
            *current = selection.contribution_revision.clone();
        } else {
            definition
                .contribution_revisions
                .push(selection.contribution_revision.clone());
        }
    }
    if let Some(browser) = &mut definition.host_grant.browser {
        browser.document_origins = browser
            .document_origins
            .iter()
            .map(|origin| origin.trim().to_owned())
            .filter(|origin| !origin.is_empty())
            .collect();
        browser.document_origins.sort();
        browser.document_origins.dedup();
        browser.resource_origins = browser
            .resource_origins
            .iter()
            .map(|origin| origin.trim().to_owned())
            .filter(|origin| !origin.is_empty())
            .collect();
        browser.resource_origins.sort();
        browser.resource_origins.dedup();
        browser.capabilities.sort();
        browser.capabilities.dedup();
        if browser.enabled {
            definition
                .tool_allowlist
                .extend(["browser_start".into(), "browser_stop".into()]);
            if browser
                .capabilities
                .contains(&hachimi_protocol::BrowserCapability::Observe)
            {
                definition.tool_allowlist.push("browser_observe".into());
            }
            if browser
                .capabilities
                .iter()
                .any(|capability| *capability != hachimi_protocol::BrowserCapability::Observe)
            {
                definition.tool_allowlist.push("browser_act".into());
            }
            definition.permission_config.external_targets.extend(
                browser
                    .document_origins
                    .iter()
                    .map(|origin| format!("browser:document:{origin}")),
            );
            definition.permission_config.external_targets.extend(
                browser
                    .resource_origins
                    .iter()
                    .map(|origin| format!("browser:resource:{origin}")),
            );
        }
    }
    if !definition.host_grant.connectors.is_empty()
        || definition
            .host_grant
            .browser
            .as_ref()
            .is_some_and(|browser| browser.enabled)
    {
        definition.permission_config.permission_profile = PermissionProfile::ExternalSandbox;
    }
    definition.tool_allowlist.sort();
    definition.tool_allowlist.dedup();
    definition.contribution_revisions.sort_by(|left, right| {
        (&left.plugin_id, &left.contribution_id, &left.account_id).cmp(&(
            &right.plugin_id,
            &right.contribution_id,
            &right.account_id,
        ))
    });
    definition.contribution_revisions.dedup_by(|left, right| {
        left.plugin_id == right.plugin_id
            && left.contribution_id == right.contribution_id
            && left.account_id == right.account_id
    });
    definition.permission_config.external_targets.sort();
    definition.permission_config.external_targets.dedup();
}

/// Canonicalizes all persisted authority-bearing Schedule fields before an
/// AppServer computes extension snapshots or mutation fingerprints.
pub fn normalize_schedule_definition(definition: &mut ScheduleDefinition) {
    normalize_definition(definition);
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
    if let ScheduleSpec::Event { matcher } = &definition.schedule {
        validate_event_identity("source principal", &matcher.source.principal, 256)?;
        validate_event_identity("source id", &matcher.source.id, 256)?;
        validate_event_identity("event type", &matcher.event_type, 256)?;
        if matcher
            .subject_prefix
            .as_ref()
            .is_some_and(|value| value.chars().count() > 512)
        {
            return Err(SchedulerError::InvalidSchedule(
                "event subject prefix must contain at most 512 characters".into(),
            ));
        }
        if matcher.labels.len() > 16 {
            return Err(SchedulerError::InvalidSchedule(
                "event matcher supports at most 16 labels".into(),
            ));
        }
        for (key, value) in &matcher.labels {
            validate_event_identity("event label key", key, 128)?;
            if value.chars().count() > 256 {
                return Err(SchedulerError::InvalidSchedule(
                    "event label values must contain at most 256 characters".into(),
                ));
            }
        }
        if let Some(resource) = &matcher.resource {
            validate_event_identity("event resource kind", &resource.kind, 128)?;
            validate_event_identity("event resource id", &resource.id, 512)?;
            if resource
                .revision
                .as_ref()
                .is_some_and(|value| value.is_empty() || value.chars().count() > 256)
            {
                return Err(SchedulerError::InvalidSchedule(
                    "event resource revision must contain 1-256 characters".into(),
                ));
            }
        }
    }
    if definition.host_grant.computer_unattended {
        return Err(SchedulerError::InvalidSchedule(
            "computer_unattended_unsupported".into(),
        ));
    }
    for selection in &definition.host_grant.connectors {
        if selection.allowed_actions.is_empty()
            || selection.contribution_revision.account_id.as_ref() != Some(&selection.account_id)
        {
            return Err(SchedulerError::InvalidSchedule(
                "schedule_connector_selection_invalid".into(),
            ));
        }
    }
    if let Some(browser) = &definition.host_grant.browser
        && browser.enabled
        && (browser.document_origins.is_empty() || browser.capabilities.is_empty())
    {
        return Err(SchedulerError::InvalidSchedule(
            "schedule_browser_grant_invalid".into(),
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
        (
            Some(WorkloadKind::Coding),
            hachimi_protocol::ScheduleContextTemplate::SessionContinuation { .. },
        ) => {}
        (Some(WorkloadKind::Coding), _) => {
            return Err(SchedulerError::InvalidSchedule(
                "coding schedules require a matching Project target".into(),
            ));
        }
        (Some(WorkloadKind::Office | WorkloadKind::General) | None, _) => {}
    }
    if definition.stop_conditions.max_occurrences == Some(0)
        || definition
            .stop_conditions
            .end_at_ms
            .is_some_and(|end_at| end_at <= definition.created_at_ms)
    {
        return Err(SchedulerError::InvalidSchedule(
            "stop conditions must allow at least one future occurrence".into(),
        ));
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

fn validate_event_identity(field: &str, value: &str, maximum: usize) -> Result<(), SchedulerError> {
    let length = value.chars().count();
    if length == 0 || length > maximum {
        return Err(SchedulerError::InvalidSchedule(format!(
            "{field} must contain 1-{maximum} characters"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

    use hachimi_protocol::{
        ApprovalPolicy, DeliveryPolicy, MisfirePolicy, PermissionProfile, ScheduleContextTemplate,
        SchedulePermissionConfig, ScheduleSkillSelection, SessionId, SkillId,
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
            contribution_revisions: Vec::new(),
            host_grant: hachimi_protocol::ScheduleHostGrant::default(),
            permission_config: SchedulePermissionConfig {
                permission_profile: PermissionProfile::ReadOnly,
                ..SchedulePermissionConfig::default()
            },
            permission_revision: 99,
            timeout_ms: 120_000,
            misfire_policy: MisfirePolicy::Skip,
            delivery_policy: DeliveryPolicy::TaskTabOnly,
            stop_conditions: hachimi_protocol::ScheduleStopConditions::default(),
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
    async fn host_grants_are_canonical_authority_and_change_permission_revision() {
        let now = 1_800_000_000_000;
        let (service, _) = service(now).await;
        let mut input = definition(now);
        let account_id = hachimi_protocol::ConnectorAccountId::from("crm-local");
        input
            .host_grant
            .connectors
            .push(hachimi_protocol::ScheduleConnectorSelection {
                account_id: account_id.clone(),
                contribution_revision: hachimi_protocol::ContributionRevision {
                    plugin_id: hachimi_protocol::PluginId::from("sample-crm"),
                    contribution_id: "sample-crm".into(),
                    account_id: Some(account_id.clone()),
                    content_hash: "content-v1".into(),
                    host_identity_hash: Some("host-v1".into()),
                    schema_hash: Some("schema-v1".into()),
                    action_hash: Some("actions-v1".into()),
                },
                allowed_actions: vec!["search".into()],
            });
        let created = service
            .create("user", "host-grant-create", input, true)
            .await
            .expect("create");
        assert_eq!(
            created.definition.permission_config.permission_profile,
            PermissionProfile::ExternalSandbox
        );
        assert!(
            created
                .definition
                .tool_allowlist
                .contains(&"connector_invoke".into())
        );
        assert!(
            created
                .definition
                .permission_config
                .external_targets
                .contains(&format!("connector:{account_id}:search"))
        );
        assert_eq!(
            created.active_grant.as_ref().unwrap().scope.host_grant,
            created.definition.host_grant
        );

        let mut updated = created.definition.clone();
        updated.host_grant.connectors[0]
            .allowed_actions
            .push("update".into());
        let updated = service
            .update(updated, created.definition.config_revision)
            .await
            .expect("update");
        assert_eq!(updated.permission_revision, 2);
        assert_eq!(updated.health, ScheduleHealth::NeedsAuthorization);
    }

    #[tokio::test]
    async fn unattended_computer_is_stably_rejected() {
        let now = 1_800_000_000_000;
        let (service, _) = service(now).await;
        let mut input = definition(now);
        input.host_grant.computer_unattended = true;
        let error = service
            .create("user", "computer-unattended", input, false)
            .await
            .expect_err("computer must be rejected");
        assert!(
            error
                .to_string()
                .contains("computer_unattended_unsupported")
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
    async fn successful_and_max_occurrence_stop_conditions_disable_the_schedule() {
        let now = 1_800_000_000_000;
        let (service, _) = service(now).await;
        let mut stop_after_success = definition(now);
        stop_after_success.id = ScheduleId::from("stop-after-success");
        stop_after_success.stop_conditions.stop_after_success = true;
        let success_schedule = service
            .create("user", "stop-after-success", stop_after_success, true)
            .await
            .expect("create")
            .definition;
        let success_task = service
            .run_now(&success_schedule.id)
            .await
            .expect("run now");
        for _ in 0..100 {
            if service
                .store
                .get_task_run(&success_task.id)
                .await
                .expect("task")
                .is_some_and(|task| task.status == TaskRunStatus::Succeeded)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        for _ in 0..100 {
            if service
                .store
                .get_schedule(&success_schedule.id)
                .await
                .expect("schedule")
                .is_some_and(|schedule| !schedule.enabled)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            !service
                .store
                .get_schedule(&success_schedule.id)
                .await
                .expect("schedule")
                .expect("row")
                .enabled
        );

        let mut limited = definition(now);
        limited.id = ScheduleId::from("max-occurrences");
        limited.stop_conditions.max_occurrences = Some(2);
        let limited = service
            .create("user", "max-occurrences", limited, true)
            .await
            .expect("create")
            .definition;
        for _ in 0..2 {
            let task = service.run_now(&limited.id).await.expect("run now");
            for _ in 0..100 {
                if service
                    .store
                    .get_task_run(&task.id)
                    .await
                    .expect("task")
                    .is_some_and(|task| task.status == TaskRunStatus::Succeeded)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        }
        for _ in 0..100 {
            if service
                .store
                .get_schedule(&limited.id)
                .await
                .expect("schedule")
                .is_some_and(|schedule| !schedule.enabled)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            !service
                .store
                .get_schedule(&limited.id)
                .await
                .expect("schedule")
                .expect("row")
                .enabled
        );
    }

    #[test]
    fn session_continuation_task_ledger_binds_the_requesting_session() {
        let now = 1_800_000_000_000;
        let mut schedule = definition(now);
        let session_id = SessionId::from("continuation-session");
        schedule.context_template = ScheduleContextTemplate::SessionContinuation {
            session_id: session_id.clone(),
        };
        let task = task_record(
            &schedule,
            TaskRunId::from("continuation-task"),
            TaskRunTrigger::Scheduled,
            now + 1_000,
            "continuation:1".into(),
            TaskRunStatus::Queued,
            None,
            now,
        );
        assert_eq!(task.requester_session_id, Some(session_id));
        assert_eq!(task.execution_session_id, None);
        assert_eq!(task.run_id, None);
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

    include!("service_system_clock_soak_test.rs");
}
