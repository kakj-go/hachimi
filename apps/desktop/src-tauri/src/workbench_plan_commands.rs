use super::*;

pub(super) fn start_workbench_activity_bridge(
    app: AppHandle,
    mut receiver: tokio::sync::broadcast::Receiver<hachimi_protocol::SessionRunActivity>,
) {
    tauri::async_runtime::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(activity) => {
                    let _ = app.emit(WORKBENCH_SESSION_ACTIVITY_EVENT, activity);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

pub(super) fn emit_workbench_run_completion(app: &AppHandle, run: RunRecord) {
    let _ = app.emit_to("workbench", WORKBENCH_RUN_EVENT, &run);
}

pub(super) async fn finalize_review_run(
    store: &AgentStore,
    snapshot: &WorkbenchTaskSnapshot,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if snapshot.run.purpose != hachimi_protocol::RunPurpose::Review {
        return Ok(());
    }
    let Some(current) = store.get_run(&snapshot.run.id).await? else {
        return Ok(());
    };
    if current.status != hachimi_protocol::RunStatus::Succeeded {
        return Ok(());
    }
    let Some(review) = store.get_review_by_run(&snapshot.run.id).await? else {
        return Ok(());
    };
    let transcript = store.list_transcript(&snapshot.session.id).await?;
    let final_text = transcript
        .iter()
        .rev()
        .find(|item| {
            item.run_id.as_ref() == Some(&snapshot.run.id)
                && item.kind == hachimi_protocol::TranscriptItemKind::Assistant
                && item.status == hachimi_protocol::ItemStatus::Completed
        })
        .and_then(|item| match &item.payload {
            hachimi_protocol::ItemPayload::Assistant { text, .. } => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let parsed = hachimi_agent::parse_review_output(&final_text);
    let checkout = snapshot
        .checkout
        .as_ref()
        .ok_or("Review Run is missing its Project checkout")?;
    let findings = hachimi_agent::materialize_review_findings(
        &review.id,
        Path::new(&checkout.path),
        &parsed.output,
    );
    store
        .complete_review(
            &review,
            &parsed.output,
            &findings,
            parsed.used_plain_text_fallback,
            i64::try_from(epoch_millis()).unwrap_or(i64::MAX),
        )
        .await?;
    Ok(())
}

#[tauri::command]
pub(super) async fn revise_workbench_plan(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: hachimi_protocol::PlanRevisionRequest,
) -> Result<WorkbenchTaskSnapshot, CommandError> {
    let client = state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    if request.idempotency_key.trim().is_empty() || request.idempotency_key.len() > 128 {
        return Err(CommandError::new(
            "invalid_idempotency_key",
            "idempotency key must contain 1-128 bytes",
        ));
    }
    let model_snapshot = state.settings.read().llm.clone();
    let snapshot = state
        .workbench
        .revise_plan(
            &request,
            model_snapshot,
            &client.client_id.0,
            &CancellationToken::new(),
        )
        .await
        .map_err(|error| CommandError::operation("workbench_plan_revise_failed", error))?;
    if snapshot.run.status == hachimi_protocol::RunStatus::Queued {
        spawn_workbench_run(app, client, snapshot.clone(), Vec::new());
    }
    Ok(snapshot)
}

#[tauri::command]
pub(super) async fn execute_workbench_git(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: hachimi_protocol::WorkbenchGitRequest,
) -> Result<hachimi_protocol::WorkbenchGitResponse, CommandError> {
    let client = state.authorize(&window, ControlMethod::WorkbenchWindow)?;
    require_window(&window, "workbench")?;
    let needs_message = matches!(
        &request.action,
        hachimi_protocol::WorkbenchGitAction::Commit { message: None }
    );
    let mut generated_message = None;
    if needs_message {
        let runs = state
            .agent_store
            .list_runs(&request.session_id)
            .await
            .map_err(|error| CommandError::operation("workbench_git_session_failed", error))?;
        if let Some(run) = runs.last() {
            let summaries = state
                .agent_store
                .list_run_summaries(&request.session_id)
                .await
                .unwrap_or_default();
            let context = summaries.last().map_or_else(
                || "Summarize the current working tree changes.".to_owned(),
                |summary| {
                    let files = summary
                        .files
                        .iter()
                        .take(24)
                        .map(|file| file.path.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!(
                        "Changed files: {}. Additions: {}. Deletions: {}. Paths: {files}",
                        summary.changed_files, summary.additions, summary.deletions
                    )
                },
            );
            if let Ok(generated) = state
                .agent_executor
                .generate_auxiliary_text(
                    &run.configuration,
                    "Write one concise Git commit subject for the described changes. Return only the subject, without quotes, Markdown, a body, or a trailing period. Do not invent details.",
                    &context,
                    80,
                    CancellationToken::new(),
                )
                .await
                && let Some(subject) = generated
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .map(|line| line.trim_matches(['\'', '"']))
                    .filter(|line| !line.is_empty())
            {
                generated_message = Some(subject.chars().take(160).collect::<String>());
            }
        }
    }
    let response = state
        .workbench
        .execute_git(&request, &client.client_id.0, generated_message.as_deref())
        .await
        .map_err(|error| CommandError::operation("workbench_git_action_failed", error))?;
    if let Ok(environment) = state
        .workbench
        .environment_snapshot(&request.session_id)
        .await
    {
        crate::environment_commands::emit_workbench_environment(
            &app,
            &environment,
            vec![
                hachimi_protocol::WorkbenchEnvironmentChangeReason::Git,
                hachimi_protocol::WorkbenchEnvironmentChangeReason::Files,
            ],
        );
    }
    Ok(response)
}
