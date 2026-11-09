use super::*;

#[test]
fn version_two_migration_preserves_rows_and_rolls_back_on_failure() {
    for fail in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.db");
        let db = Connection::open(&path).unwrap();
        db.execute_batch(include_str!("../fixtures/admission-v2.sql"))
            .unwrap();
        db.execute_batch("INSERT INTO issuer VALUES(1,zeroblob(32),zeroblob(32),100,0);
            INSERT INTO runs VALUES(3,'old-run','old-task',x'7b7d',zeroblob(32),zeroblob(32),'{}','[]','runtime-old','completed','cancellation');
            INSERT INTO admissions VALUES('old-key',x'1234','old-run');
            UPDATE sqlite_sequence SET seq=20 WHERE name='runs';").unwrap();
        if fail {
            db.execute_batch("CREATE VIEW synthetic_view AS SELECT run_id FROM runs")
                .unwrap();
        }
        if fail {
            assert!(matches!(Store::open(&path), Err(Error::Storage)));
            assert_eq!(
                db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                    .unwrap(),
                2
            );
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM sqlite_schema WHERE name='runs_v3'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
            db.execute_batch("DROP VIEW synthetic_view").unwrap();
        }
        let mut store = Store::open(&path).unwrap();
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            6
        );
        assert_eq!(
            store.run_state("old-run").unwrap().unwrap().result,
            Some(RunResult::Cancellation)
        );
        assert_eq!(
            store.cancel_queued("old-run"),
            Ok(Cancellation::AlreadyCancelled)
        );
        let row: (i64, String, Vec<u8>, String, String) = db
            .query_row(
                "SELECT sequence,runtime_id,task_snapshot,arguments,task_id FROM runs",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(
            row,
            (
                3,
                "runtime-old".into(),
                b"{}".to_vec(),
                "[]".into(),
                "old-task".into()
            )
        );
        assert_eq!(
            db.query_row("SELECT fingerprint FROM admissions", [], |r| r
                .get::<_, Vec<u8>>(0))
                .unwrap(),
            [0x12, 0x34]
        );
        let key = store.issue_key().unwrap();
        store
            .admit(
                &key,
                &config::load("examples/rustward.toml").unwrap(),
                "synthetic-report",
                &BTreeMap::from([("name".into(), "sample".into())]),
                "runtime-new",
            )
            .unwrap();
        assert_eq!(
            db.query_row("SELECT max(sequence) FROM runs", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            21
        );
        let mut session = Session::open(&path, Limits::default()).unwrap();
        blocked(&mut session, Block::Paused);
    }
}

#[test]
fn version_three_migration_preserves_admitted_and_active_identity() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.db");
    let db = Connection::open(&path).unwrap();
    db.execute_batch(include_str!("../fixtures/admission-v3.sql"))
        .unwrap();
    let id = "a".repeat(64);
    db.execute(
        "INSERT INTO issuer VALUES(1,zeroblob(32),zeroblob(32),100,0)",
        [],
    )
    .unwrap();
    let snapshot = br#"{"executable":"/synthetic/program","working_directory":"/synthetic"}"#;
    db.execute("INSERT INTO runs VALUES(7,?1,'retained/task',?2,x'1234',x'5678','{}','[]','runtime-old','running',NULL,1)", rusqlite::params![id,snapshot]).unwrap();
    db.execute("INSERT INTO launches(run_id,start_state,invocation,control_group,log_bytes) VALUES(?1,'accepted',zeroblob(16),'/synthetic/original',1024)", [&id]).unwrap();
    db.execute(
        "INSERT INTO admissions VALUES('permanent-key',x'abc0',?1)",
        [&id],
    )
    .unwrap();
    db.execute("UPDATE sqlite_sequence SET seq=99 WHERE name='runs'", [])
        .unwrap();
    let mut store = Store::open(&path).unwrap();
    assert_eq!(
        store.run_state(&id).unwrap().unwrap().phase,
        RunPhase::Running
    );
    type StoredTiming = (Vec<u8>, Vec<u8>, Vec<u8>, Option<i64>, Option<i64>);
    let row: StoredTiming = db.query_row("SELECT task_snapshot,task_revision,config_revision,dispatched_at,completed_at FROM runs", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    assert_eq!(
        row,
        (
            snapshot.to_vec(),
            vec![0x12, 0x34],
            vec![0x56, 0x78],
            None,
            None
        )
    );
    assert_eq!(
        db.query_row("SELECT fingerprint FROM admissions", [], |r| r
            .get::<_, Vec<u8>>(0))
            .unwrap(),
        vec![0xab, 0xc0]
    );
    assert_eq!(
        db.query_row(
            "SELECT seq FROM sqlite_sequence WHERE name='runs'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        99
    );
    let mut session = Session::open(&path, Limits::default()).unwrap();
    let record = session.record(&id).unwrap().unwrap();
    assert_eq!(
        record.identity.unwrap().control_group,
        "/synthetic/original"
    );
    assert!(record.review_required);
    assert_eq!(
        session.run_state(&id).unwrap().unwrap().result,
        Some(RunResult::Interruption)
    );
    assert_eq!(session.times(&id).unwrap().unwrap().dispatched_at_ms, None);
}

#[test]
fn version_four_review_migration_preserves_history_and_rolls_back_on_failure() {
    for fail in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.db");
        let db = Connection::open(&path).unwrap();
        db.execute_batch(include_str!("../fixtures/admission-v4.sql"))
            .unwrap();
        let id = "b".repeat(64);
        db.execute_batch("INSERT INTO issuer VALUES(1,zeroblob(32),zeroblob(32),100,0)")
            .unwrap();
        db.execute("INSERT INTO runs VALUES(7,?1,'retained/task',x'7b7d',x'1234',x'5678','{}','[]','runtime-old','completed','interruption',1,NULL,NULL)",[&id]).unwrap();
        db.execute("INSERT INTO launches(run_id,start_state,invocation,control_group,cleanup_confirmed,review_required,log_bytes,stop_reason,output_failed) VALUES(?1,'uncertain',zeroblob(16),'/synthetic/original',1,1,123,'storage',1)",[&id]).unwrap();
        db.execute(
            "INSERT INTO admissions VALUES('permanent-key',x'abc0',?1)",
            [&id],
        )
        .unwrap();
        db.execute_batch("UPDATE sqlite_sequence SET seq=99 WHERE name='runs'")
            .unwrap();
        if fail {
            db.execute_batch("CREATE TABLE acknowledgements (synthetic INTEGER)")
                .unwrap();
            assert!(matches!(Store::open(&path), Err(Error::Storage)));
            assert_eq!(
                db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                    .unwrap(),
                4
            );
            assert_eq!(db.query_row("SELECT count(*) FROM pragma_table_info('launches') WHERE name='evidence_version'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
            db.execute_batch("DROP TABLE acknowledgements").unwrap();
        }
        let mut store = Store::open(&path).unwrap();
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            6
        );
        let review = store.review(&id).unwrap().unwrap();
        assert_eq!(review.evidence_version, 1);
        assert_eq!(review.task_id, "retained/task");
        assert_eq!(review.runtime_id, "runtime-old");
        assert_eq!(review.task_revision, vec![0x12, 0x34]);
        assert_eq!(review.config_revision, vec![0x56, 0x78]);
        assert_eq!(
            review.launch.identity.as_ref().unwrap().control_group,
            "/synthetic/original"
        );
        assert!(review.launch.output_failed);
        assert_eq!(review.times.dispatched_at_ms, None);
        assert_eq!(review.times.completed_at_ms, None);
        assert_eq!(review.state.result, Some(RunResult::Interruption));
        store
            .acknowledge(
                &rustward::review::new_acknowledgement_id().unwrap(),
                &review,
                "Reviewed the retained failure",
            )
            .unwrap();
        assert_eq!(store.run_state(&id).unwrap(), Some(review.state));
        assert_eq!(
            db.query_row("SELECT task_snapshot FROM runs", [], |r| r
                .get::<_, Vec<u8>>(0))
                .unwrap(),
            b"{}"
        );
        assert_eq!(
            db.query_row("SELECT fingerprint FROM admissions", [], |r| r
                .get::<_, Vec<u8>>(0))
                .unwrap(),
            vec![0xab, 0xc0]
        );
        assert_eq!(
            db.query_row(
                "SELECT seq FROM sqlite_sequence WHERE name='runs'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            99
        );
    }
}
