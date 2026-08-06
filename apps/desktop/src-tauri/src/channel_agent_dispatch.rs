//! AppServer-owned Channel ingress dispatch into the unified Agent Runtime.

use hachimi_agent::{
    AgentRunCreateRequest, AgentRunLaunchRequest, AgentRunLauncher, AgentRunPriority,
    AgentRunRequest,
};
use hachimi_protocol::{
    ApprovalPolicy, AttachmentId, AuthorityMode, BehaviorMode, CONTROL_PROTOCOL_VERSION,
    ChannelMessagePart, ClientId, EnterpriseAttachmentDownloadRequest, EntryProfile,
    HostRevisionSnapshot, ItemPayload, ItemStatus, McpToolSelection, MutationContext,
    ProviderCapabilities, RequestId, RunBudget, RunOrigin, RunPurpose, RunStatus,
    SessionContextBinding, SessionRecord, SkillId, StructuredOutputMode, TranscriptItemKind,
    VerifiedChannelMessage, WorkloadKind,
};
use sha2::{Digest as _, Sha256};
use tauri::{AppHandle, Manager};

use crate::{DesktopState, epoch_millis};

pub(super) async fn process_ingress(
    app: &AppHandle,
    gateway: &hachimi_gateway::GatewayHost,
    principal: &str,
    message: &VerifiedChannelMessage,
) -> Result<hachimi_protocol::IngressReceipt, Box<dyn std::error::Error + Send + Sync>> {
    let state = app.state::<DesktopState>();
    let store = state.agent_store.clone();
    if let Some(command) = hachimi_gateway::parse_control_command(message)? {
        return process_control_command(&store, gateway, message, command).await;
    }
    let channel_grant = gateway.ingress_grant_snapshot(&message.event_key).await?;
    let binding = gateway.resolve_binding(message).await?;
    let existing_session = match binding.session_id.as_ref() {
        Some(session_id) => Some(
            store
                .get_session(session_id)
                .await?
                .ok_or("Channel Session disappeared")?,
        ),
        None => None,
    };
    let context = existing_session.as_ref().map_or_else(
        || SessionContextBinding::Workspace {
            workspace_id: hachimi_protocol::WorkspaceId::random(),
        },
        |session| session.context.clone(),
    );
    let authorization_revision = binding
        .authorization
        .as_ref()
        .map_or(1, |authorization| authorization.revision);
    let authorization_id = binding
        .authorization
        .as_ref()
        .map(|authorization| authorization.id.clone());
    let mut permission_policy = channel_grant.permission_policy.clone();
    permission_policy.revision = authorization_revision;
    let runtime_grants = prepare_runtime_grants(&state, &channel_grant).await;
    let settings = state.settings.read().llm.clone();
    let create_request = AgentRunCreateRequest {
        principal: principal.to_owned(),
        idempotency_key: format!(
            "channel:{}:{}:{}",
            message.event_key.provider_id,
            message.event_key.account_id,
            message.event_key.external_message_id
        ),
        context,
        origin: RunOrigin::Channel {
            channel: message.address.provider_id.clone(),
            account: message.address.account_id.clone(),
            peer: message.address.chat_id.clone(),
            thread: message.address.topic_id.clone().unwrap_or_default(),
            message_id: message.message_id(),
        },
        title: format!(
            "{} · {}",
            message.address.provider_id,
            message
                .actor
                .display_name
                .as_deref()
                .unwrap_or(&message.actor.external_id)
        )
        .chars()
        .take(200)
        .collect(),
        prompt: channel_prompt(message),
        attachment_ids: Vec::new(),
        parent_session_id: None,
        source_run_id: None,
        purpose: RunPurpose::Task,
        model_snapshot: settings.clone(),
        entry_profile: EntryProfile::Workbench,
        workload_override: Some(WorkloadKind::General),
        behavior_mode: BehaviorMode::Default,
        execution_target: None,
        approval_policy: ApprovalPolicy::NeverPrompt,
        permission_profile: permission_policy.level,
        budget: RunBudget::default(),
        requested_capabilities: requested_capabilities(&settings),
        created_at_ms: now_ms(),
    };
    let launched = AgentRunLauncher::new(store.clone())
        .launch_channel(
            AgentRunLaunchRequest {
                create: create_request,
                policy: permission_policy,
                authority_mode: AuthorityMode::Unattended,
            },
            hachimi_storage::ChannelRunBindingInput {
                event_key: message.event_key.clone(),
                binding_key_hash: binding.binding_key_hash,
                binding_key_json: binding.binding_key_json,
                account_id: message.address.account_id.clone(),
                authorization_id,
                authorization_revision,
                identity_group_id: binding.identity_group_id,
                timestamp_ms: now_ms(),
            },
        )
        .await?;
    let session = launched.created.session;
    let run = launched.created.run;
    let authority = launched.authority;
    let (mcp_tool_allowlist, host_revision_snapshot) = match runtime_grants {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(run_id = %run.id, %error, "Channel grant snapshot is unavailable or drifted");
            store
                .transition_run(
                    &run.id,
                    RunStatus::Interrupted,
                    Some("channel_grant_needs_attention"),
                )
                .await?;
            gateway
                .enqueue_reactive_text_delivery(
                    hachimi_gateway::ReactiveDeliverySource {
                        event_key: &message.event_key,
                        run_id: Some(&run.id),
                        final_item_id: "grant-needs-attention",
                    },
                    message.address.clone(),
                    "此会话的工具授权已变更，需要在 Hachimi 中重新确认。",
                    Some(message.event_key.external_message_id.clone()),
                    now_ms(),
                )
                .await?;
            return Ok(gateway
                .finish_ingress(&message.event_key, &session.id, &run.id, true, now_ms())
                .await?);
        }
    };
    let attachment_ids = match prepare_channel_attachments(&state, message, &run).await {
        Ok(attachment_ids) => attachment_ids,
        Err(error) => {
            tracing::warn!(run_id = %run.id, %error, "Channel attachment preparation failed");
            store
                .transition_run(
                    &run.id,
                    RunStatus::Interrupted,
                    Some("channel_attachment_preparation_failed"),
                )
                .await?;
            gateway
                .enqueue_reactive_text_delivery(
                    hachimi_gateway::ReactiveDeliverySource {
                        event_key: &message.event_key,
                        run_id: Some(&run.id),
                        final_item_id: "attachment-needs-attention",
                    },
                    message.address.clone(),
                    "附件已安全接收，但需要在 Hachimi 中继续处理。",
                    Some(message.event_key.external_message_id.clone()),
                    now_ms(),
                )
                .await?;
            return Ok(gateway
                .finish_ingress(&message.event_key, &session.id, &run.id, true, now_ms())
                .await?);
        }
    };

    if run.status == RunStatus::Queued {
        let execution = state
            .agent_executor
            .clone()
            .execute(AgentRunRequest {
                principal: principal.to_owned(),
                session: session.clone(),
                run: run.clone(),
                authority,
                priority: AgentRunPriority::Background,
                user_input_availability: hachimi_agent::UserInputAvailability::Unavailable,
                capability_grants: launched.capability_grants,
                sandbox_snapshot: state.sandbox_snapshot().report,
                attachment_ids: attachment_ids.clone(),
                skill_allowlist: channel_grant
                    .skill_ids
                    .iter()
                    .cloned()
                    .map(SkillId::new)
                    .collect(),
                mcp_tool_allowlist: mcp_tool_allowlist.clone(),
                run_tool_allowlist: None,
                host_revision_snapshot,
                workload_override: Some(WorkloadKind::General),
                recovery_checkpoint: None,
                parent_agent_task_id: None,
                parent_run_id: None,
                agent_depth: 0,
            })
            .await;
        if let Err(error) = execution {
            tracing::warn!(run_id = %run.id, %error, "Channel Agent Run failed");
        }
    }
    let run = store
        .get_run(&run.id)
        .await?
        .ok_or("Channel Run disappeared")?;
    let needs_attention = run.status != RunStatus::Succeeded;
    let (final_item_id, text) = if needs_attention {
        (
            "run-needs-attention".to_owned(),
            "此请求需要在 Hachimi 中继续处理。".to_owned(),
        )
    } else {
        final_assistant_output(&store, &session.id, &run.id)
            .await
            .filter(|(_, text)| !text.trim().is_empty())
            .ok_or("Channel Run has no stable Assistant output")?
    };
    gateway
        .enqueue_reactive_text_delivery(
            hachimi_gateway::ReactiveDeliverySource {
                event_key: &message.event_key,
                run_id: Some(&run.id),
                final_item_id: &final_item_id,
            },
            message.address.clone(),
            &text,
            Some(message.event_key.external_message_id.clone()),
            now_ms(),
        )
        .await?;
    Ok(gateway
        .finish_ingress(
            &message.event_key,
            &session.id,
            &run.id,
            needs_attention,
            now_ms(),
        )
        .await?)
}

