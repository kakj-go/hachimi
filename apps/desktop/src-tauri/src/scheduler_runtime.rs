use std::{path::Path, sync::Arc, time::Duration};

use hachimi_protocol::{RuntimeComponentId, RuntimeComponentState};
use hachimi_scheduler::{SchedulerRuntimeEvent, SchedulerRuntimeObserver, SchedulerService};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::{DesktopState, epoch_millis, runtime_supervisor::RuntimeSupervisor};

const RESTART_GUARD_MS: i64 = 10 * 60 * 1_000;
const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SchedulerRestartMarker {
    version: u32,
    last_restart_at_ms: i64,
    error_code: String,
}

pub(super) fn start_desktop_scheduler(
    app: &AppHandle,
    scheduler: Arc<SchedulerService>,
    enabled: bool,
) {
    let supervisor = app.state::<DesktopState>().runtime_supervisor.clone();
    if !enabled {
        supervisor.update(
            RuntimeComponentId::Scheduler,
            RuntimeComponentState::Degraded,
            Some("scheduler_disabled"),
            false,
            0,
            None,
        );
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        run_scheduler_supervisor(app, scheduler, supervisor).await;
    });
}

async fn run_scheduler_supervisor(
    app: AppHandle,
    scheduler: Arc<SchedulerService>,
    supervisor: RuntimeSupervisor,
) {
    let retry = supervisor.retry_signal(RuntimeComponentId::Scheduler);
    let shutdown = supervisor.shutdown_token();
    loop {
        if shutdown.is_cancelled() {
            return;
        }
        if let Err((code, detail)) = reconcile_with_retry(&scheduler, &supervisor).await {
            scheduler.suspend();
            tracing::error!(error_code = code, error = %detail, "Scheduler reconciliation exhausted retries");
            if handle_terminal_failure(&app, &supervisor, code).await {
                return;
            }
            tokio::select! {
                () = shutdown.cancelled() => return,
                () = retry.notified() => {}
            }
            continue;
        }

        let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel();
        let observer: SchedulerRuntimeObserver = Arc::new(move |event| {
            let _ = events_tx.send(event);
        });
        let handle = Arc::clone(&scheduler).start_with_observer(observer);
        *app.state::<DesktopState>().scheduler_handle.lock() = Some(handle);

        let mut terminal = None;
        loop {
            let event = tokio::select! {
                () = shutdown.cancelled() => return,
                event = events_rx.recv() => event,
            };
            let Some(event) = event else { break };
            match event {
                SchedulerRuntimeEvent::Ready | SchedulerRuntimeEvent::Recovered => {
                    supervisor.ready(RuntimeComponentId::Scheduler);
                }
                SchedulerRuntimeEvent::Retrying {
                    attempt,
                    error_code,
                } => {
                    let delay = RETRY_DELAYS[(attempt.saturating_sub(1) as usize).min(2)];
                    supervisor.update(
                        RuntimeComponentId::Scheduler,
                        RuntimeComponentState::Retrying,
                        Some(error_code),
                        false,
                        attempt,
                        Some(now_ms().saturating_add(delay.as_millis() as i64)),
                    );
                }
                SchedulerRuntimeEvent::Failed { error_code, detail } => {
                    tracing::error!(error_code, error = %detail, "Scheduler runtime exhausted retries");
                    terminal = Some(error_code);
                    break;
                }
            }
        }
        app.state::<DesktopState>().scheduler_handle.lock().take();
        let Some(code) = terminal else {
            return;
        };
        if handle_terminal_failure(&app, &supervisor, code).await {
            return;
        }
        tokio::select! {
            () = shutdown.cancelled() => return,
            () = retry.notified() => {}
        }
    }
}

async fn reconcile_with_retry(
    scheduler: &SchedulerService,
    supervisor: &RuntimeSupervisor,
) -> Result<(), (&'static str, String)> {
    for (attempt, retry_delay) in RETRY_DELAYS
        .iter()
        .copied()
        .map(Some)
        .chain(std::iter::once(None))
        .enumerate()
    {
        match scheduler.reconcile_startup().await {
            Ok(_) => return Ok(()),
            Err(error) if retry_delay.is_some() => {
                let retry_attempt = (attempt + 1) as u32;
                let delay = retry_delay.expect("guarded by is_some");
                supervisor.update(
                    RuntimeComponentId::Scheduler,
                    RuntimeComponentState::Retrying,
                    Some("scheduler_reconciliation_failed"),
                    false,
                    retry_attempt,
                    Some(now_ms().saturating_add(delay.as_millis() as i64)),
                );
                tracing::warn!(%error, retry_attempt, "Scheduler startup reconciliation failed");
                tokio::time::sleep(delay).await;
            }
            Err(error) => return Err(("scheduler_reconciliation_failed", error.to_string())),
        }
    }
    unreachable!("retry loop always returns")
}

async fn handle_terminal_failure(
    app: &AppHandle,
    supervisor: &RuntimeSupervisor,
    error_code: &'static str,
) -> bool {
    let marker_path = app
        .state::<DesktopState>()
        .storage_layout
        .root
        .join("runtime/scheduler-restart.json");
    let now = now_ms();
    if recent_restart(&marker_path, now) {
        supervisor.update(
            RuntimeComponentId::Scheduler,
            RuntimeComponentState::Degraded,
            Some("scheduler_restart_rate_limited"),
            true,
            3,
            None,
        );
        return false;
    }
    if let Err(error) = write_marker(&marker_path, now, error_code) {
        tracing::error!(%error, "Scheduler restart marker could not be persisted");
        supervisor.update(
            RuntimeComponentId::Scheduler,
            RuntimeComponentState::Degraded,
            Some("scheduler_restart_marker_failed"),
            true,
            3,
            None,
        );
        return false;
    }
    supervisor.update(
        RuntimeComponentId::Scheduler,
        RuntimeComponentState::Failed,
        Some("scheduler_restart_required"),
        false,
        3,
        None,
    );
    prepare_for_restart(app).await;
    app.request_restart();
    true
}

async fn prepare_for_restart(app: &AppHandle) {
    let state = app.state::<DesktopState>();
    if let Err(error) = state.save_settings() {
        tracing::warn!(code = error.code, message = %error.message, "Settings flush before Scheduler restart failed");
    }
    crate::shutdown_coordinator::shutdown(&state).await;
}

fn recent_restart(path: &Path, now_ms: i64) -> bool {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<SchedulerRestartMarker>(&bytes).ok())
        .is_some_and(|marker| {
            marker.version == 1
                && now_ms.saturating_sub(marker.last_restart_at_ms) < RESTART_GUARD_MS
        })
}

fn write_marker(path: &Path, now_ms: i64, error_code: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "scheduler marker path has no parent".to_owned())?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let marker = SchedulerRestartMarker {
        version: 1,
        last_restart_at_ms: now_ms,
        error_code: error_code.to_owned(),
    };
    let encoded = serde_json::to_vec(&marker).map_err(|error| error.to_string())?;
    std::fs::write(path, encoded).map_err(|error| error.to_string())
}

fn now_ms() -> i64 {
    i64::try_from(epoch_millis()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_marker_enforces_ten_minute_guard() {
        let fixture = tempfile::tempdir().expect("fixture");
        let marker = fixture.path().join("scheduler-restart.json");
        write_marker(&marker, 1_000_000, "scheduler_storage_unavailable").expect("marker");
        assert!(recent_restart(&marker, 1_000_000 + RESTART_GUARD_MS - 1));
        assert!(!recent_restart(&marker, 1_000_000 + RESTART_GUARD_MS));
    }
}
