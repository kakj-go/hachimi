use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use hachimi_protocol::{
    ControlMethod, McpServerTransport, MutationContext, SandboxBootstrapState,
    SandboxRepairRequest, SandboxRuntimeSnapshot,
};
use tauri::{AppHandle, Manager, State, WebviewWindow};

use super::{CommandError, DesktopState, require_window};

#[derive(Debug, Clone, Default)]
pub(super) struct SandboxActivityTracker {
    state: Arc<AtomicU64>,
}

impl SandboxActivityTracker {
    const REPAIRING: u64 = 1 << 63;

    pub(super) fn try_enter(&self) -> Option<SandboxActivityGuard> {
        let mut current = self.state.load(Ordering::Acquire);
        loop {
            if current & Self::REPAIRING != 0 {
                return None;
            }
            let next = current.checked_add(1)?;
            match self.state.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Some(SandboxActivityGuard {
                        state: Arc::clone(&self.state),
                    });
                }
                Err(observed) => current = observed,
            }
        }
    }

    pub(super) fn is_busy(&self) -> bool {
        self.state.load(Ordering::Acquire) != 0
    }

    fn try_begin_repair(&self) -> Option<SandboxRepairGuard> {
        self.state
            .compare_exchange(0, Self::REPAIRING, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| SandboxRepairGuard {
                state: Arc::clone(&self.state),
            })
    }
}

pub(super) struct SandboxActivityGuard {
    state: Arc<AtomicU64>,
}

impl Drop for SandboxActivityGuard {
    fn drop(&mut self) {
        self.state.fetch_sub(1, Ordering::AcqRel);
    }
}

struct SandboxRepairGuard {
    state: Arc<AtomicU64>,
}

impl Drop for SandboxRepairGuard {
    fn drop(&mut self) {
        self.state.store(0, Ordering::Release);
    }
}

pub(super) fn enter_sandbox_activity(
    state: &DesktopState,
) -> Result<SandboxActivityGuard, CommandError> {
    state.sandbox_activity.try_enter().ok_or_else(|| {
        CommandError::new(
            "sandbox_repair_in_progress",
            "Windows sandbox repair is in progress; retry after attestation completes",
        )
    })
}

fn authorize(
    window: &WebviewWindow,
    state: &DesktopState,
) -> Result<hachimi_protocol::ClientContext, CommandError> {
    let client = state.authorize(window, ControlMethod::WorkbenchWindow)?;
    require_window(window, "workbench")?;
    Ok(client)
}

fn validate_context(
    context: &MutationContext,
    client: &hachimi_protocol::ClientContext,
) -> Result<(), CommandError> {
    if context.protocol_version != hachimi_protocol::CONTROL_PROTOCOL_VERSION
        || context.client_id != client.client_id
        || context.request_id.0.trim().is_empty()
        || context.idempotency_key.trim().is_empty()
        || context.idempotency_key.len() > 128
        || context.expected_run_id.is_some()
        || context.expected_generation.is_some()
    {
        return Err(CommandError::new(
            "sandbox_repair_context_invalid",
            "sandbox repair requires a bounded direct-user mutation context",
        ));
    }
    Ok(())
}

#[tauri::command]
pub(super) async fn get_sandbox_status(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<SandboxRuntimeSnapshot, CommandError> {
    authorize(&window, &state)?;
    Ok(state.sandbox_runtime.snapshot())
}

#[tauri::command]
pub(super) async fn get_sandbox_bootstrap_state(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<SandboxBootstrapState, CommandError> {
    authorize(&window, &state)?;
    Ok(state.sandbox_runtime.bootstrap_state())
}

#[tauri::command]
pub(super) async fn refresh_sandbox_status(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<SandboxRuntimeSnapshot, CommandError> {
    authorize(&window, &state)?;
    Ok(state.sandbox_runtime.refresh().await)
}

#[tauri::command]
pub(super) async fn attest_sandbox(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<SandboxBootstrapState, CommandError> {
    authorize(&window, &state)?;
    state.sandbox_runtime.refresh().await;
    Ok(state.sandbox_runtime.bootstrap_state())
}

#[tauri::command]
pub(super) async fn repair_sandbox(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: SandboxRepairRequest,
) -> Result<SandboxRuntimeSnapshot, CommandError> {
    let client = authorize(&window, &state)?;
    validate_context(&request.context, &client)?;
    let agent_busy = !state.agent_executor.registry().is_empty();
    let process_busy = state.process_registry.has_active_processes().await;
    let mcp_busy = state
        .mcp_control
        .ready_runtimes()
        .await
        .map_err(|error| CommandError::operation("sandbox_mcp_state_failed", error))?
        .into_iter()
        .any(|runtime| {
            matches!(
                runtime.configuration.transport,
                McpServerTransport::Stdio { .. }
            )
        });
    if state.sandbox_activity.is_busy() || agent_busy || process_busy || mcp_busy {
        return Err(CommandError::new(
            "sandbox_busy",
            "stop active Agent, Process, Workspace, and stdio MCP activity before repairing the sandbox",
        ));
    }
    let _repair_guard = state.sandbox_activity.try_begin_repair().ok_or_else(|| {
        CommandError::new(
            "sandbox_busy",
            "Sandbox activity started while repair was being prepared; stop it and retry",
        )
    })?;
    let resource_root = app
        .path()
        .resource_dir()
        .map_err(|error| CommandError::operation("sandbox_resource_lookup_failed", error))?;
    crate::managed_sandbox_runtime::stage(&state.storage_layout.root, &resource_root)
        .map_err(|error| CommandError::operation("sandbox_runtime_restage_failed", error))?;
    state
        .sandbox_runtime
        .repair()
        .await
        .map_err(|error| CommandError::new(error.code, error.message))
}

#[cfg(test)]
mod tests {
    use super::SandboxActivityTracker;

    #[test]
    fn repair_and_sandbox_activity_are_mutually_exclusive() {
        let tracker = SandboxActivityTracker::default();
        let activity = tracker.try_enter().expect("activity");
        assert!(tracker.try_begin_repair().is_none());
        drop(activity);

        let repair = tracker.try_begin_repair().expect("repair");
        assert!(tracker.try_enter().is_none());
        drop(repair);

        assert!(tracker.try_enter().is_some());
    }
}
