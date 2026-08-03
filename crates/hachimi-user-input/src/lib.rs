//! Persistent user-input requests with memory-only answer delivery.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use futures_util::future::BoxFuture;
use hachimi_protocol::{
    RunId, UserInputAnswer, UserInputRequestId, UserInputRequestRecord, UserInputResolution,
    UserInputResolutionAction,
};
use hachimi_storage::AgentStore;
use parking_lot::Mutex;
use thiserror::Error;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserInputOutcome {
    pub request: UserInputRequestRecord,
    pub action: UserInputResolutionAction,
    /// Answers may contain secrets and must never be logged or persisted.
    pub answers: Vec<UserInputAnswer>,
}

pub type UserInputFuture = BoxFuture<'static, Result<UserInputOutcome, UserInputError>>;
pub type UserInputResolveFuture =
    BoxFuture<'static, Result<UserInputRequestRecord, UserInputError>>;
pub type UserInputCancelFuture = BoxFuture<'static, Result<u64, UserInputError>>;

#[derive(Debug, Error)]
pub enum UserInputError {
    #[error("user input persistence failed: {0}")]
    Store(String),
    #[error("interactive user input is unavailable")]
    Unavailable,
    #[error("user input request expired")]
    Expired,
    #[error("user input waiter ended before resolution")]
    WaiterClosed,
}

pub trait UserInputBroker: Send + Sync {
    fn request(
        &self,
        request: UserInputRequestRecord,
        cancellation: CancellationToken,
    ) -> UserInputFuture;

    fn resolve(&self, resolution: UserInputResolution) -> UserInputResolveFuture;

    fn cancel_run(&self, run_id: RunId) -> UserInputCancelFuture;
}

#[derive(Debug, Default)]
pub struct NonInteractiveUserInput;

impl UserInputBroker for NonInteractiveUserInput {
    fn request(
        &self,
        _request: UserInputRequestRecord,
        _cancellation: CancellationToken,
    ) -> UserInputFuture {
        Box::pin(async { Err(UserInputError::Unavailable) })
    }

    fn resolve(&self, _resolution: UserInputResolution) -> UserInputResolveFuture {
        Box::pin(async { Err(UserInputError::Unavailable) })
    }

    fn cancel_run(&self, _run_id: RunId) -> UserInputCancelFuture {
        Box::pin(async { Ok(0) })
    }
}

type UserInputWaiter = oneshot::Sender<UserInputResolution>;

#[derive(Debug, Clone)]
pub struct PersistentUserInputBroker {
    store: AgentStore,
    waiters: Arc<Mutex<HashMap<UserInputRequestId, UserInputWaiter>>>,
}

