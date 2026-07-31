//! Local Codex-style Plugin Bundle and deterministic Connector Host.

mod channel_manifest;
mod connector_driver;
mod connector_manifest;
mod connector_sidecar;
mod contribution_runtime;
mod enterprise_attachment;
mod enterprise_connector;
mod error;
mod hook_runtime;
mod lifecycle;
#[cfg(test)]
mod lifecycle_matrix_tests;

pub use channel_manifest::ChannelSidecarDefinition;
pub use connector_driver::{
    ConnectorDriver, ConnectorDriverContext, ConnectorDriverFuture, ConnectorDriverRegistry,
};
pub(crate) use connector_manifest::connector_descriptor;
pub use connector_sidecar::SandboxedStdioConnectorDriver;
use contribution_runtime::{
    connector_health, contribution_runtime_state, decode_installed_contribution,
    event_source_descriptor_valid, failed_contribution, parse_connector_health,
    parse_plugin_status, plugin_status, read_json_object,
    transition_allowed as contribution_transition_allowed, validate_static_surface,
};
pub use enterprise_connector::EnterpriseConnectorDriver;
pub use error::ExtensionHostError;
use error::connector_error_code;
pub use lifecycle::PluginLifecycleRecoveryReport;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use hachimi_core::FeatureAvailability;
use hachimi_enterprise::EnterpriseApiClient;
use hachimi_protocol::{
    ConnectorAccount, ConnectorAccountId, ConnectorAccountUpsert, ConnectorHealth,
    ConnectorInvocationRequest, ConnectorInvocationResult, ConnectorRevision, ContributionRevision,
    ContributionRuntimeState, InstalledContribution, InstalledPlugin, PluginContribution,
    PluginContributionKind, PluginLifecycleJournalStatus, PluginLifecycleOperation,
    PluginLifecyclePhase, PluginManifest, PluginPermissionDiff, PluginRevisionStatus, PluginStatus,
};
use hachimi_storage::AgentStore;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::Row;
use zip::ZipArchive;

const MANIFEST_RELATIVE_PATH: &str = ".codex-plugin/plugin.json";
const MAX_PLUGIN_FILES: usize = 2_000;
const MAX_PLUGIN_FILE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_PLUGIN_TOTAL_BYTES: u64 = 100 * 1024 * 1024;

#[must_use]
pub const fn availability() -> FeatureAvailability {
    FeatureAvailability::Available
}

#[derive(Clone)]
pub struct PluginHost {
    store: AgentStore,
    install_root: PathBuf,
    drivers: ConnectorDriverRegistry,
    enterprise_api: EnterpriseApiClient,
    hook_backend: Arc<parking_lot::RwLock<Option<Arc<dyn hachimi_sandbox::SandboxBackend>>>>,
    hook_acl_roots: Arc<parking_lot::Mutex<BTreeSet<PathBuf>>>,
}

impl std::fmt::Debug for PluginHost {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PluginHost")
            .field("install_root", &self.install_root)
            .field("drivers", &self.drivers)
            .field("enterprise_api", &self.enterprise_api)
            .field("hook_runtime_attached", &self.hook_backend.read().is_some())
            .finish_non_exhaustive()
    }
}

impl PluginHost {
    #[must_use]
    pub fn new(store: AgentStore, install_root: impl Into<PathBuf>) -> Self {
        Self {
            store,
            install_root: install_root.into(),
            drivers: ConnectorDriverRegistry::with_builtin_drivers(),
            enterprise_api: EnterpriseApiClient::default(),
            hook_backend: Arc::new(parking_lot::RwLock::new(None)),
            hook_acl_roots: Arc::new(parking_lot::Mutex::new(BTreeSet::new())),
        }
    }

    #[must_use]
    pub fn with_driver_registry(
        store: AgentStore,
        install_root: impl Into<PathBuf>,
        drivers: ConnectorDriverRegistry,
    ) -> Self {
        Self {
            store,
            install_root: install_root.into(),
            drivers,
            enterprise_api: EnterpriseApiClient::default(),
            hook_backend: Arc::new(parking_lot::RwLock::new(None)),
            hook_acl_roots: Arc::new(parking_lot::Mutex::new(BTreeSet::new())),
        }
    }

    pub async fn install_local(
        &self,
        source: impl AsRef<Path>,
    ) -> Result<InstalledPlugin, ExtensionHostError> {
        let source = source
            .as_ref()
            .canonicalize()
            .map_err(|_| ExtensionHostError::InvalidSource)?;
        if source.is_file() {
            return self.install_archive(&source).await;
        }
        self.install_directory(&source).await
    }

    async fn install_archive(&self, source: &Path) -> Result<InstalledPlugin, ExtensionHostError> {
        if source.extension().and_then(|value| value.to_str()) != Some("zip") {
            return Err(ExtensionHostError::InvalidSource);
        }
        fs::create_dir_all(&self.install_root)?;
        let staging = tempfile::Builder::new()
            .prefix(".bundle-import-")
            .tempdir_in(&self.install_root)?;
        let bundle_root = extract_bundle_archive(source, staging.path())?;
        self.install_directory(&bundle_root).await
    }

    async fn install_directory(
        &self,
        source: &Path,
    ) -> Result<InstalledPlugin, ExtensionHostError> {
        if !source.is_dir() {
            return Err(ExtensionHostError::InvalidSource);
        }
        let manifest_path = source.join(MANIFEST_RELATIVE_PATH);
        let manifest: PluginManifest = serde_json::from_slice(
            &fs::read(&manifest_path)
                .map_err(|error| ExtensionHostError::InvalidManifest(error.to_string()))?,
        )
        .map_err(|error| ExtensionHostError::InvalidManifest(error.to_string()))?;
        validate_manifest(&manifest, source)?;
        let (content_hash, files) = hash_bundle(source)?;
        let destination = self
            .install_root
            .join(manifest.id.as_str())
            .join(&content_hash);
        if !destination.is_dir() {
            let staging = self.install_root.join(format!(
                ".{}.{}.staging",
                manifest.id.as_str(),
                std::process::id()
            ));
            if staging.exists() {
                return Err(ExtensionHostError::InvalidSource);
            }
            fs::create_dir_all(&staging)?;
            let copy_result = copy_bundle(source, &staging, &files);
            if let Err(error) = copy_result {
                let _ = fs::remove_dir_all(&staging);
                return Err(error);
            }
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(&staging, &destination)?;
        }
        let now = now_ms();
        let existing = self.get(&manifest.id).await?;
        let permission_diff = plugin_permission_diff(existing.as_ref(), &manifest);
        let status = existing
            .as_ref()
            .map_or(PluginStatus::Disabled, |installed| {
                if installed.content_hash == content_hash {
                    installed.status
                } else {
                    PluginStatus::NeedsAttention
                }
            });
        let installed_at_ms = existing
            .as_ref()
            .map_or(now, |installed| installed.installed_at_ms);
        let installed = InstalledPlugin {
            manifest,
            content_hash,
            root_path: destination.to_string_lossy().into_owned(),
            status,
            diagnostics: if status == PluginStatus::NeedsAttention {
                let mut diagnostics =
                    vec!["plugin content changed; review contributions before re-enabling".into()];
                if permission_diff.requires_confirmation {
                    diagnostics.push(format!(
                        "plugin requested additional scopes: {}",
                        permission_diff.added_scopes.join(", ")
                    ));
                }
                diagnostics
            } else {
                Vec::new()
            },
            installed_at_ms,
            updated_at_ms: now,
        };
        let operation = if existing.is_some() {
            PluginLifecycleOperation::Update
        } else {
            PluginLifecycleOperation::Install
        };
        let journal_id = self
            .begin_lifecycle(
                &installed.manifest.id,
                operation,
                existing.as_ref().map(|plugin| plugin.content_hash.as_str()),
                Some(&installed.content_hash),
            )
            .await?;
        self.stage_revision(&installed).await?;
        self.advance_lifecycle(&journal_id, PluginLifecyclePhase::Validate)
            .await?;
        self.set_revision_status(
            &installed.manifest.id,
            &installed.content_hash,
            PluginRevisionStatus::Validated,
            None,
        )
        .await?;
        self.advance_lifecycle(&journal_id, PluginLifecyclePhase::PermissionReview)
            .await?;
        self.advance_lifecycle(&journal_id, PluginLifecyclePhase::Activate)
            .await?;
        self.set_revision_status(
            &installed.manifest.id,
            &installed.content_hash,
            PluginRevisionStatus::Activating,
            None,
        )
        .await?;
        let activation = async {
            persist_plugin(&self.store, &installed).await?;
            sqlx::query(
                "INSERT INTO plugin_permission_diffs(plugin_id, diff_json, updated_at_ms) VALUES(?, ?, ?) ON CONFLICT(plugin_id) DO UPDATE SET diff_json = excluded.diff_json, updated_at_ms = excluded.updated_at_ms",
            )
            .bind(installed.manifest.id.as_str())
            .bind(serde_json::to_string(&permission_diff)?)
            .bind(now)
            .execute(self.store.pool())
            .await?;
            self.reconcile_contributions(&installed, installed.status == PluginStatus::Enabled)
                .await
        }
        .await;
        let contributions = match activation {
            Ok(contributions) => contributions,
            Err(error) => {
                self.rollback_failed_candidate(
                    &installed,
                    existing.as_ref(),
                    &journal_id,
                    "plugin_activation_failed",
                )
                .await?;
                return Err(error);
            }
        };
        self.advance_lifecycle(&journal_id, PluginLifecyclePhase::HealthCheck)
            .await?;
        if let Some(unhealthy) = contributions
            .iter()
            .find(|contribution| contribution.state == ContributionRuntimeState::Failed)
        {
            let diagnostic = unhealthy
                .diagnostic
                .clone()
                .unwrap_or_else(|| "plugin_contribution_unhealthy".into());
            self.rollback_failed_candidate(&installed, existing.as_ref(), &journal_id, &diagnostic)
                .await?;
            return Err(ExtensionHostError::UnsupportedContribution(format!(
                "{}:{}:{diagnostic}",
                unhealthy.kind.as_str(),
                unhealthy.contribution_id
            )));
        }
        self.commit_revision(
            &installed.manifest.id,
            &installed.content_hash,
            existing.as_ref().map(|plugin| plugin.content_hash.as_str()),
        )
        .await?;
        self.finish_lifecycle(&journal_id, PluginLifecycleJournalStatus::Committed, None)
            .await?;
        self.mark_stale_schedules(&installed.manifest.id, &installed.content_hash)
            .await?;
        Ok(installed)
    }

