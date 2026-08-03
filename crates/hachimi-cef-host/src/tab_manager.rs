use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cef::*;
use hachimi_browser::{
    CefBounds, CefBrowserShortcut, CefHostCommand, CefHostCommandEnvelope, CefHostEvent,
    CefHostFailure, CefHostMessage, CefHostResponse, CefObservation, CefTabState,
};
use hachimi_protocol::{BrowserNavigationErrorKind, BrowserTabId};
use parking_lot::Mutex;

use crate::error_page::{error_page, navigation_error};
use crate::ipc::EventSink;

struct ManagedTab {
    browser: Option<Browser>,
    devtools_registration: Option<Registration>,
    create_request_id: Option<u64>,
    state: CefTabState,
    bounds: CefBounds,
    visible: bool,
    agent_allowed_origins: Option<BTreeSet<String>>,
}

#[derive(Default)]
struct ManagerState {
    context_ready: bool,
    shutting_down: bool,
    parent_hwnd: usize,
    tabs: BTreeMap<BrowserTabId, ManagedTab>,
    downloads: BTreeMap<(BrowserTabId, u32), DownloadItemCallback>,
    download_directory: Option<PathBuf>,
    ask_where_to_save_downloads: bool,
}

#[derive(Clone)]
pub struct TabManager {
    sink: EventSink,
    state: Arc<Mutex<ManagerState>>,
}

impl TabManager {
    pub fn new(parent_hwnd: usize, sink: EventSink) -> Self {
        Self {
            sink,
            state: Arc::new(Mutex::new(ManagerState {
                parent_hwnd,
                ..ManagerState::default()
            })),
        }
    }

    pub fn mark_context_ready(&self) {
        self.state.lock().context_ready = true;
    }

    pub fn is_empty(&self) -> bool {
        self.state.lock().tabs.is_empty()
    }

    pub fn dispatch(&self, envelope: CefHostCommandEnvelope) {
        if !self.state.lock().context_ready {
            self.sink.response(
                envelope.request_id,
                Err(CefHostFailure::new(
                    "cef_runtime_not_ready",
                    "CEF browser context is still starting",
                    true,
                )),
            );
            return;
        }
        let mut task = BrowserCommandTask::new(self.clone(), envelope.clone());
        if post_task(ThreadId::UI, Some(&mut task)) == 0 {
            self.sink.response(
                envelope.request_id,
                Err(CefHostFailure::new(
                    "cef_ui_dispatch_failed",
                    "failed to dispatch browser command to the CEF UI thread",
                    true,
                )),
            );
        }
    }

    pub fn close_all(&self) {
        let envelope = CefHostCommandEnvelope::new(u64::MAX, CefHostCommand::Shutdown);
        self.dispatch(envelope);
    }

    fn execute(&self, envelope: CefHostCommandEnvelope) {
        let request_id = envelope.request_id;
        let result = match envelope.command {
            CefHostCommand::SetParentWindow { parent_hwnd } => self.set_parent_window(parent_hwnd),
            CefHostCommand::CreateTab {
                tab_id,
                url,
                bounds,
                visible,
            } => {
                self.create_tab(request_id, tab_id, &url, bounds, visible);
                return;
            }
            CefHostCommand::CloseTab { tab_id } => self.close_tab(&tab_id),
            CefHostCommand::ActivateTab { tab_id } => self.activate_tab(&tab_id),
            CefHostCommand::SetBounds { tab_id, bounds } => self.set_bounds(&tab_id, bounds),
            CefHostCommand::SetVisible { tab_id, visible } => self.set_visible(&tab_id, visible),
            CefHostCommand::SetAgentNavigationPolicy {
                tab_id,
                allowed_origins,
            } => self.set_agent_navigation_policy(&tab_id, &allowed_origins),
            CefHostCommand::ClearAgentNavigationPolicy { tab_id } => {
                self.clear_agent_navigation_policy(&tab_id)
            }
            CefHostCommand::Focus { tab_id } => self.focus(&tab_id),
            CefHostCommand::Navigate { tab_id, url } => self.navigate(&tab_id, &url),
            CefHostCommand::Back { tab_id } => self.with_browser(&tab_id, |browser| {
                if browser.can_go_back() != 0 {
                    browser.go_back();
                }
                Ok(CefHostResponse::Acknowledged)
            }),
            CefHostCommand::Forward { tab_id } => self.with_browser(&tab_id, |browser| {
                if browser.can_go_forward() != 0 {
                    browser.go_forward();
                }
                Ok(CefHostResponse::Acknowledged)
            }),
            CefHostCommand::Reload {
                tab_id,
                ignore_cache,
            } => self.with_browser(&tab_id, |browser| {
                if ignore_cache {
                    browser.reload_ignore_cache();
                } else {
                    browser.reload();
                }
                Ok(CefHostResponse::Acknowledged)
            }),
            CefHostCommand::Stop { tab_id } => self.with_browser(&tab_id, |browser| {
                browser.stop_load();
                Ok(CefHostResponse::Acknowledged)
            }),
            CefHostCommand::ConfigureDownloads {
                directory,
                ask_where_to_save,
            } => self.configure_downloads(directory.as_deref(), ask_where_to_save),
            CefHostCommand::ClearBrowsingData { cookies, cache } => {
                self.clear_browsing_data(cookies, cache)
            }
            CefHostCommand::CancelDownload {
                tab_id,
                download_id,
            } => self.cancel_download(&tab_id, download_id),
            CefHostCommand::Observe { tab_id } => self.observe(&tab_id),
            CefHostCommand::DevTools {
                tab_id,
                method,
                params,
                full_access,
            } => return self.devtools(request_id, &tab_id, &method, params, full_access),
            CefHostCommand::Shutdown => self.shutdown(),
        };
        if request_id != u64::MAX {
            self.sink.response(request_id, result);
        }
    }

