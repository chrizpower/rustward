use rustward::{
    supervision::*,
    systemd::{CgroupFs, Manager, Systemd, UserBus},
};
use std::{
    fs::File,
    os::fd::AsFd,
    thread,
    time::{Duration, Instant},
};

struct TestUnits(Vec<String>);
impl Drop for TestUnits {
    fn drop(&mut self) {
        if let Ok(mut bus) = UserBus::connect() {
            for unit in &self.0 {
                let _ = bus.stop(unit, StopMode::Graceful);
            }
        }
    }
}

fn wait_for(
    supervisor: &mut impl Supervisor,
    run_id: &RunId,
    expected: Option<&ExecutionIdentity>,
    accept: impl Fn(&Observation) -> bool,
) -> Observation {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let observed = supervisor.observe(run_id, expected).unwrap();
        if accept(&observed) {
            return observed;
        }
        assert!(
            Instant::now() < deadline,
            "observation deadline: {observed:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "requires an isolated Debian 13 host with a dedicated systemd user manager and cgroup v2"]
fn confirms_child_cleanup_and_concurrent_unit_isolation() {
    let mut supervisor = Systemd::new(UserBus::connect().unwrap(), CgroupFs::default()).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let output = tempfile::tempfile().unwrap();
    let error = File::options().write(true).open("/dev/null").unwrap();
    let mut cleanup = TestUnits(vec![]);
    let mut ids = vec![];
    for script in ["sleep 120 >/dev/null 2>&1 & printf synthetic", "sleep 120"] {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).unwrap();
        let run_id = RunId::new(&hex::encode(bytes)).unwrap();
        cleanup
            .0
            .push(format!("rustward-run-{}.service", run_id.as_str()));
        let launch = Launch {
            run_id: run_id.clone(),
            executable: "/bin/sh".into(),
            arguments: vec!["-c".into(), script.into()],
            working_directory: directory.path().to_str().unwrap().into(),
            cleanup_grace: Duration::from_millis(100),
        };
        assert_eq!(
            supervisor.start(
                &launch,
                Output {
                    stdout: output.as_fd(),
                    stderr: error.as_fd()
                }
            ),
            Ok(StartOutcome::Accepted)
        );
        ids.push(run_id);
    }
    let exited = wait_for(&mut supervisor, &ids[0], None, |observation| {
        matches!(
            observation,
            Observation::Known {
                state: ExecutionState::Exited,
                exit: Some(ProcessExit::Code(0)),
                cleanup: Cleanup::Pending,
                ..
            }
        )
    });
    let Observation::Known {
        identity: first, ..
    } = exited
    else {
        unreachable!()
    };
    let running = wait_for(&mut supervisor, &ids[1], None, |observation| {
        matches!(
            observation,
            Observation::Known {
                state: ExecutionState::Running,
                ..
            }
        )
    });
    let Observation::Known {
        identity: second, ..
    } = running
    else {
        unreachable!()
    };
    assert_ne!(first.invocation, second.invocation);
    assert_ne!(first.control_group, second.control_group);
    assert_eq!(
        supervisor.stop(&ids[0], &first, StopMode::Graceful),
        Ok(StopOutcome::Accepted)
    );
    wait_for(&mut supervisor, &ids[0], Some(&first), |observation| {
        matches!(
            observation,
            Observation::Known {
                cleanup: Cleanup::Confirmed,
                ..
            }
        )
    });
    assert!(matches!(
        supervisor.observe(&ids[1], Some(&second)).unwrap(),
        Observation::Known {
            state: ExecutionState::Running,
            ..
        }
    ));
    supervisor.release(&ids[0], &first).unwrap();
    assert_eq!(
        supervisor.stop(&ids[1], &second, StopMode::Graceful),
        Ok(StopOutcome::Accepted)
    );
    wait_for(&mut supervisor, &ids[1], Some(&second), |observation| {
        matches!(
            observation,
            Observation::Known {
                cleanup: Cleanup::Confirmed,
                ..
            }
        )
    });
    supervisor.release(&ids[1], &second).unwrap();
}
