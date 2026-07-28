//! Embedded, transport-neutral control plane for Phase 0.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use hachimi_approvals::{ApprovalBroker, NonInteractiveApproval};
use hachimi_audit::{AuditEvent, AuditSink, NoopAudit};
use hachimi_capabilities::CapabilityRegistry;
use hachimi_core::FeatureFlags;
use hachimi_policy::{DefaultPolicy, PolicyContext, PolicyDecision, PolicyEngine};
use hachimi_protocol::{
    CONTROL_PROTOCOL_VERSION, ClientContext, ControlError, ControlErrorCode, ControlMethod,
    ControlRequest, ControlResponse,
};
use serde_json::{Value, json};

mod agent_lifecycle;
mod app_server;
mod app_server_domain;
mod mcp_service;

pub use agent_lifecycle::{AgentLifecycleError, AgentLifecycleService};
pub use app_server::{
    AppServer, AppServerContext, AppServerError, AppServerRequest, AppServerResponse,
};
pub use app_server_domain::*;
pub use mcp_service::{
    EmptyMcpSecretResolver, McpControlService, McpControlServiceError, McpReadyRuntime,
    McpSecretResolver, mcp_host_identity_hash,
};

#[derive(Debug, Clone)]
pub struct PersistentControlAuditSink {
    store: hachimi_storage::AgentStore,
}

impl PersistentControlAuditSink {
    #[must_use]
    pub const fn new(store: hachimi_storage::AgentStore) -> Self {
        Self { store }
    }
}

impl AuditSink for PersistentControlAuditSink {
    fn record(&self, event: AuditEvent) {
        let store = self.store.clone();
        let principal = event
            .principal
            .unwrap_or_else(|| "service:control-plane".to_owned());
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = store
                    .append_audit_metadata(hachimi_storage::AuditMetadataRecord {
                        principal,
                        session_id: None,
                        run_id: None,
                        run_generation: None,
                        operation: event.operation.to_owned(),
                        target_summary: "control_method".into(),
                        decision: event.outcome.to_owned(),
                        result_code: event.outcome.to_owned(),
                        created_at_ms: unix_time_ms(),
                    })
                    .await;
            });
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationError {
    pub code: ControlErrorCode,
    pub message: String,
}

pub trait InProcessTransport: Send + Sync {
    fn dispatch(
        &self,
        client: &ClientContext,
        request: ControlRequest<Value>,
    ) -> ControlResponse<Value>;
}

pub struct ControlPlane {
    feature_flags: FeatureFlags,
    policy: Arc<dyn PolicyEngine>,
    approvals: Arc<dyn ApprovalBroker>,
    capabilities: Arc<CapabilityRegistry>,
    audit: Arc<dyn AuditSink>,
    event_sequence: AtomicU64,
}

impl std::fmt::Debug for ControlPlane {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ControlPlane")
            .field("feature_flags", &self.feature_flags)
            .field(
                "event_sequence",
                &self.event_sequence.load(Ordering::Relaxed),
            )
            .finish_non_exhaustive()
    }
}

impl Default for ControlPlane {
    fn default() -> Self {
        Self::new(FeatureFlags::all_disabled())
    }
}

impl ControlPlane {
    #[must_use]
    pub fn new(feature_flags: FeatureFlags) -> Self {
        Self::with_audit(feature_flags, Arc::new(NoopAudit))
    }

    #[must_use]
    pub fn with_audit(feature_flags: FeatureFlags, audit: Arc<dyn AuditSink>) -> Self {
        Self {
            feature_flags,
            policy: Arc::new(DefaultPolicy),
            approvals: Arc::new(NonInteractiveApproval),
            capabilities: Arc::new(CapabilityRegistry::default()),
            audit,
            event_sequence: AtomicU64::new(0),
        }
    }

    #[must_use]
    pub const fn feature_flags(&self) -> FeatureFlags {
        self.feature_flags
    }

    #[must_use]
    pub const fn approval_broker(&self) -> &Arc<dyn ApprovalBroker> {
        &self.approvals
    }

    #[must_use]
    pub const fn capability_registry(&self) -> &Arc<CapabilityRegistry> {
        &self.capabilities
    }

    #[must_use]
    pub fn registered_tools(&self) -> Vec<String> {
        Vec::new()
    }

    #[must_use]
    pub fn has_high_permission_runtime(&self) -> bool {
        self.feature_flags.any_privileged_enabled() || !self.capabilities.is_empty()
    }

