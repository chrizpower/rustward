use super::*;
use crate::{
    admission::{Admission, Store},
    config,
    safety::Safety,
};
use serde_json::Value;
use std::collections::BTreeMap;

#[test]
fn admission_and_cancel_count_commits_only_and_leave_secrets_out() {
    let (sender, records) = mpsc::channel();
    let telemetry = Telemetry::start(move |bytes| {
        sender
            .send(serde_json::from_slice::<Value>(bytes).unwrap())
            .unwrap();
        Ok(())
    })
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.db");
    let mut store = Store::open_with_safety(&path, Safety::observed(telemetry.clone())).unwrap();
    telemetry.seed_queue(store.queue_depth());
    telemetry.state(0, State::Idle);
    let config = config::load("examples/rustward.toml").unwrap();
    let secret = "synthetic secret value";
    let values = BTreeMap::from([("name".to_string(), secret.into())]);
    let submit = |store: &mut Store, key: &str| {
        store.admit(
            key,
            &config,
            "synthetic-report",
            &values,
            "synthetic-runtime",
        )
    };
    assert_eq!(
        submit(&mut store, "sensitive bad key"),
        Err(Error::InvalidKey)
    );
    let key = store.issue_key().unwrap();
    let Ok(Admission::Admitted { run_id }) = submit(&mut store, &key) else {
        panic!("admission failed")
    };
    assert_eq!(
        submit(&mut store, &key),
        Ok(Admission::AlreadyAdmitted {
            run_id: run_id.clone()
        })
    );
    assert!(telemetry.scrape().contains("rustward_queue_depth 1\n"));
    store.cancel_queued(&run_id).unwrap();
    store.cancel_queued(&run_id).unwrap();
    let scrape = telemetry.scrape();
    for metric in [
        "rustward_queue_depth 0",
        "rustward_admissions_total{trigger=\"manual\"} 1",
        "rustward_admission_rejections_total{reason=\"invalid_key\"} 1",
        "rustward_completions_total{result=\"cancellation\"} 1",
        "rustward_run_duration_seconds_count 0",
    ] {
        assert!(scrape.contains(&format!("{metric}\n")), "{metric}");
    }
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TRIGGER reject_insert BEFORE INSERT ON admissions
        BEGIN SELECT RAISE(ABORT, 'secret error'); END;",
    )
    .unwrap();
    let fresh = store.issue_key().unwrap();
    assert_eq!(submit(&mut store, &fresh), Err(Error::Storage));
    assert!(
        telemetry
            .scrape()
            .contains("rustward_storage_failures_total{component=\"state\"} 1")
    );
    assert!(
        telemetry
            .scrape()
            .contains("rustward_admissions_total{trigger=\"manual\"} 1")
    );
    telemetry.flush();
    let emitted: Vec<_> = records.try_iter().collect();
    assert!(
        emitted
            .iter()
            .any(|entry| entry["event"] == "run_admitted" && entry["run_id"] == run_id)
    );
    assert!(emitted.iter().all(|entry| entry["timestamp_ms"].is_i64()));
    let serialized = serde_json::to_string(&emitted).unwrap();
    for forbidden in [&key, &fresh, secret, "sensitive bad key", "secret error"] {
        assert!(!serialized.contains(forbidden));
        assert!(!scrape.contains(forbidden));
    }

    drop(store);
    let restarted = Telemetry::start(|_| Ok(())).unwrap();
    let mut store = Store::open_with_safety(&path, Safety::observed(restarted.clone())).unwrap();
    restarted.seed_queue(store.queue_depth());
    assert!(matches!(
        submit(&mut store, &key),
        Ok(Admission::AlreadyAdmitted { .. })
    ));
    assert!(
        restarted
            .scrape()
            .contains("rustward_admissions_total{trigger=\"manual\"} 0")
    );
    assert!(
        restarted
            .scrape()
            .contains("rustward_completions_total{result=\"cancellation\"} 0")
    );
}

