use super::*;
use hachimi_protocol::{ComputerAction, ComputerWindowIdentity};

#[test]
fn computer_summary_contains_only_hashed_identity_and_action_category() {
    let target = ComputerWindowIdentity {
        app_id: "C:\\Sensitive\\secret-app.exe".into(),
        process_id: 42,
        window_handle: "0x1234".into(),
        fingerprint: "window-title-and-process-fingerprint".into(),
        title: "Customer password reset - secret@example.com".into(),
        elevated: false,
        protected_desktop: false,
        hachimi_owned: false,
    };
    let action = ComputerAction::TypeText {
        text: "hunter2 at x=123 y=456".into(),
    };
    let summary = computer_target_summary(&target, computer_action_category(&action));

    assert!(summary.starts_with("computer:app_sha256:"));
    assert!(summary.contains(":window_sha256:"));
    assert!(summary.ends_with(":action:type_text"));
    for forbidden in [
        target.app_id.as_str(),
        target.fingerprint.as_str(),
        target.title.as_str(),
        target.window_handle.as_str(),
        "hunter2",
        "screenshot",
        "image_token",
    ] {
        assert!(!summary.contains(forbidden), "leaked {forbidden}");
    }
}

#[test]
fn browser_summary_hashes_origin_and_excludes_page_content() {
    let summary = browser_target_summary(
        "https://user:password@example.test/private?secret=1",
        "navigate",
    );
    assert!(summary.starts_with("browser:origin_sha256:"));
    assert!(summary.ends_with(":action:navigate"));
    assert!(!summary.contains("example.test"));
    assert!(!summary.contains("password"));
    assert!(!summary.contains("secret"));
}

#[test]
fn connector_source_reads_only_declared_metadata() {
    assert_eq!(
        connector_source(&json!({
            "sourceUrl": "https://Example.test:443/items/1#details",
            "sourceTitle": "Item 1",
            "body": "https://ignored.test"
        })),
        Some(("https://example.test/items/1".into(), Some("Item 1".into())))
    );
    assert!(connector_source(&json!({ "body": "https://ignored.test" })).is_none());
}
