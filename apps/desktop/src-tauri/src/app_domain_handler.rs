//! Desktop-owned implementations of transport-neutral App Server domains.
//!
//! Native dialogs and WebView event bridging remain Tauri adapter concerns;
//! MCP and Skill business operations run here so background and future local
//! transports do not need to call a Tauri command.

use std::{
    collections::BTreeMap, future::Future, path::PathBuf, pin::Pin, sync::Arc, time::Duration,
};

use hachimi_control_plane::{
    AppServerContext, AppServerDomainError, AppServerDomainHandler, AppServerDomainRequest,
    AppServerDomainResponse, DomainFuture, FsAppRequest, FsAppResponse, McpAppRequest,
    McpAppResponse, ReviewAppRequest, ReviewAppResponse, ScheduleAppRequest, ScheduleAppResponse,
    SkillsAppRequest, SkillsAppResponse, TaskAppRequest, TaskAppResponse,
};
use hachimi_core::FeatureFlags;
use hachimi_protocol::{
    CheckoutId, CheckoutRecord, ClientContext, DiffScope, McpServerHealthState, McpServerTransport,
    MutationContext, ReviewStartRequest, ReviewStartSnapshot, RunDiffSnapshot, RunRecord,
    ScheduleContextTemplate, ScheduleDefinition, ScheduleSkillSelection, SessionId,
    SkillDiagnosticSeverity, TaskInteractiveContinuation, TaskRunId,
};
use hachimi_storage::{AgentStore, AgentStoreError, IdempotentMutationClaim};
use parking_lot::Mutex;
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

use crate::workbench_commands::workspace_worker_path;
use crate::workspace_commands::{ActiveWorkspaceSearch, ActiveWorkspaceWatch};
use crate::{McpControlService, sandbox_commands::SandboxActivityTracker};
use hachimi_process::ProcessRegistry;
use hachimi_sandbox::SandboxRuntimeManager;
use hachimi_workspace::{WorkspaceHostClient, WorkspaceOperation, WorkspaceOutput};

const FILE_OPERATION_TIMEOUT: Duration = Duration::from_secs(20);
const DIFF_OPERATION_TIMEOUT: Duration = Duration::from_secs(60);

mod local_hosts;
mod plugin_products;
mod process;

struct DomainWorkspace {
    checkout: CheckoutRecord,
    run: RunRecord,
}

pub(super) type DesktopDomainLaunchFuture<T> =
    Pin<Box<dyn Future<Output = Result<T, AppServerDomainError>> + Send + 'static>>;

pub(super) trait DesktopDomainRunLauncher: Send + Sync {
    fn start_review(
        &self,
        client: ClientContext,
        request: ReviewStartRequest,
    ) -> DesktopDomainLaunchFuture<ReviewStartSnapshot>;

    fn continue_task(
        &self,
        client: ClientContext,
        task_run_id: TaskRunId,
        idempotency_key: String,
    ) -> DesktopDomainLaunchFuture<TaskInteractiveContinuation>;

    fn dispatch_channel_ingress(
        &self,
        principal: String,
        message: hachimi_protocol::VerifiedChannelMessage,
    ) -> DesktopDomainLaunchFuture<hachimi_protocol::IngressReceipt>;
}

#[derive(Clone)]
pub(super) struct DesktopAppDomainHandler {
    store: AgentStore,
    mcp: McpControlService,
    skills: hachimi_skills::SkillHost,
    scheduler: Arc<hachimi_scheduler::SchedulerService>,
    processes: Arc<ProcessRegistry>,
    sandbox_runtime: Arc<SandboxRuntimeManager>,
    run_launcher: Arc<dyn DesktopDomainRunLauncher>,
    workspace_watches: Arc<Mutex<BTreeMap<hachimi_protocol::FsWatchId, ActiveWorkspaceWatch>>>,
    workspace_searches: Arc<Mutex<BTreeMap<hachimi_protocol::FsSearchId, ActiveWorkspaceSearch>>>,
    features: FeatureFlags,
    sandbox_activity: SandboxActivityTracker,
    browser: Arc<hachimi_browser::BrowserHost>,
    computer: Arc<hachimi_computer::ComputerHost>,
    plugins: hachimi_extensions::PluginHost,
    plugin_surfaces: crate::plugin_content_protocol::PluginSurfaceRegistry,
    gateway: hachimi_gateway::GatewayHost,
    loopback_channel: hachimi_gateway::LoopbackWebhookChannel,
    mock_poll_channel: hachimi_gateway::MockPollChannel,
}

pub(super) struct DesktopAppDomainDependencies {
    pub store: AgentStore,
    pub mcp: McpControlService,
    pub skills: hachimi_skills::SkillHost,
    pub scheduler: Arc<hachimi_scheduler::SchedulerService>,
    pub processes: Arc<ProcessRegistry>,
    pub sandbox_runtime: Arc<SandboxRuntimeManager>,
    pub run_launcher: Arc<dyn DesktopDomainRunLauncher>,
    pub workspace_watches: Arc<Mutex<BTreeMap<hachimi_protocol::FsWatchId, ActiveWorkspaceWatch>>>,
    pub workspace_searches:
        Arc<Mutex<BTreeMap<hachimi_protocol::FsSearchId, ActiveWorkspaceSearch>>>,
    pub browser: Arc<hachimi_browser::BrowserHost>,
    pub computer: Arc<hachimi_computer::ComputerHost>,
    pub plugins: hachimi_extensions::PluginHost,
    pub plugin_surfaces: crate::plugin_content_protocol::PluginSurfaceRegistry,
    pub gateway: hachimi_gateway::GatewayHost,
    pub loopback_channel: hachimi_gateway::LoopbackWebhookChannel,
    pub mock_poll_channel: hachimi_gateway::MockPollChannel,
}

impl std::fmt::Debug for DesktopAppDomainHandler {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DesktopAppDomainHandler")
            .field("features", &self.features)
            .finish_non_exhaustive()
    }
}

impl DesktopAppDomainHandler {
    pub(super) fn new(
        dependencies: DesktopAppDomainDependencies,
        features: FeatureFlags,
        sandbox_activity: SandboxActivityTracker,
    ) -> Self {
        let DesktopAppDomainDependencies {
            store,
            mcp,
            skills,
            scheduler,
            processes,
            sandbox_runtime,
            run_launcher,
            workspace_watches,
            workspace_searches,
            browser,
            computer,
            plugins,
            plugin_surfaces,
            gateway,
            loopback_channel,
            mock_poll_channel,
        } = dependencies;
        Self {
            store,
            mcp,
            skills,
            scheduler,
            processes,
            sandbox_runtime,
            run_launcher,
            workspace_watches,
            workspace_searches,
            features,
            sandbox_activity,
            browser,
            computer,
            plugins,
            plugin_surfaces,
            gateway,
            loopback_channel,
            mock_poll_channel,
        }
    }

