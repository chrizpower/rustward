use std::sync::Arc;

use rusqlite::{TransactionBehavior, params};

use super::{Permit, Session};
use crate::admission::{Error, storage};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchFailure {
    RuntimeMissing,
    RuntimeInvalid,
    RuntimeChanged,
    RuntimeUnsafe,
    RuntimeCommand,
    WorkDirectory,
    WorkStorage,
}
impl LaunchFailure {
    fn text(self) -> &'static str {
        match self {
            Self::RuntimeMissing => "runtime_missing",
            Self::RuntimeInvalid => "runtime_invalid",
            Self::RuntimeChanged => "runtime_changed",
            Self::RuntimeUnsafe => "runtime_unsafe",
            Self::RuntimeCommand => "runtime_command",
            Self::WorkDirectory => "work_directory",
            Self::WorkStorage => "work_storage",
        }
    }
}

impl Session {
    /// Consume an unstarted permit and record a definite failure. Output must
    /// still be handled before finish can commit completion and release capacity.
    pub fn fail_before_start(
        &mut self,
        permit: Permit,
        failure: LaunchFailure,
    ) -> Result<(), Error> {
        if !Arc::ptr_eq(&self.owner, &permit.owner)
            || self
                .active
                .get(permit.launch.run_id.as_str())
                .is_none_or(|active| active.start_called)
        {
            return Err(Error::InvalidTransition);
        }
        if failure == LaunchFailure::WorkStorage {
            self.safety.fail();
        }
        let run_id = permit.launch.run_id.as_str();
        let saved = self.store.perform(|connection| {
            let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            let ready: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM runs JOIN launches USING(run_id)
                WHERE run_id=?1 AND phase='launching' AND result IS NULL AND start_state='intent')",[run_id],|r|r.get(0)).map_err(storage)?;
            if !ready { return Err(Error::InvalidTransition); }
            let changed = transaction.execute("UPDATE runs SET phase='cleaning_up',result='failure' WHERE run_id=?1",[run_id]).map_err(storage)?;
            if changed != 1 { return Err(Error::Storage); }
            let changed = transaction.execute("UPDATE launches SET start_state='rejected',cleanup_confirmed=1,launch_failure=?1 WHERE run_id=?2",params![failure.text(),run_id]).map_err(storage)?;
            if changed != 1 { return Err(Error::Storage); }
            transaction.commit().map_err(storage)
        });
        if !matches!(saved, Err(Error::InvalidTransition)) {
            // The consumed permit cannot launch a process, including when its
            // failure could not be saved. This is not durable cleanup evidence.
            self.active.get_mut(run_id).unwrap().cleaned = true;
        }
        if matches!(saved, Err(Error::Storage | Error::Busy)) {
            self.store.close();
            self.safety.fail();
        }
        saved
    }
}
