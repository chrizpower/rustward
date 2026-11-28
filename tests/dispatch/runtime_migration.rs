use super::*;

#[test]
fn version_five_and_six_migration_keep_old_acknowledgements_and_runtime_identity() {
    for version in [5, 6] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.db");
        let db = Connection::open(&path).unwrap();
        db.execute_batch(include_str!("../fixtures/admission-v5.sql"))
            .unwrap();
        if version == 6 {
            db.execute_batch(include_str!("../fixtures/admission-v6-extension.sql"))
                .unwrap();
        }
        let id = "b".repeat(64);
        let acknowledgement_id = "a".repeat(64);
        let original = serde_json::json!({
            "acknowledgement_id": acknowledgement_id,
            "review": {
                "run_id": id, "evidence_version": 3,
                "task_id":"retained/task", "runtime_id":"runtime-old",
                "task_revision":[18,52], "config_revision":[86,120],
                "state":{"phase":"Completed","result":"Interruption"},
                "launch":{
                    "start_state":"uncertain", "identity":null,"exit":null,
                    "cleanup_confirmed":true,"review_required":true,"log_bytes":123,
                    "stop_reason":null,"output_failed":false
                },
                "times":{"dispatched_at_ms":null,"completed_at_ms":null}
            },
            "note":"Reviewed old evidence", "acknowledged_at_ms":12345
        })
        .to_string();
        db.execute_batch("INSERT INTO issuer VALUES(1,zeroblob(32),zeroblob(32),100,0)")
            .unwrap();
        db.execute("INSERT INTO runs VALUES(7,?1,'retained/task',x'7b7d',x'1234',x'5678','{}','[]','runtime-old','completed','interruption',1,NULL,NULL)",[&id]).unwrap();
        db.execute("INSERT INTO launches(run_id,start_state,cleanup_confirmed,review_required,log_bytes,evidence_version) VALUES(?1,'uncertain',1,0,123,3)",[&id]).unwrap();
        db.execute(
            "INSERT INTO admissions VALUES('permanent-key',x'abc0',?1)",
            [&id],
        )
        .unwrap();
        db.execute(
            "INSERT INTO acknowledgements VALUES(?1,1,?2)",
            rusqlite::params![acknowledgement_id, original],
        )
        .unwrap();
        db.execute_batch("UPDATE sqlite_sequence SET seq=99 WHERE name='runs'")
            .unwrap();
        let mut store = Store::open(&path).unwrap();
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            10
        );
        let acknowledgement = store.acknowledgement(&acknowledgement_id).unwrap().unwrap();
        assert_eq!(acknowledgement.review.launch.launch_failure, None);
        assert_eq!(acknowledgement.review.launch.capture_setup_failure, None);
        assert_eq!(acknowledgement.review.launch.output_evidence, None);
        let encoded = serde_json::to_value(&acknowledgement.review.launch).unwrap();
        assert!(encoded.get("output_evidence").is_none());
        assert_eq!(acknowledgement.review.runtime_id, "runtime-old");
        assert_eq!(
            store
                .acknowledge(
                    &acknowledgement_id,
                    &acknowledgement.review,
                    &acknowledgement.note
                )
                .unwrap(),
            acknowledgement
        );
        assert_eq!(
            db.query_row("SELECT record FROM acknowledgements", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            original
        );
        assert_eq!(
            db.query_row("SELECT runtime_id FROM runs", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "runtime-old"
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
        assert_eq!(session.record(&id).unwrap().unwrap().launch_failure, None);
        assert_eq!(
            session.run_state(&id).unwrap().unwrap().result,
            Some(RunResult::Interruption)
        );
    }
}