    async fn rollback_failed_candidate(
        &self,
        candidate: &InstalledPlugin,
        previous: Option<&InstalledPlugin>,
        journal_id: &str,
        error_code: &str,
    ) -> Result<(), ExtensionHostError> {
        self.set_revision_status(
            &candidate.manifest.id,
            &candidate.content_hash,
            PluginRevisionStatus::Failed,
            Some(error_code),
        )
        .await?;
        if let Some(previous) = previous {
            persist_plugin(&self.store, previous).await?;
            self.reconcile_contributions(previous, previous.status == PluginStatus::Enabled)
                .await?;
            self.finish_lifecycle(
                journal_id,
                PluginLifecycleJournalStatus::RolledBack,
                Some(error_code),
            )
            .await?;
        } else {
            sqlx::query("DELETE FROM plugin_installations WHERE plugin_id = ?")
                .bind(candidate.manifest.id.as_str())
                .execute(self.store.pool())
                .await?;
            self.finish_lifecycle(
                journal_id,
                PluginLifecycleJournalStatus::Failed,
                Some(error_code),
            )
            .await?;
        }
        Ok(())
    }

    pub async fn get(
        &self,
        plugin_id: &hachimi_protocol::PluginId,
    ) -> Result<Option<InstalledPlugin>, ExtensionHostError> {
        let row = sqlx::query(
            "SELECT manifest_json, content_hash, root_path, status, diagnostics_json, installed_at_ms, updated_at_ms FROM plugin_installations WHERE plugin_id = ?",
        )
        .bind(plugin_id.as_str())
        .fetch_optional(self.store.pool())
        .await?;
        row.map(decode_plugin).transpose()
    }

    pub async fn permission_diff(
        &self,
        plugin_id: &hachimi_protocol::PluginId,
    ) -> Result<Option<PluginPermissionDiff>, ExtensionHostError> {
        let row = sqlx::query("SELECT diff_json FROM plugin_permission_diffs WHERE plugin_id = ?")
            .bind(plugin_id.as_str())
            .fetch_optional(self.store.pool())
            .await?;
        row.map(|row| serde_json::from_str(row.get("diff_json")))
            .transpose()
            .map_err(ExtensionHostError::from)
    }

    pub async fn list(&self) -> Result<Vec<InstalledPlugin>, ExtensionHostError> {
        let rows = sqlx::query(
            "SELECT manifest_json, content_hash, root_path, status, diagnostics_json, installed_at_ms, updated_at_ms FROM plugin_installations ORDER BY plugin_id",
        )
        .fetch_all(self.store.pool())
        .await?;
        rows.into_iter().map(decode_plugin).collect()
    }

    pub async fn set_enabled(
        &self,
        plugin_id: &hachimi_protocol::PluginId,
        enabled: bool,
    ) -> Result<InstalledPlugin, ExtensionHostError> {
        let installed = self
            .get(plugin_id)
            .await?
            .ok_or(ExtensionHostError::PluginNotFound)?;
        if enabled && installed.status == PluginStatus::Invalid {
            return Err(ExtensionHostError::InvalidManifest(
                "invalid plugins cannot be enabled".into(),
            ));
        }
        let operation = if enabled {
            PluginLifecycleOperation::Enable
        } else {
            PluginLifecycleOperation::Disable
        };
        let journal_id = self
            .begin_lifecycle(
                plugin_id,
                operation,
                Some(&installed.content_hash),
                Some(&installed.content_hash),
            )
            .await?;
        self.advance_lifecycle(&journal_id, PluginLifecyclePhase::Validate)
            .await?;
        if enabled && let Err(error) = validate_supported_contributions(&installed, &self.drivers) {
            self.finish_lifecycle(
                &journal_id,
                PluginLifecycleJournalStatus::Failed,
                Some("plugin_contribution_unsupported"),
            )
            .await?;
            return Err(error);
        }
        self.advance_lifecycle(&journal_id, PluginLifecyclePhase::PermissionReview)
            .await?;
        self.advance_lifecycle(&journal_id, PluginLifecyclePhase::Activate)
            .await?;
        let contributions = self.reconcile_contributions(&installed, enabled).await;
        let contributions = match contributions {
            Ok(contributions) => contributions,
            Err(error) => {
                let _ = self
                    .reconcile_contributions(&installed, installed.status == PluginStatus::Enabled)
                    .await;
                self.finish_lifecycle(
                    &journal_id,
                    PluginLifecycleJournalStatus::RolledBack,
                    Some("plugin_activation_failed"),
                )
                .await?;
                return Err(error);
            }
        };
        self.advance_lifecycle(&journal_id, PluginLifecyclePhase::HealthCheck)
            .await?;
        if enabled
            && let Some(blocked) = contributions.iter().find(|contribution| {
                matches!(
                    contribution.state,
                    ContributionRuntimeState::Degraded
                        | ContributionRuntimeState::Unsupported
                        | ContributionRuntimeState::Failed
                )
            })
        {
            self.reconcile_contributions(&installed, installed.status == PluginStatus::Enabled)
                .await?;
            self.finish_lifecycle(
                &journal_id,
                PluginLifecycleJournalStatus::RolledBack,
                Some("plugin_health_check_failed"),
            )
            .await?;
            return Err(ExtensionHostError::UnsupportedContribution(format!(
                "{}:{}:{}",
                blocked.kind.as_str(),
                blocked.contribution_id,
                blocked
                    .diagnostic
                    .as_deref()
                    .unwrap_or("runtime unavailable")
            )));
        }
        let status = if enabled {
            PluginStatus::Enabled
        } else {
            PluginStatus::Disabled
        };
        sqlx::query(
            "UPDATE plugin_installations SET status = ?, diagnostics_json = '[]', updated_at_ms = ? WHERE plugin_id = ?",
        )
        .bind(plugin_status(status))
        .bind(now_ms())
        .bind(plugin_id.as_str())
        .execute(self.store.pool())
        .await?;
        sqlx::query("UPDATE plugin_revisions SET plugin_status = ?, updated_at_ms = ? WHERE plugin_id = ? AND revision = ?")
            .bind(plugin_status(status))
            .bind(now_ms())
            .bind(plugin_id.as_str())
            .bind(&installed.content_hash)
            .execute(self.store.pool())
            .await?;
        let updated = self
            .get(plugin_id)
            .await?
            .ok_or(ExtensionHostError::PluginNotFound)?;
        self.finish_lifecycle(&journal_id, PluginLifecycleJournalStatus::Committed, None)
            .await?;
        Ok(updated)
    }

