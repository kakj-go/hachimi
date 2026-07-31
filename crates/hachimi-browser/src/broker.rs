use std::{
    collections::BTreeMap,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use hachimi_protocol::{
    BrowserAction, BrowserFileToken, BrowserImportedDownload, BrowserNetworkPolicy,
    BrowserNetworkRuleKind, BrowserProfileKind, BrowserSessionId, BrowserWaitState,
};
use headless_chrome::{
    Browser, LaunchOptions, Tab,
    browser::tab::ModifierKey,
    browser::tab::RequestPausedDecision,
    protocol::cdp::{
        DOM,
        Fetch::{FailRequest, RequestPattern, RequestStage, events::RequestPausedEvent},
        Network::{ErrorReason, ResourceType},
        Page::{
            GetFrameTree, GetLayoutMetrics, GetNavigationHistory, NavigateToHistoryEntry,
            SetDownloadBehavior, SetDownloadBehaviorBehaviorOption, StopLoading,
        },
    },
};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    BrowserHostError,
    policy_proxy::{PolicyProxy, ProxyNetworkDenial},
};

pub type BrowserBrokerFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, BrowserHostError>> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq)]
pub struct BrokerObservation {
    pub url: String,
    pub title: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BrokerActionResult {
    pub result_code: String,
    pub output: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerNetworkDenial {
    pub origin: String,
    pub network_kind: BrowserNetworkRuleKind,
    pub private_network: bool,
    pub observed_at_ms: i64,
}

pub trait BrowserBroker: Send + Sync {
    fn attest_profile<'a>(
        &'a self,
        profile_kind: BrowserProfileKind,
    ) -> BrowserBrokerFuture<'a, ()> {
        let _ = profile_kind;
        Box::pin(async { Err(BrowserHostError::BrokerUnavailable) })
    }

    fn start<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
        profile_kind: BrowserProfileKind,
        initial_url: Option<&'a str>,
        initial_network_policy: BrowserNetworkPolicy,
        extension_identity: Option<&'a str>,
    ) -> BrowserBrokerFuture<'a, ()>;

    fn observe<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
    ) -> BrowserBrokerFuture<'a, BrokerObservation>;

    fn act<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
        expected_origin: &'a str,
        action: &'a BrowserAction,
    ) -> BrowserBrokerFuture<'a, BrokerActionResult>;

    fn stage_upload<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
        source: &'a Path,
    ) -> BrowserBrokerFuture<'a, BrowserFileToken> {
        let _ = (session_id, source);
        Box::pin(async { Err(BrowserHostError::BrokerUnavailable) })
    }

    fn import_download<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
        download_token: &'a str,
        destination: &'a Path,
    ) -> BrowserBrokerFuture<'a, BrowserImportedDownload> {
        let _ = (session_id, download_token, destination);
        Box::pin(async { Err(BrowserHostError::BrokerUnavailable) })
    }

    fn set_network_policy<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
        policy: BrowserNetworkPolicy,
    ) -> BrowserBrokerFuture<'a, ()> {
        let _ = (session_id, policy);
        Box::pin(async { Err(BrowserHostError::BrokerUnavailable) })
    }

    fn drain_network_denials<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
    ) -> BrowserBrokerFuture<'a, Vec<BrokerNetworkDenial>> {
        let _ = session_id;
        Box::pin(async { Ok(Vec::new()) })
    }

    fn take_over<'a>(&'a self, session_id: &'a BrowserSessionId) -> BrowserBrokerFuture<'a, ()>;

    fn stop<'a>(&'a self, session_id: &'a BrowserSessionId) -> BrowserBrokerFuture<'a, ()>;
}

#[derive(Debug, Default)]
pub struct UnavailableBrowserBroker;

impl BrowserBroker for UnavailableBrowserBroker {
    fn start<'a>(
        &'a self,
        _session_id: &'a BrowserSessionId,
        _profile_kind: BrowserProfileKind,
        _initial_url: Option<&'a str>,
        _initial_network_policy: BrowserNetworkPolicy,
        _extension_identity: Option<&'a str>,
    ) -> BrowserBrokerFuture<'a, ()> {
        Box::pin(async { Err(BrowserHostError::BrokerUnavailable) })
    }

    fn observe<'a>(
        &'a self,
        _session_id: &'a BrowserSessionId,
    ) -> BrowserBrokerFuture<'a, BrokerObservation> {
        Box::pin(async { Err(BrowserHostError::BrokerUnavailable) })
    }

    fn act<'a>(
        &'a self,
        _session_id: &'a BrowserSessionId,
        _expected_origin: &'a str,
        _action: &'a BrowserAction,
    ) -> BrowserBrokerFuture<'a, BrokerActionResult> {
        Box::pin(async { Err(BrowserHostError::BrokerUnavailable) })
    }

    fn stage_upload<'a>(
        &'a self,
        _session_id: &'a BrowserSessionId,
        _source: &'a Path,
    ) -> BrowserBrokerFuture<'a, BrowserFileToken> {
        Box::pin(async { Err(BrowserHostError::BrokerUnavailable) })
    }

    fn import_download<'a>(
        &'a self,
        _session_id: &'a BrowserSessionId,
        _download_token: &'a str,
        _destination: &'a Path,
    ) -> BrowserBrokerFuture<'a, BrowserImportedDownload> {
        Box::pin(async { Err(BrowserHostError::BrokerUnavailable) })
    }

    fn take_over<'a>(&'a self, _session_id: &'a BrowserSessionId) -> BrowserBrokerFuture<'a, ()> {
        Box::pin(async { Err(BrowserHostError::BrokerUnavailable) })
    }

    fn stop<'a>(&'a self, _session_id: &'a BrowserSessionId) -> BrowserBrokerFuture<'a, ()> {
        Box::pin(async { Err(BrowserHostError::BrokerUnavailable) })
    }
}

struct ManagedBrowserSession {
    browser: Arc<Browser>,
    tab: Arc<Tab>,
    profile_root: PathBuf,
    download_root: PathBuf,
    upload_root: PathBuf,
    policy_proxy: PolicyProxy,
    request_policy: Arc<ManagedRequestPolicy>,
    taken_over: bool,
}

#[derive(Debug)]
struct ManagedRequestPolicy {
    main_frame_id: String,
    policy: RwLock<BrowserNetworkPolicy>,
    denials: Mutex<Vec<BrokerNetworkDenial>>,
}

impl ManagedRequestPolicy {
    fn new(main_frame_id: String, policy: BrowserNetworkPolicy) -> Self {
        Self {
            main_frame_id,
            policy: RwLock::new(policy),
            denials: Mutex::new(Vec::new()),
        }
    }

    fn update(&self, policy: BrowserNetworkPolicy) {
        *self.policy.write() = policy;
    }

    fn drain_denials(&self) -> Vec<BrokerNetworkDenial> {
        std::mem::take(&mut *self.denials.lock())
    }

