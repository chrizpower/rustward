use super::*;
#[path = "../runtime/common.rs"]
mod prepared;
use prepared::RuntimeFixture;
use rustward::{dispatch::PrepareError, runtime};
use std::fs;

struct RuntimeFake {
    path: PathBuf,
    calls: Vec<(String, String, Vec<String>)>,
    outcome: StartOutcome,
}
impl RuntimeFake {
    fn new(path: &std::path::Path) -> Self {
        Self {
            path: path.to_owned(),
            calls: vec![],
            outcome: StartOutcome::Accepted,
        }
    }
}
impl Supervisor for RuntimeFake {
    fn start(&mut self, launch: &Launch, _: Output<'_>) -> Result<StartOutcome, SupervisorError> {
        Connection::open(&self.path)
            .unwrap()
            .execute_batch("BEGIN IMMEDIATE;ROLLBACK")
            .unwrap();
        self.calls.push((
            launch.executable.clone(),
            launch.working_directory.clone(),
            launch.arguments.clone(),
        ));
        Ok(self.outcome)
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
fn task(prepared: &RuntimeFixture, id: &str) -> rustward::config::Config {
    let path = prepared.catalog_path.join(id);
    common::validate(&format!("label='Prepared task'\nexecutable='{}'\nworking_directory='{}'\narguments=[{{literal=\"space and 'quotes'\"}},{{input='count'}}]\ninputs=[{{id='count',type='integer',default=2}}]\n",path.join("bin/task").display(),path.display())).unwrap()
}
fn admit(fixture: &mut Fixture, prepared: &RuntimeFixture, id: &str) -> (String, String) {
    let key = fixture.store.issue_key().unwrap();
    let Admission::Admitted { run_id } = fixture
        .store
        .admit(
            &key,
            &task(prepared, id),
            "synthetic-report",
            &BTreeMap::new(),
            id,
        )
        .unwrap()
    else {
        panic!()
    };
    (key, run_id)
}
fn prepared_start(
    session: &mut Session,
    permit: rustward::dispatch::PreparedPermit,
    fake: &mut RuntimeFake,
) -> Result<StartOutcome, PrepareError> {
    let (out, err) = UnixStream::pair().unwrap();
    session.start_prepared(
        permit,
        fake,
        Output {
            stdout: out.as_fd(),
            stderr: err.as_fd(),
        },
    )
}

#[test]
fn prepared_launch_preserves_admitted_runtime_paths_arguments_and_writable_data() {
    let prepared = RuntimeFixture::new();
    let (next, _) = prepared.add(b"fixed code B");
    let mut fixture = Fixture::new();
    let (key, first) = admit(&mut fixture, &prepared, &prepared.id);
    let (_, second) = admit(&mut fixture, &prepared, &next);
    assert_eq!(
        fixture.store.admit(
            &key,
            &task(&prepared, &next),
            "synthetic-report",
            &BTreeMap::new(),
            &next
        ),
        Ok(Admission::AlreadyAdmitted {
            run_id: first.clone()
        })
    );
    let mut session = fixture.session(Limits {
        active_runs: 2,
        ..Limits::default()
    });
    let catalog = prepared.catalog();
    let work = prepared.work();
    let permit = claim(&mut session);
    let first = session.prepare_runtime(permit, &catalog, &work).unwrap();
    assert_eq!(first.runtime_id(), prepared.id);
    assert_eq!(first.execution_uid(), prepared.policy.execution_uid);
    assert_eq!(
        first.launch().executable,
        prepared.path().join("bin/task").to_str().unwrap()
    );
    assert_eq!(
        first.launch().working_directory,
        prepared.path().to_str().unwrap()
    );
    assert_eq!(first.work_owner_uid(), work.owner_uid());
    let first_work = first.work_directory().to_owned();
    fs::write(first_work.join("evidence"), b"keep").unwrap();
    let mut fake = RuntimeFake::new(&fixture.path);
    prepared_start(&mut session, first, &mut fake).unwrap();
    prepared.add(b"fixed code C published while A is active");
    prepared.catalog().verify(&prepared.id).unwrap();
    let permit = claim(&mut session);
    assert_eq!(permit.launch().run_id.as_str(), second);
    let second = session.prepare_runtime(permit, &catalog, &work).unwrap();
    assert_ne!(second.work_directory(), first_work);
    prepared_start(&mut session, second, &mut fake).unwrap();
    assert_eq!(fake.calls.len(), 2);
    assert_eq!(fake.calls[0].2, ["space and 'quotes'", "2"]);
    assert_ne!(fake.calls[0].0, fake.calls[1].0);
    assert_eq!(fs::read(first_work.join("evidence")).unwrap(), b"keep");
}

#[test]
fn missing_runtime_fails_without_substitution_or_rewriting_admission() {
    let prepared = RuntimeFixture::new();
    let (next, _) = prepared.add(b"fixed B");
    let mut fixture = Fixture::new();
    let (key, id) = admit(&mut fixture, &prepared, &prepared.id);
    admit(&mut fixture, &prepared, &next);
    fs::remove_dir_all(prepared.path()).unwrap();
    let mut session = fixture.session(Limits::default());
    let permit = claim(&mut session);
    assert!(matches!(
        session.prepare_runtime(permit, &prepared.catalog(), &prepared.work()),
        Err(PrepareError::Runtime(runtime::Error::Missing))
    ));
    assert_eq!(
        session
            .record(&id)
            .unwrap()
            .unwrap()
            .launch_failure
            .as_deref(),
        Some("runtime_missing")
    );
    assert_eq!(
        session.run_state(&id).unwrap().unwrap().result,
        Some(RunResult::Failure)
    );
    assert_eq!(
        fixture.store.admit(
            &key,
            &task(&prepared, &next),
            "synthetic-report",
            &BTreeMap::new(),
            &next
        ),
        Ok(Admission::AlreadyAdmitted { run_id: id.clone() })
    );
    session.finish(&id, 0).unwrap();
    let permit = claim(&mut session);
    let next = session
        .prepare_runtime(permit, &prepared.catalog(), &prepared.work())
        .unwrap();
    let mut fake = RuntimeFake::new(&fixture.path);
    prepared_start(&mut session, next, &mut fake).unwrap();
    assert_eq!(fake.calls.len(), 1);
}

#[test]
fn changed_runtime_after_preparation_never_reaches_the_supervisor() {
    let prepared = RuntimeFixture::new();
    let mut fixture = Fixture::new();
    let (_, id) = admit(&mut fixture, &prepared, &prepared.id);
    let mut session = fixture.session(Limits::default());
    let permit = claim(&mut session);
    let permit = session
        .prepare_runtime(permit, &prepared.catalog(), &prepared.work())
        .unwrap();
    let work = permit.work_directory().to_owned();
    fs::write(prepared.path().join("data/value"), b"changed").unwrap();
    let mut fake = RuntimeFake::new(&fixture.path);
    assert_eq!(
        prepared_start(&mut session, permit, &mut fake),
        Err(PrepareError::Runtime(runtime::Error::Changed))
    );
    assert!(fake.calls.is_empty());
    assert!(work.exists());
    assert_eq!(
        session
            .record(&id)
            .unwrap()
            .unwrap()
            .launch_failure
            .as_deref(),
        Some("runtime_changed")
    );
}

#[test]
fn invalid_runtime_identity_and_paths_are_rejected_with_retained_failures() {
    let prepared = RuntimeFixture::new();
    let mut fixture = Fixture::new();
    let (_, id) = fixture.admit();
    let mut session = fixture.session(Limits::default());
    let permit = claim(&mut session);
    assert!(matches!(
        session.prepare_runtime(permit, &prepared.catalog(), &prepared.work()),
        Err(PrepareError::Runtime(runtime::Error::Invalid))
    ));
    assert_eq!(
        session
            .record(&id)
            .unwrap()
            .unwrap()
            .launch_failure
            .as_deref(),
        Some("runtime_invalid")
    );
    assert_eq!(
        fixture
            .db()
            .query_row("SELECT runtime_id FROM runs WHERE run_id=?1", [&id], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
        "runtime-1"
    );
    session.finish(&id, 0).unwrap();
    let key = fixture.store.issue_key().unwrap();
    let Admission::Admitted { run_id } = fixture
        .store
        .admit(
            &key,
            &config::load("examples/rustward.toml").unwrap(),
            "synthetic-report",
            &BTreeMap::from([("name".into(), "sample".into())]),
            &prepared.id,
        )
        .unwrap()
    else {
        panic!()
    };
    let permit = claim(&mut session);
    assert!(matches!(
        session.prepare_runtime(permit, &prepared.catalog(), &prepared.work()),
        Err(PrepareError::Runtime(runtime::Error::Command))
    ));
    assert_eq!(
        session
            .record(&run_id)
            .unwrap()
            .unwrap()
            .launch_failure
            .as_deref(),
        Some("runtime_command")
    );
}

#[test]
fn work_directory_reuse_and_replacement_are_not_allowed() {
    for replace in [false, true] {
        let prepared = RuntimeFixture::new();
        let mut fixture = Fixture::new();
        let (_, id) = admit(&mut fixture, &prepared, &prepared.id);
        let mut session = fixture.session(Limits::default());
        let permit = claim(&mut session);
        let work = prepared.work();
        if replace {
            let permit = session
                .prepare_runtime(permit, &prepared.catalog(), &work)
                .unwrap();
            let path = permit.work_directory().to_owned();
            fs::rename(&path, prepared.work_path.join("old")).unwrap();
            fs::create_dir(&path).unwrap();
            prepared::mode(&path, 0o700);
            let mut fake = RuntimeFake::new(&fixture.path);
            assert_eq!(
                prepared_start(&mut session, permit, &mut fake),
                Err(PrepareError::Runtime(runtime::Error::WorkDirectory))
            );
            assert!(fake.calls.is_empty());
        } else {
            let dir = work.create(&RunId::new(&id).unwrap()).unwrap();
            fs::write(dir.path().join("retain"), b"original").unwrap();
            assert!(matches!(
                session.prepare_runtime(permit, &prepared.catalog(), &work),
                Err(PrepareError::Runtime(runtime::Error::WorkDirectory))
            ));
            assert_eq!(fs::read(dir.path().join("retain")).unwrap(), b"original");
        }
        assert_eq!(
            session
                .record(&id)
                .unwrap()
                .unwrap()
                .launch_failure
                .as_deref(),
            Some("work_directory")
        );
    }
}

#[test]
fn cancellation_and_failure_write_errors_prevent_verification_side_effects() {
    for cancel in [false, true] {
        let prepared = RuntimeFixture::new();
        let mut fixture = Fixture::new();
        let (_, id) = admit(&mut fixture, &prepared, &prepared.id);
        let mut session = fixture.session(Limits::default());
        let permit = claim(&mut session);
        if cancel {
            session.cancel_active(&id).unwrap();
            assert!(matches!(
                session.prepare_runtime(permit, &prepared.catalog(), &prepared.work()),
                Err(PrepareError::State(Error::InvalidTransition))
            ));
        } else {
            fs::write(prepared.path().join("data/value"), b"changed").unwrap();
            fixture.db().execute_batch("CREATE TRIGGER fail_write BEFORE UPDATE ON runs BEGIN SELECT RAISE(FAIL,'synthetic fault'); END;").unwrap();
            assert!(matches!(
                session.prepare_runtime(permit, &prepared.catalog(), &prepared.work()),
                Err(PrepareError::State(Error::Storage))
            ));
            assert!(matches!(session.claim_next(), Err(Error::Closed)));
        }
        assert!(!prepared.work_path.join(id).exists());
    }
}

#[test]
fn prepared_permits_preserve_owner_review_and_storage_gates() {
    for gate in ["owner", "review", "storage"] {
        let prepared = RuntimeFixture::new();
        let mut fixture = Fixture::new();
        admit(&mut fixture, &prepared, &prepared.id);
        if gate == "review" {
            admit(&mut fixture, &prepared, &prepared.id);
        }
        let mut session = fixture.session(Limits {
            active_runs: 2,
            ..Limits::default()
        });
        let permit = claim(&mut session);
        let first = session
            .prepare_runtime(permit, &prepared.catalog(), &prepared.work())
            .unwrap();
        let mut fake = RuntimeFake::new(&fixture.path);
        match gate {
            "owner" => {
                let other = Fixture::new();
                let mut other_session = other.session(Limits::default());
                assert_eq!(
                    prepared_start(&mut other_session, first, &mut fake),
                    Err(PrepareError::State(Error::InvalidTransition))
                );
                assert!(fake.calls.is_empty());
            }
            "review" => {
                let permit = claim(&mut session);
                let second = session
                    .prepare_runtime(permit, &prepared.catalog(), &prepared.work())
                    .unwrap();
                fake.outcome = StartOutcome::Uncertain;
                prepared_start(&mut session, second, &mut fake).unwrap();
                assert_eq!(
                    prepared_start(&mut session, first, &mut fake),
                    Err(PrepareError::State(Error::Closed))
                );
                assert_eq!(fake.calls.len(), 1);
            }
            _ => {
                session.storage_failure();
                assert_eq!(
                    prepared_start(&mut session, first, &mut fake),
                    Err(PrepareError::State(Error::Closed))
                );
                assert!(fake.calls.is_empty());
            }
        }
    }
}

#[test]
fn work_directory_must_be_outside_the_runtime_catalog() {
    let prepared = RuntimeFixture::new();
    let mut fixture = Fixture::new();
    let (_, id) = admit(&mut fixture, &prepared, &prepared.id);
    let path = prepared.catalog_path.join("work");
    fs::create_dir(&path).unwrap();
    prepared::mode(&path, 0o700);
    let work = runtime::WorkRoot::open(&path).unwrap();
    let mut session = fixture.session(Limits::default());
    let permit = claim(&mut session);
    assert!(matches!(
        session.prepare_runtime(permit, &prepared.catalog(), &work),
        Err(PrepareError::Runtime(runtime::Error::WorkDirectory))
    ));
    assert!(!path.join(&id).exists());
}
