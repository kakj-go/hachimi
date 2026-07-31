use futures_util::StreamExt;
use hachimi_model_runtime::ModelRuntimeError;
use hachimi_protocol::{
    ModelCompactionRequest, ModelCompactionResult, ModelEvent, ModelFinishReason, ModelMessage,
    ModelRequest, ModelRole, ModelToolCall, TokenUsage, ToolCallId,
};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{
    OpenAiCompatibleRuntime, is_context_overflow, provider_error_detail, request_error, send_event,
};

pub(super) async fn send_model_request(
    runtime: &OpenAiCompatibleRuntime,
    request: ModelRequest,
    cancellation: &CancellationToken,
    sender: &mpsc::Sender<Result<ModelEvent, ModelRuntimeError>>,
) -> Result<(), ModelRuntimeError> {
    if !request.tools.is_empty() && !runtime.capabilities.tool_calls {
        return Err(ModelRuntimeError::UnsupportedCapability("tool_calls"));
    }
    let endpoint = format!(
        "{}/responses",
        runtime.settings.base_url.trim_end_matches('/')
    );
    let mut body = json!({
        "model": runtime.settings.model_name,
        "input": messages_to_items(&request.messages),
        "stream": true,
        "store": false,
    });
    if !request.tools.is_empty() {
        body["tools"] = Value::Array(
            request
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.input_schema,
                        "strict": runtime.capabilities.strict_json_schema,
                    })
                })
                .collect(),
        );
        body["parallel_tool_calls"] =
            Value::Bool(request.parallel_tool_calls && runtime.capabilities.parallel_tool_calls);
    }
    if let Some(max_output_tokens) = request.max_output_tokens.or_else(|| {
        (runtime.settings.max_output_tokens > 0).then_some(runtime.settings.max_output_tokens)
    }) {
        body["max_output_tokens"] = Value::from(max_output_tokens);
    }
    if runtime.capabilities.reasoning_summary {
        body["reasoning"] = json!({ "summary": "auto" });
    }
    let response = send_request(runtime, endpoint, body, cancellation).await?;
    let is_event_stream = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("text/event-stream"));
    if !is_event_stream {
        let value: Value = response.json().await.map_err(|_| {
            ModelRuntimeError::InvalidStream("Responses returned invalid JSON".into())
        })?;
        for event in response_events(&value, runtime.capabilities.reasoning_summary)? {
            send_event(sender, event, cancellation).await?;
        }
        return Ok(());
    }
    stream_response(
        response,
        runtime.capabilities.reasoning_summary,
        cancellation,
        sender,
    )
    .await
}

pub(super) async fn compact(
    runtime: &OpenAiCompatibleRuntime,
    request: ModelCompactionRequest,
    cancellation: CancellationToken,
) -> Result<ModelCompactionResult, ModelRuntimeError> {
    let compact_endpoint = format!(
        "{}/responses/compact",
        runtime.settings.base_url.trim_end_matches('/')
    );
    let compact_body = json!({
        "model": runtime.settings.model_name,
        "input": messages_to_items(&request.messages),
    });
    let compact_response =
        send_request(runtime, compact_endpoint, compact_body, &cancellation).await?;
    let compacted: Value = compact_response.json().await.map_err(|_| {
        ModelRuntimeError::InvalidStream("remote compaction returned invalid JSON".into())
    })?;
    let output = compacted
        .get("output")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .ok_or_else(|| {
            ModelRuntimeError::InvalidStream("remote compaction omitted opaque output".into())
        })?;
    if output
        .iter()
        .any(|item| item.get("type").and_then(Value::as_str).is_none())
    {
        return Err(ModelRuntimeError::InvalidStream(
            "remote compaction returned a malformed opaque Item".into(),
        ));
    }
    let mut summary_input = output.clone();
    summary_input.push(json!({
        "role": "user",
        "content": "Return a visible Markdown continuity record with these headings: Current goal, Constraints and decisions, Completed work, Pending work, Important identifiers, Verification and failures. Preserve facts and unresolved work. Do not expose hidden reasoning, encrypted content, secrets, grants, approvals, credentials, cookies, host tokens, or screenshot data.",
    }));
    let summary_endpoint = format!(
        "{}/responses",
        runtime.settings.base_url.trim_end_matches('/')
    );
    let summary_body = json!({
        "model": runtime.settings.model_name,
        "input": summary_input,
        "store": false,
        "stream": false,
        "max_output_tokens": request.max_output_tokens,
    });
    let summary_response =
        send_request(runtime, summary_endpoint, summary_body, &cancellation).await?;
    let summary: Value = summary_response.json().await.map_err(|_| {
        ModelRuntimeError::InvalidStream("compacted summary returned invalid JSON".into())
    })?;
    let (text, usage) = visible_text_and_usage(&summary)?;
    Ok(ModelCompactionResult {
        replacement_messages: vec![ModelMessage::assistant(text, Vec::new())],
        usage,
    })
}

