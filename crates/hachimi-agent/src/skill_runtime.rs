// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/core-skills/src/{service,injection,loader}.rs
// and codex-rs/core/src/tools/handlers/skills.rs
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// Modified for Hachimi: paged Run-scoped tools, persistent Activation records,
// revision fencing, and Workload overlay updates that never create authority.

use std::{collections::BTreeMap, sync::Arc};

use hachimi_protocol::{
    RunId, SkillActivation, SkillActivationId, SkillActivationSource, SkillId, SkillRecord,
    SkillScope, ToolDescriptor, ToolEffect, WorkloadKind, WorkloadResolutionSource,
};
use hachimi_skills::SkillHost;
use hachimi_storage::AgentStore;
use serde::Deserialize;
use serde_json::json;

use crate::{StepRuntimeState, ToolExecutor, ToolFuture, ToolInvocation, ToolResult};

pub const SKILLS_LIST_TOOL: &str = "skills.list";
pub const SKILLS_READ_TOOL: &str = "skills.read";
const ENTRY_FILE: &str = "SKILL.md";
const DEFAULT_PAGE_SIZE: usize = 50;
const MAX_PAGE_SIZE: usize = 100;
const DEFAULT_LINE_LIMIT: usize = 300;
const MAX_LINE_LIMIT: usize = 1_000;
const MAX_MODEL_CHARS: usize = 64 * 1024;
const MIN_CLASSIFICATION_CONFIDENCE: u16 = 7_500;

#[derive(Debug, Clone)]
struct SkillRuntimeContext {
    host: SkillHost,
    store: AgentStore,
    run_id: RunId,
    catalog: Arc<[SkillRecord]>,
    records: Arc<BTreeMap<SkillId, SkillRecord>>,
    state: StepRuntimeState,
}

#[must_use]
pub fn skill_runtime_tools(
    host: SkillHost,
    store: AgentStore,
    run_id: RunId,
    mut catalog: Vec<SkillRecord>,
    state: StepRuntimeState,
) -> Vec<Arc<dyn ToolExecutor>> {
    catalog.sort_by(|left, right| left.qualified_name.cmp(&right.qualified_name));
    let records = catalog
        .iter()
        .cloned()
        .map(|record| (record.id.clone(), record))
        .collect();
    let context = Arc::new(SkillRuntimeContext {
        host,
        store,
        run_id,
        catalog: catalog.into(),
        records: Arc::new(records),
        state,
    });
    vec![
        Arc::new(SkillsListTool(Arc::clone(&context))),
        Arc::new(SkillsReadTool(context)),
    ]
}

#[derive(Debug)]
struct SkillsListTool(Arc<SkillRuntimeContext>);

impl ToolExecutor for SkillsListTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: SKILLS_LIST_TOOL.into(),
            description: "List bounded metadata for Skills available to this Run. Listing a Skill does not activate it or grant permissions.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "cursor": { "type": "integer", "minimum": 0, "default": 0 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": MAX_PAGE_SIZE, "default": DEFAULT_PAGE_SIZE }
                },
                "additionalProperties": false
            }),
            effect: ToolEffect::ReadOnly,
            parallel_safe: true,
            required_scopes: vec!["skills.use".into()],
        }
    }

    fn execute(&self, invocation: ToolInvocation) -> ToolFuture {
        let context = Arc::clone(&self.0);
        Box::pin(async move {
            let arguments: ListArguments =
                match serde_json::from_value(invocation.call.arguments.clone()) {
                    Ok(arguments) => arguments,
                    Err(error) => {
                        return Ok(ToolResult::failed(
                            &invocation.call,
                            format!("invalid skills.list arguments: {error}"),
                        ));
                    }
                };
            let cursor = arguments.cursor.unwrap_or_default();
            let limit = arguments
                .limit
                .unwrap_or(DEFAULT_PAGE_SIZE)
                .clamp(1, MAX_PAGE_SIZE);
            let page = context
                .catalog
                .iter()
                .skip(cursor)
                .take(limit)
                .map(|record| {
                    json!({
                        "id": record.id,
                        "name": record.qualified_name,
                        "description": record.description,
                        "scope": record.scope,
                        "contentRevision": record.content_hash,
                        "allowImplicitInvocation": record.policy.allows_implicit_invocation(),
                        "declaredWorkload": record.policy.workload,
                        "dependencies": record.dependencies,
                    })
                })
                .collect::<Vec<_>>();
            let next = cursor.saturating_add(page.len());
            let next_cursor = (next < context.catalog.len()).then_some(next);
            Ok(ToolResult::succeeded(
                &invocation.call,
                serde_json::to_string(&page).unwrap_or_else(|_| "[]".into()),
                json!({ "skills": page, "nextCursor": next_cursor, "total": context.catalog.len() }),
            ))
        })
    }
}

