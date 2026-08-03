use hachimi_protocol::{
    BrowserAutomationLease, BrowserAutomationLeaseId, BrowserAutomationLeaseStatus,
    BrowserAutomationSurfaceKind, BrowserSessionId, RunId, SessionId,
};

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
