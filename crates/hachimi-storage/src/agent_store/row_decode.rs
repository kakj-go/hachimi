use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Serialize, de::DeserializeOwned};
use sqlx::Row;

use super::*;

pub(super) fn run_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<RunRecord, AgentStoreError> {
    let status_value = row.get::<String, _>("status");
    let status =
        RunStatus::parse(&status_value).ok_or_else(|| AgentStoreError::InvalidPersistedValue {
            kind: "run status",
            value: status_value,
        })?;
    Ok(RunRecord {
        id: RunId::new(row.get::<String, _>("id")),
        session_id: SessionId::new(row.get::<String, _>("session_id")),
        status,
        purpose: enum_from_db(row.get("purpose"), "run purpose")?,
        origin: serde_json::from_str(row.get("origin_json"))?,
        generation: u64::try_from(row.get::<i64, _>("generation")).unwrap_or_default(),
        configuration: serde_json::from_str(row.get("configuration_json"))?,
        requested_capabilities: serde_json::from_str(
            row.get::<String, _>("requested_capabilities_json").as_str(),
        )?,
        negotiated_capabilities: serde_json::from_str(
            row.get::<String, _>("negotiated_capabilities_json")
                .as_str(),
        )?,
        provider_capability_probe: serde_json::from_str(
            row.get::<String, _>("provider_capability_probe_json")
                .as_str(),
        )?,
        capability_degradations: serde_json::from_str::<Vec<CapabilityDegradation>>(
            row.get::<String, _>("capability_degradations_json")
                .as_str(),
        )?,
        failure_code: row.get("failure_code"),
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

pub(super) fn transcript_item_from_row(
    row: &sqlx::sqlite::SqliteRow,
    session_id: &SessionId,
) -> Result<TranscriptItem, AgentStoreError> {
    let kind_value = row.get::<String, _>("kind");
    let kind = enum_from_db::<TranscriptItemKind>(&kind_value, "transcript kind")?;
    let payload = serde_json::from_str(row.get("payload_json"))?;
    let status_value: String = row.get("status");
    let status =
        ItemStatus::parse(&status_value).ok_or_else(|| AgentStoreError::InvalidPersistedValue {
            kind: "item status",
            value: status_value,
        })?;
    Ok(TranscriptItem {
        id: hachimi_protocol::ItemId::new(row.get::<String, _>("id")),
        session_id: session_id.clone(),
        run_id: row.get::<Option<String>, _>("run_id").map(RunId::new),
        sequence: u64::try_from(row.get::<i64, _>("sequence")).unwrap_or_default(),
        kind,
        status,
        payload,
        relations: serde_json::from_str(row.get("relations_json"))?,
        created_at_ms: row.get("created_at_ms"),
    })
}

pub(super) fn project_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<ProjectRecord, AgentStoreError> {
    Ok(ProjectRecord {
        id: hachimi_protocol::ProjectId::new(row.get::<String, _>("id")),
        display_name: row.get("display_name"),
        root_path: row.get("root_path"),
        git_root: row.get("git_root"),
        trusted: row.get("trusted"),
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

pub(super) fn attachment_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<AttachmentRecord, AgentStoreError> {
    Ok(AttachmentRecord {
        id: AttachmentId::new(row.get::<String, _>("id")),
        content_hash: row.get("content_hash"),
        original_name: row.get("original_name"),
        mime_type: row.get("mime_type"),
        byte_size: u64::try_from(row.get::<i64, _>("byte_size")).unwrap_or_default(),
        created_at_ms: row.get("created_at_ms"),
    })
}

pub(super) fn proposed_plan_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<ProposedPlan, AgentStoreError> {
    let status_value: String = row.get("status");
    let status = ProposedPlanStatus::parse(&status_value).ok_or_else(|| {
        AgentStoreError::InvalidPersistedValue {
            kind: "proposed plan status",
            value: status_value,
        }
    })?;
    Ok(ProposedPlan {
        id: PlanId::new(row.get::<String, _>("id")),
        session_id: SessionId::new(row.get::<String, _>("session_id")),
        run_id: RunId::new(row.get::<String, _>("run_id")),
        revision: u32::try_from(row.get::<i64, _>("revision")).unwrap_or_default(),
        goal: row.get("goal"),
        assumptions: serde_json::from_str(row.get("assumptions_json"))?,
        steps: serde_json::from_str(row.get("steps_json"))?,
        affected_resources: serde_json::from_str(row.get("affected_resources_json"))?,
        verification: serde_json::from_str(row.get("verification_json"))?,
        risks: serde_json::from_str(row.get("risks_json"))?,
        open_questions: serde_json::from_str(row.get("open_questions_json"))?,
        content_markdown: row.get("content_markdown"),
        status,
        accepted_run_id: row
            .get::<Option<String>, _>("accepted_run_id")
            .map(RunId::new),
        created_at_ms: row.get("created_at_ms"),
        accepted_at_ms: row.get("accepted_at_ms"),
    })
}

pub(super) fn compaction_checkpoint_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<CompactionCheckpoint, AgentStoreError> {
    Ok(CompactionCheckpoint {
        id: CompactionCheckpointId::new(row.get::<String, _>("id")),
        session_id: SessionId::new(row.get::<String, _>("session_id")),
        run_id: row.get::<Option<String>, _>("run_id").map(RunId::new),
        previous_checkpoint_id: row
            .get::<Option<String>, _>("previous_checkpoint_id")
            .map(CompactionCheckpointId::new),
        covered_through_sequence: u64::try_from(row.get::<i64, _>("covered_through_sequence"))
            .unwrap_or_default(),
        reason: enum_from_db::<CompactionReason>(row.get("reason"), "compaction reason")?,
        lifecycle: CompactionLifecycle {
            trigger: enum_from_db(row.get("trigger"), "compaction trigger")?,
            phase: enum_from_db(row.get("phase"), "compaction phase")?,
            implementation: enum_from_db(row.get("implementation"), "compaction implementation")?,
            token_snapshot: row
                .get::<Option<String>, _>("token_snapshot_json")
                .map(|value| serde_json::from_str(&value))
                .transpose()?,
            trimmed_history_groups: u32::try_from(row.get::<i64, _>("trimmed_history_groups"))
                .unwrap_or(u32::MAX),
            summary_source: enum_from_db(row.get("summary_source"), "compaction summary source")?,
            provider_endpoint_id: row
                .get::<Option<String>, _>("provider_endpoint_id")
                .map(hachimi_protocol::ProviderEndpointId::new),
            provider_account_id: row
                .get::<Option<String>, _>("provider_account_id")
                .map(hachimi_protocol::ProviderAccountId::new),
            capability_revision: row.get("capability_revision"),
            fallback_reason: row.get("fallback_reason"),
        },
        summary: serde_json::from_str(row.get("summary_json"))?,
        quality: serde_json::from_str(row.get("quality_json"))?,
        created_at_ms: row.get("created_at_ms"),
    })
}

pub(super) fn artifact_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<ArtifactRecord, AgentStoreError> {
    let kind_value: String = row.get("kind");
    let kind =
        ArtifactKind::parse(&kind_value).ok_or_else(|| AgentStoreError::InvalidPersistedValue {
            kind: "artifact kind",
            value: kind_value,
        })?;
    Ok(ArtifactRecord {
        id: ArtifactId::new(row.get::<String, _>("id")),
        run_id: row.get::<Option<String>, _>("run_id").map(RunId::new),
        kind,
        display_name: row.get("display_name"),
        content_hash: row.get("content_hash"),
        metadata: serde_json::from_str(row.get("metadata_json"))?,
        created_at_ms: row.get("created_at_ms"),
    })
}

pub(super) fn checkout_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<CheckoutRecord, AgentStoreError> {
    Ok(CheckoutRecord {
        id: hachimi_protocol::CheckoutId::new(row.get::<String, _>("id")),
        project_id: hachimi_protocol::ProjectId::new(row.get::<String, _>("project_id")),
        kind: enum_from_db(row.get("kind"), "checkout kind")?,
        path: row.get("path"),
        base_revision: row.get("base_revision"),
        head_revision: row.get("head_revision"),
        status: enum_from_db(row.get("status"), "checkout status")?,
        pinned: row.get("pinned"),
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

pub(super) fn session_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<SessionRecord, AgentStoreError> {
    Ok(SessionRecord {
        id: SessionId::new(row.get::<String, _>("id")),
        context: serde_json::from_str(row.get("context_json"))?,
        entry_profile: enum_from_db(row.get("entry_profile"), "entry profile")?,
        title: row.get("title"),
        archived: row.get("archived"),
        pinned: row.get("pinned"),
        parent_session_id: row
            .get::<Option<String>, _>("parent_session_id")
            .map(SessionId::new),
        source_run_id: row
            .get::<Option<String>, _>("source_run_id")
            .map(RunId::new),
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

pub(super) fn approval_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<ApprovalRequestRecord, AgentStoreError> {
    let status_value: String = row.get("status");
    let status = ApprovalStatus::parse(&status_value).ok_or_else(|| {
        AgentStoreError::InvalidPersistedValue {
            kind: "approval status",
            value: status_value,
        }
    })?;
    Ok(ApprovalRequestRecord {
        id: ApprovalId::new(row.get::<String, _>("id")),
        session_id: SessionId::new(row.get::<String, _>("session_id")),
        run_id: RunId::new(row.get::<String, _>("run_id")),
        tool_call_id: ToolCallId::new(row.get::<String, _>("tool_call_id")),
        run_generation: u64::try_from(row.get::<i64, _>("run_generation")).unwrap_or_default(),
        status,
        action: row.get("action"),
        resource: row.get("resource"),
        parameter_hash: row.get("parameter_hash"),
        risk_summary: row.get("risk_summary"),
        target_host: row.get("target_host"),
        required_scopes: serde_json::from_str(row.get("required_scopes_json"))?,
        grant_scope: enum_from_db(row.get("grant_scope"), "approval grant scope")?,
        uses_remaining: u32::try_from(row.get::<i64, _>("uses_remaining")).unwrap_or_default(),
        requester_principal: row.get("requester_principal"),
        resolved_by: row.get("resolved_by"),
        expires_at_ms: row.get("expires_at_ms"),
        created_at_ms: row.get("created_at_ms"),
        resolved_at_ms: row.get("resolved_at_ms"),
    })
}

pub(super) fn mcp_server_from_row(
    row: &sqlx::sqlite::SqliteRow,
    headers: Vec<McpHeaderView>,
) -> Result<McpServerRecord, AgentStoreError> {
    let transport_kind: String = row.get("transport_kind");
    let transport = match transport_kind.as_str() {
        "stdio" => McpServerTransport::Stdio {
            command: row.get("command"),
            args: serde_json::from_str(row.get("args_json"))?,
            cwd: row.get("cwd"),
        },
        "streamable_http" => McpServerTransport::StreamableHttp {
            url: row.get::<Option<String>, _>("url").ok_or_else(|| {
                AgentStoreError::InvalidPersistedValue {
                    kind: "MCP streamable HTTP URL",
                    value: "missing".into(),
                }
            })?,
        },
        _ => {
            return Err(AgentStoreError::InvalidPersistedValue {
                kind: "MCP transport",
                value: transport_kind,
            });
        }
    };
    Ok(McpServerRecord {
        id: McpServerId::new(row.get::<String, _>("id")),
        display_name: row.get("display_name"),
        enabled: row.get("enabled"),
        transport,
        headers,
        read_only_tools: serde_json::from_str(row.get("read_only_tools_json"))?,
        startup_timeout_ms: u64::try_from(row.get::<i64, _>("startup_timeout_ms"))
            .unwrap_or_default(),
        request_timeout_ms: u64::try_from(row.get::<i64, _>("request_timeout_ms"))
            .unwrap_or_default(),
        max_message_bytes: u64::try_from(row.get::<i64, _>("max_message_bytes"))
            .unwrap_or_default(),
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

pub(super) fn mcp_header_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<McpHeaderView, AgentStoreError> {
    let secret: bool = row.get("secret");
    let configured: bool = row.get("configured");
    let value: Option<String> = row.get("value");
    let credential_reference: Option<String> = row.get("credential_reference");
    if secret && (value.is_some() || credential_reference.is_none()) {
        return Err(AgentStoreError::InvalidPersistedValue {
            kind: "MCP secret header",
            value: "invalid secret storage shape".into(),
        });
    }
    Ok(McpHeaderView {
        name: row.get("name"),
        value,
        secret,
        configured,
        credential_reference,
    })
}

pub(super) fn mcp_server_health_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<McpServerHealthRecord, AgentStoreError> {
    let state_value: String = row.get("state");
    let state = McpServerHealthState::parse(&state_value).ok_or_else(|| {
        AgentStoreError::InvalidPersistedValue {
            kind: "MCP server health state",
            value: state_value,
        }
    })?;
    Ok(McpServerHealthRecord {
        server_id: McpServerId::new(row.get::<String, _>("server_id")),
        state,
        server_name: row.get("server_name"),
        server_version: row.get("server_version"),
        protocol_version: row.get("protocol_version"),
        tool_count: u32::try_from(row.get::<i64, _>("tool_count")).unwrap_or_default(),
        error_code: row.get("error_code"),
        failure_count: u32::try_from(row.get::<i64, _>("failure_count")).unwrap_or(u32::MAX),
        next_retry_at_ms: row.get("next_retry_at_ms"),
        checked_at_ms: row.get("checked_at_ms"),
    })
}

pub(super) fn validate_mcp_server(server: &McpServerRecord) -> Result<(), AgentStoreError> {
    let valid_id = !server.id.as_str().is_empty()
        && server.id.as_str().len() <= 64
        && server
            .id
            .as_str()
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if !valid_id {
        return Err(AgentStoreError::InvalidMcpServerConfiguration("server ID"));
    }
    if server.display_name.trim().is_empty() || server.display_name.len() > 128 {
        return Err(AgentStoreError::InvalidMcpServerConfiguration(
            "display name",
        ));
    }
    match &server.transport {
        McpServerTransport::Stdio { command, args, cwd } => {
            if command.trim().is_empty() || command.len() > 4_096 || command.contains('\0') {
                return Err(AgentStoreError::InvalidMcpServerConfiguration("command"));
            }
            if args.len() > 128
                || args
                    .iter()
                    .any(|argument| argument.len() > 8_192 || argument.contains('\0'))
            {
                return Err(AgentStoreError::InvalidMcpServerConfiguration("arguments"));
            }
            if cwd
                .as_ref()
                .is_some_and(|cwd| cwd.len() > 4_096 || cwd.contains('\0'))
            {
                return Err(AgentStoreError::InvalidMcpServerConfiguration(
                    "working directory",
                ));
            }
        }
        McpServerTransport::StreamableHttp { url } => {
            let parsed = url::Url::parse(url).map_err(|_| {
                AgentStoreError::InvalidMcpServerConfiguration("streamable HTTP URL")
            })?;
            let loopback_http = parsed.scheme() == "http"
                && parsed
                    .host_str()
                    .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1"));
            if parsed.scheme() != "https" && !loopback_http {
                return Err(AgentStoreError::InvalidMcpServerConfiguration(
                    "streamable HTTP URL",
                ));
            }
            if parsed.username() != "" || parsed.password().is_some() || parsed.fragment().is_some()
            {
                return Err(AgentStoreError::InvalidMcpServerConfiguration(
                    "streamable HTTP URL",
                ));
            }
        }
    }
    if server.headers.len() > 64
        || server.headers.iter().any(|header| {
            header.name.is_empty()
                || header.name.len() > 128
                || !header
                    .name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
                || header
                    .value
                    .as_ref()
                    .is_some_and(|value| value.len() > 8_192 || value.contains(['\r', '\n', '\0']))
                || (header.secret
                    && (header.value.is_some() || header.credential_reference.is_none()))
        })
    {
        return Err(AgentStoreError::InvalidMcpServerConfiguration("headers"));
    }
    if server.read_only_tools.len() > 512
        || server.read_only_tools.iter().any(|name| {
            name.is_empty()
                || name.len() > 128
                || !name.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/')
                })
        })
    {
        return Err(AgentStoreError::InvalidMcpServerConfiguration(
            "read-only tool list",
        ));
    }
    if server.startup_timeout_ms == 0
        || server.request_timeout_ms == 0
        || !(4 * 1024..=16 * 1024 * 1024).contains(&server.max_message_bytes)
    {
        return Err(AgentStoreError::InvalidMcpServerConfiguration(
            "runtime limits",
        ));
    }
    Ok(())
}

pub(super) fn session_context_kind(
    context: &hachimi_protocol::SessionContextBinding,
) -> &'static str {
    match context {
        hachimi_protocol::SessionContextBinding::General => "general",
        hachimi_protocol::SessionContextBinding::Project { .. } => "project",
        hachimi_protocol::SessionContextBinding::Avatar { .. } => "avatar",
    }
}

pub(super) fn transcript_kind_db(kind: TranscriptItemKind) -> &'static str {
    match kind {
        TranscriptItemKind::User => "user",
        TranscriptItemKind::Assistant => "assistant",
        TranscriptItemKind::ToolExecution => "tool_execution",
        TranscriptItemKind::Plan => "plan",
        TranscriptItemKind::Approval => "approval",
        TranscriptItemKind::SystemContext => "system_context",
        TranscriptItemKind::Reasoning => "reasoning",
        TranscriptItemKind::UserInputRequest => "user_input_request",
        TranscriptItemKind::CommandExecution => "command_execution",
        TranscriptItemKind::FileChange => "file_change",
        TranscriptItemKind::McpCall => "mcp_call",
        TranscriptItemKind::DynamicToolCall => "dynamic_tool_call",
        TranscriptItemKind::CollabToolCall => "collab_tool_call",
        TranscriptItemKind::ContextCompaction => "context_compaction",
        TranscriptItemKind::Review => "review",
    }
}

pub(super) fn enum_to_db(value: &impl Serialize) -> Result<String, AgentStoreError> {
    match serde_json::to_value(value)? {
        Value::String(value) => Ok(value),
        _ => Err(AgentStoreError::InvalidPersistedValue {
            kind: "enum",
            value: "non-string serialization".into(),
        }),
    }
}

pub(super) fn enum_from_db<T: DeserializeOwned>(
    value: &str,
    kind: &'static str,
) -> Result<T, AgentStoreError> {
    serde_json::from_value(Value::String(value.into())).map_err(|_| {
        AgentStoreError::InvalidPersistedValue {
            kind,
            value: value.into(),
        }
    })
}

pub(super) fn now_ms() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}
