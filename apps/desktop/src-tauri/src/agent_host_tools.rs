//! Local Host tools exposed only through the unified Agent ToolRuntime.

use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use hachimi_agent::{ToolExecutor, ToolFuture, ToolInvocation, ToolResult};
use hachimi_protocol::{
    BrowserAction, BrowserActionRequest, BrowserAutomationLeaseId, BrowserAutomationPreference,
    BrowserAutomationSurfaceKind, BrowserCapability, BrowserNetworkPolicy, BrowserNetworkRule,
    BrowserNetworkRuleKind, BrowserObservationId, BrowserPermissionDecision, BrowserProfileKind,
    CapabilityGrantSet, ClientId, ComputerActionRequest, ComputerAppRule,
    ConnectorInvocationRequest, EnterpriseAttachmentDownloadRequest, HostPolicyDecision,
    IntegrationProviderId, ModelInputImage, MutationContext, RequestId, ScheduleHostGrant,
    SessionId, SessionSourceOrigin, ToolDescriptor, ToolEffect,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;

use crate::agent_host_tools_support::{
    browser_error_code, browser_target_summary, computer_action_category, computer_error_code,
    computer_target_summary, connector_source, now_ms, object_schema, stable_hash,
};

#[path = "agent_host_tools_registry.rs"]
mod registry;
pub(super) use registry::*;

pub(super) type EnvironmentChangeSink = Arc<dyn Fn(SessionId) + Send + Sync>;
const BROWSER_LEASE_LIFETIME_MS: i64 = 30 * 60 * 1_000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserStartArgs {
    initial_url: String,
    #[serde(default)]
    surface: Option<BrowserAutomationPreference>,
}

struct BrowserStartTool {
    host: Arc<hachimi_browser::BrowserHost>,
    embedded: Arc<crate::embedded_browser_agent::EmbeddedAgentBrowser>,
    store: hachimi_storage::AgentStore,
    session_id: SessionId,
    run_id: hachimi_protocol::RunId,
    grants: CapabilityGrantSet,
    sandbox: hachimi_protocol::SandboxCapabilityReport,
    schedule_host_grant: Option<ScheduleHostGrant>,
    environment_change_sink: EnvironmentChangeSink,
}

impl ToolExecutor for BrowserStartTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "browser_start".into(),
            description: "Acquire a Run-owned automation lease for the visible embedded browser or a paired external Chrome tab. The embedded surface controls the same tab the user sees.".into(),
            input_schema: object_schema(
                json!({
                    "initialUrl": { "type": "string", "format": "uri", "maxLength": 4096 },
                    "surface": { "type": "string", "enum": ["auto", "embedded", "external_chrome"] }
                }),
                &["initialUrl"],
            ),
            effect: ToolEffect::BrowserAct,
            parallel_safe: false,
            required_scopes: vec!["browser.control".into()],
        }
    }

    fn execute(&self, invocation: ToolInvocation) -> ToolFuture {
        let host = Arc::clone(&self.host);
        let embedded = Arc::clone(&self.embedded);
        let store = self.store.clone();
        let session_id = self.session_id.clone();
        let run_id = self.run_id.clone();
        let grants = self.grants.clone();
        let sandbox = self.sandbox.clone();
        let schedule_host_grant = self.schedule_host_grant.clone();
        let environment_change_sink = Arc::clone(&self.environment_change_sink);
        Box::pin(async move {
            let args: BrowserStartArgs =
                match serde_json::from_value(invocation.call.arguments.clone()) {
                    Ok(args) => args,
                    Err(error) => {
                        return Ok(ToolResult::failed(
                            &invocation.call,
                            format!("invalid Browser start arguments: {error}"),
                        ));
                    }
                };
            let settings = sqlx::query(
                "SELECT automation_enabled, automation_preference FROM browser_host_settings WHERE singleton = 1",
            )
            .fetch_one(store.pool())
            .await
            .map_err(|error| hachimi_agent::ToolExecutionError::Failed(error.to_string()))?;
            if !settings.get::<bool, _>("automation_enabled") {
                return Ok(ToolResult::failed(
                    &invocation.call,
                    "Browser automation is disabled in Settings",
                ));
            }
            let configured_preference =
                match settings.get::<String, _>("automation_preference").as_str() {
                    "embedded" => BrowserAutomationPreference::Embedded,
                    "external_chrome" => BrowserAutomationPreference::ExternalChrome,
                    _ => BrowserAutomationPreference::Auto,
                };
            let pairing = host.latest_confirmed_pairing();
            let surface = match crate::browser_router::select_browser_surface(
                configured_preference,
                args.surface,
                embedded.runtime_available(),
                pairing.is_some(),
                schedule_host_grant.is_some(),
            ) {
                Ok(surface) => surface,
                Err(error) => return Ok(ToolResult::failed(&invocation.call, error)),
            };
            let use_external = surface == BrowserAutomationSurfaceKind::ExternalChrome;
            let profile_kind = if use_external {
                BrowserProfileKind::ChromeExtension
            } else {
                BrowserProfileKind::Isolated
            };
            let scheduled_browser = schedule_host_grant
                .as_ref()
                .and_then(|grant| grant.browser.as_ref());
            let initial_origin = match hachimi_browser::normalized_origin(args.initial_url.trim()) {
                Ok(origin) => origin,
                Err(error) => {
                    return Ok(ToolResult::failed(&invocation.call, error.to_string()));
                }
            };
            if let Some(browser) = scheduled_browser {
                if !browser.enabled || !browser.document_origins.contains(&initial_origin) {
                    return Ok(ToolResult::failed(
                        &invocation.call,
                        "scheduled Browser origin is outside the persisted document-origin grant",
                    ));
                }
            } else if schedule_host_grant.is_some() {
                return Ok(ToolResult::failed(
                    &invocation.call,
                    "scheduled Browser access is not authorized",
                ));
            }
            if schedule_host_grant.is_none()
                && !grants.browser.origins.is_empty()
                && !grants
                    .browser
                    .origins
                    .iter()
                    .any(|allowed| allowed == &initial_origin)
            {
                return Ok(ToolResult::failed(
                    &invocation.call,
                    "Browser origin is outside the active Run grant",
                ));
            }
            let mut capabilities = Vec::new();
            if grants.browser.observe {
                capabilities.push(BrowserCapability::Observe);
            }
            if grants.browser.act {
                capabilities.push(BrowserCapability::Act);
            }
            if grants.browser.upload {
                capabilities.push(BrowserCapability::Upload);
            }
            if grants.browser.download {
                capabilities.push(BrowserCapability::Download);
            }
            if grants.browser.cookie_storage {
                capabilities.push(BrowserCapability::CookieStorage);
            }
            if grants.browser.cdp {
                capabilities.push(BrowserCapability::Cdp);
            }
            let allow_private_network = if let Some(browser) = scheduled_browser {
                browser.allow_private_network
            } else {
                match crate::browser_tool_policy::require_interactive_browser_access(
                    &store,
                    &session_id,
                    &run_id,
                    invocation.run_generation,
                    args.initial_url.trim(),
                    surface,
                    &capabilities,
                )
                .await
                {
                    Ok(allow_private_network) => allow_private_network,
                    Err(error) => return Ok(ToolResult::failed(&invocation.call, error)),
                }
            };
            if !use_external {
                if !grants.browser.observe
                    || !grants.browser.act
                    || !sandbox.os_enforced
                    || !sandbox.process_enforced
                {
                    return Ok(ToolResult::failed(
                        &invocation.call,
                        "embedded Browser control is outside the active Run grant or sandbox enforcement is unavailable",
                    ));
                }
                if let Err(error) = hachimi_browser::validate_agent_browser_target(
                    args.initial_url.trim(),
                    allow_private_network,
                )
                .await
                {
                    return Ok(ToolResult::failed(&invocation.call, error.to_string()));
                }
                let started = match embedded
                    .start(
                        &session_id,
                        &run_id,
                        invocation.run_generation,
                        args.initial_url.trim(),
                        &capabilities,
                    )
                    .await
                {
                    Ok(started) => started,
                    Err(error) => {
                        return Ok(ToolResult::failed(&invocation.call, error.to_string()));
                    }
                };
                environment_change_sink(session_id.clone());
                let content = serde_json::to_value(&started).map_err(|error| {
                    hachimi_agent::ToolExecutionError::Failed(error.to_string())
                })?;
                return Ok(ToolResult::succeeded(
                    &invocation.call,
                    content.to_string(),
                    content,
                ));
            }
            let network_rules = scheduled_browser.map_or_else(
                || {
                    vec![
                        (
                            initial_origin.clone(),
                            BrowserNetworkRuleKind::Document,
                            allow_private_network,
                        ),
                        (
                            initial_origin.clone(),
                            BrowserNetworkRuleKind::Resource,
                            allow_private_network,
                        ),
                    ]
                },
                |browser| {
                    browser
                        .document_origins
                        .iter()
                        .map(|origin| {
                            (
                                origin.clone(),
                                BrowserNetworkRuleKind::Document,
                                browser.allow_private_network,
                            )
                        })
                        .chain(browser.resource_origins.iter().map(|origin| {
                            (
                                origin.clone(),
                                BrowserNetworkRuleKind::Resource,
                                browser.allow_private_network,
                            )
                        }))
                        .collect()
                },
            );
            let initial_network_policy = BrowserNetworkPolicy {
                rules: network_rules
                    .iter()
                    .map(|(origin, kind, allow_private_network)| BrowserNetworkRule {
                        origin: origin.clone(),
                        kind: *kind,
                        allow_private_network: *allow_private_network,
                        expires_at_ms: None,
                    })
                    .collect(),
                deny_private_network_by_default: true,
                revision: 1,
            };
            let mut session = match host
                .start_session_with_network_policy(
                    profile_kind,
                    session_id,
                    run_id,
                    invocation.run_generation,
                    Some(args.initial_url.trim()),
                    Some(initial_network_policy),
                    &sandbox,
                    pairing.as_ref().map(|value| &value.id),
                )
                .await
            {
                Ok(session) => session,
                Err(error) => return Ok(ToolResult::failed(&invocation.call, error.to_string())),
            };
            let Some(origin) = session.origin.clone() else {
                return Ok(ToolResult::failed(
                    &invocation.call,
                    "Browser did not report a valid HTTP(S) origin",
                ));
            };
            for (network_origin, network_kind, allow_private_network) in network_rules {
                if let Err(error) = host
                    .grant_site_permission(
                        &session.id,
                        &session.owner_session_id,
                        &session.owner_run_id,
                        session.revision,
                        &network_origin,
                        capabilities.clone(),
                        BrowserPermissionDecision::AllowSession,
                        network_kind,
                        allow_private_network,
                        if scheduled_browser.is_some() {
                            "schedule:unattended-browser-grant"
                        } else {
                            "run:approved-browser-start"
                        },
                        None,
                    )
                    .await
                {
                    let _ = host.stop(&session.id, &session.owner_run_id).await;
                    return Ok(ToolResult::failed(&invocation.call, error.to_string()));
                }
            }
            let observation = if grants.browser.observe {
                host.observe(
                    &session.id,
                    &session.owner_run_id,
                    invocation.run_generation,
                )
                .await
                .ok()
            } else {
                None
            };
            session = host
                .session_snapshot(&session.id, &session.owner_run_id)
                .map_err(|error| hachimi_agent::ToolExecutionError::Failed(error.to_string()))?;
            let lease = store
                .create_external_browser_automation_lease(
                    &session.owner_session_id,
                    &session.owner_run_id,
                    invocation.run_generation,
                    &session.id,
                    &capabilities,
                    now_ms().saturating_add(BROWSER_LEASE_LIFETIME_MS),
                )
                .await
                .map_err(|error| hachimi_agent::ToolExecutionError::Failed(error.to_string()))?;
            if let Some(observation) = observation.as_ref() {
                store
                    .store_external_browser_lease_observation(
                        &lease.id,
                        &session.owner_session_id,
                        observation,
                    )
                    .await
                    .map_err(|error| {
                        hachimi_agent::ToolExecutionError::Failed(error.to_string())
                    })?;
            }
            persist_browser_environment(
                &store,
                &environment_change_sink,
                &session,
                observation.as_ref().map(|value| value.url.as_str()),
                observation.as_ref().map(|value| value.title.as_str()),
            )
            .await?;
            append_local_host_audit(
                &store,
                &session.owner_session_id,
                &session.owner_run_id,
                "browser.start",
                browser_target_summary(&origin, "start"),
                "succeeded",
                "session_started",
            )
            .await?;
            let content = json!({ "lease": lease, "observation": observation });
            let model_content = content.to_string();
            Ok(ToolResult::succeeded(
                &invocation.call,
                model_content,
                content,
            ))
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserSessionArgs {
    lease_id: BrowserAutomationLeaseId,
}

struct BrowserObserveTool {
    host: Arc<hachimi_browser::BrowserHost>,
    embedded: Arc<crate::embedded_browser_agent::EmbeddedAgentBrowser>,
    store: hachimi_storage::AgentStore,
    session_id: SessionId,
    run_id: hachimi_protocol::RunId,
    grants: CapabilityGrantSet,
    schedule_host_grant: Option<ScheduleHostGrant>,
    environment_change_sink: EnvironmentChangeSink,
}

impl ToolExecutor for BrowserObserveTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "browser_observe".into(),
            description: "Read a fresh screenshot, page text and accessibility tree from a Run-owned Browser lease. The observation fences the next action against tab and user-input revisions.".into(),
            input_schema: object_schema(json!({ "leaseId": { "type": "string" } }), &["leaseId"]),
            effect: ToolEffect::BrowserObserve,
            parallel_safe: false,
            required_scopes: vec!["browser.observe".into()],
        }
    }

    fn execute(&self, invocation: ToolInvocation) -> ToolFuture {
        let host = Arc::clone(&self.host);
        let embedded = Arc::clone(&self.embedded);
        let store = self.store.clone();
        let session_id = self.session_id.clone();
        let run_id = self.run_id.clone();
        let grants = self.grants.clone();
        let schedule_host_grant = self.schedule_host_grant.clone();
        let environment_change_sink = Arc::clone(&self.environment_change_sink);
        Box::pin(async move {
            let args: BrowserSessionArgs =
                match serde_json::from_value(invocation.call.arguments.clone()) {
                    Ok(args) => args,
                    Err(error) => {
                        return Ok(ToolResult::failed(
                            &invocation.call,
                            format!("invalid Browser observe arguments: {error}"),
                        ));
                    }
                };
            let routed_lease_id = args.lease_id.clone();
            let browser_session_id = match crate::browser_router::route_browser_lease(
                &store,
                &args.lease_id,
                &session_id,
                &run_id,
                invocation.run_generation,
            )
            .await
            {
                Ok(crate::browser_router::BrowserLeaseRoute::Embedded) => {
                    return match embedded
                        .observe(
                            &args.lease_id,
                            &run_id,
                            invocation.run_generation,
                            crate::browser_tool_policy::embedded_origin_policy(
                                &grants,
                                schedule_host_grant.as_ref(),
                            ),
                        )
                        .await
                    {
                        Ok(observation) => {
                            environment_change_sink(session_id.clone());
                            let content = serde_json::to_value(&observation).map_err(|error| {
                                hachimi_agent::ToolExecutionError::Failed(error.to_string())
                            })?;
                            Ok(ToolResult::succeeded(
                                &invocation.call,
                                content.to_string(),
                                content,
                            ))
                        }
                        Err(error) => Ok(ToolResult::failed(&invocation.call, error.to_string())),
                    };
                }
                Ok(crate::browser_router::BrowserLeaseRoute::ExternalChrome {
                    browser_session_id,
                    ..
                }) => browser_session_id,
                Err(error) => return Ok(ToolResult::failed(&invocation.call, error)),
            };
            let current_session = match host.session_snapshot(&browser_session_id, &run_id) {
                Ok(session) if session.owner_session_id == session_id => session,
                Ok(_) => {
                    return Ok(ToolResult::failed(
                        &invocation.call,
                        "Browser Session ownership changed",
                    ));
                }
                Err(error) => return Ok(ToolResult::failed(&invocation.call, error.to_string())),
            };
            if let Err(error) = crate::browser_tool_policy::require_external_session_access(
                &store,
                &current_session,
                schedule_host_grant.as_ref(),
                invocation.run_generation,
                &[BrowserCapability::Observe],
            )
            .await
            {
                return Ok(ToolResult::failed(&invocation.call, error));
            }
            match host
                .observe(&browser_session_id, &run_id, invocation.run_generation)
                .await
            {
                Ok(observation) => {
                    let session = host
                        .session_snapshot(&observation.browser_session_id, &run_id)
                        .map_err(|error| {
                            hachimi_agent::ToolExecutionError::Failed(error.to_string())
                        })?;
                    if session.owner_session_id != session_id {
                        return Ok(ToolResult::failed(
                            &invocation.call,
                            "Browser Session ownership changed",
                        ));
                    }
                    store
                        .store_external_browser_lease_observation(
                            &routed_lease_id,
                            &session_id,
                            &observation,
                        )
                        .await
                        .map_err(|error| {
                            hachimi_agent::ToolExecutionError::Failed(error.to_string())
                        })?;
                    persist_browser_environment(
                        &store,
                        &environment_change_sink,
                        &session,
                        Some(&observation.url),
                        Some(&observation.title),
                    )
                    .await?;
                    let mut content = serde_json::to_value(&observation).map_err(|error| {
                        hachimi_agent::ToolExecutionError::Failed(error.to_string())
                    })?;
                    if let Value::Object(fields) = &mut content {
                        fields.insert("leaseId".into(), json!(routed_lease_id));
                    }
                    let model_content = content.to_string();
                    Ok(ToolResult::succeeded(
                        &invocation.call,
                        model_content,
                        content,
                    ))
                }
                Err(error) => Ok(ToolResult::failed(&invocation.call, error.to_string())),
            }
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EmbeddedBrowserActionArgs {
    lease_id: BrowserAutomationLeaseId,
    observation_id: BrowserObservationId,
    expected_tab_revision: u64,
    expected_input_epoch: u64,
    action: BrowserAction,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExternalBrowserActionArgs {
    lease_id: BrowserAutomationLeaseId,
    observation_id: BrowserObservationId,
    expected_revision: u64,
    action: BrowserAction,
}

struct BrowserActTool {
    host: Arc<hachimi_browser::BrowserHost>,
    embedded: Arc<crate::embedded_browser_agent::EmbeddedAgentBrowser>,
    store: hachimi_storage::AgentStore,
    session_id: SessionId,
    run_id: hachimi_protocol::RunId,
    grants: CapabilityGrantSet,
    schedule_host_grant: Option<ScheduleHostGrant>,
    environment_change_sink: EnvironmentChangeSink,
}

impl ToolExecutor for BrowserActTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "browser_act".into(),
            description: "Perform one approved Browser action against an exact observation and revision. Navigation, clicks, typing, upload, download, storage, and allowlisted CDP remain separately capability-fenced.".into(),
            input_schema: object_schema(
                json!({
                    "leaseId": { "type": "string" },
                    "observationId": { "type": "string" },
                    "expectedTabRevision": { "type": "integer", "minimum": 1 },
                    "expectedInputEpoch": { "type": "integer", "minimum": 1 },
                    "expectedRevision": { "type": "integer", "minimum": 1 },
                    "action": { "type": "object" }
                }),
                &["leaseId", "observationId", "action"],
            ),
            effect: ToolEffect::BrowserAct,
            parallel_safe: false,
            required_scopes: vec!["browser.control".into()],
        }
    }

    fn execute(&self, invocation: ToolInvocation) -> ToolFuture {
        let host = Arc::clone(&self.host);
        let embedded = Arc::clone(&self.embedded);
        let store = self.store.clone();
        let session_id = self.session_id.clone();
        let run_id = self.run_id.clone();
        let grants = self.grants.clone();
        let schedule_host_grant = self.schedule_host_grant.clone();
        let environment_change_sink = Arc::clone(&self.environment_change_sink);
        Box::pin(async move {
            let requested_lease_id = match invocation
                .call
                .arguments
                .get("leaseId")
                .and_then(Value::as_str)
                .map(BrowserAutomationLeaseId::new)
            {
                Some(lease_id) => lease_id,
                None => return Ok(ToolResult::failed(&invocation.call, "leaseId is required")),
            };
            let external_browser_session_id = match crate::browser_router::route_browser_lease(
                &store,
                &requested_lease_id,
                &session_id,
                &run_id,
                invocation.run_generation,
            )
            .await
            {
                Ok(crate::browser_router::BrowserLeaseRoute::Embedded) => None,
                Ok(crate::browser_router::BrowserLeaseRoute::ExternalChrome {
                    browser_session_id,
                    ..
                }) => Some(browser_session_id),
                Err(error) => return Ok(ToolResult::failed(&invocation.call, error)),
            };
            if let Some(browser_session_id) = external_browser_session_id.as_ref() {
                let current_session = match host.session_snapshot(browser_session_id, &run_id) {
                    Ok(session) if session.owner_session_id == session_id => session,
                    Ok(_) => {
                        return Ok(ToolResult::failed(
                            &invocation.call,
                            "Browser Session ownership changed",
                        ));
                    }
                    Err(error) => {
                        return Ok(ToolResult::failed(&invocation.call, error.to_string()));
                    }
                };
                let required_capability = match serde_json::from_value::<ExternalBrowserActionArgs>(
                    invocation.call.arguments.clone(),
                ) {
                    Ok(request) => request.action.required_capability(),
                    Err(error) => {
                        return Ok(ToolResult::failed(
                            &invocation.call,
                            format!("invalid external Browser action arguments: {error}"),
                        ));
                    }
                };
                if let Err(error) = crate::browser_tool_policy::require_external_session_access(
                    &store,
                    &current_session,
                    schedule_host_grant.as_ref(),
                    invocation.run_generation,
                    &[required_capability],
                )
                .await
                {
                    return Ok(ToolResult::failed(&invocation.call, error));
                }
            }
            if external_browser_session_id.is_none() {
                let request: EmbeddedBrowserActionArgs =
                    match serde_json::from_value(invocation.call.arguments.clone()) {
                        Ok(request) => request,
                        Err(error) => {
                            return Ok(ToolResult::failed(
                                &invocation.call,
                                format!("invalid embedded Browser action arguments: {error}"),
                            ));
                        }
                    };
                if !crate::browser_tool_policy::browser_capability_allowed(
                    &grants,
                    request.action.required_capability(),
                ) {
                    return Ok(ToolResult::failed(
                        &invocation.call,
                        "Browser action exceeds the active Run capability grant",
                    ));
                }
                if let BrowserAction::Navigate { url } | BrowserAction::TabNew { url: Some(url) } =
                    &request.action
                {
                    let origin = match hachimi_browser::normalized_origin(url) {
                        Ok(origin) => origin,
                        Err(error) => {
                            return Ok(ToolResult::failed(&invocation.call, error.to_string()));
                        }
                    };
                    let allow_private_network = if let Some(schedule_browser) = schedule_host_grant
                        .as_ref()
                        .and_then(|grant| grant.browser.as_ref())
                    {
                        if !schedule_browser.enabled
                            || !schedule_browser.document_origins.contains(&origin)
                        {
                            return Ok(ToolResult::failed(
                                &invocation.call,
                                "scheduled Browser navigation exceeds its exact Origin grant",
                            ));
                        }
                        schedule_browser.allow_private_network
                    } else {
                        match crate::browser_tool_policy::require_interactive_browser_access(
                            &store,
                            &session_id,
                            &run_id,
                            invocation.run_generation,
                            url,
                            BrowserAutomationSurfaceKind::Embedded,
                            &[BrowserCapability::Observe, BrowserCapability::Act],
                        )
                        .await
                        {
                            Ok(allow_private_network) => allow_private_network,
                            Err(error) => {
                                return Ok(ToolResult::failed(&invocation.call, error));
                            }
                        }
                    };
                    if let Err(error) =
                        hachimi_browser::validate_agent_browser_target(url, allow_private_network)
                            .await
                    {
                        return Ok(ToolResult::failed(&invocation.call, error.to_string()));
                    }
                }
                return match embedded
                    .act(
                        crate::embedded_browser_agent::EmbeddedBrowserActionRequest {
                            lease_id: &request.lease_id,
                            run_id: &run_id,
                            run_generation: invocation.run_generation,
                            observation_id: &request.observation_id,
                            expected_tab_revision: request.expected_tab_revision,
                            expected_input_epoch: request.expected_input_epoch,
                            action: &request.action,
                            origin_policy: crate::browser_tool_policy::embedded_origin_policy(
                                &grants,
                                schedule_host_grant.as_ref(),
                            ),
                        },
                    )
                    .await
                {
                    Ok(output) => {
                        environment_change_sink(session_id.clone());
                        let content = json!({
                            "leaseId": request.lease_id,
                            "accepted": true,
                            "resultCode": "performed",
                            "output": output,
                        });
                        Ok(ToolResult::succeeded(
                            &invocation.call,
                            content.to_string(),
                            content,
                        ))
                    }
                    Err(error) => Ok(ToolResult::failed(&invocation.call, error.to_string())),
                };
            }
            let browser_session_id = external_browser_session_id.expect("external route checked");
            let args = match serde_json::from_value::<ExternalBrowserActionArgs>(
                invocation.call.arguments.clone(),
            ) {
                Ok(args) if args.lease_id == requested_lease_id => args,
                Ok(_) => return Ok(ToolResult::failed(&invocation.call, "leaseId changed")),
                Err(error) => {
                    return Ok(ToolResult::failed(
                        &invocation.call,
                        format!("invalid external Chrome lease action arguments: {error}"),
                    ));
                }
            };
            let mut request = BrowserActionRequest {
                browser_session_id,
                observation_id: args.observation_id,
                run_generation: invocation.run_generation,
                expected_revision: args.expected_revision,
                action: args.action,
            };
            request.run_generation = invocation.run_generation;
            let action_category =
                crate::browser_tool_policy::browser_action_category(&request.action);
            let target_origin = match &request.action {
                BrowserAction::Navigate { url } => hachimi_browser::normalized_origin(url).ok(),
                BrowserAction::TabNew { url: Some(url) } => {
                    hachimi_browser::normalized_origin(url).ok()
                }
                _ => host
                    .session_snapshot(&request.browser_session_id, &run_id)
                    .ok()
                    .and_then(|session| session.origin),
            };
            let capability_allowed = match &request.action {
                BrowserAction::Upload { .. } => grants.browser.upload,
                BrowserAction::Download { .. } => grants.browser.download,
                BrowserAction::ReadStorage | BrowserAction::WriteStorage { .. } => {
                    grants.browser.cookie_storage
                }
                BrowserAction::Cdp { .. } => grants.browser.cdp,
                _ => match request.action.required_capability() {
                    BrowserCapability::Observe => grants.browser.observe,
                    BrowserCapability::Act => grants.browser.act,
                    BrowserCapability::Upload => grants.browser.upload,
                    BrowserCapability::Download => grants.browser.download,
                    BrowserCapability::CookieStorage => grants.browser.cookie_storage,
                    BrowserCapability::Cdp => grants.browser.cdp,
                },
            };
            let schedule_capability_allowed = schedule_host_grant
                .as_ref()
                .map(|grant| {
                    grant.browser.as_ref().is_some_and(|browser| {
                        browser.enabled
                            && browser
                                .capabilities
                                .contains(&request.action.required_capability())
                    })
                })
                .unwrap_or(true);
            if !capability_allowed || !schedule_capability_allowed {
                return Ok(ToolResult::failed(
                    &invocation.call,
                    "Browser action exceeds the active Run capability grant",
                ));
            }
            if let BrowserAction::Navigate { url } | BrowserAction::TabNew { url: Some(url) } =
                &request.action
            {
                let target_origin = match hachimi_browser::normalized_origin(url) {
                    Ok(origin) => origin,
                    Err(error) => {
                        return Ok(ToolResult::failed(&invocation.call, error.to_string()));
                    }
                };
                let origin_allowed = schedule_host_grant.as_ref().map_or_else(
                    || true,
                    |grant| {
                        grant.browser.as_ref().is_some_and(|browser| {
                            browser.enabled && browser.document_origins.contains(&target_origin)
                        })
                    },
                );
                if !origin_allowed {
                    return Ok(ToolResult::failed(
                        &invocation.call,
                        "Cross-origin navigation exceeds the exact active Run origin grant",
                    ));
                }
                let session = match host.session_snapshot(&request.browser_session_id, &run_id) {
                    Ok(session) if session.owner_session_id == session_id => session,
                    Ok(_) => {
                        return Ok(ToolResult::failed(
                            &invocation.call,
                            "Browser Session ownership changed",
                        ));
                    }
                    Err(error) => {
                        return Ok(ToolResult::failed(&invocation.call, error.to_string()));
                    }
                };
                let mut capabilities = vec![BrowserCapability::Act];
                if grants.browser.observe {
                    capabilities.push(BrowserCapability::Observe);
                }
                let network_kinds = if schedule_host_grant.is_some() {
                    vec![BrowserNetworkRuleKind::Document]
                } else {
                    vec![
                        BrowserNetworkRuleKind::Document,
                        BrowserNetworkRuleKind::Resource,
                    ]
                };
                let allow_private_network = if let Some(schedule_browser) = schedule_host_grant
                    .as_ref()
                    .and_then(|grant| grant.browser.as_ref())
                {
                    schedule_browser.allow_private_network
                } else {
                    match crate::browser_tool_policy::require_interactive_browser_access(
                        &store,
                        &session_id,
                        &run_id,
                        invocation.run_generation,
                        url,
                        BrowserAutomationSurfaceKind::ExternalChrome,
                        &capabilities,
                    )
                    .await
                    {
                        Ok(allow_private_network) => allow_private_network,
                        Err(error) => return Ok(ToolResult::failed(&invocation.call, error)),
                    }
                };
                if let Err(error) =
                    hachimi_browser::validate_agent_browser_target(url, allow_private_network).await
                {
                    return Ok(ToolResult::failed(&invocation.call, error.to_string()));
                }
                for network_kind in network_kinds {
                    if let Err(error) = host
                        .grant_site_permission(
                            &session.id,
                            &session.owner_session_id,
                            &run_id,
                            session.revision,
                            &target_origin,
                            capabilities.clone(),
                            BrowserPermissionDecision::AllowSession,
                            network_kind,
                            allow_private_network,
                            if schedule_host_grant.is_some() {
                                "schedule:approved-navigation"
                            } else {
                                "run:approved-navigation"
                            },
                            Some(now_ms().saturating_add(10 * 60 * 1_000)),
                        )
                        .await
                    {
                        return Ok(ToolResult::failed(&invocation.call, error.to_string()));
                    }
                }
            }
            match host.authorize_action(&run_id, &request).await {
                Ok(mut result) => {
                    let observation = if result.accepted && grants.browser.observe {
                        host.observe(
                            &request.browser_session_id,
                            &run_id,
                            invocation.run_generation,
                        )
                        .await
                        .ok()
                    } else {
                        None
                    };
                    if let Ok(session) = host.session_snapshot(&request.browser_session_id, &run_id)
                    {
                        result.revision = session.revision;
                        persist_browser_environment(
                            &store,
                            &environment_change_sink,
                            &session,
                            observation
                                .as_ref()
                                .map(|value| value.url.as_str())
                                .or(session.current_url.as_deref()),
                            observation.as_ref().map(|value| value.title.as_str()),
                        )
                        .await?;
                    }
                    append_local_host_audit(
                        &store,
                        &session_id,
                        &run_id,
                        "browser.act",
                        browser_target_summary(
                            target_origin.as_deref().unwrap_or("unknown"),
                            action_category,
                        ),
                        if result.accepted {
                            "succeeded"
                        } else {
                            "denied"
                        },
                        &result.result_code,
                    )
                    .await?;
                    let mut content = serde_json::to_value(&result).map_err(|error| {
                        hachimi_agent::ToolExecutionError::Failed(error.to_string())
                    })?;
                    if let Value::Object(fields) = &mut content {
                        fields.insert("leaseId".into(), json!(requested_lease_id));
                    }
                    let model_content = content.to_string();
                    Ok(ToolResult::succeeded(
                        &invocation.call,
                        model_content,
                        content,
                    ))
                }
                Err(error) => {
                    append_local_host_audit(
                        &store,
                        &session_id,
                        &run_id,
                        "browser.act",
                        browser_target_summary(
                            target_origin.as_deref().unwrap_or("unknown"),
                            action_category,
                        ),
                        "denied",
                        browser_error_code(&error),
                    )
                    .await?;
                    Ok(ToolResult::failed(&invocation.call, error.to_string()))
                }
            }
        })
    }
}

struct BrowserStopTool {
    host: Arc<hachimi_browser::BrowserHost>,
    embedded: Arc<crate::embedded_browser_agent::EmbeddedAgentBrowser>,
    store: hachimi_storage::AgentStore,
    session_id: SessionId,
    run_id: hachimi_protocol::RunId,
    environment_change_sink: EnvironmentChangeSink,
}

impl ToolExecutor for BrowserStopTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "browser_stop".into(),
            description: "Release a Run-owned Browser automation lease without closing the user's persistent embedded tab.".into(),
            input_schema: object_schema(json!({ "leaseId": { "type": "string" } }), &["leaseId"]),
            effect: ToolEffect::BrowserObserve,
            parallel_safe: false,
            required_scopes: vec!["browser.observe".into()],
        }
    }

    fn execute(&self, invocation: ToolInvocation) -> ToolFuture {
        let host = Arc::clone(&self.host);
        let embedded = Arc::clone(&self.embedded);
        let store = self.store.clone();
        let session_id = self.session_id.clone();
        let run_id = self.run_id.clone();
        let environment_change_sink = Arc::clone(&self.environment_change_sink);
        Box::pin(async move {
            let args: BrowserSessionArgs =
                match serde_json::from_value(invocation.call.arguments.clone()) {
                    Ok(args) => args,
                    Err(error) => {
                        return Ok(ToolResult::failed(
                            &invocation.call,
                            format!("invalid Browser stop arguments: {error}"),
                        ));
                    }
                };
            let lease_id = args.lease_id;
            let route = match crate::browser_router::route_browser_lease(
                &store,
                &lease_id,
                &session_id,
                &run_id,
                invocation.run_generation,
            )
            .await
            {
                Ok(route) => route,
                Err(error) => return Ok(ToolResult::failed(&invocation.call, error)),
            };
            let stopped = match route {
                crate::browser_router::BrowserLeaseRoute::Embedded => embedded
                    .stop(&lease_id, &run_id, invocation.run_generation)
                    .await
                    .map_err(|error| error.to_string()),
                crate::browser_router::BrowserLeaseRoute::ExternalChrome {
                    lease,
                    browser_session_id,
                } => match host.stop(&browser_session_id, &run_id).await {
                    Ok(session) => {
                        persist_browser_environment(
                            &store,
                            &environment_change_sink,
                            &session,
                            None,
                            None,
                        )
                        .await?;
                        store
                            .set_browser_automation_lease_status(
                                &lease.id,
                                lease.revision,
                                hachimi_protocol::BrowserAutomationLeaseStatus::Expired,
                            )
                            .await
                            .map_err(|error| error.to_string())
                    }
                    Err(error) => Err(error.to_string()),
                },
            };
            match stopped {
                Ok(lease) => {
                    let content = serde_json::to_value(&lease).map_err(|error| {
                        hachimi_agent::ToolExecutionError::Failed(error.to_string())
                    })?;
                    Ok(ToolResult::succeeded(
                        &invocation.call,
                        content.to_string(),
                        content,
                    ))
                }
                Err(error) => Ok(ToolResult::failed(&invocation.call, error)),
            }
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ComputerObserveArgs {
    app_name: String,
    #[serde(default)]
    window_title: Option<String>,
}

#[derive(Debug)]
struct ComputerListWindowsTool {
    host: Arc<hachimi_computer::ComputerHost>,
}

impl ToolExecutor for ComputerListWindowsTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "computer_list_windows".into(),
            description: "List visible, non-elevated application names and window titles that are eligible for Computer authorization. Process IDs and window handles are intentionally hidden; listing does not grant Observe or Act.".into(),
            input_schema: object_schema(json!({}), &[]),
            effect: ToolEffect::ComputerObserve,
            parallel_safe: false,
            required_scopes: vec!["computer.observe".into()],
        }
    }

    fn execute(&self, invocation: ToolInvocation) -> ToolFuture {
        let host = Arc::clone(&self.host);
        Box::pin(async move {
            match host.list_windows().await {
                Ok(windows) => {
                    let candidates = windows
                        .into_iter()
                        .map(|window| {
                            json!({
                                "appName": window.app_id,
                                "windowTitle": window.title,
                            })
                        })
                        .collect::<Vec<_>>();
                    Ok(ToolResult::succeeded(
                        &invocation.call,
                        "Visible semantic Computer targets listed; no application rule was changed.",
                        json!({ "windows": candidates }),
                    ))
                }
                Err(error) => Ok(ToolResult::failed(&invocation.call, error.to_string())),
            }
        })
    }
}

