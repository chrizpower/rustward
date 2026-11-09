//! Durable launch protocol. Real execution requires the integration gates in
//! docs/temporary/dispatch.md. Tests use a fake supervisor only.
use std::{
    collections::BTreeMap,
    fs::File,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use rusqlite::{OptionalExtension, TransactionBehavior, params};
use rustix::fs::{FlockOperation, flock};
use serde::Deserialize;

use crate::{
    admission::{Error, RunState, Store, storage, utc_ms},
    supervision::{Launch, Output, RunId, StartOutcome, Supervisor},
};

mod evidence;
mod preflight;
mod runtime;
pub use preflight::LaunchFailure;
pub use runtime::{PrepareError, PreparedPermit};
mod stopping;
pub use evidence::Record;
pub(crate) use evidence::read as read_record;
pub use stopping::{StopDecision, StopReason, Times};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub active_runs: u32,
    pub log_bytes_per_run: u64,
    pub retained_log_bytes: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            active_runs: 1,
            log_bytes_per_run: 10 * 1024 * 1024,
            retained_log_bytes: 1024 * 1024 * 1024,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Block {
    Paused,
    ReviewRequired,
    ExecutionCapacity,
    LogCapacity,
    Empty,
}

pub enum Claim {
    Ready(Permit),
    Blocked(Block),
}

/// One-use launch permission. Dropping it never returns the run to the queue.
pub struct Permit {
    owner: Arc<File>,
    launch: Launch,
    runtime_id: String,
}
impl Permit {
    pub fn launch(&self) -> &Launch {
        &self.launch
    }
    pub fn runtime_id(&self) -> &str {
        &self.runtime_id
    }
}

pub struct Session {
    store: Store,
    owner: Arc<File>,
    limits: Limits,
    safety: crate::safety::Safety,
    active: BTreeMap<String, stopping::Active>,
}

#[derive(Deserialize)]
struct Command {
    executable: String,
    working_directory: String,
    #[serde(default)]
    timeout_seconds: Option<u32>,
    #[serde(default = "legacy_grace")]
    cleanup_grace_seconds: u32,
}
fn legacy_grace() -> u32 {
    30
}

impl Session {
    /// Acquire sole dispatch ownership and recover unfinished launch intents.
    /// Keep the database inode in place while any session or permit exists.
    pub fn open(path: impl AsRef<Path>, limits: Limits) -> Result<Self, Error> {
        Self::open_with_safety(path, limits, crate::safety::Safety::default())
    }

    pub fn open_with_safety(
        path: impl AsRef<Path>,
        limits: Limits,
        safety: crate::safety::Safety,
    ) -> Result<Self, Error> {
        if limits.retained_log_bytes > i64::MAX as u64 || limits.log_bytes_per_run > i64::MAX as u64
        {
            return Err(Error::InvalidLimits);
        }
        let path = path.as_ref();
        let mut store = Store::open_with_safety(path, safety.clone())?;
        let owner = File::open(path).map_err(|_| {
            safety.fail();
            Error::Storage
        })?;
        flock(&owner, FlockOperation::NonBlockingLockExclusive).map_err(|e| {
            if e == rustix::io::Errno::WOULDBLOCK {
                Error::OwnerBusy
            } else {
                safety.fail();
                Error::Storage
            }
        })?;
        store.perform(|connection| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage)?;
            transaction
                .execute_batch(
                    "UPDATE launches SET review_required=1 WHERE run_id IN
                (SELECT run_id FROM runs WHERE dispatched=1 AND phase!='completed');
                UPDATE runs SET phase='cleaning_up', result=COALESCE(result,'interruption')
                WHERE dispatched=1 AND phase!='completed';",
                )
                .map_err(storage)?;
            let invalid: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM runs LEFT JOIN launches USING(run_id)
                WHERE dispatched=1 AND phase!='completed' AND
                (phase!='cleaning_up' OR result IS NULL OR COALESCE(review_required,0)!=1))",
                    [],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            if invalid {
                return Err(Error::Storage);
            }
            transaction.commit().map_err(storage)
        })?;
        let mut active = BTreeMap::new();
        store.perform(|connection| {
            let mut statement = connection.prepare("SELECT run_id,task_snapshot FROM runs WHERE dispatched=1 AND phase!='completed'").map_err(storage)?;
            let rows = statement.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))).map_err(storage)?;
            for row in rows {
                let (id, snapshot) = row.map_err(storage)?;
                let command: Command = serde_json::from_slice(&snapshot).map_err(|_| Error::Storage)?;
                active.insert(id, stopping::Active::recovered(Duration::from_secs(command.cleanup_grace_seconds.into())));
            }
            Ok(())
        })?;
        let mut session = Self {
            store,
            owner: Arc::new(owner),
            limits,
            safety,
            active,
        };
        for id in session.active.keys().cloned().collect::<Vec<_>>() {
            let record = session.record(&id)?.ok_or(Error::Storage)?;
            session.retain_record(&id, &record);
        }
        Ok(session)
    }

    pub fn set_paused(&mut self, paused: bool) -> Result<(), Error> {
        self.store.perform(|connection| {
            let changed = connection
                .execute("UPDATE dispatch_control SET paused=?1 WHERE id=1", [paused])
                .map_err(storage)?;
            if changed != 1 {
                return Err(Error::Storage);
            }
            Ok(())
        })
    }

    pub fn run_state(&mut self, run_id: &str) -> Result<Option<RunState>, Error> {
        self.store.run_state(run_id)
    }

    /// Reserve a slot and full log allowance for the oldest queued run.
    pub fn claim_next(&mut self) -> Result<Claim, Error> {
        if self.safety.failed() {
            return Err(Error::Closed);
        }
        let limits = self.limits;
        let owner = Arc::clone(&self.owner);
        let mut timing = None;
        let claimed = self.store.perform(|connection| {
            let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            let paused: bool = transaction.query_row("SELECT paused FROM dispatch_control WHERE id=1", [], |r| r.get(0)).map_err(storage)?;
            if paused { return Ok(Claim::Blocked(Block::Paused)); }
            let review: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM launches WHERE review_required=1)", [], |r| r.get(0)).map_err(storage)?;
            if review { return Ok(Claim::Blocked(Block::ReviewRequired)); }
            let active: u32 = transaction.query_row("SELECT count(*) FROM runs WHERE dispatched=1 AND phase!='completed'", [], |r| r.get(0)).map_err(storage)?;
            if active >= limits.active_runs { return Ok(Claim::Blocked(Block::ExecutionCapacity)); }
            let used: i64 = transaction.query_row("SELECT COALESCE(sum(log_bytes),0) FROM launches", [], |r| r.get(0)).map_err(storage)?;
            let used = u64::try_from(used).map_err(|_| Error::Storage)?;
            if used > limits.retained_log_bytes || limits.log_bytes_per_run > limits.retained_log_bytes - used {
                return Ok(Claim::Blocked(Block::LogCapacity));
            }
            let row: Option<(String, Vec<u8>, String, String)> = transaction.query_row(
                "SELECT run_id, task_snapshot, arguments, runtime_id FROM runs WHERE phase='queued' ORDER BY sequence LIMIT 1",
                [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).optional().map_err(storage)?;
            let Some((run_id, snapshot, arguments, runtime_id)) = row else { return Ok(Claim::Blocked(Block::Empty)); };
            let command: Command = serde_json::from_slice(&snapshot).map_err(|_| Error::Storage)?;
            let now = Instant::now();
            timing = Some((run_id.clone(), stopping::Active::new(now, command.timeout_seconds, command.cleanup_grace_seconds)?));
            let arguments: Vec<String> = serde_json::from_str(&arguments).map_err(|_| Error::Storage)?;
            let launch = Launch { run_id: RunId::new(&run_id).map_err(|_| Error::Storage)?, executable: command.executable,
                working_directory: command.working_directory, arguments, cleanup_grace: Duration::from_secs(command.cleanup_grace_seconds.into()) };
            let changed = transaction.execute("UPDATE runs SET phase='launching', dispatched=1,dispatched_at=?2 WHERE run_id=?1 AND phase='queued'", params![run_id,utc_ms()?]).map_err(storage)?;
            if changed != 1 { return Err(Error::Storage); }
            transaction.execute("INSERT INTO launches (run_id,start_state,log_bytes) VALUES (?1,'intent',?2)", params![run_id, limits.log_bytes_per_run as i64]).map_err(storage)?;
            transaction.commit().map_err(storage)?;
            Ok(Claim::Ready(Permit { owner, launch, runtime_id }))
        });
        if matches!(&claimed, Ok(Claim::Ready(_))) {
            let (id, active) = timing.unwrap();
            self.active.insert(id, active);
        }
        claimed
    }

    /// Call start once after commit. The caller must first verify the retained
    /// runtime and prepare bounded output capture. No retry follows any reply.
    pub fn start(
        &mut self,
        permit: Permit,
        supervisor: &mut impl Supervisor,
        output: Output<'_>,
    ) -> Result<StartOutcome, Error> {
        if self.safety.failed() {
            return Err(Error::Closed);
        }
        if !Arc::ptr_eq(&permit.owner, &self.owner) {
            return Err(Error::InvalidTransition);
        }
        let run_id = permit.launch.run_id.as_str();
        if self
            .active
            .get(run_id)
            .is_some_and(|active| active.expired(Instant::now()))
        {
            self.decide_stop(run_id, StopReason::Timeout)?;
            return Err(Error::Closed);
        }
        self.store.perform(|connection| {
            let review: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM launches WHERE review_required=1)",
                    [],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            if review {
                return Err(Error::Closed);
            }
            let ready: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM runs JOIN launches USING(run_id)
                WHERE run_id=?1 AND phase='launching' AND start_state='intent')",
                    [run_id],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            if !ready {
                return Err(Error::InvalidTransition);
            }
            Ok(())
        })?;
        self.active
            .get_mut(run_id)
            .ok_or(Error::InvalidTransition)?
            .start_called = true;
        let outcome = supervisor
            .start(&permit.launch, output)
            .unwrap_or(StartOutcome::Uncertain);
        let saved = self.store.perform(|connection| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage)?;
            let (state, result, cleanup, review) = match outcome {
                StartOutcome::Accepted => ("accepted", None, false, false),
                StartOutcome::Rejected => ("rejected", Some("failure"), true, false),
                StartOutcome::Conflict => ("conflict", Some("interruption"), false, true),
                StartOutcome::Uncertain => ("uncertain", Some("interruption"), false, true),
            };
            let changed = transaction
                .execute(
                    "UPDATE launches SET start_state=?1,cleanup_confirmed=?2,review_required=?3
                WHERE run_id=?4 AND start_state='intent'",
                    params![state, cleanup, review, run_id],
                )
                .map_err(storage)?;
            if changed != 1 {
                return Err(Error::Storage);
            }
            if let Some(result) = result {
                let changed = transaction
                    .execute(
                        "UPDATE runs SET phase='cleaning_up',result=?1 WHERE run_id=?2",
                        params![result, run_id],
                    )
                    .map_err(storage)?;
                if changed != 1 {
                    return Err(Error::Storage);
                }
            }
            transaction.commit().map_err(storage)
        });
        if saved.is_err() {
            self.store.close();
            self.safety.fail();
        }
        saved?;
        if outcome == StartOutcome::Rejected {
            self.active.get_mut(run_id).unwrap().cleaned = true;
        }
        Ok(outcome)
    }
}