    pub async fn health_check(
        &self,
        plugin_id: &hachimi_protocol::PluginId,
    ) -> Result<InstalledPlugin, ExtensionHostError> {
        let installed = self
            .get(plugin_id)
            .await?
            .ok_or(ExtensionHostError::PluginNotFound)?;
        let root = PathBuf::from(&installed.root_path);
        let healthy = root.is_dir()
            && hash_bundle(&root)
                .map(|(hash, _)| hash == installed.content_hash)
                .unwrap_or(false);
        if healthy {
            self.reconcile_contributions(&installed, installed.status == PluginStatus::Enabled)
                .await?;
            self.set_revision_status(
                plugin_id,
                &installed.content_hash,
                PluginRevisionStatus::Healthy,
                None,
            )
            .await?;
            return Ok(installed);
        }
        self.set_revision_status(
            plugin_id,
            &installed.content_hash,
            PluginRevisionStatus::Failed,
            Some("plugin_bundle_integrity_failed"),
        )
        .await?;
        sqlx::query(
            "UPDATE plugin_installations SET status = 'needs_attention', diagnostics_json = ?, updated_at_ms = ? WHERE plugin_id = ?",
        )
        .bind(serde_json::to_string(&vec![
            "installed plugin content is missing or no longer matches its recorded hash",
        ])?)
        .bind(now_ms())
        .bind(plugin_id.as_str())
        .execute(self.store.pool())
        .await?;
        self.get(plugin_id)
            .await?
            .ok_or(ExtensionHostError::PluginNotFound)
    }

