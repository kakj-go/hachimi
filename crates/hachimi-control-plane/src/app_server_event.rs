//! Authenticated, typed local adapters for Scheduler Event ingress.
//!
//! Adapters select the source kind in Host code. The event payload deliberately
//! has no principal field; the domain handler binds the authenticated
//! [`AppServerContext::principal`] before matching or persistence.

use std::collections::BTreeMap;

use hachimi_protocol::{
    MutationContext, ScheduleEventIngressRequest, ScheduleEventReceipt, ScheduleEventResourceRef,
    ScheduleEventSourceKind,
};

use crate::{
    AppServer, AppServerContext, AppServerDomainRequest, AppServerDomainResponse, AppServerError,
    AppServerRequest, AppServerResponse, ScheduleAppRequest, ScheduleAppResponse,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalScheduleEvent {
    pub context: MutationContext,
    pub source_id: String,
    pub event_id: String,
    pub event_type: String,
    pub subject: Option<String>,
    pub labels: BTreeMap<String, String>,
    pub resource: Option<ScheduleEventResourceRef>,
    pub occurred_at_ms: i64,
}

impl From<ScheduleEventIngressRequest> for LocalScheduleEvent {
    fn from(request: ScheduleEventIngressRequest) -> Self {
        Self {
            context: request.context,
            source_id: request.source_id,
            event_id: request.event_id,
            event_type: request.event_type,
            subject: request.subject,
            labels: request.labels,
            resource: request.resource,
            occurred_at_ms: request.occurred_at_ms,
        }
    }
}

impl AppServer {
    pub async fn ingest_workspace_event(
        &self,
        context: &AppServerContext,
        event: LocalScheduleEvent,
    ) -> Result<ScheduleEventReceipt, AppServerError> {
        self.ingest_local_event(context, ScheduleEventSourceKind::Workspace, event)
            .await
    }

    pub async fn ingest_plugin_event(
        &self,
        context: &AppServerContext,
        event: LocalScheduleEvent,
    ) -> Result<ScheduleEventReceipt, AppServerError> {
        self.ingest_local_event(context, ScheduleEventSourceKind::Plugin, event)
            .await
    }

    pub async fn ingest_connector_event(
        &self,
        context: &AppServerContext,
        event: LocalScheduleEvent,
    ) -> Result<ScheduleEventReceipt, AppServerError> {
        self.ingest_local_event(context, ScheduleEventSourceKind::Connector, event)
            .await
    }

    pub async fn ingest_channel_event(
        &self,
        context: &AppServerContext,
        event: LocalScheduleEvent,
    ) -> Result<ScheduleEventReceipt, AppServerError> {
        self.ingest_local_event(context, ScheduleEventSourceKind::Channel, event)
            .await
    }

    pub async fn ingest_gateway_event(
        &self,
        context: &AppServerContext,
        event: LocalScheduleEvent,
    ) -> Result<ScheduleEventReceipt, AppServerError> {
        self.ingest_local_event(context, ScheduleEventSourceKind::Gateway, event)
            .await
    }

    async fn ingest_local_event(
        &self,
        context: &AppServerContext,
        source_kind: ScheduleEventSourceKind,
        event: LocalScheduleEvent,
    ) -> Result<ScheduleEventReceipt, AppServerError> {
        let response = self
            .dispatch(
                context,
                AppServerRequest::Domain(Box::new(AppServerDomainRequest::Schedule(Box::new(
                    ScheduleAppRequest::IngestEvent(ScheduleEventIngressRequest {
                        context: event.context,
                        source_kind,
                        source_id: event.source_id,
                        event_id: event.event_id,
                        event_type: event.event_type,
                        subject: event.subject,
                        labels: event.labels,
                        resource: event.resource,
                        occurred_at_ms: event.occurred_at_ms,
                    }),
                )))),
            )
            .await?;
        match response {
            AppServerResponse::Domain(response)
                if matches!(response.as_ref(), AppServerDomainResponse::Schedule(_)) =>
            {
                match *response {
                    AppServerDomainResponse::Schedule(response) => match *response {
                        ScheduleAppResponse::EventReceipt(receipt) => Ok(receipt),
                        _ => Err(local_event_response_mismatch()),
                    },
                    _ => unreachable!("guarded AppServer response variant"),
                }
            }
            _ => Err(local_event_response_mismatch()),
        }
    }
}

fn local_event_response_mismatch() -> AppServerError {
    AppServerError::Domain {
        code: "schedule_event_response_mismatch".into(),
        message: "the local Event adapter received an unexpected AppServer response".into(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use hachimi_core::{FeatureFlags, WindowKind};
    use hachimi_protocol::{
        ClientContext, ClientId, RequestId, ScheduleEventContext, ScheduleEventReceiptStatus,
        ScheduleEventSource,
    };
    use hachimi_storage::AgentStore;

    use super::*;
    use crate::{
        AgentLifecycleService, AppServerDomainError, AppServerDomainHandler, ControlPlane,
        DomainFuture,
    };

    #[derive(Debug, Default)]
    struct CapturingEventDomains {
        seen: Mutex<Vec<(ScheduleEventSourceKind, String, String)>>,
    }

    impl AppServerDomainHandler for CapturingEventDomains {
        fn dispatch<'a>(
            &'a self,
            context: &'a AppServerContext,
            request: AppServerDomainRequest,
        ) -> DomainFuture<'a> {
            Box::pin(async move {
                let AppServerDomainRequest::Schedule(request) = request else {
                    return Err(AppServerDomainError::new(
                        "unexpected_test_request",
                        "expected a Schedule Event request",
                    ));
                };
                let ScheduleAppRequest::IngestEvent(request) = *request else {
                    return Err(AppServerDomainError::new(
                        "unexpected_test_request",
                        "expected Event ingress",
                    ));
                };
                self.seen.lock().expect("seen lock").push((
                    request.source_kind,
                    context.principal.clone(),
                    request.source_id.clone(),
                ));
                Ok(AppServerDomainResponse::Schedule(Box::new(
                    ScheduleAppResponse::EventReceipt(ScheduleEventReceipt {
                        status: ScheduleEventReceiptStatus::Accepted,
                        event: ScheduleEventContext {
                            event_id: request.event_id,
                            source: ScheduleEventSource {
                                kind: request.source_kind,
                                principal: context.principal.clone(),
                                id: request.source_id,
                            },
                            event_type: request.event_type,
                            subject: request.subject,
                            labels: request.labels,
                            resource: request.resource,
                            fingerprint: "test-fingerprint".into(),
                            occurred_at_ms: request.occurred_at_ms,
                            received_at_ms: request.occurred_at_ms + 1,
                        },
                        matched_schedule_ids: Vec::new(),
                        task_runs: Vec::new(),
                    }),
                )))
            })
        }
    }

    fn local_event(client_id: &ClientId, source_id: &str) -> LocalScheduleEvent {
        LocalScheduleEvent {
            context: MutationContext {
                request_id: RequestId(format!("request-{source_id}")),
                client_id: client_id.clone(),
                protocol_version: hachimi_protocol::CONTROL_PROTOCOL_VERSION,
                idempotency_key: format!("event-{source_id}"),
                expected_run_id: None,
                expected_generation: None,
            },
            source_id: source_id.into(),
            event_id: format!("event-{source_id}"),
            event_type: "resource.changed".into(),
            subject: Some("resource://fixture".into()),
            labels: BTreeMap::new(),
            resource: None,
            occurred_at_ms: 1_800_000_000_000,
        }
    }

    #[tokio::test]
    async fn all_local_adapters_share_app_server_ingress_and_host_principal() {
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
        let domains = Arc::new(CapturingEventDomains::default());
        let server = AppServer::new(Arc::new(ControlPlane::default()), lifecycle)
            .with_domain_handler(domains.clone());
        let context = AppServerContext {
            client: ClientContext::for_window(WindowKind::Workbench),
            principal: "host:authenticated-user".into(),
        };
        let client_id = context.client.client_id.clone();

        server
            .ingest_workspace_event(&context, local_event(&client_id, "workspace"))
            .await
            .expect("workspace event");
        server
            .ingest_plugin_event(&context, local_event(&client_id, "plugin"))
            .await
            .expect("plugin event");
        server
            .ingest_connector_event(&context, local_event(&client_id, "connector"))
            .await
            .expect("connector event");
        server
            .ingest_channel_event(&context, local_event(&client_id, "channel"))
            .await
            .expect("channel event");
        server
            .ingest_gateway_event(&context, local_event(&client_id, "gateway"))
            .await
            .expect("gateway event");

        assert_eq!(
            *domains.seen.lock().expect("seen lock"),
            vec![
                (
                    ScheduleEventSourceKind::Workspace,
                    "host:authenticated-user".into(),
                    "workspace".into(),
                ),
                (
                    ScheduleEventSourceKind::Plugin,
                    "host:authenticated-user".into(),
                    "plugin".into(),
                ),
                (
                    ScheduleEventSourceKind::Connector,
                    "host:authenticated-user".into(),
                    "connector".into(),
                ),
                (
                    ScheduleEventSourceKind::Channel,
                    "host:authenticated-user".into(),
                    "channel".into(),
                ),
                (
                    ScheduleEventSourceKind::Gateway,
                    "host:authenticated-user".into(),
                    "gateway".into(),
                ),
            ]
        );
    }
}
