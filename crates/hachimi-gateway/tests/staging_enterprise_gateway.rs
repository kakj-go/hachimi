use std::{env, fs, path::PathBuf, process::Stdio, time::Duration};

use hachimi_enterprise::{EnterpriseApiClient, EnterpriseCredential, EnterpriseMessageTarget};
use hachimi_gateway::{GatewayHost, local_builtin_providers};
use hachimi_protocol::{ChannelProviderAccount, ChannelRouteKey, EnterprisePlatform};
use hachimi_storage::AgentStore;
use serde::Deserialize;
use zeroize::Zeroize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StagingConfig {
    connections: Vec<StagingConnection>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StagingConnection {
    platform: EnterprisePlatform,
    account_id: String,
    credential_ref: String,
    peer_id: String,
    group_id: String,
    callback_public_url: Option<String>,
    expect_inbound_event: bool,
}

struct ChildGuard(std::process::Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX)
}

async fn wait_for_gateway(child: &mut ChildGuard) {
    for _ in 0..120 {
        assert!(
            child
                .0
                .try_wait()
                .expect("candidate process state")
                .is_none(),
            "candidate Gateway exited before binding its product loopback endpoint"
        );
        if tokio::net::TcpStream::connect("127.0.0.1:42371")
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("candidate Gateway did not bind its product loopback endpoint");
}

#[tokio::test]
#[ignore = "requires a protected WeCom organization and HTTPS reverse proxy callback"]
async fn wecom_callback_reaches_the_candidate_gateway_and_persists_normalized_ingress() {
    assert_eq!(
        env::var("HACHIMI_STAGING_ACTIVE_GATE").as_deref(),
        Ok("enterprise")
    );
    let config_path = env::var("HACHIMI_STAGING_ENTERPRISE_CONFIG").expect("staging config path");
    let config: StagingConfig =
        serde_json::from_slice(&fs::read(config_path).expect("read config")).expect("parse config");
    let connection = config
        .connections
        .iter()
        .find(|connection| connection.platform == EnterprisePlatform::Wecom)
        .expect("WeCom connection");
    assert!(connection.expect_inbound_event);
    assert_eq!(
        connection.credential_ref,
        format!("keyring:connector:{}", connection.account_id)
    );
    let callback = connection
        .callback_public_url
        .as_deref()
        .expect("HTTPS callback public URL");
    assert!(callback.starts_with("https://"));
    assert!(
        callback
            .split('?')
            .next()
            .is_some_and(|value| value.ends_with("/v1/channels/wecom/callback"))
    );

    let executable = PathBuf::from(
        env::var_os("HACHIMI_STAGING_HACHIMI_EXE").expect("candidate product executable"),
    );
    assert!(
        executable.is_file(),
        "candidate product executable is missing"
    );
    let data_root = tempfile::tempdir().expect("Gateway data root");
    let store = AgentStore::connect(data_root.path().join("agent.sqlite3"))
        .await
        .expect("staging Gateway store");
    let builtins =
        local_builtin_providers(store.clone(), "hachimi-staging-loopback-token-00000000")
            .expect("builtin providers");
    let gateway = GatewayHost::with_registry(store.clone(), builtins.registry.clone());
    let mut accounts = builtins.accounts;
    accounts.push(ChannelProviderAccount {
        id: connection.account_id.clone(),
        provider_id: "wecom".into(),
        display_name: "WeCom release Gate".into(),
        secret_ref: Some(format!("keyring:channel:wecom:{}", connection.account_id)),
        enabled: true,
        route_allowlist: [connection.peer_id.as_str(), connection.group_id.as_str()]
            .into_iter()
            .map(|peer| ChannelRouteKey {
                channel: "wecom".into(),
                account: connection.account_id.clone(),
                peer: peer.into(),
                thread: peer.into(),
            })
            .collect(),
        config_revision: 1,
    });
    gateway
        .bootstrap_provider_accounts(&accounts)
        .await
        .expect("configure product WeCom provider");
    drop(gateway);

    let started_at_ms = now_ms();
    let process = std::process::Command::new(executable)
        .arg("--gateway")
        .env("HACHIMI_DATA_DIR", data_root.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start candidate product Gateway");
    let mut child = ChildGuard(process);
    wait_for_gateway(&mut child).await;

    let mut raw_credential = keyring::Entry::new("com.hachimi.connector", &connection.account_id)
        .expect("WeCom connector credential entry")
        .get_password()
        .expect("WeCom connector credential");
    let credential = EnterpriseCredential::parse(&raw_credential).expect("WeCom credential shape");
    raw_credential.zeroize();
    let api = EnterpriseApiClient::new().expect("enterprise API client");
    let marker = format!("HACHIMI_WECOM_CALLBACK_{}", started_at_ms);
    for (peer, group) in [
        (connection.peer_id.as_str(), false),
        (connection.group_id.as_str(), true),
    ] {
        api.send_text(
            &connection.account_id,
            &credential,
            &EnterpriseMessageTarget {
                peer: peer.into(),
                thread: group.then(|| peer.into()),
                group,
            },
            &marker,
            &format!("{marker}:{peer}"),
        )
        .await
        .expect("send WeCom callback trigger message");
    }

    for _ in 0..240 {
        assert!(
            child
                .0
                .try_wait()
                .expect("candidate process state")
                .is_none(),
            "candidate Gateway exited while waiting for the real callback"
        );
        let receipt: Option<(String, String)> = sqlx::query_as(
            "SELECT receipt.event_id, receipt.payload_hash FROM enterprise_event_receipts AS receipt WHERE receipt.platform = 'wecom' AND receipt.account_id = ? AND receipt.received_at_ms >= ? AND EXISTS(SELECT 1 FROM enterprise_event_mentions AS mention WHERE mention.platform = receipt.platform AND mention.account_id = receipt.account_id AND mention.event_id = receipt.event_id) AND EXISTS(SELECT 1 FROM enterprise_attachment_metadata AS attachment WHERE attachment.platform = receipt.platform AND attachment.account_id = receipt.account_id AND attachment.event_id = receipt.event_id) ORDER BY receipt.received_at_ms DESC LIMIT 1",
        )
        .bind(format!("channel:{}", connection.account_id))
        .bind(started_at_ms)
        .fetch_optional(store.pool())
        .await
        .expect("query candidate Gateway receipt");
        if let Some((event_id, payload_hash)) = receipt {
            assert!(!event_id.trim().is_empty());
            assert_eq!(payload_hash.len(), 64);
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!(
        "no real WeCom callback with a structured mention and attachment reached the candidate Gateway through {}",
        connection
            .callback_public_url
            .as_deref()
            .unwrap_or_default()
    );
}
