use super::*;

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
