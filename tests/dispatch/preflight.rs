use super::*;
use rustward::{admission::RunState, dispatch::LaunchFailure};

#[test]
fn definite_prelaunch_failures_never_start_and_keep_capacity_until_output_finishes() {
    for (failure, code) in [
        (LaunchFailure::RuntimeMissing, "runtime_missing"),
        (LaunchFailure::RuntimeInvalid, "runtime_invalid"),
        (LaunchFailure::RuntimeChanged, "runtime_changed"),
        (LaunchFailure::RuntimeUnsafe, "runtime_unsafe"),
        (LaunchFailure::RuntimeCommand, "runtime_command"),
        (LaunchFailure::WorkDirectory, "work_directory"),
        (LaunchFailure::WorkStorage, "work_storage"),
    ] {
        let mut fixture = Fixture::new();
        let (key, id) = fixture.admit();
        let mut session = fixture.session(Limits::default());
        let permit = claim(&mut session);
        session.fail_before_start(permit, failure).unwrap();
        let record = session.record(&id).unwrap().unwrap();
        assert_eq!(record.launch_failure.as_deref(), Some(code));
        assert!(record.cleanup_confirmed);
        assert!(!record.review_required);
        assert_eq!(
            session.run_state(&id).unwrap().unwrap(),
            RunState {
                phase: RunPhase::CleaningUp,
                result: Some(RunResult::Failure)
            }
        );
        let mut fake = Fake::new(&fixture.path);
        session.poll(&mut fake, std::time::Instant::now()).unwrap();
        assert_eq!(fake.calls, 0);
        if failure == LaunchFailure::WorkStorage {
            assert!(matches!(session.claim_next(), Err(Error::Closed)));
        } else {
            blocked(&mut session, Block::ExecutionCapacity);
        }
        session.finish(&id, 0).unwrap();
        assert_eq!(
            session.run_state(&id).unwrap().unwrap().phase,
            RunPhase::Completed
        );
        let config = config::load("examples/rustward.toml").unwrap();
        assert_eq!(
            fixture.store.admit(
                &key,
                &config,
                "synthetic-report",
                &BTreeMap::from([("name".into(), "sample".into())]),
                "runtime-new"
            ),
            Ok(Admission::AlreadyAdmitted { run_id: id })
        );
    }
}

#[test]
fn a_failure_write_cannot_partially_change_a_run_or_permit_another_start() {
    for trigger in [
        "CREATE TRIGGER fail_write BEFORE UPDATE ON launches BEGIN SELECT RAISE(FAIL,'synthetic fault'); END;",
        "CREATE TRIGGER fail_write BEFORE UPDATE ON launches BEGIN SELECT RAISE(IGNORE); END;",
    ] {
        let mut fixture = Fixture::new();
        let (_, id) = fixture.admit();
        let mut session = fixture.session(Limits::default());
        let permit = claim(&mut session);
        fixture.db().execute_batch(trigger).unwrap();
        assert_eq!(
            session.fail_before_start(permit, LaunchFailure::RuntimeMissing),
            Err(Error::Storage)
        );
        assert!(matches!(session.claim_next(), Err(Error::Closed)));
        let state = fixture.store.run_state(&id).unwrap().unwrap();
        assert_eq!(state.phase, RunPhase::Launching);
        assert_eq!(state.result, None);
        let mut fake = Fake::new(&fixture.path);
        session.poll(&mut fake, std::time::Instant::now()).unwrap();
        assert_eq!(fake.calls, 0);
        fixture
            .db()
            .execute_batch("DROP TRIGGER fail_write")
            .unwrap();
        drop(session);
        let mut recovered = fixture.session(Limits::default());
        assert_eq!(
            recovered.run_state(&id).unwrap().unwrap().result,
            Some(RunResult::Interruption)
        );
        blocked(&mut recovered, Block::ReviewRequired);
    }
}

#[test]
fn failure_cannot_replace_an_earlier_cancellation_or_cross_dispatch_owners() {
    let mut fixture = Fixture::new();
    let (_, id) = fixture.admit();
    let mut session = fixture.session(Limits::default());
    let permit = claim(&mut session);
    session.cancel_active(&id).unwrap();
    assert_eq!(
        session.fail_before_start(permit, LaunchFailure::RuntimeMissing),
        Err(Error::InvalidTransition)
    );
    assert_eq!(
        session.run_state(&id).unwrap().unwrap().result,
        Some(RunResult::Cancellation)
    );
    let mut other = Fixture::new();
    other.admit();
    let mut other_session = other.session(Limits::default());
    let other_permit = claim(&mut other_session);
    assert_eq!(
        session.fail_before_start(other_permit, LaunchFailure::RuntimeMissing),
        Err(Error::InvalidTransition)
    );
}
