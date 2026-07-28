// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/app-server/src/{message_processor,
// request_processors,thread_state,connection_cleanup,in_process}.rs
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// Modified for Hachimi: typed in-process requests, authenticated principals,
// Session/Run vocabulary, and Tauri/Scheduler-neutral lifecycle dispatch.

//! Transport-neutral asynchronous App Server façade.
//!
//! Tauri, the scheduler, and future local transports use this typed boundary
//! for Agent lifecycle operations. It owns authentication and delegates state
//! transitions to the lifecycle service; it never constructs a model loop or a
//! tool registry.

use std::sync::Arc;

use hachimi_approvals::ApprovalBroker;
use hachimi_protocol::{
    ApprovalRequestRecord, ApprovalResolution, ClientContext, ClientId, ControlInitializeRequest,
    ControlInitializeResponse, EventSubscriptionId, EventSubscriptionRequest,
    EventSubscriptionSnapshot, RunControlRequest, RunRecord, RunSteerRecord, SessionForkRequest,
    SessionMetadataUpdateRequest, SessionPage, SessionRecord, SessionResumeRequest,
    SessionResumeSnapshot, SessionSearchRequest, UserInputRequestRecord, UserInputResolution,
};
use hachimi_user_input::UserInputBroker;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{AgentLifecycleError, AgentLifecycleService, ControlPlane};
use crate::{AppServerDomainHandler, AppServerDomainRequest, AppServerDomainResponse};

#[derive(Debug, Error)]
pub enum AppServerError {
    #[error("app server authorization failed: {code:?}: {message}")]
    Authorization {
        code: hachimi_protocol::ControlErrorCode,
        message: String,
    },
    #[error("app server lifecycle failed: {0}")]
    Lifecycle(#[from] AgentLifecycleError),
    #[error("app server request is not supported by this transport")]
    Unsupported,
    #[error("app server broker failed: {0}")]
    Broker(String),
    #[error("app server domain failed: {code}: {message}")]
    Domain { code: String, message: String },
}

#[derive(Debug, Clone)]
pub struct AppServerContext {
    pub client: ClientContext,
    pub principal: String,
}

#[derive(Debug)]
pub enum AppServerRequest {
    Initialize(ControlInitializeRequest),
    SearchSessions(SessionSearchRequest),
    ResumeSession(SessionResumeRequest),
    ForkSession(SessionForkRequest),
    UpdateSession(SessionMetadataUpdateRequest),
    SteerRun(RunControlRequest),
    PrepareInterrupt(RunControlRequest),
    ResolveApproval(ApprovalResolution),
    ResolveUserInput(UserInputResolution),
    SubscribeEvents(EventSubscriptionRequest),
    UnsubscribeEvents(EventSubscriptionId),
    Domain(Box<AppServerDomainRequest>),
}

#[derive(Debug)]
pub enum AppServerResponse {
    Initialized(ControlInitializeResponse),
    Sessions(SessionPage),
    Resumed(Box<SessionResumeSnapshot>),
    Session(SessionRecord),
    Steer(RunSteerRecord),
    Interrupted(Box<RunRecord>),
    Approval(ApprovalRequestRecord),
    UserInput(UserInputRequestRecord),
    Subscription(EventSubscriptionSnapshot),
    Unsubscribed(bool),
    Domain(Box<AppServerDomainResponse>),
}

#[derive(Clone)]
pub struct AppServer {
    control_plane: Arc<ControlPlane>,
    lifecycle: AgentLifecycleService,
    approvals: Option<Arc<dyn ApprovalBroker>>,
    user_inputs: Option<Arc<dyn UserInputBroker>>,
    domains: Option<Arc<dyn AppServerDomainHandler>>,
}

impl std::fmt::Debug for AppServer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppServer")
            .field("lifecycle", &self.lifecycle)
            .finish_non_exhaustive()
    }
}

impl AppServer {
    #[must_use]
    pub fn new(control_plane: Arc<ControlPlane>, lifecycle: AgentLifecycleService) -> Self {
        Self {
            control_plane,
            lifecycle,
            approvals: None,
            user_inputs: None,
            domains: None,
        }
    }

    #[must_use]
    pub fn with_domain_handler(mut self, handler: Arc<dyn AppServerDomainHandler>) -> Self {
        self.domains = Some(handler);
        self
    }

