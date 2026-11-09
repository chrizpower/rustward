use super::{Error, storage};
use rusqlite::Transaction;

pub(super) fn migrate(transaction: &Transaction<'_>) -> Result<(), Error> {
    transaction.execute_batch("CREATE TABLE runs_v3 (
        sequence INTEGER PRIMARY KEY AUTOINCREMENT, run_id TEXT NOT NULL UNIQUE,
        task_id TEXT NOT NULL, task_snapshot BLOB NOT NULL, task_revision BLOB NOT NULL,
        config_revision BLOB NOT NULL, submitted_inputs TEXT NOT NULL, arguments TEXT NOT NULL,
        runtime_id TEXT NOT NULL, phase TEXT NOT NULL, result TEXT,
        dispatched INTEGER NOT NULL DEFAULT 0 CHECK(dispatched IN (0,1)),
        CHECK ((dispatched=0 AND ((phase='queued' AND result IS NULL) OR
               (phase='completed' AND result IS NOT NULL AND result='cancellation'))) OR
               (dispatched=1 AND ((phase IN ('launching','running') AND result IS NULL) OR
               (phase IN ('cleaning_up','completed') AND result IS NOT NULL AND
                result IN ('success','failure','interruption')))))
    );
    INSERT INTO runs_v3 SELECT *, 0 FROM runs;
    INSERT INTO sqlite_sequence (name,seq) SELECT 'runs_v3',seq FROM sqlite_sequence
        WHERE name='runs' AND NOT EXISTS(SELECT 1 FROM sqlite_sequence WHERE name='runs_v3');
    UPDATE sqlite_sequence SET seq=max(seq,COALESCE((SELECT seq FROM sqlite_sequence WHERE name='runs'),0))
        WHERE name='runs_v3';
    DROP TABLE runs;
    ALTER TABLE runs_v3 RENAME TO runs;
    CREATE TABLE dispatch_control (id INTEGER PRIMARY KEY CHECK(id=1), paused INTEGER NOT NULL CHECK(paused IN (0,1)));
    INSERT INTO dispatch_control VALUES (1,1);
    CREATE TABLE launches (
        run_id TEXT PRIMARY KEY,
        start_state TEXT NOT NULL CHECK(start_state IN ('intent','accepted','rejected','conflict','uncertain')),
        invocation BLOB, control_group TEXT,
        exit_kind TEXT, exit_value INTEGER,
        cleanup_confirmed INTEGER NOT NULL DEFAULT 0 CHECK(cleanup_confirmed IN (0,1)),
        review_required INTEGER NOT NULL DEFAULT 0 CHECK(review_required IN (0,1)),
        log_bytes INTEGER NOT NULL CHECK(log_bytes>=0),
        CHECK ((exit_kind IS NULL AND exit_value IS NULL) OR
               (exit_kind IS NOT NULL AND exit_kind IN ('code','signal') AND exit_value IS NOT NULL AND
                exit_value BETWEEN -2147483648 AND 2147483647)),
        CHECK((invocation IS NULL AND control_group IS NULL) OR
              (invocation IS NOT NULL AND length(invocation)=16 AND control_group IS NOT NULL))
    );
    PRAGMA user_version=3;").map_err(storage)
}

