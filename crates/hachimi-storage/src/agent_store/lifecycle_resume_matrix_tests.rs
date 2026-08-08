use hachimi_protocol::{
    ItemId, ItemPayload, ItemRelations, ItemStatus, SessionId, SessionResumeRequest,
    SessionSearchRequest, TranscriptItem, TranscriptItemKind,
};

use super::tests::seed;

#[tokio::test]
async fn archived_session_resume_and_transcript_page_boundaries_are_lossless() {
    let (store, session, run) = seed().await;
    store
        .update_session_metadata(&session.id, None, Some(true), None, 1_700_000_000_100)
        .await
        .expect("archive");
    for sequence in 0..201_u64 {
        store
            .append_transcript_item(TranscriptItem {
                id: ItemId::new(format!("page-item-{sequence:03}")),
                session_id: session.id.clone(),
                run_id: Some(run.id.clone()),
                sequence: 0,
                kind: TranscriptItemKind::User,
                status: ItemStatus::Completed,
                payload: ItemPayload::User {
                    text: format!("message-{sequence:03}"),
                    attachment_ids: Vec::new(),
                },
                relations: ItemRelations::default(),
                created_at_ms: 1_700_000_001_000 + sequence as i64,
            })
            .await
            .expect("transcript item");
    }

    let newest = store
        .resume_session(&SessionResumeRequest {
            session_id: session.id.clone(),
            metadata_only: false,
            transcript_before_sequence: None,
            transcript_limit: 200,
        })
        .await
        .expect("newest page");
    assert!(newest.session.archived);
    assert_eq!(newest.transcript.len(), 200);
    assert_eq!(
        newest.active_run.as_ref().map(|value| value.id.clone()),
        Some(run.id)
    );
    let cursor = newest.previous_transcript_cursor.expect("previous cursor");
    let oldest = store
        .resume_session(&SessionResumeRequest {
            session_id: session.id,
            metadata_only: false,
            transcript_before_sequence: Some(cursor),
            transcript_limit: 200,
        })
        .await
        .expect("oldest page");
    assert_eq!(oldest.transcript.len(), 1);
    assert!(
        oldest.transcript[0].sequence < newest.transcript[0].sequence,
        "the cursor must not duplicate or skip the boundary item"
    );
}

#[tokio::test]
async fn session_search_limit_two_hundred_has_no_duplicate_cursor_row() {
    let (store, template, _) = seed().await;
    for index in 0..200_u64 {
        let mut session = template.clone();
        session.id = SessionId::new(format!("search-page-{index:03}"));
        session.title = format!("Search page {index:03}");
        session.created_at_ms += index as i64 + 1;
        session.updated_at_ms += index as i64 + 1;
        store.create_session(&session).await.expect("session");
    }
    let first = store
        .search_sessions(&SessionSearchRequest {
            project_id: template.context.project_id().cloned(),
            query: None,
            archived: Some(false),
            before: None,
            limit: 200,
        })
        .await
        .expect("first page");
    assert_eq!(first.items.len(), 200);
    let second = store
        .search_sessions(&SessionSearchRequest {
            project_id: template.context.project_id().cloned(),
            query: None,
            archived: Some(false),
            before: first.next_cursor,
            limit: 200,
        })
        .await
        .expect("second page");
    assert_eq!(second.items.len(), 1);
    let unique = first
        .items
        .iter()
        .chain(&second.items)
        .map(|session| session.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(unique.len(), 201);
}
