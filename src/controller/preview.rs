use crate::{admission, capture, dispatch, runtime::WorkRoot, safety::Safety, supervision::*};
use std::{
    collections::BTreeMap,
    fs::File,
    io::Write,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use super::{
    Controller, Registry,
    preparation::{Failure, Preparation, Prepared},
};

pub(crate) fn controller(
    path: &Path,
    logs: WorkRoot,
    safety: Safety,
) -> Result<Controller<Fake>, super::Error> {
    let session = dispatch::Session::open_with_safety(
        path,
        dispatch::Limits {
            log_bytes_per_run: 2048,
            ..dispatch::Limits::default()
        },
        safety.clone(),
    )
    .map_err(super::Error::State)?;
    Controller::from_parts(session, Fake::default(), Arc::new(Prepare(logs)), safety)
}

struct Prepare(WorkRoot);

impl Preparation for Prepare {
    fn prepare(&self, permit: dispatch::Permit, limit: u64, registry: &Registry) -> Prepared {
        let mut prepared = Prepared {
            permit,
            capture: None,
            writers: None,
            work: None,
            profile: None,
            failure: None,
        };
        match capture::Capture::create(
            self.0.path(),
            &prepared.permit.launch().run_id,
            capture::Options {
                retained_bytes: limit,
            },
        ) {
            Ok((capture, writers)) => {
                registry.monitor(prepared.permit.launch().run_id.as_str(), capture.monitor());
                prepared.capture = Some(capture);
                prepared.writers = Some(writers);
                prepared.profile = Some(
                    Profile::for_run(1000, Path::new("/synthetic/runtime"), self.0.path()).unwrap(),
                );
            }
            Err(error) => prepared.failure = Some(Failure::Capture(error)),
        }
        prepared
    }

    fn remove_logs(&self, id: &RunId) -> Result<(), admission::Error> {
        let directory = self
            .0
            .checked_directory()
            .map_err(|_| admission::Error::Storage)?;
        capture::remove_logs(directory, id).map_err(|_| admission::Error::Storage)
    }

    fn recover(&self, id: &RunId, limit: u64) -> Result<capture::Report, capture::CaptureError> {
        capture::recover(self.0.path(), id, limit)
    }
}

struct Run {
    end: Instant,
    review: bool,
    failure: bool,
    stdout: Option<File>,
}

#[derive(Clone, Default)]
pub(crate) struct Fake(Arc<Mutex<BTreeMap<String, Run>>>);

fn identity(id: &RunId) -> ExecutionIdentity {
    ExecutionIdentity {
        invocation: [1; 16],
        control_group: format!("/synthetic/{}", id.as_str()),
    }
}

impl Supervisor for Fake {
    fn execution_uid(&mut self) -> Result<u32, Error> {
        Ok(1000)
    }

    fn start(&mut self, launch: &Launch, output: Output<'_>) -> Result<StartOutcome, Error> {
        let mut stdout = File::from(
            output
                .stdout
                .try_clone_to_owned()
                .map_err(|_| Error::Unavailable)?,
        );
        let mut stderr = File::from(
            output
                .stderr
                .try_clone_to_owned()
                .map_err(|_| Error::Unavailable)?,
        );
        stdout
            .write_all(b"Task started\n<img src=x onerror=alert(1)>\n")
            .map_err(|_| Error::Unavailable)?;
        stderr
            .write_all(b"Synthetic diagnostic\n")
            .map_err(|_| Error::Unavailable)?;
        let kind = launch.arguments.first().map(String::as_str);
        if kind == Some("flood") {
            stdout
                .write_all(&[b'x'; 4096])
                .map_err(|_| Error::Unavailable)?;
        }
        self.0.lock().unwrap().insert(
            launch.run_id.as_str().into(),
            Run {
                end: Instant::now()
                    + Duration::from_secs(if kind == Some("hold") { 30 } else { 2 }),
                review: kind == Some("review"),
                failure: kind == Some("failure"),
                stdout: Some(stdout),
            },
        );
        Ok(StartOutcome::Accepted)
    }

    fn start_controlled(
        &mut self,
        launch: &Launch,
        _: &Profile,
        output: Output<'_>,
    ) -> Result<StartOutcome, Error> {
        self.start(launch, output)
    }

    fn observe(&mut self, id: &RunId, _: Option<&ExecutionIdentity>) -> Result<Observation, Error> {
        let mut runs = self.0.lock().unwrap();
        let Some(run) = runs.get_mut(id.as_str()) else {
            return Ok(Observation::Missing);
        };
        let ended = Instant::now() >= run.end;
        if ended && let Some(mut stdout) = run.stdout.take() {
            let _ = stdout.write_all(b"Task finished\n");
        }
        Ok(Observation::Known {
            identity: identity(id),
            state: if ended {
                ExecutionState::Ended
            } else {
                ExecutionState::Running
            },
            exit: (ended && !run.review).then_some(ProcessExit::Code(i32::from(run.failure))),
            cleanup: if ended {
                Cleanup::Confirmed
            } else {
                Cleanup::Pending
            },
        })
    }

    fn stop(
        &mut self,
        id: &RunId,
        _: &ExecutionIdentity,
        _: StopMode,
    ) -> Result<StopOutcome, Error> {
        let mut runs = self.0.lock().unwrap();
        let run = runs.get_mut(id.as_str()).ok_or(Error::Unavailable)?;
        run.end = Instant::now();
        Ok(StopOutcome::Accepted)
    }

    fn release(&mut self, id: &RunId, _: &ExecutionIdentity) -> Result<(), Error> {
        self.0.lock().unwrap().remove(id.as_str());
        Ok(())
    }
}
