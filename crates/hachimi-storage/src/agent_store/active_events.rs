//! Bounded, process-local replay for non-authoritative streaming deltas.
//!
//! Deltas deliberately never enter SQLite. Final `item.completed` payloads remain
//! the durable source of truth, matching Codex's separation between live output
//! projection and persisted transcript items.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::Mutex,
};

use hachimi_protocol::{
    CONTROL_PROTOCOL_VERSION, ItemId, RunEventEnvelope, RunEventPayload, RunId, SessionId,
};
use tokio::sync::broadcast;

use super::{AgentStore, AgentStoreError, next_sequence_tx, now_ms};

const MAX_REPLAY_EVENTS_PER_SESSION: usize = 512;
const MAX_DELTA_CHARS: usize = 32 * 1024;

#[derive(Debug)]
pub(crate) struct ActiveRunEvents {
    replay: Mutex<BTreeMap<String, VecDeque<RunEventEnvelope>>>,
    sender: broadcast::Sender<RunEventEnvelope>,
}

impl ActiveRunEvents {
    pub(crate) fn new() -> Self {
        let (sender, _) = broadcast::channel(MAX_REPLAY_EVENTS_PER_SESSION);
        Self {
            replay: Mutex::new(BTreeMap::new()),
            sender,
        }
    }

    fn publish(&self, event: RunEventEnvelope) {
        let mut replay = self.replay.lock().expect("active event replay lock");
        let events = replay.entry(event.session_id.to_string()).or_default();
        events.push_back(event.clone());
        while events.len() > MAX_REPLAY_EVENTS_PER_SESSION {
            events.pop_front();
        }
        drop(replay);
        let _ = self.sender.send(event);
    }

    pub(crate) fn list(
        &self,
        session_id: &SessionId,
        after_sequence: u64,
    ) -> Vec<RunEventEnvelope> {
        self.replay
            .lock()
            .expect("active event replay lock")
            .get(session_id.as_str())
            .into_iter()
            .flat_map(|events| events.iter())
            .filter(|event| event.sequence > after_sequence)
            .cloned()
            .collect()
    }

    pub(crate) fn complete_item(&self, session_id: &SessionId, item_id: &ItemId) {
        let mut replay = self.replay.lock().expect("active event replay lock");
        let Some(events) = replay.get_mut(session_id.as_str()) else {
            return;
        };
        events.retain(|event| {
            !matches!(
                &event.payload,
                RunEventPayload::ItemDelta { item_id: candidate, .. } if candidate == item_id
            )
        });
        if events.is_empty() {
            replay.remove(session_id.as_str());
        }
    }

    fn subscribe(&self) -> broadcast::Receiver<RunEventEnvelope> {
        self.sender.subscribe()
    }
}

impl AgentStore {
    /// Allocates a Session-monotonic sequence while keeping the delta out of SQLite.
    pub async fn append_live_item_delta(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        item_id: &ItemId,
        delta: &str,
    ) -> Result<RunEventEnvelope, AgentStoreError> {
        let delta = delta.chars().take(MAX_DELTA_CHARS).collect::<String>();
        let created_at_ms = now_ms();
        let mut transaction = self.pool.begin().await?;
        let sequence = next_sequence_tx(&mut transaction, session_id, created_at_ms).await?;
        transaction.commit().await?;
        let envelope = RunEventEnvelope {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            sequence,
            session_id: session_id.clone(),
            run_id: Some(run_id.clone()),
            payload: RunEventPayload::ItemDelta {
                item_id: item_id.clone(),
                delta,
            },
            created_at_ms,
        };
        self.active_events.publish(envelope.clone());
        Ok(envelope)
    }

    #[must_use]
    pub fn list_active_event_replay(
        &self,
        session_id: &SessionId,
        after_sequence: u64,
    ) -> Vec<RunEventEnvelope> {
        self.active_events.list(session_id, after_sequence)
    }

    pub async fn list_event_stream(
        &self,
        session_id: &SessionId,
        after_sequence: u64,
    ) -> Result<Vec<RunEventEnvelope>, AgentStoreError> {
        let mut events = self.list_events(session_id, after_sequence).await?;
        events.extend(self.list_active_event_replay(session_id, after_sequence));
        events.sort_by_key(|event| event.sequence);
        events.dedup_by_key(|event| event.sequence);
        Ok(events)
    }

    #[must_use]
    pub fn subscribe_live_events(&self) -> broadcast::Receiver<RunEventEnvelope> {
        self.active_events.subscribe()
    }
}