    fn create_tab(
        &self,
        request_id: u64,
        tab_id: BrowserTabId,
        url: &str,
        bounds: CefBounds,
        visible: bool,
    ) {
        let Some(bounds) = bounds.validated() else {
            self.sink.response(
                request_id,
                Err(CefHostFailure::new(
                    "cef_bounds_invalid",
                    "native surface bounds are invalid",
                    false,
                )),
            );
            return;
        };
        let url = match normalized_browser_url(url) {
            Ok(url) => url,
            Err(error) => {
                self.sink.response(request_id, Err(error));
                return;
            }
        };
        {
            let mut state = self.state.lock();
            if state.tabs.contains_key(&tab_id) {
                self.sink.response(
                    request_id,
                    Err(CefHostFailure::new(
                        "cef_tab_exists",
                        "a browser with this tab id already exists",
                        false,
                    )),
                );
                return;
            }
            state.tabs.insert(
                tab_id.clone(),
                ManagedTab {
                    browser: None,
                    devtools_registration: None,
                    create_request_id: Some(request_id),
                    state: CefTabState {
                        tab_id: tab_id.clone(),
                        url: url.clone(),
                        title: String::new(),
                        loading: true,
                        can_go_back: false,
                        can_go_forward: false,
                        navigation_error: None,
                        input_epoch: 1,
                    },
                    bounds,
                    visible,
                    agent_allowed_origins: None,
                },
            );
        }

        let rect = Rect {
            x: bounds.x,
            y: bounds.y,
            width: i32::try_from(bounds.width).unwrap_or(i32::MAX),
            height: i32::try_from(bounds.height).unwrap_or(i32::MAX),
        };
        let parent_hwnd = self.state.lock().parent_hwnd;
        let parent = sys::HWND(parent_hwnd as *mut sys::HWND__);
        let window_info = WindowInfo {
            runtime_style: RuntimeStyle::ALLOY,
            ..WindowInfo::default()
        }
        .set_as_child(parent, &rect);
        let mut client = HachimiTabClient::new(self.clone(), tab_id.clone());
        let cef_url = CefString::from(url.as_str());
        let settings = BrowserSettings::default();
        let mut request_context = request_context_get_global_context();
        let created = browser_host_create_browser(
            Some(&window_info),
            Some(&mut client),
            Some(&cef_url),
            Some(&settings),
            None,
            request_context.as_mut(),
        );
        if created == 0 {
            self.state.lock().tabs.remove(&tab_id);
            self.sink.response(
                request_id,
                Err(CefHostFailure::new(
                    "cef_tab_create_failed",
                    "CEF rejected the browser creation request",
                    true,
                )),
            );
        }
    }

    fn close_tab(&self, tab_id: &BrowserTabId) -> Result<CefHostResponse, CefHostFailure> {
        self.state
            .lock()
            .downloads
            .retain(|(owner_tab_id, _), _| owner_tab_id != tab_id);
        self.with_browser(tab_id, |browser| {
            browser
                .host()
                .ok_or_else(|| runtime_missing(tab_id))?
                .close_browser(0);
            Ok(CefHostResponse::Acknowledged)
        })
    }

    fn set_parent_window(&self, parent_hwnd: u64) -> Result<CefHostResponse, CefHostFailure> {
        let parent_hwnd = usize::try_from(parent_hwnd)
            .ok()
            .filter(|parent| valid_parent_window(*parent))
            .ok_or_else(|| {
                CefHostFailure::new(
                    "cef_parent_window_invalid",
                    "native browser parent window is invalid",
                    false,
                )
            })?;
        let windows = {
            let mut state = self.state.lock();
            if state.parent_hwnd == parent_hwnd {
                return Ok(CefHostResponse::Acknowledged);
            }
            state.parent_hwnd = parent_hwnd;
            state
                .tabs
                .values()
                .filter_map(|tab| {
                    tab.browser.as_ref().and_then(|browser| {
                        browser
                            .host()
                            .map(|host| (host.window_handle().0, tab.bounds, tab.visible))
                    })
                })
                .collect::<Vec<_>>()
        };
        for (window, bounds, visible) in windows {
            reparent_window(window, parent_hwnd);
            move_window(window, bounds);
            show_window(window, visible);
        }
        Ok(CefHostResponse::Acknowledged)
    }

    fn activate_tab(&self, tab_id: &BrowserTabId) -> Result<CefHostResponse, CefHostFailure> {
        if !self.state.lock().tabs.contains_key(tab_id) {
            return Err(tab_missing(tab_id));
        }
        let windows = self
            .state
            .lock()
            .tabs
            .iter()
            .filter_map(|(id, tab)| {
                tab.browser.as_ref().and_then(|browser| {
                    browser
                        .host()
                        .map(|host| (id == tab_id, host.window_handle().0))
                })
            })
            .collect::<Vec<_>>();
        for (active, window) in windows {
            show_window(window, active);
        }
        Ok(CefHostResponse::Acknowledged)
    }

    fn set_bounds(
        &self,
        tab_id: &BrowserTabId,
        bounds: CefBounds,
    ) -> Result<CefHostResponse, CefHostFailure> {
        let bounds = bounds.validated().ok_or_else(|| {
            CefHostFailure::new(
                "cef_bounds_invalid",
                "native surface bounds are invalid",
                false,
            )
        })?;
        let window = {
            let mut state = self.state.lock();
            let tab = state
                .tabs
                .get_mut(tab_id)
                .ok_or_else(|| tab_missing(tab_id))?;
            tab.bounds = bounds;
            tab.browser
                .as_ref()
                .and_then(Browser::host)
                .map(|host| host.window_handle().0)
        };
        if let Some(window) = window {
            move_window(window, bounds);
        }
        Ok(CefHostResponse::Acknowledged)
    }

