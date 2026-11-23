use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use std::path::Path;

use super::{Error, Store, random_bytes, storage, utc_ms};
use crate::supervision::RunId;

const BATCH: i64 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    Keep,
    Cancel,
    Interrupt,
}

impl Decision {
    fn name(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Cancel => "cancel",
            Self::Interrupt => "interrupt",
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct RestoreReview {
    run_id: String,
    original_phase: String,
    original_result: Option<String>,
    decision: Option<String>,
    note: Option<String>,
    reviewed_at_ms: Option<i64>,
}

pub(crate) fn snapshot(source: &Path, destination: &Path) -> Result<(), Error> {
    let path = destination.to_str().ok_or(Error::InvalidInputs)?;
    let db =
        Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(storage)?;
    db.execute("VACUUM INTO ?1", [path])
        .map(|_| ())
        .map_err(storage)
}

impl Store {
    pub(crate) fn expire(
        &mut self,
        cutoff: i64,
        mut remove_logs: impl FnMut(&RunId) -> Result<(), Error>,
    ) -> Result<usize, Error> {
        self.perform(|db| {
            let tx = db
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage)?;
            let ids = {
                let mut query = tx
                    .prepare(
                        "SELECT runs.run_id FROM runs LEFT JOIN launches USING(run_id)
                     WHERE phase='completed' AND completed_at>=0 AND completed_at<=?1
                       AND ((dispatched=0 AND launches.run_id IS NULL) OR
                            (dispatched=1 AND cleanup_confirmed=1 AND review_required=0))
                     ORDER BY completed_at,sequence LIMIT ?2",
                    )
                    .map_err(storage)?;
                query
                    .query_map(params![cutoff, BATCH], |row| row.get::<_, String>(0))
                    .map_err(storage)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(storage)?
            };
            for id in &ids {
                remove_logs(&RunId::new(id).map_err(|_| Error::Storage)?)?;
                tx.execute("DELETE FROM launches WHERE run_id=?1", [id])
                    .map_err(storage)?;
                tx.execute("DELETE FROM runs WHERE run_id=?1", [id])
                    .map_err(storage)?;
            }
            tx.commit().map_err(storage)?;
            Ok(ids.len())
        })
    }

    pub(crate) fn protected_output(&mut self) -> Result<Vec<(String, i64)>, Error> {
        self.perform(|db| {
            let mut query = db
                .prepare(
                    "SELECT run_id,log_bytes FROM launches JOIN runs USING(run_id)
                 WHERE phase='completed' AND review_required=1 AND log_bytes>0",
                )
                .map_err(storage)?;
            query
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(storage)?
                .collect::<Result<_, _>>()
                .map_err(storage)
        })
    }

    pub(crate) fn check_integrity(&mut self) -> Result<(), Error> {
        self.perform(|db| {
            let result: String = db
                .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
                .map_err(storage)?;
            if result == "ok" {
                Ok(())
            } else {
                Err(Error::Storage)
            }
        })
    }

    pub(crate) fn restore_phase(&mut self) -> Result<Option<String>, Error> {
        self.perform(|db| {
            db.query_row("SELECT phase FROM restore_control WHERE id=1", [], |row| {
                row.get(0)
            })
            .optional()
            .map_err(storage)
        })
    }

    pub(crate) fn seal_backup(&mut self) -> Result<(), Error> {
        self.perform(|db| {
            let tx = db
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage)?;
            tx.execute_batch(
                "INSERT INTO restore_control VALUES (1,'backup')
                 ON CONFLICT(id) DO UPDATE SET phase='backup';
                 UPDATE issuer SET closed=1;
                 UPDATE dispatch_control SET paused=1;",
            )
            .map_err(storage)?;
            tx.commit().map_err(storage)
        })
    }

