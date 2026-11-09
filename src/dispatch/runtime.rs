use std::{path::Path, sync::Arc};

use super::{LaunchFailure, Permit, Session};
use crate::{
    admission::{Error, RunPhase, storage},
    runtime::{self, Catalog, WorkDirectory, WorkRoot},
    supervision::{Launch, Output, StartOutcome, Supervisor},
};

#[derive(Debug, PartialEq, Eq)]
pub enum PrepareError {
    State(Error),
    Runtime(runtime::Error),
}

pub struct PreparedPermit {
    permit: Permit,
    catalog: Catalog,
    work: WorkDirectory,
}
impl PreparedPermit {
    pub fn launch(&self) -> &Launch {
        self.permit.launch()
    }
    pub fn runtime_id(&self) -> &str {
        self.permit.runtime_id()
    }
    pub fn work_directory(&self) -> &Path {
        self.work.path()
    }
    pub fn work_owner_uid(&self) -> u32 {
        self.work.owner_uid()
    }
    pub fn execution_uid(&self) -> u32 {
        self.catalog.execution_uid()
    }
}

impl Session {
    fn check_unstarted(&mut self, permit: &Permit) -> Result<(), Error> {
        if self.safety.failed() {
            return Err(Error::Closed);
        }
        if !Arc::ptr_eq(&self.owner, &permit.owner)
            || self
                .active
                .get(permit.launch.run_id.as_str())
                .is_none_or(|active| active.start_called)
        {
            return Err(Error::InvalidTransition);
        }
        if self.active[permit.launch.run_id.as_str()].expired(std::time::Instant::now()) {
            self.decide_stop(permit.launch.run_id.as_str(), super::StopReason::Timeout)?;
            return Err(Error::Closed);
        }
        self.store.perform(|connection| {
            let review: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM launches WHERE review_required=1)",
                    [],
                    |row| row.get(0),
                )
                .map_err(storage)?;
            if review { Err(Error::Closed) } else { Ok(()) }
        })?;
        let state = self
            .store
            .run_state(permit.launch.run_id.as_str())?
            .ok_or(Error::InvalidTransition)?;
        if state.phase != RunPhase::Launching || state.result.is_some() {
            return Err(Error::InvalidTransition);
        }
        Ok(())
    }

    fn runtime_failure(&mut self, permit: Permit, error: runtime::Error) -> PrepareError {
        let reason = match error {
            runtime::Error::Missing => LaunchFailure::RuntimeMissing,
            runtime::Error::Invalid | runtime::Error::Limit => LaunchFailure::RuntimeInvalid,
            runtime::Error::Changed => LaunchFailure::RuntimeChanged,
            runtime::Error::Unsafe => LaunchFailure::RuntimeUnsafe,
            runtime::Error::Command => LaunchFailure::RuntimeCommand,
            runtime::Error::WorkDirectory => LaunchFailure::WorkDirectory,
            runtime::Error::WorkStorage => LaunchFailure::WorkStorage,
        };
        match self.fail_before_start(permit, reason) {
            Ok(()) => PrepareError::Runtime(error),
            Err(error) => PrepareError::State(error),
        }
    }

    /// Verify only the admitted runtime and paths. The writable directory does
    /// not replace working_directory or change any argument.
    pub fn prepare_runtime(
        &mut self,
        permit: Permit,
        catalog: &Catalog,
        work: &WorkRoot,
    ) -> Result<PreparedPermit, PrepareError> {
        self.check_unstarted(&permit).map_err(PrepareError::State)?;
        let checked = (|| {
            if work.path().starts_with(catalog.path()) || catalog.path().starts_with(work.path()) {
                return Err(runtime::Error::WorkDirectory);
            }
            catalog
                .verify(permit.runtime_id())?
                .check_command(permit.launch())?;
            work.create(&permit.launch.run_id)
        })();
        match checked {
            Ok(work) => Ok(PreparedPermit {
                permit,
                catalog: catalog.clone(),
                work,
            }),
            Err(error) => Err(self.runtime_failure(permit, error)),
        }
    }

    /// Recheck after capture setup. The future controller must bind the catalog's
    /// execution UID to its manager and connect controlled environment/capture.
    pub fn start_prepared(
        &mut self,
        prepared: PreparedPermit,
        supervisor: &mut impl Supervisor,
        output: Output<'_>,
    ) -> Result<StartOutcome, PrepareError> {
        self.check_unstarted(&prepared.permit)
            .map_err(PrepareError::State)?;
        let checked = (|| {
            prepared
                .catalog
                .verify(prepared.permit.runtime_id())?
                .check_command(prepared.permit.launch())?;
            prepared.work.check()
        })();
        if let Err(error) = checked {
            return Err(self.runtime_failure(prepared.permit, error));
        }
        self.start(prepared.permit, supervisor, output)
            .map_err(PrepareError::State)
    }
}