    pub async fn uninstall(
        &self,
        plugin_id: &hachimi_protocol::PluginId,
    ) -> Result<bool, ExtensionHostError> {
        let Some(installed) = self.get(plugin_id).await? else {
            return Ok(false);
        };
        let journal_id = self
            .begin_lifecycle(
                plugin_id,
                PluginLifecycleOperation::Uninstall,
                Some(&installed.content_hash),
                None,
            )
            .await?;
        self.advance_lifecycle(&journal_id, PluginLifecyclePhase::Validate)
            .await?;
        self.advance_lifecycle(&journal_id, PluginLifecyclePhase::PermissionReview)
            .await?;
        self.advance_lifecycle(&journal_id, PluginLifecyclePhase::Activate)
            .await?;
        sqlx::query("UPDATE plugin_contribution_runtime SET runtime_state = 'stopping', diagnostic = 'plugin_uninstall_in_progress', updated_at_ms = ? WHERE plugin_id = ?")
            .bind(now_ms())
            .bind(plugin_id.as_str())
            .execute(self.store.pool())
            .await?;
        for account in self
            .list_connector_accounts()
            .await?
            .into_iter()
            .filter(|account| &account.plugin_id == plugin_id && account.secret_ref.is_some())
        {
            match connector_keyring_entry(&account.id)?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(_) => return Err(ExtensionHostError::SecretStore),
            }
        }
        let result = sqlx::query("DELETE FROM plugin_installations WHERE plugin_id = ?")
            .bind(plugin_id.as_str())
            .execute(self.store.pool())
            .await?;
        if result.rows_affected() != 1 {
            self.finish_lifecycle(
                &journal_id,
                PluginLifecycleJournalStatus::Failed,
                Some("plugin_uninstall_lost_installation"),
            )
            .await?;
            return Ok(false);
        }
        self.mark_stale_schedules(plugin_id, "__plugin_uninstalled__")
            .await?;
        let install_root = self
            .install_root
            .canonicalize()
            .unwrap_or_else(|_| self.install_root.clone());
        let mut revision_roots = self
            .list_revisions(plugin_id)
            .await?
            .into_iter()
            .map(|revision| PathBuf::from(revision.root_path))
            .collect::<BTreeSet<_>>();
        revision_roots.insert(PathBuf::from(installed.root_path));
        for bundle_root in revision_roots {
            let bundle_root = bundle_root.canonicalize().unwrap_or(bundle_root);
            if bundle_root != install_root
                && bundle_root.starts_with(&install_root)
                && bundle_root.is_dir()
            {
                fs::remove_dir_all(&bundle_root)?;
            }
        }
        let now = now_ms();
        let mut tx = self.store.pool().begin().await?;
        sqlx::query("DELETE FROM plugin_revision_heads WHERE plugin_id = ?")
            .bind(plugin_id.as_str())
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE plugin_revisions SET status = 'removed', health_code = 'plugin_uninstalled', updated_at_ms = ? WHERE plugin_id = ?")
            .bind(now)
            .bind(plugin_id.as_str())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.advance_lifecycle(&journal_id, PluginLifecyclePhase::HealthCheck)
            .await?;
        self.finish_lifecycle(&journal_id, PluginLifecycleJournalStatus::Committed, None)
            .await?;
        Ok(true)
    }

    pub async fn list_contributions(
        &self,
        plugin_id: Option<&hachimi_protocol::PluginId>,
    ) -> Result<Vec<InstalledContribution>, ExtensionHostError> {
        let rows = if let Some(plugin_id) = plugin_id {
            sqlx::query("SELECT plugin_id, contribution_id, contribution_kind, content_hash, runtime_revision, runtime_state, diagnostic FROM plugin_contribution_runtime WHERE plugin_id = ? ORDER BY contribution_kind, contribution_id")
                .bind(plugin_id.as_str())
                .fetch_all(self.store.pool())
                .await?
        } else {
            sqlx::query("SELECT plugin_id, contribution_id, contribution_kind, content_hash, runtime_revision, runtime_state, diagnostic FROM plugin_contribution_runtime ORDER BY plugin_id, contribution_kind, contribution_id")
                .fetch_all(self.store.pool())
                .await?
        };
        rows.into_iter()
            .map(decode_installed_contribution)
            .collect()
    }

    pub async fn transition_contribution(
        &self,
        plugin_id: &hachimi_protocol::PluginId,
        contribution_id: &str,
        expected_runtime_revision: &str,
        state: ContributionRuntimeState,
        diagnostic: Option<&str>,
    ) -> Result<InstalledContribution, ExtensionHostError> {
        let current = self
            .list_contributions(Some(plugin_id))
            .await?
            .into_iter()
            .find(|contribution| contribution.contribution_id == contribution_id)
            .ok_or(ExtensionHostError::ContributionDrift)?;
        if current.runtime_revision != expected_runtime_revision
            || !contribution_transition_allowed(current.state, state)
        {
            return Err(ExtensionHostError::ContributionDrift);
        }
        let result = sqlx::query("UPDATE plugin_contribution_runtime SET runtime_state = ?, diagnostic = ?, updated_at_ms = ? WHERE plugin_id = ? AND contribution_id = ? AND runtime_revision = ?")
            .bind(contribution_runtime_state(state))
            .bind(diagnostic)
            .bind(now_ms())
            .bind(plugin_id.as_str())
            .bind(contribution_id)
            .bind(expected_runtime_revision)
            .execute(self.store.pool())
            .await?;
        if result.rows_affected() != 1 {
            return Err(ExtensionHostError::ContributionDrift);
        }
        self.list_contributions(Some(plugin_id))
            .await?
            .into_iter()
            .find(|contribution| contribution.contribution_id == contribution_id)
            .ok_or(ExtensionHostError::ContributionDrift)
    }

    async fn reconcile_contributions(
        &self,
        plugin: &InstalledPlugin,
        enabled: bool,
    ) -> Result<Vec<InstalledContribution>, ExtensionHostError> {
        let mut installed = Vec::with_capacity(plugin.manifest.contributions.len());
        for contribution in &plugin.manifest.contributions {
            let value = load_contribution(plugin, contribution, enabled, &self.drivers)
                .unwrap_or_else(|_| failed_contribution(plugin, contribution));
            sqlx::query("INSERT INTO plugin_contribution_runtime(plugin_id, contribution_id, contribution_kind, content_hash, runtime_revision, runtime_state, diagnostic, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(plugin_id, contribution_id) DO UPDATE SET contribution_kind = excluded.contribution_kind, content_hash = excluded.content_hash, runtime_revision = excluded.runtime_revision, runtime_state = excluded.runtime_state, diagnostic = excluded.diagnostic, updated_at_ms = excluded.updated_at_ms")
                .bind(value.plugin_id.as_str())
                .bind(&value.contribution_id)
                .bind(value.kind.as_str())
                .bind(&value.content_hash)
                .bind(&value.runtime_revision)
                .bind(contribution_runtime_state(value.state))
                .bind(&value.diagnostic)
                .bind(now_ms())
                .execute(self.store.pool())
                .await?;
            self.reconcile_hook_subscriptions(plugin, contribution, &value, enabled)
                .await?;
            installed.push(value);
        }
        Ok(installed)
    }

    pub async fn upsert_connector_account(
        &self,
        input: ConnectorAccountUpsert,
    ) -> Result<ConnectorAccount, ExtensionHostError> {
        let plugin = self.health_check(&input.plugin_id).await?;
        if plugin.status != PluginStatus::Enabled
            || input.connector_id.trim().is_empty()
            || input.display_name.trim().is_empty()
            || input
                .secret
                .as_deref()
                .is_some_and(|secret| secret.is_empty() || secret.len() > 64 * 1024)
        {
            return Err(ExtensionHostError::InvalidInvocation);
        }
        let revision = connector_revision(&plugin, &input.connector_id)?;
        let descriptor = connector_descriptor(&plugin, &input.connector_id)?;
        if self.drivers.resolve(&descriptor.host_identity).is_none() {
            return Err(ExtensionHostError::UnsupportedContribution(format!(
                "connector:{}",
                input.connector_id
            )));
        }
        let previous = self.connector_account(&input.id).await?;
        let mut secret_ref = previous.and_then(|account| account.secret_ref);
        if let Some(secret) = input.secret.as_deref() {
            let reference = connector_secret_reference(&input.id);
            connector_keyring_entry(&input.id)?
                .set_password(secret)
                .map_err(|_| ExtensionHostError::SecretStore)?;
            secret_ref = Some(reference);
        }
        let account = ConnectorAccount {
            id: input.id,
            plugin_id: input.plugin_id,
            connector_id: input.connector_id,
            display_name: input.display_name,
            secret_ref,
            revision,
            health: ConnectorHealth::Healthy,
            updated_at_ms: now_ms(),
        };
        sqlx::query(
            "INSERT INTO connector_accounts(id, plugin_id, connector_id, display_name, secret_ref, revision_json, health, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name, secret_ref = excluded.secret_ref, revision_json = excluded.revision_json, health = excluded.health, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(account.id.as_str())
        .bind(account.plugin_id.as_str())
        .bind(&account.connector_id)
        .bind(&account.display_name)
        .bind(&account.secret_ref)
        .bind(serde_json::to_string(&account.revision)?)
        .bind(connector_health(account.health))
        .bind(account.updated_at_ms)
        .execute(self.store.pool())
        .await?;
        sqlx::query("DELETE FROM connector_health_overrides WHERE account_id = ?")
            .bind(account.id.as_str())
            .execute(self.store.pool())
            .await?;
        Ok(account)
    }

    pub async fn connector_account(
        &self,
        account_id: &ConnectorAccountId,
    ) -> Result<Option<ConnectorAccount>, ExtensionHostError> {
        let row = sqlx::query(
            "SELECT account.plugin_id, account.connector_id, account.display_name, account.secret_ref, account.revision_json, COALESCE(override.health, account.health) AS health, account.updated_at_ms FROM connector_accounts AS account LEFT JOIN connector_health_overrides AS override ON override.account_id = account.id WHERE account.id = ?",
        )
        .bind(account_id.as_str())
        .fetch_optional(self.store.pool())
        .await?;
        row.map(|row| {
            Ok(ConnectorAccount {
                id: account_id.clone(),
                plugin_id: hachimi_protocol::PluginId::new(row.get::<String, _>("plugin_id")),
                connector_id: row.get("connector_id"),
                display_name: row.get("display_name"),
                secret_ref: row.get("secret_ref"),
                revision: serde_json::from_str(row.get("revision_json"))?,
                health: parse_connector_health(row.get("health"))?,
                updated_at_ms: row.get("updated_at_ms"),
            })
        })
        .transpose()
    }

    pub async fn connector_driver_descriptor(
        &self,
        plugin_id: &hachimi_protocol::PluginId,
        connector_id: &str,
    ) -> Result<hachimi_protocol::ConnectorDriverDescriptor, ExtensionHostError> {
        let plugin = self.health_check(plugin_id).await?;
        if plugin.status != PluginStatus::Enabled {
            return Err(ExtensionHostError::PluginNotFound);
        }
        let loaded = connector_descriptor(&plugin, connector_id)?;
        Ok(hachimi_protocol::ConnectorDriverDescriptor {
            plugin_id: plugin_id.clone(),
            connector_id: connector_id.to_owned(),
            runtime_kind: loaded.runtime_kind,
            revision: loaded.revision,
            actions: loaded.actions,
        })
    }

    pub async fn list_connector_accounts(
        &self,
    ) -> Result<Vec<ConnectorAccount>, ExtensionHostError> {
        let rows = sqlx::query(
            "SELECT account.id, account.plugin_id, account.connector_id, account.display_name, account.secret_ref, account.revision_json, COALESCE(override.health, account.health) AS health, account.updated_at_ms FROM connector_accounts AS account LEFT JOIN connector_health_overrides AS override ON override.account_id = account.id ORDER BY account.plugin_id, account.connector_id, account.id",
        )
        .fetch_all(self.store.pool())
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ConnectorAccount {
                    id: ConnectorAccountId::new(row.get::<String, _>("id")),
                    plugin_id: hachimi_protocol::PluginId::new(row.get::<String, _>("plugin_id")),
                    connector_id: row.get("connector_id"),
                    display_name: row.get("display_name"),
                    secret_ref: row.get("secret_ref"),
                    revision: serde_json::from_str(row.get("revision_json"))?,
                    health: parse_connector_health(row.get("health"))?,
                    updated_at_ms: row.get("updated_at_ms"),
                })
            })
            .collect()
    }

    pub async fn enabled_skill_roots(
        &self,
    ) -> Result<Vec<(hachimi_protocol::PluginId, PathBuf)>, ExtensionHostError> {
        let mut roots = Vec::new();
        for plugin in self
            .list()
            .await?
            .into_iter()
            .filter(|plugin| plugin.status == PluginStatus::Enabled)
        {
            let plugin_root = PathBuf::from(&plugin.root_path)
                .canonicalize()
                .map_err(|_| ExtensionHostError::ContributionDrift)?;
            for contribution in plugin
                .manifest
                .contributions
                .iter()
                .filter(|contribution| contribution.kind == PluginContributionKind::Skill)
            {
                let target = plugin_root
                    .join(safe_relative_path(&contribution.relative_path)?)
                    .canonicalize()
                    .map_err(|_| ExtensionHostError::ContributionDrift)?;
                if !target.starts_with(&plugin_root) {
                    return Err(ExtensionHostError::ContributionEscape);
                }
                let root = if target.is_dir() {
                    target
                } else if target.file_name().and_then(|value| value.to_str()) == Some("SKILL.md") {
                    target
                        .parent()
                        .map(Path::to_path_buf)
                        .ok_or(ExtensionHostError::ContributionEscape)?
                } else {
                    return Err(ExtensionHostError::UnsupportedContribution(format!(
                        "skill:{}",
                        contribution.id
                    )));
                };
                roots.push((plugin.manifest.id.clone(), root));
            }
        }
        roots.sort_by(|left, right| (&left.0, &left.1).cmp(&(&right.0, &right.1)));
        roots.dedup();
        Ok(roots)
    }

    pub async fn enabled_channel_sidecars(
        &self,
    ) -> Result<Vec<ChannelSidecarDefinition>, ExtensionHostError> {
        let mut definitions = Vec::new();
        for plugin in self
            .list()
            .await?
            .into_iter()
            .filter(|plugin| plugin.status == PluginStatus::Enabled)
        {
            for contribution in plugin
                .manifest
                .contributions
                .iter()
                .filter(|contribution| contribution.kind == PluginContributionKind::Channel)
            {
                if channel_manifest::builtin_enterprise_channel_provider_id(
                    &plugin,
                    &contribution.id,
                )?
                .is_some()
                {
                    continue;
                }
                definitions.push(channel_manifest::channel_sidecar_definition(
                    &plugin,
                    &contribution.id,
                )?);
            }
        }
        definitions.sort_by(|left, right| left.manifest.id.cmp(&right.manifest.id));
        Ok(definitions)
    }

    pub fn channel_sidecar_definition(
        &self,
        plugin: &InstalledPlugin,
        contribution_id: &str,
    ) -> Result<ChannelSidecarDefinition, ExtensionHostError> {
        channel_manifest::channel_sidecar_definition(plugin, contribution_id)
    }

    pub fn builtin_enterprise_channel_provider_id(
        &self,
        plugin: &InstalledPlugin,
        contribution_id: &str,
    ) -> Result<Option<String>, ExtensionHostError> {
        channel_manifest::builtin_enterprise_channel_provider_id(plugin, contribution_id)
    }

    async fn set_connector_health(
        &self,
        account_id: &ConnectorAccountId,
        health: ConnectorHealth,
    ) -> Result<(), ExtensionHostError> {
        let now = now_ms();
        let persisted = if health == ConnectorHealth::ActionDrift {
            sqlx::query("INSERT INTO connector_health_overrides(account_id, health, updated_at_ms) VALUES(?, 'action_drift', ?) ON CONFLICT(account_id) DO UPDATE SET health = excluded.health, updated_at_ms = excluded.updated_at_ms")
                .bind(account_id.as_str())
                .bind(now)
                .execute(self.store.pool())
                .await?;
            ConnectorHealth::Failed
        } else {
            sqlx::query("DELETE FROM connector_health_overrides WHERE account_id = ?")
                .bind(account_id.as_str())
                .execute(self.store.pool())
                .await?;
            health
        };
        sqlx::query("UPDATE connector_accounts SET health = ?, updated_at_ms = ? WHERE id = ?")
            .bind(connector_health(persisted))
            .bind(now)
            .bind(account_id.as_str())
            .execute(self.store.pool())
            .await?;
        Ok(())
    }

    pub async fn verify_contribution_revisions(
        &self,
        revisions: &[ContributionRevision],
    ) -> Result<(), ExtensionHostError> {
        for expected in revisions {
            let plugin = self.health_check(&expected.plugin_id).await?;
            if plugin.status != PluginStatus::Enabled
                || plugin.content_hash != expected.content_hash
            {
                return Err(ExtensionHostError::ContributionDrift);
            }
            let contribution = plugin
                .manifest
                .contributions
                .iter()
                .find(|contribution| contribution.id == expected.contribution_id)
                .ok_or(ExtensionHostError::ContributionDrift)?;
            let pins_host_revision = expected.host_identity_hash.is_some()
                || expected.schema_hash.is_some()
                || expected.action_hash.is_some();
            if contribution.kind != hachimi_protocol::PluginContributionKind::Connector {
                if pins_host_revision {
                    return Err(ExtensionHostError::ContributionDrift);
                }
                continue;
            }
            if !pins_host_revision {
                continue;
            }
            let current = connector_revision(&plugin, &expected.contribution_id)?;
            if !revision_matches(expected, &current) {
                return Err(ExtensionHostError::ContributionDrift);
            }
            let accounts = if let Some(account_id) = &expected.account_id {
                self.connector_account(account_id)
                    .await?
                    .into_iter()
                    .collect::<Vec<_>>()
            } else {
                self.list_connector_accounts()
                    .await?
                    .into_iter()
                    .filter(|account| {
                        account.plugin_id == expected.plugin_id
                            && account.connector_id == expected.contribution_id
                    })
                    .collect()
            };
            let matches = accounts.into_iter().any(|account| {
                account.plugin_id == expected.plugin_id
                    && account.connector_id == expected.contribution_id
                    && account.health == ConnectorHealth::Healthy
                    && account.revision == current
            });
            if !matches {
                return Err(ExtensionHostError::ContributionDrift);
            }
        }
        Ok(())
    }

    pub async fn revoke_connector_account(
        &self,
        account_id: &ConnectorAccountId,
    ) -> Result<ConnectorAccount, ExtensionHostError> {
        let account = self
            .connector_account(account_id)
            .await?
            .ok_or(ExtensionHostError::ConnectorNotHealthy)?;
        if let Ok(plugin) = self.health_check(&account.plugin_id).await
            && let Ok(descriptor) = connector_descriptor(&plugin, &account.connector_id)
            && let Some(driver) = self.drivers.resolve(&descriptor.host_identity)
        {
            let credential = account
                .secret_ref
                .as_deref()
                .map(|reference| connector_secret(reference, &account.id))
                .transpose()?
                .flatten();
            driver
                .revoke(ConnectorDriverContext {
                    store: self.store.clone(),
                    account: account.clone(),
                    credential,
                })
                .await?;
        }
        if account.secret_ref.is_some() {
            match connector_keyring_entry(account_id)?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(_) => return Err(ExtensionHostError::SecretStore),
            }
        }
        sqlx::query("DELETE FROM connector_health_overrides WHERE account_id = ?")
            .bind(account_id.as_str())
            .execute(self.store.pool())
            .await?;
        let result = sqlx::query(
            "UPDATE connector_accounts SET secret_ref = NULL, health = 'revoked', updated_at_ms = ? WHERE id = ?",
        )
        .bind(now_ms())
        .bind(account_id.as_str())
        .execute(self.store.pool())
        .await?;
        if result.rows_affected() != 1 {
            return Err(ExtensionHostError::ConnectorNotHealthy);
        }
        self.connector_account(account_id)
            .await?
            .ok_or(ExtensionHostError::ConnectorNotHealthy)
    }

    pub async fn invoke_connector(
        &self,
        request: &ConnectorInvocationRequest,
    ) -> Result<ConnectorInvocationResult, ExtensionHostError> {
        let account = self
            .connector_account(&request.account_id)
            .await?
            .ok_or(ExtensionHostError::ConnectorNotHealthy)?;
        if account.health != ConnectorHealth::Healthy {
            if account.health == ConnectorHealth::RateLimited {
                // Rate limiting is window based and therefore recoverable;
                // callers receive a stable diagnostic while the ledger is
                // retained for reconciliation.
                if !rate_limit_window_expired(&self.store, &account.id).await? {
                    return Err(ExtensionHostError::RateLimited);
                }
                self.set_connector_health(&account.id, ConnectorHealth::Healthy)
                    .await?;
            } else {
                return Err(ExtensionHostError::ConnectorNotHealthy);
            }
        }
        let plugin = self.health_check(&account.plugin_id).await?;
        if plugin.status != PluginStatus::Enabled {
            self.set_connector_health(&account.id, ConnectorHealth::Failed)
                .await?;
            return Err(ExtensionHostError::ConnectorDrift);
        }
        let current_revision = connector_revision(&plugin, &account.connector_id)?;
        if current_revision != account.revision {
            let health = revision_drift_health(&account.revision, &current_revision);
            self.set_connector_health(&account.id, health).await?;
            return Err(ExtensionHostError::ConnectorDrift);
        }
        let credential = if let Some(reference) = &account.secret_ref {
            let credential = connector_secret(reference, &account.id)?;
            if credential.is_none() {
                self.set_connector_health(&account.id, ConnectorHealth::Revoked)
                    .await?;
                return Err(ExtensionHostError::ConnectorNotHealthy);
            }
            credential
        } else {
            None
        };
        if current_revision != request.expected_revision {
            return Err(ExtensionHostError::ConnectorDrift);
        }
        let descriptor = connector_descriptor(&plugin, &account.connector_id)?;
        let driver = self
            .drivers
            .resolve(&descriptor.host_identity)
            .ok_or_else(|| {
                ExtensionHostError::UnsupportedContribution(format!(
                    "connector:{}",
                    account.connector_id
                ))
            })?;
        let context = ConnectorDriverContext {
            store: self.store.clone(),
            account: account.clone(),
            credential,
        };
        if driver.health(&context).await? != ConnectorHealth::Healthy {
            return Err(ExtensionHostError::ConnectorNotHealthy);
        }
        if request.idempotency_key.trim().is_empty() || request.idempotency_key.len() > 128 {
            return Err(ExtensionHostError::InvalidInvocation);
        }
        let argument_hash = value_hash(&request.arguments)?;
        if let Some(existing) = replay_invocation(&self.store, request, &argument_hash).await? {
            return Ok(existing);
        }
        let now = now_ms();
        let attempt = begin_connector_attempt(&self.store, request, now).await?;
        if !allow_connector_rate(&self.store, &account.id, now).await? {
            self.set_connector_health(&account.id, ConnectorHealth::RateLimited)
                .await?;
            finish_connector_attempt(
                &self.store,
                request,
                attempt,
                Some("rate_limited"),
                now.saturating_add(1_000),
            )
            .await?;
            return Err(ExtensionHostError::RateLimited);
        }
        let outcome = match request.action.as_str() {
            "webhook_emit" | "webhook_next" => driver.webhook(context, request).await,
            "poll" => driver.poll(context, request).await,
            _ => driver.invoke(context, request).await,
        };
        let result = match outcome {
            Ok(result) => result,
            Err(error) => {
                let (error_code, retryable) = connector_error_code(&error);
                finish_connector_attempt(
                    &self.store,
                    request,
                    attempt,
                    Some(error_code),
                    if retryable {
                        now.saturating_add(1_000_i64.saturating_mul(i64::from(attempt.min(60))))
                    } else {
                        0
                    },
                )
                .await?;
                if matches!(error, ExtensionHostError::RateLimited) {
                    self.set_connector_health(&account.id, ConnectorHealth::RateLimited)
                        .await?;
                }
                return Err(error);
            }
        };
        let metadata = json!({
            "connector": account.connector_id,
            "hostIdentityHash": account.revision.host_identity_hash,
            "schemaHash": account.revision.schema_hash
        });
        sqlx::query(
            "INSERT INTO connector_invocations(account_id, idempotency_key, action, argument_hash, result_json, metadata_json, created_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(request.account_id.as_str())
        .bind(&request.idempotency_key)
        .bind(&request.action)
        .bind(&argument_hash)
        .bind(serde_json::to_string(&result)?)
        .bind(serde_json::to_string(&metadata)?)
        .bind(now_ms())
        .execute(self.store.pool())
        .await?;
        finish_connector_attempt(&self.store, request, attempt, None, 0).await?;
        self.store
            .append_audit_metadata(hachimi_storage::AuditMetadataRecord {
                principal: format!("connector:{}", request.account_id.as_str()),
                session_id: None,
                run_id: None,
                run_generation: None,
                operation: format!("connector.{}", request.action),
                target_summary: connector_target_summary(
                    &account.connector_id,
                    request.account_id.as_str(),
                ),
                decision: "allowed".into(),
                result_code: "completed".into(),
                created_at_ms: now,
            })
            .await?;
        Ok(ConnectorInvocationResult {
            account_id: request.account_id.clone(),
            action: request.action.clone(),
            result,
            metadata,
            replayed: false,
        })
    }
}

