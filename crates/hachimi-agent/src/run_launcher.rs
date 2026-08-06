use hachimi_protocol::{
    AgentPermissionPolicy, AgentWorkspaceStatus, ApprovalPolicy, AuthorityMode,
    AuthoritySnapshotId, BehaviorMode, CapabilityGrantSet, PermissionProfile, RunAuthoritySnapshot,
    RunRecord, SessionContextBinding, SessionRecord,
};
use hachimi_storage::{ChannelRunBindingInput, CreatedAgentRun};

use crate::{
    AgentRunCreateRequest, AgentRunFactoryError,
    run_runtime::{AgentRunAuthorization, AgentRunFactory, StoredPolicyOwner},
};

#[derive(Debug, Clone)]
pub struct AgentRunLaunchRequest {
    pub create: AgentRunCreateRequest,
    pub policy: AgentPermissionPolicy,
    pub authority_mode: AuthorityMode,
}

#[derive(Debug, Clone)]
pub struct LaunchedAgentRun {
    pub created: CreatedAgentRun,
    pub authority: RunAuthoritySnapshot,
    pub capability_grants: CapabilityGrantSet,
}

#[derive(Debug, Clone)]
pub struct AgentRunLauncher {
    store: hachimi_storage::AgentStore,
    factory: AgentRunFactory,
}

impl AgentRunLauncher {
    #[must_use]
    pub fn new(store: hachimi_storage::AgentStore) -> Self {
        Self {
            factory: AgentRunFactory::new(store.clone()),
            store,
        }
    }

    pub async fn launch_new(
        &self,
        request: AgentRunLaunchRequest,
    ) -> Result<LaunchedAgentRun, AgentRunFactoryError> {
        self.launch_new_with_owner(request, Some(StoredPolicyOwner::Session))
            .await
    }

    pub async fn launch_new_with_policy_owner(
        &self,
        request: AgentRunLaunchRequest,
        owner_key: impl Into<String>,
    ) -> Result<LaunchedAgentRun, AgentRunFactoryError> {
        self.launch_new_with_owner(request, Some(StoredPolicyOwner::Key(owner_key.into())))
            .await
    }

    pub async fn launch_new_transient_policy(
        &self,
        request: AgentRunLaunchRequest,
    ) -> Result<LaunchedAgentRun, AgentRunFactoryError> {
        self.launch_new_with_owner(request, None).await
    }

    async fn launch_new_with_owner(
        &self,
        request: AgentRunLaunchRequest,
        stored_policy_owner: Option<StoredPolicyOwner>,
    ) -> Result<LaunchedAgentRun, AgentRunFactoryError> {
        let (create, stored_policy, effective_policy, authority_mode) = normalize(request);
        let stored_policy = stored_policy_owner.as_ref().map(|_| stored_policy);
        let (created, authority) = self
            .factory
            .create_authorized(
                create,
                AgentRunAuthorization {
                    effective_policy,
                    stored_policy,
                    stored_policy_owner,
                    authority_mode,
                },
            )
            .await?;
        Ok(launched(created, authority))
    }

    pub async fn launch_channel(
        &self,
        request: AgentRunLaunchRequest,
        binding: ChannelRunBindingInput,
    ) -> Result<LaunchedAgentRun, AgentRunFactoryError> {
        let (create, stored_policy, effective_policy, authority_mode) = normalize(request);
        let stored_policy_owner =
            StoredPolicyOwner::Key(format!("channel_binding:{}", binding.binding_key_hash));
        let (created, authority) = self
            .factory
            .create_channel_authorized(
                create,
                binding,
                AgentRunAuthorization {
                    effective_policy,
                    stored_policy: Some(stored_policy),
                    stored_policy_owner: Some(stored_policy_owner),
                    authority_mode,
                },
            )
            .await?;
        Ok(launched(created, authority))
    }

    pub async fn launch_in_session(
        &self,
        request: AgentRunLaunchRequest,
        session: SessionRecord,
    ) -> Result<LaunchedAgentRun, AgentRunFactoryError> {
        self.launch_in_session_with_owner(request, session, Some(StoredPolicyOwner::Session))
            .await
    }

    pub async fn launch_in_session_with_policy_owner(
        &self,
        request: AgentRunLaunchRequest,
        session: SessionRecord,
        owner_key: impl Into<String>,
    ) -> Result<LaunchedAgentRun, AgentRunFactoryError> {
        self.launch_in_session_with_owner(
            request,
            session,
            Some(StoredPolicyOwner::Key(owner_key.into())),
        )
        .await
    }