    fn set_visible(
        &self,
        tab_id: &BrowserTabId,
        visible: bool,
    ) -> Result<CefHostResponse, CefHostFailure> {
        let window = {
            let mut state = self.state.lock();
            let tab = state
                .tabs
                .get_mut(tab_id)
                .ok_or_else(|| tab_missing(tab_id))?;
            tab.visible = visible;
            tab.browser
                .as_ref()
                .and_then(Browser::host)
                .map(|host| host.window_handle().0)
        };
        if let Some(window) = window {
            show_window(window, visible);
        }
        Ok(CefHostResponse::Acknowledged)
    }

    fn set_agent_navigation_policy(
        &self,
        tab_id: &BrowserTabId,
        allowed_origins: &[String],
    ) -> Result<CefHostResponse, CefHostFailure> {
        let origins = allowed_origins
            .iter()
            .map(|origin| {
                hachimi_browser::normalized_origin(origin).map_err(|_| {
                    CefHostFailure::new(
                        "cef_agent_origin_invalid",
                        "Agent navigation policy contains an invalid origin",
                        false,
                    )
                })
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        let mut state = self.state.lock();
        let tab = state
            .tabs
            .get_mut(tab_id)
            .ok_or_else(|| tab_missing(tab_id))?;
        tab.agent_allowed_origins = Some(origins);
        Ok(CefHostResponse::Acknowledged)
    }

    fn clear_agent_navigation_policy(
        &self,
        tab_id: &BrowserTabId,
    ) -> Result<CefHostResponse, CefHostFailure> {
        let mut state = self.state.lock();
        let tab = state
            .tabs
            .get_mut(tab_id)
            .ok_or_else(|| tab_missing(tab_id))?;
        tab.agent_allowed_origins = None;
        Ok(CefHostResponse::Acknowledged)
    }

    fn agent_navigation_allowed(&self, tab_id: &BrowserTabId, target_url: &str) -> bool {
        let policy = self
            .state
            .lock()
            .tabs
            .get(tab_id)
            .and_then(|tab| tab.agent_allowed_origins.as_ref())
            .cloned();
        navigation_origin_allowed(policy.as_ref(), target_url)
    }

    fn focus(&self, tab_id: &BrowserTabId) -> Result<CefHostResponse, CefHostFailure> {
        self.with_browser(tab_id, |browser| {
            browser
                .host()
                .ok_or_else(|| runtime_missing(tab_id))?
                .set_focus(1);
            Ok(CefHostResponse::Acknowledged)
        })
    }

    fn navigate(
        &self,
        tab_id: &BrowserTabId,
        url: &str,
    ) -> Result<CefHostResponse, CefHostFailure> {
        let url = normalized_browser_url(url)?;
        self.with_browser(tab_id, |browser| {
            browser
                .main_frame()
                .ok_or_else(|| runtime_missing(tab_id))?
                .load_url(Some(&CefString::from(url.as_str())));
            Ok(CefHostResponse::Acknowledged)
        })
    }

    fn observe(&self, tab_id: &BrowserTabId) -> Result<CefHostResponse, CefHostFailure> {
        let state = self.tab_state(tab_id).ok_or_else(|| tab_missing(tab_id))?;
        Ok(CefHostResponse::Observation {
            observation: CefObservation {
                state,
                text: String::new(),
                accessibility_tree: serde_json::Value::Null,
                screenshot_base64: None,
                screenshot_mime_type: None,
                viewport_width: None,
                viewport_height: None,
            },
        })
    }

    fn cancel_download(
        &self,
        tab_id: &BrowserTabId,
        download_id: u32,
    ) -> Result<CefHostResponse, CefHostFailure> {
        let callback = self
            .state
            .lock()
            .downloads
            .get(&(tab_id.clone(), download_id))
            .cloned()
            .ok_or_else(|| {
                CefHostFailure::new(
                    "cef_download_not_active",
                    "the requested download is no longer active",
                    false,
                )
            })?;
        callback.cancel();
        Ok(CefHostResponse::Acknowledged)
    }

    fn configure_downloads(
        &self,
        directory: Option<&str>,
        ask_where_to_save: bool,
    ) -> Result<CefHostResponse, CefHostFailure> {
        let directory = directory
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from);
        if directory
            .as_ref()
            .is_some_and(|path| !path.is_absolute() || !path.is_dir())
        {
            return Err(CefHostFailure::new(
                "cef_download_directory_invalid",
                "the configured download directory is not an existing absolute directory",
                false,
            ));
        }
        let mut state = self.state.lock();
        state.download_directory = directory;
        state.ask_where_to_save_downloads = ask_where_to_save;
        Ok(CefHostResponse::Acknowledged)
    }

    fn clear_browsing_data(
        &self,
        cookies: bool,
        cache: bool,
    ) -> Result<CefHostResponse, CefHostFailure> {
        let context = request_context_get_global_context().ok_or_else(|| {
            CefHostFailure::new(
                "cef_request_context_missing",
                "the embedded browser request context is unavailable",
                true,
            )
        })?;
        if cookies {
            let manager = context.cookie_manager(None).ok_or_else(|| {
                CefHostFailure::new(
                    "cef_cookie_manager_missing",
                    "the embedded browser cookie manager is unavailable",
                    true,
                )
            })?;
            if manager.delete_cookies(None, None, None) == 0 {
                return Err(CefHostFailure::new(
                    "cef_cookie_clear_failed",
                    "CEF rejected the cookie deletion request",
                    true,
                ));
            }
        }
        if cache {
            context.clear_http_cache(None);
        }
        Ok(CefHostResponse::Acknowledged)
    }

    fn download_preferences(&self) -> (Option<PathBuf>, bool) {
        let state = self.state.lock();
        (
            state.download_directory.clone(),
            state.ask_where_to_save_downloads,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn download_updated(
        &self,
        tab_id: &BrowserTabId,
        download_id: u32,
        url: String,
        suggested_name: String,
        destination: Option<String>,
        received_bytes: u64,
        total_bytes: Option<u64>,
        complete: bool,
        cancelled: bool,
        interrupted: bool,
        callback: Option<DownloadItemCallback>,
    ) {
        let key = (tab_id.clone(), download_id);
        if complete || cancelled || interrupted {
            self.state.lock().downloads.remove(&key);
        } else if let Some(callback) = callback {
            self.state.lock().downloads.insert(key, callback);
        }
        self.sink.send(&CefHostMessage::Event {
            event: CefHostEvent::DownloadUpdated {
                tab_id: tab_id.clone(),
                download_id,
                url,
                suggested_name,
                destination,
                received_bytes,
                total_bytes,
                complete,
                cancelled,
                interrupted,
            },
        });
    }

    fn devtools(
        &self,
        request_id: u64,
        tab_id: &BrowserTabId,
        method: &str,
        params: serde_json::Value,
        full_access: bool,
    ) {
        if !allowed_devtools_method(method, full_access) {
            self.sink.response(
                request_id,
                Err(CefHostFailure::new(
                    "cef_devtools_method_denied",
                    format!("DevTools method is outside the embedded-browser allowlist: {method}"),
                    false,
                )),
            );
            return;
        }
        let browser = self
            .state
            .lock()
            .tabs
            .get(tab_id)
            .and_then(|tab| tab.browser.clone());
        let Some(browser) = browser else {
            self.sink.response(request_id, Err(runtime_missing(tab_id)));
            return;
        };
        let Some(host) = browser.host() else {
            self.sink.response(request_id, Err(runtime_missing(tab_id)));
            return;
        };
        let Ok(message_id) = i32::try_from(request_id) else {
            self.sink.response(
                request_id,
                Err(CefHostFailure::new(
                    "cef_devtools_request_overflow",
                    "DevTools request id exceeded the CEF message range",
                    true,
                )),
            );
            return;
        };
        let message = serde_json::json!({
            "id": message_id,
            "method": method,
            "params": params,
        });
        let Ok(encoded) = serde_json::to_vec(&message) else {
            self.sink.response(
                request_id,
                Err(CefHostFailure::new(
                    "cef_devtools_encode_failed",
                    "DevTools parameters could not be encoded",
                    false,
                )),
            );
            return;
        };
        if host.send_dev_tools_message(Some(&encoded)) == 0 {
            self.sink.response(
                request_id,
                Err(CefHostFailure::new(
                    "cef_devtools_dispatch_failed",
                    "CEF rejected the DevTools command",
                    true,
                )),
            );
        }
    }

    fn shutdown(&self) -> Result<CefHostResponse, CefHostFailure> {
        let browsers = {
            let mut state = self.state.lock();
            state.shutting_down = true;
            state
                .tabs
                .values()
                .filter_map(|tab| tab.browser.clone())
                .collect::<Vec<_>>()
        };
        for browser in browsers {
            if let Some(host) = browser.host() {
                host.close_browser(1);
            }
        }
        Ok(CefHostResponse::Acknowledged)
    }

    fn with_browser(
        &self,
        tab_id: &BrowserTabId,
        action: impl FnOnce(&Browser) -> Result<CefHostResponse, CefHostFailure>,
    ) -> Result<CefHostResponse, CefHostFailure> {
        let browser = self
            .state
            .lock()
            .tabs
            .get(tab_id)
            .ok_or_else(|| tab_missing(tab_id))?
            .browser
            .clone()
            .ok_or_else(|| runtime_missing(tab_id))?;
        action(&browser)
    }

    fn tab_state(&self, tab_id: &BrowserTabId) -> Option<CefTabState> {
        self.state
            .lock()
            .tabs
            .get(tab_id)
            .map(|tab| tab.state.clone())
    }

    fn browser_created(&self, tab_id: &BrowserTabId, browser: Browser) {
        let (bounds, visible, create_request_id, state) = {
            let mut manager = self.state.lock();
            let Some(tab) = manager.tabs.get_mut(tab_id) else {
                return;
            };
            tab.browser = Some(browser.clone());
            tab.state.loading = browser.is_loading() != 0;
            tab.state.can_go_back = browser.can_go_back() != 0;
            tab.state.can_go_forward = browser.can_go_forward() != 0;
            (
                tab.bounds,
                tab.visible,
                tab.create_request_id.take(),
                tab.state.clone(),
            )
        };
        if let Some(host) = browser.host() {
            let mut observer = HachimiDevToolsObserver::new(self.sink.clone());
            let registration = host.add_dev_tools_message_observer(Some(&mut observer));
            if let Some(tab) = self.state.lock().tabs.get_mut(tab_id) {
                tab.devtools_registration = registration;
            }
            let window = host.window_handle().0;
            move_window(window, bounds);
            show_window(window, visible);
        }
        if let Some(request_id) = create_request_id {
            self.sink.response(
                request_id,
                Ok(CefHostResponse::TabCreated {
                    state: state.clone(),
                }),
            );
        }
        self.emit_state(state);
    }

    fn browser_closed(&self, tab_id: &BrowserTabId) {
        self.state.lock().tabs.remove(tab_id);
    }

    fn finalize_browser_close(&self, tab_id: &BrowserTabId) {
        let mut task = FinalizeBrowserCloseTask::new(self.clone(), tab_id.clone());
        if post_task(ThreadId::UI, Some(&mut task)) == 0 {
            self.browser_closed(tab_id);
        }
    }

    fn address_changed(&self, tab_id: &BrowserTabId, url: String) {
        let state = self.update_state(tab_id, |state| {
            if url.starts_with("data:text/html") && state.navigation_error.is_some() {
                return;
            }
            state.url = url;
            state.navigation_error = None;
        });
        if let Some(state) = state {
            self.emit_state(state);
        }
    }

    fn title_changed(&self, tab_id: &BrowserTabId, title: String) {
        if let Some(state) = self.update_state(tab_id, |state| state.title = title) {
            self.emit_state(state);
        }
    }

    fn loading_changed(
        &self,
        tab_id: &BrowserTabId,
        loading: bool,
        can_go_back: bool,
        can_go_forward: bool,
    ) {
        if let Some(state) = self.update_state(tab_id, |state| {
            state.loading = loading;
            state.can_go_back = can_go_back;
            state.can_go_forward = can_go_forward;
        }) {
            self.emit_state(state);
        }
    }

    fn load_failed(
        &self,
        tab_id: &BrowserTabId,
        error_code: Errorcode,
        description: String,
        failed_url: String,
        frame: &Frame,
    ) {
        if error_code == Errorcode::ABORTED {
            return;
        }
        if self.state.lock().tabs.get(tab_id).is_some_and(|tab| {
            tab.state.navigation_error.as_ref().is_some_and(|error| {
                error.kind == BrowserNavigationErrorKind::Tls && error.failed_url == failed_url
            })
        }) {
            return;
        }
        let error = navigation_error(error_code, description, failed_url);
        let state = self.update_state(tab_id, |state| {
            state.loading = false;
            state.url.clone_from(&error.failed_url);
            state.navigation_error = Some(error.clone());
        });
        if let Some(state) = state {
            self.emit_state(state);
        }
        frame.load_url(Some(&CefString::from(error_page(&error).as_str())));
    }

    fn certificate_failed(
        &self,
        tab_id: &BrowserTabId,
        error_code: Errorcode,
        failed_url: String,
        browser: Option<&mut Browser>,
    ) {
        let error = navigation_error(error_code, format!("{error_code:?}"), failed_url);
        let state = self.update_state(tab_id, |state| {
            state.loading = false;
            state.url.clone_from(&error.failed_url);
            state.navigation_error = Some(error.clone());
        });
        if let Some(state) = state {
            self.emit_state(state);
        }
        if let Some(frame) = browser.and_then(|browser| browser.main_frame()) {
            frame.load_url(Some(&CefString::from(error_page(&error).as_str())));
        }
    }

    fn popup_requested(&self, tab_id: &BrowserTabId, target_url: String) {
        self.sink.send(&CefHostMessage::Event {
            event: CefHostEvent::PopupRequested {
                opener_tab_id: tab_id.clone(),
                target_url,
            },
        });
    }

    fn agent_navigation_blocked(&self, tab_id: &BrowserTabId, target_url: String) {
        self.sink.send(&CefHostMessage::Event {
            event: CefHostEvent::AgentNavigationBlocked {
                tab_id: tab_id.clone(),
                target_url,
            },
        });
    }

    fn user_input(&self, tab_id: &BrowserTabId) {
        if let Some(tab) = self.state.lock().tabs.get_mut(tab_id) {
            tab.agent_allowed_origins = None;
        }
        let input_epoch = self
            .update_state(tab_id, |state| {
                state.input_epoch = state.input_epoch.saturating_add(1);
            })
            .map(|state| state.input_epoch);
        if let Some(input_epoch) = input_epoch {
            self.sink.send(&CefHostMessage::Event {
                event: CefHostEvent::UserInput {
                    tab_id: tab_id.clone(),
                    input_epoch,
                },
            });
        }
    }

    fn shortcut_requested(&self, tab_id: &BrowserTabId, shortcut: CefBrowserShortcut) {
        self.sink.send(&CefHostMessage::Event {
            event: CefHostEvent::ShortcutRequested {
                tab_id: tab_id.clone(),
                shortcut,
            },
        });
    }

    fn render_process_terminated(&self, tab_id: &BrowserTabId, status: String) {
        let state = self.update_state(tab_id, |state| {
            state.loading = false;
            state.navigation_error = Some(hachimi_protocol::BrowserNavigationError {
                kind: BrowserNavigationErrorKind::Crashed,
                code: 0,
                description: status.clone(),
                failed_url: state.url.clone(),
            });
        });
        if let Some(state) = state {
            self.emit_state(state);
        }
        self.sink.send(&CefHostMessage::Event {
            event: CefHostEvent::RenderProcessTerminated {
                tab_id: tab_id.clone(),
                status,
            },
        });
    }

    fn update_state(
        &self,
        tab_id: &BrowserTabId,
        update: impl FnOnce(&mut CefTabState),
    ) -> Option<CefTabState> {
        let mut manager = self.state.lock();
        let tab = manager.tabs.get_mut(tab_id)?;
        update(&mut tab.state);
        Some(tab.state.clone())
    }

    fn emit_state(&self, state: CefTabState) {
        self.sink.send(&CefHostMessage::Event {
            event: CefHostEvent::TabStateChanged { state },
        });
    }
}

fn allowed_devtools_method(method: &str, full_access: bool) -> bool {
    let standard = matches!(
        method,
        "Accessibility.getFullAXTree"
            | "DOM.getDocument"
            | "DOM.getBoxModel"
            | "DOM.querySelector"
            | "Input.dispatchKeyEvent"
            | "Input.dispatchMouseEvent"
            | "Input.insertText"
            | "Page.captureScreenshot"
            | "Page.getLayoutMetrics"
            | "Page.navigate"
            | "Runtime.callFunctionOn"
            | "Runtime.evaluate"
    );
    standard || (full_access && valid_full_devtools_method(method))
}

fn valid_full_devtools_method(method: &str) -> bool {
    let syntactically_valid = !method.is_empty()
        && method.len() <= 128
        && method.is_ascii()
        && method.split_once('.').is_some_and(|(domain, command)| {
            !domain.is_empty()
                && !command.is_empty()
                && domain.chars().all(|value| value.is_ascii_alphanumeric())
                && command
                    .chars()
                    .all(|value| value.is_ascii_alphanumeric() || value == '_')
        });
    syntactically_valid
        && !method.starts_with("Browser.")
        && !method.starts_with("SystemInfo.")
        && !method.starts_with("Target.")
        && !matches!(
            method,
            "Page.setDownloadBehavior"
                | "Security.setIgnoreCertificateErrors"
                | "Security.handleCertificateError"
        )
}

fn navigation_origin_allowed(policy: Option<&BTreeSet<String>>, target_url: &str) -> bool {
    let Some(policy) = policy else {
        return true;
    };
    hachimi_browser::normalized_origin(target_url)
        .ok()
        .is_none_or(|origin| policy.contains(&origin))
}

wrap_dev_tools_message_observer! {
    struct HachimiDevToolsObserver {
        sink: EventSink,
    }

    impl DevToolsMessageObserver {
        fn on_dev_tools_method_result(
            &self,
            _browser: Option<&mut Browser>,
            message_id: i32,
            success: i32,
            result: Option<&[u8]>,
        ) {
            if message_id <= 0 {
                return;
            }
            let request_id = message_id as u64;
            let value = result
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(bytes).ok())
                .unwrap_or(serde_json::Value::Null);
            if success != 0 {
                self.sink.response(request_id, Ok(CefHostResponse::DevTools { result: value }));
            } else {
                self.sink.response(
                    request_id,
                    Err(CefHostFailure::new(
                        "cef_devtools_method_failed",
                        value.to_string(),
                        false,
                    )),
                );
            }
        }
    }
}

wrap_task! {
    struct BrowserCommandTask {
        manager: TabManager,
        envelope: CefHostCommandEnvelope,
    }

    impl Task {
        fn execute(&self) {
            self.manager.execute(self.envelope.clone());
        }
    }
}

wrap_task! {
    struct FinalizeBrowserCloseTask {
        manager: TabManager,
        tab_id: BrowserTabId,
    }

    impl Task {
        fn execute(&self) {
            self.manager.browser_closed(&self.tab_id);
        }
    }
}

wrap_client! {
    struct HachimiTabClient {
        manager: TabManager,
        tab_id: BrowserTabId,
    }

    impl Client {
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(HachimiDisplayHandler::new(self.manager.clone(), self.tab_id.clone()))
        }

        fn download_handler(&self) -> Option<DownloadHandler> {
            Some(HachimiDownloadHandler::new(
                self.manager.clone(),
                self.tab_id.clone(),
            ))
        }

        fn keyboard_handler(&self) -> Option<KeyboardHandler> {
            Some(HachimiKeyboardHandler::new(self.manager.clone(), self.tab_id.clone()))
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(HachimiLifeSpanHandler::new(self.manager.clone(), self.tab_id.clone()))
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(HachimiLoadHandler::new(self.manager.clone(), self.tab_id.clone()))
        }

        fn request_handler(&self) -> Option<RequestHandler> {
            Some(HachimiRequestHandler::new(self.manager.clone(), self.tab_id.clone()))
        }
    }
}

wrap_download_handler! {
    struct HachimiDownloadHandler {
        manager: TabManager,
        tab_id: BrowserTabId,
    }

    impl DownloadHandler {
        fn can_download(
            &self,
            _browser: Option<&mut Browser>,
            _url: Option<&CefString>,
            _request_method: Option<&CefString>,
        ) -> i32 {
            1
        }

        fn on_before_download(
            &self,
            _browser: Option<&mut Browser>,
            _download_item: Option<&mut DownloadItem>,
            suggested_name: Option<&CefString>,
            callback: Option<&mut BeforeDownloadCallback>,
        ) -> i32 {
            if let Some(callback) = callback {
                let (directory, ask_where_to_save) = self.manager.download_preferences();
                let suggested_name = suggested_name
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "download".into());
                let file_name = Path::new(&suggested_name)
                    .file_name()
                    .and_then(|value| value.to_str())
                    .filter(|value| !value.is_empty())
                    .unwrap_or("download");
                let destination = directory.map(|value| value.join(file_name));
                let destination = destination
                    .as_ref()
                    .map(|value| CefString::from(value.to_string_lossy().as_ref()));
                callback.cont(destination.as_ref(), i32::from(ask_where_to_save));
                1
            } else {
                0
            }
        }

        fn on_download_updated(
            &self,
            _browser: Option<&mut Browser>,
            download_item: Option<&mut DownloadItem>,
            callback: Option<&mut DownloadItemCallback>,
        ) {
            let Some(item) = download_item else {
                return;
            };
            let original_url = item.original_url();
            let suggested_name = item.suggested_file_name();
            let full_path = item.full_path();
            let destination = CefString::from(&full_path).to_string();
            self.manager.download_updated(
                &self.tab_id,
                item.id(),
                CefString::from(&original_url).to_string(),
                CefString::from(&suggested_name).to_string(),
                (!destination.is_empty()).then_some(destination),
                u64::try_from(item.received_bytes()).unwrap_or_default(),
                u64::try_from(item.total_bytes()).ok(),
                item.is_complete() != 0,
                item.is_canceled() != 0,
                item.is_interrupted() != 0,
                callback.map(|callback| callback.clone()),
            );
        }
    }
}

