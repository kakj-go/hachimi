//! AppServer-owned Channel ingress dispatch into the unified Agent Runtime.

use hachimi_agent::{AgentRunCreateRequest, AgentRunFactory, AgentRunPriority, AgentRunRequest};
use hachimi_protocol::{
    ApprovalPolicy, BehaviorMode, CapabilityGrantSet, ChannelEnvelope, ComputerGrant, EntryProfile,
    FileSystemAccess, FileSystemGrant, ItemPayload, ItemStatus, NetworkGrant, PermissionGrantScope,
    PermissionProfile, ProcessGrant, ProviderCapabilities, RunBudget, RunOrigin, RunPurpose,
    RunStatus, SessionContextBinding, StructuredOutputMode, TranscriptItemKind, WorkloadKind,
};
use tauri::{AppHandle, Manager};

use crate::{DesktopState, epoch_millis};

pub(super) async fn process_ingress(
    app: &AppHandle,
    gateway: &hachimi_gateway::GatewayHost,
    principal: &str,
    envelope: &ChannelEnvelope,
) -> Result<hachimi_protocol::IngressReceipt, Box<dyn std::error::Error + Send + Sync>> {
    let state = app.state::<DesktopState>();
    let store = state.agent_store.clone();
    let settings = state.settings.read().llm.clone();
    let create_request = AgentRunCreateRequest {
        principal: principal.to_owned(),
        idempotency_key: format!("channel:{}", envelope.message_id),
        context: SessionContextBinding::General,
        origin: RunOrigin::Channel {
            channel: envelope.route.channel.clone(),
            account: envelope.route.account.clone(),
            peer: envelope.route.peer.clone(),
            thread: envelope.route.thread.clone(),
            message_id: envelope.message_id.clone(),
        },
        title: format!("{} · {}", envelope.route.channel, envelope.sender)
            .chars()
            .take(200)
            .collect(),
        prompt: envelope.text.clone(),
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
        permission_profile: PermissionProfile::ReadOnly,
        budget: RunBudget::default(),
        requested_capabilities: requested_capabilities(&settings),
        created_at_ms: now_ms(),
    };
    let factory = AgentRunFactory::new(store.clone());
    let routed_session = gateway.session_for_route(&envelope.route).await?;
    let created = if let Some(session_id) = routed_session {
        if let Some(session) = store.get_session(&session_id).await? {
            factory.create_in_session(create_request, session).await?
        } else {
            factory.create(create_request).await?
        }
    } else {
        factory.create(create_request).await?
    };
    gateway
        .bind_route(&envelope.route, &created.session.id, now_ms())
        .await?;

    if created.run.status == RunStatus::Queued {
        let execution = state
            .agent_executor
            .clone()
            .execute(AgentRunRequest {
                principal: principal.to_owned(),
                session: created.session.clone(),
                run: created.run.clone(),
                priority: AgentRunPriority::Background,
                capability_grants: read_only_grants(&created.session.id, &created.run.id),
                sandbox_snapshot: state.sandbox_snapshot().report,
                attachment_ids: Vec::new(),
                skill_allowlist: Vec::new(),
                mcp_tool_allowlist: Vec::new(),
                run_tool_allowlist: Some(Vec::new()),
                schedule_host_grant: None,
                workload_override: Some(WorkloadKind::General),
                recovery_checkpoint: None,
                parent_agent_task_id: None,
                parent_run_id: None,
                agent_depth: 0,
            })
            .await;
        if let Err(error) = execution {
            tracing::warn!(run_id = %created.run.id, %error, "Channel Agent Run failed");
        }
    }
    let run = store
        .get_run(&created.run.id)
        .await?
        .ok_or("Channel Run disappeared")?;
    let needs_attention = run.status != RunStatus::Succeeded;
    let text = if needs_attention {
        "此请求需要在 Hachimi 中继续处理。".to_owned()
    } else {
        final_assistant_text(&store, &created.session.id, &created.run.id)
            .await
            .filter(|text| !text.trim().is_empty())
            .ok_or("Channel Run has no stable Assistant output")?
    };
    gateway
        .enqueue_delivery(
            envelope.route.clone(),
            &format!("channel-result:{}", envelope.message_id),
            &text,
            now_ms(),
        )
        .await?;
    Ok(gateway
        .finish_ingress(
            &envelope.message_id,
            &created.session.id,
            &created.run.id,
            needs_attention,
            now_ms(),
        )
        .await?)
}

fn read_only_grants(
    session_id: &hachimi_protocol::SessionId,
    run_id: &hachimi_protocol::RunId,
) -> CapabilityGrantSet {
    CapabilityGrantSet {
        profile: PermissionProfile::ReadOnly,
        scope: PermissionGrantScope::Run,
        session_id: session_id.clone(),
        run_id: Some(run_id.clone()),
        source: "channel_permission_profile".into(),
        file_system: vec![FileSystemGrant {
            access: FileSystemAccess::Read,
            roots: Vec::new(),
            globs: Vec::new(),
            special_roots: Vec::new(),
        }],
        network: NetworkGrant::default(),
        process: ProcessGrant::default(),
        browser: Default::default(),
        computer: ComputerGrant::default(),
        review_each_command: true,
        expires_at_ms: None,
    }
}

async fn final_assistant_text(
    store: &hachimi_storage::AgentStore,
    session_id: &hachimi_protocol::SessionId,
    run_id: &hachimi_protocol::RunId,
) -> Option<String> {
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
                ItemPayload::Assistant { text, .. } => Some(text),
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
