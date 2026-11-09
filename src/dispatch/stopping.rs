use std::time::{Duration, Instant};

use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::Session;
use crate::{
    admission::{Error, storage},
    capture,
    supervision::{ExecutionIdentity, Observation, RunId, StopMode, StopOutcome, Supervisor},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    Cancellation,
    Timeout,
    Capture,
    Storage,
}
impl StopReason {
    fn text(self) -> &'static str {
        match self {
            Self::Cancellation => "cancellation",
            Self::Timeout => "timeout",
            Self::Capture => "capture",
            Self::Storage => "storage",
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum StopDecision {
    Recorded,
    AlreadyEnded,
    NotActive,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Times {
    pub dispatched_at_ms: Option<i64>,
    pub completed_at_ms: Option<i64>,
}

pub(super) struct Active {
    deadline: Option<Instant>,
    pub(super) grace: Duration,
    stopping: Option<Instant>,
    identity: Option<ExecutionIdentity>,
    graceful_sent: bool,
    pub(super) start_called: bool,
    pub(super) cleaned: bool,
}
impl Active {
    pub(super) fn expired(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|deadline| now >= deadline)
    }
    pub(super) fn new(now: Instant, timeout: Option<u32>, grace: u32) -> Result<Self, Error> {
        if timeout == Some(0) || grace == 0 {
            return Err(Error::Storage);
        }
        let deadline = timeout
            .map(|seconds| {
                now.checked_add(Duration::from_secs(seconds.into()))
                    .ok_or(Error::Storage)
            })
            .transpose()?;
        Ok(Self {
            deadline,
            grace: Duration::from_secs(grace.into()),
            stopping: None,
            identity: None,
            graceful_sent: false,
            start_called: false,
            cleaned: false,
        })
    }
    pub(super) fn recovered(grace: Duration) -> Self {
        Self {
            deadline: None,
            grace,
            stopping: Some(Instant::now()),
            identity: None,
            graceful_sent: false,
            start_called: true,
            cleaned: false,
        }
    }
}

impl Session {
    pub(super) fn retain_record(&mut self, run_id: &str, record: &super::Record) {
        if let Some(active) = self.active.get_mut(run_id) {
            active.identity = record.identity.clone();
            active.cleaned = record.cleanup_confirmed;
        }
    }
    pub fn times(&mut self, run_id: &str) -> Result<Option<Times>, Error> {
        self.store.perform(|connection| {
            connection
                .query_row(
                    "SELECT dispatched_at,completed_at FROM runs WHERE run_id=?1",
                    [run_id],
                    |row| {
                        Ok(Times {
                            dispatched_at_ms: row.get(0)?,
                            completed_at_ms: row.get(1)?,
                        })
                    },
                )
                .optional()
                .map_err(storage)
        })
    }

    pub(crate) fn decide_expired(&mut self, now: Instant) -> Result<(), Error> {
        let ids: Vec<_> = self
            .active
            .iter()
            .filter(|(_, active)| active.stopping.is_none() && active.expired(now))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            if self.safety.failed() {
                break;
            }
            if self.decide_stop(&id, StopReason::Timeout)? == StopDecision::Recorded {
                self.active.get_mut(&id).unwrap().stopping = Some(now);
            }
        }
        Ok(())
    }

    pub fn cancel_active(&mut self, run_id: &str) -> Result<StopDecision, Error> {
        self.decide_stop(run_id, StopReason::Cancellation)
    }

    pub(super) fn decide_stop(
        &mut self,
        run_id: &str,
        reason: StopReason,
    ) -> Result<StopDecision, Error> {
        let not_started = self
            .active
            .get(run_id)
            .is_some_and(|active| !active.start_called);
        let saved = self.store.perform(|connection| {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            let row: Option<(bool, Option<String>, String)> = tx.query_row("SELECT dispatched,result,phase FROM runs WHERE run_id=?1", [run_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(storage)?;
            let Some((true, previous, phase)) = row else { return Ok(StopDecision::NotActive); };
            if phase == "completed" { return Ok(StopDecision::AlreadyEnded); }
            let output_failed = matches!(reason, StopReason::Capture | StopReason::Storage);
            if previous.is_some() && !output_failed { return Ok(StopDecision::AlreadyEnded); }
            let result = match previous.as_deref() {
                Some("success") if output_failed => "interruption",
                Some(value) => value,
                None => match reason { StopReason::Cancellation => "cancellation", StopReason::Timeout => "timeout", _ => "interruption" },
            };
            let changed = tx.execute("UPDATE runs SET phase='cleaning_up',result=?1 WHERE run_id=?2", params![result,run_id]).map_err(storage)?;
            if changed != 1 { return Err(Error::Storage); }
            let review = output_failed || !not_started;
            let changed = tx.execute("UPDATE launches SET stop_reason=COALESCE(stop_reason,?1), output_failed=max(output_failed,?2), review_required=max(review_required,?3),cleanup_confirmed=max(cleanup_confirmed,?4) WHERE run_id=?5", params![reason.text(),output_failed,review,not_started,run_id]).map_err(storage)?;
            if changed != 1 { return Err(Error::Storage); }
            tx.commit().map_err(storage)?;
            Ok(StopDecision::Recorded)
        });
        if matches!(saved, Err(Error::Storage | Error::Busy)) {
            self.store.close();
            self.safety.fail();
        }
        if matches!(saved, Ok(StopDecision::Recorded))
            && let Some(active) = self.active.get_mut(run_id)
        {
            active.stopping.get_or_insert_with(Instant::now);
            active.cleaned |= not_started;
        }
        saved
    }

    /// Report the capture worker's early failure signal before waiting for EOF.
    pub fn output_failure(
        &mut self,
        run_id: &str,
        failure: capture::Failure,
    ) -> Result<StopDecision, Error> {
        if failure.is_storage() {
            self.safety.fail();
            self.decide_stop(run_id, StopReason::Storage)
        } else {
            self.decide_stop(run_id, StopReason::Capture)
        }
    }

    /// Trip the shared gate even when database writes are unavailable.
    pub fn storage_failure(&self) {
        self.safety.fail();
    }

    pub(super) fn retain_observation(&mut self, run_id: &str, observation: &Observation) {
        if let Some(active) = self.active.get_mut(run_id)
            && let Observation::Known { identity, .. } = observation
            && identity.control_group.starts_with('/')
            && identity.control_group.len() <= 4096
            && !identity.control_group.contains('\0')
            && active
                .identity
                .as_ref()
                .is_none_or(|original| original == identity)
        {
            active.identity = Some(identity.clone());
        }
    }

    /// Poll with a monotonic time. Continue polling after storage failure: stop
    /// calls use retained process identity and do not require database writes.
    pub fn poll(&mut self, supervisor: &mut impl Supervisor, now: Instant) -> Result<(), Error> {
        let ids: Vec<_> = self.active.keys().cloned().collect();
        let mut failed = None;
        for id in ids {
            let active = &self.active[&id];
            if !self.safety.failed()
                && active.stopping.is_none()
                && active.deadline.is_some_and(|deadline| now >= deadline)
            {
                match self.decide_stop(&id, StopReason::Timeout) {
                    Ok(StopDecision::Recorded) => {
                        self.active.get_mut(&id).unwrap().stopping = Some(now)
                    }
                    Ok(_) => {}
                    Err(error) => {
                        failed.get_or_insert(error);
                    }
                }
            }
            if self.safety.failed() {
                self.active
                    .get_mut(&id)
                    .unwrap()
                    .stopping
                    .get_or_insert(now);
            } else {
                match self.run_state(&id) {
                    Ok(Some(state)) if state.result.is_some() => {
                        self.active
                            .get_mut(&id)
                            .unwrap()
                            .stopping
                            .get_or_insert(now);
                    }
                    Err(error) => {
                        failed.get_or_insert(error);
                    }
                    _ => {}
                }
            }
            if self.safety.failed() {
                self.active
                    .get_mut(&id)
                    .unwrap()
                    .stopping
                    .get_or_insert(now);
            }
            let active = &self.active[&id];
            if active.stopping.is_none() || active.cleaned {
                continue;
            }
            let run_id = RunId::new(&id).map_err(|_| Error::Storage)?;
            if active.identity.is_none() {
                let observation = supervisor
                    .observe(&run_id, None)
                    .unwrap_or(Observation::Uncertain);
                if !self.safety.failed() {
                    if let Err(error) = self.observe(&id, &observation) {
                        failed.get_or_insert(error);
                    }
                } else {
                    // Discovery by immutable run ID is used only for emergency
                    // stopping. Never use it to assert durable cleanup.
                    self.retain_observation(&id, &observation);
                }
            }
            let active = self.active.get_mut(&id).unwrap();
            if active.cleaned {
                continue;
            }
            let Some(identity) = &active.identity else {
                continue;
            };
            let force = now.saturating_duration_since(active.stopping.unwrap()) >= active.grace;
            if active.graceful_sent && !force {
                continue;
            }
            let mode = if force {
                StopMode::Force
            } else {
                StopMode::Graceful
            };
            let outcome = supervisor.stop(&run_id, identity, mode);
            active.graceful_sent |= matches!(outcome, Ok(StopOutcome::Accepted));
            if !matches!(outcome, Ok(StopOutcome::Accepted))
                && !self.safety.failed()
                && let Err(error) = self.observe(&id, &Observation::Uncertain)
            {
                failed.get_or_insert(error);
            }
        }
        failed.map_or(Ok(()), Err)
    }
}
