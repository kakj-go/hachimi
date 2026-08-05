use super::*;

impl AgentStore {
    pub async fn create_project(
        &self,
        project: &ProjectRecord,
    ) -> Result<ProjectRecord, AgentStoreError> {
        sqlx::query(
            "INSERT INTO projects (id, display_name, root_path, git_root, trusted, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(project.id.as_str())
        .bind(&project.display_name)
        .bind(&project.root_path)
        .bind(&project.git_root)
        .bind(project.trusted)
        .bind(project.created_at_ms)
        .bind(project.updated_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(project.clone())
    }

    pub async fn list_projects(&self) -> Result<Vec<ProjectRecord>, AgentStoreError> {
        let rows = sqlx::query("SELECT * FROM projects ORDER BY updated_at_ms DESC, id ASC")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(project_from_row).collect()
    }

    pub async fn get_project(
        &self,
        project_id: &hachimi_protocol::ProjectId,
    ) -> Result<Option<ProjectRecord>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM projects WHERE id = ?")
            .bind(project_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(project_from_row).transpose()
    }

    pub async fn update_project_display_name(
        &self,
        project_id: &hachimi_protocol::ProjectId,
        display_name: &str,
        updated_at_ms: i64,
    ) -> Result<ProjectRecord, AgentStoreError> {
        let display_name = display_name.trim();
        if display_name.is_empty() || display_name.chars().count() > 120 {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "project display name",
                value: "display name must contain 1-120 characters".into(),
            });
        }
        let changed =
            sqlx::query("UPDATE projects SET display_name = ?, updated_at_ms = ? WHERE id = ?")
                .bind(display_name)
                .bind(updated_at_ms)
                .bind(project_id.as_str())
                .execute(&self.pool)
                .await?;
        if changed.rows_affected() != 1 {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "project id",
                value: project_id.to_string(),
            });
        }
        self.get_project(project_id)
            .await?
            .ok_or_else(|| AgentStoreError::InvalidPersistedValue {
                kind: "project id",
                value: project_id.to_string(),
            })
    }

    pub async fn update_project_git_root(
        &self,
        project_id: &hachimi_protocol::ProjectId,
        git_root: Option<&str>,
        updated_at_ms: i64,
    ) -> Result<ProjectRecord, AgentStoreError> {
        let changed = sqlx::query(
            "UPDATE projects SET git_root = ?, updated_at_ms = CASE WHEN git_root IS ? THEN updated_at_ms ELSE ? END WHERE id = ?",
        )
        .bind(git_root)
        .bind(git_root)
        .bind(updated_at_ms)
        .bind(project_id.as_str())
        .execute(&self.pool)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "project id",
                value: project_id.to_string(),
            });
        }
        self.get_project(project_id)
            .await?
            .ok_or_else(|| AgentStoreError::InvalidPersistedValue {
                kind: "project id",
                value: project_id.to_string(),
            })
    }

    pub async fn upsert_attachment(
        &self,
        attachment: &AttachmentRecord,
        managed_path: &Path,
    ) -> Result<AttachmentRecord, AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        if let Some(row) = sqlx::query("SELECT * FROM attachments WHERE content_hash = ?")
            .bind(&attachment.content_hash)
            .fetch_optional(&mut *transaction)
            .await?
        {
            let existing = attachment_from_row(&row)?;
            transaction.commit().await?;
            return Ok(existing);
        }
        sqlx::query(
            "INSERT INTO attachments (id, content_hash, original_name, mime_type, byte_size, managed_path, created_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(attachment.id.as_str())
        .bind(&attachment.content_hash)
        .bind(&attachment.original_name)
        .bind(&attachment.mime_type)
        .bind(i64::try_from(attachment.byte_size).unwrap_or(i64::MAX))
        .bind(managed_path.to_string_lossy().as_ref())
        .bind(attachment.created_at_ms)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(attachment.clone())
    }

    pub async fn get_attachment(
        &self,
        attachment_id: &AttachmentId,
    ) -> Result<Option<AttachmentRecord>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM attachments WHERE id = ?")
            .bind(attachment_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(attachment_from_row).transpose()
    }

    pub async fn attach_to_run(
        &self,
        run_id: &RunId,
        attachment_ids: &[AttachmentId],
    ) -> Result<(), AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        for attachment_id in attachment_ids {
            let exists =
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM attachments WHERE id = ?")
                    .bind(attachment_id.as_str())
                    .fetch_one(&mut *transaction)
                    .await?
                    > 0;
            if !exists {
                return Err(AgentStoreError::AttachmentNotFound(attachment_id.clone()));
            }
            sqlx::query(
                "INSERT OR IGNORE INTO run_attachments (run_id, attachment_id) VALUES (?, ?)",
            )
            .bind(run_id.as_str())
            .bind(attachment_id.as_str())
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn attach_to_run_input(
        &self,
        run_id: &RunId,
        attachment_ids: &[AttachmentId],
    ) -> Result<(), AgentStoreError> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT id, payload_json FROM transcript_items WHERE run_id = ? AND kind = 'user' ORDER BY sequence ASC LIMIT 1",
        )
        .bind(run_id.as_str())
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| AgentStoreError::InvalidPersistedValue {
            kind: "run user input",
            value: run_id.to_string(),
        })?;
        let mut payload: ItemPayload = serde_json::from_str(row.get("payload_json"))?;
        let ItemPayload::User {
            attachment_ids: persisted,
            ..
        } = &mut payload
        else {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "run user input",
                value: run_id.to_string(),
            });
        };
        for attachment_id in attachment_ids {
            let exists =
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM attachments WHERE id = ?")
                    .bind(attachment_id.as_str())
                    .fetch_one(&mut *transaction)
                    .await?
                    > 0;
            if !exists {
                return Err(AgentStoreError::AttachmentNotFound(attachment_id.clone()));
            }
            sqlx::query(
                "INSERT OR IGNORE INTO run_attachments (run_id, attachment_id) VALUES (?, ?)",
            )
            .bind(run_id.as_str())
            .bind(attachment_id.as_str())
            .execute(&mut *transaction)
            .await?;
            if !persisted.contains(attachment_id) {
                persisted.push(attachment_id.clone());
            }
        }
        sqlx::query("UPDATE transcript_items SET payload_json = ? WHERE id = ?")
            .bind(serde_json::to_string(&payload)?)
            .bind(row.get::<String, _>("id"))
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn list_run_managed_attachments(
        &self,
        run_id: &RunId,
    ) -> Result<Vec<ManagedAttachmentRecord>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT attachments.* FROM attachments INNER JOIN run_attachments ON run_attachments.attachment_id = attachments.id WHERE run_attachments.run_id = ? ORDER BY attachments.created_at_ms ASC, attachments.id ASC",
        )
        .bind(run_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(ManagedAttachmentRecord {
                    attachment: attachment_from_row(row)?,
                    managed_path: PathBuf::from(row.get::<String, _>("managed_path")),
                })
            })
            .collect()
    }
}
