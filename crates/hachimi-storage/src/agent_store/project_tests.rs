use hachimi_protocol::{
    AttachmentId, AttachmentRecord, ItemId, ItemPayload, ItemStatus, TranscriptItem,
    TranscriptItemKind,
};

use super::tests;

#[tokio::test]
async fn run_input_attachment_binding_updates_transcript_idempotently() {
    let (store, session) = tests::seeded_store().await;
    let run = tests::run(&session, "run-channel-attachment");
    let user_item = TranscriptItem {
        id: ItemId::from("item-channel-attachment"),
        session_id: session.id.clone(),
        run_id: Some(run.id.clone()),
        sequence: 0,
        kind: TranscriptItemKind::User,
        status: ItemStatus::Completed,
        payload: ItemPayload::User {
            text: "inspect attachment".into(),
            attachment_ids: Vec::new(),
        },
        relations: Default::default(),
        created_at_ms: run.created_at_ms,
    };
    store
        .create_agent_run_in_session_idempotent(
            "channel:test",
            "channel-attachment",
            &session,
            &run,
            &user_item,
            &[],
        )
        .await
        .expect("run bundle");
    let attachment = AttachmentRecord {
        id: AttachmentId::from("attachment-channel-1"),
        content_hash: "a".repeat(64),
        original_name: "report.txt".into(),
        mime_type: "text/plain".into(),
        byte_size: 6,
        created_at_ms: run.created_at_ms,
    };
    let path = store.managed_artifact_root().join(&attachment.content_hash);
    std::fs::write(&path, b"report").expect("attachment fixture");
    store
        .upsert_attachment(&attachment, &path)
        .await
        .expect("attachment");
    for _ in 0..2 {
        store
            .attach_to_run_input(&run.id, std::slice::from_ref(&attachment.id))
            .await
            .expect("bind attachment");
    }
    let transcript = store
        .list_transcript(&session.id)
        .await
        .expect("transcript");
    let persisted = transcript
        .iter()
        .find(|item| item.id == user_item.id)
        .expect("user item");
    assert!(matches!(
        &persisted.payload,
        ItemPayload::User { attachment_ids, .. } if attachment_ids == std::slice::from_ref(&attachment.id)
    ));
    assert_eq!(
        store
            .list_run_managed_attachments(&run.id)
            .await
            .expect("run attachments")
            .len(),
        1
    );
}
