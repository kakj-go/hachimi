//! Policy, Approval, Sandbox-reporting, and Audit wrapper for capability tools.

use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use hachimi_approvals::ApprovalBroker;
use hachimi_audit::{AuditEvent, AuditSink};
use hachimi_policy::{
    PolicyContext, PolicyDecision, PolicyEngine, capability_grant_allows, file_system_grants_allow,
};
use hachimi_protocol::{
    ApprovalGrantScope, ApprovalId, ApprovalPolicy, ApprovalRequestRecord, ApprovalStatus,
    AuthorityMode, CapabilityGrantSet, ClientContext, FileSystemAccess, PermissionProfile,
    RunAuthoritySnapshot, RunId, RunStatus, Scope, SessionId, SideEffectExecutionId,
    SideEffectExecutionRecord, SideEffectExecutionStatus, ToolDescriptor, ToolEffect,
};
use hachimi_sandbox::SandboxStatus;
use hachimi_storage::{AgentStore, RecoveryToolFence};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{ToolCall, ToolExecutor, ToolFuture, ToolInvocation, ToolResult};

const APPROVAL_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone)]
pub struct PersistentAuditSink {
    store: AgentStore,
    principal: String,
    session_id: SessionId,
    run_id: RunId,
    run_generation: u64,
}

impl PersistentAuditSink {
    #[must_use]
    pub fn new(
        store: AgentStore,
        principal: impl Into<String>,
        session_id: SessionId,
        run_id: RunId,
        run_generation: u64,
    ) -> Self {
        Self {
            store,
            principal: principal.into(),
            session_id,
            run_id,
            run_generation,
        }
    }
}

impl AuditSink for PersistentAuditSink {
    fn record(&self, event: AuditEvent) {
        let store = self.store.clone();
        let principal = self.principal.clone();
        let session_id = self.session_id.clone();
        let run_id = self.run_id.clone();
        let generation = self.run_generation;
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = store
                    .append_audit_metadata(hachimi_storage::AuditMetadataRecord {
                        principal,
                        session_id: Some(session_id),
                        run_id: Some(run_id),
                        run_generation: Some(generation),
                        operation: event.operation.to_owned(),
                        target_summary: event
                            .target_summary
                            .unwrap_or_else(|| "tool:metadata_only".into()),
                        decision: event.outcome.to_owned(),
                        result_code: event
                            .result_code
                            .unwrap_or_else(|| event.outcome.to_owned()),
                        created_at_ms: now_ms(),
                    })
                    .await;
            });
        }
    }
}

#[derive(Clone)]
pub struct AuthorizedToolContext {
    pub client: ClientContext,
    pub principal: String,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub run_generation: u64,
    pub authority: RunAuthoritySnapshot,
    pub approval_policy: ApprovalPolicy,
    pub permission_profile: PermissionProfile,
    pub capability_grants: CapabilityGrantSet,
    pub capability_host: String,
    pub run_tool_allowlist: Option<Vec<String>>,
    pub sandbox_status: SandboxStatus,
    pub run_store: Option<AgentStore>,
    pub policy: Arc<dyn PolicyEngine>,
    pub approvals: Arc<dyn ApprovalBroker>,
    pub audit: Arc<dyn AuditSink>,
}

impl std::fmt::Debug for AuthorizedToolContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthorizedToolContext")
            .field("principal", &self.principal)
            .field("session_id", &self.session_id)
            .field("run_id", &self.run_id)
            .field("run_generation", &self.run_generation)
            .field("authority_mode", &self.authority.mode)
            .field("approval_policy", &self.approval_policy)
            .field("permission_profile", &self.permission_profile)
            .field("capability_host", &self.capability_host)
            .field("sandbox_status", &self.sandbox_status)
            .finish_non_exhaustive()
    }
}

#[must_use]
pub fn authorized_tool(
    inner: Arc<dyn ToolExecutor>,
    context: AuthorizedToolContext,
) -> Arc<dyn ToolExecutor> {
    Arc::new(AuthorizedTool { inner, context })
}

fn authority_rejection(
    context: &AuthorizedToolContext,
    call: &ToolCall,
    code: impl Into<String>,
    message: impl Into<String>,
) -> ToolResult {
    let code = code.into();
    let message = message.into();
    if context.authority.mode == AuthorityMode::Unattended {
        ToolResult::needs_attention(call, code, message)
    } else {
        ToolResult::rejected(call, message)
    }
}

struct AuthorizedTool {
    inner: Arc<dyn ToolExecutor>,
    context: AuthorizedToolContext,
}

