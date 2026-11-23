#[cfg(any(target_os = "linux", test))]
use chrono::{DateTime, Utc};
#[cfg(any(target_os = "linux", test))]
use croner::Cron;
use rusqlite::Transaction;
#[cfg(any(target_os = "linux", test))]
use rusqlite::{OptionalExtension, TransactionBehavior, params};
#[cfg(any(target_os = "linux", test))]
use serde::Serialize;

use super::{Error, storage};
#[cfg(any(target_os = "linux", test))]
use super::{Store, fingerprint, insert_run, revision};
#[cfg(any(target_os = "linux", test))]
use crate::config::Config;

pub(super) fn migrate(transaction: &Transaction<'_>) -> Result<(), Error> {
    transaction.execute_batch(
        "CREATE TABLE schedule_state (
             schedule_id TEXT PRIMARY KEY, active INTEGER NOT NULL,
             paused INTEGER NOT NULL, checkpoint_ms INTEGER NOT NULL, revision BLOB NOT NULL
         );
         CREATE TABLE schedule_control (id INTEGER PRIMARY KEY CHECK(id=1), paused INTEGER NOT NULL);
         INSERT INTO schedule_control VALUES (1,0);
         CREATE TABLE occurrences (
             schedule_id TEXT NOT NULL REFERENCES schedule_state(schedule_id), due_ms INTEGER NOT NULL,
             revision BLOB NOT NULL, decision TEXT NOT NULL, run_id TEXT UNIQUE,
             PRIMARY KEY(schedule_id,due_ms)
         );
         CREATE TABLE schedule_gaps (
             id INTEGER PRIMARY KEY, schedule_id TEXT NOT NULL REFERENCES schedule_state(schedule_id),
             after_ms INTEGER, through_ms INTEGER NOT NULL, reason TEXT NOT NULL, missed_count INTEGER,
             UNIQUE(schedule_id,after_ms,through_ms,reason)
         );
         CREATE INDEX schedule_gap_history ON schedule_gaps(schedule_id,id);
         CREATE INDEX runs_queued ON runs(sequence,run_id) WHERE phase='queued';
         PRAGMA user_version=8;"
    ).map_err(storage)
}

#[cfg(any(target_os = "linux", test))]
fn gap(
    transaction: &Transaction<'_>,
    id: &str,
    after: Option<i64>,
    through: i64,
    reason: &str,
    missed_count: Option<u32>,
) -> Result<bool, Error> {
    let inserted = transaction.execute(
        "INSERT INTO schedule_gaps(schedule_id,after_ms,through_ms,reason,missed_count) VALUES (?1,?2,?3,?4,?5)
         ON CONFLICT(schedule_id,after_ms,through_ms,reason) DO NOTHING",
        params![id, after, through, reason, missed_count],
    ).map_err(storage)?;
    Ok(inserted != 0)
}

#[cfg(any(target_os = "linux", test))]
fn missed(matcher: &Cron, after: i64, through: i64) -> Result<Option<u32>, Error> {
    // Keep outage evaluation bounded even after long downtime.
    if through - after > 24 * 60 * 60_000 {
        return Ok(None);
    }
    let mut count = 0;
    for minute in ((after + 60_000)..=through).step_by(60_000) {
        let time = DateTime::<Utc>::from_timestamp_millis(minute).ok_or(Error::Storage)?;
        count += u32::from(
            matcher
                .is_time_matching(&time)
                .map_err(|_| Error::Storage)?,
        );
    }
    Ok(Some(count))
}

#[cfg(any(target_os = "linux", test))]
#[derive(Serialize)]
pub(crate) struct Occurrence {
    due_ms: i64,
    revision: String,
    decision: String,
    run_id: Option<String>,
}

#[cfg(any(target_os = "linux", test))]
#[derive(Serialize)]
pub(crate) struct Gap {
    id: i64,
    after_ms: Option<i64>,
    through_ms: i64,
    reason: String,
    missed_count: Option<u32>,
}

