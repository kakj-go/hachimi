use hachimi_agent::ModelRuntimeError;
use hachimi_protocol::{
    ModelEvent, ModelFinishReason, ModelMessage, ModelRequest, ModelRole, ModelToolCall,
    TokenUsage, ToolCallId,
};

type Response = Vec<Result<ModelEvent, ModelRuntimeError>>;

pub(super) fn response(request: &ModelRequest) -> Option<Response> {
    if has_marker(&request.messages, "[desktop-e2e:multi-agent-child]") {
        return Some(text_response(
            "Desktop E2E child Agent completed with narrowed read-only authority.",
        ));
    }
    if has_marker(&request.messages, "[desktop-e2e:multi-agent-tools]") {
        return Some(multi_agent_response(request, "Coding"));
    }
    if has_marker(&request.messages, "[desktop-e2e:multi-agent-general]") {
        return Some(multi_agent_response(request, "General"));
    }
    if has_marker(&request.messages, "[desktop-e2e:multi-agent-office]") {
        return Some(multi_agent_response(request, "Office"));
    }
    if has_marker(
        &request.messages,
        "[desktop-e2e:agent-feature-flags-disabled-coding]",
    ) {
        return Some(disabled_feature_response(request, "Coding"));
    }
    if has_marker(
        &request.messages,
        "[desktop-e2e:agent-feature-flags-disabled-general]",
    ) {
        return Some(disabled_feature_response(request, "General"));
    }
    if has_marker(&request.messages, "[desktop-e2e:agent-git-forge]") {
        return Some(git_forge_response(request));
    }
    if has_marker(&request.messages, "[desktop-e2e:approval-recovery]") {
        return Some(approval_recovery_response(request));
    }
    if has_marker(&request.messages, "[desktop-e2e:user-input-recovery]") {
        return Some(user_input_recovery_response(request));
    }
    if has_marker(&request.messages, "[desktop-e2e:read-only-recovery]") {
        return Some(read_only_recovery_response(request));
    }
    if has_marker(
        &request.messages,
        "[desktop-e2e:idempotent-receipt-recovery]",
    ) {
        return Some(idempotent_receipt_recovery_response(request));
    }
    if has_marker(&request.messages, "[desktop-e2e:indeterminate-recovery]") {
        return Some(indeterminate_recovery_response(request));
    }
    if has_marker(&request.messages, "[desktop-e2e:agent-forge-lifecycle]") {
        return Some(forge_lifecycle_response(request));
    }
    if has_marker(&request.messages, "[desktop-e2e:agent-forge-duplicate]") {
        return Some(forge_duplicate_response(request));
    }
    if has_marker(&request.messages, "[desktop-e2e:agent-forge-unknown]") {
        return Some(forge_unknown_response(request));
    }
    if has_marker(&request.messages, "[desktop-e2e:agent-forge-revision]") {
        return Some(forge_revision_response(request));
    }
    if has_marker(&request.messages, "[desktop-e2e:agent-forge-credential]") {
        return Some(forge_credential_response(request));
    }
    if has_marker(
        &request.messages,
        "[desktop-e2e:enterprise-attachment-tool]",
    ) {
        return Some(enterprise_attachment_response(request));
    }
    None
}

pub(super) fn waits_for_process_restart(request: &ModelRequest) -> bool {
    if run_generation(request) != Some(1) {
        return false;
    }
    (has_marker(&request.messages, "[desktop-e2e:read-only-recovery]")
        && completed(request, "workspace_read_file"))
        || (has_marker(
            &request.messages,
            "[desktop-e2e:idempotent-receipt-recovery]",
        ) && completed(request, "agent.spawn"))
}

fn user_input_recovery_response(request: &ModelRequest) -> Response {
    if completed(request, "request_user_input") {
        return text_response("Desktop E2E user input unexpectedly survived process restart.");
    }
    tool_call(
        "desktop-e2e-restart-user-input",
        "request_user_input",
        serde_json::json!({
            "questions": [{
                "id": "restart_confirmation",
                "header": "Restart recovery",
                "question": "Choose how this Plan run should proceed after restart.",
                "options": [
                    {
                        "label": "Resume",
                        "description": "Continue planning after the process restart."
                    },
                    {
                        "label": "Stop",
                        "description": "Leave the interrupted Plan run unresolved."
                    }
                ]
            }]
        }),
    )
}

