//! Loopback-only MCP echo server for testing saved Streamable HTTP configuration.

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use serde_json::{Value, json};

use crate::MCP_PROTOCOL_VERSION;

const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub struct McpEchoServer {
    address: SocketAddr,
    url: String,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl McpEchoServer {
    pub fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stopping);
        let thread = thread::Builder::new()
            .name("hachimi-mcp-echo".into())
            .spawn(move || serve(listener, &stop))?;
        Ok(Self {
            address,
            url: format!("http://{address}/mcp"),
            stopping,
            thread: Some(thread),
        })
    }

    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }
}

impl Drop for McpEchoServer {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(100));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(listener: TcpListener, stopping: &AtomicBool) {
    while !stopping.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, peer)) if peer.ip().is_loopback() => {
                // The embedded Echo endpoint is a bounded self-test server.
                // Serving one request at a time avoids detached connection
                // threads outliving a refresh/stop cycle under heavy tests.
                let _ = handle_connection(stream);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            // Windows can surface a client that disconnects between the TCP
            // handshake and accept as ConnectionAborted. That connection is
            // gone, but the listener remains healthy and must keep serving.
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionAborted
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(_) => break,
        }
    }
}

fn handle_connection(mut stream: TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let request = read_request(&mut stream)?;
    let Some((head, body)) = request.split_once("\r\n\r\n") else {
        return write_status(&mut stream, "400 Bad Request", None);
    };
    let mut head_lines = head.lines();
    let Some(request_line) = head_lines.next() else {
        return write_status(&mut stream, "400 Bad Request", None);
    };
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap_or_default();
    let path = request_parts.next().unwrap_or_default();
    if path != "/mcp" {
        return write_status(&mut stream, "404 Not Found", None);
    }
    if method == "DELETE" {
        return write_status(&mut stream, "204 No Content", None);
    }
    if method != "POST" {
        return write_status(&mut stream, "405 Method Not Allowed", None);
    }
    let payload: Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(_) => return write_status(&mut stream, "400 Bad Request", None),
    };
    let Some(id) = payload.get("id").cloned() else {
        return write_status(&mut stream, "202 Accepted", None);
    };
    let method = payload
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let result = match method {
        "initialize" => json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {
                "tools": { "listChanged": false },
                "resources": { "subscribe": false, "listChanged": false },
                "prompts": { "listChanged": false }
            },
            "serverInfo": { "name": "hachimi-echo", "version": env!("CARGO_PKG_VERSION") }
        }),
        "tools/list" => json!({
            "tools": [{
                "name": "echo",
                "description": "Return the supplied text without modification.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "text": { "type": "string", "description": "Text to echo." }
                    },
                    "required": ["text"],
                    "additionalProperties": false
                }
            }]
        }),
        "tools/call" => echo_call(&payload),
        "resources/list" => json!({
            "resources": [{
                "uri": "hachimi-echo://about",
                "name": "about",
                "description": "Metadata for the bounded Hachimi Echo test server.",
                "mimeType": "text/plain"
            }]
        }),
        "resources/templates/list" => json!({ "resourceTemplates": [] }),
        "resources/read" => json!({
            "contents": [{
                "uri": payload.pointer("/params/uri").and_then(Value::as_str).unwrap_or("hachimi-echo://about"),
                "mimeType": "text/plain",
                "text": "Hachimi Echo is a local bounded MCP transport test service."
            }]
        }),
        "prompts/list" => json!({
            "prompts": [{
                "name": "echo-instruction",
                "description": "Build a deterministic Echo instruction.",
                "arguments": [{ "name": "text", "required": true }]
            }]
        }),
        "prompts/get" => json!({
            "description": "Deterministic Echo instruction.",
            "messages": [{
                "role": "user",
                "content": {
                    "type": "text",
                    "text": payload.pointer("/params/arguments/text").and_then(Value::as_str).unwrap_or_default()
                }
            }]
        }),
        "ping" => json!({}),
        _ => {
            return write_json(
                &mut stream,
                &json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": "method not found" }
                }),
            );
        }
    };
    write_json(
        &mut stream,
        &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
    )
}

fn echo_call(payload: &Value) -> Value {
    let name = payload
        .pointer("/params/name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if name != "echo" {
        return json!({
            "content": [{ "type": "text", "text": "unknown tool" }],
            "isError": true
        });
    }
    let text = payload
        .pointer("/params/arguments/text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": { "echoed": text },
        "isError": false
    })
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 2048];
    let header_end = loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() > MAX_HEADER_BYTES + MAX_BODY_BYTES {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        if bytes.len() > MAX_HEADER_BYTES {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
    };
    let headers =
        std::str::from_utf8(&bytes[..header_end]).map_err(|_| std::io::ErrorKind::InvalidData)?;
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.split_once(':').and_then(|(name, value)| {
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
        })
        .unwrap_or(0);
    if content_length > MAX_BODY_BYTES {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    while bytes.len().saturating_sub(header_end) < content_length {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    bytes.truncate(header_end + content_length);
    String::from_utf8(bytes).map_err(|_| std::io::ErrorKind::InvalidData.into())
}

fn write_json(stream: &mut TcpStream, body: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(body)?;
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(&body)
}

fn write_status(stream: &mut TcpStream, status: &str, body: Option<&str>) -> std::io::Result<()> {
    let body = body.unwrap_or_default();
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, time::Duration};

    use serde_json::json;
    use tokio_util::sync::CancellationToken;
    use url::Url;

    use super::*;
    use crate::{McpHttpClient, McpHttpServerConfig};

    #[tokio::test]
    async fn loopback_echo_supports_real_initialize_discovery_and_call() {
        let server = McpEchoServer::start().expect("echo server");
        let client = McpHttpClient::connect(
            McpHttpServerConfig {
                server_id: "hachimi-echo".into(),
                url: Url::parse(server.url()).expect("echo URL"),
                headers: BTreeMap::new(),
                startup_timeout: Duration::from_secs(2),
                request_timeout: Duration::from_secs(2),
                max_message_bytes: 1024 * 1024,
            },
            CancellationToken::new(),
        )
        .await
        .expect("connect");
        let tools = client
            .list_tools(CancellationToken::new())
            .await
            .expect("tools");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");
        let result = client
            .call_tool(
                "echo",
                json!({ "text": "Hachimi" }),
                CancellationToken::new(),
            )
            .await
            .expect("call");
        assert_eq!(result.content[0]["text"], "Hachimi");
        let resources = client
            .list_resources(CancellationToken::new())
            .await
            .expect("resources");
        assert_eq!(resources[0].uri, "hachimi-echo://about");
        let prompt = client
            .get_prompt(
                "echo-instruction",
                BTreeMap::from([("text".into(), "Hello".into())]),
                CancellationToken::new(),
            )
            .await
            .expect("prompt");
        assert_eq!(prompt.messages[0].content["text"], "Hello");
    }
}
