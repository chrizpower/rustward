use super::*;
use crate::{
    admission::{Admission, Error, RunPhase, RunResult},
    dispatch::{Block, Claim, Limits, Session},
};
use rusqlite::{Connection, params};
use std::{collections::BTreeMap, os::unix::fs::PermissionsExt};

struct Fixture {
    root: tempfile::TempDir,
    config: config::Config,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        for name in ["tasks", "schedules", LOGS] {
            fs::create_dir(root.path().join(name)).unwrap();
        }
        fs::set_permissions(root.path().join(LOGS), fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(
            root.path().join("tasks/report.toml"),
            include_str!("../../../examples/tasks/synthetic-report.toml"),
        )
        .unwrap();
        let path = root.path().join("rustward.toml");
        fs::write(
            &path,
            "version=2\n[storage]\nstate_database='state.db'\nlog_directory='logs'\n",
        )
        .unwrap();
        let config = config::load(path).unwrap();
        Self { root, config }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }

    fn submit(&self, store: &mut Store, key: &str) -> Result<Admission, Error> {
        store.admit(
            key,
            &self.config,
            "report",
            &BTreeMap::from([("name".into(), "sample".into())]),
            "synthetic-runtime",
        )
    }

    fn queue(&self, store: &mut Store) -> (String, String) {
        let key = store.issue_key().unwrap();
        let Ok(Admission::Admitted { run_id }) = self.submit(store, &key) else {
            panic!("admission failed");
        };
        (key, run_id)
    }

    fn log(&self, id: &str) -> PathBuf {
        let path = self.path(LOGS).join(id);
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let file = path.join("stdout.log");
        fs::write(&file, b"synthetic output\n").unwrap();
        files::set_private_file(&file).unwrap();
        file
    }
}

fn run(args: &[&std::ffi::OsStr]) -> Result<(), String> {
    command(args.iter().map(|arg| arg.to_os_string()))
}

fn decide(dir: &Path, id: &str, decision: &str, note: &str) -> Result<(), String> {
    run(&[
        "decide".as_ref(),
        dir.as_os_str(),
        id.as_ref(),
        decision.as_ref(),
        note.as_ref(),
    ])
}

