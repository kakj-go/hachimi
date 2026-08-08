use std::time::Instant;

pub(super) struct StartupTimeline {
    started: Instant,
    checkpoint: Instant,
}

impl StartupTimeline {
    pub(super) fn new() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            checkpoint: now,
        }
    }

    pub(super) fn checkpoint(&mut self, phase: &'static str) {
        let now = Instant::now();
        tracing::info!(
            phase,
            elapsed_ms = now.duration_since(self.checkpoint).as_millis(),
            total_ms = now.duration_since(self.started).as_millis(),
            "desktop startup phase completed"
        );
        self.checkpoint = now;
    }
}
