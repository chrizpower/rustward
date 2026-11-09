use super::*;
use rustward::{
    capture::{Failure, FailureKind, Stream},
    dispatch::StopDecision,
    safety::Safety,
};
use std::time::{Duration, Instant};

struct StopFake {
    launch: Fake,
    observation: Observation,
    stops: Vec<(String, ExecutionIdentity, StopMode)>,
    stop_reply: Result<StopOutcome, SupervisorError>,
}
impl StopFake {
    fn new(path: &std::path::Path) -> Self {
        Self {
            launch: Fake::new(path),
            observation: known(ExecutionState::Running, None, Cleanup::Pending),
            stops: vec![],
            stop_reply: Ok(StopOutcome::Accepted),
        }
    }
}
impl Supervisor for StopFake {
    fn start(
        &mut self,
        launch: &Launch,
        output: Output<'_>,
    ) -> Result<StartOutcome, SupervisorError> {
        self.launch.start(launch, output)
    }
    fn observe(
        &mut self,
        _: &RunId,
        expected: Option<&ExecutionIdentity>,
    ) -> Result<Observation, SupervisorError> {
        if let Some(expected) = expected {
            assert_eq!(*expected, identity());
        }
        Ok(self.observation.clone())
    }
    fn stop(
        &mut self,
        run: &RunId,
        expected: &ExecutionIdentity,
        mode: StopMode,
    ) -> Result<StopOutcome, SupervisorError> {
        let db = Connection::open(&self.launch.path).unwrap();
        db.execute_batch("BEGIN IMMEDIATE; ROLLBACK;").unwrap();
        self.stops
            .push((run.as_str().into(), expected.clone(), mode));
        if std::env::var_os("RUSTWARD_STOP_CRASH").is_some() {
            db.execute_batch("CREATE TABLE IF NOT EXISTS synthetic_stops(run_id TEXT)")
                .unwrap();
            db.execute("INSERT INTO synthetic_stops VALUES(?1)", [run.as_str()])
                .unwrap();
            std::process::exit(0);
        }
        self.stop_reply
    }
    fn release(&mut self, _: &RunId, _: &ExecutionIdentity) -> Result<(), SupervisorError> {
        unreachable!()
    }
}
fn launch(session: &mut Session, permit: Permit, fake: &mut StopFake) {
    let (out, err) = UnixStream::pair().unwrap();
    assert_eq!(
        session.start(
            permit,
            fake,
            Output {
                stdout: out.as_fd(),
                stderr: err.as_fd()
            }
        ),
        Ok(StartOutcome::Accepted)
    );
}
fn output_fault(kind: FailureKind) -> Failure {
    Failure {
        stream: Stream::Stdout,
        kind,
    }
}

#[test]
fn cancellation_before_start_revokes_permit_and_needs_no_process_cleanup() {
    let mut fixture = Fixture::new();
    let (_, id) = fixture.admit();
    let mut session = fixture.session(Limits::default());
    let permit = claim(&mut session);
    assert_eq!(session.cancel_active(&id), Ok(StopDecision::Recorded));
    assert_eq!(session.cancel_active(&id), Ok(StopDecision::AlreadyEnded));
    assert_eq!(
        session.cancel_active("missing"),
        Ok(StopDecision::NotActive)
    );
    let mut fake = Fake::new(&fixture.path);
    assert_eq!(
        start(&mut session, permit, &mut fake),
        Err(Error::InvalidTransition)
    );
    assert_eq!(fake.calls, 0);
    session.finish(&id, 0).unwrap();
    assert_eq!(
        session.run_state(&id).unwrap().unwrap().result,
        Some(RunResult::Cancellation)
    );
    let times = session.times(&id).unwrap().unwrap();
    assert!(times.dispatched_at_ms.is_some());
    assert!(times.completed_at_ms.is_some());
    assert_eq!(session.cancel_active(&id), Ok(StopDecision::AlreadyEnded));
}

