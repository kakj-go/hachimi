use std::time::Duration;

use hachimi_protocol::ChannelProviderHealth;

use crate::{CommandError, DesktopState};

pub(super) async fn wait_for_gateway_ready(state: &DesktopState) -> Result<(), CommandError> {
    for _ in 0..40 {
        match state.gateway.health().await {
            Ok(health) if health.running => return Ok(()),
            Ok(_) => tokio::time::sleep(Duration::from_millis(250)).await,
            Err(error) => {
                tracing::warn!(%error, "Gateway readiness lookup failed");
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
    }
    Err(CommandError::new(
        "integration_gateway_unavailable",
        "The local messaging service could not start. Retry after Hachimi finishes initializing.",
    ))
}

pub(super) async fn persisted_provider_health(
    state: &DesktopState,
) -> Result<Vec<ChannelProviderHealth>, CommandError> {
    state
        .gateway
        .persisted_provider_health()
        .await
        .map_err(|error| {
            tracing::warn!(%error, "Gateway provider health lookup failed");
            CommandError::new(
                "integration_runtime_health_unavailable",
                "The messaging connection status is temporarily unavailable.",
            )
        })
}
