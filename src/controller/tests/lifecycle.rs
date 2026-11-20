use super::*;

#[test]
fn captures_arguments_output_and_completion_in_order() {
    let mut f = Fixture::new();
    let (_, id) = f.admit();
    f.admit();
    let mut c = f.controller(Limits::default());
    assert!(matches!(
        c.start_next().unwrap(),
        Start::Blocked(Block::Paused)
    ));
    c.set_paused(false).unwrap();
    assert!(matches!(c.start_next().unwrap(),Start::Preparing(run) if run.as_str()==id));
    started(&mut c, &f);
    assert!(matches!(
        c.start_next().unwrap(),
        Start::Blocked(Block::ExecutionCapacity)
    ));
    {
        let state = f.fake.0.lock().unwrap();
        assert_eq!(
            state.launches[0].arguments,
            ["--name", "sample", "2", "false"]
        );
        assert_eq!(state.profiles[0].environment().len(), 7);
    }
    assert_eq!(c.run_state(&id).unwrap().unwrap().phase, RunPhase::Running);
    f.ended(&id);
    completed(&mut c, &id);
    let state = c.run_state(&id).unwrap().unwrap();
    assert_eq!(state.result, Some(RunResult::Success));
    assert_eq!(c.active_runs(), 0);
    let evidence = c.record(&id).unwrap().unwrap().output_evidence.unwrap();
    assert!(evidence.synchronized);
    assert!(!evidence.recovered);
    assert_eq!(evidence.stdout.observed_bytes, Some(16));
    assert_eq!(evidence.stderr.retained_bytes, 15);
    assert_eq!(
        fs::read(f.preparation.logs.join(&id).join("stdout.log")).unwrap(),
        b"synthetic output"
    );
    assert_eq!(f.fake.0.lock().unwrap().released, [id]);
    assert!(!c.failed());
}

#[test]
fn eof_does_not_complete_a_running_command_and_budget_does_not_stop_it() {
    let mut f = Fixture::new();
    let (_, id) = f.admit();
    let mut c = f.controller(Limits {
        log_bytes_per_run: 5,
        ..Limits::default()
    });
    c.set_paused(false).unwrap();
    c.start_next().unwrap();
    started(&mut c, &f);
    f.fake.0.lock().unwrap().writers.clear();
    until(|| {
        c.tick(Instant::now()).unwrap();
        c.runs[&id].final_report.is_some()
    });
    assert_eq!(c.run_state(&id).unwrap().unwrap().phase, RunPhase::Running);
    assert!(f.fake.0.lock().unwrap().stops.is_empty());
    f.ended(&id);
    completed(&mut c, &id);
    let e = c.record(&id).unwrap().unwrap().output_evidence.unwrap();
    assert_eq!(e.retained_bytes(), Some(5));
    assert!(e.stdout.truncated == Some(true) || e.stderr.truncated == Some(true));
}

#[test]
fn cancellation_and_timeout_hold_capacity_until_cleanup() {
    for timeout in [false, true] {
        let mut f = Fixture::new();
        let (_, id) = f.admit();
        let mut c = f.controller(Limits::default());
        c.set_paused(false).unwrap();
        c.start_next().unwrap();
        started(&mut c, &f);
        if timeout {
            c.tick(Instant::now() + Duration::from_secs(3601)).unwrap();
        } else {
            assert_eq!(
                c.cancel_active(&id).unwrap(),
                dispatch::StopDecision::Recorded
            );
        }
        until(|| !f.fake.0.lock().unwrap().stops.is_empty());
        assert_eq!(c.active_runs(), 1);
        c.registry.step(
            &mut f.fake.clone(),
            Instant::now() + Duration::from_secs(3700),
        );
        assert!(
            f.fake
                .0
                .lock()
                .unwrap()
                .stops
                .iter()
                .any(|(_, mode)| *mode == StopMode::Force)
        );
        f.ended(&id);
        completed(&mut c, &id);
        assert_eq!(
            c.run_state(&id).unwrap().unwrap().result,
            Some(if timeout {
                RunResult::Timeout
            } else {
                RunResult::Cancellation
            })
        );
    }
}

