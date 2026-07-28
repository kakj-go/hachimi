// SPDX-License-Identifier: MIT
// Copyright (c) 2026 OpenClaw Foundation
// Adapted from openclaw/openclaw src/process/command-queue.ts
// Commit: f6d456235cf011004f7cffc71a95acf6fbf1fa0a
// Modified for Hachimi: Tokio mutex lanes, generation fencing, cancellation, and bounded drain.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use hachimi_protocol::SessionId;
use parking_lot::{Mutex, RwLock};
use thiserror::Error;
use tokio::sync::{Mutex as AsyncMutex, Notify, OwnedMutexGuard};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneMarker {
    pub session_id: SessionId,
    pub generation: u64,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LaneError {
    #[error("session lane was cancelled")]
    Cancelled,
    #[error("session lane generation is stale")]
    StaleGeneration,
}

#[derive(Debug)]
struct LaneEpoch {
    generation: u64,
    cancellation: CancellationToken,
}

#[derive(Debug)]
struct LaneState {
    gate: Arc<AsyncMutex<()>>,
    epoch: RwLock<LaneEpoch>,
    active: AtomicUsize,
    changed: Notify,
}

impl Default for LaneState {
    fn default() -> Self {
        Self {
            gate: Arc::new(AsyncMutex::new(())),
            epoch: RwLock::new(LaneEpoch {
                generation: 1,
                cancellation: CancellationToken::new(),
            }),
            active: AtomicUsize::new(0),
            changed: Notify::new(),
        }
    }
}

#[derive(Debug, Default)]
pub struct SessionLanes {
    lanes: Mutex<HashMap<SessionId, Arc<LaneState>>>,
}

impl SessionLanes {
    fn state(&self, session_id: &SessionId) -> Arc<LaneState> {
        self.lanes
            .lock()
            .entry(session_id.clone())
            .or_insert_with(|| Arc::new(LaneState::default()))
            .clone()
    }

    #[must_use]
    pub fn marker(&self, session_id: &SessionId) -> LaneMarker {
        let state = self.state(session_id);
        let generation = state.epoch.read().generation;
        LaneMarker {
            session_id: session_id.clone(),
            generation,
        }
    }

    #[must_use]
    pub fn is_current(&self, marker: &LaneMarker) -> bool {
        self.state(&marker.session_id).epoch.read().generation == marker.generation
    }

    pub async fn enter(&self, session_id: &SessionId) -> Result<SessionLanePermit, LaneError> {
        let state = self.state(session_id);
        let (generation, cancellation) = {
            let epoch = state.epoch.read();
            (epoch.generation, epoch.cancellation.child_token())
        };
        let gate = Arc::clone(&state.gate);
        let guard = tokio::select! {
            () = cancellation.cancelled() => return Err(LaneError::Cancelled),
            guard = gate.lock_owned() => guard,
        };
        if state.epoch.read().generation != generation {
            return Err(LaneError::StaleGeneration);
        }
        state.active.fetch_add(1, Ordering::SeqCst);
        Ok(SessionLanePermit {
            state,
            marker: LaneMarker {
                session_id: session_id.clone(),
                generation,
            },
            cancellation,
            _guard: guard,
        })
    }

    pub fn reset(&self, session_id: &SessionId) -> LaneMarker {
        let state = self.state(session_id);
        let generation = {
            let mut epoch = state.epoch.write();
            epoch.cancellation.cancel();
            epoch.generation = epoch.generation.saturating_add(1);
            epoch.cancellation = CancellationToken::new();
            epoch.generation
        };
        state.changed.notify_waiters();
        LaneMarker {
            session_id: session_id.clone(),
            generation,
        }
    }

    pub async fn wait_drained(&self, timeout: Duration) -> bool {
        let states = self.lanes.lock().values().cloned().collect::<Vec<_>>();
        tokio::time::timeout(timeout, async {
            for state in states {
                while state.active.load(Ordering::SeqCst) != 0 || state.gate.try_lock().is_err() {
                    state.changed.notified().await;
                }
            }
        })
        .await
        .is_ok()
    }
}

#[derive(Debug)]
pub struct SessionLanePermit {
    state: Arc<LaneState>,
    marker: LaneMarker,
    cancellation: CancellationToken,
    _guard: OwnedMutexGuard<()>,
}

impl SessionLanePermit {
    #[must_use]
    pub const fn marker(&self) -> &LaneMarker {
        &self.marker
    }

    #[must_use]
    pub fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}

impl Drop for SessionLanePermit {
    fn drop(&mut self) {
        self.state.active.fetch_sub(1, Ordering::SeqCst);
        self.state.changed.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn one_session_is_serial_and_reset_fences_old_generation() {
        let lanes = Arc::new(SessionLanes::default());
        let session = SessionId::from("session-1");
        let first = lanes.enter(&session).await.expect("first");
        let old_marker = first.marker().clone();
        let waiting_lanes = Arc::clone(&lanes);
        let waiting_session = session.clone();
        let waiter = tokio::spawn(async move { waiting_lanes.enter(&waiting_session).await });
        tokio::task::yield_now().await;
        let new_marker = lanes.reset(&session);
        assert!(!lanes.is_current(&old_marker));
        assert!(lanes.is_current(&new_marker));
        drop(first);
        assert!(matches!(
            waiter.await.expect("join").expect_err("stale"),
            LaneError::Cancelled | LaneError::StaleGeneration
        ));
        let current = lanes.enter(&session).await.expect("current");
        assert_eq!(current.marker().generation, new_marker.generation);
        drop(current);
        assert!(lanes.wait_drained(Duration::from_secs(1)).await);
    }
}