#[derive(Debug)]
struct ComputerObserveTool {
    host: Arc<hachimi_computer::ComputerHost>,
    store: hachimi_storage::AgentStore,
    session_id: SessionId,
    run_id: hachimi_protocol::RunId,
    grants: CapabilityGrantSet,
    sandbox: hachimi_protocol::SandboxCapabilityReport,
}

impl ToolExecutor for ComputerObserveTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "computer_observe".into(),
            description: "Resolve an application by name and optional window title, then capture a fresh frame for the user-authorized, non-elevated window. Never ask the user for a PID or window handle. If application identity is ambiguous, use request_user_input to ask for the application name. The returned frame ID, fingerprint, and input epoch must be used by the next Computer action.".into(),
            input_schema: object_schema(
                json!({
                    "appName": { "type": "string", "minLength": 1, "maxLength": 256 },
                    "windowTitle": { "type": "string", "minLength": 1, "maxLength": 512 }
                }),
                &["appName"],
            ),
            effect: ToolEffect::ComputerObserve,
            parallel_safe: false,
            required_scopes: vec!["computer.observe".into()],
        }
    }

    fn execute(&self, invocation: ToolInvocation) -> ToolFuture {
        let host = Arc::clone(&self.host);
        let store = self.store.clone();
        let session_id = self.session_id.clone();
        let run_id = self.run_id.clone();
        let grants = self.grants.clone();
        let sandbox = self.sandbox.clone();
        Box::pin(async move {
            let args: ComputerObserveArgs =
                match serde_json::from_value(invocation.call.arguments.clone()) {
                    Ok(args) => args,
                    Err(error) => {
                        return Ok(ToolResult::failed(
                            &invocation.call,
                            format!("invalid Computer observe arguments: {error}"),
                        ));
                    }
                };
            if args.app_name.trim().is_empty() || args.app_name.len() > 256 {
                return Ok(ToolResult::failed(
                    &invocation.call,
                    "appName must contain 1-256 bytes",
                ));
            }
            if args
                .window_title
                .as_ref()
                .is_some_and(|value| value.trim().is_empty() || value.len() > 512)
            {
                return Ok(ToolResult::failed(
                    &invocation.call,
                    "windowTitle must contain 1-512 bytes when provided",
                ));
            }
            let settings = sqlx::query(
                "SELECT automation_enabled FROM computer_host_settings WHERE singleton = 1",
            )
            .fetch_one(store.pool())
            .await
            .map_err(|error| hachimi_agent::ToolExecutionError::Failed(error.to_string()))?;
            if !settings.get::<bool, _>("automation_enabled") {
                return Ok(ToolResult::failed(
                    &invocation.call,
                    "Computer Use automation is disabled in Settings",
                ));
            }
            let windows = match host.list_windows().await {
                Ok(windows) => windows,
                Err(error) => return Ok(ToolResult::failed(&invocation.call, error.to_string())),
            };
            let target = match hachimi_computer::resolve_window_target(
                &windows,
                &args.app_name,
                args.window_title.as_deref(),
            ) {
                Ok(target) => target,
                Err(error) => {
                    return Ok(ToolResult::failed(
                        &invocation.call,
                        crate::computer_targeting::resolution_message(&error),
                    ));
                }
            };
            let app = target.app.clone();
            match store
                .computer_host_policy_decision(&app, &session_id, &run_id)
                .await
                .map_err(|error| hachimi_agent::ToolExecutionError::Failed(error.to_string()))?
            {
                HostPolicyDecision::Block => {
                    return Ok(ToolResult::failed(
                        &invocation.call,
                        format!(
                            "Computer application is blocked by policy: {}",
                            app.display_name
                        ),
                    ));
                }
                HostPolicyDecision::Ask => {
                    let request = store
                        .create_computer_host_access_request(
                            &session_id,
                            &run_id,
                            invocation.run_generation,
                            &app,
                        )
                        .await
                        .map_err(|error| {
                            hachimi_agent::ToolExecutionError::Failed(error.to_string())
                        })?;
                    let _ = store
                        .append_event(
                            &session_id,
                            Some(&run_id),
                            "host.access_required",
                            serde_json::to_value(&request).map_err(|error| {
                                hachimi_agent::ToolExecutionError::Failed(error.to_string())
                            })?,
                        )
                        .await;
                    return Ok(ToolResult::failed(
                        &invocation.call,
                        format!(
                            "Computer application access requires user confirmation: {}",
                            request.id
                        ),
                    ));
                }
                HostPolicyDecision::Allow => {}
            }
            host.set_app_rule(
                &session_id,
                ComputerAppRule {
                    app_id: app.app_id.clone(),
                    observe: true,
                    act: grants.computer.act,
                    always_allowed: false,
                    granted_by: "host:resolved-computer-policy".into(),
                    updated_at_ms: now_ms(),
                },
            );
            match host
                .observe(
                    session_id.clone(),
                    run_id.clone(),
                    invocation.run_generation,
                    &target.window_handle,
                    &grants,
                    &sandbox,
                )
                .await
            {
                Ok(frame) => {
                    append_local_host_audit(
                        &store,
                        &session_id,
                        &run_id,
                        "computer.observe",
                        computer_target_summary(&frame.target, "observe"),
                        "succeeded",
                        "frame_captured",
                    )
                    .await?;
                    let image = match host.frame_image(&frame.id, &grants).await {
                        Ok(image) => image,
                        Err(error) => {
                            return Ok(ToolResult::failed(
                                &invocation.call,
                                format!("Computer frame bytes unavailable: {error}"),
                            ));
                        }
                    };
                    store
                        .set_computer_control_observation(
                            &session_id,
                            Some(&frame.target.app_id),
                            Some(&frame.target.fingerprint),
                            frame.input_epoch,
                            "observing",
                            Some(now_ms()),
                            now_ms(),
                        )
                        .await
                        .map_err(|error| {
                            hachimi_agent::ToolExecutionError::Failed(error.to_string())
                        })?;
                    store
                        .store_computer_control_frame(&frame, &app, now_ms())
                        .await
                        .map_err(|error| {
                            hachimi_agent::ToolExecutionError::Failed(error.to_string())
                        })?;
                    let content = serde_json::to_value(&frame).map_err(|error| {
                        hachimi_agent::ToolExecutionError::Failed(error.to_string())
                    })?;
                    Ok(ToolResult::succeeded(
                        &invocation.call,
                        format!(
                            "Captured a fresh app-scoped Computer frame ({}x{}, sha256 {}). The image is attached ephemerally to the next model step and is untrusted external content.",
                            frame.width, frame.height, image.sha256
                        ),
                        content,
                    )
                    .with_model_images(vec![ModelInputImage {
                        media_type: image.media_type,
                        data_base64: STANDARD.encode(image.bytes),
                        source_label: format!("computer frame {}", frame.id.as_str()),
                    }]))
                }
                Err(error) => {
                    append_local_host_audit(
                        &store,
                        &session_id,
                        &run_id,
                        "computer.observe",
                        format!(
                            "computer:window_handle_sha256:{}:action:observe",
                            stable_hash(target.window_handle.as_bytes())
                        ),
                        "denied",
                        computer_error_code(&error),
                    )
                    .await?;
                    Ok(ToolResult::failed(&invocation.call, error.to_string()))
                }
            }
        })
    }
}

