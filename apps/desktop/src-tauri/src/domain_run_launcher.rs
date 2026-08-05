//! Desktop bridge for App Server domain operations that create Agent Runs.
//!
//! The App Server stays transport-neutral; this adapter supplies the desktop
//! event/runtime host without requiring a WebView window to be present.

use tauri::{AppHandle, Manager};

use crate::{
    CommandError, DesktopState,
    app_domain_handler::{DesktopDomainLaunchFuture, DesktopDomainRunLauncher},
    review_commands::start_review_inner,
    scheduler_commands::continue_task_interactively_inner,
};

#[derive(Clone)]
pub(super) struct DesktopDomainRunLauncherAdapter {
    app: AppHandle,
}

impl DesktopDomainRunLauncherAdapter {
    #[must_use]
    pub(super) fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl DesktopDomainRunLauncher for DesktopDomainRunLauncherAdapter {
    fn start_review(
        &self,
        client: hachimi_protocol::ClientContext,
        request: hachimi_protocol::ReviewStartRequest,
    ) -> DesktopDomainLaunchFuture<hachimi_protocol::ReviewStartSnapshot> {
        let app = self.app.clone();
        Box::pin(async move {
            let run_app = app.clone();
            let state = app.state::<DesktopState>();
            start_review_inner(run_app, &state, client, request)
                .await
                .map_err(domain_error)
        })
    }

    fn continue_task(
        &self,
        client: hachimi_protocol::ClientContext,
        task_run_id: hachimi_protocol::TaskRunId,
        idempotency_key: String,
    ) -> DesktopDomainLaunchFuture<hachimi_protocol::TaskInteractiveContinuation> {
        let app = self.app.clone();
        Box::pin(async move {
            let run_app = app.clone();
            let state = app.state::<DesktopState>();
            continue_task_interactively_inner(run_app, &state, client, task_run_id, idempotency_key)
                .await
                .map_err(domain_error)
        })
    }

    fn dispatch_channel_ingress(
        &self,
        principal: String,
        message: hachimi_protocol::VerifiedChannelMessage,
    ) -> DesktopDomainLaunchFuture<hachimi_protocol::IngressReceipt> {
        let app = self.app.clone();
        Box::pin(async move {
            let state = app.state::<DesktopState>();
            crate::channel_agent_dispatch::process_ingress(
                &app,
                &state.gateway,
                &principal,
                &message,
            )
            .await
            .map_err(|error| {
                hachimi_control_plane::AppServerDomainError::new(
                    "channel_agent_dispatch_failed",
                    error.to_string(),
                )
            })
        })
    }
}

fn domain_error(error: CommandError) -> hachimi_control_plane::AppServerDomainError {
    hachimi_control_plane::AppServerDomainError::new(error.code, error.message)
}
