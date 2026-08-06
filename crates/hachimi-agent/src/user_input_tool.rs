//! Model-facing user-input tool backed by the persistent broker.

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use hachimi_protocol::{
    ItemId, RunId, SessionId, ToolDescriptor, ToolEffect, UserInputAnswer, UserInputQuestion,
    UserInputRequestId, UserInputRequestRecord, UserInputResolutionAction, UserInputStatus,
};
use hachimi_user_input::UserInputBroker;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{ToolExecutionError, ToolExecutor, ToolFuture, ToolInvocation, ToolResult};

pub const REQUEST_USER_INPUT_TOOL: &str = "request_user_input";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserInputArguments {
    questions: Vec<UserInputQuestion>,
}

struct RequestUserInputTool {
    broker: Arc<dyn UserInputBroker>,
    session_id: SessionId,
    run_id: RunId,
}

impl std::fmt::Debug for RequestUserInputTool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RequestUserInputTool")
            .field("session_id", &self.session_id)
            .field("run_id", &self.run_id)
            .finish_non_exhaustive()
    }
}

#[must_use]
pub fn request_user_input_tool(
    broker: Arc<dyn UserInputBroker>,
    session_id: SessionId,
    run_id: RunId,
) -> Arc<dyn ToolExecutor> {
    Arc::new(RequestUserInputTool {
        broker,
        session_id,
        run_id,
    })
}