impl ToolExecutor for AuthorizedTool {
    fn descriptor(&self) -> hachimi_protocol::ToolDescriptor {
        self.inner.descriptor()
    }

    fn execute(&self, invocation: ToolInvocation) -> ToolFuture {
        let inner = Arc::clone(&self.inner);
        let context = self.context.clone();
        Box::pin(async move {
            if invocation.cancellation.is_cancelled() {
                return Ok(ToolResult::aborted(
                    &invocation.call,
                    "tool call was cancelled before authorization",
                ));
            }
            if invocation.run_generation != context.run_generation {
                context
                    .audit
                    .record(AuditEvent::decision("tool.execute", "stale_generation"));
                return Ok(ToolResult::rejected(
                    &invocation.call,
                    "tool call belongs to a stale run generation",
                ));
            }
            let descriptor = inner.descriptor();
            if context
                .run_tool_allowlist
                .as_ref()
                .is_some_and(|allowlist| !allowlist.iter().any(|name| name == &descriptor.name))
            {
                context
                    .audit
                    .record(AuditEvent::decision("tool.execute", "run_allowlist_denied"));
                return Ok(authority_rejection(
                    &context,
                    &invocation.call,
                    "run_allowlist_denied",
                    "tool call is outside the Run tool allowlist",
                ));
            }
            let required_scope = descriptor
                .required_scopes
                .first()
                .and_then(|scope| parse_scope(scope))
                .ok_or_else(|| {
                    crate::ToolExecutionError::Failed(format!(
                        "tool {} has no valid required scope",
                        descriptor.name
                    ))
                })?;
            let resource =
                authority_resource_summary(&context, &descriptor, &invocation.call.arguments);
            let structured_decision =
                structured_authority_decision(&context, &descriptor, &invocation.call.arguments);
            let (effective_effect, forced_approval) = match structured_decision {
                StructuredAuthorityDecision::Allow { effective_effect } => (effective_effect, None),
                StructuredAuthorityDecision::RequireApproval {
                    effective_effect,
                    code,
                } => (effective_effect, Some(code)),
                StructuredAuthorityDecision::Deny { code } => {
                    context.audit.record(AuditEvent::decision(
                        "tool.execute",
                        "structured_authority_denied",
                    ));
                    return Ok(authority_rejection(
                        &context,
                        &invocation.call,
                        code,
                        format!("tool call exceeds its structured authority: {code}"),
                    ));
                }
            };
            if !capability_grant_allows(&context.capability_grants, effective_effect) {
                context.audit.record(AuditEvent::decision(
                    "tool.execute",
                    "capability_grant_denied",
                ));
                return Ok(authority_rejection(
                    &context,
                    &invocation.call,
                    "capability_grant_denied",
                    "tool call is outside the active capability grant set",
                ));
            }
            let side_effect = !matches!(
                effective_effect,
                ToolEffect::ReadOnly | ToolEffect::BrowserObserve | ToolEffect::ComputerObserve
            );
            let parameters = if side_effect {
                Some(parameter_hash(&invocation.call.arguments)?)
            } else {
                None
            };
            let mut recovery_idempotency_key = None;
            if let (Some(store), Some(parameters)) = (&context.run_store, parameters.as_deref()) {
                match store
                    .recovery_tool_fence(
                        &context.run_id,
                        invocation.run_generation,
                        &descriptor.name,
                        parameters,
                    )
                    .await
                    .map_err(|error| {
                        crate::ToolExecutionError::Failed(format!(
                            "run recovery fence lookup failed: {error}"
                        ))
                    })? {
                    Some(RecoveryToolFence::ReuseCompleted {
                        succeeded,
                        persisted_result,
                    }) => {
                        context.audit.record(AuditEvent::decision(
                            "tool.execute",
                            "recovery_result_reused",
                        ));
                        return Ok(recovered_side_effect_result(
                            &invocation.call,
                            succeeded,
                            persisted_result,
                        ));
                    }
                    Some(RecoveryToolFence::RetryWithIdempotencyKey(key)) => {
                        recovery_idempotency_key = Some(key);
                    }
                    None => {}
                }
            }
            let policy_context = PolicyContext {
                client: &context.client,
                method: None,
                required_scope,
                entry_profile: invocation.entry_profile,
                workload: invocation.workload,
                behavior_mode: invocation.behavior_mode,
                approval_policy: context.approval_policy,
                permission_profile: context.permission_profile,
                effect: effective_effect,
                action: &descriptor.name,
                resource: &resource,
                capability_host: Some(context.capability_host.as_str()),
            };
            let mut approval_id = None;
            let policy_decision = forced_approval.map_or_else(
                || context.policy.evaluate(&policy_context),
                |code| PolicyDecision::RequireApproval { code },
            );
            match policy_decision {
                PolicyDecision::Deny { code } => {
                    context
                        .audit
                        .record(AuditEvent::decision("tool.execute", "policy_denied"));
                    return Ok(authority_rejection(
                        &context,
                        &invocation.call,
                        code,
                        format!("tool call denied by policy: {code}"),
                    ));
                }
                PolicyDecision::RequireApproval { code } => {
                    let parameter_hash = parameter_hash(&invocation.call.arguments)?;
                    if let Some(store) = &context.run_store {
                        let idempotency_key =
                            format!("tool:{}:{}", invocation.run_generation, invocation.call.id);
                        if let Some(claim) = store
                            .get_side_effect_claim(
                                &context.run_id,
                                invocation.run_generation,
                                &idempotency_key,
                            )
                            .await
                            .map_err(|error| {
                                crate::ToolExecutionError::Failed(format!(
                                    "side-effect replay lookup failed: {error}"
                                ))
                            })?
                        {
                            if claim.record.tool_call_id != invocation.call.id
                                || claim.record.parameter_hash != parameter_hash
                            {
                                return Err(crate::ToolExecutionError::Failed(
                                    "side-effect replay parameters conflict with the durable claim"
                                        .into(),
                                ));
                            }
                            context.audit.record(AuditEvent::decision(
                                "tool.execute",
                                "side_effect_result_reused",
                            ));
                            return duplicate_side_effect_result(&invocation.call, claim);
                        }
                    }
                    let reusable = if context.authority.mode == AuthorityMode::Interactive {
                        if let Some(store) = &context.run_store {
                            store
                                .approved_session_tool_authority(
                                    &context.session_id,
                                    &descriptor.name,
                                    &resource,
                                    &context.capability_host,
                                )
                                .await
                                .map_err(|error| {
                                    crate::ToolExecutionError::Failed(format!(
                                        "session authority lookup failed: {error}"
                                    ))
                                })?
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    if let Some(reusable) = reusable {
                        context.audit.record(AuditEvent::decision(
                            "tool.execute",
                            "session_authority_reused",
                        ));
                        approval_id = Some(reusable.id);
                    } else {
                        let created_at_ms = now_ms();
                        let session_scope = context.authority.mode == AuthorityMode::Interactive;
                        let approval = ApprovalRequestRecord {
                            id: ApprovalId::random(),
                            session_id: context.session_id.clone(),
                            run_id: context.run_id.clone(),
                            tool_call_id: invocation.call.id.clone(),
                            run_generation: invocation.run_generation,
                            status: ApprovalStatus::Pending,
                            action: descriptor.name.clone(),
                            resource: resource.clone(),
                            parameter_hash: parameter_hash.clone(),
                            risk_summary: code.into(),
                            target_host: context.capability_host.clone(),
                            required_scopes: descriptor.required_scopes.clone(),
                            grant_scope: if session_scope {
                                ApprovalGrantScope::Session
                            } else {
                                ApprovalGrantScope::Once
                            },
                            uses_remaining: if session_scope { u32::MAX } else { 1 },
                            requester_principal: context.principal.clone(),
                            resolved_by: None,
                            expires_at_ms: (!session_scope).then(|| {
                                created_at_ms.saturating_add(
                                    i64::try_from(APPROVAL_TTL.as_millis()).unwrap_or(i64::MAX),
                                )
                            }),
                            created_at_ms,
                            resolved_at_ms: None,
                        };
                        context
                            .audit
                            .record(AuditEvent::decision("tool.execute", "approval_requested"));
                        enter_approval_wait(&context).await?;
                        let resolution = context
                            .approvals
                            .request(approval, invocation.cancellation.child_token())
                            .await;
                        leave_approval_wait(&context).await?;
                        let resolved = resolution.map_err(|error| {
                            crate::ToolExecutionError::Failed(format!(
                                "approval broker failed: {error}"
                            ))
                        })?;
                        if resolved.status != ApprovalStatus::Approved
                            || resolved.parameter_hash != parameter_hash
                            || resolved.run_generation != invocation.run_generation
                        {
                            context
                                .audit
                                .record(AuditEvent::decision("tool.execute", "approval_denied"));
                            return Ok(ToolResult::rejected(
                                &invocation.call,
                                "tool call was not approved for these exact parameters",
                            ));
                        }
                        approval_id = Some(resolved.id);
                    }
                }
                PolicyDecision::Allow => {}
            }
            if invocation.cancellation.is_cancelled() {
                return Ok(ToolResult::aborted(
                    &invocation.call,
                    "tool call was cancelled before sandbox admission",
                ));
            }
            if context.permission_profile != PermissionProfile::FullAccess
                && !context.sandbox_status.permits(effective_effect)
            {
                context
                    .audit
                    .record(AuditEvent::decision("tool.execute", "sandbox_unavailable"));
                return Ok(authority_rejection(
                    &context,
                    &invocation.call,
                    "sandbox_unavailable",
                    "tool side effect denied because no OS-enforced sandbox is active",
                ));
            }
            let mut execution_id = None;
            if side_effect && let Some(store) = &context.run_store {
                let parameters = parameters
                    .clone()
                    .expect("side-effect parameter hash must be present");
                let timestamp = now_ms();
                let record = SideEffectExecutionRecord {
                    id: SideEffectExecutionId::random(),
                    session_id: context.session_id.clone(),
                    run_id: context.run_id.clone(),
                    run_generation: invocation.run_generation,
                    tool_call_id: invocation.call.id.clone(),
                    idempotency_key: recovery_idempotency_key.unwrap_or_else(|| {
                        format!("tool:{}:{}", invocation.run_generation, invocation.call.id)
                    }),
                    parameter_hash: parameters,
                    approval_id,
                    host_request_id: None,
                    status: SideEffectExecutionStatus::Claimed,
                    result_code: None,
                    result_reference: None,
                    created_at_ms: timestamp,
                    updated_at_ms: timestamp,
                };
                let claim = store
                    .claim_side_effect_with_authority(
                        &record,
                        Some(hachimi_storage::SideEffectAuthority {
                            action: &descriptor.name,
                            resource: &resource,
                            target_host: &context.capability_host,
                        }),
                    )
                    .await
                    .map_err(|error| {
                        crate::ToolExecutionError::Failed(format!(
                            "side-effect claim failed: {error}"
                        ))
                    })?;
                if !claim.created {
                    return duplicate_side_effect_result(&invocation.call, claim);
                }
                if invocation.cancellation.is_cancelled() {
                    store
                        .cancel_claimed_side_effect(&record.id, now_ms())
                        .await
                        .map_err(|error| {
                            crate::ToolExecutionError::Failed(format!(
                                "side-effect cancellation record failed: {error}"
                            ))
                        })?;
                    return Ok(ToolResult::aborted(
                        &invocation.call,
                        "tool call was cancelled before host dispatch",
                    ));
                }
                if let Err(error) = store
                    .mark_side_effect_dispatched_if_current(
                        &record.id,
                        &record.run_id,
                        record.run_generation,
                        &format!("workspace:{}", record.id),
                        now_ms(),
                    )
                    .await
                {
                    let _ = store.cancel_claimed_side_effect(&record.id, now_ms()).await;
                    return Ok(ToolResult::aborted(
                        &invocation.call,
                        format!("host dispatch precondition failed: {error}"),
                    ));
                }
                execution_id = Some(record.id);
            }
            let execution_cancellation = invocation.cancellation.clone();
            let call = invocation.call.clone();
            if let Some(store) = &context.run_store {
                store
                    .dispatch_plugin_hook_event(
                        &hachimi_storage::PluginHookEventRecord {
                            event: "tool.before".into(),
                            session_id: Some(context.session_id.clone()),
                            run_id: Some(context.run_id.clone()),
                            run_generation: Some(context.run_generation),
                            subject: format!("{}:{}", descriptor.name, call.id),
                            result_code: "started".into(),
                            created_at_ms: now_ms(),
                        },
                        execution_cancellation.child_token(),
                    )
                    .await
                    .map_err(|error| {
                        crate::ToolExecutionError::Failed(format!(
                            "plugin tool.before hook failed: {error}"
                        ))
                    })?;
            }
            let result = inner.execute(invocation).await;
            if execution_cancellation.is_cancelled() {
                if let (Some(store), Some(execution_id)) =
                    (&context.run_store, execution_id.as_ref())
                {
                    store
                        .finish_side_effect(
                            execution_id,
                            SideEffectExecutionStatus::Indeterminate,
                            Some("late_result_after_cancellation"),
                            None,
                            None,
                            now_ms(),
                        )
                        .await
                        .map_err(|error| {
                            crate::ToolExecutionError::Failed(format!(
                                "late side-effect result record failed: {error}"
                            ))
                        })?;
                }
                return Ok(ToolResult::aborted(
                    &call,
                    "tool result arrived after cancellation and was discarded",
                ));
            }
            if let (Some(store), Some(execution_id)) = (&context.run_store, execution_id.as_ref()) {
                let (status, code, persisted) = side_effect_completion(&result);
                store
                    .finish_side_effect(
                        execution_id,
                        status,
                        Some(code),
                        None,
                        persisted.as_ref(),
                        now_ms(),
                    )
                    .await
                    .map_err(|error| {
                        crate::ToolExecutionError::Failed(format!(
                            "side-effect completion record failed: {error}"
                        ))
                    })?;
            }
            if let Some(store) = &context.run_store {
                store
                    .dispatch_plugin_hook_event(
                        &hachimi_storage::PluginHookEventRecord {
                            event: "tool.after".into(),
                            session_id: Some(context.session_id.clone()),
                            run_id: Some(context.run_id.clone()),
                            run_generation: Some(context.run_generation),
                            subject: format!("{}:{}", descriptor.name, call.id),
                            result_code: if result.is_ok() {
                                "succeeded".into()
                            } else {
                                "failed".into()
                            },
                            created_at_ms: now_ms(),
                        },
                        execution_cancellation.child_token(),
                    )
                    .await
                    .map_err(|error| {
                        crate::ToolExecutionError::Failed(format!(
                            "plugin tool.after hook failed: {error}"
                        ))
                    })?;
            }
            context.audit.record(
                AuditEvent::decision(
                    "tool.execute",
                    if result.is_ok() {
                        "completed"
                    } else {
                        "failed"
                    },
                )
                .with_metadata(
                    format!("{}:{}", context.capability_host, descriptor.name),
                    if result.is_ok() {
                        "completed"
                    } else {
                        "failed"
                    },
                ),
            );
            result
        })
    }

    fn waits_for_cancellation(&self) -> bool {
        // The wrapper itself can be blocked on a persisted approval even when the
        // wrapped tool does not need a cancellation grace period.
        true
    }
}

fn duplicate_side_effect_result(
    call: &crate::ToolCall,
    claim: hachimi_storage::SideEffectClaim,
) -> Result<ToolResult, crate::ToolExecutionError> {
    match claim.record.status {
        SideEffectExecutionStatus::Succeeded | SideEffectExecutionStatus::Failed => {
            let value = claim.persisted_result.unwrap_or_default();
            let model_content = value
                .get("modelContent")
                .and_then(Value::as_str)
                .unwrap_or("side effect already completed")
                .to_owned();
            let structured_content = value
                .get("structuredContent")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({ "replayed": true }));
            Ok(ToolResult {
                call_id: call.id.clone(),
                tool_name: call.name.clone(),
                status: if claim.record.status == SideEffectExecutionStatus::Succeeded {
                    crate::ToolResultStatus::Succeeded
                } else {
                    crate::ToolResultStatus::Failed
                },
                model_content,
                structured_content,
                model_images: Vec::new(),
            })
        }
        SideEffectExecutionStatus::Claimed
        | SideEffectExecutionStatus::Dispatched
        | SideEffectExecutionStatus::Indeterminate
        | SideEffectExecutionStatus::Cancelled => Ok(ToolResult::rejected(
            call,
            format!(
                "side effect was not replayed because its durable state is {:?}",
                claim.record.status
            ),
        )),
    }
}

fn recovered_side_effect_result(
    call: &crate::ToolCall,
    succeeded: bool,
    persisted_result: Option<Value>,
) -> ToolResult {
    let value = persisted_result.unwrap_or_default();
    let model_content = value
        .get("modelContent")
        .and_then(Value::as_str)
        .unwrap_or(if succeeded {
            "the external operation was already completed before recovery"
        } else {
            "the external operation had already failed before recovery"
        })
        .to_owned();
    let structured_content = value
        .get("structuredContent")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({ "recovered": true, "replayed": false }));
    ToolResult {
        call_id: call.id.clone(),
        tool_name: call.name.clone(),
        status: if succeeded {
            crate::ToolResultStatus::Succeeded
        } else {
            crate::ToolResultStatus::Failed
        },
        model_content,
        structured_content,
        model_images: Vec::new(),
    }
}