#[test]
fn cancellation_stops_then_forces_without_releasing_the_slot() {
    let mut fixture = Fixture::new();
    let (_, id) = fixture.admit();
    fixture.admit();
    let mut session = fixture.session(Limits::default());
    let permit = claim(&mut session);
    let mut fake = StopFake::new(&fixture.path);
    launch(&mut session, permit, &mut fake);
    session.cancel_active(&id).unwrap();
    let now = Instant::now();
    session.poll(&mut fake, now).unwrap();
    assert_eq!(fake.stops[0], (id.clone(), identity(), StopMode::Graceful));
    session.poll(&mut fake, now).unwrap();
    assert_eq!(fake.stops.len(), 1);
    session
        .poll(&mut fake, now + Duration::from_secs(31))
        .unwrap();
    assert_eq!(fake.stops[1].2, StopMode::Force);
    assert_eq!(session.finish(&id, 0), Err(Error::InvalidTransition));
    session
        .observe(
            &id,
            &known(
                ExecutionState::Ended,
                Some(ProcessExit::Code(0)),
                Cleanup::Confirmed,
            ),
        )
        .unwrap();
    session.finish(&id, 0).unwrap();
    assert_eq!(
        session.run_state(&id).unwrap().unwrap().result,
        Some(RunResult::Cancellation)
    );
    blocked(&mut session, Block::ReviewRequired);
}

#[test]
fn timeout_starts_at_dispatch_and_does_not_change_an_earlier_exit() {
    let mut fixture = Fixture::new();
    let (_, id) = fixture.admit();
    let mut session = fixture.session(Limits::default());
    let before = Instant::now();
    let permit = claim(&mut session);
    let mut fake = StopFake::new(&fixture.path);
    launch(&mut session, permit, &mut fake);
    session
        .poll(&mut fake, before + Duration::from_secs(3599))
        .unwrap();
    assert!(fake.stops.is_empty());
    session
        .poll(&mut fake, before + Duration::from_secs(3601))
        .unwrap();
    assert_eq!(
        session.run_state(&id).unwrap().unwrap().result,
        Some(RunResult::Timeout)
    );
    session
        .observe(
            &id,
            &known(
                ExecutionState::Ended,
                Some(ProcessExit::Code(0)),
                Cleanup::Confirmed,
            ),
        )
        .unwrap();
    assert_eq!(
        session.run_state(&id).unwrap().unwrap().result,
        Some(RunResult::Timeout)
    );
    assert_eq!(session.cancel_active(&id), Ok(StopDecision::AlreadyEnded));
    assert_eq!(
        session.record(&id).unwrap().unwrap().stop_reason.as_deref(),
        Some("timeout")
    );
    session.finish(&id, 0).unwrap();
    drop(session);
    let mut recovered = fixture.session(Limits::default());
    assert_eq!(
        recovered.run_state(&id).unwrap().unwrap().result,
        Some(RunResult::Timeout)
    );
}

#[test]
fn normal_exit_wins_cancellation_but_output_failure_prevents_success() {
    let mut fixture = Fixture::new();
    let (_, id) = fixture.admit();
    let mut session = fixture.session(Limits::default());
    let permit = claim(&mut session);
    let mut fake = StopFake::new(&fixture.path);
    launch(&mut session, permit, &mut fake);
    session
        .observe(
            &id,
            &known(
                ExecutionState::Exited,
                Some(ProcessExit::Code(0)),
                Cleanup::Pending,
            ),
        )
        .unwrap();
    assert_eq!(session.cancel_active(&id), Ok(StopDecision::AlreadyEnded));
    assert_eq!(
        session.output_failure(&id, output_fault(FailureKind::Read)),
        Ok(StopDecision::Recorded)
    );
    assert_eq!(
        session.run_state(&id).unwrap().unwrap().result,
        Some(RunResult::Interruption)
    );
    session.poll(&mut fake, Instant::now()).unwrap();
    assert_eq!(fake.stops.len(), 1);
    session
        .observe(
            &id,
            &known(
                ExecutionState::Ended,
                Some(ProcessExit::Code(0)),
                Cleanup::Confirmed,
            ),
        )
        .unwrap();
    assert_eq!(session.finish(&id, 0), Err(Error::InvalidTransition));
    session.finish_failed_output(&id, 0).unwrap();
    let record = session.record(&id).unwrap().unwrap();
    assert!(record.output_failed && record.review_required);
}

