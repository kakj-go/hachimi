// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/app-server-protocol/src/protocol/v2/mcp.rs
// and codex-rs/app-server/tests/suite/v2/mcp_server_elicitation.rs.
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// Modified for Hachimi: UserInputBroker projection, Run generation binding, and fail-closed tasks.

use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use futures_util::FutureExt;
use hachimi_capabilities::{
    McpServerRequest, McpServerRequestFuture, McpServerRequestHandler, McpServerRequestResponse,
};
use hachimi_protocol::{
    ItemId, RunId, SessionId, UserInputOption, UserInputQuestion, UserInputRequestId,
    UserInputRequestRecord, UserInputResolutionAction, UserInputStatus,
};
use hachimi_storage::AgentStore;
use hachimi_user_input::UserInputBroker;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

const MAX_MESSAGE_CHARS: usize = 4_096;
const MAX_SCHEMA_PROPERTIES: usize = 3;
const OMIT_VALUE: &str = "__hachimi_mcp_omit__";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    String,
    Boolean,
    Number,
    Integer,
}

#[derive(Debug, Clone)]
struct ElicitationForm {
    questions: Vec<UserInputQuestion>,
    kinds: BTreeMap<String, FieldKind>,
    optional: BTreeMap<String, bool>,
}

struct BrokeredMcpElicitation {
    broker: Arc<dyn UserInputBroker>,
    session_id: SessionId,
    run_id: RunId,
    interactive: bool,
    store: Option<AgentStore>,
}

impl std::fmt::Debug for BrokeredMcpElicitation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrokeredMcpElicitation")
            .field("session_id", &self.session_id)
            .field("run_id", &self.run_id)
            .field("interactive", &self.interactive)
            .field("persists_needs_attention", &self.store.is_some())
            .finish_non_exhaustive()
    }
}

#[must_use]
pub fn mcp_elicitation_handler(
    broker: Arc<dyn UserInputBroker>,
    session_id: SessionId,
    run_id: RunId,
    interactive: bool,
) -> Arc<dyn McpServerRequestHandler> {
    Arc::new(BrokeredMcpElicitation {
        broker,
        session_id,
        run_id,
        interactive,
        store: None,
    })
}

#[must_use]
pub fn mcp_elicitation_handler_with_store(
    broker: Arc<dyn UserInputBroker>,
    store: AgentStore,
    session_id: SessionId,
    run_id: RunId,
    interactive: bool,
) -> Arc<dyn McpServerRequestHandler> {
    Arc::new(BrokeredMcpElicitation {
        broker,
        session_id,
        run_id,
        interactive,
        store: Some(store),
    })
}