fn side_effect_completion(
    result: &Result<ToolResult, crate::ToolExecutionError>,
) -> (SideEffectExecutionStatus, &'static str, Option<Value>) {
    match result {
        Ok(result) => {
            let (status, code) = match result.status {
                crate::ToolResultStatus::Succeeded => {
                    (SideEffectExecutionStatus::Succeeded, "succeeded")
                }
                crate::ToolResultStatus::Aborted => {
                    (SideEffectExecutionStatus::Cancelled, "aborted")
                }
                crate::ToolResultStatus::TimedOut => {
                    (SideEffectExecutionStatus::Failed, "timed_out")
                }
                crate::ToolResultStatus::Failed | crate::ToolResultStatus::Rejected => {
                    (SideEffectExecutionStatus::Failed, "failed")
                }
            };
            let model_content = result
                .model_content
                .chars()
                .take(32_000)
                .collect::<String>();
            let structured_content = if serde_json::to_vec(&result.structured_content)
                .is_ok_and(|encoded| encoded.len() <= 64 * 1024)
                && !result
                    .structured_content
                    .get("redactForPersistence")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            {
                result.structured_content.clone()
            } else {
                serde_json::json!({ "truncatedOrRedacted": true })
            };
            (
                status,
                code,
                Some(serde_json::json!({
                    "modelContent": model_content,
                    "structuredContent": structured_content,
                })),
            )
        }
        Err(_) => (
            SideEffectExecutionStatus::Indeterminate,
            "executor_error_after_dispatch",
            None,
        ),
    }
}