    fn require_connectors(&self) -> Result<(), AppServerDomainError> {
        if self.features.mcp_runtime {
            Ok(())
        } else {
            Err(AppServerDomainError::new(
                "mcp_runtime_disabled",
                "MCP execution is disabled by the emergency kill switch",
            ))
        }
    }

    fn enter_sandbox_activity(
        &self,
    ) -> Result<crate::sandbox_commands::SandboxActivityGuard, AppServerDomainError> {
        self.sandbox_activity.try_enter().ok_or_else(|| {
            AppServerDomainError::new(
                "sandbox_repair_in_progress",
                "Windows sandbox repair is in progress; retry after attestation completes",
            )
        })
    }

    async fn dispatch_mcp(
        &self,
        request: McpAppRequest,
    ) -> Result<McpAppResponse, AppServerDomainError> {
        let response = match request {
            McpAppRequest::Inventory(server_id) => McpAppResponse::Inventory(
                self.mcp
                    .inventory(&server_id)
                    .await
                    .map_err(domain_error("mcp_inventory_failed"))?,
            ),
            McpAppRequest::RefreshInventory(server_id) => {
                self.require_connectors()?;
                let _activity = self.enter_sandbox_activity()?;
                McpAppResponse::Inventory(
                    self.mcp
                        .refresh_inventory(&server_id)
                        .await
                        .map_err(domain_error("mcp_inventory_refresh_failed"))?,
                )
            }
            McpAppRequest::ReadResource(request) => {
                self.require_connectors()?;
                let _activity = self.enter_sandbox_activity()?;
                McpAppResponse::Resource(
                    self.mcp
                        .read_resource(&request)
                        .await
                        .map_err(domain_error("mcp_resource_read_failed"))?,
                )
            }
            McpAppRequest::GetPrompt(request) => {
                self.require_connectors()?;
                let _activity = self.enter_sandbox_activity()?;
                McpAppResponse::Prompt(
                    self.mcp
                        .get_prompt(request)
                        .await
                        .map_err(domain_error("mcp_prompt_get_failed"))?,
                )
            }
            McpAppRequest::ListCalls(request) => McpAppResponse::Calls(
                self.mcp
                    .list_call_summaries(&request)
                    .await
                    .map_err(domain_error("mcp_call_summaries_failed"))?,
            ),
            McpAppRequest::AuthStatus(server_id) => McpAppResponse::Auth(
                self.mcp
                    .auth_status(&server_id)
                    .await
                    .map_err(domain_error("mcp_auth_status_failed"))?,
            ),
            McpAppRequest::StartOauth(request) => {
                self.require_connectors()?;
                let server_id = request.server_id.clone();
                let (response, handle) = self
                    .mcp
                    .start_oauth_login(&request)
                    .await
                    .map_err(domain_error("mcp_oauth_start_failed"))?;
                let service = self.mcp.clone();
                tokio::spawn(async move {
                    if let Err(error) = service.finish_oauth_login(&server_id, handle).await {
                        tracing::warn!(%error, "MCP OAuth login did not complete");
                    }
                });
                McpAppResponse::Oauth(response)
            }
            McpAppRequest::Logout(server_id) => McpAppResponse::Auth(
                self.mcp
                    .logout_oauth(&server_id)
                    .await
                    .map_err(domain_error("mcp_oauth_logout_failed"))?,
            ),
        };
        Ok(response)
    }

    async fn dispatch_skills(
        &self,
        request: SkillsAppRequest,
    ) -> Result<SkillsAppResponse, AppServerDomainError> {
        let response = match request {
            SkillsAppRequest::List(project_id) => {
                let context = if let Some(project_id) = project_id {
                    let project = self
                        .store
                        .get_project(&project_id)
                        .await
                        .map_err(domain_error("skills_project_lookup_failed"))?
                        .ok_or_else(|| {
                            AppServerDomainError::new(
                                "skills_project_not_found",
                                "project does not exist",
                            )
                        })?;
                    hachimi_skills::SkillCatalogContext {
                        project_root: Some(PathBuf::from(project.root_path)),
                        checkout_root: None,
                    }
                } else {
                    hachimi_skills::SkillCatalogContext::default()
                };
                SkillsAppResponse::Skills(
                    self.skills
                        .list_for_context(&context)
                        .await
                        .map_err(domain_error("skills_list_failed"))?,
                )
            }
            SkillsAppRequest::Create { name } => SkillsAppResponse::Skill(
                self.skills
                    .create(&name)
                    .await
                    .map_err(domain_error("skill_create_failed"))?,
            ),
            SkillsAppRequest::Rename { skill_id, name } => SkillsAppResponse::Skill(
                self.skills
                    .rename(&skill_id, &name)
                    .await
                    .map_err(domain_error("skill_rename_failed"))?,
            ),
            SkillsAppRequest::Remove(skill_id) => SkillsAppResponse::Removed(
                self.skills
                    .remove(&skill_id)
                    .await
                    .map_err(domain_error("skill_remove_failed"))?,
            ),
            SkillsAppRequest::SetEnabled { skill_id, enabled } => SkillsAppResponse::Skill(
                self.skills
                    .set_enabled(&skill_id, enabled)
                    .await
                    .map_err(domain_error("skill_enable_failed"))?,
            ),
            SkillsAppRequest::Tree(skill_id) => SkillsAppResponse::Tree(
                self.skills
                    .tree(&skill_id)
                    .await
                    .map_err(domain_error("skill_tree_failed"))?,
            ),
            SkillsAppRequest::ReadFile {
                skill_id,
                relative_path,
            } => SkillsAppResponse::File(
                self.skills
                    .read_file(&skill_id, &relative_path)
                    .await
                    .map_err(domain_error("skill_read_failed"))?,
            ),
            SkillsAppRequest::ReadPreviewResource(request) => SkillsAppResponse::PreviewResource(
                self.skills
                    .read_preview_resource(&request)
                    .await
                    .map_err(domain_error("skill_preview_resource_failed"))?,
            ),
            SkillsAppRequest::WriteFile(request) => SkillsAppResponse::File(
                self.skills
                    .write_file(&request)
                    .await
                    .map_err(domain_error("skill_write_failed"))?,
            ),
            SkillsAppRequest::CreateEntry(request) => SkillsAppResponse::Tree(
                self.skills
                    .create_entry(&request)
                    .await
                    .map_err(domain_error("skill_entry_create_failed"))?,
            ),
            SkillsAppRequest::RenameEntry(request) => SkillsAppResponse::Tree(
                self.skills
                    .rename_entry(&request)
                    .await
                    .map_err(domain_error("skill_entry_rename_failed"))?,
            ),
            SkillsAppRequest::RemoveEntry {
                skill_id,
                relative_path,
            } => SkillsAppResponse::Tree(
                self.skills
                    .remove_entry(&skill_id, &relative_path)
                    .await
                    .map_err(domain_error("skill_entry_remove_failed"))?,
            ),
            SkillsAppRequest::Validate(skill_id) => SkillsAppResponse::Skill(
                self.skills
                    .validate(&skill_id)
                    .await
                    .map_err(domain_error("skill_validate_failed"))?,
            ),
        };
        Ok(response)
    }

