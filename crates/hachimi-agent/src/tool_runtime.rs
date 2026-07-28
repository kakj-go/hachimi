// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/core/src/tools/parallel.rs and lifecycle.rs
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// Modified for Hachimi: narrow executor trait, read/write gate, bounded teardown, and DTO results.

use std::{sync::Arc, time::Duration};

use hachimi_protocol::{BehaviorMode, EntryProfile, WorkloadKind};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::{
    ToolCall, ToolExecutionError, ToolExecutor, ToolInvocation, ToolRegistry, ToolRegistryError,
    ToolResult,
};

#[derive(Debug)]
pub struct ToolRuntime {
    registry: Arc<ToolRegistry>,
    parallel_execution: Arc<RwLock<()>>,
}

impl ToolRuntime {
    #[must_use]
    pub fn new(registry: Arc<ToolRegistry>) -> Self {
        Self {
            registry,
            parallel_execution: Arc::new(RwLock::new(())),
        }
    }

    /// Builds the model-visible registry inside the Agent kernel. Hosts provide
    /// narrow executors, but cannot create an alternate router or ToolLoop.
    pub fn from_executors(
        executors: Vec<Arc<dyn ToolExecutor>>,
    ) -> Result<Self, ToolRegistryError> {
        let mut registry = ToolRegistry::new();
        for executor in executors {
            registry.register(executor)?;
        }
        Ok(Self::new(Arc::new(registry)))
    }

