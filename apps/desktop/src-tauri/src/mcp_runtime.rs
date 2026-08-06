use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hachimi_control_plane::McpControlService;
use hachimi_protocol::{McpServerHealthState, RuntimeComponentId, RuntimeComponentState};

use crate::runtime_supervisor::RuntimeSupervisor;

pub(super) fn start_mcp_runtime(
    control: McpControlService,
    supervisor: RuntimeSupervisor,
    enabled: bool,
) {
    if !enabled {
        supervisor.update(
            RuntimeComponentId::Mcp,
            RuntimeComponentState::Degraded,
            Some("mcp_runtime_disabled"),
            false,
            0,
            None,
        );
        return;
    }
    tauri::async_runtime::spawn(async move {
        let retry = supervisor.retry_signal(RuntimeComponentId::Mcp);
        let shutdown = supervisor.shutdown_token();
        match control.reconcile_startup().await {
            Ok(_) => publish_health(&control, &supervisor).await,
            Err(error) => {
                tracing::warn!(%error, "MCP startup reconciliation failed");
                publish_reconcile_failure(&supervisor);
            }
        }
        loop {
            let manual = tokio::select! {
                () = shutdown.cancelled() => break,
                () = tokio::time::sleep(Duration::from_secs(1)) => false,
                () = retry.notified() => true,
            };
            let result = if manual {
                control.retry_failed_now().await
            } else {
                control.retry_due(now_ms()).await
            };
            match result {
                Ok(report) if !report.views.is_empty() => {
                    publish_health(&control, &supervisor).await;
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(%error, "MCP runtime retry failed");
                    publish_reconcile_failure(&supervisor);
                }
            }
        }
    });
}

async fn publish_health(control: &McpControlService, supervisor: &RuntimeSupervisor) {
    let Ok(servers) = control.list().await else {
        publish_reconcile_failure(supervisor);
        return;
    };
    let failed = servers
        .iter()
        .filter(|server| {
            server.configuration.enabled && server.health.state == McpServerHealthState::Failed
        })
        .collect::<Vec<_>>();
    if failed.is_empty() {
        supervisor.ready(RuntimeComponentId::Mcp);
        return;
    }
    let attempt = failed
        .iter()
        .map(|server| server.health.failure_count)
        .max()
        .unwrap_or_default();
    let next_retry = failed
        .iter()
        .filter_map(|server| server.health.next_retry_at_ms)
        .min();
    supervisor.update(
        RuntimeComponentId::Mcp,
        RuntimeComponentState::Retrying,
        Some("mcp_server_unavailable"),
        true,
        attempt,
        next_retry,
    );
}

fn publish_reconcile_failure(supervisor: &RuntimeSupervisor) {
    supervisor.update(
        RuntimeComponentId::Mcp,
        RuntimeComponentState::Retrying,
        Some("mcp_reconcile_failed"),
        true,
        1,
        Some(now_ms().saturating_add(60_000)),
    );
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(i64::MAX)
}
