mod common;
use common::validate;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Barrier},
    thread,
};

use hmac::{Hmac, Mac};
use rusqlite::{Connection, params};
use rustward::admission::{Admission, Cancellation, Error, RunPhase, RunResult, RunState, Store};
use sha2::Sha256;
use tempfile::TempDir;

fn setup() -> (TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("state.db")).unwrap();
    (directory, store)
}

fn admit(store: &mut Store, key: &str) -> Result<Admission, Error> {
    store.admit(
        key,
        &validate(include_str!("../examples/tasks/synthetic-report.toml")).unwrap(),
        "synthetic-report",
        &BTreeMap::from([("name".into(), "sample".into())]),
        "runtime-1",
    )
}

fn new_run(store: &mut Store) -> (String, String) {
    let key = store.issue_key().unwrap();
    let Admission::Admitted { run_id } = admit(store, &key).unwrap() else {
        panic!("expected a new run")
    };
    (key, run_id)
}

fn queued() -> RunState {
    RunState {
        phase: RunPhase::Queued,
        result: None,
    }
}
fn cancelled() -> RunState {
    RunState {
        phase: RunPhase::Completed,
        result: Some(RunResult::Cancellation),
    }
}

#[test]
fn cancellation_preserves_identity_and_releases_capacity_after_reopen() {
    let (directory, mut store) = setup();
    store.set_queue_limit(1).unwrap();
    let (key, run_id) = new_run(&mut store);
    let next = store.issue_key().unwrap();
    assert_eq!(store.run_state(&run_id), Ok(Some(queued())));
    assert_eq!(admit(&mut store, &next), Err(Error::QueueFull));
    store.set_closed(true).unwrap();
    assert_eq!(store.cancel_queued(&run_id), Ok(Cancellation::Cancelled));
    assert_eq!(store.run_state(&run_id), Ok(Some(cancelled())));
    drop(store);
    let mut store = Store::open(directory.path().join("state.db")).unwrap();
    assert_eq!(store.run_state(&run_id), Ok(Some(cancelled())));
    assert_eq!(
        store.cancel_queued(&run_id),
        Ok(Cancellation::AlreadyCancelled)
    );
    assert_eq!(
        admit(&mut store, &key),
        Ok(Admission::AlreadyAdmitted {
            run_id: run_id.clone()
        })
    );
    store.set_closed(false).unwrap();
    assert!(matches!(
        admit(&mut store, &next),
        Ok(Admission::Admitted { .. })
    ));
    let connection = Connection::open(directory.path().join("state.db")).unwrap();
    let counts: (u32, u32) = connection
        .query_row(
            "SELECT (SELECT count(*) FROM runs), (SELECT count(*) FROM admissions)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(counts, (2, 2));
    connection
        .execute("DELETE FROM runs WHERE run_id=?1", [&run_id])
        .unwrap();
    assert_eq!(store.run_state(&run_id), Ok(None));
    assert_eq!(store.cancel_queued(&run_id), Ok(Cancellation::NotFound));
    assert_eq!(
        admit(&mut store, &key),
        Ok(Admission::HistoryRemoved { run_id })
    );
    for missing in ["", "unknown", "'; DELETE FROM runs; --"] {
        assert_eq!(store.cancel_queued(missing), Ok(Cancellation::NotFound));
        assert_eq!(store.run_state(missing), Ok(None));
    }
}

#[test]
fn concurrent_cancellations_change_one_record_once() {
    let (directory, mut store) = setup();
    let (_, run_id) = new_run(&mut store);
    let barrier = Arc::new(Barrier::new(4));
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let mut store = Store::open(directory.path().join("state.db")).unwrap();
            let run_id = run_id.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                store.cancel_queued(&run_id).unwrap()
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(
        results
            .iter()
            .filter(|r| **r == Cancellation::Cancelled)
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|r| **r == Cancellation::AlreadyCancelled)
            .count(),
        3
    );
    assert_eq!(store.run_state(&run_id), Ok(Some(cancelled())));
}

