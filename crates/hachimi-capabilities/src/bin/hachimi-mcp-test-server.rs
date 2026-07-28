//! Deterministic stdio fixture used by MCP transport integration tests.

use std::{
    io::{BufRead, Write},
    time::Duration,
};

use hachimi_capabilities::MCP_PROTOCOL_VERSION;
use serde_json::{Value, json};

fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let duplicate_resource_cursor = arguments
        .iter()
        .any(|argument| argument == "--duplicate-resource-cursor");
    let network_probe_address = arguments
        .windows(2)
        .find(|pair| pair[0] == "--network-probe-address")
        .map(|pair| pair[1].clone());
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    let mut lines = stdin.lock().lines();
    while let Some(line) = lines.next() {
        let line = line.expect("fixture stdin");
        let request: Value = serde_json::from_str(&line).expect("fixture request");
        let Some(id) = request.get("id").cloned() else {
            continue;
        };
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if method == "tools/call"
            && let Some(progress_token) = request.pointer("/params/_meta/progressToken")
        {
            write_message(
                &mut stdout,
                &json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/progress",
                    "params": {
                        "progressToken": progress_token,
                        "progress": 1,
                        "total": 2,
                        "message": "fixture working"
                    }
                }),
            );
        }
        let result = match method {
            "initialize" => json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {
                    "tools": { "listChanged": false },
                    "resources": { "subscribe": false, "listChanged": false },
                    "prompts": { "listChanged": false }
                },
                "serverInfo": { "name": "hachimi-mcp-fixture", "version": "1.0.0" }
            }),
            "tools/list" => {
                let mut tools = vec![
                    json!({
                        "name": "echo",
                        "description": "Return the supplied text.",
                        "inputSchema": {
                            "type": "object",
                            "properties": { "text": { "type": "string" } },
                            "required": ["text"],
                            "additionalProperties": false
                        }
                    }),
                    json!({
                        "name": "wait",
                        "description": "Wait for a test-controlled duration.",
                        "inputSchema": {
                            "type": "object",
                            "properties": { "milliseconds": { "type": "integer" } },
                            "required": ["milliseconds"],
                            "additionalProperties": false
                        }
                    }),
                    json!({
                        "name": "elicit",
                        "description": "Request one boolean value from the MCP client.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {},
                            "additionalProperties": false
                        }
                    }),
                ];
                if network_probe_address.is_some() {
                    tools.push(json!({
                        "name": "network_probe",
                        "description": "Attempt the test-controlled TCP connection.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {},
                            "additionalProperties": false
                        }
                    }));
                }
                json!({ "tools": tools })
            }
            "ping" => json!({}),
            "tools/call"
                if request.pointer("/params/name").and_then(Value::as_str) == Some("elicit") =>
            {
                write_message(
                    &mut stdout,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": "fixture-elicitation-1",
                        "method": "elicitation/create",
                        "params": {
                            "mode": "form",
                            "message": "Allow this fixture request?",
                            "requestedSchema": {
                                "type": "object",
                                "properties": {
                                    "confirmed": { "type": "boolean", "title": "Confirm" }
                                },
                                "required": ["confirmed"]
                            }
                        }
                    }),
                );
                let response = lines
                    .next()
                    .expect("fixture elicitation response line")
                    .expect("fixture elicitation response");
                let response: Value =
                    serde_json::from_str(&response).expect("fixture elicitation JSON response");
                let action = response
                    .pointer("/result/action")
                    .and_then(Value::as_str)
                    .unwrap_or("cancel");
                let confirmed = response
                    .pointer("/result/content/confirmed")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                json!({
                    "content": [{
                        "type": "text",
                        "text": format!("elicitation {action}; confirmed={confirmed}")
                    }],
                    "structuredContent": { "action": action, "confirmed": confirmed },
                    "isError": action != "accept"
                })
            }
            "tools/call" => tool_call(&request, network_probe_address.as_deref()),
            "resources/list" => list_resources(&request, duplicate_resource_cursor),
            "resources/templates/list" => json!({
                "resourceTemplates": [{
                    "uriTemplate": "fixture://notes/{id}",
                    "name": "fixture-note",
                    "description": "A deterministic note resource.",
                    "mimeType": "text/plain"
                }]
            }),
            "resources/read" => read_resource(&request),
            "prompts/list" => json!({
                "prompts": [{
                    "name": "summarize-note",
                    "description": "Summarize a fixture note.",
                    "arguments": [{
                        "name": "topic",
                        "description": "Topic to emphasize.",
                        "required": true
                    }]
                }]
            }),
            "prompts/get" => get_prompt(&request),
            _ => {
                write_message(
                    &mut stdout,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32601, "message": "method not found" }
                    }),
                );
                continue;
            }
        };
        write_message(
            &mut stdout,
            &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        );
    }
}

