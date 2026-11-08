use std::{collections::BTreeMap, path::Path, time::Duration};

use hmac::{Hmac, Mac};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use toml::Value;

use crate::config::Config;

mod schema;

type KeyMac = Hmac<Sha256>;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    InvalidKey,
    InvalidInputs,
    InvalidRuntime,
    TooLarge,
    PayloadConflict,
    QueueFull,
    Closed,
    Busy,
    Storage,
    Randomness,
    UnsupportedSchema,
    InvalidTransition,
    InvalidLimits,
    OwnerBusy,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Admission {
    Admitted { run_id: String },
    AlreadyAdmitted { run_id: String },
    HistoryRemoved { run_id: String },
}

#[derive(Debug, PartialEq, Eq)]
pub enum RunPhase {
    Queued,
    Launching,
    Running,
    CleaningUp,
    Completed,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RunResult {
    Cancellation,
    Success,
    Failure,
    Interruption,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RunState {
    pub phase: RunPhase,
    pub result: Option<RunResult>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Cancellation {
    Cancelled,
    AlreadyCancelled,
    NotFound,
    NotQueued,
}

fn read_run_state(connection: &Connection, run_id: &str) -> Result<Option<RunState>, Error> {
    let state: Option<(String, Option<String>, bool, bool, bool)> = connection.query_row(
        "SELECT phase, result, dispatched, EXISTS(SELECT 1 FROM launches WHERE launches.run_id=runs.run_id),
         EXISTS(SELECT 1 FROM launches WHERE launches.run_id=runs.run_id AND cleanup_confirmed=1)
         FROM runs WHERE run_id=?1", [run_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
    ).optional().map_err(storage)?;
    let Some((phase, result, dispatched, has_launch, cleaned)) = state else {
        return Ok(None);
    };
    let result = match result.as_deref() {
        None => None,
        Some("cancellation") => Some(RunResult::Cancellation),
        Some("success") => Some(RunResult::Success),
        Some("failure") => Some(RunResult::Failure),
        Some("interruption") => Some(RunResult::Interruption),
        _ => return Err(Error::Storage),
    };
    if dispatched && result == Some(RunResult::Cancellation) {
        return Err(Error::Storage);
    }
    let phase = match phase.as_str() {
        "queued" if !dispatched && !has_launch && result.is_none() => RunPhase::Queued,
        "completed" if !dispatched && !has_launch && result == Some(RunResult::Cancellation) => {
            RunPhase::Completed
        }
        "launching" if dispatched && has_launch && result.is_none() => RunPhase::Launching,
        "running" if dispatched && has_launch && result.is_none() => RunPhase::Running,
        "cleaning_up" if dispatched && has_launch && result.is_some() => RunPhase::CleaningUp,
        "completed" if dispatched && has_launch && cleaned && result.is_some() => {
            RunPhase::Completed
        }
        _ => return Err(Error::Storage),
    };
    Ok(Some(RunState { phase, result }))
}

/// Durable manual admission and retained lifecycle state.
pub struct Store {
    connection: Connection,
    failed: bool,
}

fn random_bytes() -> Result<[u8; 32], Error> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| Error::Randomness)?;
    Ok(bytes)
}

pub(crate) fn storage(error: rusqlite::Error) -> Error {
    match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
            Error::Busy
        }
        _ => Error::Storage,
    }
}

fn revision(prefix: &[u8], bytes: &[u8]) -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(prefix);
    hash.update(bytes);
    hash.finalize().to_vec()
}

fn put_text(bytes: &mut Vec<u8>, text: &str) -> Result<(), Error> {
    if bytes.len() + 8 + text.len() > 65536 {
        return Err(Error::TooLarge);
    }
    bytes.extend_from_slice(&(text.len() as u64).to_be_bytes());
    bytes.extend_from_slice(text.as_bytes());
    Ok(())
}

fn fingerprint(task_id: &str, values: &BTreeMap<String, Value>) -> Result<Vec<u8>, Error> {
    if values.len() > 256 {
        return Err(Error::TooLarge);
    }
    let mut bytes = b"rustward-request-v1".to_vec();
    put_text(&mut bytes, task_id)?;
    bytes.extend_from_slice(&(values.len() as u64).to_be_bytes());
    for (id, value) in values {
        put_text(&mut bytes, id)?;
        match value {
            Value::String(text) => {
                bytes.push(b's');
                put_text(&mut bytes, text)?;
            }
            Value::Integer(number) => {
                bytes.push(b'i');
                bytes.extend_from_slice(&number.to_be_bytes());
            }
            Value::Boolean(value) => bytes.extend_from_slice(&[b'b', u8::from(*value)]),
            _ => return Err(Error::InvalidInputs),
        }
    }
    if bytes.len() > 65536 {
        return Err(Error::TooLarge);
    }
    Ok(Sha256::digest(bytes).to_vec())
}

