use super::*;
use rustward::review::{Acknowledgement, Review, new_acknowledgement_id};

pub(super) fn completed_review(fixture: &mut Fixture) -> (Session, String, Review) {
    let (_, id) = fixture.admit();
    let mut session = fixture.session(Limits::default());
    let permit = claim(&mut session);
    let mut fake = Fake::new(&fixture.path);
    fake.reply = Ok(StartOutcome::Uncertain);
    assert_eq!(
        start(&mut session, permit, &mut fake),
        Ok(StartOutcome::Uncertain)
    );
    session
        .observe(&id, &known(ExecutionState::Ended, None, Cleanup::Confirmed))
        .unwrap();
    session.finish(&id, 0).unwrap();
    let review = fixture.store.review(&id).unwrap().unwrap();
    (session, id, review)
}

#[test]
fn acknowledgement_retains_one_note_and_reviewed_evidence_without_changing_the_run() {
    let mut fixture = Fixture::new();
    let (mut session, id, review) = completed_review(&mut fixture);
    let before = session.run_state(&id).unwrap();
    let times = session.times(&id).unwrap();
    blocked(&mut session, Block::ReviewRequired);
    let request = new_acknowledgement_id().unwrap();
    let note = "I checked the remote status. Work ended; new work can start.";
    let acknowledgement = fixture.store.acknowledge(&request, &review, note).unwrap();
    assert_eq!(acknowledgement.review, review);
    assert_eq!(acknowledgement.note, note);
    assert!(acknowledgement.acknowledged_at_ms > 0);
    assert_eq!(
        fixture.store.acknowledgement(&request).unwrap(),
        Some(acknowledgement.clone())
    );
    assert_eq!(fixture.store.review(&id).unwrap(), None);
    assert_eq!(session.run_state(&id).unwrap(), before);
    assert_eq!(session.times(&id).unwrap(), times);
    let record = session.record(&id).unwrap().unwrap();
    assert!(!record.review_required);
    assert_eq!(record.identity, review.launch.identity);
    assert_eq!(record.exit, review.launch.exit);
    fixture.admit();
    claim(&mut session);
    drop(session);
    drop(fixture.store);
    let mut reopened = Store::open(&fixture.path).unwrap();
    assert_eq!(
        reopened.acknowledgement(&request).unwrap(),
        Some(acknowledgement.clone())
    );
    assert_eq!(
        reopened.acknowledge(&request, &review, note).unwrap(),
        acknowledgement
    );
}

#[test]
fn exact_retries_survive_removed_history_but_changed_payloads_cannot_reuse_the_id() {
    let mut fixture = Fixture::new();
    let (session, id, review) = completed_review(&mut fixture);
    let request = new_acknowledgement_id().unwrap();
    let note = "Reviewed external work; I accept the remaining overlap risk.";
    let original = fixture.store.acknowledge(&request, &review, note).unwrap();
    assert_eq!(
        fixture
            .store
            .acknowledge(&request, &review, "different note"),
        Err(Error::PayloadConflict)
    );
    let mut edited = review.clone();
    edited.evidence_version += 1;
    assert_eq!(
        fixture.store.acknowledge(&request, &edited, note),
        Err(Error::PayloadConflict)
    );
    fixture
        .db()
        .execute("DELETE FROM launches WHERE run_id=?1", [&id])
        .unwrap();
    fixture
        .db()
        .execute("DELETE FROM runs WHERE run_id=?1", [&id])
        .unwrap();
    drop(session);
    assert_eq!(
        fixture.store.acknowledge(&request, &review, note),
        Ok(original)
    );
}

#[test]
fn stale_evidence_and_modified_snapshots_cannot_clear_review() {
    let mut fixture = Fixture::new();
    let (mut session, id, review) = completed_review(&mut fixture);
    let mut edited = review.clone();
    edited.runtime_id = "different-runtime".into();
    assert_eq!(
        fixture
            .store
            .acknowledge(&new_acknowledgement_id().unwrap(), &edited, "Checked"),
        Err(Error::ReviewChanged)
    );
    fixture
        .db()
        .execute(
            "UPDATE launches SET review_required=1 WHERE run_id=?1",
            [&id],
        )
        .unwrap();
    assert_eq!(
        fixture
            .store
            .acknowledge(&new_acknowledgement_id().unwrap(), &review, "Checked"),
        Err(Error::ReviewChanged)
    );
    blocked(&mut session, Block::ReviewRequired);
    let current = fixture.store.review(&id).unwrap().unwrap();
    assert!(current.evidence_version > review.evidence_version);
    fixture
        .store
        .acknowledge(
            &new_acknowledgement_id().unwrap(),
            &current,
            "Reviewed the new evidence",
        )
        .unwrap();
    assert_eq!(fixture.store.review(&id).unwrap(), None);
    assert_eq!(
        fixture
            .store
            .acknowledge(&new_acknowledgement_id().unwrap(), &current, "Reviewed"),
        Err(Error::ReviewChanged)
    );
}

