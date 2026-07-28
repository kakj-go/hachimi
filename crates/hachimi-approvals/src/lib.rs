//! Persisted asynchronous approval requests bound to exact Run generations and parameter hashes.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use futures_util::future::BoxFuture;
use hachimi_protocol::{
    ApprovalId, ApprovalRequestRecord, ApprovalResolution, ApprovalStatus, RunId,
};
use hachimi_storage::AgentStore;
use parking_lot::Mutex;
use thiserror::Error;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

pub type ApprovalFuture = BoxFuture<'static, Result<ApprovalRequestRecord, ApprovalError>>;
pub type ApprovalResolveFuture = BoxFuture<'static, Result<ApprovalRequestRecord, ApprovalError>>;
pub type ApprovalCancelFuture = BoxFuture<'static, Result<u64, ApprovalError>>;

#[derive(Debug, Error)]
pub enum ApprovalError {
    #[error("approval persistence failed: {0}")]
    Store(String),
    #[error("interactive approval is unavailable")]
    Unavailable,
    #[error("approval waiter ended before resolution")]
    WaiterClosed,
}

pub trait ApprovalBroker: Send + Sync {
    fn request(
        &self,
        request: ApprovalRequestRecord,
        cancellation: CancellationToken,
    ) -> ApprovalFuture;

    fn resolve(&self, resolution: ApprovalResolution) -> ApprovalResolveFuture;

    fn cancel_run(&self, run_id: RunId) -> ApprovalCancelFuture;
}

#[derive(Debug, Default)]
pub struct NonInteractiveApproval;

impl ApprovalBroker for NonInteractiveApproval {
    fn request(
        &self,
        mut request: ApprovalRequestRecord,
        _cancellation: CancellationToken,
    ) -> ApprovalFuture {
        Box::pin(async move {
            request.status = ApprovalStatus::Denied;
            request.resolved_by = Some("system:non-interactive".into());
            request.resolved_at_ms = Some(now_ms());
            Ok(request)
        })
    }

    fn resolve(&self, _resolution: ApprovalResolution) -> ApprovalResolveFuture {
        Box::pin(async { Err(ApprovalError::Unavailable) })
    }

    fn cancel_run(&self, _run_id: RunId) -> ApprovalCancelFuture {
        Box::pin(async { Ok(0) })
    }
}

type ApprovalWaiter = oneshot::Sender<ApprovalRequestRecord>;

#[derive(Debug, Clone)]
pub struct PersistentApprovalBroker {
    store: AgentStore,
    waiters: Arc<Mutex<HashMap<ApprovalId, ApprovalWaiter>>>,
}

impl PersistentApprovalBroker {
    #[must_use]
    pub fn new(store: AgentStore) -> Self {
        Self {
            store,
            waiters: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl ApprovalBroker for PersistentApprovalBroker {
    fn request(
        &self,
        request: ApprovalRequestRecord,
        cancellation: CancellationToken,
    ) -> ApprovalFuture {
        let store = self.store.clone();
        let waiters = Arc::clone(&self.waiters);
        Box::pin(async move {
            let (sender, receiver) = oneshot::channel();
            waiters.lock().insert(request.id.clone(), sender);
            if let Err(error) = store.create_approval(&request).await {
                waiters.lock().remove(&request.id);
                return Err(ApprovalError::Store(error.to_string()));
            }
            let wait = async {
                match request.expires_at_ms {
                    Some(expires_at_ms) => {
                        let remaining = expires_at_ms.saturating_sub(now_ms());
                        match tokio::time::timeout(
                            Duration::from_millis(u64::try_from(remaining).unwrap_or_default()),
                            receiver,
                        )
                        .await
                        {
                            Ok(result) => result.map_err(|_| ApprovalError::WaiterClosed),
                            Err(_) => {
                                let resolution = ApprovalResolution {
                                    approval_id: request.id.clone(),
                                    decision: ApprovalStatus::Denied,
                                    parameter_hash: request.parameter_hash.clone(),
                                    run_generation: request.run_generation,
                                    resolved_by: "system:expired".into(),
                                    resolved_at_ms: expires_at_ms,
                                };
                                store
                                    .resolve_approval(&resolution)
                                    .await
                                    .map_err(|error| ApprovalError::Store(error.to_string()))
                            }
                        }
                    }
                    None => receiver.await.map_err(|_| ApprovalError::WaiterClosed),
                }
            };
            let outcome = tokio::select! {
                outcome = wait => outcome,
                () = cancellation.cancelled() => {
                    store
                        .cancel_run_approvals(&request.run_id, now_ms())
                        .await
                        .map_err(|error| ApprovalError::Store(error.to_string()))?;
                    store
                        .get_approval(&request.id)
                        .await
                        .map_err(|error| ApprovalError::Store(error.to_string()))?
                        .ok_or_else(|| ApprovalError::Store("cancelled approval disappeared".into()))
                }
            };
            waiters.lock().remove(&request.id);
            outcome
        })
    }

    fn resolve(&self, resolution: ApprovalResolution) -> ApprovalResolveFuture {
        let store = self.store.clone();
        let waiters = Arc::clone(&self.waiters);
        Box::pin(async move {
            let resolved = store
                .resolve_approval(&resolution)
                .await
                .map_err(|error| ApprovalError::Store(error.to_string()))?;
            if let Some(waiter) = waiters.lock().remove(&resolved.id) {
                let _ = waiter.send(resolved.clone());
            }
            Ok(resolved)
        })
    }

    fn cancel_run(&self, run_id: RunId) -> ApprovalCancelFuture {
        let store = self.store.clone();
        let waiters = Arc::clone(&self.waiters);
        Box::pin(async move {
            let pending = store
                .list_pending_approvals()
                .await
                .map_err(|error| ApprovalError::Store(error.to_string()))?;
            let affected = pending
                .iter()
                .filter(|approval| approval.run_id == run_id)
                .map(|approval| approval.id.clone())
                .collect::<Vec<_>>();
            let count = store
                .cancel_run_approvals(&run_id, now_ms())
                .await
                .map_err(|error| ApprovalError::Store(error.to_string()))?;
            for approval_id in affected {
                let waiter = { waiters.lock().remove(&approval_id) };
                if let Some(waiter) = waiter
                    && let Some(record) = store
                        .get_approval(&approval_id)
                        .await
                        .map_err(|error| ApprovalError::Store(error.to_string()))?
                {
                    let _ = waiter.send(record);
                }
            }
            Ok(count)
        })
    }
}

fn now_ms() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}
