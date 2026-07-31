use std::path::PathBuf;

use hachimi_protocol::{
    InstalledPlugin, PluginId, PluginLifecycleJournalRecord, PluginLifecycleJournalStatus,
    PluginLifecycleOperation, PluginLifecyclePhase, PluginRevisionHead, PluginRevisionRecord,
    PluginRevisionStatus, PluginStatus, ScheduleHealth,
};
use sqlx::Row;

use super::{ExtensionHostError, PluginHost, hash_bundle, now_ms, persist_plugin};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginLifecycleRecoveryReport {
    pub committed: Vec<String>,
    pub rolled_back: Vec<String>,
    pub failed: Vec<String>,
}

impl PluginHost {
    pub async fn revision_head(
        &self,
        plugin_id: &PluginId,
    ) -> Result<Option<PluginRevisionHead>, ExtensionHostError> {
        sqlx::query(
            "SELECT current_revision, known_good_revision, updated_at_ms FROM plugin_revision_heads WHERE plugin_id = ?",
        )
        .bind(plugin_id.as_str())
        .fetch_optional(self.store.pool())
        .await?
        .map(|row| {
            Ok(PluginRevisionHead {
                plugin_id: plugin_id.clone(),
                current_revision: row.get("current_revision"),
                known_good_revision: row.get("known_good_revision"),
                updated_at_ms: row.get("updated_at_ms"),
            })
        })
        .transpose()
    }

    pub async fn list_revisions(
        &self,
        plugin_id: &PluginId,
    ) -> Result<Vec<PluginRevisionRecord>, ExtensionHostError> {
        let rows = sqlx::query("SELECT revision, manifest_json, content_hash, root_path, plugin_status, status, health_code, created_at_ms, updated_at_ms FROM plugin_revisions WHERE plugin_id = ? ORDER BY created_at_ms DESC, revision DESC")
            .bind(plugin_id.as_str())
            .fetch_all(self.store.pool())
            .await?;
        rows.into_iter()
            .map(|row| decode_revision(plugin_id, row))
            .collect()
    }

    pub async fn lifecycle_journal(
        &self,
        plugin_id: Option<&PluginId>,
    ) -> Result<Vec<PluginLifecycleJournalRecord>, ExtensionHostError> {
        let rows = if let Some(plugin_id) = plugin_id {
            sqlx::query("SELECT id, plugin_id, operation, phase, status, source_revision, candidate_revision, error_code, created_at_ms, updated_at_ms FROM plugin_lifecycle_journal WHERE plugin_id = ? ORDER BY created_at_ms DESC, id DESC")
                .bind(plugin_id.as_str())
                .fetch_all(self.store.pool())
                .await?
        } else {
            sqlx::query("SELECT id, plugin_id, operation, phase, status, source_revision, candidate_revision, error_code, created_at_ms, updated_at_ms FROM plugin_lifecycle_journal ORDER BY created_at_ms DESC, id DESC")
                .fetch_all(self.store.pool())
                .await?
        };
        rows.into_iter().map(decode_journal).collect()
    }