fn decode_key(key: &str) -> Result<Vec<u8>, Error> {
    if key.len() != 194
        || key
            .bytes()
            .any(|b| !b.is_ascii_digit() && !(b'a'..=b'f').contains(&b))
    {
        return Err(Error::InvalidKey);
    }
    let bytes = hex::decode(key).map_err(|_| Error::InvalidKey)?;
    if bytes[0] != 1 {
        return Err(Error::InvalidKey);
    }
    Ok(bytes)
}

impl Store {
    pub(crate) fn close(&mut self) {
        self.failed = true;
    }

    /// Open a dedicated database in an existing protected directory.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() || path == Path::new(":memory:") {
            return Err(Error::Storage);
        }
        let mut connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(storage)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(storage)?;
        connection
            .pragma_update(None, "journal_mode", "DELETE")
            .map_err(storage)?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(storage)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let version: i64 = transaction
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(storage)?;
        if version == 0 {
            transaction.execute_batch(
                "CREATE TABLE issuer (id INTEGER PRIMARY KEY CHECK(id=1), generation BLOB NOT NULL, secret BLOB NOT NULL,
                 queue_limit INTEGER NOT NULL CHECK(queue_limit>=0), closed INTEGER NOT NULL);
                 CREATE TABLE runs (sequence INTEGER PRIMARY KEY AUTOINCREMENT, run_id TEXT NOT NULL UNIQUE,
                 task_id TEXT NOT NULL, task_snapshot BLOB NOT NULL, task_revision BLOB NOT NULL,
                 config_revision BLOB NOT NULL, submitted_inputs TEXT NOT NULL, arguments TEXT NOT NULL,
                 runtime_id TEXT NOT NULL, phase TEXT NOT NULL CHECK(phase='queued'));
                 CREATE TABLE admissions (request_key TEXT PRIMARY KEY, fingerprint BLOB NOT NULL, run_id TEXT NOT NULL UNIQUE);
                 PRAGMA user_version=1;"
            ).map_err(storage)?;
            transaction
                .execute(
                    "INSERT INTO issuer VALUES (1, ?1, ?2, 100, 0)",
                    params![random_bytes()?.as_slice(), random_bytes()?.as_slice()],
                )
                .map_err(storage)?;
        } else if !(1..=3).contains(&version) {
            return Err(Error::UnsupportedSchema);
        }
        if version == 0 || version == 1 {
            transaction
                .execute_batch(
                    "CREATE TABLE runs_v2 (
                    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                    run_id TEXT NOT NULL UNIQUE,
                    task_id TEXT NOT NULL,
                    task_snapshot BLOB NOT NULL,
                    task_revision BLOB NOT NULL,
                    config_revision BLOB NOT NULL,
                    submitted_inputs TEXT NOT NULL,
                    arguments TEXT NOT NULL,
                    runtime_id TEXT NOT NULL,
                    phase TEXT NOT NULL,
                    result TEXT,
                    CHECK ((phase='queued' AND result IS NULL) OR
                           (phase='completed' AND result IS NOT NULL AND result='cancellation'))
                 );
                 INSERT INTO runs_v2 SELECT *, NULL FROM runs;
                 INSERT INTO sqlite_sequence (name, seq)
                    SELECT 'runs_v2', seq FROM sqlite_sequence WHERE name='runs'
                    AND NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name='runs_v2');
                 UPDATE sqlite_sequence SET seq=max(seq, COALESCE(
                    (SELECT seq FROM sqlite_sequence WHERE name='runs'), 0))
                    WHERE name='runs_v2';
                 DROP TABLE runs;
                 ALTER TABLE runs_v2 RENAME TO runs;
                 PRAGMA user_version=2;",
                )
                .map_err(storage)?;
        }
        if version < 3 {
            schema::migrate(&transaction)?;
        }
        transaction.commit().map_err(storage)?;
        Ok(Self {
            connection,
            failed: false,
        })
    }

    pub(crate) fn perform<T>(
        &mut self,
        action: impl FnOnce(&mut Connection) -> Result<T, Error>,
    ) -> Result<T, Error> {
        if self.failed {
            return Err(Error::Closed);
        }
        let result = action(&mut self.connection);
        if matches!(result, Err(Error::Storage)) {
            self.failed = true;
        }
        result
    }

    /// Issue an unbound key without inserting a run or admission record.
    pub fn issue_key(&mut self) -> Result<String, Error> {
        self.perform(|connection| {
            let (generation, secret): (Vec<u8>, Vec<u8>) = connection
                .query_row(
                    "SELECT generation, secret FROM issuer WHERE id=1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(storage)?;
            if generation.len() != 32 || secret.len() != 32 {
                return Err(Error::Storage);
            }
            let mut bytes = vec![1];
            bytes.extend_from_slice(&generation);
            bytes.extend_from_slice(&random_bytes()?);
            let mut mac = KeyMac::new_from_slice(&secret).unwrap();
            mac.update(&bytes);
            bytes.extend_from_slice(&mac.finalize().into_bytes());
            Ok(hex::encode(bytes))
        })
    }

    pub fn set_queue_limit(&mut self, limit: u32) -> Result<(), Error> {
        self.perform(|connection| {
            connection
                .execute("UPDATE issuer SET queue_limit=?1 WHERE id=1", [limit])
                .map_err(storage)?;
            Ok(())
        })
    }

    pub fn set_closed(&mut self, closed: bool) -> Result<(), Error> {
        self.perform(|connection| {
            connection
                .execute("UPDATE issuer SET closed=?1 WHERE id=1", [closed])
                .map_err(storage)?;
            Ok(())
        })
    }

    /// Read retained lifecycle state without changing the run.
    pub fn run_state(&mut self, run_id: &str) -> Result<Option<RunState>, Error> {
        self.perform(|connection| read_run_state(connection, run_id))
    }

    /// Complete a queued run with a cancellation result and preserve its admission.
    pub fn cancel_queued(&mut self, run_id: &str) -> Result<Cancellation, Error> {
        self.perform(|connection| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage)?;
            match read_run_state(&transaction, run_id)? {
                None => return Ok(Cancellation::NotFound),
                Some(RunState {
                    phase: RunPhase::Completed,
                    result: Some(RunResult::Cancellation),
                }) => {
                    return Ok(Cancellation::AlreadyCancelled);
                }
                Some(RunState {
                    phase: RunPhase::Queued,
                    ..
                }) => {}
                Some(_) => return Ok(Cancellation::NotQueued),
            }
            let changed = transaction
                .execute(
                    "UPDATE runs SET phase='completed', result='cancellation'
                 WHERE run_id=?1 AND phase='queued' AND result IS NULL",
                    [run_id],
                )
                .map_err(storage)?;
            if changed != 1 {
                return Err(Error::Storage);
            }
            transaction.commit().map_err(storage)?;
            Ok(Cancellation::Cancelled)
        })
    }

    /// Admit a new request or return its original identity before current validation.
    pub fn admit(
        &mut self,
        key: &str,
        config: &Config,
        task_id: &str,
        submitted_values: &BTreeMap<String, Value>,
        runtime_id: &str,
    ) -> Result<Admission, Error> {
        self.perform(|connection| {
            let key_bytes = decode_key(key)?;
            let fingerprint = fingerprint(task_id, submitted_values)?;
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage)?;
            let existing: Option<(Vec<u8>, String, bool)> = transaction.query_row(
                "SELECT a.fingerprint, a.run_id, EXISTS(SELECT 1 FROM runs r WHERE r.run_id=a.run_id)
                 FROM admissions a WHERE request_key=?1", [key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            ).optional().map_err(storage)?;
            if let Some((original, run_id, history_exists)) = existing {
                if original != fingerprint {
                    return Err(Error::PayloadConflict);
                }
                return Ok(if history_exists {
                    Admission::AlreadyAdmitted { run_id }
                } else {
                    Admission::HistoryRemoved { run_id }
                });
            }
            let (generation, secret, limit, closed): (Vec<u8>, Vec<u8>, u32, bool) = transaction.query_row(
                "SELECT generation, secret, queue_limit, closed FROM issuer WHERE id=1", [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            ).map_err(storage)?;
            if generation.len() != 32 || secret.len() != 32 {
                return Err(Error::Storage);
            }
            if key_bytes[1..33] != generation {
                return Err(Error::InvalidKey);
            }
            let mut mac = KeyMac::new_from_slice(&secret).unwrap();
            mac.update(&key_bytes[..65]);
            mac.verify_slice(&key_bytes[65..]).map_err(|_| Error::InvalidKey)?;
            if closed {
                return Err(Error::Closed);
            }
            if runtime_id.is_empty() || runtime_id.len() > 1024 {
                return Err(Error::InvalidRuntime);
            }
            let arguments = config.arguments(task_id, submitted_values).map_err(|_| Error::InvalidInputs)?;
            let (snapshot, config_snapshot) = config.admission_snapshot(task_id);
            let arguments = serde_json::to_string(&arguments).unwrap();
            if snapshot.len() + arguments.len() > 1024 * 1024 {
                return Err(Error::TooLarge);
            }
            let count: u32 = transaction.query_row("SELECT count(*) FROM runs WHERE phase='queued'", [], |row| row.get(0)).map_err(storage)?;
            if count >= limit {
                return Err(Error::QueueFull);
            }
            let run_id = hex::encode(random_bytes()?);
            transaction.execute(
                "INSERT INTO runs (run_id, task_id, task_snapshot, task_revision, config_revision, submitted_inputs, arguments, runtime_id, phase)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'queued')",
                params![run_id, task_id, snapshot, revision(b"rustward-definition-v1", &snapshot), revision(b"rustward-configuration-v2", &config_snapshot), serde_json::to_string(submitted_values).unwrap(), arguments, runtime_id]
            ).map_err(storage)?;
            transaction.execute("INSERT INTO admissions VALUES (?1, ?2, ?3)", params![key, fingerprint, run_id]).map_err(storage)?;
            transaction.commit().map_err(storage)?;
            Ok(Admission::Admitted { run_id })
        })
    }
}
