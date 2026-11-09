mod common;

use std::{
    collections::BTreeMap,
    os::{fd::AsFd, unix::net::UnixStream},
    path::PathBuf,
    sync::{Arc, Barrier},
    thread,
};

use rusqlite::Connection;
use rustward::{
    admission::{Admission, Cancellation, Error, RunPhase, RunResult, Store},
    config,
    dispatch::{Block, Claim, Limits, Permit, Session},
    supervision::{
        Cleanup, Error as SupervisorError, ExecutionIdentity, ExecutionState, Launch, Observation,
        Output, ProcessExit, RunId, StartOutcome, StopMode, StopOutcome, Supervisor,
    },
};
use tempfile::TempDir;

#[path = "dispatch/migration.rs"]
mod migration;
#[path = "dispatch/recovery.rs"]
mod recovery;
#[path = "dispatch/review.rs"]
mod review;
#[path = "dispatch/review_failure.rs"]
mod review_failure;
#[path = "dispatch/stopping.rs"]
mod stopping;

#[test]
fn shared_failure_gate_blocks_an_already_granted_start() {
    let mut fixture = Fixture::new();
    fixture.admit();
    let safety = rustward::safety::Safety::default();
    let mut session =
        Session::open_with_safety(&fixture.path, Limits::default(), safety.clone()).unwrap();
    session.set_paused(false).unwrap();
    let permit = claim(&mut session);
    let mut fake = Fake::new(&fixture.path);
    safety.fail();
    assert_eq!(start(&mut session, permit, &mut fake), Err(Error::Closed));
    assert_eq!(fake.calls, 0);
}

struct Fixture {
    _root: TempDir,
    path: PathBuf,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.db");
        let store = Store::open(&path).unwrap();
        Self {
            _root: root,
            path,
            store,
        }
    }
    fn admit(&mut self) -> (String, String) {
        let key = self.store.issue_key().unwrap();
        let config = config::load("examples/rustward.toml").unwrap();
        let Admission::Admitted { run_id } = self
            .store
            .admit(
                &key,
                &config,
                "synthetic-report",
                &BTreeMap::from([("name".into(), "sample".into())]),
                "runtime-1",
            )
            .unwrap()
        else {
            panic!()
        };
        (key, run_id)
    }
    fn session(&self, limits: Limits) -> Session {
        let mut session = Session::open(&self.path, limits).unwrap();
        session.set_paused(false).unwrap();
        session
    }
    fn db(&self) -> Connection {
        Connection::open(&self.path).unwrap()
    }
}
fn claim(session: &mut Session) -> Permit {
    match session.claim_next().unwrap() {
        Claim::Ready(permit) => permit,
        Claim::Blocked(block) => panic!("blocked: {block:?}"),
    }
}
fn blocked(session: &mut Session, expected: Block) {
    match session.claim_next().unwrap() {
        Claim::Blocked(block) => assert_eq!(block, expected),
        Claim::Ready(_) => panic!("unexpected permit"),
    }
}
fn identity() -> ExecutionIdentity {
    ExecutionIdentity {
        invocation: [1; 16],
        control_group: "/synthetic/run".into(),
    }
}
fn known(state: ExecutionState, exit: Option<ProcessExit>, cleanup: Cleanup) -> Observation {
    Observation::Known {
        identity: identity(),
        state,
        exit,
        cleanup,
    }
}
fn start(session: &mut Session, permit: Permit, fake: &mut Fake) -> Result<StartOutcome, Error> {
    let (out, err) = UnixStream::pair().unwrap();
    session.start(
        permit,
        fake,
        Output {
            stdout: out.as_fd(),
            stderr: err.as_fd(),
        },
    )
}