async fn send_request(
    runtime: &OpenAiCompatibleRuntime,
    endpoint: String,
    body: Value,
    cancellation: &CancellationToken,
) -> Result<reqwest::Response, ModelRuntimeError> {
    let mut request = runtime.client.post(endpoint).json(&body);
    if let Some(secret) = runtime.api_key.as_deref().filter(|value| !value.is_empty()) {
        request = request.bearer_auth(secret);
    }
    let response = tokio::select! {
        () = cancellation.cancelled() => return Err(ModelRuntimeError::Cancelled),
        response = request.send() => response.map_err(|error| ModelRuntimeError::Provider(request_error(&error).to_string()))?,
    };
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let response_body = response.text().await.unwrap_or_default();
    let detail = provider_error_detail(&response_body)
        .unwrap_or_else(|| "provider rejected the Responses request".into());
    if is_context_overflow(status, &detail) {
        return Err(ModelRuntimeError::ContextOverflow);
    }
    Err(ModelRuntimeError::Provider(format!(
        "HTTP {status}: {detail}"
    )))
}

async fn stream_response(
    response: reqwest::Response,
    allow_reasoning_summary: bool,
    cancellation: &CancellationToken,
    sender: &mpsc::Sender<Result<ModelEvent, ModelRuntimeError>>,
) -> Result<(), ModelRuntimeError> {
    let mut stream = response.bytes_stream();
    let mut pending = Vec::<u8>::new();
    let mut completed = false;
    loop {
        let next = tokio::select! {
            () = cancellation.cancelled() => return Err(ModelRuntimeError::Cancelled),
            next = stream.next() => next,
        };
        let Some(chunk) = next else { break };
        let chunk = chunk
            .map_err(|error| ModelRuntimeError::Provider(request_error(&error).to_string()))?;
        pending.extend_from_slice(&chunk);
        while let Some(position) = pending.iter().position(|byte| *byte == b'\n') {
            let mut line = pending.drain(..=position).collect::<Vec<_>>();
            while matches!(line.last(), Some(b'\n' | b'\r')) {
                line.pop();
            }
            let line = std::str::from_utf8(&line).map_err(|_| {
                ModelRuntimeError::InvalidStream("Responses stream was not UTF-8".into())
            })?;
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            let value: Value = serde_json::from_str(data).map_err(|_| {
                ModelRuntimeError::InvalidStream("Responses emitted invalid SSE JSON".into())
            })?;
            for event in stream_events(&value, allow_reasoning_summary)? {
                if matches!(event, ModelEvent::Completed { .. }) {
                    completed = true;
                }
                send_event(sender, event, cancellation).await?;
            }
        }
    }
    if completed {
        Ok(())
    } else {
        Err(ModelRuntimeError::InvalidStream(
            "Responses stream ended before response.completed".into(),
        ))
    }
}

fn messages_to_items(messages: &[ModelMessage]) -> Vec<Value> {
    let mut items = Vec::new();
    for message in messages {
        match message.role {
            ModelRole::System => items.push(json!({
                "role": "developer",
                "content": message.content,
            })),
            ModelRole::User if !message.input_images.is_empty() => {
                let mut content = vec![json!({
                    "type": "input_text",
                    "text": message.content,
                })];
                content.extend(
                    message
                        .input_images
                        .iter()
                        .filter(|image| {
                            matches!(
                                image.media_type.as_str(),
                                "image/png" | "image/jpeg" | "image/webp" | "image/gif"
                            )
                        })
                        .map(|image| {
                            json!({
                                "type": "input_image",
                                "image_url": format!(
                                    "data:{};base64,{}",
                                    image.media_type, image.data_base64
                                ),
                                "detail": "high",
                            })
                        }),
                );
                items.push(json!({ "role": "user", "content": content }));
            }
            ModelRole::User => items.push(json!({
                "role": "user",
                "content": message.content,
            })),
            ModelRole::Assistant => {
                if !message.content.is_empty() {
                    items.push(json!({
                        "role": "assistant",
                        "content": message.content,
                    }));
                }
                items.extend(message.tool_calls.iter().map(|call| {
                    json!({
                        "type": "function_call",
                        "call_id": call.id.as_str(),
                        "name": call.name,
                        "arguments": call.arguments.to_string(),
                    })
                }));
            }
            ModelRole::Tool => items.push(json!({
                "type": "function_call_output",
                "call_id": message.tool_call_id.as_ref().map(ToolCallId::as_str),
                "output": message.content,
            })),
        }
    }
    items
}