fn read_only_recovery_response(request: &ModelRequest) -> Response {
    if !completed(request, "workspace_read_file") {
        return tool_call(
            "desktop-e2e-restart-read",
            "workspace_read_file",
            serde_json::json!({ "path": "README.md" }),
        );
    }
    text_response("Desktop E2E read-only checkpoint resumed on generation 2.")
}

fn idempotent_receipt_recovery_response(request: &ModelRequest) -> Response {
    if !completed(request, "agent.spawn") {
        return tool_call(
            "desktop-e2e-restart-agent-spawn",
            "agent.spawn",
            serde_json::json!({
                "title": "Restart receipt child",
                "prompt": "[desktop-e2e:multi-agent-child] finish without tools",
                "permissionProfile": "read_only",
                "maxModelRequests": 2,
                "maxToolCalls": 1
            }),
        );
    }
    let receipts = request
        .messages
        .iter()
        .filter(|message| {
            message.role == ModelRole::Tool && message.name.as_deref() == Some("agent.spawn")
        })
        .count();
    if receipts != 1 {
        return failure(format!(
            "Expected one durable agent.spawn receipt, found {receipts}"
        ));
    }
    text_response("Desktop E2E idempotent receipt resumed exactly once on generation 2.")
}

fn indeterminate_recovery_response(request: &ModelRequest) -> Response {
    if completed(request, crate::desktop_e2e::BLOCKING_EXTERNAL_EFFECT_TOOL) {
        return failure("Blocking external effect must never be replayed after restart".into());
    }
    tool_call(
        "desktop-e2e-restart-indeterminate",
        crate::desktop_e2e::BLOCKING_EXTERNAL_EFFECT_TOOL,
        serde_json::json!({ "operationId": "dispatch-without-receipt" }),
    )
}

