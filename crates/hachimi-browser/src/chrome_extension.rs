use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use hachimi_protocol::{
    BrowserAction, BrowserFileToken, BrowserImportedDownload, BrowserNetworkPolicy,
    BrowserProfileKind, BrowserSessionId,
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::oneshot;

use crate::{
    BrokerActionResult, BrokerObservation, BrowserBroker, BrowserBrokerFuture, BrowserHostError,
    broker::{import_download_file, resolve_upload_token, stage_download_file, stage_upload_file},
};

const EXTENSION_COMMAND_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_SCREENSHOT_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionCommand {
    pub command_id: String,
    pub session_id: BrowserSessionId,
    #[serde(flatten)]
    pub kind: ExtensionCommandKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ExtensionCommandKind {
    Start {
        initial_url: Option<String>,
        task_tab_group: String,
        network_policy: BrowserNetworkPolicy,
    },
    SetNetworkPolicy {
        network_policy: BrowserNetworkPolicy,
    },
    Observe,
    Act {
        expected_origin: String,
        action: BrowserAction,
    },
    Resume {
        task_tab_group: String,
        network_policy: BrowserNetworkPolicy,
    },
    TakeOver,
    Stop,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionCommandResult {
    pub command_id: String,
    pub ok: bool,
    pub error_code: Option<String>,
    pub observation: Option<BrokerObservationPayload>,
    pub action: Option<BrokerActionPayload>,
    #[serde(default)]
    pub owner_tab_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrokerObservationPayload {
    pub url: String,
    pub title: String,
    pub text: String,
    #[serde(default)]
    pub screenshot_base64: Option<String>,
    #[serde(default)]
    pub viewport_width: Option<u32>,
    #[serde(default)]
    pub viewport_height: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrokerActionPayload {
    pub result_code: String,
    pub output: Option<Value>,
}

struct InFlightCommand {
    extension_identity: String,
    completion: oneshot::Sender<ExtensionCommandResult>,
}

#[derive(Default)]
struct ChromeExtensionState {
    tokens: BTreeMap<String, String>,
    sessions: BTreeMap<BrowserSessionId, ExtensionOwnedSession>,
    pending: VecDeque<(String, ExtensionCommand)>,
    in_flight: BTreeMap<String, InFlightCommand>,
}

#[derive(Clone)]
struct ExtensionOwnedSession {
    extension_identity: String,
    owner_tab_id: i64,
}

#[derive(Clone)]
pub struct ChromeExtensionBroker {
    state: Arc<Mutex<ChromeExtensionState>>,
    staging_root: PathBuf,
}

impl std::fmt::Debug for ChromeExtensionBroker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChromeExtensionBroker")
            .finish_non_exhaustive()
    }
}

impl ChromeExtensionBroker {
    #[must_use]
    pub fn new(staging_root: impl Into<PathBuf>) -> Self {
        Self {
            state: Arc::new(Mutex::new(ChromeExtensionState::default())),
            staging_root: staging_root.into(),
        }
    }

    pub fn register_identity(&self, identity: &str) -> Result<String, BrowserHostError> {
        let identity = identity.trim();
        if identity.is_empty() || identity.chars().count() > 256 {
            return Err(BrowserHostError::InvalidInput);
        }
        let token = format!("browser-extension-{}", uuid::Uuid::new_v4());
        let mut state = self.state.lock();
        state.tokens.retain(|_, current| current != identity);
        state.tokens.insert(token.clone(), identity.to_owned());
        Ok(token)
    }

    pub fn claim(&self, token: &str) -> Result<Option<ExtensionCommand>, BrowserHostError> {
        let mut state = self.state.lock();
        let identity = state
            .tokens
            .get(token)
            .cloned()
            .ok_or(BrowserHostError::ExtensionAuthenticationFailed)?;
        let Some(index) = state
            .pending
            .iter()
            .position(|(owner, _)| owner == &identity)
        else {
            return Ok(None);
        };
        let (_, command) = state
            .pending
            .remove(index)
            .ok_or(BrowserHostError::BrokerUnavailable)?;
        Ok(Some(command))
    }

    pub fn complete(
        &self,
        token: &str,
        result: ExtensionCommandResult,
    ) -> Result<(), BrowserHostError> {
        let mut state = self.state.lock();
        let identity = state
            .tokens
            .get(token)
            .cloned()
            .ok_or(BrowserHostError::ExtensionAuthenticationFailed)?;
        let command = state
            .in_flight
            .remove(&result.command_id)
            .ok_or(BrowserHostError::ExtensionCommandInvalid)?;
        if command.extension_identity != identity {
            state.in_flight.insert(result.command_id.clone(), command);
            return Err(BrowserHostError::ExtensionAuthenticationFailed);
        }
        command
            .completion
            .send(result)
            .map_err(|_| BrowserHostError::ExtensionCommandInvalid)
    }

    async fn request(
        &self,
        identity: String,
        session_id: BrowserSessionId,
        kind: ExtensionCommandKind,
    ) -> Result<ExtensionCommandResult, BrowserHostError> {
        let command_id = uuid::Uuid::new_v4().to_string();
        let command = ExtensionCommand {
            command_id: command_id.clone(),
            session_id,
            kind,
        };
        let (sender, receiver) = oneshot::channel();
        {
            let mut state = self.state.lock();
            if !state.tokens.values().any(|value| value == &identity) {
                return Err(BrowserHostError::ExtensionAuthenticationFailed);
            }
            state.in_flight.insert(
                command_id.clone(),
                InFlightCommand {
                    extension_identity: identity.clone(),
                    completion: sender,
                },
            );
            state.pending.push_back((identity, command));
        }
        let result = match tokio::time::timeout(EXTENSION_COMMAND_TIMEOUT, receiver).await {
            Ok(Ok(result)) => result,
            _ => {
                let mut state = self.state.lock();
                state.in_flight.remove(&command_id);
                state
                    .pending
                    .retain(|(_, command)| command.command_id != command_id);
                return Err(BrowserHostError::ExtensionCommandTimeout);
            }
        };
        if !result.ok {
            return Err(BrowserHostError::Broker(
                result
                    .error_code
                    .unwrap_or_else(|| "browser_extension_command_failed".into()),
            ));
        }
        Ok(result)
    }
}

impl BrowserBroker for ChromeExtensionBroker {
    fn start<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
        profile_kind: BrowserProfileKind,
        initial_url: Option<&'a str>,
        initial_network_policy: BrowserNetworkPolicy,
        extension_identity: Option<&'a str>,
    ) -> BrowserBrokerFuture<'a, ()> {
        let session_id = session_id.clone();
        let initial_url = initial_url.map(str::to_owned);
        let identity = extension_identity.map(str::to_owned);
        Box::pin(async move {
            if profile_kind != BrowserProfileKind::ChromeExtension {
                return Err(BrowserHostError::BrokerUnsupportedMode);
            }
            let identity = identity.ok_or(BrowserHostError::PairingInvalid)?;
            let result = self
                .request(
                    identity.clone(),
                    session_id.clone(),
                    ExtensionCommandKind::Start {
                        network_policy: initial_network_policy,
                        initial_url,
                        task_tab_group: format!("Hachimi {}", session_id.as_str()),
                    },
                )
                .await?;
            let owner_tab_id = result
                .owner_tab_id
                .filter(|tab_id| *tab_id > 0)
                .ok_or(BrowserHostError::ExtensionCommandInvalid)?;
            self.state.lock().sessions.insert(
                session_id,
                ExtensionOwnedSession {
                    extension_identity: identity,
                    owner_tab_id,
                },
            );
            Ok(())
        })
    }

    fn observe<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
    ) -> BrowserBrokerFuture<'a, BrokerObservation> {
        let session_id = session_id.clone();
        Box::pin(async move {
            let identity = self
                .state
                .lock()
                .sessions
                .get(&session_id)
                .map(|session| session.extension_identity.clone())
                .ok_or(BrowserHostError::SessionNotFound)?;
            let result = self
                .request(identity, session_id, ExtensionCommandKind::Observe)
                .await?;
            let observation = result
                .observation
                .ok_or(BrowserHostError::ExtensionCommandInvalid)?;
            let screenshot_png = observation
                .screenshot_base64
                .map(|encoded| {
                    let bytes = BASE64_STANDARD
                        .decode(encoded)
                        .map_err(|_| BrowserHostError::ExtensionCommandInvalid)?;
                    if bytes.len() > MAX_SCREENSHOT_BYTES
                        || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
                    {
                        return Err(BrowserHostError::ExtensionCommandInvalid);
                    }
                    Ok(bytes)
                })
                .transpose()?;
            let viewport = observation
                .viewport_width
                .zip(observation.viewport_height)
                .filter(|(width, height)| {
                    *width > 0 && *height > 0 && *width <= 32_768 && *height <= 32_768
                });
            Ok(BrokerObservation {
                url: observation.url,
                title: observation.title,
                text: observation.text,
                screenshot_png,
                viewport_width: viewport.map(|(width, _)| width),
                viewport_height: viewport.map(|(_, height)| height),
            })
        })
    }

    fn act<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
        expected_origin: &'a str,
        action: &'a BrowserAction,
    ) -> BrowserBrokerFuture<'a, BrokerActionResult> {
        let session_id = session_id.clone();
        let expected_origin = expected_origin.to_owned();
        let original_action = action.clone();
        let mut action = original_action.clone();
        Box::pin(async move {
            let owned_session = self
                .state
                .lock()
                .sessions
                .get(&session_id)
                .cloned()
                .ok_or(BrowserHostError::SessionNotFound)?;
            if let BrowserAction::Upload { file_token, .. } = &mut action {
                let upload_root = self.staging_root.join(session_id.as_str()).join("uploads");
                let candidate = resolve_upload_token(&session_id, &upload_root, file_token)?;
                *file_token = candidate.to_string_lossy().into_owned();
            }
            let result = self
                .request(
                    owned_session.extension_identity.clone(),
                    session_id.clone(),
                    ExtensionCommandKind::Act {
                        expected_origin,
                        action,
                    },
                )
                .await?;
            let returned_owner_tab_id = result
                .owner_tab_id
                .filter(|tab_id| *tab_id > 0)
                .ok_or(BrowserHostError::ExtensionCommandInvalid)?;
            let owner_change_expected = matches!(
                original_action,
                BrowserAction::TabNew { .. }
                    | BrowserAction::TabSwitch { .. }
                    | BrowserAction::TabClose { .. }
            );
            if !owner_change_expected && returned_owner_tab_id != owned_session.owner_tab_id {
                return Err(BrowserHostError::ExtensionCommandInvalid);
            }
            if owner_change_expected {
                let mut state = self.state.lock();
                let current = state
                    .sessions
                    .get_mut(&session_id)
                    .filter(|current| {
                        current.extension_identity == owned_session.extension_identity
                            && current.owner_tab_id == owned_session.owner_tab_id
                    })
                    .ok_or(BrowserHostError::ExtensionCommandInvalid)?;
                current.owner_tab_id = returned_owner_tab_id;
            }
            let mut action = result
                .action
                .ok_or(BrowserHostError::ExtensionCommandInvalid)?;
            if matches!(original_action, BrowserAction::Download { .. }) {
                let output = action
                    .output
                    .as_ref()
                    .ok_or(BrowserHostError::DownloadFailed)?;
                let owner_tab_id = output
                    .get("ownerTabId")
                    .and_then(Value::as_i64)
                    .ok_or(BrowserHostError::DownloadFailed)?;
                let download_id = output
                    .get("downloadId")
                    .and_then(Value::as_i64)
                    .filter(|download_id| *download_id >= 0)
                    .ok_or(BrowserHostError::DownloadFailed)?;
                if owner_tab_id != owned_session.owner_tab_id
                    || result.owner_tab_id != Some(owned_session.owner_tab_id)
                    || download_id == i64::MAX
                {
                    return Err(BrowserHostError::DownloadFailed);
                }
                let source = action
                    .output
                    .as_ref()
                    .and_then(|output| output.get("hostDownloadPath"))
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                    .ok_or(BrowserHostError::DownloadFailed)?;
                let declared_mime = action
                    .output
                    .as_ref()
                    .and_then(|output| output.get("declaredMime"))
                    .and_then(Value::as_str);
                let root = self
                    .staging_root
                    .join(session_id.as_str())
                    .join("downloads");
                let allow_unknown_type = match original_action {
                    BrowserAction::Download {
                        allow_unknown_type, ..
                    } => allow_unknown_type,
                    _ => false,
                };
                let (staged, validated_mime) = stage_download_file(
                    &session_id,
                    &root,
                    &source,
                    declared_mime,
                    allow_unknown_type,
                )?;
                action.output = Some(serde_json::json!({
                    "downloadToken": staged.token,
                    "fileName": staged.file_name,
                    "size": staged.size,
                    "sha256": staged.sha256,
                    "mime": validated_mime,
                    "expiresAtMs": staged.expires_at_ms,
                    "importRequired": true,
                }));
            }
            Ok(BrokerActionResult {
                result_code: action.result_code,
                output: action.output,
            })
        })
    }

    fn stage_upload<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
        source: &'a Path,
    ) -> BrowserBrokerFuture<'a, BrowserFileToken> {
        let session_id = session_id.clone();
        let source = source.to_path_buf();
        let root = self.staging_root.join(session_id.as_str()).join("uploads");
        Box::pin(async move {
            if !self.state.lock().sessions.contains_key(&session_id) {
                return Err(BrowserHostError::SessionNotFound);
            }
            tokio::task::spawn_blocking(move || stage_upload_file(&session_id, &root, &source))
                .await
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?
        })
    }

    fn import_download<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
        download_token: &'a str,
        destination: &'a Path,
    ) -> BrowserBrokerFuture<'a, BrowserImportedDownload> {
        let session_id = session_id.clone();
        let token = download_token.to_owned();
        let destination = destination.to_path_buf();
        let root = self
            .staging_root
            .join(session_id.as_str())
            .join("downloads");
        Box::pin(async move {
            if !self.state.lock().sessions.contains_key(&session_id) {
                return Err(BrowserHostError::SessionNotFound);
            }
            tokio::task::spawn_blocking(move || {
                import_download_file(&session_id, &root, &token, &destination)
            })
            .await
            .map_err(|error| BrowserHostError::Broker(error.to_string()))?
        })
    }

    fn set_network_policy<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
        policy: BrowserNetworkPolicy,
    ) -> BrowserBrokerFuture<'a, ()> {
        let session_id = session_id.clone();
        Box::pin(async move {
            let identity = self
                .state
                .lock()
                .sessions
                .get(&session_id)
                .map(|session| session.extension_identity.clone())
                .ok_or(BrowserHostError::SessionNotFound)?;
            self.request(
                identity,
                session_id,
                ExtensionCommandKind::SetNetworkPolicy {
                    network_policy: policy,
                },
            )
            .await?;
            Ok(())
        })
    }

    fn take_over<'a>(&'a self, session_id: &'a BrowserSessionId) -> BrowserBrokerFuture<'a, ()> {
        let session_id = session_id.clone();
        Box::pin(async move {
            let identity = self
                .state
                .lock()
                .sessions
                .get(&session_id)
                .map(|session| session.extension_identity.clone())
                .ok_or(BrowserHostError::SessionNotFound)?;
            self.request(identity, session_id.clone(), ExtensionCommandKind::TakeOver)
                .await?;
            Ok(())
        })
    }

    fn resume<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
        task_tab_group: &'a str,
        network_policy: BrowserNetworkPolicy,
    ) -> BrowserBrokerFuture<'a, ()> {
        let session_id = session_id.clone();
        let task_tab_group = task_tab_group.to_owned();
        Box::pin(async move {
            let identity = self
                .state
                .lock()
                .sessions
                .get(&session_id)
                .map(|session| session.extension_identity.clone())
                .ok_or(BrowserHostError::SessionNotFound)?;
            let result = self
                .request(
                    identity.clone(),
                    session_id.clone(),
                    ExtensionCommandKind::Resume {
                        task_tab_group,
                        network_policy,
                    },
                )
                .await?;
            let owner_tab_id = result
                .owner_tab_id
                .filter(|tab_id| *tab_id > 0)
                .ok_or(BrowserHostError::ExtensionCommandInvalid)?;
            let mut state = self.state.lock();
            let session = state
                .sessions
                .get_mut(&session_id)
                .filter(|session| session.extension_identity == identity)
                .ok_or(BrowserHostError::ExtensionCommandInvalid)?;
            session.owner_tab_id = owner_tab_id;
            Ok(())
        })
    }

    fn stop<'a>(&'a self, session_id: &'a BrowserSessionId) -> BrowserBrokerFuture<'a, ()> {
        let session_id = session_id.clone();
        Box::pin(async move {
            let identity = self
                .state
                .lock()
                .sessions
                .get(&session_id)
                .map(|session| session.extension_identity.clone())
                .ok_or(BrowserHostError::SessionNotFound)?;
            self.request(identity, session_id.clone(), ExtensionCommandKind::Stop)
                .await?;
            self.state.lock().sessions.remove(&session_id);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn success(command_id: &str) -> ExtensionCommandResult {
        ExtensionCommandResult {
            command_id: command_id.into(),
            ok: true,
            error_code: None,
            observation: None,
            action: None,
            owner_tab_id: Some(42),
        }
    }

    #[tokio::test]
    async fn claim_and_completion_are_bound_to_the_paired_identity() {
        let root = tempfile::tempdir().expect("tempdir");
        let broker = ChromeExtensionBroker::new(root.path());
        let token_a = broker.register_identity("extension-a").expect("token a");
        let token_b = broker.register_identity("extension-b").expect("token b");
        let session_id = BrowserSessionId::from("session-a");

        let start_broker = broker.clone();
        let start_session = session_id.clone();
        let start = tokio::spawn(async move {
            start_broker
                .start(
                    &start_session,
                    BrowserProfileKind::ChromeExtension,
                    Some("https://example.com"),
                    BrowserNetworkPolicy {
                        rules: Vec::new(),
                        deny_private_network_by_default: true,
                        revision: 1,
                    },
                    Some("extension-a"),
                )
                .await
        });
        tokio::task::yield_now().await;
        assert!(broker.claim(&token_b).expect("claim b").is_none());
        let command = broker
            .claim(&token_a)
            .expect("claim a")
            .expect("start command");
        assert_eq!(command.session_id, session_id);
        assert!(matches!(command.kind, ExtensionCommandKind::Start { .. }));

        assert_eq!(
            broker.complete(&token_b, success(&command.command_id)),
            Err(BrowserHostError::ExtensionAuthenticationFailed)
        );
        broker
            .complete(&token_a, success(&command.command_id))
            .expect("complete start");
        assert!(start.await.expect("join").is_ok());
        assert_eq!(
            broker.complete(&token_a, success(&command.command_id)),
            Err(BrowserHostError::ExtensionCommandInvalid)
        );

        let observe_broker = broker.clone();
        let observe_session = session_id.clone();
        let observe = tokio::spawn(async move { observe_broker.observe(&observe_session).await });
        tokio::task::yield_now().await;
        let command = broker
            .claim(&token_a)
            .expect("claim observe")
            .expect("observe command");
        broker
            .complete(
                &token_a,
                ExtensionCommandResult {
                    command_id: command.command_id,
                    ok: true,
                    error_code: None,
                    observation: Some(BrokerObservationPayload {
                        url: "https://example.com/page".into(),
                        title: "Example".into(),
                        text: "untrusted page text".into(),
                        screenshot_base64: Some(
                            BASE64_STANDARD.encode(b"\x89PNG\r\n\x1a\nobservation"),
                        ),
                        viewport_width: Some(1280),
                        viewport_height: Some(720),
                    }),
                    action: None,
                    owner_tab_id: Some(42),
                },
            )
            .expect("complete observe");
        let observation = observe.await.expect("join").expect("observation");
        assert_eq!(observation.title, "Example");
        assert_eq!(
            observation.screenshot_png.as_deref(),
            Some(&b"\x89PNG\r\n\x1a\nobservation"[..])
        );
        assert_eq!(observation.viewport_width, Some(1280));

        let rogue_download = root.path().join("rogue.txt");
        std::fs::write(&rogue_download, b"rogue tab download\n").expect("rogue download");
        let act_broker = broker.clone();
        let act_session = session_id.clone();
        let rejected = tokio::spawn(async move {
            act_broker
                .act(
                    &act_session,
                    "https://example.com",
                    &BrowserAction::Download {
                        selector: "#download".into(),
                        allow_unknown_type: false,
                    },
                )
                .await
        });
        tokio::task::yield_now().await;
        let command = broker
            .claim(&token_a)
            .expect("claim download")
            .expect("download command");
        broker
            .complete(
                &token_a,
                ExtensionCommandResult {
                    command_id: command.command_id,
                    ok: true,
                    error_code: None,
                    observation: None,
                    action: Some(BrokerActionPayload {
                        result_code: "download_quarantined".into(),
                        output: Some(serde_json::json!({
                            "hostDownloadPath": rogue_download,
                            "declaredMime": "text/plain",
                            "downloadId": 7,
                            "ownerTabId": 99
                        })),
                    }),
                    owner_tab_id: Some(42),
                },
            )
            .expect("complete rogue download");
        assert_eq!(
            rejected.await.expect("join"),
            Err(BrowserHostError::DownloadFailed)
        );
    }

    #[test]
    fn rotating_an_identity_token_revokes_the_old_bearer() {
        let root = tempfile::tempdir().expect("tempdir");
        let broker = ChromeExtensionBroker::new(root.path());
        let old = broker.register_identity("extension-a").expect("old");
        let new = broker.register_identity("extension-a").expect("new");
        assert_ne!(old, new);
        assert!(matches!(
            broker.claim(&old),
            Err(BrowserHostError::ExtensionAuthenticationFailed)
        ));
        assert!(broker.claim(&new).expect("new token").is_none());
    }

    #[tokio::test]
    async fn tab_actions_update_extension_ownership_and_reject_unexpected_owner_changes() {
        let root = tempfile::tempdir().expect("tempdir");
        let broker = ChromeExtensionBroker::new(root.path());
        let token = broker.register_identity("extension-a").expect("token");
        let session_id = BrowserSessionId::from("session-tabs");
        let start_broker = broker.clone();
        let start_session = session_id.clone();
        let start = tokio::spawn(async move {
            start_broker
                .start(
                    &start_session,
                    BrowserProfileKind::ChromeExtension,
                    None,
                    BrowserNetworkPolicy {
                        rules: Vec::new(),
                        deny_private_network_by_default: true,
                        revision: 1,
                    },
                    Some("extension-a"),
                )
                .await
        });
        tokio::task::yield_now().await;
        let start_command = broker.claim(&token).expect("claim").expect("start");
        broker
            .complete(&token, success(&start_command.command_id))
            .expect("complete start");
        start.await.expect("join").expect("started");

        let tab_broker = broker.clone();
        let tab_session = session_id.clone();
        let switch = tokio::spawn(async move {
            tab_broker
                .act(
                    &tab_session,
                    "https://example.com",
                    &BrowserAction::TabSwitch {
                        tab_id: "99".into(),
                    },
                )
                .await
        });
        tokio::task::yield_now().await;
        let switch_command = broker.claim(&token).expect("claim").expect("switch");
        broker
            .complete(
                &token,
                ExtensionCommandResult {
                    command_id: switch_command.command_id,
                    ok: true,
                    error_code: None,
                    observation: None,
                    action: Some(BrokerActionPayload {
                        result_code: "tab_switched".into(),
                        output: Some(serde_json::json!({
                            "tabId": "99",
                            "origin": "https://example.com"
                        })),
                    }),
                    owner_tab_id: Some(99),
                },
            )
            .expect("complete switch");
        switch.await.expect("join").expect("switch result");

        let click_broker = broker.clone();
        let click_session = session_id.clone();
        let click = tokio::spawn(async move {
            click_broker
                .act(
                    &click_session,
                    "https://example.com",
                    &BrowserAction::Click {
                        selector: "#continue".into(),
                    },
                )
                .await
        });
        tokio::task::yield_now().await;
        let click_command = broker.claim(&token).expect("claim").expect("click");
        broker
            .complete(
                &token,
                ExtensionCommandResult {
                    command_id: click_command.command_id,
                    ok: true,
                    error_code: None,
                    observation: None,
                    action: Some(BrokerActionPayload {
                        result_code: "clicked".into(),
                        output: None,
                    }),
                    owner_tab_id: Some(42),
                },
            )
            .expect("complete click");
        assert_eq!(
            click.await.expect("join"),
            Err(BrowserHostError::ExtensionCommandInvalid)
        );
    }

    #[tokio::test]
    async fn takeover_preserves_the_session_for_an_explicit_resume() {
        let root = tempfile::tempdir().expect("tempdir");
        let broker = ChromeExtensionBroker::new(root.path());
        let token = broker.register_identity("extension-a").expect("token");
        let session_id = BrowserSessionId::from("session-resume");
        let start_broker = broker.clone();
        let start_session = session_id.clone();
        let start = tokio::spawn(async move {
            start_broker
                .start(
                    &start_session,
                    BrowserProfileKind::ChromeExtension,
                    None,
                    BrowserNetworkPolicy {
                        rules: Vec::new(),
                        deny_private_network_by_default: true,
                        revision: 1,
                    },
                    Some("extension-a"),
                )
                .await
        });
        tokio::task::yield_now().await;
        let command = broker.claim(&token).expect("claim").expect("start");
        broker
            .complete(&token, success(&command.command_id))
            .expect("complete start");
        start.await.expect("join").expect("started");

        let takeover_broker = broker.clone();
        let takeover_session = session_id.clone();
        let takeover =
            tokio::spawn(async move { takeover_broker.take_over(&takeover_session).await });
        tokio::task::yield_now().await;
        let command = broker.claim(&token).expect("claim").expect("takeover");
        assert!(matches!(command.kind, ExtensionCommandKind::TakeOver));
        broker
            .complete(
                &token,
                ExtensionCommandResult {
                    owner_tab_id: None,
                    ..success(&command.command_id)
                },
            )
            .expect("complete takeover");
        takeover.await.expect("join").expect("taken over");

        let resume_broker = broker.clone();
        let resume_session = session_id.clone();
        let resume = tokio::spawn(async move {
            resume_broker
                .resume(
                    &resume_session,
                    "Hachimi resumed",
                    BrowserNetworkPolicy {
                        rules: Vec::new(),
                        deny_private_network_by_default: true,
                        revision: 2,
                    },
                )
                .await
        });
        tokio::task::yield_now().await;
        let command = broker.claim(&token).expect("claim").expect("resume");
        assert!(matches!(command.kind, ExtensionCommandKind::Resume { .. }));
        broker
            .complete(
                &token,
                ExtensionCommandResult {
                    owner_tab_id: Some(84),
                    ..success(&command.command_id)
                },
            )
            .expect("complete resume");
        resume.await.expect("join").expect("resumed");
        assert_eq!(
            broker
                .state
                .lock()
                .sessions
                .get(&session_id)
                .map(|session| session.owner_tab_id),
            Some(84)
        );
    }
}