wrap_display_handler! {
    struct HachimiDisplayHandler {
        manager: TabManager,
        tab_id: BrowserTabId,
    }

    impl DisplayHandler {
        fn on_address_change(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            url: Option<&CefString>,
        ) {
            if frame.is_some_and(|frame| frame.is_main() != 0) {
                self.manager.address_changed(
                    &self.tab_id,
                    url.map(CefString::to_string).unwrap_or_default(),
                );
            }
        }

        fn on_title_change(&self, _browser: Option<&mut Browser>, title: Option<&CefString>) {
            self.manager.title_changed(
                &self.tab_id,
                title.map(CefString::to_string).unwrap_or_default(),
            );
        }
    }
}

wrap_keyboard_handler! {
    struct HachimiKeyboardHandler {
        manager: TabManager,
        tab_id: BrowserTabId,
    }

    impl KeyboardHandler {
        fn on_pre_key_event(
            &self,
            _browser: Option<&mut Browser>,
            event: Option<&KeyEvent>,
            _os_event: Option<&mut sys::MSG>,
            _is_keyboard_shortcut: Option<&mut i32>,
        ) -> i32 {
            self.manager.user_input(&self.tab_id);
            let Some(shortcut) = event.and_then(browser_shortcut) else {
                return 0;
            };
            self.manager.shortcut_requested(&self.tab_id, shortcut);
            1
        }
    }
}

