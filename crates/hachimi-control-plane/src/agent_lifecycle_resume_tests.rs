use std::time::Duration;

use hachimi_core::FeatureFlags;
use hachimi_protocol::{
    ClientId, EventSubscriptionRequest, RunStatus, SandboxCapabilityReport, SandboxReadiness,
    SessionResumeRequest,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::{AgentLifecycleService, tests::seed_running_run};

fn sandbox() -> SandboxCapabilityReport {
    SandboxCapabilityReport {
        backend: "test".into(),
        readiness: SandboxReadiness::Degraded,
        os_enforced: false,
        filesystem_enforced: false,
        process_enforced: false,
        network_enforced: false,
        version: None,
        stable_error_code: Some("test_only".into()),
        diagnostics: Vec::new(),
    }
}

#[tokio::test]
async fn two_clients_rejoin_one_generation_and_receive_post_snapshot_event_once() {
    let (store, session, run) = seed_running_run().await;
    let service =
        AgentLifecycleService::new(store.clone(), FeatureFlags::all_disabled(), sandbox());
    let first_resume = service
        .resume_session(&SessionResumeRequest {
            session_id: session.id.clone(),
            metadata_only: true,
            transcript_before_sequence: None,
            transcript_limit: 0,
        })
        .await
        .expect("first resume");
    let second_resume = service
        .resume_session(&SessionResumeRequest {
            session_id: session.id.clone(),
            metadata_only: true,
            transcript_before_sequence: None,
            transcript_limit: 0,
        })
        .await
        .expect("second resume");
    for snapshot in [&first_resume, &second_resume] {
        let active = snapshot.active_run.as_ref().expect("active run");
        assert_eq!(active.id, run.id);
        assert_eq!(active.generation, run.generation);
    }

    let first = service
        .subscribe(
            ClientId("resume-client-one".into()),
            &EventSubscriptionRequest {
                session_id: session.id.clone(),
                after_sequence: first_resume.snapshot_sequence,
            },
        )
        .await
        .expect("first subscription");
    let second = service
        .subscribe(
            ClientId("resume-client-two".into()),
            &EventSubscriptionRequest {
                session_id: session.id.clone(),
                after_sequence: second_resume.snapshot_sequence,
            },
        )
        .await
        .expect("second subscription");
    let first_cancel = CancellationToken::new();
    let second_cancel = CancellationToken::new();
    let mut first_stream = service
        .open_event_stream(&first.subscription.id, first_cancel.clone())
        .expect("first stream");
    let mut second_stream = service
        .open_event_stream(&second.subscription.id, second_cancel.clone())
        .expect("second stream");
    store
        .append_event(
            &session.id,
            Some(&run.id),
            "after.rejoin",
            json!({ "once": true }),
        )
        .await
        .expect("event");

    for stream in [&mut first_stream, &mut second_stream] {
        let batch = tokio::time::timeout(Duration::from_secs(1), stream.recv())
            .await
            .expect("event timeout")
            .expect("event batch");
        assert_eq!(batch.catch_up.len(), 1);
        assert_eq!(batch.catch_up[0].event_name(), "after.rejoin");
        assert!(
            tokio::time::timeout(Duration::from_millis(150), stream.recv())
                .await
                .is_err(),
            "the post-snapshot event must not be replayed twice"
        );
    }
    assert!(service.unsubscribe(&first.subscription.id));
    assert_eq!(
        store
            .get_run(&run.id)
            .await
            .expect("run")
            .expect("run")
            .status,
        RunStatus::Running
    );
    first_cancel.cancel();
    second_cancel.cancel();
}