#[test]
fn capture_failure_preserves_cancellation_and_failure_results() {
    for exit in [None, Some(ProcessExit::Code(1))] {
        let mut fixture = Fixture::new();
        let (_, id) = fixture.admit();
        let mut session = fixture.session(Limits::default());
        let permit = claim(&mut session);
        let mut fake = StopFake::new(&fixture.path);
        launch(&mut session, permit, &mut fake);
        let expected = if let Some(exit) = exit {
            session
                .observe(
                    &id,
                    &known(ExecutionState::Exited, Some(exit), Cleanup::Pending),
                )
                .unwrap();
            RunResult::Failure
        } else {
            session.cancel_active(&id).unwrap();
            RunResult::Cancellation
        };
        session
            .output_failure(&id, output_fault(FailureKind::Worker))
            .unwrap();
        assert_eq!(
            session.run_state(&id).unwrap().unwrap().result,
            Some(expected)
        );
    }
}

#[test]
fn storage_failure_stops_all_active_runs_even_when_writes_fail() {
    let mut fixture = Fixture::new();
    let (_, first) = fixture.admit();
    let (_, second) = fixture.admit();
    let safety = Safety::default();
    let mut session = Session::open_with_safety(
        &fixture.path,
        Limits {
            active_runs: 2,
            ..Limits::default()
        },
        safety.clone(),
    )
    .unwrap();
    session.set_paused(false).unwrap();
    let mut fake = StopFake::new(&fixture.path);
    for _ in 0..2 {
        let permit = claim(&mut session);
        launch(&mut session, permit, &mut fake);
    }
    fixture.db().execute_batch("CREATE TRIGGER fail_write BEFORE UPDATE ON runs BEGIN SELECT RAISE(FAIL,'synthetic storage failure'); END;").unwrap();
    assert_eq!(
        session.output_failure(&first, output_fault(FailureKind::Write)),
        Err(Error::Storage)
    );
    assert!(safety.failed());
    let now = Instant::now() + Duration::from_secs(7200);
    session.poll(&mut fake, now).unwrap();
    assert_eq!(fake.stops.len(), 2);
    assert!(
        fake.stops
            .iter()
            .all(|(_, _, mode)| *mode == StopMode::Graceful)
    );
    assert!(fake.stops.iter().any(|(id, _, _)| id == &second));
    session
        .poll(&mut fake, now + Duration::from_secs(31))
        .unwrap();
    assert_eq!(fake.stops.len(), 4);
    assert!(matches!(session.claim_next(), Err(Error::Closed)));
    assert_eq!(session.finish(&first, 0), Err(Error::Closed));
}

#[test]
fn recovery_keeps_identity_and_never_restarts_stopped_work() {
    let mut fixture = Fixture::new();
    let (_, id) = fixture.admit();
    let mut session = fixture.session(Limits::default());
    let permit = claim(&mut session);
    let mut fake = StopFake::new(&fixture.path);
    launch(&mut session, permit, &mut fake);
    session
        .observe(&id, &known(ExecutionState::Running, None, Cleanup::Pending))
        .unwrap();
    session.cancel_active(&id).unwrap();
    drop(session);
    let mut recovered = fixture.session(Limits::default());
    fake.observation = Observation::Missing;
    recovered.poll(&mut fake, Instant::now()).unwrap();
    assert_eq!(fake.launch.calls, 1);
    assert_eq!(fake.stops[0].1, identity());
    assert_eq!(
        recovered.run_state(&id).unwrap().unwrap().result,
        Some(RunResult::Cancellation)
    );
    assert_eq!(recovered.finish(&id, 0), Err(Error::InvalidTransition));
}

