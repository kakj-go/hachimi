use std::{sync::Arc, time::Duration};

use hachimi_protocol::ChannelEnvelope;
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{DesktopState, epoch_millis};

pub(super) fn start_gateway_runtime(app: AppHandle, enabled: bool, token: String) {
    if !enabled {
        return;
    }
    let wake = Arc::new(tokio::sync::Notify::new());
    tauri::async_runtime::spawn(run_wake_listener(Arc::clone(&wake), token));
    tauri::async_runtime::spawn(async move {
        let mut next_provider_reload = tokio::time::Instant::now();
        let mut next_process_check = tokio::time::Instant::now();
        loop {
            let gateway = app.state::<DesktopState>().gateway.clone();
            let now = now_ms();
            if tokio::time::Instant::now() >= next_process_check {
                match gateway.health().await {
                    Ok(health) if health.startup_registered => match std::env::current_exe() {
                        Ok(executable) => {
                            if let Err(error) = tokio::task::spawn_blocking(move || {
                                crate::gateway_process::ensure_running(&executable)
                            })
                            .await
                            .unwrap_or_else(|error| Err(std::io::Error::other(error.to_string())))
                            {
                                tracing::warn!(%error, "Registered per-user Gateway recovery failed");
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%error, "Gateway executable lookup failed");
                        }
                    },
                    Ok(_) => {}
                    Err(error) => tracing::warn!(%error, "Gateway health lookup failed"),
                }
                next_process_check = tokio::time::Instant::now() + Duration::from_secs(1);
            }
            if tokio::time::Instant::now() >= next_provider_reload {
                if let Err(error) = gateway.reload_configuration().await {
                    tracing::warn!(%error, "Gateway provider configuration reload failed");
                }
                next_provider_reload = tokio::time::Instant::now() + Duration::from_secs(2);
            }
            match gateway.claim_next_ingress(now).await {
                Ok(Some(envelope)) => {
                    if let Err(error) = dispatch_claimed_ingress(&app, &envelope).await {
                        tracing::warn!(message_id = %envelope.message_id, %error, "Gateway ingress needs attention");
                        let _ = gateway
                            .fail_ingress(&envelope.message_id, "agent_dispatch_failed", now_ms())
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
                () = tokio::time::sleep(Duration::from_millis(250)) => {}
                () = wake.notified() => {}
            }
        }
    });
}

async fn run_wake_listener(wake: Arc<tokio::sync::Notify>, token: String) {
    let listener = match tokio::net::TcpListener::bind("127.0.0.1:42373").await {
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
    envelope: &ChannelEnvelope,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use hachimi_control_plane::{
        AppServerContext, AppServerDomainRequest, AppServerRequest, AppServerResponse,
        ChannelAppRequest,
    };
    use hachimi_core::WindowKind;
    use hachimi_protocol::ClientContext;

    let principal = format!(
        "channel:{}:{}",
        envelope.route.channel, envelope.route.account
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
                    envelope: envelope.clone(),
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
