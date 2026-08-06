use std::{sync::Arc, time::Duration};

use hachimi_protocol::{RuntimeComponentId, RuntimeComponentState, VerifiedChannelMessage};
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{DesktopState, epoch_millis, runtime_supervisor::RuntimeSupervisor};

pub(super) fn start_gateway_runtime(
    app: AppHandle,
    enabled: bool,
    token: String,
    supervisor: RuntimeSupervisor,
) {
    if !enabled {
        supervisor.update(
            RuntimeComponentId::Gateway,
            RuntimeComponentState::Degraded,
            Some("gateway_disabled"),
            false,
            0,
            None,
        );
        return;
    }
    let wake = Arc::new(tokio::sync::Notify::new());
    tauri::async_runtime::spawn(run_wake_listener(Arc::clone(&wake), token));
    tauri::async_runtime::spawn(async move {
        let mut next_reconciliation = tokio::time::Instant::now();
        let mut next_process_check = tokio::time::Instant::now();
        let mut next_spawn = tokio::time::Instant::now();
        let mut child: Option<std::process::Child> = None;
        let mut launch_started = None;
        let mut attempt = 0_u32;
        let retry = supervisor.retry_signal(RuntimeComponentId::Gateway);
        let shutdown = supervisor.shutdown_token();
        let executable = std::env::current_exe();
        let log_path = app
            .state::<DesktopState>()
            .storage_layout
            .logs()
            .join("gateway.log");
        loop {
            let gateway = app.state::<DesktopState>().gateway.clone();
            let now = now_ms();
            if tokio::time::Instant::now() >= next_process_check {
                let exited = child
                    .as_mut()
                    .and_then(|process| process.try_wait().transpose())
                    .transpose();
                match exited {
                    Ok(Some(status)) => {
                        child = None;
                        launch_started = None;
                        attempt = attempt.saturating_add(1);
                        let code = if status.success() {
                            "gateway_stopped"
                        } else {
                            "gateway_process_exited"
                        };
                        let _ = gateway.record_runtime_failure(attempt, code, now).await;
                        let delay = restart_delay(attempt);
                        next_spawn = tokio::time::Instant::now() + delay;
                        supervisor.update(
                            RuntimeComponentId::Gateway,
                            RuntimeComponentState::Retrying,
                            Some(code),
                            true,
                            attempt,
                            Some(now.saturating_add(to_millis(delay))),
                        );
                    }
                    Ok(None) => {}
                    Err(error) => tracing::warn!(%error, "Gateway process status lookup failed"),
                }
                match gateway.health().await {
                    Ok(health) if health.running => {
                        attempt = 0;
                        launch_started = None;
                        supervisor.ready(RuntimeComponentId::Gateway);
                    }
                    Ok(_) => {
                        if child.is_some()
                            && launch_started.is_some_and(|started: tokio::time::Instant| {
                                started.elapsed() >= Duration::from_secs(10)
                            })
                        {
                            if let Some(mut process) = child.take() {
                                let _ = process.kill();
                                let _ = process.wait();
                            }
                            launch_started = None;
                            attempt = attempt.saturating_add(1);
                            let code = "gateway_ready_timeout";
                            let _ = gateway.record_runtime_failure(attempt, code, now).await;
                            let delay = restart_delay(attempt);
                            next_spawn = tokio::time::Instant::now() + delay;
                            supervisor.update(
                                RuntimeComponentId::Gateway,
                                RuntimeComponentState::Retrying,
                                Some(code),
                                true,
                                attempt,
                                Some(now.saturating_add(to_millis(delay))),
                            );
                        } else if child.is_none() && tokio::time::Instant::now() >= next_spawn {
                            attempt = attempt.saturating_add(1);
                            match executable.as_ref() {
                                Ok(executable) => {
                                    match crate::gateway_process::spawn(executable, &log_path) {
                                        Ok(process) => {
                                            child = Some(process);
                                            launch_started = Some(tokio::time::Instant::now());
                                            let _ =
                                                gateway.record_runtime_start(attempt, now).await;
                                            supervisor.update(
                                                RuntimeComponentId::Gateway,
                                                RuntimeComponentState::Starting,
                                                None,
                                                false,
                                                attempt,
                                                None,
                                            );
                                        }
                                        Err(error) => {
                                            let code =
                                                if error.kind() == std::io::ErrorKind::AddrInUse {
                                                    "gateway_port_in_use"
                                                } else {
                                                    "gateway_process_start_failed"
                                                };
                                            tracing::warn!(%error, code, "Gateway process start failed");
                                            let _ = gateway
                                                .record_runtime_failure(attempt, code, now)
                                                .await;
                                            let delay = restart_delay(attempt);
                                            next_spawn = tokio::time::Instant::now() + delay;
                                            supervisor.update(
                                                RuntimeComponentId::Gateway,
                                                RuntimeComponentState::Retrying,
                                                Some(code),
                                                true,
                                                attempt,
                                                Some(now.saturating_add(to_millis(delay))),
                                            );
                                        }
                                    }
                                }
                                Err(error) => {
                                    tracing::warn!(%error, "Gateway executable lookup failed");
                                    supervisor.update(
                                        RuntimeComponentId::Gateway,
                                        RuntimeComponentState::Failed,
                                        Some("gateway_executable_lookup_failed"),
                                        false,
                                        attempt,
                                        None,
                                    );
                                }
                            }
                        }
                    }
                    Err(error) => tracing::warn!(%error, "Gateway health lookup failed"),
                }
                next_process_check = tokio::time::Instant::now() + Duration::from_secs(1);
            }
            if tokio::time::Instant::now() >= next_reconciliation {
                if let Err(error) = gateway.reconcile_startup(now).await {
                    tracing::warn!(%error, "Gateway reconciliation failed");
                }
                next_reconciliation = tokio::time::Instant::now() + Duration::from_secs(30);
            }
            match gateway.claim_next_ingress(now).await {
                Ok(Some(envelope)) => {
                    if let Err(error) = dispatch_claimed_ingress(&app, &envelope).await {
                        tracing::warn!(message_id = %envelope.message_id(), %error, "Gateway ingress needs attention");
                        let _ = gateway
                            .fail_ingress(&envelope.event_key, "agent_dispatch_failed", now_ms())
                            .await;
                    }
                }
                Ok(None) => {}
                Err(error) => tracing::warn!(%error, "Gateway ingress claim failed"),
            }
            match gateway.process_next_provider_delivery(now_ms()).await {
                Ok(Some(_)) | Ok(None) => {}
                Err(error) => tracing::warn!(%error, "Gateway delivery claim failed"),
            }
            tokio::select! {
                () = shutdown.cancelled() => break,
                () = tokio::time::sleep(Duration::from_millis(250)) => {}
                () = wake.notified() => {}
                () = retry.notified() => {
                    if let Some(mut process) = child.take() {
                        let _ = process.kill();
                        let _ = process.wait();
                    }
                    launch_started = None;
                    attempt = 0;
                    next_spawn = tokio::time::Instant::now();
                    let _ = gateway.clear_runtime(now_ms()).await;
                }
            }
        }
        if let Some(mut process) = child {
            let _ = process.kill();
            let _ = process.wait();
        }
    });
}

