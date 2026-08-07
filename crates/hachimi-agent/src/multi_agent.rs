//! Typed Multi-Agent tools backed by the single AgentRunExecutor.

use std::{
    collections::HashSet,
    sync::{Arc, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use hachimi_protocol::{
    AgentTaskId, AgentTaskMessageId, AgentTaskMessageRecord, AgentTaskRecord, AgentTaskStatus,
    AuthorityMode, BehaviorMode, CapabilityGrantSet, ClientId, FileSystemAccess, MutationContext,
    PermissionGrantScope, PermissionProfile, ProcessGrant, RequestId, RunBudget, RunId, RunPurpose,
    RunRecoveryDecisionAction, RunRecoveryDecisionRequest, RunRecoveryState, RunStatus, SkillId,
    ToolDescriptor, ToolEffect, ToolRecoveryPolicy,
};
#[cfg(test)]
use hachimi_protocol::{EntryProfile, RunOrigin};
use hachimi_storage::AgentStore;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::{
    AgentRunCreateRequest, AgentRunExecutor, AgentRunLaunchRequest, AgentRunLauncher,
    AgentRunPriority, AgentRunRequest, ToolExecutionError, ToolExecutor, ToolFuture,
    ToolInvocation, ToolResult, UserInputAvailability,
};

pub const AGENT_SPAWN_TOOL: &str = "agent.spawn";
pub const AGENT_SEND_TOOL: &str = "agent.send";
pub const AGENT_WAIT_TOOL: &str = "agent.wait";
pub const AGENT_CANCEL_TOOL: &str = "agent.cancel";
pub const AGENT_COLLECT_TOOL: &str = "agent.collect";

#[derive(Clone)]
pub struct MultiAgentCoordinator {
    store: AgentStore,
    executor: Arc<OnceLock<AgentRunExecutor>>,
    active_tasks: Arc<Mutex<HashSet<AgentTaskId>>>,
    lease_owner: Arc<str>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MultiAgentReconciliationReport {
    pub inspected: u64,
    pub resumed: u64,
    pub synchronized_terminal: u64,
    pub needs_attention: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub handled_recovery_run_ids: Vec<RunId>,
}

const TASK_LEASE_DURATION_MS: i64 = 30_000;
const TASK_LEASE_RENEW_MS: u64 = 10_000;

impl std::fmt::Debug for MultiAgentCoordinator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MultiAgentCoordinator")
            .finish_non_exhaustive()
    }
}

impl MultiAgentCoordinator {
    #[must_use]
    pub fn new(store: AgentStore) -> Self {
        Self {
            store,
            executor: Arc::new(OnceLock::new()),
            active_tasks: Arc::new(Mutex::new(HashSet::new())),
            lease_owner: format!("agent-runtime:{}:{}", std::process::id(), now_ms()).into(),
        }
    }

    pub fn install_executor(&self, executor: AgentRunExecutor) -> Result<(), AgentRunExecutor> {
        self.executor.set(executor)
    }

    #[must_use]
    pub fn tools_for_parent(&self, parent: AgentRunRequest) -> Vec<Arc<dyn ToolExecutor>> {
        if parent.agent_depth >= 3 {
            return Vec::new();
        }
        [
            AgentToolKind::Spawn,
            AgentToolKind::Send,
            AgentToolKind::Wait,
            AgentToolKind::Cancel,
            AgentToolKind::Collect,
        ]
        .into_iter()
        .map(|kind| {
            Arc::new(AgentTool {
                kind,
                coordinator: self.clone(),
                parent: parent.clone(),
            }) as Arc<dyn ToolExecutor>
        })
        .collect()
    }

    async fn launch_task_execution(
        &self,
        request: AgentRunRequest,
        cancellation: CancellationToken,
    ) -> Result<bool, String> {
        let executor = self
            .executor
            .get()
            .cloned()
            .ok_or_else(|| "multi_agent_executor_not_ready".to_owned())?;
        let task_id = request
            .parent_agent_task_id
            .clone()
            .ok_or_else(|| "multi_agent_task_identity_missing".to_owned())?;
        if !self.active_tasks.lock().insert(task_id.clone()) {
            return Ok(false);
        }
        let claim = match self
            .store
            .claim_agent_task_execution(
                &task_id,
                &self.lease_owner,
                now_ms(),
                TASK_LEASE_DURATION_MS,
            )
            .await
        {
            Ok(Some(claim)) => claim,
            Ok(None) => {
                self.active_tasks.lock().remove(&task_id);
                return Ok(false);
            }
            Err(error) => {
                self.active_tasks.lock().remove(&task_id);
                return Err(format!("agent_task_lease_claim_failed:{error}"));
            }
        };
        let coordinator = self.clone();
        let child_run_id = request.run.id.clone();
        let child_generation = request.run.generation;
        tokio::spawn(async move {
            let _ = coordinator
                .store
                .transition_agent_task(&task_id, AgentTaskStatus::Running, None, None, now_ms())
                .await;
            let execution = executor.execute(request);
            tokio::pin!(execution);
            let mut heartbeat = tokio::time::interval(Duration::from_millis(TASK_LEASE_RENEW_MS));
            heartbeat.tick().await;
            let mut lease_lost = false;
            loop {
                tokio::select! {
                    _ = cancellation.cancelled() => {
                        let _ = executor.registry().cancel(&child_run_id, child_generation);
                        let _ = execution.await;
                        break;
                    }
                    _ = &mut execution => break,
                    _ = heartbeat.tick() => {
                        match coordinator.store.renew_agent_task_execution_lease(
                            &task_id,
                            claim.execution_generation,
                            &claim.lease_owner,
                            now_ms(),
                            TASK_LEASE_DURATION_MS,
                        ).await {
                            Ok(true) => {}
                            _ => {
                                lease_lost = true;
                                let _ = executor.registry().cancel(&child_run_id, child_generation);
                                let _ = execution.await;
                                break;
                            }
                        }
                    }
                }
            }
            let _ = coordinator
                .store
                .reconcile_agent_task_from_run(&task_id, now_ms())
                .await;
            if lease_lost {
                let _ = coordinator
                    .store
                    .transition_agent_task(
                        &task_id,
                        AgentTaskStatus::Failed,
                        None,
                        Some("agent_task_lease_lost"),
                        now_ms(),
                    )
                    .await;
            }
            let _ = coordinator
                .store
                .release_agent_task_execution_lease(
                    &task_id,
                    claim.execution_generation,
                    &claim.lease_owner,
                    now_ms(),
                )
                .await;
            coordinator.active_tasks.lock().remove(&task_id);
        });
        Ok(true)
    }

    pub async fn reconcile_startup(&self) -> Result<MultiAgentReconciliationReport, String> {
        if self.executor.get().is_none() {
            return Err("multi_agent_executor_not_ready".into());
        }
        let tasks = self
            .store
            .list_nonterminal_agent_tasks()
            .await
            .map_err(|error| format!("agent_task_reconcile_list_failed:{error}"))?;
        let pending_recoveries = self
            .store
            .list_pending_run_recoveries()
            .await
            .map_err(|error| format!("agent_task_recovery_list_failed:{error}"))?;
        let mut report = MultiAgentReconciliationReport {
            inspected: u64::try_from(tasks.len()).unwrap_or(u64::MAX),
            ..Default::default()
        };
        for task in tasks {
            let Some(parent_run) = self
                .store
                .get_run(&task.parent_run_id)
                .await
                .map_err(|error| format!("agent_task_parent_lookup_failed:{error}"))?
            else {
                fail_reconciled_task(&self.store, &task, "agent_task_parent_missing").await;
                report.failed = report.failed.saturating_add(1);
                continue;
            };
            let Some(mut child_run) = self
                .store
                .get_run(&task.child_run_id)
                .await
                .map_err(|error| format!("agent_task_child_lookup_failed:{error}"))?
            else {
                fail_reconciled_task(&self.store, &task, "agent_task_child_run_missing").await;
                report.failed = report.failed.saturating_add(1);
                continue;
            };
            if parent_run.status == RunStatus::Cancelled {
                if !child_run.status.is_terminal()
                    && child_run.status.can_transition_to(RunStatus::Cancelled)
                {
                    let _ = self
                        .store
                        .transition_run(
                            &child_run.id,
                            RunStatus::Cancelled,
                            Some("parent_cancelled_during_reconciliation"),
                        )
                        .await;
                }
                let _ = self
                    .store
                    .transition_agent_task(
                        &task.id,
                        AgentTaskStatus::Cancelled,
                        None,
                        Some("parent_cancelled"),
                        now_ms(),
                    )
                    .await;
                report.cancelled = report.cancelled.saturating_add(1);
                continue;
            }
            if child_run.status.is_terminal() {
                let _ = self
                    .store
                    .reconcile_agent_task_from_run(&task.id, now_ms())
                    .await;
                report.synchronized_terminal = report.synchronized_terminal.saturating_add(1);
                continue;
            }

            let mut recovery_checkpoint = None;
            if matches!(
                child_run.status,
                RunStatus::Recovering | RunStatus::WaitingRecoveryDecision
            ) {
                let Some(recovery) = pending_recoveries
                    .iter()
                    .find(|entry| entry.recovery.run_id == child_run.id)
                    .cloned()
                else {
                    fail_reconciled_task(&self.store, &task, "agent_task_recovery_record_missing")
                        .await;
                    report.failed = report.failed.saturating_add(1);
                    continue;
                };
                // Every child Agent run is launched with unattended authority,
                // regardless of the parent/source. It must never resume a
                // state that requires a user prompt after restart.
                if matches!(
                    recovery.recovery.previous_status,
                    RunStatus::WaitingApproval | RunStatus::WaitingUserInput
                ) {
                    let _ = self
                        .store
                        .transition_agent_task(
                            &task.id,
                            AgentTaskStatus::NeedsAttention,
                            None,
                            Some("unattended_child_interaction_required"),
                            now_ms(),
                        )
                        .await;
                    report.needs_attention = report.needs_attention.saturating_add(1);
                    continue;
                }
                if recovery.recovery.state != RunRecoveryState::EligibleAuto {
                    fail_reconciled_task(&self.store, &task, "agent_task_recovery_not_provable")
                        .await;
                    report.failed = report.failed.saturating_add(1);
                    continue;
                }
                let principal = ClientId("system:multi-agent-recovery".into());
                let resolved = self
                    .store
                    .resolve_run_recovery(
                        &RunRecoveryDecisionRequest {
                            context: MutationContext {
                                request_id: RequestId(format!(
                                    "agent-task-recovery:{}",
                                    recovery.recovery.id
                                )),
                                client_id: principal.clone(),
                                protocol_version: hachimi_protocol::CONTROL_PROTOCOL_VERSION,
                                idempotency_key: format!(
                                    "agent-task-recovery:{}",
                                    recovery.recovery.id
                                ),
                                expected_run_id: Some(child_run.id.clone()),
                                expected_generation: Some(recovery.recovery.interrupted_generation),
                            },
                            recovery_id: recovery.recovery.id.clone(),
                            expected_run_id: child_run.id.clone(),
                            expected_interrupted_generation: recovery
                                .recovery
                                .interrupted_generation,
                            action: RunRecoveryDecisionAction::ResumeSafeRemainder,
                        },
                        &principal.0,
                        now_ms(),
                    )
                    .await
                    .map_err(|error| format!("agent_task_recovery_resolve_failed:{error}"))?;
                recovery_checkpoint = resolved.checkpoint;
                report.handled_recovery_run_ids.push(child_run.id.clone());
                child_run = self
                    .store
                    .get_run(&child_run.id)
                    .await
                    .map_err(|error| format!("agent_task_child_reload_failed:{error}"))?
                    .ok_or_else(|| "agent_task_child_run_missing_after_recovery".to_owned())?;
            }
            if child_run.status != RunStatus::Queued {
                fail_reconciled_task(&self.store, &task, "agent_task_child_not_restartable").await;
                report.failed = report.failed.saturating_add(1);
                continue;
            }
            let request = match self
                .recovered_agent_request(&task, child_run, recovery_checkpoint)
                .await
            {
                Ok(request) => request,
                Err(code) => {
                    fail_reconciled_task(&self.store, &task, &code).await;
                    report.failed = report.failed.saturating_add(1);
                    continue;
                }
            };
            if self
                .launch_task_execution(request, CancellationToken::new())
                .await?
            {
                report.resumed = report.resumed.saturating_add(1);
            }
        }
        Ok(report)
    }

    async fn recovered_agent_request(
        &self,
        task: &AgentTaskRecord,
        run: hachimi_protocol::RunRecord,
        recovery_checkpoint: Option<hachimi_protocol::RunStepCheckpoint>,
    ) -> Result<AgentRunRequest, String> {
        let session = self
            .store
            .get_session(&task.child_session_id)
            .await
            .map_err(|error| format!("agent_task_session_lookup_failed:{error}"))?
            .ok_or_else(|| "agent_task_child_session_missing".to_owned())?;
        let mut grants = self
            .store
            .latest_capability_grants_snapshot(&run.id)
            .await
            .map_err(|error| format!("agent_task_grant_lookup_failed:{error}"))?
            .ok_or_else(|| "agent_task_recovery_grant_snapshot_missing".to_owned())?;
        if grants.session_id != session.id
            || grants.run_id.as_ref() != Some(&run.id)
            || grants
                .expires_at_ms
                .is_some_and(|expires| expires <= now_ms())
        {
            return Err("agent_task_recovery_grant_invalid".into());
        }
        let authority = self
            .store
            .authority_snapshot(&run.id)
            .await
            .map_err(|error| format!("agent_task_authority_lookup_failed:{error}"))?
            .filter(|authority| authority.session_id == session.id && authority.run_id == run.id)
            .ok_or_else(|| "agent_task_recovery_authority_snapshot_missing".to_owned())?;
        let sandbox = self
            .store
            .latest_sandbox_report(&run.id)
            .await
            .map_err(|error| format!("agent_task_sandbox_lookup_failed:{error}"))?
            .ok_or_else(|| "agent_task_recovery_sandbox_snapshot_missing".to_owned())?;
        grants.source = format!("multi_agent_recovery:{}", task.id);
        self.store
            .persist_run_security_snapshot(&grants, &sandbox, now_ms())
            .await
            .map_err(|error| format!("agent_task_security_reissue_failed:{error}"))?;
        let attachment_ids = self
            .store
            .list_run_managed_attachments(&run.id)
            .await
            .map_err(|error| format!("agent_task_attachment_lookup_failed:{error}"))?
            .into_iter()
            .map(|attachment| attachment.attachment.id)
            .collect();
        // Child runs are always unattended, so Host revision drift must be
        // pinned for every source, not only Scheduled runs.
        let host_revision_snapshot = Some(Default::default());
        Ok(AgentRunRequest {
            principal: "system:multi-agent-recovery".into(),
            session,
            run,
            authority,
            priority: AgentRunPriority::Background,
            user_input_availability: UserInputAvailability::Unavailable,
            capability_grants: grants,
            sandbox_snapshot: sandbox,
            attachment_ids,
            skill_allowlist: Vec::new(),
            mcp_tool_allowlist: Vec::new(),
            // Tool/Skill/MCP allowlists were not present before v29; restart is
            // deliberately fail-closed while the model consumes durable results.
            run_tool_allowlist: Some(Vec::new()),
            host_revision_snapshot,
            workload_override: None,
            recovery_checkpoint,
            parent_agent_task_id: Some(task.id.clone()),
            parent_run_id: Some(task.parent_run_id.clone()),
            agent_depth: task.depth,
        })
    }
}

async fn fail_reconciled_task(store: &AgentStore, task: &AgentTaskRecord, code: &str) {
    if let Ok(Some(run)) = store.get_run(&task.child_run_id).await
        && !run.status.is_terminal()
        && run.status.can_transition_to(RunStatus::Failed)
    {
        let _ = store
            .transition_run(&run.id, RunStatus::Failed, Some(code))
            .await;
    }
    let _ = store
        .transition_agent_task(
            &task.id,
            AgentTaskStatus::Failed,
            None,
            Some(code),
            now_ms(),
        )
        .await;
}

#[derive(Debug, Clone, Copy)]
enum AgentToolKind {
    Spawn,
    Send,
    Wait,
    Cancel,
    Collect,
}

struct AgentTool {
    kind: AgentToolKind,
    coordinator: MultiAgentCoordinator,
    parent: AgentRunRequest,
}

impl ToolExecutor for AgentTool {
    fn descriptor(&self) -> ToolDescriptor {
        let (name, description, effect, schema) = match self.kind {
            AgentToolKind::Spawn => (
                AGENT_SPAWN_TOOL,
                "Spawn a bounded child Agent Task with narrower-or-equal permissions and reserved budget.",
                ToolEffect::ExternalSideEffect,
                json!({
                    "type": "object",
                    "properties": {
                        "title": {"type": "string", "minLength": 1, "maxLength": 200},
                        "prompt": {"type": "string", "minLength": 1, "maxLength": 32000},
                        "permissionProfile": {"type": "string", "enum": ["read_only", "inherit"]},
                        "maxModelRequests": {"type": "integer", "minimum": 1},
                        "maxToolCalls": {"type": "integer", "minimum": 1},
                        "toolAllowlist": {"type": "array", "items": {"type": "string"}},
                        "skillIds": {"type": "array", "items": {"type": "string"}}
                    },
                    "required": ["title", "prompt"],
                    "additionalProperties": false
                }),
            ),
            AgentToolKind::Send => (
                AGENT_SEND_TOOL,
                "Send bounded steering content to one active child Task.",
                ToolEffect::ExternalSideEffect,
                task_schema(true),
            ),
            AgentToolKind::Wait => (
                AGENT_WAIT_TOOL,
                "Wait for selected child Tasks to finish or require attention.",
                ToolEffect::ReadOnly,
                json!({
                    "type": "object",
                    "properties": {
                        "taskIds": {"type": "array", "items": {"type": "string"}},
                        "timeoutMs": {"type": "integer", "minimum": 0, "maximum": 60000}
                    },
                    "additionalProperties": false
                }),
            ),
            AgentToolKind::Cancel => (
                AGENT_CANCEL_TOOL,
                "Cancel one child Task and all of its descendants.",
                ToolEffect::ExternalSideEffect,
                task_schema(false),
            ),
            AgentToolKind::Collect => (
                AGENT_COLLECT_TOOL,
                "Collect child status, bounded summaries, usage, artifacts, and messages.",
                ToolEffect::ReadOnly,
                json!({"type": "object", "properties": {}, "additionalProperties": false}),
            ),
        };
        ToolDescriptor {
            name: name.into(),
            description: description.into(),
            input_schema: schema,
            effect,
            parallel_safe: matches!(self.kind, AgentToolKind::Wait | AgentToolKind::Collect),
            required_scopes: vec!["agent.run".into()],
        }
    }

    fn recovery_policy(&self) -> ToolRecoveryPolicy {
        match self.kind {
            AgentToolKind::Wait | AgentToolKind::Collect => ToolRecoveryPolicy::ReadOnlyReplayable,
            AgentToolKind::Spawn | AgentToolKind::Send | AgentToolKind::Cancel => {
                ToolRecoveryPolicy::IdempotentWithReceipt
            }
        }
    }

    fn execute(&self, invocation: ToolInvocation) -> ToolFuture {
        let coordinator = self.coordinator.clone();
        let parent = self.parent.clone();
        let kind = self.kind;
        Box::pin(async move {
            match kind {
                AgentToolKind::Spawn => spawn_child(coordinator, parent, invocation).await,
                AgentToolKind::Send => send_child(coordinator, parent, invocation).await,
                AgentToolKind::Wait => wait_children(coordinator, parent, invocation).await,
                AgentToolKind::Cancel => cancel_child(coordinator, parent, invocation).await,
                AgentToolKind::Collect => collect_children(coordinator, parent, invocation).await,
            }
        })
    }
}

async fn spawn_child(
    coordinator: MultiAgentCoordinator,
    parent: AgentRunRequest,
    invocation: ToolInvocation,
) -> Result<ToolResult, ToolExecutionError> {
    if coordinator.executor.get().is_none() {
        return Err(failed("Multi-Agent executor is not ready"));
    }
    let title = required_string(&invocation.call.arguments, "title", 200)?;
    let prompt = required_string(&invocation.call.arguments, "prompt", 32_000)?;
    let permission =
        optional_string(&invocation.call.arguments, "permissionProfile")?.unwrap_or("read_only");
    let permission_profile = match permission {
        "read_only" => PermissionProfile::ReadOnly,
        "inherit" => parent.run.configuration.permission_profile,
        _ => return Err(failed("permissionProfile must be read_only or inherit")),
    };
    if permission_profile != PermissionProfile::ReadOnly
        && permission_profile != parent.run.configuration.permission_profile
    {
        return Err(failed("child permission profile cannot widen the parent"));
    }
    let budget = child_budget(&parent.run.configuration.budget, &invocation.call.arguments)?;
    let requested_tool_allowlist = string_array(&invocation.call.arguments, "toolAllowlist")?;
    let requested_skills = string_array(&invocation.call.arguments, "skillIds")?;
    let tool_allowlist = intersect_names(
        parent.run_tool_allowlist.as_deref(),
        requested_tool_allowlist.as_deref(),
    );
    let skill_allowlist = intersect_skills(&parent.skill_allowlist, requested_skills.as_deref());
    let now = now_ms();
    let mut child_policy = parent.authority.policy.clone();
    child_policy.level = permission_profile;
    let launched = AgentRunLauncher::new(coordinator.store.clone())
        .launch_new_transient_policy(AgentRunLaunchRequest {
            create: AgentRunCreateRequest {
                principal: parent.principal.clone(),
                idempotency_key: format!("agent-spawn:{}:{}", parent.run.id, invocation.call.id),
                context: parent.session.context.clone(),
                origin: parent.run.origin.clone(),
                title: title.clone(),
                prompt,
                attachment_ids: Vec::new(),
                parent_session_id: Some(parent.session.id.clone()),
                source_run_id: Some(parent.run.id.clone()),
                purpose: RunPurpose::Task,
                model_snapshot: parent.run.configuration.model_snapshot.clone(),
                entry_profile: parent.run.configuration.entry_profile,
                workload_override: parent.workload_override,
                behavior_mode: BehaviorMode::Default,
                execution_target: parent.run.configuration.execution_target.clone(),
                approval_policy: parent.run.configuration.approval_policy,
                permission_profile,
                budget: budget.clone(),
                requested_capabilities: parent.run.requested_capabilities,
                created_at_ms: now,
            },
            policy: child_policy,
            authority_mode: AuthorityMode::Unattended,
        })
        .await
        .map_err(|error| failed(error.to_string()))?;
    let created = launched.created;
    if let Some(existing) = coordinator
        .store
        .get_agent_task_by_child_run(&created.run.id)
        .await
        .map_err(store_failed)?
    {
        return Ok(ToolResult::succeeded(
            &invocation.call,
            format!("reused child task {}", existing.id),
            serde_json::to_value(existing).unwrap_or_default(),
        ));
    }
    let task_id = AgentTaskId::random();
    let parent_task = match parent.parent_agent_task_id.as_ref() {
        Some(id) => coordinator
            .store
            .get_agent_task(id)
            .await
            .map_err(store_failed)?,
        None => None,
    };
    let task = AgentTaskRecord {
        id: task_id.clone(),
        root_task_id: parent_task
            .as_ref()
            .map(|task| task.root_task_id.clone())
            .unwrap_or_else(|| task_id.clone()),
        root_run_id: parent_task
            .as_ref()
            .map(|task| task.root_run_id.clone())
            .unwrap_or_else(|| parent.run.id.clone()),
        parent_task_id: parent.parent_agent_task_id.clone(),
        parent_session_id: parent.session.id.clone(),
        parent_run_id: parent.run.id.clone(),
        child_session_id: created.session.id.clone(),
        child_run_id: created.run.id.clone(),
        title,
        depth: parent.agent_depth.saturating_add(1),
        status: AgentTaskStatus::Queued,
        reserved_budget: budget,
        usage: Default::default(),
        artifact_ids: Vec::new(),
        result_summary: None,
        error_code: None,
        created_at_ms: now,
        started_at_ms: None,
        finished_at_ms: None,
        updated_at_ms: now,
    };
    let child_grants = narrow_grants(
        &parent.capability_grants,
        &created.session.id,
        &created.run.id,
        permission_profile,
    );
    let child_authority = launched.authority;
    coordinator
        .store
        .persist_run_security_snapshot(&child_grants, &parent.sandbox_snapshot, now)
        .await
        .map_err(store_failed)?;
    coordinator
        .store
        .create_agent_task(&task)
        .await
        .map_err(store_failed)?;
    let child_request = AgentRunRequest {
        principal: parent.principal.clone(),
        session: created.session,
        run: created.run,
        authority: child_authority,
        priority: AgentRunPriority::Background,
        user_input_availability: UserInputAvailability::Unavailable,
        capability_grants: child_grants,
        sandbox_snapshot: parent.sandbox_snapshot,
        attachment_ids: Vec::new(),
        skill_allowlist,
        mcp_tool_allowlist: parent.mcp_tool_allowlist,
        run_tool_allowlist: tool_allowlist,
        host_revision_snapshot: parent.host_revision_snapshot,
        workload_override: parent.workload_override,
        recovery_checkpoint: None,
        parent_agent_task_id: Some(task_id.clone()),
        parent_run_id: Some(parent.run.id.clone()),
        agent_depth: parent.agent_depth.saturating_add(1),
    };
    let started = coordinator
        .launch_task_execution(child_request, invocation.cancellation.child_token())
        .await
        .map_err(failed)?;
    if !started {
        return Ok(ToolResult::succeeded(
            &invocation.call,
            format!("reused leased child task {task_id}"),
            serde_json::to_value(task).unwrap_or_default(),
        ));
    }
    Ok(ToolResult::succeeded(
        &invocation.call,
        format!("spawned child task {task_id}"),
        serde_json::to_value(task).unwrap_or_default(),
    ))
}

async fn send_child(
    coordinator: MultiAgentCoordinator,
    parent: AgentRunRequest,
    invocation: ToolInvocation,
) -> Result<ToolResult, ToolExecutionError> {
    let task = owned_task(
        &coordinator.store,
        &parent.run.id,
        &invocation.call.arguments,
    )
    .await?;
    let content = required_string(&invocation.call.arguments, "content", 32_000)?;
    let run = coordinator
        .store
        .get_run(&task.child_run_id)
        .await
        .map_err(store_failed)?
        .ok_or_else(|| failed("child Run not found"))?;
    if run.status.is_terminal() {
        return Ok(ToolResult::rejected(
            &invocation.call,
            "child Task is already terminal",
        ));
    }
    let now = now_ms();
    let message = AgentTaskMessageRecord {
        id: AgentTaskMessageId::random(),
        task_id: task.id.clone(),
        sender_run_id: parent.run.id,
        recipient_run_id: task.child_run_id.clone(),
        content: content.clone(),
        created_at_ms: now,
        delivered_at_ms: Some(now),
    };
    coordinator
        .store
        .append_agent_task_message(&message)
        .await
        .map_err(store_failed)?;
    coordinator
        .store
        .enqueue_run_steer(
            &task.child_run_id,
            &task.child_run_id,
            run.generation,
            &content,
            now,
        )
        .await
        .map_err(store_failed)?;
    Ok(ToolResult::succeeded(
        &invocation.call,
        "message delivered to child Task",
        serde_json::to_value(message).unwrap_or_default(),
    ))
}

async fn wait_children(
    coordinator: MultiAgentCoordinator,
    parent: AgentRunRequest,
    invocation: ToolInvocation,
) -> Result<ToolResult, ToolExecutionError> {
    let requested = string_array(&invocation.call.arguments, "taskIds")?;
    let timeout = invocation
        .call
        .arguments
        .get("timeoutMs")
        .and_then(Value::as_u64)
        .unwrap_or(30_000)
        .min(60_000);
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout);
    loop {
        let mut tasks = coordinator
            .store
            .list_agent_tasks_for_parent(&parent.run.id)
            .await
            .map_err(store_failed)?;
        if let Some(ids) = requested.as_ref() {
            tasks.retain(|task| ids.iter().any(|id| id == task.id.as_str()));
        }
        for task in &tasks {
            let _ = coordinator
                .store
                .reconcile_agent_task_from_run(&task.id, now_ms())
                .await;
        }
        tasks = coordinator
            .store
            .list_agent_tasks_for_parent(&parent.run.id)
            .await
            .map_err(store_failed)?;
        if let Some(ids) = requested.as_ref() {
            tasks.retain(|task| ids.iter().any(|id| id == task.id.as_str()));
        }
        if tasks
            .iter()
            .all(|task| task.status.is_terminal() || task.status == AgentTaskStatus::NeedsAttention)
            || tokio::time::Instant::now() >= deadline
        {
            return Ok(ToolResult::succeeded(
                &invocation.call,
                "child Task wait completed",
                json!({"tasks": tasks, "timedOut": tokio::time::Instant::now() >= deadline}),
            ));
        }
        tokio::select! {
            _ = invocation.cancellation.cancelled() => return Ok(ToolResult::aborted(&invocation.call, "wait cancelled")),
            () = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
}

async fn cancel_child(
    coordinator: MultiAgentCoordinator,
    parent: AgentRunRequest,
    invocation: ToolInvocation,
) -> Result<ToolResult, ToolExecutionError> {
    let task = owned_task(
        &coordinator.store,
        &parent.run.id,
        &invocation.call.arguments,
    )
    .await?;
    let subtree = coordinator
        .store
        .list_agent_task_subtree(&task.id)
        .await
        .map_err(store_failed)?;
    for descendant in subtree {
        let run = coordinator
            .store
            .get_run(&descendant.child_run_id)
            .await
            .map_err(store_failed)?
            .ok_or_else(|| failed("child Run not found"))?;
        if !run.status.is_terminal() {
            if let Some(executor) = coordinator.executor.get() {
                let _ = executor.registry().cancel(&run.id, run.generation);
            }
            if run.status.can_transition_to(RunStatus::Cancelled) {
                let _ = coordinator
                    .store
                    .transition_run(&run.id, RunStatus::Cancelled, Some("parent_cancelled"))
                    .await;
            }
        }
        if !descendant.status.is_terminal() {
            let _ = coordinator
                .store
                .transition_agent_task(
                    &descendant.id,
                    AgentTaskStatus::Cancelled,
                    None,
                    Some("parent_cancelled"),
                    now_ms(),
                )
                .await;
        }
    }
    let task = coordinator
        .store
        .get_agent_task(&task.id)
        .await
        .map_err(store_failed)?
        .ok_or_else(|| failed("child Task not found after cancellation"))?;
    Ok(ToolResult::succeeded(
        &invocation.call,
        "child Task cancelled",
        serde_json::to_value(task).unwrap_or_default(),
    ))
}

async fn collect_children(
    coordinator: MultiAgentCoordinator,
    parent: AgentRunRequest,
    invocation: ToolInvocation,
) -> Result<ToolResult, ToolExecutionError> {
    for task in coordinator
        .store
        .list_agent_tasks_for_parent(&parent.run.id)
        .await
        .map_err(store_failed)?
    {
        let _ = coordinator
            .store
            .reconcile_agent_task_from_run(&task.id, now_ms())
            .await;
    }
    let collection = coordinator
        .store
        .collect_agent_tasks(&parent.run.id)
        .await
        .map_err(store_failed)?;
    let summaries = collection
        .tasks
        .iter()
        .map(|task| {
            format!(
                "{}: {:?}: {}",
                task.id,
                task.status,
                task.result_summary.as_deref().unwrap_or("no summary")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(ToolResult::succeeded(
        &invocation.call,
        summaries,
        serde_json::to_value(collection).unwrap_or_default(),
    ))
}

async fn owned_task(
    store: &AgentStore,
    parent_run_id: &RunId,
    arguments: &Value,
) -> Result<AgentTaskRecord, ToolExecutionError> {
    let id = AgentTaskId::new(required_string(arguments, "taskId", 128)?);
    let task = store
        .get_agent_task(&id)
        .await
        .map_err(store_failed)?
        .ok_or_else(|| failed("child Task not found"))?;
    if &task.parent_run_id != parent_run_id {
        return Err(failed("child Task does not belong to this parent Run"));
    }
    Ok(task)
}

fn narrow_grants(
    parent: &CapabilityGrantSet,
    child_session_id: &hachimi_protocol::SessionId,
    child_run_id: &RunId,
    profile: PermissionProfile,
) -> CapabilityGrantSet {
    let mut grants = parent.clone();
    grants.profile = profile;
    grants.scope = PermissionGrantScope::Run;
    grants.session_id = child_session_id.clone();
    grants.run_id = Some(child_run_id.clone());
    grants.source = format!(
        "multi_agent_parent:{}",
        parent.run_id.as_ref().map_or("unknown", RunId::as_str)
    );
    if profile == PermissionProfile::ReadOnly {
        for file in &mut grants.file_system {
            if file.access == FileSystemAccess::Write {
                file.access = FileSystemAccess::Read;
            }
        }
        grants.process = ProcessGrant::default();
        grants.browser.act = false;
        grants.browser.upload = false;
        grants.browser.download = false;
        grants.browser.cookie_storage = false;
        grants.browser.cdp = false;
        grants.computer.act = false;
    }
    grants
}

fn child_budget(parent: &RunBudget, arguments: &Value) -> Result<RunBudget, ToolExecutionError> {
    let default_models = (parent.max_model_requests / 4).max(1);
    let default_tools = (parent.max_tool_calls / 4).max(1);
    let models = optional_u32(arguments, "maxModelRequests")?.unwrap_or(default_models);
    let tools = optional_u32(arguments, "maxToolCalls")?.unwrap_or(default_tools);
    if models == 0
        || tools == 0
        || models > parent.max_model_requests
        || tools > parent.max_tool_calls
    {
        return Err(failed(
            "child budget must be positive and no larger than the parent budget",
        ));
    }
    Ok(RunBudget {
        max_model_requests: models,
        max_tool_calls: tools,
        max_parallel_read_tools: parent.max_parallel_read_tools.min(4),
        model_timeout_ms: parent.model_timeout_ms,
        tool_timeout_ms: parent.tool_timeout_ms,
    })
}

fn intersect_names(parent: Option<&[String]>, requested: Option<&[String]>) -> Option<Vec<String>> {
    match (parent, requested) {
        (None, None) => None,
        (Some(parent), None) => Some(parent.to_vec()),
        (None, Some(requested)) => Some(requested.to_vec()),
        (Some(parent), Some(requested)) => Some(
            requested
                .iter()
                .filter(|name| parent.contains(name))
                .cloned()
                .collect(),
        ),
    }
}

fn intersect_skills(parent: &[SkillId], requested: Option<&[String]>) -> Vec<SkillId> {
    match requested {
        None => parent.to_vec(),
        Some(requested) => parent
            .iter()
            .filter(|id| requested.iter().any(|value| value == id.as_str()))
            .cloned()
            .collect(),
    }
}

fn task_schema(with_content: bool) -> Value {
    let mut properties =
        serde_json::Map::from_iter([("taskId".into(), json!({"type": "string", "minLength": 1}))]);
    let mut required = vec!["taskId"];
    if with_content {
        properties.insert(
            "content".into(),
            json!({"type": "string", "minLength": 1, "maxLength": 32000}),
        );
        required.push("content");
    }
    json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false})
}

fn required_string(
    arguments: &Value,
    field: &str,
    max: usize,
) -> Result<String, ToolExecutionError> {
    let value = arguments
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| failed(format!("{field} is required")))?;
    if value.chars().count() > max {
        return Err(failed(format!("{field} is too long")));
    }
    Ok(value.to_owned())
}

fn optional_string<'a>(
    arguments: &'a Value,
    field: &str,
) -> Result<Option<&'a str>, ToolExecutionError> {
    match arguments.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.as_str())),
        _ => Err(failed(format!("{field} must be a string"))),
    }
}