async fn enter_approval_wait(
    context: &AuthorizedToolContext,
) -> Result<(), crate::ToolExecutionError> {
    let Some(store) = &context.run_store else {
        return Ok(());
    };
    let run = store
        .get_run(&context.run_id)
        .await
        .map_err(run_state_error)?
        .ok_or_else(|| {
            crate::ToolExecutionError::Failed(format!(
                "run {} disappeared before approval",
                context.run_id
            ))
        })?;
    if run.generation != context.run_generation {
        return Err(crate::ToolExecutionError::Failed(
            "run generation changed before approval".into(),
        ));
    }
    match run.status {
        RunStatus::Running => {
            store
                .transition_run(&context.run_id, RunStatus::WaitingApproval, None)
                .await
                .map_err(run_state_error)?;
            Ok(())
        }
        RunStatus::WaitingApproval => Ok(()),
        status => Err(crate::ToolExecutionError::Failed(format!(
            "run cannot wait for approval while it is {status:?}"
        ))),
    }
}

async fn leave_approval_wait(
    context: &AuthorizedToolContext,
) -> Result<(), crate::ToolExecutionError> {
    let Some(store) = &context.run_store else {
        return Ok(());
    };
    let run = store
        .get_run(&context.run_id)
        .await
        .map_err(run_state_error)?
        .ok_or_else(|| {
            crate::ToolExecutionError::Failed(format!(
                "run {} disappeared after approval",
                context.run_id
            ))
        })?;
    if run.generation != context.run_generation {
        return Err(crate::ToolExecutionError::Failed(
            "run generation changed while approval was pending".into(),
        ));
    }
    if run.status == RunStatus::WaitingApproval {
        store
            .transition_run(&context.run_id, RunStatus::Running, None)
            .await
            .map_err(run_state_error)?;
    }
    Ok(())
}