async fn prepare_channel_attachments(
    state: &DesktopState,
    message: &VerifiedChannelMessage,
    run: &hachimi_protocol::RunRecord,
) -> Result<Vec<AttachmentId>, Box<dyn std::error::Error + Send + Sync>> {
    let media = message
        .parts
        .iter()
        .filter_map(|part| match part {
            ChannelMessagePart::Text { .. } => None,
            ChannelMessagePart::Image { media }
            | ChannelMessagePart::File { media }
            | ChannelMessagePart::Audio { media }
            | ChannelMessagePart::Video { media } => Some(media),
        })
        .collect::<Vec<_>>();
    if media.is_empty() {
        return Ok(Vec::new());
    }
    let mut attachment_ids = Vec::with_capacity(media.len());
    let mut downloaded_bytes = 0_u64;
    for descriptor in media {
        let metadata_hash = hachimi_gateway::remote_media_metadata_hash(descriptor)?;
        let idempotency_key = channel_media_idempotency_key(message, run, descriptor);
        let result = state
            .plugin_host
            .download_enterprise_attachment(&EnterpriseAttachmentDownloadRequest {
                context: MutationContext {
                    request_id: RequestId(idempotency_key.clone()),
                    client_id: ClientId("channel-runtime".into()),
                    protocol_version: CONTROL_PROTOCOL_VERSION,
                    idempotency_key: idempotency_key.clone(),
                    expected_run_id: Some(run.id.clone()),
                    expected_generation: Some(run.generation),
                },
                provider_id: descriptor.provider_id,
                account_id: message.event_key.account_id.clone(),
                event_id: message.event_key.external_message_id.clone(),
                remote_id: descriptor.remote_id.clone(),
                metadata_hash,
                run_id: run.id.clone(),
                run_generation: run.generation,
                idempotency_key,
            })
            .await?;
        downloaded_bytes = downloaded_bytes.saturating_add(result.byte_size);
        if downloaded_bytes > 50 * 1024 * 1024 {
            return Err("Channel message attachments exceed the 50 MiB limit".into());
        }
        state
            .agent_store
            .attach_to_run_input(&run.id, std::slice::from_ref(&result.attachment_id))
            .await?;
        attachment_ids.push(result.attachment_id);
    }
    Ok(attachment_ids)
}

