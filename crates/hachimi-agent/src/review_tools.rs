// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex/codex-rs/core/src/session/review.rs
// @ 4c43465133428898aa84f0bfc02c306ed65fb66a.
// Modified for Hachimi: a target-bound read-only Workspace Host tool.

use std::{sync::Arc, time::Duration};

use hachimi_protocol::{ReviewTarget, ToolDescriptor, ToolEffect};
use hachimi_workspace::{WorkspaceHostClient, WorkspaceOperation, WorkspaceOutput};
use serde_json::json;

use crate::{ToolExecutionError, ToolExecutor, ToolFuture, ToolInvocation, ToolResult};

pub const REVIEW_DIFF_TOOL: &str = "workspace_review_diff";
const REVIEW_TIMEOUT: Duration = Duration::from_secs(65);
const MAX_MODEL_RESULT_CHARS: usize = 128 * 1024;

#[must_use]
pub fn review_diff_tool(
    client: Arc<WorkspaceHostClient>,
    target: ReviewTarget,
) -> Arc<dyn ToolExecutor> {
    Arc::new(ReviewDiffTool { client, target })
}

#[derive(Debug)]
struct ReviewDiffTool {
    client: Arc<WorkspaceHostClient>,
    target: ReviewTarget,
}

impl ToolExecutor for ReviewDiffTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: REVIEW_DIFF_TOOL.into(),
            description:
                "Read the immutable Review target Diff selected when this Review Run started."
                    .into(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            }),
            effect: ToolEffect::ReadOnly,
            parallel_safe: true,
            required_scopes: vec!["workspace.read".into()],
        }
    }

    fn execute(&self, invocation: ToolInvocation) -> ToolFuture {
        let client = Arc::clone(&self.client);
        let target = self.target.clone();
        Box::pin(async move {
            if invocation.run_generation != client.run_generation() {
                return Ok(ToolResult::failed(
                    &invocation.call,
                    "Review Diff rejected a stale Run generation",
                ));
            }
            let output = client
                .execute(
                    WorkspaceOperation::GitReviewDiff { target },
                    REVIEW_TIMEOUT,
                    invocation.cancellation.child_token(),
                )
                .await;
            match output {
                Ok(_output) if invocation.cancellation.is_cancelled() => Ok(ToolResult::aborted(
                    &invocation.call,
                    "Review Diff arrived after cancellation",
                )),
                Ok(output) => {
                    let model_content = match &output {
                        WorkspaceOutput::Process {
                            stdout,
                            stderr,
                            exit_code,
                            truncated,
                        } => bounded(&format!(
                            "exit_code={exit_code:?} truncated={truncated}\nstdout:\n{stdout}\nstderr:\n{stderr}"
                        )),
                        _ => "Review Diff Host returned an unexpected output".into(),
                    };
                    let structured = serde_json::to_value(&output).map_err(|error| {
                        ToolExecutionError::Failed(format!(
                            "Review Diff serialization failed: {error}"
                        ))
                    })?;
                    Ok(ToolResult::succeeded(
                        &invocation.call,
                        model_content,
                        structured,
                    ))
                }
                Err(error) => Ok(ToolResult::failed(
                    &invocation.call,
                    format!("Review Diff Host {:?}: {}", error.code, error.message),
                )),
            }
        })
    }

    fn waits_for_cancellation(&self) -> bool {
        true
    }
}

fn bounded(value: &str) -> String {
    value.chars().take(MAX_MODEL_RESULT_CHARS).collect()
}
