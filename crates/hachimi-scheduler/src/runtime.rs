use std::{future::Future, sync::Arc, sync::atomic::Ordering, time::Duration};

use tokio_util::sync::CancellationToken;

use crate::{SchedulerError, SchedulerService};

const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
];
const HEALTHY_RECOVERY_WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchedulerRuntimeEvent {
    Ready,
    Retrying {
        attempt: u32,
        error_code: &'static str,
    },
    Recovered,
    Failed {
        error_code: &'static str,
        detail: String,
    },
}

pub type SchedulerRuntimeObserver = Arc<dyn Fn(SchedulerRuntimeEvent) + Send + Sync>;

struct RetryOutcome<T> {
    value: T,
    recovered: bool,
}

impl SchedulerService {
    #[must_use]
    pub fn start(self: Arc<Self>) -> SchedulerHandle {
        self.start_with_observer(Arc::new(|_| {}))
    }

    #[must_use]
    pub fn start_with_observer(
        self: Arc<Self>,
        observer: SchedulerRuntimeObserver,
    ) -> SchedulerHandle {
        self.accepting.store(true, Ordering::SeqCst);
        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let service = Arc::clone(&self);
        let join = tokio::spawn(async move {
            observer(SchedulerRuntimeEvent::Ready);
            let mut recovering_since = None;
            loop {
                if task_cancellation.is_cancelled() {
                    break;
                }
                let wake_at = match retry_operation(&task_cancellation, &observer, || async {
                    service
                        .store
                        .next_schedule_wakeup()
                        .await
                        .map_err(SchedulerError::from)
                })
                .await
                {
                    Ok(Some(outcome)) => {
                        if outcome.recovered {
                            recovering_since = Some(tokio::time::Instant::now());
                        }
                        outcome.value
                    }
                    Ok(None) => break,
                    Err(error) => {
                        fail(&service, &observer, error);
                        break;
                    }
                };
                let delay = Duration::from_millis(super::service::scheduler_delay_ms(
                    wake_at,
                    service.clock.now_ms(),
                ));
                let recovery_delay = recovering_since.map_or(HEALTHY_RECOVERY_WINDOW, |started| {
                    HEALTHY_RECOVERY_WINDOW.saturating_sub(started.elapsed())
                });
                tokio::select! {
                    () = task_cancellation.cancelled() => break,
                    () = service.wake.notified() => continue,
                    () = tokio::time::sleep(recovery_delay), if recovering_since.is_some() => {
                        recovering_since = None;
                        observer(SchedulerRuntimeEvent::Recovered);
                    }
                    () = tokio::time::sleep(delay) => {
                        match retry_operation(&task_cancellation, &observer, || service.trigger_due()).await {
                            Ok(Some(outcome)) => {
                                if outcome.recovered {
                                    recovering_since = Some(tokio::time::Instant::now());
                                }
                            }
                            Ok(None) => break,
                            Err(error) => {
                                fail(&service, &observer, error);
                                break;
                            }
                        }
                    }
                }
            }
        });
        SchedulerHandle {
            cancellation,
            join: Some(join),
        }
    }
}

async fn retry_operation<T, F, Fut>(
    cancellation: &CancellationToken,
    observer: &SchedulerRuntimeObserver,
    mut operation: F,
) -> Result<Option<RetryOutcome<T>>, SchedulerError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, SchedulerError>>,
{
    let mut retries = 0_usize;
    loop {
        match operation().await {
            Ok(value) => {
                return Ok(Some(RetryOutcome {
                    value,
                    recovered: retries > 0,
                }));
            }
            Err(error) if retries < RETRY_DELAYS.len() => {
                let error_code = scheduler_error_code(&error);
                retries += 1;
                observer(SchedulerRuntimeEvent::Retrying {
                    attempt: retries as u32,
                    error_code,
                });
                tokio::select! {
                    () = cancellation.cancelled() => return Ok(None),
                    () = tokio::time::sleep(RETRY_DELAYS[retries - 1]) => {}
                }
            }
            Err(error) => return Err(error),
        }
    }
}

fn fail(service: &SchedulerService, observer: &SchedulerRuntimeObserver, error: SchedulerError) {
    service.accepting.store(false, Ordering::SeqCst);
    observer(SchedulerRuntimeEvent::Failed {
        error_code: scheduler_error_code(&error),
        detail: error.to_string(),
    });
}

fn scheduler_error_code(error: &SchedulerError) -> &'static str {
    match error {
        SchedulerError::Store(_) => "scheduler_storage_unavailable",
        SchedulerError::Serialization(_) => "scheduler_state_invalid",
        SchedulerError::InvalidSchedule(_) | SchedulerError::NoFutureOccurrence => {
            "scheduler_schedule_invalid"
        }
        SchedulerError::Unavailable => "scheduler_unavailable",
    }
}

#[derive(Debug)]
pub struct SchedulerHandle {
    cancellation: CancellationToken,
    join: Option<tokio::task::JoinHandle<()>>,
}

impl SchedulerHandle {
    pub async fn stop(mut self) {
        self.cancellation.cancel();
        if let Some(join) = self.join.take() {
            let _ = join.await;
        }
    }
}

impl Drop for SchedulerHandle {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}
