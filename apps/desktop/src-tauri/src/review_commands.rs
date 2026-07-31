// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex/codex-rs/app-server/src/request_processors/turn_processor.rs
// and codex-rs/core/src/session/review.rs
// @ 4c43465133428898aa84f0bfc02c306ed65fb66a.
// Modified for Hachimi: Tauri adapter, persisted Review Run, and detached lineage.

use hachimi_agent::{AgentRunCreateRequest, AgentRunFactory, build_review_prompt};
use hachimi_protocol::{
    ApprovalPolicy, BehaviorMode, ItemId, ItemPayload, ItemRelations, ItemStatus,
    PermissionProfile, ReviewDelivery, ReviewFinding, ReviewFindingUpdateRequest, ReviewId,
    ReviewRecord, ReviewSnapshot, ReviewStartRequest, ReviewStartSnapshot, RunId, RunOrigin,
    RunPurpose, RunRecord, RunStatus, TranscriptItem, TranscriptItemKind, WorkbenchTaskSnapshot,
};
use tauri::{AppHandle, State, WebviewWindow};

use super::*;

fn review_error(code: &'static str, error: impl std::fmt::Display) -> CommandError {
    CommandError::operation(code, error)
}

async fn dispatch_review(
    window: &WebviewWindow,
    state: &DesktopState,
    request: hachimi_control_plane::ReviewAppRequest,
) -> Result<hachimi_control_plane::ReviewAppResponse, CommandError> {
    let client = state.authorize(window, ControlMethod::WorkbenchWindow)?;
    require_window(window, "workbench")?;
    let context = hachimi_control_plane::AppServerContext {
        principal: client.client_id.0.clone(),
        client,
    };
    match state
        .app_server
        .dispatch(
            &context,
            hachimi_control_plane::AppServerRequest::Domain(Box::new(
                hachimi_control_plane::AppServerDomainRequest::Review(request),
            )),
        )
        .await
        .map_err(|error| review_error("review_app_server_failed", error))?
    {
        hachimi_control_plane::AppServerResponse::Domain(response) => match *response {
            hachimi_control_plane::AppServerDomainResponse::Review(response) => Ok(*response),
            _ => Err(CommandError::new(
                "review_app_server_protocol_mismatch",
                "App Server returned a response for a different domain",
            )),
        },
        _ => Err(CommandError::new(
            "review_app_server_protocol_mismatch",
            "App Server returned a response for a different domain",
        )),
    }
}

fn validate_review_target(target: &hachimi_protocol::ReviewTarget) -> Result<(), CommandError> {
    let value = match target {
        hachimi_protocol::ReviewTarget::UncommittedChanges => return Ok(()),
        hachimi_protocol::ReviewTarget::BaseBranch(value)
        | hachimi_protocol::ReviewTarget::Commit(value)
        | hachimi_protocol::ReviewTarget::Custom(value) => value.trim(),
    };
    if value.is_empty() || value.chars().count() > 4_000 || value.chars().any(char::is_control) {
        return Err(CommandError::new(
            "review_target_invalid",
            "Review target must contain usable bounded text",
        ));
    }
    Ok(())
}

#[tauri::command]
pub(super) async fn start_review(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ReviewStartRequest,
) -> Result<ReviewStartSnapshot, CommandError> {
    match dispatch_review(
        &window,
        &state,
        hachimi_control_plane::ReviewAppRequest::Start(request),
    )
    .await?
    {
        hachimi_control_plane::ReviewAppResponse::Started(snapshot) => Ok(snapshot),
        _ => Err(CommandError::new(
            "review_response_mismatch",
            "expected started Review",
        )),
    }
}