async fn persist_plugin(
    store: &AgentStore,
    plugin: &InstalledPlugin,
) -> Result<(), ExtensionHostError> {
    sqlx::query(
        "INSERT INTO plugin_installations(plugin_id, manifest_json, content_hash, root_path, status, diagnostics_json, installed_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(plugin_id) DO UPDATE SET manifest_json = excluded.manifest_json, content_hash = excluded.content_hash, root_path = excluded.root_path, status = excluded.status, diagnostics_json = excluded.diagnostics_json, updated_at_ms = excluded.updated_at_ms",
    )
    .bind(plugin.manifest.id.as_str())
    .bind(serde_json::to_string(&plugin.manifest)?)
    .bind(&plugin.content_hash)
    .bind(&plugin.root_path)
    .bind(plugin_status(plugin.status))
    .bind(serde_json::to_string(&plugin.diagnostics)?)
    .bind(plugin.installed_at_ms)
    .bind(plugin.updated_at_ms)
    .execute(store.pool())
    .await?;
    Ok(())
}

fn decode_plugin(row: sqlx::sqlite::SqliteRow) -> Result<InstalledPlugin, ExtensionHostError> {
    Ok(InstalledPlugin {
        manifest: serde_json::from_str(row.get("manifest_json"))?,
        content_hash: row.get("content_hash"),
        root_path: row.get("root_path"),
        status: parse_plugin_status(row.get("status"))?,
        diagnostics: serde_json::from_str(row.get("diagnostics_json"))?,
        installed_at_ms: row.get("installed_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

fn validate_manifest(manifest: &PluginManifest, source: &Path) -> Result<(), ExtensionHostError> {
    if manifest.manifest_version != 1
        || manifest.id.as_str().is_empty()
        || manifest.id.as_str().len() > 128
        || !manifest.id.as_str().bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
        || manifest.name.trim().is_empty()
        || manifest.version.trim().is_empty()
        || manifest.contributions.len() > 256
    {
        return Err(ExtensionHostError::InvalidManifest(
            "manifest identity or version is invalid".into(),
        ));
    }
    let mut identities = BTreeMap::new();
    for contribution in &manifest.contributions {
        if contribution.id.trim().is_empty()
            || identities
                .insert((&contribution.kind, &contribution.id), ())
                .is_some()
        {
            return Err(ExtensionHostError::InvalidManifest(
                "contribution identity or kind is invalid".into(),
            ));
        }
        let relative = safe_relative_path(&contribution.relative_path)?;
        let target = source.join(relative);
        let metadata =
            fs::symlink_metadata(&target).map_err(|_| ExtensionHostError::ContributionEscape)?;
        if metadata.file_type().is_symlink() {
            return Err(ExtensionHostError::SymbolicLink);
        }
        if !target.starts_with(source) || (!metadata.is_file() && !metadata.is_dir()) {
            return Err(ExtensionHostError::ContributionEscape);
        }
        if target
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "dll" | "exe" | "com" | "scr" | "msi" | "sys" | "node"
                )
            })
        {
            return Err(ExtensionHostError::InvalidManifest(
                "native and in-process plugin binaries are forbidden".into(),
            ));
        }
        if matches!(
            contribution.kind,
            hachimi_protocol::PluginContributionKind::Asset
                | hachimi_protocol::PluginContributionKind::CustomUi
        ) && !metadata.is_dir()
        {
            return Err(ExtensionHostError::InvalidManifest(
                "asset and custom UI contributions must point to static directories".into(),
            ));
        }
        if contribution.kind == hachimi_protocol::PluginContributionKind::CustomUi
            && !target.join("index.html").is_file()
        {
            return Err(ExtensionHostError::InvalidManifest(
                "custom UI contribution is missing index.html".into(),
            ));
        }
    }
    Ok(())
}

fn safe_relative_path(value: &str) -> Result<PathBuf, ExtensionHostError> {
    let path = Path::new(value);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(ExtensionHostError::ContributionEscape);
    }
    Ok(path.to_path_buf())
}

