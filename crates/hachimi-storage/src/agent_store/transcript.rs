use hachimi_protocol::{
    ItemId, ItemPayload, ItemStatus, RunEventPayload, TranscriptItem, TranscriptItemKind,
};
use sqlx::Row;

use super::{
    AgentStore, AgentStoreError, append_event_typed_tx, now_ms, transcript_item_from_row,
    transcript_kind_db,
};

impl AgentStore {
    /// Converts a streamed item before completion when the provider emitted a
    /// malformed Plan block. The stable item id lets live clients replace the
    /// provisional Plan projection with ordinary commentary.
    pub async fn complete_transcript_item_as_kind(
        &self,
        item_id: &ItemId,
        kind: TranscriptItemKind,
        status: ItemStatus,
        payload: ItemPayload,
    ) -> Result<TranscriptItem, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query("SELECT * FROM transcript_items WHERE id = ?")
            .bind(item_id.as_str())
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| AgentStoreError::InvalidPersistedValue {
                kind: "transcript item",
                value: item_id.to_string(),
            })?;
        let session_id = hachimi_protocol::SessionId::new(row.get::<String, _>("session_id"));
        let run_id = row
            .get::<Option<String>, _>("run_id")
            .map(hachimi_protocol::RunId::new);
        let current = transcript_item_from_row(&row, &session_id)?;
        if current.status != ItemStatus::InProgress {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "transcript item status",
                value: format!("{:?}", current.status),
            });
        }
        sqlx::query(
            "UPDATE transcript_items SET kind = ?, status = ?, payload_json = ? WHERE id = ? AND status = 'in_progress'",
        )
        .bind(transcript_kind_db(kind))
        .bind(status.as_str())
        .bind(serde_json::to_string(&payload)?)
        .bind(item_id.as_str())
        .execute(&mut *transaction)
        .await?;
        let row = sqlx::query("SELECT * FROM transcript_items WHERE id = ?")
            .bind(item_id.as_str())
            .fetch_one(&mut *transaction)
            .await?;
        let item = transcript_item_from_row(&row, &session_id)?;
        append_event_typed_tx(
            &mut transaction,
            &session_id,
            run_id.as_ref(),
            "item.completed",
            Some(RunEventPayload::ItemCompleted {
                item: Box::new(item.clone()),
            }),
            serde_json::json!({ "itemId": item_id, "status": status }),
            now_ms(),
        )
        .await?;
        transaction.commit().await?;
        self.active_events.complete_item(&session_id, item_id);
        Ok(item)
    }
}