impl ToolExecutor for RequestUserInputTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: REQUEST_USER_INPUT_TOOL.into(),
            description: "Ask the user one to three short questions when their input is required to continue. This supplies data only and cannot grant permission or approval.".into(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["questions"],
                "properties": {
                    "questions": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 3,
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["id", "header", "prompt", "options", "secret"],
                            "properties": {
                                "id": { "type": "string", "minLength": 1, "maxLength": 64 },
                                "header": { "type": "string", "minLength": 1, "maxLength": 64 },
                                "prompt": { "type": "string", "minLength": 1, "maxLength": 1000 },
                                "options": {
                                    "type": "array",
                                    "maxItems": 3,
                                    "items": {
                                        "type": "object",
                                        "additionalProperties": false,
                                        "required": ["label", "value"],
                                        "properties": {
                                            "label": { "type": "string", "minLength": 1, "maxLength": 80 },
                                            "value": { "type": "string", "maxLength": 4000 },
                                            "description": { "type": ["string", "null"], "maxLength": 240 }
                                        }
                                    }
                                },
                                "secret": { "type": "boolean" },
                                "autoResolutionMs": { "type": ["integer", "null"], "minimum": 60000, "maximum": 240000 },
                                "defaultAnswer": { "type": ["string", "null"], "maxLength": 4000 }
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
            match serde_json::from_value::<UserInputArguments>(invocation.call.arguments.clone()) {
                Ok(arguments) => arguments,
                Err(error) => {
                    return Box::pin(async move {
                        Err(ToolExecutionError::Failed(format!(
                            "invalid user-input request: {error}"
                        )))
                    });
                }
            };
        let broker = Arc::clone(&self.broker);
        let session_id = self.session_id.clone();
        let run_id = self.run_id.clone();
        Box::pin(async move {
            validate_questions(&arguments.questions)?;
            let created_at_ms = now_ms();
            let expires_at_ms = arguments
                .questions
                .iter()
                .filter_map(|question| question.auto_resolution_ms)
                .min()
                .and_then(|timeout| {
                    i64::try_from(timeout)
                        .ok()
                        .and_then(|timeout| created_at_ms.checked_add(timeout))
                });
            let contains_secret = arguments.questions.iter().any(|question| question.secret);
            let request = UserInputRequestRecord {
                id: UserInputRequestId::random(),
                session_id,
                run_id,
                run_generation: invocation.run_generation,
                item_id: ItemId::random(),
                questions: arguments.questions,
                display_answers: Vec::new(),
                status: UserInputStatus::Pending,
                expires_at_ms,
                created_at_ms,
                resolved_at_ms: None,
                resolved_by: None,
            };
            let outcome = broker
                .request(request, invocation.cancellation)
                .await
                .map_err(|error| ToolExecutionError::Failed(error.to_string()))?;
            let model_content = answer_content(outcome.action, &outcome.answers);
            Ok(ToolResult::succeeded(
                &invocation.call,
                model_content,
                json!({
                    "requestId": outcome.request.id,
                    "action": outcome.action,
                    "answerCount": outcome.answers.len(),
                    "containsSecret": contains_secret,
                    "redactForPersistence": true
                }),
            ))
        })
    }

    fn waits_for_cancellation(&self) -> bool {
        true
    }
}

fn validate_questions(questions: &[UserInputQuestion]) -> Result<(), ToolExecutionError> {
    if questions.is_empty() || questions.len() > 3 {
        return Err(ToolExecutionError::Failed(
            "user input must contain one to three questions".into(),
        ));
    }
    if questions
        .iter()
        .any(|question| question.secret && question.auto_resolution_ms.is_some())
    {
        return Err(ToolExecutionError::Failed(
            "secret questions cannot be auto-resolved".into(),
        ));
    }
    Ok(())
}

fn answer_content(action: UserInputResolutionAction, answers: &[UserInputAnswer]) -> String {
    let mapped = answers
        .iter()
        .map(|answer| json!({ "questionId": answer.question_id, "answer": answer.value }))
        .collect::<Vec<Value>>();
    serde_json::to_string(&json!({ "action": action, "answers": mapped }))
        .unwrap_or_else(|_| "{\"answers\":[]}".into())
}

fn now_ms() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use futures_util::FutureExt;
    use hachimi_protocol::{UserInputResolution, UserInputStatus};
    use hachimi_user_input::{
        UserInputCancelFuture, UserInputError, UserInputFuture, UserInputOutcome,
        UserInputResolveFuture,
    };

    use super::*;

    #[derive(Default)]
    struct ImmediateBroker {
        seen: Mutex<Option<UserInputRequestRecord>>,
    }

    impl UserInputBroker for ImmediateBroker {
        fn request(
            &self,
            request: UserInputRequestRecord,
            _cancellation: tokio_util::sync::CancellationToken,
        ) -> UserInputFuture {
            *self.seen.lock().expect("seen") = Some(request.clone());
            async move {
                Ok(UserInputOutcome {
                    request: UserInputRequestRecord {
                        status: UserInputStatus::Resolved,
                        ..request
                    },
                    action: UserInputResolutionAction::Submit,
                    answers: vec![UserInputAnswer {
                        question_id: "choice".into(),
                        value: "continue".into(),
                    }],
                })
            }
            .boxed()
        }

        fn resolve(&self, _resolution: UserInputResolution) -> UserInputResolveFuture {
            async { Err(UserInputError::Unavailable) }.boxed()
        }

        fn cancel_run(&self, _run_id: RunId) -> UserInputCancelFuture {
            async { Ok(0) }.boxed()
        }
    }

    #[tokio::test]
    async fn answers_are_returned_to_the_model_but_marked_for_persistence_redaction() {
        let broker = Arc::new(ImmediateBroker::default());
        let tool = request_user_input_tool(
            broker.clone(),
            SessionId::from("session"),
            RunId::from("run"),
        );
        let call = crate::ToolCall {
            id: hachimi_protocol::ToolCallId::from("call"),
            name: REQUEST_USER_INPUT_TOOL.into(),
            arguments: json!({
                "questions": [{
                    "id": "choice",
                    "header": "Choice",
                    "prompt": "Continue?",
                    "options": [{"label": "Continue", "value": "continue", "description": null}],
                    "secret": true,
                    "autoResolutionMs": null,
                    "defaultAnswer": null
                }]
            }),
            step_revision: 1,
            tool_plan_hash: "fixture-plan".into(),
            registry_revision: "fixture-registry".into(),
        };
        let result = tool
            .execute(ToolInvocation {
                call,
                entry_profile: hachimi_protocol::EntryProfile::Workbench,
                workload: hachimi_protocol::WorkloadKind::Coding,
                behavior_mode: hachimi_protocol::BehaviorMode::Default,
                run_generation: 7,
                step_revision: 1,
                tool_plan_hash: "fixture-plan".into(),
                registry_revision: "fixture-registry".into(),
                cancellation: tokio_util::sync::CancellationToken::new(),
            })
            .await
            .expect("tool result");
        assert!(result.model_content.contains("continue"));
        assert_eq!(result.structured_content["redactForPersistence"], true);
        let seen = broker.seen.lock().expect("seen").clone().expect("request");
        assert_eq!(seen.run_generation, 7);
    }
}