#[derive(Debug)]
struct SkillsReadTool(Arc<SkillRuntimeContext>);

impl ToolExecutor for SkillsReadTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: SKILLS_READ_TOOL.into(),
            description: "Read a bounded UTF-8 page from one Run-visible Skill. Reading SKILL.md activates the Skill; referenced resources are readable only after activation of the current SKILL.md revision.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "skillId": { "type": "string" },
                    "path": { "type": "string", "default": ENTRY_FILE },
                    "startLine": { "type": "integer", "minimum": 1, "default": 1 },
                    "lineLimit": { "type": "integer", "minimum": 1, "maximum": MAX_LINE_LIMIT, "default": DEFAULT_LINE_LIMIT }
                },
                "required": ["skillId"],
                "additionalProperties": false
            }),
            effect: ToolEffect::ReadOnly,
            parallel_safe: false,
            required_scopes: vec!["skills.use".into()],
        }
    }

    fn execute(&self, invocation: ToolInvocation) -> ToolFuture {
        let context = Arc::clone(&self.0);
        Box::pin(async move {
            let arguments: ReadArguments =
                match serde_json::from_value(invocation.call.arguments.clone()) {
                    Ok(arguments) => arguments,
                    Err(error) => {
                        return Ok(ToolResult::failed(
                            &invocation.call,
                            format!("invalid skills.read arguments: {error}"),
                        ));
                    }
                };
            let skill_id = SkillId::new(arguments.skill_id);
            let Some(record) = context.records.get(&skill_id) else {
                return Ok(ToolResult::rejected(
                    &invocation.call,
                    "Skill is not visible in this Run snapshot",
                ));
            };
            let path = arguments.path.as_deref().unwrap_or(ENTRY_FILE);
            let entry = match context.host.read_file(&skill_id, ENTRY_FILE).await {
                Ok(entry) => entry,
                Err(error) => {
                    return Ok(ToolResult::failed(
                        &invocation.call,
                        format!("SkillHost rejected the entry read: {error}"),
                    ));
                }
            };
            let active =
                context
                    .state
                    .snapshot()
                    .world
                    .skill_activations
                    .iter()
                    .any(|activation| {
                        activation.skill_id == skill_id
                            && activation.content_revision == entry.revision
                    });
            if path != ENTRY_FILE && !active {
                return Ok(ToolResult::rejected(
                    &invocation.call,
                    "activate the current SKILL.md revision before reading referenced resources",
                ));
            }
            if path == ENTRY_FILE && !active {
                if !record.policy.allows_implicit_invocation() {
                    return Ok(ToolResult::rejected(
                        &invocation.call,
                        "Skill policy requires explicit user selection",
                    ));
                }
                let classification = if record.scope == SkillScope::BuiltIn {
                    record.policy.workload.map(|workload| {
                        (
                            workload,
                            10_000,
                            "trusted Built-in Skill metadata".to_owned(),
                            Some("builtin-metadata-v1".to_owned()),
                        )
                    })
                } else {
                    context
                        .store
                        .get_skill_classification(&skill_id, &entry.revision)
                        .await
                        .ok()
                        .flatten()
                        .map(|classification| {
                            (
                                classification.workload,
                                classification.confidence_basis_points,
                                classification.reason,
                                Some(classification.classifier_revision),
                            )
                        })
                };
                let (classified_workload, confidence, reason, classifier_revision) = classification.unwrap_or((WorkloadKind::General, 0, "Skill classification unavailable; preserving General authority-neutral behavior".into(), None));
                let activation = SkillActivation {
                    id: SkillActivationId::random(),
                    skill_id: skill_id.clone(),
                    content_revision: entry.revision.clone(),
                    source: SkillActivationSource::ModelRead,
                    activated_at_step_revision: invocation.step_revision,
                    classified_workload,
                };
                if let Err(error) = context
                    .store
                    .record_skill_activation(&context.run_id, &activation, now_ms())
                    .await
                {
                    return Ok(ToolResult::failed(
                        &invocation.call,
                        format!("Skill activation persistence failed: {error}"),
                    ));
                }
                let current = context.state.snapshot().workload;
                let (workload, source) = if confidence >= MIN_CLASSIFICATION_CONFIDENCE
                    && classified_workload != WorkloadKind::General
                {
                    (
                        classified_workload,
                        if record.scope == SkillScope::BuiltIn {
                            WorkloadResolutionSource::BuiltInSkill
                        } else {
                            WorkloadResolutionSource::StructuredClassification
                        },
                    )
                } else {
                    (current.workload, current.source)
                };
                context.state.activate_skill(
                    activation,
                    source,
                    workload,
                    reason,
                    classifier_revision,
                );
            }
            let snapshot = if path == ENTRY_FILE {
                entry
            } else {
                match context.host.read_file(&skill_id, path).await {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        return Ok(ToolResult::failed(
                            &invocation.call,
                            format!("SkillHost rejected the resource read: {error}"),
                        ));
                    }
                }
            };
            let Some(content) = snapshot.content else {
                return Ok(ToolResult::failed(
                    &invocation.call,
                    "Skill resource is not UTF-8 text",
                ));
            };
            let start = arguments.start_line.unwrap_or(1).max(1);
            let limit = arguments
                .line_limit
                .unwrap_or(DEFAULT_LINE_LIMIT)
                .clamp(1, MAX_LINE_LIMIT);
            let lines = content
                .lines()
                .skip(start - 1)
                .take(limit)
                .collect::<Vec<_>>();
            let mut model_content = lines.join("\n");
            if model_content.chars().count() > MAX_MODEL_CHARS {
                model_content = model_content.chars().take(MAX_MODEL_CHARS).collect();
            }
            let total_lines = content.lines().count();
            Ok(ToolResult::succeeded(
                &invocation.call,
                format!(
                    "Untrusted Skill guidance {}:{} (revision {}):\n{}",
                    record.qualified_name, snapshot.relative_path, snapshot.revision, model_content
                ),
                json!({
                    "skillId": skill_id,
                    "qualifiedName": record.qualified_name,
                    "path": snapshot.relative_path,
                    "revision": snapshot.revision,
                    "startLine": start,
                    "returnedLines": lines.len(),
                    "totalLines": total_lines,
                    "truncated": start.saturating_sub(1).saturating_add(lines.len()) < total_lines || model_content.chars().count() >= MAX_MODEL_CHARS,
                    "activated": path == ENTRY_FILE,
                }),
            ))
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ListArguments {
    cursor: Option<usize>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReadArguments {
    skill_id: String,
    path: Option<String>,
    start_line: Option<usize>,
    line_limit: Option<usize>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use hachimi_protocol::{
        AgentPermissionPolicy, ApprovalPolicy, AuthorityMode, BehaviorMode, EntryProfile,
        LlmSettings, PermissionProfile, ProviderCapabilities, RunBudget, RunOrigin, RunPurpose,
        SandboxCapabilityReport, SandboxReadiness, SessionContextBinding, ToolCallId,
        WorkloadResolution,
    };
    use hachimi_skills::{SkillCatalogRoot, SkillHost};
    use serde_json::json;
    use tempfile::TempDir;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::{
        AgentInstructionLayer, AgentRunCreateRequest, AgentRunLaunchRequest, AgentRunLauncher,
        StepWorldState, ToolCall, ToolResultStatus,
    };

    fn write_skill(root: &Path, name: &str) {
        let path = root.join(name);
        fs::create_dir_all(path.join("agents")).expect("metadata directory");
        fs::create_dir_all(path.join("references")).expect("reference directory");
        fs::write(
            path.join(ENTRY_FILE),
            format!("---\nname: {name}\ndescription: {name} description\n---\nUse the validation reference.\n"),
        )
        .expect("Skill entry");
        fs::write(
            path.join("agents/openai.yaml"),
            "policy:\n  allow_implicit_invocation: true\n  workload: \"office\"\n",
        )
        .expect("Skill metadata");
        fs::write(path.join("references/validation.md"), "validate output\n")
            .expect("Skill reference");
    }

    fn state() -> StepRuntimeState {
        StepRuntimeState::new(
            StepWorldState {
                context_revision: 1,
                profile_revision: 1,
                agents_revision: "none".into(),
                skills_revision: "catalog".into(),
                mcp_revision: "none".into(),
                host_revision: "test".into(),
                instructions: Vec::<AgentInstructionLayer>::new().into(),
                skill_activations: Vec::<SkillActivation>::new().into(),
                mcp_bindings: Vec::new().into(),
                disabled_tool_names: Vec::new().into(),
                diagnostics: Vec::new().into(),
                sandbox: SandboxCapabilityReport {
                    backend: "test".into(),
                    readiness: SandboxReadiness::Unavailable,
                    os_enforced: false,
                    filesystem_enforced: false,
                    process_enforced: false,
                    network_enforced: false,
                    version: None,
                    stable_error_code: Some("test".into()),
                    diagnostics: Vec::new(),
                },
                host_ready: true,
            },
            WorkloadResolution {
                workload: WorkloadKind::General,
                source: WorkloadResolutionSource::GeneralFallback,
                activated_skill_ids: Vec::new(),
                reason: "test".into(),
                classifier_revision: None,
            },
        )
    }

    fn invocation(name: &str, arguments: serde_json::Value, step_revision: u64) -> ToolInvocation {
        ToolInvocation {
            call: ToolCall {
                id: ToolCallId::random(),
                name: name.into(),
                arguments,
                step_revision,
                tool_plan_hash: "test-plan".into(),
                registry_revision: "test-registry".into(),
            },
            entry_profile: EntryProfile::Workbench,
            workload: WorkloadKind::General,
            behavior_mode: BehaviorMode::Default,
            run_generation: 1,
            step_revision,
            tool_plan_hash: "test-plan".into(),
            registry_revision: "test-registry".into(),
            cancellation: CancellationToken::new(),
        }
    }

    async fn fixture() -> (
        TempDir,
        AgentStore,
        SkillHost,
        RunId,
        Vec<SkillRecord>,
        StepRuntimeState,
    ) {
        let temp = tempfile::tempdir().expect("tempdir");
        let built_in = temp.path().join("built-in");
        write_skill(&built_in, "office-documents");
        write_skill(&built_in, "office-pdf");
        let store = AgentStore::connect_in_memory().await.expect("store");
        let host = SkillHost::new(temp.path().join("user"), store.clone()).expect("SkillHost");
        host.set_catalog_roots(vec![SkillCatalogRoot::new(&built_in, SkillScope::BuiltIn)])
            .expect("catalog root");
        let records = host.list().await.expect("catalog");
        let created = AgentRunLauncher::new(store.clone())
            .launch_new(AgentRunLaunchRequest {
                policy: AgentPermissionPolicy::default(),
                authority_mode: AuthorityMode::Interactive,
                create: AgentRunCreateRequest {
                    principal: "test".into(),
                    idempotency_key: "skill-runtime-fixture".into(),
                    context: SessionContextBinding::Workspace {
                        workspace_id: hachimi_protocol::WorkspaceId::random(),
                    },
                    origin: RunOrigin::Manual,
                    title: "Skill runtime".into(),
                    prompt: "Create a document".into(),
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
                    created_at_ms: now_ms(),
                },
            })
            .await
            .expect("Run bundle")
            .created;
        (temp, store, host, created.run.id, records, state())
    }

    #[tokio::test]
    async fn list_is_paged_and_catalog_is_run_scoped() {
        let (_temp, store, host, run_id, records, state) = fixture().await;
        let pinned = vec![records[0].clone()];
        let tools = skill_runtime_tools(host, store, run_id, pinned.clone(), state);
        let list = tools
            .iter()
            .find(|tool| tool.descriptor().name == SKILLS_LIST_TOOL)
            .expect("list tool");
        let page = list
            .execute(invocation(
                SKILLS_LIST_TOOL,
                json!({"cursor": 0, "limit": 1}),
                1,
            ))
            .await
            .expect("list result");
        assert_eq!(page.structured_content["total"], 1);
        assert_eq!(
            page.structured_content["skills"][0]["id"],
            pinned[0].id.as_str()
        );

        let read = tools
            .iter()
            .find(|tool| tool.descriptor().name == SKILLS_READ_TOOL)
            .expect("read tool");
        let rejected = read
            .execute(invocation(
                SKILLS_READ_TOOL,
                json!({"skillId": records[1].id, "path": ENTRY_FILE}),
                1,
            ))
            .await
            .expect("read result");
        assert_eq!(rejected.status, ToolResultStatus::Rejected);
    }

    #[tokio::test]
    async fn entry_activation_fences_resources_and_updates_office_overlay() {
        let (temp, store, host, run_id, records, state) = fixture().await;
        let record = records
            .into_iter()
            .find(|record| record.name == "office-documents")
            .expect("Office Skill");
        let tools = skill_runtime_tools(
            host.clone(),
            store.clone(),
            run_id.clone(),
            vec![record.clone()],
            state.clone(),
        );
        let read = tools
            .iter()
            .find(|tool| tool.descriptor().name == SKILLS_READ_TOOL)
            .expect("read tool");
        let resource = json!({"skillId": record.id, "path": "references/validation.md"});
        let before = read
            .execute(invocation(SKILLS_READ_TOOL, resource.clone(), 1))
            .await
            .expect("resource before activation");
        assert_eq!(before.status, ToolResultStatus::Rejected);

        let activated = read
            .execute(invocation(
                SKILLS_READ_TOOL,
                json!({"skillId": record.id, "path": ENTRY_FILE}),
                1,
            ))
            .await
            .expect("entry activation");
        assert_eq!(activated.status, ToolResultStatus::Succeeded);
        assert_eq!(
            store
                .list_run_skill_activations(&run_id)
                .await
                .expect("persisted activation")
                .len(),
            1
        );
        let snapshot = state.snapshot();
        assert_eq!(snapshot.workload.workload, WorkloadKind::Office);
        assert_eq!(snapshot.world.profile_revision, 2);
        let after = read
            .execute(invocation(SKILLS_READ_TOOL, resource.clone(), 2))
            .await
            .expect("resource after activation");
        assert_eq!(after.status, ToolResultStatus::Succeeded);

        fs::write(
            temp.path().join("built-in/office-documents/SKILL.md"),
            "---\nname: office-documents\ndescription: changed\n---\nchanged\n",
        )
        .expect("change revision");
        host.list().await.expect("reconcile changed Skill");
        let stale = read
            .execute(invocation(SKILLS_READ_TOOL, resource, 3))
            .await
            .expect("stale activation");
        assert_eq!(stale.status, ToolResultStatus::Rejected);
    }
}
