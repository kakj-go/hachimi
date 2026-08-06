use hachimi_protocol::{
    AgentTaskCollection, AgentTaskId, AgentTaskMessageId, AgentTaskMessageRecord, AgentTaskRecord,
    AgentTaskStatus, ArtifactId, ItemId, ItemPayload, RunBudget, RunEventPayload, RunId, SessionId,
};
use serde_json::json;
use sqlx::Row;

use super::{AgentStore, AgentStoreError};

const MAX_AGENT_DEPTH: u8 = 3;
const MAX_CHILDREN_PER_PARENT: i64 = 16;
const MAX_ACTIVE_CHILDREN_PER_ROOT: i64 = 4;

#[derive(Debug, Clone, PartialEq)]
pub struct AgentTaskExecutionClaim {
    pub task: AgentTaskRecord,
    pub execution_generation: u64,
    pub lease_owner: String,
    pub lease_expires_at_ms: i64,
}

impl AgentStore {
    pub async fn link_agent_task_transcript_item(
        &self,
        task_id: &AgentTaskId,
        item_id: &ItemId,
    ) -> Result<(), AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query("SELECT relations_json FROM transcript_items WHERE id = ? AND kind = 'collab_tool_call'")
            .bind(item_id.as_str())
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| AgentStoreError::InvalidPersistedValue {
                kind: "collab transcript item",
                value: item_id.to_string(),
            })?;
        let mut relations: hachimi_protocol::ItemRelations =
            serde_json::from_str(row.get("relations_json"))?;
        relations.agent_task_id = Some(task_id.clone());
        sqlx::query(
            "INSERT INTO agent_task_transcript_items (agent_task_id, item_id) VALUES (?, ?) ON CONFLICT(agent_task_id) DO UPDATE SET item_id = excluded.item_id",
        )
        .bind(task_id.as_str())
        .bind(item_id.as_str())
        .execute(&mut *transaction)
        .await?;
        sqlx::query("UPDATE transcript_items SET relations_json = ? WHERE id = ?")
            .bind(serde_json::to_string(&relations)?)
            .bind(item_id.as_str())
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn create_agent_task(
        &self,
        task: &AgentTaskRecord,
    ) -> Result<AgentTaskRecord, AgentStoreError> {
        if task.status != AgentTaskStatus::Queued
            || task.depth == 0
            || task.depth > MAX_AGENT_DEPTH
            || task.title.trim().is_empty()
            || task.title.chars().count() > 200
        {
            return Err(AgentStoreError::AgentTaskLimitExceeded);
        }
        let mut transaction = self.pool.begin().await?;
        let direct_children: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM agent_tasks WHERE parent_run_id = ?")
                .bind(task.parent_run_id.as_str())
                .fetch_one(&mut *transaction)
                .await?;
        let active_root: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_tasks WHERE root_run_id = ? AND status IN ('queued', 'running', 'waiting')",
        )
        .bind(task.root_run_id.as_str())
        .fetch_one(&mut *transaction)
        .await?;
        if direct_children >= MAX_CHILDREN_PER_PARENT || active_root >= MAX_ACTIVE_CHILDREN_PER_ROOT
        {
            return Err(AgentStoreError::AgentTaskLimitExceeded);
        }

        let parent_run =
            sqlx::query("SELECT session_id, configuration_json FROM runs WHERE id = ?")
                .bind(task.parent_run_id.as_str())
                .fetch_optional(&mut *transaction)
                .await?
                .ok_or_else(|| AgentStoreError::RunNotFound(task.parent_run_id.clone()))?;
        if parent_run.get::<String, _>("session_id") != task.parent_session_id.as_str() {
            return Err(AgentStoreError::AgentTaskLimitExceeded);
        }
        let parent_configuration: hachimi_protocol::RunConfiguration =
            serde_json::from_str(parent_run.get("configuration_json"))?;
        validate_budget_reservation(
            &mut transaction,
            &task.parent_run_id,
            &parent_configuration.budget,
            &task.reserved_budget,
        )
        .await?;

        match task.parent_task_id.as_ref() {
            Some(parent_task_id) => {
                let parent = sqlx::query(
                    "SELECT root_task_id, root_run_id, child_run_id, depth FROM agent_tasks WHERE id = ?",
                )
                .bind(parent_task_id.as_str())
                .fetch_optional(&mut *transaction)
                .await?
                .ok_or_else(|| AgentStoreError::AgentTaskNotFound(parent_task_id.clone()))?;
                if parent.get::<String, _>("root_task_id") != task.root_task_id.as_str()
                    || parent.get::<String, _>("root_run_id") != task.root_run_id.as_str()
                    || parent.get::<String, _>("child_run_id") != task.parent_run_id.as_str()
                    || parent.get::<i64, _>("depth") + 1 != i64::from(task.depth)
                {
                    return Err(AgentStoreError::AgentTaskLimitExceeded);
                }
            }
            None => {
                if task.depth != 1
                    || task.root_task_id != task.id
                    || task.root_run_id != task.parent_run_id
                {
                    return Err(AgentStoreError::AgentTaskLimitExceeded);
                }
            }
        }
        let child = sqlx::query("SELECT session_id FROM runs WHERE id = ?")
            .bind(task.child_run_id.as_str())
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| AgentStoreError::RunNotFound(task.child_run_id.clone()))?;
        if child.get::<String, _>("session_id") != task.child_session_id.as_str() {
            return Err(AgentStoreError::AgentTaskLimitExceeded);
        }

        sqlx::query(
            "INSERT INTO agent_tasks (id, root_task_id, root_run_id, parent_task_id, parent_session_id, parent_run_id, child_session_id, child_run_id, title, depth, status, reserved_budget_json, usage_json, artifact_ids_json, result_summary, error_code, created_at_ms, started_at_ms, finished_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(task.id.as_str())
        .bind(task.root_task_id.as_str())
        .bind(task.root_run_id.as_str())
        .bind(task.parent_task_id.as_ref().map(AgentTaskId::as_str))
        .bind(task.parent_session_id.as_str())
        .bind(task.parent_run_id.as_str())
        .bind(task.child_session_id.as_str())
        .bind(task.child_run_id.as_str())
        .bind(task.title.trim())
        .bind(i64::from(task.depth))
        .bind(task.status.as_str())
        .bind(serde_json::to_string(&task.reserved_budget)?)
        .bind(serde_json::to_string(&task.usage)?)
        .bind(serde_json::to_string(&task.artifact_ids)?)
        .bind(&task.result_summary)
        .bind(&task.error_code)
        .bind(task.created_at_ms)
        .bind(task.started_at_ms)
        .bind(task.finished_at_ms)
        .bind(task.updated_at_ms)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(task.clone())
    }

    pub async fn get_agent_task(
        &self,
        task_id: &AgentTaskId,
    ) -> Result<Option<AgentTaskRecord>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM agent_tasks WHERE id = ?")
            .bind(task_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(task_from_row).transpose()
    }

    pub async fn get_agent_task_by_child_run(
        &self,
        child_run_id: &RunId,
    ) -> Result<Option<AgentTaskRecord>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM agent_tasks WHERE child_run_id = ?")
            .bind(child_run_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(task_from_row).transpose()
    }

    pub async fn list_agent_tasks_for_parent(
        &self,
        parent_run_id: &RunId,
    ) -> Result<Vec<AgentTaskRecord>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM agent_tasks WHERE parent_run_id = ? ORDER BY created_at_ms, id",
        )
        .bind(parent_run_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(task_from_row).collect()
    }

    pub async fn list_agent_tasks_for_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<AgentTaskRecord>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM agent_tasks WHERE parent_session_id = ? ORDER BY created_at_ms, id",
        )
        .bind(session_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(task_from_row).collect()
    }

    pub async fn list_nonterminal_agent_tasks(
        &self,
    ) -> Result<Vec<AgentTaskRecord>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM agent_tasks WHERE status NOT IN ('succeeded', 'failed', 'cancelled') ORDER BY depth, created_at_ms, id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(task_from_row).collect()
    }

    pub async fn claim_agent_task_execution(
        &self,
        task_id: &AgentTaskId,
        lease_owner: &str,
        now_ms: i64,
        lease_duration_ms: i64,
    ) -> Result<Option<AgentTaskExecutionClaim>, AgentStoreError> {
        if lease_owner.trim().is_empty() || lease_duration_ms <= 0 {
            return Err(AgentStoreError::InvalidAgentTaskTransition);
        }
        let lease_expires_at_ms = now_ms.saturating_add(lease_duration_ms);
        let mut transaction = self.pool.begin().await?;
        let changed = sqlx::query(
            "UPDATE agent_tasks SET execution_generation = execution_generation + 1, lease_owner = ?, lease_expires_at_ms = ?, last_reconciled_at_ms = ?, updated_at_ms = MAX(updated_at_ms, ?) WHERE id = ? AND status IN ('queued', 'running', 'waiting') AND (lease_owner IS NULL OR lease_expires_at_ms IS NULL OR lease_expires_at_ms <= ?)",
        )
        .bind(lease_owner)
        .bind(lease_expires_at_ms)
        .bind(now_ms)
        .bind(now_ms)
        .bind(task_id.as_str())
        .bind(now_ms)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if changed != 1 {
            transaction.commit().await?;
            return Ok(None);
        }
        let row = sqlx::query("SELECT * FROM agent_tasks WHERE id = ?")
            .bind(task_id.as_str())
            .fetch_one(&mut *transaction)
            .await?;
        let execution_generation = u64::try_from(row.get::<i64, _>("execution_generation"))
            .map_err(|_| AgentStoreError::InvalidPersistedValue {
                kind: "agent task execution generation",
                value: row.get::<i64, _>("execution_generation").to_string(),
            })?;
        let task = task_from_row(&row)?;
        transaction.commit().await?;
        Ok(Some(AgentTaskExecutionClaim {
            task,
            execution_generation,
            lease_owner: lease_owner.to_owned(),
            lease_expires_at_ms,
        }))
    }

    pub async fn renew_agent_task_execution_lease(
        &self,
        task_id: &AgentTaskId,
        execution_generation: u64,
        lease_owner: &str,
        now_ms: i64,
        lease_duration_ms: i64,
    ) -> Result<bool, AgentStoreError> {
        if lease_duration_ms <= 0 {
            return Ok(false);
        }
        Ok(sqlx::query(
            "UPDATE agent_tasks SET lease_expires_at_ms = ?, last_reconciled_at_ms = ? WHERE id = ? AND execution_generation = ? AND lease_owner = ? AND status IN ('queued', 'running', 'waiting')",
        )
        .bind(now_ms.saturating_add(lease_duration_ms))
        .bind(now_ms)
        .bind(task_id.as_str())
        .bind(i64::try_from(execution_generation).unwrap_or(i64::MAX))
        .bind(lease_owner)
        .execute(&self.pool)
        .await?
        .rows_affected()
            == 1)
    }

    pub async fn release_agent_task_execution_lease(
        &self,
        task_id: &AgentTaskId,
        execution_generation: u64,
        lease_owner: &str,
        reconciled_at_ms: i64,
    ) -> Result<bool, AgentStoreError> {
        Ok(sqlx::query(
            "UPDATE agent_tasks SET lease_owner = NULL, lease_expires_at_ms = NULL, last_reconciled_at_ms = ?, updated_at_ms = MAX(updated_at_ms, ?) WHERE id = ? AND execution_generation = ? AND lease_owner = ?",
        )
        .bind(reconciled_at_ms)
        .bind(reconciled_at_ms)
        .bind(task_id.as_str())
        .bind(i64::try_from(execution_generation).unwrap_or(i64::MAX))
        .bind(lease_owner)
        .execute(&self.pool)
        .await?
        .rows_affected()
            == 1)
    }

    pub async fn list_agent_task_subtree(
        &self,
        task_id: &AgentTaskId,
    ) -> Result<Vec<AgentTaskRecord>, AgentStoreError> {
        let rows = sqlx::query(
            "WITH RECURSIVE subtree(id) AS (SELECT id FROM agent_tasks WHERE id = ? UNION ALL SELECT child.id FROM agent_tasks child JOIN subtree parent ON child.parent_task_id = parent.id) SELECT task.* FROM agent_tasks task JOIN subtree ON subtree.id = task.id ORDER BY task.depth DESC, task.created_at_ms DESC, task.id DESC",
        )
        .bind(task_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(task_from_row).collect()
    }

    pub async fn transition_agent_task(
        &self,
        task_id: &AgentTaskId,
        next: AgentTaskStatus,
        result_summary: Option<&str>,
        error_code: Option<&str>,
        updated_at_ms: i64,
    ) -> Result<AgentTaskRecord, AgentStoreError> {
        let current = self
            .get_agent_task(task_id)
            .await?
            .ok_or_else(|| AgentStoreError::AgentTaskNotFound(task_id.clone()))?;
        if !can_transition(current.status, next) {
            return Err(AgentStoreError::InvalidAgentTaskTransition);
        }
        let started_at_ms = (next == AgentTaskStatus::Running)
            .then_some(updated_at_ms)
            .or(current.started_at_ms);
        let finished_at_ms = next.is_terminal().then_some(updated_at_ms);
        sqlx::query(
            "UPDATE agent_tasks SET status = ?, result_summary = COALESCE(?, result_summary), error_code = ?, started_at_ms = ?, finished_at_ms = ?, lease_owner = CASE WHEN ? IN ('succeeded', 'failed', 'cancelled', 'needs_attention') THEN NULL ELSE lease_owner END, lease_expires_at_ms = CASE WHEN ? IN ('succeeded', 'failed', 'cancelled', 'needs_attention') THEN NULL ELSE lease_expires_at_ms END, last_reconciled_at_ms = ?, updated_at_ms = ? WHERE id = ? AND status = ?",
        )
        .bind(next.as_str())
        .bind(result_summary.map(|value| value.chars().take(32_000).collect::<String>()))
        .bind(error_code)
        .bind(started_at_ms)
        .bind(finished_at_ms)
        .bind(next.as_str())
        .bind(next.as_str())
        .bind(updated_at_ms)
        .bind(updated_at_ms)
        .bind(task_id.as_str())
        .bind(current.status.as_str())
        .execute(&self.pool)
        .await?;
        let task = self
            .get_agent_task(task_id)
            .await?
            .ok_or_else(|| AgentStoreError::AgentTaskNotFound(task_id.clone()))?;
        self.sync_agent_task_transcript_item(&task).await?;
        Ok(task)
    }

    pub async fn reconcile_agent_task_from_run(
        &self,
        task_id: &AgentTaskId,
        updated_at_ms: i64,
    ) -> Result<AgentTaskRecord, AgentStoreError> {
        let task = self
            .get_agent_task(task_id)
            .await?
            .ok_or_else(|| AgentStoreError::AgentTaskNotFound(task_id.clone()))?;
        let run = self
            .get_run(&task.child_run_id)
            .await?
            .ok_or_else(|| AgentStoreError::RunNotFound(task.child_run_id.clone()))?;
        let next = match run.status {
            hachimi_protocol::RunStatus::Queued | hachimi_protocol::RunStatus::Preparing => {
                AgentTaskStatus::Queued
            }
            hachimi_protocol::RunStatus::Running
            | hachimi_protocol::RunStatus::Recovering
            | hachimi_protocol::RunStatus::Cancelling => AgentTaskStatus::Running,
            hachimi_protocol::RunStatus::WaitingApproval
            | hachimi_protocol::RunStatus::WaitingUserInput
            | hachimi_protocol::RunStatus::WaitingRecoveryDecision
            | hachimi_protocol::RunStatus::Interrupted => AgentTaskStatus::NeedsAttention,
            hachimi_protocol::RunStatus::Succeeded => AgentTaskStatus::Succeeded,
            hachimi_protocol::RunStatus::Cancelled => AgentTaskStatus::Cancelled,
            hachimi_protocol::RunStatus::Failed
            | hachimi_protocol::RunStatus::TimedOut
            | hachimi_protocol::RunStatus::Lost => AgentTaskStatus::Failed,
        };
        let usage = self
            .get_run_usage_snapshot(&task.child_run_id)
            .await?
            .map(|snapshot| snapshot.billed_usage)
            .unwrap_or_default();
        let artifact_rows =
            sqlx::query("SELECT id FROM artifacts WHERE run_id = ? ORDER BY created_at_ms, id")
                .bind(task.child_run_id.as_str())
                .fetch_all(&self.pool)
                .await?;
        let artifact_ids = artifact_rows
            .iter()
            .map(|row| ArtifactId::new(row.get::<String, _>("id")))
            .collect::<Vec<_>>();
        let result_summary =
            latest_assistant_summary(self, &task.child_session_id, &task.child_run_id).await?;
        sqlx::query(
            "UPDATE agent_tasks SET status = ?, usage_json = ?, artifact_ids_json = ?, result_summary = COALESCE(?, result_summary), error_code = ?, started_at_ms = CASE WHEN ? = 'running' THEN COALESCE(started_at_ms, ?) ELSE started_at_ms END, finished_at_ms = CASE WHEN ? IN ('succeeded', 'failed', 'cancelled') THEN COALESCE(finished_at_ms, ?) ELSE finished_at_ms END, lease_owner = CASE WHEN ? IN ('succeeded', 'failed', 'cancelled', 'needs_attention') THEN NULL ELSE lease_owner END, lease_expires_at_ms = CASE WHEN ? IN ('succeeded', 'failed', 'cancelled', 'needs_attention') THEN NULL ELSE lease_expires_at_ms END, last_reconciled_at_ms = ?, updated_at_ms = ? WHERE id = ?",
        )
        .bind(next.as_str())
        .bind(serde_json::to_string(&usage)?)
        .bind(serde_json::to_string(&artifact_ids)?)
        .bind(result_summary)
        .bind(&run.failure_code)
        .bind(next.as_str())
        .bind(updated_at_ms)
        .bind(next.as_str())
        .bind(updated_at_ms)
        .bind(next.as_str())
        .bind(next.as_str())
        .bind(updated_at_ms)
        .bind(updated_at_ms)
        .bind(task_id.as_str())
        .execute(&self.pool)
        .await?;
        let task = self
            .get_agent_task(task_id)
            .await?
            .ok_or_else(|| AgentStoreError::AgentTaskNotFound(task_id.clone()))?;
        self.sync_agent_task_transcript_item(&task).await?;
        Ok(task)
    }

    async fn sync_agent_task_transcript_item(
        &self,
        task: &AgentTaskRecord,
    ) -> Result<(), AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let Some(row) = sqlx::query(
            "SELECT transcript_items.* FROM transcript_items JOIN agent_task_transcript_items ON agent_task_transcript_items.item_id = transcript_items.id WHERE agent_task_transcript_items.agent_task_id = ?",
        )
        .bind(task.id.as_str())
        .fetch_optional(&mut *transaction)
        .await?
        else {
            transaction.commit().await?;
            return Ok(());
        };
        let session_id = SessionId::new(row.get::<String, _>("session_id"));
        let mut item = super::transcript_item_from_row(&row, &session_id)?;
        let ItemPayload::CollabToolCall {
            tool_name,
            parent_run_id,
            ..
        } = item.payload
        else {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "agent task transcript payload",
                value: item.id.to_string(),
            });
        };
        item.payload = ItemPayload::CollabToolCall {
            tool_name,
            agent_task_id: Some(task.id.clone()),
            parent_run_id,
            child_run_id: Some(task.child_run_id.clone()),
            title: task.title.clone(),
            status: task.status.as_str().to_owned(),
            summary: task.result_summary.clone(),
            usage: task.usage,
        };
        item.relations.agent_task_id = Some(task.id.clone());
        item.relations.artifact_ids = task.artifact_ids.clone();
        sqlx::query(
            "UPDATE transcript_items SET payload_json = ?, relations_json = ? WHERE id = ?",
        )
        .bind(serde_json::to_string(&item.payload)?)
        .bind(serde_json::to_string(&item.relations)?)
        .bind(item.id.as_str())
        .execute(&mut *transaction)
        .await?;
        super::append_event_typed_tx(
            &mut transaction,
            &item.session_id,
            item.run_id.as_ref(),
            "item.completed",
            Some(RunEventPayload::ItemCompleted {
                item: Box::new(item.clone()),
            }),
            json!({ "itemId": item.id, "status": item.status }),
            task.updated_at_ms,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn append_agent_task_message(
        &self,
        message: &AgentTaskMessageRecord,
    ) -> Result<AgentTaskMessageRecord, AgentStoreError> {
        let task = self
            .get_agent_task(&message.task_id)
            .await?
            .ok_or_else(|| AgentStoreError::AgentTaskNotFound(message.task_id.clone()))?;
        if message.content.trim().is_empty()
            || message.content.chars().count() > 32_000
            || message.recipient_run_id != task.child_run_id
            || message.sender_run_id != task.parent_run_id
        {
            return Err(AgentStoreError::AgentTaskLimitExceeded);
        }
        sqlx::query(
            "INSERT INTO agent_task_messages (id, task_id, sender_run_id, recipient_run_id, content, created_at_ms, delivered_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(message.id.as_str())
        .bind(message.task_id.as_str())
        .bind(message.sender_run_id.as_str())
        .bind(message.recipient_run_id.as_str())
        .bind(message.content.trim())
        .bind(message.created_at_ms)
        .bind(message.delivered_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(message.clone())
    }

    pub async fn collect_agent_tasks(
        &self,
        parent_run_id: &RunId,
    ) -> Result<AgentTaskCollection, AgentStoreError> {
        let tasks = self.list_agent_tasks_for_parent(parent_run_id).await?;
        let rows = sqlx::query(
            "SELECT m.* FROM agent_task_messages m JOIN agent_tasks t ON t.id = m.task_id WHERE t.parent_run_id = ? ORDER BY m.created_at_ms, m.id",
        )
        .bind(parent_run_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        let messages = rows
            .iter()
            .map(message_from_row)
            .collect::<Result<_, _>>()?;
        Ok(AgentTaskCollection { tasks, messages })
    }
}

async fn validate_budget_reservation(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    parent_run_id: &RunId,
    parent: &RunBudget,
    requested: &RunBudget,
) -> Result<(), AgentStoreError> {
    if requested.max_model_requests == 0
        || requested.max_tool_calls == 0
        || requested.max_model_requests > parent.max_model_requests
        || requested.max_tool_calls > parent.max_tool_calls
        || requested.max_parallel_read_tools > parent.max_parallel_read_tools
        || requested.model_timeout_ms > parent.model_timeout_ms
        || requested.tool_timeout_ms > parent.tool_timeout_ms
    {
        return Err(AgentStoreError::AgentTaskLimitExceeded);
    }
    let rows = sqlx::query(
        "SELECT reserved_budget_json FROM agent_tasks WHERE parent_run_id = ? AND status NOT IN ('succeeded', 'failed', 'cancelled')",
    )
    .bind(parent_run_id.as_str())
    .fetch_all(&mut **transaction)
    .await?;
    let mut reserved_model = requested.max_model_requests;
    let mut reserved_tools = requested.max_tool_calls;
    for row in rows {
        let budget: RunBudget = serde_json::from_str(row.get("reserved_budget_json"))?;
        reserved_model = reserved_model.saturating_add(budget.max_model_requests);
        reserved_tools = reserved_tools.saturating_add(budget.max_tool_calls);
    }
    if reserved_model > parent.max_model_requests || reserved_tools > parent.max_tool_calls {
        return Err(AgentStoreError::AgentTaskLimitExceeded);
    }
    Ok(())
}

fn can_transition(current: AgentTaskStatus, next: AgentTaskStatus) -> bool {
    current == next
        || matches!(
            (current, next),
            (AgentTaskStatus::Queued, AgentTaskStatus::Running)
                | (AgentTaskStatus::Queued, AgentTaskStatus::NeedsAttention)
                | (AgentTaskStatus::Queued, AgentTaskStatus::Failed)
                | (AgentTaskStatus::Queued, AgentTaskStatus::Cancelled)
                | (AgentTaskStatus::Running, AgentTaskStatus::Waiting)
                | (AgentTaskStatus::Running, AgentTaskStatus::NeedsAttention)
                | (AgentTaskStatus::Running, AgentTaskStatus::Succeeded)
                | (AgentTaskStatus::Running, AgentTaskStatus::Failed)
                | (AgentTaskStatus::Running, AgentTaskStatus::Cancelled)
                | (AgentTaskStatus::Waiting, AgentTaskStatus::Running)
                | (AgentTaskStatus::Waiting, AgentTaskStatus::NeedsAttention)
                | (AgentTaskStatus::Waiting, AgentTaskStatus::Succeeded)
                | (AgentTaskStatus::Waiting, AgentTaskStatus::Failed)
                | (AgentTaskStatus::Waiting, AgentTaskStatus::Cancelled)
                | (AgentTaskStatus::NeedsAttention, AgentTaskStatus::Running)
                | (AgentTaskStatus::NeedsAttention, AgentTaskStatus::Failed)
                | (AgentTaskStatus::NeedsAttention, AgentTaskStatus::Cancelled)
        )
}

async fn latest_assistant_summary(
    store: &AgentStore,
    session_id: &SessionId,
    run_id: &RunId,
) -> Result<Option<String>, AgentStoreError> {
    let row = sqlx::query(
        "SELECT payload_json FROM transcript_items WHERE session_id = ? AND run_id = ? AND kind IN ('assistant', 'plan') AND status = 'completed' ORDER BY sequence DESC LIMIT 1",
    )
    .bind(session_id.as_str())
    .bind(run_id.as_str())
    .fetch_optional(store.pool())
    .await?;
    let Some(row) = row else { return Ok(None) };
    let payload: ItemPayload = serde_json::from_str(row.get("payload_json"))?;
    let summary = match payload {
        ItemPayload::Assistant { text, .. } => text,
        ItemPayload::Plan { text, .. } => text,
        _ => return Ok(None),
    };
    Ok(Some(summary.chars().take(32_000).collect()))
}

fn task_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<AgentTaskRecord, AgentStoreError> {
    let status = row.get::<String, _>("status");
    Ok(AgentTaskRecord {
        id: AgentTaskId::new(row.get::<String, _>("id")),
        root_task_id: AgentTaskId::new(row.get::<String, _>("root_task_id")),
        root_run_id: RunId::new(row.get::<String, _>("root_run_id")),
        parent_task_id: row
            .get::<Option<String>, _>("parent_task_id")
            .map(AgentTaskId::new),
        parent_session_id: SessionId::new(row.get::<String, _>("parent_session_id")),
        parent_run_id: RunId::new(row.get::<String, _>("parent_run_id")),
        child_session_id: SessionId::new(row.get::<String, _>("child_session_id")),
        child_run_id: RunId::new(row.get::<String, _>("child_run_id")),
        title: row.get("title"),
        depth: u8::try_from(row.get::<i64, _>("depth")).unwrap_or(u8::MAX),
        status: AgentTaskStatus::parse(&status).ok_or_else(|| {
            AgentStoreError::InvalidPersistedValue {
                kind: "agent task status",
                value: status,
            }
        })?,
        reserved_budget: serde_json::from_str(row.get("reserved_budget_json"))?,
        usage: serde_json::from_str(row.get("usage_json"))?,
        artifact_ids: serde_json::from_str(row.get("artifact_ids_json"))?,
        result_summary: row.get("result_summary"),
        error_code: row.get("error_code"),
        created_at_ms: row.get("created_at_ms"),
        started_at_ms: row.get("started_at_ms"),
        finished_at_ms: row.get("finished_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

fn message_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<AgentTaskMessageRecord, AgentStoreError> {
    Ok(AgentTaskMessageRecord {
        id: AgentTaskMessageId::new(row.get::<String, _>("id")),
        task_id: AgentTaskId::new(row.get::<String, _>("task_id")),
        sender_run_id: RunId::new(row.get::<String, _>("sender_run_id")),
        recipient_run_id: RunId::new(row.get::<String, _>("recipient_run_id")),
        content: row.get("content"),
        created_at_ms: row.get("created_at_ms"),
        delivered_at_ms: row.get("delivered_at_ms"),
    })
}