fn response_events(
    value: &Value,
    allow_reasoning_summary: bool,
) -> Result<Vec<ModelEvent>, ModelRuntimeError> {
    let object = value.get("object").and_then(Value::as_str);
    if object != Some("response") {
        return Err(ModelRuntimeError::InvalidStream(
            "Responses JSON omitted object=response".into(),
        ));
    }
    if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
        return Err(ModelRuntimeError::Provider(provider_error(error)));
    }
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| ModelRuntimeError::InvalidStream("Responses JSON omitted status".into()))?;
    let mut events = output_item_events(value, allow_reasoning_summary)?;
    if let Some(usage) = response_usage(value)? {
        events.push(ModelEvent::Usage { usage });
    }
    events.push(ModelEvent::Completed {
        finish_reason: status_finish_reason(status, value)?,
    });
    Ok(events)
}

fn stream_events(
    value: &Value,
    allow_reasoning_summary: bool,
) -> Result<Vec<ModelEvent>, ModelRuntimeError> {
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| ModelRuntimeError::InvalidStream("Responses event omitted type".into()))?;
    match event_type {
        "response.output_text.delta" => Ok(vec![ModelEvent::TextDelta {
            delta: required_string(value, "delta", "output-text delta")?.into(),
        }]),
        "response.reasoning_summary_text.delta" if allow_reasoning_summary => {
            Ok(vec![ModelEvent::ReasoningDelta {
                delta: required_string(value, "delta", "reasoning-summary delta")?.into(),
            }])
        }
        "response.reasoning_summary_text.delta" => Ok(Vec::new()),
        "response.function_call_arguments.delta" => Ok(vec![ModelEvent::ToolCallDelta {
            index: value
                .get("output_index")
                .and_then(Value::as_u64)
                .and_then(|index| u32::try_from(index).ok())
                .ok_or_else(|| {
                    ModelRuntimeError::InvalidStream(
                        "function-call delta omitted output_index".into(),
                    )
                })?,
            id: None,
            name_delta: String::new(),
            arguments_delta: required_string(value, "delta", "function-call delta")?.into(),
        }]),
        "response.output_item.added" => {
            let Some(item) = value.get("item") else {
                return Err(ModelRuntimeError::InvalidStream(
                    "output_item.added omitted item".into(),
                ));
            };
            if item.get("type").and_then(Value::as_str) != Some("function_call") {
                return Ok(Vec::new());
            }
            Ok(vec![ModelEvent::ToolCallDelta {
                index: value
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .and_then(|index| u32::try_from(index).ok())
                    .ok_or_else(|| {
                        ModelRuntimeError::InvalidStream(
                            "function-call item omitted output_index".into(),
                        )
                    })?,
                id: Some(ToolCallId::new(required_string(
                    item,
                    "call_id",
                    "function-call item",
                )?)),
                name_delta: required_string(item, "name", "function-call item")?.into(),
                arguments_delta: item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
            }])
        }
        "response.output_item.done" => {
            let Some(item) = value.get("item") else {
                return Err(ModelRuntimeError::InvalidStream(
                    "output_item.done omitted item".into(),
                ));
            };
            if item.get("type").and_then(Value::as_str) != Some("function_call") {
                return Ok(Vec::new());
            }
            // Streaming function calls are assembled from the indexed added/delta events.
            // Validate the terminal Item, but do not emit a second completed call with a
            // different provider output index.
            let _ = parse_function_call(item)?;
            Ok(Vec::new())
        }
        "response.completed" => {
            let response = value.get("response").ok_or_else(|| {
                ModelRuntimeError::InvalidStream("response.completed omitted response".into())
            })?;
            let mut events = Vec::new();
            if let Some(usage) = response_usage(response)? {
                events.push(ModelEvent::Usage { usage });
            }
            let status = response
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("completed");
            events.push(ModelEvent::Completed {
                finish_reason: status_finish_reason(status, response)?,
            });
            Ok(events)
        }
        "response.failed" | "error" => Err(ModelRuntimeError::Provider(provider_error(
            value.get("response").unwrap_or(value),
        ))),
        "response.incomplete" => {
            let response = value.get("response").ok_or_else(|| {
                ModelRuntimeError::InvalidStream("response.incomplete omitted response".into())
            })?;
            Ok(vec![ModelEvent::Completed {
                finish_reason: status_finish_reason("incomplete", response)?,
            }])
        }
        "response.created"
        | "response.in_progress"
        | "response.content_part.added"
        | "response.content_part.done"
        | "response.output_text.done"
        | "response.function_call_arguments.done"
        | "response.reasoning_summary_part.added"
        | "response.reasoning_summary_part.done"
        | "response.reasoning_summary_text.done" => Ok(Vec::new()),
        _ => Ok(Vec::new()),
    }
}

