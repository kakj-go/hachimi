use super::*;
use hachimi_protocol::BrowserNetworkRule;

fn rule(origin: &str, kind: BrowserNetworkRuleKind) -> BrowserNetworkRule {
    BrowserNetworkRule {
        origin: origin.into(),
        kind,
        allow_private_network: false,
        expires_at_ms: None,
    }
}

#[test]
fn managed_request_policy_never_promotes_resource_grants_to_documents() {
    let resource_only = BrowserNetworkPolicy {
        rules: vec![rule(
            "https://static.example.com",
            BrowserNetworkRuleKind::Resource,
        )],
        deny_private_network_by_default: true,
        revision: 1,
    };
    assert!(!request_matches_policy(
        &resource_only,
        "https://static.example.com/page",
        BrowserNetworkRuleKind::Document,
        1
    ));
    assert!(request_matches_policy(
        &resource_only,
        "https://static.example.com/app.js",
        BrowserNetworkRuleKind::Resource,
        1
    ));
    let document = BrowserNetworkPolicy {
        rules: vec![rule(
            "https://app.example.com",
            BrowserNetworkRuleKind::Document,
        )],
        deny_private_network_by_default: true,
        revision: 2,
    };
    assert!(request_matches_policy(
        &document,
        "https://app.example.com/",
        BrowserNetworkRuleKind::Document,
        1
    ));
    assert!(request_matches_policy(
        &document,
        "https://app.example.com/app.js",
        BrowserNetworkRuleKind::Resource,
        1
    ));
}

#[test]
fn managed_request_classification_is_main_frame_and_fail_closed() {
    assert_eq!(
        request_network_kind("main", "main", &ResourceType::Document),
        Some(BrowserNetworkRuleKind::Document)
    );
    assert_eq!(
        request_network_kind("main", "child", &ResourceType::Document),
        Some(BrowserNetworkRuleKind::Resource)
    );
    assert_eq!(
        request_network_kind("main", "main", &ResourceType::Script),
        Some(BrowserNetworkRuleKind::Resource)
    );
    assert_eq!(
        request_network_kind("main", "main", &ResourceType::Other),
        None
    );
}

#[tokio::test]
#[ignore = "requires the prepared managed Chromium runtime and an interactive desktop"]
async fn managed_chromium_observes_uploads_and_downloads_a_real_page() {
    let executable = std::env::var_os("HACHIMI_MANAGED_CHROMIUM")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../apps/desktop/src-tauri/managed-chromium/chrome.exe")
        });
    assert!(executable.is_file(), "managed Chromium runtime is missing");
    let profiles = tempfile::tempdir().expect("profiles");
    let downloads = tempfile::tempdir().expect("downloads");
    let broker = ManagedChromiumBroker::new(executable, profiles.path(), downloads.path());
    let session_id = BrowserSessionId::from("managed-smoke");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("fixture listener");
    let address = listener.local_addr().expect("fixture address");
    let fixture = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        for _ in 0..10 {
            let accepted = tokio::time::timeout(Duration::from_secs(30), listener.accept())
                .await
                .expect("fixture timeout")
                .expect("fixture accept");
            let (mut stream, _) = accepted;
            let mut request = [0_u8; 4096];
            let read = stream.read(&mut request).await.expect("fixture request");
            let request = String::from_utf8_lossy(&request[..read]);
            let download = request.lines().next().is_some_and(|line| {
                line.contains("/download.txt") || line.contains("%2Fdownload.txt")
            });
            let (content_type, disposition, body): (&str, &str, &[u8]) = if download {
                (
                    "text/plain",
                    "Content-Disposition: attachment; filename=managed-browser-smoke.txt\r\n",
                    b"managed browser download ready\n",
                )
            } else {
                ("text/html", "", b"<title>Hachimi Smoke</title><body>managed chromium ready<input id=upload type=file><a id=download href=/download.txt download>download</a></body>")
            };
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\n{disposition}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.expect("fixture header");
            stream.write_all(body).await.expect("fixture body");
            if download {
                break;
            }
        }
    });
    broker
        .start(
            &session_id,
            BrowserProfileKind::Isolated,
            None,
            BrowserNetworkPolicy {
                rules: Vec::new(),
                deny_private_network_by_default: true,
                revision: 1,
            },
            None,
        )
        .await
        .expect("start managed Chromium");
    let origin = format!("http://127.0.0.1:{}", address.port());
    broker
        .set_network_policy(
            &session_id,
            BrowserNetworkPolicy {
                rules: vec![
                    BrowserNetworkRule {
                        origin: origin.clone(),
                        kind: BrowserNetworkRuleKind::Document,
                        allow_private_network: true,
                        expires_at_ms: None,
                    },
                    BrowserNetworkRule {
                        origin: origin.clone(),
                        kind: BrowserNetworkRuleKind::Resource,
                        allow_private_network: true,
                        expires_at_ms: None,
                    },
                ],
                deny_private_network_by_default: true,
                revision: 2,
            },
        )
        .await
        .expect("private fixture grant");
    let tab = Arc::clone(
        &broker
            .sessions
            .lock()
            .get(&session_id)
            .expect("managed session")
            .tab,
    );
    let page = format!("{origin}/smoke");
    tokio::task::spawn_blocking(move || {
        tab.navigate_to(&page).expect("navigate fixture");
        tab.wait_until_navigated().expect("wait for fixture");
    })
    .await
    .expect("navigation task");
    let observation = broker.observe(&session_id).await.expect("observe");
    assert_eq!(observation.title, "Hachimi Smoke");
    assert!(observation.text.contains("managed chromium ready"));
    let upload_source = profiles.path().join("managed-upload.txt");
    std::fs::write(&upload_source, b"managed browser upload ready\n").expect("upload source");
    let upload = broker
        .stage_upload(&session_id, &upload_source)
        .await
        .expect("stage upload");
    let uploaded = broker
        .act(
            &session_id,
            &origin,
            &BrowserAction::Upload {
                selector: "#upload".into(),
                file_token: upload.token,
            },
        )
        .await
        .expect("upload");
    assert_eq!(uploaded.result_code, "uploaded");
    let downloaded = broker
        .act(
            &session_id,
            &origin,
            &BrowserAction::Download {
                selector: "#download".into(),
                allow_unknown_type: false,
            },
        )
        .await
        .expect("download");
    assert_eq!(downloaded.result_code, "download_quarantined");
    let download_token = downloaded
        .output
        .as_ref()
        .and_then(|value| value.get("downloadToken"))
        .and_then(Value::as_str)
        .expect("download token");
    let destination = profiles.path().join("managed-browser-import.txt");
    broker
        .import_download(&session_id, download_token, &destination)
        .await
        .expect("import download");
    assert_eq!(
        std::fs::read_to_string(&destination).expect("imported download"),
        "managed browser download ready\n"
    );
    broker.stop(&session_id).await.expect("stop");
    fixture.await.expect("fixture task");
}