    async fn launch_in_session_with_owner(
        &self,
        request: AgentRunLaunchRequest,
        session: SessionRecord,
        stored_policy_owner: Option<StoredPolicyOwner>,
    ) -> Result<LaunchedAgentRun, AgentRunFactoryError> {
        let (create, stored_policy, effective_policy, authority_mode) = normalize(request);
        let stored_policy = stored_policy_owner.as_ref().map(|_| stored_policy);
        let (created, authority) = self
            .factory
            .create_in_session_authorized(
                create,
                session,
                AgentRunAuthorization {
                    effective_policy,
                    stored_policy,
                    stored_policy_owner,
                    authority_mode,
                },
            )
            .await?;
        Ok(launched(created, authority))
    }

    pub async fn launch_in_session_transient_policy(
        &self,
        request: AgentRunLaunchRequest,
        session: SessionRecord,
    ) -> Result<LaunchedAgentRun, AgentRunFactoryError> {
        self.launch_in_session_with_owner(request, session, None)
            .await
    }

    pub async fn authorize_existing(
        &self,
        session: &SessionRecord,
        run: &RunRecord,
        policy: AgentPermissionPolicy,
        authority_mode: AuthorityMode,
    ) -> Result<(RunAuthoritySnapshot, CapabilityGrantSet), AgentRunFactoryError> {
        self.authorize(
            session,
            run,
            policy.clone(),
            authority_mode,
            Some(policy),
            Some(StoredPolicyOwner::Session),
        )
        .await
    }

    pub async fn authorize_existing_with_policy_owner(
        &self,
        session: &SessionRecord,
        run: &RunRecord,
        policy: AgentPermissionPolicy,
        authority_mode: AuthorityMode,
        owner_key: impl Into<String>,
    ) -> Result<(RunAuthoritySnapshot, CapabilityGrantSet), AgentRunFactoryError> {
        self.authorize(
            session,
            run,
            policy.clone(),
            authority_mode,
            Some(policy),
            Some(StoredPolicyOwner::Key(owner_key.into())),
        )
        .await
    }

    async fn authorize(
        &self,
        session: &SessionRecord,
        run: &RunRecord,
        policy: AgentPermissionPolicy,
        authority_mode: AuthorityMode,
        stored_policy: Option<AgentPermissionPolicy>,
        stored_policy_owner: Option<StoredPolicyOwner>,
    ) -> Result<(RunAuthoritySnapshot, CapabilityGrantSet), AgentRunFactoryError> {
        let workspace_root = match &session.context {
            SessionContextBinding::Workspace { workspace_id } => {
                self.store
                    .workspace(workspace_id)
                    .await?
                    .filter(|workspace| workspace.status == AgentWorkspaceStatus::Ready)
                    .ok_or_else(|| AgentRunFactoryError::UnexpectedExecutionTarget)?
                    .root_path
            }
            SessionContextBinding::Project { checkout_id, .. } => {
                self.store
                    .get_checkout(checkout_id)
                    .await?
                    .ok_or_else(|| {
                        AgentRunFactoryError::Store(
                            hachimi_storage::AgentStoreError::CheckoutNotFound(checkout_id.clone()),
                        )
                    })?
                    .path
            }
        };
        let grants = hachimi_policy::expand_permission_policy(
            &policy,
            authority_mode,
            run.configuration.behavior_mode,
            session.id.clone(),
            run.id.clone(),
            workspace_root.clone(),
        );
        let authority = RunAuthoritySnapshot {
            id: AuthoritySnapshotId::random(),
            session_id: session.id.clone(),
            run_id: run.id.clone(),
            policy: policy.clone(),
            mode: authority_mode,
            source: format!("{:?}", run.origin),
            workspace_root,
            created_at_ms: run.created_at_ms,
        };
        if let (Some(stored_policy), Some(owner)) = (stored_policy, stored_policy_owner) {
            self.store
                .store_permission_policy(&owner.key(&session.id), &stored_policy, run.created_at_ms)
                .await?;
        }
        self.store.persist_authority_snapshot(&authority).await?;
        Ok((authority, grants))
    }
}

fn launched(created: CreatedAgentRun, authority: RunAuthoritySnapshot) -> LaunchedAgentRun {
    let capability_grants = hachimi_policy::expand_permission_policy(
        &authority.policy,
        authority.mode,
        created.run.configuration.behavior_mode,
        created.session.id.clone(),
        created.run.id.clone(),
        authority.workspace_root.clone(),
    );
    LaunchedAgentRun {
        created,
        authority,
        capability_grants,
    }
}