fn channel_media_idempotency_key(
    message: &VerifiedChannelMessage,
    run: &hachimi_protocol::RunRecord,
    media: &hachimi_protocol::RemoteMediaDescriptor,
) -> String {
    let mut hasher = Sha256::new();
    for value in [
        message.event_key.provider_id.as_str(),
        message.event_key.account_id.as_str(),
        message.event_key.external_message_id.as_str(),
        run.id.as_str(),
        media.remote_id.as_str(),
    ] {
        hasher.update(value.len().to_le_bytes());
        hasher.update(value.as_bytes());
    }
    let digest = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("channel-media:{digest}")
}

fn channel_prompt(message: &VerifiedChannelMessage) -> String {
    let text = message.text();
    if text.trim().is_empty() {
        "请处理随消息发送的附件。".into()
    } else {
        text
    }
}

async fn process_control_command(
    store: &hachimi_storage::AgentStore,
    gateway: &hachimi_gateway::GatewayHost,
    message: &VerifiedChannelMessage,
    command: hachimi_gateway::ChannelControlCommand,
) -> Result<hachimi_protocol::IngressReceipt, Box<dyn std::error::Error + Send + Sync>> {
    let (session_id, result_code, response) = match command {
        hachimi_gateway::ChannelControlCommand::Connect { .. } => {
            let resolution = gateway.resolve_binding(message).await?;
            let session = if let Some(session_id) = resolution.session_id {
                store
                    .get_session(&session_id)
                    .await?
                    .ok_or("Channel Session disappeared")?
            } else {
                create_control_session(store, message, "已连接的消息会话").await?
            };
            gateway.bind_message(message, &session.id, now_ms()).await?;
            (
                Some(session.id),
                "connected",
                "连接成功，后续消息将继续使用当前会话。".to_owned(),
            )
        }
        hachimi_gateway::ChannelControlCommand::New => {
            gateway.reset_binding(message).await?;
            let session = create_control_session(store, message, "新的消息会话").await?;
            gateway.bind_message(message, &session.id, now_ms()).await?;
            (
                Some(session.id),
                "session_created",
                "已创建新的会话。".to_owned(),
            )
        }
        hachimi_gateway::ChannelControlCommand::Status => {
            let resolution = gateway.resolve_binding(message).await?;
            let response = match resolution.session_id.as_ref() {
                Some(session_id) => format!("当前会话：{}", session_id.as_str()),
                None => "当前已授权，尚未创建会话。".into(),
            };
            (resolution.session_id, "status_reported", response)
        }
        hachimi_gateway::ChannelControlCommand::Link { code } => {
            gateway.authorize_message(message).await?;
            let session = create_control_session(store, message, "跨平台共享会话").await?;
            let group = gateway
                .consume_identity_link_code(message, &code, &session.id, now_ms())
                .await?;
            gateway
                .bind_message(message, &group.session_id, now_ms())
                .await?;
            (
                Some(group.session_id),
                "identity_linked",
                "身份关联成功，后续私聊将进入新的跨平台共享会话。".to_owned(),
            )
        }
    };
    gateway
        .enqueue_reactive_text_delivery(
            hachimi_gateway::ReactiveDeliverySource {
                event_key: &message.event_key,
                run_id: None,
                final_item_id: "control-response",
            },
            message.address.clone(),
            &response,
            Some(message.event_key.external_message_id.clone()),
            now_ms(),
        )
        .await?;
    Ok(gateway
        .finish_control_ingress(
            &message.event_key,
            session_id.as_ref(),
            result_code,
            now_ms(),
        )
        .await?)
}

