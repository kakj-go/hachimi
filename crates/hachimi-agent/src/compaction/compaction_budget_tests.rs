use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use futures_util::stream;
use hachimi_protocol::{ProviderCapabilities, RunId, TranscriptItemKind};
use tokio_util::sync::CancellationToken;

use super::{
    CompactionPolicy, ModelRequest, ModelRuntime, SemanticCompactor, extract_identifiers,
    merge_identifiers, prepare_source,
    tests::{item, seeded_compaction_store},
};

struct LargeWindowModel {
    calls: Arc<AtomicUsize>,
}

impl ModelRuntime for LargeWindowModel {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            text_input: true,
            context_window: Some(1_000_000),
            max_output_tokens: Some(4_096),
            ..ProviderCapabilities::default()
        }
    }

    fn stream(
        &self,
        _request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> crate::ModelEventStream {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(stream::empty())
    }
}

#[tokio::test]
async fn known_context_window_does_not_fall_back_to_character_trigger() {
    let (store, session, _) = seeded_compaction_store().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let checkpoint = SemanticCompactor::new(
        store,
        Arc::new(LargeWindowModel {
            calls: Arc::clone(&calls),
        }),
    )
    .with_policy(CompactionPolicy {
        automatic_trigger_chars: 0,
        ..CompactionPolicy::default()
    })
    .compact_if_needed(&session.id, None, CancellationToken::new())
    .await
    .expect("budget check");

    assert!(checkpoint.is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn preserves_recent_tail_and_stops_before_current_run() {
    let transcript = (1..=8)
        .map(|sequence| {
            let run = if sequence == 8 { "current" } else { "old" };
            item(
                sequence,
                run,
                if sequence % 2 == 0 {
                    TranscriptItemKind::Assistant
                } else {
                    TranscriptItemKind::User
                },
                &format!("message {sequence}"),
            )
        })
        .collect::<Vec<_>>();
    let source = prepare_source(
        &transcript,
        Some(&RunId::from("current")),
        None,
        CompactionPolicy {
            automatic_trigger_chars: 0,
            recent_tail_items: 2,
            ..CompactionPolicy::default()
        },
        false,
        false,
    )
    .expect("source");
    assert_eq!(source.covered_through_sequence, 4);
    assert_eq!(source.recent_tail_items, 3);
    assert!(!source.rendered.contains("message 8"));
}

#[test]
fn extracts_only_bounded_continuity_identifiers() {
    let identifiers = extract_identifiers(
        "src/lib.rs run-42 deadbeefcafebaad https://example.com TOKEN=secret ordinary",
        16,
    );
    assert!(identifiers.contains(&"src/lib.rs".to_owned()));
    assert!(identifiers.contains(&"run-42".to_owned()));
    assert!(identifiers.contains(&"deadbeefcafebaad".to_owned()));
    assert!(!identifiers.iter().any(|value| value.contains("secret")));
    assert!(
        !identifiers
            .iter()
            .any(|value| value.contains("example.com"))
    );
}

#[test]
fn older_checkpoint_identifiers_have_retention_priority() {
    let previous = vec!["run-old".into(), "src/old.rs".into()];
    let merged = merge_identifiers(&previous, vec!["run-new".into(), "src/new.rs".into()], 3);
    assert_eq!(merged, vec!["run-old", "src/old.rs", "run-new"]);
}
