// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/app-server-protocol/src/protocol/v2/mcp.rs and
// codex-rs/core/src/mcp_tool_call.rs.
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// Modified for Hachimi: durable Session event projection with Run generation fencing.

use std::sync::Arc;

use futures_util::FutureExt;
use hachimi_capabilities::{McpProgressFuture, McpProgressHandler, McpProgressNotification};
use hachimi_protocol::{McpServerId, McpToolProgressRecord, RunId, SessionId};
use hachimi_storage::AgentStore;

struct StoreMcpProgressHandler {
    store: AgentStore,
    server_id: McpServerId,
    session_id: SessionId,
    run_id: RunId,
}

impl std::fmt::Debug for StoreMcpProgressHandler {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StoreMcpProgressHandler")
            .field("server_id", &self.server_id)
            .field("session_id", &self.session_id)
            .field("run_id", &self.run_id)
            .finish_non_exhaustive()
    }
}

#[must_use]
pub fn mcp_progress_handler(
    store: AgentStore,
    server_id: McpServerId,
    session_id: SessionId,
    run_id: RunId,
) -> Arc<dyn McpProgressHandler> {
    Arc::new(StoreMcpProgressHandler {
        store,
        server_id,
        session_id,
        run_id,
    })
}

impl McpProgressHandler for StoreMcpProgressHandler {
    fn progress(&self, notification: McpProgressNotification) -> McpProgressFuture {
        if notification.server_id != self.server_id.as_str()
            || notification.correlation.session_id != self.session_id
            || notification.correlation.run_id != self.run_id
        {
            return async {}.boxed();
        }
        let store = self.store.clone();
        let progress = McpToolProgressRecord {
            server_id: self.server_id.clone(),
            session_id: self.session_id.clone(),
            run_id: self.run_id.clone(),
            run_generation: notification.correlation.run_generation,
            tool_call_id: notification.correlation.tool_call_id,
            progress: notification.progress,
            total: notification.total,
            message: notification.message,
        };
        async move {
            // Progress is untrusted display data. A storage failure must not fail or retry the
            // actual side effect, and no server-supplied payload is written to ordinary logs.
            let _ = store.append_mcp_tool_progress(&progress).await;
        }
        .boxed()
    }
}
