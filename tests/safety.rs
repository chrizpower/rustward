#![cfg(target_os = "linux")]

use std::collections::BTreeMap;

use rustward::{
    admission::{Admission, Error, Store},
    config,
    dispatch::{Claim, Limits, Session},
    safety::Safety,
};

#[test]
fn storage_failure_closes_connected_admission_and_dispatch_without_a_write() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.db");
    let safety = Safety::default();
    let mut store = Store::open_with_safety(&path, safety.clone()).unwrap();
    let mut peer = Store::open_with_safety(&path, safety.clone()).unwrap();
    let mut dispatch = Session::open_with_safety(&path, Limits::default(), safety.clone()).unwrap();
    dispatch.set_paused(false).unwrap();
    let config = config::load("examples/rustward.toml").unwrap();
    let values = BTreeMap::from([("name".into(), "sample".into())]);
    let key = store.issue_key().unwrap();
    let new_key = store.issue_key().unwrap();
    let Admission::Admitted { run_id } = store
        .admit(&key, &config, "synthetic-report", &values, "runtime-1")
        .unwrap()
    else {
        panic!()
    };
    assert!(matches!(dispatch.claim_next().unwrap(), Claim::Ready(_)));

    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_write BEFORE UPDATE ON issuer BEGIN SELECT RAISE(FAIL,'synthetic fault'); END;").unwrap();
    assert_eq!(store.set_closed(true), Err(Error::Storage));
    assert!(safety.failed());
    assert_eq!(
        peer.admit(&new_key, &config, "synthetic-report", &values, "runtime-1"),
        Err(Error::Closed)
    );
    assert!(matches!(dispatch.claim_next(), Err(Error::Closed)));
    assert_eq!(
        peer.admit(&key, &config, "synthetic-report", &values, "runtime-2"),
        Ok(Admission::AlreadyAdmitted { run_id })
    );
}

#[test]
fn failure_gate_is_shared_and_cannot_be_cleared_by_reopening() {
    let root = tempfile::tempdir().unwrap();
    let safety = Safety::default();
    assert!(matches!(
        Store::open_with_safety(root.path().join("missing/state.db"), safety.clone()),
        Err(Error::Storage)
    ));
    let mut dispatch = Session::open_with_safety(
        root.path().join("state.db"),
        Limits::default(),
        safety.clone(),
    )
    .unwrap();
    dispatch.set_paused(false).unwrap();
    assert!(matches!(dispatch.claim_next(), Err(Error::Closed)));
    assert!(safety.failed());
}