#[test]
fn backup_restore_requires_individual_decisions_and_a_new_issuer() {
    let fixture = Fixture::new();
    let database = fixture.path(DATABASE);
    let config = fixture.path("rustward.toml");
    let archive = fixture.path("backup");
    let restored = fixture.path("restored");
    let mut store = Store::open(&database).unwrap();
    let (active_key, active) = fixture.queue(&mut store);
    let (kept_key, kept) = fixture.queue(&mut store);
    let (cancelled_key, cancelled) = fixture.queue(&mut store);
    let (_, completed) = fixture.queue(&mut store);
    store.cancel_queued(&completed).unwrap();
    let unknown_old_key = store.issue_key().unwrap();
    let original_log = fixture.log(&active);
    let mut dispatch = Session::open(&database, Limits::default()).unwrap();
    dispatch.set_paused(false).unwrap();
    assert!(matches!(dispatch.claim_next(), Ok(Claim::Ready(_))));
    let backup_args = ["backup".as_ref(), config.as_os_str(), archive.as_os_str()];
    assert!(run(&backup_args).unwrap_err().contains("lock"));
    assert!(!archive.exists());
    drop(dispatch);
    drop(store);
    run(&backup_args).unwrap();
    assert!(run(&backup_args).is_err());
    assert_eq!(
        Session::open(archive.join(DATABASE), Limits::default()).err(),
        Some(Error::Closed)
    );
    assert!(run(&["release".as_ref(), archive.as_os_str()]).is_err());
    let corrupted = fixture.path("corrupted");
    let backup_log = archive.join(LOGS).join(&active).join("stdout.log");
    fs::write(&backup_log, b"changed").unwrap();
    assert!(
        restore(&archive, &corrupted)
            .unwrap_err()
            .contains("manifest")
    );
    assert!(!corrupted.exists());
    fs::copy(&original_log, &backup_log).unwrap();

    run(&[
        "restore".as_ref(),
        archive.as_os_str(),
        restored.as_os_str(),
    ])
    .unwrap();
    assert!(restore(&archive, &restored).is_err());
    assert!(!restored.join(MANIFEST).exists());
    let restore_db = restored.join(DATABASE);
    assert_eq!(
        fs::read(restored.join(LOGS).join(&active).join("stdout.log")).unwrap(),
        b"synthetic output\n"
    );
    let mut store = Store::open(&restore_db).unwrap();
    assert_eq!(store.restore_phase().unwrap().as_deref(), Some("review"));
    assert_eq!(store.restore_reviews().unwrap().len(), 3);
    run(&["reviews".as_ref(), restored.as_os_str()]).unwrap();
    assert_eq!(
        store.run_state(&active).unwrap().unwrap().phase,
        RunPhase::Launching
    );
    assert_eq!(
        fixture.submit(&mut store, &active_key),
        Ok(Admission::AlreadyAdmitted {
            run_id: active.clone()
        })
    );
    assert_eq!(
        fixture.submit(&mut store, &unknown_old_key),
        Err(Error::InvalidKey)
    );
    let fresh = store.issue_key().unwrap();
    assert_eq!(fixture.submit(&mut store, &fresh), Err(Error::Closed));

    assert_eq!(
        Session::open(&restore_db, Limits::default()).err(),
        Some(Error::Closed)
    );
    assert!(run(&["release".as_ref(), restored.as_os_str()]).is_err());
    assert!(decide(&restored, &active, "keep", "unsafe").is_err());
    assert!(decide(&restored, &active, "interrupt", " ").is_err());
    assert!(decide(&restored, &active, "unknown", "unknown").is_err());
    assert!(decide(&restored, "invalid", "keep", "no run").is_err());
    assert!(run(&[]).is_err());
    decide(&restored, &kept, "keep", "Confirmed this task did not run").unwrap();
    decide(
        &restored,
        &cancelled,
        "cancel",
        "Discarded after checking external state",
    )
    .unwrap();
    decide(
        &restored,
        &active,
        "interrupt",
        "Confirmed the original host has no remaining process",
    )
    .unwrap();
    assert!(decide(&restored, &active, "interrupt", "Repeated decision").is_err());
    assert_eq!(
        fixture.submit(&mut store, &cancelled_key),
        Ok(Admission::AlreadyAdmitted {
            run_id: cancelled.clone()
        })
    );
    assert_eq!(
        store.run_state(&cancelled).unwrap().unwrap().result,
        Some(RunResult::Cancellation)
    );
    assert_eq!(
        store.run_state(&active).unwrap().unwrap().result,
        Some(RunResult::Interruption)
    );
    assert_eq!(
        store.run_state(&completed).unwrap().unwrap().result,
        Some(RunResult::Cancellation)
    );
    assert_eq!(store.pending_reviews(None, 10).unwrap()[0].run_id, active);
    assert_eq!(
        Session::open(&restore_db, Limits::default()).err(),
        Some(Error::Closed)
    );
    run(&["release".as_ref(), restored.as_os_str()]).unwrap();

    let mut dispatch = Session::open(&restore_db, Limits::default()).unwrap();
    assert!(matches!(
        dispatch.claim_next(),
        Ok(Claim::Blocked(Block::Paused))
    ));
    dispatch.set_paused(false).unwrap();
    assert!(matches!(
        dispatch.claim_next(),
        Ok(Claim::Blocked(Block::ReviewRequired))
    ));
    let review = store.pending_reviews(None, 10).unwrap().remove(0);
    store
        .acknowledge(&"a".repeat(64), &review, "Checked external effects")
        .unwrap();
    assert!(matches!(dispatch.claim_next(), Ok(Claim::Ready(_))));
    assert_eq!(
        dispatch.run_state(&kept).unwrap().unwrap().phase,
        RunPhase::Launching
    );
    store.set_closed(false).unwrap();
    assert!(matches!(
        fixture.submit(&mut store, &fresh),
        Ok(Admission::Admitted { .. })
    ));
    assert_eq!(
        fixture.submit(&mut store, &kept_key),
        Ok(Admission::AlreadyAdmitted { run_id: kept })
    );
    assert_eq!(
        fixture.submit(&mut store, &unknown_old_key),
        Err(Error::InvalidKey)
    );
}

