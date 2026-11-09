use super::{Bound, Controller, Error, Run, emergency::valid_identity, preparation::Failure};
use crate::{
    admission,
    capture::{self, FailureKind},
    supervision::{Observation, Supervisor},
};
use std::time::Instant;

impl<S: Supervisor + Clone + Send + 'static> Controller<S> {
    pub(super) fn advance(&mut self, run: &mut Run, now: Instant) -> Result<bool, Error> {
        if run.failed_job {
            return Err(Error::Resources);
        }
        if run
            .job
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished)
        {
            let mut prepared = match run.job.take().unwrap().join() {
                Ok(result) => result,
                Err(_) => {
                    self.registry.safety.fail();
                    run.failed_job = true;
                    return Err(Error::Resources);
                }
            };
            run.capture = prepared.capture.take();
            run.work = prepared.work.take();
            let writers = prepared.writers.take();
            let saved = match prepared.failure {
                Some(Failure::Runtime(error)) => {
                    self.session.fail_before_start(prepared.permit, error)
                }
                Some(Failure::Capture(error)) => {
                    self.session.fail_capture_setup(prepared.permit, error)
                }
                None => {
                    let fault = run.capture.as_ref().and_then(capture::Capture::failure);
                    if self.registry.safety.failed() {
                        self.session
                            .abort_unstarted(prepared.permit)
                            .map(|()| run.failure_saved = true)
                    } else if let Some(failure) = fault {
                        drop(prepared.permit);
                        self.session
                            .output_failure(run.id.as_str(), failure)
                            .map(|_| run.failure_saved = true)
                    } else {
                        let profile = prepared.profile.as_ref().ok_or(Error::Context)?;
                        let mut bound = Bound {
                            inner: &mut self.supervisor,
                            profile,
                            registry: &self.registry,
                        };
                        self.session
                            .start(
                                prepared.permit,
                                &mut bound,
                                writers.as_ref().ok_or(Error::Resources)?.output(),
                            )
                            .map(|_| ())
                    }
                }
            };
            drop(writers);
            match saved {
                Err(admission::Error::Closed | admission::Error::InvalidTransition) => self
                    .session
                    .retain_unstarted(run.id.as_str())
                    .map_err(Error::State)?,
                result => result.map_err(Error::State)?,
            }
            if run.capture.is_none() {
                run.final_report = Some(Ok(capture::Report::default()));
            }
        }
        if run.job.is_some() {
            return Ok(false);
        }
        self.poll_run(run, now)
    }
    fn poll_run(&mut self, run: &mut Run, now: Instant) -> Result<bool, Error> {
        let id = run.id.as_str();
        if !run.failure_saved
            && let Some(failure) = run.capture.as_ref().and_then(capture::Capture::failure)
        {
            self.session
                .output_failure(id, failure)
                .map_err(Error::State)?;
            run.failure_saved = true;
        }
        let entries = self.registry.snapshots();
        let entry = entries
            .into_iter()
            .find(|entry| entry.id == run.id)
            .ok_or(Error::Resources)?;
        if entry.uncertain && !run.uncertainty_saved {
            self.session
                .observe(id, &Observation::Uncertain)
                .map_err(Error::State)?;
            run.uncertainty_saved = true;
        }
        let mut record = self
            .session
            .record(id)
            .map_err(Error::State)?
            .ok_or(Error::State(admission::Error::Storage))?;
        if entry.called && !run.released {
            let observed = self
                .supervisor
                .observe(&run.id, record.identity.as_ref())
                .unwrap_or(Observation::Uncertain);
            if let Observation::Known { identity, .. } = &observed
                && valid_identity(identity)
            {
                self.registry.update(id, |state| {
                    if state
                        .identity
                        .as_ref()
                        .is_none_or(|previous| previous == identity)
                    {
                        state.identity = Some(identity.clone());
                    }
                });
            }
            if run.last.as_ref() != Some(&observed)
                && !(record.start_state == "rejected"
                    && record.cleanup_confirmed
                    && observed == Observation::Missing)
            {
                self.session.observe(id, &observed).map_err(Error::State)?;
                run.last = Some(observed.clone());
            }
            record = self
                .session
                .record(id)
                .map_err(Error::State)?
                .ok_or(Error::State(admission::Error::Storage))?;
            self.registry.update(id, |state| {
                state.identity = record.identity.clone();
                state.cleaned |= record.cleanup_confirmed;
            });
            if record.cleanup_confirmed {
                match observed {
                    Observation::Missing => run.released = true,
                    Observation::Known { .. } => {
                        if let Some(identity) = &record.identity {
                            if self.supervisor.release(&run.id, identity).is_ok() {
                                run.released = true;
                            } else if !run.uncertainty_saved {
                                self.session
                                    .observe(id, &Observation::Uncertain)
                                    .map_err(Error::State)?;
                                run.uncertainty_saved = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        } else if !entry.called && record.cleanup_confirmed {
            run.released = true;
        }
        let state = self
            .session
            .run_state(id)
            .map_err(Error::State)?
            .ok_or(Error::State(admission::Error::Storage))?;
        if state.result.is_some() {
            self.registry.update(id, |entry| {
                entry.requested.get_or_insert(now);
            });
        }
        if run.final_report.is_none()
            && let Some(capture) = run.capture.as_mut()
            && let Some(report) = capture.try_finish()
        {
            if let Err(error) = report
                && !run.failure_saved
            {
                self.session
                    .output_failure(id, error.failure)
                    .map_err(Error::State)?;
                run.failure_saved = true;
            }
            run.final_report = Some(report);
            run.capture.take();
        }
        if !record.cleanup_confirmed || !run.released {
            return Ok(false);
        }
        if run.recovered && run.recovery.is_none() && run.final_report.is_none() {
            let preparation = self.preparation.clone();
            let id = run.id.clone();
            let limit = record.log_bytes;
            run.recovery = Some(
                std::thread::Builder::new()
                    .name("rustward-output-recovery".into())
                    .spawn(move || preparation.recover(&id, limit))
                    .map_err(|_| {
                        self.registry.safety.fail();
                        run.failed_job = true;
                        Error::Resources
                    })?,
            );
        }
        if run
            .recovery
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished)
        {
            run.final_report = Some(run.recovery.take().unwrap().join().map_err(|_| {
                self.registry.safety.fail();
                run.failed_job = true;
                Error::Resources
            })?);
        }
        let Some(report) = run.final_report else {
            return Ok(false);
        };
        if run.recovered && !run.failure_saved {
            let failure = report
                .err()
                .map(|error| error.failure)
                .unwrap_or(capture::Failure {
                    stream: capture::Stream::Stdout,
                    kind: FailureKind::Abandoned,
                });
            self.session
                .output_failure(id, failure)
                .map_err(Error::State)?;
            run.failure_saved = true;
        }
        if !run.recovered && entry.monitor.is_none() {
            self.session.finish(id, 0).map_err(Error::State)?;
            return Ok(true);
        }
        let (report, synchronized) = match report {
            Ok(report) => (report, true),
            Err(error) => (error.report, false),
        };
        let evidence = capture::Evidence::from_report(report, synchronized, run.recovered);
        self.session
            .finish_evidence(id, &evidence)
            .map_err(Error::State)?;
        Ok(true)
    }
}
