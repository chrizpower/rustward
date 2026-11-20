//! Execution coordination. See docs/ARCHITECTURE.md.
use crate::{
    admission,
    capture::{self, Capture},
    dispatch::{self, Claim, Session},
    runtime::WorkDirectory,
    safety::Safety,
    supervision::{self, Observation, Profile, RunId, Supervisor},
};
use std::{collections::BTreeMap, path::Path, sync::Arc, thread::JoinHandle, time::Instant};

mod emergency;
mod flow;
mod preparation;
use emergency::{Guard, Registry};
pub use preparation::Roots;
use preparation::{Preparation, Prepared};

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    State(admission::Error),
    Context,
    Supervisor,
    Resources,
}
pub enum Start {
    Preparing(RunId),
    Blocked(dispatch::Block),
}
struct Run {
    id: RunId,
    job: Option<JoinHandle<Prepared>>,
    recovery: Option<JoinHandle<Result<capture::Report, capture::CaptureError>>>,
    capture: Option<Capture>,
    work: Option<WorkDirectory>,
    final_report: Option<Result<capture::Report, capture::CaptureError>>,
    released: bool,
    recovered: bool,
    failure_saved: bool,
    uncertainty_saved: bool,
    last: Option<Observation>,
    failed_job: bool,
}
impl Run {
    fn new(id: RunId) -> Self {
        Self {
            id,
            job: None,
            recovery: None,
            capture: None,
            work: None,
            final_report: None,
            released: false,
            recovered: false,
            failure_saved: false,
            uncertainty_saved: false,
            last: None,
            failed_job: false,
        }
    }
}

/// Supervisor clones must permit concurrent stop and observation calls.
pub struct Controller<S: Supervisor + Clone + Send + 'static> {
    session: Session,
    supervisor: S,
    registry: Registry,
    preparation: Arc<dyn Preparation>,
    runs: BTreeMap<String, Run>,
    // Keep stopping active while capture Drop can wait on file I/O.
    emergency: Guard,
}
impl<S: Supervisor + Clone + Send + 'static> Controller<S> {
    pub fn open(
        path: impl AsRef<Path>,
        roots: Roots,
        limits: dispatch::Limits,
        mut supervisor: S,
        safety: Safety,
    ) -> Result<Self, Error> {
        let uid = current_user()?;
        if !roots.binding(uid) {
            return Err(Error::Context);
        }
        if supervisor.execution_uid().map_err(|_| Error::Supervisor)? != uid {
            return Err(Error::Context);
        }
        let session =
            Session::open_with_safety(path, limits, safety.clone()).map_err(Error::State)?;
        Self::from_parts(session, supervisor, Arc::new(roots), safety)
    }
    fn from_parts(
        mut session: Session,
        mut supervisor: S,
        preparation: Arc<dyn Preparation>,
        safety: Safety,
    ) -> Result<Self, Error> {
        let registry = Registry::new(safety);
        let mut runs = BTreeMap::new();
        for (id, record, grace, called) in session.control_records().map_err(Error::State)? {
            let reference_failed =
                called && supervisor.reference(&id, record.identity.as_ref()).is_err();
            registry.insert(
                id.clone(),
                record.identity,
                record.cleanup_confirmed,
                called,
                grace,
            );
            registry.update(id.as_str(), |entry| {
                entry.requested = Some(Instant::now());
                entry.uncertain = reference_failed && !record.cleanup_confirmed;
            });
            let mut run = Run::new(id.clone());
            run.recovered = true;
            runs.insert(id.as_str().into(), run);
        }
        let emergency = Guard::start(supervisor.clone(), registry.clone()).map_err(|_| {
            registry.safety.fail();
            Error::Resources
        })?;
        Ok(Self {
            session,
            supervisor,
            registry,
            preparation,
            runs,
            emergency,
        })
    }
    pub fn set_paused(&mut self, paused: bool) -> Result<(), Error> {
        self.session.set_paused(paused).map_err(Error::State)
    }
    pub fn run_state(&mut self, id: &str) -> Result<Option<admission::RunState>, Error> {
        self.session.run_state(id).map_err(Error::State)
    }
    pub fn record(&mut self, id: &str) -> Result<Option<dispatch::Record>, Error> {
        self.session.record(id).map_err(Error::State)
    }
    pub fn active_runs(&self) -> usize {
        self.runs.len()
    }
    pub(crate) fn local_cleanup_pending(&self) -> bool {
        self.registry
            .snapshots()
            .iter()
            .any(|entry| entry.called && !entry.cleaned)
    }

    pub fn failed(&self) -> bool {
        self.registry.safety.failed()
    }
    pub fn cancel_active(&mut self, id: &str) -> Result<dispatch::StopDecision, Error> {
        let result = self.session.cancel_active(id).map_err(Error::State)?;
        if result == dispatch::StopDecision::Recorded {
            self.registry
                .update(id, |entry| entry.requested = Some(Instant::now()));
        }
        Ok(result)
    }
    pub fn start_next(&mut self) -> Result<Start, Error> {
        let permit = match self
            .session
            .claim_with_reclamation(|id| self.preparation.remove_logs(id))
            .map_err(Error::State)?
        {
            Claim::Blocked(block) => return Ok(Start::Blocked(block)),
            Claim::Ready(permit) => permit,
        };
        let id = permit.launch().run_id.clone();
        self.registry.insert(
            id.clone(),
            None,
            false,
            false,
            permit.launch().cleanup_grace,
        );
        let mut run = Run::new(id.clone());
        let preparation = Arc::clone(&self.preparation);
        let registry = self.registry.clone();
        let limit = self.session.limits().log_bytes_per_run;
        match std::thread::Builder::new()
            .name("rustward-prepare".into())
            .spawn(move || preparation.prepare(permit, limit, &registry))
        {
            Ok(job) => run.job = Some(job),
            Err(_) => {
                self.registry.safety.fail();
                let saved = self.session.retain_unstarted(id.as_str());
                run.final_report = Some(Ok(capture::Report::default()));
                self.runs.insert(id.as_str().into(), run);
                saved.map_err(Error::State)?;
                return Err(Error::Resources);
            }
        }
        self.runs.insert(id.as_str().into(), run);
        Ok(Start::Preparing(id))
    }
    pub fn tick(&mut self, now: Instant) -> Result<(), Error> {
        if self.emergency.failed() {
            self.registry.safety.fail();
            return Err(Error::Resources);
        }
        let mut error = self.session.decide_expired(now).err().map(Error::State);
        for id in self.runs.keys().cloned().collect::<Vec<_>>() {
            let mut run = self.runs.remove(&id).unwrap();
            match self.advance(&mut run, now) {
                Ok(true) => self.registry.remove(&id),
                Ok(false) => {
                    self.runs.insert(id, run);
                }
                Err(failure) => {
                    error.get_or_insert(failure);
                    self.runs.insert(id, run);
                }
            }
        }
        error.map_or(Ok(()), Err)
    }
}
impl<S: Supervisor + Clone + Send + 'static> Drop for Controller<S> {
    fn drop(&mut self) {
        if !self.runs.is_empty() {
            self.registry.safety.fail();
        }
    }
}

