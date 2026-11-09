use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::Session;
use crate::{
    admission::{Error, storage, utc_ms},
    supervision::{Cleanup, ExecutionIdentity, ExecutionState, Observation, ProcessExit},
};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Record {
    pub start_state: String,
    pub identity: Option<ExecutionIdentity>,
    pub exit: Option<ProcessExit>,
    pub cleanup_confirmed: bool,
    pub review_required: bool,
    pub log_bytes: u64,
    pub stop_reason: Option<String>,
    pub output_failed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_failure: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_setup_failure: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_evidence: Option<crate::capture::Evidence>,
}

pub(crate) fn read(connection: &Connection, run_id: &str) -> Result<Option<Record>, Error> {
    connection.query_row("SELECT start_state,invocation,control_group,cleanup_confirmed,review_required,log_bytes,exit_kind,exit_value,stop_reason,output_failed,launch_failure,capture_setup_failure,output_evidence
        FROM launches WHERE run_id=?1", [run_id], |r| {
        let invocation: Option<Vec<u8>> = r.get(1)?;
        let group: Option<String> = r.get(2)?;
        Ok((r.get::<_, String>(0)?,invocation,group,r.get(3)?,r.get(4)?,r.get::<_, i64>(5)?,r.get::<_, Option<String>>(6)?,r.get::<_, Option<i32>>(7)?,r.get::<_, Option<String>>(8)?,r.get::<_, bool>(9)?,r.get::<_, Option<String>>(10)?,r.get::<_, Option<String>>(11)?,r.get::<_, Option<String>>(12)?))
    }).optional().map_err(storage)?.map(|(start_state, invocation, group, cleanup_confirmed, review_required, log_bytes, exit_kind, exit_value, stop_reason, output_failed, launch_failure, capture_setup_failure, output_evidence)| {
        let identity = match (invocation, group) {
            (None, None) => None,
            (Some(bytes), Some(control_group)) => Some(ExecutionIdentity {
                invocation: bytes.try_into().map_err(|_| Error::Storage)?, control_group,
            }),
            _ => return Err(Error::Storage),
        };
        let exit = match (exit_kind.as_deref(), exit_value) {
            (None, None) => None,
            (Some("code"), Some(value)) => Some(ProcessExit::Code(value)),
            (Some("signal"), Some(value)) => Some(ProcessExit::Signal(value)),
            _ => return Err(Error::Storage),
        };
        let output_evidence=output_evidence.map(|source|serde_json::from_str(&source).map_err(|_|Error::Storage)).transpose()?;
        Ok(Record { start_state, identity, exit, cleanup_confirmed, review_required, stop_reason, output_failed, launch_failure, capture_setup_failure, output_evidence, log_bytes: u64::try_from(log_bytes).map_err(|_| Error::Storage)? })
    }).transpose()
}

impl Session {
    pub fn record(&mut self, run_id: &str) -> Result<Option<Record>, Error> {
        self.store.perform(|connection| read(connection, run_id))
    }