#[test]
fn local_cleanup_and_output_completion_are_required() {
    let mut fixture = Fixture::new();
    let (_, id) = fixture.admit();
    let mut session = fixture.session(Limits::default());
    let permit = claim(&mut session);
    let mut fake = Fake::new(&fixture.path);
    fake.reply = Ok(StartOutcome::Uncertain);
    start(&mut session, permit, &mut fake).unwrap();
    let active = fixture.store.review(&id).unwrap().unwrap();
    assert_eq!(
        fixture
            .store
            .acknowledge(&new_acknowledgement_id().unwrap(), &active, "Checked"),
        Err(Error::NotCompleted)
    );
    session
        .observe(&id, &known(ExecutionState::Ended, None, Cleanup::Confirmed))
        .unwrap();
    let cleaned = fixture.store.review(&id).unwrap().unwrap();
    assert_eq!(
        fixture
            .store
            .acknowledge(&new_acknowledgement_id().unwrap(), &cleaned, "Checked"),
        Err(Error::NotCompleted)
    );
    session.finish(&id, 0).unwrap();
    assert_eq!(
        fixture
            .store
            .acknowledge(&new_acknowledgement_id().unwrap(), &cleaned, "Checked"),
        Err(Error::ReviewChanged)
    );
    let completed = fixture.store.review(&id).unwrap().unwrap();
    fixture
        .store
        .acknowledge(
            &new_acknowledgement_id().unwrap(),
            &completed,
            "Checked after output completion",
        )
        .unwrap();
}

#[test]
fn note_and_request_bounds_fail_without_clearing_review() {
    let mut fixture = Fixture::new();
    let (_session, id, review) = completed_review(&mut fixture);
    for note in [
        "".to_owned(),
        " \n\t".into(),
        "bad\0note".into(),
        "x".repeat(4097),
        "é".repeat(2049),
    ] {
        assert_eq!(
            fixture
                .store
                .acknowledge(&new_acknowledgement_id().unwrap(), &review, &note),
            Err(Error::InvalidReview)
        );
    }
    for request in ["".into(), "a".repeat(63), "A".repeat(64), "g".repeat(64)] {
        assert_eq!(
            fixture.store.acknowledge(&request, &review, "Checked"),
            Err(Error::InvalidReview)
        );
    }
    assert!(fixture.store.review(&id).unwrap().is_some());
    fixture
        .store
        .acknowledge(
            &new_acknowledgement_id().unwrap(),
            &review,
            &"é".repeat(2048),
        )
        .unwrap();
    assert_eq!(
        fixture.store.acknowledgement(&"0".repeat(64)).unwrap(),
        None
    );
    assert_eq!(fixture.store.review("missing").unwrap(), None);
}

#[test]
fn acknowledgement_preserves_controls_and_uncertainty_reported_after_review() {
    let mut fixture = Fixture::new();
    let (_, first) = fixture.admit();
    let (_, second) = fixture.admit();
    let safety = rustward::safety::Safety::default();
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
    let permits = [claim(&mut session), claim(&mut session)];
    let mut fake = Fake::new(&fixture.path);
    fake.reply = Ok(StartOutcome::Accepted);
    for permit in permits {
        start(&mut session, permit, &mut fake).unwrap();
    }
    let mut first_review = None;
    for id in [&first, &second] {
        session.observe(id, &Observation::Missing).unwrap();
        session
            .observe(id, &known(ExecutionState::Ended, None, Cleanup::Confirmed))
            .unwrap();
        session.finish(id, 0).unwrap();
        if id == &first {
            first_review = fixture.store.review(&first).unwrap();
        }
    }
    let one = first_review.unwrap();
    let two = fixture.store.review(&second).unwrap().unwrap();
    fixture
        .store
        .acknowledge(
            &new_acknowledgement_id().unwrap(),
            &one,
            "Checked first run",
        )
        .unwrap();
    blocked(&mut session, Block::ReviewRequired);
    session.set_paused(true).unwrap();
    fixture
        .store
        .acknowledge(
            &new_acknowledgement_id().unwrap(),
            &two,
            "Checked second run",
        )
        .unwrap();
    blocked(&mut session, Block::Paused);
    safety.fail();
    session.set_paused(false).unwrap();
    assert!(matches!(session.claim_next(), Err(Error::Closed)));
}

