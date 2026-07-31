use std::{future::Future, pin::Pin, sync::Arc};

use hachimi_protocol::{
    ConnectorAccount, ConnectorDriverDescriptor, ConnectorHealth, ConnectorInvocationRequest,
    ConnectorRevision, ConnectorRuntimeKind, PluginId,
};
use hachimi_storage::AgentStore;
use parking_lot::RwLock;
use serde_json::{Value, json};
use sqlx::Row;

use crate::{ExtensionHostError, now_ms, object_argument, string_argument};

pub type ConnectorDriverFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, ExtensionHostError>> + Send + 'a>>;

#[derive(Clone)]
pub struct ConnectorDriverContext {
    pub store: AgentStore,
    pub account: ConnectorAccount,
    pub credential: Option<String>,
}

pub trait ConnectorDriver: Send + Sync {
    fn descriptor(
        &self,
        plugin_id: &PluginId,
        connector_id: &str,
        revision: ConnectorRevision,
    ) -> ConnectorDriverDescriptor;

    fn health<'a>(
        &'a self,
        _context: &'a ConnectorDriverContext,
    ) -> ConnectorDriverFuture<'a, ConnectorHealth> {
        Box::pin(async { Ok(ConnectorHealth::Healthy) })
    }

    fn invoke<'a>(
        &'a self,
        context: ConnectorDriverContext,
        request: &'a ConnectorInvocationRequest,
    ) -> ConnectorDriverFuture<'a, Value>;

    fn webhook<'a>(
        &'a self,
        context: ConnectorDriverContext,
        request: &'a ConnectorInvocationRequest,
    ) -> ConnectorDriverFuture<'a, Value> {
        self.invoke(context, request)
    }

    fn poll<'a>(
        &'a self,
        context: ConnectorDriverContext,
        request: &'a ConnectorInvocationRequest,
    ) -> ConnectorDriverFuture<'a, Value> {
        self.invoke(context, request)
    }

    fn revoke<'a>(&'a self, _context: ConnectorDriverContext) -> ConnectorDriverFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Clone, Default)]
pub struct ConnectorDriverRegistry {
    drivers: Arc<RwLock<std::collections::BTreeMap<String, Arc<dyn ConnectorDriver>>>>,
}

impl std::fmt::Debug for ConnectorDriverRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConnectorDriverRegistry")
            .field(
                "host_identities",
                &self.drivers.read().keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl ConnectorDriverRegistry {
    #[must_use]
    pub fn with_builtin_drivers() -> Self {
        let registry = Self::default();
        registry.register("hachimi.sample-crm.local.v1", Arc::new(SampleCrmDriver));
        for (host_identity, driver) in crate::enterprise_connector::builtin_enterprise_drivers() {
            registry.register(host_identity, driver);
        }
        registry
    }

    pub fn register(&self, host_identity: &str, driver: Arc<dyn ConnectorDriver>) {
        self.drivers
            .write()
            .insert(host_identity.to_owned(), driver);
    }

    pub fn resolve(&self, host_identity: &str) -> Option<Arc<dyn ConnectorDriver>> {
        self.drivers.read().get(host_identity).cloned()
    }
}

#[derive(Debug)]
struct SampleCrmDriver;

impl SampleCrmDriver {
    async fn execute_action(
        context: ConnectorDriverContext,
        request: &ConnectorInvocationRequest,
    ) -> Result<Value, ExtensionHostError> {
        execute_deterministic_action(context, request).await
    }
}

impl ConnectorDriver for SampleCrmDriver {
    fn descriptor(
        &self,
        plugin_id: &PluginId,
        connector_id: &str,
        revision: ConnectorRevision,
    ) -> ConnectorDriverDescriptor {
        ConnectorDriverDescriptor {
            plugin_id: plugin_id.clone(),
            connector_id: connector_id.to_owned(),
            runtime_kind: ConnectorRuntimeKind::Builtin,
            revision,
            actions: vec![
                "get".into(),
                "search".into(),
                "create".into(),
                "update".into(),
                "webhook_emit".into(),
                "webhook_next".into(),
                "poll".into(),
            ],
        }
    }

    fn invoke<'a>(
        &'a self,
        context: ConnectorDriverContext,
        request: &'a ConnectorInvocationRequest,
    ) -> ConnectorDriverFuture<'a, Value> {
        Box::pin(async move { Self::execute_action(context, request).await })
    }
}