    fn require_scheduler(&self) -> Result<(), AppServerDomainError> {
        if self.features.scheduler {
            Ok(())
        } else {
            Err(AppServerDomainError::new(
                "scheduler_disabled",
                "the persistent task scheduler is disabled in this build",
            ))
        }
    }

    fn validate_mutation(
        context: &AppServerContext,
        mutation: &MutationContext,
    ) -> Result<(), AppServerDomainError> {
        if mutation.protocol_version != hachimi_protocol::CONTROL_PROTOCOL_VERSION {
            return Err(AppServerDomainError::new(
                "protocol_version_mismatch",
                "the request protocol version is not supported",
            ));
        }
        if mutation.client_id != context.client.client_id {
            return Err(AppServerDomainError::new(
                "client_precondition_failed",
                "the mutation client does not match the authenticated client",
            ));
        }
        if mutation.idempotency_key.trim().is_empty() || mutation.idempotency_key.len() > 128 {
            return Err(AppServerDomainError::new(
                "invalid_idempotency_key",
                "idempotency key must contain 1-128 bytes",
            ));
        }
        Ok(())
    }

    async fn idempotent<T, Operation, OperationFuture>(
        &self,
        context: &AppServerContext,
        mutation: &MutationContext,
        method: &'static str,
        resource_id: &str,
        operation: Operation,
    ) -> Result<T, AppServerDomainError>
    where
        T: Serialize + DeserializeOwned,
        Operation: FnOnce() -> OperationFuture,
        OperationFuture: Future<Output = Result<T, AppServerDomainError>>,
    {
        execute_idempotent(
            &self.store,
            context,
            mutation,
            method,
            resource_id,
            operation,
        )
        .await
    }