    fn decide(&self, intercepted: &RequestPausedEvent) -> RequestPausedDecision {
        let params = &intercepted.params;
        let kind =
            request_network_kind(&self.main_frame_id, &params.frame_id, &params.resource_Type);
        let allowed = kind.is_some_and(|kind| {
            request_matches_policy(&self.policy.read(), &params.request.url, kind, epoch_ms())
        });
        tracing::debug!(
            resource_type = ?params.resource_Type,
            main_frame = params.frame_id == self.main_frame_id,
            allowed,
            "managed Browser request policy decision"
        );
        if params.resource_Type == ResourceType::Other {
            tracing::debug!(
                main_frame = params.frame_id == self.main_frame_id,
                allowed,
                "managed Browser received an unclassified request"
            );
        }
        if allowed {
            return RequestPausedDecision::Continue(None);
        }
        if let Ok(origin) = origin_of(&params.request.url) {
            self.denials.lock().push(BrokerNetworkDenial {
                origin,
                network_kind: kind.unwrap_or(BrowserNetworkRuleKind::Resource),
                private_network: false,
                observed_at_ms: epoch_ms(),
            });
        }
        RequestPausedDecision::Fail(FailRequest {
            request_id: params.request_id.clone(),
            error_reason: ErrorReason::AccessDenied,
        })
    }
}

pub struct ManagedChromiumBroker {
    executable: PathBuf,
    profiles_root: PathBuf,
    downloads_root: PathBuf,
    sessions: Arc<Mutex<BTreeMap<BrowserSessionId, ManagedBrowserSession>>>,
}

impl std::fmt::Debug for ManagedChromiumBroker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedChromiumBroker")
            .field("executable", &self.executable)
            .field("profiles_root", &self.profiles_root)
            .field("downloads_root", &self.downloads_root)
            .finish_non_exhaustive()
    }
}