fn extract_bundle_archive(
    source: &Path,
    destination: &Path,
) -> Result<PathBuf, ExtensionHostError> {
    let file = fs::File::open(source)?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| ExtensionHostError::InvalidManifest(error.to_string()))?;
    if archive.len() > MAX_PLUGIN_FILES {
        return Err(ExtensionHostError::BundleTooLarge);
    }
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| ExtensionHostError::InvalidManifest(error.to_string()))?;
        let relative = entry
            .enclosed_name()
            .ok_or(ExtensionHostError::ContributionEscape)?;
        validate_archive_relative_path(&relative)?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170_000 == 0o120_000)
        {
            return Err(ExtensionHostError::SymbolicLink);
        }
        if entry.size() > MAX_PLUGIN_FILE_BYTES {
            return Err(ExtensionHostError::BundleTooLarge);
        }
        total = total.saturating_add(entry.size());
        if total > MAX_PLUGIN_TOTAL_BYTES {
            return Err(ExtensionHostError::BundleTooLarge);
        }
        let output = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&output)?;
            continue;
        }
        if !entry.is_file() {
            return Err(ExtensionHostError::InvalidSource);
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut target = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&output)?;
        std::io::copy(&mut entry, &mut target)?;
    }
    if destination.join(MANIFEST_RELATIVE_PATH).is_file() {
        return Ok(destination.to_path_buf());
    }
    let mut roots = fs::read_dir(destination)?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .collect::<Vec<_>>();
    roots.sort_by_key(std::fs::DirEntry::file_name);
    if roots.len() == 1 && roots[0].path().join(MANIFEST_RELATIVE_PATH).is_file() {
        return Ok(roots.remove(0).path());
    }
    Err(ExtensionHostError::InvalidManifest(
        "bundle archive must contain one plugin manifest at its root".into(),
    ))
}

fn validate_archive_relative_path(path: &Path) -> Result<(), ExtensionHostError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| match component {
            Component::Normal(name) => name.to_string_lossy().contains([':', '\0']),
            _ => true,
        })
    {
        return Err(ExtensionHostError::ContributionEscape);
    }
    Ok(())
}