fn list_resources(request: &Value, duplicate_cursor: bool) -> Value {
    if duplicate_cursor {
        return json!({
            "resources": [{ "uri": "fixture://loop", "name": "loop" }],
            "nextCursor": "same-cursor"
        });
    }
    match request.pointer("/params/cursor").and_then(Value::as_str) {
        None => json!({
            "resources": [{
                "uri": "fixture://notes/one",
                "name": "note-one",
                "description": "First fixture note.",
                "mimeType": "text/plain",
                "size": 20
            }],
            "nextCursor": "fixture-page-2"
        }),
        Some("fixture-page-2") => json!({
            "resources": [{
                "uri": "fixture://notes/two",
                "name": "note-two",
                "mimeType": "text/plain"
            }]
        }),
        Some(_) => json!({ "resources": [] }),
    }
}

fn read_resource(request: &Value) -> Value {
    let uri = request
        .pointer("/params/uri")
        .and_then(Value::as_str)
        .unwrap_or_default();
    json!({
        "contents": [{
            "uri": uri,
            "mimeType": "text/plain",
            "text": format!("fixture content for {uri}")
        }]
    })
}

fn get_prompt(request: &Value) -> Value {
    let topic = request
        .pointer("/params/arguments/topic")
        .and_then(Value::as_str)
        .unwrap_or("general");
    json!({
        "description": "Deterministic fixture prompt.",
        "messages": [{
            "role": "user",
            "content": {
                "type": "text",
                "text": format!("Summarize the note with emphasis on {topic}.")
            }
        }]
    })
}

fn tool_call(request: &Value, network_probe_address: Option<&str>) -> Value {
    let name = request
        .pointer("/params/name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let arguments = request.pointer("/params/arguments").unwrap_or(&Value::Null);
    match name {
        "echo" => {
            let text = arguments
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            json!({
                "content": [{ "type": "text", "text": text }],
                "structuredContent": { "echoed": true },
                "isError": false
            })
        }
        "wait" => {
            let milliseconds = arguments
                .get("milliseconds")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            std::thread::sleep(Duration::from_millis(milliseconds));
            json!({
                "content": [{ "type": "text", "text": "waited" }],
                "isError": false
            })
        }
        "network_probe" => {
            let connected = network_probe_address
                .and_then(|address| address.parse().ok())
                .is_some_and(|address| {
                    std::net::TcpStream::connect_timeout(&address, Duration::from_millis(750))
                        .is_ok()
                });
            json!({
                "content": [{ "type": "text", "text": format!("connected={connected}") }],
                "structuredContent": { "connected": connected },
                "isError": false
            })
        }
        _ => json!({
            "content": [{ "type": "text", "text": "unknown fixture tool" }],
            "isError": true
        }),
    }
}

fn write_message(stdout: &mut impl Write, value: &Value) {
    serde_json::to_writer(&mut *stdout, value).expect("fixture response");
    stdout.write_all(b"\n").expect("fixture newline");
    stdout.flush().expect("fixture flush");
}