fn output_item_events(
    value: &Value,
    allow_reasoning_summary: bool,
) -> Result<Vec<ModelEvent>, ModelRuntimeError> {
    let output = value
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ModelRuntimeError::InvalidStream("Responses JSON omitted output Items".into())
        })?;
    let mut events = Vec::new();
    for item in output {
        match item.get("type").and_then(Value::as_str) {
            Some("message") => {
                let content = item
                    .get("content")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        ModelRuntimeError::InvalidStream("message Item omitted content".into())
                    })?;
                for part in content {
                    if part.get("type").and_then(Value::as_str) == Some("output_text") {
                        events.push(ModelEvent::TextDelta {
                            delta: required_string(part, "text", "output_text")?.into(),
                        });
                    }
                }
            }
            Some("function_call") => events.push(ModelEvent::ToolCallCompleted {
                call: parse_function_call(item)?,
            }),
            Some("reasoning") if allow_reasoning_summary => {
                if let Some(summary) = item.get("summary").and_then(Value::as_array) {
                    for part in summary {
                        if part.get("type").and_then(Value::as_str) == Some("summary_text") {
                            events.push(ModelEvent::ReasoningDelta {
                                delta: required_string(part, "text", "reasoning summary")?.into(),
                            });
                        }
                    }
                }
            }
            Some("reasoning") => {}
            Some(other) => {
                return Err(ModelRuntimeError::InvalidStream(format!(
                    "Responses returned an unrequested output Item: {other}"
                )));
            }
            None => {
                return Err(ModelRuntimeError::InvalidStream(
                    "Responses output Item omitted type".into(),
                ));
            }
        }
    }
    Ok(events)
}

fn parse_function_call(value: &Value) -> Result<ModelToolCall, ModelRuntimeError> {
    let arguments = required_string(value, "arguments", "function-call Item")?;
    Ok(ModelToolCall {
        id: ToolCallId::new(required_string(value, "call_id", "function-call Item")?),
        name: required_string(value, "name", "function-call Item")?.into(),
        arguments: serde_json::from_str(arguments).map_err(|_| {
            ModelRuntimeError::InvalidStream(
                "Responses function-call arguments were not valid JSON".into(),
            )
        })?,
    })
}

fn visible_text_and_usage(value: &Value) -> Result<(String, TokenUsage), ModelRuntimeError> {
    if value.get("object").and_then(Value::as_str) != Some("response")
        || value.get("status").and_then(Value::as_str) != Some("completed")
    {
        return Err(ModelRuntimeError::InvalidStream(
            "compacted summary did not complete as a Response".into(),
        ));
    }
    let events = output_item_events(value, false)?;
    if events
        .iter()
        .any(|event| !matches!(event, ModelEvent::TextDelta { .. }))
    {
        return Err(ModelRuntimeError::InvalidStream(
            "compacted summary returned a tool call".into(),
        ));
    }
    let text = events
        .into_iter()
        .filter_map(|event| match event {
            ModelEvent::TextDelta { delta } => Some(delta),
            _ => None,
        })
        .collect::<String>();
    if text.trim().is_empty() {
        return Err(ModelRuntimeError::InvalidStream(
            "compacted summary omitted visible text".into(),
        ));
    }
    Ok((text, response_usage(value)?.unwrap_or_default()))
}

