// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/core/src/session/{step_context,turn_context,world_state}.rs
// and codex-rs/core/src/tools/spec_plan.rs
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// Modified for Hachimi: provider-neutral model view, workload overlay, and capability fencing.

use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use parking_lot::RwLock;

use hachimi_protocol::{
    BehaviorMode, EntryProfile, McpToolSelection, ModelMessage, ProviderCapabilities, RunBudget,
    RunId, RunOrigin, SandboxCapabilityReport, SandboxReadiness, SessionContextBinding, SessionId,
    SkillActivation, ToolDescriptor, WorkloadResolution,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInstructionLayer {
    pub relative_directory: String,
    pub source_path: String,
    pub content_hash: String,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepWorldState {
    pub context_revision: u64,
    pub profile_revision: u64,
    pub agents_revision: String,
    pub skills_revision: String,
    pub mcp_revision: String,
    pub host_revision: String,
    pub instructions: Arc<[AgentInstructionLayer]>,
    pub skill_activations: Arc<[SkillActivation]>,
    pub mcp_bindings: Arc<[McpToolSelection]>,
    pub disabled_tool_names: Arc<[String]>,
    pub diagnostics: Arc<[String]>,
    pub sandbox: SandboxCapabilityReport,
    pub host_ready: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepRuntimeSnapshot {
    pub world: StepWorldState,
    pub workload: WorkloadResolution,
}

#[derive(Debug, Clone)]
pub struct StepRuntimeState {
    inner: Arc<RwLock<StepRuntimeSnapshot>>,
}

impl StepRuntimeState {
    #[must_use]
    pub fn new(world: StepWorldState, workload: WorkloadResolution) -> Self {
        Self {
            inner: Arc::new(RwLock::new(StepRuntimeSnapshot { world, workload })),
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> StepRuntimeSnapshot {
        self.inner.read().clone()
    }

    pub fn narrow_sandbox(&self, sandbox: SandboxCapabilityReport) {
        let mut state = self.inner.write();
        let narrowed = intersect_sandbox_reports(&state.world.sandbox, &sandbox);
        if state.world.sandbox != narrowed {
            state.world.sandbox = narrowed;
            state.world.context_revision = state.world.context_revision.saturating_add(1);
        }
    }

    pub fn apply_world_refresh(&self, mut world: StepWorldState) -> bool {
        let mut state = self.inner.write();
        if state.world.agents_revision == world.agents_revision
            && state.world.mcp_revision == world.mcp_revision
            && state.world.host_revision == world.host_revision
            && state.world.instructions == world.instructions
            && state.world.skill_activations == world.skill_activations
            && state.world.skills_revision == world.skills_revision
            && state.world.mcp_bindings == world.mcp_bindings
            && state.world.disabled_tool_names == world.disabled_tool_names
            && state.world.diagnostics == world.diagnostics
            && state.world.sandbox == world.sandbox
            && state.world.host_ready == world.host_ready
        {
            return false;
        }
        world.context_revision = state.world.context_revision.saturating_add(1);
        world.profile_revision = state.world.profile_revision;
        state.world = world;
        true
    }

    pub fn activate_skill(
        &self,
        activation: SkillActivation,
        source: hachimi_protocol::WorkloadResolutionSource,
        workload: hachimi_protocol::WorkloadKind,
        reason: String,
        classifier_revision: Option<String>,
    ) -> bool {
        let mut state = self.inner.write();
        if state.world.skill_activations.iter().any(|existing| {
            existing.skill_id == activation.skill_id
                && existing.content_revision == activation.content_revision
        }) {
            return false;
        }
        let mut activations = state.world.skill_activations.to_vec();
        activations.push(activation);
        activations.sort_by(|left, right| left.skill_id.cmp(&right.skill_id));
        state.world.skill_activations = activations.into();
        state.world.skills_revision = canonical_hash(&state.world.skill_activations);
        state.world.context_revision = state.world.context_revision.saturating_add(1);
        if state.workload.workload != workload {
            state.world.profile_revision = state.world.profile_revision.saturating_add(1);
        }
        state.workload = WorkloadResolution {
            workload,
            source,
            activated_skill_ids: state
                .world
                .skill_activations
                .iter()
                .map(|value| value.skill_id.clone())
                .collect(),
            reason,
            classifier_revision,
        };
        true
    }
}

fn intersect_sandbox_reports(
    initial: &SandboxCapabilityReport,
    current: &SandboxCapabilityReport,
) -> SandboxCapabilityReport {
    let os_enforced = initial.os_enforced && current.os_enforced;
    let filesystem_enforced = initial.filesystem_enforced && current.filesystem_enforced;
    let process_enforced = initial.process_enforced && current.process_enforced;
    let network_enforced = initial.network_enforced && current.network_enforced;
    let readiness = if os_enforced
        && filesystem_enforced
        && process_enforced
        && network_enforced
        && initial.readiness == SandboxReadiness::Ready
        && current.readiness == SandboxReadiness::Ready
    {
        SandboxReadiness::Ready
    } else if initial.readiness == SandboxReadiness::Unavailable
        || current.readiness == SandboxReadiness::Unavailable
    {
        SandboxReadiness::Unavailable
    } else if initial.readiness == SandboxReadiness::SetupRequired
        || current.readiness == SandboxReadiness::SetupRequired
    {
        SandboxReadiness::SetupRequired
    } else {
        SandboxReadiness::Degraded
    };
    let mut diagnostics = initial.diagnostics.clone();
    diagnostics.extend(current.diagnostics.iter().cloned());
    diagnostics.sort();
    diagnostics.dedup();
    SandboxCapabilityReport {
        backend: initial.backend.clone(),
        readiness,
        os_enforced,
        filesystem_enforced,
        process_enforced,
        network_enforced,
        version: initial.version.clone(),
        stable_error_code: current
            .stable_error_code
            .clone()
            .or_else(|| initial.stable_error_code.clone()),
        diagnostics,
    }
}

pub type StepWorldStateRefreshFuture = Pin<
    Box<dyn Future<Output = Result<StepWorldState, crate::ModelRuntimeError>> + Send + 'static>,
>;

pub trait StepWorldStateRefresher: Send + Sync {
    fn refresh(
        &self,
        current: StepRuntimeSnapshot,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> StepWorldStateRefreshFuture;
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolPlan {
    hash: String,
    descriptors: Arc<[ToolDescriptor]>,
}

impl ToolPlan {
    #[must_use]
    pub fn build(
        entry_profile: EntryProfile,
        workload: hachimi_protocol::WorkloadKind,
        mode: BehaviorMode,
        provider: ProviderCapabilities,
        mut descriptors: Vec<ToolDescriptor>,
        run_allowlist: Option<&[String]>,
        disabled_tool_names: &[String],
    ) -> Self {
        descriptors.retain(|descriptor| {
            crate::profile_allows_tool(entry_profile, workload, &descriptor.name)
                && (mode != BehaviorMode::Plan
                    || descriptor.effect == hachimi_protocol::ToolEffect::ReadOnly)
                && run_allowlist
                    .is_none_or(|allowlist| allowlist.iter().any(|name| name == &descriptor.name))
                && !disabled_tool_names
                    .iter()
                    .any(|name| name == &descriptor.name)
        });
        descriptors.sort_by(|left, right| left.name.cmp(&right.name));
        if !provider.tool_calls {
            descriptors.clear();
        }
        let hash = canonical_hash(&(
            entry_profile,
            workload,
            mode,
            provider.tool_calls,
            provider.strict_json_schema,
            &descriptors,
        ));
        Self {
            hash,
            descriptors: descriptors.into(),
        }
    }

    #[must_use]
    pub fn hash(&self) -> &str {
        &self.hash
    }

    #[must_use]
    pub fn descriptors(&self) -> &[ToolDescriptor] {
        &self.descriptors
    }

    #[must_use]
    pub fn allows(&self, name: &str) -> bool {
        self.descriptors
            .iter()
            .any(|descriptor| descriptor.name == name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepContext {
    pub session_id: SessionId,
    pub run_id: RunId,
    pub run_generation: u64,
    pub step_revision: u64,
    pub registry_revision: String,
    pub entry_profile: EntryProfile,
    pub workload: WorkloadResolution,
    pub behavior_mode: BehaviorMode,
    pub origin: RunOrigin,
    pub context: SessionContextBinding,
    pub world: StepWorldState,
    pub model_messages: Arc<[ModelMessage]>,
    pub budget: RunBudget,
    pub provider: ProviderCapabilities,
    pub tool_plan: ToolPlan,
}

impl StepContext {
    #[must_use]
    pub fn context_hash(&self) -> String {
        canonical_hash(&(
            (
                self.session_id.as_str(),
                self.run_id.as_str(),
                self.run_generation,
                self.step_revision,
            ),
            (
                &self.registry_revision,
                self.entry_profile,
                self.workload.workload,
                self.behavior_mode,
            ),
            (
                self.world.context_revision,
                self.world.profile_revision,
                &self.world.agents_revision,
                &self.world.skills_revision,
                &self.world.mcp_revision,
                &self.world.host_revision,
            ),
            (
                &self.world.disabled_tool_names,
                &self.world.diagnostics,
                self.tool_plan.hash(),
            ),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct StepContextInput {
    pub session_id: SessionId,
    pub run_id: RunId,
    pub run_generation: u64,
    pub entry_profile: EntryProfile,
    pub workload: WorkloadResolution,
    pub behavior_mode: BehaviorMode,
    pub origin: RunOrigin,
    pub context: SessionContextBinding,
    pub world: StepWorldState,
    pub model_messages: Vec<ModelMessage>,
    pub budget: RunBudget,
    pub provider: ProviderCapabilities,
    pub registered_tools: Vec<ToolDescriptor>,
    pub registry_revision: String,
    pub run_tool_allowlist: Option<Vec<String>>,
}

#[derive(Debug, Default)]
pub struct StepContextFactory {
    next_step_revision: AtomicU64,
}

impl StepContextFactory {
    #[must_use]
    pub fn capture(&self, input: StepContextInput) -> Arc<StepContext> {
        let step_revision = self
            .next_step_revision
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        let tool_plan = ToolPlan::build(
            input.entry_profile,
            input.workload.workload,
            input.behavior_mode,
            input.provider,
            input.registered_tools,
            input.run_tool_allowlist.as_deref(),
            &input.world.disabled_tool_names,
        );
        Arc::new(StepContext {
            session_id: input.session_id,
            run_id: input.run_id,
            run_generation: input.run_generation,
            step_revision,
            registry_revision: input.registry_revision,
            entry_profile: input.entry_profile,
            workload: input.workload,
            behavior_mode: input.behavior_mode,
            origin: input.origin,
            context: input.context,
            world: input.world,
            model_messages: input.model_messages.into(),
            budget: input.budget,
            provider: input.provider,
            tool_plan,
        })
    }
}

fn canonical_hash(value: &impl Serialize) -> String {
    let bytes = serde_json::to_vec(value).expect("StepContext hash inputs are serializable");
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hachimi_protocol::{
        ScheduleId, TaskRunId, ToolEffect, WorkloadKind, WorkloadResolutionSource,
    };

    fn resolution() -> WorkloadResolution {
        WorkloadResolution {
            workload: WorkloadKind::Coding,
            source: WorkloadResolutionSource::UserOverride,
            activated_skill_ids: Vec::new(),
            reason: "test".into(),
            classifier_revision: None,
        }
    }

    #[test]
    fn tool_plan_is_stable_and_plan_mode_removes_side_effects() {
        let descriptors = vec![
            ToolDescriptor {
                name: "workspace_write_file".into(),
                description: "write".into(),
                input_schema: serde_json::json!({"type":"object"}),
                effect: ToolEffect::WorkspaceWrite,
                parallel_safe: false,
                required_scopes: Vec::new(),
            },
            ToolDescriptor {
                name: "workspace_read_file".into(),
                description: "read".into(),
                input_schema: serde_json::json!({"type":"object"}),
                effect: ToolEffect::ReadOnly,
                parallel_safe: true,
                required_scopes: Vec::new(),
            },
        ];
        let first = ToolPlan::build(
            EntryProfile::Workbench,
            resolution().workload,
            BehaviorMode::Plan,
            ProviderCapabilities {
                tool_calls: true,
                ..ProviderCapabilities::default()
            },
            descriptors.clone(),
            None,
            &[],
        );
        let second = ToolPlan::build(
            EntryProfile::Workbench,
            resolution().workload,
            BehaviorMode::Plan,
            ProviderCapabilities {
                tool_calls: true,
                ..ProviderCapabilities::default()
            },
            descriptors,
            None,
            &[],
        );
        assert_eq!(first.hash(), second.hash());
        assert!(first.allows("workspace_read_file"));
        assert!(!first.allows("workspace_write_file"));
    }

    fn equivalent_input(origin: RunOrigin) -> StepContextInput {
        StepContextInput {
            session_id: SessionId::from("same-session"),
            run_id: RunId::from("same-run"),
            run_generation: 1,
            entry_profile: EntryProfile::Workbench,
            workload: resolution(),
            behavior_mode: BehaviorMode::Default,
            origin,
            context: SessionContextBinding::General,
            world: StepWorldState {
                context_revision: 1,
                profile_revision: 1,
                agents_revision: "agents".into(),
                skills_revision: "skills".into(),
                mcp_revision: "mcp".into(),
                host_revision: "host".into(),
                instructions: Arc::from([]),
                skill_activations: Arc::from([]),
                mcp_bindings: Arc::from([]),
                disabled_tool_names: Arc::from([]),
                diagnostics: Arc::from([]),
                sandbox: SandboxCapabilityReport {
                    backend: "test".into(),
                    readiness: hachimi_protocol::SandboxReadiness::Unavailable,
                    os_enforced: false,
                    filesystem_enforced: false,
                    process_enforced: false,
                    network_enforced: false,
                    version: None,
                    stable_error_code: None,
                    diagnostics: Vec::new(),
                },
                host_ready: true,
            },
            model_messages: vec![ModelMessage::user("same request")],
            budget: RunBudget::default(),
            provider: ProviderCapabilities {
                tool_calls: true,
                strict_json_schema: true,
                ..ProviderCapabilities::default()
            },
            registered_tools: vec![ToolDescriptor {
                name: "request_user_input".into(),
                description: "request input".into(),
                input_schema: serde_json::json!({"type":"object"}),
                effect: ToolEffect::ReadOnly,
                parallel_safe: false,
                required_scopes: Vec::new(),
            }],
            registry_revision: "registry-fixture".into(),
            run_tool_allowlist: Some(vec!["request_user_input".into()]),
        }
    }

    #[test]
    fn interactive_and_scheduled_origins_share_the_same_semantic_step_and_tool_plan() {
        let interactive =
            StepContextFactory::default().capture(equivalent_input(RunOrigin::Interactive));
        let scheduled =
            StepContextFactory::default().capture(equivalent_input(RunOrigin::Scheduled {
                schedule_id: ScheduleId::from("schedule"),
                task_run_id: TaskRunId::from("task"),
                scheduled_for_ms: 1_800_000_000_000,
                event_context: None,
            }));

        assert_eq!(interactive.context_hash(), scheduled.context_hash());
        assert_eq!(interactive.tool_plan.hash(), scheduled.tool_plan.hash());
        assert_eq!(
            interactive.tool_plan.descriptors(),
            scheduled.tool_plan.descriptors()
        );
        assert_ne!(interactive.origin, scheduled.origin);
    }

    #[test]
    fn sandbox_repair_cannot_widen_an_existing_run_snapshot() {
        let input = equivalent_input(RunOrigin::Interactive);
        let state = StepRuntimeState::new(input.world, input.workload);
        state.narrow_sandbox(SandboxCapabilityReport {
            backend: "windows-appcontainer".into(),
            readiness: SandboxReadiness::Ready,
            os_enforced: true,
            filesystem_enforced: true,
            process_enforced: true,
            network_enforced: true,
            version: Some("2".into()),
            stable_error_code: None,
            diagnostics: Vec::new(),
        });
        let snapshot = state.snapshot();
        assert_eq!(
            snapshot.world.sandbox.readiness,
            SandboxReadiness::Unavailable
        );
        assert!(!snapshot.world.sandbox.os_enforced);
        assert!(!snapshot.world.sandbox.filesystem_enforced);
        assert!(!snapshot.world.sandbox.process_enforced);
        assert!(!snapshot.world.sandbox.network_enforced);
    }

    #[test]
    fn mcp_schema_or_host_revision_change_advances_context_revision() {
        let input = equivalent_input(RunOrigin::Interactive);
        let state = StepRuntimeState::new(input.world.clone(), input.workload);
        let mut refreshed = input.world;
        refreshed.mcp_revision = "schema-and-host-v2".into();
        assert!(state.apply_world_refresh(refreshed));
        let snapshot = state.snapshot();
        assert_eq!(snapshot.world.context_revision, 2);
        assert_eq!(snapshot.world.mcp_revision, "schema-and-host-v2");
    }
}
