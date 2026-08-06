use hachimi_protocol::{
    ComputerAppDescriptor, ComputerFrame, ComputerFrameId, ComputerWindowIdentity, EntryProfile,
    PermissionProfile, SessionId, SessionRecord,
};
use sqlx::Row;

use super::{
    HostActionLedgerInput,
    tests::{run, seeded_store},
};

#[tokio::test]
async fn computer_projection_and_host_action_ledger_are_durable_and_non_replayable() {
    let (store, parent) = seeded_store().await;
    let timestamp = super::now_ms();
    let session = SessionRecord {
        id: SessionId::from("host-control-session"),
        context: parent.context.clone(),
        entry_profile: EntryProfile::Workbench,
        title: "Host control".into(),
        archived: false,
        pinned: false,
        parent_session_id: None,
        source_run_id: None,
        created_at_ms: timestamp,
        updated_at_ms: timestamp,
    };
    store.create_session(&session).await.expect("session");
    let mut host_run = run(&session, "host-control-run");
    host_run.configuration.permission_profile = PermissionProfile::FullAccess;
    store
        .create_run_idempotent("host-test", "host-control-run", &host_run)
        .await
        .expect("run");

    let input = HostActionLedgerInput {
        session_id: session.id.clone(),
        run_id: host_run.id.clone(),
        generation: host_run.generation,
        action_id: "action-hash".into(),
        action_kind: "computer.window_focus".into(),
        target_fingerprint_hash: "target-hash".into(),
        observation_revision: "frame-1:1".into(),
        now_ms: timestamp,
    };
    assert!(store.prepare_host_action(&input).await.expect("prepare"));
    assert!(!store.prepare_host_action(&input).await.expect("duplicate"));
    for (status, result) in [
        ("approved", Some("run_grant")),
        ("dispatched", None),
        ("completed", Some("performed")),
    ] {
        store
            .update_host_action(&session.id, &input.action_id, status, result, timestamp + 1)
            .await
            .expect("status update");
    }
    store
        .set_computer_control_observation(
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
    let frame = ComputerFrame {
        id: ComputerFrameId::from("frame-1"),
        session_id: session.id.clone(),
        run_id: host_run.id.clone(),
        run_generation: host_run.generation,
        target: ComputerWindowIdentity {
            app_id: "notepad.exe".into(),
            app: ComputerAppDescriptor {
                app_id: "notepad.exe".into(),
                display_name: "Notepad".into(),
                executable_name: "notepad.exe".into(),
                executable_path: None,
                publisher: None,
                publisher_verified: false,
                package_family_name: None,
                app_user_model_id: None,
                file_identity: None,
                identity_hash: "app-hash".into(),
            },
            process_id: 42,
            window_handle: "0x1234".into(),
            fingerprint: "window-fingerprint".into(),
            title: "Notepad".into(),
            elevated: false,
            protected_desktop: false,
            hachimi_owned: false,
        },
        image_token: "ephemeral-secret-token".into(),
        width: 800,
        height: 600,
        input_epoch: 4,
        created_at_ms: timestamp,
        expires_at_ms: timestamp + 30_000,
    };
    store
        .store_computer_control_frame(
            &frame,
            &ComputerAppDescriptor {
                app_id: "notepad.exe".into(),
                display_name: "Notepad".into(),
                executable_name: "notepad.exe".into(),
                executable_path: None,
                publisher: None,
                publisher_verified: false,
                package_family_name: None,
                app_user_model_id: None,
                file_identity: None,
                identity_hash: "app-hash".into(),
            },
            timestamp + 2,
        )
        .await
        .expect("frame projection");
    let persisted_frame_json = sqlx::query_scalar::<_, String>(
        "SELECT latest_frame_json FROM computer_control_sessions WHERE session_id = ?",
    )
    .bind(session.id.as_str())
    .fetch_one(store.pool())
    .await
    .expect("persisted frame");
    assert!(!persisted_frame_json.contains("ephemeral-secret-token"));
    assert!(
        serde_json::from_str::<ComputerFrame>(&persisted_frame_json)
            .expect("frame metadata")
            .image_token
            .is_empty()
    );

    let row = sqlx::query("SELECT status, result_code, target_fingerprint_hash FROM host_action_ledger WHERE session_id = ? AND action_id = ?")
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
