use hachimi_protocol::{
    BrowserAutomationLease, BrowserAutomationLeaseId, BrowserAutomationLeaseStatus,
    BrowserAutomationPreference, BrowserAutomationSurfaceKind, BrowserSessionId, RunId, SessionId,
};

pub(super) fn select_browser_surface(
    configured: BrowserAutomationPreference,
    requested: Option<BrowserAutomationPreference>,
    embedded_healthy: bool,
    external_pairing_healthy: bool,
    scheduled: bool,
) -> Result<BrowserAutomationSurfaceKind, &'static str> {
    if scheduled {
        return if requested == Some(BrowserAutomationPreference::ExternalChrome) {
            Err("scheduled Browser runs only support the embedded surface")
        } else if !embedded_healthy {
            Err("embedded Browser runtime is unavailable for scheduled runs")
        } else {
            Ok(BrowserAutomationSurfaceKind::Embedded)
        };
    }
    if requested == Some(BrowserAutomationPreference::Embedded) {
        return embedded_healthy
            .then_some(BrowserAutomationSurfaceKind::Embedded)
            .ok_or("embedded Browser was explicitly requested but its runtime is unavailable");
    }
    if requested == Some(BrowserAutomationPreference::ExternalChrome) {
        return external_pairing_healthy
            .then_some(BrowserAutomationSurfaceKind::ExternalChrome)
            .ok_or(
                "external Chrome was explicitly requested but extension pairing is unavailable",
            );
    }
    match configured {
        BrowserAutomationPreference::ExternalChrome if external_pairing_healthy => {
            Ok(BrowserAutomationSurfaceKind::ExternalChrome)
        }
        BrowserAutomationPreference::Auto if embedded_healthy => {
            Ok(BrowserAutomationSurfaceKind::Embedded)
        }
        BrowserAutomationPreference::Auto if external_pairing_healthy => {
            Ok(BrowserAutomationSurfaceKind::ExternalChrome)
        }
        BrowserAutomationPreference::Embedded | BrowserAutomationPreference::ExternalChrome
            if embedded_healthy =>
        {
            Ok(BrowserAutomationSurfaceKind::Embedded)
        }
        BrowserAutomationPreference::Auto
        | BrowserAutomationPreference::Embedded
        | BrowserAutomationPreference::ExternalChrome => {
            Err("no healthy Browser automation surface is available")
        }
    }
}

pub(super) enum BrowserLeaseRoute {
    Embedded,
    ExternalChrome {
        lease: Box<BrowserAutomationLease>,
        browser_session_id: BrowserSessionId,
    },
}

pub(super) async fn route_browser_lease(
    store: &hachimi_storage::AgentStore,
    lease_id: &BrowserAutomationLeaseId,
    owner_session_id: &SessionId,
    owner_run_id: &RunId,
    run_generation: u64,
) -> Result<BrowserLeaseRoute, String> {
    let lease = store
        .browser_automation_lease(lease_id)
        .await
        .map_err(|error| error.to_string())?;
    if lease.owner_session_id != *owner_session_id
        || lease.owner_run_id != *owner_run_id
        || lease.run_generation != run_generation
    {
        return Err("browser automation lease ownership changed".into());
    }
    if lease.status != BrowserAutomationLeaseStatus::Active || lease.expires_at_ms <= now_ms() {
        return Err("browser automation lease is inactive or expired".into());
    }
    match lease.surface {
        BrowserAutomationSurfaceKind::Embedded => Ok(BrowserLeaseRoute::Embedded),
        BrowserAutomationSurfaceKind::ExternalChrome => {
            let browser_session_id = store
                .external_browser_session_for_lease(lease_id)
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "external Chrome lease target is unavailable".to_owned())?;
            Ok(BrowserLeaseRoute::ExternalChrome {
                lease: Box::new(lease),
                browser_session_id,
            })
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_prefers_embedded_and_explicit_embedded_wins() {
        assert_eq!(
            select_browser_surface(BrowserAutomationPreference::Auto, None, true, true, false),
            Ok(BrowserAutomationSurfaceKind::Embedded)
        );
        assert_eq!(
            select_browser_surface(
                BrowserAutomationPreference::ExternalChrome,
                Some(BrowserAutomationPreference::Embedded),
                true,
                true,
                false,
            ),
            Ok(BrowserAutomationSurfaceKind::Embedded)
        );
    }

    #[test]
    fn configured_external_follows_pairing_health_without_unsafe_failure() {
        assert_eq!(
            select_browser_surface(
                BrowserAutomationPreference::ExternalChrome,
                None,
                true,
                true,
                false,
            ),
            Ok(BrowserAutomationSurfaceKind::ExternalChrome)
        );
        assert_eq!(
            select_browser_surface(
                BrowserAutomationPreference::ExternalChrome,
                None,
                true,
                false,
                false,
            ),
            Ok(BrowserAutomationSurfaceKind::Embedded)
        );
    }

    #[test]
    fn explicit_external_never_falls_back_and_schedules_stay_embedded() {
        assert!(
            select_browser_surface(
                BrowserAutomationPreference::Auto,
                Some(BrowserAutomationPreference::ExternalChrome),
                true,
                false,
                false,
            )
            .is_err()
        );
        assert_eq!(
            select_browser_surface(
                BrowserAutomationPreference::ExternalChrome,
                None,
                true,
                true,
                true
            ),
            Ok(BrowserAutomationSurfaceKind::Embedded)
        );
        assert!(
            select_browser_surface(
                BrowserAutomationPreference::ExternalChrome,
                Some(BrowserAutomationPreference::ExternalChrome),
                true,
                true,
                true,
            )
            .is_err()
        );
    }

    #[test]
    fn auto_falls_back_to_an_authorized_external_browser_only_after_cef_degrades() {
        assert_eq!(
            select_browser_surface(BrowserAutomationPreference::Auto, None, false, true, false),
            Ok(BrowserAutomationSurfaceKind::ExternalChrome)
        );
        assert!(
            select_browser_surface(BrowserAutomationPreference::Auto, None, false, false, false)
                .is_err()
        );
    }
}
