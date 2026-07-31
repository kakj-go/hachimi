use std::{
    io::{self, Read},
    time::Duration,
};

use serde_json::{Value, json};

fn main() {
    let mode = std::env::args()
        .skip(1)
        .find_map(|argument| argument.strip_prefix("--mode=").map(str::to_owned))
        .unwrap_or_else(|| "success".into());
    if mode == "timeout" {
        std::thread::sleep(Duration::from_secs(30));
        return;
    }
    if mode == "malformed" {
        println!("not-json");
        return;
    }
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).expect("stdin");
    let request: Value = serde_json::from_str(input.trim()).expect("JSON-RPC request");
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    if mode == "error" {
        println!(
            "{}",
            json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":"fixture failure"}})
        );
        return;
    }
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "hook.invoke" => {
            let event = params
                .get("event")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .replace('.', "_");
            json!({
                "resultCode": format!("hook_{event}_ok"),
                "metadata": [{"key":"fixture","value":"deterministic"}]
            })
        }
        "health" if params.get("account").is_some() => json!({"state":"healthy"}),
        "health" => json!({"status":"healthy"}),
        "invoke" | "webhook" | "poll" => json!({
            "method": method,
            "ok": true,
            "secretInArgvOrEnvironment": secret_in_process_metadata(&params)
        }),
        "revoke" | "configure" | "start" | "stop" | "reload" | "ack" => {
            json!({"ok":true,"method":method})
        }
        "receive" => params.get("envelope").cloned().unwrap_or(Value::Null),
        "deliver" => json!({
            "delivered": true,
            "retryable": false,
            "resultCode": "fixture_delivered"
        }),
        _ => json!({"ok":true,"method":method}),
    };
    println!("{}", json!({"jsonrpc":"2.0","id":id,"result":result}));
}

fn secret_in_process_metadata(params: &Value) -> bool {
    let secret = params
        .get("credential")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if secret.is_empty() {
        return false;
    }
    std::env::args().any(|argument| argument.contains(secret))
        || std::env::vars().any(|(key, value)| key.contains(secret) || value.contains(secret))
}