    pub(super) async fn begin_lifecycle(
        &self,
        plugin_id: &PluginId,
        operation: PluginLifecycleOperation,
        source_revision: Option<&str>,
        candidate_revision: Option<&str>,
    ) -> Result<String, ExtensionHostError> {
        let active: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM plugin_lifecycle_journal WHERE plugin_id = ? AND status = 'in_progress'",
        )
        .bind(plugin_id.as_str())
        .fetch_one(self.store.pool())
        .await?;
        if active != 0 {
            return Err(ExtensionHostError::LifecycleConflict);
        }
        let id = uuid::Uuid::now_v7().to_string();
        let now = now_ms();
        sqlx::query("INSERT INTO plugin_lifecycle_journal(id, plugin_id, operation, phase, status, source_revision, candidate_revision, error_code, created_at_ms, updated_at_ms) VALUES(?, ?, ?, 'stage', 'in_progress', ?, ?, NULL, ?, ?)")
            .bind(&id)
            .bind(plugin_id.as_str())
            .bind(lifecycle_operation(operation))
            .bind(source_revision)
            .bind(candidate_revision)
            .bind(now)
            .bind(now)
            .execute(self.store.pool())
            .await?;
        Ok(id)
    }

    pub(super) async fn advance_lifecycle(
        &self,
        id: &str,
        phase: PluginLifecyclePhase,
    ) -> Result<(), ExtensionHostError> {
        let result = sqlx::query("UPDATE plugin_lifecycle_journal SET phase = ?, updated_at_ms = ? WHERE id = ? AND status = 'in_progress'")
            .bind(lifecycle_phase(phase))
            .bind(now_ms())
            .bind(id)
            .execute(self.store.pool())
            .await?;
        if result.rows_affected() != 1 {
            return Err(ExtensionHostError::LifecycleConflict);
        }
        Ok(())
    }

    pub(super) async fn finish_lifecycle(
        &self,
        id: &str,
        status: PluginLifecycleJournalStatus,
        error_code: Option<&str>,
    ) -> Result<(), ExtensionHostError> {
        let phase = if status == PluginLifecycleJournalStatus::RolledBack {
            PluginLifecyclePhase::Rollback
        } else {
            PluginLifecyclePhase::Commit
        };
        sqlx::query("UPDATE plugin_lifecycle_journal SET phase = ?, status = ?, error_code = ?, updated_at_ms = ? WHERE id = ? AND status = 'in_progress'")
            .bind(lifecycle_phase(phase))
            .bind(journal_status(status))
            .bind(error_code)
            .bind(now_ms())
            .bind(id)
            .execute(self.store.pool())
            .await?;
        Ok(())
    }

    pub(super) async fn stage_revision(
        &self,
        plugin: &InstalledPlugin,
    ) -> Result<(), ExtensionHostError> {
        let now = now_ms();
        sqlx::query("INSERT INTO plugin_revisions(plugin_id, revision, manifest_json, content_hash, root_path, plugin_status, status, health_code, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, 'staged', NULL, ?, ?) ON CONFLICT(plugin_id, revision) DO UPDATE SET manifest_json = excluded.manifest_json, root_path = excluded.root_path, plugin_status = excluded.plugin_status, status = CASE WHEN plugin_revisions.status = 'healthy' THEN plugin_revisions.status ELSE 'staged' END, health_code = NULL, updated_at_ms = excluded.updated_at_ms")
            .bind(plugin.manifest.id.as_str())
            .bind(&plugin.content_hash)
            .bind(serde_json::to_string(&plugin.manifest)?)
            .bind(&plugin.content_hash)
            .bind(&plugin.root_path)
            .bind(super::plugin_status(plugin.status))
            .bind(now)
            .bind(now)
            .execute(self.store.pool())
            .await?;
        Ok(())
    }

    pub(super) async fn set_revision_status(
        &self,
        plugin_id: &PluginId,
        revision: &str,
        status: PluginRevisionStatus,
        health_code: Option<&str>,
    ) -> Result<(), ExtensionHostError> {
        let result = sqlx::query("UPDATE plugin_revisions SET status = ?, health_code = ?, updated_at_ms = ? WHERE plugin_id = ? AND revision = ?")
            .bind(revision_status(status))
            .bind(health_code)
            .bind(now_ms())
            .bind(plugin_id.as_str())
            .bind(revision)
            .execute(self.store.pool())
            .await?;
        if result.rows_affected() != 1 {
            return Err(ExtensionHostError::PluginRevisionNotFound);
        }
        Ok(())
    }

    pub(super) async fn commit_revision(
        &self,
        plugin_id: &PluginId,
        revision: &str,
        previous_revision: Option<&str>,
    ) -> Result<(), ExtensionHostError> {
        let now = now_ms();
        let mut tx = self.store.pool().begin().await?;
        if let Some(previous) = previous_revision.filter(|value| *value != revision) {
            sqlx::query("UPDATE plugin_revisions SET status = 'superseded', updated_at_ms = ? WHERE plugin_id = ? AND revision = ? AND status <> 'removed'")
                .bind(now)
                .bind(plugin_id.as_str())
                .bind(previous)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("UPDATE plugin_revisions SET status = 'healthy', health_code = NULL, updated_at_ms = ? WHERE plugin_id = ? AND revision = ?")
            .bind(now)
            .bind(plugin_id.as_str())
            .bind(revision)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO plugin_revision_heads(plugin_id, current_revision, known_good_revision, updated_at_ms) VALUES(?, ?, ?, ?) ON CONFLICT(plugin_id) DO UPDATE SET current_revision = excluded.current_revision, known_good_revision = excluded.known_good_revision, updated_at_ms = excluded.updated_at_ms")
            .bind(plugin_id.as_str())
            .bind(revision)
            .bind(revision)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn rollback(
        &self,
        plugin_id: &PluginId,
        revision: Option<&str>,
    ) -> Result<InstalledPlugin, ExtensionHostError> {
        let current = self
            .get(plugin_id)
            .await?
            .ok_or(ExtensionHostError::PluginNotFound)?;
        let head = self
            .revision_head(plugin_id)
            .await?
            .ok_or(ExtensionHostError::PluginRevisionNotFound)?;
        let target_revision = if let Some(revision) = revision {
            revision.to_owned()
        } else {
            sqlx::query_scalar::<_, String>("SELECT revision FROM plugin_revisions WHERE plugin_id = ? AND revision <> ? AND status = 'superseded' ORDER BY updated_at_ms DESC, created_at_ms DESC LIMIT 1")
                .bind(plugin_id.as_str())
                .bind(&head.current_revision)
                .fetch_optional(self.store.pool())
                .await?
                .or_else(|| {
                    head.known_good_revision
                        .filter(|value| value != &head.current_revision)
                })
                .ok_or(ExtensionHostError::PluginRevisionNotFound)?
        };
        let journal = self
            .begin_lifecycle(
                plugin_id,
                PluginLifecycleOperation::Rollback,
                Some(&current.content_hash),
                Some(&target_revision),
            )
            .await?;
        let outcome = self.restore_revision(plugin_id, &target_revision).await;
        match outcome {
            Ok(restored) => {
                self.finish_lifecycle(&journal, PluginLifecycleJournalStatus::Committed, None)
                    .await?;
                Ok(restored)
            }
            Err(error) => {
                self.finish_lifecycle(
                    &journal,
                    PluginLifecycleJournalStatus::Failed,
                    Some("plugin_rollback_failed"),
                )
                .await?;
                Err(error)
            }
        }
    }

    pub async fn reconcile_lifecycle(
        &self,
    ) -> Result<PluginLifecycleRecoveryReport, ExtensionHostError> {
        let pending = sqlx::query("SELECT id, plugin_id, operation, source_revision, candidate_revision FROM plugin_lifecycle_journal WHERE status = 'in_progress' ORDER BY created_at_ms, id")
            .fetch_all(self.store.pool())
            .await?;
        let mut report = PluginLifecycleRecoveryReport::default();
        for row in pending {
            let id = row.get::<String, _>("id");
            let plugin_id = PluginId::new(row.get::<String, _>("plugin_id"));
            let operation = parse_lifecycle_operation(row.get("operation"))?;
            let source_revision = row.get::<Option<String>, _>("source_revision");
            let candidate_revision = row.get::<Option<String>, _>("candidate_revision");
            let current = self.get(&plugin_id).await?;
            if operation == PluginLifecycleOperation::Uninstall {
                let status = if current.is_none() {
                    PluginLifecycleJournalStatus::Committed
                } else {
                    PluginLifecycleJournalStatus::RolledBack
                };
                self.finish_lifecycle(
                    &id,
                    status,
                    Some(if current.is_none() {
                        "startup_uninstall_completed"
                    } else {
                        "startup_uninstall_rolled_back"
                    }),
                )
                .await?;
                if current.is_none() {
                    report.committed.push(id);
                } else {
                    if let Some(plugin) = current.as_ref() {
                        self.reconcile_contributions(
                            plugin,
                            plugin.status == PluginStatus::Enabled,
                        )
                        .await?;
                    }
                    report.rolled_back.push(id);
                }
                continue;
            }
            let candidate_is_current = current.as_ref().is_some_and(|plugin| {
                candidate_revision.as_deref() == Some(plugin.content_hash.as_str())
                    && PathBuf::from(&plugin.root_path).is_dir()
            });
            if candidate_is_current {
                let candidate = candidate_revision.as_deref().expect("checked candidate");
                self.commit_revision(&plugin_id, candidate, source_revision.as_deref())
                    .await?;
                self.finish_lifecycle(
                    &id,
                    PluginLifecycleJournalStatus::Committed,
                    Some("startup_commit_reconciled"),
                )
                .await?;
                report.committed.push(id);
            } else if let Some(source) = source_revision {
                match self.restore_revision(&plugin_id, &source).await {
                    Ok(_) => {
                        if let Some(candidate) = candidate_revision.as_deref() {
                            let _ = self
                                .set_revision_status(
                                    &plugin_id,
                                    candidate,
                                    PluginRevisionStatus::Failed,
                                    Some("startup_rollback_reconciled"),
                                )
                                .await;
                        }
                        self.finish_lifecycle(
                            &id,
                            PluginLifecycleJournalStatus::RolledBack,
                            Some("startup_rollback_reconciled"),
                        )
                        .await?;
                        report.rolled_back.push(id);
                    }
                    Err(_) => {
                        self.finish_lifecycle(
                            &id,
                            PluginLifecycleJournalStatus::Failed,
                            Some("startup_reconciliation_failed"),
                        )
                        .await?;
                        report.failed.push(id);
                    }
                }
            } else {
                self.finish_lifecycle(
                    &id,
                    PluginLifecycleJournalStatus::Failed,
                    Some("startup_candidate_missing"),
                )
                .await?;
                report.failed.push(id);
            }
        }
        Ok(report)
    }

    async fn restore_revision(
        &self,
        plugin_id: &PluginId,
        revision: &str,
    ) -> Result<InstalledPlugin, ExtensionHostError> {
        let row = sqlx::query("SELECT manifest_json, content_hash, root_path, plugin_status, created_at_ms FROM plugin_revisions WHERE plugin_id = ? AND revision = ? AND status <> 'removed'")
            .bind(plugin_id.as_str())
            .bind(revision)
            .fetch_optional(self.store.pool())
            .await?
            .ok_or(ExtensionHostError::PluginRevisionNotFound)?;
        let root_path = row.get::<String, _>("root_path");
        let content_hash = row.get::<String, _>("content_hash");
        let actual_hash = hash_bundle(&PathBuf::from(&root_path))?.0;
        if actual_hash != content_hash {
            return Err(ExtensionHostError::ContributionDrift);
        }
        let status = super::parse_plugin_status(row.get("plugin_status"))?;
        let restored = InstalledPlugin {
            manifest: serde_json::from_str(row.get("manifest_json"))?,
            content_hash,
            root_path,
            status,
            diagnostics: Vec::new(),
            installed_at_ms: row.get("created_at_ms"),
            updated_at_ms: now_ms(),
        };
        let previous_revision = self
            .revision_head(plugin_id)
            .await?
            .map(|head| head.current_revision);
        persist_plugin(&self.store, &restored).await?;
        self.reconcile_contributions(&restored, status == PluginStatus::Enabled)
            .await?;
        self.commit_revision(plugin_id, revision, previous_revision.as_deref())
            .await?;
        self.mark_stale_schedules(plugin_id, &restored.content_hash)
            .await?;
        Ok(restored)
    }

    pub(super) async fn mark_stale_schedules(
        &self,
        plugin_id: &PluginId,
        active_content_hash: &str,
    ) -> Result<(), ExtensionHostError> {
        let rows = sqlx::query("SELECT id, contribution_revisions_json FROM schedule_definitions")
            .fetch_all(self.store.pool())
            .await?;
        for row in rows {
            let revisions: Vec<hachimi_protocol::ContributionRevision> =
                serde_json::from_str(row.get("contribution_revisions_json"))?;
            let stale = revisions.iter().any(|revision| {
                &revision.plugin_id == plugin_id && revision.content_hash != active_content_hash
            });
            if stale {
                sqlx::query("UPDATE schedule_definitions SET health = ?, health_reason = 'plugin_revision_changed', updated_at_ms = ? WHERE id = ?")
                    .bind(ScheduleHealth::NeedsAttention.as_str())
                    .bind(now_ms())
                    .bind(row.get::<String, _>("id"))
                    .execute(self.store.pool())
                    .await?;
            }
        }
        Ok(())
    }
}

fn decode_revision(
    plugin_id: &PluginId,
    row: sqlx::sqlite::SqliteRow,
) -> Result<PluginRevisionRecord, ExtensionHostError> {
    Ok(PluginRevisionRecord {
        plugin_id: plugin_id.clone(),
        revision: row.get("revision"),
        manifest: serde_json::from_str(row.get("manifest_json"))?,
        content_hash: row.get("content_hash"),
        root_path: row.get("root_path"),
        plugin_status: super::parse_plugin_status(row.get("plugin_status"))?,
        status: parse_revision_status(row.get("status"))?,
        health_code: row.get("health_code"),
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

fn decode_journal(
    row: sqlx::sqlite::SqliteRow,
) -> Result<PluginLifecycleJournalRecord, ExtensionHostError> {
    Ok(PluginLifecycleJournalRecord {
        id: row.get("id"),
        plugin_id: PluginId::new(row.get::<String, _>("plugin_id")),
        operation: parse_lifecycle_operation(row.get("operation"))?,
        phase: parse_lifecycle_phase(row.get("phase"))?,
        status: parse_journal_status(row.get("status"))?,
        source_revision: row.get("source_revision"),
        candidate_revision: row.get("candidate_revision"),
        error_code: row.get("error_code"),
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

const fn revision_status(status: PluginRevisionStatus) -> &'static str {
    match status {
        PluginRevisionStatus::Staged => "staged",
        PluginRevisionStatus::Validated => "validated",
        PluginRevisionStatus::Activating => "activating",
        PluginRevisionStatus::Healthy => "healthy",
        PluginRevisionStatus::Failed => "failed",
        PluginRevisionStatus::Superseded => "superseded",
        PluginRevisionStatus::Removed => "removed",
    }
}

fn parse_revision_status(value: String) -> Result<PluginRevisionStatus, ExtensionHostError> {
    match value.as_str() {
        "staged" => Ok(PluginRevisionStatus::Staged),
        "validated" => Ok(PluginRevisionStatus::Validated),
        "activating" => Ok(PluginRevisionStatus::Activating),
        "healthy" => Ok(PluginRevisionStatus::Healthy),
        "failed" => Ok(PluginRevisionStatus::Failed),
        "superseded" => Ok(PluginRevisionStatus::Superseded),
        "removed" => Ok(PluginRevisionStatus::Removed),
        _ => Err(ExtensionHostError::InvalidManifest(
            "invalid plugin revision status".into(),
        )),
    }
}

const fn lifecycle_operation(operation: PluginLifecycleOperation) -> &'static str {
    match operation {
        PluginLifecycleOperation::Install => "install",
        PluginLifecycleOperation::Update => "update",
        PluginLifecycleOperation::Enable => "enable",
        PluginLifecycleOperation::Disable => "disable",
        PluginLifecycleOperation::Rollback => "rollback",
        PluginLifecycleOperation::Uninstall => "uninstall",
        PluginLifecycleOperation::Reconcile => "reconcile",
    }
}

fn parse_lifecycle_operation(
    value: String,
) -> Result<PluginLifecycleOperation, ExtensionHostError> {
    match value.as_str() {
        "install" => Ok(PluginLifecycleOperation::Install),
        "update" => Ok(PluginLifecycleOperation::Update),
        "enable" => Ok(PluginLifecycleOperation::Enable),
        "disable" => Ok(PluginLifecycleOperation::Disable),
        "rollback" => Ok(PluginLifecycleOperation::Rollback),
        "uninstall" => Ok(PluginLifecycleOperation::Uninstall),
        "reconcile" => Ok(PluginLifecycleOperation::Reconcile),
        _ => Err(ExtensionHostError::InvalidManifest(
            "invalid plugin lifecycle operation".into(),
        )),
    }
}

const fn lifecycle_phase(phase: PluginLifecyclePhase) -> &'static str {
    match phase {
        PluginLifecyclePhase::Stage => "stage",
        PluginLifecyclePhase::Validate => "validate",
        PluginLifecyclePhase::PermissionReview => "permission_review",
        PluginLifecyclePhase::Activate => "activate",
        PluginLifecyclePhase::HealthCheck => "health_check",
        PluginLifecyclePhase::Commit => "commit",
        PluginLifecyclePhase::Rollback => "rollback",
    }
}

fn parse_lifecycle_phase(value: String) -> Result<PluginLifecyclePhase, ExtensionHostError> {
    match value.as_str() {
        "stage" => Ok(PluginLifecyclePhase::Stage),
        "validate" => Ok(PluginLifecyclePhase::Validate),
        "permission_review" => Ok(PluginLifecyclePhase::PermissionReview),
        "activate" => Ok(PluginLifecyclePhase::Activate),
        "health_check" => Ok(PluginLifecyclePhase::HealthCheck),
        "commit" => Ok(PluginLifecyclePhase::Commit),
        "rollback" => Ok(PluginLifecyclePhase::Rollback),
        _ => Err(ExtensionHostError::InvalidManifest(
            "invalid plugin lifecycle phase".into(),
        )),
    }
}

const fn journal_status(status: PluginLifecycleJournalStatus) -> &'static str {
    match status {
        PluginLifecycleJournalStatus::InProgress => "in_progress",
        PluginLifecycleJournalStatus::Committed => "committed",
        PluginLifecycleJournalStatus::RolledBack => "rolled_back",
        PluginLifecycleJournalStatus::Failed => "failed",
    }
}

fn parse_journal_status(value: String) -> Result<PluginLifecycleJournalStatus, ExtensionHostError> {
    match value.as_str() {
        "in_progress" => Ok(PluginLifecycleJournalStatus::InProgress),
        "committed" => Ok(PluginLifecycleJournalStatus::Committed),
        "rolled_back" => Ok(PluginLifecycleJournalStatus::RolledBack),
        "failed" => Ok(PluginLifecycleJournalStatus::Failed),
        _ => Err(ExtensionHostError::InvalidManifest(
            "invalid plugin lifecycle journal status".into(),
        )),
    }
}