#[test]
fn schedule_counts_only_committed_occurrences_and_known_gaps() {
    let telemetry = Telemetry::start(|_| Ok(())).unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store =
        Store::open_with_safety(root.path().join("db"), Safety::observed(telemetry.clone()))
            .unwrap();
    telemetry.seed_queue(store.queue_depth());
    telemetry.state(0, State::Idle);
    let config = config::load("examples/rustward.toml").unwrap();
    let matchers = admission::schedule::matchers(&config);
    let due = 2 * 60 * 60_000;
    store
        .activate_schedules(&config, &matchers, due - 60_000)
        .unwrap();
    store
        .evaluate_schedules(&config, &matchers, "runtime", due, false)
        .unwrap();
    store
        .evaluate_schedules(&config, &matchers, "runtime", due, false)
        .unwrap();
    store
        .evaluate_schedules(&config, &matchers, "runtime", due + 120_000, true)
        .unwrap();
    assert!(
        telemetry
            .scrape()
            .contains("rustward_admissions_total{trigger=\"schedule\"} 1")
    );
    assert!(telemetry.scrape().contains("rustward_queue_depth 1\n"));
    assert!(
        telemetry
            .scrape()
            .contains("rustward_schedule_skips_total{reason=\"evaluation_gap\"} 0")
    );
    let soon = due + 23 * 60 * 60_000;
    store
        .evaluate_schedules(&config, &matchers, "runtime", soon, false)
        .unwrap();
    store
        .evaluate_schedules(&config, &matchers, "runtime", due + 24 * 60 * 60_000, false)
        .unwrap();
    assert!(
        telemetry
            .scrape()
            .contains("rustward_schedule_skips_total{reason=\"evaluation_gap\"} 1")
    );
}

#[test]
fn a_stalled_log_sink_drops_records_without_blocking_the_producer() {
    let (entered, waiting) = mpsc::sync_channel(1);
    let (release, blocked) = mpsc::sync_channel(1);
    let telemetry = Telemetry::start(move |_| {
        if entered.try_send(()).is_ok() {
            let _ = blocked.recv_timeout(Duration::from_secs(3));
        }
        Err(io::ErrorKind::BrokenPipe.into())
    })
    .unwrap();
    telemetry.log(Event::new("first", "test"));
    waiting.recv_timeout(Duration::from_secs(1)).unwrap();
    let producer = telemetry.clone();
    let (finish, finished) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        for _ in 0..LOG_QUEUE + 3 {
            producer.log(Event::new("record", "test"));
        }
        finish.send(()).unwrap();
    });
    let progress = finished.recv_timeout(Duration::from_secs(1));
    let scrape = telemetry.scrape();
    release.send(()).unwrap();
    drop(waiting);
    worker.join().unwrap();
    progress.unwrap();
    assert!(scrape.contains("rustward_service_logs_dropped_total 3\n"));
}

#[test]
fn histogram_block_and_storage_samples_have_fixed_labels() {
    let telemetry = Telemetry::start(|_| Ok(())).unwrap();
    telemetry.seed_queue(Some(3));
    telemetry.state(1, State::Review);
    telemetry.completed(
        "synthetic-run",
        "synthetic-task",
        "success",
        Some(850),
        true,
    );
    telemetry.completed(
        "slow-run",
        "synthetic-task",
        "timeout",
        Some(3_600_001),
        false,
    );
    telemetry.add(Counter::LogFailure, 1);
    let scrape = telemetry.scrape();
    assert!(scrape.len() < 8192);
    for metric in [
        "rustward_dispatch_state{state=\"review\"} 1",
        "rustward_dispatch_state{state=\"log_capacity\"} 0",
        "rustward_active_runs 1",
        "rustward_run_duration_seconds_bucket{le=\"0.5\"} 0",
        "rustward_run_duration_seconds_bucket{le=\"1\"} 1",
        "rustward_run_duration_seconds_bucket{le=\"+Inf\"} 2",
        "rustward_run_duration_seconds_sum 3600.851",
        "rustward_log_truncations_total 1",
        "rustward_storage_failures_total{component=\"task_log\"} 1",
    ] {
        assert!(scrape.contains(&format!("{metric}\n")), "{metric}");
    }
    assert!(!scrape.contains("synthetic-run"));
    telemetry.state(0, State::LogCapacity);
    assert!(
        telemetry
            .scrape()
            .contains("rustward_dispatch_state{state=\"log_capacity\"} 1")
    );
    telemetry.state(0, State::Failed);
    assert!(!telemetry.scrape().contains("rustward_queue_depth 3"));
    assert!(
        telemetry
            .scrape()
            .contains("rustward_queue_depth_available 0")
    );
}

