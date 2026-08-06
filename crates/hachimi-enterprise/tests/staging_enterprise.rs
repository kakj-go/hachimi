use std::{collections::BTreeSet, env, fs, time::Duration};

use hachimi_enterprise::{
    EnterpriseApiClient, EnterpriseCredential, EnterpriseDownloadInput, EnterpriseMessageTarget,
    spawn_enterprise_stream,
};
use hachimi_protocol::IntegrationProviderId;
use serde::Deserialize;
use zeroize::Zeroize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StagingConfig {
    secret_refs: Vec<String>,
    connections: Vec<StagingConnection>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StagingConnection {
    platform: IntegrationProviderId,
    account_id: String,
    credential_ref: String,
    department_id: String,
    peer_id: String,
    group_id: String,
    expect_inbound_event: bool,
}

fn load_credential(connection: &StagingConnection) -> EnterpriseCredential {
    assert_eq!(
        connection.credential_ref,
        format!("keyring:connector:{}", connection.account_id)
    );
    let mut raw = keyring::Entry::new("com.hachimi.connector", &connection.account_id)
        .expect("enterprise credential entry")
        .get_password()
        .expect("enterprise staging credential");
    let credential =
        EnterpriseCredential::parse(&raw).expect("valid product enterprise credential");
    raw.zeroize();
    credential
}

#[tokio::test]
#[ignore = "requires protected enterprise test organizations and sends messages"]
async fn enterprise_product_adapters_conform_against_staging() {
    assert_eq!(
        env::var("HACHIMI_STAGING_ACTIVE_GATE").as_deref(),
        Ok("enterprise")
    );
    let path = env::var("HACHIMI_STAGING_ENTERPRISE_CONFIG").expect("staging config path");
    let config: StagingConfig =
        serde_json::from_slice(&fs::read(path).expect("read staging config"))
            .expect("parse staging config");
    let platforms = config
        .connections
        .iter()
        .map(|value| value.platform)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        platforms,
        BTreeSet::from([
            IntegrationProviderId::WecomApp,
            IntegrationProviderId::DingTalk,
            IntegrationProviderId::Feishu,
        ])
    );
    let api = EnterpriseApiClient::new().expect("enterprise API client");
    for connection in &config.connections {
        assert!(config.secret_refs.contains(&connection.credential_ref));
        let credential = load_credential(connection);
        assert_eq!(credential.platform(), connection.platform);
        api.account_identity(&connection.account_id, &credential)
            .await
            .expect("account identity");
        let departments = api
            .departments(
                &connection.account_id,
                &credential,
                Some(&connection.department_id),
                None,
                Some(20),
            )
            .await
            .expect("department page");
        assert!(
            !departments.items.is_empty(),
            "staging department page must not be empty"
        );
        let members = api
            .members(
                &connection.account_id,
                &credential,
                &connection.department_id,
                None,
                Some(20),
            )
            .await
            .expect("member page");
        assert!(
            !members.items.is_empty(),
            "staging member page must not be empty"
        );
        let marker = format!("HACHIMI_STAGING_{}", connection.platform.as_str());
        api.send_text(
            &connection.account_id,
            &credential,
            &EnterpriseMessageTarget {
                peer: connection.peer_id.clone(),
                thread: None,
                group: false,
            },
            &marker,
            &format!("{marker}_DIRECT"),
        )
        .await
        .expect("direct message");
        api.send_text(
            &connection.account_id,
            &credential,
            &EnterpriseMessageTarget {
                peer: connection.group_id.clone(),
                thread: Some(connection.group_id.clone()),
                group: true,
            },
            &marker,
            &format!("{marker}_GROUP"),
        )
        .await
        .expect("group message");

        if connection.expect_inbound_event
            && matches!(
                connection.platform,
                IntegrationProviderId::DingTalk | IntegrationProviderId::Feishu
            )
        {
            let (runtime, mut receiver) = spawn_enterprise_stream(api.clone(), credential);
            let event = tokio::time::timeout(Duration::from_secs(120), receiver.recv())
                .await
                .expect("inbound event timeout")
                .expect("inbound event stream closed");
            assert_eq!(event.platform, connection.platform);
            assert!(!event.event_id.is_empty());
            assert!(
                !event.mentions.is_empty(),
                "the protected inbound fixture must contain a structured mention"
            );
            let attachment = event
                .attachments
                .first()
                .expect("the protected inbound fixture must contain an allowed attachment");
            let root = tempfile::tempdir().expect("attachment staging root");
            let destination = root.path().join("enterprise-attachment.part");
            let download_credential = load_credential(connection);
            let receipt = api
                .download_attachment_to(EnterpriseDownloadInput {
                    account_id: &connection.account_id,
                    credential: &download_credential,
                    event_id: &event.event_id,
                    remote_id: &attachment.remote_id,
                    resource_key: attachment.resource_key.as_deref(),
                    destination: &destination,
                    max_bytes: 25 * 1024 * 1024,
                })
                .await
                .expect("download real allowed attachment through the product transport");
            assert!(receipt.byte_size > 0 && receipt.byte_size <= 25 * 1024 * 1024);
            assert_eq!(receipt.content_hash.len(), 64);
            assert_eq!(
                fs::metadata(&destination).expect("download metadata").len(),
                receipt.byte_size
            );
            runtime.stop().await;
        }
        api.revoke(&connection.account_id);
    }
}