    #[must_use]
    pub fn registry(&self) -> &Arc<ToolRegistry> {
        &self.registry
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn execute(
        &self,
        call: ToolCall,
        entry_profile: EntryProfile,
        workload: WorkloadKind,
        mode: BehaviorMode,
        run_generation: u64,
        step_revision: u64,
        tool_plan_hash: &str,
        registry_revision: &str,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<ToolResult, ToolExecutionError> {
        if cancellation.is_cancelled() {
            return Ok(ToolResult::aborted(&call, "aborted before tool admission"));
        }
        if !call.arguments.is_object() {
            return Err(ToolExecutionError::InvalidArguments);
        }
        if call.step_revision != step_revision
            || call.tool_plan_hash != tool_plan_hash
            || call.registry_revision != registry_revision
            || self.registry.revision() != registry_revision
        {
            return Err(ToolExecutionError::StaleToolPlan(call.name.clone()));
        }
        let executor = self
            .registry
            .executor(&call.name)
            .ok_or_else(|| ToolExecutionError::UnknownTool(call.name.clone()))?;
        if !self
            .registry
            .is_allowed(&call.name, entry_profile, workload, mode)
        {
            return Err(ToolExecutionError::ForbiddenInMode(call.name.clone()));
        }
        let parallel_safe = executor.descriptor().parallel_safe;
        if parallel_safe {
            let gate = Arc::clone(&self.parallel_execution);
            let _guard = tokio::select! {
                () = cancellation.cancelled() => return Ok(ToolResult::aborted(&call, "aborted before tool admission")),
                guard = gate.read_owned() => guard,
            };
            if cancellation.is_cancelled() {
                return Ok(ToolResult::aborted(&call, "aborted before tool dispatch"));
            }
            Self::dispatch(
                executor,
                call,
                entry_profile,
                workload,
                mode,
                run_generation,
                step_revision,
                tool_plan_hash,
                registry_revision,
                timeout,
                cancellation,
            )
            .await
        } else {
            let gate = Arc::clone(&self.parallel_execution);
            let _guard = tokio::select! {
                () = cancellation.cancelled() => return Ok(ToolResult::aborted(&call, "aborted before tool admission")),
                guard = gate.write_owned() => guard,
            };
            if cancellation.is_cancelled() {
                return Ok(ToolResult::aborted(&call, "aborted before tool dispatch"));
            }
            Self::dispatch(
                executor,
                call,
                entry_profile,
                workload,
                mode,
                run_generation,
                step_revision,
                tool_plan_hash,
                registry_revision,
                timeout,
                cancellation,
            )
            .await
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn dispatch(
        executor: Arc<dyn crate::ToolExecutor>,
        call: ToolCall,
        entry_profile: EntryProfile,
        workload: WorkloadKind,
        mode: BehaviorMode,
        run_generation: u64,
        step_revision: u64,
        tool_plan_hash: &str,
        registry_revision: &str,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<ToolResult, ToolExecutionError> {
        let invocation_cancellation = cancellation.child_token();
        let invocation = ToolInvocation {
            call: call.clone(),
            entry_profile,
            workload,
            behavior_mode: mode,
            run_generation,
            step_revision,
            tool_plan_hash: tool_plan_hash.to_owned(),
            registry_revision: registry_revision.to_owned(),
            cancellation: invocation_cancellation.clone(),
        };
        let waits_for_cancellation = executor.waits_for_cancellation();
        let future = executor.execute(invocation);
        tokio::pin!(future);
        tokio::select! {
            result = &mut future => result,
            () = cancellation.cancelled() => {
                invocation_cancellation.cancel();
                if waits_for_cancellation {
                    let _ = tokio::time::timeout(Duration::from_secs(5), &mut future).await;
                }
                Ok(ToolResult::aborted(&call, "aborted by user"))
            }
            () = tokio::time::sleep(timeout) => {
                invocation_cancellation.cancel();
                if waits_for_cancellation {
                    let _ = tokio::time::timeout(Duration::from_secs(5), &mut future).await;
                }
                Ok(ToolResult::timed_out(&call, "tool execution timed out"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use hachimi_protocol::{ToolDescriptor, ToolEffect};
    use serde_json::{Value, json};

    use super::*;
    use crate::{ToolExecutor, ToolFuture};

    struct TrackingTool {
        descriptor: ToolDescriptor,
        active: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
    }

    impl ToolExecutor for TrackingTool {
        fn descriptor(&self) -> ToolDescriptor {
            self.descriptor.clone()
        }

        fn execute(&self, invocation: ToolInvocation) -> ToolFuture {
            let active = Arc::clone(&self.active);
            let peak = Arc::clone(&self.peak);
            Box::pin(async move {
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(current, Ordering::SeqCst);
                tokio::select! {
                    () = invocation.cancellation.cancelled() => {},
                    () = tokio::time::sleep(Duration::from_millis(30)) => {},
                }
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(ToolResult::succeeded(&invocation.call, "ok", Value::Null))
            })
        }

        fn waits_for_cancellation(&self) -> bool {
            true
        }
    }

    fn tracking_tool(
        name: &str,
        effect: ToolEffect,
        active: &Arc<AtomicUsize>,
        peak: &Arc<AtomicUsize>,
    ) -> Arc<dyn ToolExecutor> {
        Arc::new(TrackingTool {
            descriptor: ToolDescriptor {
                name: name.into(),
                description: name.into(),
                input_schema: json!({ "type": "object" }),
                effect,
                parallel_safe: effect == ToolEffect::ReadOnly,
                required_scopes: Vec::new(),
            },
            active: Arc::clone(active),
            peak: Arc::clone(peak),
        })
    }

    fn call(id: &str, name: &str, registry_revision: &str) -> ToolCall {
        ToolCall {
            id: hachimi_protocol::ToolCallId::from(id),
            name: name.into(),
            arguments: json!({}),
            step_revision: 1,
            tool_plan_hash: "fixture-plan".into(),
            registry_revision: registry_revision.into(),
        }
    }

    #[tokio::test]
    async fn read_tools_run_in_parallel_and_write_takes_exclusive_gate() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry
            .register(tracking_tool(
                "workspace_read_file",
                ToolEffect::ReadOnly,
                &active,
                &peak,
            ))
            .expect("read");
        registry
            .register(tracking_tool(
                "workspace_write_file",
                ToolEffect::WorkspaceWrite,
                &active,
                &peak,
            ))
            .expect("write");
        let runtime = Arc::new(ToolRuntime::new(Arc::new(registry)));
        let registry_revision = runtime.registry().revision().to_owned();
        let read_one = runtime.execute(
            call("1", "workspace_read_file", &registry_revision),
            EntryProfile::Workbench,
            WorkloadKind::Coding,
            BehaviorMode::Default,
            1,
            1,
            "fixture-plan",
            &registry_revision,
            Duration::from_secs(1),
            CancellationToken::new(),
        );
        let read_two = runtime.execute(
            call("2", "workspace_read_file", &registry_revision),
            EntryProfile::Workbench,
            WorkloadKind::Coding,
            BehaviorMode::Default,
            1,
            1,
            "fixture-plan",
            &registry_revision,
            Duration::from_secs(1),
            CancellationToken::new(),
        );
        let (one, two) = tokio::join!(read_one, read_two);
        one.expect("one");
        two.expect("two");
        assert_eq!(peak.load(Ordering::SeqCst), 2);

        peak.store(0, Ordering::SeqCst);
        let read = runtime.execute(
            call("3", "workspace_read_file", &registry_revision),
            EntryProfile::Workbench,
            WorkloadKind::Coding,
            BehaviorMode::Default,
            1,
            1,
            "fixture-plan",
            &registry_revision,
            Duration::from_secs(1),
            CancellationToken::new(),
        );
        let write = runtime.execute(
            call("4", "workspace_write_file", &registry_revision),
            EntryProfile::Workbench,
            WorkloadKind::Coding,
            BehaviorMode::Default,
            1,
            1,
            "fixture-plan",
            &registry_revision,
            Duration::from_secs(1),
            CancellationToken::new(),
        );
        let (read, write) = tokio::join!(read, write);
        read.expect("read");
        write.expect("write");
        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancellation_returns_a_model_visible_abort_result() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry
            .register(tracking_tool(
                "workspace_read_file",
                ToolEffect::ReadOnly,
                &active,
                &peak,
            ))
            .expect("read");
        let runtime = ToolRuntime::new(Arc::new(registry));
        let registry_revision = runtime.registry().revision().to_owned();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let result = runtime
            .execute(
                call("1", "workspace_read_file", &registry_revision),
                EntryProfile::Workbench,
                WorkloadKind::Coding,
                BehaviorMode::Default,
                1,
                1,
                "fixture-plan",
                &registry_revision,
                Duration::from_secs(1),
                cancellation,
            )
            .await
            .expect("aborted result");
        assert_eq!(result.status, crate::ToolResultStatus::Aborted);
    }

    #[tokio::test]
    async fn stale_registry_revision_is_rejected_before_executor_dispatch() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry
            .register(tracking_tool(
                "workspace_read_file",
                ToolEffect::ReadOnly,
                &active,
                &peak,
            ))
            .expect("read");
        let runtime = ToolRuntime::new(Arc::new(registry));
        let error = runtime
            .execute(
                call("stale", "workspace_read_file", "stale-registry"),
                EntryProfile::Workbench,
                WorkloadKind::Coding,
                BehaviorMode::Default,
                1,
                1,
                "fixture-plan",
                "stale-registry",
                Duration::from_secs(1),
                CancellationToken::new(),
            )
            .await
            .expect_err("stale registry must be fenced");
        assert_eq!(
            error,
            ToolExecutionError::StaleToolPlan("workspace_read_file".into())
        );
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }
}