fn browser_shortcut(event: &KeyEvent) -> Option<CefBrowserShortcut> {
    shortcut_for_key(
        event.type_ == KeyEventType::RAWKEYDOWN,
        event.modifiers,
        event.windows_key_code,
    )
}

fn shortcut_for_key(
    raw_key_down: bool,
    modifiers: u32,
    windows_key_code: i32,
) -> Option<CefBrowserShortcut> {
    const CONTROL_DOWN: u32 = 1 << 2;
    const ALT_DOWN: u32 = 1 << 3;
    if !raw_key_down {
        return None;
    }
    let control = modifiers & CONTROL_DOWN != 0;
    let alt = modifiers & ALT_DOWN != 0;
    match (control, alt, windows_key_code) {
        (true, false, 0x4c) => Some(CefBrowserShortcut::FocusAddress),
        (true, false, 0x54) => Some(CefBrowserShortcut::NewTab),
        (true, false, 0x57) => Some(CefBrowserShortcut::CloseTab),
        (true, false, 0x52) => Some(CefBrowserShortcut::Reload),
        (false, true, 0x25) => Some(CefBrowserShortcut::Back),
        (false, true, 0x27) => Some(CefBrowserShortcut::Forward),
        _ => None,
    }
}

#[cfg(test)]
mod shortcut_tests {
    use super::*;