#[cfg(any(target_os = "linux", test))]
impl Store {
    pub(crate) fn activate_schedules(
        &mut self,
        config: &Config,
        matchers: &[(Cron, Vec<u8>)],
        now: i64,
    ) -> Result<(), Error> {
        #[cfg(target_os = "linux")]
        let telemetry = self.safety.telemetry().cloned();
        #[cfg(target_os = "linux")]
        let mut gaps = Vec::new();
        self.perform(|connection| {
            let tx = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage)?;
            let active = {
                let mut statement = tx
                    .prepare("SELECT schedule_id FROM schedule_state WHERE active=1")
                    .map_err(storage)?;
                statement
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(storage)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(storage)?
            };
            for id in active {
                if config
                    .schedules()
                    .binary_search_by(|schedule| schedule.id().cmp(&id))
                    .is_err()
                {
                    tx.execute(
                        "UPDATE schedule_state SET active=0 WHERE schedule_id=?1",
                        [&id],
                    )
                    .map_err(storage)?;
                }
            }
            for (schedule, (matcher, revision)) in config.schedules().iter().zip(matchers) {
                let id = schedule.id();
                let previous: Option<(bool, i64, Vec<u8>)> = tx
                    .query_row(
                        "SELECT active,checkpoint_ms,revision FROM schedule_state WHERE schedule_id=?1",
                        [id],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()
                    .map_err(storage)?;
                tx.execute(
                    "INSERT INTO schedule_state VALUES (?1,1,0,?2,?3)
                     ON CONFLICT(schedule_id) DO UPDATE SET active=1,
                     paused=CASE WHEN active=1 THEN paused ELSE 0 END,
                     checkpoint_ms=max(checkpoint_ms,excluded.checkpoint_ms),revision=excluded.revision",
                    params![id, now, revision],
                )
                .map_err(storage)?;
                let (after, through, reason, count) = match previous {
                    None => (None, now, "first_installation", None),
                    Some((active, prior, old_revision)) if prior < now => {
                        let count = if active && old_revision == *revision { missed(matcher, prior, now)? } else { None };
                        (Some(prior), now, "activation", count)
                    }
                    Some((_, prior, _)) if prior > now => (Some(now), prior, "clock_change", None),
                    _ => continue,
                };
                let _inserted = gap(&tx, id, after, through, reason, count)?;
                #[cfg(target_os = "linux")]
                if _inserted && telemetry.is_some() {
                    gaps.push((id, reason, count));
                }
            }
            tx.commit().map_err(storage)
        })?;
        #[cfg(target_os = "linux")]
        if let Some(telemetry) = telemetry {
            for (id, reason, count) in gaps {
                telemetry.gap(id, reason, count);
            }
        }
        Ok(())
    }