fn response_usage(value: &Value) -> Result<Option<TokenUsage>, ModelRuntimeError> {
    let Some(usage) = value.get("usage") else {
        return Ok(None);
    };
    if usage.is_null() {
        return Ok(None);
    }
    let input_tokens = usage
        .get("input_tokens")
        .and_then(Value::as_u64)
        .ok_or_else(|| ModelRuntimeError::InvalidStream("usage omitted input_tokens".into()))?;
    let output_tokens = usage
        .get("output_tokens")
        .and_then(Value::as_u64)
        .ok_or_else(|| ModelRuntimeError::InvalidStream("usage omitted output_tokens".into()))?;
    Ok(Some(TokenUsage {
        input_tokens,
        output_tokens,
    }))
}

fn status_finish_reason(
    status: &str,
    value: &Value,
) -> Result<ModelFinishReason, ModelRuntimeError> {
    match status {
        "completed" => Ok(ModelFinishReason::Stop),
        "incomplete" => {
            let reason = value
                .pointer("/incomplete_details/reason")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            Ok(match reason {
                "max_output_tokens" => ModelFinishReason::Length,
                "content_filter" => ModelFinishReason::ContentFilter,
                _ => ModelFinishReason::Unknown,
            })
        }
        "failed" | "cancelled" => Err(ModelRuntimeError::Provider(provider_error(value))),
        _ => Err(ModelRuntimeError::InvalidStream(format!(
            "Responses returned unknown status: {status}"
        ))),
    }
}

fn provider_error(value: &Value) -> String {
    value
        .pointer("/error/message")
        .or_else(|| value.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("Responses request failed")
        .chars()
        .take(1_000)
        .collect()
}

fn required_string<'a>(
    value: &'a Value,
    field: &str,
    kind: &str,
) -> Result<&'a str, ModelRuntimeError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| ModelRuntimeError::InvalidStream(format!("{kind} omitted {field}")))
}

