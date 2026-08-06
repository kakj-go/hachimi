use std::path::{Component, Path, PathBuf};

use hachimi_control_plane::AppServerDomainError;
use hachimi_protocol::{
    InstalledContribution, InstalledPlugin, McpHeaderView, McpServerId, McpServerRecord,
    McpServerTransport, PluginContribution, PluginContributionKind, PluginContributionSurface,
    PluginId, PluginUiBridgeMethod, ScheduleDefinition, ScheduleId,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::Row;

use super::{DesktopAppDomainHandler, domain_error, now_ms};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginMcpDescriptor {
    display_name: String,
    transport: McpServerTransport,
    #[serde(default)]
    read_only_tools: Vec<String>,
    #[serde(default = "default_startup_timeout")]
    startup_timeout_ms: u64,
    #[serde(default = "default_request_timeout")]
    request_timeout_ms: u64,
    #[serde(default = "default_message_limit")]
    max_message_bytes: u64,
}

impl DesktopAppDomainHandler {
    pub(super) async fn plugin_contribution_surface(
        &self,
        plugin_id: &PluginId,
        contribution_id: &str,
    ) -> Result<PluginContributionSurface, AppServerDomainError> {
        let runtime = self
            .plugins
            .list_contributions(Some(plugin_id))
            .await
            .map_err(domain_error("plugin_surface_runtime_lookup_failed"))?
            .into_iter()
            .find(|runtime| runtime.contribution_id == contribution_id)
            .ok_or_else(|| {
                AppServerDomainError::new(
                    "plugin_contribution_not_found",
                    "Plugin contribution is not installed",
                )
            })?;
        let mut surface = self
            .plugin_surfaces
            .surface(plugin_id, contribution_id)
            .unwrap_or(PluginContributionSurface {
                plugin_id: plugin_id.clone(),
                contribution_id: contribution_id.to_owned(),
                kind: runtime.kind,
                runtime_revision: runtime.runtime_revision.clone(),
                runtime_state: runtime.state,
                diagnostic: runtime.diagnostic.clone(),
                last_result_code: None,
                entry_url: None,
                asset_base_url: None,
                allowed_bridge_methods: Vec::<PluginUiBridgeMethod>::new(),
            });
        surface.runtime_state = runtime.state;
        surface.diagnostic = runtime.diagnostic;
        if runtime.kind == PluginContributionKind::Hook {
            surface.last_result_code = sqlx::query_scalar(
                "SELECT result_code FROM plugin_hook_executions WHERE plugin_id = ? AND contribution_id = ? ORDER BY id DESC LIMIT 1",
            )
            .bind(plugin_id.as_str())
            .bind(contribution_id)
            .fetch_optional(self.store.pool())
            .await
            .map_err(domain_error("plugin_hook_outcome_lookup_failed"))?;
        }
        Ok(surface)
    }

    pub(super) async fn reconcile_plugin_products(
        &self,
        plugin: &InstalledPlugin,
        enabled: bool,
    ) -> Result<(), AppServerDomainError> {
        let enterprise_disabled = is_enterprise_plugin(&plugin.manifest.id)
            && !self.features.runtime_features.enterprise_integrations;
        if enterprise_disabled && enabled {
            return Err(AppServerDomainError::new(
                "feature_disabled",
                "enterprise_integrations",
            ));
        }
        let runtimes = self
            .plugins
            .list_contributions(Some(&plugin.manifest.id))
            .await
            .map_err(domain_error("plugin_product_runtime_lookup_failed"))?;
        for contribution in &plugin.manifest.contributions {
            let Some(runtime) = runtimes
                .iter()
                .find(|runtime| runtime.contribution_id == contribution.id)
            else {
                continue;
            };
            if matches!(
                runtime.state,
                hachimi_protocol::ContributionRuntimeState::Failed
                    | hachimi_protocol::ContributionRuntimeState::Unsupported
            ) {
                continue;
            }
            if enterprise_disabled {
                self.plugins
                    .transition_contribution(
                        &plugin.manifest.id,
                        &contribution.id,
                        &runtime.runtime_revision,
                        hachimi_protocol::ContributionRuntimeState::Disabled,
                        Some("feature_disabled:enterprise_integrations"),
                    )
                    .await
                    .map_err(domain_error("plugin_product_state_transition_failed"))?;
                continue;
            }
            let transitional_state = if enabled {
                hachimi_protocol::ContributionRuntimeState::Starting
            } else {
                hachimi_protocol::ContributionRuntimeState::Registered
            };
            self.plugins
                .transition_contribution(
                    &plugin.manifest.id,
                    &contribution.id,
                    &runtime.runtime_revision,
                    transitional_state,
                    Some(if enabled {
                        "plugin_product_starting"
                    } else {
                        "plugin_product_registered_disabled"
                    }),
                )
                .await
                .map_err(domain_error("plugin_product_state_transition_failed"))?;
            let reconciliation = match contribution.kind {
                PluginContributionKind::Mcp => {
                    self.reconcile_plugin_mcp(plugin, contribution, runtime, enabled)
                        .await
                }
                PluginContributionKind::ScheduledTaskTemplate => {
                    self.reconcile_schedule_template(plugin, contribution, runtime, enabled)
                        .await
                }
                PluginContributionKind::BrowserExtension
                | PluginContributionKind::Asset
                | PluginContributionKind::CustomUi => {
                    async {
                        self.persist_passive_binding(plugin, contribution, runtime, enabled)
                            .await?;
                        self.plugin_surfaces
                            .reconcile(
                                &plugin.manifest.id,
                                contribution,
                                runtime,
                                &contribution_target(plugin, contribution)?,
                                enabled,
                            )
                            .map_err(domain_error("plugin_surface_reconcile_failed"))
                    }
                    .await
                }
                PluginContributionKind::Channel => {
                    self.reconcile_plugin_channel(plugin, contribution, runtime, enabled)
                        .await
                }
                PluginContributionKind::Skill
                | PluginContributionKind::Hook
                | PluginContributionKind::EventSource
                | PluginContributionKind::Connector => Ok(()),
            };
            if let Err(error) = reconciliation {
                let _ = self
                    .plugins
                    .transition_contribution(
                        &plugin.manifest.id,
                        &contribution.id,
                        &runtime.runtime_revision,
                        hachimi_protocol::ContributionRuntimeState::Failed,
                        Some(&error.code),
                    )
                    .await;
                return Err(error);
            }
            self.plugins
                .transition_contribution(
                    &plugin.manifest.id,
                    &contribution.id,
                    &runtime.runtime_revision,
                    if enabled {
                        hachimi_protocol::ContributionRuntimeState::Active
                    } else {
                        hachimi_protocol::ContributionRuntimeState::Disabled
                    },
                    runtime.diagnostic.as_deref(),
                )
                .await
                .map_err(domain_error("plugin_product_state_commit_failed"))?;
        }
        Ok(())
    }

    pub(super) async fn remove_plugin_products(
        &self,
        plugin_id: &PluginId,
    ) -> Result<(), AppServerDomainError> {
        self.plugin_surfaces.remove_plugin(plugin_id);
        self.gateway
            .set_plugin_providers_enabled(plugin_id, false)
            .await
            .map_err(domain_error("plugin_channel_disable_failed"))?;
        let rows = sqlx::query(
            "SELECT resource_kind, resource_id FROM plugin_runtime_bindings WHERE plugin_id = ?",
        )
        .bind(plugin_id.as_str())
        .fetch_all(self.store.pool())
        .await
        .map_err(domain_error("plugin_product_binding_lookup_failed"))?;
        for row in rows {
            let kind = row.get::<String, _>("resource_kind");
            let resource_id = row.get::<String, _>("resource_id");
            match removal_action(&kind) {
                ProductRemovalAction::Mcp => {
                    self.mcp
                        .remove(&McpServerId::new(resource_id))
                        .await
                        .map_err(domain_error("plugin_mcp_remove_failed"))?;
                }
                ProductRemovalAction::Schedule => {
                    self.scheduler
                        .remove(&ScheduleId::new(resource_id))
                        .await
                        .map_err(domain_error("plugin_schedule_template_remove_failed"))?;
                }
                ProductRemovalAction::BuiltinChannel => {
                    self.gateway
                        .set_builtin_provider_contribution_enabled(&resource_id, false)
                        .await
                        .map_err(domain_error("plugin_builtin_channel_disable_failed"))?;
                }
                ProductRemovalAction::BindingOnly => {}
            }
        }
        Ok(())
    }

    pub(crate) async fn reconcile_plugin_startup(&self) -> Result<(), AppServerDomainError> {
        self.plugins
            .reconcile_lifecycle()
            .await
            .map_err(domain_error("plugin_lifecycle_reconcile_failed"))?;
        self.refresh_plugin_sidecar_drivers().await?;
        for plugin in self
            .plugins
            .list()
            .await
            .map_err(domain_error("plugin_startup_list_failed"))?
        {
            let enabled = plugin.status == hachimi_protocol::PluginStatus::Enabled;
            self.reconcile_plugin_products(&plugin, enabled).await?;
        }
        self.refresh_plugin_skill_roots().await
    }

    async fn reconcile_plugin_channel(
        &self,
        plugin: &InstalledPlugin,
        contribution: &PluginContribution,
        runtime: &InstalledContribution,
        enabled: bool,
    ) -> Result<(), AppServerDomainError> {
        if let Some(provider_id) = self
            .plugins
            .builtin_enterprise_channel_provider_id(plugin, &contribution.id)
            .map_err(domain_error("plugin_builtin_channel_descriptor_invalid"))?
        {
            self.gateway
                .set_builtin_provider_contribution_enabled(&provider_id, enabled)
                .await
                .map_err(domain_error("plugin_builtin_channel_reconcile_failed"))?;
            return self
                .upsert_binding(
                    plugin,
                    contribution,
                    runtime,
                    "builtin_channel",
                    &provider_id,
                    enabled,
                    json!({
                        "runtime": "builtin_enterprise",
                        "requiresAccountConfiguration": true,
                        "providerId": provider_id,
                    }),
                )
                .await;
        }
        if enabled {
            let definition = self
                .plugins
                .channel_sidecar_definition(plugin, &contribution.id)
                .map_err(domain_error("plugin_channel_descriptor_invalid"))?;
            let backend: std::sync::Arc<dyn hachimi_sandbox::SandboxBackend> =
                self.sandbox_runtime.clone();
            let provider = hachimi_gateway::SandboxedStdioChannelProvider::new(
                backend,
                definition.manifest,
                definition.bundle_root,
                definition.executable,
                definition.args,
            )
            .map_err(domain_error("plugin_channel_runtime_invalid"))?;
            self.gateway
                .register_provider(std::sync::Arc::new(provider), true)
                .await
                .map_err(domain_error("plugin_channel_register_failed"))?;
        } else {
            self.gateway
                .set_plugin_providers_enabled(&plugin.manifest.id, false)
                .await
                .map_err(domain_error("plugin_channel_disable_failed"))?;
        }
        self.persist_passive_binding(plugin, contribution, runtime, enabled)
            .await
    }

    async fn reconcile_plugin_mcp(
        &self,
        plugin: &InstalledPlugin,
        contribution: &PluginContribution,
        runtime: &InstalledContribution,
        plugin_enabled: bool,
    ) -> Result<(), AppServerDomainError> {
        let target = contribution_target(plugin, contribution)?;
        let mut descriptor: PluginMcpDescriptor = serde_json::from_slice(
            &std::fs::read(&target).map_err(domain_error("plugin_mcp_descriptor_read_failed"))?,
        )
        .map_err(domain_error("plugin_mcp_descriptor_invalid"))?;
        descriptor.transport = resolve_mcp_transport(plugin, descriptor.transport)?;
        descriptor.read_only_tools.sort();
        descriptor.read_only_tools.dedup();
        let resource_id = binding_id("plugin-mcp", plugin, contribution);
        let server_id = McpServerId::new(resource_id.clone());
        let previous_binding = self
            .binding_revision(&plugin.manifest.id, &contribution.id, "mcp")
            .await?;
        let existing = self
            .store
            .get_mcp_server(&server_id)
            .await
            .map_err(domain_error("plugin_mcp_lookup_failed"))?;
        let same_revision = previous_binding.as_deref() == Some(&runtime.runtime_revision);
        let created_at_ms = existing
            .as_ref()
            .map_or_else(now_ms, |value| value.created_at_ms);
        let keep_enabled = mcp_should_remain_enabled(
            plugin_enabled,
            same_revision,
            existing.as_ref().is_some_and(|value| value.enabled),
        );
        let record = McpServerRecord {
            id: server_id,
            display_name: descriptor.display_name,
            enabled: keep_enabled,
            transport: descriptor.transport,
            headers: Vec::<McpHeaderView>::new(),
            read_only_tools: descriptor.read_only_tools,
            startup_timeout_ms: descriptor.startup_timeout_ms,
            request_timeout_ms: descriptor.request_timeout_ms,
            max_message_bytes: descriptor.max_message_bytes,
            created_at_ms,
            updated_at_ms: now_ms(),
        };
        self.mcp
            .upsert(&record)
            .await
            .map_err(domain_error("plugin_mcp_register_failed"))?;
        self.upsert_binding(
            plugin,
            contribution,
            runtime,
            "mcp",
            &resource_id,
            keep_enabled,
            json!({"displayName": record.display_name, "disabledByDefault": true}),
        )
        .await
    }

    async fn reconcile_schedule_template(
        &self,
        plugin: &InstalledPlugin,
        contribution: &PluginContribution,
        runtime: &InstalledContribution,
        plugin_enabled: bool,
    ) -> Result<(), AppServerDomainError> {
        let resource_id = binding_id("plugin-schedule", plugin, contribution);
        let schedule_id = ScheduleId::new(resource_id.clone());
        let previous_revision = self
            .binding_revision(
                &plugin.manifest.id,
                &contribution.id,
                "scheduled_task_template",
            )
            .await?;
        let current = self
            .store
            .get_schedule(&schedule_id)
            .await
            .map_err(domain_error("plugin_schedule_template_lookup_failed"))?;
        if previous_revision.as_deref() != Some(&runtime.runtime_revision) || current.is_none() {
            if current.is_some() {
                self.scheduler
                    .remove(&schedule_id)
                    .await
                    .map_err(domain_error("plugin_schedule_template_replace_failed"))?;
            }
            let target = contribution_target(plugin, contribution)?;
            let mut draft: ScheduleDefinition = serde_json::from_slice(
                &std::fs::read(target)
                    .map_err(domain_error("plugin_schedule_template_read_failed"))?,
            )
            .map_err(domain_error("plugin_schedule_template_invalid"))?;
            prepare_schedule_template(&mut draft, plugin, schedule_id.clone());
            self.scheduler
                .create(
                    &format!("plugin:{}", plugin.manifest.id),
                    &format!(
                        "plugin-template:{}:{}",
                        plugin.content_hash, contribution.id
                    ),
                    draft,
                )
                .await
                .map_err(domain_error("plugin_schedule_template_register_failed"))?;
        } else if !plugin_enabled
            && let Some(schedule) = current
            && schedule.enabled
        {
            self.scheduler
                .set_enabled(&schedule.id, false, schedule.config_revision)
                .await
                .map_err(domain_error("plugin_schedule_template_disable_failed"))?;
        }
        self.upsert_binding(
            plugin,
            contribution,
            runtime,
            "scheduled_task_template",
            &resource_id,
            false,
            json!({"disabledDraft": true}),
        )
        .await
    }

    async fn persist_passive_binding(
        &self,
        plugin: &InstalledPlugin,
        contribution: &PluginContribution,
        runtime: &InstalledContribution,
        enabled: bool,
    ) -> Result<(), AppServerDomainError> {
        let kind = contribution.kind.as_str();
        let resource_id = binding_id(kind, plugin, contribution);
        let target = contribution_target(plugin, contribution)?;
        let metadata = passive_binding_metadata(contribution.kind, runtime, &target);
        self.upsert_binding(
            plugin,
            contribution,
            runtime,
            kind,
            &resource_id,
            enabled,
            metadata,
        )
        .await
    }

    async fn binding_revision(
        &self,
        plugin_id: &PluginId,
        contribution_id: &str,
        resource_kind: &str,
    ) -> Result<Option<String>, AppServerDomainError> {
        sqlx::query_scalar(
            "SELECT runtime_revision FROM plugin_runtime_bindings WHERE plugin_id = ? AND contribution_id = ? AND resource_kind = ?",
        )
        .bind(plugin_id.as_str())
        .bind(contribution_id)
        .bind(resource_kind)
        .fetch_optional(self.store.pool())
        .await
        .map_err(domain_error("plugin_product_binding_lookup_failed"))
    }

    #[allow(clippy::too_many_arguments)]
    async fn upsert_binding(
        &self,
        plugin: &InstalledPlugin,
        contribution: &PluginContribution,
        runtime: &InstalledContribution,
        resource_kind: &str,
        resource_id: &str,
        enabled: bool,
        metadata: Value,
    ) -> Result<(), AppServerDomainError> {
        sqlx::query("INSERT INTO plugin_runtime_bindings(plugin_id, contribution_id, resource_kind, resource_id, runtime_revision, metadata_json, enabled, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(plugin_id, contribution_id, resource_kind) DO UPDATE SET resource_id = excluded.resource_id, runtime_revision = excluded.runtime_revision, metadata_json = excluded.metadata_json, enabled = excluded.enabled, updated_at_ms = excluded.updated_at_ms")
            .bind(plugin.manifest.id.as_str())
            .bind(&contribution.id)
            .bind(resource_kind)
            .bind(resource_id)
            .bind(&runtime.runtime_revision)
            .bind(serde_json::to_string(&metadata).map_err(domain_error("plugin_product_binding_encode_failed"))?)
            .bind(enabled)
            .bind(now_ms())
            .execute(self.store.pool())
            .await
            .map_err(domain_error("plugin_product_binding_store_failed"))?;
        Ok(())
    }
}

fn is_enterprise_plugin(plugin_id: &PluginId) -> bool {
    matches!(
        plugin_id.as_str(),
        "dingtalk" | "feishu" | "wecom_ai_bot" | "wecom_app" | "wechat_ilink"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProductRemovalAction {
    Mcp,
    Schedule,
    BuiltinChannel,
    BindingOnly,
}

fn removal_action(resource_kind: &str) -> ProductRemovalAction {
    match resource_kind {
        "mcp" => ProductRemovalAction::Mcp,
        "scheduled_task_template" => ProductRemovalAction::Schedule,
        "builtin_channel" => ProductRemovalAction::BuiltinChannel,
        _ => ProductRemovalAction::BindingOnly,
    }
}

const fn mcp_should_remain_enabled(
    plugin_enabled: bool,
    same_revision: bool,
    existing_enabled: bool,
) -> bool {
    plugin_enabled && same_revision && existing_enabled
}

fn prepare_schedule_template(
    draft: &mut ScheduleDefinition,
    plugin: &InstalledPlugin,
    schedule_id: ScheduleId,
) {
    draft.id = schedule_id;
    draft.enabled = false;
    draft.created_by = format!("plugin:{}", plugin.manifest.id);
    draft.next_run_at_ms = None;
}

fn passive_binding_metadata(
    kind: PluginContributionKind,
    runtime: &InstalledContribution,
    target: &Path,
) -> Value {
    match kind {
        PluginContributionKind::Asset => json!({
            "readOnly": true,
            "scheme": "hachimi-plugin-asset",
            "rootHash": runtime.content_hash,
        }),
        PluginContributionKind::CustomUi => json!({
            "sandbox": "allow-scripts",
            "csp": "default-src 'none'; img-src data: hachimi-plugin-asset: http://hachimi-plugin-asset.localhost; style-src 'self'; script-src 'self'; connect-src hachimi-plugin-asset: http://hachimi-plugin-asset.localhost; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; navigate-to 'none'",
            "typedBridgeOnly": true,
        }),
        PluginContributionKind::BrowserExtension => json!({
            "contentHash": runtime.content_hash,
            "requiresExplicitChromeInstall": true,
        }),
        PluginContributionKind::Channel => json!({
            "descriptorPathHash": stable_hash(target.to_string_lossy().as_bytes()),
            "requiresAccountConfiguration": true,
        }),
        _ => Value::Null,
    }
}

fn contribution_target(
    plugin: &InstalledPlugin,
    contribution: &PluginContribution,
) -> Result<PathBuf, AppServerDomainError> {
    let relative = Path::new(&contribution.relative_path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(AppServerDomainError::new(
            "plugin_product_path_invalid",
            "Plugin product path escapes the installed bundle",
        ));
    }
    let root = PathBuf::from(&plugin.root_path)
        .canonicalize()
        .map_err(domain_error("plugin_product_root_missing"))?;
    let target = root
        .join(relative)
        .canonicalize()
        .map_err(domain_error("plugin_product_missing"))?;
    if !target.starts_with(&root) {
        return Err(AppServerDomainError::new(
            "plugin_product_path_invalid",
            "Plugin product path escapes the installed bundle",
        ));
    }
    Ok(target)
}

fn resolve_mcp_transport(
    plugin: &InstalledPlugin,
    transport: McpServerTransport,
) -> Result<McpServerTransport, AppServerDomainError> {
    match transport {
        McpServerTransport::Stdio { command, args, cwd } => {
            let command_path = Path::new(&command);
            if command_path.is_absolute()
                || command_path
                    .components()
                    .any(|component| !matches!(component, Component::Normal(_)))
            {
                return Err(AppServerDomainError::new(
                    "plugin_mcp_command_invalid",
                    "Plugin MCP command must be a bundle-relative executable",
                ));
            }
            let root = PathBuf::from(&plugin.root_path)
                .canonicalize()
                .map_err(domain_error("plugin_mcp_root_missing"))?;
            let executable = root
                .join(command_path)
                .canonicalize()
                .map_err(domain_error("plugin_mcp_command_missing"))?;
            if !executable.starts_with(&root) || !executable.is_file() {
                return Err(AppServerDomainError::new(
                    "plugin_mcp_command_invalid",
                    "Plugin MCP command escapes the installed bundle",
                ));
            }
            let cwd = cwd.map_or_else(
                || Ok(root.clone()),
                |cwd| {
                    let relative = Path::new(&cwd);
                    if relative.is_absolute()
                        || relative
                            .components()
                            .any(|component| !matches!(component, Component::Normal(_)))
                    {
                        return Err(AppServerDomainError::new(
                            "plugin_mcp_cwd_invalid",
                            "Plugin MCP cwd must remain inside the bundle",
                        ));
                    }
                    let resolved = root
                        .join(relative)
                        .canonicalize()
                        .map_err(domain_error("plugin_mcp_cwd_missing"))?;
                    resolved
                        .starts_with(&root)
                        .then_some(resolved)
                        .ok_or_else(|| {
                            AppServerDomainError::new(
                                "plugin_mcp_cwd_invalid",
                                "Plugin MCP cwd escapes the installed bundle",
                            )
                        })
                },
            )?;
            Ok(McpServerTransport::Stdio {
                command: executable.to_string_lossy().into_owned(),
                args,
                cwd: Some(cwd.to_string_lossy().into_owned()),
            })
        }
        remote @ McpServerTransport::StreamableHttp { .. } => Ok(remote),
    }
}

fn binding_id(prefix: &str, plugin: &InstalledPlugin, contribution: &PluginContribution) -> String {
    let digest = stable_hash(
        format!(
            "{}:{}:{}",
            plugin.manifest.id,
            contribution.kind.as_str(),
            contribution.id
        )
        .as_bytes(),
    );
    format!("{prefix}-{}", &digest[..24])
}

fn stable_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

const fn default_startup_timeout() -> u64 {
    30_000
}

const fn default_request_timeout() -> u64 {
    60_000
}

const fn default_message_limit() -> u64 {
    4 * 1024 * 1024
}

#[cfg(test)]
mod tests {
    use super::*;
    use hachimi_protocol::{
        AgentPermissionPolicy, ContributionRuntimeState, DeliveryPolicy, EntryProfile,
        HostRevisionSnapshot, MisfirePolicy, PermissionProfile, PluginManifest, PluginStatus,
        ScheduleContextTemplate, ScheduleHealth, ScheduleSpec, ScheduleStopConditions,
    };

    fn plugin() -> InstalledPlugin {
        InstalledPlugin {
            manifest: PluginManifest {
                manifest_version: 1,
                id: PluginId::from("product-test"),
                name: "Product test".into(),
                version: "1.0.0".into(),
                description: "fixture".into(),
                contributions: Vec::new(),
            },
            content_hash: "plugin-hash".into(),
            root_path: ".".into(),
            status: PluginStatus::Enabled,
            diagnostics: Vec::new(),
            installed_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn runtime(kind: PluginContributionKind) -> InstalledContribution {
        InstalledContribution {
            plugin_id: PluginId::from("product-test"),
            contribution_id: "fixture".into(),
            kind,
            content_hash: "contribution-hash".into(),
            runtime_revision: "runtime-revision".into(),
            state: ContributionRuntimeState::Active,
            diagnostic: None,
        }
    }

    #[test]
    fn mcp_upgrade_never_preserves_enablement_across_revision_drift() {
        assert!(mcp_should_remain_enabled(true, true, true));
        assert!(!mcp_should_remain_enabled(true, false, true));
        assert!(!mcp_should_remain_enabled(false, true, true));
        assert!(!mcp_should_remain_enabled(true, true, false));
    }

    #[test]
    fn uninstall_cleanup_removes_owned_mcp_and_schedule_products() {
        assert_eq!(removal_action("mcp"), ProductRemovalAction::Mcp);
        assert_eq!(
            removal_action("scheduled_task_template"),
            ProductRemovalAction::Schedule
        );
        assert_eq!(removal_action("asset"), ProductRemovalAction::BindingOnly);
    }

    #[test]
    fn passive_products_are_read_only_and_custom_ui_is_bridge_only() {
        let asset = passive_binding_metadata(
            PluginContributionKind::Asset,
            &runtime(PluginContributionKind::Asset),
            Path::new("asset.txt"),
        );
        assert_eq!(asset["readOnly"], true);
        assert_eq!(asset["scheme"], "hachimi-plugin-asset");

        let custom_ui = passive_binding_metadata(
            PluginContributionKind::CustomUi,
            &runtime(PluginContributionKind::CustomUi),
            Path::new("index.html"),
        );
        assert_eq!(custom_ui["sandbox"], "allow-scripts");
        assert_eq!(custom_ui["typedBridgeOnly"], true);
        let csp = custom_ui["csp"].as_str().expect("CSP");
        assert!(csp.contains("default-src 'none'"));
        assert!(csp.contains("connect-src hachimi-plugin-asset:"));
        assert!(csp.contains("frame-src 'none'"));
        assert!(csp.contains("form-action 'none'"));

        let browser_extension = passive_binding_metadata(
            PluginContributionKind::BrowserExtension,
            &runtime(PluginContributionKind::BrowserExtension),
            Path::new("extension"),
        );
        assert_eq!(browser_extension["requiresExplicitChromeInstall"], true);

        let channel = passive_binding_metadata(
            PluginContributionKind::Channel,
            &runtime(PluginContributionKind::Channel),
            Path::new("channel.json"),
        );
        assert_eq!(channel["requiresAccountConfiguration"], true);
        assert_eq!(
            channel["descriptorPathHash"].as_str().map(str::len),
            Some(64)
        );
    }

    #[test]
    fn scheduled_template_is_always_an_unauthorized_disabled_draft() {
        let mut draft = ScheduleDefinition {
            id: ScheduleId::from("untrusted-id"),
            name: "Template".into(),
            enabled: true,
            prompt: "fixture".into(),
            schedule: ScheduleSpec::Every {
                interval_ms: 60_000,
                anchor_ms: 60_000,
            },
            entry_profile: EntryProfile::Workbench,
            workload_override: None,
            context_template: ScheduleContextTemplate::Workspace {
                workspace: hachimi_protocol::ScheduleWorkspaceSpec::Managed,
                conversation_mode: hachimi_protocol::ScheduleConversationMode::PerRunSession,
            },
            skill_allowlist: Vec::new(),
            skill_revisions: Vec::new(),
            mcp_tool_allowlist: Vec::new(),
            contribution_revisions: Vec::new(),
            host_revision_snapshot: HostRevisionSnapshot::default(),
            permission_policy: AgentPermissionPolicy {
                level: PermissionProfile::ReadOnly,
                ..AgentPermissionPolicy::default()
            },
            permission_revision: 99,
            timeout_ms: 60_000,
            misfire_policy: MisfirePolicy::Skip,
            delivery_policy: DeliveryPolicy::TaskTabOnly,
            stop_conditions: ScheduleStopConditions::default(),
            config_revision: 99,
            created_by: "untrusted".into(),
            next_run_at_ms: Some(123),
            health: ScheduleHealth::Healthy,
            health_reason: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        prepare_schedule_template(
            &mut draft,
            &plugin(),
            ScheduleId::from("plugin-owned-draft"),
        );
        assert_eq!(draft.id.as_str(), "plugin-owned-draft");
        assert!(!draft.enabled);
        assert_eq!(draft.created_by, "plugin:product-test");
        assert_eq!(draft.next_run_at_ms, None);
    }
}