    #[test]
    fn only_fixed_browser_shortcuts_are_forwarded() {
        assert_eq!(
            shortcut_for_key(true, 1 << 2, 0x4c),
            Some(CefBrowserShortcut::FocusAddress)
        );
        assert_eq!(
            shortcut_for_key(true, 1 << 3, 0x25),
            Some(CefBrowserShortcut::Back)
        );
        assert_eq!(shortcut_for_key(true, 0, 0x4c), None);
        assert_eq!(shortcut_for_key(false, 1 << 2, 0x4c), None);
    }

    #[test]
    fn agent_navigation_policy_blocks_cross_origin_redirects_only_while_active() {
        let origins = BTreeSet::from(["https://example.com".to_owned()]);
        assert!(navigation_origin_allowed(
            Some(&origins),
            "https://example.com/next"
        ));
        assert!(!navigation_origin_allowed(
            Some(&origins),
            "https://accounts.example.net/login"
        ));
        assert!(navigation_origin_allowed(
            None,
            "https://accounts.example.net/login"
        ));
    }
}

wrap_life_span_handler! {
    struct HachimiLifeSpanHandler {
        manager: TabManager,
        tab_id: BrowserTabId,
    }

    impl LifeSpanHandler {
        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: i32,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            _target_disposition: WindowOpenDisposition,
            _user_gesture: i32,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut i32>,
        ) -> i32 {
            self.manager.popup_requested(
                &self.tab_id,
                target_url.map(CefString::to_string).unwrap_or_default(),
            );
            1
        }

        fn on_after_created(&self, browser: Option<&mut Browser>) {
            if let Some(browser) = browser.cloned() {
                self.manager.browser_created(&self.tab_id, browser);
            }
        }

        fn on_before_close(&self, _browser: Option<&mut Browser>) {
            self.manager.finalize_browser_close(&self.tab_id);
        }
    }
}

