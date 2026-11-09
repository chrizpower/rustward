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
