use super::*;

#[test]
fn intent_and_accepted_launches_recover_without_new_permits() {
    for accepted in [false, true] {
        let mut fixture = Fixture::new();
        let (key, run) = fixture.admit();
        let (_, queued) = fixture.admit();
        let mut session = fixture.session(Limits::default());
        let permit = claim(&mut session);
        let mut fake = Fake::new(&fixture.path);
        if accepted {
            start(&mut session, permit, &mut fake).unwrap();
        } else {
            drop(permit);
        }
        drop(session);
        let mut session = fixture.session(Limits::default());
        assert_eq!(
            session.run_state(&run).unwrap().unwrap().result,
            Some(RunResult::Interruption)
        );
        assert_eq!(
            session.run_state(&queued).unwrap().unwrap().phase,
            RunPhase::Queued
        );
        blocked(&mut session, Block::ReviewRequired);
        assert_eq!(
            session.record(&run).unwrap().unwrap().log_bytes,
            10 * 1024 * 1024
        );
        session.observe(&run, &Observation::Missing).unwrap();
        assert!(!session.record(&run).unwrap().unwrap().cleanup_confirmed);
        assert_eq!(session.finish(&run, 0), Err(Error::InvalidTransition));
        session
            .observe(
                &run,
                &known(
                    ExecutionState::Ended,
                    Some(ProcessExit::Code(0)),
                    Cleanup::Confirmed,
                ),
            )
            .unwrap();
        assert_eq!(
            session.run_state(&run).unwrap().unwrap().result,
            Some(RunResult::Interruption)
        );
        session.finish(&run, 0).unwrap();
        assert_eq!(fake.calls, usize::from(accepted));
        drop(session);
        let mut session = fixture.session(Limits::default());
        blocked(&mut session, Block::ReviewRequired);
        let config = config::load("examples/rustward.toml").unwrap();
        assert_eq!(
            fixture.store.admit(
                &key,
                &config,
                "synthetic-report",
                &BTreeMap::from([("name".into(), "sample".into())]),
                "changed-runtime"
            ),
            Ok(Admission::AlreadyAdmitted { run_id: run })
        );
    }
}

#[test]
fn uncertain_replies_and_errors_block_without_claiming_success() {
    for reply in [
        Ok(StartOutcome::Conflict),
        Ok(StartOutcome::Uncertain),
        Err(SupervisorError::Unavailable),
    ] {
        let mut fixture = Fixture::new();
        let (_, run) = fixture.admit();
        let mut session = fixture.session(Limits::default());
        let permit = claim(&mut session);
        let mut fake = Fake::new(&fixture.path);
        fake.reply = reply;
        start(&mut session, permit, &mut fake).unwrap();
        assert_eq!(fake.calls, 1);
        let record = session.record(&run).unwrap().unwrap();
        assert_eq!(
            record.start_state,
            if reply == Ok(StartOutcome::Conflict) {
                "conflict"
            } else {
                "uncertain"
            }
        );
        assert!(record.review_required);
        assert!(!record.cleanup_confirmed);
        assert_eq!(
            session.run_state(&run).unwrap().unwrap().result,
            Some(RunResult::Interruption)
        );
        blocked(&mut session, Block::ReviewRequired);
    }
}