pub(super) fn migrate_stopping(transaction: &Transaction<'_>) -> Result<(), Error> {
    transaction.execute_batch("CREATE TABLE runs_v4 (
        sequence INTEGER PRIMARY KEY AUTOINCREMENT, run_id TEXT NOT NULL UNIQUE,
        task_id TEXT NOT NULL, task_snapshot BLOB NOT NULL, task_revision BLOB NOT NULL,
        config_revision BLOB NOT NULL, submitted_inputs TEXT NOT NULL, arguments TEXT NOT NULL,
        runtime_id TEXT NOT NULL, phase TEXT NOT NULL, result TEXT,
        dispatched INTEGER NOT NULL DEFAULT 0 CHECK(dispatched IN (0,1)),
        dispatched_at INTEGER, completed_at INTEGER,
        CHECK ((dispatched=0 AND ((phase='queued' AND result IS NULL) OR
               (phase='completed' AND result IS NOT NULL AND result='cancellation'))) OR
               (dispatched=1 AND ((phase IN ('launching','running') AND result IS NULL) OR
               (phase IN ('cleaning_up','completed') AND result IS NOT NULL AND
                result IN ('success','failure','interruption','cancellation','timeout')))))
    );
    INSERT INTO runs_v4 SELECT *, NULL, NULL FROM runs;
    INSERT INTO sqlite_sequence (name,seq) SELECT 'runs_v4',seq FROM sqlite_sequence
        WHERE name='runs' AND NOT EXISTS(SELECT 1 FROM sqlite_sequence WHERE name='runs_v4');
    UPDATE sqlite_sequence SET seq=max(seq,COALESCE((SELECT seq FROM sqlite_sequence WHERE name='runs'),0))
        WHERE name='runs_v4';
    DROP TABLE runs;
    ALTER TABLE runs_v4 RENAME TO runs;
    ALTER TABLE launches ADD COLUMN stop_reason TEXT CHECK(stop_reason IN ('cancellation','timeout','capture','storage'));
    ALTER TABLE launches ADD COLUMN output_failed INTEGER NOT NULL DEFAULT 0 CHECK(output_failed IN (0,1));
    PRAGMA user_version=4;").map_err(storage)
}

pub(super) fn migrate_review(transaction: &Transaction<'_>) -> Result<(), Error> {
    transaction.execute_batch("ALTER TABLE launches ADD COLUMN evidence_version INTEGER NOT NULL DEFAULT 0
        CHECK(typeof(evidence_version)='integer' AND evidence_version>=0);
    UPDATE launches SET evidence_version=review_required;
    CREATE TABLE acknowledgements (
        acknowledgement_id TEXT PRIMARY KEY NOT NULL,
        encoding INTEGER NOT NULL CHECK(encoding=1),
        record TEXT NOT NULL
    );
    CREATE TRIGGER launch_evidence_insert AFTER INSERT ON launches
    WHEN NEW.review_required=1 BEGIN
        UPDATE launches SET evidence_version=evidence_version+1 WHERE run_id=NEW.run_id;
    END;
    CREATE TRIGGER launch_evidence_update AFTER UPDATE OF start_state,invocation,control_group,
        exit_kind,exit_value,cleanup_confirmed,review_required,log_bytes,stop_reason,output_failed ON launches
    WHEN NEW.review_required=1 BEGIN
        UPDATE launches SET evidence_version=evidence_version+1 WHERE run_id=NEW.run_id;
    END;
    CREATE TRIGGER run_evidence_update AFTER UPDATE OF phase,result,dispatched_at,completed_at ON runs
    WHEN EXISTS(SELECT 1 FROM launches WHERE run_id=NEW.run_id AND review_required=1) BEGIN
        UPDATE launches SET evidence_version=evidence_version+1 WHERE run_id=NEW.run_id;
    END;
    PRAGMA user_version=5;").map_err(storage)
}

pub(super) fn migrate_runtime(transaction: &Transaction<'_>) -> Result<(), Error> {
    transaction
        .execute_batch(
            "ALTER TABLE launches ADD COLUMN launch_failure TEXT
        CHECK(launch_failure IN ('runtime_missing','runtime_invalid','runtime_changed',
            'runtime_unsafe','runtime_command','work_directory','work_storage'));
    CREATE TRIGGER launch_failure_evidence AFTER UPDATE OF launch_failure ON launches
    WHEN NEW.review_required=1 BEGIN
        UPDATE launches SET evidence_version=evidence_version+1 WHERE run_id=NEW.run_id;
    END;
    PRAGMA user_version=6;",
        )
        .map_err(storage)
}
