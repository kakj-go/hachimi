use std::{collections::BTreeSet, env, fs, path::Path, time::Duration};

use hachimi_channel_providers::{
    IlinkMediaKind, ProviderAdapter, ProviderEventFrame, TransportProof, WechatIlinkAdapter,
    WechatIlinkClient, WecomAiBotAdapter, WecomAiBotDeliveryResult, WecomAiBotMediaKind,
    WecomAiBotTransport, WecomAiBotTransportEvent,
};
use hachimi_enterprise::{
    EnterpriseApiClient, EnterpriseCredential, EnterpriseEventAuth, EnterpriseMediaKind,
    EnterpriseMessageTarget, EnterpriseRawEvent, spawn_enterprise_stream, verify_enterprise_event,
};
use hachimi_protocol::{ChannelMessagePart, IntegrationProviderId};
use serde::Deserialize;
use serde_json::Value;
use zeroize::Zeroizing;

const REAL_EVENT_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StagingConfig {
    secret_refs: Vec<String>,
    connections: Vec<StagingConnection>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StagingConnection {
    provider_id: IntegrationProviderId,
    account_id: String,
    tenant_key: String,
    credential_ref: String,
    dm_peer_id: String,
    group_id: Option<String>,
    image_fixture_path: String,
    file_fixture_path: String,
    callback_fixture_path: Option<String>,
    conversation_secret_ref: Option<String>,
    expect_inbound_event: bool,
    expect_text: bool,
    expect_image: bool,
    expect_file: bool,
    expect_restart_recovery: bool,
    expect_credential_revocation: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IlinkCredential {
    provider_id: IntegrationProviderId,
    bot_token: String,
    bot_id: String,
    base_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WecomAiCredential {
    provider_id: IntegrationProviderId,
    bot_id: String,
    secret: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WecomCallbackCapture {
    events: Vec<WecomCallbackEvent>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WecomCallbackEvent {
    timestamp: String,
    nonce: String,
    signature: String,
    encrypted: String,
}

#[derive(Default)]
struct PartCoverage {
    text: bool,
    image: bool,
    file: bool,
}

impl PartCoverage {
    fn observe(&mut self, parts: &[ChannelMessagePart]) {
        for part in parts {
            match part {
                ChannelMessagePart::Text { text } => self.text |= !text.trim().is_empty(),
                ChannelMessagePart::Image { .. } => self.image = true,
                ChannelMessagePart::File { .. } => self.file = true,
                ChannelMessagePart::Audio { .. } | ChannelMessagePart::Video { .. } => {}
            }
        }
    }

    fn complete(&self, connection: &StagingConnection) -> bool {
        (!connection.expect_text || self.text)
            && (!connection.expect_image || self.image)
            && (!connection.expect_file || self.file)
    }
}

#[tokio::test]
#[ignore = "requires five protected platform accounts, real inbound messages, and sends media"]
async fn five_official_channel_product_transports_conform_against_staging() {
    assert_eq!(
        env::var("HACHIMI_STAGING_ACTIVE_GATE").as_deref(),
        Ok("channels")
    );
    let path = env::var("HACHIMI_STAGING_CHANNELS_CONFIG").expect("staging config path");
    let config: StagingConfig = serde_json::from_slice(
        &fs::read(path).unwrap_or_else(|_| panic!("read protected Channel staging config")),
    )
    .expect("parse Channel staging config");
    let providers = config
        .connections
        .iter()
        .map(|connection| connection.provider_id)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        providers,
        BTreeSet::from([
            IntegrationProviderId::DingTalk,
            IntegrationProviderId::Feishu,
            IntegrationProviderId::WecomAiBot,
            IntegrationProviderId::WecomApp,
            IntegrationProviderId::WechatIlink,
        ])
    );

    for connection in &config.connections {
        assert!(connection.expect_inbound_event);
        assert!(connection.expect_restart_recovery);
        assert!(connection.expect_credential_revocation);
        assert!(config.secret_refs.contains(&connection.credential_ref));
        match connection.provider_id {
            IntegrationProviderId::DingTalk
            | IntegrationProviderId::Feishu
            | IntegrationProviderId::WecomApp => exercise_enterprise(connection).await,
            IntegrationProviderId::WecomAiBot => exercise_wecom_ai(connection).await,
            IntegrationProviderId::WechatIlink => exercise_ilink(&config, connection).await,
        }
        exercise_credential_revocation(connection);
    }
}

async fn exercise_enterprise(connection: &StagingConnection) {
    let raw = read_secret(&connection.credential_ref);
    let credential = EnterpriseCredential::parse(&raw).expect("enterprise credential shape");
    assert_eq!(credential.platform(), connection.provider_id);
    assert_eq!(credential.tenant_id(), connection.tenant_key);
    let api = EnterpriseApiClient::new().expect("enterprise API client");
    api.account_identity(&connection.account_id, &credential)
        .await
        .expect("enterprise credential probe");
    let dm = EnterpriseMessageTarget {
        peer: connection.dm_peer_id.clone(),
        thread: None,
        group: false,
    };
    api.send_text(
        &connection.account_id,
        &credential,
        &dm,
        "HACHIMI_CHANNEL_STAGING_TEXT",
        &format!("{}:text", connection.account_id),
    )
    .await
    .expect("enterprise text delivery");
    send_enterprise_media(&api, connection, &credential, &dm).await;
    if let Some(group_id) = &connection.group_id {
        api.send_text(
            &connection.account_id,
            &credential,
            &EnterpriseMessageTarget {
                peer: group_id.clone(),
                thread: Some(group_id.clone()),
                group: true,
            },
            "HACHIMI_CHANNEL_STAGING_GROUP",
            &format!("{}:group", connection.account_id),
        )
        .await
        .expect("enterprise group delivery");
    }

    if connection.provider_id == IntegrationProviderId::WecomApp {
        verify_wecom_callback_capture(connection, &credential).await;
    } else {
        let (runtime, mut events) = spawn_enterprise_stream(api.clone(), credential);
        let mut coverage = PartCoverage::default();
        tokio::time::timeout(REAL_EVENT_TIMEOUT, async {
            while !coverage.complete(connection) {
                let event = events.recv().await.expect("enterprise stream closed");
                if !event.text.trim().is_empty() {
                    coverage.text = true;
                }
                for attachment in event.attachments {
                    observe_remote_media(&mut coverage, &attachment);
                }
            }
        })
        .await
        .expect("enterprise inbound coverage timeout");
        runtime.stop().await;
    }

    let restarted = EnterpriseApiClient::new().expect("restarted enterprise API client");
    restarted
        .account_identity(
            &connection.account_id,
            &EnterpriseCredential::parse(&raw).unwrap(),
        )
        .await
        .expect("enterprise restart credential recovery");
}

async fn send_enterprise_media(
    api: &EnterpriseApiClient,
    connection: &StagingConnection,
    credential: &EnterpriseCredential,
    target: &EnterpriseMessageTarget,
) {
    for (kind, path, mime, key) in [
        (
            EnterpriseMediaKind::Image,
            connection.image_fixture_path.as_str(),
            "image/png",
            "image",
        ),
        (
            EnterpriseMediaKind::File,
            connection.file_fixture_path.as_str(),
            "application/pdf",
            "file",
        ),
    ] {
        let bytes = read_fixture(path);
        let file_name = Path::new(path)
            .file_name()
            .and_then(|value| value.to_str())
            .expect("fixture file name");
        api.send_media(
            &connection.account_id,
            credential,
            target,
            kind,
            file_name,
            mime,
            &bytes,
            &format!("{}:{key}", connection.account_id),
        )
        .await
        .expect("enterprise media delivery");
    }
}

async fn exercise_wecom_ai(connection: &StagingConnection) {
    let raw = read_secret(&connection.credential_ref);
    let credential: WecomAiCredential = serde_json::from_str(&raw).expect("WeCom AI credential");
    assert_eq!(credential.provider_id, IntegrationProviderId::WecomAiBot);
    assert_eq!(credential.bot_id, connection.tenant_key);
    let (transport, mut events) =
        WecomAiBotTransport::spawn(credential.bot_id.clone(), credential.secret.clone())
            .expect("WeCom AI transport");
    assert_delivered(
        transport
            .send_text(
                connection.dm_peer_id.clone(),
                false,
                "HACHIMI_CHANNEL_STAGING_TEXT".into(),
                format!("{}:text", connection.account_id),
            )
            .await,
    );
    for (kind, path, key) in [
        (
            WecomAiBotMediaKind::Image,
            connection.image_fixture_path.as_str(),
            "image",
        ),
        (
            WecomAiBotMediaKind::File,
            connection.file_fixture_path.as_str(),
            "file",
        ),
    ] {
        assert_delivered(
            transport
                .send_media(
                    connection.dm_peer_id.clone(),
                    false,
                    kind,
                    fixture_name(path),
                    read_fixture(path),
                    format!("{}:{key}", connection.account_id),
                )
                .await,
        );
    }
    if let Some(group_id) = &connection.group_id {
        assert_delivered(
            transport
                .send_text(
                    group_id.clone(),
                    true,
                    "HACHIMI_CHANNEL_STAGING_GROUP".into(),
                    format!("{}:group", connection.account_id),
                )
                .await,
        );
    }
    let adapter = WecomAiBotAdapter;
    let mut coverage = PartCoverage::default();
    tokio::time::timeout(REAL_EVENT_TIMEOUT, async {
        while !coverage.complete(connection) {
            match events.recv().await.expect("WeCom AI event stream closed") {
                WecomAiBotTransportEvent::Message {
                    payload,
                    connection_id,
                    received_at_ms,
                } => {
                    let message = adapter
                        .normalize(ProviderEventFrame {
                            account_id: connection.account_id.clone(),
                            tenant_key: connection.tenant_key.clone(),
                            payload,
                            proof: TransportProof::Stream {
                                connection_id,
                                received_at_ms,
                            },
                        })
                        .expect("normalize real WeCom AI event");
                    coverage.observe(&message.parts);
                }
                WecomAiBotTransportEvent::AuthenticationExpired => {
                    panic!("WeCom AI credential was revoked during the gate")
                }
                WecomAiBotTransportEvent::Degraded => {}
            }
        }
    })
    .await
    .expect("WeCom AI inbound coverage timeout");
    transport.stop().await;

    let (restarted, _) = WecomAiBotTransport::spawn(credential.bot_id, credential.secret)
        .expect("restart WeCom AI transport");
    assert_delivered(
        restarted
            .send_text(
                connection.dm_peer_id.clone(),
                false,
                "HACHIMI_CHANNEL_STAGING_RESTART".into(),
                format!("{}:restart", connection.account_id),
            )
            .await,
    );
    restarted.stop().await;
}

async fn exercise_ilink(config: &StagingConfig, connection: &StagingConnection) {
    assert!(connection.group_id.is_none());
    let raw = read_secret(&connection.credential_ref);
    let credential: IlinkCredential = serde_json::from_str(&raw).expect("iLink credential");
    assert_eq!(credential.provider_id, IntegrationProviderId::WechatIlink);
    assert_eq!(credential.bot_id, connection.tenant_key);
    let context_ref = connection
        .conversation_secret_ref
        .as_deref()
        .expect("iLink conversation secret reference");
    assert!(config.secret_refs.iter().any(|value| value == context_ref));
    let context_token = read_secret(context_ref);
    let client = WechatIlinkClient::authenticated(&credential.base_url, &credential.bot_token)
        .expect("iLink client");
    client
        .send_text(
            &connection.dm_peer_id,
            "HACHIMI_CHANNEL_STAGING_TEXT",
            &context_token,
            &format!("{}:text", connection.account_id),
        )
        .await
        .expect("iLink text delivery");
    for (kind, path, key) in [
        (
            IlinkMediaKind::Image,
            connection.image_fixture_path.as_str(),
            "image",
        ),
        (
            IlinkMediaKind::File,
            connection.file_fixture_path.as_str(),
            "file",
        ),
    ] {
        client
            .send_media(
                &connection.dm_peer_id,
                &context_token,
                kind,
                &fixture_name(path),
                &read_fixture(path),
                &format!("{}:{key}", connection.account_id),
            )
            .await
            .expect("iLink media delivery");
    }
    let adapter = WechatIlinkAdapter;
    let mut cursor = String::new();
    let mut coverage = PartCoverage::default();
    tokio::time::timeout(REAL_EVENT_TIMEOUT, async {
        while !coverage.complete(connection) {
            let batch = client.get_updates(&cursor).await.expect("iLink long poll");
            cursor = batch.cursor;
            for payload in batch.messages {
                let normalized = adapter.normalize(ProviderEventFrame {
                    account_id: connection.account_id.clone(),
                    tenant_key: connection.tenant_key.clone(),
                    payload,
                    proof: TransportProof::IlinkPoll {
                        bot_id: credential.bot_id.clone(),
                        received_at_ms: now_ms(),
                    },
                });
                if let Ok(message) = normalized {
                    coverage.observe(&message.parts);
                }
            }
        }
    })
    .await
    .expect("iLink inbound coverage timeout");

    let restarted = WechatIlinkClient::authenticated(credential.base_url, credential.bot_token)
        .expect("restarted iLink client");
    restarted
        .send_text(
            &connection.dm_peer_id,
            "HACHIMI_CHANNEL_STAGING_RESTART",
            &context_token,
            &format!("{}:restart", connection.account_id),
        )
        .await
        .expect("iLink restart delivery");
}

async fn verify_wecom_callback_capture(
    connection: &StagingConnection,
    credential: &EnterpriseCredential,
) {
    let path = connection
        .callback_fixture_path
        .as_deref()
        .expect("WeCom callback capture path");
    let capture = tokio::time::timeout(REAL_EVENT_TIMEOUT, async {
        loop {
            if let Ok(bytes) = fs::read(path)
                && let Ok(capture) = serde_json::from_slice::<WecomCallbackCapture>(&bytes)
                && !capture.events.is_empty()
            {
                break capture;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .expect("WeCom callback capture timeout");
    let mut coverage = PartCoverage::default();
    for event in capture.events {
        let verified = verify_enterprise_event(
            credential,
            EnterpriseRawEvent {
                platform: IntegrationProviderId::WecomApp,
                account_id: connection.account_id.clone(),
                tenant_id: connection.tenant_key.clone(),
                event_id: None,
                event_type: None,
                peer: None,
                thread: None,
                sender: None,
                text: None,
                mentions: Vec::new(),
                attachments: Vec::new(),
                payload: Value::Null,
                auth: EnterpriseEventAuth::WecomCallback {
                    timestamp: event.timestamp,
                    nonce: event.nonce,
                    signature: event.signature,
                    encrypted: event.encrypted,
                },
            },
            now_ms(),
        )
        .expect("verify captured WeCom callback");
        coverage.text |= !verified.text.trim().is_empty();
        for attachment in verified.attachments {
            observe_remote_media(&mut coverage, &attachment);
        }
    }
    assert!(coverage.complete(connection));
}

fn assert_delivered(result: WecomAiBotDeliveryResult) {
    assert!(matches!(result, WecomAiBotDeliveryResult::Delivered { .. }));
}

fn observe_remote_media(
    coverage: &mut PartCoverage,
    media: &hachimi_protocol::RemoteMediaDescriptor,
) {
    if media.resource_key.as_deref() == Some("image")
        || media
            .mime_type
            .as_deref()
            .is_some_and(|value| value.starts_with("image/"))
    {
        coverage.image = true;
    } else {
        coverage.file = true;
    }
}

fn read_secret(reference: &str) -> Zeroizing<String> {
    let username = reference
        .strip_prefix("keyring:integration:")
        .expect("integration keyring reference");
    Zeroizing::new(
        keyring::Entry::new("com.hachimi.integration", username)
            .expect("integration credential entry")
            .get_password()
            .expect("protected integration credential"),
    )
}

fn exercise_credential_revocation(connection: &StagingConnection) {
    let username = connection
        .credential_ref
        .strip_prefix("keyring:integration:")
        .expect("integration keyring reference");
    let entry = keyring::Entry::new("com.hachimi.integration", username)
        .expect("integration credential entry");
    let raw = Zeroizing::new(entry.get_password().expect("credential before revocation"));
    entry
        .delete_credential()
        .expect("temporary credential revocation");
    assert!(entry.get_password().is_err());
    entry
        .set_password(&raw)
        .expect("restore temporary staging credential");
}

fn read_fixture(path: &str) -> Vec<u8> {
    let bytes = fs::read(path).unwrap_or_else(|_| panic!("read protected media fixture"));
    assert!(!bytes.is_empty() && bytes.len() <= 25 * 1024 * 1024);
    bytes
}

fn fixture_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .expect("fixture file name")
        .to_owned()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(i64::MAX)
}
