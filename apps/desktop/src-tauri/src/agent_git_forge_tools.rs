use std::sync::Arc;

use hachimi_agent::{ToolExecutionError, ToolExecutor, ToolFuture, ToolInvocation, ToolResult};
use hachimi_protocol::{
    ForgeChangeMutation, RunId, SessionId, ToolDescriptor, ToolEffect, ToolRecoveryPolicy,
};
use hachimi_workspace::WorkspaceHostClient;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub(super) struct AgentGitForgeToolContext {
    pub(super) workspace: Arc<WorkspaceHostClient>,
    pub(super) store: hachimi_storage::AgentStore,
    pub(super) session_id: SessionId,
    pub(super) run_id: RunId,
    pub(super) network_grant: hachimi_protocol::NetworkGrant,
    pub(super) mutations_enabled: bool,
}

#[derive(Debug, Clone, Copy)]
enum AgentGitForgeToolKind {
    GitRemotes,
    GitPush,
    ForgeQuery,
    ForgeMutate,
}

#[derive(Clone)]
struct AgentGitForgeTool {
    kind: AgentGitForgeToolKind,
    context: AgentGitForgeToolContext,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitPushArgs {
    remote_name: String,
    expected_remote_url_hash: String,
    source_ref: String,
    target_ref: String,
    expected_commit_oid: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ForgeQueryArgs {
    remote_name: String,
    expected_remote_url_hash: String,
    number: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ForgeMutateArgs {
    remote_name: String,
    expected_remote_url_hash: String,
    mutation: ForgeChangeMutation,
    expected_revision: Option<String>,
    expected_commit_oid: String,
}

pub(super) fn agent_git_forge_tool_executors(
    context: AgentGitForgeToolContext,
) -> Vec<Arc<dyn ToolExecutor>> {
    let mut kinds = vec![AgentGitForgeToolKind::GitRemotes];
    if context.network_grant.unrestricted_hosts || !context.network_grant.hosts.is_empty() {
        kinds.push(AgentGitForgeToolKind::ForgeQuery);
    }
    if context.mutations_enabled && context.network_grant.enabled {
        kinds.push(AgentGitForgeToolKind::GitPush);
    }
    if context.mutations_enabled
        && (context.network_grant.unrestricted_hosts || !context.network_grant.hosts.is_empty())
    {
        kinds.push(AgentGitForgeToolKind::ForgeMutate);
    }
    kinds
        .into_iter()
        .map(|kind| {
            Arc::new(AgentGitForgeTool {
                kind,
                context: context.clone(),
            }) as Arc<dyn ToolExecutor>
        })
        .collect()
}

impl ToolExecutor for AgentGitForgeTool {
    fn descriptor(&self) -> ToolDescriptor {
        let (name, description, schema, effect) = match self.kind {
            AgentGitForgeToolKind::GitRemotes => (
                "git.remotes",
                "List the current Project checkout's Git remotes with credentials redacted and a stable URL hash for later fenced operations.",
                object_schema(json!({}), &[]),
                ToolEffect::ReadOnly,
            ),
            AgentGitForgeToolKind::GitPush => (
                "git.push",
                "Push one exact commit from the current Project checkout to a standard Git remote after remote URL, source ref, target ref, and commit OID fencing.",
                object_schema(
                    json!({
                        "remoteName": string_schema(1, 255),
                        "expectedRemoteUrlHash": hash_schema(64),
                        "sourceRef": string_schema(1, 1024),
                        "targetRef": string_schema(1, 1024),
                        "expectedCommitOid": hash_schema(40)
                    }),
                    &[
                        "remoteName",
                        "expectedRemoteUrlHash",
                        "sourceRef",
                        "targetRef",
                        "expectedCommitOid",
                    ],
                ),
                ToolEffect::ExternalSideEffect,
            ),
            AgentGitForgeToolKind::ForgeQuery => (
                "forge.change.query",
                "Query one PR/MR from the Forge derived from a current Git remote. API endpoints and credentials are resolved by the Host.",
                object_schema(
                    json!({
                        "remoteName": string_schema(1, 255),
                        "expectedRemoteUrlHash": hash_schema(64),
                        "number": { "type": "integer", "minimum": 1 }
                    }),
                    &["remoteName", "expectedRemoteUrlHash", "number"],
                ),
                ToolEffect::ReadOnly,
            ),
            AgentGitForgeToolKind::ForgeMutate => (
                "forge.change.mutate",
                "Create, update, close, or merge one PR/MR on the Forge derived from a current Git remote, fenced by revision and source commit OID.",
                object_schema(
                    json!({
                        "remoteName": string_schema(1, 255),
                        "expectedRemoteUrlHash": hash_schema(64),
                        "mutation": forge_mutation_schema(),
                        "expectedRevision": { "type": ["string", "null"], "maxLength": 1024 },
                        "expectedCommitOid": hash_schema(40)
                    }),
                    &[
                        "remoteName",
                        "expectedRemoteUrlHash",
                        "mutation",
                        "expectedRevision",
                        "expectedCommitOid",
                    ],
                ),
                ToolEffect::ExternalSideEffect,
            ),
        };
        ToolDescriptor {
            name: name.into(),
            description: description.into(),
            input_schema: schema,
            effect,
            parallel_safe: effect == ToolEffect::ReadOnly,
            required_scopes: vec![if effect == ToolEffect::ReadOnly {
                "workspace.read".into()
            } else {
                "workspace.write".into()
            }],
        }
    }

    fn recovery_policy(&self) -> ToolRecoveryPolicy {
        match self.kind {
            AgentGitForgeToolKind::GitRemotes | AgentGitForgeToolKind::ForgeQuery => {
                ToolRecoveryPolicy::ReadOnlyReplayable
            }
            AgentGitForgeToolKind::GitPush | AgentGitForgeToolKind::ForgeMutate => {
                ToolRecoveryPolicy::IdempotentWithReceipt
            }
        }
    }

    fn execute(&self, invocation: ToolInvocation) -> ToolFuture {
        let kind = self.kind;
        let context = self.context.clone();
        Box::pin(async move {
            match kind {
                AgentGitForgeToolKind::GitRemotes => git_remotes(context, invocation).await,
                AgentGitForgeToolKind::GitPush => git_push(context, invocation).await,
                AgentGitForgeToolKind::ForgeQuery => forge_query(context, invocation).await,
                AgentGitForgeToolKind::ForgeMutate => forge_mutate(context, invocation).await,
            }
        })
    }
}

async fn git_remotes(
    context: AgentGitForgeToolContext,
    invocation: ToolInvocation,
) -> Result<ToolResult, ToolExecutionError> {
    match crate::git_forge_host::list_git_remotes(
        &context.workspace,
        invocation.cancellation.child_token(),
    )
    .await
    {
        Ok(remotes) => succeeded(&invocation, remotes),
        Err(error) => Ok(ToolResult::failed(&invocation.call, error.to_string())),
    }
}

async fn git_push(
    context: AgentGitForgeToolContext,
    invocation: ToolInvocation,
) -> Result<ToolResult, ToolExecutionError> {
    let args: GitPushArgs = match parse_arguments(&invocation) {
        Ok(args) => args,
        Err(message) => return Ok(ToolResult::failed(&invocation.call, message)),
    };
    let remote = match crate::git_forge_host::resolve_git_remote(
        &context.workspace,
        &args.remote_name,
        &args.expected_remote_url_hash,
        invocation.cancellation.child_token(),
    )
    .await
    {
        Ok(remote) => remote,
        Err(error) => return Ok(ToolResult::failed(&invocation.call, error.to_string())),
    };
    if !crate::git_forge_host::network_grant_allows_remote(
        &context.network_grant,
        &remote.display_url,
    ) {
        return Ok(ToolResult::rejected(
            &invocation.call,
            "Git remote is outside the active network grant",
        ));
    }
    let result = crate::git_forge_host::push_git_remote(
        &context.workspace,
        crate::git_forge_host::GitPushSpec {
            remote_name: args.remote_name,
            expected_remote_url_hash: args.expected_remote_url_hash,
            source_ref: args.source_ref,
            target_ref: args.target_ref,
            expected_commit_oid: args.expected_commit_oid,
        },
        invocation.cancellation.child_token(),
    )
    .await;
    mutation_result(&invocation, result)
}

async fn forge_query(
    context: AgentGitForgeToolContext,
    invocation: ToolInvocation,
) -> Result<ToolResult, ToolExecutionError> {
    let args: ForgeQueryArgs = match parse_arguments(&invocation) {
        Ok(args) => args,
        Err(message) => return Ok(ToolResult::failed(&invocation.call, message)),
    };
    let repository = match crate::git_forge_host::resolve_forge_repository(
        &context.workspace,
        &args.remote_name,
        &args.expected_remote_url_hash,
        invocation.cancellation.child_token(),
    )
    .await
    {
        Ok(repository) => repository,
        Err(error) => return Ok(ToolResult::failed(&invocation.call, error.to_string())),
    };
    if !crate::git_forge_host::network_grant_allows_remote(
        &context.network_grant,
        &repository.api_base_url,
    ) {
        return Ok(ToolResult::rejected(
            &invocation.call,
            "Forge API is outside the active network grant",
        ));
    }
    match crate::git_forge_host::query_forge_change(&repository, args.number).await {
        Ok(change) => succeeded(&invocation, change),
        Err(error) => Ok(ToolResult::failed(&invocation.call, error.to_string())),
    }
}

async fn forge_mutate(
    context: AgentGitForgeToolContext,
    invocation: ToolInvocation,
) -> Result<ToolResult, ToolExecutionError> {
    let args: ForgeMutateArgs = match parse_arguments(&invocation) {
        Ok(args) => args,
        Err(message) => return Ok(ToolResult::failed(&invocation.call, message)),
    };
    let repository = match crate::git_forge_host::resolve_forge_repository(
        &context.workspace,
        &args.remote_name,
        &args.expected_remote_url_hash,
        invocation.cancellation.child_token(),
    )
    .await
    {
        Ok(repository) => repository,
        Err(error) => return Ok(ToolResult::failed(&invocation.call, error.to_string())),
    };
    if !crate::git_forge_host::network_grant_allows_remote(
        &context.network_grant,
        &repository.api_base_url,
    ) {
        return Ok(ToolResult::rejected(
            &invocation.call,
            "Forge API is outside the active network grant",
        ));
    }
    let metadata = crate::git_forge_host::mutation_metadata(&args.mutation, &repository);
    let result = crate::git_forge_host::mutate_forge_change(
        &context.store,
        &repository,
        &args.mutation,
        crate::git_forge_host::ForgeMutationLedgerContext {
            session_id: context.session_id,
            run_id: context.run_id,
            run_generation: invocation.run_generation,
            operation_kind: metadata.operation_kind.into(),
            source_ref: metadata.source_ref,
            target_ref: metadata.target_ref,
            expected_commit_oid: args.expected_commit_oid,
            expected_revision: args.expected_revision,
            approval_id: None,
            idempotency_key: agent_idempotency_key(&invocation),
            request_hash: argument_hash(&invocation.call.arguments)?,
        },
    )
    .await;
    mutation_result(&invocation, result)
}

fn mutation_result<T: serde::Serialize>(
    invocation: &ToolInvocation,
    result: Result<T, crate::git_forge_host::GitForgeHostError>,
) -> Result<ToolResult, ToolExecutionError> {
    match result {
        Ok(value) => succeeded(invocation, value),
        Err(error) if error.indeterminate => Err(ToolExecutionError::Failed(error.to_string())),
        Err(error) => Ok(ToolResult::failed(&invocation.call, error.to_string())),
    }
}

fn succeeded<T: serde::Serialize>(
    invocation: &ToolInvocation,
    value: T,
) -> Result<ToolResult, ToolExecutionError> {
    let content = serde_json::to_value(value)
        .map_err(|error| ToolExecutionError::Failed(error.to_string()))?;
    Ok(ToolResult::succeeded(
        &invocation.call,
        content.to_string(),
        content,
    ))
}

fn parse_arguments<T: for<'de> Deserialize<'de>>(invocation: &ToolInvocation) -> Result<T, String> {
    serde_json::from_value(invocation.call.arguments.clone())
        .map_err(|error| format!("{} arguments are invalid: {error}", invocation.call.name))
}

fn agent_idempotency_key(invocation: &ToolInvocation) -> String {
    let hash = Sha256::digest(
        format!(
            "{}:{}:{}",
            invocation.call.name, invocation.call.id, invocation.run_generation
        )
        .as_bytes(),
    );
    format!("agent:{}", hex_hash(&hash))
}

fn argument_hash(arguments: &Value) -> Result<String, ToolExecutionError> {
    let bytes = serde_json::to_vec(arguments)
        .map_err(|error| ToolExecutionError::Failed(error.to_string()))?;
    Ok(hex_hash(&Sha256::digest(bytes)))
}

fn hex_hash(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn object_schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

fn string_schema(min_length: u64, max_length: u64) -> Value {
    json!({ "type": "string", "minLength": min_length, "maxLength": max_length })
}

fn hash_schema(length: u64) -> Value {
    json!({
        "type": "string",
        "minLength": length,
        "maxLength": length,
        "pattern": "^[0-9a-fA-F]+$"
    })
}

fn forge_mutation_schema() -> Value {
    json!({
        "oneOf": [
            {
                "type": "object",
                "properties": {
                    "kind": { "const": "create" },
                    "title": string_schema(1, 1024),
                    "body": { "type": "string", "maxLength": 65536 },
                    "source_ref": string_schema(1, 1024),
                    "target_ref": string_schema(1, 1024)
                },
                "required": ["kind", "title", "body", "source_ref", "target_ref"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "kind": { "const": "update" },
                    "number": { "type": "integer", "minimum": 1 },
                    "title": string_schema(1, 1024),
                    "body": { "type": "string", "maxLength": 65536 },
                    "source_ref": string_schema(1, 1024),
                    "target_ref": string_schema(1, 1024)
                },
                "required": ["kind", "number", "title", "body", "source_ref", "target_ref"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "kind": { "const": "close" },
                    "number": { "type": "integer", "minimum": 1 }
                },
                "required": ["kind", "number"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "kind": { "const": "merge" },
                    "number": { "type": "integer", "minimum": 1 },
                    "merge_title": { "type": ["string", "null"], "maxLength": 1024 },
                    "merge_message": { "type": ["string", "null"], "maxLength": 65536 }
                },
                "required": ["kind", "number", "merge_title", "merge_message"],
                "additionalProperties": false
            }
        ]
    })
}

#[cfg(test)]
mod tests {
    use hachimi_agent::{ToolCall, ToolPlan, ToolPlanConstraints};
    use hachimi_protocol::{
        BehaviorMode, CheckoutId, EntryProfile, ProviderCapabilities, ToolCallId, WorkloadKind,
    };
    use tokio_util::sync::CancellationToken;

    use super::*;

    async fn context(network_enabled: bool, mutations_enabled: bool) -> AgentGitForgeToolContext {
        AgentGitForgeToolContext {
            workspace: Arc::new(WorkspaceHostClient::new(
                "worker",
                ".",
                CheckoutId::from("checkout").as_str(),
                1,
            )),
            store: hachimi_storage::AgentStore::connect_in_memory()
                .await
                .expect("store"),
            session_id: SessionId::from("session"),
            run_id: RunId::from("run"),
            network_grant: hachimi_protocol::NetworkGrant {
                enabled: network_enabled,
                unrestricted_hosts: false,
                hosts: network_enabled
                    .then(|| "forge.example.test".into())
                    .into_iter()
                    .collect(),
                protocols: network_enabled
                    .then(|| "https".into())
                    .into_iter()
                    .collect(),
            },
            mutations_enabled,
        }
    }

    #[tokio::test]
    async fn registration_intersects_network_and_mutation_features() {
        async fn names(network: bool, mutations: bool) -> Vec<String> {
            agent_git_forge_tool_executors(context(network, mutations).await)
                .into_iter()
                .map(|tool| tool.descriptor().name)
                .collect()
        }
        assert_eq!(names(false, true).await, vec!["git.remotes"]);
        assert_eq!(
            names(true, false).await,
            vec!["git.remotes", "forge.change.query"]
        );
        assert_eq!(
            names(true, true).await,
            vec![
                "git.remotes",
                "forge.change.query",
                "git.push",
                "forge.change.mutate"
            ]
        );
    }

    #[tokio::test]
    async fn descriptors_are_available_to_every_entry_and_keep_effect_metadata() {
        for tool in agent_git_forge_tool_executors(context(true, true).await) {
            let descriptor = tool.descriptor();
            assert!(!descriptor.name.is_empty());
            assert_eq!(
                descriptor.parallel_safe,
                descriptor.effect == ToolEffect::ReadOnly
            );
        }
    }

    #[tokio::test]
    async fn tool_plan_is_profile_independent_and_plan_mode_removes_mutations() {
        let descriptors = agent_git_forge_tool_executors(context(true, true).await)
            .into_iter()
            .map(|tool| tool.descriptor())
            .collect::<Vec<_>>();
        let provider = ProviderCapabilities {
            tool_calls: true,
            ..ProviderCapabilities::default()
        };
        let names = |entry_profile, workload, behavior_mode| {
            ToolPlan::build(
                entry_profile,
                workload,
                behavior_mode,
                provider,
                descriptors.clone(),
                ToolPlanConstraints {
                    run_allowlist: None,
                    disabled_tool_names: &[],
                    capability_grants: None,
                    host_ready: true,
                },
            )
            .descriptors()
            .iter()
            .map(|descriptor| descriptor.name.clone())
            .collect::<Vec<_>>()
        };
        let coding = names(
            EntryProfile::Workbench,
            WorkloadKind::Coding,
            BehaviorMode::Default,
        );
        assert_eq!(
            coding,
            names(
                EntryProfile::Workbench,
                WorkloadKind::Office,
                BehaviorMode::Default
            )
        );
        assert_eq!(
            coding,
            names(
                EntryProfile::PetConversation,
                WorkloadKind::General,
                BehaviorMode::Default
            )
        );
        assert_eq!(
            names(
                EntryProfile::Workbench,
                WorkloadKind::Coding,
                BehaviorMode::Plan
            ),
            vec!["forge.change.query", "git.remotes"]
        );
    }

    #[test]
    fn generated_agent_idempotency_key_is_bounded_and_not_model_controlled() {
        let invocation = ToolInvocation {
            call: ToolCall {
                id: ToolCallId::from("call"),
                name: "forge.change.mutate".into(),
                arguments: json!({ "idempotencyKey": "model-value" }),
                step_revision: 1,
                tool_plan_hash: "plan".into(),
                registry_revision: "registry".into(),
            },
            entry_profile: EntryProfile::Workbench,
            workload: WorkloadKind::Coding,
            behavior_mode: hachimi_protocol::BehaviorMode::Default,
            run_generation: 7,
            step_revision: 1,
            tool_plan_hash: "plan".into(),
            registry_revision: "registry".into(),
            cancellation: CancellationToken::new(),
        };
        let key = agent_idempotency_key(&invocation);
        assert!(key.len() <= 128);
        assert!(!key.contains("model-value"));
    }

    #[test]
    fn unknown_mutation_outcomes_escape_as_executor_errors() {
        let invocation = ToolInvocation {
            call: ToolCall {
                id: ToolCallId::from("call"),
                name: "git.push".into(),
                arguments: json!({}),
                step_revision: 1,
                tool_plan_hash: "plan".into(),
                registry_revision: "registry".into(),
            },
            entry_profile: EntryProfile::Workbench,
            workload: WorkloadKind::Coding,
            behavior_mode: BehaviorMode::Default,
            run_generation: 1,
            step_revision: 1,
            tool_plan_hash: "plan".into(),
            registry_revision: "registry".into(),
            cancellation: CancellationToken::new(),
        };
        let unknown = crate::git_forge_host::GitForgeHostError {
            code: "git_push_indeterminate",
            message: "unknown".into(),
            indeterminate: true,
        };
        assert!(mutation_result::<Value>(&invocation, Err(unknown)).is_err());

        let rejected = crate::git_forge_host::GitForgeHostError {
            code: "git_push_invalid",
            message: "invalid".into(),
            indeterminate: false,
        };
        let result = mutation_result::<Value>(&invocation, Err(rejected)).expect("tool result");
        assert_eq!(result.status, hachimi_agent::ToolResultStatus::Failed);
    }
}