pub(super) async fn start_review_inner(
    app: AppHandle,
    state: &DesktopState,
    client: hachimi_protocol::ClientContext,
    request: ReviewStartRequest,
) -> Result<ReviewStartSnapshot, CommandError> {
    if !state.control_plane.feature_flags().workspace_tools {
        return Err(CommandError::new(
            "workspace_tools_disabled",
            "Review requires read-only Workspace tools",
        ));
    }
    if request.context.client_id != client.client_id
        || request.context.protocol_version != hachimi_protocol::CONTROL_PROTOCOL_VERSION
        || request.context.idempotency_key.trim().is_empty()
        || request.context.idempotency_key.len() > 128
    {
        return Err(CommandError::new(
            "review_mutation_invalid",
            "Review mutation context is invalid",
        ));
    }
    validate_review_target(&request.target)?;
    let source = state
        .workbench
        .session_snapshot(&request.session_id)
        .await
        .map_err(|error| review_error("review_session_failed", error))?;
    let source_run = source.runs.last().cloned().ok_or_else(|| {
        CommandError::new("review_source_run_missing", "Session has no source Run")
    })?;
    if !source_run.status.is_terminal() {
        return Err(CommandError::new(
            "review_source_run_active",
            "Review cannot start while the source Session has an active Run",
        ));
    }
    if request
        .context
        .expected_run_id
        .as_ref()
        .is_some_and(|run_id| run_id != &source_run.id)
        || request
            .context
            .expected_generation
            .is_some_and(|generation| generation != source_run.generation)
    {
        return Err(CommandError::new(
            "review_source_precondition_failed",
            "Review source Run generation changed",
        ));
    }
    let project_id = source.session.context.project_id().ok_or_else(|| {
        CommandError::new(
            "review_project_required",
            "Review requires a Project Session",
        )
    })?;
    let checkout_id = source.session.context.checkout_id().ok_or_else(|| {
        CommandError::new("review_checkout_required", "Review requires a Checkout")
    })?;
    let project = state
        .agent_store
        .get_project(project_id)
        .await
        .map_err(|error| review_error("review_project_failed", error))?
        .ok_or_else(|| CommandError::new("review_project_missing", "Project does not exist"))?;
    let checkout = state
        .agent_store
        .get_checkout(checkout_id)
        .await
        .map_err(|error| review_error("review_checkout_failed", error))?
        .ok_or_else(|| CommandError::new("review_checkout_missing", "Checkout does not exist"))?;
    let prompt = build_review_prompt(&request.target);
    let now = i64::try_from(epoch_millis()).unwrap_or(i64::MAX);
    let model_snapshot = state.settings.read().llm.clone();
    let (session, run) = match request.delivery {
        ReviewDelivery::Detached => {
            let created = AgentRunFactory::new(state.agent_store.clone())
                .create(AgentRunCreateRequest {
                    principal: client.client_id.0.clone(),
                    idempotency_key: request.context.idempotency_key.clone(),
                    context: source.session.context.clone(),
                    origin: RunOrigin::Handoff {
                        source_session_id: source.session.id.clone(),
                        source_run_id: source_run.id.clone(),
                    },
                    title: format!("Review: {}", source.session.title)
                        .chars()
                        .take(200)
                        .collect(),
                    prompt: prompt.clone(),
                    attachment_ids: Vec::new(),
                    parent_session_id: Some(source.session.id.clone()),
                    source_run_id: Some(source_run.id.clone()),
                    purpose: RunPurpose::Review,
                    model_snapshot,
                    entry_profile: source.session.entry_profile,
                    workload_override: source_run.configuration.workload_override,
                    behavior_mode: BehaviorMode::Default,
                    execution_target: source_run.configuration.execution_target.clone(),
                    approval_policy: ApprovalPolicy::NeverPrompt,
                    permission_profile: PermissionProfile::ReadOnly,
                    budget: source_run.configuration.budget.clone(),
                    requested_capabilities: source_run.requested_capabilities,
                    created_at_ms: now,
                })
                .await
                .map_err(|error| review_error("review_run_create_failed", error))?;
            (created.session, created.run)
        }
        ReviewDelivery::Inline => {
            let mut configuration = source_run.configuration.clone();
            configuration.model_snapshot = model_snapshot;
            configuration.behavior_mode = BehaviorMode::Default;
            configuration.approval_policy = ApprovalPolicy::NeverPrompt;
            configuration.permission_profile = PermissionProfile::ReadOnly;
            configuration.accepted_plan_id = None;
            configuration.accepted_plan_revision = None;
            let candidate = RunRecord {
                id: RunId::random(),
                session_id: source.session.id.clone(),
                status: RunStatus::Queued,
                purpose: RunPurpose::Review,
                origin: RunOrigin::Interactive,
                generation: 1,
                configuration,
                requested_capabilities: source_run.requested_capabilities,
                negotiated_capabilities: hachimi_protocol::ProviderCapabilities::default(),
                provider_capability_probe: None,
                capability_degradations: Vec::new(),
                failure_code: None,
                created_at_ms: now,
                updated_at_ms: now,
            };
            let run = state
                .agent_store
                .create_run_idempotent(
                    &client.client_id.0,
                    &request.context.idempotency_key,
                    &candidate,
                )
                .await
                .map_err(|error| review_error("review_run_create_failed", error))?;
            if run.id == candidate.id {
                state
                    .agent_store
                    .append_transcript_item(TranscriptItem {
                        id: ItemId::random(),
                        session_id: source.session.id.clone(),
                        run_id: Some(run.id.clone()),
                        sequence: 0,
                        kind: TranscriptItemKind::User,
                        status: ItemStatus::Completed,
                        payload: ItemPayload::User {
                            text: prompt.clone(),
                            attachment_ids: Vec::new(),
                        },
                        relations: ItemRelations::default(),
                        created_at_ms: now,
                    })
                    .await
                    .map_err(|error| review_error("review_item_create_failed", error))?;
            }
            (source.session.clone(), run)
        }
    };
    let review = state
        .agent_store
        .create_review_record(&ReviewRecord {
            id: ReviewId::random(),
            session_id: session.id.clone(),
            run_id: run.id.clone(),
            target: request.target,
            delivery: request.delivery,
            created_at_ms: now,
        })
        .await
        .map_err(|error| review_error("review_record_create_failed", error))?;
    if run.status == RunStatus::Queued {
        spawn_workbench_run(
            app,
            client,
            WorkbenchTaskSnapshot {
                project: Some(project),
                checkout: Some(checkout),
                session: session.clone(),
                run: run.clone(),
            },
            Vec::new(),
        );
    }
    Ok(ReviewStartSnapshot {
        review,
        session,
        run,
    })
}