    async fn pin_schedule_runtime_revisions(
        &self,
        schedule: &mut ScheduleDefinition,
    ) -> Result<(), AppServerDomainError> {
        crate::host_revision_snapshots::validate_enterprise_attachment_scope(schedule)
            .map_err(|error| AppServerDomainError::new(error.code, error.message))?;
        crate::host_revision_snapshots::validate_connector_revision_selections(
            &self.plugins,
            &schedule.host_revision_snapshot.connectors,
        )
        .await
        .map_err(|error| AppServerDomainError::new(error.code, error.message))?;
        self.plugins
            .verify_contribution_revisions(&schedule.contribution_revisions)
            .await
            .map_err(domain_error("schedule_contribution_drift"))?;
        let skill_context = match &schedule.context_template {
            ScheduleContextTemplate::Workspace { workspace, .. } => {
                let project_root = match workspace {
                    hachimi_protocol::ScheduleWorkspaceSpec::Managed => self
                        .store
                        .workspace_for_owner(hachimi_storage::WorkspaceOwnerRef::Schedule(
                            &schedule.id,
                        ))
                        .await
                        .map_err(domain_error("schedule_workspace_lookup_failed"))?
                        .map(|workspace| PathBuf::from(workspace.root_path)),
                    hachimi_protocol::ScheduleWorkspaceSpec::SelectedDirectory { root_path } => {
                        Some(PathBuf::from(root_path))
                    }
                };
                hachimi_skills::SkillCatalogContext {
                    project_root,
                    checkout_root: None,
                }
            }
        };
        let skills = self
            .skills
            .list_for_context(&skill_context)
            .await
            .map_err(domain_error("schedule_skill_validation_failed"))?;
        let servers = self
            .mcp
            .list()
            .await
            .map_err(domain_error("schedule_mcp_validation_failed"))?;
        let mut skill_revisions = Vec::with_capacity(schedule.skill_allowlist.len());
        for skill_id in &schedule.skill_allowlist {
            let skill = skills
                .iter()
                .find(|skill| &skill.id == skill_id && skill.enabled)
                .ok_or_else(|| {
                    AppServerDomainError::new(
                        "schedule_skill_unavailable",
                        format!("Skill {skill_id} is disabled, missing, or invalid"),
                    )
                })?;
            if skill
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == SkillDiagnosticSeverity::Error)
            {
                return Err(AppServerDomainError::new(
                    "schedule_skill_invalid",
                    format!("Skill {skill_id} has blocking diagnostics"),
                ));
            }
            for dependency in &skill.dependencies {
                if dependency.kind.eq_ignore_ascii_case("mcp")
                    && !mcp_dependency_available(dependency, &servers)
                {
                    return Err(AppServerDomainError::new(
                        "schedule_skill_dependency_missing",
                        format!(
                            "Skill {} requires unavailable MCP dependency {}",
                            skill.qualified_name, dependency.value
                        ),
                    ));
                }
            }
            skill_revisions.push(ScheduleSkillSelection {
                skill_id: skill.id.clone(),
                content_hash: skill.content_hash.clone(),
                tree_revision: skill.tree_revision.clone(),
            });
        }
        if !schedule.mcp_tool_allowlist.is_empty() && !self.features.mcp_runtime {
            return Err(AppServerDomainError::new(
                "schedule_mcp_unavailable",
                "MCP connectors are disabled",
            ));
        }
        if !schedule.mcp_tool_allowlist.is_empty() {
            let runtimes = self
                .mcp
                .ready_runtimes()
                .await
                .map_err(domain_error("schedule_mcp_validation_failed"))?;
            for selection in &schedule.mcp_tool_allowlist {
                let valid = runtimes.iter().find(|runtime| {
                    runtime.configuration.id == selection.server_id
                        && runtime.tools.iter().any(|tool| {
                            tool.name == selection.tool_name
                                && schema_hash(&tool.input_schema) == selection.schema_hash
                                && hachimi_control_plane::mcp_host_identity_hash(
                                    &runtime.configuration,
                                ) == selection.host_identity_hash
                        })
                });
                let Some(runtime) = valid else {
                    return Err(AppServerDomainError::new(
                        "schedule_mcp_schema_changed",
                        format!(
                            "MCP tool {} on {} is unavailable or changed",
                            selection.tool_name, selection.server_id
                        ),
                    ));
                };
                let requires_write = !runtime
                    .configuration
                    .read_only_tools
                    .contains(&selection.tool_name);
                if !schedule.permission_policy.allows_mcp(
                    &selection.server_id,
                    &selection.tool_name,
                    &selection.schema_hash,
                    requires_write,
                ) {
                    return Err(AppServerDomainError::new(
                        "schedule_mcp_tool_not_authorized",
                        "MCP tools require an exact persisted rule",
                    ));
                }
            }
        }
        for selection in &schedule.host_revision_snapshot.connectors {
            for action in &selection.allowed_actions {
                if !schedule
                    .permission_policy
                    .allows_connector(&selection.account_id, action, true)
                {
                    return Err(AppServerDomainError::new(
                        "schedule_connector_action_not_authorized",
                        "Connector actions require an exact persisted writable rule",
                    ));
                }
            }
        }
        schedule.skill_revisions = skill_revisions;
        hachimi_scheduler::normalize_schedule_definition(schedule);
        Ok(())
    }

    async fn dispatch_schedule(
        &self,
        context: &AppServerContext,
        request: ScheduleAppRequest,
    ) -> Result<ScheduleAppResponse, AppServerDomainError> {
        self.require_scheduler()?;
        let response = match request {
            ScheduleAppRequest::Create(request) => {
                let mut definition = request.definition;
                hachimi_scheduler::normalize_schedule_definition(&mut definition);
                self.pin_schedule_runtime_revisions(&mut definition).await?;
                let fingerprint = mutation_fingerprint(definition.id.as_str(), &definition)?;
                ScheduleAppResponse::Created(
                    self.idempotent(
                        context,
                        &request.context,
                        "schedule.create.command",
                        &fingerprint,
                        || async {
                            self.scheduler
                                .create(
                                    &context.principal,
                                    &request.context.idempotency_key,
                                    definition,
                                )
                                .await
                                .map_err(domain_error("scheduler_failed"))
                        },
                    )
                    .await?,
                )
            }
            ScheduleAppRequest::Get(schedule_id) => ScheduleAppResponse::Snapshot(
                self.scheduler
                    .get(&schedule_id)
                    .await
                    .map_err(domain_error("scheduler_failed"))?,
            ),
            ScheduleAppRequest::List => ScheduleAppResponse::Schedules(
                self.scheduler
                    .list()
                    .await
                    .map_err(domain_error("scheduler_failed"))?,
            ),
            ScheduleAppRequest::Preview { schedule, count } => {
                ScheduleAppResponse::Preview(self.scheduler.preview(&schedule, count.clamp(1, 20)))
            }
            ScheduleAppRequest::Update(request) => {
                let mut definition = request.definition;
                hachimi_scheduler::normalize_schedule_definition(&mut definition);
                self.pin_schedule_runtime_revisions(&mut definition).await?;
                let fingerprint = mutation_fingerprint(
                    definition.id.as_str(),
                    &(&definition, request.expected_config_revision),
                )?;
                let revision = request.expected_config_revision;
                ScheduleAppResponse::Schedule(
                    self.idempotent(
                        context,
                        &request.context,
                        "schedule.update",
                        &fingerprint,
                        || async {
                            self.scheduler
                                .update(definition, revision)
                                .await
                                .map_err(domain_error("scheduler_failed"))
                        },
                    )
                    .await?,
                )
            }
            ScheduleAppRequest::SetEnabled {
                context: mutation,
                schedule_id,
                enabled,
                expected_config_revision,
            } => {
                let fingerprint = mutation_fingerprint(
                    schedule_id.as_str(),
                    &(enabled, expected_config_revision),
                )?;
                ScheduleAppResponse::Schedule(
                    self.idempotent(
                        context,
                        &mutation,
                        "schedule.set_enabled",
                        &fingerprint,
                        || async {
                            self.scheduler
                                .set_enabled(&schedule_id, enabled, expected_config_revision)
                                .await
                                .map_err(domain_error("scheduler_failed"))
                        },
                    )
                    .await?,
                )
            }
            ScheduleAppRequest::Remove {
                context: mutation,
                schedule_id,
            } => {
                let resource = schedule_id.as_str().to_owned();
                ScheduleAppResponse::Removed(
                    self.idempotent(context, &mutation, "schedule.remove", &resource, || async {
                        self.scheduler
                            .remove(&schedule_id)
                            .await
                            .map_err(domain_error("scheduler_failed"))
                    })
                    .await?,
                )
            }
            ScheduleAppRequest::RunNow {
                context: mutation,
                schedule_id,
            } => {
                let resource = schedule_id.as_str().to_owned();
                ScheduleAppResponse::Task(
                    self.idempotent(
                        context,
                        &mutation,
                        "schedule.run_now",
                        &resource,
                        || async {
                            self.scheduler
                                .run_now(&schedule_id)
                                .await
                                .map_err(domain_error("scheduler_failed"))
                        },
                    )
                    .await?,
                )
            }
            ScheduleAppRequest::IngestEvent(request) => {
                Self::validate_mutation(context, &request.context)?;
                let envelope = hachimi_protocol::ScheduleEventEnvelope {
                    event_id: request.event_id,
                    source: hachimi_protocol::ScheduleEventSource {
                        kind: request.source_kind,
                        principal: context.principal.clone(),
                        id: request.source_id,
                    },
                    event_type: request.event_type,
                    subject: request.subject,
                    labels: request.labels,
                    resource: request.resource,
                    occurred_at_ms: request.occurred_at_ms,
                };
                ScheduleAppResponse::EventReceipt(
                    self.scheduler
                        .ingest_event(envelope)
                        .await
                        .map_err(schedule_event_domain_error)?,
                )
            }
            ScheduleAppRequest::ListEvents { limit } => ScheduleAppResponse::EventReceipts(
                self.store
                    .list_schedule_event_receipts(limit)
                    .await
                    .map_err(domain_error("scheduler_store_failed"))?,
            ),
        };
        Ok(response)
    }

    async fn dispatch_task(
        &self,
        context: &AppServerContext,
        request: TaskAppRequest,
    ) -> Result<TaskAppResponse, AppServerDomainError> {
        self.require_scheduler()?;
        let response = match request {
            TaskAppRequest::Get(task_run_id) => TaskAppResponse::Task(
                self.store
                    .get_task_run(&task_run_id)
                    .await
                    .map_err(domain_error("scheduler_store_failed"))?,
            ),
            TaskAppRequest::List { schedule_id, limit } => TaskAppResponse::Tasks(
                self.store
                    .list_task_runs(schedule_id.as_ref(), limit)
                    .await
                    .map_err(domain_error("scheduler_store_failed"))?,
            ),
            TaskAppRequest::Cancel {
                context: mutation,
                task_run_id,
            } => {
                let resource = task_run_id.as_str().to_owned();
                TaskAppResponse::Updated(
                    self.idempotent(context, &mutation, "task.cancel", &resource, || async {
                        self.scheduler
                            .cancel_task(&task_run_id)
                            .await
                            .map_err(domain_error("scheduler_failed"))
                    })
                    .await?,
                )
            }
            TaskAppRequest::Retry {
                context: mutation,
                task_run_id,
            } => {
                let resource = task_run_id.as_str().to_owned();
                TaskAppResponse::Updated(
                    self.idempotent(context, &mutation, "task.retry", &resource, || async {
                        self.scheduler
                            .retry(&task_run_id)
                            .await
                            .map_err(domain_error("scheduler_failed"))
                    })
                    .await?,
                )
            }
            TaskAppRequest::ContinueInteractively {
                context: mutation,
                task_run_id,
            } => {
                let resource = task_run_id.as_str().to_owned();
                TaskAppResponse::Continuation(Box::new(
                    self.idempotent(
                        context,
                        &mutation,
                        "task.continue_interactively",
                        &resource,
                        || {
                            self.run_launcher.continue_task(
                                context.client.clone(),
                                task_run_id,
                                mutation.idempotency_key.clone(),
                            )
                        },
                    )
                    .await?,
                ))
            }
        };
        Ok(response)
    }

    async fn dispatch_review(
        &self,
        context: &AppServerContext,
        request: ReviewAppRequest,
    ) -> Result<ReviewAppResponse, AppServerDomainError> {
        let response = match request {
            ReviewAppRequest::Start(request) => {
                let fingerprint = mutation_fingerprint(request.session_id.as_str(), &request)?;
                let mutation = request.context.clone();
                ReviewAppResponse::Started(
                    self.idempotent(context, &mutation, "review.start", &fingerprint, || {
                        self.run_launcher
                            .start_review(context.client.clone(), request)
                    })
                    .await?,
                )
            }
            ReviewAppRequest::Get(review_id) => ReviewAppResponse::Review(
                self.store
                    .review_snapshot(&review_id)
                    .await
                    .map_err(domain_error("review_get_failed"))?,
            ),
            ReviewAppRequest::List(session_id) => {
                let records = self
                    .store
                    .list_reviews(&session_id)
                    .await
                    .map_err(domain_error("review_list_failed"))?;
                let mut snapshots = Vec::with_capacity(records.len());
                for record in records {
                    snapshots.push(
                        self.store
                            .review_snapshot(&record.id)
                            .await
                            .map_err(domain_error("review_get_failed"))?,
                    );
                }
                ReviewAppResponse::Reviews(snapshots)
            }
            ReviewAppRequest::UpdateFinding(request) => {
                let fingerprint = mutation_fingerprint(
                    request.review_id.as_str(),
                    &(&request.finding_id, request.status),
                )?;
                ReviewAppResponse::Finding(
                    self.idempotent(
                        context,
                        &request.context,
                        "review.update_finding",
                        &fingerprint,
                        || async {
                            self.store
                                .update_review_finding_status(
                                    &request.review_id,
                                    &request.finding_id,
                                    request.status,
                                )
                                .await
                                .map_err(domain_error("review_finding_update_failed"))
                        },
                    )
                    .await?,
                )
            }
        };
        Ok(response)
    }

    async fn resolve_session_workspace(
        &self,
        session_id: &SessionId,
        checkout_id: &CheckoutId,
    ) -> Result<DomainWorkspace, AppServerDomainError> {
        let checkout = self
            .resolve_session_checkout(session_id, checkout_id)
            .await?;
        let run = self
            .store
            .list_runs(session_id)
            .await
            .map_err(domain_error("workspace_runs_failed"))?
            .into_iter()
            .last()
            .ok_or_else(|| {
                AppServerDomainError::new("workspace_run_not_found", "session has no run")
            })?;
        Ok(DomainWorkspace { checkout, run })
    }

    async fn resolve_session_checkout(
        &self,
        session_id: &SessionId,
        checkout_id: &CheckoutId,
    ) -> Result<CheckoutRecord, AppServerDomainError> {
        let session = self
            .store
            .get_session(session_id)
            .await
            .map_err(domain_error("workspace_session_failed"))?
            .ok_or_else(|| {
                AppServerDomainError::new("workspace_session_not_found", "session does not exist")
            })?;
        if session.context.checkout_id() != Some(checkout_id) {
            return Err(AppServerDomainError::new(
                "workspace_checkout_mismatch",
                "checkout is not bound to this session",
            ));
        }
        self.store
            .get_checkout(checkout_id)
            .await
            .map_err(domain_error("workspace_checkout_failed"))?
            .ok_or_else(|| {
                AppServerDomainError::new("workspace_checkout_not_found", "checkout does not exist")
            })
    }

    async fn resolve_diff_workspace(
        &self,
        scope: &DiffScope,
    ) -> Result<DomainWorkspace, AppServerDomainError> {
        match scope {
            DiffScope::Run { run_id } => {
                let run = self
                    .store
                    .get_run(run_id)
                    .await
                    .map_err(domain_error("workspace_run_failed"))?
                    .ok_or_else(|| {
                        AppServerDomainError::new("workspace_run_not_found", "run does not exist")
                    })?;
                let session = self
                    .store
                    .get_session(&run.session_id)
                    .await
                    .map_err(domain_error("workspace_session_failed"))?
                    .ok_or_else(|| {
                        AppServerDomainError::new(
                            "workspace_session_not_found",
                            "session does not exist",
                        )
                    })?;
                let checkout_id = session.context.checkout_id().ok_or_else(|| {
                    AppServerDomainError::new(
                        "workspace_context_not_project",
                        "session is not bound to a project checkout",
                    )
                })?;
                let checkout = self
                    .store
                    .get_checkout(checkout_id)
                    .await
                    .map_err(domain_error("workspace_checkout_failed"))?
                    .ok_or_else(|| {
                        AppServerDomainError::new(
                            "workspace_checkout_not_found",
                            "checkout does not exist",
                        )
                    })?;
                Ok(DomainWorkspace { checkout, run })
            }
            DiffScope::Session {
                session_id,
                checkout_id,
            } => {
                let session = self
                    .store
                    .get_session(session_id)
                    .await
                    .map_err(domain_error("workspace_session_failed"))?
                    .ok_or_else(|| {
                        AppServerDomainError::new(
                            "workspace_session_not_found",
                            "session does not exist",
                        )
                    })?;
                if session.context.checkout_id() != Some(checkout_id) {
                    return Err(AppServerDomainError::new(
                        "workspace_session_checkout_mismatch",
                        "session is not bound to the requested checkout",
                    ));
                }
                let checkout = self
                    .store
                    .get_checkout(checkout_id)
                    .await
                    .map_err(domain_error("workspace_checkout_failed"))?
                    .ok_or_else(|| {
                        AppServerDomainError::new(
                            "workspace_checkout_not_found",
                            "checkout does not exist",
                        )
                    })?;
                let run = self
                    .store
                    .list_runs(session_id)
                    .await
                    .map_err(domain_error("workspace_runs_failed"))?
                    .into_iter()
                    .last()
                    .ok_or_else(|| {
                        AppServerDomainError::new("workspace_run_not_found", "session has no run")
                    })?;
                Ok(DomainWorkspace { checkout, run })
            }
            DiffScope::Checkout { checkout_id } | DiffScope::Branch { checkout_id, .. } => {
                let checkout = self
                    .store
                    .get_checkout(checkout_id)
                    .await
                    .map_err(domain_error("workspace_checkout_failed"))?
                    .ok_or_else(|| {
                        AppServerDomainError::new(
                            "workspace_checkout_not_found",
                            "checkout does not exist",
                        )
                    })?;
                let session = self
                    .store
                    .list_sessions(Some(&checkout.project_id))
                    .await
                    .map_err(domain_error("workspace_sessions_failed"))?
                    .into_iter()
                    .rev()
                    .find(|session| session.context.checkout_id() == Some(checkout_id))
                    .ok_or_else(|| {
                        AppServerDomainError::new(
                            "workspace_session_not_found",
                            "checkout has no session",
                        )
                    })?;
                let run = self
                    .store
                    .list_runs(&session.id)
                    .await
                    .map_err(domain_error("workspace_runs_failed"))?
                    .into_iter()
                    .last()
                    .ok_or_else(|| {
                        AppServerDomainError::new("workspace_run_not_found", "session has no run")
                    })?;
                Ok(DomainWorkspace { checkout, run })
            }
        }
    }

    fn workspace_client(workspace: &DomainWorkspace) -> WorkspaceHostClient {
        WorkspaceHostClient::new(
            workspace_worker_path(),
            &workspace.checkout.path,
            workspace.checkout.id.as_str(),
            workspace.run.generation,
        )
    }

    async fn diff_base_revision(
        &self,
        scope: &DiffScope,
        workspace: &DomainWorkspace,
    ) -> Result<Option<String>, AppServerDomainError> {
        match scope {
            DiffScope::Branch { branch, .. } => Ok(Some(format!("{branch}...HEAD"))),
            DiffScope::Session { session_id, .. } => Ok(self
                .store
                .get_session_environment_state(session_id)
                .await
                .map_err(domain_error("workspace_environment_failed"))?
                .and_then(|state| state.baseline_revision)),
            DiffScope::Run { .. } | DiffScope::Checkout { .. } => {
                Ok(workspace.checkout.base_revision.clone())
            }
        }
    }

    async fn dispatch_fs(
        &self,
        request: FsAppRequest,
    ) -> Result<FsAppResponse, AppServerDomainError> {
        let response = match request {
            FsAppRequest::List(request) => {
                let workspace = self
                    .resolve_session_workspace(&request.session_id, &request.checkout_id)
                    .await?;
                match Self::workspace_client(&workspace)
                    .execute(
                        WorkspaceOperation::ListDirectoryPage {
                            path: request.path,
                            cursor: request.cursor,
                            limit: request.limit,
                        },
                        FILE_OPERATION_TIMEOUT,
                        tokio_util::sync::CancellationToken::new(),
                    )
                    .await
                    .map_err(workspace_domain_error)?
                {
                    WorkspaceOutput::DirectoryPage { page } => FsAppResponse::List(page),
                    _ => return Err(workspace_protocol_error("directory page")),
                }
            }
            FsAppRequest::ReadChunk(request) => {
                let workspace = self
                    .resolve_session_workspace(&request.session_id, &request.checkout_id)
                    .await?;
                match Self::workspace_client(&workspace)
                    .execute(
                        WorkspaceOperation::ReadFileChunk {
                            path: request.path,
                            offset: request.offset,
                            limit: request.limit,
                            if_match: request.if_match,
                        },
                        FILE_OPERATION_TIMEOUT,
                        tokio_util::sync::CancellationToken::new(),
                    )
                    .await
                    .map_err(workspace_domain_error)?
                {
                    WorkspaceOutput::FileChunk { chunk } => FsAppResponse::FileChunk(chunk),
                    _ => return Err(workspace_protocol_error("file chunk")),
                }
            }
            FsAppRequest::DiffGet(scope) => {
                let workspace = self.resolve_diff_workspace(&scope).await?;
                if let DiffScope::Run { run_id } = &scope {
                    let snapshot = self
                        .store
                        .get_run_diff_manifest(run_id)
                        .await
                        .map_err(domain_error("workspace_diff_store_failed"))?
                        .unwrap_or_else(|| RunDiffSnapshot {
                            scope: scope.clone(),
                            files: Vec::new(),
                            artifact_id: None,
                            truncated: false,
                            generated_at_ms: now_ms(),
                        });
                    FsAppResponse::Diff(snapshot)
                } else {
                    let base_revision = self.diff_base_revision(&scope, &workspace).await?;
                    match Self::workspace_client(&workspace)
                        .execute(
                            WorkspaceOperation::GitDiffStructured {
                                scope,
                                base_revision,
                            },
                            DIFF_OPERATION_TIMEOUT,
                            tokio_util::sync::CancellationToken::new(),
                        )
                        .await
                        .map_err(workspace_domain_error)?
                    {
                        WorkspaceOutput::Diff { snapshot } => FsAppResponse::Diff(snapshot),
                        _ => return Err(workspace_protocol_error("structured diff")),
                    }
                }
            }
            FsAppRequest::DiffReadFile(request) => {
                let workspace = self.resolve_diff_workspace(&request.scope).await?;
                match &request.scope {
                    DiffScope::Run { run_id } => {
                        let snapshot = self
                            .store
                            .get_run_diff_manifest(run_id)
                            .await
                            .map_err(domain_error("workspace_diff_store_failed"))?
                            .ok_or_else(|| {
                                AppServerDomainError::new(
                                    "workspace_diff_not_found",
                                    "Run Diff does not exist",
                                )
                            })?;
                        if !snapshot
                            .files
                            .iter()
                            .any(|file| file.path == request.path && file.too_large && !file.binary)
                        {
                            return Err(AppServerDomainError::new(
                                "workspace_diff_file_not_materialized",
                                "this file Diff is already inline or cannot be materialized as text",
                            ));
                        }
                        let artifact_id = snapshot.artifact_id.ok_or_else(|| {
                            AppServerDomainError::new(
                                "workspace_diff_artifact_not_found",
                                "Run Diff artifact does not exist",
                            )
                        })?;
                        FsAppResponse::DiffFile(
                            self.store
                                .read_managed_run_diff_file_chunk(
                                    run_id,
                                    &artifact_id,
                                    &request.path,
                                    request.offset,
                                    request.limit,
                                    request.if_match.as_deref(),
                                )
                                .await
                                .map_err(domain_error("workspace_diff_artifact_read_failed"))?,
                        )
                    }
                    DiffScope::Checkout { .. }
                    | DiffScope::Session { .. }
                    | DiffScope::Branch { .. } => {
                        let base_revision =
                            self.diff_base_revision(&request.scope, &workspace).await?;
                        match Self::workspace_client(&workspace)
                            .execute(
                                WorkspaceOperation::GitDiffFileChunk {
                                    scope: request.scope,
                                    path: request.path,
                                    base_revision,
                                    offset: request.offset,
                                    limit: request.limit,
                                    if_match: request.if_match,
                                },
                                DIFF_OPERATION_TIMEOUT,
                                tokio_util::sync::CancellationToken::new(),
                            )
                            .await
                            .map_err(workspace_domain_error)?
                        {
                            WorkspaceOutput::DiffFileChunk { chunk } => {
                                FsAppResponse::DiffFile(chunk)
                            }
                            _ => return Err(workspace_protocol_error("file Diff chunk")),
                        }
                    }
                }
            }
            FsAppRequest::Watch(request) => {
                let workspace = self
                    .resolve_session_workspace(&request.session_id, &request.checkout_id)
                    .await?;
                let cancellation = tokio_util::sync::CancellationToken::new();
                let watch = Self::workspace_client(&workspace)
                    .start_watch(
                        request.session_id.clone(),
                        request.path,
                        request.recursive,
                        1,
                        cancellation.clone(),
                    )
                    .await
                    .map_err(workspace_domain_error)?;
                let registration = watch.registration.clone();
                self.workspace_watches.lock().insert(
                    registration.id.clone(),
                    ActiveWorkspaceWatch {
                        session_id: request.session_id,
                        checkout_id: workspace.checkout.id,
                        generation: registration.generation,
                        cancellation,
                        watch: Arc::new(tokio::sync::Mutex::new(Some(watch))),
                    },
                );
                FsAppResponse::Watch(registration)
            }
            FsAppRequest::Unwatch(watch_id) => {
                let removed = self.workspace_watches.lock().remove(&watch_id);
                if let Some(active) = &removed {
                    active.cancellation.cancel();
                }
                FsAppResponse::Unwatched(removed.is_some())
            }
            FsAppRequest::SearchStart(request) => {
                let workspace = self
                    .resolve_session_workspace(&request.session_id, &request.checkout_id)
                    .await?;
                let search_id = hachimi_protocol::FsSearchId::random();
                let search = Self::workspace_client(&workspace)
                    .start_file_search(
                        search_id.clone(),
                        request.query.clone(),
                        request.max_results,
                        1,
                        tokio_util::sync::CancellationToken::new(),
                    )
                    .await
                    .map_err(workspace_domain_error)?;
                let initial = search
                    .wait_for_snapshot(1, &request.query, FILE_OPERATION_TIMEOUT)
                    .await
                    .map_err(workspace_domain_error)?;
                self.workspace_searches.lock().insert(
                    search_id,
                    ActiveWorkspaceSearch {
                        session_id: request.session_id,
                        checkout_id: workspace.checkout.id,
                        generation: 1,
                        session: search,
                    },
                );
                FsAppResponse::Search(initial)
            }
            FsAppRequest::SearchUpdate(request) => {
                let (session_id, checkout_id, generation, search) = {
                    let mut searches = self.workspace_searches.lock();
                    let active = searches.get_mut(&request.search_id).ok_or_else(|| {
                        AppServerDomainError::new(
                            "workspace_search_not_found",
                            "file search does not exist",
                        )
                    })?;
                    if active.generation != request.expected_generation {
                        return Err(AppServerDomainError::new(
                            "workspace_search_stale_generation",
                            "file search generation changed",
                        ));
                    }
                    active.generation = active.generation.saturating_add(1);
                    (
                        active.session_id.clone(),
                        active.checkout_id.clone(),
                        active.generation,
                        active.session.clone(),
                    )
                };
                let _workspace = self
                    .resolve_session_workspace(&session_id, &checkout_id)
                    .await?;
                search
                    .update(generation, request.query.clone())
                    .await
                    .map_err(workspace_domain_error)?;
                FsAppResponse::Search(
                    search
                        .wait_for_snapshot(generation, &request.query, FILE_OPERATION_TIMEOUT)
                        .await
                        .map_err(workspace_domain_error)?,
                )
            }
            FsAppRequest::SearchCancel(search_id) => {
                let removed = self.workspace_searches.lock().remove(&search_id);
                if let Some(active) = &removed {
                    active.session.cancel();
                }
                FsAppResponse::SearchCancelled(removed.is_some())
            }
        };
        Ok(response)
    }
}