    pub(crate) fn begin_restore(&mut self, log_bytes: impl Fn(&str) -> u64) -> Result<(), Error> {
        self.perform(|db| {
            let tx = db
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage)?;
            if tx
                .execute(
                    "UPDATE restore_control SET phase='review' WHERE phase='backup'",
                    [],
                )
                .map_err(storage)?
                != 1
            {
                return Err(Error::InvalidTransition);
            }
            tx.execute(
                "UPDATE issuer SET generation=?1,secret=?2,closed=1",
                params![random_bytes()?.as_slice(), random_bytes()?.as_slice()],
            )
            .map_err(storage)?;
            tx.execute_batch(
                "UPDATE dispatch_control SET paused=1;
                 INSERT INTO restore_reviews (generation,run_id,original_phase,original_result)
                 SELECT generation,run_id,phase,result FROM runs CROSS JOIN issuer
                 WHERE phase!='completed';",
            )
            .map_err(storage)?;
            let active = {
                let mut query = tx
                    .prepare("SELECT run_id FROM runs WHERE dispatched=1 AND phase!='completed'")
                    .map_err(storage)?;
                query
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(storage)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(storage)?
            };
            for id in active {
                let bytes = i64::try_from(log_bytes(&id)).map_err(|_| Error::Storage)?;
                if tx
                    .execute(
                        "UPDATE launches SET log_bytes=?2 WHERE run_id=?1",
                        params![id, bytes],
                    )
                    .map_err(storage)?
                    != 1
                {
                    return Err(Error::Storage);
                }
            }
            tx.commit().map_err(storage)
        })
    }

    pub(crate) fn restore_reviews(&mut self) -> Result<Vec<RestoreReview>, Error> {
        self.perform(|db| {
            let mut query = db
                .prepare(
                    "SELECT run_id,original_phase,original_result,decision,note,reviewed_at
                 FROM restore_reviews JOIN issuer USING(generation) ORDER BY run_id",
                )
                .map_err(storage)?;
            query
                .query_map([], |row| {
                    Ok(RestoreReview {
                        run_id: row.get(0)?,
                        original_phase: row.get(1)?,
                        original_result: row.get(2)?,
                        decision: row.get(3)?,
                        note: row.get(4)?,
                        reviewed_at_ms: row.get(5)?,
                    })
                })
                .map_err(storage)?
                .collect::<Result<_, _>>()
                .map_err(storage)
        })
    }

    pub(crate) fn decide_restore(
        &mut self,
        id: &RunId,
        decision: Decision,
        note: &str,
    ) -> Result<(), Error> {
        if note.trim().is_empty() || note.len() > 4096 {
            return Err(Error::InvalidReview);
        }
        self.perform(|db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            let run = id.as_str();
            let phase: Option<String> = tx.query_row(
                "SELECT original_phase FROM restore_reviews JOIN issuer USING(generation)
                 CROSS JOIN restore_control WHERE restore_control.phase='review' AND run_id=?1 AND decision IS NULL",
                [run], |row| row.get(0)).optional().map_err(storage)?;
            let now = utc_ms()?;
            match (phase.as_deref(), decision) {
                (Some("queued"), Decision::Keep) => {}
                (Some("queued"), Decision::Cancel) => {
                    tx.execute("UPDATE runs SET phase='completed',result='cancellation',completed_at=?2
                                WHERE run_id=?1 AND phase='queued'", params![run, now]).map_err(storage)?;
                }
                (Some("launching" | "running" | "cleaning_up"), Decision::Interrupt) => {
                    if tx.execute("UPDATE launches SET cleanup_confirmed=1,review_required=1 WHERE run_id=?1", [run])
                        .map_err(storage)? != 1 {
                        return Err(Error::Storage);
                    }
                    tx.execute("UPDATE runs SET phase='completed',result='interruption',completed_at=?2
                                WHERE run_id=?1", params![run, now]).map_err(storage)?;
                }
                _ => return Err(Error::InvalidTransition),
            }
            tx.execute("UPDATE restore_reviews SET decision=?2,note=?3,reviewed_at=?4
                        WHERE run_id=?1 AND generation=(SELECT generation FROM issuer)",
                params![run, decision.name(), note, now]).map_err(storage)?;
            tx.commit().map_err(storage)
        })
    }

    pub(crate) fn release_restore(&mut self) -> Result<(), Error> {
        self.perform(|db| {
            if db.execute(
                "UPDATE restore_control SET phase='released' WHERE phase='review'
                 AND NOT EXISTS (SELECT 1 FROM restore_reviews JOIN issuer USING(generation) WHERE decision IS NULL)",
                [],
            ).map_err(storage)? != 1 {
                return Err(Error::InvalidTransition);
            }
            Ok(())
        })
    }
}