struct Fake {
    path: PathBuf,
    calls: usize,
    reply: Result<StartOutcome, SupervisorError>,
    fail_reply_write: bool,
    busy_after_start: bool,
    held_writer: Option<Connection>,
}
impl Fake {
    fn new(path: &std::path::Path) -> Self {
        Self {
            path: path.into(),
            calls: 0,
            reply: Ok(StartOutcome::Accepted),
            fail_reply_write: false,
            busy_after_start: false,
            held_writer: None,
        }
    }
}
impl Supervisor for Fake {
    fn start(&mut self, launch: &Launch, _: Output<'_>) -> Result<StartOutcome, SupervisorError> {
        let connection = Connection::open(&self.path).unwrap();
        // A second write transaction must be possible during the external call.
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        let state: (String, String) = connection
            .query_row(
                "SELECT phase,start_state FROM runs JOIN launches USING(run_id) WHERE run_id=?1",
                [launch.run_id.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, ("launching".into(), "intent".into()));
        connection.execute_batch("COMMIT").unwrap();
        assert_eq!(launch.executable, "/synthetic/runtime/bin/report");
        assert_eq!(launch.arguments, ["--name", "sample", "2", "false"]);
        self.calls += 1;
        connection
            .execute_batch("CREATE TABLE IF NOT EXISTS synthetic_starts (run_id TEXT)")
            .unwrap();
        connection
            .execute(
                "INSERT INTO synthetic_starts VALUES (?1)",
                [launch.run_id.as_str()],
            )
            .unwrap();
        if std::env::var("RUSTWARD_DISPATCH_CRASH").as_deref() == Ok("reply") {
            std::process::exit(0);
        }
        if self.fail_reply_write {
            connection.execute_batch("CREATE TRIGGER fail_reply BEFORE UPDATE ON launches BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
        }
        if self.busy_after_start {
            connection.execute_batch("BEGIN IMMEDIATE").unwrap();
            self.held_writer = Some(connection);
        }
        self.reply
    }
    fn observe(
        &mut self,
        _: &RunId,
        _: Option<&ExecutionIdentity>,
    ) -> Result<Observation, SupervisorError> {
        unreachable!()
    }
    fn stop(
        &mut self,
        _: &RunId,
        _: &ExecutionIdentity,
        _: StopMode,
    ) -> Result<StopOutcome, SupervisorError> {
        unreachable!()
    }
    fn release(&mut self, _: &RunId, _: &ExecutionIdentity) -> Result<(), SupervisorError> {
        unreachable!()
    }
}

#[test]
fn fifo_capacity_pause_and_admission_controls_are_independent() {
    let mut fixture = Fixture::new();
    let (_, first) = fixture.admit();
    let (_, second) = fixture.admit();
    let mut session = Session::open(&fixture.path, Limits::default()).unwrap();
    blocked(&mut session, Block::Paused);
    fixture.store.set_closed(true).unwrap();
    session.set_paused(false).unwrap();
    let permit = claim(&mut session);
    assert_eq!(permit.launch().run_id.as_str(), first);
    assert_eq!(permit.runtime_id(), "runtime-1");
    assert_eq!(permit.launch().working_directory, "/synthetic/runtime");
    assert_eq!(
        session.run_state(&first).unwrap().unwrap().phase,
        RunPhase::Launching
    );
    assert_eq!(
        fixture.store.cancel_queued(&first),
        Ok(Cancellation::NotQueued)
    );
    blocked(&mut session, Block::ExecutionCapacity);
    let mut fake = Fake::new(&fixture.path);
    assert_eq!(
        start(&mut session, permit, &mut fake),
        Ok(StartOutcome::Accepted)
    );
    assert_eq!(
        session.record(&first).unwrap().unwrap().start_state,
        "accepted"
    );
    assert_eq!(
        session.run_state(&first).unwrap().unwrap().phase,
        RunPhase::Launching
    );
    session
        .observe(
            &first,
            &known(ExecutionState::Running, None, Cleanup::Pending),
        )
        .unwrap();
    assert_eq!(
        session.run_state(&first).unwrap().unwrap().phase,
        RunPhase::Running
    );
    assert_eq!(session.finish(&first, 0), Err(Error::InvalidTransition));
    session.set_paused(true).unwrap();
    session
        .observe(
            &first,
            &known(
                ExecutionState::Exited,
                Some(ProcessExit::Code(0)),
                Cleanup::Pending,
            ),
        )
        .unwrap();
    assert_eq!(session.finish(&first, 0), Err(Error::InvalidTransition));
    session
        .observe(
            &first,
            &known(
                ExecutionState::Ended,
                Some(ProcessExit::Code(0)),
                Cleanup::Confirmed,
            ),
        )
        .unwrap();
    assert_eq!(
        session.run_state(&first).unwrap().unwrap().phase,
        RunPhase::CleaningUp
    );
    assert_eq!(
        session.finish(&first, 11 * 1024 * 1024),
        Err(Error::InvalidTransition)
    );
    session.finish(&first, 12).unwrap();
    let state = session.run_state(&first).unwrap().unwrap();
    assert_eq!(state.phase, RunPhase::Completed);
    assert_eq!(state.result, Some(RunResult::Success));
    assert_eq!(
        fixture.store.cancel_queued(&first),
        Ok(Cancellation::NotQueued)
    );
    assert_eq!(session.record(&first).unwrap().unwrap().log_bytes, 12);
    blocked(&mut session, Block::Paused);
    session.set_paused(false).unwrap();
    assert_eq!(claim(&mut session).launch().run_id.as_str(), second);
    assert_eq!(fake.calls, 1);
    assert_eq!(session.finish(&first, 12), Err(Error::InvalidTransition));
    assert_eq!(
        session.observe(&first, &Observation::Missing),
        Err(Error::InvalidTransition)
    );
    assert_eq!(session.finish("missing", 0), Err(Error::InvalidTransition));
    assert_eq!(
        session.observe("missing", &Observation::Missing),
        Err(Error::InvalidTransition)
    );
    assert_eq!(session.record("missing"), Ok(None));
}

#[test]
fn log_accounting_preserves_retained_bytes_and_blocks_new_claims() {
    let mut fixture = Fixture::new();
    let (_, run) = fixture.admit();
    fixture.admit();
    let mut session = fixture.session(Limits {
        active_runs: 2,
        log_bytes_per_run: 10,
        retained_log_bytes: 15,
    });
    let permit = claim(&mut session);
    blocked(&mut session, Block::LogCapacity);
    let mut fake = Fake::new(&fixture.path);
    fake.reply = Ok(StartOutcome::Rejected);
    assert_eq!(
        start(&mut session, permit, &mut fake),
        Ok(StartOutcome::Rejected)
    );
    let record = session.record(&run).unwrap().unwrap();
    assert!(record.cleanup_confirmed);
    assert!(!record.review_required);
    assert_eq!(
        session.run_state(&run).unwrap().unwrap().result,
        Some(RunResult::Failure)
    );
    session.finish(&run, 6).unwrap();
    blocked(&mut session, Block::LogCapacity);
    drop(session);
    let mut session = fixture.session(Limits {
        active_runs: 1,
        log_bytes_per_run: 10,
        retained_log_bytes: 16,
    });
    let permit = claim(&mut session);
    assert_eq!(
        start(&mut session, permit, &mut fake),
        Ok(StartOutcome::Rejected)
    );
    let second: String = fixture
        .db()
        .query_row(
            "SELECT run_id FROM runs ORDER BY sequence DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    session.finish(&second, 0).unwrap();
    blocked(&mut session, Block::Empty);
}

#[test]
fn lock_covers_aliases_competing_sessions_and_outstanding_permits() {
    let mut fixture = Fixture::new();
    fixture.admit();
    let mut session = fixture.session(Limits::default());
    let alias = fixture.path.with_file_name("alias.db");
    std::fs::hard_link(&fixture.path, &alias).unwrap();
    assert!(matches!(
        Session::open(&alias, Limits::default()),
        Err(Error::OwnerBusy)
    ));
    let permit = claim(&mut session);
    drop(session);
    assert!(matches!(
        Session::open(&fixture.path, Limits::default()),
        Err(Error::OwnerBusy)
    ));
    drop(permit);
    let mut next = fixture.session(Limits::default());
    blocked(&mut next, Block::ReviewRequired);
}

#[test]
fn competing_dispatchers_grant_one_permit() {
    let mut fixture = Fixture::new();
    fixture.admit();
    let mut session = fixture.session(Limits::default());
    session.set_paused(false).unwrap();
    drop(session);
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let path = fixture.path.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let session = Session::open(path, Limits::default());
                barrier.wait();
                match session {
                    Ok(mut session) => {
                        let permit = claim(&mut session);
                        barrier.wait();
                        drop(permit);
                        true
                    }
                    Err(Error::OwnerBusy) => {
                        barrier.wait();
                        false
                    }
                    _ => panic!("unexpected ownership result"),
                }
            })
        })
        .collect();
    assert_eq!(
        handles
            .into_iter()
            .map(|h| h.join().unwrap() as usize)
            .sum::<usize>(),
        1
    );
}