async fn execute_idempotent<T, Operation, OperationFuture>(
    store: &AgentStore,
    context: &AppServerContext,
    mutation: &MutationContext,
    method: &'static str,
    resource_id: &str,
    operation: Operation,
) -> Result<T, AppServerDomainError>
where
    T: Serialize + DeserializeOwned,
    Operation: FnOnce() -> OperationFuture,
    OperationFuture: Future<Output = Result<T, AppServerDomainError>>,
{
    DesktopAppDomainHandler::validate_mutation(context, mutation)?;
    let principal = &context.client.client_id.0;
    let claim = store
        .claim_idempotent_mutation::<T>(
            principal,
            method,
            &mutation.idempotency_key,
            resource_id,
            now_ms(),
        )
        .await
        .map_err(|error| match error {
            AgentStoreError::IdempotencyConflict => AppServerDomainError::new(
                "idempotency_conflict",
                "the idempotency key was already used for another resource",
            ),
            other => AppServerDomainError::new("mutation_store_failed", other.to_string()),
        })?;
    match claim {
        IdempotentMutationClaim::Completed(response) => return Ok(response),
        IdempotentMutationClaim::Indeterminate => {
            return Err(AppServerDomainError::new(
                "idempotency_indeterminate",
                "the original mutation stopped before its result was confirmed",
            ));
        }
        IdempotentMutationClaim::Claimed => {}
    }
    let response = match operation().await {
        Ok(response) => response,
        Err(error) => {
            let _ = store
                .abandon_idempotent_mutation(principal, method, &mutation.idempotency_key)
                .await;
            return Err(error);
        }
    };
    store
        .complete_idempotent_mutation(principal, method, &mutation.idempotency_key, &response)
        .await
        .map_err(domain_error("mutation_store_failed"))?;
    Ok(response)
}

