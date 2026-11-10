use super::*;
use crate::{
    admission::{Admission, RunPhase, RunResult, Store},
    dispatch::{Block, Limits},
    supervision::*,
};
use preparation::{Failure, Prepared};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Write,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

mod emergency;
mod failures;
mod lifecycle;
mod native;

#[derive(Default)]
struct State {
    launches: Vec<Launch>,
    references: Vec<(String, Option<ExecutionIdentity>)>,
    profiles: Vec<Profile>,
    writers: BTreeMap<String, (File, File)>,
    observations: BTreeMap<String, Observation>,
    stops: Vec<(String, StopMode)>,
    released: Vec<String>,
    reject: Option<StartOutcome>,
    uid: Option<u32>,
    release_failure: bool,
    stop_response: Option<StopOutcome>,
    panic_stop: bool,
}
#[derive(Clone, Default)]
struct Fake(Arc<Mutex<State>>);
fn identity() -> ExecutionIdentity {
    ExecutionIdentity {
        invocation: [1; 16],
        control_group: "/synthetic/run".into(),
    }
}
fn known(ended: bool) -> Observation {
    Observation::Known {
        identity: identity(),
        state: if ended {
            ExecutionState::Ended
        } else {
            ExecutionState::Running
        },
        exit: ended.then_some(ProcessExit::Code(0)),
        cleanup: if ended {
            Cleanup::Confirmed
        } else {
            Cleanup::Pending
        },
    }
}
impl Supervisor for Fake {
    fn reference(
        &mut self,
        id: &RunId,
        expected: Option<&ExecutionIdentity>,
    ) -> Result<(), supervision::Error> {
        self.0
            .lock()
            .unwrap()
            .references
            .push((id.as_str().into(), expected.cloned()));
        Ok(())
    }
    fn execution_uid(&mut self) -> Result<u32, supervision::Error> {
        Ok(self.0.lock().unwrap().uid.unwrap_or(1000))
    }
    fn start(&mut self, _: &Launch, _: Output<'_>) -> Result<StartOutcome, supervision::Error> {
        panic!("uncontrolled start")
    }
    fn start_controlled(
        &mut self,
        launch: &Launch,
        profile: &Profile,
        output: Output<'_>,
    ) -> Result<StartOutcome, supervision::Error> {
        let mut state = self.0.lock().unwrap();
        state.launches.push(launch.clone());
        state.profiles.push(profile.clone());
        let reply = state.reject.unwrap_or(StartOutcome::Accepted);
        if reply != StartOutcome::Rejected {
            let mut out = File::from(output.stdout.try_clone_to_owned().unwrap());
            let mut err = File::from(output.stderr.try_clone_to_owned().unwrap());
            out.write_all(b"synthetic output").unwrap();
            err.write_all(b"synthetic error").unwrap();
            state
                .writers
                .insert(launch.run_id.as_str().into(), (out, err));
            state
                .observations
                .insert(launch.run_id.as_str().into(), known(false));
        }
        Ok(reply)
    }
    fn observe(
        &mut self,
        id: &RunId,
        expected: Option<&ExecutionIdentity>,
    ) -> Result<Observation, supervision::Error> {
        let state = self.0.lock().unwrap();
        let observed = state
            .observations
            .get(id.as_str())
            .cloned()
            .unwrap_or(Observation::Missing);
        if let (Some(expected), Observation::Known { identity, .. }) = (expected, &observed) {
            assert_eq!(expected, identity);
        }
        Ok(observed)
    }
    fn stop(
        &mut self,
        id: &RunId,
        expected: &ExecutionIdentity,
        mode: StopMode,
    ) -> Result<StopOutcome, supervision::Error> {
        assert_eq!(expected, &identity());
        let mut state = self.0.lock().unwrap();
        state.stops.push((id.as_str().into(), mode));
        let panic_stop = state.panic_stop;
        let response = state.stop_response.unwrap_or(StopOutcome::Accepted);
        drop(state);
        assert!(!panic_stop, "synthetic supervisor panic");
        Ok(response)
    }
    fn release(
        &mut self,
        id: &RunId,
        expected: &ExecutionIdentity,
    ) -> Result<(), supervision::Error> {
        assert_eq!(expected, &identity());
        let mut state = self.0.lock().unwrap();
        if state.release_failure {
            return Err(supervision::Error::Unavailable);
        }
        state.released.push(id.as_str().into());
        state.writers.remove(id.as_str());
        Ok(())
    }
}
struct Prepare {
    logs: PathBuf,
    hold: AtomicBool,
    entered: AtomicBool,
    hold_after_capture: AtomicBool,
    failure: Mutex<Option<Failure>>,
}
impl Preparation for Prepare {
    fn prepare(&self, permit: dispatch::Permit, limit: u64, registry: &Registry) -> Prepared {
        self.entered.store(true, Ordering::Release);
        while self.hold.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(1));
        }
        let failure = self.failure.lock().unwrap().take();
        if matches!(
            failure,
            Some(Failure::Capture(capture::SetupError::Storage))
                | Some(Failure::Runtime(dispatch::LaunchFailure::WorkStorage))
        ) {
            registry.safety.fail();
        }
        let (capture, writers) = if failure.is_none() {
            let (capture, writers) = Capture::create(
                &self.logs,
                &permit.launch().run_id,
                capture::Options {
                    retained_bytes: limit,
                },
            )
            .unwrap();
            registry.monitor(permit.launch().run_id.as_str(), capture.monitor());
            (Some(capture), Some(writers))
        } else {
            (None, None)
        };
        while self.hold_after_capture.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(1));
        }
        Prepared {
            permit,
            capture,
            writers,
            work: None,
            profile: Some(
                Profile::for_run(1000, Path::new("/synthetic/runtime"), &self.logs).unwrap(),
            ),
            failure,
        }
    }
    fn recover(&self, id: &RunId, limit: u64) -> Result<capture::Report, capture::CaptureError> {
        capture::recover(&self.logs, id, limit)
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    path: PathBuf,
    store: Store,
    preparation: Arc<Prepare>,
    fake: Fake,
    safety: Safety,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.db");
        let store = Store::open(&path).unwrap();
        let logs = root.path().join("logs");
        fs::create_dir(&logs).unwrap();
        fs::set_permissions(&logs, fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            _root: root,
            path,
            store,
            preparation: Arc::new(Prepare {
                logs,
                hold: AtomicBool::new(false),
                entered: AtomicBool::new(false),
                hold_after_capture: AtomicBool::new(false),
                failure: Mutex::new(None),
            }),
            fake: Fake::default(),
            safety: Safety::default(),
        }
    }
    fn admit(&mut self) -> (String, String) {
        let key = self.store.issue_key().unwrap();
        let config = crate::config::load("examples/rustward.toml").unwrap();
        let Admission::Admitted { run_id } = self
            .store
            .admit(
                &key,
                &config,
                "synthetic-report",
                &BTreeMap::from([("name".into(), "sample".into())]),
                "runtime-1",
            )
            .unwrap()
        else {
            panic!()
        };
        (key, run_id)
    }
    fn controller(&self, limits: Limits) -> Controller<Fake> {
        let session = Session::open_with_safety(&self.path, limits, self.safety.clone()).unwrap();
        Controller::from_parts(
            session,
            self.fake.clone(),
            self.preparation.clone(),
            self.safety.clone(),
        )
        .unwrap()
    }
    fn ended(&self, id: &str) {
        self.fake
            .0
            .lock()
            .unwrap()
            .observations
            .insert(id.into(), known(true));
    }
}
fn until(mut condition: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < end, "condition did not complete");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn started(controller: &mut Controller<Fake>, fixture: &Fixture) {
    until(|| {
        controller.tick(Instant::now()).unwrap();
        !fixture.fake.0.lock().unwrap().launches.is_empty()
    });
}
fn completed(controller: &mut Controller<Fake>, id: &str) {
    until(|| {
        controller.tick(Instant::now()).unwrap();
        controller.run_state(id).unwrap().unwrap().phase == RunPhase::Completed
    });
}
