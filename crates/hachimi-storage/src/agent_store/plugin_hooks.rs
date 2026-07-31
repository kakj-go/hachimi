use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, RwLock},
};

use hachimi_protocol::{
    PluginHookEvent, PluginHookInvocation, PluginHookOutcome, RunId, SessionId,
};
use sha2::{Digest, Sha256};
use sqlx::Row;
use tokio_util::sync::CancellationToken;

use super::{AgentStore, AgentStoreError};

const ALLOWED_HOOK_EVENTS: [&str; 6] = [
    "run.before",
    "run.after",
    "tool.before",
    "tool.after",
    "schedule.before",
    "schedule.after",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginHookEventRecord {
    pub event: String,
    pub session_id: Option<SessionId>,
    pub run_id: Option<RunId>,
    pub run_generation: Option<u64>,
    pub subject: String,
    pub result_code: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginHookSubscription {
    pub plugin_id: String,
    pub contribution_id: String,
    pub runtime_revision: String,
}

pub type PluginHookRuntimeFuture<'a> =
    Pin<Box<dyn Future<Output = Result<PluginHookOutcome, String>> + Send + 'a>>;

pub trait PluginHookRuntime: Send + Sync {
    fn invoke<'a>(
        &'a self,
        subscription: &'a PluginHookSubscription,
        invocation: PluginHookInvocation,
        cancellation: CancellationToken,
    ) -> PluginHookRuntimeFuture<'a>;
}

#[derive(Default)]
pub(crate) struct PluginHookRuntimeSlot {
    runtime: RwLock<Option<Arc<dyn PluginHookRuntime>>>,
}

impl std::fmt::Debug for PluginHookRuntimeSlot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PluginHookRuntimeSlot")
            .field(
                "attached",
                &self.runtime.read().is_ok_and(|runtime| runtime.is_some()),
            )
            .finish()
    }
}

impl PluginHookRuntimeSlot {
    fn get(&self) -> Option<Arc<dyn PluginHookRuntime>> {
        self.runtime.read().ok()?.clone()
    }

    fn set(&self, runtime: Arc<dyn PluginHookRuntime>) -> Result<(), AgentStoreError> {
        *self.runtime.write().map_err(|_| {
            AgentStoreError::PluginHook("plugin_hook_runtime_state_poisoned".into())
        })? = Some(runtime);
        Ok(())
    }
}

impl AgentStore {
    pub fn attach_plugin_hook_runtime(
        &self,
        runtime: Arc<dyn PluginHookRuntime>,
    ) -> Result<(), AgentStoreError> {
        self.plugin_hooks.set(runtime)
    }

    /// Executes enabled Hook subscriptions at one fixed lifecycle point. The
    /// runtime receives only lineage plus a hash of the subject; it cannot mutate
    /// Runs, grants, approvals, tool input, or persisted user content.
    pub async fn record_plugin_hook_event(
        &self,
        record: &PluginHookEventRecord,
    ) -> Result<u64, AgentStoreError> {
        self.dispatch_plugin_hook_event(record, CancellationToken::new())
            .await
    }

    pub async fn dispatch_plugin_hook_event(
        &self,
        record: &PluginHookEventRecord,
        cancellation: CancellationToken,
    ) -> Result<u64, AgentStoreError> {
        if !ALLOWED_HOOK_EVENTS.contains(&record.event.as_str())
            || record.subject.is_empty()
            || record.result_code.is_empty()
            || record.result_code.len() > 128
        {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "plugin hook event",
                value: record.event.clone(),
            });
        }
        let subscriptions = sqlx::query(
            "SELECT plugin_id, contribution_id, runtime_revision FROM plugin_hook_subscriptions WHERE event = ? AND enabled = 1 ORDER BY plugin_id, contribution_id",
        )
        .bind(&record.event)
        .fetch_all(&self.pool)
        .await?;
        let subject_hash = Sha256::digest(record.subject.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let event = parse_hook_event(&record.event)?;
        let invocation = PluginHookInvocation {
            event,
            session_id: record.session_id.clone(),
            run_id: record.run_id.clone(),
            run_generation: record.run_generation,
            subject_hash: subject_hash.clone(),
        };
        let runtime = self.plugin_hooks.get();
        for row in &subscriptions {
            let subscription = PluginHookSubscription {
                plugin_id: row.get("plugin_id"),
                contribution_id: row.get("contribution_id"),
                runtime_revision: row.get("runtime_revision"),
            };
            let outcome = if let Some(runtime) = &runtime {
                runtime
                    .invoke(
                        &subscription,
                        invocation.clone(),
                        cancellation.child_token(),
                    )
                    .await
            } else {
                Err("plugin_hook_runtime_unavailable".into())
            };
            let outcome = match outcome.and_then(|outcome| {
                validate_hook_outcome(&outcome)
                    .map(|()| outcome)
                    .map_err(|error| {
                        if let AgentStoreError::PluginHook(code) = error {
                            code
                        } else {
                            "plugin_hook_outcome_invalid".into()
                        }
                    })
            }) {
                Ok(outcome) => outcome,
                Err(code) => {
                    let code = stable_hook_error_code(&code);
                    persist_hook_failure(self, &subscription, record, &subject_hash, &code).await?;
                    return Err(AgentStoreError::PluginHook(code));
                }
            };
            let mut transaction = self.pool.begin().await?;
            sqlx::query(
                "INSERT INTO plugin_hook_executions(plugin_id, contribution_id, event, session_id, run_id, run_generation, subject_hash, result_code, created_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&subscription.plugin_id)
            .bind(&subscription.contribution_id)
            .bind(&record.event)
            .bind(record.session_id.as_ref().map(SessionId::as_str))
            .bind(record.run_id.as_ref().map(RunId::as_str))
            .bind(
                record
                    .run_generation
                    .map(|value| i64::try_from(value).unwrap_or(i64::MAX)),
            )
            .bind(&subject_hash)
            .bind(&outcome.result_code)
            .bind(record.created_at_ms)
            .execute(&mut *transaction)
            .await?;
            transaction.commit().await?;
        }
        Ok(u64::try_from(subscriptions.len()).unwrap_or(u64::MAX))
    }
}

