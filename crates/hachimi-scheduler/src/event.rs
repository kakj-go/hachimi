//! Typed, authority-neutral event ingress for Event Schedules.

use hachimi_protocol::{
    ScheduleEventContext, ScheduleEventEnvelope, ScheduleEventReceipt, ScheduleEventResourceRef,
};
use sha2::{Digest, Sha256};

use crate::{SchedulerError, SchedulerService};

impl SchedulerService {
    pub async fn ingest_event(
        &self,
        envelope: ScheduleEventEnvelope,
    ) -> Result<ScheduleEventReceipt, SchedulerError> {
        self.ensure_accepting()?;
        validate_event_envelope(&envelope)?;
        let received_at_ms = self.clock.now_ms();
        let fingerprint = event_fingerprint(&envelope)?;
        let event = ScheduleEventContext {
            event_id: envelope.event_id,
            source: envelope.source,
            event_type: envelope.event_type,
            subject: envelope.subject,
            labels: envelope.labels,
            resource: envelope.resource,
            fingerprint,
            occurred_at_ms: envelope.occurred_at_ms,
            received_at_ms,
        };
        self.store
            .cleanup_schedule_event_ledger(received_at_ms)
            .await?;
        let claim = self.store.ingest_schedule_event(&event).await?;
        let receipt = claim.receipt;
        for launch in claim.launch_claims {
            self.launch_claim(launch.schedule, launch.claim);
        }
        Ok(receipt)
    }
}

