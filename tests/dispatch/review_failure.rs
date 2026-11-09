use super::review::completed_review;
use super::*;
use rustward::review::new_acknowledgement_id;

#[test]
fn failed_or_ignored_writes_never_partially_acknowledge() {
    for trigger in [
        "CREATE TRIGGER fail_write BEFORE INSERT ON acknowledgements BEGIN SELECT RAISE(FAIL,'synthetic fault'); END;",
        "CREATE TRIGGER fail_write BEFORE INSERT ON acknowledgements BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER fail_write BEFORE UPDATE OF review_required ON launches WHEN NEW.review_required=0 BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER fail_write AFTER INSERT ON acknowledgements BEGIN UPDATE launches SET review_required=1; END;",
    ] {
        let mut fixture = Fixture::new();
        let (_session, id, review) = completed_review(&mut fixture);
        fixture.db().execute_batch(trigger).unwrap();
        let request = new_acknowledgement_id().unwrap();
        assert_eq!(
            fixture.store.acknowledge(&request, &review, "Checked"),
            Err(Error::Storage)
        );
        assert_eq!(fixture.store.review(&id), Err(Error::Closed));
        let db = fixture.db();
        assert_eq!(
            db.query_row("SELECT count(*) FROM acknowledgements", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(
            db.query_row(
                "SELECT review_required FROM launches WHERE run_id=?1",
                [&id],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
        );
        db.execute_batch("DROP TRIGGER fail_write").unwrap();
        let mut reopened = Store::open(&fixture.path).unwrap();
        assert_eq!(reopened.review(&id).unwrap(), Some(review.clone()));
        reopened
            .acknowledge(&request, &review, "Checked after failure review")
            .unwrap();
    }
}

#[test]
fn busy_acknowledgement_can_retry_without_an_external_side_effect() {
    let mut fixture = Fixture::new();
    let (_session, id, review) = completed_review(&mut fixture);
    let lock = fixture.db();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let request = new_acknowledgement_id().unwrap();
    assert_eq!(
        fixture.store.acknowledge(&request, &review, "Checked"),
        Err(Error::Busy)
    );
    lock.execute_batch("ROLLBACK").unwrap();
    assert_eq!(fixture.store.review(&id).unwrap(), Some(review.clone()));
    fixture
        .store
        .acknowledge(&request, &review, "Checked")
        .unwrap();
}

#[test]
fn invalid_retained_record_closes_the_handle_instead_of_claiming_a_retry() {
    let mut fixture = Fixture::new();
    let (_session, _id, review) = completed_review(&mut fixture);
    let request = new_acknowledgement_id().unwrap();
    fixture
        .store
        .acknowledge(&request, &review, "Checked")
        .unwrap();
    fixture
        .db()
        .execute(
            "UPDATE acknowledgements SET record='invalid json' WHERE acknowledgement_id=?1",
            [&request],
        )
        .unwrap();
    assert_eq!(fixture.store.acknowledgement(&request), Err(Error::Storage));
    assert_eq!(
        fixture.store.acknowledge(&request, &review, "Checked"),
        Err(Error::Closed)
    );
}

#[test]
fn release_preserves_other_capacity_limits_and_storage_failure() {
    let mut fixture = Fixture::new();
    let (session, _id, review) = completed_review(&mut fixture);
    let safety = rustward::safety::Safety::default();
    let mut reviewer = Store::open_with_safety(&fixture.path, safety.clone()).unwrap();
    safety.fail();
    reviewer
        .acknowledge(&new_acknowledgement_id().unwrap(), &review, "Checked")
        .unwrap();
    assert!(safety.failed());
    drop(session);
    let mut limited = fixture.session(Limits {
        active_runs: 0,
        ..Limits::default()
    });
    blocked(&mut limited, Block::ExecutionCapacity);
    drop(limited);
    let mut limited = fixture.session(Limits {
        retained_log_bytes: 0,
        ..Limits::default()
    });
    blocked(&mut limited, Block::LogCapacity);
}

#[test]
fn recovery_changes_evidence_without_clearing_review() {
    let mut fixture = Fixture::new();
    let (_, id) = fixture.admit();
    let mut session = fixture.session(Limits::default());
    let permit = claim(&mut session);
    let mut fake = Fake::new(&fixture.path);
    fake.reply = Ok(StartOutcome::Uncertain);
    start(&mut session, permit, &mut fake).unwrap();
    let before = fixture.store.review(&id).unwrap().unwrap();
    drop(session);
    let mut recovered = fixture.session(Limits::default());
    let after = fixture.store.review(&id).unwrap().unwrap();
    assert!(after.evidence_version > before.evidence_version);
    assert_eq!(
        fixture
            .store
            .acknowledge(&new_acknowledgement_id().unwrap(), &before, "Checked"),
        Err(Error::ReviewChanged)
    );
    recovered
        .observe(&id, &known(ExecutionState::Ended, None, Cleanup::Confirmed))
        .unwrap();
    recovered.finish(&id, 0).unwrap();
    let completed = fixture.store.review(&id).unwrap().unwrap();
    fixture
        .store
        .acknowledge(
            &new_acknowledgement_id().unwrap(),
            &completed,
            "Reviewed recovered work",
        )
        .unwrap();
    drop(recovered);
    let mut reopened = fixture.session(Limits::default());
    assert_eq!(fixture.store.review(&id).unwrap(), None);
    blocked(&mut reopened, Block::Empty);
}

#[test]
fn lost_acknowledgement_reply_child() {
    let Some(path) = std::env::var_os("RUSTWARD_REVIEW_CRASH") else {
        return;
    };
    let mut store = Store::open(path).unwrap();
    let reviewed = store.pending_reviews(None, 1).unwrap().pop().unwrap();
    store
        .acknowledge(&"a".repeat(64), &reviewed, "Checked before reply loss")
        .unwrap();
    std::process::exit(0);
}

#[test]
fn lost_reply_after_commit_returns_the_original_record_after_restart() {
    let mut fixture = Fixture::new();
    let (session, id, review) = completed_review(&mut fixture);
    drop(session);
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "review_failure::lost_acknowledgement_reply_child",
        ])
        .env("RUSTWARD_REVIEW_CRASH", &fixture.path)
        .output()
        .unwrap();
    assert!(child.status.success(), "{child:?}");
    let request = "a".repeat(64);
    let original = fixture.store.acknowledgement(&request).unwrap().unwrap();
    assert_eq!(fixture.store.review(&id).unwrap(), None);
    assert_eq!(
        fixture
            .store
            .acknowledge(&request, &review, "Checked before reply loss")
            .unwrap(),
        original
    );
}

#[test]
fn evidence_version_overflow_closes_dispatch_instead_of_reusing_an_old_version() {
    let mut fixture = Fixture::new();
    let (_, id) = fixture.admit();
    let mut session = fixture.session(Limits::default());
    let permit = claim(&mut session);
    let mut fake = Fake::new(&fixture.path);
    fake.reply = Ok(StartOutcome::Uncertain);
    start(&mut session, permit, &mut fake).unwrap();
    fixture
        .db()
        .execute(
            "UPDATE launches SET evidence_version=?1 WHERE run_id=?2",
            rusqlite::params![i64::MAX, id],
        )
        .unwrap();
    let last = fixture.store.review(&id).unwrap().unwrap();
    assert_eq!(
        session.observe(&id, &known(ExecutionState::Ended, None, Cleanup::Confirmed)),
        Err(Error::Storage)
    );
    assert!(matches!(session.claim_next(), Err(Error::Closed)));
    assert_eq!(
        fixture
            .db()
            .query_row(
                "SELECT evidence_version FROM launches WHERE run_id=?1",
                [&id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        i64::MAX
    );
    assert_eq!(
        fixture.store.acknowledge(
            &new_acknowledgement_id().unwrap(),
            &last,
            "Checked last representable evidence"
        ),
        Err(Error::NotCompleted)
    );
}