fn string_array(arguments: &Value, field: &str) -> Result<Option<Vec<String>>, ToolExecutionError> {
    let Some(value) = arguments.get(field) else {
        return Ok(None);
    };
    let values = value
        .as_array()
        .ok_or_else(|| failed(format!("{field} must be an array")))?;
    if values.len() > 256 {
        return Err(failed(format!("{field} is too large")));
    }
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| failed(format!("{field} must contain strings")))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn optional_u32(arguments: &Value, field: &str) -> Result<Option<u32>, ToolExecutionError> {
    match arguments.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| failed(format!("{field} must be a positive integer"))),
    }
}

fn store_failed(error: hachimi_storage::AgentStoreError) -> ToolExecutionError {
    failed(error.to_string())
}
fn failed(message: impl Into<String>) -> ToolExecutionError {
    ToolExecutionError::Failed(message.into())
}
fn now_ms() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as StdMutex};

    use futures_util::stream;
    use hachimi_protocol::{
        AgentPermissionPolicy, ApprovalPolicy, AuthorityMode, LlmSettings, ModelEvent,
        ModelFinishReason, ModelMessage, ModelRequest, ProviderCapabilities,
        RecoveryRevisionSnapshot, RunStepCheckpoint, RunStepCheckpointId, RunStepPhase,
        SandboxCapabilityReport, SandboxReadiness, ScopedPermissionRules, SessionContextBinding,
        TokenUsage, WorkloadKind, WorkloadResolution, WorkloadResolutionSource,
    };
    use hachimi_protocol::{BrowserGrant, ComputerGrant, FileSystemGrant, NetworkGrant, SessionId};
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::{
        AgentInstructionLayer, AgentPreparationFuture, AgentRunCreateRequest, AgentRunPreparer,
        ModelClientFuture, ModelEventStream, ModelRuntime, ModelRuntimeError, ModelRuntimeFactory,
        PreparedAgentRun, StepRuntimeState, StepWorldState,
    };

    #[derive(Debug, Default)]
    struct RecoveryModel {
        requests: StdMutex<u64>,
    }

    impl ModelRuntime for RecoveryModel {
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                text_input: true,
                streaming_usage: true,
                context_window: Some(16_384),
                max_output_tokens: Some(1_024),
                ..ProviderCapabilities::default()
            }
        }

        fn stream(
            &self,
            _request: ModelRequest,
            cancellation: CancellationToken,
        ) -> ModelEventStream {
            *self.requests.lock().expect("requests") += 1;
            if cancellation.is_cancelled() {
                return Box::pin(stream::iter([Err(ModelRuntimeError::Cancelled)]));
            }
            Box::pin(stream::iter([
                Ok(ModelEvent::TextDelta {
                    delta: "recovered child completed".into(),
                }),
                Ok(ModelEvent::Usage {
                    usage: TokenUsage {
                        input_tokens: 3,
                        output_tokens: 2,
                    },
                }),
                Ok(ModelEvent::Completed {
                    finish_reason: ModelFinishReason::Stop,
                }),
            ]))
        }
    }

    #[derive(Debug, Clone)]
    struct RecoveryFactory(Arc<RecoveryModel>);

    impl ModelRuntimeFactory for RecoveryFactory {
        fn create_session(
            &self,
            _configuration: &hachimi_protocol::RunConfiguration,
        ) -> ModelClientFuture {
            let model = Arc::clone(&self.0);
            Box::pin(async move { Ok(model as Arc<dyn crate::ModelClientSession>) })
        }
    }

    #[derive(Debug, Default)]
    struct RecoveryPreparer;

    impl AgentRunPreparer for RecoveryPreparer {
        fn prepare(
            &self,
            _request: AgentRunRequest,
            _checkpoint: Option<hachimi_protocol::CompactionCheckpoint>,
            _model: Arc<dyn ModelRuntime>,
            _cancellation: CancellationToken,
        ) -> AgentPreparationFuture {
            Box::pin(async move {
                Ok(PreparedAgentRun {
                    initial_messages: vec![ModelMessage::user("resume child")],
                    tool_executors: Vec::new(),
                    host_context: Some("multi-agent-recovery-test".into()),
                    state: StepRuntimeState::new(
                        StepWorldState {
                            context_revision: 1,
                            profile_revision: 1,
                            agents_revision: String::new(),
                            skills_revision: String::new(),
                            mcp_revision: String::new(),
                            host_revision: "multi-agent-test-host".into(),
                            instructions: Vec::<AgentInstructionLayer>::new().into(),
                            skill_activations: Vec::new().into(),
                            mcp_bindings: Vec::new().into(),
                            disabled_tool_names: Vec::new().into(),
                            diagnostics: Vec::new().into(),
                            sandbox: test_sandbox(),
                            host_ready: true,
                        },
                        WorkloadResolution {
                            workload: WorkloadKind::General,
                            source: WorkloadResolutionSource::GeneralFallback,
                            activated_skill_ids: Vec::new(),
                            reason: "multi-agent recovery test".into(),
                            classifier_revision: None,
                        },
                    ),
                    world_refresher: None,
                    diff_tracker: None,
                })
            })
        }
    }

    fn test_sandbox() -> SandboxCapabilityReport {
        SandboxCapabilityReport {
            backend: "test".into(),
            readiness: SandboxReadiness::Unavailable,
            os_enforced: false,
            filesystem_enforced: false,
            process_enforced: false,
            network_enforced: false,
            version: None,
            stable_error_code: Some("read_only_test".into()),
            diagnostics: Vec::new(),
        }
    }

    async fn create_test_run(
        store: &AgentStore,
        idempotency_key: &str,
        title: &str,
        origin: RunOrigin,
        parent: Option<(
            &hachimi_protocol::SessionRecord,
            &hachimi_protocol::RunRecord,
        )>,
    ) -> hachimi_storage::CreatedAgentRun {
        crate::AgentRunLauncher::new(store.clone())
            .launch_new(crate::AgentRunLaunchRequest {
                create: AgentRunCreateRequest {
                    principal: "test".into(),
                    idempotency_key: idempotency_key.into(),
                    context: parent.map_or_else(
                        || SessionContextBinding::Workspace {
                            workspace_id: hachimi_protocol::WorkspaceId::random(),
                        },
                        |(session, _)| session.context.clone(),
                    ),
                    origin,
                    title: title.into(),
                    prompt: title.into(),
                    attachment_ids: Vec::new(),
                    parent_session_id: parent.map(|(session, _)| session.id.clone()),
                    source_run_id: parent.map(|(_, run)| run.id.clone()),
                    purpose: RunPurpose::Task,
                    model_snapshot: LlmSettings::default(),
                    entry_profile: EntryProfile::Workbench,
                    workload_override: None,
                    behavior_mode: BehaviorMode::Default,
                    execution_target: None,
                    approval_policy: ApprovalPolicy::NeverPrompt,
                    permission_profile: PermissionProfile::ReadOnly,
                    budget: RunBudget::default(),
                    requested_capabilities: ProviderCapabilities {
                        text_input: true,
                        streaming_usage: true,
                        ..ProviderCapabilities::default()
                    },
                    created_at_ms: now_ms(),
                },
                policy: AgentPermissionPolicy {
                    level: PermissionProfile::ReadOnly,
                    rules: ScopedPermissionRules::default(),
                    revision: 0,
                },
                authority_mode: AuthorityMode::Unattended,
            })
            .await
            .expect("test run")
            .created
    }

    fn provider_revision(capabilities: &ProviderCapabilities) -> String {
        Sha256::digest(serde_json::to_vec(capabilities).expect("capabilities"))
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn test_task(
        id: &str,
        parent: &hachimi_storage::CreatedAgentRun,
        child: &hachimi_storage::CreatedAgentRun,
        status: AgentTaskStatus,
    ) -> AgentTaskRecord {
        AgentTaskRecord {
            id: AgentTaskId::from(id),
            root_task_id: AgentTaskId::from(id),
            root_run_id: parent.run.id.clone(),
            parent_task_id: None,
            parent_session_id: parent.session.id.clone(),
            parent_run_id: parent.run.id.clone(),
            child_session_id: child.session.id.clone(),
            child_run_id: child.run.id.clone(),
            title: id.into(),
            depth: 1,
            status,
            reserved_budget: RunBudget {
                max_model_requests: 2,
                max_tool_calls: 2,
                ..RunBudget::default()
            },
            usage: Default::default(),
            artifact_ids: Vec::new(),
            result_summary: None,
            error_code: None,
            created_at_ms: now_ms(),
            started_at_ms: None,
            finished_at_ms: None,
            updated_at_ms: now_ms(),
        }
    }

    #[tokio::test]
    async fn startup_reconciliation_reattaches_safe_child_and_syncs_usage() {
        let fixture = tempfile::tempdir().expect("fixture");
        let database = fixture.path().join("agent.sqlite3");
        let store = AgentStore::connect(&database).await.expect("store");
        let parent = create_test_run(&store, "parent", "Parent", RunOrigin::Manual, None).await;
        let child = create_test_run(
            &store,
            "child",
            "Child",
            RunOrigin::Manual,
            Some((&parent.session, &parent.run)),
        )
        .await;
        let task = AgentTaskRecord {
            id: AgentTaskId::from("task-recovery"),
            root_task_id: AgentTaskId::from("task-recovery"),
            root_run_id: parent.run.id.clone(),
            parent_task_id: None,
            parent_session_id: parent.session.id.clone(),
            parent_run_id: parent.run.id.clone(),
            child_session_id: child.session.id.clone(),
            child_run_id: child.run.id.clone(),
            title: "Recovered child".into(),
            depth: 1,
            status: AgentTaskStatus::Queued,
            reserved_budget: RunBudget {
                max_model_requests: 2,
                max_tool_calls: 2,
                ..RunBudget::default()
            },
            usage: Default::default(),
            artifact_ids: Vec::new(),
            result_summary: None,
            error_code: None,
            created_at_ms: now_ms(),
            started_at_ms: None,
            finished_at_ms: None,
            updated_at_ms: now_ms(),
        };
        let grants = hachimi_policy::expand_permission_profile(
            PermissionProfile::ReadOnly,
            BehaviorMode::Default,
            child.session.id.clone(),
            child.run.id.clone(),
            "general://multi-agent-recovery".into(),
        );
        store
            .persist_run_security_snapshot(&grants, &test_sandbox(), now_ms())
            .await
            .expect("security snapshot");
        store.create_agent_task(&task).await.expect("task");
        store
            .transition_agent_task(&task.id, AgentTaskStatus::Running, None, None, now_ms())
            .await
            .expect("task running");
        store
            .transition_run(&child.run.id, RunStatus::Preparing, None)
            .await
            .expect("preparing");
        store
            .transition_run(&child.run.id, RunStatus::Running, None)
            .await
            .expect("running");
        let model = Arc::new(RecoveryModel::default());
        let capabilities = model.capabilities();
        store
            .record_run_step_checkpoint(&RunStepCheckpoint {
                id: RunStepCheckpointId::random(),
                session_id: child.session.id.clone(),
                run_id: child.run.id.clone(),
                run_generation: child.run.generation,
                step_index: 1,
                phase: RunStepPhase::Sampling,
                tool_call_id: None,
                tool_name: None,
                side_effect_execution_id: None,
                recovery_policy: ToolRecoveryPolicy::ReadOnlyReplayable,
                parameter_hash: None,
                world_revision: "multi-agent-test-host".into(),
                provider_revision: provider_revision(&capabilities),
                revision_snapshot: RecoveryRevisionSnapshot {
                    host_revision: "multi-agent-test-host".into(),
                    provider_revision: provider_revision(&capabilities),
                    ..Default::default()
                },
                created_at_ms: now_ms(),
            })
            .await
            .expect("checkpoint");
        drop(store);

        let store = AgentStore::connect(&database)
            .await
            .expect("reopened store");
        let recovered = store
            .recover_interrupted()
            .await
            .expect("recover interrupted");
        assert!(recovered.auto_resume_run_ids.contains(&child.run.id));
        let coordinator = MultiAgentCoordinator::new(store.clone());
        let executor = AgentRunExecutor::new(
            store.clone(),
            Arc::new(crate::AgentExecutorRegistry::new(2)),
            Arc::new(RecoveryFactory(Arc::clone(&model))),
            Arc::new(RecoveryPreparer),
        );
        coordinator.install_executor(executor).expect("executor");
        let report = coordinator.reconcile_startup().await.expect("reconcile");
        assert_eq!(report.resumed, 1);
        assert_eq!(report.handled_recovery_run_ids, vec![child.run.id.clone()]);
        for _ in 0..100 {
            let current = store
                .get_agent_task(&task.id)
                .await
                .expect("task")
                .expect("task row");
            if current.status.is_terminal() {
                assert_eq!(current.status, AgentTaskStatus::Succeeded);
                assert_eq!(
                    current.usage,
                    TokenUsage {
                        input_tokens: 3,
                        output_tokens: 2,
                    }
                );
                assert_eq!(
                    current.result_summary.as_deref(),
                    Some("recovered child completed")
                );
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("reconciled child did not finish");
    }

    #[tokio::test]
    async fn startup_reconciliation_cascades_parent_cancellation() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let parent =
            create_test_run(&store, "cancel-parent", "Parent", RunOrigin::Manual, None).await;
        let child = create_test_run(
            &store,
            "cancel-child",
            "Child",
            RunOrigin::Manual,
            Some((&parent.session, &parent.run)),
        )
        .await;
        let task = test_task("task-cancel", &parent, &child, AgentTaskStatus::Queued);
        store.create_agent_task(&task).await.expect("task");
        store
            .transition_run(&parent.run.id, RunStatus::Cancelled, Some("user_cancelled"))
            .await
            .expect("cancel parent");
        let model = Arc::new(RecoveryModel::default());
        let coordinator = MultiAgentCoordinator::new(store.clone());
        coordinator
            .install_executor(AgentRunExecutor::new(
                store.clone(),
                Arc::new(crate::AgentExecutorRegistry::new(2)),
                Arc::new(RecoveryFactory(Arc::clone(&model))),
                Arc::new(RecoveryPreparer),
            ))
            .expect("executor");
        let report = coordinator.reconcile_startup().await.expect("reconcile");
        assert_eq!(report.cancelled, 1);
        assert_eq!(
            store
                .get_run(&child.run.id)
                .await
                .expect("child")
                .expect("child run")
                .status,
            RunStatus::Cancelled
        );
        assert_eq!(
            store
                .get_agent_task(&task.id)
                .await
                .expect("task")
                .expect("task row")
                .status,
            AgentTaskStatus::Cancelled
        );
        assert_eq!(*model.requests.lock().expect("requests"), 0);
    }

    #[tokio::test]
    async fn unattended_child_waiting_for_approval_becomes_needs_attention() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let parent = create_test_run(
            &store,
            "scheduled-parent",
            "Parent",
            RunOrigin::Manual,
            None,
        )
        .await;
        let child = create_test_run(
            &store,
            "background-child",
            "Child",
            RunOrigin::Channel {
                channel: "test".into(),
                account: "account".into(),
                peer: "peer".into(),
                thread: "thread".into(),
                message_id: hachimi_protocol::ChannelMessageId::from("message"),
            },
            Some((&parent.session, &parent.run)),
        )
        .await;
        let task = test_task("task-background", &parent, &child, AgentTaskStatus::Queued);
        store.create_agent_task(&task).await.expect("task");
        store
            .transition_agent_task(&task.id, AgentTaskStatus::Running, None, None, now_ms())
            .await
            .expect("task running");
        store
            .transition_run(&child.run.id, RunStatus::Preparing, None)
            .await
            .expect("preparing");
        store
            .transition_run(&child.run.id, RunStatus::Running, None)
            .await
            .expect("running");
        store
            .transition_run(&child.run.id, RunStatus::WaitingApproval, None)
            .await
            .expect("approval wait");
        store.recover_interrupted().await.expect("recover");
        let model = Arc::new(RecoveryModel::default());
        let coordinator = MultiAgentCoordinator::new(store.clone());
        coordinator
            .install_executor(AgentRunExecutor::new(
                store.clone(),
                Arc::new(crate::AgentExecutorRegistry::new(2)),
                Arc::new(RecoveryFactory(model)),
                Arc::new(RecoveryPreparer),
            ))
            .expect("executor");
        let report = coordinator.reconcile_startup().await.expect("reconcile");
        assert_eq!(report.needs_attention, 1);
        let reconciled = store
            .get_agent_task(&task.id)
            .await
            .expect("task")
            .expect("task row");
        assert_eq!(reconciled.status, AgentTaskStatus::NeedsAttention);
        assert_eq!(
            reconciled.error_code.as_deref(),
            Some("unattended_child_interaction_required")
        );
    }

    #[test]
    fn child_allowlists_are_monotonic_intersections() {
        let parent = vec!["workspace.read".into(), "browser.observe".into()];
        let requested = vec!["browser.observe".into(), "computer.act".into()];
        assert_eq!(
            intersect_names(Some(&parent), Some(&requested)),
            Some(vec!["browser.observe".into()])
        );
        assert_eq!(intersect_names(Some(&parent), None), Some(parent));

        let skills = vec![SkillId::from("safe"), SkillId::from("coding")];
        assert_eq!(
            intersect_skills(&skills, Some(&["coding".into(), "missing".into()])),
            vec![SkillId::from("coding")]
        );
    }

    #[test]
    fn read_only_child_grants_remove_every_mutating_authority() {
        let parent_run_id = RunId::from("parent-run");
        let parent = CapabilityGrantSet {
            profile: PermissionProfile::Writable,
            scope: PermissionGrantScope::Run,
            session_id: SessionId::from("parent-session"),
            run_id: Some(parent_run_id),
            source: "parent".into(),
            file_system: vec![FileSystemGrant {
                access: FileSystemAccess::Write,
                roots: vec!["C:\\workspace".into()],
                globs: Vec::new(),
                files: Vec::new(),
                special_roots: Vec::new(),
            }],
            network: NetworkGrant {
                enabled: true,
                hosts: vec!["example.test".into()],
                protocols: vec!["https".into()],
                unrestricted_hosts: false,
            },
            process: ProcessGrant {
                spawn: true,
                interactive: true,
                allowed_commands: vec!["git".into()],
                unrestricted_commands: false,
            },
            browser: BrowserGrant {
                observe: true,
                act: true,
                upload: true,
                download: true,
                cookie_storage: true,
                cdp: true,
                origins: vec!["https://example.test".into()],
                unrestricted_origins: false,
            },
            computer: ComputerGrant {
                observe: true,
                act: true,
                allowed_applications: vec!["editor".into()],
                unrestricted_targets: false,
                max_actions: Some(4),
            },
            review_each_command: false,
            expires_at_ms: Some(42),
        };
        let child = narrow_grants(
            &parent,
            &SessionId::from("child-session"),
            &RunId::from("child-run"),
            PermissionProfile::ReadOnly,
        );
        assert_eq!(child.file_system[0].access, FileSystemAccess::Read);
        assert!(!child.process.spawn && !child.process.interactive);
        assert!(child.browser.observe && !child.browser.act && !child.browser.cdp);
        assert!(child.computer.observe && !child.computer.act);
        assert!(
            child.network.enabled,
            "read-only network scope stays bounded by parent"
        );
        assert_eq!(child.profile, PermissionProfile::ReadOnly);
        assert_eq!(child.scope, PermissionGrantScope::Run);
        assert_eq!(child.run_id, Some(RunId::from("child-run")));
    }
}
