use super::*;

#[test]
fn setup_failures_have_no_supervisor_call() {
    for failure in [
        Failure::Runtime(dispatch::LaunchFailure::RuntimeChanged),
        Failure::Runtime(dispatch::LaunchFailure::WorkStorage),
        Failure::Capture(capture::SetupError::AlreadyExists),
        Failure::Capture(capture::SetupError::Storage),
        Failure::Capture(capture::SetupError::Resources),
    ] {
        let mut f = Fixture::new();
        let (_, id) = f.admit();
        *f.preparation.failure.lock().unwrap() = Some(failure);
        let mut c = f.controller(Limits::default());
        c.set_paused(false).unwrap();
        c.start_next().unwrap();
        completed(&mut c, &id);
        assert!(f.fake.0.lock().unwrap().launches.is_empty());
        let r = c.record(&id).unwrap().unwrap();
        assert!(r.launch_failure.is_some() || r.capture_setup_failure.is_some());
        assert_eq!(r.output_evidence, None);
        assert_eq!(
            c.run_state(&id).unwrap().unwrap().result,
            Some(RunResult::Failure)
        );
    }
}

#[test]
fn rejection_completes_but_lost_reply_and_conflict_require_review() {
    for outcome in [
        StartOutcome::Rejected,
        StartOutcome::Uncertain,
        StartOutcome::Conflict,
    ] {
        let mut f = Fixture::new();
        let (_, id) = f.admit();
        f.fake.0.lock().unwrap().reject = Some(outcome);
        let mut c = f.controller(Limits::default());
        c.set_paused(false).unwrap();
        c.start_next().unwrap();
        started(&mut c, &f);
        if outcome != StartOutcome::Rejected {
            f.ended(&id);
        }
        completed(&mut c, &id);
        assert_eq!(f.fake.0.lock().unwrap().launches.len(), 1);
        let rejected = outcome == StartOutcome::Rejected;
        assert_eq!(c.record(&id).unwrap().unwrap().review_required, !rejected);
        assert_eq!(
            c.run_state(&id).unwrap().unwrap().result,
            Some(if rejected {
                RunResult::Failure
            } else {
                RunResult::Interruption
            })
        );
    }
}

#[test]
fn emergency_stop_does_not_need_polling_database_or_preparation() {
    let mut f = Fixture::new();
    let (_, id) = f.admit();
    let (_, second) = f.admit();
    let mut c = f.controller(Limits {
        active_runs: 2,
        ..Limits::default()
    });
    c.set_paused(false).unwrap();
    c.start_next().unwrap();
    started(&mut c, &f);
    f.preparation.hold.store(true, Ordering::Release);
    f.preparation.entered.store(false, Ordering::Release);
    c.start_next().unwrap();
    until(|| f.preparation.entered.load(Ordering::Acquire));
    let db = rusqlite::Connection::open(&f.path).unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let monitor = c
        .registry
        .snapshots()
        .into_iter()
        .find(|e| e.id.as_str() == id)
        .unwrap()
        .monitor
        .unwrap();
    monitor.inject_failure(capture::FailureKind::Read);
    monitor.inject_failure(capture::FailureKind::Sync);
    until(|| !f.fake.0.lock().unwrap().stops.is_empty());
    assert!(c.failed());
    assert!(monitor.storage_failed());
    assert_eq!(monitor.failure().unwrap().kind, capture::FailureKind::Read);
    assert!(
        f.fake
            .0
            .lock()
            .unwrap()
            .stops
            .iter()
            .all(|(run, _)| *run == id)
    );
    db.execute_batch("ROLLBACK").unwrap();
    f.preparation.hold.store(false, Ordering::Release);
    f.ended(&id);
    until(|| {
        let _ = c.tick(Instant::now());
        c.run_state(&second).unwrap().unwrap().phase == RunPhase::Completed
    });
    completed(&mut c, &id);
    assert_eq!(
        c.run_state(&id).unwrap().unwrap().result,
        Some(RunResult::Interruption)
    );
    assert_eq!(f.fake.0.lock().unwrap().launches.len(), 1);
}

#[test]
fn missing_unit_does_not_release_capacity_and_release_failure_requires_review() {
    let mut f = Fixture::new();
    let (_, id) = f.admit();
    let mut c = f.controller(Limits::default());
    c.set_paused(false).unwrap();
    c.start_next().unwrap();
    started(&mut c, &f);
    f.fake.0.lock().unwrap().observations.clear();
    c.tick(Instant::now()).unwrap();
    assert_eq!(c.active_runs(), 1);
    assert!(!c.record(&id).unwrap().unwrap().cleanup_confirmed);
    f.ended(&id);
    f.fake.0.lock().unwrap().release_failure = true;
    c.tick(Instant::now()).unwrap();
    assert_eq!(c.active_runs(), 1);
    f.fake.0.lock().unwrap().release_failure = false;
    completed(&mut c, &id);
    assert!(c.record(&id).unwrap().unwrap().review_required);
}

#[test]
fn manager_uid_change_closes_execution_without_a_start() {
    let mut f = Fixture::new();
    let (_, id) = f.admit();
    f.fake.0.lock().unwrap().uid = Some(1001);
    let mut c = f.controller(Limits::default());
    c.set_paused(false).unwrap();
    c.start_next().unwrap();
    until(|| {
        let _ = c.tick(Instant::now());
        c.failed()
    });
    assert!(f.fake.0.lock().unwrap().launches.is_empty());
    assert_ne!(
        c.run_state(&id).unwrap().unwrap().phase,
        RunPhase::Completed
    );
}

