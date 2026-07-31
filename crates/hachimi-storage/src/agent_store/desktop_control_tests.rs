use hachimi_protocol::{EntryProfile, PermissionProfile, SessionId, SessionRecord};
use sqlx::Row;

use super::{
    DesktopControlActionLedgerInput,
    tests::{run, seeded_store},
};

#[tokio::test]
async fn state_and_action_ledger_are_durable_and_non_replayable() {
    let (store, parent) = seeded_store().await;
    let timestamp = super::now_ms();
    let session = SessionRecord {
        id: SessionId::from("desktop-control-session"),
        context: parent.context.clone(),
        entry_profile: EntryProfile::DesktopControl,
        title: "Desktop control".into(),
        archived: false,
        pinned: false,
        parent_session_id: None,
        source_run_id: None,
        created_at_ms: timestamp,
        updated_at_ms: timestamp,
    };
    store.create_session(&session).await.expect("session");
    let mut desktop_run = run(&session, "desktop-control-run");
    desktop_run.configuration.entry_profile = EntryProfile::DesktopControl;
    desktop_run.configuration.permission_profile = PermissionProfile::ExternalSandbox;
    store
        .create_run_idempotent("desktop-test", "desktop-control-run", &desktop_run)
        .await
        .expect("run");
    store
        .upsert_desktop_control_session(&session.id, timestamp)
        .await
        .expect("desktop session state");
    assert!(
        store
            .desktop_control_session_exists(&session.id)
            .await
            .expect("desktop session exists")
    );

    let input = DesktopControlActionLedgerInput {
        session_id: session.id.clone(),
        run_id: desktop_run.id.clone(),
        generation: desktop_run.generation,
        action_id: "action-hash".into(),
        action_kind: "computer.window_focus".into(),
        target_fingerprint_hash: "target-hash".into(),
        observation_revision: "frame-1:1".into(),
        now_ms: timestamp,
    };
    assert!(
        store
            .prepare_desktop_control_action(&input)
            .await
            .expect("prepare")
    );
    assert!(
        !store
            .prepare_desktop_control_action(&input)
            .await
            .expect("duplicate")
    );
    for (status, result) in [
        ("approved", Some("run_grant")),
        ("dispatched", None),
        ("completed", Some("performed")),
    ] {
        store
            .update_desktop_control_action(
                &session.id,
                &input.action_id,
                status,
                result,
                timestamp + 1,
            )
            .await
            .expect("status update");
    }
    store
        .set_desktop_control_computer_observation(
            &session.id,
            Some("notepad.exe"),
            Some("window-fingerprint"),
            4,
            "observing",
            Some(timestamp + 2),
            timestamp + 2,
        )
        .await
        .expect("observation state");

    let row = sqlx::query("SELECT status, result_code, target_fingerprint_hash FROM desktop_control_action_ledger WHERE session_id = ? AND action_id = ?")
        .bind(session.id.as_str())
        .bind(&input.action_id)
        .fetch_one(store.pool())
        .await
        .expect("ledger row");
    assert_eq!(row.get::<String, _>("status"), "completed");
    assert_eq!(row.get::<String, _>("result_code"), "performed");
    assert_eq!(
        row.get::<String, _>("target_fingerprint_hash"),
        "target-hash"
    );
}