    pub fn authorize(
        &self,
        client: &ClientContext,
        method: ControlMethod,
    ) -> Result<(), AuthorizationError> {
        let required_scope = method.required_scope(client.window_kind);
        let policy_context = PolicyContext::control(client, method, required_scope);
        let decision = self.policy.evaluate(&policy_context);
        if let PolicyDecision::Deny { code } = decision {
            self.audit.record(
                AuditEvent::decision(method.as_str(), "denied")
                    .with_principal(client.client_id.0.clone()),
            );
            return Err(AuthorizationError {
                code: ControlErrorCode::PermissionDenied,
                message: format!("authorization denied: {code}"),
            });
        }
        if let PolicyDecision::RequireApproval { code } = decision {
            return Err(AuthorizationError {
                code: ControlErrorCode::ApprovalRequired,
                message: format!("approval required: {code}"),
            });
        }

        self.audit.record(
            AuditEvent::decision(method.as_str(), "allowed")
                .with_principal(client.client_id.0.clone()),
        );
        Ok(())
    }

    #[must_use]
    pub fn next_event_sequence(&self) -> u64 {
        self.event_sequence.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn error_response(
        request: &ControlRequest<Value>,
        code: ControlErrorCode,
        message: impl Into<String>,
    ) -> ControlResponse<Value> {
        ControlResponse {
            id: request.id.clone(),
            ok: false,
            payload: None,
            error: Some(ControlError {
                code,
                message: message.into(),
            }),
        }
    }
}

fn unix_time_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}