#[test]
fn cancellation_and_dispatch_have_one_durable_winner() {
    for _ in 0..8 {
        let mut fixture = Fixture::new();
        let (_, run) = fixture.admit();
        let mut session = fixture.session(Limits::default());
        let barrier = Arc::new(Barrier::new(2));
        let mut store = Store::open(&fixture.path).unwrap();
        let other = Arc::clone(&barrier);
        let run_copy = run.clone();
        let cancel = thread::spawn(move || {
            other.wait();
            store.cancel_queued(&run_copy).unwrap()
        });
        barrier.wait();
        let claimed = session.claim_next().unwrap();
        match (cancel.join().unwrap(), claimed) {
            (Cancellation::Cancelled, Claim::Blocked(Block::Empty)) => assert_eq!(
                session.run_state(&run).unwrap().unwrap().phase,
                RunPhase::Completed
            ),
            (Cancellation::NotQueued, Claim::Ready(permit)) => {
                assert_eq!(permit.launch().run_id.as_str(), run)
            }
            _ => panic!("invalid race outcome"),
        }
    }
}

#[test]
fn constraints_limits_and_missing_data_fail_without_claiming() {
    let mut fixture = Fixture::new();
    fixture.admit();
    for limits in [
        Limits {
            log_bytes_per_run: u64::MAX,
            ..Limits::default()
        },
        Limits {
            retained_log_bytes: u64::MAX,
            ..Limits::default()
        },
    ] {
        assert!(matches!(
            Session::open(&fixture.path, limits),
            Err(Error::InvalidLimits)
        ));
    }
    let mut session = fixture.session(Limits {
        active_runs: 0,
        ..Limits::default()
    });
    blocked(&mut session, Block::ExecutionCapacity);
    drop(session);
    fixture
        .db()
        .execute("UPDATE runs SET task_snapshot=x'7b7d'", [])
        .unwrap();
    let mut session = fixture.session(Limits::default());
    assert!(matches!(session.claim_next(), Err(Error::Storage)));
    assert!(matches!(session.claim_next(), Err(Error::Closed)));
    assert_eq!(
        fixture
            .db()
            .query_row("SELECT phase FROM runs", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "queued"
    );
}