impl AppServerDomainHandler for DesktopAppDomainHandler {
    fn dispatch<'a>(
        &'a self,
        context: &'a AppServerContext,
        request: AppServerDomainRequest,
    ) -> DomainFuture<'a> {
        Box::pin(async move {
            match request {
                AppServerDomainRequest::Mcp(request) => self
                    .dispatch_mcp(request)
                    .await
                    .map(AppServerDomainResponse::Mcp),
                AppServerDomainRequest::Skills(request) => self
                    .dispatch_skills(request)
                    .await
                    .map(AppServerDomainResponse::Skills),
                AppServerDomainRequest::Schedule(request) => self
                    .dispatch_schedule(context, *request)
                    .await
                    .map(Box::new)
                    .map(AppServerDomainResponse::Schedule),
                AppServerDomainRequest::Task(request) => self
                    .dispatch_task(context, request)
                    .await
                    .map(|response| AppServerDomainResponse::Task(Box::new(response))),
                AppServerDomainRequest::Review(request) => self
                    .dispatch_review(context, request)
                    .await
                    .map(Box::new)
                    .map(AppServerDomainResponse::Review),
                AppServerDomainRequest::Fs(request) => self
                    .dispatch_fs(request)
                    .await
                    .map(AppServerDomainResponse::Fs),
                AppServerDomainRequest::Process(request) => self
                    .dispatch_process(context, request)
                    .await
                    .map(AppServerDomainResponse::Process),
                AppServerDomainRequest::Browser(request) => self
                    .dispatch_browser(context, request)
                    .await
                    .map(AppServerDomainResponse::Browser),
                AppServerDomainRequest::Computer(request) => self
                    .dispatch_computer(context, request)
                    .await
                    .map(AppServerDomainResponse::Computer),
                AppServerDomainRequest::Plugin(request) => self
                    .dispatch_plugin(request)
                    .await
                    .map(AppServerDomainResponse::Plugin),
                AppServerDomainRequest::Connector(request) => self
                    .dispatch_connector(request)
                    .await
                    .map(AppServerDomainResponse::Connector),
                AppServerDomainRequest::Channel(request) => self
                    .dispatch_channel(context, request)
                    .await
                    .map(AppServerDomainResponse::Channel),
                AppServerDomainRequest::Gateway(request) => self
                    .dispatch_gateway(request)
                    .await
                    .map(AppServerDomainResponse::Gateway),
            }
        })
    }
}