fn normalize(
    mut request: AgentRunLaunchRequest,
) -> (
    AgentRunCreateRequest,
    AgentPermissionPolicy,
    AgentPermissionPolicy,
    AuthorityMode,
) {
    let stored_policy = request.policy.clone();
    if request.create.behavior_mode == BehaviorMode::Plan {
        request.policy.level = PermissionProfile::ReadOnly;
    }
    request.create.permission_profile = request.policy.level;
    request.create.approval_policy = match (request.policy.level, request.authority_mode) {
        (PermissionProfile::FullAccess, _) | (_, AuthorityMode::Unattended) => {
            ApprovalPolicy::NeverPrompt
        }
        (_, AuthorityMode::Interactive) => ApprovalPolicy::OnlyWhenNeeded,
    };
    (
        request.create,
        stored_policy,
        request.policy,
        request.authority_mode,
    )
}

#[cfg(test)]
mod tests {
    use hachimi_protocol::{
        AgentPermissionPolicy, AgentWorkspaceOwner, ApprovalPolicy, AuthorityMode, BehaviorMode,
        EntryProfile, LlmSettings, PermissionProfile, ProviderCapabilities, RunBudget, RunOrigin,
        RunPurpose, RunStatus, ScheduleId, ScopedPermissionRules, SessionContextBinding, TaskRunId,
        WorkspaceId,
    };

    use super::*;

    fn request(idempotency_key: &str, origin: RunOrigin, revision: u64) -> AgentRunLaunchRequest {
        AgentRunLaunchRequest {
            create: AgentRunCreateRequest {
                principal: "test".into(),
                idempotency_key: idempotency_key.into(),
                context: SessionContextBinding::Workspace {
                    workspace_id: WorkspaceId::random(),
                },
                origin,
                title: idempotency_key.into(),
                prompt: idempotency_key.into(),
                attachment_ids: Vec::new(),
                parent_session_id: None,
                source_run_id: None,
                purpose: RunPurpose::Task,
                model_snapshot: LlmSettings::default(),
                entry_profile: EntryProfile::Workbench,
                workload_override: None,
                behavior_mode: BehaviorMode::Default,
                execution_target: None,
                approval_policy: ApprovalPolicy::NeverPrompt,
                permission_profile: PermissionProfile::ReadOnly,
                budget: RunBudget::default(),
                requested_capabilities: ProviderCapabilities::default(),
                created_at_ms: i64::try_from(revision).unwrap_or(i64::MAX) + 1,
            },
            policy: AgentPermissionPolicy {
                level: PermissionProfile::ReadOnly,
                rules: ScopedPermissionRules::default(),
                revision,
            },
            authority_mode: AuthorityMode::Unattended,
        }
    }

    #[tokio::test]
    async fn persists_source_owner_policies_without_creating_session_copies() {
        let store = hachimi_storage::AgentStore::connect_in_memory()
            .await
            .expect("store");
        let launcher = AgentRunLauncher::new(store.clone());

        let manual = launcher
            .launch_new(request("manual-owner", RunOrigin::Manual, 1))
            .await
            .expect("manual launch");
        assert_eq!(
            store
                .permission_policy(&format!("session:{}", manual.created.session.id))
                .await
                .expect("manual policy")
                .expect("manual owner")
                .revision,
            1
        );

        let scheduled = launcher
            .launch_new_with_policy_owner(
                request(
                    "schedule-owner",
                    RunOrigin::Scheduled {
                        schedule_id: ScheduleId::from("schedule-1"),
                        task_run_id: TaskRunId::from("task-1"),
                        scheduled_for_ms: 1,
                        event_context: None,
                    },
                    2,
                ),
                "schedule:schedule-1",
            )
            .await
            .expect("scheduled launch");
        assert_eq!(
            store
                .permission_policy("schedule:schedule-1")
                .await
                .expect("schedule policy")
                .expect("schedule owner")
                .revision,
            2
        );
        assert!(
            store
                .permission_policy(&format!("session:{}", scheduled.created.session.id))
                .await
                .expect("schedule Session policy")
                .is_none()
        );

        let pet = launcher
            .launch_new_with_policy_owner(
                request("pet-owner", RunOrigin::Pet, 3),
                "profile:pet_conversation",
            )
            .await
            .expect("Pet launch");
        assert_eq!(
            store
                .permission_policy("profile:pet_conversation")
                .await
                .expect("Pet policy")
                .expect("Pet owner")
                .revision,
            3
        );
        assert!(
            store
                .permission_policy(&format!("session:{}", pet.created.session.id))
                .await
                .expect("Pet Session policy")
                .is_none()
        );
    }

