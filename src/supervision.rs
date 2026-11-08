use std::{os::fd::BorrowedFd, time::Duration};

pub const DEFAULT_CLEANUP_GRACE: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunId(String);

impl RunId {
    pub fn new(value: &str) -> Result<Self, Error> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::InvalidRequest);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub struct Launch {
    pub run_id: RunId,
    pub executable: String,
    pub arguments: Vec<String>,
    pub working_directory: String,
    pub cleanup_grace: Duration,
}

pub struct Output<'a> {
    pub stdout: BorrowedFd<'a>,
    pub stderr: BorrowedFd<'a>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionIdentity {
    pub invocation: [u8; 16],
    pub control_group: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessExit {
    Code(i32),
    Signal(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionState {
    Starting,
    Running,
    Exited,
    Stopping,
    Ended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cleanup {
    Pending,
    Confirmed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Observation {
    Missing,
    Uncertain,
    Known {
        identity: ExecutionIdentity,
        state: ExecutionState,
        exit: Option<ProcessExit>,
        cleanup: Cleanup,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartOutcome {
    Accepted,
    Rejected,
    Conflict,
    Uncertain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopMode {
    Graceful,
    Force,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopOutcome {
    Accepted,
    Missing,
    Uncertain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidRequest,
    Unavailable,
    IncompatibleManager,
    IdentityMismatch,
    CleanupPending,
}

/// The caller must durably record launch intent before calling start once.
/// Accepted means a start job was queued, not that the program executed.
pub trait Supervisor {
    fn start(&mut self, launch: &Launch, output: Output<'_>) -> Result<StartOutcome, Error>;
    fn observe(
        &mut self,
        run_id: &RunId,
        expected: Option<&ExecutionIdentity>,
    ) -> Result<Observation, Error>;
    fn stop(
        &mut self,
        run_id: &RunId,
        expected: &ExecutionIdentity,
        mode: StopMode,
    ) -> Result<StopOutcome, Error>;
    fn release(&mut self, run_id: &RunId, expected: &ExecutionIdentity) -> Result<(), Error>;
}