    #[must_use]
    pub fn with_brokers(
        mut self,
        approvals: Arc<dyn ApprovalBroker>,
        user_inputs: Arc<dyn UserInputBroker>,
    ) -> Self {
        self.approvals = Some(approvals);
        self.user_inputs = Some(user_inputs);
        self
    }

    #[must_use]
    pub const fn lifecycle(&self) -> &AgentLifecycleService {
        &self.lifecycle
    }

    pub async fn dispatch(
        &self,
        context: &AppServerContext,
        request: AppServerRequest,
    ) -> Result<AppServerResponse, AppServerError> {
        let method = match &request {
            AppServerRequest::Domain(request) => request.control_method(),
            _ => hachimi_protocol::ControlMethod::WorkbenchWindow,
        };
        self.authorize(context, method)?;
        let response = match request {
            AppServerRequest::Initialize(request) => {
                AppServerResponse::Initialized(self.lifecycle.initialize(&request)?)
            }
            AppServerRequest::SearchSessions(request) => {
                AppServerResponse::Sessions(self.lifecycle.search_sessions(&request).await?)
            }
            AppServerRequest::ResumeSession(request) => {
                AppServerResponse::Resumed(Box::new(self.lifecycle.resume_session(&request).await?))
            }
            AppServerRequest::ForkSession(request) => AppServerResponse::Session(
                self.lifecycle
                    .fork_session(&context.client.client_id, &context.principal, &request)
                    .await?,
            ),
            AppServerRequest::UpdateSession(request) => AppServerResponse::Session(
                self.lifecycle
                    .update_session_metadata(&context.client.client_id, &request)
                    .await?,
            ),
            AppServerRequest::SteerRun(request) => AppServerResponse::Steer(
                self.lifecycle
                    .steer_run(&context.client.client_id, &request)
                    .await?,
            ),
            AppServerRequest::PrepareInterrupt(request) => {
                AppServerResponse::Interrupted(Box::new(
                    self.lifecycle
                        .prepare_interrupt(&context.client.client_id, &request)
                        .await?,
                ))
            }
            AppServerRequest::ResolveApproval(mut resolution) => {
                resolution.resolved_by = context.principal.clone();
                let broker = self.approvals.as_ref().ok_or(AppServerError::Unsupported)?;
                AppServerResponse::Approval(
                    broker
                        .resolve(resolution)
                        .await
                        .map_err(|error| AppServerError::Broker(error.to_string()))?,
                )
            }
            AppServerRequest::ResolveUserInput(mut resolution) => {
                resolution.resolved_by = context.principal.clone();
                let broker = self
                    .user_inputs
                    .as_ref()
                    .ok_or(AppServerError::Unsupported)?;
                AppServerResponse::UserInput(
                    broker
                        .resolve(resolution)
                        .await
                        .map_err(|error| AppServerError::Broker(error.to_string()))?,
                )
            }
            AppServerRequest::SubscribeEvents(request) => AppServerResponse::Subscription(
                self.lifecycle
                    .subscribe(context.client.client_id.clone(), &request)
                    .await?,
            ),
            AppServerRequest::UnsubscribeEvents(subscription_id) => {
                AppServerResponse::Unsubscribed(self.lifecycle.unsubscribe(&subscription_id))
            }
            AppServerRequest::Domain(request) => {
                let handler = self.domains.as_ref().ok_or(AppServerError::Unsupported)?;
                AppServerResponse::Domain(Box::new(
                    handler.dispatch(context, *request).await.map_err(|error| {
                        AppServerError::Domain {
                            code: error.code,
                            message: error.message,
                        }
                    })?,
                ))
            }
        };
        Ok(response)
    }

    pub fn open_event_stream(
        &self,
        subscription_id: &EventSubscriptionId,
        cancellation: CancellationToken,
    ) -> Result<mpsc::Receiver<EventSubscriptionSnapshot>, AppServerError> {
        Ok(self
            .lifecycle
            .open_event_stream(subscription_id, cancellation)?)
    }

    pub fn unsubscribe_client(&self, client_id: &ClientId) -> usize {
        self.lifecycle.unsubscribe_client(client_id)
    }