#[test]
fn capture_failure_preserves_an_earlier_failure_but_replaces_success() {
    for code in [0, 1] {
        let mut f = Fixture::new();
        let (_, id) = f.admit();
        let mut c = f.controller(Limits::default());
        c.set_paused(false).unwrap();
        c.start_next().unwrap();
        started(&mut c, &f);
        f.fake.0.lock().unwrap().observations.insert(
            id.clone(),
            Observation::Known {
                identity: identity(),
                state: ExecutionState::Exited,
                exit: Some(ProcessExit::Code(code)),
                cleanup: Cleanup::Pending,
            },
        );
        c.tick(Instant::now()).unwrap();
        let monitor = c.registry.snapshots()[0].monitor.clone().unwrap();
        monitor.inject_failure(capture::FailureKind::Read);
        until(|| !f.fake.0.lock().unwrap().stops.is_empty());
        f.ended(&id);
        completed(&mut c, &id);
        assert_eq!(
            c.run_state(&id).unwrap().unwrap().result,
            Some(if code == 0 {
                RunResult::Interruption
            } else {
                RunResult::Failure
            })
        );
        assert!(c.record(&id).unwrap().unwrap().output_failed);
    }
}

#[test]
fn failed_completion_commit_preserves_capacity_and_does_not_publish_output_evidence() {
    let mut f = Fixture::new();
    let (_, id) = f.admit();
    let mut c = f.controller(Limits::default());
    c.set_paused(false).unwrap();
    c.start_next().unwrap();
    started(&mut c, &f);
    let db = rusqlite::Connection::open(&f.path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_output BEFORE UPDATE OF output_evidence ON launches BEGIN SELECT RAISE(ABORT,'synthetic'); END").unwrap();
    f.ended(&id);
    until(|| c.tick(Instant::now()).is_err());
    assert!(c.failed());
    assert_eq!(c.active_runs(), 1);
    assert_eq!(
        db.query_row("SELECT phase FROM runs WHERE run_id=?1", [&id], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "cleaning_up"
    );
    assert!(
        db.query_row(
            "SELECT output_evidence FROM launches WHERE run_id=?1",
            [&id],
            |r| r.get::<_, Option<String>>(0)
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn unfinished_output_recovery_never_supports_success_and_missing_files_require_review() {
    let mut f = Fixture::new();
    let (_, id) = f.admit();
    {
        let mut c = f.controller(Limits::default());
        c.set_paused(false).unwrap();
        c.start_next().unwrap();
        started(&mut c, &f);
        f.fake.0.lock().unwrap().observations.insert(
            id.clone(),
            Observation::Known {
                identity: identity(),
                state: ExecutionState::Exited,
                exit: Some(ProcessExit::Code(0)),
                cleanup: Cleanup::Pending,
            },
        );
        c.tick(Instant::now()).unwrap();
    }
    fs::remove_file(f.preparation.logs.join(&id).join("stderr.log")).unwrap();
    f.ended(&id);
    f.safety = Safety::default();
    let mut c = f.controller(Limits::default());
    completed(&mut c, &id);
    let r = c.record(&id).unwrap().unwrap();
    assert!(r.review_required);
    assert!(!r.output_evidence.unwrap().synchronized);
    assert_eq!(
        c.run_state(&id).unwrap().unwrap().result,
        Some(RunResult::Interruption)
    );
}

#[test]
fn broken_capture_before_start_consumes_the_permit_without_a_launch() {
    let mut f = Fixture::new();
    let (_, id) = f.admit();
    f.preparation
        .hold_after_capture
        .store(true, Ordering::Release);
    let mut c = f.controller(Limits::default());
    c.set_paused(false).unwrap();
    c.start_next().unwrap();
    until(|| c.registry.snapshots()[0].monitor.is_some());
    c.registry.snapshots()[0]
        .monitor
        .as_ref()
        .unwrap()
        .inject_failure(capture::FailureKind::Read);
    f.preparation
        .hold_after_capture
        .store(false, Ordering::Release);
    completed(&mut c, &id);
    assert!(f.fake.0.lock().unwrap().launches.is_empty());
    let record = c.record(&id).unwrap().unwrap();
    assert!(record.output_failed);
    assert!(record.review_required);
    assert_eq!(
        c.run_state(&id).unwrap().unwrap().result,
        Some(RunResult::Interruption)
    );
}

#[test]
fn review_during_preparation_prevents_another_start_and_retains_no_start_cleanup() {
    let mut f = Fixture::new();
    let (_, first) = f.admit();
    let (_, second) = f.admit();
    let mut c = f.controller(Limits {
        active_runs: 2,
        ..Limits::default()
    });
    c.set_paused(false).unwrap();
    c.start_next().unwrap();
    started(&mut c, &f);
    f.preparation
        .hold_after_capture
        .store(true, Ordering::Release);
    c.start_next().unwrap();
    until(|| {
        c.registry
            .snapshots()
            .iter()
            .all(|entry| entry.monitor.is_some())
    });
    c.registry
        .snapshots()
        .into_iter()
        .find(|entry| entry.id.as_str() == first)
        .unwrap()
        .monitor
        .unwrap()
        .inject_failure(capture::FailureKind::Read);
    c.tick(Instant::now()).unwrap();
    f.preparation
        .hold_after_capture
        .store(false, Ordering::Release);
    completed(&mut c, &second);
    assert_eq!(f.fake.0.lock().unwrap().launches.len(), 1);
    let record = c.record(&second).unwrap().unwrap();
    assert!(record.cleanup_confirmed);
    assert!(record.review_required);
    assert_eq!(
        c.run_state(&second).unwrap().unwrap().result,
        Some(RunResult::Interruption)
    );
    f.ended(&first);
    completed(&mut c, &first);
}