impl McpServerRequestHandler for BrokeredMcpElicitation {
    fn handle(
        &self,
        request: McpServerRequest,
        cancellation: CancellationToken,
    ) -> McpServerRequestFuture {
        if request.method != "elicitation/create" {
            return async { McpServerRequestResponse::method_not_found() }.boxed();
        }
        if !self.interactive {
            let store = self.store.clone();
            let session_id = self.session_id.clone();
            let run_id = self.run_id.clone();
            let server_id = request.server_id;
            let request_id = request.request_id.as_json();
            let tool_call_id = request
                .correlation
                .as_ref()
                .map(|correlation| correlation.tool_call_id.clone());
            return async move {
                if let Some(store) = store {
                    let _ = store
                        .append_event(
                            &session_id,
                            Some(&run_id),
                            "mcp.elicitation.needs_attention",
                            json!({
                                "serverId": server_id,
                                "requestId": request_id,
                                "toolCallId": tool_call_id,
                                "reason": "interactive_input_unavailable",
                            }),
                        )
                        .await;
                }
                McpServerRequestResponse::result(json!({
                    "action": "cancel",
                    "_meta": { "hachimiReason": "interactive_input_unavailable" }
                }))
            }
            .boxed();
        }
        let Some(correlation) = request.correlation else {
            return async {
                McpServerRequestResponse::invalid_request(
                    "elicitation is not bound to an active Run",
                )
            }
            .boxed();
        };
        if correlation.session_id != self.session_id || correlation.run_id != self.run_id {
            return async {
                McpServerRequestResponse::invalid_request("elicitation Run correlation is stale")
            }
            .boxed();
        }
        let form = match parse_form(&request.server_id, &request.params) {
            Ok(form) => form,
            Err(message) => {
                return async move { McpServerRequestResponse::invalid_request(message) }.boxed();
            }
        };
        let broker = Arc::clone(&self.broker);
        let session_id = self.session_id.clone();
        let run_id = self.run_id.clone();
        async move {
            let created_at_ms = now_ms();
            let request = UserInputRequestRecord {
                id: UserInputRequestId::random(),
                session_id,
                run_id,
                run_generation: correlation.run_generation,
                item_id: ItemId::random(),
                questions: form.questions.clone(),
                display_answers: Vec::new(),
                status: UserInputStatus::Pending,
                expires_at_ms: None,
                created_at_ms,
                resolved_at_ms: None,
                resolved_by: None,
            };
            let outcome = match broker.request(request, cancellation).await {
                Ok(outcome) => outcome,
                Err(_) => {
                    return McpServerRequestResponse::result(json!({
                        "action": "cancel",
                        "_meta": { "hachimiReason": "input_cancelled_or_unavailable" }
                    }));
                }
            };
            match outcome.action {
                UserInputResolutionAction::Submit => {
                    match answers_to_content(&form, &outcome.answers) {
                        Ok(content) => McpServerRequestResponse::result(json!({
                            "action": "accept",
                            "content": content,
                        })),
                        Err(message) => McpServerRequestResponse::invalid_request(message),
                    }
                }
                UserInputResolutionAction::Decline => {
                    McpServerRequestResponse::result(json!({ "action": "decline" }))
                }
                UserInputResolutionAction::Cancel => {
                    McpServerRequestResponse::result(json!({ "action": "cancel" }))
                }
            }
        }
        .boxed()
    }
}

fn parse_form(server_id: &str, params: &Value) -> Result<ElicitationForm, String> {
    let object = params
        .as_object()
        .ok_or_else(|| "elicitation params must be an object".to_owned())?;
    let mode = object.get("mode").and_then(Value::as_str).unwrap_or("form");
    if mode != "form" {
        return Err("only bounded MCP form elicitation is supported".into());
    }
    let message = bounded_required(object.get("message"), "elicitation message")?;
    let schema = object
        .get("requestedSchema")
        .and_then(Value::as_object)
        .ok_or_else(|| "elicitation requestedSchema must be an object".to_owned())?;
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        return Err("elicitation schema type must be object".into());
    }
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| "elicitation schema properties are missing".to_owned())?;
    if properties.is_empty() || properties.len() > MAX_SCHEMA_PROPERTIES {
        return Err("elicitation must contain one to three fields".into());
    }
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .collect::<std::collections::BTreeSet<_>>()
        })
        .unwrap_or_default();

    let mut questions = Vec::with_capacity(properties.len());
    let mut kinds = BTreeMap::new();
    let mut optional = BTreeMap::new();
    for (id, schema) in properties {
        if id.trim().is_empty() || id.len() > 64 {
            return Err("elicitation field name is invalid".into());
        }
        let schema = schema
            .as_object()
            .ok_or_else(|| format!("elicitation field {id} is not an object"))?;
        let kind = match schema.get("type").and_then(Value::as_str) {
            Some("string") => FieldKind::String,
            Some("boolean") => FieldKind::Boolean,
            Some("number") => FieldKind::Number,
            Some("integer") => FieldKind::Integer,
            _ => return Err(format!("elicitation field {id} uses an unsupported type")),
        };
        let is_optional = !required.contains(id.as_str());
        let secret = kind == FieldKind::String
            && schema.get("format").and_then(Value::as_str) == Some("password");
        let mut options = field_options(kind, schema)?;
        if is_optional && !secret {
            if options.len() >= 3 {
                return Err(format!(
                    "optional elicitation field {id} has no room for a skip option"
                ));
            }
            options.push(UserInputOption {
                label: "Skip".into(),
                value: OMIT_VALUE.into(),
                description: Some("Do not include this optional field in the MCP response.".into()),
            });
        }
        let title = schema
            .get("title")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(id);
        let description = schema
            .get("description")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty());
        let prompt = description.map_or_else(
            || message.clone(),
            |description| format!("{message}\n\n{}", bounded(description, 2_048)),
        );
        let default_answer = if secret {
            None
        } else {
            schema.get("default").and_then(default_to_answer)
        };
        questions.push(UserInputQuestion {
            id: id.clone(),
            header: bounded(title, 64),
            prompt: bounded(
                &format!(
                    "MCP server {server_id} requests untrusted data. This does not grant permission.\n\n{prompt}"
                ),
                MAX_MESSAGE_CHARS,
            ),
            options,
            secret,
            auto_resolution_ms: None,
            default_answer,
        });
        kinds.insert(id.clone(), kind);
        optional.insert(id.clone(), is_optional);
    }
    Ok(ElicitationForm {
        questions,
        kinds,
        optional,
    })
}