impl InProcessTransport for ControlPlane {
    fn dispatch(
        &self,
        client: &ClientContext,
        request: ControlRequest<Value>,
    ) -> ControlResponse<Value> {
        if request.protocol_version != CONTROL_PROTOCOL_VERSION {
            return Self::error_response(
                &request,
                ControlErrorCode::InvalidProtocolVersion,
                format!("expected protocol version {CONTROL_PROTOCOL_VERSION}"),
            );
        }

        if request.client_id != client.client_id {
            return Self::error_response(
                &request,
                ControlErrorCode::InvalidRequest,
                "request client ID does not match the authenticated client",
            );
        }

        let Some(method) = ControlMethod::parse(&request.method) else {
            return Self::error_response(
                &request,
                ControlErrorCode::UnknownMethod,
                "unknown control method",
            );
        };

        if let Err(error) = self.authorize(client, method) {
            return Self::error_response(&request, error.code, error.message);
        }

        ControlResponse {
            id: request.id,
            ok: true,
            payload: Some(json!({ "authorized": true })),
            error: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use hachimi_core::WindowKind;
    use hachimi_protocol::{ClientId, RequestId};
    use hachimi_storage::AgentStore;
    use sqlx::Row;

    use super::*;

    fn request(version: u32, method: &str) -> ControlRequest<Value> {
        ControlRequest {
            protocol_version: version,
            id: RequestId("request-1".into()),
            client_id: ClientId("window:pet".into()),
            method: method.into(),
            params: Value::Null,
            idempotency_key: None,
        }
    }

    #[test]
    fn protocol_version_mismatch_is_rejected() {
        let plane = ControlPlane::default();
        let client = ClientContext::for_window(WindowKind::Pet);
        let response = plane.dispatch(&client, request(99, "system.bootstrap"));
        assert_eq!(
            response.error.expect("error").code,
            ControlErrorCode::InvalidProtocolVersion
        );
    }

    #[test]
    fn unknown_method_is_rejected() {
        let plane = ControlPlane::default();
        let client = ClientContext::for_window(WindowKind::Pet);
        let response = plane.dispatch(
            &client,
            request(CONTROL_PROTOCOL_VERSION, "computer.run_anything"),
        );
        assert_eq!(
            response.error.expect("error").code,
            ControlErrorCode::UnknownMethod
        );
    }

    #[test]
    fn mismatched_client_id_is_rejected() {
        let plane = ControlPlane::default();
        let client = ClientContext::for_window(WindowKind::Pet);
        let mut request = request(CONTROL_PROTOCOL_VERSION, "system.bootstrap");
        request.client_id = ClientId("window:settings".into());
        let response = plane.dispatch(&client, request);
        assert_eq!(
            response.error.expect("error").code,
            ControlErrorCode::InvalidRequest
        );
    }

    #[test]
    fn pet_cannot_read_settings() {
        let plane = ControlPlane::default();
        let client = ClientContext::for_window(WindowKind::Pet);
        let response = plane.dispatch(&client, request(CONTROL_PROTOCOL_VERSION, "settings.read"));
        assert_eq!(
            response.error.expect("error").code,
            ControlErrorCode::PermissionDenied
        );
    }

    #[test]
    fn settings_cannot_use_pet_window_operations() {
        let plane = ControlPlane::default();
        let client = ClientContext::for_window(WindowKind::Settings);
        let mut request = request(CONTROL_PROTOCOL_VERSION, "window.interact");
        request.client_id = client.client_id.clone();
        let response = plane.dispatch(&client, request);
        assert_eq!(
            response.error.expect("error").code,
            ControlErrorCode::PermissionDenied
        );
    }

    #[test]
    fn future_runtimes_are_absent_when_flags_are_closed() {
        let plane = ControlPlane::default();
        assert!(plane.registered_tools().is_empty());
        assert!(!plane.has_high_permission_runtime());
    }

    #[test]
    fn workbench_ui_flag_does_not_enable_privileged_runtime() {
        let plane = ControlPlane::new(FeatureFlags {
            workbench: true,
            ..FeatureFlags::all_disabled()
        });
        assert!(plane.registered_tools().is_empty());
        assert!(!plane.has_high_permission_runtime());
    }

    #[test]
    fn workbench_can_manage_settings_but_not_pet_or_workspace_operations() {
        let plane = ControlPlane::default();
        let client = ClientContext::for_window(WindowKind::Workbench);
        let mut llm = request(CONTROL_PROTOCOL_VERSION, "llm.test");
        llm.client_id = client.client_id.clone();
        assert!(plane.dispatch(&client, llm).ok);
        let mut connectors = request(CONTROL_PROTOCOL_VERSION, "connectors.manage");
        connectors.client_id = client.client_id.clone();
        assert!(plane.dispatch(&client, connectors).ok);

        let mut pet = request(CONTROL_PROTOCOL_VERSION, "window.interact");
        pet.client_id = client.client_id.clone();
        assert_eq!(
            plane.dispatch(&client, pet).error.expect("denied").code,
            ControlErrorCode::PermissionDenied
        );
        assert!(
            !client
                .scopes
                .contains(&hachimi_protocol::Scope::WorkspaceRead)
        );
        assert!(
            !client
                .scopes
                .contains(&hachimi_protocol::Scope::WorkspaceExec)
        );
    }

    #[test]
    fn pet_can_open_workbench_but_cannot_test_llm() {
        let plane = ControlPlane::default();
        let client = ClientContext::for_window(WindowKind::Pet);
        assert!(
            plane
                .dispatch(&client, request(CONTROL_PROTOCOL_VERSION, "workbench.open"))
                .ok
        );
        assert_eq!(
            plane
                .dispatch(&client, request(CONTROL_PROTOCOL_VERSION, "llm.test"))
                .error
                .expect("denied")
                .code,
            ControlErrorCode::PermissionDenied
        );
        assert_eq!(
            plane
                .dispatch(
                    &client,
                    request(CONTROL_PROTOCOL_VERSION, "connectors.manage")
                )
                .error
                .expect("denied")
                .code,
            ControlErrorCode::PermissionDenied
        );
    }

    #[test]
    fn event_sequence_is_monotonic() {
        let plane = ControlPlane::default();
        assert_eq!(plane.next_event_sequence(), 1);
        assert_eq!(plane.next_event_sequence(), 2);
    }

    #[tokio::test]
    async fn control_audit_persists_only_authenticated_metadata() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let plane = ControlPlane::with_audit(
            FeatureFlags::all_disabled(),
            Arc::new(PersistentControlAuditSink::new(store.clone())),
        );
        let client = ClientContext::for_window(WindowKind::Workbench);
        let mut control_request = request(CONTROL_PROTOCOL_VERSION, "llm.test");
        control_request.client_id = client.client_id.clone();
        let response = plane.dispatch(&client, control_request);
        assert!(response.ok);
        let mut rows = Vec::new();
        for _ in 0..20 {
            rows = sqlx::query(
                "SELECT principal, operation, target_summary, result_code FROM audit_events",
            )
            .fetch_all(store.pool())
            .await
            .expect("audit rows");
            if !rows.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get::<String, _>("principal"), client.client_id.0);
        assert_eq!(rows[0].get::<String, _>("operation"), "llm.test");
        assert_eq!(rows[0].get::<String, _>("target_summary"), "control_method");
        assert_eq!(rows[0].get::<String, _>("result_code"), "allowed");
        let serialized = serde_json::to_string(&rows[0].get::<String, _>("target_summary"))
            .expect("metadata serialization");
        assert!(!serialized.contains("prompt"));
        assert!(!serialized.contains("secret"));
    }
}