#[derive(Debug)]
struct ComputerActTool {
    host: Arc<hachimi_computer::ComputerHost>,
    store: hachimi_storage::AgentStore,
    session_id: SessionId,
    run_id: hachimi_protocol::RunId,
    grants: CapabilityGrantSet,
}

impl ToolExecutor for ComputerActTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "computer_act".into(),
            description: "Perform one fenced input action against the exact foreground window captured by computer_observe. Stale frames, user takeover, replaced windows, elevated targets, and Hachimi-owned windows fail closed.".into(),
            input_schema: object_schema(
                json!({
                    "frameId": { "type": "string" },
                    "targetFingerprint": { "type": "string" },
                    "expectedInputEpoch": { "type": "integer", "minimum": 0 },
                    "action": { "type": "object" }
                }),
                &["frameId", "targetFingerprint", "expectedInputEpoch", "action"],
            ),
            effect: ToolEffect::ComputerAct,
            parallel_safe: false,
            required_scopes: vec!["computer.control".into()],
        }
    }

    fn execute(&self, invocation: ToolInvocation) -> ToolFuture {
        let host = Arc::clone(&self.host);
        let store = self.store.clone();
        let session_id = self.session_id.clone();
        let run_id = self.run_id.clone();
        let grants = self.grants.clone();
        Box::pin(async move {
            let mut request: ComputerActionRequest =
                match serde_json::from_value(invocation.call.arguments.clone()) {
                    Ok(request) => request,
                    Err(error) => {
                        return Ok(ToolResult::failed(
                            &invocation.call,
                            format!("invalid Computer action arguments: {error}"),
                        ));
                    }
                };
            request.run_generation = invocation.run_generation;
            let frame = host.frame_snapshot(&request.frame_id);
            let action_category = computer_action_category(&request.action);
            match host.act(&request, &grants).await {
                Ok(result) => {
                    if let Some(frame) = frame.as_ref() {
                        store
                            .set_computer_control_observation(
                                &session_id,
                                Some(&frame.target.app_id),
                                Some(&frame.target.fingerprint),
                                result.next_input_epoch,
                                "observing",
                                Some(now_ms()),
                                now_ms(),
                            )
                            .await
                            .map_err(|error| {
                                hachimi_agent::ToolExecutionError::Failed(error.to_string())
                            })?;
                    }
                    let target_summary = frame.as_ref().map_or_else(
                        || {
                            format!(
                                "computer:frame_sha256:{}:action:{action_category}",
                                stable_hash(request.frame_id.as_str().as_bytes())
                            )
                        },
                        |frame| computer_target_summary(&frame.target, action_category),
                    );
                    append_local_host_audit(
                        &store,
                        &session_id,
                        &run_id,
                        "computer.act",
                        target_summary,
                        "succeeded",
                        &result.result_code,
                    )
                    .await?;
                    let content = serde_json::to_value(&result).map_err(|error| {
                        hachimi_agent::ToolExecutionError::Failed(error.to_string())
                    })?;
                    Ok(ToolResult::succeeded(
                        &invocation.call,
                        "Computer action completed.",
                        content,
                    ))
                }
                Err(error) => {
                    let target_summary = frame.as_ref().map_or_else(
                        || {
                            format!(
                                "computer:fingerprint_sha256:{}:action:{action_category}",
                                stable_hash(request.target_fingerprint.as_bytes())
                            )
                        },
                        |frame| computer_target_summary(&frame.target, action_category),
                    );
                    append_local_host_audit(
                        &store,
                        &session_id,
                        &run_id,
                        "computer.act",
                        target_summary,
                        "denied",
                        computer_error_code(&error),
                    )
                    .await?;
                    Ok(ToolResult::failed(&invocation.call, error.to_string()))
                }
            }
        })
    }
}

