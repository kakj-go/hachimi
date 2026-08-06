//! Workspace tool adapters. Filesystem and process access stays in the independent worker.

use std::{sync::Arc, time::Duration};

use hachimi_protocol::{CheckoutId, RunId, SessionId, ToolDescriptor, ToolEffect};
use hachimi_storage::AgentStore;
use hachimi_workspace::{WorkspaceHostClient, WorkspaceOperation, WorkspaceOutput};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    ToolExecutionError, ToolExecutor, ToolFuture, ToolInvocation, ToolRegistry, ToolRegistryError,
    ToolResult,
};

const HOST_TIMEOUT: Duration = Duration::from_secs(125);
const MAX_MODEL_RESULT_CHARS: usize = 128 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceToolKind {
    ReadFile,
    ListDirectory,
    SearchText,
    WriteFile,
    ReplaceText,
    GitStatus,
    GitDiff,
    Exec,
}

impl WorkspaceToolKind {
    fn name(self) -> &'static str {
        match self {
            Self::ReadFile => "workspace_read_file",
            Self::ListDirectory => "workspace_list_directory",
            Self::SearchText => "workspace_search_text",
            Self::WriteFile => "workspace_write_file",
            Self::ReplaceText => "workspace_replace_text",
            Self::GitStatus => "workspace_git_status",
            Self::GitDiff => "workspace_git_diff",
            Self::Exec => "workspace_exec",
        }
    }

    fn effect(self) -> ToolEffect {
        match self {
            Self::ReadFile
            | Self::ListDirectory
            | Self::SearchText
            | Self::GitStatus
            | Self::GitDiff => ToolEffect::ReadOnly,
            Self::WriteFile | Self::ReplaceText => ToolEffect::WorkspaceWrite,
            Self::Exec => ToolEffect::Process,
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::ReadFile => "Read a UTF-8 text file inside an authorized Workspace root.",
            Self::ListDirectory => "List one directory inside an authorized Workspace root.",
            Self::SearchText => "Search authorized Workspace files without following symlinks.",
            Self::WriteFile => {
                "Create or replace a UTF-8 file. Existing files require the SHA-256 returned by read_file."
            }
            Self::ReplaceText => {
                "Replace exact text in a UTF-8 file, guarded by its previously read SHA-256."
            }
            Self::GitStatus => "Return Git short status for the active Workspace.",
            Self::GitDiff => "Return the unstaged Git diff for the active Workspace.",
            Self::Exec => {
                "Run one program without a shell or PTY inside an authorized Workspace root."
            }
        }
    }

    fn input_schema(self) -> Value {
        match self {
            Self::ReadFile => object_schema(json!({ "path": { "type": "string" } }), &["path"]),
            Self::ListDirectory => {
                object_schema(json!({ "path": { "type": "string", "default": "" } }), &[])
            }
            Self::SearchText => object_schema(
                json!({
                    "path": { "type": "string", "default": "" },
                    "query": { "type": "string" },
                    "caseSensitive": { "type": "boolean", "default": true },
                    "maxResults": { "type": "integer", "minimum": 1, "maximum": 200, "default": 50 }
                }),
                &["query"],
            ),
            Self::WriteFile => object_schema(
                json!({
                    "path": { "type": "string" },
                    "content": { "type": "string" },
                    "expectedSha256": { "type": ["string", "null"] }
                }),
                &["path", "content"],
            ),
            Self::ReplaceText => object_schema(
                json!({
                    "path": { "type": "string" },
                    "oldText": { "type": "string" },
                    "newText": { "type": "string" },
                    "expectedSha256": { "type": "string" },
                    "replaceAll": { "type": "boolean", "default": false }
                }),
                &["path", "oldText", "newText", "expectedSha256"],
            ),
            Self::GitStatus | Self::GitDiff => object_schema(json!({}), &[]),
            Self::Exec => object_schema(
                json!({
                    "program": { "type": "string" },
                    "args": { "type": "array", "items": { "type": "string" }, "default": [] },
                    "cwd": { "type": "string", "default": "" },
                    "timeoutMs": { "type": "integer", "minimum": 1, "maximum": 120000, "default": 120000 }
                }),
                &["program"],
            ),
        }
    }
}