fn hash_bundle(source: &Path) -> Result<(String, Vec<PathBuf>), ExtensionHostError> {
    let mut files = Vec::new();
    collect_files(source, source, &mut files)?;
    files.sort();
    if files.len() > MAX_PLUGIN_FILES {
        return Err(ExtensionHostError::BundleTooLarge);
    }
    let mut total = 0_u64;
    let mut hasher = Sha256::new();
    for relative in &files {
        let path = source.join(relative);
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(ExtensionHostError::SymbolicLink);
        }
        if metadata.len() > MAX_PLUGIN_FILE_BYTES {
            return Err(ExtensionHostError::BundleTooLarge);
        }
        total = total.saturating_add(metadata.len());
        if total > MAX_PLUGIN_TOTAL_BYTES {
            return Err(ExtensionHostError::BundleTooLarge);
        }
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(fs::read(path)?);
        hasher.update([0xff]);
    }
    Ok((hex_digest(hasher.finalize().as_slice()), files))
}

fn collect_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<(), ExtensionHostError> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() {
            return Err(ExtensionHostError::SymbolicLink);
        }
        if metadata.is_dir() {
            collect_files(root, &entry.path(), files)?;
        } else if metadata.is_file() {
            files.push(
                entry
                    .path()
                    .strip_prefix(root)
                    .map_err(|_| ExtensionHostError::ContributionEscape)?
                    .to_path_buf(),
            );
        }
    }
    Ok(())
}

fn copy_bundle(
    source: &Path,
    destination: &Path,
    files: &[PathBuf],
) -> Result<(), ExtensionHostError> {
    for relative in files {
        let destination_file = destination.join(relative);
        if let Some(parent) = destination_file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source.join(relative), destination_file)?;
    }
    Ok(())
}

fn validate_supported_contributions(
    plugin: &InstalledPlugin,
    drivers: &ConnectorDriverRegistry,
) -> Result<(), ExtensionHostError> {
    for contribution in &plugin.manifest.contributions {
        if contribution.kind == hachimi_protocol::PluginContributionKind::Connector {
            let descriptor = connector_descriptor(plugin, &contribution.id)?;
            if drivers.resolve(&descriptor.host_identity).is_none() {
                return Err(ExtensionHostError::UnsupportedContribution(format!(
                    "connector:{}",
                    contribution.id
                )));
            }
        }
    }
    Ok(())
}