fn domain_error<E: std::fmt::Display>(
    code: &'static str,
) -> impl FnOnce(E) -> AppServerDomainError {
    move |error| AppServerDomainError::new(code, error.to_string())
}

fn schedule_event_domain_error(error: hachimi_scheduler::SchedulerError) -> AppServerDomainError {
    let code = if matches!(
        error,
        hachimi_scheduler::SchedulerError::Store(AgentStoreError::ScheduleEventConflict)
    ) {
        "schedule_event_conflict"
    } else {
        "scheduler_failed"
    };
    AppServerDomainError::new(code, error.to_string())
}

fn workspace_domain_error(error: hachimi_workspace::WorkspaceError) -> AppServerDomainError {
    AppServerDomainError::new(
        format!("workspace_{:?}", error.code).to_lowercase(),
        error.message,
    )
}

fn workspace_protocol_error(expected: &str) -> AppServerDomainError {
    AppServerDomainError::new(
        "workspace_protocol_mismatch",
        format!("workspace worker did not return {expected}"),
    )
}

fn mutation_fingerprint<T: Serialize>(
    resource_id: &str,
    input: &T,
) -> Result<String, AppServerDomainError> {
    let bytes = serde_json::to_vec(input).map_err(domain_error("mutation_fingerprint_failed"))?;
    let hash = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("{resource_id}:{hash}"))
}