#[test]
fn canceled_preparation_never_launches_and_keeps_its_slot_until_io_returns() {
    let mut f = Fixture::new();
    let (_, id) = f.admit();
    f.preparation.hold.store(true, Ordering::Release);
    let mut c = f.controller(Limits::default());
    c.set_paused(false).unwrap();
    c.start_next().unwrap();
    until(|| f.preparation.entered.load(Ordering::Acquire));
    c.cancel_active(&id).unwrap();
    c.tick(Instant::now()).unwrap();
    assert_eq!(c.active_runs(), 1);
    f.preparation.hold.store(false, Ordering::Release);
    until(|| {
        let _ = c.tick(Instant::now());
        c.run_state(&id).unwrap().unwrap().phase == RunPhase::Completed
    });
    assert!(f.fake.0.lock().unwrap().launches.is_empty());
    assert_eq!(
        c.run_state(&id).unwrap().unwrap().result,
        Some(RunResult::Cancellation)
    );
}

#[test]
fn recovery_synchronizes_partial_logs_without_starting_again() {
    let mut f = Fixture::new();
    let (_, id) = f.admit();
    {
        let mut c = f.controller(Limits::default());
        c.set_paused(false).unwrap();
        c.start_next().unwrap();
        started(&mut c, &f);
    }
    f.ended(&id);
    f.safety = Safety::default();
    let mut c = f.controller(Limits::default());
    completed(&mut c, &id);
    assert_eq!(f.fake.0.lock().unwrap().launches.len(), 1);
    assert_eq!(
        f.fake.0.lock().unwrap().references,
        vec![(id.clone(), Some(identity()))]
    );
    let r = c.record(&id).unwrap().unwrap();
    assert!(r.review_required);
    assert!(r.output_failed);
    let e = r.output_evidence.unwrap();
    assert!(e.recovered);
    assert_eq!(e.stdout.observed_bytes, None);
    assert_eq!(e.stdout.truncated, None);
    assert_eq!(
        c.run_state(&id).unwrap().unwrap().result,
        Some(RunResult::Interruption)
    );
    assert!(matches!(
        c.start_next().unwrap(),
        Start::Blocked(Block::ReviewRequired)
    ));
}

fn complete_for_retention(f: &Fixture, c: &mut Controller<Fake>, id: &str) {
    assert!(matches!(c.start_next().unwrap(), Start::Preparing(run) if run.as_str() == id));
    until(|| {
        c.tick(Instant::now()).unwrap();
        f.fake.0.lock().unwrap().writers.contains_key(id)
    });
    f.ended(id);
    completed(c, id);
}

#[test]
fn reclaims_only_the_oldest_logs_needed_for_dispatch() {
    let mut f = Fixture::new();
    let (_, oldest) = f.admit();
    let (_, newer) = f.admit();
    let (_, queued) = f.admit();
    let mut c = f.controller(Limits {
        log_bytes_per_run: 40,
        retained_log_bytes: 80,
        ..Limits::default()
    });
    c.set_paused(false).unwrap();
    complete_for_retention(&f, &mut c, &oldest);
    complete_for_retention(&f, &mut c, &newer);
    complete_for_retention(&f, &mut c, &queued);
    assert!(!f.preparation.logs.join(&oldest).exists());
    assert!(f.preparation.logs.join(&newer).join("stdout.log").is_file());
    assert_eq!(
        c.run_state(&oldest).unwrap().unwrap().result,
        Some(RunResult::Success)
    );
    let record = c.record(&oldest).unwrap().unwrap();
    assert_eq!(record.log_bytes, 0);
    assert_eq!(record.output_evidence.unwrap().retained_bytes(), Some(31));
}

#[test]
fn keeps_review_output_when_dispatch_is_blocked() {
    let mut f = Fixture::new();
    let (_, protected) = f.admit();
    let (_, queued) = f.admit();
    let limits = Limits {
        log_bytes_per_run: 40,
        retained_log_bytes: 40,
        ..Limits::default()
    };
    {
        let mut c = f.controller(limits);
        c.set_paused(false).unwrap();
        c.start_next().unwrap();
        started(&mut c, &f);
    }
    f.ended(&protected);
    f.safety = Safety::default();
    let mut c = f.controller(limits);
    completed(&mut c, &protected);
    assert!(matches!(
        c.start_next().unwrap(),
        Start::Blocked(Block::ReviewRequired)
    ));
    assert!(
        f.preparation
            .logs
            .join(&protected)
            .join("stdout.log")
            .is_file()
    );
    assert_eq!(c.record(&protected).unwrap().unwrap().log_bytes, 31);
    assert_eq!(
        c.run_state(&queued).unwrap().unwrap().phase,
        RunPhase::Queued
    );
}

