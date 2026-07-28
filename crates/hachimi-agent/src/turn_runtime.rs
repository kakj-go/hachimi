// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/core/src/session/{turn,step_context}.rs
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// Modified for Hachimi: persistent typed projection and provider-neutral model sessions.

use std::sync::Arc;

use hachimi_protocol::{ModelMessage, RunRecord};
use hachimi_storage::AgentStore;
use tokio_util::sync::CancellationToken;

use crate::{
    ModelRuntime, PersistedRunError, PersistedToolLoop, RunStepContext, ToolLoopDriver,
    ToolLoopOutcome, ToolRuntime,
};

/// The only Turn execution object created by AgentRunExecutor.
///
/// The lower-level loop and projector remain private implementation details so
/// Workbench, Scheduler and transports cannot assemble an alternative runtime.
pub struct TurnRuntime {
    projected_loop: PersistedToolLoop,
}

impl std::fmt::Debug for TurnRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TurnRuntime")
            .finish_non_exhaustive()
    }
}

impl TurnRuntime {
    #[must_use]
    pub fn new(store: AgentStore, model: Arc<dyn ModelRuntime>, tools: Arc<ToolRuntime>) -> Self {
        let driver = Arc::new(ToolLoopDriver::new(model, tools));
        Self {
            projected_loop: PersistedToolLoop::new(store, driver),
        }
    }

    pub async fn execute(
        &self,
        run: RunRecord,
        messages: Vec<ModelMessage>,
        step_context: RunStepContext,
        cancellation: CancellationToken,
    ) -> Result<ToolLoopOutcome, PersistedRunError> {
        self.projected_loop
            .execute_with_step_context(run, messages, step_context, cancellation)
            .await
    }
}