impl ManagedChromiumBroker {
    #[must_use]
    pub fn new(
        executable: impl Into<PathBuf>,
        profiles_root: impl Into<PathBuf>,
        downloads_root: impl Into<PathBuf>,
    ) -> Self {
        let profiles_root = profiles_root.into();
        let downloads_root = downloads_root.into();
        let _ = std::fs::remove_dir_all(&profiles_root);
        let _ = std::fs::remove_dir_all(&downloads_root);
        Self {
            executable: executable.into(),
            profiles_root,
            downloads_root,
            sessions: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }
}

impl BrowserBroker for ManagedChromiumBroker {
    fn attest_profile<'a>(
        &'a self,
        profile_kind: BrowserProfileKind,
    ) -> BrowserBrokerFuture<'a, ()> {
        let executable = self.executable.clone();
        let profiles_root = self.profiles_root.clone();
        let downloads_root = self.downloads_root.clone();
        Box::pin(async move {
            if profile_kind != BrowserProfileKind::Isolated {
                return Err(BrowserHostError::BrokerUnsupportedMode);
            }
            if !executable.is_file() {
                return Err(BrowserHostError::BrokerUnavailable);
            }
            std::fs::create_dir_all(&profiles_root)
                .and_then(|_| std::fs::create_dir_all(&downloads_root))
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            let proxy = PolicyProxy::start(BrowserNetworkPolicy {
                rules: Vec::new(),
                deny_private_network_by_default: true,
                revision: 1,
            })
            .await?;
            proxy.stop();
            Ok(())
        })
    }

    fn start<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
        profile_kind: BrowserProfileKind,
        initial_url: Option<&'a str>,
        initial_network_policy: BrowserNetworkPolicy,
        _extension_identity: Option<&'a str>,
    ) -> BrowserBrokerFuture<'a, ()> {
        let executable = self.executable.clone();
        let profile_root = self.profiles_root.join(session_id.as_str());
        let download_root = self.downloads_root.join(session_id.as_str());
        let initial_url = initial_url.map(str::to_owned);
        let session_id = session_id.clone();
        let sessions = Arc::clone(&self.sessions);
        Box::pin(async move {
            if profile_kind != BrowserProfileKind::Isolated {
                return Err(BrowserHostError::BrokerUnsupportedMode);
            }
            if !executable.is_file() {
                return Err(BrowserHostError::BrokerUnavailable);
            }
            let initial_policy = initial_network_policy;
            let policy_proxy = PolicyProxy::start(initial_policy.clone()).await?;
            let proxy_endpoint = policy_proxy.endpoint();
            let launched = tokio::task::spawn_blocking(move || {
                std::fs::create_dir_all(&profile_root)
                    .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
                std::fs::create_dir_all(&download_root)
                    .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
                let upload_root = profile_root.join("uploads");
                std::fs::create_dir_all(&upload_root)
                    .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
                let chromium_args = vec![
                    std::ffi::OsStr::new("--disable-quic"),
                    std::ffi::OsStr::new("--dns-over-https-mode=off"),
                    std::ffi::OsStr::new("--proxy-bypass-list=<-loopback>"),
                ];
                let options = LaunchOptions::default_builder()
                    .path(Some(executable))
                    .user_data_dir(Some(profile_root.clone()))
                    .headless(false)
                    .sandbox(true)
                    .enable_logging(false)
                    .ignore_certificate_errors(false)
                    .window_size(Some((1280, 900)))
                    .proxy_server(Some(proxy_endpoint.as_str()))
                    .args(chromium_args)
                    .build()
                    .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
                let browser = Arc::new(
                    Browser::new(options)
                        .map_err(|error| BrowserHostError::Broker(error.to_string()))?,
                );
                let tab = browser
                    .new_tab()
                    .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
                let main_frame_id = tab
                    .call_method(GetFrameTree(None))
                    .map_err(|error| BrowserHostError::Broker(error.to_string()))?
                    .frame_tree
                    .frame
                    .id;
                let request_policy =
                    Arc::new(ManagedRequestPolicy::new(main_frame_id, initial_policy));
                tab.enable_fetch(
                    Some(&[RequestPattern {
                        url_pattern: None,
                        resource_Type: None,
                        request_stage: Some(RequestStage::Request),
                    }]),
                    None,
                )
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
                let interception_policy = Arc::clone(&request_policy);
                tab.enable_request_interception(Arc::new(
                    move |_transport, _session_id, intercepted| {
                        interception_policy.decide(&intercepted)
                    },
                ))
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
                tab.call_method(SetDownloadBehavior {
                    behavior: SetDownloadBehaviorBehaviorOption::Deny,
                    download_path: None,
                })
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
                tab.register_loading_failed_handling(
                    "managed-browser-policy",
                    Box::new(|_response, failure| {
                        tracing::debug!(
                            resource_type = ?failure.Type,
                            error_text = %failure.error_text,
                            canceled = ?failure.canceled,
                            blocked_reason = ?failure.blocked_reason,
                            "managed Browser network request failed"
                        );
                    }),
                )
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
                if let Some(url) = initial_url {
                    tab.navigate_to(&url)
                        .and_then(Tab::wait_until_navigated)
                        .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
                }
                Ok::<_, BrowserHostError>(ManagedBrowserSession {
                    browser,
                    tab,
                    profile_root,
                    download_root,
                    upload_root,
                    policy_proxy,
                    request_policy,
                    taken_over: false,
                })
            })
            .await
            .map_err(|error| BrowserHostError::Broker(error.to_string()))??;
            sessions.lock().insert(session_id, launched);
            Ok(())
        })
    }

    fn observe<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
    ) -> BrowserBrokerFuture<'a, BrokerObservation> {
        let tab = {
            let sessions = self.sessions.lock();
            let session = sessions
                .get(session_id)
                .ok_or(BrowserHostError::SessionNotFound);
            match session {
                Ok(session) if !session.taken_over => Ok(Arc::clone(&session.tab)),
                Ok(_) => Err(BrowserHostError::SessionInactive),
                Err(error) => Err(error),
            }
        };
        Box::pin(async move {
            let tab = tab?;
            tokio::task::spawn_blocking(move || {
                let url = tab.get_url();
                let title = tab
                    .get_title()
                    .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
                let text = tab
                    .find_element("body")
                    .and_then(|body| body.get_inner_text())
                    .unwrap_or_default();
                Ok(BrokerObservation { url, title, text })
            })
            .await
            .map_err(|error| BrowserHostError::Broker(error.to_string()))?
        })
    }

    fn act<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
        expected_origin: &'a str,
        action: &'a BrowserAction,
    ) -> BrowserBrokerFuture<'a, BrokerActionResult> {
        let runtime = {
            let sessions = self.sessions.lock();
            sessions.get(session_id).map(|session| {
                (
                    Arc::clone(&session.browser),
                    Arc::clone(&session.tab),
                    session.download_root.clone(),
                    session.upload_root.clone(),
                    session.policy_proxy.endpoint(),
                    session.taken_over,
                )
            })
        };
        let action = action.clone();
        let browser_session_id = session_id.clone();
        let update_session_id = browser_session_id.clone();
        let expected_origin = expected_origin.to_owned();
        Box::pin(async move {
            let (browser, tab, download_root, upload_root, proxy_endpoint, taken_over) =
                runtime.ok_or(BrowserHostError::SessionNotFound)?;
            if taken_over {
                return Err(BrowserHostError::SessionInactive);
            }
            tokio::task::spawn_blocking(move || {
                let current_origin = origin_of(&tab.get_url())?;
                if current_origin != expected_origin {
                    return Err(BrowserHostError::StaleObservation);
                }
                perform_action(
                    &browser_session_id,
                    &browser,
                    &tab,
                    &download_root,
                    &upload_root,
                    &proxy_endpoint,
                    &action,
                )
            })
            .await
            .map_err(|error| BrowserHostError::Broker(error.to_string()))?
            .map(|(result, next_tab)| {
                if let Some(next_tab) = next_tab
                    && let Some(session) = self.sessions.lock().get_mut(&update_session_id)
                    && !session.taken_over
                {
                    session.tab = next_tab;
                }
                result
            })
        })
    }

    fn stage_upload<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
        source: &'a Path,
    ) -> BrowserBrokerFuture<'a, BrowserFileToken> {
        let runtime = self.sessions.lock().get(session_id).map(|session| {
            (
                session.upload_root.clone(),
                session.taken_over,
                session_id.clone(),
            )
        });
        let source = source.to_path_buf();
        Box::pin(async move {
            let (upload_root, taken_over, session_id) =
                runtime.ok_or(BrowserHostError::SessionNotFound)?;
            if taken_over {
                return Err(BrowserHostError::SessionInactive);
            }
            tokio::task::spawn_blocking(move || {
                stage_upload_file(&session_id, &upload_root, &source)
            })
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
        let runtime = self.sessions.lock().get(session_id).map(|session| {
            (
                session.download_root.clone(),
                session.taken_over,
                session_id.clone(),
            )
        });
        let token = download_token.to_owned();
        let destination = destination.to_path_buf();
        Box::pin(async move {
            let (download_root, taken_over, session_id) =
                runtime.ok_or(BrowserHostError::SessionNotFound)?;
            if taken_over {
                return Err(BrowserHostError::SessionInactive);
            }
            tokio::task::spawn_blocking(move || {
                import_download_file(&session_id, &download_root, &token, &destination)
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
        let result = self
            .sessions
            .lock()
            .get(session_id)
            .filter(|session| !session.taken_over)
            .map(|session| {
                session.policy_proxy.update(policy.clone());
                session.request_policy.update(policy);
            })
            .ok_or(BrowserHostError::SessionNotFound);
        Box::pin(async move { result })
    }

    fn drain_network_denials<'a>(
        &'a self,
        session_id: &'a BrowserSessionId,
    ) -> BrowserBrokerFuture<'a, Vec<BrokerNetworkDenial>> {
        let values = self
            .sessions
            .lock()
            .get(session_id)
            .filter(|session| !session.taken_over)
            .map(|session| {
                let mut denials = session.request_policy.drain_denials();
                denials.extend(session.policy_proxy.drain_denials().into_iter().map(
                    |value: ProxyNetworkDenial| BrokerNetworkDenial {
                        origin: value.origin,
                        network_kind: BrowserNetworkRuleKind::Resource,
                        private_network: value.private_network,
                        observed_at_ms: value.observed_at_ms,
                    },
                ));
                denials
            })
            .ok_or(BrowserHostError::SessionNotFound)
            .map(|mut values| {
                values.sort_by(|left, right| {
                    left.observed_at_ms
                        .cmp(&right.observed_at_ms)
                        .then_with(|| left.origin.cmp(&right.origin))
                        .then_with(|| {
                            network_kind_rank(left.network_kind)
                                .cmp(&network_kind_rank(right.network_kind))
                        })
                });
                values.dedup_by(|left, right| {
                    left.origin == right.origin
                        && left.network_kind == right.network_kind
                        && left.private_network == right.private_network
                });
                values
            });
        Box::pin(async move { values })
    }

    fn take_over<'a>(&'a self, session_id: &'a BrowserSessionId) -> BrowserBrokerFuture<'a, ()> {
        let result = self
            .sessions
            .lock()
            .get_mut(session_id)
            .map(|session| {
                session.policy_proxy.update(BrowserNetworkPolicy {
                    rules: Vec::new(),
                    deny_private_network_by_default: true,
                    revision: u64::MAX,
                });
                session.request_policy.update(BrowserNetworkPolicy {
                    rules: Vec::new(),
                    deny_private_network_by_default: true,
                    revision: u64::MAX,
                });
                session.taken_over = true;
                let _ = std::fs::remove_dir_all(&session.download_root);
                let _ = std::fs::remove_dir_all(&session.upload_root);
            })
            .ok_or(BrowserHostError::SessionNotFound);
        Box::pin(async move { result })
    }

    fn stop<'a>(&'a self, session_id: &'a BrowserSessionId) -> BrowserBrokerFuture<'a, ()> {
        let runtime = self.sessions.lock().remove(session_id);
        Box::pin(async move {
            let Some(runtime) = runtime else {
                return Err(BrowserHostError::SessionNotFound);
            };
            runtime.policy_proxy.stop();
            tokio::task::spawn_blocking(move || {
                let _ = runtime.tab.close(true);
                drop(runtime.browser);
                let _ = std::fs::remove_dir_all(runtime.profile_root);
                let _ = std::fs::remove_dir_all(runtime.download_root);
            })
            .await
            .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            Ok(())
        })
    }
}

fn perform_action(
    session_id: &BrowserSessionId,
    browser: &Browser,
    tab: &Tab,
    download_root: &Path,
    upload_root: &Path,
    proxy_endpoint: &str,
    action: &BrowserAction,
) -> Result<(BrokerActionResult, Option<Arc<Tab>>), BrowserHostError> {
    let mut output = None;
    let mut next_tab = None;
    let result_code = match action {
        BrowserAction::Navigate { url } => {
            tab.navigate_to(url)
                .and_then(Tab::wait_until_navigated)
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            "navigated"
        }
        BrowserAction::Back | BrowserAction::Forward => {
            let history = tab
                .call_method(GetNavigationHistory(None))
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            let current = i64::from(history.current_index);
            let target = if matches!(action, BrowserAction::Back) {
                current - 1
            } else {
                current + 1
            };
            let entry = usize::try_from(target)
                .ok()
                .and_then(|index| history.entries.get(index))
                .ok_or(BrowserHostError::InvalidInput)?;
            tab.call_method(NavigateToHistoryEntry { entry_id: entry.id })
                .and_then(|_| tab.wait_until_navigated().map(|_| ()))
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            output = Some(json!({ "origin": origin_of(&tab.get_url())? }));
            if matches!(action, BrowserAction::Back) {
                "went_back"
            } else {
                "went_forward"
            }
        }
        BrowserAction::Reload { ignore_cache } => {
            tab.reload(*ignore_cache, None)
                .and_then(Tab::wait_until_navigated)
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            "reloaded"
        }
        BrowserAction::Stop => {
            tab.call_method(StopLoading(None))
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            "loading_stopped"
        }
        BrowserAction::Click { selector } => {
            let element = tab
                .find_element(selector)
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            element
                .click()
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            "clicked"
        }
        BrowserAction::Hover { selector } => {
            tab.find_element(selector)
                .and_then(|element| element.move_mouse_over().map(|_| ()))
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            "hovered"
        }
        BrowserAction::DoubleClick { selector } => {
            tab.find_element(selector)
                .and_then(|element| {
                    element
                        .call_js_fn(
                            "function(){this.dispatchEvent(new MouseEvent('dblclick',{bubbles:true,cancelable:true,view:window,detail:2}))}",
                            Vec::new(),
                            true,
                        )
                        .map(|_| ())
                })
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            "double_clicked"
        }
        BrowserAction::Scroll {
            selector,
            delta_x,
            delta_y,
        } => {
            if let Some(selector) = selector {
                tab.find_element(selector)
                    .and_then(|element| {
                        element
                            .call_js_fn(
                                "function(dx,dy){this.scrollBy(dx,dy)}",
                                vec![json!(delta_x), json!(delta_y)],
                                true,
                            )
                            .map(|_| ())
                    })
                    .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            } else {
                tab.evaluate(&format!("window.scrollBy({delta_x},{delta_y})"), false)
                    .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            }
            "scrolled"
        }
        BrowserAction::DragDrop {
            source_selector,
            target_selector,
        } => {
            tab.find_element(source_selector)
                .and_then(|element| {
                    element
                        .call_js_fn(
                            "function(targetSelector){const target=document.querySelector(targetSelector);if(!target)throw new Error('target_not_found');const data=new DataTransfer();this.dispatchEvent(new DragEvent('dragstart',{bubbles:true,dataTransfer:data}));target.dispatchEvent(new DragEvent('dragenter',{bubbles:true,dataTransfer:data}));target.dispatchEvent(new DragEvent('dragover',{bubbles:true,cancelable:true,dataTransfer:data}));target.dispatchEvent(new DragEvent('drop',{bubbles:true,cancelable:true,dataTransfer:data}));this.dispatchEvent(new DragEvent('dragend',{bubbles:true,dataTransfer:data}))}",
                            vec![json!(target_selector)],
                            true,
                        )
                        .map(|_| ())
                })
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            "dragged"
        }
        BrowserAction::Clear { selector } => {
            set_element_value(tab, selector, "")?;
            "cleared"
        }
        BrowserAction::Fill { selector, text } => {
            set_element_value(tab, selector, text)?;
            "filled"
        }
        BrowserAction::SelectOption { selector, value } => {
            tab.find_element(selector)
                .and_then(|element| {
                    element
                        .call_js_fn(
                            "function(value){this.value=value;this.dispatchEvent(new Event('input',{bubbles:true}));this.dispatchEvent(new Event('change',{bubbles:true}))}",
                            vec![json!(value)],
                            true,
                        )
                        .map(|_| ())
                })
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            "option_selected"
        }
        BrowserAction::PressKeys { keys } => {
            press_browser_keys(tab, keys)?;
            "keys_pressed"
        }
        BrowserAction::WaitFor {
            selector,
            state,
            timeout_ms,
        } => {
            wait_for_browser_state(tab, selector.as_deref(), *state, *timeout_ms)?;
            "wait_satisfied"
        }
        BrowserAction::TabList => {
            let tabs = browser
                .get_tabs()
                .lock()
                .map_err(|_| BrowserHostError::Broker("browser tab state poisoned".into()))?
                .clone();
            output = Some(Value::Array(
                tabs.iter()
                    .map(|candidate| {
                        json!({
                            "tabId": candidate.get_target_id(),
                            "url": candidate.get_url(),
                            "title": candidate.get_title().unwrap_or_default(),
                            "active": candidate.get_target_id() == tab.get_target_id(),
                        })
                    })
                    .collect(),
            ));
            "tabs_listed"
        }
        BrowserAction::TabNew { url } => {
            let candidate = browser
                .new_tab()
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            if let Some(url) = url {
                candidate
                    .navigate_to(url)
                    .and_then(Tab::wait_until_navigated)
                    .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            }
            output = Some(json!({
                "tabId": candidate.get_target_id(),
                "origin": url.as_deref().map(origin_of).transpose()?,
            }));
            next_tab = Some(candidate);
            "tab_created"
        }
        BrowserAction::TabSwitch { tab_id } => {
            let candidate = find_tab(browser, tab_id)?;
            output = Some(json!({
                "tabId": candidate.get_target_id(),
                "origin": origin_of(&candidate.get_url())?,
            }));
            next_tab = Some(candidate);
            "tab_switched"
        }
        BrowserAction::TabClose { tab_id } => {
            let tabs = browser
                .get_tabs()
                .lock()
                .map_err(|_| BrowserHostError::Broker("browser tab state poisoned".into()))?
                .clone();
            if tabs.len() <= 1 {
                return Err(BrowserHostError::InvalidInput);
            }
            let closing = tabs
                .iter()
                .find(|candidate| candidate.get_target_id() == tab_id)
                .cloned()
                .ok_or(BrowserHostError::InvalidInput)?;
            let replacement = if closing.get_target_id() == tab.get_target_id() {
                tabs.iter()
                    .find(|candidate| candidate.get_target_id() != tab_id)
                    .cloned()
                    .ok_or(BrowserHostError::InvalidInput)?
            } else {
                tabs.iter()
                    .find(|candidate| candidate.get_target_id() == tab.get_target_id())
                    .cloned()
                    .ok_or(BrowserHostError::InvalidInput)?
            };
            closing
                .close(true)
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            output = Some(json!({
                "tabId": replacement.get_target_id(),
                "origin": origin_of(&replacement.get_url())?,
            }));
            next_tab = Some(replacement);
            "tab_closed"
        }
        BrowserAction::TypeText { selector, text } => {
            let element = tab
                .find_element(selector)
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            element
                .type_into(text)
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            "typed"
        }
        BrowserAction::Upload {
            selector,
            file_token,
        } => {
            let file = resolve_upload_token(session_id, upload_root, file_token)?;
            let file = file.to_string_lossy().into_owned();
            let element = tab
                .find_element(selector)
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            element
                .set_input_files(&[&file])
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            "uploaded"
        }
        BrowserAction::Download {
            selector,
            allow_unknown_type,
        } => {
            let (token, validated_mime) = download_link_to_quarantine(
                session_id,
                tab,
                download_root,
                proxy_endpoint,
                selector,
                *allow_unknown_type,
            )?;
            output = Some(json!({
                "downloadToken": token.token,
                "fileName": token.file_name,
                "size": token.size,
                "sha256": token.sha256,
                "mime": validated_mime,
                "expiresAtMs": token.expires_at_ms,
            }));
            "download_quarantined"
        }
        BrowserAction::ReadStorage => {
            let local = tab
                .evaluate(
                    "JSON.stringify(Object.fromEntries(Object.entries(localStorage)))",
                    false,
                )
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?
                .value
                .and_then(|value| value.as_str().map(str::to_owned))
                .and_then(|value| serde_json::from_str::<Value>(&value).ok())
                .unwrap_or_else(|| json!({}));
            let cookies = tab
                .get_cookies()
                .ok()
                .and_then(|cookies| serde_json::to_value(cookies).ok())
                .unwrap_or_else(|| json!([]));
            output = Some(json!({ "localStorage": local, "cookies": cookies }));
            "storage_read"
        }
        BrowserAction::WriteStorage { entries } => {
            let entries = entries.as_object().ok_or(BrowserHostError::InvalidInput)?;
            let encoded = serde_json::to_string(entries)
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            tab.evaluate(
                &format!(
                    "Object.entries({encoded}).forEach(([key,value]) => localStorage.setItem(key, String(value)))"
                ),
                false,
            )
            .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            "storage_written"
        }
        BrowserAction::Cdp { method, params } => {
            output = Some(perform_allowlisted_cdp(tab, method, params)?);
            "cdp_allowlisted"
        }
    };
    Ok((
        BrokerActionResult {
            result_code: result_code.into(),
            output,
        },
        next_tab,
    ))
}

fn find_tab(browser: &Browser, tab_id: &str) -> Result<Arc<Tab>, BrowserHostError> {
    browser
        .get_tabs()
        .lock()
        .map_err(|_| BrowserHostError::Broker("browser tab state poisoned".into()))?
        .iter()
        .find(|candidate| candidate.get_target_id() == tab_id)
        .cloned()
        .ok_or(BrowserHostError::InvalidInput)
}

fn set_element_value(tab: &Tab, selector: &str, text: &str) -> Result<(), BrowserHostError> {
    tab.find_element(selector)
        .and_then(|element| {
            element
                .call_js_fn(
                    "function(value){this.focus();this.value=value;this.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertText',data:value}));this.dispatchEvent(new Event('change',{bubbles:true}))}",
                    vec![json!(text)],
                    true,
                )
                .map(|_| ())
        })
        .map_err(|error| BrowserHostError::Broker(error.to_string()))
}

fn press_browser_keys(tab: &Tab, keys: &[String]) -> Result<(), BrowserHostError> {
    let modifier = |value: &str| match value.to_ascii_lowercase().as_str() {
        "alt" => Some(ModifierKey::Alt),
        "ctrl" | "control" => Some(ModifierKey::Ctrl),
        "meta" | "command" => Some(ModifierKey::Meta),
        "shift" => Some(ModifierKey::Shift),
        _ => None,
    };
    if keys.len() > 1
        && keys[..keys.len() - 1]
            .iter()
            .all(|key| modifier(key).is_some())
    {
        let modifiers = keys[..keys.len() - 1]
            .iter()
            .filter_map(|key| modifier(key))
            .collect::<Vec<_>>();
        tab.press_key_with_modifiers(&keys[keys.len() - 1], Some(&modifiers))
            .map(|_| ())
            .map_err(|error| BrowserHostError::Broker(error.to_string()))
    } else {
        for key in keys {
            tab.press_key(key)
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
        }
        Ok(())
    }
}

fn wait_for_browser_state(
    tab: &Tab,
    selector: Option<&str>,
    state: BrowserWaitState,
    timeout_ms: u64,
) -> Result<(), BrowserHostError> {
    if state == BrowserWaitState::NavigationComplete {
        return tab
            .wait_until_navigated()
            .map(|_| ())
            .map_err(|error| BrowserHostError::Broker(error.to_string()));
    }
    let selector = selector.ok_or(BrowserHostError::InvalidInput)?;
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let element = tab.find_element(selector);
        let satisfied = match state {
            BrowserWaitState::Attached => element.is_ok(),
            BrowserWaitState::Visible => {
                element.is_ok_and(|element| element.get_box_model().is_ok())
            }
            BrowserWaitState::Hidden => element.is_err(),
            BrowserWaitState::NavigationComplete => unreachable!("handled above"),
        };
        if satisfied {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(BrowserHostError::Broker("browser_wait_timeout".into()));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn perform_allowlisted_cdp(
    tab: &Tab,
    method: &str,
    params: &Value,
) -> Result<Value, BrowserHostError> {
    match method {
        "DOM.getDocument" => {
            let request: DOM::GetDocument = serde_json::from_value(params.clone())
                .map_err(|_| BrowserHostError::InvalidInput)?;
            if request.depth.is_some_and(|depth| depth > 2) || request.pierce == Some(true) {
                return Err(BrowserHostError::InvalidInput);
            }
            serialize_cdp_result(
                tab.call_method(request)
                    .map_err(|error| BrowserHostError::Broker(error.to_string()))?,
            )
        }
        "DOM.querySelector" => {
            let request: DOM::QuerySelector = serde_json::from_value(params.clone())
                .map_err(|_| BrowserHostError::InvalidInput)?;
            if request.selector.is_empty() || request.selector.chars().count() > 4_096 {
                return Err(BrowserHostError::InvalidInput);
            }
            serialize_cdp_result(
                tab.call_method(request)
                    .map_err(|error| BrowserHostError::Broker(error.to_string()))?,
            )
        }
        "DOM.getAttributes" => {
            let request: DOM::GetAttributes = serde_json::from_value(params.clone())
                .map_err(|_| BrowserHostError::InvalidInput)?;
            serialize_cdp_result(
                tab.call_method(request)
                    .map_err(|error| BrowserHostError::Broker(error.to_string()))?,
            )
        }
        "DOM.getBoxModel" => {
            let request: DOM::GetBoxModel = serde_json::from_value(params.clone())
                .map_err(|_| BrowserHostError::InvalidInput)?;
            if request.object_id.is_some() || request.backend_node_id.is_some() {
                return Err(BrowserHostError::InvalidInput);
            }
            serialize_cdp_result(
                tab.call_method(request)
                    .map_err(|error| BrowserHostError::Broker(error.to_string()))?,
            )
        }
        "Page.getLayoutMetrics" => serialize_cdp_result(
            tab.call_method(GetLayoutMetrics(None))
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?,
        ),
        "Page.reload" => {
            let ignore_cache = params
                .get("ignoreCache")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            tab.reload(ignore_cache, None)
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            Ok(json!({ "accepted": true }))
        }
        "Page.stopLoading" => {
            tab.call_method(StopLoading(None))
                .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
            Ok(json!({ "accepted": true }))
        }
        _ => Err(BrowserHostError::CdpMethodUnsupported),
    }
}

fn serialize_cdp_result<T: Serialize>(value: T) -> Result<Value, BrowserHostError> {
    serde_json::to_value(value).map_err(|error| BrowserHostError::Broker(error.to_string()))
}

fn download_link_to_quarantine(
    session_id: &BrowserSessionId,
    tab: &Tab,
    download_root: &Path,
    proxy_endpoint: &str,
    selector: &str,
    allow_unknown_type: bool,
) -> Result<(BrowserFileToken, String), BrowserHostError> {
    let element = tab
        .find_element(selector)
        .map_err(|_| BrowserHostError::DownloadFailed)?;
    let href = element
        .get_attribute_value("href")
        .map_err(|_| BrowserHostError::DownloadFailed)?
        .filter(|value| !value.trim().is_empty())
        .ok_or(BrowserHostError::DownloadFailed)?;
    let page_url = url::Url::parse(&tab.get_url()).map_err(|_| BrowserHostError::DownloadFailed)?;
    let download_url = page_url
        .join(&href)
        .map_err(|_| BrowserHostError::DownloadFailed)?;
    if !matches!(download_url.scheme(), "http" | "https") {
        return Err(BrowserHostError::DownloadFailed);
    }

    let client = reqwest::blocking::Client::builder()
        .proxy(reqwest::Proxy::all(proxy_endpoint).map_err(|_| BrowserHostError::DownloadFailed)?)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| BrowserHostError::DownloadFailed)?;
    let mut request = client.get(download_url.clone());
    let cookies = tab
        .get_cookies()
        .unwrap_or_default()
        .into_iter()
        .map(|cookie| format!("{}={}", cookie.name, cookie.value))
        .collect::<Vec<_>>()
        .join("; ");
    if !cookies.is_empty() {
        request = request.header(reqwest::header::COOKIE, cookies);
    }
    let mut response = request
        .send()
        .map_err(|_| BrowserHostError::DownloadFailed)?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > MAX_BROWSER_FILE_BYTES)
    {
        return Err(BrowserHostError::DownloadFailed);
    }
    let declared_mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let file_name = response
        .headers()
        .get(reqwest::header::CONTENT_DISPOSITION)
        .and_then(|value| value.to_str().ok())
        .and_then(content_disposition_file_name)
        .or_else(|| {
            download_url
                .path_segments()
                .and_then(Iterator::last)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
        .ok_or(BrowserHostError::DownloadFailed)?;
    safe_file_name(Path::new(&file_name)).map_err(|_| BrowserHostError::DownloadFailed)?;

    std::fs::create_dir_all(download_root)
        .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
    cleanup_expired_file_tokens(download_root);
    let token_text = opaque_file_token(&file_name);
    let staged = download_root.join(&token_text);
    let write_result = (|| {
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)
            .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
        let copied = std::io::copy(
            &mut std::io::Read::take(&mut response, MAX_BROWSER_FILE_BYTES + 1),
            &mut output,
        )
        .map_err(|_| BrowserHostError::DownloadFailed)?;
        if copied == 0 || copied > MAX_BROWSER_FILE_BYTES {
            return Err(BrowserHostError::DownloadFailed);
        }
        output
            .sync_all()
            .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
        Ok(copied)
    })();
    let size = match write_result {
        Ok(size) => size,
        Err(error) => {
            let _ = std::fs::remove_file(&staged);
            return Err(error);
        }
    };
    let mime = match validate_download_file(&staged, declared_mime.as_deref(), allow_unknown_type) {
        Ok(mime) => mime,
        Err(error) => {
            let _ = std::fs::remove_file(&staged);
            return Err(error);
        }
    };
    let bytes =
        std::fs::read(&staged).map_err(|error| BrowserHostError::Broker(error.to_string()))?;
    let sha256 = hex_digest(&Sha256::digest(bytes));
    let expires_at_ms = write_file_token_metadata(
        session_id,
        download_root,
        &token_text,
        &file_name,
        size,
        &sha256,
    )?;
    Ok((
        BrowserFileToken {
            browser_session_id: session_id.clone(),
            token: token_text,
            file_name,
            size,
            sha256,
            expires_at_ms,
        },
        mime,
    ))
}

fn content_disposition_file_name(value: &str) -> Option<String> {
    value.split(';').skip(1).find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        name.eq_ignore_ascii_case("filename")
            .then(|| value.trim().trim_matches('"').trim_matches('\'').to_owned())
    })
}

pub(crate) fn resolve_upload_token(
    session_id: &BrowserSessionId,
    root: &Path,
    token: &str,
) -> Result<PathBuf, BrowserHostError> {
    validate_staged_file(session_id, root, token)
        .map(|(path, _)| path)
        .map_err(|_| BrowserHostError::UploadTokenInvalid)
}

const MAX_BROWSER_FILE_BYTES: u64 = 100 * 1024 * 1024;
const BROWSER_FILE_TTL_MS: i64 = 60 * 60 * 1_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserFileMetadata {
    version: u32,
    session_id: String,
    token: String,
    file_name: String,
    size: u64,
    sha256: String,
    created_at_ms: i64,
    expires_at_ms: i64,
}

pub(crate) fn stage_upload_file(
    session_id: &BrowserSessionId,
    root: &Path,
    source: &Path,
) -> Result<BrowserFileToken, BrowserHostError> {
    let metadata =
        std::fs::symlink_metadata(source).map_err(|_| BrowserHostError::UploadTokenInvalid)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_BROWSER_FILE_BYTES
    {
        return Err(BrowserHostError::UploadTokenInvalid);
    }
    let file_name = safe_file_name(source)?;
    let token = opaque_file_token(&file_name);
    std::fs::create_dir_all(root).map_err(|error| BrowserHostError::Broker(error.to_string()))?;
    cleanup_expired_file_tokens(root);
    let destination = root.join(&token);
    copy_new(source, &destination)?;
    let bytes =
        std::fs::read(&destination).map_err(|error| BrowserHostError::Broker(error.to_string()))?;
    let sha256 = hex_digest(&Sha256::digest(bytes));
    let expires_at_ms = write_file_token_metadata(
        session_id,
        root,
        &token,
        &file_name,
        metadata.len(),
        &sha256,
    )?;
    Ok(BrowserFileToken {
        browser_session_id: session_id.clone(),
        token,
        file_name,
        size: metadata.len(),
        sha256,
        expires_at_ms,
    })
}

pub(crate) fn stage_download_file(
    session_id: &BrowserSessionId,
    root: &Path,
    source: &Path,
    declared_mime: Option<&str>,
    allow_unknown_type: bool,
) -> Result<(BrowserFileToken, String), BrowserHostError> {
    let mime = validate_download_file(source, declared_mime, allow_unknown_type)?;
    stage_upload_file(session_id, root, source).map(|token| (token, mime))
}

pub(crate) fn import_download_file(
    session_id: &BrowserSessionId,
    root: &Path,
    token: &str,
    destination: &Path,
) -> Result<BrowserImportedDownload, BrowserHostError> {
    let (canonical_source, token_metadata) = validate_staged_file(session_id, root, token)
        .map_err(|_| BrowserHostError::DownloadFailed)?;
    copy_new(&canonical_source, destination)?;
    let bytes =
        std::fs::read(destination).map_err(|error| BrowserHostError::Broker(error.to_string()))?;
    Ok(BrowserImportedDownload {
        browser_session_id: session_id.clone(),
        download_token: token.to_owned(),
        destination: destination.to_string_lossy().into_owned(),
        size: token_metadata.size,
        sha256: hex_digest(&Sha256::digest(bytes)),
    })
}

fn write_file_token_metadata(
    session_id: &BrowserSessionId,
    root: &Path,
    token: &str,
    file_name: &str,
    size: u64,
    sha256: &str,
) -> Result<i64, BrowserHostError> {
    let created_at_ms = epoch_ms();
    let expires_at_ms = created_at_ms.saturating_add(BROWSER_FILE_TTL_MS);
    let metadata = BrowserFileMetadata {
        version: 1,
        session_id: session_id.as_str().to_owned(),
        token: token.to_owned(),
        file_name: file_name.to_owned(),
        size,
        sha256: sha256.to_owned(),
        created_at_ms,
        expires_at_ms,
    };
    let encoded = serde_json::to_vec(&metadata)
        .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
    let path = file_metadata_path(root, token);
    let write = (|| {
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
        std::io::Write::write_all(&mut output, &encoded)
            .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
        output
            .sync_all()
            .map_err(|error| BrowserHostError::Broker(error.to_string()))
    })();
    if let Err(error) = write {
        let _ = std::fs::remove_file(root.join(token));
        return Err(error);
    }
    Ok(expires_at_ms)
}

fn validate_staged_file(
    session_id: &BrowserSessionId,
    root: &Path,
    token: &str,
) -> Result<(PathBuf, BrowserFileMetadata), BrowserHostError> {
    validate_file_token_text(token)?;
    let canonical_root = root
        .canonicalize()
        .map_err(|_| BrowserHostError::UploadTokenInvalid)?;
    let canonical_file = root
        .join(token)
        .canonicalize()
        .map_err(|_| BrowserHostError::UploadTokenInvalid)?;
    let canonical_metadata = file_metadata_path(root, token)
        .canonicalize()
        .map_err(|_| BrowserHostError::UploadTokenInvalid)?;
    if !canonical_file.starts_with(&canonical_root)
        || !canonical_metadata.starts_with(&canonical_root)
    {
        return Err(BrowserHostError::UploadTokenInvalid);
    }
    let file_metadata = std::fs::symlink_metadata(&canonical_file)
        .map_err(|_| BrowserHostError::UploadTokenInvalid)?;
    let sidecar_metadata = std::fs::symlink_metadata(&canonical_metadata)
        .map_err(|_| BrowserHostError::UploadTokenInvalid)?;
    if !file_metadata.is_file()
        || file_metadata.file_type().is_symlink()
        || file_metadata.len() == 0
        || file_metadata.len() > MAX_BROWSER_FILE_BYTES
        || !sidecar_metadata.is_file()
        || sidecar_metadata.file_type().is_symlink()
        || sidecar_metadata.len() > 16 * 1024
    {
        return Err(BrowserHostError::UploadTokenInvalid);
    }
    let encoded =
        std::fs::read(&canonical_metadata).map_err(|_| BrowserHostError::UploadTokenInvalid)?;
    let metadata: BrowserFileMetadata =
        serde_json::from_slice(&encoded).map_err(|_| BrowserHostError::UploadTokenInvalid)?;
    let now = epoch_ms();
    if metadata.version != 1
        || metadata.session_id != session_id.as_str()
        || metadata.token != token
        || metadata.size != file_metadata.len()
        || metadata.created_at_ms > now.saturating_add(5 * 60 * 1_000)
        || metadata.expires_at_ms <= now
        || metadata.expires_at_ms != metadata.created_at_ms.saturating_add(BROWSER_FILE_TTL_MS)
    {
        return Err(BrowserHostError::UploadTokenInvalid);
    }
    let bytes = std::fs::read(&canonical_file).map_err(|_| BrowserHostError::UploadTokenInvalid)?;
    if metadata.sha256 != hex_digest(&Sha256::digest(bytes)) {
        return Err(BrowserHostError::UploadTokenInvalid);
    }
    Ok((canonical_file, metadata))
}

fn validate_file_token_text(token: &str) -> Result<(), BrowserHostError> {
    (!token.is_empty()
        && token.len() <= 160
        && !token.contains(['/', '\\', ':'])
        && token != "."
        && token != "..")
        .then_some(())
        .ok_or(BrowserHostError::UploadTokenInvalid)
}

fn file_metadata_path(root: &Path, token: &str) -> PathBuf {
    root.join(format!("{token}.hachimi.json"))
}

fn cleanup_expired_file_tokens(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let now = epoch_ms();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.ends_with(".hachimi.json"))
        {
            continue;
        }
        let Ok(encoded) = std::fs::read(&path) else {
            continue;
        };
        let Ok(metadata) = serde_json::from_slice::<BrowserFileMetadata>(&encoded) else {
            continue;
        };
        if metadata.expires_at_ms <= now && validate_file_token_text(&metadata.token).is_ok() {
            let _ = std::fs::remove_file(root.join(metadata.token));
            let _ = std::fs::remove_file(path);
        }
    }
}