fn mcp_dependency_available(
    dependency: &hachimi_protocol::SkillToolDependency,
    servers: &[hachimi_protocol::McpServerView],
) -> bool {
    servers.iter().any(|server| {
        if !server.configuration.enabled || server.health.state != McpServerHealthState::Ready {
            return false;
        }
        if server.configuration.id.as_str() == dependency.value {
            return true;
        }
        match &server.configuration.transport {
            McpServerTransport::StreamableHttp { url } => dependency
                .url
                .as_ref()
                .is_some_and(|expected| expected == url),
            McpServerTransport::Stdio { command, .. } => dependency
                .command
                .as_ref()
                .is_some_and(|expected| expected == command),
        }
    })
}

fn schema_hash(value: &serde_json::Value) -> String {
    Sha256::digest(serde_json::to_vec(value).unwrap_or_default())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn plugin_skill_namespace(plugin_id: &str) -> String {
    let slug = plugin_id
        .bytes()
        .map(|byte| {
            if byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' {
                char::from(byte)
            } else {
                '-'
            }
        })
        .take(40)
        .collect::<String>()
        .trim_matches('-')
        .to_owned();
    let hash = Sha256::digest(plugin_id.as_bytes());
    let suffix = hash[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!(
        "plugin-{}-{suffix}",
        if slug.is_empty() { "local" } else { &slug }
    )
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use hachimi_core::WindowKind;
    use hachimi_protocol::{ClientId, RequestId};

    use super::*;

    fn mutation_context(idempotency_key: &str) -> (AppServerContext, MutationContext) {
        let client = ClientContext {
            client_id: ClientId("domain-test".into()),
            window_kind: WindowKind::Workbench,
            scopes: Default::default(),
        };
        let context = AppServerContext {
            principal: client.client_id.0.clone(),
            client: client.clone(),
        };
        let mutation = MutationContext {
            request_id: RequestId("request-1".into()),
            client_id: client.client_id,
            protocol_version: hachimi_protocol::CONTROL_PROTOCOL_VERSION,
            idempotency_key: idempotency_key.into(),
            expected_run_id: None,
            expected_generation: None,
        };
        (context, mutation)
    }

    #[tokio::test]
    async fn completed_domain_mutation_replays_snapshot_without_relaunching() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let (context, mutation) = mutation_context("review-start-once");
        let launches = AtomicUsize::new(0);

        let first = execute_idempotent(
            &store,
            &context,
            &mutation,
            "review.start",
            "session-1:fingerprint",
            || async {
                launches.fetch_add(1, Ordering::SeqCst);
                Ok::<_, AppServerDomainError>("review-snapshot".to_owned())
            },
        )
        .await
        .expect("first review start");
        let replay = execute_idempotent(
            &store,
            &context,
            &mutation,
            "review.start",
            "session-1:fingerprint",
            || async {
                launches.fetch_add(1, Ordering::SeqCst);
                Ok::<_, AppServerDomainError>("unexpected".to_owned())
            },
        )
        .await
        .expect("replayed review start");

        assert_eq!(first, "review-snapshot");
        assert_eq!(replay, first);
        assert_eq!(launches.load(Ordering::SeqCst), 1);
    }
}
