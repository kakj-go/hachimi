use std::time::Duration;

#[tokio::test]
#[ignore = "real SystemClock release soak"]
async fn system_clock_at_every_and_six_field_cron_soak_without_duplicate_invocations() {
    let _short_interval_guard = crate::calendar::enable_release_soak_short_intervals();
    let store = AgentStore::connect_in_memory().await.expect("store");
    let launches = Arc::new(AtomicUsize::new(0));
    let service = Arc::new(SchedulerService::new(
        store.clone(),
        Arc::new(SystemClock),
        Arc::new(BundledIanaTimeZoneResolver),
        Arc::new(CountingLauncher(Arc::clone(&launches))),
        Arc::new(NoopNotificationAdapter),
    ));
    let now = SystemClock.now_ms();

    let mut at = definition(now);
    at.id = ScheduleId::from("system-clock-at");
    at.name = "System clock At".into();
    at.schedule = ScheduleSpec::At {
        timestamp_ms: now + 800,
    };
    service
        .create("release-soak", "system-clock-at", at, true)
        .await
        .expect("At schedule");

    let mut every = definition(now);
    every.id = ScheduleId::from("system-clock-every");
    every.name = "System clock Every".into();
    every.schedule = ScheduleSpec::Every {
        interval_ms: 300,
        anchor_ms: now + 300,
    };
    service
        .create("release-soak", "system-clock-every", every, true)
        .await
        .expect("Every schedule");

    let mut cron = definition(now);
    cron.id = ScheduleId::from("system-clock-cron");
    cron.name = "System clock Cron".into();
    cron.schedule = ScheduleSpec::Cron {
        expression: "*/1 * * * * *".into(),
        timezone: "UTC".into(),
    };
    service
        .create("release-soak", "system-clock-cron", cron, true)
        .await
        .expect("Cron schedule");

    let handle = Arc::clone(&service).start();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while tokio::time::Instant::now() < deadline {
        let tasks = store.list_task_runs(None, 500).await.expect("soak tasks");
        let every_count = tasks
            .iter()
            .filter(|task| {
                task.schedule_id.as_ref() == Some(&ScheduleId::from("system-clock-every"))
            })
            .count();
        let at_seen = tasks
            .iter()
            .any(|task| task.schedule_id.as_ref() == Some(&ScheduleId::from("system-clock-at")));
        let cron_seen = tasks
            .iter()
            .any(|task| task.schedule_id.as_ref() == Some(&ScheduleId::from("system-clock-cron")));
        if every_count >= 20 && at_seen && cron_seen {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    drop(handle);
    assert!(
        launches.load(Ordering::SeqCst) >= 22,
        "natural timer did not produce the required 20+ occurrence soak"
    );
    tokio::time::sleep(Duration::from_millis(100)).await;

    let tasks = store.list_task_runs(None, 500).await.expect("task runs");
    let keys = tasks
        .iter()
        .map(|task| task.invocation_key.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        keys.len(),
        tasks.len(),
        "an occurrence was invoked more than once"
    );
    assert!(tasks
        .iter()
        .any(|task| task.schedule_id.as_ref() == Some(&ScheduleId::from("system-clock-at"))));
    assert!(tasks
        .iter()
        .any(|task| task.schedule_id.as_ref() == Some(&ScheduleId::from("system-clock-cron"))));
    let mut every_occurrences = tasks
        .iter()
        .filter(|task| {
            task.schedule_id.as_ref() == Some(&ScheduleId::from("system-clock-every"))
        })
        .filter_map(|task| task.scheduled_for_ms)
        .collect::<Vec<_>>();
    every_occurrences.sort_unstable();
    assert!(every_occurrences.len() >= 20);
    assert!(
        every_occurrences
            .windows(2)
            .all(|pair| pair[1] > pair[0] && (pair[1] - pair[0]) % 300 == 0),
        "Every occurrences drifted away from their fixed anchor"
    );
    assert!(
        service.active_launches.lock().is_empty(),
        "completed natural-clock invocations leaked active workers"
    );
    let at = store
        .get_schedule(&ScheduleId::from("system-clock-at"))
        .await
        .expect("At schedule")
        .expect("At row");
    assert!(
        !at.enabled,
        "one-shot At schedule was not disabled after firing"
    );
}
