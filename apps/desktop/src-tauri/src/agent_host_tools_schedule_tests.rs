use std::{
    fs,
    io::{Read as _, Write as _},
    net::TcpListener,
    thread,
};

use hachimi_agent::{
    AgentRunCreateRequest, AgentRunFactory, ToolCall, ToolExecutor, ToolInvocation,
    ToolResultStatus,
};
use hachimi_enterprise::EnterpriseApiClient;
use hachimi_protocol::{
    ApprovalPolicy, BehaviorMode, ConnectorAccountId, ConnectorAccountUpsert, ContributionRevision,
    EntryProfile, LlmSettings, ModelToolCall, PermissionProfile, PluginContribution,
    PluginContributionKind, PluginId, PluginManifest, ProviderCapabilities, RunBudget,
    RunEventPayload, RunOrigin, RunPurpose, ScheduleConnectorSelection, ScheduleHostGrant,
    SessionContextBinding, ToolCallId, WorkloadKind,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::EnterpriseAttachmentDownloadTool;

const CONNECTOR_ID: &str = "sample-crm";
const INTEGRATION_ID: &str = "connector:scheduled-attachment-account";
const EVENT_ID: &str = "scheduled-event-1";
const REMOTE_ID: &str = "scheduled-remote-1";

struct Fixture {
    _source: tempfile::TempDir,
    _installs: tempfile::TempDir,
    store: hachimi_storage::AgentStore,
    host: hachimi_extensions::PluginHost,
    session_id: hachimi_protocol::SessionId,
    run_id: hachimi_protocol::RunId,
    selection: ScheduleConnectorSelection,
    server: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    async fn new() -> Self {
        let source = tempfile::tempdir().expect("plugin source");
        let installs = tempfile::tempdir().expect("plugin installs");
        write_plugin(source.path());
        let (endpoint, server) = enterprise_server();
        let store = hachimi_storage::AgentStore::connect_in_memory()
            .await
            .expect("store");
        let api = EnterpriseApiClient::with_loopback_endpoint(&endpoint).expect("loopback API");
        let host = hachimi_extensions::PluginHost::new(store.clone(), installs.path())
            .with_enterprise_api_client(api);
        let installed = host.install_local(source.path()).await.expect("install");
        host.set_enabled(&installed.manifest.id, true)
            .await
            .expect("enable");
        let connector_account_id = ConnectorAccountId::new("scheduled-attachment-account");
        host.upsert_connector_account(ConnectorAccountUpsert {
            id: connector_account_id.clone(),
            plugin_id: installed.manifest.id.clone(),
            connector_id: CONNECTOR_ID.into(),
            display_name: "Scheduled attachment fixture".into(),
            secret: Some(
                json!({
                    "platform": "wecom",
                    "corpId": "fixture-corp",
                    "corpSecret": "fixture-secret",
                    "agentId": 1,
                    "callbackToken": "fixture-token",
                    "encodingAesKey": "fixture-encoding-key"
                })
                .to_string(),
            ),
        })
        .await
        .expect("connector account");
        let descriptor = host
            .connector_driver_descriptor(&installed.manifest.id, CONNECTOR_ID)
            .await
            .expect("descriptor");
        let selection = ScheduleConnectorSelection {
            account_id: connector_account_id,
            contribution_revision: ContributionRevision {
                plugin_id: installed.manifest.id.clone(),
                contribution_id: CONNECTOR_ID.into(),
                account_id: None,
                content_hash: installed.content_hash,
                host_identity_hash: Some(descriptor.revision.host_identity_hash),
                schema_hash: Some(descriptor.revision.schema_hash),
                action_hash: Some(descriptor.revision.action_hash),
            },
            allowed_actions: vec![crate::schedule_host_grants::ENTERPRISE_ATTACHMENT_ACTION.into()],
        };
        let pinned_account = selection.account_id.clone();
        let mut selection = selection;
        selection.contribution_revision.account_id = Some(pinned_account);

        let created = AgentRunFactory::new(store.clone())
            .create(AgentRunCreateRequest {
                principal: "service:scheduler".into(),
                idempotency_key: "scheduled-attachment-fixture".into(),
                context: SessionContextBinding::General,
                origin: RunOrigin::Interactive,
                title: "Scheduled attachment fixture".into(),
                prompt: "download one attachment".into(),
                attachment_ids: Vec::new(),
                parent_session_id: None,
                source_run_id: None,
                purpose: RunPurpose::Task,
                model_snapshot: LlmSettings::default(),
                entry_profile: EntryProfile::Workbench,
                workload_override: Some(WorkloadKind::General),
                behavior_mode: BehaviorMode::Default,
                execution_target: None,
                approval_policy: ApprovalPolicy::NeverPrompt,
                permission_profile: PermissionProfile::ExternalSandbox,
                budget: RunBudget::default(),
                requested_capabilities: ProviderCapabilities::default(),
                created_at_ms: 1_800_000_000_000,
            })
            .await
            .expect("run");
        seed_attachment_metadata(&store).await;
        Self {
            _source: source,
            _installs: installs,
            store,
            host,
            session_id: created.session.id,
            run_id: created.run.id,
            selection,
            server: Some(server),
        }
    }

    fn tool(&self, grant: ScheduleHostGrant) -> EnterpriseAttachmentDownloadTool {
        EnterpriseAttachmentDownloadTool {
            host: self.host.clone(),
            store: self.store.clone(),
            session_id: self.session_id.clone(),
            run_id: self.run_id.clone(),
            schedule_host_grant: Some(grant),
        }
    }

    fn invocation(&self, idempotency_key: &str) -> ToolInvocation {
        let call = ModelToolCall {
            id: ToolCallId::random(),
            name: crate::schedule_host_grants::ENTERPRISE_ATTACHMENT_TOOL.into(),
            arguments: json!({
                "platform": "wecom",
                "accountId": INTEGRATION_ID,
                "eventId": EVENT_ID,
                "remoteId": REMOTE_ID,
                "metadataHash": "a".repeat(64),
                "idempotencyKey": idempotency_key
            }),
        };
        ToolInvocation {
            call: ToolCall::bind(call, 1, "scheduled-plan", "scheduled-registry"),
            entry_profile: EntryProfile::Workbench,
            workload: WorkloadKind::General,
            behavior_mode: BehaviorMode::Default,
            run_generation: 1,
            step_revision: 1,
            tool_plan_hash: "scheduled-plan".into(),
            registry_revision: "scheduled-registry".into(),
            cancellation: CancellationToken::new(),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            server.join().expect("enterprise server");
        }
    }
}

#[tokio::test]
async fn scheduled_attachment_download_is_exactly_fenced_and_creates_an_artifact() {
    let fixture = Fixture::new().await;

    let missing = fixture
        .tool(ScheduleHostGrant::default())
        .execute(fixture.invocation("attachment-missing-grant"))
        .await
        .expect_err("missing grant must fail");
    assert!(
        missing
            .to_string()
            .contains("schedule_enterprise_attachment_not_authorized")
    );

    let mut drifted = fixture.selection.clone();
    drifted.contribution_revision.content_hash = "drifted-content".into();
    let drift = fixture
        .tool(ScheduleHostGrant {
            connectors: vec![drifted],
            ..ScheduleHostGrant::default()
        })
        .execute(fixture.invocation("attachment-drift"))
        .await
        .expect_err("revision drift must fail");
    assert!(
        drift
            .to_string()
            .contains("schedule_connector_action_drift")
    );

    let result = fixture
        .tool(ScheduleHostGrant {
            connectors: vec![fixture.selection.clone()],
            ..ScheduleHostGrant::default()
        })
        .execute(fixture.invocation("attachment-success"))
        .await
        .expect("tool execution");
    assert_eq!(result.status, ToolResultStatus::Succeeded);
    assert_eq!(
        result
            .structured_content
            .get("mimeType")
            .and_then(serde_json::Value::as_str),
        Some("application/pdf")
    );
    let artifacts = fixture
        .store
        .list_session_artifacts(&fixture.session_id)
        .await
        .expect("artifacts");
    assert_eq!(artifacts.len(), 1);
    assert_eq!(
        artifacts[0].kind,
        hachimi_protocol::ArtifactKind::EnterpriseAttachment
    );

    let events = fixture
        .store
        .list_events(&fixture.session_id, 0)
        .await
        .expect("events");
    assert!(crate::scheduler_commands::has_schedule_host_grant_attention(&events, &fixture.run_id));
    assert_eq!(
        crate::scheduler_commands::scheduled_completion_status(
            hachimi_protocol::RunStatus::Succeeded,
            false,
            true,
        ),
        hachimi_protocol::TaskRunStatus::NeedsAttention
    );
    let codes = events
        .iter()
        .filter_map(|event| match &event.payload {
            RunEventPayload::Generic { event, data }
                if event == crate::schedule_host_grants::SCHEDULE_HOST_GRANT_ATTENTION_EVENT =>
            {
                data.get("code").and_then(serde_json::Value::as_str)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        codes,
        vec![
            "schedule_enterprise_attachment_not_authorized",
            "schedule_connector_action_drift"
        ]
    );
}

fn write_plugin(root: &std::path::Path) {
    fs::create_dir_all(root.join(".codex-plugin")).expect("manifest directory");
    fs::create_dir_all(root.join("connectors")).expect("connector directory");
    fs::write(
        root.join("connectors/sample-crm.json"),
        br#"{"hostIdentity":"hachimi.sample-crm.local.v1","transport":"local","actions":["get"],"webhook":false,"poll":false,"externalNetwork":false}"#,
    )
    .expect("connector descriptor");
    let manifest = PluginManifest {
        manifest_version: 1,
        id: PluginId::new("scheduled-attachment-plugin"),
        name: "Scheduled attachment fixture".into(),
        version: "1.0.0".into(),
        description: "Local scheduled attachment fixture".into(),
        contributions: vec![PluginContribution {
            kind: PluginContributionKind::Connector,
            id: CONNECTOR_ID.into(),
            relative_path: "connectors/sample-crm.json".into(),
            required_scopes: vec!["connectors.invoke".into()],
        }],
    };
    fs::write(
        root.join(".codex-plugin/plugin.json"),
        serde_json::to_vec_pretty(&manifest).expect("manifest JSON"),
    )
    .expect("manifest");
}

async fn seed_attachment_metadata(store: &hachimi_storage::AgentStore) {
    sqlx::query("INSERT INTO enterprise_integration_accounts(id, platform, connector_account_id, channel_account_id, tenant_identity_hash, ingress_mode, event_source_id, state, diagnostic, credential_revision, source_account_updated_at_ms, created_at_ms, updated_at_ms) VALUES(?, 'wecom', ?, NULL, 'tenant-hash', 'encrypted_callback', 'fixture-source', 'healthy', NULL, 1, 1, 1, 1)")
        .bind(INTEGRATION_ID)
        .bind("scheduled-attachment-account")
        .execute(store.pool())
        .await
        .expect("integration account");
    sqlx::query("INSERT INTO enterprise_attachment_metadata(platform, account_id, event_id, remote_id, file_name, mime_type, declared_size_bytes, metadata_hash, artifact_id, created_at_ms) VALUES('wecom', ?, ?, ?, 'scheduled.pdf', 'application/pdf', 24, ?, NULL, 1)")
        .bind(INTEGRATION_ID)
        .bind(EVENT_ID)
        .bind(REMOTE_ID)
        .bind("a".repeat(64))
        .execute(store.pool())
        .await
        .expect("attachment metadata");
}

fn enterprise_server() -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("enterprise listener");
    let address = listener.local_addr().expect("enterprise address");
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().expect("enterprise request");
            let mut request = [0_u8; 4096];
            let size = stream.read(&mut request).expect("request bytes");
            let request = String::from_utf8_lossy(&request[..size]);
            let (content_type, body) = if request.contains("/cgi-bin/gettoken") {
                (
                    "application/json",
                    br#"{"errcode":0,"access_token":"fixture-access-token","expires_in":7200}"#
                        .as_slice(),
                )
            } else {
                (
                    "application/pdf",
                    b"%PDF-1.7\nscheduled fixture\n".as_slice(),
                )
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .expect("response headers");
            stream.write_all(body).expect("response body");
            stream.flush().expect("response flush");
        }
    });
    (format!("http://{address}"), server)
}
