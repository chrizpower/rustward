CREATE TABLE issuer (id INTEGER PRIMARY KEY CHECK(id=1), generation BLOB NOT NULL, secret BLOB NOT NULL,
queue_limit INTEGER NOT NULL CHECK(queue_limit>=0), closed INTEGER NOT NULL);
CREATE TABLE runs (sequence INTEGER PRIMARY KEY AUTOINCREMENT, run_id TEXT NOT NULL UNIQUE,
task_id TEXT NOT NULL, task_snapshot BLOB NOT NULL, task_revision BLOB NOT NULL,
config_revision BLOB NOT NULL, submitted_inputs TEXT NOT NULL, arguments TEXT NOT NULL,
runtime_id TEXT NOT NULL, phase TEXT NOT NULL CHECK(phase='queued'));
CREATE TABLE admissions (request_key TEXT PRIMARY KEY, fingerprint BLOB NOT NULL, run_id TEXT NOT NULL UNIQUE);
PRAGMA user_version=1;