wrap_load_handler! {
    struct HachimiLoadHandler {
        manager: TabManager,
        tab_id: BrowserTabId,
    }

    impl LoadHandler {
        fn on_loading_state_change(
            &self,
            _browser: Option<&mut Browser>,
            loading: i32,
            can_go_back: i32,
            can_go_forward: i32,
        ) {
            self.manager.loading_changed(
                &self.tab_id,
                loading != 0,
                can_go_back != 0,
                can_go_forward != 0,
            );
        }

        fn on_load_error(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            error_code: Errorcode,
            error_text: Option<&CefString>,
            failed_url: Option<&CefString>,
        ) {
            let Some(frame) = frame.filter(|frame| frame.is_main() != 0) else {
                return;
            };
            self.manager.load_failed(
                &self.tab_id,
                error_code,
                error_text.map(CefString::to_string).unwrap_or_default(),
                failed_url.map(CefString::to_string).unwrap_or_default(),
                frame,
            );
        }
    }
}

wrap_request_handler! {
    struct HachimiRequestHandler {
        manager: TabManager,
        tab_id: BrowserTabId,
    }

    impl RequestHandler {
        fn on_before_browse(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            user_gesture: i32,
            _is_redirect: i32,
        ) -> i32 {
            if user_gesture != 0 {
                self.manager.user_input(&self.tab_id);
                return 0;
            }
            if frame.is_some_and(|frame| frame.is_main() != 0)
                && let Some(request) = request
            {
                let target_url = CefString::from(&request.url()).to_string();
                if !self
                    .manager
                    .agent_navigation_allowed(&self.tab_id, &target_url)
                {
                    self.manager
                        .agent_navigation_blocked(&self.tab_id, target_url);
                    return 1;
                }
            }
            0
        }

        fn on_certificate_error(
            &self,
            browser: Option<&mut Browser>,
            cert_error: Errorcode,
            request_url: Option<&CefString>,
            _ssl_info: Option<&mut Sslinfo>,
            callback: Option<&mut Callback>,
        ) -> i32 {
            self.manager.certificate_failed(
                &self.tab_id,
                cert_error,
                request_url.map(CefString::to_string).unwrap_or_default(),
                browser,
            );
            if let Some(callback) = callback {
                callback.cancel();
                1
            } else {
                0
            }
        }

        fn on_render_process_terminated(
            &self,
            _browser: Option<&mut Browser>,
            status: TerminationStatus,
            error_code: i32,
            error_string: Option<&CefString>,
        ) {
            let details = error_string.map(CefString::to_string).unwrap_or_default();
            self.manager.render_process_terminated(
                &self.tab_id,
                format!("{status:?} ({error_code}): {details}"),
            );
        }
    }
}