fn current_user() -> Result<u32, Error> {
    use std::io::Read;
    let uid = rustix::process::geteuid().as_raw();
    if uid == 0 {
        return Err(Error::Context);
    }
    let mut text = String::new();
    std::fs::File::open("/proc/self/status")
        .and_then(|file| file.take(16_385).read_to_string(&mut text))
        .map_err(|_| Error::Context)?;
    if text.len() > 16_384 {
        return Err(Error::Context);
    }
    for name in ["CapEff:", "CapPrm:", "CapInh:", "CapAmb:"] {
        let value = text
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .ok_or(Error::Context)?;
        if u64::from_str_radix(value.trim(), 16).map_err(|_| Error::Context)? != 0 {
            return Err(Error::Context);
        }
    }
    Ok(uid)
}

struct Bound<'a, S> {
    inner: &'a mut S,
    profile: &'a Profile,
    registry: &'a Registry,
}
impl<S: Supervisor> Supervisor for Bound<'_, S> {
    fn start(
        &mut self,
        launch: &supervision::Launch,
        output: supervision::Output<'_>,
    ) -> Result<supervision::StartOutcome, supervision::Error> {
        if self.inner.execution_uid() != Ok(self.profile.execution_uid()) {
            self.registry.safety.fail();
            return Err(supervision::Error::IdentityMismatch);
        }
        self.registry
            .update(launch.run_id.as_str(), |entry| entry.called = true);
        self.inner.start_controlled(launch, self.profile, output)
    }
    fn observe(
        &mut self,
        id: &RunId,
        expected: Option<&supervision::ExecutionIdentity>,
    ) -> Result<Observation, supervision::Error> {
        self.inner.observe(id, expected)
    }
    fn stop(
        &mut self,
        id: &RunId,
        expected: &supervision::ExecutionIdentity,
        mode: supervision::StopMode,
    ) -> Result<supervision::StopOutcome, supervision::Error> {
        self.inner.stop(id, expected, mode)
    }
    fn release(
        &mut self,
        id: &RunId,
        expected: &supervision::ExecutionIdentity,
    ) -> Result<(), supervision::Error> {
        self.inner.release(id, expected)
    }
}

#[cfg(test)]
pub(crate) mod tests;
