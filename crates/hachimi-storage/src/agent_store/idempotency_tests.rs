use super::{AgentStore, AgentStoreError, IdempotentMutationClaim, now_ms};

#[tokio::test]
async fn mutation_idempotency_claims_cache_responses_and_fence_conflicts() {
    let store = AgentStore::connect_in_memory().await.expect("store");
    let first = store
        .claim_idempotent_mutation::<serde_json::Value>(
            "user",
            "schedule.update",
            "key-1",
            "schedule-1",
            now_ms(),
        )
        .await
        .expect("claim");
    assert_eq!(first, IdempotentMutationClaim::Claimed);
    let in_flight = store
        .claim_idempotent_mutation::<serde_json::Value>(
            "user",
            "schedule.update",
            "key-1",
            "schedule-1",
            now_ms(),
        )
        .await
        .expect("repeat claim");
    assert_eq!(in_flight, IdempotentMutationClaim::Indeterminate);

    let response = serde_json::json!({ "revision": 2 });
    store
        .complete_idempotent_mutation("user", "schedule.update", "key-1", &response)
        .await
        .expect("complete");
    let replay = store
        .claim_idempotent_mutation::<serde_json::Value>(
            "user",
            "schedule.update",
            "key-1",
            "schedule-1",
            now_ms(),
        )
        .await
        .expect("replay");
    assert_eq!(replay, IdempotentMutationClaim::Completed(response));
    assert!(matches!(
        store
            .claim_idempotent_mutation::<serde_json::Value>(
                "user",
                "schedule.update",
                "key-1",
                "schedule-2",
                now_ms(),
            )
            .await,
        Err(AgentStoreError::IdempotencyConflict)
    ));

    store
        .claim_idempotent_mutation::<bool>(
            "user",
            "schedule.remove",
            "key-2",
            "schedule-1",
            now_ms(),
        )
        .await
        .expect("removal claim");
    store
        .abandon_idempotent_mutation("user", "schedule.remove", "key-2")
        .await
        .expect("abandon");
    assert_eq!(
        store
            .claim_idempotent_mutation::<bool>(
                "user",
                "schedule.remove",
                "key-2",
                "schedule-1",
                now_ms(),
            )
            .await
            .expect("reclaim"),
        IdempotentMutationClaim::Claimed
    );
}