fn run_state_error(error: hachimi_storage::AgentStoreError) -> crate::ToolExecutionError {
    crate::ToolExecutionError::Failed(format!("approval run state failed: {error}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StructuredAuthorityDecision {
    Allow {
        effective_effect: ToolEffect,
    },
    RequireApproval {
        effective_effect: ToolEffect,
        code: &'static str,
    },
    Deny {
        code: &'static str,
    },
}

fn structured_authority_decision(
    context: &AuthorizedToolContext,
    descriptor: &ToolDescriptor,
    arguments: &Value,
) -> StructuredAuthorityDecision {
    let allow = |effective_effect| StructuredAuthorityDecision::Allow { effective_effect };
    if context.authority.policy.level == PermissionProfile::FullAccess {
        return allow(descriptor.effect);
    }

    if context.capability_host == "workspace-worker" {
        let required_access = match descriptor.effect {
            ToolEffect::WorkspaceWrite | ToolEffect::Process => FileSystemAccess::Write,
            _ => FileSystemAccess::Read,
        };
        let mut checked_path = false;
        for key in ["path", "cwd"] {
            if let Some(path) = arguments.get(key).and_then(Value::as_str) {
                checked_path = true;
                let Some(target) =
                    workspace_authority_target(&context.authority.workspace_root, path)
                else {
                    return StructuredAuthorityDecision::Deny {
                        code: "workspace_path_outside_root",
                    };
                };
                if !file_system_grants_allow(
                    &context.capability_grants.file_system,
                    required_access,
                    &target,
                ) {
                    return interactive_extra_authority(
                        context,
                        descriptor.effect,
                        "workspace_path_requires_approval",
                    );
                }
            }
        }
        if !checked_path
            && !file_system_grants_allow(
                &context.capability_grants.file_system,
                required_access,
                std::path::Path::new(&context.authority.workspace_root),
            )
        {
            return StructuredAuthorityDecision::Deny {
                code: "workspace_path_not_authorized",
            };
        }
        if descriptor.effect == ToolEffect::Process
            && context
                .authority
                .policy
                .rules
                .file_system
                .iter()
                .any(|grant| grant.access == FileSystemAccess::Deny || !grant.globs.is_empty())
        {
            return StructuredAuthorityDecision::Deny {
                code: "process_filesystem_scope_not_os_enforceable",
            };
        }
        if descriptor.effect == ToolEffect::Process
            && !context
                .capability_grants
                .process
                .allowed_commands
                .is_empty()
        {
            let Some(program) = arguments.get("program").and_then(Value::as_str) else {
                return StructuredAuthorityDecision::Deny {
                    code: "process_program_missing",
                };
            };
            if !context
                .capability_grants
                .process
                .allowed_commands
                .iter()
                .any(|allowed| string_rule_matches(allowed, program))
            {
                return match context.authority.mode {
                    AuthorityMode::Interactive => StructuredAuthorityDecision::RequireApproval {
                        effective_effect: descriptor.effect,
                        code: "process_command_requires_approval",
                    },
                    AuthorityMode::Unattended => StructuredAuthorityDecision::Deny {
                        code: "process_command_not_preconfigured",
                    },
                };
            }
        }
    }

    if let Some(server_id) = context.capability_host.strip_prefix("mcp:")
        && server_id != "resources"
    {
        let matching_rule = context.authority.policy.rules.mcp.iter().find(|rule| {
            rule.server_id.as_str() == server_id
                && hachimi_capabilities::mcp_exposed_tool_name(server_id, &rule.tool_name)
                    == descriptor.name
                && rule.schema_hash == descriptor_schema_hash(descriptor)
                && (descriptor.effect == ToolEffect::ReadOnly || !rule.read_only)
        });
        if matching_rule.is_some() {
            return allow(descriptor.effect);
        }
        return interactive_extra_authority(
            context,
            descriptor.effect,
            "mcp_tool_requires_approval",
        );
    }

    if descriptor.name == "connector_invoke" {
        let Some(account_id) = arguments.get("accountId").and_then(Value::as_str) else {
            return StructuredAuthorityDecision::Deny {
                code: "connector_account_missing",
            };
        };
        let Some(action) = arguments.get("action").and_then(Value::as_str) else {
            return StructuredAuthorityDecision::Deny {
                code: "connector_action_missing",
            };
        };
        let Some(action_revision) = arguments
            .pointer("/expectedRevision/actionHash")
            .and_then(Value::as_str)
        else {
            return StructuredAuthorityDecision::Deny {
                code: "connector_revision_missing",
            };
        };
        if let Some(rule) = context
            .authority
            .policy
            .rules
            .connectors
            .iter()
            .find(|rule| {
                rule.account_id.as_str() == account_id
                    && rule.actions.iter().any(|allowed| allowed == action)
                    && rule.contribution_revision == action_revision
            })
        {
            let read_only = rule
                .read_only_actions
                .iter()
                .any(|read_only| read_only == action);
            if context.authority.policy.level != PermissionProfile::ReadOnly || read_only {
                return allow(if read_only {
                    ToolEffect::ReadOnly
                } else {
                    descriptor.effect
                });
            }
        }
        return interactive_extra_authority(
            context,
            descriptor.effect,
            "connector_action_requires_approval",
        );
    }

    if descriptor.name == "enterprise.download_attachment" {
        let Some(account_id) = arguments.get("accountId").and_then(Value::as_str) else {
            return StructuredAuthorityDecision::Deny {
                code: "connector_account_missing",
            };
        };
        let allowed = context
            .authority
            .policy
            .rules
            .connectors
            .iter()
            .any(|rule| {
                rule.account_id.as_str() == account_id
                    && rule
                        .actions
                        .iter()
                        .any(|action| action == "download_attachment")
                    && !rule.contribution_revision.trim().is_empty()
            });
        if allowed && context.authority.policy.level != PermissionProfile::ReadOnly {
            return allow(descriptor.effect);
        }
        return interactive_extra_authority(
            context,
            descriptor.effect,
            "connector_download_requires_approval",
        );
    }

    allow(descriptor.effect)
}

fn interactive_extra_authority(
    context: &AuthorizedToolContext,
    effect: ToolEffect,
    code: &'static str,
) -> StructuredAuthorityDecision {
    match context.authority.mode {
        AuthorityMode::Unattended => StructuredAuthorityDecision::Deny { code },
        AuthorityMode::Interactive
            if context.authority.policy.level == PermissionProfile::ReadOnly
                && !matches!(
                    effect,
                    ToolEffect::ReadOnly | ToolEffect::BrowserObserve | ToolEffect::ComputerObserve
                ) =>
        {
            StructuredAuthorityDecision::Deny {
                code: "read_only_authority_cannot_request_write",
            }
        }
        AuthorityMode::Interactive => StructuredAuthorityDecision::RequireApproval {
            effective_effect: effect,
            code,
        },
    }
}

fn workspace_authority_target(workspace_root: &str, path: &str) -> Option<std::path::PathBuf> {
    use std::path::Component;

    if path.is_empty() {
        return Some(std::path::PathBuf::from(workspace_root));
    }
    if path.contains(['\0', '%']) {
        return None;
    }
    let path = std::path::Path::new(path);
    if path.is_absolute() {
        return path
            .components()
            .all(|component| {
                matches!(
                    component,
                    Component::Prefix(_) | Component::RootDir | Component::Normal(_)
                )
            })
            .then(|| path.to_path_buf());
    }
    if path.to_string_lossy().contains(':')
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    Some(std::path::Path::new(workspace_root).join(path))
}

fn string_rule_matches(rule: &str, value: &str) -> bool {
    rule == "*"
        || rule.eq_ignore_ascii_case(value)
        || rule.strip_suffix('*').is_some_and(|prefix| {
            value
                .get(..prefix.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
        })
}

fn parse_scope(value: &str) -> Option<Scope> {
    serde_json::from_value(Value::String(value.into())).ok()
}

fn parameter_hash(arguments: &Value) -> Result<String, crate::ToolExecutionError> {
    let encoded = serde_json::to_vec(arguments).map_err(|error| {
        crate::ToolExecutionError::Failed(format!("tool parameters could not be hashed: {error}"))
    })?;
    Ok(Sha256::digest(encoded)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn descriptor_schema_hash(descriptor: &ToolDescriptor) -> String {
    let schema = serde_json::to_vec(&descriptor.input_schema).unwrap_or_default();
    Sha256::digest(schema)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn authority_resource_summary(
    context: &AuthorizedToolContext,
    descriptor: &ToolDescriptor,
    arguments: &Value,
) -> String {
    if let Some(server_id) = context.capability_host.strip_prefix("mcp:")
        && server_id != "resources"
    {
        let schema_hash = descriptor_schema_hash(descriptor);
        return format!(
            "mcp:{server_id}:tool:{}:schema:{schema_hash}",
            descriptor.name
        );
    }
    if descriptor.name == "connector_invoke" {
        let account = arguments
            .get("accountId")
            .and_then(Value::as_str)
            .unwrap_or("missing");
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("missing");
        let revision = arguments
            .pointer("/expectedRevision/actionHash")
            .and_then(Value::as_str)
            .unwrap_or("missing");
        return format!("connector:{account}:action:{action}:revision:{revision}");
    }
    if descriptor.name == "enterprise.download_attachment" {
        let account = arguments
            .get("accountId")
            .and_then(Value::as_str)
            .unwrap_or("missing");
        return format!("connector:{account}:action:download_attachment");
    }
    if matches!(
        descriptor.effect,
        ToolEffect::BrowserAct | ToolEffect::BrowserObserve
    ) {
        let lease = arguments
            .get("leaseId")
            .and_then(Value::as_str)
            .unwrap_or("new");
        let action = arguments
            .pointer("/action/kind")
            .and_then(Value::as_str)
            .unwrap_or_else(|| {
                if descriptor.effect == ToolEffect::BrowserObserve {
                    "observe"
                } else {
                    "act"
                }
            });
        let url = arguments
            .get("url")
            .or_else(|| arguments.pointer("/action/url"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        return format!("browser:{lease}:action:{action}:target:{url}");
    }
    if matches!(
        descriptor.effect,
        ToolEffect::ComputerAct | ToolEffect::ComputerObserve
    ) {
        let target = arguments
            .get("appName")
            .or_else(|| arguments.get("targetFingerprint"))
            .and_then(Value::as_str)
            .unwrap_or("computer");
        return format!("computer:{target}:action:{}", descriptor.name);
    }
    resource_summary(arguments)
}

fn resource_summary(arguments: &Value) -> String {
    ["path", "program", "cwd", "url", "accountId"]
        .into_iter()
        .find_map(|key| {
            arguments
                .get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
        })
        .map(|value| value.chars().take(512).collect())
        .unwrap_or_else(|| "workspace-checkout".into())
}

fn now_ms() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}

#[cfg(test)]
#[path = "security_tools_tests.rs"]
mod tests;