    pub(crate) fn evaluate_schedules(
        &mut self,
        config: &Config,
        matchers: &[(Cron, Vec<u8>)],
        runtime_id: &str,
        minute: i64,
        clock_changed: bool,
    ) -> Result<(), Error> {
        let safety = self.safety.clone();
        #[cfg(target_os = "linux")]
        let mut events = Vec::new();
        #[cfg(target_os = "linux")]
        let mut gaps = Vec::new();
        self.perform(|connection| {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            let (closed, limit): (bool, u32) = tx.query_row(
                "SELECT closed,queue_limit FROM issuer WHERE id=1", [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).map_err(storage)?;
            let paused: bool = tx.query_row("SELECT paused FROM schedule_control WHERE id=1", [], |row| row.get(0)).map_err(storage)?;
            let time = DateTime::<Utc>::from_timestamp_millis(minute).ok_or(Error::Storage)?;
            for (schedule, (matcher, revision)) in config.schedules().iter().zip(matchers) {
                let id = schedule.id();
                let (checkpoint, schedule_paused): (i64, bool) = tx.query_row(
                    "SELECT checkpoint_ms,paused FROM schedule_state WHERE schedule_id=?1 AND active=1",
                    [id], |row| Ok((row.get(0)?, row.get(1)?)),
                ).map_err(storage)?;
                if clock_changed || minute - checkpoint > 60_000 {
                    let reason = if clock_changed { "clock_change" } else { "evaluation_gap" };
                    let count = if clock_changed { None } else { missed(matcher, checkpoint, minute)? };
                    let _inserted = gap(&tx, id, Some(checkpoint.min(minute)), checkpoint.max(minute), reason, count)?;
                    #[cfg(target_os = "linux")]
                    if _inserted && safety.telemetry().is_some() {
                        gaps.push((id, reason, count));
                    }
                } else if minute > checkpoint && matcher.is_time_matching(&time).map_err(|_| Error::Storage)? {
                    let queued: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM runs CROSS JOIN occurrences USING(run_id)
                         WHERE phase='queued' AND schedule_id=?1)",
                        [id], |row| row.get(0),
                    ).map_err(storage)?;
                    let (decision, run) = if paused || schedule_paused {
                        ("paused", None)
                    } else if closed || safety.failed() {
                        ("admission_closed", None)
                    } else if queued {
                        ("schedule_queued", None)
                    } else {
                        let admitted = fingerprint(schedule.task(), schedule.inputs()).and_then(|_| {
                            insert_run(&tx, config, schedule.task(), schedule.inputs(), runtime_id, limit)
                        });
                        match admitted {
                            Ok(run) => ("admitted", Some(run)),
                            Err(Error::QueueFull) => ("queue_full", None),
                            Err(Error::TooLarge) => ("too_large", None),
                            Err(error) => return Err(error),
                        }
                    };
                    tx.execute(
                        "INSERT INTO occurrences VALUES (?1,?2,?3,?4,?5)",
                        params![id, minute, revision, decision, run],
                    ).map_err(storage)?;
                    #[cfg(target_os = "linux")]
                    if safety.telemetry().is_some() {
                        events.push((id, schedule.task(), decision, run));
                    }
                }
                tx.execute(
                    "UPDATE schedule_state SET checkpoint_ms=max(checkpoint_ms,?2) WHERE schedule_id=?1",
                    params![id, minute],
                ).map_err(storage)?;
            }
            tx.commit().map_err(storage)
        })?;
        #[cfg(target_os = "linux")]
        if let Some(telemetry) = safety.telemetry() {
            for (id, reason, count) in gaps {
                telemetry.gap(id, reason, count);
            }
            for (id, task, decision, run) in events {
                telemetry.occurrence(id, task, decision, run.as_deref());
            }
        }
        Ok(())
    }

    pub(crate) fn pause_schedule(&mut self, id: Option<&str>, paused: bool) -> Result<(), Error> {
        self.perform(|connection| {
            let changed = if let Some(id) = id {
                connection.execute(
                    "UPDATE schedule_state SET paused=?2 WHERE schedule_id=?1 AND active=1",
                    params![id, paused],
                )
            } else {
                connection.execute("UPDATE schedule_control SET paused=?1 WHERE id=1", [paused])
            }
            .map_err(storage)?;
            if changed != 1 {
                return Err(if id.is_some() {
                    Error::UnknownSchedule
                } else {
                    Error::Storage
                });
            }
            Ok(())
        })
    }

    pub(crate) fn schedule_history(
        &mut self,
        id: &str,
        after_due: Option<i64>,
        after_gap: Option<i64>,
    ) -> Result<serde_json::Value, Error> {
        self.perform(|connection| {
            let state: Option<(bool, bool, i64)> = connection.query_row(
                "SELECT active,paused,checkpoint_ms FROM schedule_state WHERE schedule_id=?1",
                [id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).optional().map_err(storage)?;
            let (active, paused, checkpoint_ms) = state.ok_or(Error::UnknownSchedule)?;
            let global: bool = connection.query_row("SELECT paused FROM schedule_control WHERE id=1", [], |row| row.get(0)).map_err(storage)?;
            let mut occurrences = connection.prepare(
                "SELECT due_ms,revision,decision,runs.run_id FROM occurrences
                 LEFT JOIN runs USING(run_id) WHERE schedule_id=?1 AND due_ms>?2 ORDER BY due_ms LIMIT 20"
            ).map_err(storage)?;
            let occurrences = occurrences.query_map(params![id, after_due.unwrap_or(-1)], |row| Ok(Occurrence {
                due_ms: row.get(0)?, revision: hex::encode(row.get::<_, Vec<u8>>(1)?), decision: row.get(2)?, run_id: row.get(3)?,
            })).map_err(storage)?.collect::<Result<Vec<_>, _>>().map_err(storage)?;
            let mut gaps = connection.prepare(
                "SELECT id,after_ms,through_ms,reason,missed_count FROM schedule_gaps WHERE schedule_id=?1 AND id>?2 ORDER BY id LIMIT 20"
            ).map_err(storage)?;
            let gaps = gaps.query_map(params![id, after_gap.unwrap_or(0)], |row| Ok(Gap {
                id: row.get(0)?, after_ms: row.get(1)?, through_ms: row.get(2)?, reason: row.get(3)?, missed_count: row.get(4)?,
            })).map_err(storage)?.collect::<Result<Vec<_>, _>>().map_err(storage)?;
            Ok(serde_json::json!({"active":active,"paused":paused,"global_paused":global,"checkpoint_ms":checkpoint_ms,
                "occurrences":occurrences,"gaps":gaps}))
        })
    }
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn matchers(config: &Config) -> Vec<(Cron, Vec<u8>)> {
    config
        .schedules()
        .iter()
        .map(|schedule| {
            let encoded = serde_json::to_vec(schedule).expect("validated schedule");
            (
                schedule.matcher(),
                revision(b"rustward-schedule-v1", &encoded),
            )
        })
        .collect()
}

#[cfg(test)]
#[path = "schedule_tests.rs"]
mod tests;