#[test]
fn rejected_submissions_and_scheduled_skips_use_bounded_reasons() {
    let telemetry = Telemetry::start(|_| Ok(())).unwrap();
    for (error, reason) in [
        (Error::InvalidInputs, "invalid_inputs"),
        (Error::TooLarge, "too_large"),
        (Error::PayloadConflict, "payload_conflict"),
        (Error::QueueFull, "queue_full"),
        (Error::Closed, "closed"),
        (Error::Busy, "busy"),
        (Error::InvalidRuntime, "other"),
    ] {
        telemetry.rejected(&error);
        assert!(telemetry.scrape().contains(&format!(
            "rustward_admission_rejections_total{{reason=\"{reason}\"}} 1\n"
        )));
    }
    for reason in [
        "paused",
        "admission_closed",
        "schedule_queued",
        "queue_full",
        "too_large",
    ] {
        telemetry.occurrence("private schedule", "private task", reason, None);
        assert!(telemetry.scrape().contains(&format!(
            "rustward_schedule_skips_total{{reason=\"{reason}\"}} 1\n"
        )));
    }
    telemetry.gap("private schedule", "activation", Some(7));
    assert!(
        telemetry
            .scrape()
            .contains("rustward_schedule_skips_total{reason=\"activation\"} 7")
    );
    assert!(!telemetry.scrape().contains("private schedule"));
}

#[test]
fn service_log_write_errors_are_counted_separately_from_task_storage() {
    let telemetry = Telemetry::start(|_| Err(io::ErrorKind::BrokenPipe.into())).unwrap();
    telemetry.log(Event::new("run_completed", "execution"));
    telemetry.flush();
    let scrape = telemetry.scrape();
    assert!(scrape.contains("rustward_service_logs_dropped_total 1"));
    assert!(scrape.contains("rustward_storage_failures_total{component=\"task_log\"} 0"));
    assert!(scrape.contains("rustward_storage_failures_total{component=\"state\"} 0"));
}

#[test]
fn monitoring_stales_the_service_heartbeat_and_distinguishes_readiness() {
    let metrics = metrics::Metrics::default();
    assert_eq!(metrics.probe().state, "unresponsive");
    for (state, live, ready, label) in [
        (State::Idle, true, true, "idle"),
        (State::Paused, true, false, "paused"),
        (State::Review, true, false, "review"),
        (State::ExecutionCapacity, true, true, "execution_capacity"),
        (State::LogCapacity, true, false, "log_capacity"),
        (State::Draining, true, false, "draining"),
        (State::Failed, false, false, "failed"),
    ] {
        metrics.state(0, state);
        let current = metrics.probe();
        assert_eq!(
            (current.live, current.ready, current.state),
            (live, ready, label)
        );
    }
    metrics.state(0, State::Idle);
    let stalled = metrics.probe_at(u64::MAX);
    assert_eq!(
        (stalled.live, stalled.ready, stalled.state),
        (false, false, "unresponsive")
    );
}

#[test]
fn unknown_schedule_gaps_are_visible_without_inventing_skipped_occurrences() {
    let telemetry = Telemetry::start(|_| Ok(())).unwrap();
    telemetry.gap("first", "first_installation", None);
    assert!(
        !telemetry
            .scrape()
            .lines()
            .any(|line| line.starts_with("rustward_last_schedule_gap_timestamp_seconds "))
    );
    telemetry.gap("first", "clock_change", None);
    let data = telemetry.scrape();
    assert!(data.contains("rustward_schedule_unknown_gaps_total 1\n"));
    assert!(data.contains("rustward_schedule_skips_total{reason=\"activation\"} 0"));
    assert!(
        data.lines()
            .any(|line| line.starts_with("rustward_last_schedule_gap_timestamp_seconds "))
    );
}