fn event_fingerprint(envelope: &ScheduleEventEnvelope) -> Result<String, SchedulerError> {
    let bytes = serde_json::to_vec(envelope)?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn validate_event_envelope(envelope: &ScheduleEventEnvelope) -> Result<(), SchedulerError> {
    validate_text("event id", &envelope.event_id, 256)?;
    validate_text("source principal", &envelope.source.principal, 256)?;
    validate_text("source id", &envelope.source.id, 256)?;
    validate_text("event type", &envelope.event_type, 256)?;
    if envelope
        .subject
        .as_ref()
        .is_some_and(|value| value.chars().count() > 512)
    {
        return Err(SchedulerError::InvalidSchedule(
            "event subject must contain at most 512 characters".into(),
        ));
    }
    if envelope.labels.len() > 16 {
        return Err(SchedulerError::InvalidSchedule(
            "event envelope supports at most 16 labels".into(),
        ));
    }
    for (key, value) in &envelope.labels {
        validate_text("event label key", key, 128)?;
        if value.chars().count() > 256 {
            return Err(SchedulerError::InvalidSchedule(
                "event label values must contain at most 256 characters".into(),
            ));
        }
    }
    if let Some(resource) = &envelope.resource {
        validate_resource(resource)?;
    }
    Ok(())
}

fn validate_resource(resource: &ScheduleEventResourceRef) -> Result<(), SchedulerError> {
    validate_text("event resource kind", &resource.kind, 128)?;
    validate_text("event resource id", &resource.id, 512)?;
    if resource
        .revision
        .as_ref()
        .is_some_and(|value| value.is_empty() || value.chars().count() > 256)
    {
        return Err(SchedulerError::InvalidSchedule(
            "event resource revision must contain 1-256 characters".into(),
        ));
    }
    Ok(())
}

fn validate_text(field: &str, value: &str, maximum: usize) -> Result<(), SchedulerError> {
    let length = value.chars().count();
    if length == 0 || length > maximum {
        return Err(SchedulerError::InvalidSchedule(format!(
            "{field} must contain 1-{maximum} characters"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::{
            Arc,
            atomic::{AtomicI64, AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use hachimi_protocol::{
        AgentPermissionPolicy, DeliveryPolicy, EntryProfile, HostRevisionSnapshot, MisfirePolicy,
        PermissionProfile, ScheduleContextTemplate, ScheduleDefinition, ScheduleEventEnvelope,
        ScheduleEventMatcher, ScheduleEventSource, ScheduleEventSourceKind, ScheduleHealth,
        ScheduleId, ScheduleSpec, ScheduleStopConditions, TaskRunRecord, TaskRunStatus,
        TaskRunTrigger, WorkloadKind,
    };
    use hachimi_storage::{AgentStore, AgentStoreError};
    use tokio_util::sync::CancellationToken;

    use crate::{
        BundledIanaTimeZoneResolver, Clock, NoopNotificationAdapter, ScheduleLaunchFuture,
        ScheduleRunCompletion, ScheduleRunLauncher, SchedulerError, SchedulerService,
    };

    #[derive(Debug)]
    struct TestClock(AtomicI64);

    impl Clock for TestClock {
        fn now_ms(&self) -> i64 {
            self.0.fetch_add(1, Ordering::SeqCst)
        }
    }

    #[derive(Debug)]
    struct CountingLauncher(Arc<AtomicUsize>);

    impl ScheduleRunLauncher for CountingLauncher {
        fn launch(
            &self,
            _schedule: ScheduleDefinition,
            _task_run: TaskRunRecord,
            _cancellation: CancellationToken,
        ) -> ScheduleLaunchFuture {
            let launches = Arc::clone(&self.0);
            Box::pin(async move {
                launches.fetch_add(1, Ordering::SeqCst);
                Ok(ScheduleRunCompletion {
                    status: TaskRunStatus::Succeeded,
                    result_summary: Some("event handled".into()),
                    error_code: None,
                    error_summary: None,
                    artifact_ids: Vec::new(),
                })
            })
        }
    }

    fn event_definition(
        id: &str,
        source_kind: ScheduleEventSourceKind,
        source_id: &str,
        event_type: &str,
    ) -> ScheduleDefinition {
        ScheduleDefinition {
            id: ScheduleId::from(id),
            name: id.into(),
            enabled: true,
            prompt: "Handle the referenced resource using authorized tools.".into(),
            schedule: ScheduleSpec::Event {
                matcher: ScheduleEventMatcher {
                    source: ScheduleEventSource {
                        kind: source_kind,
                        principal: "host:trusted".into(),
                        id: source_id.into(),
                    },
                    event_type: event_type.into(),
                    subject_prefix: Some("resource://".into()),
                    labels: BTreeMap::from([("environment".into(), "test".into())]),
                    resource: None,
                },
            },
            entry_profile: EntryProfile::Workbench,
            workload_override: Some(WorkloadKind::Office),
            context_template: ScheduleContextTemplate::Workspace {
                workspace: hachimi_protocol::ScheduleWorkspaceSpec::Managed,
                conversation_mode: hachimi_protocol::ScheduleConversationMode::PerRunSession,
            },
            skill_allowlist: Vec::new(),
            skill_revisions: Vec::new(),
            mcp_tool_allowlist: Vec::new(),
            contribution_revisions: Vec::new(),
            host_revision_snapshot: HostRevisionSnapshot::default(),
            permission_policy: AgentPermissionPolicy {
                level: PermissionProfile::ReadOnly,
                ..AgentPermissionPolicy::default()
            },
            permission_revision: 0,
            timeout_ms: 120_000,
            misfire_policy: MisfirePolicy::Skip,
            delivery_policy: DeliveryPolicy::TaskTabOnly,
            stop_conditions: ScheduleStopConditions::default(),
            config_revision: 0,
            created_by: String::new(),
            next_run_at_ms: Some(123),
            health: ScheduleHealth::Invalid,
            health_reason: None,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    fn envelope(
        event_id: &str,
        source_kind: ScheduleEventSourceKind,
        source_id: &str,
        event_type: &str,
    ) -> ScheduleEventEnvelope {
        ScheduleEventEnvelope {
            event_id: event_id.into(),
            source: ScheduleEventSource {
                kind: source_kind,
                principal: "host:trusted".into(),
                id: source_id.into(),
            },
            event_type: event_type.into(),
            subject: Some("resource://document/1".into()),
            labels: BTreeMap::from([("environment".into(), "test".into())]),
            resource: None,
            occurred_at_ms: 1_800_000_000_000,
        }
    }

    #[tokio::test]
    async fn all_local_source_adapters_share_exact_matching_and_event_only_timing() {
        let launches = Arc::new(AtomicUsize::new(0));
        let service = SchedulerService::new(
            AgentStore::connect_in_memory().await.expect("store"),
            Arc::new(TestClock(AtomicI64::new(1_800_000_000_100))),
            Arc::new(BundledIanaTimeZoneResolver),
            Arc::new(CountingLauncher(Arc::clone(&launches))),
            Arc::new(NoopNotificationAdapter),
        );
        let kinds = [
            ScheduleEventSourceKind::Workspace,
            ScheduleEventSourceKind::Plugin,
            ScheduleEventSourceKind::Connector,
            ScheduleEventSourceKind::Channel,
            ScheduleEventSourceKind::Gateway,
        ];
        for (index, kind) in kinds.into_iter().enumerate() {
            let source_id = format!("source-{index}");
            let event_type = format!("resource.changed.{index}");
            let created = service
                .create(
                    "owner",
                    &format!("create-{index}"),
                    event_definition(&format!("schedule-{index}"), kind, &source_id, &event_type),
                )
                .await
                .expect("create event schedule");
            assert_eq!(created.definition.next_run_at_ms, None);
            assert!(service.preview(&created.definition.schedule, 5).valid);
            assert!(
                service
                    .preview(&created.definition.schedule, 5)
                    .next_occurrences_ms
                    .is_empty()
            );
            let receipt = service
                .ingest_event(envelope(
                    &format!("event-{index}"),
                    kind,
                    &source_id,
                    &event_type,
                ))
                .await
                .expect("ingest event");
            assert_eq!(receipt.matched_schedule_ids, vec![created.definition.id]);
            assert_eq!(receipt.task_runs[0].trigger, TaskRunTrigger::Event);
            assert!(receipt.task_runs[0].event_context.is_some());
        }
        tokio::time::timeout(Duration::from_secs(1), async {
            while launches.load(Ordering::SeqCst) < 5 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("all Event launches");
        assert_eq!(launches.load(Ordering::SeqCst), 5);
    }

    #[tokio::test]
    async fn event_id_replay_conflict_fan_out_and_source_principal_are_deterministic() {
        let service = SchedulerService::new(
            AgentStore::connect_in_memory().await.expect("store"),
            Arc::new(TestClock(AtomicI64::new(1_800_000_100_000))),
            Arc::new(BundledIanaTimeZoneResolver),
            Arc::new(CountingLauncher(Arc::new(AtomicUsize::new(0)))),
            Arc::new(NoopNotificationAdapter),
        );
        for id in ["fanout-a", "fanout-b"] {
            service
                .create(
                    "owner",
                    id,
                    event_definition(
                        id,
                        ScheduleEventSourceKind::Plugin,
                        "calendar",
                        "meeting.changed",
                    ),
                )
                .await
                .expect("create fanout schedule");
        }
        let event = envelope(
            "meeting-1",
            ScheduleEventSourceKind::Plugin,
            "calendar",
            "meeting.changed",
        );
        let mut untrusted = event.clone();
        untrusted.event_id = "untrusted".into();
        untrusted.source.principal = "caller:spoofed".into();
        assert!(
            service
                .ingest_event(untrusted)
                .await
                .expect("untrusted event receipt")
                .matched_schedule_ids
                .is_empty()
        );

        let accepted = service.ingest_event(event.clone()).await.expect("accepted");
        assert_eq!(
            accepted.status,
            hachimi_protocol::ScheduleEventReceiptStatus::Accepted
        );
        assert_eq!(accepted.matched_schedule_ids.len(), 2);
        let replayed = service.ingest_event(event.clone()).await.expect("replayed");
        assert_eq!(
            replayed.status,
            hachimi_protocol::ScheduleEventReceiptStatus::Replayed
        );
        assert_eq!(replayed.task_runs, accepted.task_runs);

        let mut conflicting = event;
        conflicting.subject = Some("resource://document/other".into());
        assert!(matches!(
            service.ingest_event(conflicting).await,
            Err(SchedulerError::Store(
                AgentStoreError::ScheduleEventConflict
            ))
        ));
        let receipts = service
            .store
            .list_schedule_event_receipts(10)
            .await
            .expect("event receipts");
        assert!(receipts.iter().any(|receipt| {
            receipt.event.event_id == "meeting-1"
                && receipt.status == hachimi_protocol::ScheduleEventReceiptStatus::Conflict
        }));
    }
}
