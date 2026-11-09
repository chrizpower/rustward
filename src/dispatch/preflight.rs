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
        self.fail_unstarted(permit, Some(failure), None)
    }

    pub(crate) fn abort_unstarted(&mut self, permit: Permit) -> Result<(), Error> {
        if !Arc::ptr_eq(&self.owner, &permit.owner)
            || self
                .active
                .get(permit.launch.run_id.as_str())
                .is_none_or(|active| active.start_called)
        {
            return Err(Error::InvalidTransition);
        }
        self.decide_stop(permit.launch.run_id.as_str(), super::StopReason::Storage)?;
        Ok(())
    }

    pub(crate) fn retain_unstarted(&mut self, run_id: &str) -> Result<(), Error> {
        if self
            .active
            .get(run_id)
            .is_none_or(|active| active.start_called)
        {
            return Err(Error::InvalidTransition);
        }
        let saved=self.store.perform(|connection| {
            let tx=connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            let (phase,result):(String,Option<String>)=tx.query_row("SELECT phase,result FROM runs WHERE run_id=?1",[run_id],|row|Ok((row.get(0)?,row.get(1)?))).map_err(storage)?;
            if phase=="completed" {return Err(Error::InvalidTransition);}
            let changed=tx.execute("UPDATE runs SET phase='cleaning_up',result=COALESCE(result,'interruption') WHERE run_id=?1",[run_id]).map_err(storage)?;
            if changed!=1 {return Err(Error::Storage);}
            let changed=tx.execute("UPDATE launches SET start_state='rejected',cleanup_confirmed=1,review_required=max(review_required,?2) WHERE run_id=?1",params![run_id,result.is_none()]).map_err(storage)?;
            if changed!=1 {return Err(Error::Storage);}
            tx.commit().map_err(storage)
        });
        if matches!(saved, Err(Error::Storage | Error::Busy)) {
            self.store.close();
            self.safety.fail();
        }
        if saved.is_ok() {
            self.active.get_mut(run_id).unwrap().cleaned = true;
        }
        saved
    }

    pub fn fail_capture_setup(
        &mut self,
        permit: Permit,
        failure: crate::capture::SetupError,
    ) -> Result<(), Error> {
        self.fail_unstarted(permit, None, Some(failure))
    }

    fn fail_unstarted(
        &mut self,
        permit: Permit,
        failure: Option<LaunchFailure>,
        capture: Option<crate::capture::SetupError>,
    ) -> Result<(), Error> {
        if !Arc::ptr_eq(&self.owner, &permit.owner)
            || self
                .active
                .get(permit.launch.run_id.as_str())
                .is_none_or(|active| active.start_called)
        {
            return Err(Error::InvalidTransition);
        }
        if failure == Some(LaunchFailure::WorkStorage)
            || capture == Some(crate::capture::SetupError::Storage)
        {
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
            let capture=capture.map(|failure|match failure {crate::capture::SetupError::AlreadyExists=>"exists",crate::capture::SetupError::Storage=>"storage",crate::capture::SetupError::Resources=>"resources"});
            let changed = transaction.execute("UPDATE launches SET start_state='rejected',cleanup_confirmed=1,launch_failure=?1,capture_setup_failure=?3 WHERE run_id=?2",params![failure.map(LaunchFailure::text),run_id,capture]).map_err(storage)?;
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

impl From<crate::runtime::Error> for LaunchFailure {
    fn from(error: crate::runtime::Error) -> Self {
        match error {
            crate::runtime::Error::Missing => Self::RuntimeMissing,
            crate::runtime::Error::Invalid | crate::runtime::Error::Limit => Self::RuntimeInvalid,
            crate::runtime::Error::Changed => Self::RuntimeChanged,
            crate::runtime::Error::Unsafe => Self::RuntimeUnsafe,
            crate::runtime::Error::Command => Self::RuntimeCommand,
            crate::runtime::Error::WorkDirectory => Self::WorkDirectory,
            crate::runtime::Error::WorkStorage => Self::WorkStorage,
        }
    }
}