    fn authorize(
        &self,
        context: &AppServerContext,
        method: hachimi_protocol::ControlMethod,
    ) -> Result<(), AppServerError> {
        self.control_plane
            .authorize(&context.client, method)
            .map_err(|error| AppServerError::Authorization {
                code: error.code,
                message: error.message,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AppServerDomainError, AppServerDomainHandler, AppServerDomainRequest,
        AppServerDomainResponse, DomainFuture, FsAppRequest, FsAppResponse,
    };
    use hachimi_core::{FeatureFlags, WindowKind};
    use hachimi_protocol::{MutationContext, SessionId, SessionMetadataUpdateRequest};
    use hachimi_storage::AgentStore;

    #[tokio::test]
    async fn typed_dispatch_rejects_a_client_mismatch_before_mutation() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let lifecycle = AgentLifecycleService::new(
            store,
            FeatureFlags::all_disabled(),
            hachimi_protocol::SandboxCapabilityReport {
                backend: "test".into(),
                readiness: hachimi_protocol::SandboxReadiness::Unavailable,
                os_enforced: false,
                filesystem_enforced: false,
                process_enforced: false,
                network_enforced: false,
                version: None,
                stable_error_code: None,
                diagnostics: Vec::new(),
            },
        );
        let server = AppServer::new(Arc::new(ControlPlane::default()), lifecycle);
        let context = AppServerContext {
            client: ClientContext::for_window(WindowKind::Workbench),
            principal: "service:scheduler".into(),
        };
        let error = server
            .dispatch(
                &context,
                AppServerRequest::UpdateSession(SessionMetadataUpdateRequest {
                    context: MutationContext {
                        request_id: hachimi_protocol::RequestId("request".into()),
                        client_id: hachimi_protocol::ClientId("other-client".into()),
                        protocol_version: hachimi_protocol::CONTROL_PROTOCOL_VERSION,
                        idempotency_key: "idempotent".into(),
                        expected_run_id: None,
                        expected_generation: None,
                    },
                    session_id: SessionId::from("missing"),
                    title: Some("should not mutate".into()),
                    archived: None,
                    pinned: None,
                }),
            )
            .await
            .expect_err("missing session should be rejected");
        assert!(matches!(error, AppServerError::Lifecycle(_)));
    }

    #[derive(Debug)]
    struct TestDomains;

    impl AppServerDomainHandler for TestDomains {
        fn dispatch<'a>(
            &'a self,
            _context: &'a AppServerContext,
            request: AppServerDomainRequest,
        ) -> DomainFuture<'a> {
            Box::pin(async move {
                match request {
                    AppServerDomainRequest::Fs(FsAppRequest::SearchCancel(search_id)) => {
                        Ok(AppServerDomainResponse::Fs(FsAppResponse::SearchCancelled(
                            search_id.as_str() == "search-1",
                        )))
                    }
                    _ => Err(AppServerDomainError::new(
                        "unexpected_test_request",
                        "test handler received a different request",
                    )),
                }
            })
        }
    }

    #[tokio::test]
    async fn typed_domain_requests_route_without_a_tauri_transport() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let lifecycle = AgentLifecycleService::new(
            store,
            FeatureFlags::all_disabled(),
            hachimi_protocol::SandboxCapabilityReport {
                backend: "test".into(),
                readiness: hachimi_protocol::SandboxReadiness::Unavailable,
                os_enforced: false,
                filesystem_enforced: false,
                process_enforced: false,
                network_enforced: false,
                version: None,
                stable_error_code: None,
                diagnostics: Vec::new(),
            },
        );
        let server = AppServer::new(Arc::new(ControlPlane::default()), lifecycle)
            .with_domain_handler(Arc::new(TestDomains));
        let context = AppServerContext {
            client: ClientContext::for_window(WindowKind::Workbench),
            principal: "user:test".into(),
        };
        let response = server
            .dispatch(
                &context,
                AppServerRequest::Domain(Box::new(AppServerDomainRequest::Fs(
                    FsAppRequest::SearchCancel(hachimi_protocol::FsSearchId::from("search-1")),
                ))),
            )
            .await
            .expect("domain response");
        assert!(matches!(
            response,
            AppServerResponse::Domain(response)
                if matches!(
                    *response,
                    AppServerDomainResponse::Fs(FsAppResponse::SearchCancelled(true))
                )
        ));
    }
}
