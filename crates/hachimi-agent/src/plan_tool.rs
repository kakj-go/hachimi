//! Built-in execution-plan progress tool.

use std::sync::Arc;

use hachimi_protocol::{PlanStep, PlanStepId, PlanStepStatus, RunId, ToolDescriptor, ToolEffect};
use hachimi_storage::AgentStore;
use serde::Deserialize;
use serde_json::json;

use crate::{ToolExecutionError, ToolExecutor, ToolFuture, ToolInvocation, ToolResult};

pub const UPDATE_PLAN_TOOL: &str = "update_plan";

#[derive(Debug, Deserialize)]
struct UpdatePlanArguments {
    explanation: Option<String>,
    plan: Vec<UpdatePlanStep>,
}

#[derive(Debug, Deserialize)]
struct UpdatePlanStep {
    step: String,
    status: PlanStepStatus,
}

struct UpdatePlanTool {
    store: AgentStore,
    run_id: RunId,
    update_sink: Option<Arc<dyn Fn() + Send + Sync>>,
}

#[must_use]
pub fn update_plan_tool(
    store: AgentStore,
    run_id: RunId,
    update_sink: Option<Arc<dyn Fn() + Send + Sync>>,
) -> Arc<dyn ToolExecutor> {
    Arc::new(UpdatePlanTool {
        store,
        run_id,
        update_sink,
    })
}

impl ToolExecutor for UpdatePlanTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: UPDATE_PLAN_TOOL.into(),
            description: "Update the current execution plan with the complete ordered step list and each step's real pending, in_progress, or completed status.".into(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["plan"],
                "properties": {
                    "explanation": { "type": ["string", "null"], "maxLength": 2000 },
                    "plan": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 128,
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["step", "status"],
                            "properties": {
                                "step": { "type": "string", "minLength": 1, "maxLength": 2000 },
                                "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] }
                            }
                        }
                    }
                }
            }),
            effect: ToolEffect::ReadOnly,
            parallel_safe: false,
            required_scopes: vec!["agent.run".into()],
        }
    }

    fn execute(&self, invocation: ToolInvocation) -> ToolFuture {
        let arguments =
            serde_json::from_value::<UpdatePlanArguments>(invocation.call.arguments.clone());
        let store = self.store.clone();
        let run_id = self.run_id.clone();
        let update_sink = self.update_sink.clone();
        Box::pin(async move {
            let arguments = arguments.map_err(|error| {
                ToolExecutionError::Failed(format!("invalid plan update: {error}"))
            })?;
            if arguments.plan.is_empty()
                || arguments
                    .plan
                    .iter()
                    .filter(|step| step.status == PlanStepStatus::InProgress)
                    .count()
                    > 1
            {
                return Err(ToolExecutionError::Failed(
                    "plan must contain steps and at most one in-progress step".into(),
                ));
            }
            let steps = arguments
                .plan
                .into_iter()
                .map(|step| PlanStep {
                    id: PlanStepId::random(),
                    description: step.step.trim().to_owned(),
                    status: step.status,
                })
                .collect::<Vec<_>>();
            store
                .update_execution_plan(&run_id, arguments.explanation.as_deref(), &steps)
                .await
                .map_err(|error| ToolExecutionError::Failed(error.to_string()))?;
            if let Some(update_sink) = update_sink {
                update_sink();
            }
            Ok(ToolResult::succeeded(
                &invocation.call,
                format!("Updated plan with {} steps.", steps.len()),
                json!({ "steps": steps }),
            ))
        })
    }
}