#[test]
fn staged_upload_is_session_bound_hash_bound_and_expiring() {
    let source_root = tempfile::tempdir().expect("source tempdir");
    let staging_root = tempfile::tempdir().expect("staging tempdir");
    let source = source_root.path().join("safe.txt");
    std::fs::write(&source, b"safe upload").expect("source");
    let session_id = BrowserSessionId::from("browser-session");
    let staged = stage_upload_file(&session_id, staging_root.path(), &source).expect("stage");
    assert!(
        resolve_upload_token(&session_id, staging_root.path(), &staged.token)
            .expect("resolve")
            .is_file()
    );
    assert!(
        resolve_upload_token(
            &BrowserSessionId::from("different-session"),
            staging_root.path(),
            &staged.token
        )
        .is_err()
    );
    std::fs::write(staging_root.path().join(&staged.token), b"tampered")
        .expect("tamper staged file");
    assert!(resolve_upload_token(&session_id, staging_root.path(), &staged.token).is_err());
    let staged = stage_upload_file(&session_id, staging_root.path(), &source).expect("restage");
    let sidecar = file_metadata_path(staging_root.path(), &staged.token);
    let mut metadata: BrowserFileMetadata =
        serde_json::from_slice(&std::fs::read(&sidecar).expect("read sidecar")).expect("metadata");
    metadata.expires_at_ms = epoch_ms().saturating_sub(1);
    metadata.created_at_ms = metadata.expires_at_ms.saturating_sub(BROWSER_FILE_TTL_MS);
    std::fs::write(&sidecar, serde_json::to_vec(&metadata).expect("serialize"))
        .expect("expire sidecar");
    assert!(resolve_upload_token(&session_id, staging_root.path(), &staged.token).is_err());
}

#[test]
fn download_validation_rejects_executables_and_mime_disguises() {
    let root = tempfile::tempdir().expect("root");
    let executable = root.path().join("invoice.pdf");
    std::fs::write(&executable, b"MZfake executable").expect("write executable");
    assert_eq!(
        validate_download_file(&executable, Some("application/pdf"), false),
        Err(BrowserHostError::DownloadFailed)
    );
    let mismatch = root.path().join("image.png");
    std::fs::write(&mismatch, b"%PDF-1.7\n").expect("write mismatch");
    assert_eq!(
        validate_download_file(&mismatch, Some("image/png"), false),
        Err(BrowserHostError::DownloadFailed)
    );
    let unknown = root.path().join("archive.bin");
    std::fs::write(&unknown, [1_u8, 2, 3, 0, 4]).expect("write unknown");
    assert_eq!(
        validate_download_file(&unknown, Some("application/octet-stream"), false),
        Err(BrowserHostError::DownloadConfirmationRequired)
    );
    assert_eq!(
        validate_download_file(&unknown, Some("application/octet-stream"), true),
        Ok("application/octet-stream".to_owned())
    );
}

#[test]
fn download_import_rejects_traversal_and_never_overwrites() {
    let source_root = tempfile::tempdir().expect("source tempdir");
    let quarantine = tempfile::tempdir().expect("quarantine tempdir");
    let destination_root = tempfile::tempdir().expect("destination tempdir");
    let source = source_root.path().join("result.json");
    std::fs::write(&source, br#"{"ok":true}"#).expect("source");
    let session_id = BrowserSessionId::from("browser-session");
    let staged = stage_upload_file(&session_id, quarantine.path(), &source).expect("stage");
    let destination = destination_root.path().join("imported.json");
    let imported =
        import_download_file(&session_id, quarantine.path(), &staged.token, &destination)
            .expect("import");
    assert_eq!(imported.size, br#"{"ok":true}"#.len() as u64);
    assert_eq!(
        std::fs::read(&destination).expect("read"),
        br#"{"ok":true}"#
    );
    assert!(
        import_download_file(&session_id, quarantine.path(), &staged.token, &destination).is_err()
    );
    assert!(
        import_download_file(
            &session_id,
            quarantine.path(),
            "..\\outside.txt",
            &destination_root.path().join("outside.txt")
        )
        .is_err()
    );
}
