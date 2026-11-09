//! One-run administrator acknowledgement. Transport authentication and retention
//! reclamation are service integration work; see docs/temporary/review.md.
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::{
    admission::{Error, RunPhase, RunState, Store, read_run_state, storage, utc_ms},
    dispatch::{Record, Times, read_record},
    supervision::RunId,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    pub run_id: String,
    pub evidence_version: i64,
    pub task_id: String,
    pub runtime_id: String,
    pub task_revision: Vec<u8>,
    pub config_revision: Vec<u8>,
    pub state: RunState,
    pub launch: Record,
    pub times: Times,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Acknowledgement {
    pub acknowledgement_id: String,
    pub review: Review,
    pub note: String,
    pub acknowledged_at_ms: i64,
}

/// An acknowledgement identity used for matching retries, not an admission key
/// or authorization credential.
pub fn new_acknowledgement_id() -> Result<String, Error> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| Error::Randomness)?;
    Ok(hex::encode(bytes))
}

fn read_review(connection: &Connection, run_id: &str) -> Result<Option<Review>, Error> {
    let Some(launch) = read_record(connection, run_id)? else {
        return Ok(None);
    };
    if !launch.review_required {
        return Ok(None);
    }
    let state = read_run_state(connection, run_id)?.ok_or(Error::Storage)?;
    connection.query_row("SELECT task_id,runtime_id,task_revision,config_revision,
        dispatched_at,completed_at,evidence_version FROM runs JOIN launches USING(run_id) WHERE run_id=?1",
        [run_id], |r| Ok(Review {
            run_id: run_id.to_owned(), evidence_version: r.get(6)?, task_id: r.get(0)?, runtime_id: r.get(1)?,
            task_revision: r.get(2)?, config_revision: r.get(3)?, state, launch,
            times: Times { dispatched_at_ms: r.get(4)?, completed_at_ms: r.get(5)? },
        })).map(Some).map_err(storage)
}

fn read_acknowledgement(
    connection: &Connection,
    acknowledgement_id: &str,
) -> Result<Option<Acknowledgement>, Error> {
    let record: Option<(i64, String)> = connection
        .query_row(
            "SELECT encoding,record FROM acknowledgements WHERE acknowledgement_id=?1",
            [acknowledgement_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(storage)?;
    record
        .map(|(encoding, text)| {
            if encoding != 1 {
                return Err(Error::Storage);
            }
            let result: Acknowledgement =
                serde_json::from_str(&text).map_err(|_| Error::Storage)?;
            if result.acknowledgement_id != acknowledgement_id {
                return Err(Error::Storage);
            }
            Ok(result)
        })
        .transpose()
}

impl Store {
    /// Return one consistent snapshot. Keep it unchanged for acknowledgement.
    pub fn review(&mut self, run_id: &str) -> Result<Option<Review>, Error> {
        self.perform(|connection| {
            let transaction = connection.transaction().map_err(storage)?;
            let review = read_review(&transaction, run_id)?;
            transaction.commit().map_err(storage)?;
            Ok(review)
        })
    }

    /// Read pending reviews in run ID order, at most 100 per page. Pass the last
    /// returned run ID as `after` for the next page. Pages are separate snapshots.
    pub fn pending_reviews(
        &mut self,
        after: Option<&str>,
        limit: u16,
    ) -> Result<Vec<Review>, Error> {
        if limit == 0 || limit > 100 {
            return Err(Error::InvalidLimits);
        }
        self.perform(|connection| {
            let transaction = connection.transaction().map_err(storage)?;
            let ids = {
                let mut statement = transaction.prepare("SELECT run_id FROM launches WHERE review_required=1 AND run_id>?1 ORDER BY run_id LIMIT ?2").map_err(storage)?;
                statement.query_map(params![after.unwrap_or(""),limit], |r| r.get::<_, String>(0)).map_err(storage)?.collect::<Result<Vec<_>, _>>().map_err(storage)?
            };
            let reviews = ids.iter().map(|id| read_review(&transaction,id)?.ok_or(Error::Storage)).collect::<Result<Vec<_>, _>>()?;
            transaction.commit().map_err(storage)?;
            Ok(reviews)
        })
    }

    pub fn acknowledgement(
        &mut self,
        acknowledgement_id: &str,
    ) -> Result<Option<Acknowledgement>, Error> {
        self.perform(|connection| read_acknowledgement(connection, acknowledgement_id))
    }

    /// Store the note and exact reviewed evidence, then clear only this run's
    /// review block. Matching retries return before current-state validation.
    pub fn acknowledge(
        &mut self,
        acknowledgement_id: &str,
        reviewed: &Review,
        note: &str,
    ) -> Result<Acknowledgement, Error> {
        if RunId::new(acknowledgement_id).is_err()
            || note.trim().is_empty()
            || note.len() > 4096
            || note.contains('\0')
        {
            return Err(Error::InvalidReview);
        }
        self.perform(|connection| {
            let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            if let Some(original) = read_acknowledgement(&transaction, acknowledgement_id)? {
                if original.review != *reviewed || original.note != note { return Err(Error::PayloadConflict); }
                return Ok(original);
            }
            let current = read_review(&transaction, &reviewed.run_id)?.ok_or(Error::ReviewChanged)?;
            if current != *reviewed { return Err(Error::ReviewChanged); }
            if current.state.phase != RunPhase::Completed || !current.launch.cleanup_confirmed {
                return Err(Error::NotCompleted);
            }
            let acknowledgement = Acknowledgement { acknowledgement_id: acknowledgement_id.to_owned(), review: current, note: note.to_owned(), acknowledged_at_ms: utc_ms()? };
            let record = serde_json::to_string(&acknowledgement).map_err(|_| Error::Storage)?;
            let inserted = transaction.execute("INSERT INTO acknowledgements VALUES (?1,1,?2)", params![acknowledgement_id,record]).map_err(storage)?;
            if inserted != 1 { return Err(Error::Storage); }
            let changed = transaction.execute("UPDATE launches SET review_required=0 WHERE run_id=?1 AND evidence_version=?2 AND review_required=1", params![reviewed.run_id,reviewed.evidence_version]).map_err(storage)?;
            if changed != 1 { return Err(Error::Storage); }
            transaction.commit().map_err(storage)?;
            Ok(acknowledgement)
        })
    }
}
