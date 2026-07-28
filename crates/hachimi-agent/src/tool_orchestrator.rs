// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/core/src/tools/{orchestrator,router,sandboxing}.rs
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// Modified for Hachimi: StepContext fencing and a transport-neutral Tool result contract.

use std::{sync::Arc, time::Duration};

use hachimi_protocol::ModelToolCall;
use tokio_util::sync::CancellationToken;

use crate::{StepContext, ToolCall, ToolExecutionError, ToolResult, ToolRuntime};

#[derive(Debug, Clone)]
pub struct ToolOrchestrator {
    runtime: Arc<ToolRuntime>,
}

impl ToolOrchestrator {
    #[must_use]
    pub const fn new(runtime: Arc<ToolRuntime>) -> Self {
        Self { runtime }
    }

    #[must_use]
    pub fn runtime(&self) -> &Arc<ToolRuntime> {
        &self.runtime
    }

    pub fn bind_call(&self, model_call: ModelToolCall, step: &StepContext) -> ToolCall {
        ToolCall::bind(
            model_call,
            step.step_revision,
            step.tool_plan.hash().to_owned(),
            step.registry_revision.clone(),
        )
    }

    pub async fn execute(
        &self,
        call: ToolCall,
        step: &StepContext,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<(ToolCall, ToolResult), ToolExecutionError> {
        if call.step_revision != step.step_revision
            || call.tool_plan_hash != step.tool_plan.hash()
            || call.registry_revision != step.registry_revision
            || self.runtime.registry().revision() != step.registry_revision
            || !step.tool_plan.allows(&call.name)
        {
            return Err(ToolExecutionError::StaleToolPlan(call.name));
        }
        let result = self
            .runtime
            .execute(
                call.clone(),
                step.entry_profile,
                step.workload.workload,
                step.behavior_mode,
                step.run_generation,
                step.step_revision,
                step.tool_plan.hash(),
                &step.registry_revision,
                timeout,
                cancellation,
            )
            .await?;
        Ok((call, result))
    }
}