fn normalized_browser_url(value: &str) -> Result<String, CefHostFailure> {
    if value.eq_ignore_ascii_case("about:blank") {
        return Ok("about:blank".into());
    }
    hachimi_browser::normalized_url(value).map_err(|_| {
        CefHostFailure::new(
            "cef_url_invalid",
            "only normalized HTTP(S) URLs without credentials are allowed",
            false,
        )
    })
}

#[cfg(target_os = "windows")]
fn valid_parent_window(parent: usize) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::IsWindow;
    parent != 0 && unsafe { IsWindow(parent as *mut std::ffi::c_void) != 0 }
}

#[cfg(not(target_os = "windows"))]
fn valid_parent_window(parent: usize) -> bool {
    parent != 0
}

#[cfg(target_os = "windows")]
fn reparent_window(window: *mut sys::HWND__, parent: usize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::SetParent;
    unsafe {
        SetParent(window.cast(), parent as *mut std::ffi::c_void);
    }
}

#[cfg(not(target_os = "windows"))]
fn reparent_window(_window: *mut sys::HWND__, _parent: usize) {}

fn tab_missing(tab_id: &BrowserTabId) -> CefHostFailure {
    CefHostFailure::new(
        "cef_tab_not_found",
        format!("browser tab {tab_id} does not exist"),
        false,
    )
}

fn runtime_missing(tab_id: &BrowserTabId) -> CefHostFailure {
    CefHostFailure::new(
        "cef_tab_not_loaded",
        format!("browser tab {tab_id} has not finished creating its native runtime"),
        true,
    )
}

#[cfg(target_os = "windows")]
fn move_window(window: *mut sys::HWND__, bounds: CefBounds) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SWP_NOACTIVATE, SWP_NOOWNERZORDER, SetWindowPos,
    };
    unsafe {
        // The CEF child and WebView2 controller are siblings under the Tauri
        // window. Keep CEF above WebView2 so the native page is visible inside
        // the reported surface bounds without making it a topmost window.
        SetWindowPos(
            window.cast(),
            std::ptr::null_mut(),
            bounds.x,
            bounds.y,
            i32::try_from(bounds.width).unwrap_or(i32::MAX),
            i32::try_from(bounds.height).unwrap_or(i32::MAX),
            SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
    }
}

#[cfg(not(target_os = "windows"))]
fn move_window(_window: sys::cef_window_handle_t, _bounds: CefBounds) {}

#[cfg(target_os = "windows")]
fn show_window(window: *mut sys::HWND__, visible: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWNA, ShowWindow};
    unsafe {
        ShowWindow(window.cast(), if visible { SW_SHOWNA } else { SW_HIDE });
    }
}

#[cfg(not(target_os = "windows"))]
fn show_window(_window: sys::cef_window_handle_t, _visible: bool) {}

#[cfg(test)]
mod tests {
    use super::{allowed_devtools_method, valid_full_devtools_method};

    #[test]
    fn standard_devtools_methods_stay_minimal() {
        assert!(allowed_devtools_method("Runtime.evaluate", false));
        assert!(allowed_devtools_method("Page.captureScreenshot", false));
        assert!(!allowed_devtools_method("Network.enable", false));
    }

    #[test]
    fn full_devtools_still_denies_escape_and_security_methods() {
        assert!(allowed_devtools_method("Network.enable", true));
        assert!(valid_full_devtools_method("Performance.getMetrics"));
        assert!(!valid_full_devtools_method("Browser.close"));
        assert!(!valid_full_devtools_method("Target.getTargets"));
        assert!(!valid_full_devtools_method(
            "Security.setIgnoreCertificateErrors"
        ));
        assert!(!valid_full_devtools_method("Page.setDownloadBehavior"));
    }
}
