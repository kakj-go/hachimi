use std::{
    collections::BTreeMap,
    fs,
    net::TcpStream,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use hachimi_browser::{
    BrokerActionResult, BrokerObservation, BrowserBroker, BrowserBrokerFuture, BrowserHost,
    BrowserHostError, SystemBrowserClock,
};
use hachimi_protocol::{
    BrowserAction, BrowserActionRequest, BrowserCapability, BrowserFileToken,
    BrowserImportedDownload, BrowserNetworkPolicy, BrowserNetworkRuleKind,
    BrowserPermissionDecision, BrowserProfileKind, BrowserSessionId, BrowserSessionStatus, RunId,
    SandboxCapabilityReport, SandboxReadiness, SessionId,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tungstenite::{Message, WebSocket, connect, stream::MaybeTlsStream};

struct CdpBroker {
    socket: Mutex<WebSocket<MaybeTlsStream<TcpStream>>>,
    next_id: Mutex<u64>,
    staged: Mutex<BTreeMap<String, PathBuf>>,
}

impl CdpBroker {
    fn connect(url: &str) -> Result<Self, BrowserHostError> {
        let (mut socket, _) = connect(url).map_err(broker_error)?;
        if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .map_err(broker_error)?;
            stream
                .set_write_timeout(Some(Duration::from_secs(10)))
                .map_err(broker_error)?;
        }
        let broker = Self {
            socket: Mutex::new(socket),
            next_id: Mutex::new(1),
            staged: Mutex::new(BTreeMap::new()),
        };
        broker.call("Page.enable", json!({}))?;
        broker.call("Runtime.enable", json!({}))?;
        Ok(broker)
    }

    fn call(&self, method: &str, params: Value) -> Result<Value, BrowserHostError> {
        let id = {
            let mut next = self.next_id.lock().map_err(|_| broker_error("id lock"))?;
            let id = *next;
            *next = next.saturating_add(1);
            id
        };
        let mut socket = self
            .socket
            .lock()
            .map_err(|_| broker_error("socket lock"))?;
        socket
            .send(Message::Text(
                json!({"id": id, "method": method, "params": params})
                    .to_string()
                    .into(),
            ))
            .map_err(broker_error)?;
        loop {
            let message = socket.read().map_err(broker_error)?;
            let Message::Text(text) = message else {
                continue;
            };
            let value: Value = serde_json::from_str(&text).map_err(broker_error)?;
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = value.get("error") {
                return Err(broker_error(error));
            }
            return Ok(value.get("result").cloned().unwrap_or_else(|| json!({})));
        }
    }

    fn evaluate(&self, expression: String) -> Result<Value, BrowserHostError> {
        self.call(
            "Runtime.evaluate",
            json!({"expression": expression, "returnByValue": true, "awaitPromise": true}),
        )
        .and_then(|value| {
            value
                .pointer("/result/value")
                .cloned()
                .ok_or_else(|| broker_error("CDP evaluation omitted a value"))
        })
    }
}

impl BrowserBroker for CdpBroker {
    fn attest_profile<'a>(&'a self, profile: BrowserProfileKind) -> BrowserBrokerFuture<'a, ()> {
        Box::pin(async move {
            (profile == BrowserProfileKind::Isolated)
                .then_some(())
                .ok_or(BrowserHostError::BrokerUnsupportedMode)
        })
    }

    fn start<'a>(
        &'a self,
        _session_id: &'a BrowserSessionId,
        profile: BrowserProfileKind,
        initial_url: Option<&'a str>,
        _policy: BrowserNetworkPolicy,
        _identity: Option<&'a str>,
    ) -> BrowserBrokerFuture<'a, ()> {
        Box::pin(async move {
            if profile != BrowserProfileKind::Isolated {
                return Err(BrowserHostError::BrokerUnsupportedMode);
            }
            let _ = initial_url;
            Ok(())
        })
    }

    fn observe<'a>(
        &'a self,
        _session_id: &'a BrowserSessionId,
    ) -> BrowserBrokerFuture<'a, BrokerObservation> {
        Box::pin(async move {
            let page = self.evaluate(
                "({url: location.href, title: document.title, text: document.body?.innerText || '', width: innerWidth, height: innerHeight})".into(),
            )?;
            let screenshot = self.call("Page.captureScreenshot", json!({"format": "png"}))?;
            let screenshot_png = screenshot
                .get("data")
                .and_then(Value::as_str)
                .map(|data| BASE64_STANDARD.decode(data).map_err(broker_error))
                .transpose()?;
            Ok(BrokerObservation {
                url: page
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
                title: page
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
                text: page
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
                screenshot_png,
                viewport_width: page
                    .get("width")
                    .and_then(Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok()),
                viewport_height: page
                    .get("height")
                    .and_then(Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok()),
            })
        })
    }

    fn act<'a>(
        &'a self,
        _session_id: &'a BrowserSessionId,
        _expected_origin: &'a str,
        action: &'a BrowserAction,
    ) -> BrowserBrokerFuture<'a, BrokerActionResult> {
        Box::pin(async move {
            let output = match action {
                BrowserAction::Click { selector } => {
                    self.evaluate(format!(
                        "document.querySelector({})?.click(); location.href",
                        json!(selector)
                    ))?;
                    json!({"url": self.evaluate("location.href".into())?})
                }
                BrowserAction::Fill { selector, text }
                | BrowserAction::TypeText { selector, text } => {
                    self.evaluate(format!("const e=document.querySelector({}); if(e){{e.value={};e.dispatchEvent(new Event('input',{{bubbles:true}}));}}; location.href", json!(selector), json!(text)))?;
                    json!({"url": self.evaluate("location.href".into())?})
                }
                BrowserAction::Scroll {
                    delta_x, delta_y, ..
                } => {
                    self.evaluate(format!("scrollBy({delta_x},{delta_y}); location.href"))?;
                    json!({"url": self.evaluate("location.href".into())?})
                }
                BrowserAction::Navigate { url } => {
                    self.call("Page.navigate", json!({"url": url}))?;
                    json!({"url": url})
                }
                BrowserAction::Reload { ignore_cache } => {
                    self.call("Page.reload", json!({"ignoreCache": ignore_cache}))?;
                    json!({"url": self.evaluate("location.href".into())?})
                }
                _ => json!({"url": self.evaluate("location.href".into())?}),
            };
            Ok(BrokerActionResult {
                result_code: "ok".into(),
                output: Some(output),
            })
        })
    }

    fn stage_upload<'a>(
        &'a self,
        session: &'a BrowserSessionId,
        source: &'a Path,
    ) -> BrowserBrokerFuture<'a, BrowserFileToken> {
        Box::pin(async move {
            let bytes = fs::read(source).map_err(broker_error)?;
            let token = format!("stress-upload-{}", uuid::Uuid::new_v4());
            self.staged
                .lock()
                .map_err(|_| broker_error("staging lock"))?
                .insert(token.clone(), source.to_owned());
            Ok(BrowserFileToken {
                browser_session_id: session.clone(),
                token,
                file_name: source
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into(),
                size: bytes.len() as u64,
                sha256: hex_sha256(&bytes),
                expires_at_ms: now_ms() + 60_000,
            })
        })
    }

    fn import_download<'a>(
        &'a self,
        session: &'a BrowserSessionId,
        token: &'a str,
        destination: &'a Path,
    ) -> BrowserBrokerFuture<'a, BrowserImportedDownload> {
        Box::pin(async move {
            let bytes = b"Hachimi Browser Host download fixture\n";
            fs::write(destination, bytes).map_err(broker_error)?;
            Ok(BrowserImportedDownload {
                browser_session_id: session.clone(),
                download_token: token.into(),
                destination: destination.display().to_string(),
                size: bytes.len() as u64,
                sha256: hex_sha256(bytes),
            })
        })
    }

    fn set_network_policy<'a>(
        &'a self,
        _session: &'a BrowserSessionId,
        _policy: BrowserNetworkPolicy,
    ) -> BrowserBrokerFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn resume<'a>(
        &'a self,
        _session: &'a BrowserSessionId,
        _group: &'a str,
        _policy: BrowserNetworkPolicy,
    ) -> BrowserBrokerFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn take_over<'a>(&'a self, _session: &'a BrowserSessionId) -> BrowserBrokerFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn stop<'a>(&'a self, _session: &'a BrowserSessionId) -> BrowserBrokerFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