#[tauri::command]
pub(super) async fn get_review(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    review_id: ReviewId,
) -> Result<ReviewSnapshot, CommandError> {
    match dispatch_review(
        &window,
        &state,
        hachimi_control_plane::ReviewAppRequest::Get(review_id),
    )
    .await?
    {
        hachimi_control_plane::ReviewAppResponse::Review(review) => Ok(review),
        _ => Err(CommandError::new(
            "review_response_mismatch",
            "expected Review",
        )),
    }
}

#[tauri::command]
pub(super) async fn list_reviews(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    session_id: hachimi_protocol::SessionId,
) -> Result<Vec<ReviewSnapshot>, CommandError> {
    match dispatch_review(
        &window,
        &state,
        hachimi_control_plane::ReviewAppRequest::List(session_id),
    )
    .await?
    {
        hachimi_control_plane::ReviewAppResponse::Reviews(reviews) => Ok(reviews),
        _ => Err(CommandError::new(
            "review_response_mismatch",
            "expected Review list",
        )),
    }
}

#[tauri::command]
pub(super) async fn update_review_finding(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ReviewFindingUpdateRequest,
) -> Result<ReviewFinding, CommandError> {
    match dispatch_review(
        &window,
        &state,
        hachimi_control_plane::ReviewAppRequest::UpdateFinding(request),
    )
    .await?
    {
        hachimi_control_plane::ReviewAppResponse::Finding(finding) => Ok(finding),
        _ => Err(CommandError::new(
            "review_response_mismatch",
            "expected Review finding",
        )),
    }
}