impl PersistentUserInputBroker {
    #[must_use]
    pub fn new(store: AgentStore) -> Self {
        Self {
            store,
            waiters: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl UserInputBroker for PersistentUserInputBroker {
    fn request(
        &self,
        request: UserInputRequestRecord,
        cancellation: CancellationToken,
    ) -> UserInputFuture {
        let store = self.store.clone();
        let waiters = Arc::clone(&self.waiters);
        Box::pin(async move {
            let (sender, receiver) = oneshot::channel();
            waiters.lock().insert(request.id.clone(), sender);
            if let Err(error) = store.create_user_input_request(&request).await {
                waiters.lock().remove(&request.id);
                return Err(UserInputError::Store(error.to_string()));
            }
            let wait = async {
                if let Some(expires_at_ms) = request.expires_at_ms {
                    let remaining = expires_at_ms.saturating_sub(now_ms());
                    match tokio::time::timeout(
                        Duration::from_millis(u64::try_from(remaining).unwrap_or_default()),
                        receiver,
                    )
                    .await
                    {
                        Ok(result) => result.map_err(|_| UserInputError::WaiterClosed),
                        Err(_) => {
                            let defaults = default_answers(&request);
                            if defaults.len() == request.questions.len() {
                                let resolution = UserInputResolution {
                                    request_id: request.id.clone(),
                                    expected_run_id: request.run_id.clone(),
                                    expected_generation: request.run_generation,
                                    action: UserInputResolutionAction::Submit,
                                    answers: defaults,
                                    resolved_by: "system:auto-resolution".into(),
                                    resolved_at_ms: expires_at_ms,
                                };
                                store
                                    .resolve_user_input(&resolution)
                                    .await
                                    .map_err(|error| UserInputError::Store(error.to_string()))?;
                                Ok(resolution)
                            } else {
                                store
                                    .expire_user_input(&request.id, expires_at_ms)
                                    .await
                                    .map_err(|error| UserInputError::Store(error.to_string()))?;
                                Err(UserInputError::Expired)
                            }
                        }
                    }
                } else {
                    receiver.await.map_err(|_| UserInputError::WaiterClosed)
                }
            };
            let resolution = tokio::select! {
                outcome = wait => outcome,
                () = cancellation.cancelled() => {
                    store
                        .cancel_run_user_inputs(&request.run_id, now_ms(), "system:cancelled")
                        .await
                        .map_err(|error| UserInputError::Store(error.to_string()))?;
                    waiters.lock().remove(&request.id);
                    return Err(UserInputError::WaiterClosed);
                }
            };
            waiters.lock().remove(&request.id);
            let resolution = resolution?;
            let resolved = store
                .get_user_input_request(&request.id)
                .await
                .map_err(|error| UserInputError::Store(error.to_string()))?
                .ok_or_else(|| UserInputError::Store("resolved request disappeared".into()))?;
            Ok(UserInputOutcome {
                request: resolved,
                action: resolution.action,
                answers: resolution.answers,
            })
        })
    }

    fn resolve(&self, resolution: UserInputResolution) -> UserInputResolveFuture {
        let store = self.store.clone();
        let waiters = Arc::clone(&self.waiters);
        Box::pin(async move {
            let resolved = store
                .resolve_user_input(&resolution)
                .await
                .map_err(|error| UserInputError::Store(error.to_string()))?;
            if let Some(waiter) = waiters.lock().remove(&resolved.id) {
                let _ = waiter.send(resolution);
            }
            Ok(resolved)
        })
    }

    fn cancel_run(&self, run_id: RunId) -> UserInputCancelFuture {
        let store = self.store.clone();
        let waiters = Arc::clone(&self.waiters);
        Box::pin(async move {
            let pending = store
                .list_pending_user_inputs(None)
                .await
                .map_err(|error| UserInputError::Store(error.to_string()))?;
            let affected = pending
                .iter()
                .filter(|request| request.run_id == run_id)
                .map(|request| request.id.clone())
                .collect::<Vec<_>>();
            let count = store
                .cancel_run_user_inputs(&run_id, now_ms(), "system:cancelled")
                .await
                .map_err(|error| UserInputError::Store(error.to_string()))?;
            for request_id in affected {
                waiters.lock().remove(&request_id);
            }
            Ok(count)
        })
    }
}

fn default_answers(request: &UserInputRequestRecord) -> Vec<UserInputAnswer> {
    request
        .questions
        .iter()
        .filter_map(|question| {
            if question.secret {
                return None;
            }
            question
                .default_answer
                .as_ref()
                .or_else(|| question.options.first().map(|option| &option.value))
                .map(|value| UserInputAnswer {
                    question_id: question.id.clone(),
                    value: value.clone(),
                })
        })
        .collect()
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
        EntryProfile, ExecutionTarget, ItemId, LlmSettings, PermissionProfile, ProjectId,
        ProjectRecord, ProviderCapabilities, RunBudget, RunConfiguration, RunDriverKind, RunOrigin,
        RunPurpose, RunRecord, RunStatus, SessionContextBinding, SessionId, SessionRecord,
        UserInputOption, UserInputQuestion, UserInputStatus, WorkloadKind,
    };

    use super::*;

    async fn seed_waiting_run(suffix: &str) -> (AgentStore, SessionRecord, RunRecord) {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let now = now_ms();
        let project = ProjectRecord {
            id: ProjectId::new(format!("project-{suffix}")),
            display_name: "User input".into(),
            root_path: format!("C:\\user-input-{suffix}"),
            git_root: None,
            trusted: true,
            created_at_ms: now,
            updated_at_ms: now,
        };
        store.create_project(&project).await.expect("project");
        let checkout = CheckoutRecord {
            id: CheckoutId::new(format!("checkout-{suffix}")),
            project_id: project.id.clone(),
            kind: CheckoutKind::Local,
            path: project.root_path.clone(),
            base_revision: None,
            head_revision: None,
            status: CheckoutStatus::Ready,
            pinned: false,
            created_at_ms: now,
            updated_at_ms: now,
        };
        store.create_checkout(&checkout).await.expect("checkout");
        let session = SessionRecord {
            id: SessionId::new(format!("session-{suffix}")),
            context: SessionContextBinding::Project {
                project_id: project.id.clone(),
                checkout_id: checkout.id,
            },
            entry_profile: EntryProfile::Workbench,
            title: "User input".into(),
            archived: false,
            pinned: false,
            parent_session_id: None,
            source_run_id: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        store.create_session(&session).await.expect("session");
        let run = RunRecord {
            id: RunId::new(format!("run-{suffix}")),
            session_id: session.id.clone(),
            status: RunStatus::Queued,
            purpose: RunPurpose::Task,
            origin: RunOrigin::Interactive,
            generation: 2,
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
            .create_run_idempotent("test", &format!("run-{suffix}"), &run)
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

    fn request(
        session: &SessionRecord,
        run: &RunRecord,
        id: &str,
        question: UserInputQuestion,
        expires_at_ms: Option<i64>,
    ) -> UserInputRequestRecord {
        UserInputRequestRecord {
            id: UserInputRequestId::from(id),
            session_id: session.id.clone(),
            run_id: run.id.clone(),
            run_generation: run.generation,
            item_id: ItemId::new(format!("{id}-item")),
            questions: vec![question],
            display_answers: Vec::new(),
            status: UserInputStatus::Pending,
            expires_at_ms,
            created_at_ms: now_ms(),
            resolved_at_ms: None,
            resolved_by: None,
        }
    }

    #[tokio::test]
    async fn timeout_uses_non_secret_default_and_marks_request_resolved() {
        let (store, session, run) = seed_waiting_run("default").await;
        let broker = PersistentUserInputBroker::new(store.clone());
        let outcome = broker
            .request(
                request(
                    &session,
                    &run,
                    "default-input",
                    UserInputQuestion {
                        id: "choice".into(),
                        header: "Choice".into(),
                        prompt: "Choose".into(),
                        options: vec![
                            UserInputOption {
                                label: "Continue".into(),
                                value: "continue-secret-to-store-check".into(),
                                description: Some("Continue".into()),
                            },
                            UserInputOption {
                                label: "Stop".into(),
                                value: "stop".into(),
                                description: Some("Stop".into()),
                            },
                        ],
                        secret: false,
                        auto_resolution_ms: Some(1),
                        default_answer: None,
                    },
                    Some(now_ms()),
                ),
                CancellationToken::new(),
            )
            .await
            .expect("auto resolution");
        assert_eq!(outcome.request.status, UserInputStatus::Resolved);
        assert_eq!(outcome.answers[0].value, "continue-secret-to-store-check");
        assert_eq!(
            store
                .get_user_input_request(&outcome.request.id)
                .await
                .expect("get request")
                .expect("request")
                .status,
            UserInputStatus::Resolved
        );
    }

    #[tokio::test]
    async fn secret_timeout_expires_and_cancel_run_wakes_waiter() {
        let (store, session, run) = seed_waiting_run("secret-timeout").await;
        let broker = PersistentUserInputBroker::new(store.clone());
        let expired = broker
            .request(
                request(
                    &session,
                    &run,
                    "secret-timeout",
                    UserInputQuestion {
                        id: "secret".into(),
                        header: "Secret".into(),
                        prompt: "Enter secret".into(),
                        options: Vec::new(),
                        secret: true,
                        auto_resolution_ms: Some(1),
                        default_answer: None,
                    },
                    Some(now_ms()),
                ),
                CancellationToken::new(),
            )
            .await;
        assert!(matches!(expired, Err(UserInputError::Expired)));

        let (store, session, run) = seed_waiting_run("cancel").await;
        let broker = PersistentUserInputBroker::new(store.clone());
        let request = request(
            &session,
            &run,
            "cancel-input",
            UserInputQuestion {
                id: "answer".into(),
                header: "Answer".into(),
                prompt: "Answer".into(),
                options: Vec::new(),
                secret: false,
                auto_resolution_ms: None,
                default_answer: None,
            },
            None,
        );
        let request_id = request.id.clone();
        let waiting = tokio::spawn(broker.request(request, CancellationToken::new()));
        for _ in 0..50 {
            if store
                .get_user_input_request(&request_id)
                .await
                .expect("get request")
                .is_some()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(broker.cancel_run(run.id).await.expect("cancel"), 1);
        assert!(matches!(
            waiting.await.expect("join"),
            Err(UserInputError::WaiterClosed)
        ));
        assert_eq!(
            store
                .get_user_input_request(&request_id)
                .await
                .expect("get cancelled")
                .expect("cancelled request")
                .status,
            UserInputStatus::Cancelled
        );
    }

    #[tokio::test]
    async fn explicit_decline_resumes_waiter_without_persisting_answers() {
        let (store, session, run) = seed_waiting_run("decline").await;
        let broker = PersistentUserInputBroker::new(store.clone());
        let pending = request(
            &session,
            &run,
            "decline-input",
            UserInputQuestion {
                id: "answer".into(),
                header: "Answer".into(),
                prompt: "Answer".into(),
                options: Vec::new(),
                secret: false,
                auto_resolution_ms: None,
                default_answer: None,
            },
            None,
        );
        let request_id = pending.id.clone();
        let waiting = tokio::spawn(broker.request(pending, CancellationToken::new()));
        for _ in 0..50 {
            if store
                .get_user_input_request(&request_id)
                .await
                .expect("get request")
                .is_some()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        broker
            .resolve(UserInputResolution {
                request_id,
                expected_run_id: run.id.clone(),
                expected_generation: run.generation,
                action: UserInputResolutionAction::Decline,
                answers: Vec::new(),
                resolved_by: "test".into(),
                resolved_at_ms: now_ms(),
            })
            .await
            .expect("decline");
        let outcome = waiting.await.expect("join").expect("outcome");
        assert_eq!(outcome.action, UserInputResolutionAction::Decline);
        assert!(outcome.answers.is_empty());
        assert_eq!(outcome.request.status, UserInputStatus::Cancelled);
        assert_eq!(
            store
                .get_run(&run.id)
                .await
                .expect("run")
                .expect("run record")
                .status,
            hachimi_protocol::RunStatus::Running
        );
    }
}
