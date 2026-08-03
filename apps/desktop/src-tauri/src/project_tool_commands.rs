use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

const PROJECT_TOOL_PROMPT: &str = "Open project tools";

#[tauri::command]
pub(super) async fn get_workbench_project_tool_context(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    project_id: ProjectId,
) -> Result<WorkbenchSessionSnapshot, CommandError> {
    let client = state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    if !state.control_plane.feature_flags().workspace_tools {
        return Err(CommandError::new(
            "workspace_tools_disabled",
            "workspace tools are disabled in this build",
        ));
    }

    let existing_context = state
        .agent_store
        .get_project_tool_context_ids(&project_id)
        .await
        .map_err(|error| CommandError::operation("project_tool_context_get_failed", error))?;

    if let Some(context) = existing_context.as_ref()
        && let Some(run) = state
            .agent_store
            .get_run(&context.run_id)
            .await
            .map_err(|error| CommandError::operation("project_tool_run_get_failed", error))?
        && !run.status.is_terminal()
    {
        ensure_project_tool_security(&state, &context.session_id, &run, &context.checkout_id)
            .await?;
        return state
            .workbench
            .session_snapshot(&context.session_id)
            .await
            .map_err(|error| CommandError::operation("project_tool_snapshot_failed", error));
    }

    let request = WorkbenchTaskStartRequest {
        idempotency_key: format!("project-tools:{}:{}", project_id, uuid::Uuid::now_v7()),
        entry_profile: hachimi_protocol::EntryProfile::Workbench,
        session_id: existing_context
            .as_ref()
            .map(|context| context.session_id.clone()),
        project_id: Some(project_id.clone()),
        prompt: PROJECT_TOOL_PROMPT.into(),
        execution_target: Some(hachimi_protocol::ExecutionTarget::Local {
            project_id: project_id.clone(),
        }),
        behavior_mode: hachimi_protocol::BehaviorMode::Default,
        approval_policy: hachimi_protocol::ApprovalPolicy::OnlyWhenNeeded,
        attachment_ids: Vec::new(),
        skill_ids: Vec::new(),
    };
    let llm_settings = state.settings.read().llm.clone();
    let snapshot = state
        .workbench
        .create_task(
            &request,
            llm_settings,
            &client.client_id.0,
            &request.idempotency_key,
            &CancellationToken::new(),
        )
        .await
        .map_err(|error| CommandError::operation("project_tool_context_create_failed", error))?;
    let checkout = snapshot.checkout.as_ref().ok_or_else(|| {
        CommandError::new(
            "project_tool_checkout_missing",
            "project tool context requires a local Checkout",
        )
    })?;

    let run = if snapshot.run.status == hachimi_protocol::RunStatus::Queued {
        state
            .agent_store
            .transition_run(
                &snapshot.run.id,
                hachimi_protocol::RunStatus::Preparing,
                None,
            )
            .await
            .map_err(|error| CommandError::operation("project_tool_run_start_failed", error))?;
        state
            .agent_store
            .transition_run(&snapshot.run.id, hachimi_protocol::RunStatus::Running, None)
            .await
            .map_err(|error| CommandError::operation("project_tool_run_start_failed", error))?
    } else {
        snapshot.run.clone()
    };

    ensure_project_tool_security(&state, &snapshot.session.id, &run, &checkout.id).await?;
    state
        .agent_store
        .bind_project_tool_context(
            &project_id,
            &snapshot.session.id,
            &run.id,
            &checkout.id,
            now_ms(),
        )
        .await
        .map_err(|error| CommandError::operation("project_tool_context_bind_failed", error))?;
    state
        .workbench
        .session_snapshot(&snapshot.session.id)
        .await
        .map_err(|error| CommandError::operation("project_tool_snapshot_failed", error))
}

async fn ensure_project_tool_security(
    state: &DesktopState,
    session_id: &SessionId,
    run: &hachimi_protocol::RunRecord,
    checkout_id: &hachimi_protocol::CheckoutId,
) -> Result<(), CommandError> {
    if state
        .agent_store
        .latest_active_capability_grants(&run.id)
        .await
        .map_err(|error| CommandError::operation("project_tool_grant_get_failed", error))?
        .is_some()
        && state
            .agent_store
            .latest_sandbox_report(&run.id)
            .await
            .map_err(|error| CommandError::operation("project_tool_sandbox_get_failed", error))?
            .is_some()
    {
        return Ok(());
    }
    let checkout = state
        .agent_store
        .get_checkout(checkout_id)
        .await
        .map_err(|error| CommandError::operation("project_tool_checkout_get_failed", error))?
        .ok_or_else(|| {
            CommandError::new(
                "project_tool_checkout_missing",
                "project tool Checkout does not exist",
            )
        })?;
    let mut grants = expand_permission_profile(
        hachimi_protocol::PermissionProfile::ExternalSandbox,
        hachimi_protocol::BehaviorMode::Default,
        session_id.clone(),
        run.id.clone(),
        checkout.path,
    );
    grants.source = "project_tools_direct_user".into();
    state
        .agent_store
        .persist_run_security_snapshot(&grants, &state.sandbox_snapshot().report, now_ms())
        .await
        .map_err(|error| CommandError::operation("project_tool_security_store_failed", error))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