    /// Retain supervisor evidence. Uncertainty and identity mismatch require
    /// review and never replace an earlier result or execution identity.
    pub fn observe(&mut self, run_id: &str, observation: &Observation) -> Result<(), Error> {
        self.retain_observation(run_id, observation);
        let saved = self.store.perform(|connection| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage)?;
            let record = read(&transaction, run_id)?.ok_or(Error::InvalidTransition)?;
            let (phase, previous): (String, Option<String>) = transaction
                .query_row(
                    "SELECT phase,result FROM runs WHERE run_id=?1",
                    [run_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(storage)?;
            if phase == "completed" {
                return Err(Error::InvalidTransition);
            }
            let known = match observation {
                Observation::Known {
                    identity,
                    state,
                    exit,
                    cleanup,
                } if identity.control_group.starts_with('/')
                    && identity.control_group.len() <= 4096
                    && !identity.control_group.contains('\0')
                    && record
                        .identity
                        .as_ref()
                        .is_none_or(|original| original == identity)
                    && !matches!(
                        (state, cleanup),
                        (
                            ExecutionState::Starting
                                | ExecutionState::Running
                                | ExecutionState::Stopping,
                            Cleanup::Confirmed
                        )
                    )
                    && !(exit.is_some()
                        && matches!(state, ExecutionState::Starting | ExecutionState::Running)) =>
                {
                    Some((identity, state, exit, cleanup))
                }
                _ => None,
            };
            let mut result = previous;
            let mut next_phase = phase;
            let mut review = record.review_required;
            let mut cleaned = record.cleanup_confirmed;
            if let Some((identity, state, exit, cleanup)) = known {
                let changed = transaction
                    .execute(
                        "UPDATE launches SET invocation=?1,control_group=?2 WHERE run_id=?3",
                        params![
                            identity.invocation.as_slice(),
                            identity.control_group,
                            run_id
                        ],
                    )
                    .map_err(storage)?;
                if changed != 1 {
                    return Err(Error::Storage);
                }
                if let Some(exit) = exit {
                    if let Some(original) = &record.exit {
                        review |= original != exit;
                    } else {
                        let (kind, value) = match exit {
                            ProcessExit::Code(value) => ("code", value),
                            ProcessExit::Signal(value) => ("signal", value),
                        };
                        let changed = transaction
                            .execute(
                                "UPDATE launches SET exit_kind=?1,exit_value=?2 WHERE run_id=?3",
                                params![kind, value, run_id],
                            )
                            .map_err(storage)?;
                        if changed != 1 {
                            return Err(Error::Storage);
                        }
                    }
                }
                if result.is_none() {
                    if let Some(exit) = exit {
                        result = Some(
                            if *exit == ProcessExit::Code(0) {
                                "success"
                            } else {
                                "failure"
                            }
                            .into(),
                        );
                    } else if matches!(
                        state,
                        ExecutionState::Exited | ExecutionState::Stopping | ExecutionState::Ended
                    ) {
                        result = Some("interruption".into());
                        review = true;
                    } else if *state == ExecutionState::Running {
                        next_phase = "running".into();
                    }
                }
                cleaned |= *cleanup == Cleanup::Confirmed;
            } else {
                result.get_or_insert_with(|| "interruption".into());
                review = true;
            }
            if result.is_some() {
                next_phase = "cleaning_up".into();
            }
            let changed = transaction
                .execute(
                    "UPDATE runs SET phase=?1,result=?2 WHERE run_id=?3",
                    params![next_phase, result, run_id],
                )
                .map_err(storage)?;
            if changed != 1 {
                return Err(Error::Storage);
            }
            let changed = transaction
                .execute(
                    "UPDATE launches SET cleanup_confirmed=?1,review_required=?2 WHERE run_id=?3",
                    params![cleaned, review, run_id],
                )
                .map_err(storage)?;
            if changed != 1 {
                return Err(Error::Storage);
            }
            transaction.commit().map_err(storage)
        });
        if matches!(saved, Err(Error::Storage | Error::Busy)) {
            self.store.close();
            self.safety.fail();
        }
        if saved.is_ok() {
            let record = self.record(run_id)?.ok_or(Error::Storage)?;
            self.retain_record(run_id, &record);
        }
        saved
    }

    /// Record final completion only after confirmed cleanup and synchronized
    /// capture. Retained bytes must not exceed this run's reserved allowance.
    pub fn finish(&mut self, run_id: &str, retained_bytes: u64) -> Result<(), Error> {
        self.finish_output(run_id, retained_bytes, false, None)
    }

    /// Complete failed output handling after all workers have ended. This does
    /// not assert synchronization of bytes whose storage failed.
    pub fn finish_failed_output(&mut self, run_id: &str, retained_bytes: u64) -> Result<(), Error> {
        self.finish_output(run_id, retained_bytes, true, None)
    }

    pub fn finish_evidence(
        &mut self,
        run_id: &str,
        evidence: &crate::capture::Evidence,
    ) -> Result<(), Error> {
        let record = self.record(run_id)?.ok_or(Error::InvalidTransition)?;
        if !evidence.valid()
            || ((!evidence.synchronized || evidence.recovered) && !record.output_failed)
        {
            return Err(Error::InvalidTransition);
        }
        let bytes = evidence.retained_bytes().ok_or(Error::InvalidTransition)?;
        self.finish_output(run_id, bytes, record.output_failed, Some(evidence))
    }

    fn finish_output(
        &mut self,
        run_id: &str,
        retained_bytes: u64,
        failed_output: bool,
        evidence: Option<&crate::capture::Evidence>,
    ) -> Result<(), Error> {
        let saved = self.store.perform(|connection| {
            let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            let record = read(&transaction, run_id)?.ok_or(Error::InvalidTransition)?;
            if record.output_failed != failed_output || !record.cleanup_confirmed || retained_bytes > record.log_bytes { return Err(Error::InvalidTransition); }
            let ready: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM runs WHERE run_id=?1 AND phase='cleaning_up' AND result IS NOT NULL)", [run_id], |r| r.get(0)).map_err(storage)?;
            if !ready { return Err(Error::InvalidTransition); }
            let changed = transaction.execute("UPDATE runs SET phase='completed',completed_at=?2 WHERE run_id=?1 AND phase='cleaning_up' AND result IS NOT NULL", params![run_id,utc_ms()?]).map_err(storage)?;
            if changed != 1 { return Err(Error::Storage); }
            let encoded=evidence.map(serde_json::to_string).transpose().map_err(|_|Error::Storage)?;
            let changed = transaction.execute("UPDATE launches SET log_bytes=?1,output_evidence=?3 WHERE run_id=?2", params![retained_bytes as i64,run_id,encoded]).map_err(storage)?;
            if changed != 1 { return Err(Error::Storage); }
            transaction.commit().map_err(storage)
        });
        if matches!(saved, Err(Error::Storage | Error::Busy)) {
            self.store.close();
            self.safety.fail();
        }
        if saved.is_ok() {
            self.active.remove(run_id);
        }
        saved
    }
}