#[test]
fn retains_identity_and_first_exit_result_across_uncertainty_and_reopen() {
    let mut fixture = Fixture::new();
    let (_, run) = fixture.admit();
    let mut session = fixture.session(Limits::default());
    let permit = claim(&mut session);
    start(&mut session, permit, &mut Fake::new(&fixture.path)).unwrap();
    session
        .observe(
            &run,
            &known(ExecutionState::Running, None, Cleanup::Pending),
        )
        .unwrap();
    session
        .observe(
            &run,
            &known(
                ExecutionState::Exited,
                Some(ProcessExit::Signal(9)),
                Cleanup::Pending,
            ),
        )
        .unwrap();
    let changed = Observation::Known {
        identity: ExecutionIdentity {
            invocation: [2; 16],
            control_group: "/other".into(),
        },
        state: ExecutionState::Ended,
        exit: Some(ProcessExit::Code(0)),
        cleanup: Cleanup::Confirmed,
    };
    session.observe(&run, &changed).unwrap();
    let record = session.record(&run).unwrap().unwrap();
    assert_eq!(record.identity, Some(identity()));
    assert_eq!(record.exit, Some(ProcessExit::Signal(9)));
    assert!(!record.cleanup_confirmed);
    assert!(record.review_required);
    assert_eq!(
        session.run_state(&run).unwrap().unwrap().result,
        Some(RunResult::Failure)
    );
    drop(session);
    let mut session = fixture.session(Limits::default());
    assert_eq!(
        session.run_state(&run).unwrap().unwrap().result,
        Some(RunResult::Failure)
    );
    session
        .observe(
            &run,
            &known(
                ExecutionState::Ended,
                Some(ProcessExit::Code(0)),
                Cleanup::Confirmed,
            ),
        )
        .unwrap();
    session.finish(&run, 0).unwrap();
    blocked(&mut session, Block::ReviewRequired);
}

#[test]
fn missing_exit_and_inconsistent_evidence_require_review() {
    for observation in [
        Observation::Uncertain,
        known(ExecutionState::Ended, None, Cleanup::Confirmed),
        known(ExecutionState::Running, None, Cleanup::Confirmed),
        known(
            ExecutionState::Running,
            Some(ProcessExit::Code(0)),
            Cleanup::Pending,
        ),
        Observation::Known {
            identity: ExecutionIdentity {
                invocation: [1; 16],
                control_group: "relative".into(),
            },
            state: ExecutionState::Running,
            exit: None,
            cleanup: Cleanup::Pending,
        },
    ] {
        let mut fixture = Fixture::new();
        let (_, run) = fixture.admit();
        let mut session = fixture.session(Limits::default());
        let permit = claim(&mut session);
        start(&mut session, permit, &mut Fake::new(&fixture.path)).unwrap();
        session.observe(&run, &observation).unwrap();
        assert_eq!(
            session.run_state(&run).unwrap().unwrap().result,
            Some(RunResult::Interruption)
        );
        blocked(&mut session, Block::ReviewRequired);
    }
}

