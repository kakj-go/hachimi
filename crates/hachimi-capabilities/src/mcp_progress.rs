// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/app-server-protocol/src/protocol/v2/mcp.rs and
// codex-rs/core/src/mcp_tool_call.rs.
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// Modified for Hachimi: MCP progress-token validation and Session/Run/generation correlation.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use serde_json::Value;

use crate::McpRunCorrelation;

const MAX_PROGRESS_MESSAGE_CHARS: usize = 1_024;

#[derive(Debug, Clone, PartialEq)]
pub struct McpProgressNotification {
    pub server_id: String,
    pub correlation: McpRunCorrelation,
    pub progress: f64,
    pub total: Option<f64>,
    pub message: Option<String>,
}

pub type McpProgressFuture = BoxFuture<'static, ()>;

pub trait McpProgressHandler: Send + Sync {
    fn progress(&self, notification: McpProgressNotification) -> McpProgressFuture;
}

pub(crate) async fn dispatch_progress_notification(
    server_id: &str,
    message: &Value,
    correlation: Option<McpRunCorrelation>,
    handler: Option<Arc<dyn McpProgressHandler>>,
) -> bool {
    if message.get("method").and_then(Value::as_str) != Some("notifications/progress") {
        return false;
    }
    let Some(correlation) = correlation else {
        return true;
    };
    let Some(handler) = handler else {
        return true;
    };
    let Some(params) = message.get("params").and_then(Value::as_object) else {
        return true;
    };
    let token_matches = params.get("progressToken").is_some_and(|token| {
        token.as_str() == Some(correlation.tool_call_id.as_str())
            || token
                .as_i64()
                .is_some_and(|token| token.to_string() == correlation.tool_call_id.as_str())
    });
    if !token_matches {
        return true;
    }
    let Some(progress) = params.get("progress").and_then(Value::as_f64) else {
        return true;
    };
    if !progress.is_finite() || progress.is_sign_negative() {
        return true;
    }
    let total = params.get("total").and_then(Value::as_f64);
    if total.is_some_and(|total| !total.is_finite() || total <= 0.0) {
        return true;
    }
    let message = params
        .get("message")
        .and_then(Value::as_str)
        .map(|message| message.chars().take(MAX_PROGRESS_MESSAGE_CHARS).collect());
    handler
        .progress(McpProgressNotification {
            server_id: server_id.to_owned(),
            correlation,
            progress,
            total,
            message,
        })
        .await;
    true
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use futures_util::FutureExt;
    use hachimi_protocol::{RunId, SessionId, ToolCallId};
    use serde_json::json;

    use super::*;

    #[derive(Default)]
    struct Capture(Mutex<Vec<McpProgressNotification>>);

    impl McpProgressHandler for Capture {
        fn progress(&self, notification: McpProgressNotification) -> McpProgressFuture {
            self.0.lock().expect("capture").push(notification);
            async {}.boxed()
        }
    }

    fn correlation() -> McpRunCorrelation {
        McpRunCorrelation {
            session_id: SessionId::from("session"),
            run_id: RunId::from("run"),
            run_generation: 3,
            tool_call_id: ToolCallId::from("call-1"),
        }
    }

    #[tokio::test]
    async fn only_matching_bounded_progress_is_forwarded() {
        let capture = Arc::new(Capture::default());
        assert!(
            dispatch_progress_notification(
                "fixture",
                &json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/progress",
                    "params": {
                        "progressToken": "call-1",
                        "progress": 2,
                        "total": 5,
                        "message": "working"
                    }
                }),
                Some(correlation()),
                Some(capture.clone()),
            )
            .await
        );
        assert_eq!(capture.0.lock().expect("capture").len(), 1);
        dispatch_progress_notification(
            "fixture",
            &json!({
                "method": "notifications/progress",
                "params": { "progressToken": "stale", "progress": 4 }
            }),
            Some(correlation()),
            Some(capture.clone()),
        )
        .await;
        assert_eq!(capture.0.lock().expect("capture").len(), 1);
    }
}