async fn execute_deterministic_action(
    context: ConnectorDriverContext,
    request: &ConnectorInvocationRequest,
) -> Result<Value, ExtensionHostError> {
    let mut transaction = context.store.pool().begin().await?;
    let now = now_ms();
    let result = match request.action.as_str() {
        "get" => {
            let record_id = string_argument(&request.arguments, "id")?;
            let row = sqlx::query(
                "SELECT data_json, revision FROM sample_crm_records WHERE account_id = ? AND record_id = ?",
            )
            .bind(request.account_id.as_str())
            .bind(record_id)
            .fetch_optional(&mut *transaction)
            .await?;
            row.map_or(Value::Null, |row| {
                json!({
                    "id": record_id,
                    "data": serde_json::from_str::<Value>(row.get("data_json")).unwrap_or(Value::Null),
                    "revision": row.get::<i64, _>("revision")
                })
            })
        }
        "search" => {
            let query = request
                .arguments
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let rows = sqlx::query(
                "SELECT record_id, data_json, revision FROM sample_crm_records WHERE account_id = ? ORDER BY record_id LIMIT 100",
            )
            .bind(request.account_id.as_str())
            .fetch_all(&mut *transaction)
            .await?;
            Value::Array(
                rows.into_iter()
                    .filter_map(|row| {
                        let data_json = row.get::<String, _>("data_json");
                        (query.is_empty() || data_json.to_ascii_lowercase().contains(&query)).then(|| {
                            json!({
                                "id": row.get::<String, _>("record_id"),
                                "data": serde_json::from_str::<Value>(&data_json).unwrap_or(Value::Null),
                                "revision": row.get::<i64, _>("revision")
                            })
                        })
                    })
                    .collect(),
            )
        }
        "create" => {
            let record_id = string_argument(&request.arguments, "id")?;
            let data = object_argument(&request.arguments, "data")?;
            sqlx::query(
                "INSERT INTO sample_crm_records(account_id, record_id, data_json, revision, updated_at_ms) VALUES(?, ?, ?, 1, ?)",
            )
            .bind(request.account_id.as_str())
            .bind(record_id)
            .bind(serde_json::to_string(data)?)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
            json!({"id": record_id, "data": data, "revision": 1})
        }
        "update" => {
            let record_id = string_argument(&request.arguments, "id")?;
            let data = object_argument(&request.arguments, "data")?;
            let expected_revision = request
                .arguments
                .get("expectedRevision")
                .and_then(Value::as_i64)
                .ok_or(ExtensionHostError::InvalidInvocation)?;
            let updated = sqlx::query(
                "UPDATE sample_crm_records SET data_json = ?, revision = revision + 1, updated_at_ms = ? WHERE account_id = ? AND record_id = ? AND revision = ?",
            )
            .bind(serde_json::to_string(data)?)
            .bind(now)
            .bind(request.account_id.as_str())
            .bind(record_id)
            .bind(expected_revision)
            .execute(&mut *transaction)
            .await?;
            if updated.rows_affected() != 1 {
                return Err(ExtensionHostError::ConnectorDrift);
            }
            json!({"id": record_id, "data": data, "revision": expected_revision + 1})
        }
        "webhook_emit" => {
            let event_id = string_argument(&request.arguments, "eventId")?;
            let payload = request
                .arguments
                .get("payload")
                .cloned()
                .ok_or(ExtensionHostError::InvalidInvocation)?;
            sqlx::query(
                "INSERT OR IGNORE INTO connector_webhook_events(account_id, event_id, payload_json, status, created_at_ms, updated_at_ms) VALUES(?, ?, ?, 'queued', ?, ?)",
            )
            .bind(request.account_id.as_str())
            .bind(event_id)
            .bind(serde_json::to_string(&payload)?)
            .bind(now)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
            json!({"eventId": event_id, "queued": true})
        }
        "webhook_next" => {
            let row = sqlx::query(
                "SELECT event_id, payload_json FROM connector_webhook_events WHERE account_id = ? AND status = 'queued' ORDER BY created_at_ms, event_id LIMIT 1",
            )
            .bind(request.account_id.as_str())
            .fetch_optional(&mut *transaction)
            .await?;
            if let Some(row) = row {
                let event_id = row.get::<String, _>("event_id");
                let payload: Value = serde_json::from_str(row.get("payload_json"))?;
                sqlx::query("UPDATE connector_webhook_events SET status = 'delivered', updated_at_ms = ? WHERE account_id = ? AND event_id = ? AND status = 'queued'")
                    .bind(now)
                    .bind(request.account_id.as_str())
                    .bind(&event_id)
                    .execute(&mut *transaction)
                    .await?;
                json!({"eventId": event_id, "payload": payload})
            } else {
                Value::Null
            }
        }
        "poll" => {
            let cursor =
                sqlx::query("SELECT cursor FROM connector_poll_state WHERE account_id = ?")
                    .bind(request.account_id.as_str())
                    .fetch_optional(&mut *transaction)
                    .await?
                    .map(|row| row.get::<i64, _>("cursor"))
                    .unwrap_or(0);
            let rows = sqlx::query("SELECT rowid, record_id, data_json, revision FROM sample_crm_records WHERE account_id = ? AND rowid > ? ORDER BY rowid LIMIT 100")
                .bind(request.account_id.as_str())
                .bind(cursor)
                .fetch_all(&mut *transaction)
                .await?;
            let next_cursor = rows.last().map_or(cursor, |row| row.get::<i64, _>("rowid"));
            sqlx::query("INSERT INTO connector_poll_state(account_id, cursor, updated_at_ms) VALUES(?, ?, ?) ON CONFLICT(account_id) DO UPDATE SET cursor = excluded.cursor, updated_at_ms = excluded.updated_at_ms")
                .bind(request.account_id.as_str())
                .bind(next_cursor)
                .bind(now)
                .execute(&mut *transaction)
                .await?;
            Value::Array(rows.into_iter().map(|row| json!({
                "id": row.get::<String, _>("record_id"),
                "data": serde_json::from_str::<Value>(row.get("data_json")).unwrap_or(Value::Null),
                "revision": row.get::<i64, _>("revision")
            })).collect())
        }
        _ => return Err(ExtensionHostError::InvalidInvocation),
    };
    transaction.commit().await?;
    Ok(result)
}
