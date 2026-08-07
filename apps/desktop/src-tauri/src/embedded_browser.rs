use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use hachimi_browser::{
    CEF_IPC_PROTOCOL_VERSION, CefHostCommand, CefHostCommandEnvelope, CefHostEvent, CefHostFailure,
    CefHostMessage, CefHostResponse, CefTabState,
};
use hachimi_protocol::{
    BrowserTabId, BrowserWorkspace, BrowserWorkspaceChangeReason, BrowserWorkspaceChanged,
    BrowserWorkspaceId, BrowserWorkspaceRuntimeState, EmbeddedBrowserPermissionRequiredEvent,
    RuntimeComponentId, RuntimeComponentState, SessionSourceOrigin,
    WorkbenchEnvironmentChangeReason, WorkbenchEnvironmentChanged,
};
use hachimi_storage::{AgentStore, BrowserTabRuntimeUpdate};
use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindow};
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::ChildStdin,
    sync::{Mutex as AsyncMutex, oneshot, watch},
};

const HOST_READY_TIMEOUT: Duration = Duration::from_secs(15);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);
const WORKSPACE_CHANGED_EVENT: &str = "browser:workspace-changed";
const TAB_STATE_CHANGED_EVENT: &str = "browser:tab-state-changed";
const DOWNLOAD_UPDATED_EVENT: &str = "browser:download-updated";
const SHORTCUT_REQUESTED_EVENT: &str = "browser:shortcut-requested";
const RUNTIME_CRASHED_EVENT: &str = "browser:runtime-crashed";

#[derive(Debug, Error)]
pub enum EmbeddedBrowserError {
    #[error("CEF browser runtime is not bundled: {0}")]
    RuntimeMissing(String),
    #[error("CEF browser runtime failed to start: {0}")]
    StartFailed(String),
    #[error("CEF browser runtime did not become ready")]
    ReadyTimeout,
    #[error("CEF browser runtime stopped unexpectedly")]
    RuntimeCrashed,
    #[error("CEF browser command timed out")]
    CommandTimeout,
    #[error("CEF browser IPC failed: {0}")]
    Ipc(String),
    #[error("CEF browser rejected the command ({code}): {message}")]
    Rejected { code: String, message: String },
}

impl EmbeddedBrowserError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::RuntimeMissing(_) => "cef_runtime_missing",
            Self::StartFailed(_) => "cef_start_failed",
            Self::ReadyTimeout => "cef_ready_timeout",
            Self::RuntimeCrashed => "cef_runtime_crashed",
            Self::CommandTimeout => "cef_command_timeout",
            Self::Ipc(_) => "cef_ipc_failed",
            Self::Rejected { .. } => "cef_command_rejected",
        }
    }
}

struct RuntimeConnection {
    generation: u64,
    stdin: AsyncMutex<ChildStdin>,
    ready: watch::Receiver<bool>,
}

struct EmbeddedBrowserRuntime<R: Runtime> {
    app: AppHandle<R>,
    store: AgentStore,
    host_executable: PathBuf,
    profile_dir: PathBuf,
    log_file: PathBuf,
    connection: AsyncMutex<Option<Arc<RuntimeConnection>>>,
    restart_lock: AsyncMutex<()>,
    pending: Mutex<BTreeMap<u64, oneshot::Sender<Result<CefHostResponse, EmbeddedBrowserError>>>>,
    tab_workspaces: Mutex<BTreeMap<BrowserTabId, BrowserWorkspaceId>>,
    loaded_tabs: Mutex<BTreeSet<BrowserTabId>>,
    layout_revisions: Mutex<BTreeMap<BrowserTabId, u64>>,
    next_request_id: AtomicU64,
    next_generation: AtomicU64,
    active_generation: AtomicU64,
}

#[derive(Clone)]
pub struct EmbeddedBrowserService<R: Runtime> {
    runtime: Arc<EmbeddedBrowserRuntime<R>>,
}