#[derive(Debug)]
struct ComputerStopTool {
    host: Arc<hachimi_computer::ComputerHost>,
    store: hachimi_storage::AgentStore,
    session_id: SessionId,
}

impl ToolExecutor for ComputerStopTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "computer_stop".into(),
            description:
                "Stop Computer control for this Session and invalidate every outstanding frame."
                    .into(),
            input_schema: object_schema(json!({}), &[]),
            effect: ToolEffect::ComputerObserve,
            parallel_safe: false,
            required_scopes: vec!["computer.observe".into()],
        }
    }

    fn execute(&self, invocation: ToolInvocation) -> ToolFuture {
        let host = Arc::clone(&self.host);
        let store = self.store.clone();
        let session_id = self.session_id.clone();
        Box::pin(async move {
            let epoch = host.take_over(&session_id);
            store
                .set_computer_control_observation(
                    &session_id,
                    None,
                    None,
                    epoch,
                    "stopped",
                    None,
                    now_ms(),
                )
                .await
                .map_err(|error| hachimi_agent::ToolExecutionError::Failed(error.to_string()))?;
            Ok(ToolResult::succeeded(
                &invocation.call,
                "Computer control stopped.",
                json!({ "inputEpoch": epoch }),
            ))
        })
    }
}

#[derive(Debug)]
struct ConnectorListTool {
    host: hachimi_extensions::PluginHost,
    schedule_host_grant: Option<ScheduleHostGrant>,
}