#[cfg(test)]
mod tests {
    use futures_util::StreamExt;
    use hachimi_model_runtime::{ModelRuntime, ModelRuntimeError};
    use hachimi_protocol::{
        LlmSettings, ModelCompactionRequest, ModelEvent, ModelFinishReason, ModelMessage,
        ModelRequest, ProviderProtocolKind, StructuredOutputMode,
    };
    use serde_json::json;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };
    use tokio_util::sync::CancellationToken;

    use super::{output_item_events, response_events, stream_events};
    use crate::OpenAiCompatibleRuntime;

    #[test]
    fn strict_json_response_maps_text_tools_summary_and_usage() {
        let events = response_events(
            &json!({
                "object": "response",
                "status": "completed",
                "output": [
                    { "type": "reasoning", "summary": [{ "type": "summary_text", "text": "Checked inputs." }] },
                    { "type": "message", "content": [{ "type": "output_text", "text": "done" }] },
                    { "type": "function_call", "call_id": "call-1", "name": "read", "arguments": "{\"path\":\"a\"}" }
                ],
                "usage": { "input_tokens": 7, "output_tokens": 3 }
            }),
            true,
        )
        .expect("response");
        assert!(events.iter().any(|event| matches!(event, ModelEvent::ReasoningDelta { delta } if delta == "Checked inputs.")));
        assert!(events.iter().any(
            |event| matches!(event, ModelEvent::ToolCallCompleted { call } if call.name == "read")
        ));
        assert!(matches!(
            events.last(),
            Some(ModelEvent::Completed {
                finish_reason: ModelFinishReason::Stop
            })
        ));
    }

    #[test]
    fn hidden_reasoning_is_not_projected_without_summary_capability() {
        let events = output_item_events(
            &json!({ "output": [{
                "type": "reasoning",
                "encrypted_content": "opaque",
                "summary": [{ "type": "summary_text", "text": "public" }],
                "content": [{ "type": "reasoning_text", "text": "hidden" }]
            }] }),
            false,
        )
        .expect("output");
        assert!(events.is_empty());
    }

    #[test]
    fn streaming_summary_accepts_only_public_summary_event() {
        let visible = stream_events(
            &json!({ "type": "response.reasoning_summary_text.delta", "delta": "safe" }),
            true,
        )
        .expect("visible");
        assert!(
            matches!(visible.as_slice(), [ModelEvent::ReasoningDelta { delta }] if delta == "safe")
        );
        let raw = stream_events(
            &json!({ "type": "response.reasoning_text.delta", "delta": "hidden" }),
            true,
        )
        .expect("ignored");
        assert!(raw.is_empty());
    }

    #[tokio::test]
    async fn responses_sse_conformance_maps_text_usage_and_completion() {
        let (base_url, server) = serve(vec![
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"HACHIMI_OK\"}\n\n\
             data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":3,\"output_tokens\":2}}}\n\n"
                .into(),
        ])
        .await;
        let runtime = runtime(base_url, false, false);
        let mut stream = runtime.stream(
            ModelRequest {
                messages: vec![ModelMessage::user("hello")],
                tools: Vec::new(),
                parallel_tool_calls: false,
                max_output_tokens: Some(32),
            },
            CancellationToken::new(),
        );
        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event.expect("event"));
        }
        assert!(events.iter().any(
            |event| matches!(event, ModelEvent::TextDelta { delta } if delta == "HACHIMI_OK")
        ));
        assert!(events.iter().any(|event| matches!(
            event,
            ModelEvent::Usage { usage } if usage.input_tokens == 3 && usage.output_tokens == 2
        )));
        server.await.expect("server");
    }

    #[tokio::test]
    async fn remote_compaction_uses_opaque_item_then_returns_visible_summary() {
        let (base_url, server) = serve(vec![
            json!({
                "output": [{ "type": "compaction", "encrypted_content": "opaque" }]
            })
            .to_string(),
            json!({
                "object": "response",
                "status": "completed",
                "output": [{
                    "type": "message",
                    "content": [{ "type": "output_text", "text": "## Current goal\nContinue safely.\n## Pending work\nNone." }]
                }],
                "usage": { "input_tokens": 8, "output_tokens": 6 }
            })
            .to_string(),
        ])
        .await;
        let runtime = runtime(base_url, false, true);
        let result = runtime
            .compact(
                ModelCompactionRequest {
                    messages: vec![ModelMessage::user("long context")],
                    max_output_tokens: 256,
                },
                CancellationToken::new(),
            )
            .await
            .expect("compact");
        assert!(
            result.replacement_messages[0]
                .content
                .contains("Current goal")
        );
        server.await.expect("server");
    }

    #[tokio::test]
    async fn cancellation_interrupts_a_pending_responses_request() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base_url = format!("http://{}/v1", listener.local_addr().expect("address"));
        let server = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.expect("accept");
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        });
        let runtime = runtime(base_url, false, false);
        let cancellation = CancellationToken::new();
        let mut stream = runtime.stream(
            ModelRequest {
                messages: vec![ModelMessage::user("cancel")],
                tools: Vec::new(),
                parallel_tool_calls: false,
                max_output_tokens: None,
            },
            cancellation.clone(),
        );
        cancellation.cancel();
        assert!(matches!(
            stream.next().await,
            Some(Err(ModelRuntimeError::Cancelled))
        ));
        server.abort();
    }

    fn runtime(
        base_url: String,
        reasoning_summary: bool,
        remote_compaction: bool,
    ) -> OpenAiCompatibleRuntime {
        OpenAiCompatibleRuntime::tool_calling(
            LlmSettings {
                base_url,
                model_name: "mock-responses".into(),
                protocol: ProviderProtocolKind::Responses,
                reasoning_summary,
                remote_compaction,
                structured_output_mode: StructuredOutputMode::Disabled,
                ..LlmSettings::default()
            },
            None,
        )
        .expect("runtime")
    }

    async fn serve(bodies: Vec<String>) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base_url = format!("http://{}/v1", listener.local_addr().expect("address"));
        let server = tokio::spawn(async move {
            for body in bodies {
                let (mut socket, _) = listener.accept().await.expect("accept");
                let request = read_request(&mut socket).await;
                assert!(request.starts_with("POST /v1/responses"));
                assert!(request.contains("\"store\":false") || request.contains("/compact"));
                let content_type = if body.starts_with("data:") {
                    "text/event-stream"
                } else {
                    "application/json"
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.expect("write");
            }
        });
        (base_url, server)
    }

    async fn read_request(socket: &mut TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = socket.read(&mut buffer).await.expect("read");
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
            let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.strip_prefix("content-length: ")
                        .or_else(|| line.strip_prefix("Content-Length: "))
                })
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or_default();
            if bytes.len() >= header_end + 4 + content_length {
                break;
            }
        }
        String::from_utf8(bytes).expect("request utf8")
    }
}