#[test]
fn missing_or_rejected_stop_never_proves_cleanup() {
    for reply in [
        Ok(StopOutcome::Missing),
        Ok(StopOutcome::Uncertain),
        Err(SupervisorError::Unavailable),
    ] {
        let mut fixture = Fixture::new();
        let (_, id) = fixture.admit();
        let mut session = fixture.session(Limits::default());
        let permit = claim(&mut session);
        let mut fake = StopFake::new(&fixture.path);
        fake.stop_reply = reply;
        launch(&mut session, permit, &mut fake);
        session.cancel_active(&id).unwrap();
        session.poll(&mut fake, Instant::now()).unwrap();
        assert_eq!(session.finish(&id, 0), Err(Error::InvalidTransition));
        assert!(session.record(&id).unwrap().unwrap().review_required);
    }
}

#[test]
fn admitted_timing_survives_task_edits_and_matching_retries() {
    let mut fixture = Fixture::new();
    let original = include_str!("../../examples/tasks/synthetic-report.toml");
    let first = common::validate(&format!(
        "timeout_seconds = 2\ncleanup_grace_seconds = 1\n{original}"
    ))
    .unwrap();
    let changed = common::validate(&format!(
        "timeout_seconds = 500\ncleanup_grace_seconds = 100\n{original}"
    ))
    .unwrap();
    let key = fixture.store.issue_key().unwrap();
    let values = BTreeMap::from([("name".into(), "sample".into())]);
    let Admission::Admitted { run_id } = fixture
        .store
        .admit(&key, &first, "synthetic-report", &values, "runtime-1")
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        fixture
            .store
            .admit(&key, &changed, "synthetic-report", &values, "runtime-2"),
        Ok(Admission::AlreadyAdmitted {
            run_id: run_id.clone()
        })
    );
    let mut session = fixture.session(Limits::default());
    let before = Instant::now();
    let permit = claim(&mut session);
    assert_eq!(permit.launch().cleanup_grace, Duration::from_secs(1));
    let mut fake = StopFake::new(&fixture.path);
    launch(&mut session, permit, &mut fake);
    session
        .poll(&mut fake, before + Duration::from_secs(3))
        .unwrap();
    assert_eq!(
        session.run_state(&run_id).unwrap().unwrap().result,
        Some(RunResult::Timeout)
    );
    session
        .poll(&mut fake, before + Duration::from_secs(5))
        .unwrap();
    assert_eq!(fake.stops.last().unwrap().2, StopMode::Force);
}

#[test]
fn stop_exit_child() {
    let Some(path) = std::env::var_os("RUSTWARD_STOP_CRASH") else {
        return;
    };
    let path = PathBuf::from(path);
    let mut session = Session::open(&path, Limits::default()).unwrap();
    let mut fake = StopFake::new(&path);
    session.poll(&mut fake, Instant::now()).unwrap();
    panic!("expected process exit during the stop call");
}