impl ToolExecutor for ConnectorListTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "connector_list_accounts".into(),
            description: "List local Connector account metadata and pinned Host/Schema/Action revisions. Secrets are never returned.".into(),
            input_schema: object_schema(json!({}), &[]),
            effect: ToolEffect::ReadOnly,
            parallel_safe: true,
            required_scopes: vec!["connectors.invoke".into()],
        }
    }

    fn execute(&self, invocation: ToolInvocation) -> ToolFuture {
        let host = self.host.clone();
        let schedule_host_grant = self.schedule_host_grant.clone();
        Box::pin(async move {
            match host.list_connector_accounts().await {
                Ok(mut accounts) => {
                    if let Some(grant) = &schedule_host_grant {
                        accounts.retain(|account| {
                            grant
                                .connectors
                                .iter()
                                .any(|selection| selection.account_id == account.id)
                        });
                    }
                    for account in &mut accounts {
                        account.secret_ref = None;
                    }
                    let content = serde_json::to_value(&accounts).map_err(|error| {
                        hachimi_agent::ToolExecutionError::Failed(error.to_string())
                    })?;
                    let model_content = content.to_string();
                    Ok(ToolResult::succeeded(
                        &invocation.call,
                        model_content,
                        content,
                    ))
                }
                Err(error) => Ok(ToolResult::failed(&invocation.call, error.to_string())),
            }
        })
    }
}