fn field_options(
    kind: FieldKind,
    schema: &Map<String, Value>,
) -> Result<Vec<UserInputOption>, String> {
    if kind == FieldKind::Boolean {
        return Ok(vec![
            UserInputOption {
                label: "Yes".into(),
                value: "true".into(),
                description: None,
            },
            UserInputOption {
                label: "No".into(),
                value: "false".into(),
                description: None,
            },
        ]);
    }
    let Some(values) = schema.get("enum").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    if values.len() < 2 || values.len() > 3 {
        return Err("elicitation enum must contain two or three values".into());
    }
    values
        .iter()
        .map(|value| {
            let value = value
                .as_str()
                .ok_or_else(|| "elicitation enum values must be strings".to_owned())?;
            Ok(UserInputOption {
                label: bounded(value, 80),
                value: bounded(value, 4_000),
                description: None,
            })
        })
        .collect()
}

fn answers_to_content(
    form: &ElicitationForm,
    answers: &[hachimi_protocol::UserInputAnswer],
) -> Result<Value, String> {
    let by_id = answers
        .iter()
        .map(|answer| (answer.question_id.as_str(), answer.value.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut content = Map::new();
    for (id, kind) in &form.kinds {
        let answer = by_id
            .get(id.as_str())
            .ok_or_else(|| format!("elicitation answer for {id} is missing"))?;
        if *answer == OMIT_VALUE && form.optional.get(id).copied().unwrap_or(false) {
            continue;
        }
        let value = match kind {
            FieldKind::String => Value::String((*answer).to_owned()),
            FieldKind::Boolean => Value::Bool(
                answer
                    .parse::<bool>()
                    .map_err(|_| format!("elicitation answer for {id} is not boolean"))?,
            ),
            FieldKind::Number => json!(
                answer
                    .parse::<f64>()
                    .map_err(|_| format!("elicitation answer for {id} is not a number"))?
            ),
            FieldKind::Integer => json!(
                answer
                    .parse::<i64>()
                    .map_err(|_| format!("elicitation answer for {id} is not an integer"))?
            ),
        };
        content.insert(id.clone(), value);
    }
    Ok(Value::Object(content))
}

fn bounded_required(value: Option<&Value>, label: &str) -> Result<String, String> {
    let value = value
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{label} is missing"))?;
    Ok(bounded(value, MAX_MESSAGE_CHARS))
}

fn bounded(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn default_to_answer(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
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

    use hachimi_capabilities::{McpRunCorrelation, McpServerRequestId};
    use hachimi_protocol::{ToolCallId, UserInputAnswer, UserInputResolution};
    use hachimi_user_input::{
        UserInputCancelFuture, UserInputError, UserInputFuture, UserInputOutcome,
        UserInputResolveFuture,
    };

    use super::*;

    #[derive(Default)]
    struct ImmediateBroker {
        seen: Mutex<Option<UserInputRequestRecord>>,
        action: UserInputResolutionAction,
    }

    impl UserInputBroker for ImmediateBroker {
        fn request(
            &self,
            request: UserInputRequestRecord,
            _cancellation: CancellationToken,
        ) -> UserInputFuture {
            *self.seen.lock().expect("seen") = Some(request.clone());
            let action = self.action;
            async move {
                Ok(UserInputOutcome {
                    request: UserInputRequestRecord {
                        status: UserInputStatus::Resolved,
                        ..request
                    },
                    action,
                    answers: if action == UserInputResolutionAction::Submit {
                        vec![UserInputAnswer {
                            question_id: "confirmed".into(),
                            value: "true".into(),
                        }]
                    } else {
                        Vec::new()
                    },
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
    async fn form_round_trip_is_bound_to_run_and_returns_typed_content() {
        let session_id = SessionId::from("session");
        let run_id = RunId::from("run");
        let broker = Arc::new(ImmediateBroker::default());
        let handler =
            mcp_elicitation_handler(broker.clone(), session_id.clone(), run_id.clone(), true);
        let response = handler
            .handle(
                McpServerRequest {
                    server_id: "fixture".into(),
                    request_id: McpServerRequestId::String("ask-1".into()),
                    method: "elicitation/create".into(),
                    params: json!({
                        "mode": "form",
                        "message": "Allow this request?",
                        "requestedSchema": {
                            "type": "object",
                            "properties": { "confirmed": { "type": "boolean" } },
                            "required": ["confirmed"]
                        }
                    }),
                    correlation: Some(McpRunCorrelation {
                        session_id,
                        run_id,
                        run_generation: 4,
                        tool_call_id: ToolCallId::from("tool-call"),
                    }),
                },
                CancellationToken::new(),
            )
            .await;
        assert_eq!(
            response,
            McpServerRequestResponse::result(json!({
                "action": "accept",
                "content": { "confirmed": true }
            }))
        );
        let request = broker.seen.lock().expect("seen").clone().expect("request");
        assert_eq!(request.run_generation, 4);
        assert!(
            request.questions[0]
                .prompt
                .contains("does not grant permission")
        );
    }

    #[tokio::test]
    async fn non_interactive_runs_cancel_without_creating_user_input() {
        let broker = Arc::new(ImmediateBroker::default());
        let handler = mcp_elicitation_handler(
            broker.clone(),
            SessionId::from("session"),
            RunId::from("run"),
            false,
        );
        let response = handler
            .handle(
                McpServerRequest {
                    server_id: "fixture".into(),
                    request_id: McpServerRequestId::Integer(1),
                    method: "elicitation/create".into(),
                    params: json!({}),
                    correlation: None,
                },
                CancellationToken::new(),
            )
            .await;
        assert!(matches!(response, McpServerRequestResponse::Result(_)));
        assert!(broker.seen.lock().expect("seen").is_none());
    }

    #[tokio::test]
    async fn explicit_decline_is_returned_without_form_content() {
        let session_id = SessionId::from("session");
        let run_id = RunId::from("run");
        let broker = Arc::new(ImmediateBroker {
            action: UserInputResolutionAction::Decline,
            ..ImmediateBroker::default()
        });
        let handler = mcp_elicitation_handler(broker, session_id.clone(), run_id.clone(), true);
        let response = handler
            .handle(
                McpServerRequest {
                    server_id: "fixture".into(),
                    request_id: McpServerRequestId::Integer(2),
                    method: "elicitation/create".into(),
                    params: json!({
                        "message": "Allow?",
                        "requestedSchema": {
                            "type": "object",
                            "properties": { "confirmed": { "type": "boolean" } },
                            "required": ["confirmed"]
                        }
                    }),
                    correlation: Some(McpRunCorrelation {
                        session_id,
                        run_id,
                        run_generation: 1,
                        tool_call_id: ToolCallId::from("tool-call"),
                    }),
                },
                CancellationToken::new(),
            )
            .await;
        assert_eq!(
            response,
            McpServerRequestResponse::result(json!({ "action": "decline" }))
        );
    }
}
