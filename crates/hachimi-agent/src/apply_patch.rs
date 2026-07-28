// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/core/src/tools/{handlers,runtimes}/apply_patch.rs
// and codex-rs/apply-patch/src/{lib,parser,invocation,streaming_parser}.rs
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// Modified for Hachimi: typed ToolExecution, generation fencing, Workspace Worker
// transaction dispatch, and Run Diff baseline persistence.

use std::{collections::BTreeSet, sync::Arc, time::Duration};

use hachimi_protocol::{CheckoutId, RunId, SessionId, ToolDescriptor, ToolEffect};
use hachimi_storage::AgentStore;
use hachimi_workspace::{
    WorkspaceErrorCode, WorkspaceHostClient, WorkspaceOperation, WorkspaceOutput, parse_apply_patch,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    RunDiffTracker, ToolExecutionError, ToolExecutor, ToolFuture, ToolInvocation, ToolResult,
};

pub const APPLY_PATCH_TOOL: &str = "apply_patch";
const PATCH_TIMEOUT: Duration = Duration::from_secs(125);
const MAX_MODEL_RESULT_CHARS: usize = 128 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApplyPatchArguments {
    patch: String,
}

#[derive(Debug)]
struct ApplyPatchTool {
    client: Arc<WorkspaceHostClient>,
    diff_tracker: Arc<RunDiffTracker>,
}

impl ToolExecutor for ApplyPatchTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: APPLY_PATCH_TOOL.into(),
            description: "Apply one bounded multi-file patch inside the active checkout. Supports add, update, delete, and move hunks; all targets are preflighted before the Workspace Worker commits the transaction.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "patch": { "type": "string", "minLength": 1, "maxLength": 4194304 }
                },
                "required": ["patch"],
                "additionalProperties": false
            }),
            effect: ToolEffect::WorkspaceWrite,
            parallel_safe: false,
            required_scopes: vec!["workspace.write".into()],
        }
    }

    fn execute(&self, invocation: ToolInvocation) -> ToolFuture {
        let client = Arc::clone(&self.client);
        let tracker = Arc::clone(&self.diff_tracker);
        Box::pin(async move {
            if invocation.run_generation != client.run_generation() {
                return Ok(ToolResult::failed(
                    &invocation.call,
                    "apply_patch rejected a stale run generation",
                ));
            }
            let arguments: ApplyPatchArguments =
                match serde_json::from_value(invocation.call.arguments.clone()) {
                    Ok(arguments) => arguments,
                    Err(error) => {
                        return Ok(ToolResult::failed(
                            &invocation.call,
                            format!("invalid apply_patch arguments: {error}"),
                        ));
                    }
                };
            let plan = match parse_apply_patch(&arguments.patch) {
                Ok(plan) => plan,
                Err(error) => {
                    return Ok(ToolResult::failed(
                        &invocation.call,
                        format!("invalid patch: {}", error.message),
                    ));
                }
            };
            let move_pairs = plan.move_pairs();
            let moved = move_pairs
                .iter()
                .flat_map(|(source, destination)| [*source, *destination])
                .collect::<BTreeSet<_>>();
            for (source, destination) in move_pairs {
                if let Err(error) = tracker
                    .capture_before_move(source, destination, invocation.cancellation.child_token())
                    .await
                {
                    return Ok(ToolResult::failed(
                        &invocation.call,
                        format!("run move baseline capture failed: {error}"),
                    ));
                }
            }
            for target in plan
                .targets()
                .iter()
                .filter(|target| !moved.contains(target.as_str()))
            {
                if let Err(error) = tracker
                    .capture_before_write(target, invocation.cancellation.child_token())
                    .await
                {
                    return Ok(ToolResult::failed(
                        &invocation.call,
                        format!("run patch baseline capture failed: {error}"),
                    ));
                }
            }
            let output = client
                .execute(
                    WorkspaceOperation::ApplyPatch {
                        patch: arguments.patch,
                    },
                    PATCH_TIMEOUT,
                    invocation.cancellation.child_token(),
                )
                .await;
            match output {
                Ok(output @ WorkspaceOutput::Patch { .. }) => {
                    if invocation.cancellation.is_cancelled() {
                        return Ok(ToolResult::aborted(
                            &invocation.call,
                            "apply_patch result arrived after cancellation",
                        ));
                    }
                    if let Err(error) = tracker.refresh(&invocation.cancellation).await {
                        return Ok(ToolResult::failed(
                            &invocation.call,
                            format!("run Diff refresh failed after patch: {error}"),
                        ));
                    }
                    let structured = serde_json::to_value(&output).map_err(|error| {
                        ToolExecutionError::Failed(format!(
                            "patch result serialization failed: {error}"
                        ))
                    })?;
                    Ok(ToolResult::succeeded(
                        &invocation.call,
                        bounded_model_content(&structured),
                        structured,
                    ))
                }
                Ok(_) => Ok(ToolResult::failed(
                    &invocation.call,
                    "Workspace Host returned an unexpected patch result",
                )),
                Err(error) => {
                    let message = format!("workspace patch {:?}: {}", error.code, error.message);
                    Ok(match error.code {
                        WorkspaceErrorCode::Cancelled => {
                            ToolResult::aborted(&invocation.call, message)
                        }
                        WorkspaceErrorCode::TimedOut => {
                            ToolResult::timed_out(&invocation.call, message)
                        }
                        _ => ToolResult::failed(&invocation.call, message),
                    })
                }
            }
        })
    }

    fn waits_for_cancellation(&self) -> bool {
        true
    }
}

#[must_use]
pub fn apply_patch_tool(
    client: Arc<WorkspaceHostClient>,
    store: AgentStore,
    session_id: SessionId,
    run_id: RunId,
    checkout_id: CheckoutId,
) -> Arc<dyn ToolExecutor> {
    Arc::new(ApplyPatchTool {
        diff_tracker: Arc::new(RunDiffTracker::new(
            store,
            Arc::clone(&client),
            session_id,
            run_id,
            checkout_id,
        )),
        client,
    })
}

fn bounded_model_content(value: &Value) -> String {
    let encoded = serde_json::to_string(value)
        .unwrap_or_else(|error| format!("patch result could not be serialized: {error}"));
    if encoded.chars().count() <= MAX_MODEL_RESULT_CHARS {
        return encoded;
    }
    let head = encoded
        .chars()
        .take(MAX_MODEL_RESULT_CHARS / 2)
        .collect::<String>();
    let tail = encoded
        .chars()
        .rev()
        .take(MAX_MODEL_RESULT_CHARS / 2)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{head}\n… patch result clipped by host adapter …\n{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn descriptor_is_one_non_parallel_workspace_side_effect() {
        let client = Arc::new(WorkspaceHostClient::new(
            "worker", "checkout", "checkout", 1,
        ));
        let tracker = Arc::new(RunDiffTracker::new(
            AgentStore::connect_in_memory().await.expect("store"),
            Arc::clone(&client),
            SessionId::random(),
            RunId::random(),
            CheckoutId::random(),
        ));
        let descriptor = ApplyPatchTool {
            client,
            diff_tracker: tracker,
        }
        .descriptor();
        assert_eq!(descriptor.name, APPLY_PATCH_TOOL);
        assert_eq!(descriptor.effect, ToolEffect::WorkspaceWrite);
        assert!(!descriptor.parallel_safe);
    }
}