fn run_generation(request: &ModelRequest) -> Option<u64> {
    request.messages.iter().find_map(|message| {
        let marker = "run_generation=";
        let start = message.content.find(marker)? + marker.len();
        let digits = message.content[start..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>();
        digits.parse().ok()
    })
}

fn disabled_feature_response(request: &ModelRequest, expected_workload: &str) -> Response {
    if !request.messages.iter().any(|message| {
        message.role == ModelRole::System
            && message
                .content
                .contains(&format!("workload={expected_workload}"))
    }) {
        return failure(format!(
            "Feature-flag task did not resolve to {expected_workload}"
        ));
    }
    let forbidden = [
        "agent.spawn",
        "agent.send",
        "agent.wait",
        "agent.cancel",
        "agent.collect",
        "enterprise.download_attachment",
        "git.push",
        "forge.change.mutate",
    ];
    if let Some(exposed) = forbidden.iter().find(|name| has_tool(request, name)) {
        return failure(format!("Disabled Agent feature exposed {exposed}"));
    }
    if expected_workload == "Coding" && !has_tool(request, "git.remotes") {
        return failure("Git mutation feature flag removed the read-only git.remotes tool".into());
    }
    text_response(&format!(
        "Desktop E2E {expected_workload} feature flags removed Multi-Agent, enterprise attachment, and Git/Forge mutations from the real ToolPlan."
    ))
}

fn multi_agent_response(request: &ModelRequest, expected_workload: &str) -> Response {
    if !request.messages.iter().any(|message| {
        message.role == ModelRole::System
            && message
                .content
                .contains(&format!("workload={expected_workload}"))
    }) {
        return failure(format!(
            "Multi-Agent task did not resolve to {expected_workload}"
        ));
    }
    let required = [
        "agent.spawn",
        "agent.send",
        "agent.wait",
        "agent.cancel",
        "agent.collect",
        "enterprise.download_attachment",
    ];
    if let Some(missing) = required.iter().find(|name| !has_tool(request, name)) {
        return failure(format!(
            "Unified {expected_workload} ToolPlan is missing {missing}"
        ));
    }
    if !completed(request, "agent.spawn") {
        return tool_call(
            "desktop-e2e-agent-spawn",
            "agent.spawn",
            serde_json::json!({
                "title": "Desktop E2E child",
                "prompt": "[desktop-e2e:multi-agent-child] finish without tools",
                "permissionProfile": "read_only",
                "maxModelRequests": 4,
                "maxToolCalls": 2,
                "toolAllowlist": []
            }),
        );
    }
    if !completed(request, "agent.wait") {
        return tool_call(
            "desktop-e2e-agent-wait",
            "agent.wait",
            serde_json::json!({"timeoutMs": 10_000}),
        );
    }
    if !completed(request, "agent.collect") {
        return tool_call(
            "desktop-e2e-agent-collect",
            "agent.collect",
            serde_json::json!({}),
        );
    }
    let collected = tool_content(request, "agent.collect").unwrap_or_default();
    if !collected.contains("desktop-e2e") && !collected.contains("Desktop E2E") {
        return failure("Multi-Agent collect did not return the child Task".into());
    }
    text_response(&format!(
        "Desktop E2E {expected_workload} unified ToolPlan completed; Multi-Agent and enterprise attachment capabilities were profile-independent."
    ))
}

fn git_forge_response(request: &ModelRequest) -> Response {
    for name in [
        "git.remotes",
        "git.push",
        "forge.change.query",
        "forge.change.mutate",
    ] {
        if !has_tool(request, name) {
            return failure(format!("Coding ToolPlan is missing {name}"));
        }
    }
    if !completed(request, "git.remotes") {
        return tool_call(
            "desktop-e2e-git-remotes",
            "git.remotes",
            serde_json::json!({}),
        );
    }
    let Some(origin) = remote_record(request, "origin") else {
        return failure("git.remotes did not return the redacted origin fixture".into());
    };
    let Some(local) = remote_record(request, "local-e2e") else {
        return failure("git.remotes did not return the local push fixture".into());
    };
    if origin.get("displayUrl").and_then(serde_json::Value::as_str)
        != Some("https://github.com/hachimi/desktop-e2e.git")
    {
        return failure("git.remotes exposed an unexpected origin URL".into());
    }
    let Some(oid) = marker_value(&request.messages, "oid=") else {
        return failure("Git push fixture OID is missing".into());
    };
    let pushes = tool_contents(request, "git.push");
    if pushes.is_empty() {
        let Some(remote_hash) = local
            .get("remoteUrlHash")
            .and_then(serde_json::Value::as_str)
        else {
            return failure("local Remote hash is missing".into());
        };
        return tool_call(
            "desktop-e2e-git-push-success",
            "git.push",
            serde_json::json!({
                "remoteName": "local-e2e",
                "expectedRemoteUrlHash": remote_hash,
                "sourceRef": "HEAD",
                "targetRef": "refs/heads/agent-success",
                "expectedCommitOid": oid
            }),
        );
    }
    if !pushes[0].contains("git_push_confirmed") {
        return failure("Git push did not return a confirmed receipt".into());
    }
    if pushes.len() == 1 {
        return tool_call(
            "desktop-e2e-git-push-drift",
            "git.push",
            serde_json::json!({
                "remoteName": "origin",
                "expectedRemoteUrlHash": "0".repeat(64),
                "sourceRef": "HEAD",
                "targetRef": "refs/heads/desktop-e2e",
                "expectedCommitOid": oid
            }),
        );
    }
    if !pushes[1].contains("forge_remote_drift") {
        return failure("Git push did not stop at the expected Remote hash fence".into());
    }
    text_response(
        "Desktop E2E successful push and drift fencing completed with exact approval and receipt checks.",
    )
}

fn approval_recovery_response(request: &ModelRequest) -> Response {
    if !has_tool(request, "browser_start") {
        return failure("Approval recovery ToolPlan is missing browser_start".into());
    }
    if !completed(request, "browser_start") {
        let Some(initial_url) = marker_value(&request.messages, "url=") else {
            return failure("Approval recovery Browser URL is missing".into());
        };
        return tool_call(
            "desktop-e2e-approval-browser",
            "browser_start",
            serde_json::json!({
                "initialUrl": initial_url,
                "surface": "embedded"
            }),
        );
    }
    text_response("Desktop E2E approval recovery Browser action completed.")
}

fn forge_lifecycle_response(request: &ModelRequest) -> Response {
    if let Some(response) = require_forge_tools_and_remotes(request) {
        return response;
    }
    let Some(remote) = remote_record(request, "forge-e2e") else {
        return failure("loopback Forge Remote is missing".into());
    };
    let Some(remote_hash) = remote
        .get("remoteUrlHash")
        .and_then(serde_json::Value::as_str)
    else {
        return failure("loopback Forge Remote hash is missing".into());
    };
    let Some(oid) = marker_value(&request.messages, "oid=") else {
        return failure("Forge fixture OID is missing".into());
    };
    if !completed(request, "forge.change.query") {
        return forge_query_call("desktop-e2e-forge-query", remote_hash, 1);
    }
    let Some(queried) = tool_json(request, "forge.change.query") else {
        return failure("Forge query did not return structured state".into());
    };
    let mutations = tool_json_values(request, "forge.change.mutate");
    let previous_revision = mutations
        .last()
        .unwrap_or(&queried)
        .get("revision")
        .and_then(serde_json::Value::as_str);
    let Some(previous_revision) = previous_revision else {
        return failure("Forge mutation revision is missing".into());
    };
    match mutations.len() {
        0 => forge_mutation_call(
            "desktop-e2e-forge-update",
            remote_hash,
            serde_json::json!({
                "kind": "update", "number": 1, "title": "Desktop E2E updated",
                "body": "updated body", "source_ref": "feature-1", "target_ref": "main"
            }),
            Some(previous_revision),
            &oid,
        ),
        1 => forge_mutation_call(
            "desktop-e2e-forge-close",
            remote_hash,
            serde_json::json!({"kind": "close", "number": 1}),
            Some(previous_revision),
            &oid,
        ),
        2 => forge_mutation_call(
            "desktop-e2e-forge-create",
            remote_hash,
            serde_json::json!({
                "kind": "create", "title": "Desktop E2E created", "body": "created body",
                "source_ref": "feature-2", "target_ref": "main"
            }),
            None,
            &oid,
        ),
        3 => {
            let Some(number) = mutations[2]
                .get("number")
                .and_then(serde_json::Value::as_u64)
            else {
                return failure("created Forge change number is missing".into());
            };
            forge_mutation_call(
                "desktop-e2e-forge-merge",
                remote_hash,
                serde_json::json!({
                    "kind": "merge", "number": number,
                    "merge_title": "Desktop E2E merge", "merge_message": "merge body"
                }),
                Some(previous_revision),
                &oid,
            )
        }
        4 => text_response(
            "Desktop E2E Forge query/create/update/close/merge completed through Agent tools.",
        ),
        _ => failure("Forge lifecycle dispatched an unexpected duplicate mutation".into()),
    }
}

fn forge_unknown_response(request: &ModelRequest) -> Response {
    if let Some(response) = require_forge_tools_and_remotes(request) {
        return response;
    }
    let Some(remote_hash) = remote_hash(request, "forge-e2e") else {
        return failure("loopback Forge Remote hash is missing".into());
    };
    let Some(oid) = marker_value(&request.messages, "oid=") else {
        return failure("Forge fixture OID is missing".into());
    };
    if !completed(request, "forge.change.mutate") {
        return forge_mutation_call(
            "desktop-e2e-forge-unknown-create",
            &remote_hash,
            serde_json::json!({
                "kind": "create", "title": "Unknown response create",
                "body": "reconcile without replay", "source_ref": "feature-unknown",
                "target_ref": "main"
            }),
            None,
            &oid,
        );
    }
    let result = tool_content(request, "forge.change.mutate").unwrap_or_default();
    if !result.contains("Unknown response create") {
        return failure("unknown Forge mutation was not reconciled to its remote record".into());
    }
    text_response("Desktop E2E unknown Forge mutation reconciled without replay.")
}

fn forge_duplicate_response(request: &ModelRequest) -> Response {
    if let Some(response) = require_forge_tools_and_remotes(request) {
        return response;
    }
    let Some(remote_hash) = remote_hash(request, "forge-e2e") else {
        return failure("loopback Forge Remote hash is missing".into());
    };
    let Some(oid) = marker_value(&request.messages, "oid=") else {
        return failure("Forge fixture OID is missing".into());
    };
    let mutations = tool_json_values(request, "forge.change.mutate");
    if mutations.len() < 2 {
        return forge_mutation_call(
            "desktop-e2e-forge-duplicate-create",
            &remote_hash,
            serde_json::json!({
                "kind": "create", "title": "Duplicate invocation create",
                "body": "must dispatch once", "source_ref": "feature-duplicate",
                "target_ref": "main"
            }),
            None,
            &oid,
        );
    }
    if mutations[0] != mutations[1] {
        return failure("duplicate Agent invocation did not reuse the persisted result".into());
    }
    text_response(
        "Desktop E2E duplicate Agent invocation reused one unified side effect and one Forge domain receipt.",
    )
}

fn forge_revision_response(request: &ModelRequest) -> Response {
    if let Some(response) = require_forge_tools_and_remotes(request) {
        return response;
    }
    let Some(remote_hash) = remote_hash(request, "forge-e2e") else {
        return failure("loopback Forge Remote hash is missing".into());
    };
    let Some(oid) = marker_value(&request.messages, "oid=") else {
        return failure("Forge fixture OID is missing".into());
    };
    if !completed(request, "forge.change.query") {
        return forge_query_call("desktop-e2e-forge-revision-query", &remote_hash, 1);
    }
    if !completed(request, "forge.change.mutate") {
        let Some(revision) = tool_json(request, "forge.change.query")
            .and_then(|value| value.get("revision").cloned())
            .and_then(|value| value.as_str().map(str::to_owned))
        else {
            return failure("Forge query revision is missing".into());
        };
        return forge_mutation_call(
            "desktop-e2e-forge-stale-update",
            &remote_hash,
            serde_json::json!({
                "kind": "update", "number": 1, "title": "stale update",
                "body": "must fail", "source_ref": "feature-1", "target_ref": "main"
            }),
            Some(&revision),
            &oid,
        );
    }
    if !tool_content(request, "forge.change.mutate")
        .is_some_and(|content| content.contains("forge_revision_conflict"))
    {
        return failure("Forge concurrent revision was not fenced".into());
    }
    text_response("Desktop E2E forge_revision_conflict stopped concurrent mutation dispatch.")
}

fn forge_credential_response(request: &ModelRequest) -> Response {
    if let Some(response) = require_forge_tools_and_remotes(request) {
        return response;
    }
    let Some(remote_hash) = remote_hash(request, "forge-e2e") else {
        return failure("loopback Forge Remote hash is missing".into());
    };
    if !completed(request, "forge.change.query") {
        return forge_query_call("desktop-e2e-forge-revoked-query", &remote_hash, 1);
    }
    if !tool_content(request, "forge.change.query")
        .is_some_and(|content| content.contains("forge_credential_failed"))
    {
        return failure("revoked Forge credential did not fail closed".into());
    }
    text_response("Desktop E2E forge_credential_failed confirmed revoked credentials fail closed.")
}

fn enterprise_attachment_response(request: &ModelRequest) -> Response {
    if !has_tool(request, "enterprise.download_attachment") {
        return failure("Unified ToolPlan is missing enterprise.download_attachment".into());
    }
    for name in ["agent.spawn", "agent.wait", "agent.collect"] {
        if !has_tool(request, name) {
            return failure(format!("General ToolPlan is missing {name}"));
        }
    }
    text_response(
        "Desktop E2E General unified ToolPlan exposed profile-independent Multi-Agent and enterprise attachment capabilities.",
    )
}

fn require_forge_tools_and_remotes(request: &ModelRequest) -> Option<Response> {
    for name in [
        "git.remotes",
        "git.push",
        "forge.change.query",
        "forge.change.mutate",
    ] {
        if !has_tool(request, name) {
            return Some(failure(format!("Coding ToolPlan is missing {name}")));
        }
    }
    (!completed(request, "git.remotes")).then(|| {
        tool_call(
            "desktop-e2e-forge-remotes",
            "git.remotes",
            serde_json::json!({}),
        )
    })
}

fn forge_query_call(id: &str, remote_hash: &str, number: u64) -> Response {
    tool_call(
        id,
        "forge.change.query",
        serde_json::json!({
            "remoteName": "forge-e2e",
            "expectedRemoteUrlHash": remote_hash,
            "number": number
        }),
    )
}

fn forge_mutation_call(
    id: &str,
    remote_hash: &str,
    mutation: serde_json::Value,
    expected_revision: Option<&str>,
    oid: &str,
) -> Response {
    tool_call(
        id,
        "forge.change.mutate",
        serde_json::json!({
            "remoteName": "forge-e2e",
            "expectedRemoteUrlHash": remote_hash,
            "mutation": mutation,
            "expectedRevision": expected_revision,
            "expectedCommitOid": oid
        }),
    )
}

fn remote_record(request: &ModelRequest, name: &str) -> Option<serde_json::Value> {
    tool_json(request, "git.remotes")?
        .as_array()?
        .iter()
        .find(|remote| remote.get("name").and_then(serde_json::Value::as_str) == Some(name))
        .cloned()
}

fn remote_hash(request: &ModelRequest, name: &str) -> Option<String> {
    remote_record(request, name)?
        .get("remoteUrlHash")?
        .as_str()
        .map(str::to_owned)
}

fn marker_value(messages: &[ModelMessage], prefix: &str) -> Option<String> {
    messages
        .iter()
        .filter(|message| message.role == ModelRole::User)
        .flat_map(|message| message.content.split_whitespace())
        .find_map(|part| part.strip_prefix(prefix).map(str::to_owned))
}

fn has_marker(messages: &[ModelMessage], marker: &str) -> bool {
    messages
        .iter()
        .any(|message| message.role == ModelRole::User && message.content.contains(marker))
}

fn has_tool(request: &ModelRequest, name: &str) -> bool {
    request.tools.iter().any(|tool| tool.name == name)
}

fn completed(request: &ModelRequest, name: &str) -> bool {
    request
        .messages
        .iter()
        .any(|message| message.role == ModelRole::Tool && message.name.as_deref() == Some(name))
}

fn tool_content<'a>(request: &'a ModelRequest, name: &str) -> Option<&'a str> {
    request
        .messages
        .iter()
        .rev()
        .find(|message| message.role == ModelRole::Tool && message.name.as_deref() == Some(name))
        .map(|message| message.content.as_str())
}