#[derive(Debug)]
struct WorkspaceTool {
    kind: WorkspaceToolKind,
    client: Arc<WorkspaceHostClient>,
    diff_tracker: Option<Arc<crate::RunDiffTracker>>,
}

impl ToolExecutor for WorkspaceTool {
    fn descriptor(&self) -> ToolDescriptor {
        let effect = self.kind.effect();
        ToolDescriptor {
            name: self.kind.name().into(),
            description: self.kind.description().into(),
            input_schema: self.kind.input_schema(),
            effect,
            parallel_safe: effect == ToolEffect::ReadOnly,
            required_scopes: vec![
                match effect {
                    ToolEffect::ReadOnly => "workspace.read",
                    ToolEffect::WorkspaceWrite => "workspace.write",
                    ToolEffect::Process => "workspace.exec",
                    _ => unreachable!("workspace tools use workspace effects"),
                }
                .into(),
            ],
        }
    }

    fn execute(&self, invocation: ToolInvocation) -> ToolFuture {
        let kind = self.kind;
        let client = Arc::clone(&self.client);
        let diff_tracker = self.diff_tracker.clone();
        Box::pin(async move {
            if invocation.run_generation != client.run_generation() {
                return Ok(ToolResult::failed(
                    &invocation.call,
                    "workspace tool rejected a stale run generation",
                ));
            }
            let operation = match operation_from_arguments(kind, invocation.call.arguments.clone())
            {
                Ok(operation) => operation,
                Err(message) => return Ok(ToolResult::failed(&invocation.call, message)),
            };
            let write_path = match &operation {
                WorkspaceOperation::WriteFile { path, .. }
                | WorkspaceOperation::ReplaceText { path, .. } => Some(path.clone()),
                _ => None,
            };
            let is_exec = matches!(operation, WorkspaceOperation::Exec { .. });
            if let (Some(tracker), Some(path)) = (&diff_tracker, write_path.as_deref())
                && let Err(error) = tracker
                    .capture_before_write(path, invocation.cancellation.child_token())
                    .await
            {
                return Ok(ToolResult::failed(
                    &invocation.call,
                    format!("run baseline capture failed: {error}"),
                ));
            }
            if is_exec
                && let Some(tracker) = &diff_tracker
                && let Err(error) = tracker
                    .capture_before_exec(invocation.cancellation.child_token())
                    .await
            {
                return Ok(ToolResult::failed(
                    &invocation.call,
                    format!("pre-Exec Run baseline capture failed: {error}"),
                ));
            }
            match client
                .execute(
                    operation,
                    HOST_TIMEOUT,
                    invocation.cancellation.child_token(),
                )
                .await
            {
                Ok(output) => {
                    if invocation.cancellation.is_cancelled() {
                        return Ok(ToolResult::aborted(
                            &invocation.call,
                            "workspace result arrived after cancellation",
                        ));
                    }
                    if let (Some(tracker), Some(path), WorkspaceOutput::Write { sha256, .. }) =
                        (&diff_tracker, write_path.as_deref(), &output)
                        && let Err(error) = tracker
                            .record_write_and_refresh(path, sha256, &invocation.cancellation)
                            .await
                    {
                        return Ok(ToolResult::failed(
                            &invocation.call,
                            format!("run diff update failed: {error}"),
                        ));
                    }
                    if is_exec
                        && let Some(tracker) = &diff_tracker
                        && matches!(output, WorkspaceOutput::Process { .. })
                        && let Err(error) = tracker
                            .record_exec_and_refresh(&invocation.cancellation)
                            .await
                    {
                        return Ok(ToolResult::failed(
                            &invocation.call,
                            format!("post-Exec Run Diff update failed: {error}"),
                        ));
                    }
                    let structured_content = serde_json::to_value(&output).map_err(|error| {
                        ToolExecutionError::Failed(format!(
                            "workspace result serialization failed: {error}"
                        ))
                    })?;
                    let model_content = bounded_model_content(&output);
                    Ok(ToolResult::succeeded(
                        &invocation.call,
                        model_content,
                        structured_content,
                    ))
                }
                Err(error) => {
                    let message = format!("workspace host {:?}: {}", error.code, error.message);
                    Ok(match error.code {
                        hachimi_workspace::WorkspaceErrorCode::Cancelled => {
                            ToolResult::aborted(&invocation.call, message)
                        }
                        hachimi_workspace::WorkspaceErrorCode::TimedOut => {
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

pub fn register_workspace_tools(
    registry: &mut ToolRegistry,
    client: Arc<WorkspaceHostClient>,
) -> Result<(), ToolRegistryError> {
    for tool in workspace_tool_executors(client) {
        registry.register(tool)?;
    }
    Ok(())
}

#[must_use]
pub fn workspace_tool_executors(client: Arc<WorkspaceHostClient>) -> Vec<Arc<dyn ToolExecutor>> {
    [
        WorkspaceToolKind::ReadFile,
        WorkspaceToolKind::ListDirectory,
        WorkspaceToolKind::SearchText,
        WorkspaceToolKind::WriteFile,
        WorkspaceToolKind::ReplaceText,
        WorkspaceToolKind::GitStatus,
        WorkspaceToolKind::GitDiff,
        WorkspaceToolKind::Exec,
    ]
    .into_iter()
    .map(|kind| {
        Arc::new(WorkspaceTool {
            kind,
            client: Arc::clone(&client),
            diff_tracker: None,
        }) as Arc<dyn ToolExecutor>
    })
    .collect()
}

#[must_use]
pub fn workspace_tool_executors_with_diff_tracking(
    client: Arc<WorkspaceHostClient>,
    store: AgentStore,
    session_id: SessionId,
    run_id: RunId,
    checkout_id: CheckoutId,
) -> Vec<Arc<dyn ToolExecutor>> {
    workspace_tool_executors_with_diff_tracker(client, store, session_id, run_id, checkout_id).0
}

#[must_use]
pub fn workspace_tool_executors_with_diff_tracker(
    client: Arc<WorkspaceHostClient>,
    store: AgentStore,
    session_id: SessionId,
    run_id: RunId,
    checkout_id: CheckoutId,
) -> (Vec<Arc<dyn ToolExecutor>>, Arc<crate::RunDiffTracker>) {
    let tracker = Arc::new(crate::RunDiffTracker::new(
        store,
        Arc::clone(&client),
        session_id,
        run_id,
        checkout_id,
    ));
    let executors = [
        WorkspaceToolKind::ReadFile,
        WorkspaceToolKind::ListDirectory,
        WorkspaceToolKind::SearchText,
        WorkspaceToolKind::WriteFile,
        WorkspaceToolKind::ReplaceText,
        WorkspaceToolKind::GitStatus,
        WorkspaceToolKind::GitDiff,
        WorkspaceToolKind::Exec,
    ]
    .into_iter()
    .map(|kind| {
        Arc::new(WorkspaceTool {
            kind,
            client: Arc::clone(&client),
            diff_tracker: Some(Arc::clone(&tracker)),
        }) as Arc<dyn ToolExecutor>
    })
    .collect();
    (executors, tracker)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PathArguments {
    #[serde(default)]
    path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchArguments {
    #[serde(default)]
    path: String,
    query: String,
    #[serde(default = "default_true")]
    case_sensitive: bool,
    #[serde(default = "default_search_results")]
    max_results: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WriteArguments {
    path: String,
    content: String,
    #[serde(default)]
    expected_sha256: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplaceArguments {
    path: String,
    old_text: String,
    new_text: String,
    expected_sha256: String,
    #[serde(default)]
    replace_all: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecArguments {
    program: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    cwd: String,
    #[serde(default = "default_exec_timeout")]
    timeout_ms: u64,
}

fn operation_from_arguments(
    kind: WorkspaceToolKind,
    arguments: Value,
) -> Result<WorkspaceOperation, String> {
    let invalid = |error: serde_json::Error| format!("invalid {} arguments: {error}", kind.name());
    Ok(match kind {
        WorkspaceToolKind::ReadFile => {
            let arguments: PathArguments = serde_json::from_value(arguments).map_err(invalid)?;
            WorkspaceOperation::ReadFile {
                path: arguments.path,
            }
        }
        WorkspaceToolKind::ListDirectory => {
            let arguments: PathArguments = serde_json::from_value(arguments).map_err(invalid)?;
            WorkspaceOperation::ListDirectory {
                path: arguments.path,
            }
        }
        WorkspaceToolKind::SearchText => {
            let arguments: SearchArguments = serde_json::from_value(arguments).map_err(invalid)?;
            WorkspaceOperation::SearchText {
                path: arguments.path,
                query: arguments.query,
                case_sensitive: arguments.case_sensitive,
                max_results: arguments.max_results,
            }
        }
        WorkspaceToolKind::WriteFile => {
            let arguments: WriteArguments = serde_json::from_value(arguments).map_err(invalid)?;
            WorkspaceOperation::WriteFile {
                path: arguments.path,
                content: arguments.content,
                expected_sha256: arguments.expected_sha256,
            }
        }
        WorkspaceToolKind::ReplaceText => {
            let arguments: ReplaceArguments = serde_json::from_value(arguments).map_err(invalid)?;
            WorkspaceOperation::ReplaceText {
                path: arguments.path,
                old_text: arguments.old_text,
                new_text: arguments.new_text,
                expected_sha256: arguments.expected_sha256,
                replace_all: arguments.replace_all,
            }
        }
        WorkspaceToolKind::GitStatus => WorkspaceOperation::GitStatus,
        WorkspaceToolKind::GitDiff => WorkspaceOperation::GitDiff,
        WorkspaceToolKind::Exec => {
            let arguments: ExecArguments = serde_json::from_value(arguments).map_err(invalid)?;
            WorkspaceOperation::Exec {
                program: arguments.program,
                args: arguments.args,
                cwd: arguments.cwd,
                timeout_ms: arguments.timeout_ms,
            }
        }
    })
}

fn object_schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

fn default_true() -> bool {
    true
}

fn default_search_results() -> usize {
    50
}

fn default_exec_timeout() -> u64 {
    120_000
}

fn bounded_model_content(output: &WorkspaceOutput) -> String {
    let encoded = serde_json::to_string(output)
        .unwrap_or_else(|error| format!("workspace result could not be serialized: {error}"));
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
    format!("{head}\n… workspace result clipped by host adapter …\n{tail}")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use hachimi_protocol::{BehaviorMode, EntryProfile, WorkloadKind};

    use super::*;

    #[test]
    fn plan_registry_advertises_only_read_tools() {
        let client = Arc::new(WorkspaceHostClient::new(
            Path::new("worker"),
            Path::new("checkout"),
            "checkout",
            1,
        ));
        let mut registry = ToolRegistry::new();
        register_workspace_tools(&mut registry, client).expect("register");
        let descriptors = registry.descriptors(
            EntryProfile::Workbench,
            WorkloadKind::Coding,
            BehaviorMode::Plan,
        );
        assert_eq!(descriptors.len(), 5);
        assert!(
            descriptors
                .iter()
                .all(|descriptor| descriptor.effect == ToolEffect::ReadOnly)
        );
        assert!(registry.executor("workspace_exec").is_some());
        assert!(!registry.is_allowed(
            "workspace_exec",
            EntryProfile::Workbench,
            WorkloadKind::Coding,
            BehaviorMode::Plan
        ));
    }

    #[test]
    fn write_arguments_preserve_optimistic_hash() {
        let operation = operation_from_arguments(
            WorkspaceToolKind::WriteFile,
            json!({
                "path": "README.md",
                "content": "updated",
                "expectedSha256": "abc"
            }),
        )
        .expect("arguments");
        assert!(matches!(
            operation,
            WorkspaceOperation::WriteFile { expected_sha256: Some(value), .. } if value == "abc"
        ));
    }
}