async fn create_control_session(
    store: &hachimi_storage::AgentStore,
    message: &VerifiedChannelMessage,
    label: &str,
) -> Result<SessionRecord, hachimi_storage::AgentStoreError> {
    let timestamp_ms = now_ms();
    let session_id = hachimi_protocol::SessionId::random();
    let workspace = store
        .ensure_managed_workspace(
            hachimi_protocol::WorkspaceId::random(),
            hachimi_storage::WorkspaceOwnerRef::Session(&session_id),
            timestamp_ms,
        )
        .await?;
    let session = SessionRecord {
        id: session_id,
        context: SessionContextBinding::Workspace {
            workspace_id: workspace.id,
        },
        entry_profile: EntryProfile::Workbench,
        title: format!("{} · {}", message.address.provider_id, label),
        archived: false,
        pinned: false,
        parent_session_id: None,
        source_run_id: None,
        created_at_ms: timestamp_ms,
        updated_at_ms: timestamp_ms,
    };
    store.create_session(&session).await
}

async fn prepare_runtime_grants(
    state: &DesktopState,
    grant: &hachimi_protocol::ChannelGrant,
) -> Result<(Vec<McpToolSelection>, Option<HostRevisionSnapshot>), String> {
    if grant.permission_policy.level == hachimi_protocol::PermissionProfile::FullAccess {
        return Ok((Vec::new(), None));
    }
    let mut mcp_tools = Vec::new();
    if !grant.mcp_server_ids.is_empty() {
        if !state.control_plane.feature_flags().mcp_runtime {
            return Err("MCP runtime is disabled".into());
        }
        let runtimes = state
            .mcp_control
            .ready_runtimes()
            .await
            .map_err(|error| error.to_string())?;
        for requested in &grant.mcp_server_ids {
            let runtime = runtimes
                .iter()
                .find(|runtime| runtime.configuration.id.as_str() == requested)
                .ok_or_else(|| format!("MCP server {requested} is not ready"))?;
            let host_identity_hash =
                hachimi_control_plane::mcp_host_identity_hash(&runtime.configuration);
            mcp_tools.extend(runtime.tools.iter().filter_map(|tool| {
                let schema_hash = json_hash(&tool.input_schema);
                let requires_write = !runtime.configuration.read_only_tools.contains(&tool.name);
                grant
                    .permission_policy
                    .allows_mcp(
                        &runtime.configuration.id,
                        &tool.name,
                        &schema_hash,
                        requires_write,
                    )
                    .then(|| McpToolSelection {
                        server_id: runtime.configuration.id.clone(),
                        tool_name: tool.name.clone(),
                        schema_hash,
                        host_identity_hash: host_identity_hash.clone(),
                    })
            }));
        }
    }
    mcp_tools.sort_by(|left, right| {
        (&left.server_id, &left.tool_name).cmp(&(&right.server_id, &right.tool_name))
    });
    mcp_tools.dedup_by(|left, right| {
        left.server_id == right.server_id && left.tool_name == right.tool_name
    });

    let host_revision_snapshot = crate::host_revision_snapshots::snapshot_from_permission_policy(
        &grant.permission_policy,
        &grant.connector_selections,
    );
    if let Some(snapshot) = &host_revision_snapshot {
        for selection in &snapshot.connectors {
            for action in &selection.allowed_actions {
                let requires_write = !grant.permission_policy.rules.connectors.iter().any(|rule| {
                    rule.account_id == selection.account_id
                        && rule
                            .read_only_actions
                            .iter()
                            .any(|read_only| read_only == action)
                });
                if !grant.permission_policy.allows_connector(
                    &selection.account_id,
                    action,
                    requires_write,
                ) {
                    return Err(format!(
                        "Connector action {action} on {} is outside the Channel permission policy",
                        selection.account_id
                    ));
                }
            }
        }
        crate::host_revision_snapshots::validate_connector_revision_selections(
            &state.plugin_host,
            &snapshot.connectors,
        )
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    }
    Ok((mcp_tools, host_revision_snapshot))
}