struct ConnectorInvokeTool {
    host: hachimi_extensions::PluginHost,
    store: hachimi_storage::AgentStore,
    session_id: SessionId,
    run_id: hachimi_protocol::RunId,
    schedule_host_grant: Option<ScheduleHostGrant>,
    environment_change_sink: EnvironmentChangeSink,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnterpriseAttachmentDownloadArgs {
    provider_id: IntegrationProviderId,
    account_id: String,
    event_id: String,
    remote_id: String,
    metadata_hash: String,
    idempotency_key: String,
}

#[derive(Debug)]
struct EnterpriseAttachmentDownloadTool {
    host: hachimi_extensions::PluginHost,
    store: hachimi_storage::AgentStore,
    session_id: SessionId,
    run_id: hachimi_protocol::RunId,
    schedule_host_grant: Option<ScheduleHostGrant>,
}

impl ToolExecutor for EnterpriseAttachmentDownloadTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "enterprise.download_attachment".into(),
            description: "Explicitly download one enterprise message attachment into the managed Artifact store after size, MIME, magic, metadata, Run, generation, and idempotency checks.".into(),
            input_schema: object_schema(
                json!({
                    "providerId": { "type": "string", "enum": ["wecom_app", "dingtalk", "feishu"] },
                    "accountId": { "type": "string", "minLength": 1, "maxLength": 128 },
                    "eventId": { "type": "string", "minLength": 1, "maxLength": 512 },
                    "remoteId": { "type": "string", "minLength": 1, "maxLength": 1024 },
                    "metadataHash": { "type": "string", "pattern": "^[0-9a-f]{64}$" },
                    "idempotencyKey": { "type": "string", "minLength": 1, "maxLength": 128 }
                }),
                &["providerId", "accountId", "eventId", "remoteId", "metadataHash", "idempotencyKey"],
            ),
            effect: ToolEffect::ExternalSideEffect,
            parallel_safe: false,
            required_scopes: vec!["connectors.invoke".into()],
        }
    }

    fn execute(&self, invocation: ToolInvocation) -> ToolFuture {
        let host = self.host.clone();
        let store = self.store.clone();
        let session_id = self.session_id.clone();
        let run_id = self.run_id.clone();
        let schedule_host_grant = self.schedule_host_grant.clone();
        Box::pin(async move {
            let args: EnterpriseAttachmentDownloadArgs =
                match serde_json::from_value(invocation.call.arguments.clone()) {
                    Ok(args) => args,
                    Err(error) => {
                        return Ok(ToolResult::failed(
                            &invocation.call,
                            format!("enterprise_attachment_arguments_invalid:{error}"),
                        ));
                    }
                };
            if let Some(grant) = &schedule_host_grant
                && let Err(error) =
                    crate::schedule_host_grants::authorize_enterprise_attachment_download(
                        &host,
                        &grant.connectors,
                        &args.account_id,
                    )
                    .await
            {
                store
                    .append_event(
                        &session_id,
                        Some(&run_id),
                        crate::schedule_host_grants::SCHEDULE_HOST_GRANT_ATTENTION_EVENT,
                        json!({ "code": error.code, "message": error.message }),
                    )
                    .await
                    .map_err(|store_error| {
                        hachimi_agent::ToolExecutionError::Failed(format!(
                            "schedule_host_grant_event_failed:{store_error}"
                        ))
                    })?;
                return Err(hachimi_agent::ToolExecutionError::Failed(format!(
                    "{}:{}",
                    error.code, error.message
                )));
            }
            let request = EnterpriseAttachmentDownloadRequest {
                context: MutationContext {
                    request_id: RequestId(invocation.call.id.as_str().to_owned()),
                    client_id: ClientId("agent-runtime".into()),
                    protocol_version: hachimi_protocol::CONTROL_PROTOCOL_VERSION,
                    idempotency_key: args.idempotency_key.clone(),
                    expected_run_id: Some(run_id.clone()),
                    expected_generation: Some(invocation.run_generation),
                },
                provider_id: args.provider_id,
                account_id: args.account_id,
                event_id: args.event_id,
                remote_id: args.remote_id,
                metadata_hash: args.metadata_hash,
                run_id,
                run_generation: invocation.run_generation,
                idempotency_key: args.idempotency_key,
            };
            match host.download_enterprise_attachment(&request).await {
                Ok(result) => {
                    let content = serde_json::to_value(&result).map_err(|error| {
                        hachimi_agent::ToolExecutionError::Failed(error.to_string())
                    })?;
                    Ok(ToolResult::succeeded(
                        &invocation.call,
                        content.to_string(),
                        content,
                    ))
                }
                Err(error) => Ok(ToolResult::failed(
                    &invocation.call,
                    format!("{}:{error}", error.code()),
                )),
            }
        })
    }
}