async fn run_wake_listener(wake: Arc<tokio::sync::Notify>, token: String) {
    let listener =
        match tokio::net::TcpListener::bind(crate::gateway_process::desktop_wake_address()).await {
            Ok(listener) => listener,
            Err(error) => {
                tracing::warn!(%error, "Gateway wake IPC is unavailable");
                return;
            }
        };
    loop {
        let Ok((mut stream, peer)) = listener.accept().await else {
            continue;
        };
        if !peer.ip().is_loopback() {
            continue;
        }
        let wake = Arc::clone(&wake);
        let token = token.clone();
        tokio::spawn(async move {
            let mut bytes = Vec::with_capacity(1024);
            loop {
                if bytes.len() >= 8 * 1024 {
                    return;
                }
                let mut chunk = [0_u8; 1024];
                let Ok(read) = stream.read(&mut chunk).await else {
                    return;
                };
                if read == 0 {
                    return;
                }
                bytes.extend_from_slice(&chunk[..read]);
                if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let Ok(header) = std::str::from_utf8(&bytes) else {
                return;
            };
            let authorized = header.starts_with("POST /v1/gateway/wake HTTP/1.1\r\n")
                && header
                    .lines()
                    .any(|line| line == format!("Authorization: Bearer {token}"));
            let (status, reason) = if authorized {
                wake.notify_one();
                (202, "Accepted")
            } else {
                (401, "Unauthorized")
            };
            let _ = stream
                .write_all(
                    format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .await;
            let _ = stream.shutdown().await;
        });
    }
}

async fn dispatch_claimed_ingress(
    app: &AppHandle,
    message: &VerifiedChannelMessage,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use hachimi_control_plane::{
        AppServerContext, AppServerDomainRequest, AppServerRequest, AppServerResponse,
        ChannelAppRequest,
    };
    use hachimi_core::WindowKind;
    use hachimi_protocol::ClientContext;

    let principal = format!(
        "channel:{}:{}",
        message.address.provider_id, message.address.account_id
    );
    let client = ClientContext::for_window(WindowKind::Workbench);
    let context = AppServerContext { principal, client };
    let response = app
        .state::<DesktopState>()
        .app_server
        .dispatch(
            &context,
            AppServerRequest::Domain(Box::new(AppServerDomainRequest::Channel(
                ChannelAppRequest::DispatchIngress {
                    message: message.clone(),
                },
            ))),
        )
        .await?;
    match response {
        AppServerResponse::Domain(_) => Ok(()),
        _ => Err("channel AppServer returned an unexpected response".into()),
    }
}

fn now_ms() -> i64 {
    i64::try_from(epoch_millis()).unwrap_or(i64::MAX)
}

fn restart_delay(attempt: u32) -> Duration {
    Duration::from_secs(match attempt {
        0 | 1 => 1,
        2 => 2,
        3 => 5,
        4 => 10,
        _ => 30,
    })
}

fn to_millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}