fn json_hash(value: &(impl serde::Serialize + ?Sized)) -> String {
    serde_json::to_vec(value).map_or_else(
        |_| String::new(),
        |bytes| {
            Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        },
    )
}

async fn final_assistant_output(
    store: &hachimi_storage::AgentStore,
    session_id: &hachimi_protocol::SessionId,
    run_id: &hachimi_protocol::RunId,
) -> Option<(String, String)> {
    store
        .list_transcript(session_id)
        .await
        .ok()?
        .into_iter()
        .rev()
        .find_map(|item| {
            if item.run_id.as_ref() != Some(run_id)
                || item.kind != TranscriptItemKind::Assistant
                || item.status != ItemStatus::Completed
            {
                return None;
            }
            match item.payload {
                ItemPayload::Assistant { text, .. } => Some((item.id.as_str().to_owned(), text)),
                _ => None,
            }
        })
}

fn requested_capabilities(settings: &hachimi_protocol::LlmSettings) -> ProviderCapabilities {
    let structured = settings.structured_output_mode != StructuredOutputMode::Disabled;
    ProviderCapabilities {
        tool_calls: true,
        parallel_tool_calls: true,
        strict_json_schema: structured,
        output_schema: structured,
        text_input: true,
        image_input: true,
        streaming_usage: true,
        http_transport: true,
        context_window: (settings.max_input_tokens > 0)
            .then_some(u64::from(settings.max_input_tokens)),
        max_output_tokens: (settings.max_output_tokens > 0)
            .then_some(u64::from(settings.max_output_tokens)),
        ..ProviderCapabilities::default()
    }
}

fn now_ms() -> i64 {
    i64::try_from(epoch_millis()).unwrap_or(i64::MAX)
}