impl ToolExecutor for ConnectorInvokeTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "connector_invoke".into(),
            description: "Invoke one deterministic local Connector action with an idempotency key and exact pinned Host/Schema/Action revision.".into(),
            input_schema: object_schema(
                json!({
                    "accountId": { "type": "string" },
                    "action": { "type": "string" },
                    "arguments": { "type": "object" },
                    "idempotencyKey": { "type": "string", "minLength": 1, "maxLength": 128 },
                    "expectedRevision": {
                        "type": "object",
                        "properties": {
                            "hostIdentityHash": { "type": "string" },
                            "schemaHash": { "type": "string" },
                            "actionHash": { "type": "string" }
                        },
                        "required": ["hostIdentityHash", "schemaHash", "actionHash"],
                        "additionalProperties": false
                    }
                }),
                &["accountId", "action", "arguments", "idempotencyKey", "expectedRevision"],
            ),
            effect: ToolEffect::ExternalSideEffect,
            parallel_safe: false,
            required_scopes: vec!["connectors.invoke".into()],
        }
    }

    fn execute(&self, invocation: ToolInvocation) -> ToolFuture {
        let host = self.host.clone();
        let store = self.store.clone();
        let session_id = self.session_id.clone();
        let run_id = self.run_id.clone();
        let schedule_host_grant = self.schedule_host_grant.clone();
        let environment_change_sink = Arc::clone(&self.environment_change_sink);
        Box::pin(async move {
            let request: ConnectorInvocationRequest =
                match serde_json::from_value(invocation.call.arguments.clone()) {
                    Ok(request) => request,
                    Err(error) => {
                        return Ok(ToolResult::failed(
                            &invocation.call,
                            format!("invalid Connector arguments: {error}"),
                        ));
                    }
                };
            if request.action == crate::schedule_host_grants::ENTERPRISE_ATTACHMENT_ACTION {
                return Ok(ToolResult::rejected(
                    &invocation.call,
                    "download_attachment is available only through enterprise.download_attachment",
                ));
            }
            if let Some(grant) = &schedule_host_grant {
                let Some(selection) = grant
                    .connectors
                    .iter()
                    .find(|selection| selection.account_id == request.account_id)
                else {
                    return Ok(ToolResult::failed(
                        &invocation.call,
                        "Connector account is outside the persisted ScheduleHostGrant",
                    ));
                };
                let revision = &selection.contribution_revision;
                let revision_matches = revision.host_identity_hash.as_deref()
                    == Some(request.expected_revision.host_identity_hash.as_str())
                    && revision.schema_hash.as_deref()
                        == Some(request.expected_revision.schema_hash.as_str())
                    && revision.action_hash.as_deref()
                        == Some(request.expected_revision.action_hash.as_str());
                if !selection.allowed_actions.contains(&request.action) || !revision_matches {
                    return Ok(ToolResult::failed(
                        &invocation.call,
                        "Connector action or revision is outside the persisted ScheduleHostGrant",
                    ));
                }
            }
            match host.invoke_connector(&request).await {
                Ok(result) => {
                    if let Some((source_url, title)) = connector_source(&result.metadata)
                        && store
                            .upsert_session_web_source(
                                &session_id,
                                Some(&run_id),
                                SessionSourceOrigin::Connector,
                                &source_url,
                                title.as_deref(),
                                None,
                            )
                            .await
                            .is_ok()
                    {
                        environment_change_sink(session_id.clone());
                    }
                    let content = serde_json::to_value(&result).map_err(|error| {
                        hachimi_agent::ToolExecutionError::Failed(error.to_string())
                    })?;
                    let model_content = content.to_string();
                    Ok(ToolResult::succeeded(
                        &invocation.call,
                        model_content,
                        content,
                    ))
                }
                Err(error) => Ok(ToolResult::failed(&invocation.call, error.to_string())),
            }
        })
    }
}

