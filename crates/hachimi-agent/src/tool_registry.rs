// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/core/src/tools/{registry,router}.rs
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// Modified for Hachimi: transport-neutral DTOs, Profile/Plan filtering, and narrow executors.

use std::{collections::BTreeMap, sync::Arc};

use hachimi_protocol::{
    BehaviorMode, EntryProfile, ModelInputImage, ModelToolCall, ToolCallId, ToolDescriptor,
    ToolEffect, ToolRecoveryPolicy, WorkloadKind,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::ToolFuture;

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub name: String,
    pub arguments: Value,
    pub step_revision: u64,
    pub tool_plan_hash: String,
    pub registry_revision: String,
}

impl ToolCall {
    #[must_use]
    pub fn bind(
        call: ModelToolCall,
        step_revision: u64,
        tool_plan_hash: impl Into<String>,
        registry_revision: impl Into<String>,
    ) -> Self {
        Self {
            id: call.id,
            name: call.name,
            arguments: call.arguments,
            step_revision,
            tool_plan_hash: tool_plan_hash.into(),
            registry_revision: registry_revision.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ToolInvocation {
    pub call: ToolCall,
    pub entry_profile: EntryProfile,
    pub workload: WorkloadKind,
    pub behavior_mode: BehaviorMode,
    pub run_generation: u64,
    pub step_revision: u64,
    pub tool_plan_hash: String,
    pub registry_revision: String,
    pub cancellation: CancellationToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolResultStatus {
    Succeeded,
    Failed,
    Rejected,
    Aborted,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    pub call_id: ToolCallId,
    pub tool_name: String,
    pub status: ToolResultStatus,
    pub model_content: String,
    pub structured_content: Value,
    /// Ephemeral inputs for the next model step. Run projection deliberately never persists these.
    pub model_images: Vec<ModelInputImage>,
}

impl ToolResult {
    #[must_use]
    pub fn succeeded(call: &ToolCall, model_content: impl Into<String>, content: Value) -> Self {
        Self {
            call_id: call.id.clone(),
            tool_name: call.name.clone(),
            status: ToolResultStatus::Succeeded,
            model_content: model_content.into(),
            structured_content: content,
            model_images: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_model_images(mut self, images: Vec<ModelInputImage>) -> Self {
        self.model_images = images;
        self
    }

    #[must_use]
    pub fn failed(call: &ToolCall, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            call_id: call.id.clone(),
            tool_name: call.name.clone(),
            status: ToolResultStatus::Failed,
            structured_content: serde_json::json!({ "error": message }),
            model_content: message,
            model_images: Vec::new(),
        }
    }

    #[must_use]
    pub fn aborted(call: &ToolCall, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            call_id: call.id.clone(),
            tool_name: call.name.clone(),
            status: ToolResultStatus::Aborted,
            structured_content: serde_json::json!({ "aborted": true, "message": message }),
            model_content: message,
            model_images: Vec::new(),
        }
    }

    #[must_use]
    pub fn rejected(call: &ToolCall, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            call_id: call.id.clone(),
            tool_name: call.name.clone(),
            status: ToolResultStatus::Rejected,
            structured_content: serde_json::json!({ "rejected": true, "message": message }),
            model_content: message,
            model_images: Vec::new(),
        }
    }

    #[must_use]
    pub fn needs_attention(
        call: &ToolCall,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        let code = code.into();
        let message = message.into();
        Self {
            call_id: call.id.clone(),
            tool_name: call.name.clone(),
            status: ToolResultStatus::Rejected,
            structured_content: serde_json::json!({
                "rejected": true,
                "needsAttention": true,
                "code": code,
                "message": message,
            }),
            model_content: message,
            model_images: Vec::new(),
        }
    }

    #[must_use]
    pub fn timed_out(call: &ToolCall, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            call_id: call.id.clone(),
            tool_name: call.name.clone(),
            status: ToolResultStatus::TimedOut,
            structured_content: serde_json::json!({ "timedOut": true, "message": message }),
            model_content: message,
            model_images: Vec::new(),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ToolExecutionError {
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    #[error("tool call was produced by a stale or incompatible ToolPlan: {0}")]
    StaleToolPlan(String),
    #[error("tool arguments must be a JSON object")]
    InvalidArguments,
    #[error("tool is not permitted in the active behavior mode: {0}")]
    ForbiddenInMode(String),
    #[error("tool execution failed: {0}")]
    Failed(String),
}

pub trait ToolExecutor: Send + Sync {
    fn descriptor(&self) -> ToolDescriptor;

    /// Recovery policy is supplied by the trusted executor implementation, not
    /// by model output or a Plugin/MCP manifest. Mutation tools fail closed.
    fn recovery_policy(&self) -> ToolRecoveryPolicy {
        match self.descriptor().effect {
            ToolEffect::ReadOnly | ToolEffect::BrowserObserve | ToolEffect::ComputerObserve => {
                ToolRecoveryPolicy::ReadOnlyReplayable
            }
            ToolEffect::WorkspaceWrite
            | ToolEffect::Process
            | ToolEffect::ExternalSideEffect
            | ToolEffect::BrowserAct
            | ToolEffect::ComputerAct => ToolRecoveryPolicy::NonReplayable,
        }
    }

    fn execute(&self, invocation: ToolInvocation) -> ToolFuture;

    fn waits_for_cancellation(&self) -> bool {
        false
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ToolRegistryError {
    #[error("tool name is empty")]
    EmptyName,
    #[error("duplicate tool name: {0}")]
    DuplicateName(String),
}

pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn ToolExecutor>>,
    revision: String,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self {
            tools: BTreeMap::new(),
            revision: descriptor_revision(Vec::new()),
        }
    }
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ToolRegistry")
            .field("tools", &self.tools.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl ToolRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, executor: Arc<dyn ToolExecutor>) -> Result<(), ToolRegistryError> {
        let name = executor.descriptor().name.trim().to_owned();
        if name.is_empty() {
            return Err(ToolRegistryError::EmptyName);
        }
        if self.tools.contains_key(&name) {
            return Err(ToolRegistryError::DuplicateName(name));
        }
        self.tools.insert(name, executor);
        self.revision = descriptor_revision(self.all_descriptors());
        Ok(())
    }

    #[must_use]
    pub fn executor(&self, name: &str) -> Option<Arc<dyn ToolExecutor>> {
        self.tools.get(name).cloned()
    }

    #[must_use]
    pub fn recovery_policy(&self, name: &str) -> ToolRecoveryPolicy {
        self.tools
            .get(name)
            .map_or(ToolRecoveryPolicy::NonReplayable, |tool| {
                tool.recovery_policy()
            })
    }

    #[must_use]
    pub fn all_descriptors(&self) -> Vec<ToolDescriptor> {
        self.tools.values().map(|tool| tool.descriptor()).collect()
    }

    #[must_use]
    pub fn revision(&self) -> &str {
        &self.revision
    }

    #[must_use]
    pub fn descriptors(
        &self,
        _entry_profile: EntryProfile,
        _workload: WorkloadKind,
        mode: BehaviorMode,
    ) -> Vec<ToolDescriptor> {
        self.tools
            .values()
            .map(|tool| tool.descriptor())
            .filter(|descriptor| {
                mode != BehaviorMode::Plan
                    || matches!(
                        descriptor.effect,
                        ToolEffect::ReadOnly
                            | ToolEffect::BrowserObserve
                            | ToolEffect::ComputerObserve
                    )
            })
            .collect()
    }

    #[must_use]
    pub fn is_allowed(
        &self,
        name: &str,
        _entry_profile: EntryProfile,
        _workload: WorkloadKind,
        mode: BehaviorMode,
    ) -> bool {
        self.executor(name).is_some_and(|executor| {
            mode != BehaviorMode::Plan
                || matches!(
                    executor.descriptor().effect,
                    ToolEffect::ReadOnly | ToolEffect::BrowserObserve | ToolEffect::ComputerObserve
                )
        })
    }
}

fn descriptor_revision(mut descriptors: Vec<ToolDescriptor>) -> String {
    descriptors.sort_by(|left, right| left.name.cmp(&right.name));
    let encoded = serde_json::to_vec(&descriptors).expect("tool descriptors are serializable");
    Sha256::digest(encoded)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::future;

    use hachimi_protocol::ToolEffect;

    use super::*;

    struct StaticTool(ToolDescriptor);

    impl ToolExecutor for StaticTool {
        fn descriptor(&self) -> ToolDescriptor {
            self.0.clone()
        }

        fn execute(&self, invocation: ToolInvocation) -> ToolFuture {
            Box::pin(future::ready(Ok(ToolResult::succeeded(
                &invocation.call,
                "ok",
                Value::Null,
            ))))
        }
    }

    fn descriptor(name: &str, effect: ToolEffect) -> ToolDescriptor {
        ToolDescriptor {
            name: name.into(),
            description: name.into(),
            input_schema: serde_json::json!({ "type": "object" }),
            effect,
            parallel_safe: effect == ToolEffect::ReadOnly,
            required_scopes: Vec::new(),
        }
    }

    #[test]
    fn duplicate_names_are_rejected() {
        let mut registry = ToolRegistry::new();
        registry
            .register(Arc::new(StaticTool(descriptor(
                "workspace_read_file",
                ToolEffect::ReadOnly,
            ))))
            .expect("first");
        assert_eq!(
            registry.register(Arc::new(StaticTool(descriptor(
                "workspace_read_file",
                ToolEffect::ReadOnly
            )))),
            Err(ToolRegistryError::DuplicateName(
                "workspace_read_file".into()
            ))
        );
    }

    #[test]
    fn plan_mode_advertises_only_non_mutating_tools() {
        let mut registry = ToolRegistry::new();
        registry
            .register(Arc::new(StaticTool(descriptor(
                "workspace_read_file",
                ToolEffect::ReadOnly,
            ))))
            .expect("read");
        registry
            .register(Arc::new(StaticTool(descriptor(
                "workspace_write_file",
                ToolEffect::WorkspaceWrite,
            ))))
            .expect("write");
        registry
            .register(Arc::new(StaticTool(descriptor(
                "workspace_exec",
                ToolEffect::Process,
            ))))
            .expect("exec");
        registry
            .register(Arc::new(StaticTool(descriptor(
                "browser_observe",
                ToolEffect::BrowserObserve,
            ))))
            .expect("browser observe");
        registry
            .register(Arc::new(StaticTool(descriptor(
                "computer_observe",
                ToolEffect::ComputerObserve,
            ))))
            .expect("computer observe");
        let names = registry
            .descriptors(
                EntryProfile::Workbench,
                WorkloadKind::Coding,
                BehaviorMode::Plan,
            )
            .into_iter()
            .map(|descriptor| descriptor.name)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec!["browser_observe", "computer_observe", "workspace_read_file"]
        );
        for name in ["browser_observe", "computer_observe", "workspace_read_file"] {
            assert!(registry.is_allowed(
                name,
                EntryProfile::Workbench,
                WorkloadKind::Coding,
                BehaviorMode::Plan
            ));
        }
        assert!(!registry.is_allowed(
            "workspace_write_file",
            EntryProfile::Workbench,
            WorkloadKind::Coding,
            BehaviorMode::Plan
        ));
        assert!(registry.is_allowed(
            "workspace_exec",
            EntryProfile::Workbench,
            WorkloadKind::Office,
            BehaviorMode::Default
        ));
    }

    #[test]
    fn registry_revision_is_stable_and_changes_with_descriptors() {
        let mut first = ToolRegistry::new();
        let empty = first.revision().to_owned();
        first
            .register(Arc::new(StaticTool(descriptor(
                "workspace_read_file",
                ToolEffect::ReadOnly,
            ))))
            .expect("register");
        let populated = first.revision().to_owned();
        assert_ne!(empty, populated);

        let mut second = ToolRegistry::new();
        second
            .register(Arc::new(StaticTool(descriptor(
                "workspace_read_file",
                ToolEffect::ReadOnly,
            ))))
            .expect("register");
        assert_eq!(populated, second.revision());
    }
}