#[test]
fn admission_and_cancellation_serialize_capacity_checks() {
    let (directory, mut store) = setup();
    store.set_queue_limit(1).unwrap();
    let (_, run_id) = new_run(&mut store);
    let key = store.issue_key().unwrap();
    let mut other = Store::open(directory.path().join("state.db")).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let other_barrier = barrier.clone();
    let cancel = thread::spawn(move || {
        other_barrier.wait();
        other.cancel_queued(&run_id)
    });
    barrier.wait();
    let admitted = admit(&mut store, &key);
    assert_eq!(cancel.join().unwrap(), Ok(Cancellation::Cancelled));
    match admitted {
        Err(Error::QueueFull) => {
            assert!(matches!(
                admit(&mut store, &key),
                Ok(Admission::Admitted { .. })
            ));
        }
        Ok(Admission::Admitted { .. }) => {}
        other => panic!("unexpected admission result: {other:?}"),
    }
    let next = store.issue_key().unwrap();
    assert_eq!(admit(&mut store, &next), Err(Error::QueueFull));
}

#[test]
fn failed_cancellation_keeps_the_run_queued_and_closes_the_handle() {
    for trigger in [
        "CREATE TRIGGER fail_cancel AFTER UPDATE ON runs BEGIN SELECT RAISE(ABORT, 'synthetic failure'); END;",
        "CREATE TRIGGER fail_cancel BEFORE UPDATE ON runs BEGIN SELECT RAISE(IGNORE); END;",
    ] {
        let (directory, mut store) = setup();
        let (key, run_id) = new_run(&mut store);
        let path = directory.path().join("state.db");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(trigger).unwrap();
        assert_eq!(store.cancel_queued(&run_id), Err(Error::Storage));
        assert_eq!(store.run_state(&run_id), Err(Error::Closed));
        assert_eq!(store.cancel_queued(&run_id), Err(Error::Closed));
        connection
            .execute_batch("DROP TRIGGER fail_cancel")
            .unwrap();
        drop(store);
        let mut store = Store::open(path).unwrap();
        assert_eq!(store.run_state(&run_id), Ok(Some(queued())));
        assert_eq!(
            admit(&mut store, &key),
            Ok(Admission::AlreadyAdmitted {
                run_id: run_id.clone()
            })
        );
        assert_eq!(store.cancel_queued(&run_id), Ok(Cancellation::Cancelled));
    }
}

#[test]
fn constraints_and_state_reads_reject_inconsistent_lifecycle_fields() {
    for (phase, result) in [
        ("completed", None),
        ("queued", Some("cancellation")),
        ("completed", Some("success")),
        ("running", None),
    ] {
        let (directory, mut store) = setup();
        let (_, run_id) = new_run(&mut store);
        let connection = Connection::open(directory.path().join("state.db")).unwrap();
        assert!(
            connection
                .execute(
                    "UPDATE runs SET phase=?1, result=?2",
                    params![phase, result]
                )
                .is_err()
        );
        assert_eq!(store.run_state(&run_id), Ok(Some(queued())));
        connection
            .pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        connection
            .execute(
                "UPDATE runs SET phase=?1, result=?2",
                params![phase, result],
            )
            .unwrap();
        assert_eq!(store.cancel_queued(&run_id), Err(Error::Storage));
        let mut reader = Store::open(directory.path().join("state.db")).unwrap();
        assert_eq!(reader.run_state(&run_id), Err(Error::Storage));
        assert_eq!(reader.issue_key(), Err(Error::Closed));
    }
}

fn version_one(path: &Path, with_run: bool) -> String {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(include_str!("fixtures/admission-v1.sql"))
        .unwrap();
    let generation = [7u8; 32];
    let secret = [9u8; 32];
    connection
        .execute(
            "INSERT INTO issuer VALUES (1, ?1, ?2, 1, 0)",
            params![generation.as_slice(), secret.as_slice()],
        )
        .unwrap();
    let mut bytes = vec![1];
    bytes.extend_from_slice(&generation);
    bytes.extend_from_slice(&[11u8; 32]);
    let mut mac = Hmac::<Sha256>::new_from_slice(&secret).unwrap();
    mac.update(&bytes);
    bytes.extend_from_slice(&mac.finalize().into_bytes());
    let key = hex::encode(bytes);
    if with_run {
        connection.execute("INSERT INTO runs VALUES (7, 'legacy-run', 'synthetic-report', x'7b7d', zeroblob(32), zeroblob(32), '{\"name\":\"sample\"}', '[\"synthetic\"]', 'runtime-old', 'queued')", []).unwrap();
        connection
            .execute("UPDATE sqlite_sequence SET seq=25 WHERE name='runs'", [])
            .unwrap();
        connection
            .execute(
                "INSERT INTO admissions VALUES (?1, ?2, 'legacy-run')",
                params![
                    key,
                    hex::decode("a9dc0d353689888ac2da5f5528137d75a6362a3f125073ed3bdd4f6305b3576e")
                        .unwrap()
                ],
            )
            .unwrap();
    }
    key
}