#[ignore = "opt-in real managed Chromium BrowserHost stress"]
async fn real_managed_chromium_runs_through_browser_host_api() {
    let ws = std::env::var("HACHIMI_BROWSER_CDP_WS_URL").expect("CDP websocket URL");
    let fixture_url = std::env::var("HACHIMI_BROWSER_FIXTURE_URL").expect("fixture URL");
    let seconds = std::env::var("HACHIMI_STRESS_PHASE_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30_u64)
        .clamp(1, 450);
    let broker = Arc::new(CdpBroker::connect(&ws).expect("CDP broker"));
    eprintln!("browser_host_stress_stage=broker_connected");
    let host = BrowserHost::with_broker(Arc::new(SystemBrowserClock), broker);
    let session_id = SessionId::from("browser-real-stress-session");
    let run_id = RunId::from("browser-real-stress-run");
    let session = host
        .start_session(
            BrowserProfileKind::Isolated,
            session_id.clone(),
            run_id.clone(),
            1,
            Some(&fixture_url),
            &sandbox(),
            None,
        )
        .await
        .expect("start BrowserHost session");
    eprintln!("browser_host_stress_stage=session_started");
    host.grant_site_permission(
        &session.id,
        &session_id,
        &run_id,
        session.revision,
        &fixture_url,
        vec![
            BrowserCapability::Observe,
            BrowserCapability::Act,
            BrowserCapability::Upload,
            BrowserCapability::Download,
        ],
        BrowserPermissionDecision::AllowSession,
        BrowserNetworkRuleKind::Document,
        true,
        "stress:test",
        None,
    )
    .await
    .expect("grant");
    eprintln!("browser_host_stress_stage=permission_granted");
    let temp = tempfile::tempdir().expect("temp");
    let upload = temp.path().join("upload.txt");
    fs::write(&upload, "upload").expect("upload fixture");
    let token = host
        .stage_upload(&session.id, &run_id, &upload)
        .await
        .expect("stage upload");
    host.import_download(
        &session.id,
        &run_id,
        "download-token",
        &temp.path().join("download.txt"),
    )
    .await
    .expect("import download");
    assert!(!token.token.is_empty());
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut iterations = 0_u64;
    while Instant::now() < deadline {
        let observation = host
            .observe(&session.id, &run_id, 1)
            .await
            .expect("observe");
        let action = BrowserActionRequest {
            browser_session_id: session.id.clone(),
            observation_id: observation.id.clone(),
            run_generation: 1,
            expected_revision: observation.browser_revision,
            action: BrowserAction::Click {
                selector: "#increment".into(),
            },
        };
        host.authorize_action(&run_id, &action).await.expect("act");
        assert_eq!(
            host.authorize_action(&run_id, &action).await,
            Err(BrowserHostError::StaleObservation)
        );
        iterations += 1;
    }
    let current = host
        .session_snapshot(&session.id, &run_id)
        .expect("snapshot");
    host.revoke_site_permission(
        &session.id,
        &session_id,
        &run_id,
        current.revision,
        &fixture_url,
    )
    .await
    .expect("revoke");
    host.take_over(&session.id, &run_id)
        .await
        .expect("takeover");
    host.resume(&session.id, &run_id).await.expect("resume");
    let stopped = host.stop(&session.id, &run_id).await.expect("stop");
    assert_eq!(stopped.status, BrowserSessionStatus::Stopped);
    assert!(iterations > 0);
    eprintln!("browser_host_real_stress_iterations={iterations}");
}

fn sandbox() -> SandboxCapabilityReport {
    SandboxCapabilityReport {
        backend: "stress".into(),
        readiness: SandboxReadiness::Ready,
        os_enforced: true,
        filesystem_enforced: true,
        process_enforced: true,
        network_enforced: true,
        version: Some("1".into()),
        stable_error_code: None,
        diagnostics: Vec::new(),
    }
}
fn broker_error(error: impl std::fmt::Display) -> BrowserHostError {
    BrowserHostError::Broker(error.to_string())
}
fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