async fn persist_hook_failure(
    store: &AgentStore,
    subscription: &PluginHookSubscription,
    record: &PluginHookEventRecord,
    subject_hash: &str,
    code: &str,
) -> Result<(), AgentStoreError> {
    let mut transaction = store.pool.begin().await?;
    sqlx::query(
        "INSERT INTO plugin_hook_executions(plugin_id, contribution_id, event, session_id, run_id, run_generation, subject_hash, result_code, created_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&subscription.plugin_id)
    .bind(&subscription.contribution_id)
    .bind(&record.event)
    .bind(record.session_id.as_ref().map(SessionId::as_str))
    .bind(record.run_id.as_ref().map(RunId::as_str))
    .bind(
        record
            .run_generation
            .map(|value| i64::try_from(value).unwrap_or(i64::MAX)),
    )
    .bind(subject_hash)
    .bind(code)
    .bind(record.created_at_ms)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE plugin_contribution_runtime SET runtime_state = 'failed', diagnostic = ?, updated_at_ms = ? WHERE plugin_id = ? AND contribution_id = ? AND runtime_revision = ?",
    )
    .bind(code)
    .bind(record.created_at_ms)
    .bind(&subscription.plugin_id)
    .bind(&subscription.contribution_id)
    .bind(&subscription.runtime_revision)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE plugin_hook_subscriptions SET enabled = 0, updated_at_ms = ? WHERE plugin_id = ? AND contribution_id = ?",
    )
    .bind(record.created_at_ms)
    .bind(&subscription.plugin_id)
    .bind(&subscription.contribution_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

fn stable_hook_error_code(value: &str) -> String {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'));
    if valid {
        value.into()
    } else {
        "plugin_hook_runtime_failed".into()
    }
}

fn parse_hook_event(value: &str) -> Result<PluginHookEvent, AgentStoreError> {
    Ok(match value {
        "run.before" => PluginHookEvent::RunBefore,
        "run.after" => PluginHookEvent::RunAfter,
        "tool.before" => PluginHookEvent::ToolBefore,
        "tool.after" => PluginHookEvent::ToolAfter,
        "schedule.before" => PluginHookEvent::ScheduleBefore,
        "schedule.after" => PluginHookEvent::ScheduleAfter,
        _ => {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "plugin hook event",
                value: value.into(),
            });
        }
    })
}

fn validate_hook_outcome(outcome: &PluginHookOutcome) -> Result<(), AgentStoreError> {
    let valid_code = !outcome.result_code.is_empty()
        && outcome.result_code.len() <= 128
        && outcome
            .result_code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'));
    let valid_metadata = outcome.metadata.len() <= 32
        && outcome.metadata.iter().all(|entry| {
            !entry.key.is_empty()
                && entry.key.len() <= 64
                && entry.value.len() <= 512
                && !entry.key.to_ascii_lowercase().contains("secret")
                && !entry.key.to_ascii_lowercase().contains("token")
        });
    if valid_code && valid_metadata {
        Ok(())
    } else {
        Err(AgentStoreError::PluginHook(
            "plugin_hook_outcome_invalid".into(),
        ))
    }
}
