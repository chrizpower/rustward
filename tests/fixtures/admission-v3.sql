CREATE TABLE issuer (id INTEGER PRIMARY KEY CHECK(id=1), generation BLOB NOT NULL, secret BLOB NOT NULL,
queue_limit INTEGER NOT NULL CHECK(queue_limit>=0), closed INTEGER NOT NULL);
CREATE TABLE runs (sequence INTEGER PRIMARY KEY AUTOINCREMENT, run_id TEXT NOT NULL UNIQUE,
task_id TEXT NOT NULL, task_snapshot BLOB NOT NULL, task_revision BLOB NOT NULL,
config_revision BLOB NOT NULL, submitted_inputs TEXT NOT NULL, arguments TEXT NOT NULL,
runtime_id TEXT NOT NULL, phase TEXT NOT NULL, result TEXT,
CHECK ((phase='queued' AND result IS NULL) OR
       (phase='completed' AND result IS NOT NULL AND result='cancellation')));
CREATE TABLE admissions (request_key TEXT PRIMARY KEY, fingerprint BLOB NOT NULL, run_id TEXT NOT NULL UNIQUE);
PRAGMA user_version=2;

CREATE TABLE runs_v3 (
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
    PRAGMA user_version=3;
