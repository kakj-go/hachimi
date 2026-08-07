use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use hachimi_protocol::{
    RuntimeComponentHealth, RuntimeComponentId, RuntimeComponentState, RuntimeHealthSnapshot,
};
use parking_lot::RwLock;
use tauri::{AppHandle, Emitter, State, WebviewWindow};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::{CommandError, ControlMethod, DesktopState, epoch_millis, require_window};

pub(super) const RUNTIME_HEALTH_EVENT: &str = "runtime-health-changed";

#[derive(Clone)]
pub(super) struct RuntimeSupervisor {
    app: AppHandle,
    state: Arc<RwLock<SupervisorState>>,
    retries: Arc<BTreeMap<RuntimeComponentId, Arc<Notify>>>,
    shutdown: CancellationToken,
    shutdown_started: Arc<AtomicBool>,
}

struct SupervisorState {
    components: BTreeMap<RuntimeComponentId, RuntimeComponentHealth>,
    internal_resource_issues: BTreeMap<String, BTreeSet<String>>,
    revision: u64,
}

impl RuntimeSupervisor {
    pub(super) fn new(app: AppHandle) -> Self {
        let now = now_ms();
        let components = component_ids()
            .into_iter()
            .map(|component| {
                (
                    component,
                    RuntimeComponentHealth {
                        component,
                        state: RuntimeComponentState::Starting,
                        error_code: None,
                        retryable: false,
                        attempt: 0,
                        next_retry_at_ms: None,
                        updated_at_ms: now,
                    },
                )
            })
            .collect();
        let retries = component_ids()
            .into_iter()
            .map(|component| (component, Arc::new(Notify::new())))
            .collect();
        Self {
            app,
            state: Arc::new(RwLock::new(SupervisorState {
                components,
                internal_resource_issues: BTreeMap::new(),
                revision: 1,
            })),
            retries: Arc::new(retries),
            shutdown: CancellationToken::new(),
            shutdown_started: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(super) fn snapshot(&self) -> RuntimeHealthSnapshot {
        let state = self.state.read();
        RuntimeHealthSnapshot {
            components: state.components.values().cloned().collect(),
            revision: state.revision,
        }
    }

    pub(super) fn update(
        &self,
        component: RuntimeComponentId,
        state: RuntimeComponentState,
        error_code: Option<&str>,
        retryable: bool,
        attempt: u32,
        next_retry_at_ms: Option<i64>,
    ) {
        let snapshot = {
            let mut current = self.state.write();
            current.revision = current.revision.saturating_add(1);
            current.components.insert(
                component,
                RuntimeComponentHealth {
                    component,
                    state,
                    error_code: error_code.map(str::to_owned),
                    retryable,
                    attempt,
                    next_retry_at_ms,
                    updated_at_ms: now_ms(),
                },
            );
            RuntimeHealthSnapshot {
                components: current.components.values().cloned().collect(),
                revision: current.revision,
            }
        };
        let _ = self.app.emit(RUNTIME_HEALTH_EVENT, snapshot);
    }

    pub(super) fn ready(&self, component: RuntimeComponentId) {
        self.update(
            component,
            RuntimeComponentState::Ready,
            None,
            false,
            0,
            None,
        );
    }

    pub(super) fn replace_internal_resource_issues(
        &self,
        owner: &str,
        issues: impl IntoIterator<Item = impl Into<String>>,
    ) {
        let first_issue = {
            let mut state = self.state.write();
            let issues = issues.into_iter().map(Into::into).collect::<BTreeSet<_>>();
            if issues.is_empty() {
                state.internal_resource_issues.remove(owner);
            } else {
                state
                    .internal_resource_issues
                    .insert(owner.to_owned(), issues);
            }
            state
                .internal_resource_issues
                .values()
                .flat_map(BTreeSet::iter)
                .next()
                .cloned()
        };
        if let Some(code) = first_issue {
            self.update(
                RuntimeComponentId::InternalResources,
                RuntimeComponentState::Degraded,
                Some(&code),
                true,
                0,
                None,
            );
        } else {
            self.ready(RuntimeComponentId::InternalResources);
        }
    }

    pub(super) fn retry_signal(&self, component: RuntimeComponentId) -> Arc<Notify> {
        Arc::clone(
            self.retries
                .get(&component)
                .expect("every runtime component has a retry signal"),
        )
    }

    pub(super) fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    pub(super) fn begin_shutdown(&self) -> bool {
        if self.shutdown_started.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.shutdown.cancel();
        true
    }

    pub(super) fn is_shutting_down(&self) -> bool {
        self.shutdown.is_cancelled()
    }

    fn request_retry(&self, component: RuntimeComponentId) -> Result<(), CommandError> {
        let current = self
            .state
            .read()
            .components
            .get(&component)
            .cloned()
            .ok_or_else(|| CommandError::new("runtime_component_unknown", "Unknown runtime."))?;
        if !current.retryable {
            return Err(CommandError::new(
                "runtime_retry_unavailable",
                "This runtime cannot be retried in its current state.",
            ));
        }
        self.update(
            component,
            RuntimeComponentState::Starting,
            None,
            false,
            0,
            None,
        );
        self.retry_signal(component).notify_waiters();
        Ok(())
    }
}

#[tauri::command]
pub(super) fn get_runtime_health(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<RuntimeHealthSnapshot, CommandError> {
    // Runtime health is a Workbench diagnostic surface. WindowInteract is
    // reserved for the Pet window and would incorrectly produce missing_scope
    // for every Workbench settings page.
    state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    Ok(state.runtime_supervisor.snapshot())
}

#[tauri::command]
pub(super) fn retry_runtime_component(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    component: RuntimeComponentId,
) -> Result<RuntimeHealthSnapshot, CommandError> {
    state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    state.runtime_supervisor.request_retry(component)?;
    Ok(state.runtime_supervisor.snapshot())
}

fn component_ids() -> [RuntimeComponentId; 7] {
    [
        RuntimeComponentId::Gateway,
        RuntimeComponentId::InternalResources,
        RuntimeComponentId::Mcp,
        RuntimeComponentId::Scheduler,
        RuntimeComponentId::BrowserExtension,
        RuntimeComponentId::Cef,
        RuntimeComponentId::ComputerUse,
    ]
}

fn now_ms() -> i64 {
    i64::try_from(epoch_millis()).unwrap_or(i64::MAX)
}