#[test]
fn retention_retries_partial_log_removal_and_keeps_protected_history() {
    let fixture = Fixture::new();
    let path = fixture.path(DATABASE);
    let mut store = Store::open(&path).unwrap();
    let (old_key, old) = fixture.queue(&mut store);
    let (retry_key, retry) = fixture.queue(&mut store);
    let (_, unknown) = fixture.queue(&mut store);
    let (_, future) = fixture.queue(&mut store);
    let (_, queued) = fixture.queue(&mut store);
    let (_, protected) = fixture.queue(&mut store);
    let (_, active) = fixture.queue(&mut store);
    for id in [&old, &retry, &unknown, &future] {
        store.cancel_queued(id).unwrap();
    }
    let db = Connection::open(&path).unwrap();
    db.execute("UPDATE runs SET completed_at=CASE run_id WHEN ?1 THEN 10 WHEN ?2 THEN 11 WHEN ?3 THEN NULL ELSE 100 END WHERE phase='completed'",
        params![old, retry, unknown]).unwrap();
    db.execute("UPDATE runs SET phase='completed',dispatched=1,result='interruption',completed_at=5 WHERE run_id=?1", [&protected]).unwrap();
    db.execute(
        "UPDATE runs SET phase='running',dispatched=1 WHERE run_id=?1",
        [&active],
    )
    .unwrap();
    db.execute("INSERT INTO launches (run_id,start_state,cleanup_confirmed,review_required,log_bytes) VALUES (?1,'accepted',1,1,17),(?2,'accepted',0,0,17)",
        params![protected,active]).unwrap();
    let old_log = fixture.log(&old);
    let retry_log = fixture.log(&retry);
    let protected_log = fixture.log(&protected);
    let logs = WorkRoot::open(fixture.path(LOGS)).unwrap();
    let root = logs.checked_directory().unwrap();
    assert_eq!(
        store.expire(11, |id| {
            if id.as_str() == retry {
                return Err(Error::Storage);
            }
            crate::capture::remove_logs(root, id).map_err(|_| Error::Storage)
        }),
        Err(Error::Storage)
    );
    assert!(!old_log.exists());
    assert!(retry_log.exists());
    let mut store = Store::open(&path).unwrap();
    assert!(store.run_state(&old).unwrap().is_some());
    assert_eq!(
        store.expire(11, |id| crate::capture::remove_logs(root, id)
            .map_err(|_| Error::Storage)),
        Ok(2)
    );
    assert!(!retry_log.exists());
    assert!(protected_log.exists());
    assert_eq!(
        fixture.submit(&mut store, &old_key),
        Ok(Admission::HistoryRemoved { run_id: old })
    );
    assert_eq!(
        fixture.submit(&mut store, &retry_key),
        Ok(Admission::HistoryRemoved { run_id: retry })
    );
    for id in [unknown, future, queued, protected, active] {
        assert!(store.run_state(&id).unwrap().is_some());
    }
    for _ in 0..33 {
        let (_, id) = fixture.queue(&mut store);
        store.cancel_queued(&id).unwrap();
    }
    assert_eq!(store.expire(i64::MAX, |_| Ok(())), Ok(32));
    assert_eq!(store.expire(i64::MAX, |_| Ok(())), Ok(2));
    assert_eq!(store.expire(i64::MAX, |_| Ok(())), Ok(0));
}