#[test]
fn pending_reviews_use_bounded_pages_and_do_not_group_acknowledgements() {
    let mut fixture = Fixture::new();
    let (session, id, _) = completed_review(&mut fixture);
    assert_eq!(
        fixture.store.pending_reviews(None, 0),
        Err(Error::InvalidLimits)
    );
    assert_eq!(
        fixture.store.pending_reviews(None, 101),
        Err(Error::InvalidLimits)
    );
    let page = fixture.store.pending_reviews(None, 1).unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].run_id, id);
    assert!(
        fixture
            .store
            .pending_reviews(Some(&id), 1)
            .unwrap()
            .is_empty()
    );
    drop(session);
}

pub(super) fn concurrent_acknowledgements(same_id: bool) {
    let mut fixture = Fixture::new();
    let (_session, _, review) = completed_review(&mut fixture);
    let barrier = Arc::new(Barrier::new(2));
    let request = new_acknowledgement_id().unwrap();
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let mut store = Store::open(&fixture.path).unwrap();
            let request = if same_id {
                request.clone()
            } else {
                new_acknowledgement_id().unwrap()
            };
            let review = review.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                store.acknowledge(&request, &review, "Checked")
            })
        })
        .collect();
    let mut results: Vec<Result<Acknowledgement, Error>> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert_eq!(
        fixture
            .db()
            .query_row("SELECT count(*) FROM acknowledgements", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    if same_id {
        assert_eq!(results.pop().unwrap(), results.pop().unwrap());
    } else {
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert!(results.contains(&Err(Error::ReviewChanged)));
    }
}

#[test]
fn competing_identical_retries_return_one_record() {
    concurrent_acknowledgements(true);
}
#[test]
fn competing_different_ids_cannot_acknowledge_the_same_review_twice() {
    concurrent_acknowledgements(false);
}

#[test]
fn an_old_retry_cannot_release_new_uncertainty() {
    let mut fixture = Fixture::new();
    let (mut session, id, review) = completed_review(&mut fixture);
    let request = new_acknowledgement_id().unwrap();
    let original = fixture
        .store
        .acknowledge(&request, &review, "Checked original evidence")
        .unwrap();
    fixture
        .db()
        .execute(
            "UPDATE launches SET review_required=1 WHERE run_id=?1",
            [&id],
        )
        .unwrap();
    let current = fixture.store.review(&id).unwrap().unwrap();
    assert!(current.evidence_version > review.evidence_version);
    assert_eq!(
        fixture
            .store
            .acknowledge(&request, &review, "Checked original evidence")
            .unwrap(),
        original
    );
    blocked(&mut session, Block::ReviewRequired);
    assert_eq!(
        fixture.store.acknowledge(
            &new_acknowledgement_id().unwrap(),
            &review,
            "Checked original evidence"
        ),
        Err(Error::ReviewChanged)
    );
    fixture
        .store
        .acknowledge(
            &new_acknowledgement_id().unwrap(),
            &current,
            "Checked the new uncertainty",
        )
        .unwrap();
    assert_eq!(fixture.store.review(&id).unwrap(), None);
}

#[test]
fn pending_pages_preserve_order_when_an_earlier_review_is_acknowledged() {
    let mut fixture = Fixture::new();
    let mut ids: Vec<_> = (0..3).map(|_| fixture.admit().1).collect();
    let mut session = fixture.session(Limits {
        active_runs: 3,
        ..Limits::default()
    });
    let permits = vec![
        claim(&mut session),
        claim(&mut session),
        claim(&mut session),
    ];
    let mut fake = Fake::new(&fixture.path);
    for permit in permits {
        start(&mut session, permit, &mut fake).unwrap();
    }
    for id in &ids {
        session.observe(id, &Observation::Missing).unwrap();
        session
            .observe(id, &known(ExecutionState::Ended, None, Cleanup::Confirmed))
            .unwrap();
        session.finish(id, 0).unwrap();
    }
    ids.sort();
    let first = fixture
        .store
        .pending_reviews(None, 1)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(first.run_id, ids[0]);
    fixture
        .store
        .acknowledge(
            &new_acknowledgement_id().unwrap(),
            &first,
            "Checked first review",
        )
        .unwrap();
    let rest = fixture
        .store
        .pending_reviews(Some(&first.run_id), 2)
        .unwrap();
    assert_eq!(
        rest.iter()
            .map(|review| review.run_id.clone())
            .collect::<Vec<_>>(),
        ids[1..]
    );
}