fn tool_json(request: &ModelRequest, name: &str) -> Option<serde_json::Value> {
    parse_tool_json(tool_content(request, name)?)
}

fn tool_contents<'a>(request: &'a ModelRequest, name: &str) -> Vec<&'a str> {
    request
        .messages
        .iter()
        .filter(|message| message.role == ModelRole::Tool && message.name.as_deref() == Some(name))
        .map(|message| message.content.as_str())
        .collect()
}

fn tool_json_values(request: &ModelRequest, name: &str) -> Vec<serde_json::Value> {
    tool_contents(request, name)
        .into_iter()
        .filter_map(parse_tool_json)
        .collect()
}

fn parse_tool_json(content: &str) -> Option<serde_json::Value> {
    serde_json::from_str(content).ok().or_else(|| {
        let (_, payload) = content.split_once('\n')?;
        serde_json::from_str(payload).ok()
    })
}

fn tool_call(id: &str, name: &str, arguments: serde_json::Value) -> Response {
    vec![
        Ok(ModelEvent::ToolCallCompleted {
            call: ModelToolCall {
                id: ToolCallId::from(id),
                name: name.into(),
                arguments,
            },
        }),
        Ok(ModelEvent::Completed {
            finish_reason: ModelFinishReason::ToolCalls,
        }),
    ]
}

fn text_response(text: &str) -> Response {
    vec![
        Ok(ModelEvent::TextDelta { delta: text.into() }),
        Ok(ModelEvent::Usage {
            usage: TokenUsage {
                input_tokens: 48,
                output_tokens: 16,
            },
        }),
        Ok(ModelEvent::Completed {
            finish_reason: ModelFinishReason::Stop,
        }),
    ]
}

fn failure(message: String) -> Response {
    vec![Err(ModelRuntimeError::Provider(message))]
}