    #[tokio::test]
    async fn transient_child_policy_does_not_overwrite_its_source_owner() {
        let store = hachimi_storage::AgentStore::connect_in_memory()
            .await
            .expect("store");
        let launcher = AgentRunLauncher::new(store.clone());
        launcher
            .launch_new_with_policy_owner(
                request("pet-parent", RunOrigin::Pet, 7),
                "profile:pet_conversation",
            )
            .await
            .expect("parent launch");
        let child = launcher
            .launch_new_transient_policy(request("pet-child", RunOrigin::Pet, 99))
            .await
            .expect("child launch");

        assert_eq!(
            store
                .permission_policy("profile:pet_conversation")
                .await
                .expect("Pet policy")
                .expect("Pet owner")
                .revision,
            7
        );
        assert!(
            store
                .permission_policy(&format!("session:{}", child.created.session.id))
                .await
                .expect("child Session policy")
                .is_none()
        );
        assert!(
            store
                .authority_snapshot(&child.created.run.id)
                .await
                .expect("child authority")
                .is_some()
        );
    }

    #[tokio::test]
    async fn scheduled_workspace_supports_per_run_and_shared_session_launches() {
        let store = hachimi_storage::AgentStore::connect_in_memory()
            .await
            .expect("store");
        let launcher = AgentRunLauncher::new(store.clone());
        let schedule_id = ScheduleId::from("schedule-workspace-owner");
        let workspace_id = WorkspaceId::random();
        let workspace = store
            .ensure_managed_workspace(
                workspace_id.clone(),
                hachimi_storage::WorkspaceOwnerRef::Schedule(&schedule_id),
                1,
            )
            .await
            .expect("Schedule Workspace");

        let scheduled_origin = RunOrigin::Scheduled {
            schedule_id: schedule_id.clone(),
            task_run_id: TaskRunId::from("task-per-run"),
            scheduled_for_ms: 2,
            event_context: None,
        };
        let mut per_run_request = request("schedule-per-run", scheduled_origin, 1);
        per_run_request.create.context = SessionContextBinding::Workspace {
            workspace_id: workspace_id.clone(),
        };
        let owner_key = format!("schedule:{schedule_id}");
        let per_run = launcher
            .launch_new_with_policy_owner(per_run_request, owner_key.clone())
            .await
            .expect("per-Run Schedule launch");
        assert_eq!(per_run.authority.workspace_root, workspace.root_path);

        store
            .transition_run(&per_run.created.run.id, RunStatus::Preparing, None)
            .await
            .expect("prepare scheduled Run");
        store
            .transition_run(&per_run.created.run.id, RunStatus::Running, None)
            .await
            .expect("start scheduled Run");
        store
            .transition_run(&per_run.created.run.id, RunStatus::Succeeded, None)
            .await
            .expect("finish scheduled Run");
        let mut shared_request = request("schedule-shared", RunOrigin::Manual, 2);
        shared_request.create.context = SessionContextBinding::Workspace {
            workspace_id: workspace_id.clone(),
        };
        let shared_session = store
            .get_session(&per_run.created.session.id)
            .await
            .expect("Session")
            .expect("persisted Session");
        let shared = launcher
            .launch_in_session_with_policy_owner(shared_request, shared_session, owner_key)
            .await
            .expect("shared Session Schedule continuation");
        assert_eq!(shared.created.session.id, per_run.created.session.id);
        assert_eq!(shared.authority.workspace_root, workspace.root_path);
        assert!(matches!(
            store
                .workspace(&workspace_id)
                .await
                .expect("Workspace")
                .expect("Workspace row")
                .owner,
            AgentWorkspaceOwner::Schedule { schedule_id: owner } if owner == schedule_id
        ));
    }

    #[tokio::test]
    async fn transient_child_inherits_only_a_verified_parent_workspace() {
        let store = hachimi_storage::AgentStore::connect_in_memory()
            .await
            .expect("store");
        let launcher = AgentRunLauncher::new(store.clone());
        let parent = launcher
            .launch_new(request("workspace-parent", RunOrigin::Manual, 1))
            .await
            .expect("parent launch");
        let mut child_request = request("workspace-child", RunOrigin::Manual, 2);
        child_request.create.context = parent.created.session.context.clone();
        child_request.create.parent_session_id = Some(parent.created.session.id.clone());
        child_request.create.source_run_id = Some(parent.created.run.id.clone());
        let child = launcher
            .launch_new_transient_policy(child_request)
            .await
            .expect("verified child launch");
        assert_eq!(
            child.authority.workspace_root,
            parent.authority.workspace_root
        );
        assert_eq!(
            child.created.session.parent_session_id,
            Some(parent.created.session.id.clone())
        );

        let mut invalid = request("workspace-child-invalid", RunOrigin::Manual, 3);
        invalid.create.context = parent.created.session.context.clone();
        invalid.create.parent_session_id = Some(parent.created.session.id);
        assert!(matches!(
            launcher.launch_new_transient_policy(invalid).await,
            Err(AgentRunFactoryError::UnexpectedExecutionTarget)
        ));
    }
}
