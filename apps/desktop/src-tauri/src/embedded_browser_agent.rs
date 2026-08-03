use std::{collections::BTreeMap, sync::Arc};

use hachimi_browser::{CefHostCommand, CefHostResponse, normalized_browser_input};
use hachimi_protocol::{
    BrowserAction, BrowserAutomationLease, BrowserAutomationLeaseId, BrowserAutomationLeaseStatus,
    BrowserAutomationSurfaceKind, BrowserCapability, BrowserObservationId, BrowserTabId,
    EmbeddedBrowserPermissionRequiredEvent, RunId, SessionId,
};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, Wry};
use thiserror::Error;

use crate::embedded_browser::{EmbeddedBrowserError, EmbeddedBrowserService};

const LEASE_LIFETIME_MS: i64 = 30 * 60 * 1_000;

#[derive(Debug, Error)]
pub enum EmbeddedAgentBrowserError {
    #[error("browser Run no longer exists")]
    RunMissing,
    #[error("browser Run ownership or generation changed")]
    StaleRun,
    #[error("browser automation lease is inactive or expired")]
    LeaseInactive,
    #[error("browser automation lease ownership changed")]
    LeaseOwnership,
    #[error("browser observation is stale; observe the tab again")]
    StaleObservation,
    #[error("browser site permission is missing for the current tab origin")]
    SitePermissionMissing,
    #[error("browser site permission requires user confirmation: {0}")]
    SitePermissionRequired(hachimi_protocol::ItemId),
    #[error("embedded browser capability is not supported: {0}")]
    CapabilityNotSupported(&'static str),
    #[error("embedded browser action is invalid: {0}")]
    InvalidAction(String),
    #[error("embedded browser window is unavailable")]
    WindowMissing,
    #[error(transparent)]
    Runtime(#[from] EmbeddedBrowserError),
    #[error("browser storage failed: {0}")]
    Storage(String),
}

#[derive(Debug, Clone)]
struct ObservationFence {
    observation_id: BrowserObservationId,
    tab_revision: u64,
    input_epoch: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedBrowserStarted {
    pub lease: BrowserAutomationLease,
    pub workspace_id: hachimi_protocol::BrowserWorkspaceId,
    pub tab_id: BrowserTabId,
    pub url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedBrowserObservation {
    pub lease_id: BrowserAutomationLeaseId,
    pub workspace_id: hachimi_protocol::BrowserWorkspaceId,
    pub tab_id: BrowserTabId,
    pub observation_id: BrowserObservationId,
    pub run_generation: u64,
    pub tab_revision: u64,
    pub input_epoch: u64,
    pub url: String,
    pub title: String,
    pub text: String,
    pub accessibility_tree: Value,
    pub screenshot_base64: Option<String>,
    pub screenshot_mime_type: Option<String>,
    pub viewport_width: Option<u32>,
    pub viewport_height: Option<u32>,
    pub external_content: bool,
}

pub struct EmbeddedBrowserActionRequest<'a> {
    pub lease_id: &'a BrowserAutomationLeaseId,
    pub run_id: &'a RunId,
    pub run_generation: u64,
    pub observation_id: &'a BrowserObservationId,
    pub expected_tab_revision: u64,
    pub expected_input_epoch: u64,
    pub action: &'a BrowserAction,
    pub origin_policy: EmbeddedBrowserOriginPolicy<'a>,
}

#[derive(Clone, Copy)]
pub struct EmbeddedBrowserOriginPolicy<'a> {
    pub allowed_origins: &'a [String],
    pub allow_unlisted_origin: bool,
    pub allow_private_network: bool,
    pub require_site_permission: bool,
}

#[derive(Clone)]
pub struct EmbeddedAgentBrowser {
    app: AppHandle,
    store: hachimi_storage::AgentStore,
    runtime: Arc<EmbeddedBrowserService<Wry>>,
    observations: Arc<Mutex<BTreeMap<BrowserAutomationLeaseId, ObservationFence>>>,
    full_cdp_access_allowed: bool,
}

impl EmbeddedAgentBrowser {
    pub fn new(
        app: AppHandle,
        store: hachimi_storage::AgentStore,
        runtime: Arc<EmbeddedBrowserService<Wry>>,
        full_cdp_access_allowed: bool,
    ) -> Self {
        Self {
            app,
            store,
            runtime,
            observations: Arc::new(Mutex::new(BTreeMap::new())),
            full_cdp_access_allowed,
        }
    }

    pub async fn start(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        run_generation: u64,
        initial_url: &str,
    ) -> Result<EmbeddedBrowserStarted, EmbeddedAgentBrowserError> {
        self.validate_run(session_id, run_id, run_generation)
            .await?;
        let url = normalized_browser_input(initial_url)
            .map_err(|error| EmbeddedAgentBrowserError::InvalidAction(error.to_string()))?;
        let window = self
            .app
            .get_webview_window("workbench")
            .or_else(|| self.app.get_webview_window("pet"))
            .ok_or(EmbeddedAgentBrowserError::WindowMissing)?;
        let persisted = self
            .store
            .get_or_create_browser_workspace(session_id, Some(&url))
            .await
            .map_err(storage)?;
        let mut workspace = self.runtime.open_workspace(&window, &persisted).await?;
        let active = workspace
            .tabs
            .iter()
            .find(|tab| tab.id == workspace.active_tab_id)
            .cloned()
            .ok_or_else(|| {
                EmbeddedAgentBrowserError::Storage("active browser tab is missing".into())
            })?;
        let tab_id = if active.url == "about:blank" {
            self.runtime
                .command(
                    &window,
                    CefHostCommand::Navigate {
                        tab_id: active.id.clone(),
                        url: url.clone(),
                    },
                )
                .await?;
            active.id
        } else {
            workspace = self
                .store
                .create_browser_tab(&workspace.id, workspace.revision, Some(&url))
                .await
                .map_err(storage)?;
            let tab = workspace
                .tabs
                .iter()
                .find(|tab| tab.id == workspace.active_tab_id)
                .ok_or_else(|| {
                    EmbeddedAgentBrowserError::Storage("task browser tab is missing".into())
                })?;
            self.runtime
                .create_tab_runtime(&window, &workspace.id, &tab.id, &tab.url)
                .await?;
            self.runtime
                .command(
                    &window,
                    CefHostCommand::ActivateTab {
                        tab_id: tab.id.clone(),
                    },
                )
                .await?;
            tab.id.clone()
        };
        let lease = self
            .store
            .create_browser_automation_lease(
                BrowserAutomationSurfaceKind::Embedded,
                Some(&workspace.id),
                Some(&tab_id),
                session_id,
                run_id,
                run_generation,
                &[BrowserCapability::Observe, BrowserCapability::Act],
                now_ms().saturating_add(LEASE_LIFETIME_MS),
            )
            .await
            .map_err(storage)?;
        self.sync_agent_navigation_policy(&lease, &tab_id).await?;
        Ok(EmbeddedBrowserStarted {
            lease,
            workspace_id: workspace.id,
            tab_id,
            url,
        })
    }

    pub async fn require_site_permission(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        run_generation: u64,
        url: &str,
        allow_private_network: bool,
        lease_id: Option<&BrowserAutomationLeaseId>,
    ) -> Result<(), EmbeddedAgentBrowserError> {
        let origin = hachimi_browser::normalized_origin(url)
            .map_err(|error| EmbeddedAgentBrowserError::InvalidAction(error.to_string()))?;
        let private_network = matches!(
            hachimi_browser::validate_agent_browser_target(url, false).await,
            Err(hachimi_browser::BrowserHostError::PrivateNetworkDenied)
        );
        if self
            .store
            .embedded_browser_site_permission(&origin, session_id, run_id, private_network)
            .await
            .map_err(storage)?
            .is_some()
        {
            return Ok(());
        }
        if private_network && !allow_private_network {
            return Err(EmbeddedAgentBrowserError::InvalidAction(
                "private_network_denied".into(),
            ));
        }
        let (workspace, tab, lease_id) = if let Some(lease_id) = lease_id {
            let lease = self
                .store
                .browser_automation_lease(lease_id)
                .await
                .map_err(storage)?;
            let workspace_id = lease
                .workspace_id
                .ok_or(EmbeddedAgentBrowserError::LeaseInactive)?;
            let tab_id = lease
                .tab_id
                .ok_or(EmbeddedAgentBrowserError::LeaseInactive)?;
            let workspace = self
                .store
                .browser_workspace(&workspace_id)
                .await
                .map_err(storage)?;
            let tab = workspace
                .tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .cloned()
                .ok_or(EmbeddedAgentBrowserError::LeaseInactive)?;
            (workspace, tab, Some(lease_id))
        } else {
            let workspace = self
                .store
                .get_or_create_browser_workspace(session_id, None)
                .await
                .map_err(storage)?;
            let tab = workspace
                .tabs
                .iter()
                .find(|tab| tab.id == workspace.active_tab_id)
                .cloned()
                .ok_or_else(|| {
                    EmbeddedAgentBrowserError::Storage("active browser tab is missing".into())
                })?;
            (workspace, tab, None)
        };
        let request = self
            .store
            .create_embedded_browser_permission_request(
                &workspace.id,
                &tab.id,
                lease_id,
                session_id,
                run_id,
                run_generation,
                &origin,
                private_network,
                tab.revision,
            )
            .await
            .map_err(storage)?;
        let event = EmbeddedBrowserPermissionRequiredEvent {
            request: request.clone(),
            reason_code: if private_network {
                "agent_private_site_permission_required".into()
            } else {
                "agent_site_permission_required".into()
            },
        };
        let _ = self.app.emit("browser:permission-required", &event);
        let _ = self
            .store
            .append_event(
                session_id,
                Some(run_id),
                "browser.permission_required",
                serde_json::to_value(&event)
                    .map_err(|error| EmbeddedAgentBrowserError::Storage(error.to_string()))?,
            )
            .await;
        Err(EmbeddedAgentBrowserError::SitePermissionRequired(
            request.id,
        ))
    }

    pub async fn observe(
        &self,
        lease_id: &BrowserAutomationLeaseId,
        run_id: &RunId,
        run_generation: u64,
        origin_policy: EmbeddedBrowserOriginPolicy<'_>,
    ) -> Result<EmbeddedBrowserObservation, EmbeddedAgentBrowserError> {
        let lease = self
            .validate_lease(lease_id, run_id, run_generation)
            .await?;
        let workspace_id = lease
            .workspace_id
            .clone()
            .ok_or(EmbeddedAgentBrowserError::LeaseInactive)?;
        let tab_id = lease
            .tab_id
            .clone()
            .ok_or(EmbeddedAgentBrowserError::LeaseInactive)?;
        let workspace = self
            .store
            .browser_workspace(&workspace_id)
            .await
            .map_err(storage)?;
        let tab = workspace
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .ok_or(EmbeddedAgentBrowserError::LeaseInactive)?;
        validate_tab_origin(&tab.url, origin_policy).await?;
        if origin_policy.require_site_permission {
            self.require_site_permission(
                &lease.owner_session_id,
                run_id,
                run_generation,
                &tab.url,
                origin_policy.allow_private_network,
                Some(&lease.id),
            )
            .await?;
        }
        self.sync_agent_navigation_policy(&lease, &tab_id).await?;

        let page = self.devtools(&tab_id, "Runtime.evaluate", json!({
            "expression": "(() => ({url: location.href, title: document.title, text: document.body?.innerText ?? '', width: innerWidth, height: innerHeight}))()",
            "returnByValue": true,
            "awaitPromise": true,
        })).await?;
        let value = page
            .pointer("/result/value")
            .cloned()
            .unwrap_or(Value::Null);
        let accessibility_tree = self
            .devtools(&tab_id, "Accessibility.getFullAXTree", json!({}))
            .await
            .unwrap_or(Value::Null);
        let screenshot = self
            .devtools(
                &tab_id,
                "Page.captureScreenshot",
                json!({ "format": "png", "fromSurface": true }),
            )
            .await
            .ok();
        let observation_id = BrowserObservationId::random();
        self.observations.lock().insert(
            lease.id.clone(),
            ObservationFence {
                observation_id: observation_id.clone(),
                tab_revision: tab.revision,
                input_epoch: tab.input_epoch,
            },
        );
        Ok(EmbeddedBrowserObservation {
            lease_id: lease.id,
            workspace_id,
            tab_id,
            observation_id,
            run_generation,
            tab_revision: tab.revision,
            input_epoch: tab.input_epoch,
            url: value
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or(&tab.url)
                .to_owned(),
            title: value
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or(&tab.title)
                .to_owned(),
            text: value
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            accessibility_tree,
            screenshot_base64: screenshot
                .as_ref()
                .and_then(|value| value.get("data"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            screenshot_mime_type: screenshot
                .as_ref()
                .and_then(|value| value.get("data"))
                .map(|_| "image/png".to_owned()),
            viewport_width: value
                .get("width")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok()),
            viewport_height: value
                .get("height")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok()),
            external_content: true,
        })
    }

    pub async fn act(
        &self,
        request: EmbeddedBrowserActionRequest<'_>,
    ) -> Result<Value, EmbeddedAgentBrowserError> {
        let EmbeddedBrowserActionRequest {
            lease_id,
            run_id,
            run_generation,
            observation_id,
            expected_tab_revision,
            expected_input_epoch,
            action,
            origin_policy,
        } = request;
        let lease = self
            .validate_lease(lease_id, run_id, run_generation)
            .await?;
        let workspace_id = lease
            .workspace_id
            .clone()
            .ok_or(EmbeddedAgentBrowserError::LeaseInactive)?;
        let tab_id = lease
            .tab_id
            .clone()
            .ok_or(EmbeddedAgentBrowserError::LeaseInactive)?;
        let fence = self
            .observations
            .lock()
            .get(lease_id)
            .cloned()
            .ok_or(EmbeddedAgentBrowserError::StaleObservation)?;
        if fence.observation_id != *observation_id
            || fence.tab_revision != expected_tab_revision
            || fence.input_epoch != expected_input_epoch
        {
            return Err(EmbeddedAgentBrowserError::StaleObservation);
        }
        let current = self
            .store
            .browser_workspace(&workspace_id)
            .await
            .map_err(storage)?;
        let current_tab = current
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .ok_or(EmbeddedAgentBrowserError::StaleObservation)?;
        if current_tab.revision != fence.tab_revision
            || current_tab.input_epoch != fence.input_epoch
        {
            return Err(EmbeddedAgentBrowserError::StaleObservation);
        }
        validate_tab_origin(&current_tab.url, origin_policy).await?;
        if origin_policy.require_site_permission {
            self.require_site_permission(
                &lease.owner_session_id,
                run_id,
                run_generation,
                &current_tab.url,
                origin_policy.allow_private_network,
                Some(&lease.id),
            )
            .await?;
        }
        self.sync_agent_navigation_policy(&lease, &tab_id).await?;
        self.observations.lock().remove(lease_id);
        match action {
            BrowserAction::Navigate { url } => {
                let url = normalized_browser_input(url)
                    .map_err(|error| EmbeddedAgentBrowserError::InvalidAction(error.to_string()))?;
                self.runtime
                    .command(
                        &self.window()?,
                        CefHostCommand::Navigate {
                            tab_id,
                            url: url.clone(),
                        },
                    )
                    .await?;
                Ok(json!({ "url": url }))
            }
            BrowserAction::Back => self.runtime_action(CefHostCommand::Back { tab_id }).await,
            BrowserAction::Forward => {
                self.runtime_action(CefHostCommand::Forward { tab_id })
                    .await
            }
            BrowserAction::Reload { ignore_cache } => {
                self.runtime_action(CefHostCommand::Reload {
                    tab_id,
                    ignore_cache: *ignore_cache,
                })
                .await
            }
            BrowserAction::Stop => self.runtime_action(CefHostCommand::Stop { tab_id }).await,
            BrowserAction::Click { selector } | BrowserAction::DoubleClick { selector } => {
                let clicks = if matches!(action, BrowserAction::DoubleClick { .. }) {
                    2
                } else {
                    1
                };
                let point = self.selector_point(&tab_id, selector).await?;
                for kind in ["mousePressed", "mouseReleased"] {
                    self.devtools(&tab_id, "Input.dispatchMouseEvent", json!({
                        "type": kind, "x": point.0, "y": point.1, "button": "left", "clickCount": clicks,
                    })).await?;
                }
                Ok(json!({ "clicked": selector }))
            }
            BrowserAction::Hover { selector } => {
                let point = self.selector_point(&tab_id, selector).await?;
                self.devtools(
                    &tab_id,
                    "Input.dispatchMouseEvent",
                    json!({ "type": "mouseMoved", "x": point.0, "y": point.1 }),
                )
                .await?;
                Ok(json!({ "hovered": selector }))
            }
            BrowserAction::DragDrop {
                source_selector,
                target_selector,
            } => {
                let source = self.selector_point(&tab_id, source_selector).await?;
                let target = self.selector_point(&tab_id, target_selector).await?;
                self.devtools(
                    &tab_id,
                    "Input.dispatchMouseEvent",
                    json!({ "type": "mousePressed", "x": source.0, "y": source.1, "button": "left", "clickCount": 1 }),
                )
                .await?;
                self.devtools(
                    &tab_id,
                    "Input.dispatchMouseEvent",
                    json!({ "type": "mouseMoved", "x": target.0, "y": target.1, "button": "left" }),
                )
                .await?;
                self.devtools(
                    &tab_id,
                    "Input.dispatchMouseEvent",
                    json!({ "type": "mouseReleased", "x": target.0, "y": target.1, "button": "left", "clickCount": 1 }),
                )
                .await?;
                Ok(json!({ "dragged": source_selector, "dropped": target_selector }))
            }
            BrowserAction::Fill { selector, text } | BrowserAction::TypeText { selector, text } => {
                self.focus_selector(
                    &tab_id,
                    selector,
                    matches!(action, BrowserAction::Fill { .. }),
                )
                .await?;
                self.devtools(&tab_id, "Input.insertText", json!({ "text": text }))
                    .await?;
                Ok(json!({ "filled": selector }))
            }
            BrowserAction::Clear { selector } => {
                self.focus_selector(&tab_id, selector, true).await?;
                self.devtools(
                    &tab_id,
                    "Input.dispatchKeyEvent",
                    json!({ "type": "keyDown", "key": "Backspace", "code": "Backspace" }),
                )
                .await?;
                self.devtools(
                    &tab_id,
                    "Input.dispatchKeyEvent",
                    json!({ "type": "keyUp", "key": "Backspace", "code": "Backspace" }),
                )
                .await?;
                Ok(json!({ "cleared": selector }))
            }
            BrowserAction::SelectOption { selector, value } => {
                let selector_literal = serde_json::to_string(selector)
                    .map_err(|error| EmbeddedAgentBrowserError::InvalidAction(error.to_string()))?;
                let value_literal = serde_json::to_string(value)
                    .map_err(|error| EmbeddedAgentBrowserError::InvalidAction(error.to_string()))?;
                let result = self
                    .devtools(
                        &tab_id,
                        "Runtime.evaluate",
                        json!({
                            "expression": format!("(() => {{ const e=document.querySelector({selector_literal}); if(!(e instanceof HTMLSelectElement)) return false; const option=[...e.options].find(o=>o.value==={value_literal}); if(!option) return false; e.focus(); option.selected=true; e.dispatchEvent(new Event('input',{{bubbles:true}})); e.dispatchEvent(new Event('change',{{bubbles:true}})); return true; }})()"),
                            "returnByValue": true,
                        }),
                    )
                    .await?;
                if result.pointer("/result/value").and_then(Value::as_bool) != Some(true) {
                    return Err(EmbeddedAgentBrowserError::InvalidAction(
                        "select_option_not_found".into(),
                    ));
                }
                Ok(json!({ "selected": value }))
            }
            BrowserAction::PressKeys { keys } => {
                for key in keys {
                    self.devtools(
                        &tab_id,
                        "Input.dispatchKeyEvent",
                        json!({ "type": "keyDown", "key": key }),
                    )
                    .await?;
                }
                for key in keys.iter().rev() {
                    self.devtools(
                        &tab_id,
                        "Input.dispatchKeyEvent",
                        json!({ "type": "keyUp", "key": key }),
                    )
                    .await?;
                }
                Ok(json!({ "pressed": keys }))
            }
            BrowserAction::Scroll {
                selector: _,
                delta_x,
                delta_y,
            } => {
                self.devtools(&tab_id, "Input.dispatchMouseEvent", json!({ "type": "mouseWheel", "x": 1, "y": 1, "deltaX": delta_x, "deltaY": delta_y })).await?;
                Ok(json!({ "scrolled": true }))
            }
            BrowserAction::WaitFor {
                selector,
                state,
                timeout_ms,
            } => {
                self.wait_for(&tab_id, selector.as_deref(), *state, *timeout_ms)
                    .await
            }
            BrowserAction::TabList => {
                let workspace = self
                    .store
                    .browser_workspace(&workspace_id)
                    .await
                    .map_err(storage)?;
                Ok(serde_json::to_value(workspace.tabs)
                    .map_err(|error| EmbeddedAgentBrowserError::Storage(error.to_string()))?)
            }
            BrowserAction::TabNew { url } => {
                let url = url
                    .as_deref()
                    .map(normalized_browser_input)
                    .transpose()
                    .map_err(|error| EmbeddedAgentBrowserError::InvalidAction(error.to_string()))?;
                let workspace = self
                    .store
                    .browser_workspace(&workspace_id)
                    .await
                    .map_err(storage)?;
                let updated = self
                    .store
                    .create_browser_tab(&workspace_id, workspace.revision, url.as_deref())
                    .await
                    .map_err(storage)?;
                let created = updated
                    .tabs
                    .iter()
                    .find(|tab| tab.id == updated.active_tab_id)
                    .ok_or_else(|| {
                        EmbeddedAgentBrowserError::Storage("created browser tab is missing".into())
                    })?;
                if let Err(error) = self
                    .runtime
                    .create_tab_runtime(&self.window()?, &workspace_id, &created.id, &created.url)
                    .await
                {
                    let _ = self
                        .store
                        .close_browser_tab(&workspace_id, &created.id, updated.revision)
                        .await;
                    return Err(error.into());
                }
                self.runtime
                    .command(
                        &self.window()?,
                        CefHostCommand::ActivateTab {
                            tab_id: created.id.clone(),
                        },
                    )
                    .await?;
                let rebound = self
                    .store
                    .update_browser_automation_lease_target(
                        lease_id,
                        lease.revision,
                        &workspace_id,
                        &created.id,
                    )
                    .await
                    .map_err(storage)?;
                self.sync_agent_navigation_policy(&rebound, &created.id)
                    .await?;
                Ok(
                    json!({ "tabId": created.id, "url": created.url, "leaseRevision": rebound.revision }),
                )
            }
            BrowserAction::TabSwitch { tab_id } => {
                let workspace = self
                    .store
                    .browser_workspace(&workspace_id)
                    .await
                    .map_err(storage)?;
                let target = workspace
                    .tabs
                    .iter()
                    .find(|tab| tab.id.as_str() == tab_id)
                    .cloned()
                    .ok_or_else(|| {
                        EmbeddedAgentBrowserError::InvalidAction("browser_tab_not_found".into())
                    })?;
                let updated = self
                    .store
                    .activate_browser_tab(&workspace_id, &target.id, workspace.revision)
                    .await
                    .map_err(storage)?;
                if !self.runtime.is_tab_loaded(&target.id) {
                    self.runtime
                        .create_tab_runtime(&self.window()?, &workspace_id, &target.id, &target.url)
                        .await?;
                }
                self.runtime
                    .command(
                        &self.window()?,
                        CefHostCommand::ActivateTab {
                            tab_id: target.id.clone(),
                        },
                    )
                    .await?;
                let rebound = self
                    .store
                    .update_browser_automation_lease_target(
                        lease_id,
                        lease.revision,
                        &workspace_id,
                        &target.id,
                    )
                    .await
                    .map_err(storage)?;
                self.sync_agent_navigation_policy(&rebound, &target.id)
                    .await?;
                Ok(
                    json!({ "tabId": target.id, "workspaceRevision": updated.revision, "leaseRevision": rebound.revision }),
                )
            }
            BrowserAction::TabClose { tab_id } => {
                let workspace = self
                    .store
                    .browser_workspace(&workspace_id)
                    .await
                    .map_err(storage)?;
                let closing = workspace
                    .tabs
                    .iter()
                    .find(|tab| tab.id.as_str() == tab_id)
                    .ok_or_else(|| {
                        EmbeddedAgentBrowserError::InvalidAction("browser_tab_not_found".into())
                    })?;
                self.runtime
                    .close_tab_runtime(&self.window()?, &closing.id)
                    .await?;
                let updated = self
                    .store
                    .close_browser_tab(&workspace_id, &closing.id, workspace.revision)
                    .await
                    .map_err(storage)?;
                let active = updated
                    .tabs
                    .iter()
                    .find(|tab| tab.id == updated.active_tab_id)
                    .ok_or_else(|| {
                        EmbeddedAgentBrowserError::Storage("active browser tab is missing".into())
                    })?;
                if !self.runtime.is_tab_loaded(&active.id) {
                    self.runtime
                        .create_tab_runtime(&self.window()?, &workspace_id, &active.id, &active.url)
                        .await?;
                }
                self.runtime
                    .command(
                        &self.window()?,
                        CefHostCommand::ActivateTab {
                            tab_id: active.id.clone(),
                        },
                    )
                    .await?;
                let current_lease = self
                    .store
                    .browser_automation_lease(lease_id)
                    .await
                    .map_err(storage)?;
                let rebound = self
                    .store
                    .update_browser_automation_lease_target(
                        lease_id,
                        current_lease.revision,
                        &workspace_id,
                        &active.id,
                    )
                    .await
                    .map_err(storage)?;
                self.sync_agent_navigation_policy(&rebound, &active.id)
                    .await?;
                Ok(
                    json!({ "closedTabId": tab_id, "tabId": active.id, "leaseRevision": rebound.revision }),
                )
            }
            BrowserAction::Upload { .. } => Err(EmbeddedAgentBrowserError::CapabilityNotSupported(
                "automatic_upload",
            )),
            BrowserAction::Cdp { method, params } => {
                let settings = self
                    .store
                    .embedded_browser_settings(self.full_cdp_access_allowed)
                    .await
                    .map_err(storage)?;
                if !settings.full_cdp_access {
                    return Err(EmbeddedAgentBrowserError::CapabilityNotSupported(
                        "full_cdp_access_disabled",
                    ));
                }
                Ok(self
                    .devtools_with_access(&tab_id, method, params.clone(), true)
                    .await?)
            }
            _ => Err(EmbeddedAgentBrowserError::CapabilityNotSupported("action")),
        }
    }

    pub async fn stop(
        &self,
        lease_id: &BrowserAutomationLeaseId,
        run_id: &RunId,
        run_generation: u64,
    ) -> Result<BrowserAutomationLease, EmbeddedAgentBrowserError> {
        let lease = self
            .validate_lease(lease_id, run_id, run_generation)
            .await?;
        self.observations.lock().remove(lease_id);
        if let Some(tab_id) = lease.tab_id.as_ref() {
            let _ = self
                .runtime
                .command(
                    &self.window()?,
                    CefHostCommand::ClearAgentNavigationPolicy {
                        tab_id: tab_id.clone(),
                    },
                )
                .await;
        }
        self.store
            .set_browser_automation_lease_status(
                lease_id,
                lease.revision,
                BrowserAutomationLeaseStatus::Expired,
            )
            .await
            .map_err(storage)
    }

    async fn validate_run(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        generation: u64,
    ) -> Result<(), EmbeddedAgentBrowserError> {
        let run = self
            .store
            .get_run(run_id)
            .await
            .map_err(storage)?
            .ok_or(EmbeddedAgentBrowserError::RunMissing)?;
        if run.session_id != *session_id || run.generation != generation || run.status.is_terminal()
        {
            return Err(EmbeddedAgentBrowserError::StaleRun);
        }
        Ok(())
    }

    async fn validate_lease(
        &self,
        lease_id: &BrowserAutomationLeaseId,
        run_id: &RunId,
        generation: u64,
    ) -> Result<BrowserAutomationLease, EmbeddedAgentBrowserError> {
        let lease = self
            .store
            .browser_automation_lease(lease_id)
            .await
            .map_err(storage)?;
        if lease.owner_run_id != *run_id || lease.run_generation != generation {
            return Err(EmbeddedAgentBrowserError::LeaseOwnership);
        }
        self.validate_run(&lease.owner_session_id, run_id, generation)
            .await?;
        if lease.surface != BrowserAutomationSurfaceKind::Embedded
            || lease.status != BrowserAutomationLeaseStatus::Active
            || lease.expires_at_ms <= now_ms()
        {
            return Err(EmbeddedAgentBrowserError::LeaseInactive);
        }
        Ok(lease)
    }

    async fn devtools(
        &self,
        tab_id: &BrowserTabId,
        method: &str,
        params: Value,
    ) -> Result<Value, EmbeddedAgentBrowserError> {
        self.devtools_with_access(tab_id, method, params, false)
            .await
    }

    async fn devtools_with_access(
        &self,
        tab_id: &BrowserTabId,
        method: &str,
        params: Value,
        full_access: bool,
    ) -> Result<Value, EmbeddedAgentBrowserError> {
        match self
            .runtime
            .command(
                &self.window()?,
                CefHostCommand::DevTools {
                    tab_id: tab_id.clone(),
                    method: method.to_owned(),
                    params,
                    full_access,
                },
            )
            .await?
        {
            CefHostResponse::DevTools { result } => Ok(result),
            _ => Err(EmbeddedAgentBrowserError::InvalidAction(
                "unexpected DevTools response".into(),
            )),
        }
    }

    async fn sync_agent_navigation_policy(
        &self,
        lease: &BrowserAutomationLease,
        tab_id: &BrowserTabId,
    ) -> Result<(), EmbeddedAgentBrowserError> {
        let allowed_origins = self
            .store
            .embedded_browser_allowed_origins(&lease.owner_session_id, &lease.owner_run_id)
            .await
            .map_err(storage)?;
        self.runtime
            .command(
                &self.window()?,
                CefHostCommand::SetAgentNavigationPolicy {
                    tab_id: tab_id.clone(),
                    allowed_origins,
                },
            )
            .await?;
        Ok(())
    }

    async fn runtime_action(
        &self,
        command: CefHostCommand,
    ) -> Result<Value, EmbeddedAgentBrowserError> {
        self.runtime.command(&self.window()?, command).await?;
        Ok(json!({ "accepted": true }))
    }

    async fn selector_point(
        &self,
        tab_id: &BrowserTabId,
        selector: &str,
    ) -> Result<(f64, f64), EmbeddedAgentBrowserError> {
        let selector = serde_json::to_string(selector)
            .map_err(|error| EmbeddedAgentBrowserError::InvalidAction(error.to_string()))?;
        let result = self.devtools(tab_id, "Runtime.evaluate", json!({
            "expression": format!("(() => {{ const e=document.querySelector({selector}); if(!e) return null; const r=e.getBoundingClientRect(); return {{x:r.left+r.width/2,y:r.top+r.height/2}}; }})()"),
            "returnByValue": true,
        })).await?;
        let point = result
            .pointer("/result/value")
            .ok_or_else(|| EmbeddedAgentBrowserError::InvalidAction("selector_not_found".into()))?;
        Ok((
            point.get("x").and_then(Value::as_f64).unwrap_or(0.0),
            point.get("y").and_then(Value::as_f64).unwrap_or(0.0),
        ))
    }

    async fn focus_selector(
        &self,
        tab_id: &BrowserTabId,
        selector: &str,
        select_all: bool,
    ) -> Result<(), EmbeddedAgentBrowserError> {
        let selector = serde_json::to_string(selector)
            .map_err(|error| EmbeddedAgentBrowserError::InvalidAction(error.to_string()))?;
        let result = self.devtools(tab_id, "Runtime.evaluate", json!({
            "expression": format!("(() => {{ const e=document.querySelector({selector}); if(!e) return false; e.focus(); if({select_all} && typeof e.select==='function') e.select(); return true; }})()"),
            "returnByValue": true,
        })).await?;
        if result.pointer("/result/value").and_then(Value::as_bool) != Some(true) {
            return Err(EmbeddedAgentBrowserError::InvalidAction(
                "selector_not_found".into(),
            ));
        }
        Ok(())
    }

    async fn wait_for(
        &self,
        tab_id: &BrowserTabId,
        selector: Option<&str>,
        state: hachimi_protocol::BrowserWaitState,
        timeout_ms: u64,
    ) -> Result<Value, EmbeddedAgentBrowserError> {
        let deadline =
            tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms.min(60_000));
        loop {
            let expression = if let Some(selector) = selector {
                let selector = serde_json::to_string(selector)
                    .map_err(|error| EmbeddedAgentBrowserError::InvalidAction(error.to_string()))?;
                format!(
                    "(() => {{ const e=document.querySelector({selector}); return {{attached:!!e,visible:!!e && !!(e.offsetWidth||e.offsetHeight||e.getClientRects().length)}}; }})()"
                )
            } else {
                "({attached:true,visible:document.readyState==='complete'})".to_owned()
            };
            let result = self
                .devtools(
                    tab_id,
                    "Runtime.evaluate",
                    json!({ "expression": expression, "returnByValue": true }),
                )
                .await?;
            let value = result
                .pointer("/result/value")
                .cloned()
                .unwrap_or(Value::Null);
            let matched = match state {
                hachimi_protocol::BrowserWaitState::Attached => {
                    value.get("attached").and_then(Value::as_bool) == Some(true)
                }
                hachimi_protocol::BrowserWaitState::Visible
                | hachimi_protocol::BrowserWaitState::NavigationComplete => {
                    value.get("visible").and_then(Value::as_bool) == Some(true)
                }
                hachimi_protocol::BrowserWaitState::Hidden => {
                    value.get("visible").and_then(Value::as_bool) != Some(true)
                }
            };
            if matched {
                return Ok(json!({ "matched": true }));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(EmbeddedAgentBrowserError::InvalidAction(
                    "browser_wait_timeout".into(),
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    fn window(&self) -> Result<tauri::WebviewWindow, EmbeddedAgentBrowserError> {
        self.app
            .get_webview_window("workbench")
            .or_else(|| self.app.get_webview_window("pet"))
            .ok_or(EmbeddedAgentBrowserError::WindowMissing)
    }
}

async fn validate_tab_origin(
    url: &str,
    policy: EmbeddedBrowserOriginPolicy<'_>,
) -> Result<(), EmbeddedAgentBrowserError> {
    if url.eq_ignore_ascii_case("about:blank") {
        return Ok(());
    }
    let origin = hachimi_browser::normalized_origin(url)
        .map_err(|error| EmbeddedAgentBrowserError::InvalidAction(error.to_string()))?;
    if !policy.allow_unlisted_origin
        && !policy
            .allowed_origins
            .iter()
            .any(|allowed| allowed == &origin)
    {
        return Err(EmbeddedAgentBrowserError::SitePermissionMissing);
    }
    hachimi_browser::validate_agent_browser_target(url, policy.allow_private_network)
        .await
        .map_err(|error| EmbeddedAgentBrowserError::InvalidAction(error.to_string()))
}

fn storage(error: impl std::fmt::Display) -> EmbeddedAgentBrowserError {
    EmbeddedAgentBrowserError::Storage(error.to_string())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(i64::MAX)
}