async fn persist_browser_environment(
    store: &hachimi_storage::AgentStore,
    environment_change_sink: &EnvironmentChangeSink,
    session: &hachimi_protocol::BrowserSession,
    url: Option<&str>,
    title: Option<&str>,
) -> Result<(), hachimi_agent::ToolExecutionError> {
    store
        .upsert_session_browser(session)
        .await
        .map_err(|error| hachimi_agent::ToolExecutionError::Failed(error.to_string()))?;
    if let Some(url) = url
        .or(session.current_url.as_deref())
        .and_then(hachimi_storage::canonical_session_source_url)
    {
        store
            .upsert_session_web_source(
                &session.owner_session_id,
                Some(&session.owner_run_id),
                SessionSourceOrigin::Browser,
                &url,
                title,
                None,
            )
            .await
            .map_err(|error| hachimi_agent::ToolExecutionError::Failed(error.to_string()))?;
    }
    environment_change_sink(session.owner_session_id.clone());
    Ok(())
}

async fn append_local_host_audit(
    store: &hachimi_storage::AgentStore,
    session_id: &SessionId,
    run_id: &hachimi_protocol::RunId,
    operation: &str,
    target_summary: String,
    decision: &str,
    result_code: &str,
) -> Result<(), hachimi_agent::ToolExecutionError> {
    let run_generation = store
        .get_run(run_id)
        .await
        .map_err(|error| hachimi_agent::ToolExecutionError::Failed(error.to_string()))?
        .map(|run| run.generation);
    store
        .append_audit_metadata(hachimi_storage::AuditMetadataRecord {
            principal: "local-host-broker".into(),
            session_id: Some(session_id.clone()),
            run_id: Some(run_id.clone()),
            run_generation,
            operation: operation.into(),
            target_summary,
            decision: decision.into(),
            result_code: result_code.into(),
            created_at_ms: now_ms(),
        })
        .await
        .map_err(|error| hachimi_agent::ToolExecutionError::Failed(error.to_string()))
}

#[cfg(test)]
#[path = "agent_host_tools_audit_tests.rs"]
mod audit_tests;

#[cfg(test)]
#[path = "agent_host_tools_schedule_tests.rs"]
mod schedule_tests;