#[test]
fn backup_rejects_unsafe_files_and_invalid_manifest() {
    let fixture = Fixture::new();
    let mut store = Store::open(fixture.path(DATABASE)).unwrap();
    let (_, id) = fixture.queue(&mut store);
    let log = fixture.log(&id);
    let archive = fixture.path("backup");
    let config = fixture.path("rustward.toml");
    let bad = log.with_file_name("unexpected");
    fs::write(&bad, b"extra").unwrap();
    assert!(backup(&config, &archive).is_err());
    fs::remove_file(bad).unwrap();
    fs::set_permissions(&log, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(backup(&config, &archive).is_err());
    files::set_private_file(&log).unwrap();
    backup(&config, &archive).unwrap();
    assert!(backup(&archive.join("missing.toml"), &fixture.path("invalid")).is_err());
    fs::write(archive.join(MANIFEST), b"invalid").unwrap();
    assert!(restore(&archive, &fixture.path("invalid")).is_err());
    assert!(!fixture.path("invalid").exists());
}

#[test]
fn backup_requires_protected_completed_output() {
    let fixture = Fixture::new();
    let database = fixture.path(DATABASE);
    let mut store = Store::open(&database).unwrap();
    let (_, id) = fixture.queue(&mut store);
    let db = Connection::open(&database).unwrap();
    db.execute(
        "UPDATE runs SET phase='completed',dispatched=1,result='interruption',completed_at=1 WHERE run_id=?1",
        [&id],
    ).unwrap();
    db.execute(
        "INSERT INTO launches (run_id,start_state,cleanup_confirmed,review_required,log_bytes)
         VALUES (?1,'accepted',1,1,17)",
        [&id],
    )
    .unwrap();
    let archive = fixture.path("backup");
    let config = fixture.path("rustward.toml");
    assert!(
        backup(&config, &archive)
            .unwrap_err()
            .contains("Protected output")
    );
    assert!(!archive.exists());
    fixture.log(&id);
    backup(&config, &archive).unwrap();
    let target = fixture.path("restored");
    restore(&archive, &target).unwrap();
    let mut store = Store::open(target.join(DATABASE)).unwrap();
    assert!(store.restore_reviews().unwrap().is_empty());
    assert_eq!(store.pending_reviews(None, 10).unwrap()[0].run_id, id);
    assert!(backup(&config, &target.join("missing/backup")).is_err());
    fs::write(
        &config,
        "version=2\n[storage]\nstate_database='restored/state.db'\nlog_directory='restored/logs'\n",
    )
    .unwrap();
    assert!(
        backup(&config, &fixture.path("too-early"))
            .unwrap_err()
            .contains("Finish restore review")
    );
    store.release_restore().unwrap();
    backup(&config, &fixture.path("second-backup")).unwrap();
}

#[test]
fn service_applies_retention_and_closes_after_a_storage_failure() {
    use super::super::{Safety, Service};
    use crate::controller::tests::Fixture as ControllerFixture;
    let fixture = Fixture::new();
    let config = fixture.path("rustward.toml");
    let settings = fs::read_to_string(&config).unwrap();
    fs::write(&config, format!("{settings}\n[retention]\ndays=0\n")).unwrap();
    let control = ControllerFixture::new();
    let mut store = Store::open_with_safety(&control.path, control.safety.clone()).unwrap();
    let (_, expired) = fixture.queue(&mut store);
    store.cancel_queued(&expired).unwrap();
    let mut service = Service::new(
        config::load(&config).unwrap(),
        "synthetic-runtime".to_owned(),
        store,
        control.controller(Limits::default()),
        control.safety.clone(),
    )
    .unwrap();
    service.step().unwrap();
    assert!(service.store.run_state(&expired).unwrap().is_none());

    let (_, failed) = fixture.queue(&mut service.store);
    service.store.cancel_queued(&failed).unwrap();
    let db = Connection::open(&control.path).unwrap();
    db.execute_batch(
        "CREATE TRIGGER fail_expiry BEFORE DELETE ON runs
        BEGIN SELECT RAISE(ABORT, 'synthetic storage fault'); END;",
    )
    .unwrap();
    service.next_retention = std::time::Instant::now();
    service.step().unwrap();
    assert!(control.safety.failed());
    assert!(service.step().is_err());
    let mut reopened = Store::open_with_safety(&control.path, Safety::default()).unwrap();
    assert!(reopened.run_state(&failed).unwrap().is_some());
}

#[test]
fn backup_does_not_upgrade_the_source_database() {
    let fixture = Fixture::new();
    let database = fixture.path(DATABASE);
    drop(Store::open(&database).unwrap());
    let source = Connection::open(&database).unwrap();
    source
        .execute_batch(
            "DROP TABLE restore_reviews; DROP TABLE restore_control;
        DROP INDEX runs_completed; PRAGMA user_version=8;",
        )
        .unwrap();
    let archive = fixture.path("backup");
    backup(&fixture.path("rustward.toml"), &archive).unwrap();
    assert_eq!(
        source
            .pragma_query_value(None, "user_version", |row| row.get::<_, i32>(0))
            .unwrap(),
        8
    );
    let mut backup = Store::open(archive.join(DATABASE)).unwrap();
    assert_eq!(backup.restore_phase().unwrap().as_deref(), Some("backup"));
}

#[test]
fn retirement_lists_unfinished_runs_and_unresolved_reviews() {
    let fixture = Fixture::new();
    let path = fixture.path(DATABASE);
    let mut store = Store::open(&path).unwrap();
    let (_, queued) = fixture.queue(&mut store);
    let (_, active) = fixture.queue(&mut store);
    let (_, review) = fixture.queue(&mut store);
    let (_, finished) = fixture.queue(&mut store);
    store.cancel_queued(&finished).unwrap();
    drop(store);
    let db = Connection::open(&path).unwrap();
    db.execute(
        "UPDATE runs SET runtime_id='queued' WHERE run_id=?1",
        [&queued],
    )
    .unwrap();
    db.execute(
        "UPDATE runs SET runtime_id='active',phase='running',dispatched=1 WHERE run_id=?1",
        [&active],
    )
    .unwrap();
    db.execute("UPDATE runs SET runtime_id='review',phase='completed',result='interruption',dispatched=1 WHERE run_id=?1", [&review]).unwrap();
    db.execute("INSERT INTO launches (run_id,start_state,cleanup_confirmed,review_required,log_bytes) VALUES (?1,'accepted',1,1,0)", [&review]).unwrap();
    let lock = lock_database(&path).unwrap();
    assert!(execute(&["runtime-references".into(), path.clone().into_os_string()]).is_err());
    drop(lock);
    execute(&["runtime-references".into(), path.clone().into_os_string()]).unwrap();
    assert_eq!(
        runtime_references(&path).unwrap(),
        ["active", "queued", "review"]
    );
}