fn copy_new(source: &Path, destination: &Path) -> Result<(), BrowserHostError> {
    let mut input =
        std::fs::File::open(source).map_err(|error| BrowserHostError::Broker(error.to_string()))?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
    std::io::copy(&mut input, &mut output)
        .map_err(|error| BrowserHostError::Broker(error.to_string()))?;
    output
        .sync_all()
        .map_err(|error| BrowserHostError::Broker(error.to_string()))
}

fn safe_file_name(path: &Path) -> Result<String, BrowserHostError> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty() && value.chars().count() <= 240)
        .ok_or(BrowserHostError::InvalidInput)?;
    if name.contains(['/', '\\', ':']) || matches!(name, "." | "..") {
        return Err(BrowserHostError::InvalidInput);
    }
    Ok(name.to_owned())
}

fn opaque_file_token(file_name: &str) -> String {
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| {
            value.len() <= 16
                && value
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())
        })
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    format!("{}{}", uuid::Uuid::new_v4(), extension)
}

fn epoch_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|value| i64::try_from(value.as_millis()).ok())
        .unwrap_or_default()
}

fn origin_of(value: &str) -> Result<String, BrowserHostError> {
    let url = url::Url::parse(value).map_err(|_| BrowserHostError::InvalidOrigin)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(BrowserHostError::InvalidOrigin);
    }
    let host = url.host_str().ok_or(BrowserHostError::InvalidOrigin)?;
    let port = url
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    Ok(format!(
        "{}://{}{}",
        url.scheme(),
        host.to_ascii_lowercase(),
        port
    ))
}