#[test]
fn deletion_failure_keeps_the_charge_and_restart_retries_partial_removal() {
    let mut f = Fixture::new();
    let (_, old) = f.admit();
    let (_, queued) = f.admit();
    let limits = Limits {
        log_bytes_per_run: 40,
        retained_log_bytes: 40,
        ..Limits::default()
    };
    let directory = f.preparation.logs.join(&old);
    let stderr = directory.join("stderr.log");
    {
        let mut c = f.controller(limits);
        c.set_paused(false).unwrap();
        complete_for_retention(&f, &mut c, &old);
        fs::rename(&stderr, directory.join("saved")).unwrap();
        std::os::unix::fs::symlink("saved", &stderr).unwrap();
        assert!(matches!(
            c.start_next(),
            Err(crate::controller::Error::State(admission::Error::Storage))
        ));
        assert!(c.failed());
        assert!(!directory.join("stdout.log").exists());
        assert!(directory.join("saved").is_file());
    }
    fs::remove_file(&stderr).unwrap();
    fs::rename(directory.join("saved"), &stderr).unwrap();
    f.safety = Safety::default();
    let mut c = f.controller(limits);
    assert_eq!(c.record(&old).unwrap().unwrap().log_bytes, 31);
    assert_eq!(
        c.run_state(&queued).unwrap().unwrap().phase,
        RunPhase::Queued
    );
    complete_for_retention(&f, &mut c, &queued);
    assert!(!directory.exists());
    assert_eq!(c.record(&old).unwrap().unwrap().log_bytes, 0);
}

#[test]
fn rolled_back_claim_retries_after_durable_file_removal() {
    let mut f = Fixture::new();
    let (_, old) = f.admit();
    let (_, queued) = f.admit();
    let limits = Limits {
        log_bytes_per_run: 40,
        retained_log_bytes: 40,
        ..Limits::default()
    };
    let db = rusqlite::Connection::open(&f.path).unwrap();
    {
        let mut c = f.controller(limits);
        c.set_paused(false).unwrap();
        complete_for_retention(&f, &mut c, &old);
        db.execute_batch(
            "CREATE TRIGGER reject_claim BEFORE INSERT ON launches
             BEGIN SELECT RAISE(ABORT, 'synthetic claim failure'); END;",
        )
        .unwrap();
        assert!(matches!(
            c.start_next(),
            Err(crate::controller::Error::State(admission::Error::Storage))
        ));
        assert!(!f.preparation.logs.join(&old).exists());
    }
    db.execute_batch("DROP TRIGGER reject_claim").unwrap();
    f.safety = Safety::default();
    let mut c = f.controller(limits);
    assert_eq!(c.record(&old).unwrap().unwrap().log_bytes, 31);
    assert_eq!(
        c.run_state(&queued).unwrap().unwrap().phase,
        RunPhase::Queued
    );
    complete_for_retention(&f, &mut c, &queued);
    assert_eq!(c.record(&old).unwrap().unwrap().log_bytes, 0);
}

#[test]
fn insufficient_reclaimable_output_leaves_existing_logs_intact() {
    let mut f = Fixture::new();
    let (_, complete) = f.admit();
    let (_, active) = f.admit();
    let (_, queued) = f.admit();
    let mut c = f.controller(Limits {
        active_runs: 2,
        log_bytes_per_run: 40,
        retained_log_bytes: 75,
    });
    c.set_paused(false).unwrap();
    complete_for_retention(&f, &mut c, &complete);
    assert!(matches!(c.start_next().unwrap(), Start::Preparing(id) if id.as_str() == active));
    assert!(matches!(
        c.start_next().unwrap(),
        Start::Blocked(Block::LogCapacity)
    ));
    assert_eq!(
        c.run_state(&queued).unwrap().unwrap().phase,
        RunPhase::Queued
    );
    assert!(
        f.preparation
            .logs
            .join(&complete)
            .join("stdout.log")
            .is_file()
    );
    until(|| {
        c.tick(Instant::now()).unwrap();
        f.fake.0.lock().unwrap().writers.contains_key(&active)
    });
    f.ended(&active);
    completed(&mut c, &active);
}