fn plugin_permission_diff(
    previous: Option<&InstalledPlugin>,
    manifest: &PluginManifest,
) -> PluginPermissionDiff {
    let previous_scopes = previous
        .into_iter()
        .flat_map(|plugin| &plugin.manifest.contributions)
        .flat_map(|contribution| contribution.required_scopes.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>();
    let requested_scopes = manifest
        .contributions
        .iter()
        .flat_map(|contribution| contribution.required_scopes.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>();
    let added_scopes = requested_scopes
        .difference(&previous_scopes)
        .cloned()
        .collect::<Vec<_>>();
    PluginPermissionDiff {
        plugin_id: manifest.id.clone(),
        previous_scopes: previous_scopes.into_iter().collect(),
        requested_scopes: requested_scopes.into_iter().collect(),
        requires_confirmation: previous.is_some() && !added_scopes.is_empty(),
        added_scopes,
    }
}

fn load_contribution(
    plugin: &InstalledPlugin,
    contribution: &PluginContribution,
    enabled: bool,
    drivers: &ConnectorDriverRegistry,
) -> Result<InstalledContribution, ExtensionHostError> {
    let root = PathBuf::from(&plugin.root_path)
        .canonicalize()
        .map_err(|_| ExtensionHostError::ContributionDrift)?;
    let target = root
        .join(safe_relative_path(&contribution.relative_path)?)
        .canonicalize()
        .map_err(|_| ExtensionHostError::ContributionDrift)?;
    if !target.starts_with(&root) {
        return Err(ExtensionHostError::ContributionEscape);
    }
    let content_hash = if target.is_dir() {
        hash_bundle(&target)?.0
    } else {
        hex_digest(Sha256::digest(fs::read(&target)?).as_slice())
    };
    let runtime_revision = value_hash(&json!({
        "pluginContentHash": plugin.content_hash,
        "contributionContentHash": content_hash,
        "kind": contribution.kind,
        "id": contribution.id,
        "requiredScopes": contribution.required_scopes,
    }))?;
    let disabled_state = (!enabled).then_some(ContributionRuntimeState::Disabled);
    let (state, diagnostic) = match contribution.kind {
        PluginContributionKind::Skill => {
            let skill_file = if target.is_dir() {
                target.join("SKILL.md")
            } else {
                target.clone()
            };
            if skill_file.file_name().and_then(|value| value.to_str()) == Some("SKILL.md")
                && skill_file.is_file()
            {
                (
                    disabled_state.unwrap_or(ContributionRuntimeState::Active),
                    None,
                )
            } else {
                (
                    ContributionRuntimeState::Failed,
                    Some("plugin_skill_entry_missing".into()),
                )
            }
        }
        PluginContributionKind::Hook => {
            let descriptor = read_json_object(&target)?;
            let has_legacy_events = descriptor
                .get("events")
                .and_then(Value::as_array)
                .is_some_and(|events| {
                    !events.is_empty()
                        && events.iter().all(|event| {
                            event.as_str().is_some_and(|event| {
                                matches!(
                                    event,
                                    "run.before"
                                        | "run.after"
                                        | "tool.before"
                                        | "tool.after"
                                        | "schedule.before"
                                        | "schedule.after"
                                )
                            })
                        })
                });
            let requires_runtime_upgrade = has_legacy_events
                && (descriptor.get("protocolVersion").is_none()
                    || descriptor.get("runtime").is_none()
                    || descriptor.get("entrypoint").is_none());
            if requires_runtime_upgrade {
                (
                    ContributionRuntimeState::Degraded,
                    Some("plugin_hook_runtime_upgrade_required".into()),
                )
            } else if crate::hook_runtime::hook_descriptor(plugin, contribution).is_ok() {
                (
                    disabled_state.unwrap_or(ContributionRuntimeState::Active),
                    None,
                )
            } else {
                (
                    ContributionRuntimeState::Failed,
                    Some("plugin_hook_descriptor_invalid".into()),
                )
            }
        }
        PluginContributionKind::EventSource => {
            let descriptor = read_json_object(&target)?;
            if event_source_descriptor_valid(&descriptor) {
                (
                    disabled_state.unwrap_or(ContributionRuntimeState::Active),
                    Some("event_source_uses_authenticated_scheduler_ingress".into()),
                )
            } else {
                (
                    ContributionRuntimeState::Failed,
                    Some("plugin_event_source_descriptor_invalid".into()),
                )
            }
        }
        PluginContributionKind::Mcp => {
            read_json_object(&target)?;
            (
                disabled_state.unwrap_or(ContributionRuntimeState::Active),
                Some("plugin_mcp_registered_disabled_by_default".into()),
            )
        }
        PluginContributionKind::Connector => {
            let descriptor = connector_descriptor(plugin, &contribution.id)?;
            if drivers.resolve(&descriptor.host_identity).is_some() {
                (
                    disabled_state.unwrap_or(ContributionRuntimeState::Active),
                    None,
                )
            } else {
                (
                    ContributionRuntimeState::Unsupported,
                    Some("connector_driver_unavailable".into()),
                )
            }
        }
        PluginContributionKind::BrowserExtension => {
            let manifest = target.join("manifest.json");
            if target.is_dir() && read_json_object(&manifest).is_ok() {
                (
                    disabled_state.unwrap_or(ContributionRuntimeState::Active),
                    Some("browser_extension_requires_explicit_install".into()),
                )
            } else {
                (
                    ContributionRuntimeState::Failed,
                    Some("browser_extension_manifest_invalid".into()),
                )
            }
        }
        PluginContributionKind::ScheduledTaskTemplate => {
            read_json_object(&target)?;
            (
                disabled_state.unwrap_or(ContributionRuntimeState::Active),
                Some("scheduled_template_registered_disabled_by_default".into()),
            )
        }
        PluginContributionKind::Asset => {
            if target.is_dir() && validate_static_surface(&target, false).is_ok() {
                (
                    disabled_state.unwrap_or(ContributionRuntimeState::Active),
                    Some("asset_read_only".into()),
                )
            } else {
                (
                    ContributionRuntimeState::Failed,
                    Some("plugin_asset_root_invalid".into()),
                )
            }
        }
        PluginContributionKind::CustomUi => {
            if !target.join("index.html").is_file()
                || validate_static_surface(&target, true).is_err()
            {
                (
                    ContributionRuntimeState::Failed,
                    Some("custom_ui_tauri_ipc_or_active_object_denied".into()),
                )
            } else {
                (
                    disabled_state.unwrap_or(ContributionRuntimeState::Active),
                    Some("custom_ui_csp_sandbox_bridge_only".into()),
                )
            }
        }
        PluginContributionKind::Channel => {
            read_json_object(&target)?;
            (
                disabled_state.unwrap_or(ContributionRuntimeState::Active),
                Some("channel_provider_requires_account_configuration".into()),
            )
        }
    };
    Ok(InstalledContribution {
        plugin_id: plugin.manifest.id.clone(),
        contribution_id: contribution.id.clone(),
        kind: contribution.kind,
        content_hash,
        runtime_revision,
        state,
        diagnostic,
    })
}

fn connector_revision(
    plugin: &InstalledPlugin,
    connector_id: &str,
) -> Result<ConnectorRevision, ExtensionHostError> {
    connector_descriptor(plugin, connector_id).map(|descriptor| descriptor.revision)
}

fn revision_matches(expected: &ContributionRevision, current: &ConnectorRevision) -> bool {
    expected
        .host_identity_hash
        .as_ref()
        .is_none_or(|value| value == &current.host_identity_hash)
        && expected
            .schema_hash
            .as_ref()
            .is_none_or(|value| value == &current.schema_hash)
        && expected
            .action_hash
            .as_ref()
            .is_none_or(|value| value == &current.action_hash)
}

fn revision_drift_health(
    expected: &ConnectorRevision,
    current: &ConnectorRevision,
) -> ConnectorHealth {
    if expected.host_identity_hash != current.host_identity_hash {
        ConnectorHealth::HostIdentityDrift
    } else if expected.schema_hash != current.schema_hash {
        ConnectorHealth::SchemaDrift
    } else {
        ConnectorHealth::ActionDrift
    }
}

fn connector_secret_reference(account_id: &ConnectorAccountId) -> String {
    format!("keyring:connector:{}", account_id.as_str())
}

fn connector_keyring_entry(
    account_id: &ConnectorAccountId,
) -> Result<keyring::Entry, ExtensionHostError> {
    keyring::Entry::new("com.hachimi.connector", account_id.as_str())
        .map_err(|_| ExtensionHostError::SecretStore)
}

fn connector_secret(
    reference: &str,
    account_id: &ConnectorAccountId,
) -> Result<Option<String>, ExtensionHostError> {
    if reference != connector_secret_reference(account_id) {
        return Err(ExtensionHostError::SecretStore);
    }
    match connector_keyring_entry(account_id)?.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(ExtensionHostError::SecretStore),
    }
}

async fn replay_invocation(
    store: &AgentStore,
    request: &ConnectorInvocationRequest,
    argument_hash: &str,
) -> Result<Option<ConnectorInvocationResult>, ExtensionHostError> {
    let row = sqlx::query(
        "SELECT action, argument_hash, result_json, metadata_json FROM connector_invocations WHERE account_id = ? AND idempotency_key = ?",
    )
    .bind(request.account_id.as_str())
    .bind(&request.idempotency_key)
    .fetch_optional(store.pool())
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.get::<String, _>("action") != request.action
        || row.get::<String, _>("argument_hash") != argument_hash
    {
        return Err(ExtensionHostError::IdempotencyConflict);
    }
    Ok(Some(ConnectorInvocationResult {
        account_id: request.account_id.clone(),
        action: request.action.clone(),
        result: serde_json::from_str(row.get("result_json"))?,
        metadata: serde_json::from_str(row.get("metadata_json"))?,
        replayed: true,
    }))
}

async fn begin_connector_attempt(
    store: &AgentStore,
    request: &ConnectorInvocationRequest,
    now: i64,
) -> Result<u32, ExtensionHostError> {
    sqlx::query(
        "INSERT INTO connector_retry_ledger(account_id, idempotency_key, attempt, next_attempt_at_ms, last_error, updated_at_ms) VALUES(?, ?, 1, NULL, NULL, ?) ON CONFLICT(account_id, idempotency_key) DO UPDATE SET attempt = connector_retry_ledger.attempt + 1, next_attempt_at_ms = NULL, last_error = NULL, updated_at_ms = excluded.updated_at_ms",
    )
    .bind(request.account_id.as_str())
    .bind(&request.idempotency_key)
    .bind(now)
    .execute(store.pool())
    .await?;
    let row = sqlx::query(
        "SELECT attempt FROM connector_retry_ledger WHERE account_id = ? AND idempotency_key = ?",
    )
    .bind(request.account_id.as_str())
    .bind(&request.idempotency_key)
    .fetch_one(store.pool())
    .await?;
    Ok(u32::try_from(row.get::<i64, _>("attempt")).unwrap_or(u32::MAX))
}

async fn finish_connector_attempt(
    store: &AgentStore,
    request: &ConnectorInvocationRequest,
    attempt: u32,
    error: Option<&str>,
    next_attempt_at_ms: i64,
) -> Result<(), ExtensionHostError> {
    sqlx::query(
        "UPDATE connector_retry_ledger SET attempt = ?, next_attempt_at_ms = ?, last_error = ?, updated_at_ms = ? WHERE account_id = ? AND idempotency_key = ?",
    )
    .bind(i64::from(attempt))
    .bind((next_attempt_at_ms > 0).then_some(next_attempt_at_ms))
    .bind(error)
    .bind(now_ms())
    .bind(request.account_id.as_str())
    .bind(&request.idempotency_key)
    .execute(store.pool())
    .await?;
    Ok(())
}

async fn allow_connector_rate(
    store: &AgentStore,
    account_id: &ConnectorAccountId,
    now: i64,
) -> Result<bool, ExtensionHostError> {
    const WINDOW_MS: i64 = 1_000;
    const MAX_CALLS: i64 = 5;
    let row = sqlx::query(
        "SELECT window_started_at_ms, invocation_count FROM connector_rate_limits WHERE account_id = ?",
    )
    .bind(account_id.as_str())
    .fetch_optional(store.pool())
    .await?;
    let (window, count) = row
        .map(|row| {
            (
                row.get::<i64, _>("window_started_at_ms"),
                row.get::<i64, _>("invocation_count"),
            )
        })
        .unwrap_or((now, 0));
    let (window, count) = if now.saturating_sub(window) >= WINDOW_MS {
        (now, 0)
    } else {
        (window, count)
    };
    let allowed = count < MAX_CALLS;
    sqlx::query(
        "INSERT INTO connector_rate_limits(account_id, window_started_at_ms, invocation_count, updated_at_ms) VALUES(?, ?, ?, ?) ON CONFLICT(account_id) DO UPDATE SET window_started_at_ms = excluded.window_started_at_ms, invocation_count = excluded.invocation_count, updated_at_ms = excluded.updated_at_ms",
    )
    .bind(account_id.as_str())
    .bind(window)
    .bind(count + i64::from(allowed))
    .bind(now)
    .execute(store.pool())
    .await?;
    Ok(allowed)
}

async fn rate_limit_window_expired(
    store: &AgentStore,
    account_id: &ConnectorAccountId,
) -> Result<bool, ExtensionHostError> {
    let row =
        sqlx::query("SELECT window_started_at_ms FROM connector_rate_limits WHERE account_id = ?")
            .bind(account_id.as_str())
            .fetch_optional(store.pool())
            .await?;
    Ok(row.is_none_or(|row| {
        now_ms().saturating_sub(row.get::<i64, _>("window_started_at_ms")) >= 1_000
    }))
}

fn value_hash(value: &Value) -> Result<String, ExtensionHostError> {
    Ok(hex_digest(
        Sha256::digest(serde_json::to_vec(value)?).as_slice(),
    ))
}

fn connector_target_summary(connector_id: &str, account_id: &str) -> String {
    format!(
        "connector:{connector_id}:account_sha256:{}",
        hex_digest(Sha256::digest(account_id.as_bytes()).as_slice())
    )
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn string_argument<'a>(value: &'a Value, name: &str) -> Result<&'a str, ExtensionHostError> {
    value
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.chars().count() <= 128)
        .ok_or(ExtensionHostError::InvalidInvocation)
}

fn object_argument<'a>(
    value: &'a Value,
    name: &str,
) -> Result<&'a serde_json::Map<String, Value>, ExtensionHostError> {
    value
        .get(name)
        .and_then(Value::as_object)
        .filter(|value| value.len() <= 128)
        .ok_or(ExtensionHostError::InvalidInvocation)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
