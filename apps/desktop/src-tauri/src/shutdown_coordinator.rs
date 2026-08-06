use std::time::Duration;

use crate::DesktopState;

pub(super) async fn shutdown(state: &DesktopState) {
    if !state.runtime_supervisor.begin_shutdown() {
        return;
    }
    state.voice_runtime.stop();
    let scheduler = state.scheduler_handle.lock().take();
    let processes = state.process_registry.clone();
    let browser = state.embedded_browser.clone();
    let cleanup = async move {
        let scheduler_stop = async move {
            if let Some(handle) = scheduler {
                handle.stop().await;
            }
        };
        tokio::join!(scheduler_stop, processes.shutdown(), browser.shutdown());
    };
    if tokio::time::timeout(Duration::from_secs(5), cleanup)
        .await
        .is_err()
    {
        tracing::warn!(
            "Runtime shutdown exceeded five seconds; startup reconciliation will recover unfinished state"
        );
    }
}