fn stored_run(connection: &Connection) -> Vec<rusqlite::types::Value> {
    connection
        .query_row(
            "SELECT sequence, run_id, task_id, task_snapshot, task_revision,
                config_revision, submitted_inputs, arguments, runtime_id, phase FROM runs",
            [],
            |row| (0..10).map(|index| row.get(index)).collect(),
        )
        .unwrap()
}

#[test]
fn migrates_version_one_without_changing_admission_or_run_data() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    let key = version_one(&path, true);
    let connection = Connection::open(&path).unwrap();
    let original = stored_run(&connection);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        9
    );
    assert_eq!(store.run_state("legacy-run"), Ok(Some(queued())));
    assert_eq!(
        admit(&mut store, &key),
        Ok(Admission::AlreadyAdmitted {
            run_id: "legacy-run".into()
        })
    );
    let migrated = stored_run(&connection);
    assert_eq!(original, migrated);
    let blobs: Vec<Vec<u8>> = connection.query_row(
        "SELECT task_snapshot, task_revision, config_revision, generation, secret FROM runs, issuer",
        [],
        |row| (0..5).map(|index| row.get(index)).collect(),
    ).unwrap();
    assert_eq!(
        blobs,
        vec![
            b"{}".to_vec(),
            vec![0; 32],
            vec![0; 32],
            vec![7; 32],
            vec![9; 32]
        ]
    );
    let next = store.issue_key().unwrap();
    assert_eq!(admit(&mut store, &next), Err(Error::QueueFull));
    store.cancel_queued("legacy-run").unwrap();
    assert!(matches!(
        admit(&mut store, &next),
        Ok(Admission::Admitted { .. })
    ));
    let sequence: i64 = connection
        .query_row("SELECT max(sequence) FROM runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sequence, 26);
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.run_state("legacy-run"), Ok(Some(cancelled())));
}

#[test]
fn migrates_empty_version_one_and_preserves_deleted_sequence_numbers() {
    for previous_sequence in [None, Some(25)] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.db");
        let key = version_one(&path, false);
        let connection = Connection::open(&path).unwrap();
        if let Some(sequence) = previous_sequence {
            connection
                .execute(
                    "INSERT INTO sqlite_sequence VALUES ('runs', ?1)",
                    [sequence],
                )
                .unwrap();
        }
        let mut store = Store::open(&path).unwrap();
        assert!(matches!(
            admit(&mut store, &key),
            Ok(Admission::Admitted { .. })
        ));
        let sequence: i64 = connection
            .query_row("SELECT sequence FROM runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sequence, previous_sequence.unwrap_or(0) + 1);
    }
}

#[test]
fn failed_migration_rolls_back_table_replacement_and_version() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    let key = version_one(&path, true);
    let connection = Connection::open(&path).unwrap();
    // The view makes table rename fail after the migration drops the old table.
    connection
        .execute_batch("CREATE VIEW synthetic_view AS SELECT run_id FROM runs")
        .unwrap();
    assert!(matches!(Store::open(&path), Err(Error::Storage)));
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        1
    );
    let rows: u32 = connection
        .query_row(
            "SELECT count(*) FROM runs WHERE sequence=7 AND phase='queued'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 1);
    let scratch_tables: u32 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name='runs_v2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(scratch_tables, 0);
    connection
        .execute_batch("DROP VIEW synthetic_view")
        .unwrap();
    let mut store = Store::open(&path).unwrap();
    assert_eq!(
        admit(&mut store, &key),
        Ok(Admission::AlreadyAdmitted {
            run_id: "legacy-run".into()
        })
    );
}