#[test]
fn process_exit_during_stop_keeps_result_slot_and_admission() {
    let mut fixture = Fixture::new();
    let (key, id) = fixture.admit();
    let mut session = fixture.session(Limits::default());
    let permit = claim(&mut session);
    let mut fake = StopFake::new(&fixture.path);
    launch(&mut session, permit, &mut fake);
    session
        .observe(&id, &known(ExecutionState::Running, None, Cleanup::Pending))
        .unwrap();
    session.cancel_active(&id).unwrap();
    drop(session);
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "stopping::stop_exit_child"])
        .env("RUSTWARD_STOP_CRASH", &fixture.path)
        .output()
        .unwrap();
    assert!(child.status.success(), "{child:?}");
    let mut recovered = fixture.session(Limits::default());
    assert_eq!(
        recovered.run_state(&id).unwrap().unwrap().result,
        Some(RunResult::Cancellation)
    );
    assert_eq!(
        recovered.record(&id).unwrap().unwrap().identity,
        Some(identity())
    );
    assert_eq!(recovered.finish(&id, 0), Err(Error::InvalidTransition));
    assert_eq!(
        fixture
            .db()
            .query_row("SELECT count(*) FROM synthetic_stops", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        fixture
            .db()
            .query_row("SELECT count(*) FROM synthetic_starts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let config = config::load("examples/rustward.toml").unwrap();
    let values = BTreeMap::from([("name".into(), "sample".into())]);
    assert_eq!(
        fixture
            .store
            .admit(&key, &config, "synthetic-report", &values, "runtime-new"),
        Ok(Admission::AlreadyAdmitted { run_id: id })
    );
}

#[test]
fn expired_permit_cannot_start_when_polling_was_delayed() {
    let mut fixture = Fixture::new();
    let source = format!(
        "timeout_seconds = 1\n{}",
        include_str!("../../examples/tasks/synthetic-report.toml")
    );
    let config = common::validate(&source).unwrap();
    let key = fixture.store.issue_key().unwrap();
    let values = BTreeMap::from([("name".into(), "sample".into())]);
    let Admission::Admitted { run_id } = fixture
        .store
        .admit(&key, &config, "synthetic-report", &values, "runtime-1")
        .unwrap()
    else {
        panic!()
    };
    let mut session = fixture.session(Limits::default());
    let permit = claim(&mut session);
    thread::sleep(Duration::from_millis(1100));
    let mut fake = Fake::new(&fixture.path);
    assert_eq!(start(&mut session, permit, &mut fake), Err(Error::Closed));
    assert_eq!(fake.calls, 0);
    assert_eq!(
        session.run_state(&run_id).unwrap().unwrap().result,
        Some(RunResult::Timeout)
    );
    session.finish(&run_id, 0).unwrap();
}

#[test]
fn failed_observation_write_keeps_identity_for_emergency_stopping() {
    let mut fixture = Fixture::new();
    let (_, id) = fixture.admit();
    let mut session = fixture.session(Limits::default());
    let permit = claim(&mut session);
    let mut fake = StopFake::new(&fixture.path);
    launch(&mut session, permit, &mut fake);
    fixture.db().execute_batch("CREATE TRIGGER fail_write BEFORE UPDATE ON launches BEGIN SELECT RAISE(FAIL,'synthetic fault'); END;").unwrap();
    assert_eq!(
        session.observe(&id, &known(ExecutionState::Running, None, Cleanup::Pending)),
        Err(Error::Storage)
    );
    fake.observation = Observation::Missing;
    session.poll(&mut fake, Instant::now()).unwrap();
    assert_eq!(fake.stops[0].1, identity());
}

#[test]
fn legacy_queued_snapshot_keeps_no_timeout_and_thirty_second_grace() {
    let mut fixture = Fixture::new();
    let (_, id) = fixture.admit();
    let db = fixture.db();
    let bytes: Vec<u8> = db
        .query_row("SELECT task_snapshot FROM runs", [], |r| r.get(0))
        .unwrap();
    let mut legacy: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    legacy.as_object_mut().unwrap().remove("timeout_seconds");
    legacy
        .as_object_mut()
        .unwrap()
        .remove("cleanup_grace_seconds");
    db.execute(
        "UPDATE runs SET task_snapshot=?1",
        [serde_json::to_vec(&legacy).unwrap()],
    )
    .unwrap();
    let mut session = fixture.session(Limits::default());
    let permit = claim(&mut session);
    assert_eq!(permit.launch().cleanup_grace, Duration::from_secs(30));
    let mut fake = StopFake::new(&fixture.path);
    launch(&mut session, permit, &mut fake);
    session
        .poll(&mut fake, Instant::now() + Duration::from_secs(7200))
        .unwrap();
    assert!(fake.stops.is_empty());
    assert_eq!(session.run_state(&id).unwrap().unwrap().result, None);
}
