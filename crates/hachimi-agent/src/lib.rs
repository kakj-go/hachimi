//! Unified Harness Agent kernel shared by Workbench, Office, Computer and Voice entry points.

mod agents_md;
mod apply_patch;
mod compaction;
mod dynamic_tools;
mod mcp_elicitation;
mod mcp_progress;
mod mcp_resource_tools;
mod mcp_tools;
mod model_view;
mod multi_agent;
mod profiles;
mod review;
mod review_tools;
mod run_diff;
mod run_projection;
mod run_runtime;
#[cfg(test)]
mod run_runtime_tests;
mod security_tools;
mod session_lane;
mod skill_runtime;
mod step_context;
mod tool_loop;
mod tool_orchestrator;
mod tool_registry;
mod tool_runtime;
mod turn_runtime;
mod user_input_tool;
mod workload_resolver;
mod workspace_tools;

use std::{future::Future, pin::Pin};

pub use agents_md::{
    AgentsFile, AgentsFileReader, AgentsMdError, AgentsMdLoader, AgentsMdSnapshot,
    DEFAULT_AGENTS_MD_BUDGET,
};
pub use apply_patch::{APPLY_PATCH_TOOL, apply_patch_tool};
pub use model_view::{
    ModelView, ModelViewLimits, build_model_view, build_model_view_with_checkpoint,
};
pub use multi_agent::{
    AGENT_CANCEL_TOOL, AGENT_COLLECT_TOOL, AGENT_SEND_TOOL, AGENT_SPAWN_TOOL, AGENT_WAIT_TOOL,
    MultiAgentCoordinator,
};
pub use profiles::{
    WorkloadProfileSpec, profile_allows_tool, profile_runtime_context, workload_profile_spec,
};
pub use review::{
    ParsedReviewOutput, build_review_prompt, materialize_review_findings, parse_review_output,
    priority_to_severity,
};
pub use review_tools::{REVIEW_DIFF_TOOL, review_diff_tool};
pub use run_diff::RunDiffTracker;
pub use run_projection::{PersistedRunError, PersistedToolLoop, RunStepContext};
pub use run_runtime::{
    ActiveAgentRun, AgentExecutionError, AgentExecutorRegistry, AgentExecutorRegistryError,
    AgentPreparationFuture, AgentRunCreateRequest, AgentRunExecutor, AgentRunFactory,
    AgentRunFactoryError, AgentRunPreparer, AgentRunPriority, AgentRunRequest, PreparedAgentRun,
};
pub use security_tools::{AuthorizedToolContext, PersistentAuditSink, authorized_tool};
pub use session_lane::{LaneError, LaneMarker, SessionLanePermit, SessionLanes};
pub use skill_runtime::{SKILLS_LIST_TOOL, SKILLS_READ_TOOL, skill_runtime_tools};
pub use step_context::{
    AgentInstructionLayer, StepContext, StepContextFactory, StepContextInput, StepRuntimeSnapshot,
    StepRuntimeState, StepWorldState, StepWorldStateRefreshFuture, StepWorldStateRefresher,
    ToolPlan, ToolPlanConstraints,
};
pub use tool_loop::{
    LoopEvent, RunCheckpointDraft, RunCheckpointFuture, RunCheckpointReporter, SteeringFuture,
    SteeringSource, ToolLoopDriver, ToolLoopOutcome, ToolLoopRunOptions,
};
pub use tool_orchestrator::ToolOrchestrator;
pub use tool_registry::{
    ToolCall, ToolExecutionError, ToolExecutor, ToolInvocation, ToolRegistry, ToolRegistryError,
    ToolResult, ToolResultStatus,
};
pub use tool_runtime::ToolRuntime;
pub use turn_runtime::TurnRuntime;
pub use user_input_tool::{REQUEST_USER_INPUT_TOOL, request_user_input_tool};
pub use workload_resolver::{
    WORKLOAD_CLASSIFIER_REVISION, classification_for_selection, resolve_workload,
};
pub use workspace_tools::{
    WorkspaceToolKind, register_workspace_tools, workspace_tool_executors,
    workspace_tool_executors_with_diff_tracking,
};

pub type ToolFuture = Pin<Box<dyn Future<Output = Result<ToolResult, ToolExecutionError>> + Send>>;
pub use compaction::{CompactionError, CompactionPolicy, SemanticCompactor};
pub use dynamic_tools::{
    DynamicToolValidation, negotiate_provider_capabilities, validate_dynamic_tools,
};
pub use hachimi_model_runtime::{
    ModelClientFuture, ModelClientSession, ModelCompactionFuture, ModelEventStream, ModelRuntime,
    ModelRuntimeError, ModelRuntimeFactory, WorkloadClassificationFuture,
    WorkloadClassificationRequest, WorkloadClassificationResult, conservative_token_estimate,
};
pub use mcp_elicitation::{mcp_elicitation_handler, mcp_elicitation_handler_with_store};
pub use mcp_progress::mcp_progress_handler;
pub use mcp_resource_tools::{
    LIST_MCP_RESOURCE_TEMPLATES_TOOL, LIST_MCP_RESOURCES_TOOL, READ_MCP_RESOURCE_TOOL,
    mcp_resource_tool_executors,
};
pub use mcp_tools::{
    McpToolPolicy, McpToolRuntimeContext, mcp_tool_executors, mcp_tool_executors_with_gate,
    mcp_tool_executors_with_gate_and_elicitation, register_mcp_tools,
};