impl<R: Runtime> EmbeddedBrowserService<R> {
    pub fn new(app: AppHandle<R>, store: AgentStore, data_dir: &Path, resource_dir: &Path) -> Self {
        Self {
            runtime: Arc::new(EmbeddedBrowserRuntime {
                app,
                store,
                host_executable: resolve_host_executable(resource_dir),
                profile_dir: data_dir.join("browser/cef-profile"),
                log_file: data_dir.join("logs/cef.log"),
                connection: AsyncMutex::new(None),
                restart_lock: AsyncMutex::new(()),
                pending: Mutex::new(BTreeMap::new()),
                tab_workspaces: Mutex::new(BTreeMap::new()),
                loaded_tabs: Mutex::new(BTreeSet::new()),
                layout_revisions: Mutex::new(BTreeMap::new()),
                next_request_id: AtomicU64::new(1),
                next_generation: AtomicU64::new(1),
                active_generation: AtomicU64::new(0),
            }),
        }
    }

    pub async fn open_workspace(
        &self,
        window: &WebviewWindow<R>,
        workspace: &BrowserWorkspace,
    ) -> Result<BrowserWorkspace, EmbeddedBrowserError> {
        let connection = self.ensure_started(window).await?;
        self.attach_window(&connection, window).await?;
        let needs_runtime_transition =
            workspace.runtime_state != BrowserWorkspaceRuntimeState::Ready;
        if needs_runtime_transition {
            self.runtime
                .store
                .set_browser_workspace_runtime(
                    &workspace.id,
                    BrowserWorkspaceRuntimeState::Starting,
                )
                .await
                .map_err(|error| EmbeddedBrowserError::Ipc(error.to_string()))?;
        }
        self.runtime.tab_workspaces.lock().extend(
            workspace
                .tabs
                .iter()
                .map(|tab| (tab.id.clone(), workspace.id.clone())),
        );
        for tab in &workspace.tabs {
            if self.runtime.loaded_tabs.lock().contains(&tab.id) {
                continue;
            }
            self.send_on(
                &connection,
                CefHostCommand::CreateTab {
                    tab_id: tab.id.clone(),
                    url: tab.url.clone(),
                    bounds: hachimi_browser::CefBounds {
                        x: 0,
                        y: 0,
                        width: 1,
                        height: 1,
                    },
                    visible: false,
                },
            )
            .await?;
            self.runtime.loaded_tabs.lock().insert(tab.id.clone());
        }
        self.send_on(
            &connection,
            CefHostCommand::ActivateTab {
                tab_id: workspace.active_tab_id.clone(),
            },
        )
        .await?;
        let ready = if needs_runtime_transition {
            self.runtime
                .store
                .set_browser_workspace_runtime(&workspace.id, BrowserWorkspaceRuntimeState::Ready)
                .await
                .map_err(|error| EmbeddedBrowserError::Ipc(error.to_string()))?
        } else {
            self.runtime
                .store
                .browser_workspace(&workspace.id)
                .await
                .map_err(|error| EmbeddedBrowserError::Ipc(error.to_string()))?
        };
        if needs_runtime_transition {
            emit_workspace(
                &self.runtime.app,
                &ready,
                BrowserWorkspaceChangeReason::Runtime,
            );
        }
        Ok(ready)
    }