#[test]
fn intent_and_reply_write_failures_preserve_recovery_evidence() {
    let mut fixture = Fixture::new();
    let (_, run) = fixture.admit();
    let mut session = fixture.session(Limits::default());
    fixture.db().execute_batch("CREATE TRIGGER fail_intent BEFORE INSERT ON launches BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(session.claim_next(), Err(Error::Storage)));
    assert_eq!(
        fixture.store.run_state(&run).unwrap().unwrap().phase,
        RunPhase::Queued
    );
    assert_eq!(
        fixture
            .db()
            .query_row("SELECT count(*) FROM launches", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        0
    );
    fixture
        .db()
        .execute_batch("DROP TRIGGER fail_intent")
        .unwrap();
    drop(session);
    let mut session = fixture.session(Limits::default());
    let permit = claim(&mut session);
    let mut fake = Fake::new(&fixture.path);
    fake.fail_reply_write = true;
    assert_eq!(start(&mut session, permit, &mut fake), Err(Error::Storage));
    assert_eq!(fake.calls, 1);
    assert!(matches!(session.claim_next(), Err(Error::Closed)));
    assert_eq!(
        fixture
            .db()
            .query_row("SELECT start_state FROM launches", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "intent"
    );
    fixture
        .db()
        .execute_batch("DROP TRIGGER fail_reply")
        .unwrap();
    drop(session);
    let mut session = fixture.session(Limits::default());
    blocked(&mut session, Block::ReviewRequired);
    assert_eq!(
        session.run_state(&run).unwrap().unwrap().result,
        Some(RunResult::Interruption)
    );
}

#[test]
fn retained_snapshots_are_used_without_current_definitions() {
    let mut fixture = Fixture::new();
    let (key, run) = fixture.admit();
    let original:(Vec<u8>,Vec<u8>,Vec<u8>)=fixture.db().query_row("SELECT task_snapshot,config_revision,fingerprint FROM runs JOIN admissions USING(run_id)",[],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    let mut session = fixture.session(Limits::default());
    let permit = claim(&mut session);
    assert_eq!(permit.launch().run_id.as_str(), run);
    assert_eq!(permit.runtime_id(), "runtime-1");
    let mut fake = Fake::new(&fixture.path);
    start(&mut session, permit, &mut fake).unwrap();
    let retained:(Vec<u8>,Vec<u8>,Vec<u8>)=fixture.db().query_row("SELECT task_snapshot,config_revision,fingerprint FROM runs JOIN admissions USING(run_id)",[],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(retained, original);
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("tasks")).unwrap();
    std::fs::create_dir(root.path().join("schedules")).unwrap();
    std::fs::write(
        root.path().join("rustward.toml"),
        include_str!("../../examples/rustward.toml"),
    )
    .unwrap();
    let empty = config::load(root.path().join("rustward.toml")).unwrap();
    assert_eq!(
        fixture.store.admit(
            &key,
            &empty,
            "synthetic-report",
            &BTreeMap::from([("name".into(), "sample".into())]),
            ""
        ),
        Ok(Admission::AlreadyAdmitted { run_id: run })
    );
}

#[test]
fn dispatch_crash_child() {
    let Ok(mode) = std::env::var("RUSTWARD_DISPATCH_CRASH") else {
        return;
    };
    let path = PathBuf::from(std::env::var("RUSTWARD_DISPATCH_DB").unwrap());
    let mut session = Session::open(&path, Limits::default()).unwrap();
    session.set_paused(false).unwrap();
    if mode == "before" {
        std::process::exit(0);
    }
    let permit = claim(&mut session);
    if mode == "intent" {
        std::process::exit(0);
    }
    start(&mut session, permit, &mut Fake::new(&path)).unwrap();
    std::process::exit(0);
}

#[test]
fn process_exit_at_launch_boundaries_never_repeats_a_start() {
    for mode in ["before", "intent", "reply", "recorded"] {
        let mut fixture = Fixture::new();
        let (_, run) = fixture.admit();
        fixture
            .db()
            .execute_batch("CREATE TABLE synthetic_starts (run_id TEXT)")
            .unwrap();
        assert!(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "recovery::dispatch_crash_child", "--nocapture"])
                .env("RUSTWARD_DISPATCH_CRASH", mode)
                .env("RUSTWARD_DISPATCH_DB", &fixture.path)
                .status()
                .unwrap()
                .success()
        );
        let mut session = fixture.session(Limits::default());
        if mode == "before" {
            assert_eq!(
                session.run_state(&run).unwrap().unwrap().phase,
                RunPhase::Queued
            );
            assert_eq!(claim(&mut session).launch().run_id.as_str(), run);
        } else {
            blocked(&mut session, Block::ReviewRequired);
            assert_eq!(
                session.run_state(&run).unwrap().unwrap().result,
                Some(RunResult::Interruption)
            );
        }
        let starts: u32 = fixture
            .db()
            .query_row("SELECT count(*) FROM synthetic_starts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(starts, u32::from(mode == "reply" || mode == "recorded"));
    }
}

#[test]
fn evidence_and_completion_failures_roll_back_and_close_the_handle() {
    for ignored in [false, true] {
        for operation in ["observe", "finish", "recover"] {
            let mut fixture = Fixture::new();
            let (_, run) = fixture.admit();
            let mut session = fixture.session(Limits::default());
            let permit = claim(&mut session);
            start(&mut session, permit, &mut Fake::new(&fixture.path)).unwrap();
            if operation == "finish" {
                session
                    .observe(
                        &run,
                        &known(
                            ExecutionState::Ended,
                            Some(ProcessExit::Code(0)),
                            Cleanup::Confirmed,
                        ),
                    )
                    .unwrap();
            }
            let previous = session.run_state(&run).unwrap();
            let previous_record = session.record(&run).unwrap();
            let action = if ignored {
                "IGNORE"
            } else {
                "ABORT,'synthetic'"
            };
            fixture.db().execute_batch(&format!("CREATE TRIGGER fail_write BEFORE UPDATE ON launches BEGIN SELECT RAISE({action}); END;")).unwrap();
            if operation == "recover" {
                drop(session);
                assert!(matches!(
                    Session::open(&fixture.path, Limits::default()),
                    Err(Error::Storage)
                ));
            } else {
                let result = if operation == "finish" {
                    session.finish(&run, 0)
                } else {
                    session.observe(
                        &run,
                        &known(ExecutionState::Running, None, Cleanup::Pending),
                    )
                };
                assert_eq!(result, Err(Error::Storage));
                assert!(matches!(session.claim_next(), Err(Error::Closed)));
                drop(session);
            }
            assert_eq!(fixture.store.run_state(&run).unwrap(), previous);
            let identity: Option<Vec<u8>> = fixture
                .db()
                .query_row("SELECT invocation FROM launches", [], |r| r.get(0))
                .unwrap();
            assert_eq!(
                identity,
                previous_record
                    .unwrap()
                    .identity
                    .map(|i| i.invocation.to_vec())
            );
            fixture
                .db()
                .execute_batch("DROP TRIGGER fail_write")
                .unwrap();
            let mut session = fixture.session(Limits::default());
            blocked(&mut session, Block::ReviewRequired);
        }
    }
}

#[test]
fn review_block_stops_an_outstanding_permit_from_starting() {
    let mut fixture = Fixture::new();
    fixture.admit();
    fixture.admit();
    let mut session = fixture.session(Limits {
        active_runs: 2,
        ..Limits::default()
    });
    let first = claim(&mut session);
    let second = claim(&mut session);
    let mut fake = Fake::new(&fixture.path);
    fake.reply = Ok(StartOutcome::Uncertain);
    start(&mut session, first, &mut fake).unwrap();
    assert_eq!(start(&mut session, second, &mut fake), Err(Error::Closed));
    assert_eq!(fake.calls, 1);
}

#[test]
fn state_read_rejects_completion_without_cleanup_evidence() {
    let mut fixture = Fixture::new();
    let (_, run) = fixture.admit();
    let mut session = fixture.session(Limits::default());
    let permit = claim(&mut session);
    start(&mut session, permit, &mut Fake::new(&fixture.path)).unwrap();
    fixture
        .db()
        .execute("UPDATE runs SET phase='completed',result='failure'", [])
        .unwrap();
    assert_eq!(fixture.store.run_state(&run), Err(Error::Storage));
}

#[test]
fn busy_reply_write_closes_dispatch_until_explicit_recovery() {
    let mut fixture = Fixture::new();
    let (_, run) = fixture.admit();
    fixture.admit();
    let mut session = fixture.session(Limits {
        active_runs: 2,
        ..Limits::default()
    });
    let permit = claim(&mut session);
    let mut fake = Fake::new(&fixture.path);
    fake.busy_after_start = true;
    assert_eq!(start(&mut session, permit, &mut fake), Err(Error::Busy));
    assert_eq!(fake.calls, 1);
    assert!(matches!(session.claim_next(), Err(Error::Closed)));
    drop(fake);
    drop(session);
    let mut session = fixture.session(Limits::default());
    blocked(&mut session, Block::ReviewRequired);
    assert_eq!(
        session.run_state(&run).unwrap().unwrap().result,
        Some(RunResult::Interruption)
    );
}
