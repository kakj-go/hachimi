use std::io::{self, BufRead, Write};
use std::sync::Arc;

use hachimi_browser::{
    CEF_IPC_PROTOCOL_VERSION, CefHostCommandEnvelope, CefHostFailure, CefHostMessage,
    CefHostResponse,
};
use parking_lot::Mutex;

use crate::tab_manager::TabManager;

#[derive(Clone)]
pub struct EventSink {
    stdout: Arc<Mutex<io::Stdout>>,
}

impl Default for EventSink {
    fn default() -> Self {
        Self {
            stdout: Arc::new(Mutex::new(io::stdout())),
        }
    }
}

impl EventSink {
    pub fn send(&self, message: &CefHostMessage) {
        let Ok(encoded) = serde_json::to_string(message) else {
            return;
        };
        let mut stdout = self.stdout.lock();
        let _ = writeln!(stdout, "{encoded}");
        let _ = stdout.flush();
    }

    pub fn response(&self, request_id: u64, result: Result<CefHostResponse, CefHostFailure>) {
        self.send(&CefHostMessage::Response { request_id, result });
    }

    pub fn fatal(&self, code: impl Into<String>, message: impl Into<String>) {
        self.send(&CefHostMessage::Fatal {
            code: code.into(),
            message: message.into(),
        });
    }
}

pub fn run_command_loop(manager: TabManager, sink: &EventSink) -> bool {
    for line in io::stdin().lock().lines() {
        let line = match line {
            Ok(line) if !line.trim().is_empty() => line,
            Ok(_) => continue,
            Err(error) => {
                sink.fatal("cef_ipc_read_failed", error.to_string());
                break;
            }
        };
        let envelope = match serde_json::from_str::<CefHostCommandEnvelope>(&line) {
            Ok(envelope) => envelope,
            Err(error) => {
                sink.fatal("cef_ipc_decode_failed", error.to_string());
                continue;
            }
        };
        if envelope.protocol_version != CEF_IPC_PROTOCOL_VERSION {
            sink.response(
                envelope.request_id,
                Err(CefHostFailure::new(
                    "cef_ipc_version_mismatch",
                    format!(
                        "expected protocol {}, received {}",
                        CEF_IPC_PROTOCOL_VERSION, envelope.protocol_version
                    ),
                    false,
                )),
            );
            continue;
        }
        let shutdown = matches!(envelope.command, hachimi_browser::CefHostCommand::Shutdown);
        manager.dispatch(envelope);
        if shutdown {
            return true;
        }
    }
    false
}