fn request_network_kind(
    main_frame_id: &str,
    frame_id: &str,
    resource_type: &ResourceType,
) -> Option<BrowserNetworkRuleKind> {
    match resource_type {
        ResourceType::Other => None,
        ResourceType::Document if frame_id == main_frame_id => {
            Some(BrowserNetworkRuleKind::Document)
        }
        _ => Some(BrowserNetworkRuleKind::Resource),
    }
}

fn request_matches_policy(
    policy: &BrowserNetworkPolicy,
    url: &str,
    kind: BrowserNetworkRuleKind,
    now_ms: i64,
) -> bool {
    let Ok(origin) = origin_of(url) else {
        return false;
    };
    policy.rules.iter().any(|rule| {
        rule.origin == origin
            && rule
                .expires_at_ms
                .is_none_or(|expires_at| expires_at > now_ms)
            && (rule.kind == kind
                || (kind == BrowserNetworkRuleKind::Resource
                    && rule.kind == BrowserNetworkRuleKind::Document))
    })
}

const fn network_kind_rank(kind: BrowserNetworkRuleKind) -> u8 {
    match kind {
        BrowserNetworkRuleKind::Document => 0,
        BrowserNetworkRuleKind::Resource => 1,
    }
}

fn validate_download_file(
    path: &Path,
    declared_mime: Option<&str>,
    allow_unknown_type: bool,
) -> Result<String, BrowserHostError> {
    let file_name = safe_file_name(path)?;
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    if extension.as_deref().is_some_and(|value| {
        matches!(
            value,
            "exe"
                | "dll"
                | "com"
                | "scr"
                | "msi"
                | "msp"
                | "bat"
                | "cmd"
                | "ps1"
                | "vbs"
                | "vbe"
                | "js"
                | "jse"
                | "wsf"
                | "wsh"
                | "hta"
                | "lnk"
        )
    }) {
        return Err(BrowserHostError::DownloadFailed);
    }
    let bytes = std::fs::read(path).map_err(|error| BrowserHostError::Broker(error.to_string()))?;
    if bytes.is_empty()
        || bytes.starts_with(b"MZ")
        || bytes.starts_with(b"\x7fELF")
        || bytes.starts_with(&[0xfe, 0xed, 0xfa, 0xce])
        || bytes.starts_with(&[0xcf, 0xfa, 0xed, 0xfe])
        || bytes.starts_with(b"#!")
    {
        return Err(BrowserHostError::DownloadFailed);
    }
    let magic_mime = if bytes.starts_with(b"%PDF-") {
        Some("application/pdf")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if std::str::from_utf8(&bytes).is_ok() && !bytes.contains(&0) {
        let trimmed = bytes
            .iter()
            .copied()
            .skip_while(u8::is_ascii_whitespace)
            .collect::<Vec<_>>();
        if matches!(trimmed.first(), Some(b'{') | Some(b'['))
            && serde_json::from_slice::<Value>(&bytes).is_ok()
        {
            Some("application/json")
        } else {
            Some("text/plain")
        }
    } else {
        None
    };
    let extension_mime = match extension.as_deref() {
        Some("pdf") => Some("application/pdf"),
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("json") => Some("application/json"),
        Some("txt" | "md" | "csv") => Some("text/plain"),
        _ => None,
    };
    if let (Some(expected), Some(actual)) = (extension_mime, magic_mime)
        && expected != actual
    {
        return Err(BrowserHostError::DownloadFailed);
    }
    let declared_mime = declared_mime
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or(value)
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|value| !value.is_empty());
    if declared_mime.as_deref().is_some_and(|declared| {
        declared != "application/octet-stream"
            && magic_mime.is_some_and(|actual| actual != declared)
    }) {
        return Err(BrowserHostError::DownloadFailed);
    }
    let Some(mime) = magic_mime else {
        return if allow_unknown_type {
            Ok("application/octet-stream".to_owned())
        } else {
            Err(BrowserHostError::DownloadConfirmationRequired)
        };
    };
    if declared_mime.as_deref() == Some("application/octet-stream") && !allow_unknown_type {
        return Err(BrowserHostError::DownloadConfirmationRequired);
    }
    let _ = file_name;
    Ok(mime.to_owned())
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
#[path = "broker_tests.rs"]
mod tests;
