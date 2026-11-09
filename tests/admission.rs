mod common;
use common::validate;
use rusqlite::Connection;
use rustward::{
    admission::{Admission, Error, Store},
    config::Config,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Barrier},
    thread,
};
use tempfile::TempDir;
use toml::Value;

fn config() -> Config {
    validate(include_str!("../examples/tasks/synthetic-report.toml")).unwrap()
}
fn values() -> BTreeMap<String, Value> {
    BTreeMap::from([("name".into(), "sample".into())])
}
fn setup() -> (TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("state.db")).unwrap();
    (directory, store)
}
fn admit(store: &mut Store, key: &str) -> Result<Admission, Error> {
    store.admit(key, &config(), "synthetic-report", &values(), "runtime-1")
}
fn run_id(result: Admission) -> String {
    match result {
        Admission::Admitted { run_id }
        | Admission::AlreadyAdmitted { run_id }
        | Admission::HistoryRemoved { run_id } => run_id,
    }
}
fn counts(connection: &Connection) -> (u32, u32) {
    connection
        .query_row(
            "SELECT (SELECT count(*) FROM runs), (SELECT count(*) FROM admissions)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}

#[test]
fn retains_identity_and_snapshot_across_reopen_and_definition_changes() {
    let (directory, mut store) = setup();
    let key = store.issue_key().unwrap();
    let connection = Connection::open(directory.path().join("state.db")).unwrap();
    assert_eq!(counts(&connection), (0, 0));
    let id = run_id(admit(&mut store, &key).unwrap());
    assert_eq!(counts(&connection), (1, 1));
    drop(store);
    let mut store = Store::open(directory.path().join("state.db")).unwrap();
    store.set_closed(true).unwrap();
    store.set_queue_limit(0).unwrap();
    let empty = common::validate_set(&[]).unwrap();
    assert_eq!(
        store.admit(&key, &empty, "synthetic-report", &values(), ""),
        Ok(Admission::AlreadyAdmitted { run_id: id.clone() })
    );
    let (runtime, arguments, snapshot): (String, String, Vec<u8>) = connection
        .query_row(
            "SELECT runtime_id, arguments, task_snapshot FROM runs",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(runtime, "runtime-1");
    assert_eq!(
        serde_json::from_str::<Vec<String>>(&arguments).unwrap(),
        ["--name", "sample", "2", "false"]
    );
    assert!(
        String::from_utf8(snapshot)
            .unwrap()
            .contains("Synthetic report")
    );
    assert_eq!(
        store.admit(&key, &empty, "other", &values(), "runtime-1"),
        Err(Error::PayloadConflict)
    );
    connection.execute("DELETE FROM runs", []).unwrap();
    assert_eq!(
        admit(&mut store, &key),
        Ok(Admission::HistoryRemoved { run_id: id })
    );
    assert_eq!(counts(&connection), (0, 1));
}

#[test]
fn rejection_does_not_bind_key_or_reserve_capacity() {
    let (directory, mut store) = setup();
    let key = store.issue_key().unwrap();
    assert_eq!(
        store.admit(
            &key,
            &config(),
            "synthetic-report",
            &BTreeMap::new(),
            "runtime-1"
        ),
        Err(Error::InvalidInputs)
    );
    assert_eq!(
        store.admit(&key, &config(), "synthetic-report", &values(), ""),
        Err(Error::InvalidRuntime)
    );
    store.set_closed(true).unwrap();
    assert_eq!(admit(&mut store, &key), Err(Error::Closed));
    store.set_closed(false).unwrap();
    store.set_queue_limit(0).unwrap();
    assert_eq!(admit(&mut store, &key), Err(Error::QueueFull));
    store.set_queue_limit(1).unwrap();
    let id = run_id(admit(&mut store, &key).unwrap());
    let next = store.issue_key().unwrap();
    assert_eq!(admit(&mut store, &next), Err(Error::QueueFull));
    assert_eq!(
        admit(&mut store, &key),
        Ok(Admission::AlreadyAdmitted { run_id: id })
    );
    store.set_queue_limit(2).unwrap();
    assert!(matches!(
        admit(&mut store, &next),
        Ok(Admission::Admitted { .. })
    ));
    assert_eq!(
        counts(&Connection::open(directory.path().join("state.db")).unwrap()),
        (2, 2)
    );
}

#[test]
fn compares_typed_submitted_values_before_defaults() {
    let (_directory, mut store) = setup();
    let key = store.issue_key().unwrap();
    admit(&mut store, &key).unwrap();
    let mut changed = values();
    changed.insert("count".into(), 2.into());
    assert_eq!(
        store.admit(&key, &config(), "synthetic-report", &changed, "runtime-1"),
        Err(Error::PayloadConflict)
    );
    for value in [2.into(), true.into(), "sample ".into(), "".into()] {
        changed = BTreeMap::from([("name".into(), value)]);
        assert_eq!(
            store.admit(&key, &config(), "synthetic-report", &changed, "runtime-1"),
            Err(Error::PayloadConflict)
        );
    }
    for invalid in [
        Value::Datetime("1979-05-27".parse().unwrap()),
        Value::from(2.5),
        Value::Float(f64::INFINITY),
        Value::Array(vec![]),
        Value::Table(Default::default()),
    ] {
        let fresh = store.issue_key().unwrap();
        assert_eq!(
            store.admit(
                &fresh,
                &config(),
                "synthetic-report",
                &BTreeMap::from([("name".into(), invalid)]),
                "runtime-1"
            ),
            Err(Error::InvalidInputs)
        );
    }
    let key = store.issue_key().unwrap();
    let mut supplied = values();
    supplied.insert("enabled".into(), false.into());
    supplied.insert("count".into(), 3.into());
    assert!(
        store
            .admit(&key, &config(), "synthetic-report", &supplied, "runtime-1")
            .is_ok()
    );
}

#[test]
fn rejects_forged_foreign_and_old_unadmitted_keys() {
    let (directory, mut store) = setup();
    let key = store.issue_key().unwrap();
    for bad in [
        "".into(),
        "z".repeat(194),
        "00".repeat(97),
        key.to_uppercase(),
        format!("{}00", &key[..192]),
    ] {
        // A changed signature must not be accepted, even when its format is valid.
        if bad != key {
            assert_eq!(admit(&mut store, &bad), Err(Error::InvalidKey));
        }
    }
    let (_other_directory, mut other) = setup();
    assert_eq!(admit(&mut other, &key), Err(Error::InvalidKey));
    let admitted_key = store.issue_key().unwrap();
    let id = run_id(admit(&mut store, &admitted_key).unwrap());
    let connection = Connection::open(directory.path().join("state.db")).unwrap();
    connection
        .execute(
            "UPDATE issuer SET generation=zeroblob(32), secret=zeroblob(32)",
            [],
        )
        .unwrap();
    assert_eq!(admit(&mut store, &key), Err(Error::InvalidKey));
    assert_eq!(
        admit(&mut store, &admitted_key),
        Ok(Admission::AlreadyAdmitted { run_id: id })
    );
}

#[test]
fn enforces_payload_and_snapshot_bounds() {
    let (_directory, mut store) = setup();
    let key = store.issue_key().unwrap();
    let many = (0..257)
        .map(|i| (format!("input-{i}"), true.into()))
        .collect();
    assert_eq!(
        store.admit(&key, &config(), "synthetic-report", &many, "runtime-1"),
        Err(Error::TooLarge)
    );
    let large = BTreeMap::from([("name".into(), "x".repeat(65536).into())]);
    assert_eq!(
        store.admit(&key, &config(), "synthetic-report", &large, "runtime-1"),
        Err(Error::TooLarge)
    );
    assert_eq!(
        store.admit(&key, &config(), &"x".repeat(65536), &values(), "runtime-1"),
        Err(Error::TooLarge)
    );
    assert_eq!(
        store.admit(
            &key,
            &config(),
            "synthetic-report",
            &values(),
            &"x".repeat(1025)
        ),
        Err(Error::InvalidRuntime)
    );
    let source = include_str!("../examples/tasks/synthetic-report.toml").replace(
        "literal = \"--name\"",
        &format!("literal = \"{}\"", "x".repeat(600 * 1024)),
    );
    assert_eq!(
        store.admit(
            &key,
            &validate(&source).unwrap(),
            "synthetic-report",
            &values(),
            "runtime-1"
        ),
        Err(Error::TooLarge)
    );
}

#[test]
fn concurrent_retries_and_queue_checks_are_serialized() {
    let (directory, mut store) = setup();
    let key = store.issue_key().unwrap();
    let barrier = Arc::new(Barrier::new(4));
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let path = directory.path().join("state.db");
            let key = key.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                let mut store = Store::open(path).unwrap();
                barrier.wait();
                admit(&mut store, &key).unwrap()
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Admission::Admitted { .. }))
            .count(),
        1
    );
    let ids: Vec<_> = results.into_iter().map(run_id).collect();
    assert!(ids.iter().all(|id| *id == ids[0]));
    store.set_queue_limit(2).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let path = directory.path().join("state.db");
            let key = store.issue_key().unwrap();
            let barrier = barrier.clone();
            thread::spawn(move || {
                let mut store = Store::open(path).unwrap();
                barrier.wait();
                admit(&mut store, &key)
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(
        results
            .iter()
            .filter(|r| **r == Err(Error::QueueFull))
            .count(),
        1
    );
    assert_eq!(
        counts(&Connection::open(directory.path().join("state.db")).unwrap()),
        (2, 2)
    );
}

#[test]
fn failed_second_write_rolls_back_both_records_and_closes_handle() {
    let (directory, mut store) = setup();
    let key = store.issue_key().unwrap();
    let connection = Connection::open(directory.path().join("state.db")).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_admission BEFORE INSERT ON admissions BEGIN SELECT RAISE(ABORT, 'synthetic failure'); END;").unwrap();
    assert_eq!(admit(&mut store, &key), Err(Error::Storage));
    assert_eq!(counts(&connection), (0, 0));
    assert_eq!(store.issue_key(), Err(Error::Closed));
    assert_eq!(store.set_closed(false), Err(Error::Closed));
    connection
        .execute_batch("DROP TRIGGER fail_admission")
        .unwrap();
    drop(store);
    let mut store = Store::open(directory.path().join("state.db")).unwrap();
    assert!(admit(&mut store, &key).is_ok());
}

#[test]
fn rejects_unsupported_schema_and_storage_failures() {
    let directory = tempfile::tempdir().unwrap();
    assert!(matches!(
        Store::open(directory.path().join("missing/state.db")),
        Err(Error::Storage)
    ));
    let path = directory.path().join("state.db");
    let connection = Connection::open(&path).unwrap();
    connection.pragma_update(None, "user_version", 6).unwrap();
    assert!(matches!(Store::open(&path), Err(Error::UnsupportedSchema)));
    let (directory, mut store) = setup();
    let connection = Connection::open(directory.path().join("state.db")).unwrap();
    connection
        .execute("UPDATE issuer SET secret=x'00'", [])
        .unwrap();
    assert_eq!(store.issue_key(), Err(Error::Storage));
    let mut store = Store::open(directory.path().join("state.db")).unwrap();
    assert_eq!(admit(&mut store, &"01".repeat(97)), Err(Error::Storage));
}

#[test]
fn equivalent_schema_defaults_have_equal_revisions() {
    let (directory, mut store) = setup();
    let first = store.issue_key().unwrap();
    admit(&mut store, &first).unwrap();
    let second = store.issue_key().unwrap();
    let source = include_str!("../examples/tasks/synthetic-report.toml")
        .replace("type = \"integer\"", "type = \"integer\", required = false");
    store
        .admit(
            &second,
            &validate(&source).unwrap(),
            "synthetic-report",
            &values(),
            "runtime-2",
        )
        .unwrap();
    let connection = Connection::open(directory.path().join("state.db")).unwrap();
    let distinct: u32 = connection
        .query_row("SELECT count(DISTINCT task_revision) FROM runs", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(distinct, 1);
}

#[test]
fn unfinished_transaction_child() {
    let Some(path) = std::env::var_os("RUSTWARD_ADMISSION_CRASH_DATABASE") else {
        return;
    };
    let connection = Connection::open(path).unwrap();
    connection.execute_batch("BEGIN IMMEDIATE;
        INSERT INTO runs (run_id, task_id, task_snapshot, task_revision, config_revision, submitted_inputs, arguments, runtime_id, phase)
        SELECT 'unfinished', task_id, task_snapshot, task_revision, config_revision, submitted_inputs, arguments, runtime_id, phase FROM runs LIMIT 1;").unwrap();
    // Exit without dropping the connection or committing the first admission write.
    std::process::exit(23);
}

#[test]
fn process_exit_before_commit_leaves_no_partial_admission() {
    let (directory, mut store) = setup();
    let key = store.issue_key().unwrap();
    admit(&mut store, &key).unwrap();
    let path = directory.path().join("state.db");
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "unfinished_transaction_child"])
        .env("RUSTWARD_ADMISSION_CRASH_DATABASE", &path)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(23));
    drop(store);
    let mut store = Store::open(&path).unwrap();
    let connection = Connection::open(path).unwrap();
    assert_eq!(counts(&connection), (1, 1));
    let key = store.issue_key().unwrap();
    admit(&mut store, &key).unwrap();
    assert_eq!(counts(&connection), (2, 2));
}

#[test]
fn rejects_transient_databases_and_reports_lock_contention() {
    assert!(matches!(Store::open(":memory:"), Err(Error::Storage)));
    assert!(matches!(Store::open(""), Err(Error::Storage)));
    let (directory, mut store) = setup();
    let key = store.issue_key().unwrap();
    let connection = Connection::open(directory.path().join("state.db")).unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert_eq!(admit(&mut store, &key), Err(Error::Busy));
    connection.execute_batch("ROLLBACK").unwrap();
    assert!(admit(&mut store, &key).is_ok());
}

#[test]
fn request_encoding_has_a_stable_version_one_fingerprint() {
    let (directory, mut store) = setup();
    let key = store.issue_key().unwrap();
    admit(&mut store, &key).unwrap();
    let connection = Connection::open(directory.path().join("state.db")).unwrap();
    let stored: Vec<u8> = connection
        .query_row("SELECT fingerprint FROM admissions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        hex::encode(stored),
        "a9dc0d353689888ac2da5f5528137d75a6362a3f125073ed3bdd4f6305b3576e"
    );
}

#[test]
fn definition_revision_tracks_labels_defaults_and_argument_order() {
    let (directory, mut store) = setup();
    let example = include_str!("../examples/tasks/synthetic-report.toml");
    for source in [
        example.to_owned(),
        example.replace("Synthetic report", "Changed label"),
        example.replace("default = 2", "default = 3"),
        example.replace(
            "    { literal = \"--name\" },\n    { input = \"name\" },",
            "    { input = \"name\" },\n    { literal = \"--name\" },",
        ),
    ] {
        let key = store.issue_key().unwrap();
        store
            .admit(
                &key,
                &validate(&source).unwrap(),
                "synthetic-report",
                &values(),
                "runtime-1",
            )
            .unwrap();
    }
    let connection = Connection::open(directory.path().join("state.db")).unwrap();
    let distinct: u32 = connection
        .query_row("SELECT count(DISTINCT task_revision) FROM runs", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(distinct, 4);
}

#[test]
fn task_snapshot_retains_timing_defaults_and_revision_encoding() {
    use sha2::{Digest, Sha256};
    let (directory, mut store) = setup();
    let key = store.issue_key().unwrap();
    admit(&mut store, &key).unwrap();
    let expected = serde_json::json!({
        "id": "synthetic-report", "label": "Synthetic report",
        "timeout_seconds": 3600, "cleanup_grace_seconds": 30,
        "executable": "/synthetic/runtime/bin/report", "working_directory": "/synthetic/runtime",
        "arguments": [{"literal":"--name"},{"input":"name"},{"input":"count"},{"input":"enabled"}],
        "inputs": [
            {"id":"name","type":"string","required":true,"default":null,"choices":null},
            {"id":"count","type":"integer","required":false,"default":2,"choices":[1,2,3]},
            {"id":"enabled","type":"boolean","required":false,"default":false,"choices":null}
        ]
    });
    let snapshot = serde_json::to_vec(&expected).unwrap();
    let connection = Connection::open(directory.path().join("state.db")).unwrap();
    let stored: (Vec<u8>, Vec<u8>, Vec<u8>) = connection
        .query_row(
            "SELECT task_snapshot, task_revision, config_revision FROM runs",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(stored.0, snapshot);
    for (prefix, bytes, revision) in [
        (b"rustward-definition-v1".as_slice(), snapshot, stored.1),
        (
            b"rustward-configuration-v2".as_slice(),
            serde_json::to_vec(
                &serde_json::json!({"version": 2, "tasks": [expected], "schedules": []}),
            )
            .unwrap(),
            stored.2,
        ),
    ] {
        let mut hash = Sha256::new();
        hash.update(prefix);
        hash.update(bytes);
        assert_eq!(hash.finalize().to_vec(), revision);
    }
}
