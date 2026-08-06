use hachimi_protocol::{
    AgentWorkspaceStatus, AuthorityMode, ItemId, ItemPayload, ItemRelations, ItemStatus,
    LlmSettings, PermissionProfile, PlanAcceptanceRequest, ProviderCapabilities, RunId, RunPurpose,
    RunRecord, RunStatus, SessionContextBinding, TranscriptItem, TranscriptItemKind,
    WorkbenchPlanAcceptanceSnapshot, WorkbenchTaskSnapshot,
};
use hachimi_storage::{AgentStoreError, AtomicRunLaunchInput};

use super::{WorkbenchError, WorkbenchService, now_ms};

impl WorkbenchService {
    pub async fn accept_plan(
        &self,
        request: &PlanAcceptanceRequest,
        model_snapshot: LlmSettings,
        principal: &str,
    ) -> Result<WorkbenchPlanAcceptanceSnapshot, WorkbenchError> {
        let plan = self
            .store
            .get_proposed_plan(&request.plan_id)
            .await?
            .ok_or_else(|| AgentStoreError::ProposedPlanNotFound(request.plan_id.clone()))?;
        if plan.revision != request.expected_revision {
            return Err(WorkbenchError::StalePlanRevision);
        }
        let user_message = request.user_message.trim();
        if user_message.is_empty() || user_message.chars().count() > 200 {
            return Err(WorkbenchError::InvalidPrompt);
        }
        let source_run = self
            .store
            .get_run(&plan.run_id)
            .await?
            .ok_or_else(|| AgentStoreError::RunNotFound(plan.run_id.clone()))?;
        if source_run.status != RunStatus::Succeeded
            || source_run.configuration.behavior_mode != hachimi_protocol::BehaviorMode::Plan
        {
            return Err(WorkbenchError::Store(
                AgentStoreError::ProposedPlanNotAcceptable(plan.id),
            ));
        }
        let session = self
            .store
            .get_session(&plan.session_id)
            .await?
            .ok_or_else(|| WorkbenchError::SessionNotFound(plan.session_id.clone()))?;
        let (project, checkout) = match &session.context {
            SessionContextBinding::Project {
                project_id,
                checkout_id,
            } => {
                let checkout = self
                    .store
                    .get_checkout(checkout_id)
                    .await?
                    .ok_or_else(|| WorkbenchError::CheckoutNotFound(checkout_id.clone()))?;
                (Some(self.project(project_id).await?), Some(checkout))
            }
            SessionContextBinding::Workspace { .. } => (None, None),
        };
        let now = now_ms();
        let requested_capabilities = source_run.requested_capabilities;
        let (workspace_root, workspace_owner) = match &session.context {
            SessionContextBinding::Workspace { workspace_id } => {
                let workspace = self
                    .store
                    .workspace(workspace_id)
                    .await?
                    .filter(|workspace| workspace.status == AgentWorkspaceStatus::Ready)
                    .ok_or_else(|| {
                        WorkbenchError::WorkspaceUnavailable(workspace_id.to_string())
                    })?;
                (workspace.root_path, Some(workspace.owner))
            }
            SessionContextBinding::Project { .. } => (
                checkout
                    .as_ref()
                    .map(|checkout| checkout.path.clone())
                    .ok_or(WorkbenchError::ProjectContextRequired)?,
                None,
            ),
        };
        let mut policy = self
            .store
            .permission_policy(&format!("session:{}", session.id))
            .await?
            .unwrap_or_default();
        if policy.level != PermissionProfile::Writable {
            policy.level = PermissionProfile::Writable;
            policy.revision = policy.revision.saturating_add(1);
        }
        let stored_policy_owner_key = format!("session:{}", session.id);
        let mut configuration = source_run.configuration;
        configuration.model_snapshot = model_snapshot;
        configuration.behavior_mode = hachimi_protocol::BehaviorMode::Default;
        configuration.permission_profile = PermissionProfile::Writable;
        configuration.accepted_plan_id = Some(plan.id.clone());
        configuration.accepted_plan_revision = Some(plan.revision);
        let candidate = RunRecord {
            id: RunId::random(),
            session_id: session.id.clone(),
            status: RunStatus::Queued,
            purpose: RunPurpose::Task,
            origin: source_run.origin.clone(),
            generation: 1,
            configuration,
            requested_capabilities,
            negotiated_capabilities: ProviderCapabilities::default(),
            provider_capability_probe: None,
            capability_degradations: Vec::new(),
            failure_code: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let (accepted_plan, run) = self
            .store
            .accept_proposed_plan_authorized_idempotent(
                principal,
                &request.idempotency_key,
                &plan.id,
                &candidate,
                AtomicRunLaunchInput {
                    proposed_workspace: None,
                    workspace_owner: workspace_owner.as_ref(),
                    stored_policy_owner_key: Some(&stored_policy_owner_key),
                    stored_policy: Some(&policy),
                    effective_policy: &policy,
                    authority_mode: AuthorityMode::Interactive,
                    workspace_root: &workspace_root,
                },
            )
            .await?;
        if run.id == candidate.id {
            self.store
                .append_transcript_item(TranscriptItem {
                    id: ItemId::random(),
                    session_id: session.id.clone(),
                    run_id: Some(run.id.clone()),
                    sequence: 0,
                    kind: TranscriptItemKind::User,
                    status: ItemStatus::Completed,
                    payload: ItemPayload::User {
                        text: user_message.to_owned(),
                        attachment_ids: Vec::new(),
                    },
                    relations: ItemRelations::default(),
                    created_at_ms: now,
                })
                .await?;
        }
        Ok(WorkbenchPlanAcceptanceSnapshot {
            plan: accepted_plan,
            task: WorkbenchTaskSnapshot {
                project,
                checkout,
                session,
                run,
            },
        })
    }
}
