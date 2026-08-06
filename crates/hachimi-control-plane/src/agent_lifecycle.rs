use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use hachimi_core::FeatureFlags;
use hachimi_protocol::{
    CONTROL_PROTOCOL_VERSION, ClientId, ControlInitializeRequest, ControlInitializeResponse,
    EventSubscriptionId, EventSubscriptionRecord, EventSubscriptionRequest,
    EventSubscriptionSnapshot, RunControlRequest, RunRecord, RunSteerRecord,
    SandboxCapabilityReport, SessionForkRequest, SessionId, SessionMetadataUpdateRequest,
    SessionPage, SessionRecord, SessionResumeRequest, SessionResumeSnapshot, SessionSearchRequest,
};
use hachimi_storage::{AgentStore, AgentStoreError};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Error)]
pub enum AgentLifecycleError {
    #[error("agent lifecycle storage failed: {0}")]
    Store(#[from] AgentStoreError),
    #[error("unsupported control protocol version: {0}")]
    InvalidProtocolVersion(u32),
    #[error("request client does not match the authenticated client")]
    ClientMismatch,
    #[error("event subscription does not exist")]
    SubscriptionNotFound,
    #[error("mutation request is missing an idempotency key")]
    MissingIdempotencyKey,
}

#[derive(Debug, Clone)]
pub struct AgentLifecycleService {
    store: AgentStore,
    feature_flags: FeatureFlags,
    sandbox: SandboxCapabilityReport,
    subscriptions: Arc<Mutex<BTreeMap<EventSubscriptionId, EventSubscriptionRecord>>>,
}

impl AgentLifecycleService {
    #[must_use]
    pub fn new(
        store: AgentStore,
        feature_flags: FeatureFlags,
        sandbox: SandboxCapabilityReport,
    ) -> Self {
        Self {
            store,
            feature_flags,
            sandbox,
            subscriptions: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn initialize(
        &self,
        request: &ControlInitializeRequest,
    ) -> Result<ControlInitializeResponse, AgentLifecycleError> {
        if request.protocol_version != CONTROL_PROTOCOL_VERSION {
            return Err(AgentLifecycleError::InvalidProtocolVersion(
                request.protocol_version,
            ));
        }
        let supported = request
            .supported_features
            .iter()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        let enabled_features = server_features(self.feature_flags)
            .into_iter()
            .filter(|feature| supported.contains(feature.as_str()))
            .collect::<Vec<_>>();
        let experimental_features = request
            .experimental_features
            .iter()
            .filter(|feature| enabled_features.contains(feature))
            .cloned()
            .collect();
        Ok(ControlInitializeResponse {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            enabled_features,
            experimental_features,
            warnings: Vec::new(),
            sandbox: self.sandbox.clone(),
        })
    }

    pub async fn search_sessions(
        &self,
        request: &SessionSearchRequest,
    ) -> Result<SessionPage, AgentLifecycleError> {
        Ok(self.store.search_sessions(request).await?)
    }

    pub async fn resume_session(
        &self,
        request: &SessionResumeRequest,
    ) -> Result<SessionResumeSnapshot, AgentLifecycleError> {
        Ok(self.store.resume_session(request).await?)
    }

    pub async fn fork_session(
        &self,
        authenticated_client: &ClientId,
        principal: &str,
        request: &SessionForkRequest,
    ) -> Result<SessionRecord, AgentLifecycleError> {
        validate_mutation_client(authenticated_client, &request.context.client_id)?;
        validate_idempotency(&request.context.idempotency_key)?;
        Ok(self
            .store
            .fork_session_idempotent(principal, request, SessionId::random(), now_ms())
            .await?)
    }

    pub async fn update_session_metadata(
        &self,
        authenticated_client: &ClientId,
        request: &SessionMetadataUpdateRequest,
    ) -> Result<SessionRecord, AgentLifecycleError> {
        validate_mutation_client(authenticated_client, &request.context.client_id)?;
        validate_idempotency(&request.context.idempotency_key)?;
        Ok(self
            .store
            .update_session_metadata(
                &request.session_id,
                request.title.as_deref(),
                request.archived,
                request.pinned,
                now_ms(),
            )
            .await?)
    }

    pub async fn steer_run(
        &self,
        authenticated_client: &ClientId,
        request: &RunControlRequest,
    ) -> Result<RunSteerRecord, AgentLifecycleError> {
        validate_mutation_client(authenticated_client, &request.context.client_id)?;
        validate_idempotency(&request.context.idempotency_key)?;
        let expected_run_id = request
            .context
            .expected_run_id
            .as_ref()
            .ok_or(AgentStoreError::RunPreconditionFailed)?;
        let expected_generation = request
            .context
            .expected_generation
            .ok_or(AgentStoreError::RunPreconditionFailed)?;
        Ok(self
            .store
            .enqueue_run_steer(
                &request.run_id,
                expected_run_id,
                expected_generation,
                request.input.as_deref().unwrap_or_default(),
                now_ms(),
            )
            .await?)
    }

    pub async fn prepare_interrupt(
        &self,
        authenticated_client: &ClientId,
        request: &RunControlRequest,
    ) -> Result<RunRecord, AgentLifecycleError> {
        validate_mutation_client(authenticated_client, &request.context.client_id)?;
        validate_idempotency(&request.context.idempotency_key)?;
        let expected_run_id = request
            .context
            .expected_run_id
            .as_ref()
            .ok_or(AgentStoreError::RunPreconditionFailed)?;
        let expected_generation = request
            .context
            .expected_generation
            .ok_or(AgentStoreError::RunPreconditionFailed)?;
        Ok(self
            .store
            .assert_run_precondition(&request.run_id, expected_run_id, expected_generation)
            .await?)
    }

    pub async fn subscribe(
        &self,
        client_id: ClientId,
        request: &EventSubscriptionRequest,
    ) -> Result<EventSubscriptionSnapshot, AgentLifecycleError> {
        let catch_up = self
            .store
            .list_event_stream(&request.session_id, request.after_sequence)
            .await?;
        let subscription = EventSubscriptionRecord {
            id: EventSubscriptionId::random(),
            session_id: request.session_id.clone(),
            client_id,
            after_sequence: catch_up
                .last()
                .map_or(request.after_sequence, |event| event.sequence),
        };
        self.subscriptions
            .lock()
            .expect("subscription lock")
            .insert(subscription.id.clone(), subscription.clone());
        Ok(EventSubscriptionSnapshot {
            subscription,
            catch_up,
        })
    }

    pub fn open_event_stream(
        &self,
        subscription_id: &EventSubscriptionId,
        cancellation: CancellationToken,
    ) -> Result<mpsc::Receiver<EventSubscriptionSnapshot>, AgentLifecycleError> {
        let mut subscription = self
            .subscriptions
            .lock()
            .expect("subscription lock")
            .get(subscription_id)
            .cloned()
            .ok_or(AgentLifecycleError::SubscriptionNotFound)?;
        let store = self.store.clone();
        let mut live_events = store.subscribe_live_events();
        let subscriptions = Arc::clone(&self.subscriptions);
        let (sender, receiver) = mpsc::channel(16);
        tokio::spawn(async move {
            let mut timer = tokio::time::interval(std::time::Duration::from_millis(100));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    () = cancellation.cancelled() => break,
                    _ = timer.tick() => {},
                    event = live_events.recv() => {
                        match event {
                            Ok(event) if event.session_id == subscription.session_id => {},
                            Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }
                if !subscriptions
                    .lock()
                    .expect("subscription lock")
                    .contains_key(&subscription.id)
                {
                    break;
                }
                let Ok(events) = store
                    .list_event_stream(&subscription.session_id, subscription.after_sequence)
                    .await
                else {
                    continue;
                };
                let Some(last) = events.last() else {
                    continue;
                };
                subscription.after_sequence = last.sequence;
                let snapshot = EventSubscriptionSnapshot {
                    subscription: subscription.clone(),
                    catch_up: events,
                };
                if sender.send(snapshot).await.is_err() {
                    break;
                }
                subscriptions
                    .lock()
                    .expect("subscription lock")
                    .insert(subscription.id.clone(), subscription.clone());
            }
        });
        Ok(receiver)
    }

    pub fn unsubscribe(&self, subscription_id: &EventSubscriptionId) -> bool {
        self.subscriptions
            .lock()
            .expect("subscription lock")
            .remove(subscription_id)
            .is_some()
    }

    pub fn unsubscribe_client(&self, client_id: &ClientId) -> usize {
        let mut subscriptions = self.subscriptions.lock().expect("subscription lock");
        let before = subscriptions.len();
        subscriptions.retain(|_, subscription| &subscription.client_id != client_id);
        before.saturating_sub(subscriptions.len())
    }
}

fn validate_mutation_client(
    authenticated: &ClientId,
    requested: &ClientId,
) -> Result<(), AgentLifecycleError> {
    if authenticated == requested {
        Ok(())
    } else {
        Err(AgentLifecycleError::ClientMismatch)
    }
}

fn validate_idempotency(value: &str) -> Result<(), AgentLifecycleError> {
    if value.trim().is_empty() || value.len() > 128 {
        Err(AgentLifecycleError::MissingIdempotencyKey)
    } else {
        Ok(())
    }
}

fn server_features(flags: FeatureFlags) -> Vec<String> {
    let mut features = vec![
        "session_lifecycle_v2".into(),
        "typed_items".into(),
        "user_input".into(),
        "event_resume".into(),
    ];
    if flags.workbench {
        features.push("workbench".into());
    }
    if flags.workspace_tools {
        features.push("workspace_tools".into());
    }
    if flags.mcp_runtime {
        features.push("mcp_runtime".into());
    }
    features
}

fn now_ms() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use hachimi_protocol::{
        ApprovalPolicy, BehaviorMode, CheckoutId, CheckoutKind, CheckoutRecord, CheckoutStatus,
        EntryProfile, ExecutionTarget, LlmSettings, PermissionProfile, ProjectId, ProjectRecord,
        ProviderCapabilities, RunBudget, RunConfiguration, RunDriverKind, RunId, RunOrigin,
        RunPurpose, RunStatus, SandboxReadiness, SessionContextBinding, SessionRecord,
        WorkloadKind,
    };
    use serde_json::json;

    use super::*;

    fn sandbox_report() -> SandboxCapabilityReport {
        SandboxCapabilityReport {
            backend: "test".into(),
            readiness: SandboxReadiness::Degraded,
            os_enforced: false,
            filesystem_enforced: false,
            process_enforced: false,
            network_enforced: false,
            version: None,
            stable_error_code: Some("test_only".into()),
            diagnostics: Vec::new(),
        }
    }

    async fn seed_running_run() -> (AgentStore, SessionRecord, RunRecord) {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let now = 1_700_000_000_000_i64;
        let project = ProjectRecord {
            id: ProjectId::from("lifecycle-project"),
            display_name: "Lifecycle".into(),
            root_path: "C:\\lifecycle".into(),
            git_root: Some("C:\\lifecycle".into()),
            trusted: true,
            created_at_ms: now,
            updated_at_ms: now,
        };
        store.create_project(&project).await.expect("project");
        let checkout = CheckoutRecord {
            id: CheckoutId::from("lifecycle-checkout"),
            project_id: project.id.clone(),
            kind: CheckoutKind::Local,
            path: project.root_path.clone(),
            base_revision: Some("main".into()),
            head_revision: None,
            status: CheckoutStatus::Ready,
            pinned: false,
            created_at_ms: now,
            updated_at_ms: now,
        };
        store.create_checkout(&checkout).await.expect("checkout");
        let session = SessionRecord {
            id: SessionId::from("lifecycle-session"),
            context: SessionContextBinding::Project {
                project_id: project.id.clone(),
                checkout_id: checkout.id,
            },
            entry_profile: EntryProfile::Workbench,
            title: "Lifecycle".into(),
            archived: false,
            pinned: false,
            parent_session_id: None,
            source_run_id: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        store.create_session(&session).await.expect("session");
        let run = RunRecord {
            id: RunId::from("lifecycle-run"),
            session_id: session.id.clone(),
            status: RunStatus::Queued,
            purpose: RunPurpose::Task,
            origin: RunOrigin::Manual,
            generation: 7,
            configuration: RunConfiguration {
                model_snapshot: LlmSettings::default(),
                driver: RunDriverKind::ToolLoop,
                entry_profile: EntryProfile::Workbench,
                workload_override: Some(WorkloadKind::Coding),
                behavior_mode: BehaviorMode::Default,
                execution_target: Some(ExecutionTarget::Local {
                    project_id: project.id,
                }),
                approval_policy: ApprovalPolicy::OnlyWhenNeeded,
                permission_profile: PermissionProfile::ReadOnly,
                budget: RunBudget::default(),
                accepted_plan_id: None,
                accepted_plan_revision: None,
            },
            requested_capabilities: ProviderCapabilities::default(),
            negotiated_capabilities: ProviderCapabilities::default(),
            provider_capability_probe: None,
            capability_degradations: Vec::new(),
            failure_code: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        store
            .create_run_idempotent("test", "lifecycle-run", &run)
            .await
            .expect("run");
        store
            .transition_run(&run.id, RunStatus::Preparing, None)
            .await
            .expect("preparing");
        store
            .transition_run(&run.id, RunStatus::Running, None)
            .await
            .expect("running");
        (store, session, run)
    }

    #[tokio::test]
    async fn subscription_catches_up_and_unsubscribes_without_stopping_run() {
        let (store, session, run) = seed_running_run().await;
        store
            .append_event(&session.id, Some(&run.id), "before.subscribe", json!({}))
            .await
            .expect("first event");
        let service = AgentLifecycleService::new(
            store.clone(),
            FeatureFlags::all_disabled(),
            sandbox_report(),
        );
        let snapshot = service
            .subscribe(
                ClientId("client-one".into()),
                &EventSubscriptionRequest {
                    session_id: session.id.clone(),
                    after_sequence: 0,
                },
            )
            .await
            .expect("subscribe");
        assert!(
            snapshot
                .catch_up
                .iter()
                .any(|event| event.event_name() == "before.subscribe")
        );
        let cancellation = CancellationToken::new();
        let mut stream = service
            .open_event_stream(&snapshot.subscription.id, cancellation.clone())
            .expect("open event stream");
        store
            .append_event(&session.id, Some(&run.id), "after.subscribe", json!({}))
            .await
            .expect("second event");
        let pushed = tokio::time::timeout(std::time::Duration::from_secs(1), stream.recv())
            .await
            .expect("push timeout")
            .expect("pushed batch");
        assert_eq!(pushed.catch_up.len(), 1);
        assert_eq!(pushed.catch_up[0].event_name(), "after.subscribe");
        assert!(
            pushed.subscription.after_sequence > snapshot.subscription.after_sequence,
            "the pushed watermark must advance monotonically"
        );

        assert!(service.unsubscribe(&snapshot.subscription.id));
        cancellation.cancel();
        assert!(!service.unsubscribe(&snapshot.subscription.id));
        assert_eq!(
            store
                .get_run(&run.id)
                .await
                .expect("get run")
                .expect("run")
                .status,
            RunStatus::Running,
            "unsubscribing must not cancel the active run"
        );
    }

    #[tokio::test]
    async fn initialize_negotiates_only_supported_server_features() {
        let service = AgentLifecycleService::new(
            AgentStore::connect_in_memory().await.expect("store"),
            FeatureFlags {
                workbench: true,
                workspace_tools: true,
                ..FeatureFlags::all_disabled()
            },
            sandbox_report(),
        );
        let response = service
            .initialize(&ControlInitializeRequest {
                client_version: "test".into(),
                protocol_version: CONTROL_PROTOCOL_VERSION,
                supported_features: vec!["workbench".into(), "mcp_runtime".into()],
                experimental_features: vec!["workbench".into()],
            })
            .expect("initialize");
        assert_eq!(response.enabled_features, vec!["workbench"]);
        assert_eq!(response.experimental_features, vec!["workbench"]);
    }
}