    pub async fn create_tab_runtime(
        &self,
        window: &WebviewWindow<R>,
        workspace_id: &BrowserWorkspaceId,
        tab_id: &BrowserTabId,
        url: &str,
    ) -> Result<(), EmbeddedBrowserError> {
        let connection = self.ensure_started(window).await?;
        self.attach_window(&connection, window).await?;
        self.runtime
            .tab_workspaces
            .lock()
            .insert(tab_id.clone(), workspace_id.clone());
        self.send_on(
            &connection,
            CefHostCommand::CreateTab {
                tab_id: tab_id.clone(),
                url: url.to_owned(),
                bounds: hachimi_browser::CefBounds {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                visible: false,
            },
        )
        .await?;
        self.runtime.loaded_tabs.lock().insert(tab_id.clone());
        Ok(())
    }

    pub fn is_tab_loaded(&self, tab_id: &BrowserTabId) -> bool {
        self.runtime.loaded_tabs.lock().contains(tab_id)
    }

    pub fn attest(&self) -> Result<(), EmbeddedBrowserError> {
        validate_runtime(&self.runtime.host_executable)
    }

    pub fn start_supervision(&self) {
        let runtime = Arc::clone(&self.runtime);
        let supervisor = runtime
            .app
            .state::<crate::DesktopState>()
            .runtime_supervisor
            .clone();
        match validate_runtime(&runtime.host_executable) {
            Ok(()) => supervisor.ready(RuntimeComponentId::Cef),
            Err(error) => supervisor.update(
                RuntimeComponentId::Cef,
                RuntimeComponentState::Degraded,
                Some(error.code()),
                true,
                0,
                None,
            ),
        }
        let retry = supervisor.retry_signal(RuntimeComponentId::Cef);
        tauri::async_runtime::spawn(async move {
            loop {
                retry.notified().await;
                if let Err(error) = validate_runtime(&runtime.host_executable) {
                    supervisor.update(
                        RuntimeComponentId::Cef,
                        RuntimeComponentState::Degraded,
                        Some(error.code()),
                        true,
                        0,
                        None,
                    );
                    continue;
                }
                let workspace_ids = runtime
                    .tab_workspaces
                    .lock()
                    .values()
                    .cloned()
                    .collect::<BTreeSet<_>>();
                if workspace_ids.is_empty() {
                    supervisor.ready(RuntimeComponentId::Cef);
                } else {
                    restart_runtime(Arc::clone(&runtime), workspace_ids).await;
                }
            }
        });
    }

    pub async fn shutdown(&self) {
        let connection = self.runtime.connection.lock().await.clone();
        if let Some(connection) = connection {
            let _ = self.send_on(&connection, CefHostCommand::Shutdown).await;
        }
    }

    pub async fn close_tab_runtime(
        &self,
        window: &WebviewWindow<R>,
        tab_id: &BrowserTabId,
    ) -> Result<(), EmbeddedBrowserError> {
        if self.is_tab_loaded(tab_id) {
            self.command(
                window,
                CefHostCommand::CloseTab {
                    tab_id: tab_id.clone(),
                },
            )
            .await?;
        }
        self.runtime.loaded_tabs.lock().remove(tab_id);
        self.runtime.layout_revisions.lock().remove(tab_id);
        self.runtime.tab_workspaces.lock().remove(tab_id);
        Ok(())
    }

    pub async fn command(
        &self,
        window: &WebviewWindow<R>,
        command: CefHostCommand,
    ) -> Result<CefHostResponse, EmbeddedBrowserError> {
        let connection = self.ensure_started(window).await?;
        self.send_on(&connection, command).await
    }

    pub async fn update_layout(
        &self,
        window: &WebviewWindow<R>,
        tab_id: &BrowserTabId,
        bounds: hachimi_browser::CefBounds,
        visible: bool,
        layout_revision: u64,
    ) -> Result<(), EmbeddedBrowserError> {
        {
            let mut revisions = self.runtime.layout_revisions.lock();
            if revisions
                .get(tab_id)
                .is_some_and(|current| *current >= layout_revision)
            {
                return Ok(());
            }
            revisions.insert(tab_id.clone(), layout_revision);
        }
        let connection = self.ensure_started(window).await?;
        self.send_on(
            &connection,
            CefHostCommand::SetBounds {
                tab_id: tab_id.clone(),
                bounds,
            },
        )
        .await?;
        self.send_on(
            &connection,
            CefHostCommand::SetVisible {
                tab_id: tab_id.clone(),
                visible,
            },
        )
        .await?;
        Ok(())
    }

    async fn ensure_started(
        &self,
        window: &WebviewWindow<R>,
    ) -> Result<Arc<RuntimeConnection>, EmbeddedBrowserError> {
        let mut slot = self.runtime.connection.lock().await;
        if let Some(connection) = slot.as_ref()
            && self.runtime.active_generation.load(Ordering::Acquire) == connection.generation
        {
            return Ok(Arc::clone(connection));
        }
        validate_runtime(&self.runtime.host_executable)?;
        std::fs::create_dir_all(&self.runtime.profile_dir)
            .map_err(|error| EmbeddedBrowserError::StartFailed(error.to_string()))?;
        if let Some(parent) = self.runtime.log_file.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| EmbeddedBrowserError::StartFailed(error.to_string()))?;
        }
        let parent_hwnd = native_window_handle(window)?;
        let mut child = hachimi_process_policy::tokio_command(
            &self.runtime.host_executable,
            hachimi_process_policy::ProcessPolicy::HiddenBackground,
        )
        .arg(format!("--hachimi-parent-hwnd={parent_hwnd}"))
        .arg(format!(
            "--hachimi-profile-dir={}",
            self.runtime.profile_dir.display()
        ))
        .arg(format!(
            "--hachimi-log-file={}",
            self.runtime.log_file.display()
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(false)
        .spawn()
        .map_err(|error| EmbeddedBrowserError::StartFailed(error.to_string()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| EmbeddedBrowserError::StartFailed("stdin pipe missing".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| EmbeddedBrowserError::StartFailed("stdout pipe missing".into()))?;
        let generation = self.runtime.next_generation.fetch_add(1, Ordering::AcqRel);
        let (ready_tx, ready_rx) = watch::channel(false);
        let connection = Arc::new(RuntimeConnection {
            generation,
            stdin: AsyncMutex::new(stdin),
            ready: ready_rx,
        });
        self.runtime
            .active_generation
            .store(generation, Ordering::Release);
        *slot = Some(Arc::clone(&connection));
        drop(slot);

        let runtime = Arc::clone(&self.runtime);
        let reader_connection = Arc::clone(&connection);
        tauri::async_runtime::spawn(async move {
            read_host_messages(runtime, reader_connection, stdout, ready_tx).await;
        });
        let runtime = Arc::clone(&self.runtime);
        tauri::async_runtime::spawn(async move {
            let status = child.wait().await;
            runtime_crashed(runtime, generation, format!("{status:?}")).await;
        });

        let mut ready = connection.ready.clone();
        tokio::time::timeout(HOST_READY_TIMEOUT, async {
            while !*ready.borrow() {
                ready
                    .changed()
                    .await
                    .map_err(|_| EmbeddedBrowserError::RuntimeCrashed)?;
            }
            Ok::<_, EmbeddedBrowserError>(())
        })
        .await
        .map_err(|_| EmbeddedBrowserError::ReadyTimeout)??;
        let settings = self
            .runtime
            .store
            .embedded_browser_settings(false)
            .await
            .map_err(|error| EmbeddedBrowserError::Ipc(error.to_string()))?;
        self.send_on(
            &connection,
            CefHostCommand::ConfigureDownloads {
                directory: settings.download_directory,
                ask_where_to_save: settings.ask_where_to_save_downloads,
            },
        )
        .await?;
        self.runtime
            .app
            .state::<crate::DesktopState>()
            .runtime_supervisor
            .ready(RuntimeComponentId::Cef);
        Ok(connection)
    }

    async fn send_on(
        &self,
        connection: &Arc<RuntimeConnection>,
        command: CefHostCommand,
    ) -> Result<CefHostResponse, EmbeddedBrowserError> {
        send_runtime_command(&self.runtime, connection, command).await
    }

    async fn attach_window(
        &self,
        connection: &Arc<RuntimeConnection>,
        window: &WebviewWindow<R>,
    ) -> Result<(), EmbeddedBrowserError> {
        let parent_hwnd = u64::try_from(native_window_handle(window)?)
            .map_err(|error| EmbeddedBrowserError::StartFailed(error.to_string()))?;
        self.send_on(connection, CefHostCommand::SetParentWindow { parent_hwnd })
            .await?;
        Ok(())
    }
}

async fn send_runtime_command<R: Runtime>(
    runtime: &Arc<EmbeddedBrowserRuntime<R>>,
    connection: &Arc<RuntimeConnection>,
    command: CefHostCommand,
) -> Result<CefHostResponse, EmbeddedBrowserError> {
    if runtime.active_generation.load(Ordering::Acquire) != connection.generation {
        return Err(EmbeddedBrowserError::RuntimeCrashed);
    }
    let request_id = runtime.next_request_id.fetch_add(1, Ordering::AcqRel);
    let envelope = CefHostCommandEnvelope::new(request_id, command);
    let mut encoded = serde_json::to_vec(&envelope)
        .map_err(|error| EmbeddedBrowserError::Ipc(error.to_string()))?;
    encoded.push(b'\n');
    let (sender, receiver) = oneshot::channel();
    runtime.pending.lock().insert(request_id, sender);
    let write_result = connection.stdin.lock().await.write_all(&encoded).await;
    if let Err(error) = write_result {
        runtime.pending.lock().remove(&request_id);
        return Err(EmbeddedBrowserError::Ipc(error.to_string()));
    }
    tokio::time::timeout(COMMAND_TIMEOUT, receiver)
        .await
        .map_err(|_| {
            runtime.pending.lock().remove(&request_id);
            EmbeddedBrowserError::CommandTimeout
        })?
        .map_err(|_| EmbeddedBrowserError::RuntimeCrashed)?
}

async fn read_host_messages<R: Runtime>(
    runtime: Arc<EmbeddedBrowserRuntime<R>>,
    connection: Arc<RuntimeConnection>,
    stdout: tokio::process::ChildStdout,
    ready: watch::Sender<bool>,
) {
    let mut lines = BufReader::new(stdout).lines();
    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(error) => {
                tracing::warn!(%error, "CEF browser IPC stdout failed");
                break;
            }
        };
        let message = match serde_json::from_str::<CefHostMessage>(&line) {
            Ok(message) => message,
            Err(error) => {
                tracing::warn!(%error, "ignored invalid CEF browser IPC message");
                continue;
            }
        };
        match message {
            CefHostMessage::Ready {
                protocol_version, ..
            } if protocol_version == CEF_IPC_PROTOCOL_VERSION => {
                let _ = ready.send(true);
            }
            CefHostMessage::Ready { .. } => {
                fail_all_pending(&runtime, EmbeddedBrowserError::RuntimeCrashed);
                break;
            }
            CefHostMessage::Response { request_id, result } => {
                if let Some(sender) = runtime.pending.lock().remove(&request_id) {
                    let result = result.map_err(rejected);
                    let _ = sender.send(result);
                }
            }
            CefHostMessage::Event { event } => {
                handle_host_event(&runtime, &connection, event).await;
            }
            CefHostMessage::Fatal { code, message } => {
                tracing::error!(%code, %message, "CEF browser host reported a fatal error");
            }
        }
    }
}

async fn handle_host_event<R: Runtime>(
    runtime: &Arc<EmbeddedBrowserRuntime<R>>,
    connection: &Arc<RuntimeConnection>,
    event: CefHostEvent,
) {
    match event {
        CefHostEvent::TabStateChanged { state } => {
            persist_tab_state(runtime, &state, false).await;
        }
        CefHostEvent::UserInput {
            tab_id,
            input_epoch: _,
        } => {
            let workspace_id = runtime.tab_workspaces.lock().get(&tab_id).cloned();
            if let Some(workspace_id) = workspace_id {
                let update = BrowserTabRuntimeUpdate {
                    user_input: true,
                    ..BrowserTabRuntimeUpdate::default()
                };
                if let Ok(mut workspace) = runtime
                    .store
                    .update_browser_tab_runtime(&workspace_id, &tab_id, update)
                    .await
                {
                    if let Ok(Some(suspended)) = runtime
                        .store
                        .suspend_active_browser_automation_for_tab(&tab_id)
                        .await
                    {
                        workspace = suspended;
                    }
                    let _ = runtime.app.emit(TAB_STATE_CHANGED_EVENT, &workspace);
                    emit_workspace(
                        &runtime.app,
                        &workspace,
                        BrowserWorkspaceChangeReason::Navigation,
                    );
                }
            }
        }
        shortcut @ CefHostEvent::ShortcutRequested { .. } => {
            let _ = runtime.app.emit(SHORTCUT_REQUESTED_EVENT, shortcut);
        }
        CefHostEvent::PopupRequested {
            opener_tab_id,
            target_url,
        } => {
            let Some(workspace_id) = runtime.tab_workspaces.lock().get(&opener_tab_id).cloned()
            else {
                return;
            };
            let runtime = Arc::clone(runtime);
            let connection = Arc::clone(connection);
            tauri::async_runtime::spawn(async move {
                let Ok(current) = runtime.store.browser_workspace(&workspace_id).await else {
                    return;
                };
                let Ok(updated) = runtime
                    .store
                    .create_browser_tab(&workspace_id, current.revision, Some(&target_url))
                    .await
                else {
                    return;
                };
                let tab_id = updated.active_tab_id.clone();
                runtime
                    .tab_workspaces
                    .lock()
                    .insert(tab_id.clone(), workspace_id.clone());
                if send_runtime_command(
                    &runtime,
                    &connection,
                    CefHostCommand::CreateTab {
                        tab_id: tab_id.clone(),
                        url: target_url,
                        bounds: hachimi_browser::CefBounds {
                            x: 0,
                            y: 0,
                            width: 1,
                            height: 1,
                        },
                        visible: false,
                    },
                )
                .await
                .is_ok()
                {
                    runtime.loaded_tabs.lock().insert(tab_id.clone());
                    let _ = send_runtime_command(
                        &runtime,
                        &connection,
                        CefHostCommand::ActivateTab { tab_id },
                    )
                    .await;
                    emit_workspace(
                        &runtime.app,
                        &updated,
                        BrowserWorkspaceChangeReason::TabCreated,
                    );
                }
            });
        }
        CefHostEvent::AgentNavigationBlocked { tab_id, target_url } => {
            let Some(workspace_id) = runtime.tab_workspaces.lock().get(&tab_id).cloned() else {
                return;
            };
            let Ok(workspace) = runtime.store.browser_workspace(&workspace_id).await else {
                return;
            };
            let Some(lease) = workspace.automation_lease.as_ref().filter(|lease| {
                lease.status == hachimi_protocol::BrowserAutomationLeaseStatus::Active
                    && lease.tab_id.as_ref() == Some(&tab_id)
            }) else {
                return;
            };
            let Ok(origin) = hachimi_browser::normalized_origin(&target_url) else {
                return;
            };
            let private_network = matches!(
                hachimi_browser::validate_agent_browser_target(&target_url, false).await,
                Err(hachimi_browser::BrowserHostError::PrivateNetworkDenied)
            );
            let Some(tab) = workspace.tabs.iter().find(|tab| tab.id == tab_id) else {
                return;
            };
            if let Ok(request) = runtime
                .store
                .create_embedded_browser_permission_request(
                    &workspace_id,
                    &tab_id,
                    Some(&lease.id),
                    &workspace.owner_session_id,
                    &lease.owner_run_id,
                    lease.run_generation,
                    &origin,
                    private_network,
                    tab.revision,
                )
                .await
            {
                let event = EmbeddedBrowserPermissionRequiredEvent {
                    request,
                    reason_code: "agent_redirect_permission_required".into(),
                };
                let _ = runtime.app.emit("browser:permission-required", &event);
                let _ = runtime
                    .store
                    .append_event(
                        &workspace.owner_session_id,
                        Some(&lease.owner_run_id),
                        "browser.permission_required",
                        serde_json::to_value(&event).unwrap_or_default(),
                    )
                    .await;
            }
        }
        CefHostEvent::RenderProcessTerminated { tab_id, status } => {
            tracing::warn!(%tab_id, %status, "CEF browser render process terminated");
        }
        CefHostEvent::RuntimeCrashed { message } => {
            runtime_crashed(Arc::clone(runtime), connection.generation, message).await;
        }
        CefHostEvent::DownloadUpdated {
            tab_id,
            download_id,
            url,
            suggested_name,
            destination,
            received_bytes,
            total_bytes,
            complete,
            cancelled,
            interrupted,
        } => match runtime
            .store
            .upsert_browser_download(hachimi_storage::BrowserDownloadRuntimeUpdate {
                runtime_id: download_id,
                tab_id,
                source_url: url,
                suggested_name,
                destination,
                received_bytes,
                total_bytes,
                complete,
                cancelled,
                interrupted,
            })
            .await
        {
            Ok(download) => {
                let _ = runtime.app.emit(DOWNLOAD_UPDATED_EVENT, download);
            }
            Err(error) => tracing::warn!(%error, "failed to persist CEF browser download"),
        },
    }
}

async fn persist_tab_state<R: Runtime>(
    runtime: &Arc<EmbeddedBrowserRuntime<R>>,
    state: &CefTabState,
    user_input: bool,
) {
    let Some(workspace_id) = runtime.tab_workspaces.lock().get(&state.tab_id).cloned() else {
        return;
    };
    let update = BrowserTabRuntimeUpdate {
        url: Some(state.url.clone()),
        title: Some(state.title.clone()),
        loading: Some(state.loading),
        can_go_back: Some(state.can_go_back),
        can_go_forward: Some(state.can_go_forward),
        runtime_loaded: Some(true),
        navigation_error: Some(state.navigation_error.clone()),
        user_input,
        ..BrowserTabRuntimeUpdate::default()
    };
    match runtime
        .store
        .update_browser_tab_runtime(&workspace_id, &state.tab_id, update)
        .await
    {
        Ok(workspace) => {
            if !state.loading
                && state.navigation_error.is_none()
                && let Some(url) = hachimi_storage::canonical_session_source_url(&state.url)
                && runtime
                    .store
                    .upsert_session_web_source(
                        &workspace.owner_session_id,
                        None,
                        SessionSourceOrigin::Browser,
                        &url,
                        (!state.title.trim().is_empty()).then_some(state.title.as_str()),
                        Some(&state.tab_id),
                    )
                    .await
                    .is_ok()
                && let Ok(Some(environment)) = runtime
                    .store
                    .get_session_environment_state(&workspace.owner_session_id)
                    .await
            {
                let _ = runtime.app.emit(
                    crate::environment_commands::WORKBENCH_ENVIRONMENT_EVENT,
                    WorkbenchEnvironmentChanged {
                        session_id: workspace.owner_session_id.clone(),
                        revision: environment.revision,
                        reasons: vec![
                            WorkbenchEnvironmentChangeReason::Browser,
                            WorkbenchEnvironmentChangeReason::Sources,
                        ],
                    },
                );
            }
            let _ = runtime.app.emit(TAB_STATE_CHANGED_EVENT, &workspace);
            emit_workspace(
                &runtime.app,
                &workspace,
                if state.navigation_error.is_some() {
                    BrowserWorkspaceChangeReason::Error
                } else if state.loading {
                    BrowserWorkspaceChangeReason::Loading
                } else {
                    BrowserWorkspaceChangeReason::Navigation
                },
            );
        }
        Err(error) => tracing::warn!(%error, "failed to persist CEF browser tab state"),
    }
}

async fn runtime_crashed<R: Runtime>(
    runtime: Arc<EmbeddedBrowserRuntime<R>>,
    generation: u64,
    message: String,
) {
    if runtime
        .active_generation
        .compare_exchange(generation, 0, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    runtime.loaded_tabs.lock().clear();
    runtime.layout_revisions.lock().clear();
    fail_all_pending(&runtime, EmbeddedBrowserError::RuntimeCrashed);
    let supervisor = runtime
        .app
        .state::<crate::DesktopState>()
        .runtime_supervisor
        .clone();
    if supervisor.is_shutting_down() {
        return;
    }
    let workspace_ids = runtime
        .tab_workspaces
        .lock()
        .values()
        .cloned()
        .collect::<BTreeSet<_>>();
    for workspace_id in &workspace_ids {
        if let Ok(workspace) = runtime
            .store
            .set_browser_workspace_runtime(workspace_id, BrowserWorkspaceRuntimeState::Failed)
            .await
        {
            emit_workspace(
                &runtime.app,
                &workspace,
                BrowserWorkspaceChangeReason::Runtime,
            );
        }
    }
    let _ = runtime.app.emit(
        RUNTIME_CRASHED_EVENT,
        serde_json::json!({ "generation": generation, "message": message }),
    );
    supervisor
        .retry_signal(RuntimeComponentId::Cef)
        .notify_one();
}

async fn restart_runtime<R: Runtime>(
    runtime: Arc<EmbeddedBrowserRuntime<R>>,
    workspace_ids: BTreeSet<BrowserWorkspaceId>,
) {
    let Ok(_restart_guard) = runtime.restart_lock.try_lock() else {
        return;
    };
    if runtime.active_generation.load(Ordering::Acquire) != 0 {
        return;
    }
    let supervisor = runtime
        .app
        .state::<crate::DesktopState>()
        .runtime_supervisor
        .clone();
    let Some(window) = runtime.app.get_webview_window("workbench") else {
        supervisor.update(
            RuntimeComponentId::Cef,
            RuntimeComponentState::Degraded,
            Some("cef_window_unavailable"),
            true,
            0,
            None,
        );
        return;
    };
    let service = EmbeddedBrowserService {
        runtime: Arc::clone(&runtime),
    };
    for (index, delay) in [1_u64, 2, 5].into_iter().enumerate() {
        if supervisor.is_shutting_down() {
            return;
        }
        let attempt = (index + 1) as u32;
        supervisor.update(
            RuntimeComponentId::Cef,
            RuntimeComponentState::Retrying,
            Some("cef_runtime_crashed"),
            false,
            attempt,
            Some(now_ms().saturating_add((delay * 1_000) as i64)),
        );
        tokio::time::sleep(Duration::from_secs(delay)).await;
        let result = async {
            service.ensure_started(&window).await?;
            for workspace_id in &workspace_ids {
                let workspace = runtime
                    .store
                    .browser_workspace(workspace_id)
                    .await
                    .map_err(|error| EmbeddedBrowserError::Ipc(error.to_string()))?;
                service.open_workspace(&window, &workspace).await?;
            }
            Ok::<(), EmbeddedBrowserError>(())
        }
        .await;
        match result {
            Ok(()) => {
                supervisor.ready(RuntimeComponentId::Cef);
                return;
            }
            Err(error) => {
                tracing::warn!(attempt, code = error.code(), %error, "CEF automatic recovery failed");
            }
        }
    }
    supervisor.update(
        RuntimeComponentId::Cef,
        RuntimeComponentState::Degraded,
        Some("cef_restart_exhausted"),
        true,
        3,
        None,
    );
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(i64::MAX)
}

fn fail_all_pending<R: Runtime>(
    runtime: &Arc<EmbeddedBrowserRuntime<R>>,
    _error: EmbeddedBrowserError,
) {
    for (_, sender) in std::mem::take(&mut *runtime.pending.lock()) {
        let _ = sender.send(Err(EmbeddedBrowserError::RuntimeCrashed));
    }
}

fn rejected(failure: CefHostFailure) -> EmbeddedBrowserError {
    EmbeddedBrowserError::Rejected {
        code: failure.code,
        message: failure.message,
    }
}

fn emit_workspace<R: Runtime>(
    app: &AppHandle<R>,
    workspace: &BrowserWorkspace,
    reason: BrowserWorkspaceChangeReason,
) {
    let _ = app.emit(
        WORKSPACE_CHANGED_EVENT,
        BrowserWorkspaceChanged {
            workspace_id: workspace.id.clone(),
            owner_session_id: workspace.owner_session_id.clone(),
            reason,
            revision: workspace.revision,
        },
    );
    let _ = app.emit(TAB_STATE_CHANGED_EVENT, workspace);
}

fn resolve_host_executable(resource_dir: &Path) -> PathBuf {
    std::env::var_os("HACHIMI_CEF_HOST")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let bundled = resource_dir.join("cef-runtime/hachimi-cef-host.exe");
            if bundled.is_file() {
                return bundled;
            }
            let adjacent = std::env::current_exe()
                .ok()
                .and_then(|path| path.parent().map(Path::to_path_buf))
                .unwrap_or_default()
                .join("cef-runtime/hachimi-cef-host.exe");
            if adjacent.is_file() {
                return adjacent;
            }
            std::env::current_dir()
                .unwrap_or_default()
                .join("target/cef-bundle/hachimi-cef-host.exe")
        })
}

fn validate_runtime(executable: &Path) -> Result<(), EmbeddedBrowserError> {
    let directory = executable
        .parent()
        .ok_or_else(|| EmbeddedBrowserError::RuntimeMissing(executable.display().to_string()))?;
    for required in [
        executable.to_path_buf(),
        directory.join("hachimi-cef-host.dll"),
        directory.join("libcef.dll"),
        directory.join("icudtl.dat"),
        directory.join("locales/en-US.pak"),
    ] {
        if !required.is_file() {
            return Err(EmbeddedBrowserError::RuntimeMissing(
                required.display().to_string(),
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn native_window_handle<R: Runtime>(
    window: &WebviewWindow<R>,
) -> Result<usize, EmbeddedBrowserError> {
    let hwnd = window
        .hwnd()
        .map_err(|error| EmbeddedBrowserError::StartFailed(error.to_string()))?;
    Ok(hwnd.0 as usize)
}

#[cfg(not(windows))]
fn native_window_handle<R: Runtime>(
    _window: &WebviewWindow<R>,
) -> Result<usize, EmbeddedBrowserError> {
    Err(EmbeddedBrowserError::StartFailed(
        "CEF native embedding is only enabled on Windows x64".into(),
    ))
}
